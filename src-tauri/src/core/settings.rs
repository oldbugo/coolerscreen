//! settings.json, exactly as 1.2's `src/settings.js` (same defaults, same sanitising, same file), plus the
//! measured-capacity slot budgets of `src/slots.js`.
use super::js::{clamp_num, num, round};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Settings are always handled as the sanitised JSON object (field order and number formatting as 1.2 writes them).
#[derive(Clone, Debug, PartialEq)]
pub struct Settings(pub Value);

impl Settings {
    pub fn mask(&self) -> &Value {
        &self.0["mask"]
    }
    /// Default corner radius in px on the 2400x1080 canvas; 0 = no rounded corners (stored as { on, r }, like 2.0).
    pub fn corner_radius(&self) -> f64 {
        let c = &self.0["mask"]["corners"];
        if c["on"] == true { c["r"].as_f64().unwrap_or(0.0) } else { 0.0 }
    }
    /// The global screen guides (left, right, curve band), always a full object.
    pub fn screen_guides(&self) -> Value {
        guides_value(&crate::live::theme::Guides::from_value(self.0.get("screenGuides").unwrap_or(&Value::Null)))
    }
    /// How many screen pixels the panel hides at each edge (default none).
    pub fn screen_fit(&self) -> super::fit::Fit {
        super::fit::Fit::from_value(self.0.get("screenFit").unwrap_or(&Value::Null))
    }
    /// Weather: the place the user picked (None until then) and the default temperature unit ("C" or "F").
    pub fn weather_value(&self) -> Value {
        let w = self.0.get("weather").unwrap_or(&Value::Null);
        json!({ "place": w.get("place").and_then(super::weather::clean_place).unwrap_or(Value::Null), "unit": if w.get("unit").and_then(Value::as_str) == Some("F") { "F" } else { "C" } })
    }
    /// Gentle snapping in the theme editor (default on).
    pub fn editor_snap(&self) -> bool {
        self.0["editor"]["snap"] != false
    }
    pub fn max_seconds(&self) -> f64 {
        self.0["maxSeconds"].as_f64().unwrap_or(10.0)
    }
    pub fn gallery_dir(&self) -> &str {
        self.0["galleryDir"].as_str().unwrap_or("")
    }
    pub fn total_budget_mb(&self) -> f64 {
        self.0["totalBudgetMB"].as_f64().unwrap_or(9.5)
    }
    pub fn upload_rate(&self) -> Option<f64> {
        self.0["uploadRate"].as_f64()
    }
    pub fn capacity(&self) -> Option<(f64, String)> {
        let c = self.0.get("capacity")?;
        Some((c["bytes"].as_f64()?, c["at"].as_str().unwrap_or("").to_string()))
    }
    pub fn slot(&self, name: &str) -> Option<&Value> {
        self.0["slots"].get(name).filter(|v| !v.is_null())
    }
    pub fn sources(&self) -> &Value {
        &self.0["sources"]
    }
    /// The 3.0 live screen settings (defaults when absent).
    pub fn live(&self) -> LiveSettings {
        LiveSettings::from_value(self.0.get("live"))
    }
    /// Run the live companion inside the app (default off).
    pub fn live_in_app(&self) -> bool {
        self.live().in_app
    }
    /// With the live companion in the app: destroy the window (and its WebView) when it is closed to the tray (default on).
    pub fn live_destroy_window(&self) -> bool {
        self.live().destroy_window
    }
    /// The theme Live shows when the user chose one with "Use this theme" that is not the Embedded one (None = follows Embedded).
    pub fn live_theme(&self) -> Option<String> {
        self.0.get("liveTheme").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from)
    }
}

/// 3.0 live screen (all off or "like 2.0" by default; `destroy_window` is on once live is on, by the user's decision).
#[derive(Clone, Debug, PartialEq)]
pub struct LiveSettings {
    pub in_app: bool,
    pub destroy_window: bool,
    /// Gallery clip (original id) used as the animated background; empty = animated gradient.
    pub background: String,
    pub show_seconds: bool,
    pub music: bool,
    pub screen_off_on_shutdown: bool,
    /// "loop" (default: the screen plays its loop clip), "keep" (live keeps running) or "off" (screen off).
    pub on_sleep: String,
    pub on_lock: String,
    /// Draw the live frames-per-second counter on the screen (off by default).
    pub fps_overlay: bool,
}

