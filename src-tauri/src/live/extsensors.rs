//! 4.0 stream B2: readings the PC cannot give without a kernel driver (CPU temperature, CPU power, CPU fan, board temperature),
//! taken from a monitoring tool the USER already runs. We never install, start or elevate anything:
//!   1. HWiNFO shared memory (`Global\HWiNFO_SENS_SM2`; HWiNFO > Settings > "Shared Memory Support"; read-only mapping),
//!   2. LibreHardwareMonitor's web server (`http://127.0.0.1:8085/data.json`; Options > "Remote Web Server").
//! The sources are only looked at while a widget shows one of these metrics (the sampler asks), HWiNFO about every 2 s, the web
//! server about every 15 s when it is not answering. `status()` says in plain words why a value is missing.
use crate::core::sync::MutexExt;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const METRICS: [&str; 4] = ["cpu.temp", "cpu.power", "cpu.fan", "board.temp"];

pub fn is_ext(metric: &str) -> bool {
    METRICS.contains(&metric)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Cat {
    Temp,
    Fan,
    Power,
    /// Frame-rate / frame-time sensors (HWiNFO PresentMon integration); never used by the four metrics.
    Fps,
    Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub cat: Cat,
    pub label: String,
    /// Sensor / hardware name the reading belongs to (HWiNFO sensor name, LHM hardware path).
    pub group: String,
    pub value: f64,
}

// ---------------------------------------------------------------- choosing

fn has(s: &str, words: &[&str]) -> bool {
    let l = s.to_lowercase();
    words.iter().any(|w| l.contains(w))
}

/// The reading that answers `metric` (None when the tool has no suitable one).
pub fn pick(metric: &str, all: &[Reading]) -> Option<f32> {
    let best = |cat: Cat, prefs: &[&[&str]], group_hint: &[&str]| -> Option<f32> {
        let cands: Vec<&Reading> = all.iter().filter(|r| r.cat == cat && r.value.is_finite()).collect();
        for words in prefs {
            // a preferred label, first inside a CPU group, then anywhere
            if let Some(r) = cands.iter().find(|r| has(&r.label, words) && (group_hint.is_empty() || has(&r.group, group_hint))).or_else(|| cands.iter().find(|r| has(&r.label, words))) {
                return Some(r.value as f32);
            }
        }
        None
    };
    match metric {
        "cpu.temp" => best(Cat::Temp, &[&["tctl", "tdie"], &["cpu die"], &["cpu package", "package"], &["core max"], &["cpu"]], &["cpu", "ryzen", "intel", "amd"]).filter(|v| (-40.0..=150.0).contains(v)),
        "cpu.power" => best(Cat::Power, &[&["cpu package power"], &["package"], &["cpu"]], &["cpu", "ryzen", "intel", "amd"]).filter(|v| *v >= 0.0),
        "cpu.fan" => best(Cat::Fan, &[&["cpu fan", "cpu_fan", "cpu opt"], &["cpu"]], &[]).filter(|v| *v >= 0.0),
        "board.temp" => best(Cat::Temp, &[&["motherboard", "mainboard"], &["system"], &["mb "], &["chipset"]], &[]).filter(|v| (-40.0..=150.0).contains(v)),
        _ => None,
    }
}

// ---------------------------------------------------------------- HWiNFO shared memory

/// Parse the HWiNFO sensor shared memory (layout of `HWiNFO_SENS_SM2`, packed, little-endian). Bounds are checked everywhere.
pub fn parse_hwinfo(b: &[u8]) -> Option<Vec<Reading>> {
    let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes(s.try_into().unwrap()));
    // "HWiS" as written by HWiNFO (0x53695748), version >= 1
    if u32_at(0)? != 0x5369_5748 {
        return None;
    }
    let (so, ss, sn) = (u32_at(20)? as usize, u32_at(24)? as usize, u32_at(28)? as usize);
    let (ro, rs, rn) = (u32_at(32)? as usize, u32_at(36)? as usize, u32_at(40)? as usize);
    if rs < 316 || ss < 136 || rn > 20_000 || sn > 2_000 {
        return None;
    }
    let cstr = |o: usize, n: usize| -> String {
        let s = b.get(o..o + n).unwrap_or(&[]);
        let end = s.iter().position(|c| *c == 0).unwrap_or(s.len());
        // HWiNFO writes ANSI / UTF-8; the degree sign is the only non-ASCII we care about
        String::from_utf8_lossy(&s[..end]).to_string()
    };
    let mut sensors: Vec<String> = Vec::with_capacity(sn);
    for i in 0..sn {
        let o = so + i * ss;
        let user = cstr(o + 136, 128);
        sensors.push(if user.is_empty() { cstr(o + 8, 128) } else { user });
    }
    let mut out = Vec::new();
    for i in 0..rn {
        let o = ro + i * rs;
        let (Some(t), Some(si)) = (u32_at(o), u32_at(o + 4)) else { break };
        let val = b.get(o + 284..o + 292).map(|s| f64::from_le_bytes(s.try_into().unwrap()))?;
        let unit = cstr(o + 268, 16);
        let label = {
            let u = cstr(o + 140, 128);
            if u.is_empty() { cstr(o + 12, 128) } else { u }
        };
        let cat = match t {
            1 => Cat::Temp,
            3 => Cat::Fan,
            5 => Cat::Power,
            // type 8 = "other": the PresentMon frame-rate / frame-time readings
            _ if unit.trim().eq_ignore_ascii_case("fps") || has(&label, &["framerate", "frame time"]) => Cat::Fps,
            _ => Cat::Other,
        };
        if cat == Cat::Other {
            continue;
        }
        // a Fahrenheit install: convert so every temperature is Celsius
        let value = if cat == Cat::Temp && unit.contains('F') { (val - 32.0) * 5.0 / 9.0 } else { val };
        out.push(Reading { cat, label, group: sensors.get(si as usize).cloned().unwrap_or_default(), value });
    }
    Some(out)
}

