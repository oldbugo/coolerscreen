//! Cleaning of the per-type widget props the editor sends (docs/v3/theme-format.md, v1.6): wrong types fall back to the
//! defaults, numbers are clamped, words outside the allowed list are replaced, lists are bounded. The engine parses the
//! same props leniently; this keeps theme.json tidy and bounded. Idempotent. Unknown keys are kept (the engine ignores them).
use serde_json::{json, Map, Value};

/// Every widget type the app accepts (the engine draws them; unknown ones are refused by `theme_set`).
pub const TYPES: [&str; 11] = ["clock", "music", "stats", "text", "image", "weather", "sensor", "date", "visualizer", "slideshow", "info"];

pub const METRICS: [&str; 15] = [
    "cpu.load", "cpu.core", "cpu.temp", "cpu.clock", "gpu.load", "gpu.temp", "gpu.clock", "gpu.vram", "gpu.power", "gpu.fan", "ram.used",
    "net.down", "net.up", "disk.used", "disk.activity",
];
pub const SENSOR_STYLES: [&str; 6] = ["number", "bar", "ring", "gauge", "graph", "sparkline"];
pub const CLOCK_STYLES: [&str; 3] = ["digital", "analog", "minimal"];
pub const DATE_STYLES: [&str; 3] = ["line", "calendar", "big"];
pub const WEATHER_STYLES: [&str; 2] = ["now", "forecast"];
pub const VIS_STYLES: [&str; 3] = ["bars-h", "ring", "wave-fade"];
pub const INFO_KINDS: [&str; 5] = ["uptime", "cpuName", "gpuName", "countdown", "topProcesses"];
pub const MAX_SLIDES: usize = 30;
pub const MAX_THRESHOLDS: usize = 6;

fn word(o: &mut Map<String, Value>, key: &str, ok: &[&str], dflt: &str) {
    let v = match o.get(key).and_then(Value::as_str) {
        Some(s) if ok.contains(&s) => s.to_string(),
        _ => dflt.to_string(),
    };
    o.insert(key.into(), json!(v));
}

fn is_hex(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

fn colour(o: &mut Map<String, Value>, key: &str, dflt: &str) {
    let c = o.get(key).and_then(Value::as_str).filter(|s| is_hex(s)).map(|s| s.to_lowercase()).unwrap_or_else(|| dflt.to_string());
    o.insert(key.into(), json!(c));
}

/// A number in `lo..=hi` (rounded to `places` decimals); missing, wrong type or NaN gives `dflt`.
fn number(o: &mut Map<String, Value>, key: &str, dflt: f64, lo: f64, hi: f64, places: i32) {
    let x = o.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(dflt).clamp(lo, hi);
    let k = 10f64.powi(places);
    let x = (x * k).round() / k;
    o.insert(key.into(), if places == 0 { json!(x as i64) } else { json!(x) });
}

/// A number or null (null = automatic).
fn opt_number(o: &mut Map<String, Value>, key: &str, lo: f64, hi: f64) {
    let v = o.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| x.clamp(lo, hi));
    o.insert(key.into(), v.map(|x| json!((x * 1000.0).round() / 1000.0)).unwrap_or(Value::Null));
}

fn text(o: &mut Map<String, Value>, key: &str, max: usize, keep_null: bool) {
    let v = match o.get(key) {
        Some(Value::String(s)) => json!(s.chars().take(max).collect::<String>()),
        _ if keep_null => Value::Null,
        _ => json!(""),
    };
    o.insert(key.into(), v);
}

/// A path inside the theme folder that stays inside it ("assets/logo.png").
pub fn safe_rel(rel: &str) -> bool {
    use std::path::{Component, Path};
    !rel.is_empty() && Path::new(rel).components().all(|c| matches!(c, Component::Normal(_)))
}

/// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM[:SS]` (what the date/time pickers produce); anything else is dropped.
pub fn valid_target(s: &str) -> bool {
    let b = s.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|d| d.iter().all(u8::is_ascii_digit));
    if !(b.len() == 10 || b.len() == 16 || b.len() == 19) || !digits(0..4) || b[4] != b'-' || !digits(5..7) || b[7] != b'-' || !digits(8..10) {
        return false;
    }
    let (m, d) = (s[5..7].parse::<u32>().unwrap_or(0), s[8..10].parse::<u32>().unwrap_or(0));
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return false;
    }
    if b.len() == 10 {
        return true;
    }
    if b[10] != b'T' || !digits(11..13) || b[13] != b':' || !digits(14..16) {
        return false;
    }
    if s[11..13].parse::<u32>().unwrap_or(99) > 23 || s[14..16].parse::<u32>().unwrap_or(99) > 59 {
        return false;
    }
    b.len() == 16 || (b[16] == b':' && digits(17..19) && s[17..19].parse::<u32>().unwrap_or(99) <= 59)
}

fn thresholds(o: &mut Map<String, Value>) {
    // absent or null = the metric's own default; a list (maybe empty) is the user's
    let Some(a) = o.get("thresholds").and_then(Value::as_array) else {
        o.remove("thresholds");
        return;
    };
    let mut rows: Vec<(f64, String)> = a
        .iter()
        .filter_map(|t| {
            let at = t.get("at").and_then(Value::as_f64).filter(|x| x.is_finite())?;
            let c = t.get("color").and_then(Value::as_str).filter(|s| is_hex(s)).map(str::to_lowercase)?;
            Some(((at.clamp(-1e9, 1e9) * 1000.0).round() / 1000.0, c))
        })
        .take(MAX_THRESHOLDS)
        .collect();
    rows.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    o.insert("thresholds".into(), Value::Array(rows.into_iter().map(|(at, color)| json!({ "at": at, "color": color })).collect()));
}

fn sensor(o: &mut Map<String, Value>) {
    word(o, "metric", &METRICS, "cpu.load");
    word(o, "style", &SENSOR_STYLES, "number");
    number(o, "core", 0.0, 0.0, 255.0, 0);
    let drive = o.get("drive").and_then(Value::as_str).map(|d| d.trim().to_uppercase()).filter(|d| d.len() == 2 && d.as_bytes()[0].is_ascii_alphabetic() && d.ends_with(':'));
    o.insert("drive".into(), json!(drive.unwrap_or_else(|| "C:".into())));
    text(o, "label", 40, true); // null = the default name, "" = none
    word(o, "unit", &["C", "F"], "C");
    let dec = o.get("decimals").and_then(Value::as_f64).filter(|x| x.is_finite());
    o.insert("decimals".into(), dec.map(|d| json!(d.clamp(0.0, 3.0).round() as i64)).unwrap_or(Value::Null));
    opt_number(o, "min", -1e9, 1e9);
    opt_number(o, "max", -1e9, 1e9);
    if let (Some(lo), Some(hi)) = (o["min"].as_f64(), o["max"].as_f64()) {
        if lo >= hi {
            o.insert("min".into(), Value::Null);
            o.insert("max".into(), Value::Null);
        }
    }
    thresholds(o);
    // colour: null = the metric's own colour
    let col = o.get("color").and_then(Value::as_str).filter(|s| is_hex(s)).map(|s| json!(s.to_lowercase())).unwrap_or(Value::Null);
    o.insert("color".into(), col);
    number(o, "history", 60.0, 5.0, 600.0, 0);
    // the engine's steps: 1, 2 (also 3), 4
    let rate = o.get("rate").and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(1.0);
    o.insert("rate".into(), json!(if rate >= 3.5 { 4 } else if rate >= 1.5 { 2 } else { 1 }));
    let ad = match o.get("adapter") {
        Some(Value::String(s)) if !s.trim().is_empty() => json!(s.chars().take(80).collect::<String>()),
        _ => Value::Null,
    };
    o.insert("adapter".into(), ad);
}

