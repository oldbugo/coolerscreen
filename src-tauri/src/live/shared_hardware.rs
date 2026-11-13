//! UC-only injected hardware readings. Standalone never enables this mode.
//! No native reads, network, filesystem access or local fallback lives here.
use crate::core::sync::MutexExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

const LEASE: Duration = Duration::from_secs(2);
static ENABLED: AtomicBool = AtomicBool::new(false);
static LOCAL_LEGACY: AtomicU64 = AtomicU64::new(0);
static LOCAL_METRICS: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Demand {
    pub cpu: bool,
    pub ram: bool,
    pub gpu: bool,
    pub hz: u8,
}
impl Default for Demand {
    fn default() -> Self {
        Self {
            cpu: false,
            ram: false,
            gpu: false,
            hz: 1,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Gpu {
    pub name: String,
    pub temp_c: Option<f64>,
    pub load_pct: Option<f64>,
    pub clock_mhz: Option<f64>,
    pub mem_used_bytes: Option<u64>,
    pub mem_total_bytes: Option<u64>,
    pub power_w: Option<f64>,
    pub fan_pct: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub version: u32,
    pub generation: u64,
    pub sequence: u64,
    pub observed_at_ms: u64,
    pub expires_at_ms: u64,
    pub cpu_load_pct: Option<f64>,
    pub ram_used_bytes: Option<u64>,
    pub ram_total_bytes: Option<u64>,
    pub gpu: Option<Gpu>,
}
fn number(value: Option<f64>, low: f64, high: f64) -> bool {
    value.is_none_or(|v| v.is_finite() && v >= low && v <= high)
}
fn bytes(used: Option<u64>, total: Option<u64>) -> bool {
    match (used, total) {
        (None, None) => true,
        (Some(u), Some(t)) => t > 0 && u <= t && t <= (1u64 << 50),
        _ => false,
    }
}
impl Snapshot {
    pub fn validate(&self, now: u64) -> Result<(), String> {
        if self.version != 1
            || self.generation == 0
            || self.sequence == 0
            || self.generation > 9_007_199_254_740_991
            || self.sequence > 9_007_199_254_740_991
            || self.observed_at_ms == 0
            || self.expires_at_ms > 9_007_199_254_740_991
            || self.observed_at_ms > now
            || self.expires_at_ms <= now
            || self.expires_at_ms <= self.observed_at_ms
            || self.expires_at_ms - self.observed_at_ms > 3000
            || !number(self.cpu_load_pct, 0., 100.)
            || !bytes(self.ram_used_bytes, self.ram_total_bytes)
        {
            return Err("Invalid shared hardware snapshot".into());
        }
        if let Some(g) = &self.gpu {
            if g.name.chars().count() > 128
                || g.name.chars().any(char::is_control)
                || !number(g.temp_c, -40., 200.)
                || !number(g.load_pct, 0., 100.)
                || !number(g.clock_mhz, 0., 20000.)
                || !number(g.power_w, 0., 10000.)
                || !number(g.fan_pct, 0., 100.)
                || !bytes(g.mem_used_bytes, g.mem_total_bytes)
            {
                return Err("Invalid shared hardware GPU reading".into());
            }
        }
        Ok(())
    }
}
#[derive(Default)]
struct State {
    snapshot: Option<Snapshot>,
    generation: u64,
    sequence: u64,
    accepted: u64,
    rejected: u64,
    demand: HashMap<(&'static str, u8), Instant>,
    history: HashMap<&'static str, Vec<(u64, f32)>>,
}
fn state() -> &'static Mutex<State> {
    static STATE: OnceLock<Mutex<State>> = OnceLock::new();
    STATE.get_or_init(|| Mutex::new(State::default()))
}
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}
/// Called only by uc-cooler-worker, before startup. An environment variable in
/// the standalone application cannot enable an unattended replacement source.
pub fn enable_for_uc_worker() {
    *state().lock_or_recover() = State::default();
    ENABLED.store(true, Ordering::SeqCst);
}
pub fn enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}
pub fn local_legacy_poll() {
    LOCAL_LEGACY.fetch_add(1, Ordering::Relaxed);
}
pub fn local_metric_poll() {
    LOCAL_METRICS.fetch_add(1, Ordering::Relaxed);
}
fn category(key: &str) -> Option<&'static str> {
    match key {
        "cpu.load" => Some("cpu"),
        "ram.used" => Some("ram"),
        "gpu.load" | "gpu.temp" | "gpu.clock" | "gpu.vram" | "gpu.power" | "gpu.fan"
        | "gpu.name" => Some("gpu"),
        _ => None,
    }
}
pub fn replaces(key: &str) -> bool {
    enabled() && category(key).is_some()
}
impl State {
    fn request(&mut self, key: &str, hz: u8, at: Instant) {
        if let Some(c) = category(key) {
            let hz = match hz {
                0 | 1 => 1,
                2 => 2,
                _ => 4,
            };
            self.demand.insert((c, hz), at);
        }
    }
    fn demand(&self, at: Instant) -> Demand {
        let active = |key| {
            self.demand
                .iter()
                .filter(|((c, _), t)| *c == key && at.saturating_duration_since(**t) < LEASE)
                .map(|((_, hz), _)| *hz)
                .max()
        };
        let cpu = active("cpu");
        let ram = active("ram");
        let gpu = active("gpu");
        Demand {
            cpu: cpu.is_some(),
            ram: ram.is_some(),
            gpu: gpu.is_some(),
            hz: cpu.into_iter().chain(ram).chain(gpu).max().unwrap_or(1),
        }
    }
    fn fresh(&self, now: u64) -> Option<&Snapshot> {
        self.snapshot
            .as_ref()
            .filter(|s| s.observed_at_ms <= now && now < s.expires_at_ms)
    }
    fn accept(&mut self, snapshot: Snapshot, now: u64) -> Result<(), String> {
        if snapshot.validate(now).is_err()
            || self.generation != 0 && snapshot.generation != self.generation
            || snapshot.generation == self.generation && snapshot.sequence <= self.sequence
            || self
                .snapshot
                .as_ref()
                .is_some_and(|old| snapshot.observed_at_ms < old.observed_at_ms)
        {
            self.rejected += 1;
            return Err("Shared hardware snapshot rejected".into());
        }
        if snapshot.generation != self.generation {
            self.history.clear();
        }
        self.generation = snapshot.generation;
        self.sequence = snapshot.sequence;
        self.accepted += 1;
        for key in [
            "cpu.load",
            "ram.used",
            "gpu.load",
            "gpu.temp",
            "gpu.clock",
            "gpu.vram",
            "gpu.power",
            "gpu.fan",
        ] {
            if let (Some(c), (Some(value), _)) = (category(key), metric(Some(&snapshot), key)) {
                if self
                    .demand
                    .iter()
                    .any(|((cat, _), t)| *cat == c && t.elapsed() < LEASE)
                {
                    let ring = self.history.entry(key).or_default();
                    ring.push((snapshot.observed_at_ms, value));
                    if ring.len() > 1200 {
                        ring.remove(0);
                    }
                }
            }
        }
        self.snapshot = Some(snapshot);
        Ok(())
    }
}
fn metric(snapshot: Option<&Snapshot>, key: &str) -> (Option<f32>, Option<f32>) {
    let Some(s) = snapshot else {
        return (None, None);
    };
    let g = s.gpu.as_ref();
    let f = |v: Option<f64>| v.map(|n| n as f32);
    let gb = |v: Option<u64>| v.map(|n| n as f32 / 1073741824.);
    match key {
        "cpu.load" => (f(s.cpu_load_pct), None),
        "ram.used" => (gb(s.ram_used_bytes), gb(s.ram_total_bytes)),
        "gpu.load" => (f(g.and_then(|g| g.load_pct)), None),
        "gpu.temp" => (f(g.and_then(|g| g.temp_c)), None),
        "gpu.clock" => (f(g.and_then(|g| g.clock_mhz)), None),
        "gpu.vram" => (
            gb(g.and_then(|g| g.mem_used_bytes)),
            gb(g.and_then(|g| g.mem_total_bytes)),
        ),
        "gpu.power" => (f(g.and_then(|g| g.power_w)), None),
        "gpu.fan" => (f(g.and_then(|g| g.fan_pct)), None),
        _ => (None, None),
    }
}
pub fn read_metric(key: &str, hz: u32) -> (Option<f32>, Option<f32>) {
    let mut s = state().lock_or_recover();
    s.request(
        key,
        match hz {
            0 | 1 => 1,
            2 => 2,
            _ => 4,
        },
        Instant::now(),
    );
    metric(s.fresh(now_ms()), key)
}
pub fn gpu_name() -> Option<String> {
    let mut s = state().lock_or_recover();
    s.request("gpu.name", 1, Instant::now());
    s.fresh(now_ms())?
        .gpu
        .as_ref()
        .map(|g| g.name.clone())
        .filter(|n| !n.is_empty())
}
pub fn history(key: &str, secs: f32) -> Vec<(f32, f32)> {
    let s = state().lock_or_recover();
    let now = now_ms();
    if s.fresh(now).is_none() {
        return Vec::new();
    }
    s.history
        .get(key)
        .map(|h| {
            h.iter()
                .filter_map(|(at, v)| {
                    let age = now.checked_sub(*at)? as f32 / 1000.;
                    (age <= secs).then_some((age, *v))
                })
                .collect()
        })
        .unwrap_or_default()
}
/// Read without registering demand: smoothing runs before actual widget drawing.
pub fn peek_metric(key: &str) -> (Option<f32>, Option<f32>) {
    let s = state().lock_or_recover();
    metric(s.fresh(now_ms()), key)
}
const STATS_KEYS: [&str; 5] = ["cpu.load", "ram.used", "gpu.load", "gpu.temp", "gpu.clock"];
pub fn stats(items: &[usize]) -> [(Option<f32>, Option<f32>); 5] {
    let mut s = state().lock_or_recover();
    let at = Instant::now();
    for index in items {
        if let Some(key) = STATS_KEYS.get(*index) {
            s.request(key, 1, at);
        }
    }
    STATS_KEYS.map(|key| metric(s.fresh(now_ms()), key))
}
pub fn stat_text(index: usize, reading: (Option<f32>, Option<f32>)) -> String {
    match (index, reading) {
        (1, (Some(used), Some(total))) => format!("{used:.1} / {total:.1} GB"),
        (0 | 2, (Some(v), _)) => format!("{v:.0} %"),
        (3, (Some(v), _)) => format!("{v:.0} °C"),
        (4, (Some(v), _)) => format!("{v:.0} MHz"),
        _ => "—".into(),
    }
}
pub fn exchange(snapshot: Option<Snapshot>) -> Result<Value, String> {
    if !enabled() {
        return Err("Shared hardware is not enabled in this worker".into());
    }
    let mut s = state().lock_or_recover();
    let now = now_ms();
    if let Some(snapshot) = snapshot {
        s.accept(snapshot, now)?;
    }
    let fresh = s.fresh(now);
    let gpu = fresh.and_then(|s| s.gpu.as_ref());
    let gpu_fields = gpu
        .map(|g| {
            [
                g.temp_c.is_some(),
                g.load_pct.is_some(),
                g.clock_mhz.is_some(),
                g.mem_used_bytes.is_some(),
                g.mem_total_bytes.is_some(),
                g.power_w.is_some(),
                g.fan_pct.is_some(),
                !g.name.is_empty(),
            ]
            .into_iter()
            .enumerate()
            .fold(0u8, |mask, (i, has)| mask | if has { 1 << i } else { 0 })
        })
        .unwrap_or(0);
    let demand = s.demand(Instant::now());
    let legacy = LOCAL_LEGACY.load(Ordering::Relaxed);
    let metrics = LOCAL_METRICS.load(Ordering::Relaxed);
    let expires_at_ms = s.snapshot.as_ref().map(|snapshot| snapshot.expires_at_ms);
    Ok(
        json!({"demand":demand,"state":{"shared":true,"accepted":s.accepted,"rejected":s.rejected,"generation":s.generation,"accepted_sequence":s.sequence,"expires_at_ms":expires_at_ms,"fresh":fresh.is_some(),"demand":demand,"cpu_present":fresh.is_some_and(|s|s.cpu_load_pct.is_some()),"ram_present":fresh.is_some_and(|s|s.ram_used_bytes.is_some()&&s.ram_total_bytes.is_some()),"gpu_fields":gpu_fields,"local_polls":legacy+metrics,"local_legacy_polls":legacy,"local_metric_polls":metrics}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot() -> Snapshot {
        Snapshot {
            version: 1,
            generation: 41,
            sequence: 1,
            observed_at_ms: 1000,
            expires_at_ms: 4000,
            cpu_load_pct: Some(42.),
            ram_used_bytes: Some(8 << 30),
            ram_total_bytes: Some(32 << 30),
            gpu: Some(Gpu {
                name: "Example GPU".into(),
                temp_c: Some(61.),
                load_pct: Some(52.),
                clock_mhz: Some(1950.),
                mem_used_bytes: Some(6 << 30),
                mem_total_bytes: Some(16 << 30),
                power_w: Some(182.5),
                fan_pct: Some(46.),
            }),
        }
    }
    #[test]
    fn strict_snapshot_schema_and_numeric_validation() {
        let good = snapshot();
        assert!(good.validate(1000).is_ok());
        let mut json = serde_json::to_value(&good).unwrap();
        json["accountId"] = json!("rejected");
        assert!(serde_json::from_value::<Snapshot>(json).is_err());
        let mut bad = good.clone();
        bad.cpu_load_pct = Some(f64::NAN);
        assert!(bad.validate(1000).is_err());
        bad = good.clone();
        bad.ram_total_bytes = None;
        assert!(bad.validate(1000).is_err());
        bad = good.clone();
        bad.gpu.as_mut().unwrap().mem_used_bytes = Some(17 << 30);
        assert!(bad.validate(1000).is_err());
        bad = good.clone();
        bad.gpu.as_mut().unwrap().name = "bad\nname".into();
        assert!(bad.validate(1000).is_err());
        bad = good.clone();
        bad.observed_at_ms = 1001;
        assert!(bad.validate(1000).is_err());
        bad = good.clone();
        bad.expires_at_ms = 4001;
        assert!(bad.validate(1000).is_err());
        assert!(good.validate(4000).is_err());
    }
    #[test]
    fn channel_generation_sequence_and_original_deadline_cannot_be_replayed() {
        let mut s = State::default();
        let good = snapshot();
        s.accept(good.clone(), 1000).unwrap();
        assert!(s.accept(good.clone(), 2000).is_err());
        let mut changed = good.clone();
        changed.generation += 1;
        changed.sequence = 2;
        assert!(s.accept(changed, 2000).is_err());
        changed = good.clone();
        changed.sequence = 2;
        changed.observed_at_ms = 999;
        assert!(s.accept(changed, 2000).is_err());
        assert!(s.fresh(3999).is_some());
        assert!(s.fresh(4000).is_none());
        assert_eq!(
            s.snapshot.as_ref().map(|snapshot| snapshot.expires_at_ms),
            Some(4000)
        );
        assert_eq!(s.accepted, 1);
        assert_eq!(s.rejected, 3);
        let mut replacement = State::default();
        replacement.accept(good, 1000).unwrap();
        assert_eq!(replacement.sequence, 1);
    }
    #[test]
    fn union_demand_comes_from_reads_and_high_rate_expires_independently() {
        let mut s = State::default();
        let at = Instant::now();
        assert_eq!(s.demand(at), Demand::default());
        s.request("cpu.load", 1, at);
        s.request("gpu.temp", 4, at);
        s.request("ram.used", 2, at);
        assert_eq!(
            s.demand(at),
            Demand {
                cpu: true,
                ram: true,
                gpu: true,
                hz: 4
            }
        );
        let later = at + Duration::from_millis(1900);
        s.request("gpu.load", 1, later);
        assert_eq!(
            s.demand(at + Duration::from_millis(2100)),
            Demand {
                cpu: false,
                ram: false,
                gpu: true,
                hz: 1
            }
        );
        assert_eq!(s.demand(at + Duration::from_secs(4)), Demand::default());
        // An exchange or hardware failure changes readings, never widget demand.
        s.accept(snapshot(), 1000).unwrap();
        assert_eq!(s.demand(at + Duration::from_secs(4)), Demand::default());
    }
    #[test]
    fn rich_fields_preserve_units_and_provider_failure_clears_every_number() {
        let mut s = State::default();
        s.accept(snapshot(), 1000).unwrap();
        let fresh = s.fresh(1001);
        assert_eq!(metric(fresh, "ram.used"), (Some(8.), Some(32.)));
        assert_eq!(metric(fresh, "gpu.vram"), (Some(6.), Some(16.)));
        assert_eq!(metric(fresh, "gpu.power"), (Some(182.5), None));
        assert_eq!(metric(fresh, "gpu.fan"), (Some(46.), None));
        assert_eq!(stat_text(1, metric(fresh, "ram.used")), "8.0 / 32.0 GB");
        assert_eq!(stat_text(3, metric(fresh, "gpu.temp")), "61 °C");
        assert_eq!(stat_text(4, metric(fresh, "gpu.clock")), "1950 MHz");
        let mut failed = snapshot();
        failed.sequence = 2;
        failed.cpu_load_pct = None;
        failed.ram_used_bytes = None;
        failed.ram_total_bytes = None;
        failed.gpu = None;
        s.accept(failed, 1001).unwrap();
        for (index, key) in STATS_KEYS.iter().enumerate() {
            let reading = metric(s.fresh(1001), key);
            assert_eq!(reading, (None, None));
            assert_eq!(stat_text(index, reading), "—");
        }
        assert_eq!(metric(s.fresh(4000), "gpu.load"), (None, None));
    }
    #[test]
    fn history_records_original_observation_once_and_never_fake_failure_zero() {
        let mut s = State::default();
        s.request("cpu.load", 1, Instant::now());
        let good = snapshot();
        s.accept(good.clone(), 1000).unwrap();
        assert_eq!(s.history["cpu.load"], vec![(1000, 42.)]);
        assert!(s.accept(good, 1001).is_err());
        assert_eq!(s.history["cpu.load"].len(), 1);
        let mut failed = snapshot();
        failed.sequence = 2;
        failed.cpu_load_pct = None;
        s.accept(failed, 1001).unwrap();
        assert_eq!(s.history["cpu.load"].len(), 1);
    }
    #[test]
    fn unshared_metric_categories_stay_local() {
        for key in [
            "cpu.core:1",
            "cpu.clock",
            "cpu.temp",
            "disk.used:C",
            "net.down:*",
            "board.temp",
        ] {
            assert!(category(key).is_none());
        }
        assert!(!enabled());
    }
    #[test]
    fn shared_mode_isolates_legacy_and_modern_renderers_from_native_collectors() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "live::shared_hardware::tests::shared_mode_helper",
                "--ignored",
                "--nocapture",
            ])
            .env("CS_SHARED_HARDWARE_TEST", "1")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stdout)
        );
    }
    #[test]
    #[ignore = "isolated process helper; never enable global source mode in parallel library tests"]
    fn shared_mode_helper() {
        if std::env::var("CS_SHARED_HARDWARE_TEST").as_deref() != Ok("1") {
            return;
        }
        enable_for_uc_worker();
        let sampler = super::super::sensors::Sampler::start();
        for key in [
            "cpu.load",
            "ram.used",
            "gpu.load",
            "gpu.temp",
            "gpu.vram",
            "gpu.power",
            "gpu.fan",
        ] {
            assert_eq!(super::super::metrics::hub().read(key, 4), (None, None));
        }
        assert!(super::super::metrics::hub().gpu_name().is_none());
        assert_eq!(LOCAL_LEGACY.load(Ordering::SeqCst), 0);
        assert_eq!(LOCAL_METRICS.load(Ordering::SeqCst), 0);
        // Rendering (not diagnostics) owns demand. Only RAM stats and a visible
        // 4 Hz GPU widget should subscribe; a hidden CPU widget cannot do so.
        enable_for_uc_worker();
        let mut theme = super::super::theme::Theme::default();
        theme.widgets = vec![
            super::super::theme::Widget::new(
                "ram",
                "stats",
                0.,
                0.,
                0.4,
                0.4,
                json!({"items":["ram"]}),
            ),
            super::super::theme::Widget::new(
                "gpu",
                "sensor",
                0.4,
                0.,
                0.3,
                0.3,
                json!({"metric":"gpu.power","rate":4}),
            ),
            super::super::theme::Widget::new(
                "hidden",
                "sensor",
                0.,
                0.5,
                0.3,
                0.3,
                json!({"metric":"cpu.load","rate":4}),
            ),
        ];
        theme.widgets[2].hidden = true;
        let mut renderer = super::super::render::Renderer::new(sampler.data.clone(), None);
        renderer.set_theme(super::super::theme::ThemeLive::new(
            theme,
            std::path::PathBuf::new(),
        ));
        renderer.render();
        let status = exchange(None).unwrap();
        assert!(status["state"]["expires_at_ms"].is_null());
        assert_eq!(
            status["demand"],
            json!({"cpu":false,"ram":true,"gpu":true,"hz":4})
        );
        assert_eq!(status["state"]["local_polls"], 0);
        let mut sample = snapshot();
        sample.observed_at_ms = now_ms();
        sample.expires_at_ms = sample.observed_at_ms + 3000;
        let original_expiry = sample.expires_at_ms;
        let mut rejected = sample.clone();
        rejected.expires_at_ms += 1;
        let status = exchange(Some(sample)).unwrap();
        assert_eq!(status["state"]["cpu_present"], true);
        assert_eq!(status["state"]["expires_at_ms"], original_expiry);
        assert!(exchange(Some(rejected)).is_err());
        assert_eq!(
            exchange(None).unwrap()["state"]["expires_at_ms"],
            original_expiry
        );
        assert_eq!(
            super::super::metrics::hub().read("gpu.power", 4),
            (Some(182.5), None)
        );
        assert_eq!(
            super::super::metrics::hub().gpu_name().as_deref(),
            Some("Example GPU")
        );
        assert_eq!(LOCAL_LEGACY.load(Ordering::SeqCst), 0);
        assert_eq!(LOCAL_METRICS.load(Ordering::SeqCst), 0);
        // A source failure is a fresh, explicitly unknown sample, never zero.
        let mut failure = snapshot();
        failure.sequence = 2;
        failure.observed_at_ms = now_ms();
        failure.expires_at_ms = failure.observed_at_ms + 3000;
        failure.cpu_load_pct = None;
        failure.ram_used_bytes = None;
        failure.ram_total_bytes = None;
        failure.gpu = None;
        exchange(Some(failure)).unwrap();
        assert_eq!(
            super::super::metrics::hub().read("gpu.power", 4),
            (None, None)
        );
        assert!(super::super::metrics::hub().gpu_name().is_none());
        super::super::metrics::hub().shutdown();
        drop(sampler);
    }
}
