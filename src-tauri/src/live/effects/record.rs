//! Recording an effect as a seamless loop (the Embedded face of an effect theme). The effect is periodic in its loop phase, so
//! frames at phase i/N for i in 0..N make a clip whose last frame leads straight into the first. The clip is only an intermediate
//! source (a lossless-ish MJPEG file at the 2400x1080 design size): the normal conversion turns it into the stored base with the
//! screen fit, corners, notch, text/picture items and the "best that fits" quality, exactly like any other clip.
use super::{EffectBg, Params};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// The screen plays its loop at this rate (library DEVICE_FPS); the recording is made at it so nothing is resampled.
pub const RECORD_FPS: u32 = 18;
pub const RECORD_W: usize = 2400;
pub const RECORD_H: usize = 1080;

/// Frames of one loop played `speed_up` times faster than the live effect (1 = same speed; more only when the loop would not
/// fit on the screen). At least 2.
pub fn loop_frames(p: &Params, speed_up: f32) -> usize {
    ((p.loop_seconds() / speed_up.max(1.0)) * RECORD_FPS as f32).round().max(2.0) as usize
}

/// Seconds the stored loop lasts.
pub fn loop_seconds(frames: usize) -> f64 {
    frames as f64 / RECORD_FPS as f64
}

/// Render `frames` frames of one loop into `out` (a .mkv, MJPEG 4:2:0 at high quality). `progress(done_fraction)` returns false
/// to stop (the partial file is removed). Written to a temporary name and renamed when complete.
pub fn render_loop(p: &Params, frames: usize, out: &Path, progress: &mut dyn FnMut(f32) -> bool) -> Result<(), String> {
    let ffmpeg = crate::media::ffmpeg::locate();
    if !ffmpeg.exists() {
        return Err(format!("ffmpeg not found at {}", ffmpeg.display()));
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension("tmp.mkv");
    let mut c = Command::new(&ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &format!("{RECORD_W}x{RECORD_H}"), "-r", &RECORD_FPS.to_string(), "-i", "-"])
        .args(["-an", "-c:v", "mjpeg", "-q:v", "2", "-pix_fmt", "yuvj420p", "-f", "matroska"])
        .arg(&tmp)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let mut child = c.spawn().map_err(|e| format!("could not start ffmpeg: {e}"))?;
    crate::media::sys::adopt_child(&child);
    let mut stdin = child.stdin.take().ok_or("no ffmpeg input")?;
    let mut fx = EffectBg::new(p.clone());
    let mut buf = vec![0u8; RECORD_W * RECORD_H * 4];
    let mut stopped = false;
    let mut failed = None;
    for i in 0..frames {
        if !progress(i as f32 / frames as f32) {
            stopped = true;
            break;
        }
        fx.draw_phase(i as f64 / frames as f64, &mut buf, RECORD_W, RECORD_H, (0, 0, RECORD_W as i64, RECORD_H as i64));
        if let Err(e) = stdin.write_all(&buf) {
            failed = Some(format!("ffmpeg stopped while the effect was recorded: {e}"));
            break;
        }
    }
    drop(stdin);
    if stopped || failed.is_some() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_file(&tmp);
        return Err(failed.unwrap_or_else(|| "stopped".into()));
    }
    let st = child.wait().map_err(|e| e.to_string())?;
    if !st.success() {
        let _ = std::fs::remove_file(&tmp);
        return Err("ffmpeg could not write the effect clip".into());
    }
    let _ = std::fs::remove_file(out);
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::effects::Kind;

    #[test]
    fn frame_counts_follow_the_loop_length() {
        let mut p = Kind::Aurora.defaults();
        assert_eq!(loop_frames(&p, 1.0), 180);
        p.speed = 2.0;
        assert_eq!(loop_frames(&p, 1.0), 90);
        assert_eq!(loop_frames(&p, 1.5), 60);
        p.speed = 3.0;
        assert!(loop_frames(&p, 100.0) >= 2);
        assert_eq!(loop_seconds(180), 10.0);
    }

    /// The recorded clip has the right frame count and its last frame is one step before its first (a seamless loop).
    #[test]
    fn a_recorded_loop_is_a_seamless_clip() {
        if !crate::media::ffmpeg::locate().exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("loop.mkv");
        let p = Kind::Drift.defaults();
        let n = 12;
        render_loop(&p, n, &out, &mut |_| true).unwrap();
        assert!(out.is_file() && std::fs::metadata(&out).unwrap().len() > 10_000);
        assert!(!out.with_extension("tmp.mkv").exists());
        // stopping removes the partial file
        let out2 = dir.path().join("stopped.mkv");
        assert!(render_loop(&p, n, &out2, &mut |f| f < 0.3).is_err());
        assert!(!out2.exists() && !out2.with_extension("tmp.mkv").exists());
    }

    /// Real conversion of a recorded loop (needs ffmpeg; `--ignored`): the stored clip has exactly the loop's frames, fits the
    /// budget, and the step from its last frame to its first is no bigger than the steps inside it (a seamless loop).
    #[test]
    #[ignore]
    fn the_converted_loop_joins_up() {
        use crate::media::{encode::Engine, encode::VersionRequest, CancelToken, MediaConfig};
        let ffmpeg = crate::media::ffmpeg::locate();
        if !ffmpeg.exists() {
            return;
        }
        for kind in Kind::ALL {
            let dir = tempfile::tempdir().unwrap();
            let mut p = kind.defaults();
            p.speed = 2.0;
            let frames = loop_frames(&p, 1.0);
            let clip = dir.path().join("loop.mkv");
            render_loop(&p, frames, &clip, &mut |_| true).unwrap();
            let eng = Engine::new(MediaConfig { ffmpeg: ffmpeg.clone(), base_dir: dir.path().join("base"), work_dir: dir.path().join("work") });
            let req = VersionRequest { start: 0.0, length: Some(loop_seconds(frames)), quality: None, detail: 0.0, frame: Default::default(), mask: Default::default(), corners: Default::default(), blend: 0.0, budget_bytes: 8_000_000, max_seconds: Some(20.0), overlay: None };
            let out = dir.path().join("out");
            let r = eng.convert(&clip, None, &req, &out, &CancelToken::new(), &|_, _| {}).unwrap();
            // decode every frame of the stored clip, small and grey
            let raw = Command::new(&ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-i"])
                .arg(out.join("device.mp4"))
                .args(["-vf", "scale=94:210", "-pix_fmt", "gray", "-f", "rawvideo", "-"])
                .output()
                .unwrap()
                .stdout;
            let fs = 94 * 210;
            let n = raw.len() / fs;
            assert_eq!(n, frames, "{}: stored frame count", kind.name());
            let diff = |a: usize, b: usize| raw[a * fs..(a + 1) * fs].iter().zip(&raw[b * fs..(b + 1) * fs]).map(|(x, y)| (*x as i32 - *y as i32).abs() as f64).sum::<f64>() / fs as f64;
            let steps: Vec<f64> = (0..n - 1).map(|i| diff(i, i + 1)).collect();
            let worst = steps.iter().cloned().fold(0.0, f64::max);
            let seam = diff(n - 1, 0);
            println!("{}: {} frames, {} KB (budget 8000 KB), quality {}, biggest step {:.2}, seam {:.2}", kind.name(), n, r.size / 1000, r.quality, worst, seam);
            assert!(r.size <= 8_000_000);
            assert!(seam <= worst * 1.5 + 0.3, "{}: the loop does not join up (seam {seam:.2} vs steps up to {worst:.2})", kind.name());
        }
    }

    /// Where the time of storing an effect goes before the upload (`--ignored --nocapture`): recording and converting a default
    /// (speed 1, 180 frame) loop with a 10.6 MB budget.
    #[test]
    #[ignore]
    fn time_the_steps_before_the_upload() {
        use crate::media::{encode::Engine, encode::VersionRequest, CancelToken, MediaConfig};
        let ffmpeg = crate::media::ffmpeg::locate();
        if !ffmpeg.exists() {
            return;
        }
        for kind in [Kind::Aurora, Kind::Plasma, Kind::Drift, Kind::Retro] {
            let dir = tempfile::tempdir().unwrap();
            let p = kind.defaults();
            let frames = loop_frames(&p, 1.0);
            let clip = dir.path().join("loop.mkv");
            let t = std::time::Instant::now();
            render_loop(&p, frames, &clip, &mut |_| true).unwrap();
            let rec = t.elapsed();
            let eng = Engine::new(MediaConfig { ffmpeg: ffmpeg.clone(), base_dir: dir.path().join("base"), work_dir: dir.path().join("work") });
            let req = VersionRequest { start: 0.0, length: Some(loop_seconds(frames)), quality: None, detail: 0.0, frame: Default::default(), mask: Default::default(), corners: Default::default(), blend: 0.0, budget_bytes: 10_600_000, max_seconds: Some(20.0), overlay: None };
            let t = std::time::Instant::now();
            let r = eng.convert(&clip, None, &req, &dir.path().join("out"), &CancelToken::new(), &|_, _| {}).unwrap();
            println!("{}: record {:.1} s, convert + quality search {:.1} s -> {} KB at quality {} (upload at 50 KB/s: {:.0} s)", kind.name(), rec.as_secs_f64(), t.elapsed().as_secs_f64(), r.size / 1000, r.quality, r.size as f64 / 50_000.0);
        }
    }
}