/// Clean the props of one widget in place. `kind` is the widget type; other types are left alone.
pub fn clean(kind: &str, props: &mut Value) {
    if !props.is_object() {
        *props = json!({});
    }
    let Some(o) = props.as_object_mut() else { return };
    match kind {
        "sensor" => sensor(o),
        "clock" => {
            // existing props keep their meaning; the style is new (digital = today's look)
            word(o, "style", &CLOCK_STYLES, "digital");
            if o.contains_key("format") {
                word(o, "format", &["24h", "12h"], "24h");
            }
        }
        "date" => {
            word(o, "style", &DATE_STYLES, "line");
            colour(o, "color", "#ffffff");
        }
        "weather" => {
            word(o, "style", &WEATHER_STYLES, "now");
            word(o, "unit", &["C", "F"], "C");
            word(o, "span", &["days", "hours"], "days");
            let hours = o["span"] == "hours";
            number(o, "count", 4.0, 3.0, if hours { 6.0 } else { 5.0 }, 0);
            colour(o, "color", "#ffffff");
        }
        "visualizer" => {
            word(o, "style", &VIS_STYLES, "bars-h");
            number(o, "bands", 32.0, 16.0, 64.0, 0);
            colour(o, "color", "#5ac8ff");
            number(o, "sensitivity", 1.0, 0.2, 4.0, 2);
        }
        "slideshow" => {
            let files: Vec<Value> = o
                .get("files")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).filter(|f| safe_rel(f)).take(MAX_SLIDES).map(|f| json!(f)).collect())
                .unwrap_or_default();
            o.insert("files".into(), Value::Array(files));
            number(o, "seconds", 8.0, 1.0, 3600.0, 0);
            number(o, "fade", 1.0, 0.0, 5.0, 1);
            word(o, "fit", &["contain", "cover"], "cover");
        }
        "info" => {
            word(o, "kind", &INFO_KINDS, "uptime");
            colour(o, "color", "#ffffff");
            text(o, "label", 40, true);
            let t = o.get("target").and_then(Value::as_str).filter(|s| valid_target(s)).map(String::from);
            o.insert("target".into(), t.map(Value::from).unwrap_or(Value::Null));
            word(o, "by", &["cpu", "ram"], "cpu");
            number(o, "count", 4.0, 3.0, 5.0, 0);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(kind: &str, v: Value) -> Value {
        let mut v = v;
        clean(kind, &mut v);
        v
    }

    #[test]
    fn sensor_defaults_and_clamps() {
        let p = c("sensor", json!({}));
        assert_eq!(p["metric"], "cpu.load");
        assert_eq!(p["style"], "number");
        assert_eq!(p["drive"], "C:");
        assert_eq!(p["history"], 60);
        assert_eq!(p["rate"], 1);
        assert!(p["min"].is_null() && p["max"].is_null() && p["label"].is_null() && p["decimals"].is_null());
        assert!(p.get("thresholds").is_none(), "absent = the metric's default");
        let p = c("sensor", json!({ "metric": "nope", "style": "pie", "history": 5, "rate": 9, "decimals": 7, "core": -3, "color": "red", "drive": "d:", "unit": "K" }));
        assert_eq!(p["metric"], "cpu.load");
        assert_eq!(p["style"], "number");
        assert_eq!((p["history"].clone(), p["rate"].clone(), p["decimals"].clone(), p["core"].clone()), (json!(5), json!(4), json!(3), json!(0)));
        assert_eq!((p["color"].clone(), p["drive"].clone(), p["unit"].clone()), (Value::Null, json!("D:"), json!("C")));
        assert_eq!(c("sensor", json!({ "history": 99999 }))["history"], 600);
        assert_eq!(c("sensor", json!({ "rate": 2 }))["rate"], 2);
    }

    #[test]
    fn sensor_range_and_thresholds() {
        let p = c("sensor", json!({ "min": 50, "max": 10 }));
        assert!(p["min"].is_null() && p["max"].is_null(), "min must be below max");
        let p = c("sensor", json!({ "min": 0, "max": 100 }));
        assert_eq!((p["min"].clone(), p["max"].clone()), (json!(0.0), json!(100.0)));
        let p = c("sensor", json!({ "thresholds": [{ "at": 85, "color": "#FF6B6B" }, { "at": 70, "color": "#f0b43c" }, { "at": "x", "color": "#fff" }, { "color": "#fff000" }, { "at": 5, "color": "blue" }] }));
        assert_eq!(p["thresholds"], json!([{ "at": 70.0, "color": "#f0b43c" }, { "at": 85.0, "color": "#ff6b6b" }]), "sorted, lower-cased, bad rows dropped");
        assert_eq!(c("sensor", json!({ "thresholds": [] }))["thresholds"], json!([]), "an empty list means none");
        assert!(c("sensor", json!({ "thresholds": null })).get("thresholds").is_none());
        let many: Vec<Value> = (0..20).map(|i| json!({ "at": i, "color": "#112233" })).collect();
        assert_eq!(c("sensor", json!({ "thresholds": many }))["thresholds"].as_array().unwrap().len(), MAX_THRESHOLDS);
    }

    #[test]
    fn other_types() {
        assert_eq!(c("clock", json!({ "style": "sundial", "format": "24h" }))["style"], "digital");
        assert_eq!(c("clock", json!({ "style": "analog" }))["style"], "analog");
        assert_eq!(c("date", json!({}))["style"], "line");
        let w = c("weather", json!({ "unit": "F", "style": "forecast", "span": "hours", "count": 99 }));
        assert_eq!((w["style"].clone(), w["unit"].clone(), w["span"].clone(), w["count"].clone()), (json!("forecast"), json!("F"), json!("hours"), json!(6)));
        assert_eq!(c("weather", json!({ "count": 99 }))["count"], 5);
        let v = c("visualizer", json!({ "bands": 500, "sensitivity": 0, "style": "ring" }));
        assert_eq!((v["bands"].clone(), v["sensitivity"].clone(), v["style"].clone()), (json!(64), json!(0.2), json!("ring")));
        assert_eq!(c("visualizer", json!({ "bands": 3 }))["bands"], 16);
        let s = c("slideshow", json!({ "files": ["assets/a.png", "../x.png", "/abs.png", "assets/../../y.png", 5, "assets/b.gif"], "seconds": 0, "fade": 99 }));
        assert_eq!(s["files"], json!(["assets/a.png", "assets/b.gif"]));
        assert_eq!((s["seconds"].clone(), s["fade"].clone(), s["fit"].clone()), (json!(1), json!(5.0), json!("cover")));
        let i = c("info", json!({ "kind": "countdown", "target": "2027-01-01T09:30", "label": "Trip", "count": 9 }));
        assert_eq!((i["kind"].clone(), i["target"].clone(), i["label"].clone(), i["count"].clone()), (json!("countdown"), json!("2027-01-01T09:30"), json!("Trip"), json!(5)));
        assert!(c("info", json!({ "target": "tomorrow" }))["target"].is_null());
        assert!(c("info", json!({ "target": "2027-13-01" }))["target"].is_null());
        assert_eq!(c("info", json!({ "kind": "bogus" }))["kind"], "uptime");
        assert_eq!(c("info", json!({ "by": "disk" }))["by"], "cpu");
        // a props value that is not an object becomes one; types the app does not clean are untouched
        assert_eq!(c("sensor", json!("x"))["metric"], "cpu.load");
        assert_eq!(c("text", json!({ "text": "hi" })), json!({ "text": "hi" }));
        // idempotent
        let once = c("sensor", json!({ "metric": "net.down", "style": "graph", "thresholds": [{ "at": 3, "color": "#abcdef" }] }));
        assert_eq!(c("sensor", once.clone()), once);
    }

    #[test]
    fn countdown_targets() {
        for ok in ["2026-12-31", "2026-12-31T23:59", "2026-12-31T23:59:59"] {
            assert!(valid_target(ok), "{ok}");
        }
        for bad in ["", "2026-12-31T24:00", "2026-12-31 10:00", "2026-00-10", "2026-12-32", "2026-12-31T10:60", "x"] {
            assert!(!valid_target(bad), "{bad}");
        }
    }
}
