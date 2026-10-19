//! 4.0 W1 "progress" widget: props, cleaning and the date maths (day / week / month / year progress bars).
//! Rules: lenient props (`#[serde(default, rename_all = "camelCase")]`), defaults behave sensibly and nothing new appears in
//! existing themes; `clean` bounds every prop (idempotent) using the helpers `word`, `colour`, `number`, `int`, `boolean`, `short`
//! of `super::morewidgets`. Drawing is in `render/progress.rs`.
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::render::LocalTime;
use super::theme::Widget;

pub const PERIODS: [&str; 4] = ["day", "week", "month", "year"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ProgressProps {
    /// Which bars, in the order shown (1..4, no duplicates).
    pub items: Vec<String>,
    pub labels: bool,
    pub decimals: u8,
    pub color: String,
    /// "windows" (follow the Windows region setting), "monday" or "sunday".
    pub week_start: String,
}
impl Default for ProgressProps {
    fn default() -> Self {
        ProgressProps { items: PERIODS.iter().map(|s| s.to_string()).collect(), labels: true, decimals: 0, color: "#5ac8ff".into(), week_start: "windows".into() }
    }
}

impl Widget {
    pub fn progress(&self) -> ProgressProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`).
pub fn clean(o: &mut Map<String, Value>) {
    let mut items: Vec<String> = Vec::new();
    if let Some(Value::Array(a)) = o.get("items") {
        for v in a {
            if let Some(s) = v.as_str() {
                if PERIODS.contains(&s) && !items.iter().any(|i| i == s) {
                    items.push(s.to_string());
                }
            }
        }
    }
    if items.is_empty() {
        items = ProgressProps::default().items;
    }
    o.insert("items".into(), json!(items));
    super::morewidgets::boolean(o, "labels", true);
    super::morewidgets::int(o, "decimals", 0, 0, 2);
    super::morewidgets::colour(o, "color", "#5ac8ff", false);
    super::morewidgets::word(o, "weekStart", &["windows", "monday", "sunday"], "windows");
}

// ------------------------------------------------------------------ date maths

pub fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
    }
}

pub fn days_in_year(y: i32) -> u32 {
    if is_leap(y) { 366 } else { 365 }
}

/// 1-based day of the year.
pub fn day_of_year(y: i32, m: u32, d: u32) -> u32 {
    (1..m).map(|mm| days_in_month(y, mm)).sum::<u32>() + d
}

/// Fraction (0..1) of the four periods elapsed at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fractions {
    pub day: f64,
    pub week: f64,
    pub month: f64,
    pub year: f64,
}

/// `week_start`: weekday the week begins on, 0 = Sunday ... 6 = Saturday.
pub fn fractions(t: &LocalTime, week_start: u16) -> Fractions {
    let (y, m, d) = (t.year as i32, (t.month as u32).clamp(1, 12), (t.day as u32).max(1));
    let secs = t.hour as f64 * 3600.0 + t.minute as f64 * 60.0 + t.second as f64 + t.millis as f64 / 1000.0;
    let day = (secs / 86400.0).clamp(0.0, 1.0);
    let since = ((t.weekday % 7) + 7 - (week_start % 7)) % 7;
    let week = ((since as f64 + day) / 7.0).clamp(0.0, 1.0);
    let month = (((d - 1) as f64 + day) / days_in_month(y, m) as f64).clamp(0.0, 1.0);
    let year = (((day_of_year(y, m, d) - 1) as f64 + day) / days_in_year(y) as f64).clamp(0.0, 1.0);
    Fractions { day, week, month, year }
}

impl Fractions {
    pub fn of(&self, period: &str) -> f64 {
        match period {
            "day" => self.day,
            "week" => self.week,
            "month" => self.month,
            _ => self.year,
        }
    }
}

/// The first weekday (0 = Sunday) for a `weekStart` setting.
pub fn week_start_day(setting: &str) -> u16 {
    match setting {
        "monday" => 1,
        "sunday" => 0,
        _ => windows_first_day(),
    }
}

/// Windows' first day of the week as a weekday number with 0 = Sunday (Windows counts 0 = Monday ... 6 = Sunday). Read once.
pub fn windows_first_day() -> u16 {
    static CACHE: std::sync::OnceLock<u16> = std::sync::OnceLock::new();
    *CACHE.get_or_init(|| from_windows_index(read_windows_index().unwrap_or(0)))
}

/// Windows index (0 = Monday ... 6 = Sunday) -> weekday number (0 = Sunday ... 6 = Saturday).
pub fn from_windows_index(i: u32) -> u16 {
    ((i % 7) as u16 + 1) % 7
}

fn read_windows_index() -> Option<u32> {
    use windows_sys::Win32::Globalization::{GetLocaleInfoEx, LOCALE_IFIRSTDAYOFWEEK, LOCALE_RETURN_NUMBER};
    let mut v: u32 = 0;
    // LOCALE_NAME_USER_DEFAULT is a null name; with LOCALE_RETURN_NUMBER the buffer receives a DWORD (2 wide chars).
    let n = unsafe { GetLocaleInfoEx(std::ptr::null(), LOCALE_IFIRSTDAYOFWEEK | LOCALE_RETURN_NUMBER, &mut v as *mut u32 as *mut u16, 2) };
    if n > 0 && v <= 6 { Some(v) } else { None }
}

/// "37 %", "37.5 %": rounded DOWN at the shown precision, so a bar never claims 100 % before the period has ended.
pub fn percent_text(frac: f64, decimals: u8) -> String {
    let d = decimals.min(2) as i32;
    let k = 10f64.powi(d);
    let v = ((frac.clamp(0.0, 1.0) * 100.0 * k) + 1e-6).floor() / k;
    format!("{:.*} %", d as usize, v)
}

pub fn label_of(period: &str) -> &'static str {
    match period {
        "day" => "Day",
        "week" => "Week",
        "month" => "Month",
        _ => "Year",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 2026-10-11 is a Sunday, 2026-10-12 a Monday, 2026-10-15 a Thursday.
    fn t(year: u16, month: u16, weekday: u16, day: u16, hour: u16, minute: u16, second: u16) -> LocalTime {
        LocalTime { year, month, weekday, day, hour, minute, second, millis: 0 }
    }
    fn near(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    #[test]
    fn leap_years_and_month_lengths() {
        assert!(is_leap(2024) && is_leap(2000) && !is_leap(1900) && !is_leap(2026));
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 12), 31);
        assert_eq!(day_of_year(2024, 12, 31), 366);
        assert_eq!(day_of_year(2026, 12, 31), 365);
        assert_eq!(day_of_year(2024, 3, 1), 61);
    }

    #[test]
    fn day_fraction() {
        near(fractions(&t(2026, 10, 4, 15, 0, 0, 0), 1).day, 0.0);
        near(fractions(&t(2026, 10, 4, 15, 12, 0, 0), 1).day, 0.5);
        near(fractions(&t(2026, 10, 4, 15, 23, 59, 59), 1).day, 86399.0 / 86400.0);
    }

    #[test]
    fn month_and_year_fractions_including_leap_year() {
        let a = fractions(&t(2026, 1, 4, 1, 0, 0, 0), 1);
        near(a.month, 0.0);
        near(a.year, 0.0);
        let z = fractions(&t(2026, 12, 4, 31, 23, 59, 59), 1);
        assert!(z.month < 1.0 && z.month > 0.9999);
        assert!(z.year < 1.0 && z.year > 0.9999);
        // 15 Feb 2024 noon: 14.5 / 29 of the month, (31 + 14.5) / 366 of the year
        let f = fractions(&t(2024, 2, 4, 15, 12, 0, 0), 1);
        near(f.month, 14.5 / 29.0);
        near(f.year, 45.5 / 366.0);
        // the same date in 2026: a 28-day February
        near(fractions(&t(2026, 2, 0, 15, 12, 0, 0), 1).month, 14.5 / 28.0);
        // 29 Feb 2024 23:59:59 is the very end of its month
        let l = fractions(&t(2024, 2, 4, 29, 23, 59, 59), 1);
        assert!(l.month > 0.9998 && l.month < 1.0);
        near(fractions(&t(2024, 3, 5, 1, 0, 0, 0), 1).year, 60.0 / 366.0);
        near(fractions(&t(2026, 3, 0, 1, 0, 0, 0), 1).year, 59.0 / 365.0);
    }

    #[test]
    fn week_boundaries_for_each_week_start() {
        // Monday 00:00 starts a Monday week; a Sunday week began the day before
        let mon = t(2026, 10, 1, 12, 0, 0, 0);
        near(fractions(&mon, 1).week, 0.0);
        near(fractions(&mon, 0).week, 1.0 / 7.0);
        // Sunday 23:59:59 is the end of a Monday week and a fresh day 1 of a Sunday week is its start
        let w = fractions(&t(2026, 10, 0, 11, 23, 59, 59), 1).week;
        assert!(w > 0.9998 && w < 1.0);
        near(fractions(&t(2026, 10, 0, 11, 0, 0, 0), 0).week, 0.0);
        // Saturday 23:59:59 is the end of a Sunday week
        let w = fractions(&t(2026, 10, 6, 10, 23, 59, 59), 0).week;
        assert!(w > 0.9998 && w < 1.0);
        // Saturday week start: Saturday noon is 0.5 / 7
        near(fractions(&t(2026, 10, 6, 10, 12, 0, 0), 6).week, 0.5 / 7.0);
        // Wednesday noon, Monday week: 2.5 / 7
        near(fractions(&t(2026, 10, 3, 14, 12, 0, 0), 1).week, 2.5 / 7.0);
    }

    #[test]
    fn windows_index_maps_to_weekday_numbers() {
        assert_eq!(from_windows_index(0), 1); // Monday
        assert_eq!(from_windows_index(5), 6); // Saturday
        assert_eq!(from_windows_index(6), 0); // Sunday
        assert_eq!(week_start_day("monday"), 1);
        assert_eq!(week_start_day("sunday"), 0);
        assert!(week_start_day("windows") < 7);
    }

    #[test]
    fn percent_text_rounds_down() {
        assert_eq!(percent_text(0.37, 0), "37 %");
        assert_eq!(percent_text(0.37, 1), "37.0 %");
        assert_eq!(percent_text(0.3749, 1), "37.4 %");
        assert_eq!(percent_text(0.37499, 2), "37.49 %");
        assert_eq!(percent_text(86399.0 / 86400.0, 0), "99 %");
        assert_eq!(percent_text(1.0, 0), "100 %");
        assert_eq!(percent_text(0.0, 2), "0.00 %");
    }

    #[test]
    fn clean_bounds_and_is_idempotent() {
        let run = |v: Value| {
            let mut o = v.as_object().cloned().unwrap_or_default();
            clean(&mut o);
            o
        };
        let d = run(json!({}));
        assert_eq!(d["items"], json!(["day", "week", "month", "year"]));
        assert_eq!(d["labels"], json!(true));
        assert_eq!(d["decimals"], json!(0));
        assert_eq!(d["color"], json!("#5ac8ff"));
        assert_eq!(d["weekStart"], json!("windows"));
        // empty / junk items fall back to the default; order kept, duplicates and unknowns dropped
        assert_eq!(run(json!({ "items": [] }))["items"], json!(["day", "week", "month", "year"]));
        assert_eq!(run(json!({ "items": ["x", 3] }))["items"], json!(["day", "week", "month", "year"]));
        assert_eq!(run(json!({ "items": "year" }))["items"], json!(["day", "week", "month", "year"]));
        assert_eq!(run(json!({ "items": ["year", "day", "year", "bogus"] }))["items"], json!(["year", "day"]));
        let b = run(json!({ "decimals": 9, "color": "red", "weekStart": "friday", "labels": "yes" }));
        assert_eq!(b["decimals"], json!(2));
        assert_eq!(b["color"], json!("#5ac8ff"));
        assert_eq!(b["weekStart"], json!("windows"));
        assert_eq!(b["labels"], json!(true));
        assert_eq!(run(json!({ "decimals": -4 }))["decimals"], json!(0));
        for v in [json!({}), json!({ "items": ["week"], "decimals": 1, "weekStart": "sunday", "labels": false }), json!({ "items": 5, "decimals": 99 })] {
            let once = run(v);
            let twice = run(Value::Object(once.clone()));
            assert_eq!(once, twice);
        }
    }

    #[test]
    fn props_parse_leniently() {
        let w = Widget { props: json!({ "items": ["month"], "weekStart": "monday", "unknown": 1 }), ..Widget::default() };
        let p = w.progress();
        assert_eq!(p.items, vec!["month".to_string()]);
        assert_eq!(p.week_start, "monday");
        assert!(p.labels);
        let bad = Widget { props: json!("garbage"), ..Widget::default() };
        assert_eq!(bad.progress(), ProgressProps::default());
    }
}
