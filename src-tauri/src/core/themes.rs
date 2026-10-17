//! 3.0 themes (docs/v3/theme-format.md): every library item is a theme = a base (the MJPEG version stored in the loop slot)
//! plus an optional live layer (widgets the engine draws on top while CoolerScreen runs) and hand-over options.
//! `theme.json` lives in the original's folder (library/<originalId>/theme.json); a missing file means the defaults.
//!
//! "Current theme" = the original whose version is in the loop slot (`on_screen`). The live engine always shows the current
//! theme; `refresh_live_theme` tells it. Applying a theme (`theme_apply`) is one visible job with three steps:
//! 1. make or reuse the base version (with the text/picture items baked in when the theme has `onBase` items),
//! 2. send it to the loop slot (the normal send job; live pauses and resumes around it),
//! 3. switch the live layer to this theme.
use super::app::App;
use super::error::{CoreError, CoreResult};
use super::gallery::{sanitize_name, unique_path};
use super::js::{self, truthy, write_atomic};
use super::scheduler::{JobHandle, Resources, TaskCtx, TaskSpec, Priority};
use super::widget_props;
use crate::live::theme::{Theme, ThemeLive};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

/// theme.json is read-modified-written by the editor, the send hook and the apply job.
static THEME_LOCK: Mutex<()> = Mutex::new(());
/// (theme, version) of the Apply that is writing to the screen: the send hook stores the version for that theme.
static APPLY_TARGET: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

fn target_set(theme: &str, vid: &str) {
    let mut g = APPLY_TARGET.lock().unwrap_or_else(|p| p.into_inner());
    g.retain(|(_, v)| v != vid);
    g.push((theme.to_string(), vid.to_string()));
}
fn target_clear(vid: &str) {
    APPLY_TARGET.lock().unwrap_or_else(|p| p.into_inner()).retain(|(_, v)| v != vid);
}

/// Members of a group sit next to each other in `widgets` (a group at the place of its lowest member); empty groups go.
fn normalize_groups(t: &mut Theme) {
    let mut out = Vec::with_capacity(t.widgets.len());
    let mut done = std::collections::HashSet::new();
    for w in &t.widgets {
        match &w.group {
            Some(g) => {
                if done.insert(g.clone()) {
                    out.extend(t.widgets.iter().filter(|x| x.group.as_ref() == Some(g)).cloned());
                }
            }
            None => out.push(w.clone()),
        }
    }
    t.widgets = out;
    let used: std::collections::HashSet<String> = t.widgets.iter().filter_map(|w| w.group.clone()).collect();
    t.groups.retain(|g| used.contains(&g.id));
    for g in t.groups.iter_mut() {
        g.name = g.name.trim().chars().take(60).collect();
    }
}

const ASSET_EXT: [&str; 6] = ["png", "jpg", "jpeg", "webp", "gif", "bmp"];
const MAX_ASSET_BYTES: usize = 25 * 1024 * 1024;
const MAX_WIDGETS: usize = 40;
/// Apply: step 1 (base) ends here, step 2 (send) ends at `SEND_END`, step 3 is the rest.
const BASE_END: f64 = 0.45;
const SEND_END: f64 = 0.97;

fn gone() -> CoreError {
    CoreError::not_found("This theme is no longer in the library.")
}

fn short_hash(s: &str) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(s.as_bytes());
    h.digest().to_string()[..16].to_string()
}

/// A path inside the theme folder that stays inside it ("assets/logo.png"); anything else is dropped.
/// The engine keeps numbers as f32: round every number to 4 decimals so 0.8 stays 0.8 in theme.json and in the page.
pub(crate) fn tidy(v: Value) -> Value {
    match v {
        Value::Number(n) if n.is_f64() => json!((n.as_f64().unwrap_or(0.0) * 10000.0).round() / 10000.0),
        Value::Array(a) => Value::Array(a.into_iter().map(tidy).collect()),
        Value::Object(o) => Value::Object(o.into_iter().map(|(k, v)| (k, tidy(v))).collect()),
        v => v,
    }
}

