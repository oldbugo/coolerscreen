//! Interfaces implemented by the other workstreams (docs/v2/api-contract.md, section 5).
//! WS2 implements `MediaEngine` in `src/media/`; WS3 implements `SourceProvider` + `SourceCatalog` in `src/sources/`.
//! Core never runs ffmpeg or talks to a source directly; it schedules these calls as tasks and keeps the library.
use super::error::{CoreError, CoreResult};
use super::scheduler::{Res, Resources, TaskCtx};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The screen plays a fixed 18 fps (hardware, 2026-10-11).
pub const DEVICE_FPS: u32 = 18;
/// A still image becomes a clip of this length.
pub const IMAGE_SECONDS: u32 = 3;

// ---------- media (WS2) ----------

/// `convert.probe` result.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProbeInfo {
    pub duration: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub codec: Option<String>,
    pub has_video: bool,
    pub is_image: bool,
    /// The source carries a 90°/270° rotation (width/height are the displayed size).
    #[serde(default)]
    pub rotated: bool,
}

/// `makeSourceAssets` result: thumb.jpg and frame.jpg are always written to the dir; `proxy` names the proxy file
/// ("proxy.mp4" / "proxy.jpg") or None when the WebView can play the source itself.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceAssets {
    pub proxy: Option<String>,
}

/// Everything an estimate or conversion needs (1.2 `convert(file, dir, opts)`). `mask` is the notch only (corners
/// are per version since 2026-10-11).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodeReq {
    /// The media to convert from (gallery file, or the extracted video of a .ht/.zip).
    pub file: PathBuf,
    /// The original's cache folder (library/<oid>); WS2 may keep caches here (raw frames, loop features).
    pub dir: PathBuf,
    pub start: f64,
    pub length: f64,
    /// "auto" or 1..100.
    pub quality: Value,
    pub detail: u32,
    /// `{ zoom, x, y }`
    pub frame: Value,
    /// Notch mask: `{ mode, circle, band }`.
    pub mask: Value,
    /// `{ on, r }`
    pub corners: Value,
    pub blend: f64,
    pub budget_bytes: u64,
    pub max_seconds: f64,
    pub info: ProbeInfo,
    /// 3.0 themes: PNG (2400x1080 RGBA) of the text/picture items baked into the base; None for a plain base.
    #[serde(default)]
    pub overlay: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopSearchReq {
    pub file: PathBuf,
    pub dir: PathBuf,
    pub original_id: String,
    pub duration: f64,
    pub fps: Option<f64>,
    pub min_len: f64,
    pub max_len: f64,
    /// Space for the loop on the screen (the search may use it to prefer loops that fit).
    pub budget_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopCardsReq {
    pub file: PathBuf,
    pub dir: PathBuf,
    /// The search result's `candidates` (start, length, blend, ...).
    pub candidates: Value,
    pub duration: f64,
    pub fps: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopPointReq {
    pub file: PathBuf,
    pub start: f64,
    pub end: f64,
    pub duration: Option<f64>,
    pub fps: Option<f64>,
}

/// Which media operation a resource plan is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaTask {
    Probe,
    Assets,
    LoopSearch,
    LoopPoint,
    BytesPerSecond,
    Estimate,
    Convert,
    Splash,
    /// Loop card previews (loop-<i>.mp4), a separate job so the size meter can run alongside.
    Cards,
}

pub trait MediaEngine: Send + Sync + 'static {
    /// True only for core's test engine (its files must never reach the real screen).
    fn is_fake(&self) -> bool {
        false
    }
    /// ffprobe-like facts about a file (never fails on unreadable input: `has_video: false`).
    fn probe(&self, ctx: &TaskCtx, file: &Path) -> CoreResult<ProbeInfo>;
    /// thumb.jpg (240x108 cover), frame.jpg (1200x540 cover) and, if needed, a proxy, written into `dir`.
    fn source_assets(&self, ctx: &TaskCtx, file: &Path, dir: &Path, info: &ProbeInfo) -> CoreResult<SourceAssets>;
    /// Whole-clip loop search: `{ frames, searched, candidates: [{ start, startFrame, length, frames, end, endFrame,
    /// cost, ratio, similarity, blend }] }`, with card previews written to `dir/loop-<i>.mp4` (core adds `preview`).
    fn loop_search(&self, ctx: &TaskCtx, req: &LoopSearchReq) -> CoreResult<Value>;
    /// `{ best, current, alternatives, searched }` (1.2 `findLoopPoint`).
    fn loop_point(&self, ctx: &TaskCtx, req: &LoopPointReq) -> CoreResult<Value>;
    /// Bytes per second of the device encode at this friendly quality (for the "Auto" longest loop).
    fn bytes_per_second(&self, ctx: &TaskCtx, req: &EncodeReq, quality: u32) -> CoreResult<f64>;
    /// 1.2 `estimate`: `{ length, start, budget, duration, fps, mode, q, detail, bytes, fits, maxFitLength? }`.
    fn estimate(&self, ctx: &TaskCtx, req: &EncodeReq) -> CoreResult<Value>;
    /// 1.2 `convert`: writes device.mp4 (MJPEG), preview.mp4 and thumb.jpg into `out_dir`; returns
    /// `{ size, q, quality, detail, fps, start, length, tooBig, ... }`. Error code `tooLong` + `maxFitLength`.
    fn convert(&self, ctx: &TaskCtx, req: &EncodeReq, out_dir: &Path) -> CoreResult<Value>;
    /// The boot splash (SPLASH v1, ~1.5 s MJPEG) written to `out`.
    fn splash(&self, ctx: &TaskCtx, out: &Path) -> CoreResult<()>;
    /// Render the loop card previews `dir/loop-<i>.mp4` (320x144) for the candidates `loop_search` returned
    /// (in order, with their `blend`). Run as its own `cards` job after the search.
    fn loop_cards(&self, ctx: &TaskCtx, req: &LoopCardsReq) -> CoreResult<()> {
        let _ = (ctx, req);
        Ok(())
    }
    /// Splash version: the cached file is `splash-v<version>.mp4` in the app folder.
    fn splash_version(&self) -> u32 {
        1
    }
    /// Resources a task needs (e.g. 1 gpu for an NVDEC proxy of a 4K source, 2 cpu for parallel trial encodes).
    fn resources(&self, task: MediaTask, info: Option<&ProbeInfo>) -> Resources {
        let _ = info;
        match task {
            MediaTask::Probe => Resources::one(Res::Io),
            _ => Resources::one(Res::Cpu),
        }
    }
}

