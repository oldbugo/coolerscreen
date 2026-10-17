//! CoolerScreen 2.0 core: scheduler, library/settings (1.2 disk layout), device service, media server and the
//! interfaces the media (WS2) and sources (WS3) workstreams implement. See docs/v2/api-contract.md.
pub mod api;
pub mod app;
pub mod autostart;
pub mod commands;
pub mod device_service;
pub mod error;
pub mod fit;
pub mod fake_media;
pub mod gallery;
pub mod jobobject;
pub mod js;
pub mod live_host;
pub mod library;
pub mod mask;
pub mod media_server;
pub mod paths;
pub mod recycle;
pub mod scheduler;
pub mod service;
pub mod settings;
pub mod sources_adapter;
pub mod theme;
pub mod themes;
#[cfg(test)]
mod tests_app;

pub use app::{App, Events};
pub use error::{CoreError, CoreResult};
