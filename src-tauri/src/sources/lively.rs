//! Source: Lively Wallpaper (local). Reads `LivelyInfo.json` in Lively's library, read-only. Video, GIF and picture
//! wallpapers are copied into the Gallery folder; web/app wallpapers aren't supported.
use super::*;
use std::path::{Path, PathBuf};

pub struct Lively;
const PAGE: usize = 36;

/// Lively's WallpaperType numbers: 7 video, 8 gif, 10 video stream, 11 picture; others are web/app types.
fn type_name(t: i64) -> &'static str {
    match t {
        0 => "app",
        1 => "web",
        2 => "web audio",
        3 => "url",
        4 => "bizhawk",
        5 => "unity",
        6 => "godot",
        7 => "video",
        8 => "gif",
        9 => "unity audio",
        10 => "video stream",
        11 => "picture",
        _ => "other",
    }
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub type_: String,
    pub file: Option<PathBuf>,
    pub action: Action,
    pub note: String,
    pub thumb: Option<PathBuf>,
}

pub fn libraries(ctx: &SourceCtx) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = vec![];
    if let Some(d) = std::env::var_os("COOLERSCREEN_LIVELY_DIR") {
        dirs.push(PathBuf::from(d)); // tests / unusual installs
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        dirs.push(local.join("Lively Wallpaper").join("Library").join("wallpapers"));
        if let Ok(rd) = std::fs::read_dir(local.join("Packages")) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().to_lowercase().contains("lively") {
                    dirs.push(e.path().join("LocalCache").join("Local").join("Lively Wallpaper").join("Library").join("wallpapers"));
                }
            }
        }
    }
    dirs.extend(ctx.option_paths("folders"));
    let mut seen = vec![];
    dirs.into_iter().filter(|d| d.is_dir() && { let fresh = !seen.contains(d); seen.push(d.clone()); fresh }).collect()
}

fn resolve(dir: &Path, v: &serde_json::Value) -> Option<PathBuf> {
    let s = v.as_str().filter(|s| !s.is_empty())?;
    let p = PathBuf::from(s);
    Some(if p.is_absolute() { p } else { dir.join(p) })
}

