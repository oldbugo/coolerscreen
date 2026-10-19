//! 4.0 stream B2: the `agenda` widget's data: the next events of an iCalendar (.ics) calendar the USER points to, either a
//! calendar link (https:// or webcal://; fetched every 15 minutes, only while an agenda widget is shown) or a file on this PC
//! (re-read when it changes). Nothing is installed, no account is used; the link is whatever the user pasted.
//!
//! Times are "naive local seconds" (days since 1970 x 86400 + time of day, in this PC's local time). UTC times (`...Z`) are
//! converted with the PC's own time zone; times with a TZID or floating times are taken as local (a calendar in another zone is
//! shown shifted by the difference: documented limitation). Recurrence: DAILY, WEEKLY (BYDAY), MONTHLY and YEARLY with INTERVAL,
//! COUNT, UNTIL and EXDATE; anything fancier is shown as its first occurrence only.
use super::morewidgets::days_from_civil;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

const REFRESH: Duration = Duration::from_secs(15 * 60);
const KEEP: Duration = Duration::from_secs(30);
const MAX_BYTES: usize = 3 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub title: String,
    /// Naive local seconds.
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgendaView {
    pub events: Vec<Event>,
    /// Loaded at least once.
    pub loaded: bool,
    pub error: Option<String>,
}

// ---------------------------------------------------------------- parsing

fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if (line.starts_with(' ') || line.starts_with('\t')) && !out.is_empty() {
            out.last_mut().unwrap().push_str(&line[1..]);
        } else {
            out.push(line.to_string());
        }
    }
    out
}

fn unescape(s: &str) -> String {
    s.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// Split `NAME;PARAM=x:VALUE` into (NAME, params, VALUE).
fn prop(line: &str) -> Option<(String, Vec<(String, String)>, String)> {
    let colon = line.find(':')?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.to_ascii_uppercase();
    let params = parts.filter_map(|p| p.split_once('=').map(|(k, v)| (k.to_ascii_uppercase(), v.trim_matches('"').to_string()))).collect();
    Some((name, params, value.to_string()))
}

/// A DATE or DATE-TIME value as (naive local seconds, all day). `Z` times are converted to local with the PC's zone.
fn parse_time(value: &str, params: &[(String, String)]) -> Option<(i64, bool)> {
    let v = value.trim();
    let num = |a: usize, b: usize| v.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(4, 6)?, num(6, 8)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let date_only = params.iter().any(|(k, val)| k == "VALUE" && val == "DATE") || v.len() == 8;
    let base = days_from_civil(y, mo, d) * 86400;
    if date_only {
        return Some((base, true));
    }
    if v.as_bytes().get(8) != Some(&b'T') {
        return None;
    }
    let (h, mi, s) = (num(9, 11)?, num(11, 13)?, num(13, 15).unwrap_or(0));
    let naive = base + h * 3600 + mi * 60 + s;
    if v.ends_with('Z') {
        return Some((utc_to_local(naive), false));
    }
    Some((naive, false))
}

/// Naive UTC seconds -> naive local seconds in this PC's zone.
#[cfg(windows)]
pub(crate) fn utc_to_local(utc: i64) -> i64 {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::Time::SystemTimeToTzSpecificLocalTime;
    let days = utc.div_euclid(86400);
    let rem = utc.rem_euclid(86400);
    let (y, m, d) = civil_from_days(days);
    let st = SYSTEMTIME { wYear: y as u16, wMonth: m as u16, wDayOfWeek: 0, wDay: d as u16, wHour: (rem / 3600) as u16, wMinute: ((rem / 60) % 60) as u16, wSecond: (rem % 60) as u16, wMilliseconds: 0 };
    let mut out: SYSTEMTIME = unsafe { std::mem::zeroed() };
    if unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &st, &mut out) } == 0 {
        return utc;
    }
    days_from_civil(out.wYear as i64, out.wMonth as i64, out.wDay as i64) * 86400 + out.wHour as i64 * 3600 + out.wMinute as i64 * 60 + out.wSecond as i64
}

#[cfg(not(windows))]
pub(crate) fn utc_to_local(utc: i64) -> i64 {
    utc
}

