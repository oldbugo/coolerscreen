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
    for k in ["originalId", "versionId", "label", "splash"] {
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
    Settings(json!({
        "mask": mask,
        "maxSeconds": num(clamp_num(s.get("maxSeconds"), 10.0, 1.0, 120.0)),
        "galleryDir": gallery_dir,
        "totalBudgetMB": num(clamp_num(s.get("totalBudgetMB"), 9.5, 1.0, 10.0)),
        "uploadRate": upload_rate,
        "capacity": capacity,
        "slots": { "startup": clean_slot(slots.and_then(|x| x.get("startup"))), "standby": clean_slot(slots.and_then(|x| x.get("standby"))) },
        "sources": clean_sources(s.get("sources")),
    }))
}

/// Reads and writes settings.json; read-modify-write goes through `update` so concurrent tasks don't lose changes.
pub struct SettingsStore {
    file: PathBuf,
    lock: Mutex<()>,
    on_change: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl SettingsStore {
    pub fn new(file: impl Into<PathBuf>) -> Self {
        SettingsStore { file: file.into(), lock: Mutex::new(()), on_change: Mutex::new(None) }
    }
    pub fn file(&self) -> &Path {
        &self.file
    }
    pub fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        *self.on_change.lock().unwrap() = Some(Box::new(f));
    }

    pub fn load(&self) -> Settings {
        match super::js::read_json(&self.file) {
            Some(v) => sanitize(&v),
            None => sanitize(&json!({})),
        }
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
        std::fs::write(&self.file, super::js::pretty(&clean.0))?;
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
    let total = cap.as_ref().map(|c| c.0).unwrap_or(s.total_budget_mb() * 1e6);
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
    let free = super::js::to_num(info.get("i_block_free"));
    let bs = super::js::to_num(info.get("i_block_size"));
    let bs = if bs.is_finite() && bs != 0.0 { bs } else { 512.0 };
    let (su, sb) = (s.slot("startup")?, s.slot("standby")?);
    if !free.is_finite() || super::js::truthy(sb.get("incomplete")) || super::js::truthy(su.get("incomplete")) {
        return None;
    }
    Some(free * bs + su["size"].as_f64()? + sb["size"].as_f64()?)
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
    fn store_round_trip_and_slots() {
        let dir = tempfile::tempdir().unwrap();
        let st = SettingsStore::new(dir.path().join("settings.json"));
        assert_eq!(st.load(), sanitize(&json!({})));
        st.record_slot("standby", Some(json!({ "name": "clip", "size": 8_774_966, "versionId": "abcdefgh1" }))).unwrap();
        st.record_slot("startup", Some(json!({ "name": "(boot splash)", "size": 540_000, "splash": "v1-x" }))).unwrap();
        let s = st.load();
        assert_eq!(s.slot("standby").unwrap()["versionId"], json!("abcdefgh1"));
        let cap = capacity_from(&json!({ "i_block_free": 4480, "i_block_size": 512 }), &s).unwrap();
        assert_eq!(cap, 4480.0 * 512.0 + 540_000.0 + 8_774_966.0);
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