impl Default for LiveSettings {
    fn default() -> Self {
        LiveSettings { in_app: false, destroy_window: true, background: String::new(), show_seconds: false, music: true, screen_off_on_shutdown: false, on_sleep: "loop".into(), on_lock: "loop".into(), fps_overlay: false }
    }
}

impl LiveSettings {
    pub fn from_value(v: Option<&Value>) -> LiveSettings {
        let d = LiveSettings::default();
        let b = |k: &str, dflt: bool| v.and_then(|x| x.get(k)).and_then(Value::as_bool).unwrap_or(dflt);
        let mode = |k: &str, dflt: &str| match v.and_then(|x| x.get(k)).and_then(Value::as_str) {
            Some(m) if ["loop", "keep", "off"].contains(&m) => m.to_string(),
            _ => dflt.to_string(),
        };
        LiveSettings {
            in_app: b("inApp", d.in_app),
            destroy_window: b("destroyWindow", d.destroy_window),
            background: str80(v.and_then(|x| x.get("background")), 80).unwrap_or_default(),
            show_seconds: b("showSeconds", d.show_seconds),
            music: b("music", d.music),
            screen_off_on_shutdown: b("screenOffOnShutdown", d.screen_off_on_shutdown),
            on_sleep: mode("onSleep", &d.on_sleep),
            on_lock: mode("onLock", &d.on_lock),
            fps_overlay: b("fpsOverlay", d.fps_overlay),
        }
    }
    /// Only the keys that differ from the defaults.
    pub fn to_min_value(&self) -> Value {
        let d = LiveSettings::default();
        let mut m = Map::new();
        if self.in_app != d.in_app { m.insert("inApp".into(), json!(self.in_app)); }
        if self.destroy_window != d.destroy_window { m.insert("destroyWindow".into(), json!(self.destroy_window)); }
        if self.background != d.background { m.insert("background".into(), json!(self.background)); }
        if self.show_seconds != d.show_seconds { m.insert("showSeconds".into(), json!(self.show_seconds)); }
        if self.music != d.music { m.insert("music".into(), json!(self.music)); }
        if self.screen_off_on_shutdown != d.screen_off_on_shutdown { m.insert("screenOffOnShutdown".into(), json!(self.screen_off_on_shutdown)); }
        if self.on_sleep != d.on_sleep { m.insert("onSleep".into(), json!(self.on_sleep)); }
        if self.on_lock != d.on_lock { m.insert("onLock".into(), json!(self.on_lock)); }
        if self.fps_overlay != d.fps_overlay { m.insert("fpsOverlay".into(), json!(self.fps_overlay)); }
        Value::Object(m)
    }
    pub fn to_full_value(&self) -> Value {
        json!({ "inApp": self.in_app, "destroyWindow": self.destroy_window, "background": self.background, "showSeconds": self.show_seconds, "music": self.music,
            "screenOffOnShutdown": self.screen_off_on_shutdown, "onSleep": self.on_sleep, "onLock": self.on_lock, "fpsOverlay": self.fps_overlay })
    }
}

pub fn defaults() -> Value {
    json!({
        "mask": {
            "mode": "band",
            "circle": { "cx": 45, "cy": 540, "r": 45, "feather": 0 },
            "band": { "edge": "left", "px": 92 },
            "corners": { "on": false, "r": 80 }
        },
        "maxSeconds": 10,
        "galleryDir": "",
        "totalBudgetMB": 9.5,
        "uploadRate": null,
        "capacity": null,
        "slots": { "startup": null, "standby": null }
    })
}

fn str80(v: Option<&Value>, n: usize) -> Option<String> {
    match v {
        Some(Value::String(s)) if !s.is_empty() => Some(s.chars().take(n).collect()),
        _ => None,
    }
}

/// Guides as JSON with tidy numbers (the engine keeps f32).
pub fn guides_value(g: &crate::live::theme::Guides) -> Value {
    super::themes::tidy(serde_json::to_value(g).unwrap_or(Value::Null))
}

