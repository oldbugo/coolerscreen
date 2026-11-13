//! Source-only process. No App, DeviceLease, media server, settings or device engine.
use cs_core::core::sync::MutexExt;
use cs_core::live::{nowplaying::MusicWatcher, shared_services::*, weather};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Read, Write},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    v: u32,
    id: u64,
    command: String,
    args: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exchange {
    generation: u64,
    sequence: u64,
    demand: Demand,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct TestControl {
    music: Option<String>,
    weather: Option<String>,
}
struct MusicActor {
    wanted: Arc<Mutex<Option<MusicDemand>>>,
    observed: Arc<Mutex<MusicObservation>>,
    stop: Arc<AtomicBool>,
    starts: Arc<AtomicU64>,
    reads: Arc<AtomicU64>,
}
fn empty_music(status: &str, pref: Option<String>) -> MusicObservation {
    MusicObservation {
        status: status.into(),
        observed_at_ms: None,
        preferred_app: pref,
        data: None,
    }
}
fn empty_weather() -> WeatherObservation {
    WeatherObservation {
        status: "inactive".into(),
        observed_at_ms: None,
        place: None,
        data: None,
    }
}
fn usable_native_music(data: &MusicData, observed: u64, now: u64) -> bool {
    observed > 0
        && observed <= now
        && now - observed < 3000
        && data.position_s.is_finite()
        && (0. ..=604800.).contains(&data.position_s)
        && data.duration_s.is_finite()
        && (0. ..=604800.).contains(&data.duration_s)
        && data.state_changed_at_ms > 0
        && data.state_changed_at_ms <= now
}
impl MusicActor {
    fn new() -> Self {
        let wanted = Arc::new(Mutex::new(None::<MusicDemand>));
        let observed = Arc::new(Mutex::new(empty_music("inactive", None)));
        let stop = Arc::new(AtomicBool::new(false));
        let starts = Arc::new(AtomicU64::new(0));
        let reads = Arc::new(AtomicU64::new(0));
        let (w, o, s, st, re) = (
            wanted.clone(),
            observed.clone(),
            stop.clone(),
            starts.clone(),
            reads.clone(),
        );
        // A crash in one round starts the loop again from a clean state (1 s, doubling to 30 s), so music does not
        // stop for good; it ends when the worker stops.
        std::thread::spawn(move || cs_core::core::sync::restarting("music source", || s.load(Ordering::SeqCst), || {
            let mut watcher: Option<MusicWatcher> = None;
            let mut previous: Option<MusicDemand> = None;
            let mut encoded: Option<(Arc<cs_core::live::nowplaying::Cover>, CoverData)> = None;
            let mut prior_reads = 0;
            while !s.load(Ordering::SeqCst) {
                let demand = w.lock_or_recover().clone();
                if demand != previous {
                    *o.lock_or_recover() = empty_music(
                        if demand.is_some() {
                            "loading"
                        } else {
                            "inactive"
                        },
                        demand.as_ref().and_then(|d| d.preferred_app.clone()),
                    );
                    match &demand {
                        Some(d) => {
                            if let Some(watcher) = &watcher {
                                watcher.set_preferred(d.preferred_app.clone());
                            } else {
                                watcher = Some(MusicWatcher::start(d.preferred_app.clone()));
                                st.fetch_add(1, Ordering::Relaxed);
                                prior_reads = 0;
                            }
                        }
                        None => {
                            watcher = None;
                            encoded = None;
                        }
                    }
                    previous = demand.clone();
                }
                if let (Some(demand), Some(watcher)) = (&demand, &watcher) {
                    let h = watcher.source_health();
                    if h.reads >= prior_reads {
                        re.fetch_add(h.reads - prior_reads, Ordering::Relaxed);
                    }
                    prior_reads = h.reads;
                    let mut observation = empty_music(h.status, demand.preferred_app.clone());
                    observation.observed_at_ms = h.observed_at_ms;
                    if h.status == "ok" {
                        if let Some(np) = watcher.get() {
                            let mut data = np.shared_data();
                            if let Some(cover) = &np.cover {
                                if encoded
                                    .as_ref()
                                    .is_none_or(|(prior, _)| !Arc::ptr_eq(prior, cover))
                                {
                                    encoded = encode_cover(cover, 1).map(|c| (cover.clone(), c));
                                }
                                data.cover = encoded.as_ref().map(|(_, c)| c.clone());
                            } else {
                                encoded = None;
                            }
                            let observed = np.observed_at_ms();
                            observation.observed_at_ms = Some(observed);
                            if usable_native_music(&data, observed, now_ms()) {
                                observation.data = Some(data);
                            } else {
                                observation.status = "error".into();
                                observation.observed_at_ms =
                                    observation.observed_at_ms.filter(|at| *at <= now_ms());
                            }
                        } else {
                            observation.status = "loading".into();
                        }
                    }
                    if *w.lock_or_recover() == Some(demand.clone()) {
                        *o.lock_or_recover() = observation;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }));
        Self {
            wanted,
            observed,
            stop,
            starts,
            reads,
        }
    }
    fn get(&self, demand: Option<MusicDemand>) -> MusicObservation {
        *self.wanted.lock_or_recover() = demand.clone();
        let observed = self.observed.lock_or_recover().clone();
        if demand.is_none() {
            empty_music("inactive", None)
        } else if observed.preferred_app != demand.as_ref().and_then(|d| d.preferred_app.clone())
            || observed.status == "inactive"
        {
            empty_music("loading", demand.and_then(|d| d.preferred_app))
        } else {
            observed
        }
    }
}
impl Drop for MusicActor {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
struct Sources {
    fake: bool,
    music: Option<MusicActor>,
    music_mode: String,
    weather_mode: String,
    fake_music: MusicObservation,
    fake_weather: WeatherObservation,
    weather_cache: HashMap<String, (WeatherObservation, u64)>,
    last_music: u64,
    last_weather: u64,
    music_starts: u64,
    music_reads: u64,
    weather_fetches: u64,
    last_demand: Demand,
    track_key: Option<(String, String, String, u64)>,
    track_id: u64,
    cover_key: Option<Vec<u8>>,
    cover_id: u64,
    generation: u64,
    sequence: u64,
}
impl Sources {
    fn new(fake: bool) -> Self {
        Self {
            fake,
            music: (!fake).then(MusicActor::new),
            music_mode: "ok".into(),
            weather_mode: "ok".into(),
            fake_music: empty_music("inactive", None),
            fake_weather: empty_weather(),
            weather_cache: HashMap::new(),
            last_music: 0,
            last_weather: 0,
            music_starts: 0,
            music_reads: 0,
            weather_fetches: 0,
            last_demand: Demand::default(),
            track_key: None,
            track_id: 0,
            cover_key: None,
            cover_id: 0,
            generation: 0,
            sequence: 0,
        }
    }
    fn state(&self) -> Value {
        json!({"music_starts":if self.fake{self.music_starts}else{self.music.as_ref().map(|m|m.starts.load(Ordering::Relaxed)).unwrap_or(0)},"music_reads":if self.fake{self.music_reads}else{self.music.as_ref().map(|m|m.reads.load(Ordering::Relaxed)).unwrap_or(0)},"weather_fetches":if self.fake{self.weather_fetches}else{weather::fetch_count()},"music_active":self.last_demand.music.is_some(),"weather_active":self.last_demand.weather.is_some(),"fake":self.fake})
    }
    fn exchange(&mut self, request: Exchange) -> Result<Value, String> {
        request.demand.validate()?;
        if request.generation == 0
            || request.sequence == 0
            || request.generation > 9_007_199_254_740_991
            || request.sequence > 9_007_199_254_740_991
            || self.generation != 0 && request.generation != self.generation
            || request.sequence <= self.sequence
        {
            return Err("Source request outside channel generation".into());
        }
        self.generation = request.generation;
        self.sequence = request.sequence;
        let now = now_ms();
        let demand = request.demand;
        let (mut music, weather) = if self.fake {
            self.fake_exchange(&demand, now)
        } else {
            (
                self.music.as_ref().unwrap().get(demand.music.clone()),
                weather::source_observation(demand.weather.clone()),
            )
        };
        if let Some(data) = &mut music.data {
            let key = (
                data.app.clone(),
                data.title.clone(),
                data.artist.clone(),
                data.track_id,
            );
            if self.track_key.as_ref() != Some(&key) {
                self.track_key = Some(key);
                self.track_id = request.sequence;
            }
            data.track_id = self.track_id;
            if let Some(cover) = &mut data.cover {
                if self.cover_key.as_ref() != Some(&cover.png) {
                    self.cover_key = Some(cover.png.clone());
                    self.cover_id = request.sequence;
                }
                cover.id = self.cover_id;
            }
        }
        self.last_demand = demand;
        let now = now_ms();
        let snapshot = Snapshot {
            version: 1,
            generation: request.generation,
            sequence: request.sequence,
            observed_at_ms: now,
            expires_at_ms: now + 3000,
            music,
            weather,
        };
        snapshot.validate(now)?;
        Ok(json!({"snapshot":snapshot,"state":self.state()}))
    }
    fn fake_exchange(
        &mut self,
        demand: &Demand,
        now: u64,
    ) -> (MusicObservation, WeatherObservation) {
        if let Some(m) = &demand.music {
            if self.last_demand.music.is_none() {
                self.music_starts += 1;
            }
            if self.last_music == 0
                || now.saturating_sub(self.last_music) >= 1000
                || self.last_demand.music != demand.music
            {
                self.music_reads += 1;
                self.last_music = now;
                let mut observation = MusicObservation {
                    status: self.music_mode.clone(),
                    observed_at_ms: Some(now),
                    preferred_app: m.preferred_app.clone(),
                    data: None,
                };
                if self.music_mode == "ok" {
                    let np = cs_core::live::nowplaying::NowPlaying::demo(
                        "Example track",
                        "Example artist",
                        true,
                        10.,
                        240.,
                    );
                    let mut data = np.shared_data();
                    data.state_changed_at_ms = self
                        .fake_music
                        .data
                        .as_ref()
                        .map(|d| d.state_changed_at_ms)
                        .unwrap_or(now);
                    data.cover = self
                        .fake_music
                        .data
                        .as_ref()
                        .and_then(|d| d.cover.clone())
                        .or_else(|| np.cover.as_ref().and_then(|c| encode_cover(c, 1)));
                    observation.data = Some(data);
                }
                self.fake_music = observation;
            }
        } else {
            self.fake_music = empty_music("inactive", None);
            self.last_music = 0;
        }
        if let Some(place) = &demand.weather {
            let cache_key = serde_json::to_string(place).unwrap_or_default();
            if self.fake_weather.place.as_ref() != Some(place) {
                let (data, attempt) = self
                    .weather_cache
                    .get(&cache_key)
                    .cloned()
                    .unwrap_or_else(|| (empty_weather(), 0));
                self.fake_weather = data;
                self.last_weather = attempt;
            }
            let changed = self.fake_weather.place.as_ref() != Some(place);
            let due = self.last_weather == 0
                || changed
                || now.saturating_sub(self.last_weather)
                    >= if self.fake_weather.status == "stale" || self.fake_weather.status == "error"
                    {
                        60_000
                    } else {
                        900_000
                    };
            if due {
                self.weather_fetches += 1;
                self.last_weather = now;
                if self.weather_mode == "ok" {
                    let mut data = weather::sample();
                    data.place = place.name.clone();
                    data.stale = false;
                    self.fake_weather = WeatherObservation {
                        status: "ok".into(),
                        observed_at_ms: Some(now),
                        place: Some(place.clone()),
                        data: Some(data),
                    };
                } else {
                    let retained = (!changed).then(|| self.fake_weather.data.clone()).flatten();
                    let observed = if retained.is_some() {
                        self.fake_weather.observed_at_ms
                    } else {
                        None
                    };
                    self.fake_weather = WeatherObservation {
                        status: if retained.is_some() { "stale" } else { "error" }.into(),
                        observed_at_ms: observed,
                        place: Some(place.clone()),
                        data: retained.map(|mut d| {
                            d.stale = true;
                            d
                        }),
                    };
                }
            }
            self.weather_cache.insert(
                cache_key.clone(),
                (self.fake_weather.clone(), self.last_weather),
            );
            if self.weather_cache.len() > 8 {
                let oldest = self
                    .weather_cache
                    .iter()
                    .filter(|(key, _)| **key != cache_key)
                    .min_by_key(|(_, (_, at))| *at)
                    .map(|(key, _)| key.clone());
                if let Some(oldest) = oldest {
                    self.weather_cache.remove(&oldest);
                }
            }
        }
        (
            self.fake_music.clone(),
            if demand.weather.is_some() {
                self.fake_weather.clone()
            } else {
                empty_weather()
            },
        )
    }
    fn test_control(&mut self, control: TestControl) -> Result<Value, String> {
        if !self.fake || !cfg!(debug_assertions) {
            return Err("Test control requires a fake debug source".into());
        }
        if let Some(m) = control.music {
            if !["ok", "none", "error"].contains(&m.as_str()) {
                return Err("Invalid fake music state".into());
            }
            self.music_mode = m;
            self.last_music = 0;
        }
        if let Some(w) = control.weather {
            if !["ok", "error"].contains(&w.as_str()) {
                return Err("Invalid fake weather state".into());
            }
            self.weather_mode = w;
            self.last_weather = 0;
        }
        Ok(self.state())
    }
}
fn frame(input: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut bytes = Vec::new();
    let n = input
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "Source pipe read failed")?;
    if n == 0 {
        return Ok(None);
    }
    if bytes.len() > MAX_FRAME || bytes.last() != Some(&b'\n') {
        return Err("Invalid source frame".into());
    }
    Ok(Some(bytes))
}
pub fn run() -> Result<(), String> {
    if std::env::var("UC_SOURCES_WORKER").as_deref() != Ok("1") {
        return Err("Source-only mode requires private host launch".into());
    }
    let fake = std::env::var("UC_SOURCES_FAKE").as_deref() == Ok("1");
    if fake && !cfg!(debug_assertions) {
        return Err("Fake sources require debug build".into());
    }
    let mut sources = Sources::new(fake);
    let mut input = BufReader::new(std::io::stdin());
    let mut output = std::io::stdout();
    while let Some(bytes) = frame(&mut input)? {
        let request: Request =
            serde_json::from_slice(&bytes).map_err(|_| "Invalid source request")?;
        if request.v != 1 || request.id == 0 {
            return Err("Invalid source envelope".into());
        }
        let shutting = request.command == "shutdown";
        let result = match request.command.as_str() {
            "exchange" => serde_json::from_value(request.args)
                .map_err(|_| "Invalid source demand".to_string())
                .and_then(|args| sources.exchange(args)),
            "test_control" => serde_json::from_value(request.args)
                .map_err(|_| "Invalid fake source control".to_string())
                .and_then(|args| sources.test_control(args)),
            "shutdown" => {
                if request.args.as_object().is_some_and(|a| a.is_empty()) {
                    Ok(json!({"stopped":true}))
                } else {
                    Err("Invalid shutdown arguments".into())
                }
            }
            _ => Err("Unknown source operation".into()),
        };
        let value = match result {
            Ok(result) => json!({"v":1,"id":request.id,"ok":true,"result":result}),
            Err(_) => {
                json!({"v":1,"id":request.id,"ok":false,"error":{"error":"Source request rejected.","code":"source_request","status":400}})
            }
        };
        let bytes = serde_json::to_vec(&value).map_err(|_| "Source reply encoding failed")?;
        if bytes.len() >= MAX_FRAME {
            return Err("Source reply exceeds frame bound".into());
        }
        output
            .write_all(&bytes)
            .and_then(|_| output.write_all(b"\n"))
            .and_then(|_| output.flush())
            .map_err(|_| "Source pipe write failed")?;
        if shutting {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_native_music_does_not_become_a_whole_source_failure() {
        let good =
            cs_core::live::nowplaying::NowPlaying::demo("Example", "Artist", true, 10., 100.)
                .shared_data();
        let now = now_ms();
        assert!(usable_native_music(&good, now, now));
        let mut invalid = good.clone();
        invalid.position_s = f64::NAN;
        assert!(!usable_native_music(&invalid, now, now));
        invalid = good.clone();
        invalid.duration_s = 604801.;
        assert!(!usable_native_music(&invalid, now, now));
        assert!(!usable_native_music(&good, now - 3000, now));
        assert!(!usable_native_music(&good, now + 1, now));
    }
    fn place(name: &str) -> weather::Place {
        weather::Place {
            name: name.into(),
            lat: -33.87,
            lon: 151.21,
            tz: "Australia/Sydney".into(),
        }
    }
    fn call(s: &mut Sources, seq: u64, demand: Demand) -> Value {
        s.exchange(Exchange {
            generation: 41,
            sequence: seq,
            demand,
        })
        .unwrap()
    }
    #[test]
    fn idle_and_place_changes_preserve_cache_and_original_success_time() {
        let mut s = Sources::new(true);
        let demand = Demand {
            music: None,
            weather: Some(place("Place A")),
        };
        let first = call(&mut s, 1, demand.clone());
        let original = first["snapshot"]["weather"]["observed_at_ms"].clone();
        assert_eq!(first["state"]["weather_fetches"], 1);
        let idle = call(&mut s, 2, Demand::default());
        assert_eq!(idle["state"]["weather_active"], false);
        assert_eq!(idle["snapshot"]["weather"]["status"], "inactive");
        let again = call(&mut s, 3, demand.clone());
        assert_eq!(again["state"]["weather_fetches"], 1);
        assert_eq!(again["snapshot"]["weather"]["observed_at_ms"], original);
        s.test_control(TestControl {
            music: None,
            weather: Some("error".into()),
        })
        .unwrap();
        let failed = call(&mut s, 4, demand);
        assert_eq!(failed["snapshot"]["weather"]["status"], "stale");
        assert_eq!(failed["snapshot"]["weather"]["observed_at_ms"], original);
        assert_eq!(failed["state"]["weather_fetches"], 2);
        let other = call(
            &mut s,
            5,
            Demand {
                music: None,
                weather: Some(place("Place B")),
            },
        );
        assert_eq!(other["snapshot"]["weather"]["status"], "error");
        assert!(other["snapshot"]["weather"]["data"].is_null());
        let a = call(
            &mut s,
            6,
            Demand {
                music: None,
                weather: Some(place("Place A")),
            },
        );
        assert_eq!(a["snapshot"]["weather"]["data"]["place"], "Place A");
        assert_eq!(a["state"]["weather_fetches"], 3);
    }
    #[test]
    fn empty_demand_never_starts_readers_and_fake_cache_is_bounded() {
        let mut s = Sources::new(true);
        let idle = call(&mut s, 1, Demand::default());
        assert_eq!(idle["state"]["music_reads"], 0);
        assert_eq!(idle["state"]["music_starts"], 0);
        assert_eq!(idle["state"]["weather_fetches"], 0);
        for seq in 2..14 {
            call(
                &mut s,
                seq,
                Demand {
                    music: None,
                    weather: Some(place(&format!("Place {seq}"))),
                },
            );
        }
        assert!(s.weather_cache.len() <= 8);
    }
    #[test]
    fn music_none_error_timing_and_ids_are_not_restart_local() {
        let demand = Demand {
            music: Some(MusicDemand {
                preferred_app: None,
            }),
            weather: None,
        };
        let mut a = Sources::new(true);
        let first = call(&mut a, 10, demand.clone());
        let cover = first["snapshot"]["music"]["data"]["cover"]["id"].clone();
        assert_eq!(first["snapshot"]["music"]["data"]["track_id"], 10);
        let repeat = call(&mut a, 11, demand.clone());
        assert_eq!(repeat["snapshot"]["music"]["data"]["cover"]["id"], cover);
        assert_eq!(
            repeat["snapshot"]["music"]["observed_at_ms"],
            first["snapshot"]["music"]["observed_at_ms"]
        );
        let mut restarted = Sources::new(true);
        let next = call(&mut restarted, 100, demand.clone());
        assert_eq!(next["snapshot"]["music"]["data"]["track_id"], 100);
        assert_ne!(next["snapshot"]["music"]["data"]["cover"]["id"], cover);
        a.test_control(TestControl {
            music: Some("none".into()),
            weather: None,
        })
        .unwrap();
        let none = call(&mut a, 12, demand.clone());
        assert_eq!(none["snapshot"]["music"]["status"], "none");
        assert!(none["snapshot"]["music"]["data"].is_null());
        a.test_control(TestControl {
            music: Some("error".into()),
            weather: None,
        })
        .unwrap();
        assert_eq!(
            call(&mut a, 13, demand)["snapshot"]["music"]["status"],
            "error"
        );
    }
    #[test]
    fn channel_and_typed_argument_bounds_are_strict() {
        let mut s = Sources::new(true);
        call(&mut s, 1, Demand::default());
        assert!(s
            .exchange(Exchange {
                generation: 42,
                sequence: 2,
                demand: Demand::default()
            })
            .is_err());
        assert!(s
            .exchange(Exchange {
                generation: 41,
                sequence: 1,
                demand: Demand::default()
            })
            .is_err());
        assert!(serde_json::from_value::<Exchange>(json!({"generation":1,"sequence":1,"demand":{"music":null,"weather":null},"path":"not allowed"})).is_err());
        assert!(frame(&mut std::io::Cursor::new(b"partial")).is_err());
    }
}
