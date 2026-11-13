//! Weather for the `weather` widget: Open-Meteo forecast (no key, non-commercial use), fetched on a background thread with the
//! app's HTTP client, cached for 15 minutes. The place comes from settings.json `weather.place` {name, lat, lon, tz} - chosen
//! by the user (never guessed from the IP). No place: nothing is fetched. Offline: the last value stays, flagged `stale`.
//! Temperatures are stored in degrees Celsius; the widget converts.
use crate::core::sync::MutexExt;
use serde_json::Value;
use serde::{Serialize,Deserialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64,Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const CACHE: Duration = Duration::from_secs(15 * 60);
/// The fetch thread ends when no widget asked for this long (the cache stays).
const IDLE_END: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub tz: String,
}

#[derive(Clone, Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Hour {
    /// Local hour of day 0..23.
    pub hour: u32,
    pub temp_c: f32,
    pub code: u32,
}

#[derive(Clone, Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Day {
    /// 0 = Monday .. 6 = Sunday.
    pub weekday: u32,
    pub hi_c: f32,
    pub lo_c: f32,
    pub code: u32,
}

#[derive(Clone, Debug,Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherData {
    pub place: String,
    pub temp_c: f32,
    pub feels_c: f32,
    pub code: u32,
    pub is_day: bool,
    pub hours: Vec<Hour>,
    pub days: Vec<Day>,
    /// The data could not be refreshed (offline); it is the last good value.
    pub stale: bool,
}

#[derive(Default)]
struct State {
    cache:HashMap<String,CacheEntry>,
    place: Option<Place>,
    last_ask: Option<Instant>,
    running: bool,
}
#[derive(Default)]
struct CacheEntry {data:Option<WeatherData>,observed_at_ms:Option<u64>,attempted:Option<Instant>,failed:bool}
static FETCHES:AtomicU64=AtomicU64::new(0);
pub fn fetch_count()->u64{FETCHES.load(Ordering::Relaxed)}
fn key(place:&Place)->String{serde_json::to_string(place).unwrap_or_default()}

fn state() -> &'static Arc<Mutex<State>> {
    static S: OnceLock<Arc<Mutex<State>>> = OnceLock::new();
    S.get_or_init(|| Arc::new(Mutex::new(State::default())))
}

/// What the widget needs: Ok(data) when there is a place and a value, Err(reason) otherwise ("no place" / "loading").
pub fn get() -> Result<WeatherData, &'static str> {
    if super::shared_services::enabled(){return super::shared_services::weather_get(read_place());}
    get_for(read_place())
}
/// Source-only explicit input: this never reads app settings or geolocation.
pub fn get_for(place:Option<Place>)->Result<WeatherData,&'static str>{
    let st = state().clone();
    let mut g = st.lock_or_recover();
    if place.is_some(){g.last_ask = Some(Instant::now());}
    g.place = place;
    if !g.running && g.place.is_some() {
        g.running = true;
        let me = st.clone();
        // A crash starts the fetcher again (1 s, doubling to 30 s) while weather is still asked for.
        std::thread::spawn(move || {
            let idle = || me.lock_or_recover().last_ask.is_none_or(|t| t.elapsed() > IDLE_END);
            if crate::core::sync::restarting("weather fetch", idle, || run(me.clone())).is_none() {
                me.lock_or_recover().running = false;
            }
        });
    }
    if g.place.is_none() {
        return Err("no place");
    }
    let place=g.place.as_ref().ok_or("no place")?;
    match g.cache.get(&key(place)).and_then(|c|c.data.as_ref()) {
        Some(d) => Ok(d.clone()),
        None => Err("loading"),
    }
}
pub fn source_observation(place:Option<Place>)->super::shared_services::WeatherObservation{
    let _=get_for(place.clone());let g=state().lock_or_recover();
    let entry=place.as_ref().and_then(|p|g.cache.get(&key(p)));
    super::shared_services::WeatherObservation{status:if place.is_none(){"inactive"}else if entry.is_some_and(|e|e.data.is_some()){if entry.is_some_and(|e|e.failed){"stale"}else{"ok"}}else if entry.is_some_and(|e|e.failed){"error"}else{"loading"}.into(),observed_at_ms:entry.and_then(|e|e.observed_at_ms),place,data:entry.and_then(|e|e.data.clone())}
}

/// settings.json `weather.place`; None when missing or incomplete.
pub fn read_place() -> Option<Place> {
    let text = std::fs::read_to_string(crate::core::paths::settings()).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    parse_place(&v["weather"]["place"])
}

pub fn parse_place(v: &Value) -> Option<Place> {
    let (lat, lon) = (v.get("lat")?.as_f64()?, v.get("lon")?.as_f64()?);
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some(Place { name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(), lat, lon, tz: v.get("tz").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("auto").to_string() })
}

