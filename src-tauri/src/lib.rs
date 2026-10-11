//! Shared core for the Tauri app and the cs2 CLI.
pub mod core;
pub mod device;

use crate::core::api::Backends;
use std::sync::Arc;

/// The media engine and sources the app runs with. WS2 (`media/`) and WS3 (`sources/`) replace the stand-ins here
/// when they land. `COOLERSCREEN_FAKE_MEDIA=1` uses core's fake engine (no ffmpeg; forces the fake screen).
pub fn backends() -> Backends {
    let mut b = Backends::default();
    if std::env::var("COOLERSCREEN_FAKE_MEDIA").is_ok_and(|v| !v.is_empty() && v != "0") {
        b.media = Arc::new(crate::core::fake_media::FakeMedia::default());
    }
    b
}
