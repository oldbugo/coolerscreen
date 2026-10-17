//! Audio levels for the `visualizer` widget: WASAPI loopback of the default output device (what the PC is playing; no
//! microphone, no permission prompt), 2048-point FFT, 64 log-spaced bands from 40 Hz to 16 kHz.
//! Implemented directly on the `windows` crate (no extra crate; only two more feature flags). The capture thread runs only
//! while a visualizer asks for levels and ends a few seconds after the last ask. Silence (no packets) decays to flat.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const BANDS: usize = 64;
const N: usize = 2048;
const KEEP: Duration = Duration::from_secs(4);

#[derive(Default)]
struct State {
    bands: Vec<f32>,
    last_used: Option<Instant>,
    running: bool,
}

pub struct Audio {
    s: Mutex<State>,
    stop: AtomicBool,
}

pub fn audio() -> &'static Arc<Audio> {
    static A: OnceLock<Arc<Audio>> = OnceLock::new();
    A.get_or_init(|| Arc::new(Audio { s: Mutex::new(State::default()), stop: AtomicBool::new(false) }))
}

impl Audio {
    /// The latest band levels (0..1 each, `BANDS` of them); registers the demand and starts the capture thread when needed.
    pub fn levels(self: &Arc<Self>) -> Vec<f32> {
        let mut g = self.s.lock().unwrap();
        g.last_used = Some(Instant::now());
        if !g.running {
            g.running = true;
            self.stop.store(false, Ordering::Relaxed);
            let me = self.clone();
            std::thread::spawn(move || me.run());
        }
        if g.bands.len() != BANDS { vec![0.0; BANDS] } else { g.bands.clone() }
    }

    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn wanted(&self) -> bool {
        !self.stop.load(Ordering::Relaxed) && self.s.lock().unwrap().last_used.is_some_and(|t| t.elapsed() < KEEP)
    }

    fn run(self: Arc<Self>) {
        while self.wanted() {
            if let Err(e) = self.capture() {
                crate::device::log(&format!("visualizer: audio capture not available ({e})"));
                // no device / device changed: try again in a bit, levels stay flat
                for _ in 0..20 {
                    if !self.wanted() {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }
        let mut g = self.s.lock().unwrap();
        g.running = false;
        g.bands.clear();
    }

    fn set_bands(&self, b: Vec<f32>) {
        self.s.lock().unwrap().bands = b;
    }

    fn capture(&self) -> Result<(), String> {
        use windows::Win32::Media::Audio::*;
        use windows::Win32::System::Com::*;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let r = (|| -> windows::core::Result<()> {
                let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
                let client: IAudioClient = dev.Activate(CLSCTX_ALL, None)?;
                let fmt = client.GetMixFormat()?;
                let (channels, rate, bits, tag) = {
                    let f = &*fmt;
                    (std::ptr::addr_of!(f.nChannels).read_unaligned() as usize, std::ptr::addr_of!(f.nSamplesPerSec).read_unaligned(), std::ptr::addr_of!(f.wBitsPerSample).read_unaligned(), std::ptr::addr_of!(f.wFormatTag).read_unaligned())
                };
                // 0xFFFE = extensible: shared-mode mix formats are float32 there; 3 = IEEE float
                let float = tag == 3 || (tag == 0xFFFE && bits == 32);
                client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, 2_000_000, 0, fmt, None)?;
                let cap: IAudioCaptureClient = client.GetService()?;
                client.Start()?;
                let mut ring = vec![0f32; N];
                let mut pos = 0usize;
                let mut smooth = vec![0f32; BANDS];
                let mut last_fft = Instant::now();
                let mut last_packet = Instant::now();
                let plan = Fft::new(N);
                while self.wanted() {
                    let n = cap.GetNextPacketSize()?;
                    if n == 0 {
                        std::thread::sleep(Duration::from_millis(8));
                    }
                    let mut got = false;
                    let mut left = n;
                    while left > 0 {
                        let (mut data, mut frames, mut flags) = (std::ptr::null_mut::<u8>(), 0u32, 0u32);
                        cap.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                        let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                        for i in 0..frames as usize {
                            // mono mix of the first two channels
                            let mut acc = 0f32;
                            if !silent {
                                for c in 0..channels.min(2) {
                                    let idx = i * channels + c;
                                    acc += if float {
                                        (data as *const f32).add(idx).read_unaligned()
                                    } else if bits == 16 {
                                        (data as *const i16).add(idx).read_unaligned() as f32 / 32768.0
                                    } else {
                                        (data as *const i32).add(idx).read_unaligned() as f32 / 2147483648.0
                                    };
                                }
                                acc /= channels.min(2) as f32;
                            }
                            ring[pos] = acc;
                            pos = (pos + 1) % N;
                        }
                        cap.ReleaseBuffer(frames)?;
                        got = true;
                        last_packet = Instant::now();
                        left = cap.GetNextPacketSize()?;
                    }
                    if last_fft.elapsed() >= Duration::from_millis(33) {
                        last_fft = Instant::now();
                        let target: Vec<f32> = if last_packet.elapsed() > Duration::from_millis(250) && !got {
                            vec![0.0; BANDS]
                        } else {
                            // unroll the ring (oldest first) and analyse
                            let mut frame: Vec<f32> = (0..N).map(|i| ring[(pos + i) % N]).collect();
                            bands_of(&plan, &mut frame, rate as f32)
                        };
                        for (s, t) in smooth.iter_mut().zip(&target) {
                            *s = if *t > *s { *t } else { *s * 0.82 + *t * 0.18 };
                        }
                        self.set_bands(smooth.clone());
                    }
                }
                client.Stop()?;
                Ok(())
            })();
            CoUninitialize();
            r.map_err(|e| e.to_string())
        }
    }
}

/// Radix-2 complex FFT with precomputed twiddles.
pub struct Fft {
    n: usize,
    cos: Vec<f32>,
    sin: Vec<f32>,
    window: Vec<f32>,
}

impl Fft {
    pub fn new(n: usize) -> Fft {
        let cos = (0..n / 2).map(|k| (-2.0 * std::f32::consts::PI * k as f32 / n as f32).cos()).collect();
        let sin = (0..n / 2).map(|k| (-2.0 * std::f32::consts::PI * k as f32 / n as f32).sin()).collect();
        let window = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (n as f32 - 1.0)).cos()).collect();
        Fft { n, cos, sin, window }
    }

    /// Magnitudes of bins 0..n/2 for a real signal (windowed), scaled so a full-scale sine is about 1.
    pub fn magnitudes(&self, x: &[f32]) -> Vec<f32> {
        let n = self.n;
        let mut re: Vec<f32> = x.iter().zip(&self.window).map(|(a, w)| a * w).collect();
        let mut im = vec![0f32; n];
        // bit reversal
        let bits = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - bits);
            if j > i {
                re.swap(i, j);
            }
        }
        let mut len = 2;
        while len <= n {
            let step = n / len;
            for start in (0..n).step_by(len) {
                for k in 0..len / 2 {
                    let (c, s) = (self.cos[k * step], self.sin[k * step]);
                    let (a, b) = (start + k, start + k + len / 2);
                    let (tr, ti) = (re[b] * c - im[b] * s, re[b] * s + im[b] * c);
                    re[b] = re[a] - tr;
                    im[b] = im[a] - ti;
                    re[a] += tr;
                    im[a] += ti;
                }
            }
            len *= 2;
        }
        (0..n / 2).map(|k| (re[k] * re[k] + im[k] * im[k]).sqrt() * 4.0 / n as f32).collect()
    }
}