fn run(st: Arc<Mutex<State>>) {
    loop {
        let (place, due, idle) = {
            let g = st.lock_or_recover();
            let idle = g.last_ask.is_none_or(|t| t.elapsed() > IDLE_END);
            let due=g.place.as_ref().map(|p|g.cache.get(&key(p)).is_none_or(|e|e.attempted.is_none_or(|t|t.elapsed()>=if e.failed{Duration::from_secs(60)}else{CACHE}))).unwrap_or(false);
            (g.place.clone(),due,idle)
        };
        if idle {
            st.lock_or_recover().running = false;
            return;
        }
        if let (Some(p), true) = (place, due) {
            FETCHES.fetch_add(1,Ordering::Relaxed);
            match fetch(&p) {
                Ok(d) => {
                    let mut g = st.lock_or_recover();
                    g.cache.insert(key(&p),CacheEntry{data:Some(d),observed_at_ms:Some(super::shared_services::now_ms()),attempted:Some(Instant::now()),failed:false});
                }
                Err(_) => {
                    crate::device::log("weather: source unavailable");
                    let mut g = st.lock_or_recover();
                    let entry=g.cache.entry(key(&p)).or_default();
                    if let Some(d) = entry.data.as_mut() {
                        d.stale = true;
                    }
                    // retry in a minute, not at the next poll
                    entry.failed=true;entry.attempted=Some(Instant::now());
                }
            }
            let mut g=st.lock_or_recover();if g.cache.len()>8{let current=g.place.as_ref().map(key);let oldest=g.cache.iter().filter(|(k,_)|Some(*k)!=current.as_ref()).min_by_key(|(_,v)|v.observed_at_ms.unwrap_or(0)).map(|(k,_)|k.clone());if let Some(oldest)=oldest{g.cache.remove(&oldest);}}
        }
        std::thread::sleep(Duration::from_secs(3));
    }
}

fn fetch(p: &Place) -> Result<WeatherData, String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={:.4}&longitude={:.4}&current=temperature_2m,apparent_temperature,weather_code,is_day&hourly=temperature_2m,weather_code&daily=weather_code,temperature_2m_max,temperature_2m_min&forecast_days=6&timezone={}",
        p.lat,
        p.lon,
        p.tz.replace('/', "%2F")
    );
    let u = reqwest::Url::parse(&url).map_err(|e| e.to_string())?;
    let v = crate::sources::http::get_json(u, "the weather service").map_err(|e| e.0)?;
    parse_forecast(&v, &p.name).ok_or_else(|| "the weather answer was not understood".to_string())
}

/// Open-Meteo forecast JSON -> data (times are local to the requested time zone).
pub fn parse_forecast(v: &Value, place: &str) -> Option<WeatherData> {
    let cur = v.get("current")?;
    let temp_c = cur.get("temperature_2m")?.as_f64()? as f32;
    let feels_c = cur.get("apparent_temperature").and_then(Value::as_f64).map(|x| x as f32).unwrap_or(temp_c);
    let code = cur.get("weather_code")?.as_u64()? as u32;
    let is_day = cur.get("is_day").and_then(Value::as_u64).unwrap_or(1) == 1;
    let now_time = cur.get("time").and_then(Value::as_str).unwrap_or("");
    let h = v.get("hourly")?;
    let (ht, hv, hc) = (h.get("time")?.as_array()?, h.get("temperature_2m")?.as_array()?, h.get("weather_code")?.as_array()?);
    let now_hour_key: String = now_time.chars().take(13).collect(); // "2026-10-18T14"
    let start = ht.iter().position(|t| t.as_str().is_some_and(|s| s.starts_with(&now_hour_key))).unwrap_or(0);
    let mut hours = Vec::new();
    for i in start..ht.len().min(hv.len()).min(hc.len()) {
        let t = ht[i].as_str().unwrap_or("");
        let hour: u32 = t.get(11..13).and_then(|x| x.parse().ok()).unwrap_or(0);
        hours.push(Hour { hour, temp_c: hv[i].as_f64().unwrap_or(0.0) as f32, code: hc[i].as_u64().unwrap_or(0) as u32 });
    }
    let d = v.get("daily")?;
    let (dt, dc, dh, dl) = (d.get("time")?.as_array()?, d.get("weather_code")?.as_array()?, d.get("temperature_2m_max")?.as_array()?, d.get("temperature_2m_min")?.as_array()?);
    let mut days = Vec::new();
    for i in 0..dt.len().min(dc.len()).min(dh.len()).min(dl.len()) {
        let s = dt[i].as_str().unwrap_or("");
        let (y, m, dd) = (s.get(0..4)?.parse().ok()?, s.get(5..7)?.parse().ok()?, s.get(8..10)?.parse().ok()?);
        days.push(Day { weekday: weekday_of(y, m, dd), hi_c: dh[i].as_f64().unwrap_or(0.0) as f32, lo_c: dl[i].as_f64().unwrap_or(0.0) as f32, code: dc[i].as_u64().unwrap_or(0) as u32 });
    }
    Some(WeatherData { place: place.to_string(), temp_c, feels_c, code, is_day, hours, days, stale: false })
}

/// 0 = Monday .. 6 = Sunday.
pub fn weekday_of(y: i32, m: u32, d: u32) -> u32 {
    let days = days_from_civil(y, m, d);
    // 1970-01-01 was a Thursday (3)
    (days + 3).rem_euclid(7) as u32
}

