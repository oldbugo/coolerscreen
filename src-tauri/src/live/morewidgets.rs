//! 4.0 stream B: the second batch of widget types (docs/v3/theme-format.md, "4.0 widgets"): `meters`, `cores`, `network`,
//! `disks`, `worldclock`, `timer`. This file holds their props (lenient typed views of `Widget.props`), the cleaning the app
//! applies to what the editor sends, the Windows time-zone lookup for world clocks, and the timer arithmetic. The drawing is
//! in `render/more.rs`. Nothing here changes an existing widget type.
use super::theme::{Widget, SENSOR_METRICS};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

pub const TYPES: [&str; 8] = ["meters", "cores", "network", "disks", "worldclock", "timer", "livetext", "volume"];

pub fn is_more(kind: &str) -> bool {
    TYPES.contains(&kind)
}

pub const MAX_ROWS: usize = 8;
pub const MAX_ZONES: usize = 4;

// ---------------------------------------------------------------- props

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MeterRow {
    pub metric: String,
    /// None = the metric's default name, "" = none.
    pub label: Option<String>,
    pub core: u32,
    pub drive: String,
}
impl Default for MeterRow {
    fn default() -> Self {
        MeterRow { metric: "cpu.load".into(), label: None, core: 0, drive: "C:".into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MetersProps {
    pub rows: Vec<MeterRow>,
    /// "C" | "F" (temperature rows).
    pub unit: String,
    /// Draw the bar under each row.
    pub bars: bool,
    /// Sampling rate in Hz: 1, 2 or 4.
    pub rate: u32,
}
impl Default for MetersProps {
    fn default() -> Self {
        let row = |m: &str| MeterRow { metric: m.into(), ..MeterRow::default() };
        MetersProps { rows: vec![row("cpu.load"), row("ram.used"), row("gpu.load"), row("gpu.temp")], unit: "C".into(), bars: true, rate: 1 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CoresProps {
    /// "bars" | "heat"
    pub style: String,
    /// Average load as a big number in the header.
    pub average: bool,
    pub color: Option<String>,
    pub rate: u32,
}
impl Default for CoresProps {
    fn default() -> Self {
        CoresProps { style: "bars".into(), average: true, color: None, rate: 2 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NetworkProps {
    pub graph: bool,
    /// A footer with the data moved since the adapters came up (about: since the PC started).
    pub totals: bool,
    /// Seconds of history in the graph (10..600).
    pub history: f32,
    /// null = the busiest adapter.
    pub adapter: Option<String>,
    pub down_color: String,
    pub up_color: String,
    pub rate: u32,
}
impl Default for NetworkProps {
    fn default() -> Self {
        NetworkProps { graph: true, totals: false, history: 60.0, adapter: None, down_color: "#5ad2a0".into(), up_color: "#aa8cff".into(), rate: 2 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DisksProps {
    /// Drive letters like "C:"; empty = every fixed drive.
    pub drives: Vec<String>,
    /// "used" | "free"
    pub show: String,
    /// 2..6
    pub max_rows: u32,
}
impl Default for DisksProps {
    fn default() -> Self {
        DisksProps { drives: Vec::new(), show: "used".into(), max_rows: 4 }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Zone {
    pub name: String,
    /// Windows time-zone id ("Tokyo Standard Time"); empty = this PC's own zone.
    pub tz: String,
}
impl Default for Zone {
    fn default() -> Self {
        Zone { name: String::new(), tz: String::new() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorldClockProps {
    pub zones: Vec<Zone>,
    /// "24h" | "12h"
    pub format: String,
    pub color: String,
}
impl Default for WorldClockProps {
    fn default() -> Self {
        let z = |n: &str, t: &str| Zone { name: n.into(), tz: t.into() };
        WorldClockProps {
            zones: vec![z("New York", "Eastern Standard Time"), z("London", "GMT Standard Time"), z("Tokyo", "Tokyo Standard Time")],
            format: "24h".into(),
            color: "#ffffff".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TimerProps {
    /// "stopwatch" | "timer" | "pomodoro"
    pub mode: String,
    /// Caption (None = the mode's name).
    pub label: Option<String>,
    pub running: bool,
    /// Wall-clock start (ms since 1970) of the current run; only meaningful while running.
    pub started_at: f64,
    /// Time collected before the current run (ms).
    pub elapsed_ms: f64,
    /// timer: length in seconds.
    pub duration: f64,
    /// pomodoro: focus and break length in minutes.
    pub work_min: f64,
    pub break_min: f64,
    pub color: String,
}
impl Default for TimerProps {
    fn default() -> Self {
        TimerProps { mode: "stopwatch".into(), label: None, running: false, started_at: 0.0, elapsed_ms: 0.0, duration: 300.0, work_min: 25.0, break_min: 5.0, color: "#ffffff".into() }
    }
}

/// What a new "Live text" says until the user types their own.
pub const DEFAULT_LIVETEXT: &str = "{time}  CPU {cpu}  GPU {gpu}";

/// Words usable in a live text: (word, what it shows). The editor lists the same words.
pub const WORDS: [(&str, &str); 19] = [
    ("time", "Time (24-hour)"),
    ("time12", "Time (12-hour)"),
    ("seconds", "Time with seconds"),
    ("date", "Date"),
    ("weekday", "Weekday"),
    ("cpu", "CPU load"),
    ("cputemp", "CPU temperature"),
    ("cpupower", "CPU power"),
    ("cpufan", "CPU fan"),
    ("gpu", "GPU load"),
    ("gputemp", "GPU temperature"),
    ("vram", "GPU memory"),
    ("ram", "RAM used"),
    ("down", "Download speed"),
    ("up", "Upload speed"),
    ("diskread", "Disk read speed"),
    ("diskwrite", "Disk write speed"),
    ("uptime", "Uptime"),
    ("volume", "Volume"),
];

/// Replace `{word}` with `value(word)`; a word the resolver does not know is left as typed, a missing value shows a dash.
pub fn expand(text: &str, mut value: impl FnMut(&str) -> Option<Option<String>>) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('}') {
            Some(j) if after[..j].chars().all(|c| c.is_ascii_alphanumeric()) && j > 0 => {
                match value(&after[..j]) {
                    Some(Some(v)) => out.push_str(&v),
                    Some(None) => out.push('\u{2013}'),
                    None => out.push_str(&rest[i..i + j + 2]),
                }
                rest = &after[j + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}


#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct VolumeProps {
    /// "bar" | "ring"
    pub style: String,
    pub color: String,
}
impl Default for VolumeProps {
    fn default() -> Self {
        VolumeProps { style: "bar".into(), color: "#5ac8ff".into() }
    }
}

impl Widget {
    pub fn volume(&self) -> VolumeProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn livetext(&self) -> super::theme::TextProps {
        let mut p = self.props.clone();
        if let Some(o) = p.as_object_mut() {
            o.remove("text");
        }
        serde_json::from_value(p).unwrap_or_default()
    }
    pub fn livetext_source(&self) -> String {
        self.props.get("text").and_then(Value::as_str).unwrap_or(DEFAULT_LIVETEXT).chars().take(200).collect()
    }
    pub fn meters(&self) -> MetersProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn cores(&self) -> CoresProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn network(&self) -> NetworkProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn disks(&self) -> DisksProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn worldclock(&self) -> WorldClockProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
    pub fn timer(&self) -> TimerProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

// ---------------------------------------------------------------- cleaning

fn word(o: &mut Map<String, Value>, key: &str, ok: &[&str], dflt: &str) {
    let v = match o.get(key).and_then(Value::as_str) {
        Some(s) if ok.contains(&s) => s.to_string(),
        _ => dflt.to_string(),
    };
    o.insert(key.into(), json!(v));
}

fn colour(o: &mut Map<String, Value>, key: &str, dflt: &str, nullable: bool) {
    let ok = |s: &str| s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit());
    let v = match o.get(key).and_then(Value::as_str) {
        Some(s) if ok(s) => json!(s.to_lowercase()),
        _ if nullable => Value::Null,
        _ => json!(dflt),
    };
    o.insert(key.into(), v);
}

fn number(o: &mut Map<String, Value>, key: &str, dflt: f64, lo: f64, hi: f64) {
    let x = o.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).unwrap_or(dflt).clamp(lo, hi);
    o.insert(key.into(), json!(x));
}

fn int(o: &mut Map<String, Value>, key: &str, dflt: i64, lo: i64, hi: i64) {
    let x = o.get(key).and_then(Value::as_f64).filter(|x| x.is_finite()).map(|x| x.round() as i64).unwrap_or(dflt).clamp(lo, hi);
    o.insert(key.into(), json!(x));
}

fn boolean(o: &mut Map<String, Value>, key: &str, dflt: bool) {
    let b = o.get(key).and_then(Value::as_bool).unwrap_or(dflt);
    o.insert(key.into(), json!(b));
}

fn short(o: &mut Map<String, Value>, key: &str, max: usize, nullable: bool) {
    let v = match o.get(key) {
        Some(Value::String(s)) => json!(s.chars().take(max).collect::<String>()),
        _ if nullable => Value::Null,
        _ => json!(""),
    };
    o.insert(key.into(), v);
}

fn drive_letter(s: &str) -> Option<String> {
    let t = s.trim();
    let c = t.chars().next().filter(|c| c.is_ascii_alphabetic())?;
    (t.len() == 1 || (t.len() == 2 && t.ends_with(':'))).then(|| format!("{}:", c.to_ascii_uppercase()))
}

/// Clean the props of one of the new types (a no-op for any other type). Idempotent.
pub fn clean(kind: &str, props: &mut Value) {
    if !is_more(kind) {
        return;
    }
    if !props.is_object() {
        *props = json!({});
    }
    let o = props.as_object_mut().unwrap();
    match kind {
        "meters" => {
            let given = o.get("rows").and_then(Value::as_array).cloned();
            let rows: Vec<Value> = match given {
                Some(a) => a
                    .iter()
                    .filter_map(Value::as_object)
                    .take(MAX_ROWS)
                    .map(|r| {
                        let mut r = r.clone();
                        word(&mut r, "metric", &SENSOR_METRICS, "cpu.load");
                        short(&mut r, "label", 24, true);
                        int(&mut r, "core", 0, 0, 255);
                        let d = r.get("drive").and_then(Value::as_str).and_then(drive_letter).unwrap_or_else(|| "C:".into());
                        r.insert("drive".into(), json!(d));
                        Value::Object(r)
                    })
                    .collect(),
                None => serde_json::to_value(MetersProps::default().rows).unwrap().as_array().cloned().unwrap_or_default(),
            };
            o.insert("rows".into(), Value::Array(rows));
            word(o, "unit", &["C", "F"], "C");
            boolean(o, "bars", true);
            int(o, "rate", 1, 1, 4);
        }
        "cores" => {
            word(o, "style", &["bars", "heat"], "bars");
            boolean(o, "average", true);
            colour(o, "color", "#5ac8ff", true);
            int(o, "rate", 2, 1, 4);
        }
        "network" => {
            boolean(o, "graph", true);
            boolean(o, "totals", false);
            number(o, "history", 60.0, 10.0, 600.0);
            short(o, "adapter", 60, true);
            if o.get("adapter").and_then(Value::as_str) == Some("") {
                o.insert("adapter".into(), Value::Null);
            }
            colour(o, "downColor", "#5ad2a0", false);
            colour(o, "upColor", "#aa8cff", false);
            int(o, "rate", 2, 1, 4);
        }
        "disks" => {
            let mut v: Vec<String> = Vec::new();
            for d in o.get("drives").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).filter_map(drive_letter) {
                if !v.contains(&d) && v.len() < 8 {
                    v.push(d);
                }
            }
            o.insert("drives".into(), json!(v));
            word(o, "show", &["used", "free"], "used");
            int(o, "maxRows", 4, 2, 6);
        }
        "worldclock" => {
            let given = o.get("zones").and_then(Value::as_array).cloned();
            let zones: Vec<Value> = match given {
                Some(a) => a
                    .iter()
                    .filter_map(Value::as_object)
                    .take(MAX_ZONES)
                    .map(|z| {
                        let mut z = z.clone();
                        short(&mut z, "name", 24, false);
                        short(&mut z, "tz", 60, false);
                        // a zone id is plain words; anything with path-like characters is dropped
                        if z.get("tz").and_then(Value::as_str).is_some_and(|t| !t.chars().all(|c| c.is_ascii_alphanumeric() || " .+-_()".contains(c))) {
                            z.insert("tz".into(), json!(""));
                        }
                        Value::Object(z)
                    })
                    .collect(),
                None => serde_json::to_value(WorldClockProps::default().zones).unwrap().as_array().cloned().unwrap_or_default(),
            };
            o.insert("zones".into(), Value::Array(zones));
            word(o, "format", &["24h", "12h"], "24h");
            colour(o, "color", "#ffffff", false);
        }
        "volume" => {
            word(o, "style", &["bar", "ring"], "bar");
            colour(o, "color", "#5ac8ff", false);
        }
        "livetext" => {
            if !o.get("text").is_some_and(Value::is_string) {
                o.insert("text".into(), json!(DEFAULT_LIVETEXT));
            }
            short(o, "text", 200, false);
            number(o, "size", 0.08, 0.02, 0.3);
            colour(o, "color", "#ffffff", false);
            number(o, "opacity", 1.0, 0.0, 1.0);
            word(o, "align", &["left", "center", "right"], "left");
            word(o, "weight", &["light", "regular", "bold"], "regular");
        }
        "timer" => {
            word(o, "mode", &["stopwatch", "timer", "pomodoro"], "stopwatch");
            short(o, "label", 30, true);
            boolean(o, "running", false);
            number(o, "startedAt", 0.0, 0.0, 1e14);
            number(o, "elapsedMs", 0.0, 0.0, 1e10);
            number(o, "duration", 300.0, 1.0, 359_999.0);
            number(o, "workMin", 25.0, 1.0, 180.0);
            number(o, "breakMin", 5.0, 1.0, 60.0);
            colour(o, "color", "#ffffff", false);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- timer arithmetic

pub fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

/// What a timer shows at `now` (ms since 1970).
#[derive(Clone, Debug, PartialEq)]
pub struct TimerView {
    /// Seconds shown (counts up for a stopwatch, down for a timer / pomodoro phase).
    pub shown_secs: f64,
    /// 0..1 progress of the timer or pomodoro phase (stopwatch: seconds into the current minute).
    pub fraction: f32,
    /// "Paused" | "Running" | "Done" | "Focus" | "Break" | "Ready"
    pub state: &'static str,
    pub done: bool,
}

pub fn timer_view(p: &TimerProps, now: f64) -> TimerView {
    let run = if p.running { (now - p.started_at).max(0.0) } else { 0.0 };
    let elapsed = (p.elapsed_ms + run).max(0.0) / 1000.0;
    let fresh = !p.running && p.elapsed_ms <= 0.0;
    match p.mode.as_str() {
        "timer" => {
            let left = (p.duration - elapsed).max(0.0);
            let done = elapsed >= p.duration;
            TimerView {
                shown_secs: left,
                fraction: (elapsed / p.duration.max(1.0)).clamp(0.0, 1.0) as f32,
                state: if done { "Done" } else if p.running { "Running" } else if fresh { "Ready" } else { "Paused" },
                done,
            }
        }
        "pomodoro" => {
            let (w, b) = (p.work_min * 60.0, p.break_min * 60.0);
            let at = elapsed % (w + b);
            let (len, into, state) = if at < w { (w, at, "Focus") } else { (b, at - w, "Break") };
            TimerView { shown_secs: len - into, fraction: (into / len).clamp(0.0, 1.0) as f32, state: if fresh { "Ready" } else if p.running { state } else { "Paused" }, done: false }
        }
        _ => TimerView { shown_secs: elapsed, fraction: ((elapsed % 60.0) / 60.0) as f32, state: if p.running { "Running" } else if fresh { "Ready" } else { "Paused" }, done: false },
    }
}

/// "07:05", "1:02:03" (the hours show only when needed).
pub fn clock_text(secs: f64) -> String {
    let s = secs.max(0.0).ceil() as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m:02}:{s:02}") }
}

// ---------------------------------------------------------------- time zones

/// Local calendar time somewhere.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoneTime {
    pub year: u16,
    pub month: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
}

/// Days since 1970-01-01 of a civil date.
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

impl ZoneTime {
    pub fn days(&self) -> i64 {
        days_from_civil(self.year as i64, self.month as i64, self.day as i64)
    }
}

type Tzi = windows_sys::Win32::System::Time::TIME_ZONE_INFORMATION;

fn tz_cache() -> &'static Mutex<HashMap<String, Option<Tzi>>> {
    static C: OnceLock<Mutex<HashMap<String, Option<Tzi>>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The zone's rules from the registry (`TZI` value of HKLM\...\Time Zones\<id>): no network, no admin.
fn load_tzi(id: &str) -> Option<Tzi> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_BINARY};
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let key = wide(&format!("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Time Zones\\{id}"));
    let mut buf = [0u8; 44];
    let mut len = buf.len() as u32;
    let st = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), wide("TZI").as_ptr(), RRF_RT_REG_BINARY, std::ptr::null_mut(), buf.as_mut_ptr() as *mut std::ffi::c_void, &mut len) };
    if st != 0 || len != 44 {
        return None;
    }
    let i32_at = |o: usize| i32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
    let st_at = |o: usize| {
        let w = |k: usize| u16::from_le_bytes(buf[o + k * 2..o + k * 2 + 2].try_into().unwrap());
        windows_sys::Win32::Foundation::SYSTEMTIME { wYear: w(0), wMonth: w(1), wDayOfWeek: w(2), wDay: w(3), wHour: w(4), wMinute: w(5), wSecond: w(6), wMilliseconds: w(7) }
    };
    let mut t: Tzi = unsafe { std::mem::zeroed() };
    t.Bias = i32_at(0);
    t.StandardBias = i32_at(4);
    t.DaylightBias = i32_at(8);
    t.StandardDate = st_at(12);
    t.DaylightDate = st_at(28);
    Some(t)
}

/// Local time in the Windows zone `id` now; an empty id is this PC's own zone, an unknown id gives None.
pub fn zone_time_now(id: &str) -> Option<ZoneTime> {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
    use windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
    let mut out: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if id.trim().is_empty() {
        unsafe { GetLocalTime(&mut out) };
    } else {
        let tzi = {
            let mut c = tz_cache().lock().unwrap();
            *c.entry(id.to_string()).or_insert_with(|| load_tzi(id))
        }?;
        let mut utc: SYSTEMTIME = unsafe { std::mem::zeroed() };
        unsafe { GetSystemTime(&mut utc) };
        if unsafe { SystemTimeToTzSpecificLocalTime(&tzi, &utc, &mut out) } == 0 {
            return None;
        }
    }
    Some(ZoneTime { year: out.wYear, month: out.wMonth, day: out.wDay, hour: out.wHour, minute: out.wMinute, second: out.wSecond })
}

// ---------------------------------------------------------------- drives

/// Fixed drive letters ("C:", ...), re-read at most every 10 seconds.
pub fn fixed_drives() -> Vec<String> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    static C: OnceLock<Mutex<(Option<Instant>, Vec<String>)>> = OnceLock::new();
    let m = C.get_or_init(|| Mutex::new((None, Vec::new())));
    let mut g = m.lock().unwrap();
    if g.0.is_none_or(|t| t.elapsed().as_secs() >= 10) {
        let mask = unsafe { GetLogicalDrives() };
        let mut v = Vec::new();
        for i in 0..26u32 {
            if mask & (1 << i) != 0 {
                let root: Vec<u16> = format!("{}:\\", (b'A' + i as u8) as char).encode_utf16().chain(std::iter::once(0)).collect();
                if unsafe { GetDriveTypeW(root.as_ptr()) } == 3 {
                    v.push(format!("{}:", (b'A' + i as u8) as char));
                }
            }
        }
        *g = (Some(Instant::now()), v);
    }
    g.1.clone()
}

/// Logical processors of this PC.
pub fn logical_cores() -> usize {
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    let mut si: SYSTEM_INFO = unsafe { std::mem::zeroed() };
    unsafe { GetSystemInfo(&mut si) };
    (si.dwNumberOfProcessors as usize).clamp(1, 256)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleaning_bounds_everything() {
        let mut p = json!({ "rows": [{ "metric": "nope", "core": -3, "drive": "d" }, 5, { "metric": "gpu.temp", "label": 4 }], "unit": "K", "rate": 99 });
        clean("meters", &mut p);
        assert_eq!(p["rows"][0]["metric"], "cpu.load");
        assert_eq!((p["rows"][0]["core"].clone(), p["rows"][0]["drive"].clone()), (json!(0), json!("D:")));
        assert_eq!(p["rows"].as_array().unwrap().len(), 2, "non-objects dropped");
        assert!(p["rows"][1]["label"].is_null());
        assert_eq!((p["unit"].clone(), p["rate"].clone()), (json!("C"), json!(4)));
        let many: Vec<Value> = (0..30).map(|_| json!({ "metric": "ram.used" })).collect();
        let mut p = json!({ "rows": many });
        clean("meters", &mut p);
        assert_eq!(p["rows"].as_array().unwrap().len(), MAX_ROWS);
        // missing rows = the default four; an explicit empty list stays empty
        let mut p = json!({});
        clean("meters", &mut p);
        assert_eq!(p["rows"].as_array().unwrap().len(), 4);
        let mut p = json!({ "rows": [] });
        clean("meters", &mut p);
        assert!(p["rows"].as_array().unwrap().is_empty());
        let mut p = json!({ "drives": ["c", "C:", "d:", "zz", 4, "E"], "show": "x", "maxRows": 40 });
        clean("disks", &mut p);
        assert_eq!(p["drives"], json!(["C:", "D:", "E:"]));
        assert_eq!((p["show"].clone(), p["maxRows"].clone()), (json!("used"), json!(6)));
        let mut p = json!({ "zones": [{ "name": "X", "tz": "..\\evil/x" }, { "name": "Tokyo", "tz": "Tokyo Standard Time" }], "format": "9h" });
        clean("worldclock", &mut p);
        assert_eq!(p["zones"][0]["tz"], "");
        assert_eq!(p["zones"][1]["tz"], "Tokyo Standard Time");
        assert_eq!(p["format"], "24h");
        let mut p = json!({ "mode": "alarm", "duration": -4, "running": "yes", "color": "red" });
        clean("timer", &mut p);
        assert_eq!((p["mode"].clone(), p["duration"].clone(), p["running"].clone(), p["color"].clone()), (json!("stopwatch"), json!(1.0), json!(false), json!("#ffffff")));
        let mut once = json!({ "style": "heat", "color": "#ABCDEF" });
        clean("cores", &mut once);
        let again = {
            let mut a = once.clone();
            clean("cores", &mut a);
            a
        };
        assert_eq!(once, again);
        assert_eq!(once["color"], "#abcdef");
        // other types are left alone
        let mut p = json!({ "x": 1 });
        clean("clock", &mut p);
        assert_eq!(p, json!({ "x": 1 }));
    }

    #[test]
    fn live_words_expand() {
        let f = |w: &str| match w {
            "cpu" => Some(Some("34 %".to_string())),
            "cputemp" => Some(None),
            _ => None,
        };
        assert_eq!(expand("CPU {cpu} {cputemp} {nope} {} { cpu } {unclosed", f), "CPU 34 % \u{2013} {nope} {} { cpu } {unclosed");
        assert_eq!(expand("no words", f), "no words");
        assert_eq!(expand("{cpu}{cpu}", f), "34 %34 %");
        let mut p = json!({ "size": 9, "align": "up" });
        clean("livetext", &mut p);
        assert_eq!((p["size"].clone(), p["align"].clone(), p["text"].clone()), (json!(0.3), json!("left"), json!(DEFAULT_LIVETEXT)));
        let mut p = json!({ "text": "hello {time}" });
        clean("livetext", &mut p);
        assert_eq!(p["text"], "hello {time}");
    }


    #[test]
    fn timer_modes() {
        let mut p = TimerProps { mode: "stopwatch".into(), running: true, started_at: 1000.0, elapsed_ms: 5000.0, ..TimerProps::default() };
        let v = timer_view(&p, 4000.0);
        assert_eq!((v.shown_secs, v.state), (8.0, "Running"));
        p.running = false;
        assert_eq!(timer_view(&p, 99999.0).shown_secs, 5.0, "paused: no time passes");
        assert_eq!(timer_view(&TimerProps::default(), 5.0).state, "Ready");
        let t = TimerProps { mode: "timer".into(), duration: 60.0, running: true, started_at: 0.0, ..TimerProps::default() };
        let v = timer_view(&t, 15_000.0);
        assert_eq!((v.shown_secs, v.done), (45.0, false));
        assert!((v.fraction - 0.25).abs() < 1e-6);
        let v = timer_view(&t, 61_000.0);
        assert_eq!((v.shown_secs, v.done, v.state), (0.0, true, "Done"));
        let pm = TimerProps { mode: "pomodoro".into(), running: true, started_at: 0.0, ..TimerProps::default() };
        assert_eq!(timer_view(&pm, 60_000.0).state, "Focus");
        let v = timer_view(&pm, (25 * 60 + 30) as f64 * 1000.0);
        assert_eq!((v.state, v.shown_secs), ("Break", 270.0));
        assert_eq!(timer_view(&pm, (30 * 60 + 10) as f64 * 1000.0).state, "Focus", "the cycle repeats");
        assert_eq!(clock_text(65.0), "01:05");
        assert_eq!(clock_text(3723.0), "1:02:03");
        assert_eq!(clock_text(0.2), "00:01");
    }

    #[test]
    fn civil_days_and_zones() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(2000, 3, 1), 11017);
        let local = zone_time_now("").unwrap();
        assert!(local.year >= 2024);
        // Tokyo is UTC+9 all year: compare with UTC via two known zones
        let tokyo = zone_time_now("Tokyo Standard Time");
        let utc = zone_time_now("UTC");
        if let (Some(t), Some(u)) = (tokyo, utc) {
            let mins = |z: ZoneTime| z.days() * 1440 + z.hour as i64 * 60 + z.minute as i64;
            let d = mins(t) - mins(u);
            assert!((539..=541).contains(&d), "Tokyo is 9 h ahead of UTC, got {d} min");
        }
        assert!(zone_time_now("No Such Zone").is_none());
    }

    #[test]
    fn drives_and_cores_exist() {
        assert!(!fixed_drives().is_empty());
        assert!(logical_cores() >= 1);
    }
}
