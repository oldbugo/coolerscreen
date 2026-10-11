//! Media pipeline (CoolerScreen 2.0, workstream 2): everything that turns a source file into what the screen plays,
//! with FFmpeg as a sidecar process. No screen/USB access here.
//!
//! Behavioural spec: the 1.2 Node implementation (src/convert.js, src/loopsearch.js, src/splash.js). Performance design:
//! docs/perf/2026-10-11-performance-investigation.md (decode once, raw intermediate, one-pass loop frames, NVDEC).
//!
//! Every long operation takes a [`CancelToken`] (cancelling kills the running ffmpeg children) and a progress callback
//! `(fraction 0..1, message)`. Nothing here blocks on a global lock while ffmpeg runs, so several operations can run in
//! parallel; the scheduler (core) decides what runs.

pub mod assets;
pub mod encode;
pub mod ffmpeg;
pub mod gpu;
pub mod graph;
pub mod loops;
pub mod probe;
pub mod verify;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub use encode::{ConvertResult, Engine, EstimateResult, VersionRequest};
pub use graph::{Corners, Frame, Mask, DEVICE_FPS, H, Q_MAX, Q_MIN, W};
pub use loops::{LoopCandidate, LoopSearch};
pub use probe::SourceInfo;

/// Cooperative cancellation shared between a caller and an operation. Cancelling kills every ffmpeg child the
/// operation is running and makes it return [`MediaError::Cancelled`] promptly.
#[derive(Clone, Default, Debug)]
pub struct CancelToken(Arc<AtomicBool>);
impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            Err(MediaError::Cancelled)
        } else {
            Ok(())
        }
    }
}

/// Progress callback: (fraction 0..1, short user-facing message). Called from worker threads.
pub type Progress<'a> = &'a (dyn Fn(f64, &str) + Sync);

/// A progress callback that ignores everything.
pub fn no_progress(_: f64, _: &str) {}

#[derive(Debug)]
pub enum MediaError {
    Cancelled,
    /// ffmpeg exited with an error (last lines of its stderr).
    Ffmpeg(String),
    /// Nothing fits the space even at the lowest quality; `max_fit_length` in seconds when known.
    TooLong { max_fit_length: Option<f64> },
    Invalid(String),
    Io(std::io::Error),
}
impl std::fmt::Display for MediaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MediaError::Cancelled => write!(f, "Cancelled."),
            MediaError::Ffmpeg(s) => write!(f, "ffmpeg failed: {s}"),
            MediaError::TooLong { max_fit_length: Some(n) } => {
                write!(f, "This part is too long to fit even at the lowest quality. Try about {n} s or less.")
            }
            MediaError::TooLong { max_fit_length: None } => {
                write!(f, "This part is too long to fit even at the lowest quality. Try a shorter length.")
            }
            MediaError::Invalid(s) => write!(f, "{s}"),
            MediaError::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for MediaError {}
impl From<std::io::Error> for MediaError {
    fn from(e: std::io::Error) -> Self {
        MediaError::Io(e)
    }
}
impl MediaError {
    /// Stable code for the UI / API (`tooLong` matches 1.2's error code).
    pub fn code(&self) -> &'static str {
        match self {
            MediaError::Cancelled => "cancelled",
            MediaError::Ffmpeg(_) => "ffmpeg",
            MediaError::TooLong { .. } => "tooLong",
            MediaError::Invalid(_) => "invalid",
            MediaError::Io(_) => "io",
        }
    }
}
pub type Result<T> = std::result::Result<T, MediaError>;

/// Where the media pipeline finds ffmpeg and keeps its caches.
#[derive(Clone, Debug)]
pub struct MediaConfig {
    /// ffmpeg executable (the bundled ffmpeg-static 6.1.1 build: NVDEC/NVENC/scale_cuda).
    pub ffmpeg: PathBuf,
    /// App data folder (1.2: %APPDATA%\CoolerScreen); corner masks and the splash are cached here.
    pub base_dir: PathBuf,
    /// Scratch folder for raw intermediates and trial encodes (cleared on Engine start).
    pub work_dir: PathBuf,
}

impl MediaConfig {
    /// Sensible defaults: `CS_FFMPEG` env, else `ffmpeg.exe` next to the executable, else the repo's ffmpeg-static;
    /// base dir `COOLERSCREEN_HOME` or %APPDATA%\CoolerScreen; work dir %TEMP%\CoolerScreen-media.
    pub fn from_env() -> Self {
        let ffmpeg = ffmpeg::locate();
        let base_dir = std::env::var_os("COOLERSCREEN_HOME").map(PathBuf::from).unwrap_or_else(|| {
            std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("CoolerScreen")
        });
        let work_dir = std::env::temp_dir().join("CoolerScreen-media");
        MediaConfig { ffmpeg, base_dir, work_dir }
    }
}

/// JS `Math.round` (half rounds up), used wherever 1.2 rounds so results match it exactly.
pub(crate) fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}
/// 1.2's `r3`: round to the millisecond.
pub(crate) fn r3(v: f64) -> f64 {
    js_round(v * 1000.0) / 1000.0
}
