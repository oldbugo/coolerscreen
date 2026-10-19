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
/// Default theme gap (px on the 2400x1080 canvas).
pub const DEFAULT_GAP: f32 = 24.0;

/// Widget types the engine draws. "weather" is reserved (a labelled placeholder).
pub const WIDGET_TYPES: [&str; 11] = ["clock", "music", "stats", "text", "image", "weather", "sensor", "date", "visualizer", "slideshow", "info"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Theme {
    pub v: u32,
    /// The version stored on the screen as this theme's base; None until first applied.
    pub base_version_id: Option<String>,
    pub widgets: Vec<Widget>,
    /// Editor groups (members are contiguous in `widgets`); the engine only uses their `hidden` flag.
    pub groups: Vec<Group>,
    /// The single "Live layer" switch: false = base only (the app stops streaming; previews show the base only).
    pub live_layer: bool,
    /// Per-theme offsets added to the global screen guides.
    pub guides_offset: GuidesOffset,
    /// Corner radius in px on 2400x1080: None = use the settings default, 0 = none for this theme.
    pub corners: Option<f32>,
    /// Snap distance in px on 2400x1080 (editor): spacing from canvas edges, guide bands and between boxes.
    pub gap: f32,
    pub handover: Handover,
    /// 4.0: a live-drawn background (`kind: "effect"`) instead of the theme's picture. None = the picture, as in 3.0.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "lenient")]
    pub background: Option<super::effects::Background>,
}

impl Default for Theme {
    fn default() -> Self {
        Theme { v: THEME_VERSION, base_version_id: None, widgets: Vec::new(), groups: Vec::new(), live_layer: true, guides_offset: GuidesOffset::default(), corners: None, gap: DEFAULT_GAP, handover: Handover::default(), background: None }
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
    /// Not drawn live and not baked.
    pub hidden: bool,
    /// Id of the group this widget belongs to.
    pub group: Option<String>,
    /// Per-type properties (see the typed structs below; unknown keys ignored, missing = default).
    pub props: Value,
}

impl Default for Widget {
    fn default() -> Self {
        Widget { id: String::new(), kind: "text".into(), x: 0.0, y: 0.0, w: 0.2, h: 0.2, on_base: false, hidden: false, group: None, props: json!({}) }
    }
}

impl Widget {
    pub fn new(id: &str, kind: &str, x: f32, y: f32, w: f32, h: f32, props: Value) -> Widget {
        Widget { id: id.into(), kind: kind.into(), x, y, w, h, on_base: false, hidden: false, group: None, props }
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
    pub fn sensor(&self) -> SensorProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn date(&self) -> DateProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn weather(&self) -> WeatherProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn visualizer(&self) -> VisualizerProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn slideshow(&self) -> SlideshowProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn info(&self) -> InfoProps {
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
pub struct Group {
    pub id: String,
    pub name: String,
    pub hidden: bool,
    pub collapsed: bool,
}

impl Default for Group {
    fn default() -> Self {
        Group { id: String::new(), name: String::new(), hidden: false, collapsed: false }
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
    /// "digital" | "analog" | "minimal"
    pub style: String,
}
impl Default for ClockProps {
    fn default() -> Self {
        ClockProps { format: "24h".into(), seconds: false, date: true, color: "#ffffff".into(), style: "digital".into() }
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


// ---------------------------------------------------------------- v1.6 widget props

/// One colour step of a sensor: from value `at` upward the colour is `color` ("#rrggbb").
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Threshold {
    pub at: f32,
    pub color: String,
}
impl Default for Threshold {
    fn default() -> Self {
        Threshold { at: 0.0, color: "#ffffff".into() }
    }
}

/// Metrics a `sensor` widget can show.
pub const SENSOR_METRICS: [&str; 20] = [
    "cpu.load", "cpu.core", "cpu.temp", "cpu.clock", "gpu.load", "gpu.temp", "gpu.clock", "gpu.vram", "gpu.power", "gpu.fan", "ram.used", "net.down",
    "net.up", "disk.used", "disk.activity", "cpu.power", "cpu.fan", "board.temp", "disk.read", "disk.write",
];
pub const SENSOR_STYLES: [&str; 6] = ["number", "bar", "ring", "gauge", "graph", "sparkline"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SensorProps {
    pub metric: String,
    /// cpu.core: which logical core (0-based).
    pub core: u32,
    /// disk.used: drive, e.g. "C:".
    pub drive: String,
    /// net.*: adapter name; None = the busiest.
    pub adapter: Option<String>,
    /// "number" | "bar" | "ring" | "gauge" | "graph" | "sparkline"
    pub style: String,
    /// None = the metric's default name, "" = no label.
    pub label: Option<String>,
    /// "C" | "F" (temperatures).
    pub unit: String,
    pub decimals: Option<u32>,
    /// Scale ends for bar/ring/gauge/graph; None = automatic per metric.
    pub min: Option<f32>,
    pub max: Option<f32>,
    /// None = the metric's default colour steps; Some(empty) = none.
    pub thresholds: Option<Vec<Threshold>>,
    /// Base colour; None = per metric default.
    pub color: Option<String>,
    /// Seconds of history for graph / sparkline.
    pub history: f32,
    /// Sampling rate in Hz: 1, 2 or 4.
    pub rate: u32,
}
impl Default for SensorProps {
    fn default() -> Self {
        SensorProps {
            metric: "cpu.load".into(),
            core: 0,
            drive: "C:".into(),
            adapter: None,
            style: "number".into(),
            label: None,
            unit: "C".into(),
            decimals: None,
            min: None,
            max: None,
            thresholds: None,
            color: None,
            history: 60.0,
            rate: 1,
        }
    }
}
impl SensorProps {
    /// History seconds 5..600, rate 1/2/4.
    pub fn history_secs(&self) -> f32 {
        if self.history.is_finite() { self.history.clamp(5.0, 600.0) } else { 60.0 }
    }
    pub fn rate_hz(&self) -> u32 {
        match self.rate {
            0 | 1 => 1,
            2 | 3 => 2,
            _ => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DateProps {
    /// "line" | "calendar" | "big"
    pub style: String,
    pub color: String,
}
impl Default for DateProps {
    fn default() -> Self {
        DateProps { style: "line".into(), color: "#ffffff".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WeatherProps {
    /// "now" | "forecast"
    pub style: String,
    /// "C" | "F"
    pub unit: String,
    /// forecast: "days" | "hours"
    pub span: String,
    /// forecast: entries after now (days 3..5, hours 3..6; clamped).
    pub count: u32,
    pub color: String,
}
impl Default for WeatherProps {
    fn default() -> Self {
        WeatherProps { style: "now".into(), unit: "C".into(), span: "days".into(), count: 4, color: "#ffffff".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VisualizerProps {
    /// 16..64
    pub bands: u32,
    /// "bars-h" | "ring" | "wave-fade"
    pub style: String,
    pub color: String,
    /// 0.2..4
    pub sensitivity: f32,
}
impl Default for VisualizerProps {
    fn default() -> Self {
        VisualizerProps { bands: 32, style: "bars-h".into(), color: "#5ac8ff".into(), sensitivity: 1.0 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SlideshowProps {
    /// Paths relative to the theme folder (png, jpg, gif...).
    pub files: Vec<String>,
    /// Seconds per image (1..3600).
    pub seconds: f32,
    /// Cross-fade seconds (0..5).
    pub fade: f32,
    /// "contain" | "cover"
    pub fit: String,
}
impl Default for SlideshowProps {
    fn default() -> Self {
        SlideshowProps { files: Vec::new(), seconds: 8.0, fade: 1.0, fit: "cover".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct InfoProps {
    /// "uptime" | "cpuName" | "gpuName" | "countdown" | "topProcesses"
    pub kind: String,
    /// countdown: ISO local time "2026-12-25T00:00" (seconds optional).
    pub target: Option<String>,
    /// countdown: caption (None = "Countdown").
    pub label: Option<String>,
    /// topProcesses: "cpu" | "ram"
    pub by: String,
    /// topProcesses: 3..5
    pub count: u32,
    pub color: String,
}
impl Default for InfoProps {
    fn default() -> Self {
        InfoProps { kind: "uptime".into(), target: None, label: None, by: "cpu".into(), count: 4, color: "#ffffff".into() }
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


// ---------------------------------------------------------------- screen guides (v1.2)

/// Canvas the guide pixel values refer to.
pub const GUIDE_W: f32 = 2400.0;
pub const GUIDE_H: f32 = 1080.0;

/// A band along the left or right edge (px wide).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Side {
    pub size: f32,
}

/// The vertical curve band: `offset` px from the canvas centre (negative = left); `left`/`right` = px from that centre
/// line to the band's left/right edge.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CurveBand {
    pub offset: f32,
    pub left: f32,
    pub right: f32,
}

/// Global screen guides (settings.json `screenGuides`, v1.4). Guides are hints, never masks.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Guides {
    pub left: Side,
    pub right: Side,
    pub curve: CurveBand,
}

/// Per-theme offsets (theme.json `guidesOffset`): same keys as `Guides`, values are deltas and may be negative.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct GuidesOffset {
    pub left: Side,
    pub right: Side,
    pub curve: CurveBand,
}

fn num(v: Option<&Value>) -> f32 {
    v.and_then(|v| v.as_f64()).map(|f| f as f32).filter(|f| f.is_finite()).unwrap_or(0.0)
}

/// Reads `{left:{size}, right:{size}, curve:{offset,left,right}}`. v1.2 shapes (curve edges top/bottom, `zones`) are migrated:
/// the curve edges are dropped, a right-edge zone becomes `right.size`, a left-edge zone `left.size`.
fn parse_parts(v: &Value, deltas: bool) -> (Side, Side, CurveBand) {
    let side = |k: &str| Side { size: num(v.get(k).and_then(|s| s.get("size"))) };
    let (mut left, mut right) = (side("left"), side("right"));
    let c = v.get("curve").filter(|c| c.is_object());
    let old_curve = c.is_some_and(|c| c.get("top").is_some() || c.get("bottom").is_some())
        || v.get("zones").is_some()
        || (!deltas && c.is_some_and(|c| c.get("offset").is_none()) && v.get("left").is_none() && v.get("right").is_none());
    let curve = match c {
        Some(c) if !old_curve => CurveBand { offset: num(c.get("offset")), left: num(c.get("left")), right: num(c.get("right")) },
        _ => CurveBand::default(),
    };
    if !deltas {
        // v1.2 global: an array of zones. (Per-theme v1.2 zone offsets were keyed by id and are dropped.)
        for z in v.get("zones").and_then(|z| z.as_array()).into_iter().flatten() {
            let size = num(z.get("size"));
            match z.get("edge").and_then(|e| e.as_str()) {
                Some("left") if left.size == 0.0 => left.size = size,
                Some("right") | None if right.size == 0.0 => right.size = size,
                _ => {}
            }
        }
    }
    (left, right, curve)
}

impl<'de> Deserialize<'de> for Guides {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Guides::from_value(&Value::deserialize(d)?))
    }
}

impl<'de> Deserialize<'de> for GuidesOffset {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(GuidesOffset::from_value(&Value::deserialize(d)?))
    }
}

impl GuidesOffset {
    pub fn from_value(v: &Value) -> GuidesOffset {
        let (left, right, curve) = parse_parts(v, true);
        let mut o = GuidesOffset { left, right, curve };
        o.clamp();
        o
    }
    /// Finite deltas within +-the canvas.
    pub fn clamp(&mut self) {
        let fix = |v: &mut f32| *v = if v.is_finite() { v.clamp(-GUIDE_W, GUIDE_W) } else { 0.0 };
        fix(&mut self.left.size);
        fix(&mut self.right.size);
        fix(&mut self.curve.offset);
        fix(&mut self.curve.left);
        fix(&mut self.curve.right);
    }
}

impl Guides {
    pub fn from_value(v: &Value) -> Guides {
        let (left, right, curve) = parse_parts(v, false);
        let mut g = Guides { left, right, curve };
        g.clamp();
        g
    }

    /// Finite values; sides 0..half the canvas; the curve centre inside the canvas and its band kept inside the canvas.
    pub fn clamp(&mut self) {
        let fix = |v: &mut f32, lo: f32, hi: f32| *v = if v.is_finite() { v.clamp(lo, hi) } else { 0.0f32.clamp(lo, hi) };
        let half = GUIDE_W / 2.0;
        fix(&mut self.left.size, 0.0, half);
        fix(&mut self.right.size, 0.0, half);
        fix(&mut self.curve.offset, -half, half);
        let centre = half + self.curve.offset;
        fix(&mut self.curve.left, 0.0, centre);
        fix(&mut self.curve.right, 0.0, GUIDE_W - centre);
    }

    /// The guides in effect for a theme: global values plus the theme's offsets, clamped.
    pub fn effective(&self, off: &GuidesOffset) -> Guides {
        let mut g = self.clone();
        g.left.size += off.left.size;
        g.right.size += off.right.size;
        g.curve.offset += off.curve.offset;
        g.curve.left += off.curve.left;
        g.curve.right += off.curve.right;
        g.clamp();
        g
    }

    /// The curve band's left and right edge x on the 2400 canvas.
    pub fn curve_edges(&self) -> (f32, f32) {
        let c = GUIDE_W / 2.0 + self.curve.offset;
        (c - self.curve.left, c + self.curve.right)
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
        self.corners = self.corners.filter(|c| c.is_finite()).map(|c| c.clamp(0.0, 300.0));
        self.gap = if self.gap.is_finite() { self.gap.clamp(0.0, 200.0) } else { DEFAULT_GAP };
        self.guides_offset.clamp();
        if let Some(b) = self.background.as_mut() {
            b.clamp();
        }
        self.handover.fade_in = if self.handover.fade_in.is_finite() { self.handover.fade_in.clamp(0.0, 3.0) } else { 0.8 };
        let mut gids = std::collections::HashSet::new();
        for (i, g) in self.groups.iter_mut().enumerate() {
            if g.id.trim().is_empty() {
                g.id = format!("g{}", i + 1);
            }
            while !gids.insert(g.id.clone()) {
                g.id.push('_');
            }
        }
        for w in self.widgets.iter_mut() {
            if w.group.as_ref().is_some_and(|g| !gids.contains(g)) {
                w.group = None;
            }
        }
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
            if !WIDGET_TYPES.contains(&w.kind.as_str()) && !super::morewidgets::is_more(&w.kind) {
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
    /// Drawn at all: the widget itself and its group are not hidden.
    pub fn is_visible(&self, w: &Widget) -> bool {
        !w.hidden && !w.group.as_ref().is_some_and(|g| self.groups.iter().any(|x| &x.id == g && x.hidden))
    }

    /// The theme draws something live: the live layer is on and at least one visible item is not just part of the stored base
    /// (`base_stored`: the background is the stored base, which already contains its `onBase` items).
    pub fn has_live_items(&self, base_stored: bool) -> bool {
        self.live_layer && self.widgets.iter().any(|w| self.is_visible(w) && !(base_stored && w.on_base && w.can_be_on_base()))
    }

    /// The live effect that replaces the picture as the background, if the theme has one.
    pub fn effect(&self) -> Option<super::effects::Params> {
        self.background.as_ref().and_then(|b| b.params())
    }

    pub fn has_base_items(&self) -> bool {
        self.widgets.iter().any(|w| w.on_base && w.can_be_on_base() && self.is_visible(w))
    }

    /// The part of the theme that shapes the stored base (for change detection: equal = the base needs no re-storing).
    pub fn base_signature(&self) -> String {
        let items: Vec<&Widget> = self.widgets.iter().filter(|w| w.on_base && w.can_be_on_base() && self.is_visible(w)).collect();
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
    /// The screen fit the base was made with (None: made before the setting existed = none). Live compares it with the current
    /// fit and scales the base picture itself when they differ.
    pub base_fit: Option<crate::core::fit::Fit>,
    /// This theme's base is not the picture the screen holds (Embedded): live must stream its base itself, and when live stops it
    /// hands over instantly (the stored picture is another one, so a fade or a first-frame cross-fade would be wrong).
    pub not_embedded: bool,
}

impl ThemeLive {
    /// Live has nothing to add to the screen's own loop: no live items, and the stored base is fitted the way the screen fit is
    /// now (otherwise streaming is what applies the fit). The engine then need not stream at all.
    pub fn nothing_to_stream(&self, fit: crate::core::fit::Fit) -> bool {
        self.theme.live_layer && !self.not_embedded && self.theme.effect().is_none() && self.base_video.is_some() && !self.theme.has_live_items(true) && self.base_fit.unwrap_or_default() == fit
    }

    /// Live stops with no fade onto the stored loop: the stored picture is not this theme's (another theme, or an effect that is
    /// not recorded yet), so a cross-fade to its first frame would be wrong.
    pub fn instant_handover(&self) -> bool {
        self.not_embedded || self.theme.effect().is_some()
    }

    pub fn has_live_items(&self) -> bool {
        (self.theme.live_layer && self.theme.effect().is_some()) || self.theme.has_live_items(self.base_video.is_some())
    }

    pub fn new(theme: Theme, theme_dir: PathBuf) -> ThemeLive {
        ThemeLive { theme, theme_dir, base_video: None, fallback_video: None, base_fit: None, not_embedded: false }
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

    #[test]
    fn v1_1_layers_groups_and_live_layer() {
        let t = Theme::from_json(r#"{"widgets":[{"id":"a","type":"text","group":"g1","onBase":true},{"id":"b","type":"text","hidden":true,"onBase":true},{"id":"c","type":"text","group":"nope"}],
            "groups":[{"id":"g1","name":"One","hidden":true},{"name":"x"}]}"#).unwrap();
        assert!(t.live_layer);
        assert_eq!(t.groups[1].id, "g2");
        assert!(!t.is_visible(&t.widgets[0]), "hidden group hides members");
        assert!(!t.is_visible(&t.widgets[1]));
        assert_eq!(t.widgets[2].group, None);
        assert!(t.is_visible(&t.widgets[2]));
        assert!(!t.has_base_items(), "hidden widgets are not baked");
        let off = Theme::from_json(r#"{"liveLayer":false}"#).unwrap();
        assert!(!off.live_layer && off.to_json().contains("\"liveLayer\": false"));
    }

    #[test]
    fn guides_parse_migrate_effective() {
        let g = Guides::from_value(&json!({ "left": { "size": 160 }, "right": { "size": 180 }, "curve": { "offset": -100, "left": 60, "right": 70 } }));
        assert_eq!((g.left.size, g.right.size, g.curve.offset, g.curve.left, g.curve.right), (160.0, 180.0, -100.0, 60.0, 70.0));
        assert_eq!(g.curve_edges(), (1040.0, 1170.0));
        // v1.2 migration: curve edges dropped, zones -> sides
        let m = Guides::from_value(&json!({ "curve": { "left": 10, "right": 90, "top": 0, "bottom": 0 }, "zones": [{ "edge": "right", "size": 180 }, { "edge": "left", "size": 120 }, { "edge": "top", "size": 50 }] }));
        assert_eq!((m.left.size, m.right.size), (120.0, 180.0));
        assert_eq!(m.curve, CurveBand::default());
        // old curve {left,right} without zones is also dropped
        assert_eq!(Guides::from_value(&json!({ "curve": { "right": 90 } })).curve, CurveBand::default());
        // clamping: band stays inside the canvas, garbage becomes 0
        let c = Guides::from_value(&json!({ "left": { "size": -5 }, "right": { "size": 99999 }, "curve": { "offset": 1100, "left": 5000, "right": 5000 } }));
        assert_eq!((c.left.size, c.right.size), (0.0, 1200.0));
        let (l, r) = c.curve_edges();
        assert!(l >= 0.0 && r <= 2400.0, "{l} {r}");
        assert_eq!(Guides::from_value(&json!("junk")), Guides::default());
        // effective = global + theme offset, clamped
        let t = Theme::from_json(r#"{"guidesOffset":{"left":{"size":-200},"right":{"size":30},"curve":{"offset":50,"right":-90}},"corners":999,"gap":5000}"#).unwrap();
        assert_eq!((t.corners, t.gap), (Some(300.0), 200.0));
        let e = g.effective(&t.guides_offset);
        assert_eq!(e.left.size, 0.0, "clamped at 0");
        assert_eq!(e.right.size, 210.0);
        assert_eq!((e.curve.offset, e.curve.left, e.curve.right), (-50.0, 60.0, 0.0));
        // old per-theme offsets (zones by id) are dropped harmlessly
        let old = Theme::from_json(r#"{"guidesOffset":{"curve":{"top":3},"zones":{"z1":30}}}"#).unwrap();
        assert_eq!(old.guides_offset, GuidesOffset::default());
        assert_eq!(Theme::from_json(r#"{"corners":null}"#).unwrap().corners, None);
        assert_eq!(Theme::from_json(r#"{"corners":0}"#).unwrap().corners, Some(0.0));
        assert_eq!(Theme::from_json("{}").unwrap().gap, 24.0);
        assert_eq!(Theme::from_json(r#"{"gap":-3}"#).unwrap().gap, 0.0);
        // round trip
        let back = Theme::from_json(&t.to_json()).unwrap();
        assert_eq!(back.guides_offset, t.guides_offset);
    }

    #[test]
    fn v1_6_props_parse_with_defaults() {
        let t = Theme::from_json(r##"{"widgets":[
            {"id":"s","type":"sensor","props":{"metric":"gpu.temp","style":"gauge","unit":"F","thresholds":[{"at":70,"color":"#f0b43c"}],"rate":3,"history":9999}},
            {"id":"c","type":"clock","props":{"style":"analog"}},
            {"id":"d","type":"date","props":{"style":"calendar"}},
            {"id":"w","type":"weather","props":{"style":"forecast","count":5}},
            {"id":"v","type":"visualizer","props":{"bands":48,"style":"ring"}},
            {"id":"p","type":"slideshow","props":{"files":["a.png","b.gif"],"seconds":5}},
            {"id":"i","type":"info","props":{"kind":"countdown","target":"2026-12-25T00:00"}}]}"##).unwrap();
        assert!(t.validate().is_empty(), "{:?}", t.validate());
        let s = t.widgets[0].sensor();
        assert_eq!((s.metric.as_str(), s.style.as_str(), s.unit.as_str()), ("gpu.temp", "gauge", "F"));
        assert_eq!(s.thresholds.as_ref().unwrap()[0].at, 70.0);
        assert_eq!((s.rate_hz(), s.history_secs()), (2, 600.0));
        assert_eq!(SensorProps::default().thresholds, None);
        assert_eq!(t.widgets[1].clock().style, "analog");
        assert_eq!(ClockProps::default().style, "digital");
        assert_eq!(t.widgets[2].date().style, "calendar");
        assert_eq!(t.widgets[3].weather().count, 5);
        assert_eq!(t.widgets[4].visualizer().bands, 48);
        assert_eq!(t.widgets[5].slideshow().files.len(), 2);
        assert_eq!(t.widgets[6].info().target.as_deref(), Some("2026-12-25T00:00"));
        assert_eq!(Widget::default().sensor(), SensorProps::default());
    }
}
