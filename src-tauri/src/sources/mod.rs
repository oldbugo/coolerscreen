//! Sources: places clips can come from besides the user's own files ("My clips" = the Gallery folder).
//!
//! Rust port of the 1.2 Node providers (`src/sources/*.js`). Every provider implements [`SourceProvider`]; the
//! contract mirrors the 1.2 interface (detect / list(page, query) / thumb / add(item, progress, cancel) -> a file path to
//! import into the Gallery). All calls are blocking: run them on a worker thread (`spawn_blocking`).
//!
//! Every source except My clips is off until enabled in Settings; that gate lives in the caller (settings), which passes
//! `enabled` to [`provider`]. Plug-ins are declarative `.json` files ([`declarative`]); they are not even read
//! until enabled.
pub mod declarative;
pub mod http;
pub mod link;
pub mod lively;
pub mod lovingcool;
pub mod otherbrands;
pub mod recorder;
pub mod themes;
pub mod wallpaperengine;
#[cfg(test)]
mod live_tests;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub type Result<T> = std::result::Result<T, SourceError>;

/// A user-facing error message (the strings are written to be shown as is).
#[derive(Debug, Clone, Serialize)]
pub struct SourceError(pub String);
impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for SourceError {}
impl From<String> for SourceError {
    fn from(s: String) -> Self {
        SourceError(s)
    }
}
impl From<&str> for SourceError {
    fn from(s: &str) -> Self {
        SourceError(s.to_string())
    }
}
impl From<std::io::Error> for SourceError {
    fn from(e: std::io::Error) -> Self {
        SourceError(e.to_string())
    }
}
pub fn err<T>(s: impl Into<String>) -> Result<T> {
    Err(SourceError(s.into()))
}

/// Cooperative cancellation (cheap to clone; the clones share one flag).
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);
impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst)
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() {
            err("Cancelled.")
        } else {
            Ok(())
        }
    }
}

/// Progress callback: fraction 0..1 and a short message (may be empty).
pub type Progress<'a> = &'a (dyn Fn(f32, &str) + Send + Sync);

