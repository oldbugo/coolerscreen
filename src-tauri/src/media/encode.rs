//! Estimate ("Best that fits") and convert, decode-once design (perf report 2026-10-11):
//!
//! 1. The selection is decoded + filtered ONCE (framing, notch, corners, blend, rotation) to raw yuvj420p frames in a
//!    temp file. The raw input must be tagged `-color_range pc` when encoding, or the range is expanded twice and the
//!    files come out 11 % bigger; with the tag, an encode from raw is bit-identical to 1.2's direct encode.
//! 2. Trial encodes from raw are cheap (130-550 ms) and give the EXACT full-length size, so no 2 s sample, no 0.97
//!    margin and no "fill the space" re-encodes are needed: two probes (q=4, q=16) -> log-size/log-q model -> one
//!    parallel round around the model's q -> the smallest q whose real file fits the budget.
//! 3. "Best that fits" = that exact q with no blur; blur levels (30/60/100) only when even q=69 does not fit (as 1.2).
//! 4. Save as version reuses the winning encode (copied), and makes the H.264 preview from the same raw frames.
//!
//! Raw intermediates are kept in an LRU cache keyed by everything that changes the frames, so a quality-only change or
//! a save right after an estimate costs nothing but the preview. Selections whose raw frames would exceed
//! [`RAW_MAX_BYTES`] fall back to 1.2's method (2 s sample from raw, extrapolate with a 0.97 margin, direct encode).
use super::assets::corner_mask_file;
use super::ffmpeg::{self, p};
use super::graph::{self, device_graph_ov, input_args_ov, mjpeg_args, q_from_quality, quality_from_q, Base, Corners, Frame, Mask};
use super::graph::{DEVICE_FPS, IMAGE_SECONDS, Q_MAX, Q_MIN};
use super::{gpu, probe, r3, CancelToken, MediaConfig, MediaError, Progress, Result, SourceInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// `CS_MEDIA_DEBUG=1`: stage timings on stderr.
fn debug(msg: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("CS_MEDIA_DEBUG").is_some_and(|v| v == "1")) {
        eprintln!("[media] {}", msg());
    }
}

/// Bytes of one stored 1080x2400 4:2:0 frame.
pub const RAW_FRAME_BYTES: u64 = 1080 * 2400 * 3 / 2;
/// Above this the selection is not kept as raw frames (about 44 s of output); see the module docs.
pub const RAW_MAX_BYTES: u64 = 3 << 30;
/// Raw intermediates kept (LRU), and their total size cap.
const CACHE_ENTRIES: usize = 3;
pub const CACHE_MAX_BYTES: u64 = 4 << 30;
/// Free space always left on the temp volume; a raw intermediate that would cut into it is not made (sampled path).
pub const DISK_RESERVE_BYTES: u64 = 2 << 30;
/// Sampled path (selections over RAW_MAX_BYTES or low disk): this many 1 s segments spread over the selection.
const SAMPLE_SEGMENTS: usize = 4;

/// The default per-process scratch folder (`%TEMP%\CoolerScreen-media\run-<pid>`); folders of other (dead) runs next to
/// it are deleted at startup, skipping files that are still locked.
pub fn default_run_dir() -> PathBuf {
    std::env::temp_dir().join("CoolerScreen-media").join(format!("run-{}", std::process::id()))
}

/// Scratch folders of every engine made in this process (for [`super::shutdown`]).
pub(crate) static RUN_DIRS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
/// Parallel trial encodes and their ffmpeg threads.
const TRIAL_THREADS: u32 = 4;

/// Auto mode blur ("simplify") levels: none first, the rest only as a last resort (see convert.js chooseAuto).
pub const AUTO_LEVELS: [f64; 4] = [0.0, 30.0, 60.0, 100.0];
/// Quality used to size loop-length suggestions.
pub const AUTO_MIN_QUALITY: f64 = 45.0;

/// One version's settings, as the library hands them over (1.2 convert/estimate opts).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRequest {
    pub start: f64,
    /// None or <= 0: as long as fits `max_seconds` and the clip.
    pub length: Option<f64>,
    /// None = "auto" (Best that fits); Some(1..100) = manual quality.
    pub quality: Option<f64>,
    /// Simplify level 0-100 (manual only; auto picks it).
    #[serde(default)]
    pub detail: f64,
    #[serde(default)]
    pub frame: Frame,
    #[serde(default)]
    pub mask: Mask,
    #[serde(default)]
    pub corners: Corners,
    #[serde(default)]
    pub blend: f64,
    pub budget_bytes: u64,
    #[serde(default)]
    pub max_seconds: Option<f64>,
    /// 3.0 themes: a 2400x1080 RGBA PNG (text/picture items) drawn over the picture, under the notch corners.
    #[serde(default)]
    pub overlay: Option<PathBuf>,
}

