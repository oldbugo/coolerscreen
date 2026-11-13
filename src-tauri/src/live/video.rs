//! Animated background: a clip decoded by the bundled ffmpeg as raw RGBA frames at canvas size, looping forever.
//! The clip keeps its own frame rate (an 18 fps file delivers 18 frames a second; the render fps is a separate setting),
//! paced in real time with -re. Decoding tries the GPU first (CUDA decode and scale on NVIDIA, then D3D11 decode with
//! software scaling) and falls back to software if ffmpeg cannot start that way. The reader keeps only the newest frame.
//! Environment overrides for experiments: CS_VIDEO_DECODE = software | auto | d3d11va | cuda | dxva2, CS_VIDEO_THREADS.
use crate::core::sync::MutexExt;
use super::{LH, LW};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FRAME_BYTES: usize = LW * LH * 4;

pub struct VideoBg {
    pub latest: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    stop: Arc<AtomicBool>,
    /// Raw process handle of ffmpeg (for CPU/RAM measurement).
    pub child_handle: Arc<Mutex<Option<usize>>>,
    /// How the clip is being decoded right now ("d3d11va", "software", ...), for logs and measurements.
    pub decode: Arc<Mutex<String>>,
    /// The running ffmpeg, so Drop kills this one only (not other ffmpegs the app has started).
    proc: Arc<Mutex<Option<Child>>>,
    thread: Option<std::thread::JoinHandle<()>>,
    /// Number of frames delivered since start (the loop position is this modulo the loop length).
    pub index: Arc<AtomicU64>,
}

impl VideoBg {
    /// `decode`: "auto" (GPU, falling back to software), "software", or an ffmpeg hwaccel name.
    pub fn start(clip: &Path, decode: &str) -> Result<VideoBg, String> {
        VideoBg::start_inner(clip, decode, false)
    }

    /// A stored base version (portrait MJPEG 1080x2400, made with transpose=1): decoded back to landscape, always in software
    /// (the GPU decoders do not take MJPEG, and it is cheap).
    pub fn start_base(clip: &Path) -> Result<VideoBg, String> {
        VideoBg::start_inner(clip, "software", true)
    }

    fn start_inner(clip: &Path, decode: &str, portrait: bool) -> Result<VideoBg, String> {
        let ffmpeg = crate::media::ffmpeg::locate();
        if !ffmpeg.exists() {
            return Err(format!("ffmpeg not found at {}", ffmpeg.display()));
        }
        if !clip.exists() {
            return Err(format!("background clip not found: {}", clip.display()));
        }
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = Arc::new(Mutex::new(None));
        let mode = std::env::var("CS_VIDEO_DECODE").unwrap_or_else(|_| decode.to_string());
        let how = Arc::new(Mutex::new(String::new()));
        let proc: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let index = Arc::new(AtomicU64::new(0));
        let (l2, s2, h2, hw2, p2, clip, ix2) = (latest.clone(), stop.clone(), handle.clone(), how.clone(), proc.clone(), clip.to_path_buf(), index.clone());
        let thread = std::thread::spawn(move || {
            // Decode methods to try in order; a start that yields no frame within 6 s moves on to the next one.
            let mut chain: Vec<String> = match if portrait { "software" } else { mode.as_str() } {
                "software" | "" => vec![],
                // Big sources (4K): CUDA decode+scale roughly halves ffmpeg CPU but costs ~100 MB more RAM; smaller ones: D3D11.
                "auto" if source_width(&ffmpeg, &clip).is_some_and(|w| w >= 3000) => vec!["cuda".into(), "d3d11va".into()],
                "auto" => vec!["d3d11va".into()],
                other => vec![other.to_string()],
            };
            chain.push("software".into());
            let mut at = 0usize;
            while !s2.load(Ordering::Relaxed) {
                let accel: &str = &chain[at];
                let Some(mut child) = spawn(&ffmpeg, &clip, accel, portrait, true) else {
                    std::thread::sleep(Duration::from_secs(2));
                    continue;
                };
                crate::media::sys::adopt_child(&child);
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsRawHandle;
                    *h2.lock_or_recover() = Some(child.as_raw_handle() as usize);
                }
                *hw2.lock_or_recover() = accel.to_string();
                let started = Instant::now();
                let mut got = false;
                let mut out = child.stdout.take().unwrap();
                *p2.lock_or_recover() = Some(child);
                loop {
                    let mut buf = vec![0u8; FRAME_BYTES];
                    if s2.load(Ordering::Relaxed) || out.read_exact(&mut buf).is_err() {
                        break;
                    }
                    got = true;
                    *l2.lock_or_recover() = Some(Arc::new(buf));
                    ix2.fetch_add(1, Ordering::Relaxed);
                }
                *h2.lock_or_recover() = None;
                if let Some(mut c) = p2.lock_or_recover().take() {
                    let _ = c.kill();
                    let _ = c.wait();
                }
                if !got && accel != "software" && started.elapsed() < Duration::from_secs(6) {
                    crate::device::log(&format!("companion: video decode with {accel} failed, trying the next method"));
                    at += 1;
                    continue;
                }
                if !s2.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        });
        Ok(VideoBg { latest, stop, child_handle: handle, decode: how, proc, thread: Some(thread), index })
    }

