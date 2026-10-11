//! The Gallery folder (default Documents\CoolerScreen\Gallery), port of 1.2 `src/gallery.js`: every
//! video/GIF/image/.ht/.zip placed there is an original. Files are matched by path; a file renamed in Explorer is
//! recognised by its unchanged size and modification time. A file that disappears is marked "missing" (its
//! versions stay); the UI offers to delete originals missing for more than 30 days and never deletes them itself.
//! A `notify` watcher (debounced 1 s) rescans the folder; files still being written are retried (stable-file check).
use super::api::MediaTask;
use super::app::App;
use super::error::{CoreError, CoreResult};
use super::js::{self, iso_now, truthy};
use super::library::{self, is_source_name, new_id};
use super::scheduler::{Priority, TaskSpec};
use super::theme;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const MEDIA_EXT: [&str; 16] =
    [".mp4", ".mov", ".mkv", ".webm", ".avi", ".m4v", ".gif", ".jpg", ".jpeg", ".png", ".bmp", ".webp", ".tif", ".tiff", ".ht", ".zip"];
pub const STABLE_MS: u64 = 1500;
pub const MISSING_NOTICE_DAYS: u64 = 30;

#[derive(Default)]
pub struct GalleryState {
    /// fileKey -> reason, for files that cannot be read (retried when the file changes).
    bad: Mutex<HashMap<String, String>>,
    /// Asset attempts made in this session ("id|mtime"): a failure is retried at the next launch, never in a loop.
    assets_tried: Mutex<HashSet<String>>,
    sync_lock: Mutex<()>,
    watcher: Mutex<Option<(PathBuf, notify::RecommendedWatcher)>>,
    rescan: Mutex<Option<Arc<super::app::Debounce>>>,
    retry_pending: std::sync::atomic::AtomicBool,
    pub(crate) assets_enabled: std::sync::atomic::AtomicBool,
}

fn ext_lower(p: &Path) -> String {
    p.extension().and_then(|e| e.to_str()).map(|e| format!(".{}", e.to_lowercase())).unwrap_or_default()
}

pub fn is_media(p: &Path) -> bool {
    MEDIA_EXT.contains(&ext_lower(p).as_str())
}

fn same_path(a: &str, b: &Path) -> bool {
    let norm = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf()).to_string_lossy().to_lowercase();
    !a.is_empty() && norm(Path::new(a)) == norm(b)
}

/// `Math.round(stat.mtimeMs)`, computed exactly from the file time (so it matches what 1.2 recorded).
pub fn mtime_ms(md: &std::fs::Metadata) -> u64 {
    let Ok(t) = md.modified() else { return 0 };
    let n = t.duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let (ms, rem) = ((n / 1_000_000) as u64, n % 1_000_000);
    if rem >= 500_000 {
        ms + 1
    } else {
        ms
    }
}

fn file_key(f: &Path, md: &std::fs::Metadata) -> String {
    format!("{}|{}|{}", std::path::absolute(f).unwrap_or_else(|_| f.to_path_buf()).to_string_lossy().to_lowercase(), md.len(), mtime_ms(md))
}

pub fn sanitize_name(n: &str) -> String {
    let s: String = n.chars().map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 0x20 { '_' } else { c }).collect();
    let s = s.trim_end_matches(['.', ' ']).trim();
    let s: String = s.chars().take(100).collect();
    if s.is_empty() {
        "clip".into()
    } else {
        s
    }
}