pub fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Debug, Default, Clone)]
struct Rule {
    freq: String,
    interval: i64,
    count: Option<i64>,
    until: Option<i64>,
    /// 0 = Sunday .. 6 = Saturday
    by_day: Vec<i64>,
}

fn parse_rule(v: &str) -> Rule {
    let mut r = Rule { interval: 1, ..Rule::default() };
    for part in v.split(';') {
        let Some((k, val)) = part.split_once('=') else { continue };
        match k.to_ascii_uppercase().as_str() {
            "FREQ" => r.freq = val.to_ascii_uppercase(),
            "INTERVAL" => r.interval = val.parse().unwrap_or(1).clamp(1, 1000),
            "COUNT" => r.count = val.parse().ok(),
            "UNTIL" => r.until = parse_time(val, &[]).map(|t| t.0 + if val.len() == 8 { 86399 } else { 0 }),
            "BYDAY" => {
                r.by_day = val
                    .split(',')
                    .filter_map(|d| {
                        let d = d.trim_start_matches(|c: char| c.is_ascii_digit() || c == '+' || c == '-');
                        ["SU", "MO", "TU", "WE", "TH", "FR", "SA"].iter().position(|x| *x == d).map(|i| i as i64)
                    })
                    .collect()
            }
            _ => {}
        }
    }
    r
}

/// Parse a calendar and return the events that END after `from` and START before `to` (recurrences expanded), sorted by start.
pub fn parse(text: &str, from: i64, to: i64) -> Vec<Event> {
    let mut out = Vec::new();
    let lines = unfold(text);
    let mut cur: Option<Vec<&String>> = None;
    for l in &lines {
        match l.trim() {
            "BEGIN:VEVENT" => cur = Some(Vec::new()),
            "END:VEVENT" => {
                if let Some(ev) = cur.take() {
                    expand(&ev, from, to, &mut out);
                }
            }
            _ => {
                if let Some(c) = cur.as_mut() {
                    c.push(l);
                }
            }
        }
    }
    out.sort_by_key(|e| (e.start, e.title.clone()));
    out
}

fn expand(lines: &[&String], from: i64, to: i64, out: &mut Vec<Event>) {
    let (mut title, mut start, mut end, mut all_day) = (String::new(), None, None, false);
    let mut rule: Option<Rule> = None;
    let mut ex: Vec<i64> = Vec::new();
    let mut duration: Option<i64> = None;
    let mut cancelled = false;
    for l in lines {
        let Some((name, params, value)) = prop(l) else { continue };
        match name.as_str() {
            "SUMMARY" => title = unescape(&value),
            "DTSTART" => {
                if let Some((t, d)) = parse_time(&value, &params) {
                    start = Some(t);
                    all_day = d;
                }
            }
            "DTEND" => end = parse_time(&value, &params).map(|t| t.0),
            "DURATION" => duration = parse_duration(&value),
            "RRULE" => rule = Some(parse_rule(&value)),
            "EXDATE" => ex.extend(value.split(',').filter_map(|v| parse_time(v, &params).map(|t| t.0))),
            "STATUS" => cancelled = value.trim().eq_ignore_ascii_case("CANCELLED"),
            _ => {}
        }
    }
    let Some(start) = start else { return };
    if cancelled {
        return;
    }
    let len = match (end, duration) {
        (Some(e), _) if e > start => e - start,
        (_, Some(d)) => d,
        _ => if all_day { 86400 } else { 0 },
    };
    let title = if title.trim().is_empty() { "(no title)".to_string() } else { title.trim().to_string() };
    let mut push = |s: i64| {
        let same_day_ex = ex.iter().any(|e| *e == s || (all_day && e.div_euclid(86400) == s.div_euclid(86400)));
        if !same_day_ex && s + len.max(1) > from && s < to {
            out.push(Event { title: title.clone(), start: s, end: s + len, all_day });
        }
    };
    let Some(r) = rule else {
        push(start);
        return;
    };
    let mut n = 0i64;
    let day0 = start.div_euclid(86400);
    let tod = start.rem_euclid(86400);
    let (y0, m0, d0) = civil_from_days(day0);
    let limit_day = to.div_euclid(86400) + 1;
    let mut i = 0i64;
    // a plain loop over candidate days / months / years, bounded
    'outer: while i < 5000 {
        let cands: Vec<i64> = match r.freq.as_str() {
            "DAILY" => vec![day0 + i * r.interval],
            "WEEKLY" => {
                let week0 = day0 - (day0 + 4).rem_euclid(7); // the Sunday on/before the start (1970-01-01 was a Thursday)
                let days: Vec<i64> = if r.by_day.is_empty() { vec![(day0 + 4).rem_euclid(7)] } else { r.by_day.clone() };
                let mut v: Vec<i64> = days.iter().map(|d| week0 + i * 7 * r.interval + d).filter(|d| *d >= day0).collect();
                v.sort();
                v
            }
            "MONTHLY" => {
                let total = y0 * 12 + (m0 - 1) + i * r.interval;
                let (y, m) = (total.div_euclid(12), total.rem_euclid(12) + 1);
                let d = days_from_civil(y, m, d0);
                // skip months without that day (e.g. the 31st)
                if civil_from_days(d) == (y, m, d0) { vec![d] } else { vec![] }
            }
            "YEARLY" => {
                let y = y0 + i * r.interval;
                let d = days_from_civil(y, m0, d0);
                if civil_from_days(d) == (y, m0, d0) { vec![d] } else { vec![] }
            }
            _ => {
                push(start);
                return;
            }
        };
        for d in cands {
            let s = d * 86400 + tod;
            n += 1;
            if r.count.is_some_and(|c| n > c) || r.until.is_some_and(|u| s > u) || d > limit_day {
                break 'outer;
            }
            push(s);
        }
        i += 1;
    }
}