fn theme_value(t: &Theme) -> Value {
    tidy(serde_json::to_value(t).unwrap_or(Value::Null))
}

/// The corner radius a version was made with (0 = none).
fn version_radius(v: &Value) -> f64 {
    let c = &super::library::effective_settings(v)["corners"];
    if c["on"] == true { c["r"].as_f64().unwrap_or(0.0) } else { 0.0 }
}

fn safe_rel(rel: &str) -> bool {
    use std::path::Component;
    let p = Path::new(rel);
    !rel.is_empty() && p.components().all(|c| matches!(c, Component::Normal(_)))
}

impl App {
    pub fn theme_dir(&self, oid: &str) -> PathBuf {
        self.lib.o_dir(oid)
    }

    /// theme.json, or the defaults when it is missing or unreadable (a broken file is kept until the next save).
    pub fn load_theme(&self, oid: &str) -> Theme {
        match Theme::load(&self.theme_dir(oid)) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("Theme of {oid}: {e}; using the defaults");
                Theme::default()
            }
        }
    }

    fn save_theme_locked(&self, oid: &str, t: &Theme) -> CoreResult<()> {
        let dir = self.theme_dir(oid);
        std::fs::create_dir_all(&dir)?;
        js::write_json(&dir.join("theme.json"), &theme_value(t))?;
        self.lib.bump();
        Ok(())
    }

    /// The original of the version in the loop slot, if that original still exists.
    ///
    /// A theme's base may be a version of another clip, so the slot record also names the theme it was stored for
    /// (`themeId`); without it (a plain push, an older build) the owner of the version is the theme.
    pub fn current_theme_oid(&self) -> Option<String> {
        let s = self.settings.load();
        let sb = s.slot("standby")?;
        let vid = sb["versionId"].as_str()?.to_string();
        let (owner, _) = self.lib.find_version(&vid)?;
        if let Some(t) = sb["themeId"].as_str().filter(|t| self.lib.read_original(t).is_some()) {
            return Some(t.to_string());
        }
        self.lib.read_original(&owner).map(|_| owner)
    }

    /// What the engine needs: the theme, its folder, the base file and the original clip as a fallback background.
    pub fn theme_live_of(&self, oid: &str, theme: Theme) -> ThemeLive {
        let mut tl = ThemeLive::new(theme.clone(), self.theme_dir(oid));
        // The base may be a version of any clip: its file is the background, and that clip is the fallback.
        let base = theme
            .base_version_id
            .as_deref()
            .and_then(|v| self.lib.find_version(v).filter(|(_, r)| r["status"] == "ready" && !self.over_budget(r)).map(|(o, _)| (o, v.to_string())));
        tl.base_video = base.as_ref().and_then(|(_, v)| self.lib.device_path(v));
        tl.base_fit = base.as_ref().map(|_| theme.base_version_id.as_deref().and_then(|v| self.lib.find_version(v)).map(|(_, r)| super::fit::Fit::from_value(&r["notch"]["fit"])).unwrap_or_default());
        let owner = base.map(|(o, _)| o).unwrap_or_else(|| oid.to_string());
        tl.theme.corners = Some(self.corner_radius_of(&theme) as f32); // the engine gets the radius in effect (theme or default)
        let o = self.lib.read_original(&owner);
        tl.fallback_video = o.as_ref().filter(|o| !truthy(o.get("isImage"))).and_then(|o| self.lib.source_path(o));
        tl
    }

    /// Tell the live engine which theme to show: the current one, or today's Standard layout when nothing was applied yet.
    pub fn refresh_live_theme(&self) {
        let tl = match self.current_theme_oid() {
            Some(oid) => {
                let t = self.load_theme(&oid);
                self.theme_live_of(&oid, t)
            }
            None => ThemeLive::new(Theme::standard_preset(), PathBuf::new()),
        };
        self.live.set_theme(tl);
    }

    /// A version bigger than the screen can hold (the loop budget): never a base, never sent.
    pub fn over_budget(&self, v: &Value) -> bool {
        v["result"]["size"].as_f64().is_some_and(|x| x > self.lib.context().budget)
    }

    /// The corner radius in px that applies to a theme: its own, or the default from Settings (0 = none).
    pub fn corner_radius_of(&self, t: &Theme) -> f64 {
        t.corners.map(|c| (c as f64).round()).unwrap_or_else(|| self.settings.load().corner_radius()).clamp(0.0, 300.0)
    }

    /// Fingerprint of what the baked items look like (None = nothing is baked into the base).
    fn overlay_key(&self, oid: &str, t: &Theme) -> Option<String> {
        if !t.has_base_items() {
            return None;
        }
        let mut sig = t.base_signature();
        for w in t.widgets.iter().filter(|w| w.on_base && w.kind == "image") {
            if let Ok(m) = std::fs::metadata(self.theme_dir(oid).join(w.image().file)) {
                sig.push_str(&format!("|{}|{:?}", m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())));
            }
        }
        Some(short_hash(&sig))
    }

    /// The editor's view of a theme.
    pub fn theme_view(&self, oid: &str) -> CoreResult<Value> {
        let o = self.lib.read_original(oid).ok_or_else(gone)?;
        let t = self.load_theme(oid);
        let current = self.current_theme_oid().as_deref() == Some(oid);
        let base = t.base_version_id.as_deref().and_then(|v| self.lib.find_version(v));
        let key = self.overlay_key(oid, &t);
        // The stored base was made without (or with other) baked items than the theme has now.
        let standby = self.settings.load().slot("standby").and_then(|s| s["versionId"].as_str().map(String::from));
        // A base was chosen (Base settings) that is not what the screen holds yet.
        let unstored = current && base.is_some() && standby != t.base_version_id;
        // "Stale" = the picture inputs differ from the stored base: screen fit with shift, notch, corner radius, baked items (a different base is `unstored`). Live-only things (widgets and props, live layer, hand-over, guides, gap, weather) never count.
        let fit_key = self.lib.context().notch_key;
        let radius = self.corner_radius_of(&t);
        let stale = current && !unstored && base.as_ref().is_some_and(|(_, v)| v["overlay"]["key"].as_str().map(String::from) != key || (version_radius(v) - radius).abs() > 0.5 || super::mask::base_key(v["notchKey"].as_str()) != fit_key);
        let owner = base.as_ref().and_then(|(boid, _)| self.lib.read_original(boid).map(|bo| json!({ "originalId": boid, "name": bo["name"] })));
        let kinds: Vec<&str> = t.widgets.iter().map(|w| w.kind.as_str()).collect();
        Ok(json!({
            "originalId": oid, "name": o["name"], "theme": theme_value(&t),
            "current": current, "baseVersionId": t.base_version_id, "baseExists": base.is_some(),
            "cornersDefault": self.settings.load().corner_radius(), "cornersEffective": radius,
            "guides": super::settings::guides_value(&crate::live::theme::Guides::from_value(&self.settings.load().screen_guides()).effective(&t.guides_offset)),
            "baseStale": stale, "baseUnstored": unstored, "baseOwner": owner, "hasBaseItems": t.has_base_items(), "widgetKinds": kinds,
            "canPreview": self.lib.source_path(&o).is_some() || base.is_some(),
        }))
    }

    pub fn theme_get(&self, oid: &str) -> CoreResult<Value> {
        self.theme_view(oid)
    }

    /// Parse and clean a theme sent by the UI (the stored base version is never taken from the UI).
    fn clean_theme(&self, oid: &str, v: &Value) -> CoreResult<Theme> {
        if !v.is_object() {
            return Err(CoreError::new("The theme is missing."));
        }
        let mut t = Theme::from_json(&v.to_string()).map_err(CoreError::new)?;
        if t.widgets.len() > MAX_WIDGETS {
            return Err(CoreError::new(format!("A theme can hold at most {MAX_WIDGETS} items.")));
        }
        if let Some(w) = t.widgets.iter().find(|w| !widget_props::TYPES.contains(&w.kind.as_str())) {
            return Err(CoreError::new(format!("Unknown item type \"{}\".", w.kind)));
        }
        for w in t.widgets.iter_mut().filter(|w| w.kind == "image") {
            // Pictures live in the theme's own folder; a path that leaves it is dropped.
            let f = w.image().file;
            if !f.is_empty() && !safe_rel(&f) {
                if let Some(o) = w.props.as_object_mut() {
                    o.insert("file".into(), json!(""));
                }
            }
        }
        for w in t.widgets.iter_mut() {
            widget_props::clean(&w.kind, &mut w.props); // sensor, date, weather, visualizer, slideshow, info (and the clock style)
        }
        t.base_version_id = self.load_theme(oid).base_version_id;
        normalize_groups(&mut t);
        Ok(t)
    }

    /// Use a version of ANY clip as this theme's base. Nothing is sent to the screen: the editor then offers to store it.
    pub fn theme_set_base(&self, oid: &str, vid: &str) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(gone)?;
        let (_, v) = self.lib.find_version(vid).ok_or_else(|| CoreError::not_found("This version no longer exists."))?;
        if v["status"] != "ready" {
            return Err(CoreError::new("That version is not ready yet."));
        }
        {
            let _g = THEME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let mut t = self.load_theme(oid);
            t.base_version_id = Some(vid.to_string());
            self.save_theme_locked(oid, &t)?;
        }
        if self.current_theme_oid().as_deref() == Some(oid) {
            self.refresh_live_theme();
        }
        self.theme_card_soon(oid);
        self.theme_view(oid)
    }

    /// Validate and save theme.json; when this is the current theme the live screen follows at once.
    pub fn theme_set(&self, oid: &str, theme: &Value) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(gone)?;
        let _g = THEME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let t = self.clean_theme(oid, theme)?;
        self.save_theme_locked(oid, &t)?;
        drop(_g);
        if self.current_theme_oid().as_deref() == Some(oid) {
            self.refresh_live_theme();
        }
        self.theme_card_soon(oid);
        self.theme_view(oid)
    }

    /// The Standard preset (clock, music, stats: today's layout), as the editor's button needs it.
    pub fn theme_standard(&self) -> Value {
        theme_value(&Theme::standard_preset())
    }

    /// One rendered frame of the theme for the editor: `theme` is the unsaved edit (default: the saved one).
    pub fn theme_preview(&self, oid: &str, theme: Option<&Value>, with_live: bool, w: u32, h: u32) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(gone)?;
        let t = match theme.filter(|t| t.is_object()) {
            Some(v) => self.clean_theme(oid, v)?,
            None => self.load_theme(oid),
        };
        let (w, h) = (w.clamp(160, 2400), h.clamp(72, 1080));
        let png = crate::live::preview::render_preview(&self.theme_live_of(oid, t), w, h, with_live).map_err(CoreError::new)?;
        let name = if with_live { "theme-preview.png" } else { "theme-preview-base.png" };
        write_atomic(&self.theme_dir(oid).join(name), &png)?;
        Ok(json!({ "url": self.lib.url(&format!("/media/o/{oid}/{name}?t={}", super::scheduler::now_ms())), "withLive": with_live, "w": w, "h": h }))
    }

    /// The small picture My themes shows (base + live layer). Rendered in the background after an edit or an apply.
    pub fn theme_card_soon(&self, oid: &str) {
        let Some(app) = self.me.lock().unwrap().upgrade() else { return };
        let oid = oid.to_string();
        std::thread::spawn(move || {
            let t = app.load_theme(&oid);
            let card = app.theme_dir(&oid).join("theme-card.png");
            if t.widgets.is_empty() {
                if card.exists() {
                    let _ = std::fs::remove_file(&card);
                    app.lib.bump();
                }
                return;
            }
            if app.lib.read_original(&oid).is_none() {
                return;
            }
            if let Ok(png) = crate::live::preview::render_preview(&app.theme_live_of(&oid, t), 640, 288, true) {
                if write_atomic(&card, &png).is_ok() {
                    app.lib.bump();
                }
            }
        });
    }

    // ---------- pictures for image items ----------

    fn store_asset(&self, oid: &str, name: &str, bytes: &[u8]) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(gone)?;
        let p = Path::new(name);
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !ASSET_EXT.contains(&ext.as_str()) {
            return Err(CoreError::new("Choose a picture file: PNG, JPG, WEBP, GIF or BMP."));
        }
        if bytes.is_empty() || bytes.len() > MAX_ASSET_BYTES {
            return Err(CoreError::new("The picture is empty or larger than 25 MB."));
        }
        let dir = self.theme_dir(oid).join("assets");
        std::fs::create_dir_all(&dir)?;
        let base = sanitize_name(&p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
        let target = unique_path(&dir, &base, &format!(".{ext}"));
        write_atomic(&target, bytes)?;
        let file = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        Ok(json!({ "file": format!("assets/{file}") }))
    }

    /// Copy a picture from a path on this PC into the theme's folder.
    pub fn theme_asset_add_path(&self, oid: &str, path: &str) -> CoreResult<Value> {
        let p = PathBuf::from(path);
        if !p.is_absolute() || !p.is_file() {
            return Err(CoreError::new(format!("File not found: {path}")));
        }
        let len = std::fs::metadata(&p)?.len() as usize;
        if len > MAX_ASSET_BYTES {
            return Err(CoreError::new("The picture is larger than 25 MB."));
        }
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        self.store_asset(oid, &name, &std::fs::read(&p)?)
    }

    /// The same from the bytes of a file chosen in the page.
    pub fn theme_asset_add_bytes(&self, oid: &str, name: &str, bytes: &[u8]) -> CoreResult<Value> {
        self.store_asset(oid, name, bytes)
    }

    // ---------- after a base was stored ----------

    /// A version is now in the loop slot: it is the base of the theme being applied (or, for a plain push, of the clip it
    /// belongs to), the slot remembers that theme, and the live layer switches to it.
    pub(crate) fn after_base_sent(&self, vid: &str) {
        let target = APPLY_TARGET.lock().unwrap_or_else(|p| p.into_inner()).iter().find(|(_, v)| v == vid).map(|(t, _)| t.clone());
        let Some(oid) = target.or_else(|| self.lib.find_version(vid).map(|(o, _)| o)) else { return };
        if self.lib.read_original(&oid).is_some() {
            let _g = THEME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let mut t = self.load_theme(&oid);
            if t.base_version_id.as_deref() != Some(vid) {
                t.base_version_id = Some(vid.to_string());
                if let Err(e) = self.save_theme_locked(&oid, &t) {
                    eprintln!("Could not save the theme's base: {}", e.message);
                }
            }
            let _ = self.settings.patch_slot("standby", &json!({ "themeId": oid }));
        }
        self.refresh_live_theme();
        self.theme_card_soon(&oid);
    }

    // ---------- apply ----------

    /// The settings the base is made with: the clip's tuning if it has one, else its best loop, else the defaults.
    fn apply_settings(&self, o: &Value) -> Value {
        if o.get("draft").is_some_and(|d| d.is_object()) {
            return o["draft"].clone();
        }
        if !truthy(o.get("isImage")) && super::library::loops_current(o) {
            if let Some(c) = o["loops"]["candidates"].get(0) {
                let (sf, of) = (o["fps"].as_f64().unwrap_or(25.0), super::library::DEVICE_FPS);
                return json!({
                    "start": c["startFrame"].as_f64().map(|f| f / sf).or(c["start"].as_f64()).unwrap_or(0.0),
                    "length": c["frames"].as_f64().map(|f| f / of).or(c["length"].as_f64()).unwrap_or(0.0),
                    "blend": c["blend"].as_f64().unwrap_or(0.0),
                });
            }
        }
        json!({})
    }

    /// The text/picture items as a PNG next to the theme (None when nothing is baked in).
    fn make_overlay(&self, oid: &str, t: &Theme) -> CoreResult<Option<Value>> {
        let Some(key) = self.overlay_key(oid, t) else { return Ok(None) };
        let dir = self.theme_dir(oid).join("theme");
        let file = dir.join(format!("overlay-{key}.png"));
        if !file.is_file() {
            let png = crate::live::preview::render_base_overlay(&self.theme_live_of(oid, t.clone()))
                .map_err(CoreError::new)?
                .ok_or_else(|| CoreError::new("The text and picture items could not be drawn."))?;
            std::fs::create_dir_all(&dir)?;
            write_atomic(&file, &png)?;
        }
        Ok(Some(json!({ "key": key, "file": file.to_string_lossy() })))
    }

    /// Wait for a sub-job while mapping its progress into `from..to`; a stop request cancels it and ends here.
    fn wait_sub(&self, ctx: &TaskCtx, h: &JobHandle, (from, to): (f64, f64), step: u32, what: &str) -> CoreResult<Value> {
        loop {
            if ctx.is_cancelled() {
                // A queued or converting job stops at once. A screen upload that is already running is never cut off
                // halfway (the screen would be left with a broken picture): it finishes, and then so does the apply.
                self.sched.cancel(&h.id);
                return match h.wait() {
                    Ok(v) => Ok(v),
                    Err(_) => Err(ctx.check().err().unwrap_or_else(CoreError::cancelled)),
                };
            }
            if let Some(r) = h.wait_timeout(Duration::from_millis(100)) {
                return r;
            }
            if let Some(j) = self.sched.job(&h.id) {
                let text = if j.status == "waiting" { "Waiting for the screen to be free".to_string() } else { what.to_string() };
                ctx.progress(from + (to - from) * j.progress.clamp(0.0, 1.0), &format!("Step {step} of 3: {text}"));
            }
        }
    }

    /// Wait until an existing version is ready (it was queued earlier by another action).
    fn wait_version(&self, ctx: &TaskCtx, vid: &str) -> CoreResult<()> {
        let mut idle_since: Option<std::time::Instant> = None;
        loop {
            ctx.check()?;
            let (_, v) = self.lib.find_version(vid).ok_or_else(|| CoreError::new("The base picture was deleted."))?;
            match v["status"].as_str() {
                Some("ready") => return Ok(()),
                Some("error") => return Err(CoreError::new(v["error"].as_str().unwrap_or("The base picture could not be made.").to_string())),
                _ => {
                    let job = self.sched.activity(0).into_iter().find(|j| j.version_id.as_deref() == Some(vid) && j.kind == "version" && !j.state.finished());
                    // queued/converting with no job behind it: it was interrupted, never finish waiting for it
                    if job.is_none() && idle_since.get_or_insert_with(std::time::Instant::now).elapsed() > Duration::from_secs(3) {
                        return Err(CoreError::new("The base picture was interrupted. Apply the theme again."));
                    }
                    if job.is_some() {
                        idle_since = None;
                    }
                    let p = job.map(|j| j.progress).unwrap_or(0.0);
                    ctx.progress(BASE_END * p, "Step 1 of 3: Preparing the picture");
                    ctx.sleep(Duration::from_millis(100))?;
                }
            }
        }
    }

    fn run_apply(&self, ctx: &TaskCtx, oid: &str) -> CoreResult<Value> {
        let o = self.lib.read_original(oid).ok_or_else(gone)?;
        let t = self.load_theme(oid);
        ctx.progress(0.0, "Step 1 of 3: Preparing the picture");
        // 1. the base: the version with the clip's settings (and the baked items), reused when it already exists
        let overlay = self.make_overlay(oid, &t)?;
        let want_key = overlay.as_ref().and_then(|ov| ov["key"].as_str().map(String::from));
        // A base was chosen already (maybe from another clip): keep its picture settings; it is re-made only when the baked
        // items differ. Otherwise the base comes from this clip's own tuning.
        let existing = t.base_version_id.as_deref().and_then(|v| self.lib.find_version(v)).filter(|(_, v)| v["status"] == "ready" && !self.over_budget(v));
        let (src_oid, mut body) = match &existing {
            Some((boid, bv)) => {
                let e = super::library::effective_settings(bv);
                let mut b = json!({});
                for k in ["start", "length", "quality", "frame", "corners", "detail", "blend"] {
                    b[k] = e.get(k).cloned().unwrap_or(Value::Null);
                }
                (boid.clone(), b)
            }
            None => (oid.to_string(), self.apply_settings(&o)),
        };
        if let Some(ov) = &overlay {
            body["overlay"] = ov.clone();
        }
        // Always "best that fits": the base must fit the screen's measured room whatever quality an older version was made with
        // (a fixed quality plus the screen fit once made a base larger than the flash).
        body["quality"] = json!("auto");
        // rounded corners: the theme's radius, or the default from Settings (0 = none)
        let radius = self.corner_radius_of(&t);
        body["corners"] = json!({ "on": radius > 0.0, "r": radius });
        let fit_key = self.lib.context().notch_key;
        let reuse = existing.as_ref().filter(|(_, bv)| bv["overlay"]["key"].as_str().map(String::from) == want_key && (version_radius(bv) - radius).abs() <= 0.5 && super::mask::base_key(bv["notchKey"].as_str()) == fit_key).map(|(_, bv)| bv["id"].as_str().unwrap_or("").to_string());
        let created = if reuse.is_some() { Err(CoreError::coded("duplicate", "reuse").with("versionId", json!(reuse))) } else { self.lib.create_version(&src_oid, &body) };
        let oid_for_version = src_oid.as_str();
        let vid = match created {
            Ok(v) => {
                let vid = v["id"].as_str().unwrap_or("").to_string();
                let h = self.version_handle(oid_for_version, &v);
                if let Err(e) = self.wait_sub(ctx, &h, (0.0, BASE_END), 1, "Preparing the picture") {
                    self.lib.delete_version(&vid); // stopped or failed half-way: no leftover half-made version
                    return Err(e);
                }
                vid
            }
            Err(e) if e.code_is("duplicate") => {
                let vid = e.extra["versionId"].as_str().unwrap_or("").to_string();
                self.wait_version(ctx, &vid)?;
                vid
            }
            Err(e) => return Err(e),
        };
        ctx.check()?;
        // 2. send it to the loop slot (live pauses while the screen is written)
        ctx.progress(BASE_END, "Step 2 of 3: Sending to the screen");
        target_set(oid, &vid);
        let sent = self.send_handle(&vid).and_then(|h| self.wait_sub(ctx, &h, (BASE_END, SEND_END), 2, "Sending to the screen"));
        if sent.is_err() {
            target_clear(&vid);
        }
        sent?;
        // 3. switch the live layer
        ctx.progress(SEND_END, "Step 3 of 3: Switching the live layer");
        self.after_base_sent(&vid);
        target_clear(&vid);
        ctx.progress(1.0, "Done");
        Ok(json!({ "originalId": oid, "versionId": vid }))
    }

    /// Apply a theme: base picture, store it on the screen, switch the live layer. Returns `{ jobId }`.
    pub fn theme_apply(&self, oid: &str) -> CoreResult<Value> {
        let o = self.lib.read_original(oid).ok_or_else(gone)?;
        if self.lib.source_path(&o).is_none() && self.load_theme(oid).base_version_id.and_then(|v| self.lib.find_version(&v)).is_none() {
            return Err(CoreError::new("The original file is missing, so this theme can't be applied."));
        }
        let name = o["name"].as_str().unwrap_or("theme").to_string();
        let spec = TaskSpec::new("apply", format!("Applying \"{name}\""), Resources::none()).original(oid).priority(Priority::User).visible();
        let (app, oid2) = (self.arc(), oid.to_string());
        let h = self.sched.submit(spec, move |ctx| app.run_apply(ctx, &oid2));
        Ok(json!({ "jobId": h.id }))
    }

    /// For the list: which kinds of items a theme has, and its card picture.
    pub(crate) fn theme_summary(&self, oid: &str, current: Option<&str>) -> Value {
        let t = self.load_theme(oid);
        let card = self.theme_dir(oid).join("theme-card.png");
        let kinds: Vec<&str> = t.widgets.iter().map(|w| w.kind.as_str()).collect();
        json!({
            "widgets": kinds, "current": current == Some(oid),
            "card": if card.is_file() && !t.widgets.is_empty() { json!(self.lib.url(&format!("/media/o/{oid}/theme-card.png?t={}", std::fs::metadata(&card).ok().and_then(|m| m.modified().ok()).and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0)))) } else { Value::Null },
        })
    }
}