/// Days since 1970-01-01 (proleptic Gregorian).
pub fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// Inverse of `days_from_civil`: (year, month, day) of a day count since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = (yoe + era * 400) as i32 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

/// Fixed example data for previews and tests.
pub fn sample() -> WeatherData {
    let hrs = [(15, 18.0, 2), (17, 17.0, 3), (19, 15.0, 61), (21, 13.0, 63), (23, 12.0, 3), (1, 11.0, 0)];
    let days = [(6, 19.0, 11.0, 2), (0, 16.0, 10.0, 61), (1, 14.0, 9.0, 63), (2, 15.0, 8.0, 3), (3, 17.0, 9.0, 0), (4, 12.0, 6.0, 71)];
    WeatherData {
        place: "Madrid".into(),
        temp_c: 18.0,
        feels_c: 17.0,
        code: 2,
        is_day: true,
        hours: hrs.iter().map(|(h, t, c)| Hour { hour: *h, temp_c: *t, code: *c }).collect(),
        days: days.iter().map(|(w, hi, lo, c)| Day { weekday: *w, hi_c: *hi, lo_c: *lo, code: *c }).collect(),
        stale: false,
    }
}

/// WMO weather code -> (icon kind, short text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Sun,
    Moon,
    PartlyDay,
    PartlyNight,
    Cloud,
    Fog,
    Drizzle,
    Rain,
    Snow,
    Thunder,
}

pub fn describe(code: u32, is_day: bool) -> (Icon, &'static str) {
    match code {
        0 => (if is_day { Icon::Sun } else { Icon::Moon }, if is_day { "Sunny" } else { "Clear" }),
        1 => (if is_day { Icon::Sun } else { Icon::Moon }, "Mostly clear"),
        2 => (if is_day { Icon::PartlyDay } else { Icon::PartlyNight }, "Partly cloudy"),
        3 => (Icon::Cloud, "Overcast"),
        45 | 48 => (Icon::Fog, "Fog"),
        51 | 53 | 55 | 56 | 57 => (Icon::Drizzle, "Drizzle"),
        61 | 63 | 65 | 66 | 67 | 80 | 81 | 82 => (Icon::Rain, if code >= 80 { "Showers" } else { "Rain" }),
        71 | 73 | 75 | 77 | 85 | 86 => (Icon::Snow, "Snow"),
        95 | 96 | 99 => (Icon::Thunder, "Thunderstorm"),
        _ => (Icon::Cloud, "Cloudy"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn weekdays_and_dates() {
        assert_eq!(weekday_of(2026, 10, 18), 6); // Sunday
        assert_eq!(weekday_of(2026, 10, 17), 5);
        assert_eq!(weekday_of(1970, 1, 1), 3);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
    }

    #[test]
    fn parses_an_open_meteo_answer() {
        let v = json!({
            "current": { "time": "2026-10-18T14:15", "temperature_2m": 18.4, "apparent_temperature": 17.0, "weather_code": 2, "is_day": 1 },
            "hourly": { "time": ["2026-10-18T13:00", "2026-10-18T14:00", "2026-10-18T15:00"], "temperature_2m": [17.0, 18.0, 19.0], "weather_code": [1, 2, 3] },
            "daily": { "time": ["2026-10-18", "2026-10-19"], "weather_code": [2, 61], "temperature_2m_max": [19.0, 15.0], "temperature_2m_min": [11.0, 9.0] }
        });
        let d = parse_forecast(&v, "Madrid").unwrap();
        assert_eq!((d.temp_c, d.code, d.is_day), (18.4, 2, true));
        assert_eq!(d.hours.len(), 2);
        assert_eq!(d.hours[0].hour, 14);
        assert_eq!(d.days[0].weekday, 6);
        assert_eq!(d.days[1].code, 61);
        assert!(parse_forecast(&json!({}), "x").is_none());
    }

    #[test]
    fn place_needs_coordinates() {
        assert!(parse_place(&json!({ "name": "A" })).is_none());
        assert!(parse_place(&json!({ "lat": 95, "lon": 0 })).is_none());
        let p = parse_place(&json!({ "name": "Madrid", "lat": 40.4, "lon": -3.7, "tz": "Europe/Madrid" })).unwrap();
        assert_eq!(p.tz, "Europe/Madrid");
        assert_eq!(describe(61, true).0, Icon::Rain);
        assert_eq!(describe(0, false).0, Icon::Moon);
    }

    #[test]
    fn parses_a_real_recorded_answer() {
        let v: Value = serde_json::from_str(include_str!("testdata/open-meteo-madrid.json")).unwrap();
        let d = parse_forecast(&v, "Madrid").unwrap();
        assert_eq!(d.days.len(), 6);
        assert_eq!(d.days[0].weekday, 5); // 2026-10-17 is a Saturday
        assert_eq!(d.hours[0].hour, 17);
        assert!(d.hours.len() >= 24);
        assert_eq!((d.temp_c, d.code, d.is_day), (21.0, 3, true));
        assert_eq!(describe(d.code, d.is_day).0, Icon::Cloud);
    }
}
