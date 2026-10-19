//! 4.0 stream B2: the PC's master volume and mute (default playback device, Windows Core Audio endpoint volume). Read only: we
//! never change the volume. A small thread polls 4 times a second while a widget asks for it and ends a few seconds after.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vol {
    /// 0..1
    pub level: f32,
    pub muted: bool,
}

#[derive(Default)]
struct State {
    vol: Option<Vol>,
    last_used: Option<Instant>,
    running: bool,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State::default()))
}

static STOP: AtomicBool = AtomicBool::new(false);
const KEEP: Duration = Duration::from_secs(4);

/// The latest volume (None until the first poll, or when there is no playback device). Registers the demand.
pub fn read() -> Option<Vol> {
    let mut g = state().lock().unwrap();
    g.last_used = Some(Instant::now());
    if !g.running {
        g.running = true;
        STOP.store(false, Ordering::Relaxed);
        std::thread::spawn(run);
    }
    g.vol
}

pub fn shutdown() {
    STOP.store(true, Ordering::Relaxed);
}

fn wanted() -> bool {
    !STOP.load(Ordering::Relaxed) && state().lock().unwrap().last_used.is_some_and(|t| t.elapsed() < KEEP)
}

fn run() {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::*;
    use windows::Win32::System::Com::*;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let mut ep: Option<IAudioEndpointVolume> = None;
        while wanted() {
            if ep.is_none() {
                ep = (|| -> windows::core::Result<IAudioEndpointVolume> {
                    let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
                    let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
                    dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
                })()
                .ok();
            }
            let v = ep.as_ref().and_then(|e| {
                let level = e.GetMasterVolumeLevelScalar().ok()?;
                let muted = e.GetMute().ok()?.as_bool();
                Some(Vol { level: level.clamp(0.0, 1.0), muted })
            });
            if v.is_none() {
                ep = None; // the default device may have changed: look again next time
            }
            state().lock().unwrap().vol = v;
            std::thread::sleep(Duration::from_millis(250));
        }
        let mut g = state().lock().unwrap();
        g.running = false;
        g.vol = None;
        drop(ep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_is_read_when_there_is_a_device() {
        let _ = read();
        std::thread::sleep(Duration::from_millis(900));
        let v = read();
        // a box without audio gives None; with a device the level is a fraction
        if let Some(v) = v {
            assert!((0.0..=1.0).contains(&v.level));
        }
        eprintln!("volume now: {v:?}");
    }
}