/// Stand-in until WS2 lands: every call fails with a clear message (probe reports "no video").
pub struct NoMedia;

impl MediaEngine for NoMedia {
    fn probe(&self, _: &TaskCtx, _: &Path) -> CoreResult<ProbeInfo> {
        Err(not_yet("Media engine"))
    }
    fn source_assets(&self, _: &TaskCtx, _: &Path, _: &Path, _: &ProbeInfo) -> CoreResult<SourceAssets> {
        Err(not_yet("Media engine"))
    }
    fn loop_search(&self, _: &TaskCtx, _: &LoopSearchReq) -> CoreResult<Value> {
        Err(not_yet("Media engine"))
    }
    fn loop_point(&self, _: &TaskCtx, _: &LoopPointReq) -> CoreResult<Value> {
        Err(not_yet("Media engine"))
    }
    fn bytes_per_second(&self, _: &TaskCtx, _: &EncodeReq, _: u32) -> CoreResult<f64> {
        Err(not_yet("Media engine"))
    }
    fn estimate(&self, _: &TaskCtx, _: &EncodeReq) -> CoreResult<Value> {
        Err(not_yet("Media engine"))
    }
    fn convert(&self, _: &TaskCtx, _: &EncodeReq, _: &Path) -> CoreResult<Value> {
        Err(not_yet("Media engine"))
    }
    fn splash(&self, _: &TaskCtx, _: &Path) -> CoreResult<()> {
        Err(not_yet("Media engine"))
    }
}

fn not_yet(what: &str) -> CoreError {
    CoreError::coded("unavailable", format!("{what} is not available in this build yet.")).status(501)
}

