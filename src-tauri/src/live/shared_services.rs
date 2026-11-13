//! Typed UC source delivery. This receiver performs no native or network reads.
use crate::core::sync::MutexExt;
use super::{
    nowplaying::{Cover, NowPlaying},
    weather::{Place, WeatherData},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::{Duration, Instant},
};
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
pub const MAX_PNG: usize = 2 * 1024 * 1024;
const SAFE: u64 = 9_007_199_254_740_991;
static ENABLED: AtomicBool = AtomicBool::new(false);
static LOCAL_MUSIC: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MusicDemand {
    pub preferred_app: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Demand {
    pub music: Option<MusicDemand>,
    pub weather: Option<Place>,
}
impl Demand {
    pub fn validate(&self) -> Result<(), String> {
        if self
            .music
            .as_ref()
            .and_then(|m| m.preferred_app.as_ref())
            .is_some_and(|p| p.chars().count() > 128 || p.chars().any(char::is_control))
        {
            return Err("Invalid music preference".into());
        }
        if self.weather.as_ref().is_some_and(|p| !place_valid(p)) {
            return Err("Invalid weather place".into());
        }
        Ok(())
    }
}
fn place_valid(p: &Place) -> bool {
    p.lat.is_finite()
        && p.lon.is_finite()
        && (-90. ..=90.).contains(&p.lat)
        && (-180. ..=180.).contains(&p.lon)
        && !p.name.trim().is_empty()
        && p.name.chars().count() <= 80
        && !p.name.chars().any(char::is_control)
        && p.tz.len() <= 64
        && p.tz
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/_+-".contains(c))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoverData {
    pub id: u64,
    pub png: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicData {
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub position_s: f64,
    pub duration_s: f64,
    pub state_changed_at_ms: u64,
    pub track_id: u64,
    pub cover: Option<CoverData>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicObservation {
    pub status: String,
    pub observed_at_ms: Option<u64>,
    pub preferred_app: Option<String>,
    pub data: Option<MusicData>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeatherObservation {
    pub status: String,
    pub observed_at_ms: Option<u64>,
    pub place: Option<Place>,
    pub data: Option<WeatherData>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub generation: u64,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub expires_at_ms: u64,
    pub music: MusicObservation,
    pub weather: WeatherObservation,
}
pub fn now_ms() -> u64 {
    super::shared_hardware::now_ms()
}
fn text(v: &str, max: usize) -> bool {
    v.chars().count() <= max && !v.chars().any(char::is_control)
}
pub fn png_dimensions(bytes: &[u8]) -> Option<(usize, usize)> {
    if bytes.len() > MAX_PNG
        || bytes.len() < 24
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[12..16] != b"IHDR"
    {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?) as usize;
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?) as usize;
    (w > 0 && h > 0 && w <= 640 && h <= 640 && w * h <= 640 * 640).then_some((w, h))
}
impl Snapshot {
    pub fn validate(&self, now: u64) -> Result<(), String> {
        if self.version != 1
            || self.generation == 0
            || self.sequence == 0
            || self.generation > SAFE
            || self.sequence > SAFE
            || self.observed_at_ms == 0
            || self.observed_at_ms > now
            || self.expires_at_ms <= now
            || self.expires_at_ms > SAFE
            || self.expires_at_ms <= self.observed_at_ms
            || self.expires_at_ms - self.observed_at_ms > 3000
        {
            return Err("Invalid service snapshot lease".into());
        }
        let m = &self.music;
        if !["inactive", "loading", "ok", "none", "error"].contains(&m.status.as_str())
            || m.observed_at_ms.is_some_and(|t| t == 0 || t > now)
            || m.preferred_app.as_ref().is_some_and(|p| !text(p, 128))
            || ((m.status == "ok") != m.data.is_some())
        {
            return Err("Invalid music observation".into());
        }
        if let Some(d) = &m.data {
            if m.observed_at_ms.is_none()
                || !text(&d.app, 256)
                || !text(&d.title, 512)
                || !text(&d.artist, 512)
                || !d.position_s.is_finite()
                || d.position_s < 0.
                || d.position_s > 604800.
                || !d.duration_s.is_finite()
                || d.duration_s < 0.
                || d.duration_s > 604800.
                || d.state_changed_at_ms == 0
                || d.state_changed_at_ms > now
                || d.track_id == 0
                || d.track_id > SAFE
                || d.cover
                    .as_ref()
                    .is_some_and(|c| c.id == 0 || c.id > SAFE || png_dimensions(&c.png).is_none())
            {
                return Err("Invalid music data".into());
            }
        }
        let w = &self.weather;
        if !["inactive", "loading", "ok", "stale", "error"].contains(&w.status.as_str())
            || w.observed_at_ms.is_some_and(|t| t == 0 || t > now)
            || w.place.as_ref().is_some_and(|p| !place_valid(p))
            || (["ok", "stale"].contains(&w.status.as_str()) != w.data.is_some())
        {
            return Err("Invalid weather observation".into());
        }
        if let Some(d) = &w.data {
            if w.observed_at_ms.is_none()
                || w.place.is_none()
                || w.place.as_ref().is_some_and(|p| p.name != d.place)
                || !text(&d.place, 80)
                || !(-150. ..=100.).contains(&d.temp_c)
                || !(-150. ..=100.).contains(&d.feels_c)
                || d.code > 99
                || d.hours.len() > 168
                || d.days.len() > 7
                || d.hours
                    .iter()
                    .any(|h| h.hour > 23 || !(-150. ..=100.).contains(&h.temp_c) || h.code > 99)
                || d.days.iter().any(|d| {
                    d.weekday > 6
                        || !(-150. ..=100.).contains(&d.hi_c)
                        || !(-150. ..=100.).contains(&d.lo_c)
                        || d.code > 99
                })
                || d.stale != (w.status == "stale")
            {
                return Err("Invalid weather data".into());
            }
        }
        Ok(())
    }
}
#[derive(Default)]
struct Receiver {
    snapshot: Option<Snapshot>,
    generation: u64,
    sequence: u64,
    music_demand: Option<(MusicDemand, Instant)>,
    weather_demand: Option<(Place, Instant)>,
    cover: Option<(u64, Arc<Cover>)>,
}
fn receiver() -> &'static Mutex<Receiver> {
    static R: OnceLock<Mutex<Receiver>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(Receiver::default()))
}
pub fn enable_for_uc_worker() {
    *receiver().lock_or_recover() = Receiver::default();
    ENABLED.store(true, Ordering::SeqCst);
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}
pub fn local_music_start() {
    LOCAL_MUSIC.fetch_add(1, Ordering::Relaxed);
}
pub fn touch_music(preferred_app: Option<String>) {
    let demand = MusicDemand { preferred_app };
    if (Demand {
        music: Some(demand.clone()),
        weather: None,
    })
    .validate()
    .is_ok()
    {
        receiver().lock_or_recover().music_demand = Some((demand, Instant::now()));
    }
}
impl Receiver {
    fn demand(&self) -> Demand {
        Demand {
            music: self
                .music_demand
                .as_ref()
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(2))
                .map(|(d, _)| d.clone()),
            weather: self
                .weather_demand
                .as_ref()
                .filter(|(_, at)| at.elapsed() < Duration::from_secs(2))
                .map(|(p, _)| p.clone()),
        }
    }
    fn fresh(&self, now: u64) -> Option<&Snapshot> {
        self.snapshot
            .as_ref()
            .filter(|s| s.observed_at_ms <= now && now < s.expires_at_ms)
    }
    fn accept(&mut self, snapshot: Snapshot, now: u64) -> Result<(), String> {
        snapshot.validate(now)?;
        if self.generation != 0 && snapshot.generation != self.generation
            || snapshot.sequence <= self.sequence
            || self
                .snapshot
                .as_ref()
                .is_some_and(|old| snapshot.observed_at_ms < old.observed_at_ms)
        {
            return Err("Service snapshot is outside its channel lease".into());
        }
        self.generation = snapshot.generation;
        self.sequence = snapshot.sequence;
        self.snapshot = Some(snapshot);
        Ok(())
    }
}
pub fn now_playing() -> Option<NowPlaying> {
    let mut r = receiver().lock_or_recover();
    let now = now_ms();
    let s = r.fresh(now)?;
    let m = &s.music;
    let at = m.observed_at_ms?;
    if m.status != "ok"
        || now.saturating_sub(at) >= 3000
        || r.demand().music.as_ref().map(|d| &d.preferred_app) != Some(&m.preferred_app)
    {
        return None;
    }
    let source = m.data.as_ref()?;
    // The renderer asks every frame. Copy metadata, never an unchanged PNG.
    let d = MusicData {
        app: source.app.clone(),
        title: source.title.clone(),
        artist: source.artist.clone(),
        playing: source.playing,
        position_s: source.position_s,
        duration_s: source.duration_s,
        state_changed_at_ms: source.state_changed_at_ms,
        track_id: source.track_id,
        cover: None,
    };
    let has_cover = source.cover.is_some();
    let changed = source
        .cover
        .as_ref()
        .filter(|cover| r.cover.as_ref().is_none_or(|(id, _)| *id != cover.id))
        .cloned();
    let cover = if has_cover {
        if let Some(c) = changed {
            r.cover = decode_cover(&c).map(|v| (c.id, v));
        }
        r.cover.as_ref().map(|(_, cover)| cover.clone())
    } else {
        r.cover = None;
        None
    };
    NowPlaying::from_shared(&d, at, cover)
}
pub fn weather_get(place: Option<Place>) -> Result<WeatherData, &'static str> {
    let mut r = receiver().lock_or_recover();
    let Some(place) = place else {
        r.weather_demand = None;
        return Err("no place");
    };
    if !place_valid(&place) {
        r.weather_demand = None;
        return Err("no place");
    }
    r.weather_demand = Some((place.clone(), Instant::now()));
    let s = r.fresh(now_ms()).ok_or("loading")?;
    if s.weather.place.as_ref() != Some(&place) {
        return Err("loading");
    }
    s.weather.data.clone().ok_or("loading")
}
pub fn exchange(snapshot: Option<Snapshot>) -> Result<Value, String> {
    if !enabled() {
        return Err("Shared services mode is not enabled".into());
    }
    let mut r = receiver().lock_or_recover();
    let now = now_ms();
    if let Some(s) = snapshot {
        r.accept(s, now)?;
    }
    let s = r.fresh(now);
    let m = s.map(|s| &s.music);
    let w = s.map(|s| &s.weather);
    let demand = r.demand();
    let music_present = m.is_some_and(|m| {
        m.status == "ok"
            && m.data.is_some()
            && m.observed_at_ms
                .is_some_and(|at| now.saturating_sub(at) < 3000)
            && demand.music.as_ref().map(|d| &d.preferred_app) == Some(&m.preferred_app)
    });
    let weather_present =
        w.is_some_and(|w| w.data.is_some() && demand.weather.as_ref() == w.place.as_ref());
    Ok(
        json!({"demand":demand,"state":{"shared":true,"fresh":s.is_some(),"expires_at_ms":r.snapshot.as_ref().map(|s|s.expires_at_ms),"accepted_sequence":r.sequence,"generation":r.generation,"music_present":music_present,"weather_present":weather_present,"music_status":m.map(|m|m.status.as_str()).unwrap_or("inactive"),"weather_status":w.map(|w|w.status.as_str()).unwrap_or("inactive"),"local_music_starts":LOCAL_MUSIC.load(Ordering::Relaxed),"local_weather_fetches":super::weather::fetch_count()}}),
    )
}
pub fn encode_cover(cover: &Cover, id: u64) -> Option<CoverData> {
    if cover.w == 0
        || cover.h == 0
        || cover.w > 640
        || cover.h > 640
        || cover.rgba.len() != cover.w * cover.h * 4
    {
        return None;
    }
    let mut rgba = cover.rgba.clone();
    for p in rgba.chunks_exact_mut(4) {
        let a = p[3] as u32;
        for v in &mut p[..3] {
            *v = ((*v as u32 * a + 127) / 255) as u8;
        }
    }
    let pixmap = tiny_skia::Pixmap::from_vec(
        rgba,
        tiny_skia::IntSize::from_wh(cover.w as u32, cover.h as u32)?,
    )?;
    let png = pixmap.encode_png().ok()?;
    (png.len() <= MAX_PNG).then_some(CoverData { id, png })
}
fn decode_cover(data: &CoverData) -> Option<Arc<Cover>> {
    png_dimensions(&data.png)?;
    let p = tiny_skia::Pixmap::decode_png(&data.png).ok()?;
    let (w, h) = (p.width() as usize, p.height() as usize);
    if w == 0 || h == 0 || w > 640 || h > 640 {
        return None;
    }
    let mut rgba = p.data().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        let a = pixel[3] as u32;
        if a > 0 {
            for v in &mut pixel[..3] {
                *v = ((*v as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some(Arc::new(Cover { w, h, rgba }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn place(name: &str) -> Place {
        Place {
            name: name.into(),
            lat: -33.87,
            lon: 151.21,
            tz: "Australia/Sydney".into(),
        }
    }
    fn snapshot(now: u64) -> Snapshot {
        let np = NowPlaying::demo("Example", "Artist", true, 10., 100.);
        let mut data = np.shared_data();
        data.state_changed_at_ms = now;
        data.cover = np.cover.as_ref().and_then(|cover| encode_cover(cover, 1));
        Snapshot {
            version: 1,
            generation: 41,
            sequence: 1,
            observed_at_ms: now,
            expires_at_ms: now + 3000,
            music: MusicObservation {
                status: "ok".into(),
                observed_at_ms: Some(now),
                preferred_app: None,
                data: Some(data),
            },
            weather: WeatherObservation {
                status: "inactive".into(),
                observed_at_ms: None,
                place: None,
                data: None,
            },
        }
    }
    #[test]
    fn strict_wire_rejects_bad_identity_timing_and_cover_bounds() {
        let now = now_ms();
        let good = snapshot(now);
        assert!(good.validate(now).is_ok());
        let mut bad = good.clone();
        bad.music.data.as_mut().unwrap().state_changed_at_ms = 0;
        assert!(bad.validate(now).is_err());
        bad = good.clone();
        bad.music.data.as_mut().unwrap().title = "T".repeat(513);
        assert!(bad.validate(now).is_err());
        bad = good.clone();
        bad.expires_at_ms = now + 3001;
        assert!(bad.validate(now).is_err());
        bad = good.clone();
        let png = &mut bad.music.data.as_mut().unwrap().cover.as_mut().unwrap().png;
        png[16..20].copy_from_slice(&641u32.to_be_bytes());
        assert!(bad.validate(now).is_err());
        let mut wire = serde_json::to_value(good).unwrap();
        wire["path"] = json!("not allowed");
        assert!(serde_json::from_value::<Snapshot>(wire).is_err());
    }
    #[test]
    fn generation_replay_and_expired_music_cannot_renew_original_observation() {
        let now = now_ms();
        let mut r = Receiver::default();
        let mut good = snapshot(now);
        r.accept(good.clone(), now).unwrap();
        assert!(r.accept(good.clone(), now).is_err());
        good.sequence = 2;
        good.generation = 42;
        assert!(r.accept(good, now).is_err());
        assert!(r.fresh(now + 3000).is_none());
        assert_eq!(r.snapshot.as_ref().unwrap().expires_at_ms, now + 3000);
    }
    #[test]
    fn pause_age_and_position_anchor_survive_repeated_delivery() {
        let now = now_ms();
        let mut data = NowPlaying::demo("Example", "Artist", false, 17., 100.).shared_data();
        data.state_changed_at_ms = now - 61_000;
        let np = NowPlaying::from_shared(&data, now, None).unwrap();
        assert!(!np.visible());
        assert!((np.position() - 17.).abs() < 0.01);
        let again = NowPlaying::from_shared(&data, now, None).unwrap();
        assert!(!again.visible());
        assert!(NowPlaying::from_shared(&data, now - 3001, None).is_none());
        data.playing = true;
        let np = NowPlaying::from_shared(&data, now - 1000, None).unwrap();
        assert!(np.position() >= 18. && np.position() < 18.1);
    }
    #[test]
    fn png_roundtrip_keeps_full_size_and_known_rgba_pixels() {
        let cover = Cover {
            w: 640,
            h: 640,
            rgba: [40, 80, 120, 255].repeat(640 * 640),
        };
        let wire = encode_cover(&cover, 1).unwrap();
        assert_eq!(png_dimensions(&wire.png), Some((640, 640)));
        let decoded = decode_cover(&wire).unwrap();
        assert_eq!(decoded.rgba, cover.rgba);
    }
    #[test]
    fn receiver_config_binding_and_local_reader_suppression_are_isolated() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live::shared_services::tests::receiver_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("CS_SHARED_SERVICES_TEST", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    }
    #[test]
    #[ignore = "isolated source receiver helper, invoked by parenttest"]
    fn receiver_helper() {
        if std::env::var("CS_SHARED_SERVICES_TEST").as_deref() != Ok("1") {
            return;
        }
        enable_for_uc_worker();
        touch_music(None);
        let now = now_ms();
        let mut s = snapshot(now);
        let status = exchange(Some(s.clone())).unwrap();
        assert_eq!(status["state"]["music_present"], true);
        let first = now_playing().unwrap();
        let again = now_playing().unwrap();
        assert!(Arc::ptr_eq(
            first.cover.as_ref().unwrap(),
            again.cover.as_ref().unwrap()
        ));
        touch_music(Some("Other".into()));
        assert!(now_playing().is_none());
        assert_eq!(exchange(None).unwrap()["state"]["music_present"], false);
        let a = place("Place A");
        let mut data = super::super::weather::sample();
        data.place = a.name.clone();
        s.sequence = 2;
        s.weather = WeatherObservation {
            status: "ok".into(),
            observed_at_ms: Some(now),
            place: Some(a.clone()),
            data: Some(data),
        };
        exchange(Some(s.clone())).unwrap();
        assert!(weather_get(Some(a)).is_ok());
        assert_eq!(exchange(None).unwrap()["state"]["weather_present"], true);
        assert!(weather_get(Some(place("Place B"))).is_err());
        assert_eq!(exchange(None).unwrap()["state"]["weather_present"], false);
        let mut invalid = place(" ");
        invalid.name = String::new();
        assert!(weather_get(Some(invalid)).is_err());
        assert!(exchange(None).unwrap()["demand"]["weather"].is_null());
        s.sequence = 3;
        s.weather.data.as_mut().unwrap().place = "Wrong place".into();
        assert!(exchange(Some(s)).is_err());
        let state = exchange(None).unwrap();
        assert_eq!(state["state"]["local_music_starts"], 0);
        assert_eq!(state["state"]["local_weather_fetches"], 0);
    }
}