/// What 1.2's estimate() returned (sizes are now exact, not extrapolated, unless `exact` is false).
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EstimateResult {
    pub mode: &'static str,
    pub q: u8,
    pub quality: u8,
    pub detail: f64,
    pub bytes: u64,
    pub fits: bool,
    pub max_fit_length: Option<f64>,
    pub start: f64,
    pub length: f64,
    pub budget: u64,
    pub duration: Option<f64>,
    pub fps: u32,
    /// true = real full-length size of that encode; false = 2 s sample extrapolation (very long selections).
    pub exact: bool,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConvertResult {
    pub size: u64,
    pub q: u8,
    pub quality: u8,
    pub detail: f64,
    pub too_big: bool,
    pub fps: u32,
    pub start: f64,
    pub length: f64,
    pub frame: Frame,
    pub blend: f64,
    pub duration: Option<f64>,
    pub is_image: bool,
}

/// convert.js prepare(): the selection resolved against the source.
#[derive(Clone, Debug)]
struct Prep {
    file: PathBuf,
    info: SourceInfo,
    start: f64,
    length: f64,
    blend: f64,
    manual_q: Option<u8>,
    budget: u64,
    base: Base,
    mask: Mask,
    corner_file: Option<PathBuf>,
    overlay_file: Option<PathBuf>,
    hw: bool,
}

impl Prep {
    fn with_detail(&self, d: f64) -> Prep {
        let mut c = self.clone();
        c.base.detail = d;
        c
    }
    fn frames(&self) -> u64 {
        super::js_round(self.length * DEVICE_FPS as f64) as u64
    }
    /// Everything that changes the decoded+filtered frames.
    fn key(&self) -> String {
        let meta = std::fs::metadata(&self.file).ok();
        let stamp = meta
            .map(|m| format!("{}-{:?}", m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())))
            .unwrap_or_default();
        format!(
            "{}|{}|{}|{}|{}|{:?}|{}|{:?}|{:?}|{:?}|{}",
            self.file.display(),
            stamp,
            self.base.start,
            self.base.length,
            self.base.blend,
            self.base.frame,
            self.base.detail,
            self.mask,
            self.corner_file,
            self.overlay_file,
            self.hw
        )
    }
    /// Input args and the full device graph (ending in [out]).
    fn graph_parts(&self, start: f64, length: f64, blend: f64, hw: bool) -> (Vec<String>, String) {
        let mut b = self.base.clone();
        b.start = start;
        b.length = length;
        b.blend = blend;
        let mut a: Vec<String> = if hw { crate::ff_args!["-hwaccel", "cuda"] } else { vec![] };
        a.extend(input_args_ov(&p(&self.file), &b, self.corner_file.as_deref().map(p).as_deref(), self.overlay_file.as_deref().map(p).as_deref()));
        (a, device_graph_ov(&b, &self.mask, self.corner_file.is_some(), self.overlay_file.is_some()))
    }
    /// Input + graph args of the full device graph (without output args).
    fn graph_args(&self, start: f64, length: f64, blend: f64, hw: bool) -> Vec<String> {
        let (mut a, g) = self.graph_parts(start, length, blend, hw);
        a.extend(crate::ff_args!["-filter_complex", g, "-map", "[out]", "-an"]);
        a
    }
}

/// First guess: log(size) linear in log(q) through q=4 and q=16 (perf report: exact in 3/4 clips, else off by 1-2).
pub(crate) fn model_guess(s4: f64, s16: f64, limit: f64) -> u8 {
    let k = (s16.ln() - s4.ln()) / (16f64.ln() - 4f64.ln());
    let qm = if k.is_finite() && k < 0.0 { (4f64.ln() + (limit.ln() - s4.ln()) / k).exp().round() } else { 16.0 };
    (qm as i64).clamp(Q_MIN as i64, Q_MAX as i64) as u8
}

/// One step of the quantiser search over the known sizes (q=Q_MAX must be known and fit): Ok(q) when q is the smallest
/// fitting quantiser (q-1 known not to fit, or q = Q_MIN), else Err(the next quantisers to try in parallel).
/// The search keeps a bracket [largest known q that does not fit, smallest known q that fits] and interpolates log(size)
/// over log(q) inside it, so a model miss costs one more round instead of a walk (2026-10-12: a 7 s 2400x1080 clip whose
/// first guess was 24 while the answer was 26 walked down from 69 in steps of 3: 17 rounds, 11 s).
pub(crate) fn next_trials(sizes: &BTreeMap<u8, u64>, scale: f64, limit: f64) -> std::result::Result<u8, Vec<u8>> {
    let fits = |b: u64| (b as f64) * scale <= limit;
    let hi = sizes.iter().filter(|(_, &b)| fits(b)).map(|(&q, _)| q).min().expect("Q_MAX fits");
    let lo = sizes.iter().filter(|(&q, &b)| q < hi && !fits(b)).map(|(&q, _)| q).max();
    if hi == Q_MIN || lo == Some(hi - 1) {
        return Ok(hi);
    }
    let lo_q = lo.unwrap_or(Q_MIN - 1); // nothing known below hi: everything down to Q_MIN is open
    let gap = hi - lo_q - 1;
    if gap <= 3 {
        return Err((lo_q + 1..hi).collect());
    }
    // interpolate in log-log between the bracket ends (lo exists here: hi > Q_MIN + 3 implies q=4 was tried)
    let qi = match lo {
        Some(l) => {
            let (sl, sh) = ((sizes[&l] as f64 * scale).ln(), (sizes[&hi] as f64 * scale).ln());
            let (ql, qh) = ((l as f64).ln(), (hi as f64).ln());
            let t = if sl > sh { (sl - limit.ln()) / (sl - sh) } else { 0.5 };
            (ql + t.clamp(0.0, 1.0) * (qh - ql)).exp().round() as i64
        }
        None => (lo_q as i64 + hi as i64) / 2,
    };
    let qi = qi.clamp(lo_q as i64 + 2, hi as i64 - 2) as u8;
    Err(vec![qi - 1, qi, qi + 1])
}

