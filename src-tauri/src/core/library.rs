//! Library = originals (files in the Gallery folder) and their versions (prepared copies for the screen), on the
//! SAME disk layout as 1.2 (`src/library.js`), so 1.2 users need no migration and 1.2 can still read it:
//!   library/<originalId>/original.json  thumb.jpg  frame.jpg  [proxy.mp4|proxy.jpg]  [original.<ext> for .ht/.zip]
//!   library/<originalId>/versions/<versionId>/version.json  device.mp4  preview.mp4  thumb.jpg
//! Records are kept as JSON objects (unknown fields survive a round trip through 2.0).
use super::error::{CoreError, CoreResult};
use super::js::{self, fmt, num, r3, read_json, round, to_num, truthy, write_json};
use super::mask::{base_key, norm_frame, notch_key};
use super::settings::{budgets, SettingsStore};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub const DEVICE_FPS: f64 = 18.0;
const O_FILES: [&str; 7] = ["thumb.jpg", "frame.jpg", "proxy.mp4", "proxy.jpg", "theme-preview.png", "theme-preview-base.png", "theme-card.png"];
const V_FILES: [&str; 3] = ["preview.mp4", "thumb.jpg", "device.mp4"];

pub fn is_id(id: &str) -> bool {
    (8..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// The extracted video of a .ht/.zip original is stored as original.<ext> (original.json is the metadata).
pub fn is_source_name(n: &str) -> bool {
    n.starts_with("original.") && n != "original.json" && !n.ends_with(".tmp")
}

fn base36(mut n: u64) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = vec![];
    loop {
        out.push(digits[(n % 36) as usize]);
        n /= 36;
        if n == 0 {
            break;
        }
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// `Date.now().toString(36) + 3 random bytes hex` (1.2 `newId`).
pub fn new_id() -> String {
    static N: AtomicU64 = AtomicU64::new(0);
    let t = super::scheduler::now_ms();
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let mut x = t
        ^ N.fetch_add(1, Ordering::Relaxed).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (std::process::id() as u64).rotate_left(29)
        ^ (nanos as u64).rotate_left(41);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    format!("{}{:06x}", base36(t), x & 0xff_ffff)
}

/// `path.basename` for Windows or POSIX separators.
pub fn basename(p: &str) -> &str {
    p.rsplit(['\\', '/']).next().unwrap_or(p)
}

/// JavaScript `String(v)`.
fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::Null) => "null".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => fmt(n.as_f64().unwrap_or(0.0)),
        Some(Value::Bool(b)) => b.to_string(),
        Some(o) => o.to_string(),
    }
}

/// `Number(v) || 0`
fn num0(v: Option<&Value>) -> f64 {
    let x = to_num(v);
    if x.is_nan() {
        0.0
    } else {
        x
    }
}

/// Current notch, budget and screen record the UI decorations are computed against (1.2 `context()`).
#[derive(Clone, Debug)]
pub struct Ctx {
    pub notch_key: String,
    pub budget: f64,
    pub standby: Option<Value>,
    pub max_seconds: f64,
    /// The notch mask (mask without corners).
    pub mask: Value,
    pub corners_default: Value,
}

pub struct Library {
    base: PathBuf,
    pub settings: Arc<SettingsStore>,
    rev: AtomicU64,
    v_index: Mutex<HashMap<String, String>>,
    media_base: RwLock<String>,
    splash_version: AtomicU32,
    on_change: Mutex<Option<Box<dyn Fn(u64) + Send + Sync>>>,
    /// Serialises read-modify-write of one record.
    write_lock: Mutex<()>,
}

impl Library {
    /// `base` = the app folder (%APPDATA%\CoolerScreen); the library is `base/library`.
    pub fn new(base: impl Into<PathBuf>, settings: Arc<SettingsStore>) -> Self {
        Library {
            base: base.into(),
            settings,
            rev: AtomicU64::new(1),
            v_index: Mutex::new(HashMap::new()),
            media_base: RwLock::new(String::new()),
            splash_version: AtomicU32::new(1),
            on_change: Mutex::new(None),
            write_lock: Mutex::new(()),
        }
    }

    pub fn base(&self) -> &Path {
        &self.base
    }
    pub fn root(&self) -> PathBuf {
        self.base.join("library")
    }
    pub fn o_dir(&self, oid: &str) -> PathBuf {
        self.root().join(oid)
    }
    pub fn v_root(&self, oid: &str) -> PathBuf {
        self.o_dir(oid).join("versions")
    }
    pub fn v_dir(&self, oid: &str, vid: &str) -> PathBuf {
        self.v_root(oid).join(vid)
    }

    /// Absolute prefix for media URLs (`http://127.0.0.1:PORT/TOKEN`); empty = 1.2's relative `/media/...`.
    pub fn set_media_base(&self, b: &str) {
        *self.media_base.write().unwrap() = b.trim_end_matches('/').to_string();
    }
    pub fn media_base(&self) -> String {
        self.media_base.read().unwrap().clone()
    }
    pub fn url(&self, rel: &str) -> String {
        format!("{}{}", self.media_base(), rel)
    }
    /// Rewrites stored relative `/media/...` URLs for the UI.
    fn abs(&self, v: &Value) -> Value {
        match v.as_str() {
            Some(s) if s.starts_with("/media/") => json!(self.url(s)),
            _ => v.clone(),
        }
    }

