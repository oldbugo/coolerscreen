//! Checks of a converted device file: what the screen requires (MJPEG mp4, 1080x2400, 18 fps, notch band black) and
//! similarity to another file (PSNR/SSIM), used by the golden tests and available to core before an upload.
use super::ffmpeg::{self, p};
use super::graph::{band_px, Mask, MaskMode, DEVICE_FPS};
use super::{probe, CancelToken, Result};
use regex::Regex;
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceCheck {
    pub mjpeg_mp4: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub codec: Option<String>,
    pub frames: usize,
    /// Max luma inside the notch band over all frames (0 = exactly black); None when there is no band.
    pub band_max: Option<u8>,
}
impl DeviceCheck {
    pub fn ok(&self) -> bool {
        self.mjpeg_mp4
            && self.width == Some(1080)
            && self.height == Some(2400)
            && self.fps == Some(DEVICE_FPS as f64)
            && self.codec.as_deref() == Some("mjpeg")
            && self.band_max.unwrap_or(0) == 0
    }
}

/// Inspect a device file. The landscape band maps to rows (left/right) or columns (top/bottom) of the stored portrait
/// frame (transpose=1 = 90 deg clockwise: landscape x -> portrait row x, landscape y -> portrait column 1079 - y).
pub fn check_device_file(ff: &Path, file: &Path, mask: &Mask) -> Result<DeviceCheck> {
    let data = std::fs::read(file)?;
    let info = probe::probe(ff, file)?;
    let gray = ffmpeg::run_capture(ff, &crate::ff_args!["-i", p(file), "-an", "-vf", "format=gray", "-f", "rawvideo", "pipe:1"], &CancelToken::new(), None)?;
    let (w, h) = (1080usize, 2400usize);
    let frames = gray.len() / (w * h);
    let mut band_max = None;
    if let (MaskMode::Band, Some(b)) = (mask.mode, mask.band) {
        let px = band_px(&b) as usize;
        if px > 0 {
            let mut m = 0u8;
            for f in 0..frames {
                let fr = &gray[f * w * h..(f + 1) * w * h];
                for row in 0..h {
                    for col in 0..w {
                        let inside = match b.edge {
                            super::graph::Edge::Left => row < px,
                            super::graph::Edge::Right => row >= h - px,
                            super::graph::Edge::Top => col >= w - px,
                            super::graph::Edge::Bottom => col < px,
                        };
                        if inside {
                            m = m.max(fr[row * w + col]);
                        }
                    }
                }
            }
            band_max = Some(m);
        }
    }
    Ok(DeviceCheck {
        mjpeg_mp4: crate::device::mjpeg::is_mjpeg_mp4(&data),
        width: info.width,
        height: info.height,
        fps: info.fps,
        codec: info.codec,
        frames,
        band_max,
    })
}

#[derive(Clone, Debug, Serialize)]
pub struct Similarity {
    pub psnr: f64,
    pub ssim: f64,
    pub identical_bytes: bool,
}

/// PSNR (dB, average; inf when identical) and SSIM (All) of `a` against `b`.
pub fn compare(ff: &Path, a: &Path, b: &Path) -> Result<Similarity> {
    let identical_bytes = std::fs::read(a)? == std::fs::read(b)?;
    let out = std::process::Command::new(ff)
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(a)
        .arg("-i")
        .arg(b)
        .args(["-lavfi", "[0:v]split[a0][a1];[1:v]split[b0][b1];[a0][b0]psnr;[a1][b1]ssim", "-f", "null", "-"])
        .output()?;
    let err = String::from_utf8_lossy(&out.stderr);
    let psnr = Regex::new(r"PSNR .*? average:(inf|[\d.]+)").unwrap().captures(&err).map(|c| c[1].parse().unwrap_or(f64::INFINITY));
    let ssim = Regex::new(r"SSIM .*? All:([\d.]+)").unwrap().captures(&err).and_then(|c| c[1].parse().ok());
    Ok(Similarity { psnr: psnr.unwrap_or(f64::NAN), ssim: ssim.unwrap_or(f64::NAN), identical_bytes })
}