fn clean_slot(v: Option<&Value>) -> Value {
    let Some(v) = v.filter(|v| v.is_object()) else { return Value::Null };
    let size = super::js::to_num(v.get("size"));
    if !size.is_finite() || size < 0.0 {
        return Value::Null;
    }
    let name = match v.get("name") {
        Some(Value::String(s)) => s.chars().take(80).collect::<String>(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string().chars().take(80).collect(),
    };
    let at = match v.get("at") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(o) => o.to_string(),
    };
    let mut r = Map::new();
    r.insert("name".into(), json!(name));
    r.insert("size".into(), num(size));
    r.insert("at".into(), json!(at));
    for k in ["originalId", "versionId", "label", "splash", "themeId"] {
        if let Some(s) = str80(v.get(k), 80) {
            r.insert(k.into(), json!(s));
        }
    }
    if super::js::truthy(v.get("incomplete")) {
        r.insert("incomplete".into(), json!(true));
    }
    Value::Object(r)
}

fn is_source_id(k: &str) -> bool {
    !k.is_empty() && k.len() <= 60 && k.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

fn clean_sources(v: Option<&Value>) -> Value {
    let empty = Value::Object(Map::new());
    let v = v.filter(|v| v.is_object()).unwrap_or(&empty);
    let mut enabled = Map::new();
    if let Some(m) = v.get("enabled").and_then(Value::as_object) {
        for (k, on) in m {
            if is_source_id(k) && on == &Value::Bool(true) {
                enabled.insert(k.clone(), json!(true));
            }
        }
    }
    let mut options = Map::new();
    if let Some(m) = v.get("options").and_then(Value::as_object) {
        for (k, o) in m {
            if !is_source_id(k) || !o.is_object() {
                continue;
            }
            let folders: Vec<Value> = o
                .get("folders")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .filter(|f| !f.trim().is_empty())
                        .map(|f| json!(f.trim().chars().take(260).collect::<String>()))
                        .take(10)
                        .collect()
                })
                .unwrap_or_default();
            options.insert(k.clone(), json!({ "folders": folders }));
        }
    }
    let ticked: Vec<Value> = match v.get("ticked").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .filter_map(Value::as_str)
            .filter(|k| *k == "mine" || enabled.contains_key(*k))
            .take(30)
            .map(|k| json!(k))
            .collect(),
        None => vec![json!("mine")],
    };
    json!({ "enabled": enabled, "options": options, "ticked": ticked })
}