    pub fn set_splash_version(&self, v: u32) {
        self.splash_version.store(v, Ordering::Relaxed);
    }
    pub fn splash_file(&self) -> PathBuf {
        self.base.join(format!("splash-v{}.mp4", self.splash_version.load(Ordering::Relaxed)))
    }
    pub fn splash_size(&self) -> Option<u64> {
        std::fs::metadata(self.splash_file()).ok().map(|m| m.len())
    }

    // ---------- change counter ----------

    pub fn set_on_change(&self, f: impl Fn(u64) + Send + Sync + 'static) {
        *self.on_change.lock().unwrap() = Some(Box::new(f));
    }
    pub fn rev(&self) -> u64 {
        self.rev.load(Ordering::SeqCst)
    }
    pub fn bump(&self) -> u64 {
        let r = self.rev.fetch_add(1, Ordering::SeqCst) + 1;
        if let Some(f) = self.on_change.lock().unwrap().as_ref() {
            f(r);
        }
        r
    }

    // ---------- originals ----------

    pub fn read_original(&self, oid: &str) -> Option<Value> {
        if !is_id(oid) {
            return None;
        }
        read_json(&self.o_dir(oid).join("original.json")).filter(Value::is_object)
    }

    /// Every write and delete of a record holds this lock and re-reads the record inside it, so concurrent patches
    /// are never lost and a deleted record is never written back.
    fn lock(&self) -> std::sync::MutexGuard<'_, ()> {
        self.write_lock.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn write_original_locked(&self, o: &Value) -> CoreResult<Value> {
        let id = o["id"].as_str().filter(|i| is_id(i)).ok_or_else(|| CoreError::new("Bad clip id"))?;
        std::fs::create_dir_all(self.o_dir(id))?;
        write_json(&self.o_dir(id).join("original.json"), o)?;
        self.bump();
        Ok(o.clone())
    }

    /// Create (or replace) an original record.
    pub fn write_original(&self, o: &Value) -> CoreResult<Value> {
        let _g = self.lock();
        self.write_original_locked(o)
    }

    pub fn patch_original(&self, oid: &str, patch: &Value) -> CoreResult<Value> {
        let _g = self.lock();
        let o = self.read_original(oid).ok_or_else(|| CoreError::new("Unknown clip"))?;
        self.write_original_locked(&js::merge(&o, patch))
    }

    /// Read-modify-write under the lock: `f(current)` returns the new record (None: leave it). `current` is None when
    /// the record does not exist (any more).
    pub fn update_original(&self, oid: &str, f: impl FnOnce(Option<Value>) -> Option<Value>) -> CoreResult<Option<Value>> {
        let _g = self.lock();
        match f(self.read_original(oid)) {
            Some(o) => self.write_original_locked(&o).map(Some),
            None => Ok(None),
        }
    }