#[cfg(windows)]
fn read_hwinfo() -> Result<Vec<Reading>, String> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ, MEMORY_BASIC_INFORMATION};
    let name: Vec<u16> = "Global\\HWiNFO_SENS_SM2".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let h = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
        if h.is_null() {
            return Err("HWiNFO is not sharing its readings (not running, or Shared Memory Support is off)".into());
        }
        let view = MapViewOfFile(h, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(h);
            return Err("HWiNFO's shared memory could not be opened".into());
        }
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let n = VirtualQuery(view.Value, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        let len = if n == 0 { 0 } else { mbi.RegionSize.min(32 * 1024 * 1024) };
        let copy = std::slice::from_raw_parts(view.Value as *const u8, len).to_vec();
        UnmapViewOfFile(view);
        CloseHandle(h);
        parse_hwinfo(&copy).ok_or_else(|| "HWiNFO's shared memory has a layout this version does not understand".to_string())
    }
}

#[cfg(not(windows))]
fn read_hwinfo() -> Result<Vec<Reading>, String> {
    Err("HWiNFO is Windows only".into())
}

// ---------------------------------------------------------------- LibreHardwareMonitor web server

fn lhm_number(s: &str, integer: bool) -> Option<f64> {
    // "45.5 °C", "1,234 RPM", "12.3 W": the number is first; a comma may be the decimal or the thousands mark
    let t = s.trim().split_whitespace().next()?;
    let t = if integer || (t.contains(',') && t.contains('.')) { t.replace(',', "") } else { t.replace(',', ".") };
    t.parse().ok()
}

/// Parse LibreHardwareMonitor's `data.json` tree. A leaf's category is its parent's text ("Temperatures", "Fans", "Powers").
pub fn parse_lhm(v: &Value) -> Vec<Reading> {
    fn walk(n: &Value, path: &mut Vec<String>, out: &mut Vec<Reading>) {
        let text = n.get("Text").and_then(Value::as_str).unwrap_or("").to_string();
        let kids = n.get("Children").and_then(Value::as_array);
        match kids {
            Some(k) if !k.is_empty() => {
                path.push(text);
                for c in k {
                    walk(c, path, out);
                }
                path.pop();
            }
            _ => {
                let (Some(val), Some(parent)) = (n.get("Value").and_then(Value::as_str), path.last()) else { return };
                let cat = match parent.to_lowercase().as_str() {
                    "temperatures" => Cat::Temp,
                    "fans" => Cat::Fan,
                    "powers" => Cat::Power,
                    _ => return,
                };
                let Some(mut x) = lhm_number(val, cat == Cat::Fan) else { return };
                if cat == Cat::Temp && val.contains('F') {
                    x = (x - 32.0) * 5.0 / 9.0;
                }
                out.push(Reading { cat, label: text, group: path[..path.len() - 1].join(" / "), value: x });
            }
        }
    }
    let mut out = Vec::new();
    walk(v, &mut Vec::new(), &mut out);
    out
}