// ---------- sources (WS3) ----------

/// Per-source context: its options from settings.json and a temp folder for downloads.
#[derive(Clone, Debug)]
pub struct SourceCtx {
    /// `settings.sources.options[<id>]`, e.g. `{ folders: [...] }`.
    pub options: Value,
    /// %APPDATA%\CoolerScreen\downloads
    pub tmp_dir: PathBuf,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detect {
    pub available: bool,
    pub status: String,
}

/// One item as the provider lists it (core cleans it for the UI, as 1.2's server did).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SourceItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub thumb: Option<String>,
    #[serde(default, rename = "type")]
    pub kind: String,
    /// "add" | "record" | "none"
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub note: String,
    /// Appears in Gallery file names ("Title [we-123].mp4") so items show as "In your gallery".
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub badge: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemPage {
    pub items: Vec<SourceItem>,
    pub total: Option<u64>,
    pub has_more: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AddOpts {
    /// Scene recording length.
    pub seconds: Option<f64>,
    /// Record (Wallpaper Engine scene) instead of copying a file.
    pub record: bool,
}

/// A file ready to be imported into the Gallery folder under `name` (core deletes it afterwards when `temp`).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddedFile {
    pub file: PathBuf,
    pub name: String,
    pub temp: bool,
}

pub trait SourceProvider: Send + Sync + 'static {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// "online" | "local" | "action" | "plugin"
    fn kind(&self) -> &str;
    /// `[{ key, label, type }]` for Settings > Sources.
    fn options(&self) -> Vec<Value> {
        vec![]
    }
    /// Extra fields for the Settings row (e.g. a plug-in `warning`).
    fn extra(&self) -> Value {
        Value::Null
    }
    fn detect(&self, ctx: &SourceCtx) -> Detect;
    fn list(&self, ctx: &SourceCtx, page: u32, refresh: bool) -> CoreResult<ItemPage> {
        let _ = (ctx, page, refresh);
        Ok(ItemPage::default())
    }
    /// Local file for a `thumb:<item>` thumbnail.
    fn thumb_path(&self, ctx: &SourceCtx, item: &str) -> Option<PathBuf> {
        let _ = (ctx, item);
        None
    }
    fn add(&self, ctx: &SourceCtx, task: &TaskCtx, item: &str, opts: &AddOpts) -> CoreResult<AddedFile> {
        let _ = (ctx, task, item, opts);
        Err(CoreError::new("This source cannot add items."))
    }
    /// Only the "link" source: download a direct link.
    fn download(&self, ctx: &SourceCtx, task: &TaskCtx, url: &str) -> CoreResult<AddedFile> {
        let _ = (ctx, task, url);
        Err(CoreError::new("This source cannot download links."))
    }
    fn resources(&self, item: &str, opts: &AddOpts) -> Resources {
        let _ = item;
        if opts.record {
            Resources::one(Res::Capture)
        } else {
            Resources::one(Res::Net)
        }
    }
}

pub trait SourceCatalog: Send + Sync + 'static {
    /// Built-ins and plug-ins (WS3 decides the plug-in format). Called on every sources request; cache inside.
    fn providers(&self) -> Vec<Arc<dyn SourceProvider>>;
    fn plugins_dir(&self) -> PathBuf;
    /// Scene recording available (AppState.desktopApp).
    fn desktop_capture(&self) -> bool {
        false
    }
}

/// Stand-in until WS3 lands: no sources besides My clips.
pub struct NoSources;

impl SourceCatalog for NoSources {
    fn providers(&self) -> Vec<Arc<dyn SourceProvider>> {
        vec![]
    }
    fn plugins_dir(&self) -> PathBuf {
        super::paths::documents_dir().join("CoolerScreen").join("Sources")
    }
}

/// What `lib.rs::run` is given.
#[derive(Clone)]
pub struct Backends {
    pub media: Arc<dyn MediaEngine>,
    pub sources: Arc<dyn SourceCatalog>,
}

impl Default for Backends {
    fn default() -> Self {
        Backends { media: Arc::new(NoMedia), sources: Arc::new(NoSources) }
    }
}