/// 1.2 `sanitize`.
pub fn sanitize(s: &Value) -> Settings {
    let d = defaults();
    let null = Value::Null;
    let m = s.get("mask").unwrap_or(&null);
    let c = m.get("circle").unwrap_or(&null);
    let b = m.get("band").unwrap_or(&null);
    let k = m.get("corners").unwrap_or(&null);
    let pick = |v: Option<&Value>, ok: &[&str], dflt: &str| -> Value {
        match v.and_then(Value::as_str) {
            Some(x) if ok.contains(&x) => json!(x),
            _ => json!(dflt),
        }
    };
    let dm = &d["mask"];
    let mask = json!({
        "mode": pick(m.get("mode"), &["circle", "band", "off"], "band"),
        "circle": {
            "cx": num(clamp_num(c.get("cx"), 45.0, 0.0, 2400.0)),
            "cy": num(clamp_num(c.get("cy"), 540.0, 0.0, 1080.0)),
            "r": num(clamp_num(c.get("r"), 45.0, 0.0, 600.0)),
            "feather": num(clamp_num(c.get("feather"), 0.0, 0.0, 200.0)),
        },
        "band": {
            "edge": pick(b.get("edge"), &["left", "right", "top", "bottom"], "left"),
            "px": num(clamp_num(b.get("px"), 92.0, 0.0, 1000.0)),
        },
        "corners": { "on": k.get("on") == Some(&Value::Bool(true)), "r": num(round(clamp_num(k.get("r"), dm["corners"]["r"].as_f64().unwrap(), 0.0, 300.0))) },
    });
    let gallery_dir = match s.get("galleryDir") {
        Some(Value::String(g)) => g.trim().chars().take(240).collect::<String>(),
        _ => String::new(),
    };
    let upload_rate = match s.get("uploadRate") {
        None | Some(Value::Null) => Value::Null,
        v => {
            let x = super::js::to_num(v);
            if x.is_finite() {
                num(x.clamp(5000.0, 5e6))
            } else {
                Value::Null
            }
        }
    };
    let capacity = match s.get("capacity") {
        Some(cap) if super::js::to_num(cap.get("bytes")) > 1e6 => {
            let at = match cap.get("at") {
                Some(Value::String(a)) => a.clone(),
                None | Some(Value::Null) => String::new(),
                Some(o) => o.to_string(),
            };
            json!({ "bytes": num(round(clamp_num(cap.get("bytes"), 0.0, 1e6, 1e8))), "at": at })
        }
        _ => Value::Null,
    };
    let slots = s.get("slots");
    let mut out = json!({
        "mask": mask,
        "maxSeconds": num(clamp_num(s.get("maxSeconds"), 10.0, 1.0, 120.0)),
        "galleryDir": gallery_dir,
        "totalBudgetMB": num(clamp_num(s.get("totalBudgetMB"), 9.5, 1.0, 10.0)),
        "uploadRate": upload_rate,
        "capacity": capacity,
        "slots": { "startup": clean_slot(slots.and_then(|x| x.get("startup"))), "standby": clean_slot(slots.and_then(|x| x.get("standby"))) },
        "sources": clean_sources(s.get("sources")),
    });
    // 3.0: screen guides and the editor's snap switch, written only when they differ from the defaults.
    let guides = crate::live::theme::Guides::from_value(s.get("screenGuides").unwrap_or(&Value::Null));
    if guides != crate::live::theme::Guides::default() {
        out["screenGuides"] = guides_value(&guides);
    }
    // 3.0 screen fit (pixels the panel hides at each edge): written only when something is hidden.
    let fit = super::fit::Fit::from_value(s.get("screenFit").unwrap_or(&Value::Null));
    if !fit.is_zero() {
        out["screenFit"] = fit.to_value();
    }
    // 3.0 weather (v1.6): the chosen place and the default unit, written only when set.
    let w = Settings(s.clone()).weather_value();
    if !w["place"].is_null() || w["unit"] == "F" {
        let mut o = json!({});
        if !w["place"].is_null() {
            o["place"] = w["place"].clone();
        }
        if w["unit"] == "F" {
            o["unit"] = json!("F");
        }
        out["weather"] = o;
    }
    if s.get("editor").and_then(|e| e.get("snap")) == Some(&Value::Bool(false)) {
        out["editor"] = json!({ "snap": false });
    }
    // 3.0 prototype B: written only when switched on, so a settings.json that never used it stays identical to 2.0.
    // 3.0 live screen: written only when something differs from the defaults, so a settings.json that never used it stays
    // identical to 2.0.
    let live = LiveSettings::from_value(s.get("live")).to_min_value();
    if !live.as_object().is_none_or(|o| o.is_empty()) {
        out["live"] = live;
    }
    // 4.0 (C2): what the screen's free space was after our last store, and an open difference; written only when set.
    if let Some(f) = s.get("screenBaseline").and_then(|b| b.get("free")).and_then(Value::as_f64).filter(|f| f.is_finite() && *f >= 0.0) {
        out["screenBaseline"] = json!({ "free": num(f), "at": str80(s["screenBaseline"].get("at"), 40).unwrap_or_default() });
    }
    if let Some(c) = s.get("screenChanged").filter(|c| c.is_object()) {
        let n = |k: &str| c.get(k).and_then(Value::as_f64).filter(|f| f.is_finite() && *f >= 0.0);
        if let (Some(was), Some(now)) = (n("was"), n("now")) {
            out["screenChanged"] = json!({ "was": num(was), "now": num(now), "blockSize": num(n("blockSize").unwrap_or(512.0)), "at": str80(c.get("at"), 40).unwrap_or_default() });
        }
    }
    // 3.0 (v1.8): the live theme when it is not the Embedded one; written only when set.
    if let Some(t) = str80(s.get("liveTheme"), 80) {
        out["liveTheme"] = json!(t);
    }
    Settings(out)
}

/// Reads and writes settings.json; read-modify-write goes through `update` so concurrent tasks don't lose changes.
pub struct SettingsStore {
    file: PathBuf,
    lock: Mutex<()>,
    on_change: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    /// The last settings read or written successfully: used when the file exists but cannot be parsed, so a bad
    /// read never turns into defaults (a default gallery folder would mark the whole library missing).
    last_good: Mutex<Option<Settings>>,
}

