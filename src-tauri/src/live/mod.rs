//! 3.0 live mode: draw the screen contents on the PC and stream them as JPEG frames (the "companion").
//! Building blocks shared by the small separate `cs-companion` program (prototype A) and, later, the Tauri app (prototype B):
//!   session   the WinUSB link: connect, frames, keepalive, stop (loop clip keeps playing afterwards)
//!   encode    rotate to portrait 756x1680 and encode 4:2:0 JPEG within the size budget
//!   render    tiny-skia compositor: background, clock, sensor panel (landscape 1680x756)
//!   sensors   CPU load, RAM, NVIDIA GPU (NVML loaded at run time, no admin)
//!   video     animated background: decode a clip through the bundled ffmpeg as raw frames
//!   theme     3.0 theme model (theme.json): widgets, hand-over, ThemeLive
//!   preview   one-frame renders without a device (editor preview, onBase overlay)
//!   pipeline  render -> encode -> send threads with latest-frame slots, pacing and reconnect
pub mod effects;
pub mod encode;
pub mod extsensors;
pub mod agenda;
pub mod audio;
pub mod fitcal;
pub mod gamefps;
pub mod gif;
pub mod metrics;
pub mod morewidgets;
pub mod mask;
pub mod nowplaying;
pub mod ping;
pub mod pipeline;
pub mod progress;
pub mod sunmoon;
pub mod todo;
pub mod headlines;
pub mod webpicture;
pub mod preview;
pub mod render;
pub mod sensors;
pub mod session;
pub mod theme;
pub mod twin;
pub mod video;
pub mod volume;
pub mod weather;

pub use preview::{render_base_overlay, render_preview};
pub use theme::{ClockProps, DateProps, InfoProps, SensorProps, SlideshowProps, Threshold, VisualizerProps, WeatherProps, Group, Guides, GuidesOffset, Handover, HandoverEnd, ImageProps, MusicIdle, MusicProps, StatsProps, TextProps, Theme, ThemeLive, Widget};
use serde::{Deserialize, Serialize};

/// Landscape canvas (the 2400x1080 design space at 0.7) and the portrait frame the screen takes.
pub const LW: usize = 1680;
pub const LH: usize = 756;
pub const PW: usize = 756;
pub const PH: usize = 1680;

/// Now-playing card settings.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicConfig {
    pub enabled: bool,
    /// Prefer the session of this app (part of its AppUserModelId, e.g. "Spotify"); null = whatever is playing.
    pub preferred_app: Option<String>,
}

impl Default for MusicConfig {
    fn default() -> Self {
        MusicConfig { enabled: true, preferred_app: None }
    }
}

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
    /// Now playing (Windows media sessions).
    pub music: MusicConfig,
    /// Notch mask: "app" follows the app's setting (settings.json), "off" draws edge to edge.
    pub notch: String,
    /// Background clip decoding: "auto" tries hardware (D3D11) and falls back to software, "software" never uses the GPU.
    pub video_decode: String,
    /// Draw the measured frames per second on the screen picture (a debugging aid; off by default).
    pub fps_overlay: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config { fps: 60, background: None, show_seconds: false, max_kb: 200, music: MusicConfig::default(), notch: "app".into(), video_decode: "auto".into(), fps_overlay: false }
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
