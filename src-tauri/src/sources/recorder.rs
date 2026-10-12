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

/// Ask, while recording, whether the frames so far are enough (e.g. they hold a seamless loop): small grey copies of
/// every frame (64x36 and 192x108, the loop search's sizes), checked every `every_s` seconds once `min_s` are in.
pub struct EarlyStop {
    pub min_s: f32,
    pub every_s: f32,
    pub check: Arc<dyn Fn(&crate::media::loops::LoopFrames) -> bool + Send + Sync>,
}

pub struct Params {
    /// The longest recording; with `early_stop` it may end sooner.
    pub seconds: f32,
    pub early_stop: Option<EarlyStop>,
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

/// BGRA frame (cropped to `crop` = (w, h, y)) to 192x108 grey (area average, BT.601 luma), then 64x36 from that.
fn small_grey(bgra: &[u8], stride_w: u32, crop: (u32, u32, u32), hi: &mut Vec<u8>, lo: &mut Vec<u8>) {
    use crate::media::loops::{FH, FW, RH, RW};
    let (cw, ch, cy) = (crop.0 as usize, crop.1 as usize, crop.2 as usize);
    let sw = stride_w as usize;
    let mut h = vec![0u8; RW * RH];
    for oy in 0..RH {
        let (y0, y1) = (cy + oy * ch / RH, cy + ((oy + 1) * ch / RH).max(oy * ch / RH + 1));
        for ox in 0..RW {
            let (x0, x1) = (ox * cw / RW, ((ox + 1) * cw / RW).max(ox * cw / RW + 1));
            let (mut s, mut k) = (0u32, 0u32);
            // every second pixel of the block is plenty for a 192x108 thumbnail
            for y in (y0..y1).step_by(2) {
                let row = &bgra[(y * sw) * 4..];
                for x in (x0..x1).step_by(2) {
                    let p = &row[x * 4..x * 4 + 3];
                    s += (29 * p[0] as u32 + 150 * p[1] as u32 + 77 * p[2] as u32) >> 8;
                    k += 1;
                }
            }
            h[oy * RW + ox] = (s / k.max(1)) as u8;
        }
    }
    for y in 0..FH {
        for x in 0..FW {
            let mut s = 0u32;
            for dy in 0..3 {
                for dx in 0..3 {
                    s += h[(y * 3 + dy) * RW + x * 3 + dx] as u32;
                }
            }
            lo.push(((s + 4) / 9) as u8);
        }
    }
    hi.extend_from_slice(&h);
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
        // early stop: grey copies + a check running on its own thread (never holding up the frame pacing)
        let (mut glo, mut ghi) = (Vec::new(), Vec::new());
        let enough = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut checking: Option<std::thread::JoinHandle<()>> = None;
        let mut next_check = p.early_stop.as_ref().map(|e| (e.min_s * p.fps as f32).round() as u64);
        let check_fn = p.early_stop.as_ref().map(|e| e.check.clone());

        for k in 0..total {
            if enough.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
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
                if let (Some(e), Some(check)) = (&p.early_stop, &check_fn) {
                    small_grey(&frame, w, (cw, ch, cy), &mut ghi, &mut glo);
                    let n = (k + 1) as u64;
                    if next_check.map_or(false, |c| n >= c) && checking.as_ref().map_or(true, |t| t.is_finished()) {
                        next_check = Some(n + (e.every_s * p.fps as f32).round().max(1.0) as u64);
                        let frames = crate::media::loops::LoopFrames { n: glo.len() / (crate::media::loops::FW * crate::media::loops::FH), lo: glo.clone(), hi: ghi.clone(), hw: false };
                        let (check, enough) = (check.clone(), enough.clone());
                        checking = Some(std::thread::spawn(move || {
                            if check(&frames) {
                                enough.store(true, std::sync::atomic::Ordering::SeqCst);
                            }
                        }));
                    }
                }
            }
            let msg = if p.early_stop.is_some() { "Recording until a smooth loop turns up" } else { "Recording" };
            progress((k + 1) as f32 / total as f32 * 0.95, msg);
        }
        if let Some(t) = checking.take() {
            let _ = t.join();
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