fn read_lhm() -> Result<Vec<Reading>, String> {
    static CLIENT: OnceLock<Option<reqwest::blocking::Client>> = OnceLock::new();
    let c = CLIENT
        .get_or_init(|| reqwest::blocking::Client::builder().timeout(Duration::from_millis(600)).connect_timeout(Duration::from_millis(300)).no_proxy().build().ok())
        .as_ref()
        .ok_or("no HTTP client")?;
    let resp = c.get("http://127.0.0.1:8085/data.json").send().map_err(|_| "LibreHardwareMonitor's web server is not answering on port 8085".to_string())?;
    if !resp.status().is_success() {
        return Err("LibreHardwareMonitor's web server answered with an error".into());
    }
    let v: Value = resp.json().map_err(|_| "LibreHardwareMonitor's answer was not understood".to_string())?;
    Ok(parse_lhm(&v))
}

// ---------------------------------------------------------------- shared snapshot

/// Why values are (not) available, in words for the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    /// "hwinfo" | "lhm" | "none"
    pub source: &'static str,
    pub ok: bool,
    pub message: String,
}

struct State {
    readings: Vec<Reading>,
    source: &'static str,
    hw_err: String,
    lhm_err: String,
    last_hw: Option<Instant>,
    last_lhm: Option<Instant>,
    asked: bool,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(State { readings: Vec::new(), source: "none", hw_err: String::new(), lhm_err: String::new(), last_hw: None, last_lhm: None, asked: false }))
}

/// Look at the sources if due (called from the sampler thread). Never blocks longer than the LHM timeout, and rarely.
fn refresh() {
    let (do_hw, do_lhm) = {
        let s = state().lock_or_recover();
        let hw_due = s.last_hw.is_none_or(|t| t.elapsed() >= Duration::from_secs(2));
        // the web server is tried only when HWiNFO has nothing, and then at most every 15 s (every 2 s while it works)
        let lhm_gap = if s.source == "lhm" { 2 } else { 15 };
        let lhm_due = s.last_lhm.is_none_or(|t| t.elapsed() >= Duration::from_secs(lhm_gap));
        (hw_due, lhm_due)
    };
    if do_hw {
        let r = read_hwinfo();
        let mut s = state().lock_or_recover();
        s.last_hw = Some(Instant::now());
        match r {
            Ok(v) => {
                s.readings = v;
                s.source = "hwinfo";
                s.hw_err.clear();
            }
            Err(e) => {
                s.hw_err = e;
                if s.source == "hwinfo" {
                    s.source = "none";
                    s.readings.clear();
                }
            }
        }
    }
    let try_lhm = do_lhm && state().lock_or_recover().source != "hwinfo";
    if try_lhm {
        let r = read_lhm();
        let mut s = state().lock_or_recover();
        s.last_lhm = Some(Instant::now());
        match r {
            Ok(v) => {
                s.readings = v;
                s.source = "lhm";
                s.lhm_err.clear();
            }
            Err(e) => {
                s.lhm_err = e;
                if s.source == "lhm" {
                    s.source = "none";
                    s.readings.clear();
                }
            }
        }
    }
}

/// Latest value of an external metric (the sampler calls this; it also keeps the sources fresh).
pub fn read(metric: &str) -> Option<f32> {
    state().lock_or_recover().asked = true;
    refresh();
    let s = state().lock_or_recover();
    pick(metric, &s.readings)
}

/// The frame rate of the game in front, from HWiNFO's PresentMon sensors: (fps, frame time in ms). None when HWiNFO is not the
/// source, has no such sensor, or the value is 0 (no game). Registers demand like `read()`.
pub fn game_fps() -> Option<(f32, Option<f32>)> {
    state().lock_or_recover().asked = true;
    refresh();
    let s = state().lock_or_recover();
    pick_game_fps(&s.readings)
}