    /// Newest first (by `created`).
    pub fn list_originals(&self) -> Vec<Value> {
        let Ok(rd) = std::fs::read_dir(self.root()) else { return vec![] };
        let mut out: Vec<Value> = rd
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|n| is_id(n))
            .filter_map(|id| self.read_original(&id))
            .collect();
        out.sort_by(|a, b| {
            let (x, y) = (a["created"].as_str().unwrap_or(""), b["created"].as_str().unwrap_or(""));
            y.cmp(x)
        });
        out
    }

    /// The media to convert from: the gallery file, or the video extracted from a .ht/.zip (kept in the cache).
    pub fn source_path(&self, o: &Value) -> Option<PathBuf> {
        let id = o["id"].as_str()?;
        if truthy(o.get("extracted")) {
            let rd = std::fs::read_dir(self.o_dir(id)).ok()?;
            return rd
                .filter_map(Result::ok)
                .find(|e| e.file_name().to_str().is_some_and(is_source_name))
                .map(|e| e.path());
        }
        let f = o["file"].as_str()?;
        if truthy(o.get("missing")) {
            return None;
        }
        let p = PathBuf::from(f);
        p.exists().then_some(p)
    }

    pub fn remove_original_cache(&self, oid: &str) {
        if !is_id(oid) {
            return;
        }
        {
            let _g = self.lock();
            let _ = std::fs::remove_dir_all(self.o_dir(oid));
            self.v_index.lock().unwrap().retain(|_, o| o != oid);
        }
        self.bump();
    }

    // ---------- versions ----------

    /// Highest `n` first.
    pub fn versions(&self, oid: &str) -> Vec<Value> {
        let Ok(rd) = std::fs::read_dir(self.v_root(oid)) else { return vec![] };
        let mut out = vec![];
        for e in rd.filter_map(Result::ok) {
            let Ok(vid) = e.file_name().into_string() else { continue };
            if !is_id(&vid) {
                continue;
            }
            if let Some(v) = read_json(&self.v_dir(oid, &vid).join("version.json")).filter(Value::is_object) {
                self.v_index.lock().unwrap().insert(vid, oid.to_string());
                out.push(v);
            }
        }
        out.sort_by(|a, b| b["n"].as_f64().unwrap_or(0.0).total_cmp(&a["n"].as_f64().unwrap_or(0.0)));
        out
    }

    /// (originalId, version record)
    pub fn find_version(&self, vid: &str) -> Option<(String, Value)> {
        if !is_id(vid) {
            return None;
        }
        let mut oid = self.v_index.lock().unwrap().get(vid).cloned();
        if oid.as_ref().is_none_or(|o| !self.v_dir(o, vid).exists()) {
            oid = self
                .list_originals()
                .iter()
                .filter_map(|o| o["id"].as_str())
                .find(|o| self.v_dir(o, vid).exists())
                .map(String::from);
        }
        let oid = oid?;
        let v = read_json(&self.v_dir(&oid, vid).join("version.json")).filter(Value::is_object)?;
        self.v_index.lock().unwrap().insert(vid.to_string(), oid.clone());
        Some((oid, v))
    }

    fn write_version_locked(&self, oid: &str, v: &Value) -> CoreResult<Value> {
        let vid = v["id"].as_str().filter(|i| is_id(i)).ok_or_else(|| CoreError::new("Bad version id"))?;
        // Never resurrect a deleted clip as a folder holding only versions.
        if !self.o_dir(oid).join("original.json").exists() {
            return Err(CoreError::new("This clip is no longer in the library.").status(404));
        }
        std::fs::create_dir_all(self.v_dir(oid, vid))?;
        write_json(&self.v_dir(oid, vid).join("version.json"), v)?;
        self.v_index.lock().unwrap().insert(vid.to_string(), oid.to_string());
        self.bump();
        Ok(v.clone())
    }

    pub fn write_version(&self, oid: &str, v: &Value) -> CoreResult<Value> {
        let _g = self.lock();
        self.write_version_locked(oid, v)
    }

    pub fn patch_version(&self, vid: &str, patch: &Value) -> CoreResult<Value> {
        let _g = self.lock();
        let (oid, v) = self.find_version(vid).ok_or_else(|| CoreError::new("Unknown version"))?;
        self.write_version_locked(&oid, &js::merge(&v, patch))
    }

    pub fn delete_version(&self, vid: &str) -> bool {
        {
            let _g = self.lock();
            let Some((oid, _)) = self.find_version(vid) else { return false };
            let _ = std::fs::remove_dir_all(self.v_dir(&oid, vid));
            self.v_index.lock().unwrap().remove(vid);
        }
        self.bump();
        true
    }

    pub fn device_path(&self, vid: &str) -> Option<PathBuf> {
        let (oid, _) = self.find_version(vid)?;
        let p = self.v_dir(&oid, vid).join("device.mp4");
        p.exists().then_some(p)
    }

    /// Files the media server may serve for an original (`source` = the media itself).
    pub fn media_path_o(&self, oid: &str, file: &str) -> Option<PathBuf> {
        if !is_id(oid) {
            return None;
        }
        if file == "source" {
            return self.source_path(&self.read_original(oid)?);
        }
        let is_loop = file.len() == 10 && file.starts_with("loop-") && file.ends_with(".mp4") && file.as_bytes()[5].is_ascii_digit();
        if !O_FILES.contains(&file) && !is_loop {
            return None;
        }
        let p = self.o_dir(oid).join(file);
        p.exists().then_some(p)
    }

    pub fn media_path_v(&self, vid: &str, file: &str) -> Option<PathBuf> {
        if !V_FILES.contains(&file) {
            return None;
        }
        let (oid, _) = self.find_version(vid)?;
        let p = self.v_dir(&oid, vid).join(file);
        p.exists().then_some(p)
    }

    // ---------- settings of a version ----------

    pub fn context(&self) -> Ctx {
        let s = self.settings.load();
        let mut notch = s.mask().clone();
        let corners = notch.as_object_mut().and_then(|m| m.remove("corners")).unwrap_or(Value::Null);
        let fit = s.screen_fit();
        // The screen fit travels with the notch (the picture is fitted before the notch is applied) and is part of its key.
        if !fit.is_zero() {
            notch["fit"] = fit.to_value();
        }
        Ctx {
            notch_key: format!("{}{}", notch_key(s.mask()), fit.key()),
            budget: budgets(&s, self.splash_size()).loop_bytes,
            standby: s.slot("standby").cloned(),
            max_seconds: s.max_seconds(),
            mask: notch,
            corners_default: corners,
        }
    }

    /// The UI decorations of a version (1.2 `decorateVersion`).
    pub fn decorate_version(&self, v: &Value, ctx: &Ctx) -> Value {
        let r = v.get("result").filter(|r| r.is_object()).cloned().unwrap_or(json!({}));
        let fps = r.get("fps").and_then(Value::as_f64).filter(|x| *x != 0.0).or_else(|| v["settings"].get("fps").and_then(Value::as_f64));
        let ready = v["status"] == "ready";
        let speed = match fps {
            Some(f) if ready && f != DEVICE_FPS => json!({ "fps": num(f), "plays": if f < DEVICE_FPS { "fast" } else { "slow" } }),
            _ => Value::Null,
        };
        let vid = v["id"].as_str().unwrap_or("");
        let on_screen = ctx.standby.as_ref().is_some_and(|sb| sb["versionId"].as_str() == Some(vid));
        let mut out = v.as_object().cloned().unwrap_or_default();
        out.insert("settings".into(), effective_settings(v));
        out.insert("oldNotch".into(), json!(ready && base_key(v["notchKey"].as_str()) != ctx.notch_key));
        out.insert("speed".into(), speed);
        out.insert("tooBig".into(), json!(ready && r.get("size").and_then(Value::as_f64).is_some_and(|x| x > ctx.budget)));
        out.insert("onScreen".into(), json!(on_screen));
        out.insert(
            "onScreenIncomplete".into(),
            json!(on_screen && ctx.standby.as_ref().is_some_and(|sb| truthy(sb.get("incomplete")))),
        );
        Value::Object(out)
    }

    fn preview_url_of(&self, o: &Value) -> Value {
        let id = o["id"].as_str().unwrap_or("");
        if o["assets"] != "ready" {
            return Value::Null;
        }
        if let Some(p) = o["proxy"].as_str().filter(|p| !p.is_empty()) {
            return json!(self.url(&format!("/media/o/{id}/{p}")));
        }
        if self.source_path(o).is_some() {
            json!(self.url(&format!("/media/o/{id}/source")))
        } else {
            Value::Null
        }
    }

    /// One gallery card (1.2 `summary`).
    pub fn summary(&self, o: &Value, ctx: &Ctx) -> Value {
        let id = o["id"].as_str().unwrap_or("");
        let vs = self.versions(id);
        let status = |v: &Value, s: &[&str]| s.iter().any(|x| v["status"] == *x);
        let standby_vid = ctx.standby.as_ref().and_then(|s| s["versionId"].as_str().map(String::from));
        let file_name = match o["file"].as_str().filter(|f| !f.is_empty()) {
            Some(f) => json!(basename(f)),
            None => o.get("fileName").cloned().unwrap_or(Value::Null),
        };
        json!({
            "id": id,
            "name": o.get("name").cloned().unwrap_or(Value::Null),
            "fileName": file_name,
            "isImage": truthy(o.get("isImage")),
            "duration": o.get("duration").cloned().unwrap_or(Value::Null),
            "width": o.get("width").cloned().unwrap_or(Value::Null),
            "height": o.get("height").cloned().unwrap_or(Value::Null),
            "created": o.get("created").cloned().unwrap_or(Value::Null),
            "assets": o.get("assets").filter(|a| truthy(Some(a))).cloned().unwrap_or(json!("pending")),
            "assetsError": o.get("assetsError").filter(|a| truthy(Some(a))).cloned().unwrap_or(Value::Null),
            "missing": o.get("missing").filter(|a| truthy(Some(a))).cloned().unwrap_or(Value::Null),
            "sourceChanged": truthy(o.get("sourceChanged")),
            "versionCount": vs.len(),
            "busy": vs.iter().any(|v| status(v, &["queued", "converting"])),
            "failed": vs.iter().any(|v| status(v, &["error"])),
            "onScreen": vs.iter().any(|v| standby_vid.is_some() && v["id"].as_str() == standby_vid.as_deref()),
            "frameUrl": if o["assets"] == "ready" { json!(self.url(&format!("/media/o/{id}/frame.jpg"))) } else { Value::Null },
            "previewUrl": self.preview_url_of(o),
            "needsRemake": vs.iter().any(|v| { let d = self.decorate_version(v, ctx); d["oldNotch"] == true || !d["speed"].is_null() }),
        })
    }

    /// The open clip (1.2 `detail`).
    pub fn detail(&self, oid: &str) -> Option<Value> {
        let o = self.read_original(oid)?;
        let ctx = self.context();
        let src = self.source_path(&o);
        let mut d = self.summary(&o, &ctx).as_object().cloned().unwrap_or_default();
        d.insert("file".into(), o.get("file").cloned().unwrap_or(Value::Null));
        d.insert("extracted".into(), json!(truthy(o.get("extracted"))));
        d.insert("draft".into(), o.get("draft").filter(|x| truthy(Some(x))).cloned().unwrap_or(Value::Null));
        d.insert("canTune".into(), json!(src.is_some() && o["assets"] == "ready"));
        d.insert("previewKind".into(), json!(if truthy(o.get("isImage")) { "image" } else { "video" }));
        d.insert("fps".into(), o.get("fps").filter(|x| truthy(Some(x))).cloned().unwrap_or(Value::Null));
        let loops = match o.get("loops") {
            Some(l) if loops_current(&o) => {
                let mut l = l.clone();
                if let Some(cs) = l.get_mut("candidates").and_then(Value::as_array_mut) {
                    for c in cs {
                        if let Some(p) = c.get("preview").cloned() {
                            c["preview"] = self.abs(&p);
                        }
                    }
                }
                l
            }
            _ => Value::Null,
        };
        d.insert("loops".into(), loops);
        let versions: Vec<Value> = self.versions(oid).iter().map(|v| self.decorate_version(v, &ctx)).collect();
        d.insert("versions".into(), json!(versions));
        Some(Value::Object(d))
    }

    // ---------- making versions ----------

    /// New queued version record (1.2 `createVersion`); error code `duplicate` (+ versionId) if the same settings
    /// were already saved, unless `body.force`.
    pub fn create_version(&self, oid: &str, body: &Value) -> CoreResult<Value> {
        let o = self.read_original(oid).ok_or_else(|| CoreError::new("Unknown clip"))?;
        if self.source_path(&o).is_none() {
            return Err(CoreError::new("The original file is missing, so a new version can't be made."));
        }
        let ctx = self.context();
        let st = clean_settings(&o, body, ctx.max_seconds, &ctx.corners_default);
        // 3.0: text/picture items baked into the base ({ key, file }); a version without them never duplicates one with.
        let overlay = body.get("overlay").filter(|v| v.is_object() && v["key"].is_string()).cloned();
        let ov_key = |v: &Value| v["overlay"]["key"].as_str().map(String::from);
        let want = overlay.as_ref().and_then(|o| o["key"].as_str().map(String::from));
        let _g = self.lock();
        let vs = self.versions(oid);
        let dup = vs.iter().find(|v| {
            v["status"] != "error"
                && ov_key(v) == want
                && base_key(v["notchKey"].as_str()) == ctx.notch_key
                && v["settings"].is_object()
                && same_settings(&effective_settings(v), &st)
                && v["settings"].get("fps").and_then(Value::as_f64).filter(|x| *x != 0.0).unwrap_or(DEVICE_FPS) == DEVICE_FPS
        });
        if let Some(d) = dup {
            if !truthy(body.get("force")) {
                return Err(CoreError::coded("duplicate", format!("Already saved as v{}.", fmt(d["n"].as_f64().unwrap_or(0.0))))
                    .with("versionId", d["id"].clone()));
            }
        }
        // (3.0: the corner radius of a version never changes the default in Settings > Themes; a theme's own radius is its override.)
        let n = vs.iter().map(|v| v["n"].as_f64().unwrap_or(0.0)).fold(0.0, f64::max) + 1.0;
        let mut settings = st.as_object().cloned().unwrap_or_default();
        settings.insert("fps".into(), num(DEVICE_FPS));
        let v = json!({
            "id": new_id(), "originalId": oid, "n": num(n), "label": null, "created": js::iso_now(),
            "settings": settings, "notch": ctx.mask, "notchKey": ctx.notch_key, "result": null, "status": "queued", "error": null, "overlay": overlay,
        });
        self.write_version_locked(oid, &v)
    }

    /// A copy of an old version with the same trim/quality/framing, the current notch and the screen's frame rate.
    pub fn remake_version(&self, vid: &str) -> CoreResult<Value> {
        let (oid, v) = self.find_version(vid).ok_or_else(|| CoreError::new("Unknown version"))?;
        let e = effective_settings(&v);
        let mut body = Map::new();
        for k in ["start", "length", "quality", "frame", "corners", "detail", "blend"] {
            body.insert(k.into(), e.get(k).cloned().unwrap_or(Value::Null));
        }
        if let Some(o) = v.get("overlay").filter(|o| o.is_object()) {
            body.insert("overlay".into(), o.clone());
        }
        body.insert("force".into(), json!(true));
        self.create_version(&oid, &Value::Object(body))
    }

    /// At startup: versions that were queued or converting when the app closed did not finish; stray temp files go.
    pub fn reset_interrupted(&self) {
        for o in self.list_originals() {
            let oid = o["id"].as_str().unwrap_or("").to_string();
            for v in self.versions(&oid) {
                if v["status"] == "queued" || v["status"] == "converting" {
                    let _ = self.write_version(
                        &oid,
                        &js::merge(&v, &json!({ "status": "error", "error": "The app was closed before this version was finished." })),
                    );
                }
            }
            if let Ok(rd) = std::fs::read_dir(self.o_dir(&oid)) {
                for e in rd.filter_map(Result::ok) {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.ends_with(".tmp") || n.ends_with(".tmp.mp4") {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
        }
    }

    /// 1.1 → originals + versions (each old library/<id>/meta.json becomes an original; its converted file v1).
    /// Returns the number of items migrated.
    pub fn migrate(&self) -> usize {
        let Ok(rd) = std::fs::read_dir(self.root()) else { return 0 };
        let ids: Vec<String> = rd.filter_map(Result::ok).filter_map(|e| e.file_name().into_string().ok()).filter(|n| is_id(n)).collect();
        let s = self.settings.load();
        let mut n = 0;
        for id in ids {
            let d = self.o_dir(&id);
            let meta_f = d.join("meta.json");
            if d.join("original.json").exists() || !meta_f.exists() {
                continue;
            }
            let Some(m) = read_json(&meta_f) else { continue };
            let g = |k: &str| m.get(k).filter(|x| truthy(Some(x))).cloned().unwrap_or(Value::Null);
            let req = m.get("requested").filter(|x| x.is_object()).cloned();
            let trim = m.get("trim").filter(|x| x.is_object()).cloned();
            let draft = match &req {
                Some(r) => json!({
                    "start": trim.as_ref().map(|t| t["start"].clone()).unwrap_or_else(|| json!(num0(r.get("start")))),
                    "length": trim.as_ref().map(|t| t["length"].clone()).unwrap_or_else(|| json!(num0(r.get("length")))),
                    "quality": r.get("quality").filter(|q| !q.is_null()).cloned().unwrap_or(json!("auto")),
                    "frame": { "zoom": 1, "x": 0.5, "y": 0.5 },
                }),
                None => Value::Null,
            };
            let o = json!({
                "id": id, "name": m.get("name").filter(|x| truthy(Some(x))).cloned().unwrap_or(json!("clip")),
                "file": g("file"), "fileName": g("source"),
                "extracted": truthy(m.get("extracted")) || !truthy(m.get("file")),
                "fileSize": g("fileSize"), "fileMtime": g("fileMtime"),
                "created": m.get("created").filter(|x| truthy(Some(x))).cloned().unwrap_or_else(|| json!(js::iso_now())),
                "duration": m.get("sourceDuration").cloned().unwrap_or(Value::Null), "isImage": truthy(m.get("isImage")),
                "width": g("srcWidth"), "height": g("srcHeight"),
                "assets": "pending", "proxy": null, "missing": null, "sourceChanged": false, "draft": draft,
            });
            let dev = d.join("device.mp4");
            if dev.exists() && truthy(m.get("size")) {
                let vid = new_id();
                let vd = self.v_dir(&id, &vid);
                let _ = std::fs::create_dir_all(&vd);
                for f in ["device.mp4", "preview.mp4", "thumb.jpg"] {
                    if d.join(f).exists() {
                        let _ = std::fs::rename(d.join(f), vd.join(f));
                    }
                }
                let r = req.clone().unwrap_or(json!({}));
                let trim = trim.clone().unwrap_or_else(|| json!({ "start": num0(r.get("start")), "length": num0(r.get("length")) }));
                let mask = m.get("mask").filter(|x| truthy(Some(x))).cloned().unwrap_or_else(|| s.mask().clone());
                let quality = match r.get("quality") {
                    None | Some(Value::Null) => json!("auto"),
                    Some(q) if q == "auto" => json!("auto"),
                    Some(q) => q.clone(),
                };
                let v = json!({
                    "id": vid, "originalId": id, "n": 1, "label": null,
                    "created": m.get("converted").filter(|x| truthy(Some(x))).cloned().unwrap_or(o["created"].clone()),
                    "settings": { "start": trim["start"], "length": trim["length"], "quality": quality,
                        "frame": { "zoom": 1, "x": 0.5, "y": 0.5 }, "fps": m.get("fps").filter(|x| truthy(Some(x))).or(r.get("fps")).cloned().unwrap_or(Value::Null) },
                    "notch": mask, "notchKey": notch_key(&mask),
                    "result": { "size": m["size"], "q": m.get("q").cloned().unwrap_or(Value::Null), "quality": m.get("quality").cloned().unwrap_or(Value::Null),
                        "fps": m.get("fps").cloned().unwrap_or(Value::Null), "start": trim["start"], "length": trim["length"],
                        "tooBig": truthy(m.get("tooBig")), "budgetBytes": m.get("budgetBytes").cloned().unwrap_or(Value::Null) },
                    "status": "ready", "error": null, "migrated": true,
                });
                let _ = write_json(&vd.join("version.json"), &v);
                if let Some(sb) = s.slot("standby") {
                    if sb.get("versionId").is_none() && sb["name"] == o["name"] && sb["size"] == m["size"] {
                        let _ = self.settings.patch_slot("standby", &json!({ "originalId": id, "versionId": vid, "label": "v1" }));
                    }
                }
            }
            let _ = write_json(&d.join("original.json"), &o);
            let _ = std::fs::rename(&meta_f, d.join("meta.v1.json"));
            n += 1;
        }
        if n > 0 {
            self.bump();
        }
        n
    }
}

// ---------- settings of a version (pure) ----------

/// `cleanCorners`
pub fn clean_corners(c: Option<&Value>) -> Value {
    let c = c.filter(|c| truthy(Some(c)));
    let on = c.is_some_and(|c| truthy(c.get("on")));
    let r = round(num0(c.and_then(|c| c.get("r"))).max(0.0).min(300.0));
    json!({ "on": on, "r": num(r) })
}

/// User settings for a version, cleaned (1.2 `cleanSettings`).
/// Stored loop results made with an older search are searched again (2 = blend-aware search with the ghost check).
pub const LOOP_SEARCH_VERSION: u32 = 2;

/// The original's stored loop results are for its current file and the current search.
pub fn loops_current(o: &Value) -> bool {
    o.get("loops").map_or(false, |l| l.is_object() && js_string(l.get("key")) == js_string(o.get("fileMtime")) && l.get("search") == Some(&json!(LOOP_SEARCH_VERSION)))
}

pub fn clean_settings(o: &Value, s: &Value, max_seconds: f64, corners_default: &Value) -> Value {
    let is_image = truthy(o.get("isImage"));
    let duration = o.get("duration").and_then(Value::as_f64).filter(|d| *d != 0.0);
    let quality = match s.get("quality") {
        None | Some(Value::Null) => json!("auto"),
        Some(q) if q == "auto" => json!("auto"),
        Some(q) => {
            let x = round(to_num(Some(q)));
            let x = if x.is_nan() || x == 0.0 { 80.0 } else { x };
            num(x.max(1.0).min(100.0))
        }
    };
    let auto = quality == "auto";
    let mut start = if is_image { 0.0 } else { r3(num0(s.get("start")).max(0.0)) };
    if let Some(d) = duration {
        if start > d - 0.05 {
            start = r3((d - 0.05).max(0.0));
        }
    }
    let mut length = to_num(s.get("length"));
    let avail = if is_image || duration.is_none() { f64::INFINITY } else { duration.unwrap() - start };
    if !(length > 0.0) {
        length = if is_image { 3.0 } else { max_seconds.min(avail) };
    }
    length = r3(length.min(avail).min(120.0).max(0.05));
    let detail = if auto { 0.0 } else { round(num0(s.get("detail")).max(0.0).min(100.0)) };
    let blend = if is_image { 0.0 } else { r3(num0(s.get("blend")).max(0.0).min(1.5)) };
    let corners = match s.get("corners").filter(|c| truthy(Some(c))) {
        Some(c) => clean_corners(Some(c)),
        None => clean_corners(Some(corners_default)),
    };
    json!({
        "start": num(start), "length": num(length), "quality": quality, "frame": norm_frame(s.get("frame").unwrap_or(&Value::Null)),
        "corners": corners, "detail": num(detail), "blend": num(blend), "manualLoop": truthy(s.get("manualLoop")),
    })
}

/// A version's settings with the fields older versions don't have filled in (corners came from the global mask then).
pub fn effective_settings(v: &Value) -> Value {
    let s = v.get("settings").filter(|s| s.is_object()).cloned().unwrap_or(json!({}));
    let corners = s
        .get("corners")
        .filter(|c| truthy(Some(c)))
        .cloned()
        .or_else(|| v.get("notch").and_then(|n| n.get("corners")).cloned());
    let mut m = s.as_object().cloned().unwrap_or_default();
    m.insert("corners".into(), clean_corners(corners.as_ref()));
    let or0 = |k: &str| s.get(k).filter(|x| truthy(Some(x))).cloned().unwrap_or(json!(0));
    m.insert("detail".into(), or0("detail"));
    m.insert("blend".into(), or0("blend"));
    Value::Object(m)
}

pub fn same_settings(a: &Value, b: &Value) -> bool {
    let n = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64);
    let on = |v: &Value| truthy(v["corners"].get("on"));
    n(a, "start") == n(b, "start")
        && n(a, "length") == n(b, "length")
        && js_string(a.get("quality")) == js_string(b.get("quality"))
        && norm_frame(&a["frame"]) == norm_frame(&b["frame"])
        && on(a) == on(b)
        && (!on(a) || n(&a["corners"], "r") == n(&b["corners"], "r"))
        && n(a, "detail") == n(b, "detail")
        && n(a, "blend") == n(b, "blend")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> (tempfile::TempDir, Library) {
        let d = tempfile::tempdir().unwrap();
        let st = Arc::new(SettingsStore::new(d.path().join("settings.json")));
        let l = Library::new(d.path(), st);
        (d, l)
    }

    fn add_original(l: &Library, file: &Path) -> String {
        let id = new_id();
        l.write_original(&json!({
            "id": id, "name": "clip", "file": file.to_string_lossy(), "fileName": "clip.mp4", "extracted": false,
            "fileSize": 10, "fileMtime": 1234, "created": js::iso_now(), "duration": 20.0, "isImage": false,
            "width": 1920, "height": 1080, "codec": "h264", "fps": 25, "assets": "ready", "assetsError": null, "proxy": null,
            "missing": null, "sourceChanged": false, "customField": { "kept": true }
        }))
        .unwrap();
        id
    }

    #[test]
    fn ids() {
        let a = new_id();
        assert!(is_id(&a), "{a}");
        assert_ne!(a, new_id());
        assert!(!is_id("ABCDEFGH"));
        assert!(!is_id("short"));
    }

    #[test]
    fn originals_versions_round_trip() {
        let (d, l) = lib();
        let src = d.path().join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        let oid = add_original(&l, &src);
        let o = l.read_original(&oid).unwrap();
        assert_eq!(o["customField"]["kept"], json!(true), "unknown fields survive");
        assert_eq!(l.source_path(&o), Some(src.clone()));

        let v1 = l.create_version(&oid, &json!({ "start": 1, "length": 5, "quality": "auto" })).unwrap();
        assert_eq!(v1["status"], "queued");
        assert_eq!(v1["n"], json!(1));
        assert_eq!(v1["notchKey"], "band:left:96");
        assert_eq!(v1["settings"]["fps"], json!(18));
        // same settings again: duplicate
        let e = l.create_version(&oid, &json!({ "start": 1, "length": 5 })).unwrap_err();
        assert!(e.code_is("duplicate"));
        assert_eq!(e.extra["versionId"], v1["id"]);
        let v2 = l.create_version(&oid, &json!({ "start": 1, "length": 5, "force": true })).unwrap();
        assert_eq!(v2["n"], json!(2));
        let vid = v1["id"].as_str().unwrap();
        assert_eq!(l.find_version(vid).unwrap().0, oid);
        l.patch_version(vid, &json!({ "status": "ready", "result": { "size": 20_000_000, "fps": 18 } })).unwrap();
        let det = l.detail(&oid).unwrap();
        assert_eq!(det["versions"][0]["n"], json!(2), "newest first");
        assert_eq!(det["versions"][1]["tooBig"], json!(true));
        assert_eq!(det["versions"][1]["oldNotch"], json!(false));
        assert_eq!(det["busy"], json!(true));
        // notch changed: old notch
        l.settings.update(|s| s["mask"]["band"]["px"] = json!(120)).unwrap();
        let det = l.detail(&oid).unwrap();
        assert_eq!(det["versions"][1]["oldNotch"], json!(true));
        assert_eq!(det["needsRemake"], json!(true));
        let r = l.remake_version(vid).unwrap();
        assert_eq!(r["notchKey"], "band:left:128");
        assert!(l.delete_version(vid));
        assert!(l.find_version(vid).is_none());
        l.reset_interrupted();
        assert_eq!(l.find_version(r["id"].as_str().unwrap()).unwrap().1["status"], "error");
        l.remove_original_cache(&oid);
        assert!(l.read_original(&oid).is_none());
    }

    #[test]
    fn concurrent_patches_are_not_lost() {
        let (d, l) = lib();
        let src = d.path().join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        let oid = add_original(&l, &src);
        let l = Arc::new(l);
        let hs: Vec<_> = (0..8)
            .map(|t| {
                let (l, oid) = (l.clone(), oid.clone());
                std::thread::spawn(move || {
                    for i in 0..25 {
                        l.patch_original(&oid, &json!({ format!("k{t}_{i}"): i })).unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let o = l.read_original(&oid).unwrap();
        assert_eq!(o.as_object().unwrap().keys().filter(|k| k.starts_with('k')).count(), 200);
        assert!(std::fs::read_dir(l.o_dir(&oid)).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }

    #[test]
    fn deleted_records_are_not_resurrected() {
        let (d, l) = lib();
        let src = d.path().join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        let oid = add_original(&l, &src);
        let v = l.create_version(&oid, &json!({ "start": 0, "length": 2 })).unwrap();
        let vid = v["id"].as_str().unwrap().to_string();
        assert!(l.delete_version(&vid));
        assert!(l.patch_version(&vid, &json!({ "status": "ready" })).is_err());
        assert!(!l.v_dir(&oid, &vid).exists());
        l.remove_original_cache(&oid);
        assert!(l.patch_original(&oid, &json!({ "x": 1 })).is_err());
        assert!(l.write_version(&oid, &v).is_err(), "no versions folder for a deleted clip");
        assert!(!l.o_dir(&oid).exists());
    }

    #[test]
    fn urls_are_absolute_with_media_base() {
        let (d, l) = lib();
        let src = d.path().join("clip.mp4");
        std::fs::write(&src, b"x").unwrap();
        let oid = add_original(&l, &src);
        l.patch_original(&oid, &json!({ "loops": { "key": "1234", "search": LOOP_SEARCH_VERSION, "candidates": [{ "preview": format!("/media/o/{oid}/loop-0.mp4?t=1") }] } })).unwrap();
        let rel = l.detail(&oid).unwrap();
        assert_eq!(rel["frameUrl"], json!(format!("/media/o/{oid}/frame.jpg")));
        l.set_media_base("http://127.0.0.1:5000/tok/");
        let det = l.detail(&oid).unwrap();
        assert_eq!(det["frameUrl"], json!(format!("http://127.0.0.1:5000/tok/media/o/{oid}/frame.jpg")));
        assert_eq!(det["previewUrl"], json!(format!("http://127.0.0.1:5000/tok/media/o/{oid}/source")));
        assert_eq!(det["loops"]["candidates"][0]["preview"], json!(format!("http://127.0.0.1:5000/tok/media/o/{oid}/loop-0.mp4?t=1")));
        // stored relative (1.2 can read it)
        assert_eq!(l.read_original(&oid).unwrap()["loops"]["candidates"][0]["preview"], json!(format!("/media/o/{oid}/loop-0.mp4?t=1")));
        assert_eq!(l.media_path_o(&oid, "source"), Some(src));
        assert!(l.media_path_o(&oid, "../x").is_none());
        assert!(l.media_path_o(&oid, "loop-0.mp4").is_none(), "not made yet");
    }

    #[test]
    fn clean_settings_like_node() {
        let o = json!({ "isImage": false, "duration": 20 });
        let s = clean_settings(&o, &json!({ "start": "19.99", "quality": "55.5", "detail": 30, "blend": 9, "frame": { "zoom": 2 } }), 10.0, &json!({ "on": true, "r": 80 }));
        // node: lib.cleanSettings({isImage:false,duration:20},{start:'19.99',quality:'55.5',detail:30,blend:9,frame:{zoom:2}},10,{on:true,r:80})
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"start":19.95,"length":0.05,"quality":56,"frame":{"zoom":2,"x":0.5,"y":0.5},"corners":{"on":true,"r":80},"detail":30,"blend":1.5,"manualLoop":false}"#
        );
        let img = clean_settings(&json!({ "isImage": true }), &json!({}), 10.0, &Value::Null);
        assert_eq!(
            serde_json::to_string(&img).unwrap(),
            r#"{"start":0,"length":3,"quality":"auto","frame":{"zoom":1,"x":0.5,"y":0.5},"corners":{"on":false,"r":0},"detail":0,"blend":0,"manualLoop":false}"#
        );
    }
}