pub fn unique_path(dir: &Path, base: &str, ext: &str) -> PathBuf {
    let mut p = dir.join(format!("{base}{ext}"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{base} ({i}){ext}"));
        i += 1;
    }
    p
}

/// Files added from a source carry its key in brackets ("Title [we-123]"): keep the key in the file name, not in
/// the clip's name.
fn strip_source_key(name: &str) -> &str {
    if let Some(open) = name.rfind(" [") {
        let inner = &name[open + 2..];
        if let Some(body) = inner.strip_suffix(']') {
            let mut parts = body.splitn(2, '-');
            let head = parts.next().unwrap_or("");
            let tail = parts.next();
            let ok_head = head.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && head.chars().all(|c| c.is_ascii_alphanumeric());
            let ok_tail = tail.is_some_and(|t| !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
            if ok_head && ok_tail {
                return &name[..open];
            }
        }
    }
    name
}

fn is_stable(file: &Path, md: &std::fs::Metadata) -> bool {
    let age = SystemTime::now().duration_since(md.modified().unwrap_or(UNIX_EPOCH)).unwrap_or_default();
    if age < Duration::from_millis(STABLE_MS) {
        return false;
    }
    std::fs::File::open(file).is_ok()
}

impl App {
    pub fn gallery_dir(&self) -> PathBuf {
        if let Some(g) = std::env::var_os("COOLERSCREEN_GALLERY") {
            return g.into();
        }
        let s = self.settings.load();
        if !s.gallery_dir().is_empty() {
            return s.gallery_dir().into();
        }
        super::paths::documents_dir().join("CoolerScreen").join("Gallery")
    }

    /// Read a gallery file into an original (new id, or `existing` to refresh one whose file changed).
    pub fn register_file(&self, file: &Path, existing: Option<&Value>) -> CoreResult<String> {
        let md = std::fs::metadata(file)?;
        let id = existing.and_then(|e| e["id"].as_str().map(String::from)).unwrap_or_else(new_id);
        let d = self.lib.o_dir(&id);
        std::fs::create_dir_all(&d)?;
        let r = (|| -> CoreResult<()> {
            let ext = ext_lower(file);
            let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("clip").to_string();
            let mut name = strip_source_key(&stem).to_string();
            let mut extracted = false;
            let mut orig = file.to_path_buf();
            if ext == ".ht" || ext == ".zip" || theme::is_theme_file(file) {
                if let Ok(rd) = std::fs::read_dir(&d) {
                    for e in rd.filter_map(Result::ok) {
                        if e.file_name().to_str().is_some_and(is_source_name) {
                            let _ = std::fs::remove_file(e.path());
                        }
                    }
                }
                let t = theme::extract_theme(file, &d)?;
                orig = d.join(format!("original{}", ext_lower(&t.path)));
                std::fs::rename(&t.path, &orig)?;
                extracted = true;
                if ext == ".ht" && !t.name.is_empty() {
                    name = t.name;
                }
            }
            let info = self.probe(&orig, Priority::User)?;
            if !info.has_video {
                return Err(CoreError::new(format!(
                    "CoolerScreen can't read \"{}\". It may be damaged or in a format that isn't supported.",
                    file.file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
                )));
            }
            for n in ["thumb.jpg", "frame.jpg", "proxy.mp4", "proxy.jpg"] {
                let _ = std::fs::remove_file(d.join(n));
            }
            let mut o = existing.and_then(|e| e.as_object().cloned()).unwrap_or_default();
            let fields = json!({
                "id": id,
                "name": existing.map(|e| e["name"].clone()).unwrap_or(json!(name)),
                "file": file.to_string_lossy(),
                "fileName": file.file_name().map(|n| n.to_string_lossy().to_string()),
                "extracted": extracted,
                "fileSize": md.len(),
                "fileMtime": mtime_ms(&md),
                "created": existing.map(|e| e["created"].clone()).unwrap_or(json!(iso_now())),
                "duration": if info.is_image { Value::Null } else { info.duration.map(js::num).unwrap_or(Value::Null) },
                "isImage": info.is_image,
                "width": info.width, "height": info.height, "codec": info.codec, "fps": info.fps.map(js::num), "rotated": info.rotated,
                "assets": "pending", "assetsError": null, "proxy": null, "missing": null,
                "sourceChanged": existing.is_some() && !self.lib.versions(&id).is_empty(),
            });
            for (k, v) in fields.as_object().unwrap() {
                o.insert(k.clone(), v.clone());
            }
            self.lib.write_original(&Value::Object(o))?;
            Ok(())
        })();
        if let Err(e) = r {
            if existing.is_none() {
                let _ = std::fs::remove_dir_all(&d);
            }
            return Err(e);
        }
        Ok(id)
    }

    /// One scan of the gallery folder (1.2 `doSync`). Returns true if some files were still being written.
    fn do_sync(&self) -> bool {
        let dir = self.gallery_dir();
        let _ = std::fs::create_dir_all(&dir);
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| {
                rd.filter_map(Result::ok)
                    .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                    .map(|e| e.path())
                    .filter(|p| is_media(p) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("~$")))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        let mut pending = false;
        let originals = self.lib.list_originals();
        let mut claimed: HashSet<String> = HashSet::new();
        let mut unknown: Vec<(PathBuf, std::fs::Metadata)> = vec![];
        // 1. Known paths: back from missing, or changed content.
        for f in &files {
            let Ok(md) = std::fs::metadata(f) else { continue };
            let Some(o) = originals.iter().find(|o| o["file"].as_str().is_some_and(|x| same_path(x, f))) else {
                unknown.push((f.clone(), md));
                continue;
            };
            let oid = o["id"].as_str().unwrap_or("").to_string();
            claimed.insert(oid.clone());
            if truthy(o.get("missing")) {
                let _ = self.lib.patch_original(&oid, &json!({ "missing": null }));
            }
            let (fs_, fm) = (o.get("fileSize").and_then(Value::as_u64), o.get("fileMtime").and_then(Value::as_f64));
            if fs_.is_none() || fm.is_none() {
                let _ = self.lib.patch_original(&oid, &json!({ "fileSize": md.len(), "fileMtime": mtime_ms(&md) }));
                continue;
            }
            if fs_ != Some(md.len()) || fm != Some(mtime_ms(&md) as f64) {
                let key = file_key(f, &md);
                if self.gallery.bad.lock().unwrap().contains_key(&key) {
                    continue;
                }
                if !is_stable(f, &md) {
                    pending = true;
                    continue;
                }
                let cur = self.lib.read_original(&oid);
                if let Err(e) = self.register_file(f, cur.as_ref()) {
                    self.gallery.bad.lock().unwrap().insert(key, e.message.clone());
                    eprintln!("Gallery: {}", e.message);
                }
            }
        }
        // 2. Originals whose file is gone: missing (kept), unless the file was renamed (same size and time).
        let gone: Vec<&Value> =
            originals.iter().filter(|o| !claimed.contains(o["id"].as_str().unwrap_or("")) && truthy(o.get("file"))).collect();
        for (f, md) in &unknown {
            let twin = gone.iter().find(|o| {
                !claimed.contains(o["id"].as_str().unwrap_or(""))
                    && o.get("fileSize").and_then(Value::as_u64) == Some(md.len())
                    && o.get("fileMtime").and_then(Value::as_f64) == Some(mtime_ms(md) as f64)
            });
            if let Some(t) = twin {
                let oid = t["id"].as_str().unwrap_or("").to_string();
                claimed.insert(oid.clone());
                let ht = matches!(ext_lower(f).as_str(), ".ht" | ".zip");
                let mut patch = json!({ "file": f.to_string_lossy(), "fileName": f.file_name().map(|n| n.to_string_lossy().to_string()), "missing": null });
                if !ht {
                    patch["name"] = json!(f.file_stem().map(|n| n.to_string_lossy().to_string()));
                }
                let _ = self.lib.patch_original(&oid, &patch);
                continue;
            }
            let key = file_key(f, md);
            if self.gallery.bad.lock().unwrap().contains_key(&key) {
                continue;
            }
            if !is_stable(f, md) {
                pending = true;
                continue;
            }
            if let Err(e) = self.register_file(f, None) {
                self.gallery.bad.lock().unwrap().insert(key, e.message.clone());
                eprintln!("Gallery: skipped {}: {}", f.display(), e.message);
            }
        }
        for o in gone {
            let oid = o["id"].as_str().unwrap_or("");
            if !claimed.contains(oid) && !truthy(o.get("missing")) {
                let _ = self.lib.patch_original(oid, &json!({ "missing": { "since": iso_now() } }));
            }
        }
        pending
    }

    /// Serialised scan of the gallery folder; afterwards queues any missing previews.
    pub fn gallery_sync(&self) {
        let pending = {
            let _g = self.gallery.sync_lock.lock().unwrap();
            self.do_sync()
        };
        if pending {
            self.schedule_rescan(Duration::from_millis(2000));
        }
        self.make_missing_assets();
    }

    /// Rescan after `delay` (watcher events and unstable files); bursts are coalesced.
    pub fn schedule_rescan(&self, delay: Duration) {
        use std::sync::atomic::Ordering;
        if self.gallery.retry_pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let app = self.arc();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            app.gallery.retry_pending.store(false, Ordering::SeqCst);
            app.gallery_sync();
        });
    }

    /// Thumbnail, notch frame and proxy for originals that don't have them yet, as background tasks (preemptible,
    /// re-prioritised by `set_focus`).
    pub fn make_missing_assets(&self) {
        use std::sync::atomic::Ordering;
        if !self.gallery.assets_enabled.load(Ordering::SeqCst) {
            return;
        }
        for o in self.lib.list_originals() {
            let ready = o["assets"] == "ready" && o.get("fps").is_some();
            let oid = o["id"].as_str().unwrap_or("").to_string();
            let key = format!("{oid}|{}", o.get("fileMtime").map(|v| v.to_string()).unwrap_or_default());
            if ready || self.gallery.assets_tried.lock().unwrap().contains(&key) {
                continue;
            }
            let Some(src) = self.lib.source_path(&o) else { continue };
            self.gallery.assets_tried.lock().unwrap().insert(key);
            let res = self.media.resources(MediaTask::Assets, None);
            let app = self.arc();
            let spec = TaskSpec::new("assets", format!("Previews for \"{}\"", o["name"].as_str().unwrap_or("clip")), res)
                .original(&oid)
                .priority(Priority::Visible)
                .auto_priority()
                .preemptible();
            self.sched.submit(spec, move |ctx| {
                let Some(cur) = app.lib.read_original(&oid) else { return Ok(Value::Null) };
                if cur["assets"] == "ready" {
                    // older entry without a source frame rate: probe only
                    let info = app.media.probe(ctx, &src)?;
                    app.lib.patch_original(&oid, &json!({ "fps": info.fps.map(js::num) }))?;
                    return Ok(Value::Null);
                }
                let r = app.media.probe(ctx, &src).and_then(|info| {
                    let a = app.media.source_assets(ctx, &src, &app.lib.o_dir(&oid), &info)?;
                    Ok((info, a))
                });
                match r {
                    Ok((info, a)) => {
                        app.lib.patch_original(&oid, &json!({ "assets": "ready", "proxy": a.proxy, "assetsError": null, "fps": info.fps.map(js::num) }))?;
                    }
                    Err(e) if e.is_cancelled() || e.code_is("preempted") => return Err(e),
                    Err(e) => {
                        app.lib.patch_original(&oid, &json!({ "assets": "error", "assetsError": e.message }))?;
                    }
                }
                Ok(Value::Null)
            });
        }
    }

    /// Start (or move) the folder watcher.
    pub fn gallery_watch(&self) {
        use notify::{RecursiveMode, Watcher};
        let dir = self.gallery_dir();
        let mut w = self.gallery.watcher.lock().unwrap();
        if w.as_ref().is_some_and(|(d, _)| d == &dir) {
            return;
        }
        *w = None;
        let deb = {
            let mut r = self.gallery.rescan.lock().unwrap();
            if r.is_none() {
                let weak = self.me.lock().unwrap().clone();
                *r = Some(super::app::Debounce::new(Duration::from_millis(1000), move || {
                    if let Some(a) = weak.upgrade() {
                        a.gallery_sync();
                    }
                }));
            }
            r.clone().unwrap()
        };
        let res = notify::recommended_watcher(move |ev: notify::Result<notify::Event>| {
            if ev.is_ok() {
                deb.poke();
            }
        });
        match res {
            Ok(mut watcher) => match watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => *w = Some((dir, watcher)),
                Err(e) => eprintln!("Gallery: cannot watch folder: {e}"),
            },
            Err(e) => eprintln!("Gallery: cannot watch folder: {e}"),
        }
    }

    /// Originals from before the Gallery folder existed (their only copy is in the cache): copy them into the folder.
    fn adopt_cache_only(&self) {
        let dir = self.gallery_dir();
        for o in self.lib.list_originals() {
            if truthy(o.get("file")) {
                continue;
            }
            let oid = o["id"].as_str().unwrap_or("").to_string();
            let d = self.lib.o_dir(&oid);
            let Some(f) = std::fs::read_dir(&d)
                .ok()
                .and_then(|rd| rd.filter_map(Result::ok).find(|e| e.file_name().to_str().is_some_and(is_source_name)))
            else {
                continue;
            };
            let target = unique_path(&dir, &sanitize_name(o["name"].as_str().unwrap_or("clip")), &ext_lower(&f.path()));
            let r = (|| -> CoreResult<()> {
                std::fs::copy(f.path(), &target)?;
                let md = std::fs::metadata(&target)?;
                self.lib.patch_original(
                    &oid,
                    &json!({ "file": target.to_string_lossy(), "fileName": target.file_name().map(|n| n.to_string_lossy().to_string()),
                        "extracted": false, "fileSize": md.len(), "fileMtime": mtime_ms(&md) }),
                )?;
                let _ = std::fs::remove_file(f.path());
                Ok(())
            })();
            if let Err(e) = r {
                eprintln!("Gallery: could not copy {} into the gallery folder: {}", o["name"], e.message);
            }
        }
    }

    /// Startup: folders, 1.1 migration, adopt cache-only originals, first scan, watcher. `assets`: also make
    /// thumbnails/previews (the app does; CLI tools skip it).
    pub fn gallery_init(&self, assets: bool) {
        use std::sync::atomic::Ordering;
        if assets {
            self.gallery.assets_enabled.store(true, Ordering::SeqCst);
        }
        let _ = std::fs::create_dir_all(self.lib.root());
        let _ = std::fs::create_dir_all(self.gallery_dir());
        let n = self.lib.migrate();
        if n > 0 {
            eprintln!("Library: migrated {n} item(s) to originals + versions.");
        }
        self.adopt_cache_only();
        self.gallery_sync();
        self.gallery_watch();
    }

    /// Copy a file into the Gallery (keeping its name, "name (2)" on clashes) and register it. Returns the id.
    pub fn add_file(&self, src: &Path, display_name: Option<&str>) -> CoreResult<String> {
        let dir = self.gallery_dir();
        std::fs::create_dir_all(&dir)?;
        let orig = display_name.map(String::from).unwrap_or_else(|| src.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
        let op = Path::new(&orig);
        let ext = ext_lower(op);
        let base = sanitize_name(&op.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
        let inside = src.parent().is_some_and(|p| same_path(&p.to_string_lossy(), &dir));
        if inside {
            if let Some(k) = self.lib.list_originals().iter().find(|o| o["file"].as_str().is_some_and(|f| same_path(f, src))) {
                return Ok(k["id"].as_str().unwrap_or("").to_string());
            }
        }
        let target = if inside { src.to_path_buf() } else { unique_path(&dir, &base, if ext.is_empty() { ".bin" } else { &ext }) };
        if !inside {
            std::fs::copy(src, &target)?;
        }
        let r = {
            let _g = self.gallery.sync_lock.lock().unwrap();
            self.register_file(&target, None)
        };
        match r {
            Ok(id) => {
                self.gallery_watch();
                self.make_missing_assets();
                Ok(id)
            }
            Err(e) => {
                if !inside {
                    let _ = std::fs::remove_file(&target);
                }
                Err(e)
            }
        }
    }

    pub fn rename_original(&self, id: &str, new_name: &str) -> CoreResult<Value> {
        let o = self.lib.read_original(id).ok_or_else(|| CoreError::new("Unknown clip"))?;
        let label: String = new_name.trim().chars().take(80).collect();
        if label.is_empty() {
            return Err(CoreError::new("Enter a name."));
        }
        let file = o["file"].as_str().map(PathBuf::from);
        let Some(file) = file.filter(|f| !truthy(o.get("missing")) && f.exists()) else {
            return self.lib.patch_original(id, &json!({ "name": label }));
        };
        let ext = file.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let target = file.parent().unwrap_or(Path::new(".")).join(format!("{}{ext}", sanitize_name(&label)));
        if !same_path(&target.to_string_lossy(), &file) && target.exists() {
            return Err(CoreError::new("A file with this name already exists in the Gallery folder."));
        }
        if target != file {
            std::fs::rename(&file, &target)?;
        }
        self.lib.patch_original(
            id,
            &json!({ "name": label, "file": target.to_string_lossy(), "fileName": target.file_name().map(|n| n.to_string_lossy().to_string()) }),
        )
    }

    /// Delete an original from the UI: its work is cancelled, its file goes to the Recycle Bin, its versions go.
    pub fn delete_original(&self, id: &str) -> CoreResult<()> {
        let Some(o) = self.lib.read_original(id) else { return Ok(()) };
        self.sched.cancel_original(id);
        // A "missing" original's file is not in the Gallery folder (maybe moved on purpose): never touch it.
        if let Some(f) = o["file"].as_str().map(PathBuf::from) {
            if !truthy(o.get("missing")) && f.exists() {
                super::recycle::recycle(&f)?;
            }
        }
        self.lib.remove_original_cache(id);
        Ok(())
    }

    /// Originals missing from the folder for longer than the notice period.
    pub fn long_missing(&self, now_ms: u64) -> Vec<Value> {
        self.lib
            .list_originals()
            .into_iter()
            .filter(|o| {
                o["missing"]["since"].as_str().and_then(js::parse_iso).is_some_and(|t| now_ms.saturating_sub(t) > MISSING_NOTICE_DAYS * 86_400_000)
            })
            .collect()
    }

    /// Delete the cache/versions of originals whose file is missing (user-confirmed in the UI).
    pub fn delete_missing(&self, ids: &[String]) -> usize {
        let mut n = 0;
        for id in ids {
            if self.lib.read_original(id).is_some_and(|o| truthy(o.get("missing"))) {
                self.sched.cancel_original(id);
                self.lib.remove_original_cache(id);
                n += 1;
            }
        }
        n
    }

    pub fn set_gallery_dir(&self, dir: &str) -> CoreResult<()> {
        let dir = dir.trim().to_string();
        if !dir.is_empty() {
            std::fs::create_dir_all(&dir)?;
        }
        self.settings.update(|s| s["galleryDir"] = json!(dir))?;
        let assets = self.gallery.assets_enabled.load(std::sync::atomic::Ordering::SeqCst);
        self.gallery_init(assets);
        Ok(())
    }
}

pub use library::basename;

#[cfg(test)]
mod tests {
    use super::*;

    /// Parity with 1.2 on real data: CS_PARITY = parity.json written by a Node script (recorded fileMtime,
    /// Math.round(stat.mtimeMs), theme extraction size + sha1). Read-only except a temp folder.
    #[test]
    #[ignore = "needs CS_PARITY (real library data)"]
    fn parity_with_node() {
        let p = std::env::var("CS_PARITY").expect("CS_PARITY");
        let rows: Vec<Value> = serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap();
        let mut themes = 0;
        for r in &rows {
            let f = Path::new(r["file"].as_str().unwrap());
            let md = std::fs::metadata(f).unwrap();
            assert_eq!(json!(mtime_ms(&md)), r["node"], "mtime of {}", f.display());
            assert_eq!(json!(mtime_ms(&md)), r["recorded"], "recorded mtime of {}", f.display());
            if let Some(t) = r.get("theme") {
                let d = tempfile::tempdir().unwrap();
                let x = theme::extract_theme(f, d.path()).unwrap();
                let buf = std::fs::read(&x.path).unwrap();
                let mut h = sha1_smol::Sha1::new();
                h.update(&buf);
                assert_eq!(json!(buf.len()), t["size"]);
                assert_eq!(json!(h.digest().to_string()), t["sha1"]);
                assert_eq!(json!(x.name), t["name"]);
                assert_eq!(json!(x.kind), t["kind"]);
                themes += 1;
            }
        }
        assert!(themes > 0);
    }

    #[test]
    fn names() {
        assert_eq!(strip_source_key("Sakura [we-2970331102]"), "Sakura");
        assert_eq!(strip_source_key("Sakura [x]"), "Sakura [x]");
        assert_eq!(strip_source_key("A [lc-ab-12]"), "A");
        assert_eq!(sanitize_name("a<b>:c. "), "a_b__c");
        assert_eq!(sanitize_name("..."), "clip");
    }
}