/// 64 log-spaced band levels (0..1) from one frame of mono samples.
pub fn bands_of(plan: &Fft, frame: &mut [f32], rate: f32) -> Vec<f32> {
    let mags = plan.magnitudes(frame);
    let n = plan.n as f32;
    let (lo, hi) = (40.0f32, (16000.0f32).min(rate / 2.0 - 100.0));
    let mut out = vec![0f32; BANDS];
    for (b, o) in out.iter_mut().enumerate() {
        let f0 = lo * (hi / lo).powf(b as f32 / BANDS as f32);
        let f1 = lo * (hi / lo).powf((b + 1) as f32 / BANDS as f32);
        let (i0, i1) = (((f0 * n / rate) as usize).max(1), ((f1 * n / rate).ceil() as usize).max(2));
        let i1 = i1.min(mags.len()).max(i0 + 1).min(mags.len());
        let peak = mags[i0.min(mags.len() - 1)..i1].iter().cloned().fold(0.0f32, f32::max);
        // dB scale: -66 dB..-6 dB mapped to 0..1, with a slight tilt that lifts the quieter highs
        let db = 20.0 * peak.max(1e-6).log10() + 3.0 * (b as f32 / BANDS as f32);
        *o = ((db + 66.0) / 60.0).clamp(0.0, 1.0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sine_lights_the_matching_band() {
        let rate = 48000.0;
        let plan = Fft::new(N);
        let mut x: Vec<f32> = (0..N).map(|i| 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / rate).sin()).collect();
        let b = bands_of(&plan, &mut x, rate);
        let top = b.iter().enumerate().max_by(|a, c| a.1.partial_cmp(c.1).unwrap()).unwrap().0;
        // 1 kHz on the 40 Hz..16 kHz log scale: ln(1000/40) / ln(400) * 64 = 34.4
        assert!((33..=36).contains(&top), "band {top}");
        assert!(b[top] > 0.7);
        let mut silence = vec![0f32; N];
        assert!(bands_of(&plan, &mut silence, rate).iter().all(|v| *v == 0.0));
    }
}
