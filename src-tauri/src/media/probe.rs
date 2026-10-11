//! Source facts from `ffmpeg -i` (port of convert.js probe()).
use super::{ffmpeg, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

pub const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "bmp", "webp", "tif", "tiff"];

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub duration: Option<f64>,
    /// Displayed size (a stored rotation of +-90 deg is applied, as ffmpeg and the browser do).
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub codec: Option<String>,
    pub has_video: bool,
    pub is_image: bool,
    /// The stream carries a +-90 deg rotation (GPU paths are skipped for these: autorotate runs on the CPU).
    #[serde(default)]
    pub rotated: bool,
}

impl SourceInfo {
    pub fn pixels(&self) -> u64 {
        self.width.unwrap_or(0) as u64 * self.height.unwrap_or(0) as u64
    }
}

pub fn is_image_path(file: &Path) -> bool {
    file.extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXT.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

struct Res {
    dur: Regex,
    size: Regex,
    codec: Regex,
    fps: Regex,
    rot: Regex,
}
fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| Res {
        dur: Regex::new(r"Duration:\s*(\d+):(\d+):(\d+(?:\.\d+)?)").unwrap(),
        size: Regex::new(r"Video:.*?(\d{2,5})x(\d{2,5})").unwrap(),
        codec: Regex::new(r"(?i)Video:\s*([a-z0-9_]+)").unwrap(),
        fps: Regex::new(r"Video:.*?(\d+(?:\.\d+)?) fps").unwrap(),
        rot: Regex::new(r"(?i)rotat(?:e|ion)[^\d-]*(-?\d+(?:\.\d+)?)").unwrap(),
    })
}

/// Parse `ffmpeg -i` stderr text.
pub fn parse(text: &str, file: &Path) -> SourceInfo {
    let r = res();
    let duration = r.dur.captures(text).map(|c| {
        c[1].parse::<f64>().unwrap() * 3600.0 + c[2].parse::<f64>().unwrap() * 60.0 + c[3].parse::<f64>().unwrap()
    });
    let (mut width, mut height) = match r.size.captures(text) {
        Some(c) => (c[1].parse().ok(), c[2].parse().ok()),
        None => (None, None),
    };
    let rotated = r
        .rot
        .captures(text)
        .and_then(|c| c[1].parse::<f64>().ok())
        .map(|v| (v.round().abs() as i64) % 180 == 90)
        .unwrap_or(false);
    if rotated {
        std::mem::swap(&mut width, &mut height);
    }
    SourceInfo {
        duration,
        width,
        height,
        fps: r.fps.captures(text).and_then(|c| c[1].parse().ok()),
        codec: r.codec.captures(text).map(|c| c[1].to_ascii_lowercase()),
        has_video: text.contains("Video:"),
        is_image: is_image_path(file),
        rotated,
    }
}

pub fn probe(ffmpeg_path: &Path, file: &Path) -> Result<SourceInfo> {
    Ok(parse(&ffmpeg::probe_text(ffmpeg_path, file)?, file))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_ffmpeg_banner() {
        let t = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':\n  Duration: 00:00:24.80, start: 0.000000, bitrate: 7117 kb/s\n  Stream #0:0[0x1](und): Video: hevc (Main) (hev1 / 0x31766568), yuv420p(tv), 1920x1080, 7000 kb/s, 25 fps, 25 tbr, 12800 tbn (default)\n";
        let i = parse(t, Path::new("a.mp4"));
        assert_eq!(i.duration, Some(24.8));
        assert_eq!((i.width, i.height), (Some(1920), Some(1080)));
        assert_eq!(i.fps, Some(25.0));
        assert_eq!(i.codec.as_deref(), Some("hevc"));
        assert!(i.has_video && !i.is_image && !i.rotated);
    }
    #[test]
    fn rotation_swaps() {
        let t = "Duration: 00:00:01.00\n Stream: Video: h264, yuv420p, 1920x1080, 30 fps\n    side data:\n      displaymatrix: rotation of -90.00 degrees\n";
        let i = parse(t, Path::new("p.MOV"));
        assert_eq!((i.width, i.height), (Some(1080), Some(1920)));
        assert!(i.rotated);
    }
}
