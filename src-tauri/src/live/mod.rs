//! 3.0 live mode: draw the screen contents on the PC and stream them as JPEG frames (the "companion").
//! Building blocks shared by the small separate `cs-companion` program (prototype A) and, later, the Tauri app (prototype B):
//!   session   the WinUSB link: connect, frames, keepalive, stop (loop clip keeps playing afterwards)
//!   encode    rotate to portrait 756x1680 and encode 4:2:0 JPEG within the size budget
//!   render    tiny-skia compositor: background, clock, sensor panel (landscape 1680x756)
//!   sensors   CPU load, RAM, NVIDIA GPU (NVML loaded at run time, no admin)
//!   video     animated background: decode a clip through the bundled ffmpeg as raw frames
//!   pipeline  render -> encode -> send threads with latest-frame slots, pacing and reconnect
pub mod encode;
pub mod pipeline;
pub mod render;
pub mod sensors;
pub mod session;
pub mod video;

use serde::{Deserialize, Serialize};

/// Landscape canvas (the 2400x1080 design space at 0.7) and the portrait frame the screen takes.
pub const LW: usize = 1680;
pub const LH: usize = 756;
pub const PW: usize = 756;
pub const PH: usize = 1680;

/// companion.json (in the CoolerScreen app folder). Defaults follow the standing list: 60 fps, nothing extra on.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Live frame rate, 30 or 60.
    pub fps: u32,
    /// Clip used as animated background (decoded with the bundled ffmpeg); none = animated gradient.
    pub background: Option<String>,
    pub show_seconds: bool,
    /// Largest frame in KB (the vendor profile for this screen uses 200).
    pub max_kb: u32,
}

impl Default for Config {
    fn default() -> Self {
        Config { fps: 60, background: None, show_seconds: false, max_kb: 200 }
    }
}

impl Config {
    pub fn path() -> std::path::PathBuf {
        crate::core::paths::base().join("companion.json")
    }
    /// Load; a missing or broken file gives the defaults (and a missing one is written, so the user can edit it).
    pub fn load() -> Config {
        let p = Config::path();
        match std::fs::read_to_string(&p) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => {
                let c = Config::default();
                c.save();
                c
            }
        }
    }
    pub fn save(&self) {
        let p = Config::path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, serde_json::to_string_pretty(self).unwrap_or_default());
    }
}