/// Parse one `LivelyInfo.json` (already read) living in `dir`.
pub fn parse_info(dir: &Path, folder_name: &str, j: &serde_json::Value) -> Entry {
    let t = j.get("Type").and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(-1);
    // Lively may store absolute paths (by design), so they are allowed, but only to existing media files of the
    // expected kind: a LivelyInfo.json can never make the app read or serve some other file.
    let has_ext = |p: &PathBuf, exts: &[&str]| p.extension().map(|e| e.to_string_lossy().to_lowercase()).is_some_and(|e| exts.contains(&e.as_str()));
    const MEDIA: [&str; 13] = ["mp4", "mov", "mkv", "webm", "avi", "m4v", "wmv", "gif", "jpg", "jpeg", "png", "bmp", "webp"];
    const IMAGES: [&str; 6] = ["jpg", "jpeg", "png", "bmp", "webp", "gif"];
    let file = j.get("FileName").and_then(|v| resolve(dir, v)).filter(|p| has_ext(p, &MEDIA) && p.is_file());
    let thumb = j
        .get("Thumbnail")
        .and_then(|v| resolve(dir, v))
        .filter(|p| has_ext(p, &IMAGES) && p.is_file())
        .or_else(|| j.get("Preview").and_then(|v| resolve(dir, v)).filter(|p| has_ext(p, &IMAGES) && p.is_file()));
    let importable = matches!(t, 7 | 8 | 11);
    let exists = file.as_ref().map_or(false, |f| f.exists());
    let tn = type_name(t);
    let key = format!("lively-{}", sha1_hex10(&dir.to_string_lossy()));
    let title: String = j.get("Title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(folder_name).chars().take(120).collect();
    Entry {
        id: key,
        title,
        type_: tn.into(),
        action: if importable && exists { Action::Add } else { Action::None },
        note: if importable { if exists { String::new() } else { "The file is missing".into() } } else { format!("\"{tn}\" wallpapers aren't supported") },
        file: if exists { file } else { None },
        thumb: thumb.filter(|p| p.exists()),
    }
}

static SCAN: ScanCache<Entry> = ScanCache::new();

/// `scan`, cached while the library folders are unchanged (30 s at most); `refresh` forces a new scan.
pub fn scan_cached(ctx: &SourceCtx, refresh: bool) -> std::sync::Arc<Vec<Entry>> {
    SCAN.get(&libraries(ctx), std::time::Duration::from_secs(30), refresh, || scan(ctx))
}

pub fn scan(ctx: &SourceCtx) -> Vec<Entry> {
    let mut items = vec![];
    for lib in libraries(ctx) {
        let Ok(rd) = std::fs::read_dir(&lib) else { continue };
        for e in rd.flatten() {
            let dir = e.path();
            let Ok(txt) = std::fs::read_to_string(dir.join("LivelyInfo.json")) else { continue };
            let Ok(j) = serde_json::from_str::<serde_json::Value>(txt.trim_start_matches('\u{feff}')) else { continue };
            items.push(parse_info(&dir, &e.file_name().to_string_lossy(), &j));
        }
    }
    items.sort_by(|a, b| (a.action == Action::None).cmp(&(b.action == Action::None)).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    items
}

impl SourceProvider for Lively {
    fn id(&self) -> &str {
        "lively"
    }
    fn name(&self) -> &str {
        "Lively Wallpaper"
    }
    fn description(&self) -> &str {
        "Video, GIF and picture wallpapers from your Lively Wallpaper library."
    }
    fn kind(&self) -> Kind {
        Kind::Local
    }
    fn options(&self) -> Vec<OptionDef> {
        vec![OptionDef { key: "folders".into(), label: "Extra library folders (one per line)".into(), type_: "folders".into() }]
    }
    fn detect(&self, ctx: &SourceCtx) -> Detect {
        if libraries(ctx).is_empty() {
            return Detect::no("Not installed (no Lively Wallpaper library found)");
        }
        let items = scan(ctx);
        let n = items.iter().filter(|i| i.action == Action::Add).count();
        Detect::ok(format!("Lively Wallpaper found: {} items ({n} can be added)", items.len()))
    }
    fn list(&self, ctx: &SourceCtx, page: u32, query: Option<&str>, refresh: bool) -> Result<ListPage> {
        let all: Vec<Entry> = scan_cached(ctx, refresh).iter().filter(|e| title_matches(&e.title, query)).cloned().collect();
        let p = page.max(1) as usize;
        let items = all.iter().skip((p - 1) * PAGE).take(PAGE).map(|i| Item {
            id: i.id.clone(), title: i.title.clone(), thumb: i.thumb.as_ref().map(|_| format!("thumb:{}", i.id)), type_: i.type_.clone(),
            action: i.action, note: i.note.clone(), key: i.id.clone(), badge: None }).collect();
        Ok(ListPage { items, total: all.len() as u64, has_more: p * PAGE < all.len() })
    }
    fn thumb_path(&self, ctx: &SourceCtx, item_id: &str) -> Option<PathBuf> {
        scan_cached(ctx, false).iter().find(|e| e.id == item_id).and_then(|e| e.thumb.clone())
    }
    fn add(&self, ctx: &SourceCtx, item_id: &str, _req: &AddRequest, progress: Progress, _cancel: &Cancel) -> Result<AddResult> {
        let Some(i) = scan_cached(ctx, false).iter().find(|e| e.id == item_id).cloned().or_else(|| scan_cached(ctx, true).iter().find(|e| e.id == item_id).cloned()) else { return err("This wallpaper is no longer there.") };
        let Some(file) = i.file.clone().filter(|_| i.action == Action::Add) else { return err(if i.note.is_empty() { "Not supported".to_string() } else { i.note }) };
        progress(0.5, "");
        let ext = file.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
        Ok(AddResult { name: format!("{} [{}]{ext}", safe_name(&i.title), i.id), file, temp: false })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn scratch_library() {
        let root = std::env::temp_dir().join(format!("cs-lively-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let a = root.join("aaa");
        let b = root.join("bbb");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("clip.mp4"), b"x").unwrap();
        std::fs::write(a.join("t.jpg"), b"x").unwrap();
        std::fs::write(a.join("LivelyInfo.json"), "\u{feff}{\"Title\":\"Rain\",\"Type\":7,\"FileName\":\"clip.mp4\",\"Thumbnail\":\"t.jpg\"}").unwrap();
        std::fs::write(b.join("LivelyInfo.json"), "{\"Title\":\"Web thing\",\"Type\":1,\"FileName\":\"index.html\"}").unwrap();
        let ctx = SourceCtx { options: serde_json::json!({ "folders": [root.to_string_lossy()] }), tmp_dir: root.clone(), data_dir: root.clone(), sources_dir: root.clone(), ffmpeg: "ffmpeg".into() };
        let page = Lively.list(&ctx, 1, None, false).unwrap();
        assert_eq!(page.total, 2);
        assert_eq!(page.items[0].title, "Rain");
        assert_eq!(page.items[0].action, Action::Add);
        assert!(page.items[0].thumb.is_some());
        assert_eq!(page.items[1].action, Action::None);
        let r = Lively.add(&ctx, &page.items[0].id, &AddRequest::default(), &|_, _| {}, &Cancel::new()).unwrap();
        assert!(r.name.starts_with("Rain [lively-") && r.name.ends_with("].mp4") && !r.temp);
        assert!(Lively.add(&ctx, &page.items[1].id, &AddRequest::default(), &|_, _| {}, &Cancel::new()).is_err());
        assert_eq!(Lively.list(&ctx, 1, Some("web"), false).unwrap().total, 1);
        let _ = std::fs::remove_dir_all(&root);
    }
}