/// A raw intermediate and the trial encodes made from it.
struct RawEntry {
    key: String,
    dir: PathBuf,
    /// raw.nut, or for a sample the concat list of its segments
    raw: PathBuf,
    /// Output seconds the raw covers (the selection, or the sample segments together).
    seconds: f64,
    sample: bool,
    /// Expected size on disk (cache accounting).
    bytes: u64,
    state: Mutex<RawState>,
}
#[derive(Default)]
struct RawState {
    ready: bool,
    /// q -> bytes of dir/q{q}.mp4
    sizes: BTreeMap<u8, u64>,
}
impl RawEntry {
    fn trial_path(&self, q: u8) -> PathBuf {
        self.dir.join(format!("q{q}.mp4"))
    }
    /// ffmpeg input args reading the raw frames (tagged full range: see the module docs).
    fn input_args(&self) -> Vec<String> {
        if self.sample {
            crate::ff_args!["-f", "concat", "-safe", "0", "-color_range", "pc", "-i", p(&self.raw)]
        } else {
            crate::ff_args!["-color_range", "pc", "-i", p(&self.raw)]
        }
    }
}

/// The media engine: configuration, GPU caps and the raw-intermediate cache. Cheap to share (`Arc<Engine>`); all
/// methods take `&self` and may run concurrently.
pub struct Engine {
    cfg: MediaConfig,
    /// This process's scratch folder: `<work_dir>/run-<pid>`.
    dir: PathBuf,
    cache: Mutex<Vec<Arc<RawEntry>>>,
    seq: std::sync::atomic::AtomicU64,
}

/// Lock a mutex while staying cancellable (an operation on the same selection may be running).
fn lock_cancellable<'a, T>(m: &'a Mutex<T>, cancel: &CancelToken) -> Result<std::sync::MutexGuard<'a, T>> {
    loop {
        cancel.check()?;
        match m.try_lock() {
            Ok(g) => return Ok(g),
            Err(std::sync::TryLockError::Poisoned(p)) => return Ok(p.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(8)),
        }
    }
}

