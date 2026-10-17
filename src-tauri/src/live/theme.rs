//! 3.0 theme model (`<library>/originals/<id>/theme.json`), exactly per docs/v3/theme-format.md.
//! A theme = base (the loop stored on the screen) + optional live layer (widgets) + hand-over options.
//! Coordinates are fractions (0..1) of the landscape 2400x1080 canvas; the engine's working canvas is 1680x756 (same aspect).
//! Parsing is lenient: missing fields take defaults, unknown fields and unknown prop keys are ignored, unknown enum words
//! fall back to the default. `clamp` brings a parsed theme into valid ranges; `validate` reports what clamp would have to fix.
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{json, Value};
use std::path::PathBuf;

pub const THEME_VERSION: u32 = 1;
/// Smallest widget box side (fraction of the canvas).
pub const MIN_BOX: f32 = 0.02;

/// Widget types the engine draws. "weather" is reserved (a labelled placeholder).
pub const WIDGET_TYPES: [&str; 6] = ["clock", "music", "stats", "text", "image", "weather"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Theme {
    pub v: u32,
    /// The version stored on the screen as this theme's base; None until first applied.
    pub base_version_id: Option<String>,
    pub widgets: Vec<Widget>,
    pub handover: Handover,
}

impl Default for Theme {
    fn default() -> Self {
        Theme { v: THEME_VERSION, base_version_id: None, widgets: Vec::new(), handover: Handover::default() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Widget {
    pub id: String,
    /// "clock" | "music" | "stats" | "text" | "image" | "weather".
    #[serde(rename = "type")]
    pub kind: String,
    /// Box top-left and size, fractions of the landscape canvas.
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    /// Text and image only: also baked into the base recording.
    pub on_base: bool,
    /// Per-type properties (see the typed structs below; unknown keys ignored, missing = default).
    pub props: Value,
}

impl Default for Widget {
    fn default() -> Self {
        Widget { id: String::new(), kind: "text".into(), x: 0.0, y: 0.0, w: 0.2, h: 0.2, on_base: false, props: json!({}) }
    }
}

impl Widget {
    pub fn new(id: &str, kind: &str, x: f32, y: f32, w: f32, h: f32, props: Value) -> Widget {
        Widget { id: id.into(), kind: kind.into(), x, y, w, h, on_base: false, props }
    }
    /// Typed view of `props`; a prop of the wrong type or a malformed object gives the defaults.
    pub fn clock(&self) -> ClockProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn music(&self) -> MusicProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn stats(&self) -> StatsProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn text(&self) -> TextProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn image(&self) -> ImageProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    /// Only text and image can be baked into the base.
    pub fn can_be_on_base(&self) -> bool {
        self.kind == "text" || self.kind == "image"
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Handover {
    /// Seconds (0..3) the live layer fades in when live starts.
    pub fade_in: f32,
    #[serde(deserialize_with = "lenient")]
    pub end: HandoverEnd,
    #[serde(deserialize_with = "lenient")]
    pub music_idle: MusicIdle,
}

impl Default for Handover {
    fn default() -> Self {
        Handover { fade_in: 0.8, end: HandoverEnd::LoopStart, music_idle: MusicIdle::Hide }
    }
}

/// What happens when live stops: fade out onto the stored loop's first frame / plain stop / fade to black and screen off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HandoverEnd {
    #[default]
    LoopStart,
    Instant,
    ScreenOff,
}

/// Music widget when nothing plays: hidden, or kept (shows an idle card).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MusicIdle {
    #[default]
    Hide,
    Keep,
}

/// An unknown word (or wrong type) gives the default instead of failing the whole theme.
fn lenient<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned + Default,
{
    let v = Value::deserialize(d)?;
    Ok(serde_json::from_value(v).unwrap_or_default())
}

// ---------------------------------------------------------------- typed props

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ClockProps {
    /// "24h" | "12h"
    pub format: String,
    pub seconds: bool,
    pub date: bool,
    pub color: String,
}
impl Default for ClockProps {
    fn default() -> Self {
        ClockProps { format: "24h".into(), seconds: false, date: true, color: "#ffffff".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MusicProps {
    pub cover: bool,
    pub progress: bool,
}
impl Default for MusicProps {
    fn default() -> Self {
        MusicProps { cover: true, progress: true }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StatsProps {
    /// "cpu" | "ram" | "gpuLoad" | "gpuTemp" | "gpuClock" | "cpuTemp"
    pub items: Vec<String>,
}
impl Default for StatsProps {
    fn default() -> Self {
        StatsProps { items: ["cpu", "ram", "gpuLoad", "gpuTemp", "gpuClock"].iter().map(|s| s.to_string()).collect() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TextProps {
    pub text: String,
    /// Fraction of canvas height, 0.02..0.3.
    pub size: f32,
    pub color: String,
    pub opacity: f32,
    /// "left" | "center" | "right"
    pub align: String,
    /// "light" | "regular" | "bold"
    pub weight: String,
}
impl Default for TextProps {
    fn default() -> Self {
        TextProps { text: String::new(), size: 0.08, color: "#ffffff".into(), opacity: 1.0, align: "left".into(), weight: "regular".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImageProps {
    /// Relative to the theme folder.
    pub file: String,
    pub opacity: f32,
    /// "contain" | "cover"
    pub fit: String,
}
impl Default for ImageProps {
    fn default() -> Self {
        ImageProps { file: String::new(), opacity: 1.0, fit: "contain".into() }
    }
}

/// "#rrggbb" (also "#rgb", leading '#' optional); anything else gives `fallback`.
pub fn parse_color(s: &str, fallback: [u8; 3]) -> [u8; 3] {
    let h = s.trim().trim_start_matches('#');
    let hex = |i: usize, n: usize| u8::from_str_radix(&h[i..i + n], 16).ok();
    if !h.is_ascii() {
        return fallback;
    }
    match h.len() {
        6 => match (hex(0, 2), hex(2, 2), hex(4, 2)) {
            (Some(r), Some(g), Some(b)) => [r, g, b],
            _ => fallback,
        },
        3 => match (hex(0, 1), hex(1, 1), hex(2, 1)) {
            (Some(r), Some(g), Some(b)) => [r * 17, g * 17, b * 17],
            _ => fallback,
        },
        _ => fallback,
    }
}

// ---------------------------------------------------------------- theme operations

impl Theme {
    /// Parse theme.json text. Lenient (see module doc); only malformed JSON is an error. The result is clamped.
    pub fn from_json(s: &str) -> Result<Theme, String> {
        let mut t: Theme = serde_json::from_str(s).map_err(|e| format!("theme.json: {e}"))?;
        t.clamp();
        Ok(t)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Load `<dir>/theme.json`; missing file = the default theme.
    pub fn load(dir: &std::path::Path) -> Result<Theme, String> {
        match std::fs::read_to_string(dir.join("theme.json")) {
            Ok(s) => Theme::from_json(&s),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Theme::default()),
            Err(e) => Err(format!("theme.json: {e}")),
        }
    }

    /// The "Standard" preset: today's layout (clock top-left, music under it, stats on the right).
    pub fn standard_preset() -> Theme {
        Theme {
            widgets: vec![
                Widget::new("clock", "clock", 0.062, 0.0635, 0.530, 0.520, json!({ "format": "24h", "seconds": false, "date": true })),
                Widget::new("music", "music", 0.062, 0.632, 0.530, 0.291, json!({ "cover": true, "progress": true })),
                Widget::new("stats", "stats", 0.619, 0.0635, 0.352, 0.873, json!({ "items": ["cpu", "ram", "gpuLoad", "gpuTemp", "gpuClock"] })),
            ],
            ..Theme::default()
        }
    }

    /// Bring everything into valid ranges: version, fade (0..3), boxes inside the canvas with a minimum size, unique
    /// non-empty ids, props always an object, onBase only where allowed, text size/opacity ranges.
    pub fn clamp(&mut self) {
        self.v = THEME_VERSION;
        self.handover.fade_in = if self.handover.fade_in.is_finite() { self.handover.fade_in.clamp(0.0, 3.0) } else { 0.8 };
        let mut seen = std::collections::HashSet::new();
        for (i, w) in self.widgets.iter_mut().enumerate() {
            for f in [&mut w.x, &mut w.y, &mut w.w, &mut w.h] {
                if !f.is_finite() {
                    *f = 0.0;
                }
            }
            w.w = w.w.clamp(MIN_BOX, 1.0);
            w.h = w.h.clamp(MIN_BOX, 1.0);
            w.x = w.x.clamp(0.0, 1.0 - w.w);
            w.y = w.y.clamp(0.0, 1.0 - w.h);
            if !w.props.is_object() {
                w.props = json!({});
            }
            if !w.can_be_on_base() {
                w.on_base = false;
            }
            let mut id = if w.id.trim().is_empty() { format!("w{}", i + 1) } else { w.id.clone() };
            while !seen.insert(id.clone()) {
                id.push('_');
            }
            w.id = id;
            if w.kind == "text" {
                if let Some(o) = w.props.as_object_mut() {
                    if let Some(s) = o.get("size").and_then(Value::as_f64) {
                        o.insert("size".into(), json!(s.clamp(0.02, 0.3)));
                    }
                }
            }
            if w.kind == "text" || w.kind == "image" {
                if let Some(o) = w.props.as_object_mut() {
                    if let Some(s) = o.get("opacity").and_then(Value::as_f64) {
                        o.insert("opacity".into(), json!(s.clamp(0.0, 1.0)));
                    }
                }
            }
        }
    }

    /// Problems `clamp` would silently repair or that make a widget useless; empty = fine.
    pub fn validate(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.v > THEME_VERSION {
            out.push(format!("theme version {} is newer than this app understands ({THEME_VERSION})", self.v));
        }
        if !(0.0..=3.0).contains(&self.handover.fade_in) {
            out.push("handover.fadeIn must be 0 to 3 seconds".into());
        }
        let mut ids = std::collections::HashSet::new();
        for w in &self.widgets {
            if w.id.trim().is_empty() || !ids.insert(w.id.as_str()) {
                out.push(format!("widget id '{}' is empty or used twice", w.id));
            }
            if !WIDGET_TYPES.contains(&w.kind.as_str()) {
                out.push(format!("widget '{}': unknown type '{}'", w.id, w.kind));
            }
            if w.w < MIN_BOX || w.h < MIN_BOX || w.x < 0.0 || w.y < 0.0 || w.x + w.w > 1.0001 || w.y + w.h > 1.0001 {
                out.push(format!("widget '{}': box outside the canvas or too small", w.id));
            }
            if w.on_base && !w.can_be_on_base() {
                out.push(format!("widget '{}': only text and image can be on the base", w.id));
            }
        }
        out
    }

    /// True when any widget is baked into the base (the app then offers "Base needs re-storing").
    pub fn has_base_items(&self) -> bool {
        self.widgets.iter().any(|w| w.on_base && w.can_be_on_base())
    }

    /// The part of the theme that shapes the stored base (for change detection: equal = the base needs no re-storing).
    pub fn base_signature(&self) -> String {
        let items: Vec<&Widget> = self.widgets.iter().filter(|w| w.on_base && w.can_be_on_base()).collect();
        serde_json::to_string(&items).unwrap_or_default()
    }
}

/// A theme plus where its files are, as the engine needs it.
#[derive(Clone, Debug, Default)]
pub struct ThemeLive {
    pub theme: Theme,
    /// Folder of theme.json (image widget paths are relative to it).
    pub theme_dir: PathBuf,
    /// The base version file: portrait MJPEG 1080x2400 (landscape made with transpose=1). Live background + hand-over target.
    pub base_video: Option<PathBuf>,
    /// The original clip, used as the live background when there is no base yet.
    pub fallback_video: Option<PathBuf>,
}

impl ThemeLive {
    pub fn new(theme: Theme, theme_dir: PathBuf) -> ThemeLive {
        ThemeLive { theme, theme_dir, base_video: None, fallback_video: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_missing_fields_take_defaults() {
        let t = Theme::from_json("{}").unwrap();
        assert_eq!(t, Theme::default());
        assert_eq!(t.handover.fade_in, 0.8);
        assert_eq!(t.handover.end, HandoverEnd::LoopStart);
        let t = Theme::from_json(r#"{"widgets":[{"type":"clock"}],"handover":{"end":"nonsense","musicIdle":"keep"}}"#).unwrap();
        assert_eq!(t.widgets[0].kind, "clock");
        assert_eq!(t.widgets[0].id, "w1");
        assert_eq!(t.handover.end, HandoverEnd::LoopStart);
        assert_eq!(t.handover.music_idle, MusicIdle::Keep);
        assert!(Theme::from_json("{ broken").is_err());
    }

    #[test]
    fn contract_example_round_trips_in_camel_case() {
        let s = r#"{ "v": 1, "baseVersionId": "abc", "widgets": [
            { "id": "w1", "type": "clock", "x": 0.08, "y": 0.18, "w": 0.48, "h": 0.42, "onBase": false,
              "props": { "format": "24h", "seconds": true, "date": true } } ],
            "handover": { "fadeIn": 0.8, "end": "loopStart", "musicIdle": "hide" } }"#;
        let t = Theme::from_json(s).unwrap();
        assert_eq!(t.base_version_id.as_deref(), Some("abc"));
        assert!(t.widgets[0].clock().seconds);
        let out = t.to_json();
        assert!(out.contains("baseVersionId") && out.contains("\"type\"") && out.contains("onBase") && out.contains("fadeIn"));
        assert_eq!(Theme::from_json(&out).unwrap(), t);
        let e = Theme { handover: Handover { end: HandoverEnd::ScreenOff, ..Handover::default() }, ..Theme::default() };
        assert!(e.to_json().contains("\"screenOff\""));
    }

    #[test]
    fn clamping_repairs_boxes_ids_and_ranges() {
        let mut t = Theme::default();
        t.handover.fade_in = 9.0;
        t.widgets = vec![
            Widget { x: 0.9, y: -1.0, w: 0.5, h: 0.0, on_base: true, kind: "clock".into(), id: "a".into(), ..Widget::default() },
            Widget { id: "a".into(), kind: "text".into(), props: json!({ "size": 5.0, "opacity": 3 }), on_base: true, ..Widget::default() },
            Widget { props: json!("oops"), ..Widget::default() },
        ];
        assert!(!t.validate().is_empty());
        t.clamp();
        assert_eq!(t.handover.fade_in, 3.0);
        let w = &t.widgets[0];
        assert!(w.x + w.w <= 1.0 + 1e-6 && w.y == 0.0 && w.h == MIN_BOX && !w.on_base);
        assert_eq!(t.widgets[1].id, "a_");
        assert_eq!(t.widgets[1].text().size, 0.3);
        assert_eq!(t.widgets[1].text().opacity, 1.0);
        assert!(t.widgets[1].on_base);
        assert_eq!(t.widgets[2].id, "w3");
        assert!(t.widgets[2].props.is_object());
        assert!(t.validate().is_empty(), "{:?}", t.validate());
    }

    #[test]
    fn standard_preset_is_valid_and_matches_todays_layout() {
        let t = Theme::standard_preset();
        assert!(t.validate().is_empty());
        let kinds: Vec<_> = t.widgets.iter().map(|w| w.kind.as_str()).collect();
        assert_eq!(kinds, ["clock", "music", "stats"]);
        // stats panel at today's 1040,48 592x660 on the 1680x756 canvas
        let s = &t.widgets[2];
        assert!(((s.x * 1680.0) - 1040.0).abs() < 1.0 && ((s.y * 756.0) - 48.0).abs() < 1.0);
        assert!(((s.w * 1680.0) - 592.0).abs() < 1.0 && ((s.h * 756.0) - 660.0).abs() < 1.0);
        assert!(!t.has_base_items());
    }

    #[test]
    fn colors_and_props_fall_back() {
        assert_eq!(parse_color("#ff8000", [1, 2, 3]), [255, 128, 0]);
        assert_eq!(parse_color("f80", [1, 2, 3]), [255, 136, 0]);
        assert_eq!(parse_color("red", [1, 2, 3]), [1, 2, 3]);
        let w = Widget { kind: "stats".into(), props: json!({ "items": 5 }), ..Widget::default() };
        assert_eq!(w.stats(), StatsProps::default());
        let w = Widget { kind: "clock".into(), props: json!({ "seconds": true, "bogus": 1 }), ..Widget::default() };
        assert!(w.clock().seconds && w.clock().date);
    }
}