impl SettingsStore {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        SettingsStore { file: file.into(), lock: Mutex::new(()), on_change: Mutex::new(None), last_good: Mutex::new(None) }
    }
    pub fn file(&self) -> &Path {
        &self.file
    }
    pub fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_change.lock().unwrap() = Some(Box::new(f));
    }

    /// Current settings. Missing file: defaults. Unreadable/unparsable file: retried briefly, then the last good
    /// settings (or, on the very first read, defaults after keeping a copy of the bad file as settings.json.bad).
    pub fn load(&self) -> Settings {
        let mut tries = 0;
        loop {
            match super::js::read_text(&self.file) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return sanitize(&json!({})),
                Ok(t) => {
                    if let Ok(v) = serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')) {
                        let s = sanitize(&v);
                        *self.last_good.lock().unwrap() = Some(s.clone());
                        return s;
                    }
                }
                Err(_) => {}
            }
            tries += 1;
            if tries >= 5 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        eprintln!("settings.json could not be read; using the last good settings");
        if let Some(s) = self.last_good.lock().unwrap().clone() {
            return s;
        }
        let mut bad = self.file.as_os_str().to_owned();
        bad.push(".bad");
        let _ = std::fs::copy(&self.file, PathBuf::from(bad));
        sanitize(&json!({}))
    }

    /// Sanitise and write (as 1.2 `save`). Returns what was written.
    pub fn save(&self, s: &Value) -> std::io::Result<Settings> {
        let _g = self.lock.lock().unwrap();
        self.save_locked(s)
    }

    fn save_locked(&self, s: &Value) -> std::io::Result<Settings> {
        let clean = sanitize(s);
        if let Some(d) = self.file.parent() {
            std::fs::create_dir_all(d)?;
        }
        super::js::write_atomic(&self.file, super::js::pretty(&clean.0).as_bytes())?;
        *self.last_good.lock().unwrap() = Some(clean.clone());
        if let Some(f) = self.on_change.lock().unwrap().as_ref() {
            f();
        }
        Ok(clean)
    }

    /// Load, change, save atomically with respect to other `update` calls.
    pub fn update(&self, f: impl FnOnce(&mut Value)) -> std::io::Result<Settings> {
        let _g = self.lock.lock().unwrap();
        let mut v = self.load().0;
        f(&mut v);
        self.save_locked(&v)
    }

    /// The temporary `cs2 edge-tune` tool saved `edge-offsets.json` next to settings.json. When settings.json has no
    /// `screenFit` yet, take those values over (once: the file is renamed afterwards). Returns true when it did.
    pub fn migrate_edge_offsets(&self) -> bool {
        let Some(dir) = self.file.parent() else { return false };
        let src = dir.join("edge-offsets.json");
        let Ok(text) = super::js::read_text(&src) else { return false };
        let Ok(old) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return false };
        let has_fit = super::js::read_text(&self.file)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok())
            .is_some_and(|v| v.get("screenFit").is_some());
        if has_fit {
            return false;
        }
        let fit = super::fit::Fit::from_value(&old);
        if self.update(|s| s["screenFit"] = fit.to_value()).is_err() {
            return false;
        }
        let _ = std::fs::rename(&src, dir.join("edge-offsets.migrated.json"));
        true
    }

    /// 1.2 `recordSlot`: what this app last wrote to a slot (null clears).
    pub fn record_slot(&self, slot: &str, v: Option<Value>) -> std::io::Result<Settings> {
        self.update(|s| {
            s["slots"][slot] = match v {
                Some(mut r) => {
                    r["at"] = json!(super::js::iso_now());
                    r
                }
                None => Value::Null,
            };
        })
    }

    /// 1.2 `patchSlot`.
    pub fn patch_slot(&self, slot: &str, patch: &Value) -> std::io::Result<Settings> {
        self.update(|s| {
            if !s["slots"][slot].is_null() {
                s["slots"][slot] = super::js::merge(&s["slots"][slot], patch);
            }
        })
    }
}

// ---------- slots (src/slots.js) ----------