fn parse_duration(v: &str) -> Option<i64> {
    // P1DT2H30M, PT1H, P2W
    let v = v.trim().strip_prefix('P')?;
    let (mut total, mut num, mut in_time) = (0i64, String::new(), false);
    for c in v.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => num.push(c),
            _ => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                total += n * match (c, in_time) {
                    ('W', _) => 604800,
                    ('D', _) => 86400,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    Some(total)
}

// ---------------------------------------------------------------- loading

struct Slot {
    last_used: Instant,
    running: bool,
    view: AgendaView,
    fetched: Option<Instant>,
    file_mtime: Option<SystemTime>,
}

fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static S: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What the source field means: a link (https / webcal) or a path of a file on this PC; None for anything else.
pub fn classify(src: &str) -> Option<Src> {
    let s = src.trim();
    let lower = s.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("webcal://") {
        return Some(Src::Url(format!("https://{}", &s[s.len() - rest.len()..])));
    }
    if lower.starts_with("https://") {
        return Some(Src::Url(s.to_string()));
    }
    if lower.starts_with("http://") || lower.contains("://") {
        return None;
    }
    let p = s.trim_matches('"');
    (p.len() > 2 && (p.to_ascii_lowercase().ends_with(".ics") || p.to_ascii_lowercase().ends_with(".ical"))).then(|| Src::File(p.to_string()))
}

#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    Url(String),
    File(String),
}

pub fn now_local_secs() -> i64 {
    let t = super::render::local_time();
    days_from_civil(t.year as i64, t.month as i64, t.day as i64) * 86400 + t.hour as i64 * 3600 + t.minute as i64 * 60 + t.second as i64
}

/// The upcoming events of `source` (starts or refreshes the loader; the first call returns `loaded: false`).
pub fn read(source: &str) -> AgendaView {
    let Some(src) = classify(source) else {
        return AgendaView { events: Vec::new(), loaded: true, error: Some("Not a calendar link or .ics file".into()) };
    };
    let key = source.trim().to_string();
    let mut g = slots().lock().unwrap();
    let s = g.entry(key.clone()).or_insert_with(|| Slot { last_used: Instant::now(), running: false, view: AgendaView::default(), fetched: None, file_mtime: None });
    s.last_used = Instant::now();
    if !s.running {
        s.running = true;
        std::thread::spawn(move || run(key, src));
    }
    s.view.clone()
}

