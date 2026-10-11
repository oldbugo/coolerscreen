//! CoolerScreen 2.0 core: scheduler, library/settings (1.2 disk layout), device service, media server and the
//! interfaces the media (WS2) and sources (WS3) workstreams implement. See docs/v2/api-contract.md.
pub mod api;
pub mod error;
pub mod js;
pub mod library;
pub mod mask;
pub mod paths;
pub mod scheduler;
pub mod settings;

pub use error::{CoreError, CoreResult};