/// What the caller gives every provider call.
#[derive(Clone, Debug)]
pub struct SourceCtx {
    /// This source's own settings (free-form JSON, e.g. `{"folders": ["D:\\Wallpapers"]}`).
    pub options: serde_json::Value,
    /// Scratch folder for downloads / recordings (files returned with `temp: true` are deleted by the caller after import).
    pub tmp_dir: PathBuf,
    /// App data folder: list caches live here.
    pub data_dir: PathBuf,
    /// Documents\CoolerScreen\Sources (declarative plug-ins).
    pub sources_dir: PathBuf,
    /// The ffmpeg sidecar (used by the Wallpaper Engine recorder).
    pub ffmpeg: PathBuf,
}
impl SourceCtx {
    pub fn option_paths(&self, key: &str) -> Vec<PathBuf> {
        match self.options.get(key) {
            Some(serde_json::Value::Array(a)) => a.iter().filter_map(|v| v.as_str()).map(PathBuf::from).collect(),
            Some(serde_json::Value::String(s)) => s.lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from).collect(),
            _ => vec![],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Online,
    Local,
    Action,
    Plugin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// "Add to gallery"
    Add,
    /// "Record" (Wallpaper Engine scene/web)
    Record,
    /// shown as not supported (see `note`)
    None,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    /// `https://...` URL, or `thumb:<id>` served through [`SourceProvider::thumb_path`].
    pub thumb: Option<String>,
    /// "theme", "video", "scene", ...
    #[serde(rename = "type")]
    pub type_: String,
    pub action: Action,
    pub note: String,
    /// Short unique text put in the Gallery file name so Browse can show "In your gallery".
    pub key: String,
    pub badge: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ListPage {
    pub items: Vec<Item>,
    pub total: u64,
    pub has_more: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Detect {
    pub available: bool,
    pub status: String,
}
impl Detect {
    pub fn ok(s: impl Into<String>) -> Self {
        Detect { available: true, status: s.into() }
    }
    pub fn no(s: impl Into<String>) -> Self {
        Detect { available: false, status: s.into() }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OptionDef {
    pub key: String,
    pub label: String,
    /// "folders" (list of paths)
    #[serde(rename = "type")]
    pub type_: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct AddRequest {
    /// Recording length for Wallpaper Engine scene/web wallpapers (3..60, default 20).
    pub seconds: Option<f32>,
}

/// The file to import into the Gallery, and the name to give it there.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddResult {
    pub file: PathBuf,
    pub name: String,
    /// true: the caller deletes `file` after copying it into the Gallery folder.
    pub temp: bool,
}

pub trait SourceProvider: Send + Sync {
    fn id(&self) -> &str;
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn kind(&self) -> Kind;
    fn options(&self) -> Vec<OptionDef> {
        vec![]
    }
    fn detect(&self, ctx: &SourceCtx) -> Detect;
    /// One page (1-based). `query` filters by title where the source supports it.
    fn list(&self, ctx: &SourceCtx, page: u32, query: Option<&str>, refresh: bool) -> Result<ListPage>;
    /// Local file behind a `thumb:<id>` thumbnail.
    fn thumb_path(&self, _ctx: &SourceCtx, _item_id: &str) -> Option<PathBuf> {
        None
    }
    /// Produce a file to import. For the "link" source `item_id` is the URL.
    fn add(&self, ctx: &SourceCtx, item_id: &str, req: &AddRequest, progress: Progress, cancel: &Cancel) -> Result<AddResult>;
}

/// Row for the Settings > Sources page.
#[derive(Clone, Debug, Serialize)]
pub struct SourceInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: Kind,
    pub options: Vec<OptionDef>,
    pub available: bool,
    pub status: String,
    pub warning: Option<String>,
}

pub const PLUGIN_WARNING: &str = "A plug-in file decides which web addresses CoolerScreen contacts and downloads files from. \
Only enable plug-ins you wrote or trust. Downloads are limited to the hosts the plug-in names, and web pages are never accepted as clips.";

pub fn builtin() -> Vec<Box<dyn SourceProvider>> {
    vec![
        Box::new(lovingcool::Lovingcool),
        Box::new(otherbrands::OtherBrands),
        Box::new(wallpaperengine::WallpaperEngine),
        Box::new(lively::Lively),
        Box::new(link::Link),
    ]
}

pub fn plugin_id(file: &Path) -> String {
    let stem = file.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let clean: String = stem.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '-' }).take(40).collect();
    format!("plugin-{clean}")
}

pub fn plugin_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map_or(false, |x| x.eq_ignore_ascii_case("json"))).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// All sources for Settings. `enabled(id)` comes from the caller's settings; plug-in files are only parsed when enabled.
pub fn describe_all(ctx_for: &dyn Fn(&str) -> SourceCtx, sources_dir: &Path, enabled: &dyn Fn(&str) -> bool) -> Vec<SourceInfo> {
    let mut out = vec![];
    for p in builtin() {
        let d = p.detect(&ctx_for(p.id()));
        out.push(SourceInfo { id: p.id().into(), name: p.name().into(), description: p.description().into(), kind: p.kind(), options: p.options(),
            available: d.available, status: d.status, warning: None });
    }
    for f in plugin_files(sources_dir) {
        let id = plugin_id(&f);
        let mut row = SourceInfo { id: id.clone(), name: f.file_name().unwrap().to_string_lossy().into(), description: format!("Plug-in file: {}", f.display()),
            kind: Kind::Plugin, options: vec![], available: true, status: "Not loaded (disabled)".into(), warning: Some(PLUGIN_WARNING.into()) };
        if enabled(&id) {
            match declarative::Declarative::load(&f) {
                Ok(p) => {
                    row.name = p.name().into();
                    row.description = p.description().into();
                    let d = p.detect(&ctx_for(&id));
                    row.available = d.available;
                    row.status = d.status;
                }
                Err(e) => {
                    row.available = false;
                    row.status = format!("Could not load: {e}");
                }
            }
        }
        out.push(row);
    }
    out
}

/// The provider for an id, only if `enabled` (the caller's settings say so). Plug-ins are loaded here, not before.
pub fn provider(id: &str, sources_dir: &Path, enabled: bool) -> Option<Box<dyn SourceProvider>> {
    if !enabled {
        return None;
    }
    if let Some(p) = builtin().into_iter().find(|p| p.id() == id) {
        return Some(p);
    }
    let f = plugin_files(sources_dir).into_iter().find(|f| plugin_id(f) == id)?;
    declarative::Declarative::load(&f).ok().map(|p| Box::new(p) as Box<dyn SourceProvider>)
}

pub fn safe_name(s: &str) -> String {
    let n: String = s.chars().map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 32 { '_' } else { c }).collect();
    let n = n.trim_end_matches(|c| c == '.' || c == ' ').trim();
    let n: String = n.chars().take(80).collect();
    if n.is_empty() {
        "wallpaper".into()
    } else {
        n
    }
}

pub(crate) fn title_matches(title: &str, query: Option<&str>) -> bool {
    match query.map(str::trim) {
        Some(q) if !q.is_empty() => title.to_lowercase().contains(&q.to_lowercase()),
        _ => true,
    }
}

pub(crate) fn sha1_hex10(s: &str) -> String {
    use sha1::{Digest, Sha1};
    let d = Sha1::digest(s.as_bytes());
    d.iter().map(|b| format!("{b:02x}")).collect::<String>()[..10].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names() {
        assert_eq!(safe_name("a:b/c. "), "a_b_c");
        assert_eq!(safe_name("   "), "wallpaper");
        assert_eq!(plugin_id(Path::new("C:/x/My Source.json")), "plugin-my-source");
        assert_eq!(sha1_hex10("abc"), "a9993e3647");
    }
}