impl Engine {
    /// Make the engine. Scratch files go to `<work_dir>/run-<pid>`; what earlier runs left in `work_dir` is deleted
    /// (best effort: files still locked by an exiting process are skipped and retried whenever a new raw intermediate is
    /// made). Call [`gpu::detect`] at startup to enable NVDEC paths.
    pub fn new(cfg: MediaConfig) -> Engine {
        let dir = cfg.work_dir.join(format!("run-{}", std::process::id()));
        let eng = Engine { cfg, dir, cache: Mutex::new(vec![]), seq: Default::default() };
        eng.remove_stale_runs();
        let _ = super::sys::remove_tree_best_effort(&eng.dir);
        let _ = std::fs::create_dir_all(&eng.dir);
        RUN_DIRS.lock().unwrap().push(eng.dir.clone());
        eng
    }
    /// Delete what dead runs left in work_dir (`run-<pid>` folders of processes that are gone, and loose files from
    /// before 2.0's per-run folders), skipping locked files. Folders of live processes (cs2 next to the app) are kept.
    fn remove_stale_runs(&self) {
        if let Ok(rd) = std::fs::read_dir(&self.cfg.work_dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p == self.dir {
                    continue;
                }
                let pid = e.file_name().to_str().and_then(|n| n.strip_prefix("run-")).and_then(|n| n.parse::<u32>().ok());
                if pid.is_some_and(super::sys::pid_alive) {
                    continue;
                }
                if p.is_dir() {
                    super::sys::remove_tree_best_effort(&p);
                } else {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }
    }
    /// This run's scratch folder.
    pub fn run_dir(&self) -> &Path {
        &self.dir
    }
    /// Delete every raw intermediate and this run's scratch folder. Call when the app exits (ffmpeg children are
    /// killed by the app's Job Object, see [`super::sys::adopt_child`]); the next start also removes leftovers.
    pub fn shutdown(&self) {
        self.clear_cache();
        let _ = super::sys::remove_tree_best_effort(&self.dir);
    }
    /// Bytes held by the raw cache.
    pub fn cache_bytes(&self) -> u64 {
        self.cache.lock().unwrap().iter().map(|e| e.bytes).sum()
    }
    pub fn config(&self) -> &MediaConfig {
        &self.cfg
    }
    pub fn ffmpeg(&self) -> &Path {
        &self.cfg.ffmpeg
    }
    pub fn probe(&self, file: &Path) -> Result<SourceInfo> {
        probe::probe(&self.cfg.ffmpeg, file)
    }
    /// Drop all raw intermediates (e.g. when the notch changes or on low disk).
    pub fn clear_cache(&self) {
        let mut c = self.cache.lock().unwrap();
        for e in c.drain(..) {
            super::sys::remove_tree_best_effort(&e.dir);
        }
    }

    /// Make room for `need` more bytes of raw frames: evict unused entries (oldest first) until the cache stays under
    /// its entry count and [`CACHE_MAX_BYTES`], and the volume keeps [`DISK_RESERVE_BYTES`] free. False when it cannot.
    fn make_room(cache: &mut Vec<Arc<RawEntry>>, need: u64, dir: &Path) -> bool {
        let evict_one = |cache: &mut Vec<Arc<RawEntry>>| match cache.iter().position(|x| Arc::strong_count(x) == 1) {
            Some(i) => {
                let old = cache.remove(i);
                super::sys::remove_tree_best_effort(&old.dir);
                true
            }
            None => false,
        };
        while cache.len() >= CACHE_ENTRIES || cache.iter().map(|e| e.bytes).sum::<u64>() + need > CACHE_MAX_BYTES {
            if !evict_one(cache) {
                return false;
            }
        }
        loop {
            match super::sys::free_bytes(dir) {
                Some(free) if free < need + DISK_RESERVE_BYTES => {
                    if !evict_one(cache) {
                        return false;
                    }
                }
                _ => return true, // enough, or unknown (non-Windows)
            }
        }
    }

    fn prepare(&self, file: &Path, info: Option<&SourceInfo>, req: &VersionRequest, cancel: &CancelToken) -> Result<Prep> {
        let info = match info {
            Some(i) => i.clone(),
            None => self.probe(file)?,
        };
        if !info.has_video {
            return Err(MediaError::Invalid("No video/image stream found in this file.".into()));
        }
        let is_image = info.is_image;
        let start = if is_image { 0.0 } else { r3(req.start.max(0.0)) };
        let avail = match (info.duration, is_image) {
            (Some(d), false) => (d - start).max(0.1),
            _ => f64::INFINITY,
        };
        let mut length = req.length.unwrap_or(0.0);
        if !(length > 0.0) {
            length = if is_image { IMAGE_SECONDS } else { req.max_seconds.filter(|m| *m > 0.0).unwrap_or(10.0).min(avail) };
        }
        length = r3(length.min(avail).max(0.1));
        if !length.is_finite() {
            length = 10.0;
        }
        let manual_q = req.quality.map(q_from_quality);
        let mut blend = if is_image { 0.0 } else { req.blend.min(1.5).min(length / 2.0).min(start).max(0.0) };
        blend = (blend * DEVICE_FPS as f64 + 1e-6).floor() / DEVICE_FPS as f64;
        let corner_file = corner_mask_file(&self.cfg.ffmpeg, &self.cfg.base_dir, &req.corners, &req.mask, cancel)?;
        let hw = gpu::use_nvdec_convert(&info);
        Ok(Prep {
            file: file.to_path_buf(),
            start,
            length,
            blend,
            manual_q,
            budget: req.budget_bytes,
            base: Base {
                start,
                length,
                blend: r3(blend),
                fps: DEVICE_FPS,
                src_fps: info.fps,
                is_image,
                frame: req.frame.norm(),
                detail: if req.quality.is_some() { req.detail } else { 0.0 },
                canvas: (graph::W, graph::H),
                flags: graph::LANCZOS.into(),
                head: String::new(),
            },
            info,
            mask: req.mask,
            corner_file,
            overlay_file: req.overlay.clone().filter(|f| f.is_file()),
            hw,
        })
    }

    /// The raw intermediate for this selection (made now, or from the cache).
    fn raw_entry(&self, c: &Prep, cancel: &CancelToken, progress: &(dyn Fn(f64) + Sync)) -> Result<Arc<RawEntry>> {
        let need = c.frames() * RAW_FRAME_BYTES;
        let raw_max = std::env::var("CS_MEDIA_RAW_MAX").ok().and_then(|v| v.parse().ok()).unwrap_or(RAW_MAX_BYTES);
        let base_key = c.key();
        let entry = {
            let mut cache = self.cache.lock().unwrap();
            let hit = cache
                .iter()
                .position(|e| e.key == format!("{base_key}|false"))
                .or_else(|| cache.iter().position(|e| e.key == format!("{base_key}|true")));
            if let Some(i) = hit {
                let e = cache.remove(i);
                cache.push(e.clone()); // most recent last
                e
            } else {
                // whole selection as raw frames when it is small enough and the disk has room, else the sampled path
                let _ = std::fs::create_dir_all(&self.dir);
                let sample = need > raw_max || !Self::make_room(&mut cache, need, &self.dir);
                if sample {
                    Self::make_room(&mut cache, 0, &self.dir);
                }
                let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let dir = self.dir.join(format!("sel{n}"));
                let (seconds, bytes) = if sample {
                    let seg = (c.length / SAMPLE_SEGMENTS as f64).min(1.0);
                    let s = seg * SAMPLE_SEGMENTS as f64;
                    (s, (s * DEVICE_FPS as f64).ceil() as u64 * RAW_FRAME_BYTES)
                } else {
                    (c.length, need)
                };
                let raw = dir.join(if sample { "segments.txt" } else { "raw.nut" });
                let e = Arc::new(RawEntry { key: format!("{base_key}|{sample}"), raw, dir, seconds, sample, bytes, state: Default::default() });
                cache.push(e.clone());
                e
            }
        };
        let mut st = lock_cancellable(&entry.state, cancel)?;
        if !st.ready {
            self.remove_stale_runs(); // retry leftovers of earlier runs that were locked at startup
            std::fs::create_dir_all(&entry.dir)?;
            let t0 = std::time::Instant::now();
            let r = if entry.sample {
                self.make_sample(c, &entry, cancel, progress)
            } else {
                let secs = entry.seconds;
                let on_time = |t: f64| progress((t / secs).min(1.0));
                let mk = |hw: bool| {
                    let mut a = c.graph_args(c.base.start, c.base.length, c.base.blend, hw);
                    a.extend(crate::ff_args!["-c:v", "rawvideo", "-f", "nut", "-y", p(&entry.raw)]);
                    ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, Some(&on_time))
                };
                match mk(c.hw) {
                    Err(MediaError::Ffmpeg(_)) if c.hw => mk(false), // NVDEC refused this stream: CPU decode
                    r => r,
                }
            };
            debug(|| format!("raw {:.2}s (sample {}, detail {}) in {:?}", entry.seconds, entry.sample, c.base.detail, t0.elapsed()));
            if let Err(e) = r {
                super::sys::remove_tree_best_effort(&entry.dir);
                return Err(e);
            }
            st.ready = true;
        }
        drop(st);
        Ok(entry)
    }