/// Assumed size of a slot we have never written.
pub const UNKNOWN_OTHER: f64 = 1.8e6;
/// Margin kept free when the capacity was measured.
pub const SAFETY: f64 = 160_000.0;
/// Splash size before it has been made once (measured: about 0.54 MB).
pub const SPLASH_EST: f64 = 560_000.0;
/// The most both clips together may be counted as occupying on the SC70. Measured 2026-10-17: 11,605,065 bytes in use with
/// 405,504 free (a loop budget of 10,908,539); /data also holds ota.cpio (~0.68 MB). Kept a little under that so the loop budget
/// is about 10.85 MB. A measurement above it (a failed upload once recorded 13.4 MB) is cut down to it.
pub const MAX_CAPACITY: f64 = 11_550_000.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Budgets {
    pub total: f64,
    pub measured: bool,
    pub capacity_at: Option<String>,
    /// Space for the user's clip (the intro holds the boot splash).
    pub loop_bytes: f64,
    pub intro: f64,
}

/// `slots.budgets`; `splash_size` = size of the cached splash file (None: not made yet).
pub fn budgets(s: &Settings, splash_size: Option<u64>) -> Budgets {
    let cap = s.capacity();
    let measured = cap.is_some();
    let total = cap.as_ref().map(|c| c.0.min(MAX_CAPACITY)).unwrap_or(s.total_budget_mb() * 1e6);
    let intro = splash_size.map(|x| x as f64).unwrap_or(SPLASH_EST);
    Budgets {
        total,
        measured,
        capacity_at: cap.map(|c| c.1),
        loop_bytes: (total - intro - if measured { SAFETY } else { 0.0 }).floor(),
        intro,
    }
}

/// `slots.capacityFrom`: free blocks + the sizes we recorded for both slots (None if unknown).
pub fn capacity_from(info: &Value, s: &Settings) -> Option<f64> {
    // The real screen nests the fields: {"cmd":"info","data":{...,"i_block_free":4480}} (1.2 read the top level only,
    // so its capacity measurement never worked on hardware; the fake screen has them at the top level).
    let info = info.get("data").filter(|d| d.is_object()).unwrap_or(info);
    let free = super::js::to_num(info.get("i_block_free"));
    let bs = super::js::to_num(info.get("i_block_size"));
    let bs = if bs.is_finite() && bs != 0.0 { bs } else { 512.0 };
    let (su, sb) = (s.slot("startup")?, s.slot("standby")?);
    if !free.is_finite() || super::js::truthy(sb.get("incomplete")) || super::js::truthy(su.get("incomplete")) {
        return None;
    }
    // A full flash (nothing free) means the last upload ran out of room: the recorded sizes are not what is stored.
    if free <= 0.0 {
        return None;
    }
    let cap = free * bs + su["size"].as_f64()? + sb["size"].as_f64()?;
    // Never more than the flash itself holds.
    let blocks = super::js::to_num(info.get("i_blocks"));
    if blocks.is_finite() && blocks > 0.0 && cap > blocks * bs {
        return None;
    }
    Some(cap.min(MAX_CAPACITY))
}

