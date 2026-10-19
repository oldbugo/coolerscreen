//! 4.0 W1 "Sun and moon" widget: props, cleaning and the astronomy (sunrise / sunset / day length, moon phase).
//! The drawing is in `render/sunmoon.rs`. Everything is computed locally (no network):
//! - Sun: the NOAA / Meeus solar-position formulas for the city's latitude and longitude and the current date.
//! - Moon: the mean synodic month from a reference new moon (good to about half a day, plenty for a picture).
use super::morewidgets::days_from_civil;
use super::theme::Widget;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SunMoonProps {
    /// Subset of "sun", "moon" (at least one).
    pub show: Vec<String>,
    /// "24h" | "12h"
    pub format: String,
    /// Colour of the sun values.
    pub color: String,
}
impl Default for SunMoonProps {
    fn default() -> Self {
        SunMoonProps { show: vec!["sun".into(), "moon".into()], format: "24h".into(), color: "#ffd27a".into() }
    }
}
impl SunMoonProps {
    pub fn shows(&self, what: &str) -> bool {
        self.show.iter().any(|s| s == what)
    }
}

impl Widget {
    pub fn sunmoon(&self) -> SunMoonProps {
        let mut p: SunMoonProps = serde_json::from_value(self.props.clone()).unwrap_or_default();
        if !p.shows("sun") && !p.shows("moon") {
            p.show = SunMoonProps::default().show;
        }
        p
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`). Idempotent.
pub fn clean(o: &mut Map<String, Value>) {
    let want: Vec<&str> = match o.get("show") {
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut show: Vec<&str> = ["sun", "moon"].into_iter().filter(|k| want.contains(k)).collect();
    if show.is_empty() {
        show = vec!["sun", "moon"];
    }
    o.insert("show".into(), json!(show));
    super::morewidgets::word(o, "format", &["24h", "12h"], "24h");
    super::morewidgets::colour(o, "color", "#ffd27a", false);
}

// ---------------------------------------------------------------- sun

/// What the sun does on one day at one place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SunUtc {
    /// Sunrise and sunset in minutes after 00:00 UTC of the given date (may be negative or above 1440: the place's
    /// local day does not line up with the UTC day).
    Times { rise: f64, set: f64 },
    /// The sun stays up all day (polar day).
    AlwaysUp,
    /// The sun stays down all day (polar night).
    AlwaysDown,
}

/// Equation of time (minutes) and solar declination (degrees) at Julian century `t` (NOAA / Meeus).
fn solar_terms(t: f64) -> (f64, f64) {
    let rad = std::f64::consts::PI / 180.0;
    let l0 = (280.46646 + t * (36000.76983 + t * 0.0003032)).rem_euclid(360.0);
    let m = 357.52911 + t * (35999.05029 - 0.0001537 * t);
    let e = 0.016708634 - t * (0.000042037 + 0.0000001267 * t);
    let c = (m * rad).sin() * (1.914602 - t * (0.004817 + 0.000014 * t)) + (2.0 * m * rad).sin() * (0.019993 - 0.000101 * t) + (3.0 * m * rad).sin() * 0.000289;
    let true_long = l0 + c;
    let omega = 125.04 - 1934.136 * t;
    let lambda = true_long - 0.00569 - 0.00478 * (omega * rad).sin();
    let eps0 = 23.0 + (26.0 + (21.448 - t * (46.815 + t * (0.00059 - t * 0.001813))) / 60.0) / 60.0;
    let eps = eps0 + 0.00256 * (omega * rad).cos();
    let dec = ((eps * rad).sin() * (lambda * rad).sin()).asin() / rad;
    let y = (eps * rad / 2.0).tan().powi(2);
    let (l0r, mr) = (l0 * rad, m * rad);
    let eq = y * (2.0 * l0r).sin() - 2.0 * e * mr.sin() + 4.0 * e * y * mr.sin() * (2.0 * l0r).cos() - 0.5 * y * y * (4.0 * l0r).sin() - 1.25 * e * e * (2.0 * mr).sin();
    (4.0 * eq / rad, dec)
}

/// Sunrise and sunset for latitude / longitude (degrees, east positive) on the calendar date `y-m-d`, as UTC minutes
/// (see `SunUtc`). Pure: no clock, no time zones. The sun's centre crosses -0.833 degrees (refraction + disc radius).
pub fn sun_utc(lat: f64, lon: f64, y: i32, m: u32, d: u32) -> SunUtc {
    let rad = std::f64::consts::PI / 180.0;
    let jd0 = days_from_civil(y as i64, m as i64, d as i64) as f64 + 2440587.5; // 00:00 UTC of the date
    let t_at = |min: f64| (jd0 + min / 1440.0 - 2451545.0) / 36525.0;
    // Hour angle (degrees) of sunrise / sunset for the declination at minute `min`, or the polar case.
    let hour_angle = |min: f64| -> Result<f64, bool> {
        let (_, dec) = solar_terms(t_at(min));
        let cos_h = (90.833 * rad).cos() / ((lat * rad).cos() * (dec * rad).cos()) - (lat * rad).tan() * (dec * rad).tan();
        if cos_h > 1.0 {
            Err(false)
        } else if cos_h < -1.0 {
            Err(true)
        } else {
            Ok(cos_h.acos() / rad)
        }
    };
    // Solar noon in UTC minutes, refined once.
    let mut noon = 720.0 - 4.0 * lon;
    noon = 720.0 - 4.0 * lon - solar_terms(t_at(noon)).0;
    noon = 720.0 - 4.0 * lon - solar_terms(t_at(noon)).0;
    let mut rise = noon;
    let mut set = noon;
    for _ in 0..3 {
        let (hr, hs) = (hour_angle(rise), hour_angle(set));
        match (hr, hs) {
            (Ok(a), Ok(b)) => {
                rise = 720.0 - 4.0 * (lon + a) - solar_terms(t_at(rise)).0;
                set = 720.0 - 4.0 * (lon - b) - solar_terms(t_at(set)).0;
            }
            (Err(up), _) | (_, Err(up)) => return if up { SunUtc::AlwaysUp } else { SunUtc::AlwaysDown },
        }
    }
    SunUtc::Times { rise, set }
}

/// The sun's day shown on the card, in local minutes of the day.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SunDay {
    Times { rise: i32, set: i32, len: i32 },
    AlwaysUp,
    AlwaysDown,
}

/// Sun times for `y-m-d` at the place, converted to local clock minutes. `fixed_offset` = Some(minutes east of UTC) uses
/// that fixed offset (the editor sample); None uses the PC's own time zone.
/// NOTE: a city in another time zone than the PC is shown in the PC's time, because the app carries no time-zone
/// database (only the PC's Windows zone is available through the system). The sunrise in Sydney seen from a PC in
/// London is therefore the London clock time of the Sydney sunrise.
pub fn sun_day(lat: f64, lon: f64, y: i32, m: u32, d: u32, fixed_offset: Option<i32>) -> SunDay {
    static CACHE: Mutex<Option<((i32, u32, u32, u64, u64, i32), SunDay)>> = Mutex::new(None);
    let key = (y, m, d, lat.to_bits(), lon.to_bits(), fixed_offset.unwrap_or(i32::MIN));
    if let Ok(g) = CACHE.lock() {
        if let Some((k, v)) = g.as_ref() {
            if *k == key {
                return *v;
            }
        }
    }
    let v = match sun_utc(lat, lon, y, m, d) {
        SunUtc::AlwaysUp => SunDay::AlwaysUp,
        SunUtc::AlwaysDown => SunDay::AlwaysDown,
        SunUtc::Times { rise, set } => {
            let day0 = days_from_civil(y as i64, m as i64, d as i64) * 1440;
            let local_min = |utc_min: f64| -> i32 {
                let total = day0 + utc_min.round() as i64; // UTC minutes since 1970
                match fixed_offset {
                    Some(off) => (total + off as i64).rem_euclid(1440) as i32,
                    None => (super::agenda::utc_to_local(total * 60).rem_euclid(86400) / 60) as i32,
                }
            };
            SunDay::Times { rise: local_min(rise), set: local_min(set), len: (set - rise).round().clamp(0.0, 1440.0) as i32 }
        }
    };
    if let Ok(mut g) = CACHE.lock() {
        *g = Some((key, v));
    }
    v
}

/// "05:41" or "5:41 AM" from minutes of the day.
pub fn fmt_time(min: i32, h12: bool) -> String {
    let (h, m) = ((min.rem_euclid(1440)) / 60, min.rem_euclid(60));
    if h12 {
        format!("{}:{:02} {}", if h % 12 == 0 { 12 } else { h % 12 }, m, if h >= 12 { "PM" } else { "AM" })
    } else {
        format!("{h:02}:{m:02}")
    }
}

/// "Day 13 h 42 m".
pub fn fmt_day_len(min: i32) -> String {
    format!("Day {} h {} m", min / 60, min % 60)
}

// ---------------------------------------------------------------- moon

pub const SYNODIC: f64 = 29.530588853;
/// Reference new moon: 2000-01-06 18:14 UTC as a Julian day.
const NEW_MOON_JD: f64 = 2451550.0 + 0.2597222;

/// Moon age in days (0 = new moon, about 14.77 = full) at unix time `secs` (UTC).
pub fn moon_age(unix_secs: f64) -> f64 {
    let jd = unix_secs / 86400.0 + 2440587.5;
    (jd - NEW_MOON_JD).rem_euclid(SYNODIC)
}

/// Lit fraction 0..1 of the disc for a moon age.
pub fn illumination(age: f64) -> f64 {
    (1.0 - (2.0 * std::f64::consts::PI * age / SYNODIC).cos()) / 2.0
}

pub fn phase_name(age: f64) -> &'static str {
    let a = age.rem_euclid(SYNODIC);
    let eighth = SYNODIC / 8.0;
    let i = ((a + eighth / 2.0) / eighth).floor() as usize % 8;
    ["New moon", "Waxing crescent", "First quarter", "Waxing gibbous", "Full moon", "Waning gibbous", "Last quarter", "Waning crescent"][i]
}

/// Current moon age from the PC's clock.
pub fn moon_age_now() -> f64 {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0);
    moon_age(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rise_set(lat: f64, lon: f64, y: i32, m: u32, d: u32) -> (f64, f64) {
        match sun_utc(lat, lon, y, m, d) {
            SunUtc::Times { rise, set } => (rise, set),
            o => panic!("expected times, got {o:?}"),
        }
    }
    fn near(a: f64, b: f64, what: &str) {
        assert!((a - b).abs() <= 3.0, "{what}: got {a:.1} min, want {b:.1} +-3");
    }

    #[test]
    fn sydney_summer_solstice() {
        // 05:41 and 20:05 AEDT (UTC+11) = 18:41 the day before and 09:05 UTC.
        let (r, s) = rise_set(-33.87, 151.21, 2026, 12, 21);
        near(r, 18.0 * 60.0 + 41.0 - 1440.0, "Sydney sunrise");
        near(s, 9.0 * 60.0 + 5.0, "Sydney sunset");
        let SunDay::Times { rise, set, len } = sun_day(-33.87, 151.21, 2026, 12, 21, Some(660)) else { panic!() };
        assert!((rise - 341).abs() <= 3 && (set - 1205).abs() <= 3, "local {rise} {set}");
        assert!((len - 864).abs() <= 4, "day length {len}");
    }

    #[test]
    fn london_midsummer() {
        // 04:43 and 21:21 BST (UTC+1) = 03:43 and 20:21 UTC.
        let (r, s) = rise_set(51.5, -0.12, 2026, 6, 21);
        near(r, 3.0 * 60.0 + 43.0, "London sunrise");
        near(s, 20.0 * 60.0 + 21.0, "London sunset");
    }

    #[test]
    fn tromso_polar_day_and_night() {
        assert_eq!(sun_utc(69.65, 18.96, 2026, 6, 21), SunUtc::AlwaysUp);
        assert_eq!(sun_utc(69.65, 18.96, 2026, 12, 21), SunUtc::AlwaysDown);
        assert_eq!(sun_day(69.65, 18.96, 2026, 6, 21, Some(120)), SunDay::AlwaysUp);
        assert_eq!(sun_day(69.65, 18.96, 2026, 12, 21, Some(60)), SunDay::AlwaysDown);
    }

    #[test]
    fn equator_and_edge_dates_are_sane() {
        // Quito: about 12 h of daylight all year; dates at the ends of the year and a leap day work.
        for (y, m, d) in [(2026, 1, 1), (2026, 12, 31), (2028, 2, 29), (2026, 3, 20)] {
            let (r, s) = rise_set(-0.18, -78.47, y, m, d);
            assert!(((s - r) - 720.0).abs() < 12.0, "{y}-{m}-{d}: {}", s - r);
        }
        // Every latitude and a spread of dates gives finite numbers or a polar answer, never a panic.
        for lat in (-90..=90).step_by(15) {
            for mo in 1..=12 {
                if let SunUtc::Times { rise, set } = sun_utc(lat as f64, 10.0, 2026, mo, 15) {
                    assert!(rise.is_finite() && set.is_finite() && set > rise);
                }
            }
        }
    }

    #[test]
    fn time_formats() {
        assert_eq!(fmt_time(341, false), "05:41");
        assert_eq!(fmt_time(341, true), "5:41 AM");
        assert_eq!(fmt_time(0, true), "12:00 AM");
        assert_eq!(fmt_time(1205, true), "8:05 PM");
        assert_eq!(fmt_day_len(822), "Day 13 h 42 m");
    }

    fn unix(y: i64, m: i64, d: i64, hour: f64) -> f64 {
        days_from_civil(y, m, d) as f64 * 86400.0 + hour * 3600.0
    }
    /// Distance in days between an age and a target age on the circular month.
    fn age_gap(age: f64, target: f64) -> f64 {
        let g = (age - target).abs();
        g.min(SYNODIC - g)
    }

    #[test]
    fn moon_phase_known_dates() {
        // New moon 2026-01-18 about 19:52 UTC; full moon 2026-02-01 about 22:09 UTC.
        assert!(age_gap(moon_age(unix(2026, 1, 18, 19.9)), 0.0) < 1.0, "new moon {}", moon_age(unix(2026, 1, 18, 19.9)));
        assert!(age_gap(moon_age(unix(2026, 2, 1, 22.1)), SYNODIC / 2.0) < 1.0, "full moon {}", moon_age(unix(2026, 2, 1, 22.1)));
        assert_eq!(phase_name(moon_age(unix(2026, 1, 18, 19.9))), "New moon");
        assert_eq!(phase_name(moon_age(unix(2026, 2, 1, 22.1))), "Full moon");
        // Reference moment itself.
        assert!(moon_age(unix(2000, 1, 6, 18.0 + 14.0 / 60.0)) < 0.01 || moon_age(unix(2000, 1, 6, 18.0 + 14.0 / 60.0)) > SYNODIC - 0.01);
    }

    #[test]
    fn illumination_bounds_and_names() {
        for i in 0..=2000 {
            let age = SYNODIC * i as f64 / 2000.0;
            let il = illumination(age);
            assert!((0.0..=1.0).contains(&il));
            assert!(!phase_name(age).is_empty());
        }
        assert!(illumination(0.0) < 1e-9);
        assert!((illumination(SYNODIC / 2.0) - 1.0).abs() < 1e-9);
        assert!((illumination(SYNODIC / 4.0) - 0.5).abs() < 1e-9);
        let names: Vec<&str> = [0.0, 3.7, 7.4, 11.1, 14.8, 18.5, 22.1, 25.8].iter().map(|a| phase_name(*a)).collect();
        assert_eq!(names, ["New moon", "Waxing crescent", "First quarter", "Waxing gibbous", "Full moon", "Waning gibbous", "Last quarter", "Waning crescent"]);
        assert_eq!(phase_name(29.4), "New moon");
        assert!(moon_age_now() >= 0.0 && moon_age_now() < SYNODIC);
    }

    #[test]
    fn clean_is_idempotent_and_bounded() {
        let run = |v: Value| -> Value {
            let mut o = v.as_object().cloned().unwrap_or_default();
            clean(&mut o);
            Value::Object(o)
        };
        for v in [
            json!({}),
            json!({ "show": [] }),
            json!({ "show": ["moon", "moon", "sun", "x", 5] }),
            json!({ "show": ["moon"], "format": "12h", "color": "#ABCDEF" }),
            json!({ "show": "sun", "format": "weird", "color": 7 }),
            json!(null),
        ] {
            let a = run(v);
            assert_eq!(run(a.clone()), a);
            let p: SunMoonProps = serde_json::from_value(a.clone()).unwrap();
            assert!(p.shows("sun") || p.shows("moon"));
            assert!(p.format == "24h" || p.format == "12h");
            assert_eq!(p.color.len(), 7);
        }
        assert_eq!(run(json!({ "show": ["moon", "sun", "sun"] }))["show"], json!(["sun", "moon"]));
        assert_eq!(run(json!({ "show": ["moon"], "color": "#ABCDEF" }))["color"], json!("#abcdef"));
        assert_eq!(run(json!({}))["show"], json!(["sun", "moon"]));
    }

    #[test]
    fn widget_props_read_leniently() {
        let w = Widget { kind: "sunmoon".into(), props: json!({ "show": [], "format": 3 }), ..Widget::default() };
        let p = w.sunmoon();
        assert!(p.shows("sun") && p.shows("moon"));
        let w = Widget { kind: "sunmoon".into(), props: json!("garbage"), ..Widget::default() };
        assert_eq!(w.sunmoon(), SunMoonProps::default());
    }
}