    /// The sampled raw: SAMPLE_SEGMENTS short segments spread evenly over the selection (decoded in parallel, no blend),
    /// read back as one stream through the concat demuxer. 1.2 used one 2 s sample from the middle, which misjudges
    /// clips whose detail changes along the selection.
    fn make_sample(&self, c: &Prep, e: &RawEntry, cancel: &CancelToken, progress: &(dyn Fn(f64) + Sync)) -> Result<()> {
        let n = SAMPLE_SEGMENTS;
        let seg = e.seconds / n as f64;
        let done = std::sync::atomic::AtomicUsize::new(0);
        let res: Vec<Result<()>> = std::thread::scope(|s| {
            let hs: Vec<_> = (0..n)
                .map(|i| {
                    let done = &done;
                    s.spawn(move || {
                        let start = if c.info.is_image { 0.0 } else { c.start + (i as f64 + 0.5) * c.length / n as f64 - seg / 2.0 };
                        let mut a = c.graph_args(start.max(0.0), seg, 0.0, c.hw);
                        a.extend(crate::ff_args!["-c:v", "rawvideo", "-f", "nut", "-y", p(&e.dir.join(format!("seg{i}.nut")))]);
                        let r = ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, None);
                        progress((done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1) as f64 / n as f64);
                        r
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for r in res {
            r?;
        }
        let list: String = (0..n).map(|i| format!("file 'seg{i}.nut'\n")).collect();
        std::fs::write(&e.raw, list)?;
        Ok(())
    }

    /// Encode the given quantisers from raw in parallel (skipping known ones); returns all known sizes.
    fn trials(&self, e: &RawEntry, qs: &[u8], cancel: &CancelToken) -> Result<BTreeMap<u8, u64>> {
        let mut st = lock_cancellable(&e.state, cancel)?;
        let todo: Vec<u8> = qs.iter().copied().filter(|q| !st.sizes.contains_key(q)).collect();
        let t0 = std::time::Instant::now();
        let results: Vec<(u8, Result<u64>)> = std::thread::scope(|s| {
            let hs: Vec<_> = todo
                .iter()
                .map(|&q| {
                    s.spawn(move || {
                        let out = e.trial_path(q);
                        let mut a = e.input_args();
                        a.extend(mjpeg_args(q));
                        a.extend(crate::ff_args!["-threads", TRIAL_THREADS, "-r", DEVICE_FPS, "-movflags", "+faststart", "-y", p(&out)]);
                        let r = ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, None).and_then(|_| Ok(std::fs::metadata(&out)?.len()));
                        (q, r)
                    })
                })
                .collect();
            hs.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (q, r) in results {
            st.sizes.insert(q, r?);
        }
        debug(|| format!("trials {todo:?} in {:?} -> {:?}", t0.elapsed(), todo.iter().map(|q| st.sizes[q]).collect::<Vec<_>>()));
        Ok(st.sizes.clone())
    }

    /// Smallest q whose (estimated) full size fits, searching from raw: probes q=4 and q=16, a log-log model, then
    /// parallel rounds around it. Returns (Some(q), sizes) or (None, sizes) when even Q_MAX does not fit.
    /// `scale` turns a raw-entry size into a full-length size (1 for full raws), `limit` is the size that must not be
    /// exceeded.
    fn search_q(&self, e: &RawEntry, scale: f64, limit: f64, cancel: &CancelToken, progress: &(dyn Fn(f64) + Sync)) -> Result<(Option<u8>, BTreeMap<u8, u64>)> {
        let fits = |b: u64| (b as f64) * scale <= limit;
        progress(0.0);
        // q=69 in the same parallel round: when even that does not fit there is nothing to search (1.2 findQ)
        let mut sizes = self.trials(e, &[4, 16, Q_MAX], cancel)?;
        if !fits(sizes[&Q_MAX]) {
            return Ok((None, sizes));
        }
        let qm = model_guess(sizes[&4] as f64 * scale, sizes[&16] as f64 * scale, limit);
        let mut want: Vec<u8> = (qm.saturating_sub(1).max(Q_MIN)..=(qm + 1).min(Q_MAX)).collect();
        for round in 0..12 {
            progress((0.35 + 0.2 * round as f64).min(0.95));
            sizes = self.trials(e, &want, cancel)?;
            match next_trials(&sizes, scale, limit) {
                Ok(q) => return Ok((Some(q), sizes)),
                Err(w) => want = w,
            }
        }
        Err(MediaError::Invalid("quality search did not converge".into()))
    }

    /// Longest length (seconds, rounded down to 0.5 s) that should fit at the worst quality (convert.js maxFitLength).
    fn max_fit_length(c: &Prep, bytes_at_worst: f64) -> Option<f64> {
        let per_sec = bytes_at_worst / c.length;
        if !(per_sec > 0.0) {
            return None;
        }
        Some(0.5f64.max(((c.budget as f64 * 0.94) / per_sec * 2.0).floor() / 2.0))
    }

    /// Best that fits over the blur levels: Ok((detail, q, entry, full bytes)) or Err(worst full bytes at q69, d0).
    #[allow(clippy::type_complexity)]
    fn choose_auto(&self, c: &Prep, cancel: &CancelToken, progress: Progress) -> Result<std::result::Result<(f64, u8, Arc<RawEntry>, f64), f64>> {
        let mut worst = None;
        for (i, &d) in AUTO_LEVELS.iter().enumerate() {
            let cd = c.with_detail(d);
            let (lo, span) = (i as f64 * 0.25, if i == 0 { 1.0 } else { 0.25 });
            let msg = "Choosing the best quality that fits";
            let e = self.raw_entry(&cd, cancel, &|f| progress((lo + span * 0.6 * f).min(0.99), msg))?;
            let scale = c.length / e.seconds;
            let limit = if e.sample { c.budget as f64 * 0.97 } else { c.budget as f64 };
            let (q, sizes) = self.search_q(&e, scale, limit, cancel, &|f| progress((lo + span * (0.6 + 0.4 * f)).min(0.99), msg))?;
            if let Some(q) = q {
                return Ok(Ok((d, q, e, sizes[&q] as f64 * scale)));
            }
            if d == 0.0 {
                worst = sizes.get(&Q_MAX).map(|&b| b as f64 * scale);
            }
        }
        Ok(Err(worst.unwrap_or(0.0)))
    }

    /// Predict the result (exact sizes) without writing a version. Port of convert.js estimate().
    pub fn estimate(&self, file: &Path, info: Option<&SourceInfo>, req: &VersionRequest, cancel: &CancelToken, progress: Progress) -> Result<EstimateResult> {
        let c = self.prepare(file, info, req, cancel)?;
        let mk = |mode, q: u8, detail, bytes: f64, fits: bool, mfl, exact| EstimateResult {
            mode,
            q,
            quality: quality_from_q(q),
            detail,
            bytes: bytes.round() as u64,
            fits,
            max_fit_length: mfl,
            start: c.start,
            length: c.length,
            budget: c.budget,
            duration: c.info.duration,
            fps: DEVICE_FPS,
            exact,
        };
        if let Some(q) = c.manual_q {
            let e = self.raw_entry(&c, cancel, &|f| progress(0.8 * f, "Estimating the size"))?;
            let sizes = self.trials(&e, &[q], cancel)?;
            let bytes = sizes[&q] as f64 * c.length / e.seconds;
            let fits = bytes <= c.budget as f64;
            progress(1.0, "Done");
            return Ok(mk("manual", q, c.base.detail, bytes, fits, if fits { None } else { Self::max_fit_length(&c, bytes) }, !e.sample));
        }
        let r = self.choose_auto(&c, cancel, progress)?;
        progress(1.0, "Done");
        Ok(match r {
            Ok((d, q, e, bytes)) => mk("auto", q, d, bytes, true, None, !e.sample),
            Err(worst) => mk("auto", Q_MAX, 0.0, worst, false, Self::max_fit_length(&c, worst), true),
        })
    }

    /// Convert into `out_dir/device.mp4` (+ preview.mp4, thumb.jpg). Auto mode reuses the estimate's winning encode
    /// when the selection was estimated already. Port of convert.js convert(); error `TooLong` when nothing fits.
    pub fn convert(&self, file: &Path, info: Option<&SourceInfo>, req: &VersionRequest, out_dir: &Path, cancel: &CancelToken, progress: Progress) -> Result<ConvertResult> {
        let c = self.prepare(file, info, req, cancel)?;
        std::fs::create_dir_all(out_dir)?;
        let out = out_dir.join("device.mp4");
        let (q, detail, e, mut size) = match c.manual_q {
            Some(q) => {
                let e = self.raw_entry(&c, cancel, &|f| progress(0.7 * f, "Preparing"))?;
                let sizes = self.trials(&e, &[q], cancel)?;
                let s = sizes[&q];
                (q, c.base.detail, e, s)
            }
            None => match self.choose_auto(&c, cancel, &|f, m| progress(0.75 * f, m))? {
                Ok((d, q, e, _)) => {
                    let s = e.state.lock().unwrap().sizes[&q];
                    (q, d, e, s)
                }
                Err(worst) => return Err(MediaError::TooLong { max_fit_length: Self::max_fit_length(&c, worst) }),
            },
        };
        let cd = c.with_detail(detail);
        let tmp = out_dir.join("device.tmp.mp4");
        let mut q = q;
        if !e.sample {
            std::fs::copy(e.trial_path(q), &tmp)?;
        } else {
            // sampled path (very long selection or low disk): ONE decode of the selection feeding three mjpeg encoders
            // (q-1, q, q+1 around the sampled estimate), keep the finest that fits; 1.2 re-encoded the whole selection
            // per step. Manual quality: just q.
            let mut tried: BTreeMap<u8, u64> = BTreeMap::new();
            let mut qs: Vec<u8> = if c.manual_q.is_some() { vec![q] } else { (q.saturating_sub(1).max(Q_MIN)..=(q + 1).min(Q_MAX)).collect() };
            let chosen = loop {
                let outs: Vec<PathBuf> = qs.iter().map(|q| out_dir.join(format!("device.q{q}.tmp.mp4"))).collect();
                let (mut a, g) = cd.graph_parts(cd.base.start, cd.base.length, cd.base.blend, cd.hw);
                let labels: String = (0..qs.len()).map(|i| format!("[o{i}]")).collect();
                a.extend(crate::ff_args!["-filter_complex", format!("{g};[out]split={}{labels}", qs.len())]);
                for (i, (q, o)) in qs.iter().zip(&outs).enumerate() {
                    a.extend(crate::ff_args!["-map", format!("[o{i}]"), "-an"]);
                    a.extend(mjpeg_args(*q));
                    a.extend(crate::ff_args!["-r", DEVICE_FPS, "-movflags", "+faststart", "-y", p(o)]);
                }
                let len = cd.length;
                let r = ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, Some(&|t: f64| progress(0.75 + 0.15 * (t / len).min(1.0), "Preparing")));
                if let Err(e) = r {
                    outs.iter().for_each(|o| drop(std::fs::remove_file(o)));
                    return Err(e);
                }
                for (q, o) in qs.iter().zip(&outs) {
                    tried.insert(*q, std::fs::metadata(o)?.len());
                }
                let pick = if c.manual_q.is_some() { Some(q) } else { tried.iter().filter(|(_, &b)| b <= c.budget).map(|(&q, _)| q).min() };
                // keep the chosen one, delete the rest
                for (qq, o) in qs.iter().zip(&outs) {
                    if Some(*qq) == pick {
                        std::fs::rename(o, &tmp)?;
                    } else {
                        let _ = std::fs::remove_file(o);
                    }
                }
                match pick {
                    // the finest that fits (q-2 is not tried: the sampled estimate is within ~1%, not worth a pass)
                    Some(p) => break p,
                    None => {
                        let top = *tried.keys().max().unwrap();
                        if top >= Q_MAX {
                            let _ = std::fs::remove_file(&tmp);
                            return Err(MediaError::TooLong { max_fit_length: Self::max_fit_length(&c, tried[&top] as f64) });
                        }
                        qs = (top + 1..=(top + 3).min(Q_MAX)).collect();
                    }
                }
            };
            q = chosen;
            size = tried[&q];
        }
        std::fs::rename(&tmp, &out)?;
        progress(0.9, "Making the preview");
        self.make_preview(&cd, &e, out_dir, cancel)?;
        let data = std::fs::read(&out)?;
        if !crate::device::mjpeg::is_mjpeg_mp4(&data) {
            return Err(MediaError::Invalid("The converted file is not an MJPEG mp4.".into()));
        }
        progress(1.0, "Done");
        Ok(ConvertResult {
            size,
            q,
            quality: quality_from_q(q),
            detail,
            too_big: size > c.budget,
            fps: DEVICE_FPS,
            start: c.start,
            length: c.length,
            frame: c.base.frame,
            blend: c.blend,
            duration: c.info.duration,
            is_image: c.info.is_image,
        })
    }

    /// preview.mp4 (1200x540 H.264, notch/corners applied) + thumb.jpg. From the raw frames when we have the whole
    /// selection (no second decode), else from the source like 1.2.
    fn make_preview(&self, c: &Prep, e: &RawEntry, dir: &Path, cancel: &CancelToken) -> Result<()> {
        let pv = dir.join("preview.mp4");
        let mut a: Vec<String> = if !e.sample {
            crate::ff_args!["-color_range", "pc", "-i", p(&e.raw), "-an", "-vf", "transpose=2,scale=1200:540:flags=bicubic,format=yuv420p"]
        } else {
            let mut b = c.base.clone();
            b.flags = graph::LANCZOS.into();
            let mut a: Vec<String> = if c.hw { crate::ff_args!["-hwaccel", "cuda"] } else { vec![] };
            a.extend(input_args_ov(&p(&c.file), &b, c.corner_file.as_deref().map(p).as_deref(), c.overlay_file.as_deref().map(p).as_deref()));
            let g = format!(
                "{};{};{};[mc]scale=1200:540:flags=bicubic,format=yuv420p[out]",
                graph::base_graph(&b),
                graph::mask_graph(&c.mask),
                graph::finish_graph(c.corner_file.is_some(), c.overlay_file.is_some())
            );
            a.extend(crate::ff_args!["-filter_complex", g, "-map", "[out]", "-an"]);
            a
        };
        a.extend(crate::ff_args!["-c:v", "libx264", "-preset", "veryfast", "-crf", "22", "-movflags", "+faststart", "-y", p(&pv)]);
        ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, None)?;
        let seek = 0.5f64.min((c.length / 2.0).max(0.0));
        ffmpeg::run(
            &self.cfg.ffmpeg,
            &crate::ff_args!["-ss", seek, "-i", p(&pv), "-frames:v", "1", "-vf", "scale=400:-2", "-q:v", "4", "-y", p(&dir.join("thumb.jpg"))],
            cancel,
            None,
        )
    }

    /// Bytes per second of output at `quality` and simplify `detail`, from one direct encode of at most 2 s
    /// (sizes loop-length suggestions; convert.js sampleBytesPerSecond).
    pub fn sample_bytes_per_second(&self, file: &Path, info: Option<&SourceInfo>, req: &VersionRequest, quality: f64, detail: f64, cancel: &CancelToken) -> Result<f64> {
        let mut r = req.clone();
        r.quality = Some(quality);
        r.detail = detail;
        let c = self.prepare(file, info, &r, cancel)?;
        let s_len = 2f64.min(c.length);
        let s_start = if c.info.is_image { 0.0 } else { c.start + (c.length / 2.0 - s_len / 2.0).max(0.0) };
        let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let out = self.dir.join(format!("bps{n}.mp4"));
        let mut a = c.graph_args(s_start, s_len, 0.0, c.hw);
        a.extend(mjpeg_args(q_from_quality(quality)));
        a.extend(crate::ff_args!["-r", DEVICE_FPS, "-y", p(&out)]);
        let res = ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, None).and_then(|_| Ok(std::fs::metadata(&out)?.len()));
        let _ = std::fs::remove_file(&out);
        Ok(res? as f64 / s_len)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run the search on a size table; returns (q, rounds after the first parallel round).
    fn simulate(table: &dyn Fn(u8) -> u64, limit: f64) -> (Option<u8>, usize) {
        let mut sizes = BTreeMap::new();
        for q in [4, 16, Q_MAX] {
            sizes.insert(q, table(q));
        }
        if sizes[&Q_MAX] as f64 > limit {
            return (None, 0);
        }
        let qm = model_guess(sizes[&4] as f64, sizes[&16] as f64, limit);
        let mut want: Vec<u8> = (qm.saturating_sub(1).max(Q_MIN)..=(qm + 1).min(Q_MAX)).collect();
        for round in 1..12 {
            for q in &want {
                sizes.insert(*q, table(*q));
            }
            match next_trials(&sizes, 1.0, limit) {
                Ok(q) => return (Some(q), round),
                Err(w) => want = w,
            }
        }
        panic!("no convergence");
    }

    /// Sizes measured on the 34 s "cyber studio" clip (7.1 s loop), log-interpolated where not measured.
    fn cyber(q: u8) -> u64 {
        let pts: [(u8, f64); 12] = [
            (2, 30.0e6), (4, 22633302.0), (16, 11046989.0), (23, 9359549.0), (26, 8882640.0), (30, 8393160.0),
            (36, 7861140.0), (42, 7500019.0), (48, 7223860.0), (54, 7003698.0), (60, 6823424.0), (69, 6627530.0),
        ];
        let i = pts.iter().position(|p| p.0 >= q).unwrap();
        if pts[i].0 == q || i == 0 {
            return pts[i].1 as u64;
        }
        let ((q0, s0), (q1, s1)) = (pts[i - 1], pts[i]);
        let t = ((q as f64).ln() - (q0 as f64).ln()) / ((q1 as f64).ln() - (q0 as f64).ln());
        (s0.ln() + t * (s1.ln() - s0.ln())).exp() as u64
    }

    #[test]
    fn search_is_exact_and_short() {
        let power = |q: u8| (40.0e6 * (q as f64).powf(-0.6)) as u64;
        for table in [&cyber as &dyn Fn(u8) -> u64, &power] {
            for limit in (6_000_000..32_000_000).step_by(97_000) {
                let truth = (Q_MIN..=Q_MAX).find(|&q| table(q) as f64 <= limit as f64);
                let (q, rounds) = simulate(table, limit as f64);
                assert_eq!(q, truth, "limit {limit}");
                assert!(rounds <= 3, "limit {limit}: {rounds} rounds");
            }
        }
        // the integration case: budget 8.94 MB -> q 26 (the old search needed 17 rounds)
        assert_eq!(simulate(&cyber, 8_940_000.0).0, Some(26));
    }
}