/// Pure choice for `game_fps`.
pub fn pick_game_fps(all: &[Reading]) -> Option<(f32, Option<f32>)> {
    const SKIP: &[&str] = &["low", "%", "avg", "average", "min", "max"];
    let fps_rows: Vec<&Reading> = all.iter().filter(|r| r.cat == Cat::Fps && r.value.is_finite() && r.value > 0.0 && has(&r.label, &["framerate"])).collect();
    let fps = fps_rows
        .iter()
        .find(|r| has(&r.label, &["(presented)"]))
        .or_else(|| fps_rows.iter().find(|r| has(&r.label, &["(displayed)"])))
        .or_else(|| fps_rows.iter().find(|r| !has(&r.label, SKIP)))
        .or_else(|| fps_rows.first())?;
    let ms = all
        .iter()
        .filter(|r| r.cat == Cat::Fps && r.value.is_finite() && r.value > 0.0 && has(&r.label, &["frame time"]) && !has(&r.label, SKIP))
        .map(|r| r.value as f32)
        .next();
    Some((fps.value as f32, ms))
}

/// What to tell the user (editor). Probes right now if nothing has asked yet.
pub fn status() -> Status {
    let asked = state().lock_or_recover().asked;
    if !asked {
        refresh();
    }
    let s = state().lock_or_recover();
    match s.source {
        "hwinfo" | "lhm" => {
            let name = if s.source == "hwinfo" { "HWiNFO" } else { "LibreHardwareMonitor" };
            let have: Vec<&str> = METRICS.iter().copied().filter(|m| pick(m, &s.readings).is_some()).collect();
            if have.contains(&"cpu.temp") {
                Status { source: if s.source == "hwinfo" { "hwinfo" } else { "lhm" }, ok: true, message: format!("Reading from {name}.") }
            } else {
                Status { source: if s.source == "hwinfo" { "hwinfo" } else { "lhm" }, ok: false, message: format!("{name} is running but has no CPU temperature sensor to read.") }
            }
        }
        _ => Status {
            source: "none",
            ok: false,
            message: "No CPU temperature source. Start HWiNFO with Shared Memory Support on, or LibreHardwareMonitor with its Remote Web Server on (port 8085). CoolerScreen never installs or starts them.".to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn put(b: &mut [u8], o: usize, s: &str) {
        b[o..o + s.len()].copy_from_slice(s.as_bytes());
    }

    /// A made-up shared memory: two sensors, four readings (CPU temp, CPU power, fan, motherboard temp).
    fn fake_shm() -> Vec<u8> {
        let (so, ss, ro, rs) = (44usize, 264usize, 44 + 2 * 264, 316usize);
        let mut b = vec![0u8; ro + 5 * rs];
        b[0..4].copy_from_slice(&0x5369_5748u32.to_le_bytes());
        b[20..24].copy_from_slice(&(so as u32).to_le_bytes());
        b[24..28].copy_from_slice(&(ss as u32).to_le_bytes());
        b[28..32].copy_from_slice(&2u32.to_le_bytes());
        b[32..36].copy_from_slice(&(ro as u32).to_le_bytes());
        b[36..40].copy_from_slice(&(rs as u32).to_le_bytes());
        b[40..44].copy_from_slice(&5u32.to_le_bytes());
        put(&mut b, so + 8, "CPU [#0]: AMD Ryzen 7 9800X3D");
        put(&mut b, so + ss + 8, "ASUS ROG STRIX (Nuvoton)");
        let rows: [(u32, u32, &str, &str, f64); 5] = [
            (1, 1, "Motherboard", "°C", 36.0),
            (1, 0, "CPU (Tctl/Tdie)", "°C", 58.5),
            (5, 0, "CPU Package Power", "W", 71.2),
            (3, 1, "CPU Fan", "RPM", 1180.0),
            (7, 0, "Total CPU Usage", "%", 12.0),
        ];
        for (i, (t, si, label, unit, v)) in rows.iter().enumerate() {
            let o = ro + i * rs;
            b[o..o + 4].copy_from_slice(&t.to_le_bytes());
            b[o + 4..o + 8].copy_from_slice(&si.to_le_bytes());
            put(&mut b, o + 12, label);
            put(&mut b, o + 268, unit);
            b[o + 284..o + 292].copy_from_slice(&v.to_le_bytes());
        }
        b
    }

    #[test]
    fn hwinfo_layout_is_parsed_and_picked() {
        let r = parse_hwinfo(&fake_shm()).unwrap();
        assert_eq!(r.len(), 4, "usage readings are skipped");
        assert_eq!(pick("cpu.temp", &r), Some(58.5));
        assert_eq!(pick("cpu.power", &r), Some(71.2));
        assert_eq!(pick("cpu.fan", &r), Some(1180.0));
        assert_eq!(pick("board.temp", &r), Some(36.0));
        // truncated or foreign data is refused, never read out of bounds
        let b = fake_shm();
        assert!(parse_hwinfo(&b[..100]).is_none() || parse_hwinfo(&b[..100]).unwrap().is_empty());
        assert!(parse_hwinfo(&[0u8; 64]).is_none());
        assert!(parse_hwinfo(&[]).is_none());
        assert_eq!(pick("cpu.temp", &[]), None);
    }

    #[test]
    fn game_fps_is_found_and_old_metrics_unchanged() {
        let mut b = fake_shm();
        let (ro, rs) = (44 + 2 * 264, 316usize);
        b.resize(ro + 8 * rs, 0);
        b[40..44].copy_from_slice(&8u32.to_le_bytes());
        let rows: [(u32, &str, &str, f64); 3] = [(8, "Framerate (Displayed)", "FPS", 58.0), (8, "Framerate (Presented)", "FPS", 61.5), (8, "Frame Time", "ms", 16.2)];
        for (i, (t, label, unit, v)) in rows.iter().enumerate() {
            let o = ro + (5 + i) * rs;
            b[o..o + 4].copy_from_slice(&t.to_le_bytes());
            b[o + 12..o + 12 + label.len()].copy_from_slice(label.as_bytes());
            b[o + 268..o + 268 + unit.len()].copy_from_slice(unit.as_bytes());
            b[o + 284..o + 292].copy_from_slice(&v.to_le_bytes());
        }
        let r = parse_hwinfo(&b).unwrap();
        assert_eq!(r.len(), 7);
        assert_eq!(pick("cpu.temp", &r), Some(58.5));
        assert_eq!(pick("board.temp", &r), Some(36.0));
        assert_eq!(pick("cpu.power", &r), Some(71.2));
        assert_eq!(pick("cpu.fan", &r), Some(1180.0));
        assert_eq!(pick_game_fps(&r), Some((61.5, Some(16.2))));
        // 0 means no game
        let z: Vec<Reading> = r.iter().cloned().map(|mut x| { if x.cat == Cat::Fps { x.value = 0.0; } x }).collect();
        assert_eq!(pick_game_fps(&z), None);
        assert_eq!(pick_game_fps(&parse_hwinfo(&fake_shm()).unwrap()), None);
        // only a displayed rate: used
        let d: Vec<Reading> = r.iter().filter(|x| !x.label.contains("Presented")).cloned().collect();
        assert_eq!(pick_game_fps(&d).map(|x| x.0), Some(58.0));
        let _ = game_fps();
    }

    #[test]
    fn lhm_tree_is_parsed_and_picked() {
        let v = json!({ "Text": "Sensor", "Children": [{ "Text": "PC", "Children": [
            { "Text": "AMD Ryzen 7 9800X3D", "Children": [
                { "Text": "Temperatures", "Children": [{ "Text": "Core (Tctl/Tdie)", "Value": "61,5 °C", "Children": [] }, { "Text": "CCD1 (Tdie)", "Value": "55.0 °C" }] },
                { "Text": "Powers", "Children": [{ "Text": "Package", "Value": "64.1 W" }] },
                { "Text": "Load", "Children": [{ "Text": "CPU Total", "Value": "9.0 %" }] } ] },
            { "Text": "Nuvoton", "Children": [{ "Text": "Fans", "Children": [{ "Text": "CPU Fan", "Value": "1,204 RPM" }] }] } ] }] });
        let r = parse_lhm(&v);
        assert_eq!(r.len(), 4);
        assert_eq!(pick("cpu.temp", &r), Some(61.5));
        assert_eq!(pick("cpu.power", &r), Some(64.1));
        assert_eq!(pick("cpu.fan", &r), Some(1204.0));
        assert_eq!(pick("board.temp", &r), None);
    }

    #[test]
    fn nonsense_temperatures_are_not_shown() {
        let r = vec![Reading { cat: Cat::Temp, label: "CPU Package".into(), group: "CPU".into(), value: 255.0 }];
        assert_eq!(pick("cpu.temp", &r), None);
    }

    #[test]
    fn status_explains_when_nothing_runs() {
        let s = status();
        // on a test machine without HWiNFO / LHM the message says what to start; with one it says where it reads from
        if !s.ok {
            assert!(s.message.contains("HWiNFO") && s.message.contains("never installs"), "{}", s.message);
        }
    }
}