fn load(src: &Src) -> Result<String, String> {
    match src {
        Src::File(p) => {
            let md = std::fs::metadata(p).map_err(|_| "The calendar file was not found".to_string())?;
            if md.len() as usize > MAX_BYTES {
                return Err("The calendar file is too large".into());
            }
            std::fs::read_to_string(p).map_err(|_| "The calendar file could not be read".to_string())
        }
        Src::Url(u) => {
            let c = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).user_agent(crate::sources::http::UA).redirect(reqwest::redirect::Policy::limited(3)).build().map_err(|_| "No network client".to_string())?;
            let r = c.get(u).send().map_err(|_| "The calendar link did not answer".to_string())?;
            if !r.status().is_success() {
                return Err(format!("The calendar link answered {}", r.status().as_u16()));
            }
            if r.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
                return Err("The calendar is too large".into());
            }
            let b = r.bytes().map_err(|_| "The calendar could not be read".to_string())?;
            if b.len() > MAX_BYTES {
                return Err("The calendar is too large".into());
            }
            String::from_utf8(b.to_vec()).map_err(|_| "The calendar is not text".to_string())
        }
    }
}

fn run(key: String, src: Src) {
    let mut cached_text: Option<String> = None;
    loop {
        let (due, idle) = {
            let g = slots().lock().unwrap();
            match g.get(&key) {
                Some(s) => {
                    let mtime = if let Src::File(p) = &src { std::fs::metadata(p).and_then(|m| m.modified()).ok() } else { None };
                    let changed = matches!(src, Src::File(_)) && mtime != s.file_mtime;
                    (s.fetched.is_none_or(|t| t.elapsed() >= REFRESH) || changed, s.last_used.elapsed() > KEEP)
                }
                None => (false, true),
            }
        };
        if idle {
            break;
        }
        if due {
            let r = load(&src);
            let mtime = if let Src::File(p) = &src { std::fs::metadata(p).and_then(|m| m.modified()).ok() } else { None };
            if let Ok(t) = &r {
                cached_text = Some(t.clone());
            }
            let mut g = slots().lock().unwrap();
            if let Some(s) = g.get_mut(&key) {
                s.fetched = Some(Instant::now());
                s.file_mtime = mtime;
                if let Err(e) = &r {
                    // keep the old events when a refresh fails
                    s.view.error = Some(e.clone());
                    s.view.loaded = true;
                } else {
                    s.view.error = None;
                }
            }
        }
        // re-expand every few seconds (the window of "upcoming" moves with the clock)
        if let Some(t) = &cached_text {
            let now = now_local_secs();
            let ev = parse(t, now - 3600, now + 90 * 86400);
            let mut g = slots().lock().unwrap();
            if let Some(s) = g.get_mut(&key) {
                s.view.events = ev;
                s.view.loaded = true;
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
    if let Some(s) = slots().lock().unwrap().get_mut(&key) {
        s.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ICS: &str = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
BEGIN:VEVENT\r\nSUMMARY:Dentist\\, Dr Lee\r\nDTSTART:20261020T143000\r\nDTEND:20261020T153000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Standup\r\nDTSTART:20261019T090000\r\nDURATION:PT15M\r\nRRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR\r\nEXDATE:20261021T090000\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Birthday\r\nDTSTART;VALUE=DATE:20261101\r\nRRULE:FREQ=YEARLY\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:A very long\r\n  title folded\r\nDTSTART:20261022T100000\r\nSTATUS:CANCELLED\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nSUMMARY:Rent\r\nDTSTART;VALUE=DATE:20261031\r\nRRULE:FREQ=MONTHLY;COUNT=3\r\nEND:VEVENT\r\n\
END:VCALENDAR\r\n";

    fn at(y: i64, m: i64, d: i64, h: i64, mi: i64) -> i64 {
        days_from_civil(y, m, d) * 86400 + h * 3600 + mi * 60
    }

    #[test]
    fn events_and_recurrences_are_expanded() {
        let ev = parse(ICS, at(2026, 10, 19, 8, 0), at(2026, 11, 8, 0, 0));
        let names: Vec<String> = ev.iter().map(|e| format!("{} {}", e.title, civil_from_days(e.start.div_euclid(86400)).2)).collect();
        // Mondays/Wednesdays/Fridays at 09:00 from 19 Oct, minus the excluded Wednesday 21st; the cancelled one is gone
        assert_eq!(names, ["Standup 19", "Dentist, Dr Lee 20", "Standup 23", "Standup 26", "Standup 28", "Standup 30", "Rent 31", "Birthday 1", "Standup 2", "Standup 4", "Standup 6"].map(String::from), "{names:?}");
        let d = &ev[1];
        assert_eq!((d.end - d.start, d.all_day), (3600, false));
        assert!(ev.iter().find(|e| e.title == "Birthday").unwrap().all_day);
        // the earlier window drops what already ended
        let later = parse(ICS, at(2026, 10, 20, 16, 0), at(2026, 10, 25, 0, 0));
        assert!(later.iter().all(|e| e.title == "Standup"));
    }

    #[test]
    fn monthly_skips_missing_days_and_counts() {
        let ev = parse(ICS, at(2026, 10, 1, 0, 0), at(2027, 3, 1, 0, 0));
        let rent: Vec<i64> = ev.iter().filter(|e| e.title == "Rent").map(|e| civil_from_days(e.start.div_euclid(86400)).1).collect();
        assert_eq!(rent, [10, 12, 1], "Oct 31, Dec 31, Jan 31: months without a 31st are skipped and not counted");
    }

    #[test]
    fn utc_times_and_durations() {
        assert_eq!(parse_duration("P1DT2H30M"), Some(95400));
        assert_eq!(parse_duration("PT45M"), Some(2700));
        assert_eq!(parse_duration("P2W"), Some(1209600));
        assert_eq!(parse_duration("1H"), None);
        let (t, d) = parse_time("20261020T000000Z", &[]).unwrap();
        assert!(!d);
        assert!((t - at(2026, 10, 20, 0, 0)).abs() <= 14 * 3600, "within a zone offset");
        assert!(parse_time("garbage", &[]).is_none());
        assert!(parse_time("20261340", &[]).is_none());
    }

    #[test]
    fn civil_round_trip() {
        for d in [-1000, 0, 11017, 20745, 30000] {
            let (y, m, dd) = civil_from_days(d);
            assert_eq!(days_from_civil(y, m, dd), d);
        }
    }

    #[test]
    fn sources_are_classified() {
        assert_eq!(classify("webcal://cal.example.com/a.ics"), Some(Src::Url("https://cal.example.com/a.ics".into())));
        assert_eq!(classify("https://x.y/z"), Some(Src::Url("https://x.y/z".into())));
        assert_eq!(classify("http://x.y/z.ics"), None);
        assert_eq!(classify("C:\\Users\\me\\cal.ics"), Some(Src::File("C:\\Users\\me\\cal.ics".into())));
        assert_eq!(classify("\"D:\\a b\\cal.ICS\""), Some(Src::File("D:\\a b\\cal.ICS".into())));
        assert_eq!(classify("notes.txt"), None);
        assert_eq!(classify(""), None);
        assert_eq!(classify("file:///c:/x.ics"), None);
    }

    #[test]
    fn a_file_is_read_and_shown() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("cal.ics");
        let tomorrow = now_local_secs() + 86400;
        let (y, m, d) = civil_from_days(tomorrow.div_euclid(86400));
        std::fs::write(&f, format!("BEGIN:VCALENDAR\nBEGIN:VEVENT\nSUMMARY:Soon\nDTSTART:{y:04}{m:02}{d:02}T120000\nEND:VEVENT\nEND:VCALENDAR\n")).unwrap();
        let p = f.to_string_lossy().to_string();
        let mut v = read(&p);
        for _ in 0..40 {
            if v.events.iter().any(|e| e.title == "Soon") {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
            v = read(&p);
        }
        assert!(v.events.iter().any(|e| e.title == "Soon"), "{v:?}");
        let missing = read("C:\\nowhere\\none.ics");
        let _ = missing;
    }
}
