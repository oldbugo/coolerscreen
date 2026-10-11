//! Records one window to an H.264 MP4 with Windows Graphics Capture (the `windows-capture` crate): it works even when
//! other windows cover the window. Frames are paced to a constant rate (the latest captured frame is repeated when the
//! window did not change) and piped as raw BGRA into the ffmpeg sidecar, which crops any title bar and encodes.
use super::{err, Cancel, Progress, Result, SourceError};
use std::io::{Read, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows_capture::capture::{Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings, MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

pub struct Params {
    pub seconds: f32,
    pub fps: u32,
    /// Capture the mouse pointer (always false in the app; true only to prove the setting works in tests).
    pub cursor: bool,
    pub out: PathBuf,
    pub ffmpeg: PathBuf,
}

#[derive(Default)]
struct Latest {
    w: u32,
    h: u32,
    seq: u64,
    data: Vec<u8>,
}
type Shared = Arc<Mutex<Latest>>;

struct Handler {
    shared: Shared,
    scratch: Vec<u8>,
}
impl GraphicsCaptureApiHandler for Handler {
    type Flags = Shared;
    type Error = Box<dyn std::error::Error + Send + Sync>;
    fn new(ctx: Context<Self::Flags>) -> std::result::Result<Self, Self::Error> {
        Ok(Handler { shared: ctx.flags, scratch: Vec::new() })
    }
    fn on_frame_arrived(&mut self, frame: &mut Frame, _c: InternalCaptureControl) -> std::result::Result<(), Self::Error> {
        let buf = frame.buffer()?;
        let (w, h) = (buf.width(), buf.height());
        let data = buf.as_nopadding_buffer(&mut self.scratch);
        let mut l = self.shared.lock().unwrap_or_else(|p| p.into_inner());
        l.data.clear();
        l.data.extend_from_slice(data);
        l.w = w;
        l.h = h;
        l.seq += 1;
        Ok(())
    }
}

/// Crop rectangle (width, height, y offset) keeping the bottom part with the screen's 20:9 shape, even-sized.
pub fn crop_for(w: u32, h: u32) -> (u32, u32, u32) {
    let ew = w & !1;
    let target = ((w as f64 * 9.0 / 20.0) as u32) & !1;
    if h > target + 2 {
        (ew, target, h - target)
    } else {
        (ew, h & !1, 0)
    }
}

/// Find the window with exactly this title (waits up to `wait`).
fn find_window(title: &str, wait: Duration, cancel: &Cancel) -> Result<Window> {
    let end = Instant::now() + wait;
    loop {
        cancel.check()?;
        if let Ok(w) = Window::from_name(title) {
            return Ok(w);
        }
        if Instant::now() > end {
            return err("The window to record did not appear.");
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// Record the window titled `title` for `p.seconds` into `p.out` (MP4). `progress(0..1, message)`.
pub fn record_window(title: &str, p: &Params, progress: Progress, cancel: &Cancel) -> Result<()> {
    let window = find_window(title, Duration::from_secs(15), cancel)?;
    let shared: Shared = Arc::new(Mutex::new(Latest::default()));
    let interval = Duration::from_secs_f64(0.8 / p.fps as f64);
    let settings = Settings::new(
        window,
        if p.cursor { CursorCaptureSettings::WithCursor } else { CursorCaptureSettings::WithoutCursor },
        DrawBorderSettings::WithoutBorder,
        SecondaryWindowSettings::Default,
        MinimumUpdateIntervalSettings::Custom(interval),
        DirtyRegionSettings::Default,
        ColorFormat::Bgra8,
        shared.clone(),
    );
    let control = Handler::start_free_threaded(settings).map_err(|e| SourceError(format!("Could not start the window capture: {e}")))?;
    let mut ff: Option<std::process::Child> = None;
    let result = (|| -> Result<()> {
        // wait for the first frame
        let first_deadline = Instant::now() + Duration::from_secs(10);
        let (w, h) = loop {
            cancel.check()?;
            {
                let l = shared.lock().unwrap_or_else(|p| p.into_inner());
                if l.seq > 0 {
                    break (l.w, l.h);
                }
            }
            if control.is_finished() || Instant::now() > first_deadline {
                return err("The window to record produced no picture.");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let (cw, ch, cy) = crop_for(w, h);
        let vf = format!("crop={cw}:{ch}:0:{cy},format=yuv420p");
        let mut child = Command::new(&p.ffmpeg)
            .creation_flags(0x0800_0000)
            .args(["-hide_banner", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "bgra", "-video_size", &format!("{w}x{h}"), "-framerate", &p.fps.to_string(), "-i", "-"])
            .args(["-vf", &vf, "-an", "-c:v", "libx264", "-preset", "veryfast", "-crf", "14", "-movflags", "+faststart", "-y"])
            .arg(&p.out)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SourceError(format!("Could not start ffmpeg: {e}")))?;
        // Ends with the app even if the app is killed (Job Object, kill on close).
        crate::core::jobobject::adopt(&child);
        let mut stdin = child.stdin.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        ff = Some(child);
        let errtext = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr.read_to_string(&mut s);
            s
        });
        let total = (p.seconds * p.fps as f32).round().max(1.0) as u64;
        let start = Instant::now();
        let mut frame: Vec<u8> = Vec::new();
        let mut have = false;
        for k in 0..total {
            let due = start + Duration::from_secs_f64(k as f64 / p.fps as f64);
            while Instant::now() < due {
                cancel.check()?;
                std::thread::sleep((due - Instant::now()).min(Duration::from_millis(20)));
            }
            cancel.check()?;
            if control.is_finished() && !have {
                return err("The window was closed while recording.");
            }
            {
                let l = shared.lock().unwrap_or_else(|p| p.into_inner());
                if l.w == w && l.h == h && l.data.len() == (w as usize * h as usize * 4) {
                    frame.clear();
                    frame.extend_from_slice(&l.data);
                    have = true;
                }
            }
            if have {
                stdin.write_all(&frame).map_err(|_| SourceError("The video encoder stopped unexpectedly.".into()))?;
            }
            progress((k + 1) as f32 / total as f32 * 0.95, "Recording");
        }
        drop(stdin);
        let status = ff.as_mut().unwrap().wait().map_err(|e| SourceError(e.to_string()))?;
        ff = None;
        if !status.success() {
            return err(format!("The video encoder failed: {}", errtext.join().unwrap_or_default().trim()));
        }
        Ok(())
    })();
    let _ = control.stop();
    if result.is_err() {
        if let Some(mut c) = ff {
            let _ = c.kill();
            let _ = c.wait();
        }
        let _ = std::fs::remove_file(&p.out);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop() {
        assert_eq!(crop_for(2400, 1080), (2400, 1080, 0));
        assert_eq!(crop_for(2400, 1111), (2400, 1080, 31));
        assert_eq!(crop_for(2401, 1090), (2400, 1080, 10));
    }
}