pub fn is_blank(slot: &Value) -> bool {
    slot["size"].as_f64().is_some_and(|x| x <= 100_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_node() {
        // node -e "console.log(JSON.stringify(require('./src/settings').sanitize({})))"
        let node = r#"{"mask":{"mode":"band","circle":{"cx":45,"cy":540,"r":45,"feather":0},"band":{"edge":"left","px":92},"corners":{"on":false,"r":80}},"maxSeconds":10,"galleryDir":"","totalBudgetMB":9.5,"uploadRate":null,"capacity":null,"slots":{"startup":null,"standby":null},"sources":{"enabled":{},"options":{},"ticked":["mine"]}}"#;
        assert_eq!(serde_json::to_string(&sanitize(&json!({})).0).unwrap(), node);
    }

    #[test]
    fn sanitizes_like_node() {
        let input = json!({
            "mask": { "mode": "circle", "circle": { "cx": "100", "cy": 5000, "r": -1 }, "band": { "edge": "middle", "px": 50.5 }, "corners": { "on": true, "r": 80.6 } },
            "maxSeconds": 500, "galleryDir": "  D:\\G  ", "uploadRate": 52000.7, "capacity": { "bytes": 10875000.4, "at": "x" },
            "slots": { "startup": { "name": "(boot splash)", "size": 540000, "at": "t", "splash": "v1-abc", "extra": 1 }, "standby": { "size": -1 } },
            "sources": { "enabled": { "we": true, "BAD": true, "x": 1 }, "options": { "we": { "folders": [" a ", "", 3] } }, "ticked": ["mine", "we", "x"] }
        });
        let s = sanitize(&input);
        let node = r#"{"mask":{"mode":"circle","circle":{"cx":100,"cy":1080,"r":0,"feather":0},"band":{"edge":"left","px":50.5},"corners":{"on":true,"r":81}},"maxSeconds":120,"galleryDir":"D:\\G","totalBudgetMB":9.5,"uploadRate":52000.7,"capacity":{"bytes":10875000,"at":"x"},"slots":{"startup":{"name":"(boot splash)","size":540000,"at":"t","splash":"v1-abc"},"standby":null},"sources":{"enabled":{"we":true},"options":{"we":{"folders":["a"]}},"ticked":["mine","we"]}}"#;
        assert_eq!(serde_json::to_string(&s.0).unwrap(), node);
    }

    #[test]
    fn concurrent_reads_never_see_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let st = std::sync::Arc::new(SettingsStore::new(dir.path().join("settings.json")));
        st.update(|s| s["galleryDir"] = json!("D:\\MyGallery")).unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writers: Vec<_> = (0..3)
            .map(|i| {
                let (st, stop) = (st.clone(), stop.clone());
                std::thread::spawn(move || {
                    let mut n = 0;
                    while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                        st.update(|s| s["maxSeconds"] = json!(1 + (n + i) % 100)).unwrap();
                        n += 1;
                    }
                })
            })
            .collect();
        let reader_store = SettingsStore::new(dir.path().join("settings.json")); // no shared last-good cache
        for _ in 0..2000 {
            assert_eq!(reader_store.load().gallery_dir(), "D:\\MyGallery", "torn or default read");
        }
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        for w in writers {
            w.join().unwrap();
        }
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }

    #[test]
    fn corrupt_file_keeps_last_good() {
        let dir = tempfile::tempdir().unwrap();
        let st = SettingsStore::new(dir.path().join("settings.json"));
        st.update(|s| s["galleryDir"] = json!("D:\\G")).unwrap();
        std::fs::write(dir.path().join("settings.json"), "{ \"galleryDir\": \"D:\\\\G\", trunc").unwrap();
        assert_eq!(st.load().gallery_dir(), "D:\\G");
        // a fresh process with only a corrupt file: defaults, and the bad file is kept for inspection
        let st2 = SettingsStore::new(dir.path().join("settings.json"));
        assert_eq!(st2.load().gallery_dir(), "");
        assert!(dir.path().join("settings.json.bad").exists());
    }

    #[test]
    fn store_round_trip_and_slots() {
        let dir = tempfile::tempdir().unwrap();
        let st = SettingsStore::new(dir.path().join("settings.json"));
        assert_eq!(st.load(), sanitize(&json!({})));
        st.record_slot("standby", Some(json!({ "name": "clip", "size": 8_774_966, "versionId": "abcdefgh1" }))).unwrap();
        st.record_slot("startup", Some(json!({ "name": "(boot splash)", "size": 540_000, "splash": "v1-x" }))).unwrap();
        let s = st.load();
        assert_eq!(s.slot("standby").unwrap()["versionId"], json!("abcdefgh1"));
        let cap = capacity_from(&json!({ "i_block_free": 4480, "i_block_size": 512 }), &s).unwrap();
        assert_eq!(cap, (4480.0 * 512.0 + 540_000.0 + 8_774_966.0f64).min(MAX_CAPACITY));
        let real = json!({ "cmd": "info", "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 4480 }, "_transport": "winusb" });
        assert_eq!(capacity_from(&real, &s), Some(cap), "real replies nest the fields under data");
        st.patch_slot("standby", &json!({ "incomplete": true })).unwrap();
        assert!(capacity_from(&json!({ "i_block_free": 4480 }), &st.load()).is_none());
        let b = budgets(&st.load(), Some(540_000));
        assert_eq!(b.loop_bytes, 9_500_000.0 - 540_000.0);
        st.update(|v| v["capacity"] = json!({ "bytes": 11_000_000, "at": "now" })).unwrap();
        let b = budgets(&st.load(), Some(540_000));
        assert!(b.measured);
        assert_eq!(b.loop_bytes, 11_000_000.0 - 540_000.0 - SAFETY);
    }
}