    pub fn frame(&self) -> Option<Arc<Vec<u8>>> {
        self.latest.lock_or_recover().clone()
    }
}

/// Width of the clip's video stream (from ffmpeg's own banner), if it can be read quickly.
fn source_width(ffmpeg: &Path, clip: &Path) -> Option<u32> {
    let t = crate::media::ffmpeg::probe_text_timeout(ffmpeg, clip, Duration::from_secs(5)).ok()?;
    let line = t.lines().find(|l| l.contains("Video:"))?;
    let re = regex::Regex::new(r"\b(\d{3,5})x(\d{3,5})\b").ok()?;
    re.captures(line)?.get(1)?.as_str().parse().ok()
}

fn spawn(ffmpeg: &Path, clip: &Path, accel: &str, portrait: bool, looping: bool) -> Option<Child> {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error"]);
    // cuda: decode and scale on the GPU, then download only the small canvas-size frame (a big saving for 4K clips).
    // other hwaccels: decode on the GPU, frames come back to system memory for the software scaler.
    let vf = if portrait {
        // The stored base is landscape rotated clockwise (transpose=1); transpose=2 turns it back.
        format!("transpose=2,scale={LW}:{LH}:flags=fast_bilinear,format=rgba")
    } else {
        match accel {
            "software" => format!("scale={LW}:{LH}:force_original_aspect_ratio=increase:flags=fast_bilinear,crop={LW}:{LH},format=rgba"),
            "cuda" => {
                c.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
                format!("scale_cuda=w={LW}:h={LH}:force_original_aspect_ratio=increase:force_divisible_by=2,hwdownload,format=nv12,crop={LW}:{LH},format=rgba")
            }
            other => {
                c.args(["-hwaccel", other]);
                format!("scale={LW}:{LH}:force_original_aspect_ratio=increase:flags=fast_bilinear,crop={LW}:{LH},format=rgba")
            }
        }
    };
    // Few decoder threads (each holds its own 4K frames in flight: this is where most of ffmpeg's memory goes) and a small probe.
    let threads = std::env::var("CS_VIDEO_THREADS").unwrap_or_else(|_| "2".into());
    c.args(["-threads", &threads, "-probesize", "2M", "-analyzeduration", "0"]);
    if looping {
        c.args(["-re", "-stream_loop", "-1"]);
    }
    c.arg("-i").arg(clip).args(["-an", "-sn", "-filter_threads", "1", "-vf"]).arg(vf);
    if !looping {
        c.args(["-frames:v", "1"]);
    }
    c.args(["-fps_mode", "passthrough", "-f", "rawvideo", "-"]).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.spawn().ok()
}

/// The first frame of a clip, landscape 1680x756 RGBA, with the same scaling the live decoder uses (`portrait`: a stored
/// base). One short ffmpeg run (about 100-300 ms); callers cache it.
pub fn first_frame(clip: &Path, portrait: bool) -> Result<Vec<u8>, String> {
    let ffmpeg = crate::media::ffmpeg::locate();
    if !ffmpeg.exists() {
        return Err(format!("ffmpeg not found at {}", ffmpeg.display()));
    }
    if !clip.exists() {
        return Err(format!("clip not found: {}", clip.display()));
    }
    let mut child = spawn(&ffmpeg, clip, "software", portrait, false).ok_or("could not start ffmpeg")?;
    let mut out = child.stdout.take().ok_or("no ffmpeg output")?;
    let mut buf = vec![0u8; FRAME_BYTES];
    let r = out.read_exact(&mut buf);
    let _ = child.kill();
    let _ = child.wait();
    r.map_err(|e| format!("no frame decoded from {}: {e}", clip.display()))?;
    Ok(buf)
}

impl Drop for VideoBg {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Killing ffmpeg unblocks the reader's read_exact (only this decoder: the app may have other ffmpegs running).
        if let Some(c) = self.proc.lock_or_recover().as_mut() {
            let _ = c.kill();
        }
        if let Some(t) = self.thread.take() {
            // Bounded: a decoder thread stuck in a blocking read must never keep a stop (or a test) from finishing.
            super::pipeline::join_bounded(vec![t], Duration::from_millis(2500));
        }
    }
}
