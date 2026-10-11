//! Shared core for the Tauri app and the cs2 CLI.
pub mod core;
pub mod device;
pub mod media;
pub mod sources;

use crate::core::api::Backends;
use crate::core::paths;
use std::sync::Arc;

/// The media engine and sources the app runs with (the seam where WS2/WS3 plug into the core).
/// `COOLERSCREEN_FAKE_MEDIA=1` uses core's fake engine (no ffmpeg; forces the fake screen).
pub fn backends() -> Backends {
    backends_with_work_dir(std::env::temp_dir().join("CoolerScreen-media"))
}

/// As `backends`, with the media engine's scratch folder (the engine clears it on start, so tools that may run
/// next to the app, like cs2, pass their own).
pub fn backends_with_work_dir(work_dir: std::path::PathBuf) -> Backends {
    let mut b = Backends::default();
    b.sources = Arc::new(crate::core::sources_adapter::Catalog::new(crate::core::sources_adapter::Env {
        data_dir: paths::base(),
        sources_dir: paths::sources_dir(),
        ffmpeg: paths::ffmpeg(),
    }));
    b.media = if std::env::var("COOLERSCREEN_FAKE_MEDIA").is_ok_and(|v| !v.is_empty() && v != "0") {
        Arc::new(crate::core::fake_media::FakeMedia::default())
    } else {
        Arc::new(crate::media::core_engine::CoreMedia::new(crate::media::MediaConfig {
            ffmpeg: paths::ffmpeg(),
            base_dir: paths::base(),
            work_dir,
        }))
    };
    b
}
