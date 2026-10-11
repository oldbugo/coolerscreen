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
use super::graph::{self, device_graph, input_args, mjpeg_args, q_from_quality, quality_from_q, Base, Corners, Frame, Mask};
use super::graph::{DEVICE_FPS, IMAGE_SECONDS, Q_MAX, Q_MIN};
use super::{gpu, probe, r3, CancelToken, MediaConfig, MediaError, Progress, Result, SourceInfo};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Bytes of one stored 1080x2400 4:2:0 frame.
pub const RAW_FRAME_BYTES: u64 = 1080 * 2400 * 3 / 2;
/// Above this the selection is not kept as raw frames (about 44 s of output); see the module docs.
pub const RAW_MAX_BYTES: u64 = 3 << 30;
/// Raw intermediates kept (LRU).
const CACHE_ENTRIES: usize = 3;
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
            "{}|{}|{}|{}|{}|{:?}|{}|{:?}|{:?}|{}",
            self.file.display(),
            stamp,
            self.base.start,
            self.base.length,
            self.base.blend,
            self.base.frame,
            self.base.detail,
            self.mask,
            self.corner_file,
            self.hw
        )
    }
    /// Input + graph args of the full device graph (without output args).
    fn graph_args(&self, start: f64, length: f64, blend: f64, hw: bool) -> Vec<String> {
        let mut b = self.base.clone();
        b.start = start;
        b.length = length;
        b.blend = blend;
        let mut a: Vec<String> = if hw { crate::ff_args!["-hwaccel", "cuda"] } else { vec![] };
        a.extend(input_args(&p(&self.file), &b, self.corner_file.as_deref().map(p).as_deref()));
        a.extend(crate::ff_args!["-filter_complex", device_graph(&b, &self.mask, self.corner_file.is_some()), "-map", "[out]", "-an"]);
        a
    }
}

/// A raw intermediate and the trial encodes made from it.
struct RawEntry {
    key: String,
    dir: PathBuf,
    raw: PathBuf,
    /// Output seconds the raw covers (the selection, or the 2 s sample).
    seconds: f64,
    sample: bool,
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
}

/// The media engine: configuration, GPU caps and the raw-intermediate cache. Cheap to share (`Arc<Engine>`); all
/// methods take `&self` and may run concurrently.
pub struct Engine {
    cfg: MediaConfig,
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
    /// Make the engine (clears stale files in the work dir). Call [`gpu::detect`] at startup to enable NVDEC paths.
    pub fn new(cfg: MediaConfig) -> Engine {
        let _ = std::fs::remove_dir_all(&cfg.work_dir);
        let _ = std::fs::create_dir_all(&cfg.work_dir);
        Engine { cfg, cache: Mutex::new(vec![]), seq: Default::default() }
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
            let _ = std::fs::remove_dir_all(&e.dir);
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
        let hw = gpu::use_nvdec(&info) && std::env::var_os("CS_NO_HW_CONVERT").is_none();
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
            hw,
        })
    }

    /// The raw intermediate for this selection (made now, or from the cache).
    fn raw_entry(&self, c: &Prep, cancel: &CancelToken, progress: &(dyn Fn(f64) + Sync)) -> Result<Arc<RawEntry>> {
        let sample = c.frames() * RAW_FRAME_BYTES > RAW_MAX_BYTES;
        let key = format!("{}|{}", c.key(), sample);
        let entry = {
            let mut cache = self.cache.lock().unwrap();
            if let Some(i) = cache.iter().position(|e| e.key == key) {
                let e = cache.remove(i);
                cache.push(e.clone()); // most recent last
                e
            } else {
                let n = self.seq.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let dir = self.cfg.work_dir.join(format!("sel{n}"));
                let seconds = if sample { 2f64.min(c.length) } else { c.length };
                let e = Arc::new(RawEntry { key, raw: dir.join("raw.nut"), dir, seconds, sample, state: Default::default() });
                cache.push(e.clone());
                // evict the oldest entries nobody is using
                while cache.len() > CACHE_ENTRIES {
                    match cache.iter().position(|x| Arc::strong_count(x) == 1) {
                        Some(i) => {
                            let old = cache.remove(i);
                            let _ = std::fs::remove_dir_all(&old.dir);
                        }
                        None => break,
                    }
                }
                e
            }
        };
        let mut st = lock_cancellable(&entry.state, cancel)?;
        if !st.ready {
            std::fs::create_dir_all(&entry.dir)?;
            let (start, length, blend) = if entry.sample {
                // 1.2 sampleEstimate: 2 s from the middle of the selection, no blend
                let s_len = 2f64.min(c.length);
                let s_start = if c.info.is_image { 0.0 } else { c.start + (c.length / 2.0 - s_len / 2.0).max(0.0) };
                (s_start, s_len, 0.0)
            } else {
                (c.base.start, c.base.length, c.base.blend)
            };
            let secs = entry.seconds;
            let on_time = |t: f64| progress((t / secs).min(1.0));
            let mk = |hw: bool| {
                let mut a = c.graph_args(start, length, blend, hw);
                a.extend(crate::ff_args!["-c:v", "rawvideo", "-f", "nut", "-y", p(&entry.raw)]);
                ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, Some(&on_time))
            };
            let r = match mk(c.hw) {
                Err(MediaError::Ffmpeg(_)) if c.hw => mk(false), // NVDEC refused this stream: CPU decode
                r => r,
            };
            if let Err(e) = r {
                let _ = std::fs::remove_file(&entry.raw);
                return Err(e);
            }
            st.ready = true;
        }
        drop(st);
        Ok(entry)
    }

    /// Encode the given quantisers from raw in parallel (skipping known ones); returns all known sizes.
    fn trials(&self, e: &RawEntry, qs: &[u8], cancel: &CancelToken) -> Result<BTreeMap<u8, u64>> {
        let mut st = lock_cancellable(&e.state, cancel)?;
        let todo: Vec<u8> = qs.iter().copied().filter(|q| !st.sizes.contains_key(q)).collect();
        let results: Vec<(u8, Result<u64>)> = std::thread::scope(|s| {
            let hs: Vec<_> = todo
                .iter()
                .map(|&q| {
                    s.spawn(move || {
                        let out = e.trial_path(q);
                        let mut a = crate::ff_args!["-color_range", "pc", "-i", p(&e.raw)];
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
        let (s4, s16) = (sizes[&4] as f64 * scale, sizes[&16] as f64 * scale);
        let k = (s16.ln() - s4.ln()) / (16f64.ln() - 4f64.ln());
        let qm = if k.is_finite() && k < 0.0 { (4f64.ln() + (limit.ln() - s4.ln()) / k).exp().round() } else { 16.0 };
        let qm = (qm as i64).clamp(Q_MIN as i64, Q_MAX as i64) as u8;
        let mut want: Vec<u8> = (qm.saturating_sub(1).max(Q_MIN)..=(qm + 1).min(Q_MAX)).collect();
        for round in 0..24 {
            progress((0.35 + 0.2 * round as f64).min(0.95));
            sizes = self.trials(e, &want, cancel)?;
            let best = sizes.iter().filter(|(_, &b)| fits(b)).map(|(&q, _)| q).min();
            match best {
                Some(q) if q == Q_MIN || sizes.contains_key(&(q - 1)) => return Ok((Some(q), sizes)),
                Some(q) => want = (q.saturating_sub(3).max(Q_MIN)..q).collect(),
                None => {
                    let top = *sizes.keys().max().unwrap();
                    if top >= Q_MAX {
                        return Ok((None, sizes));
                    }
                    want = (top + 1..=(top + 3).min(Q_MAX)).collect();
                }
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
            // very long selection: direct encode like 1.2, then step q up until it fits / down while it still fits
            let enc = |q: u8| -> Result<u64> {
                let mut a = cd.graph_args(cd.base.start, cd.base.length, cd.base.blend, cd.hw);
                a.extend(mjpeg_args(q));
                a.extend(crate::ff_args!["-r", DEVICE_FPS, "-movflags", "+faststart", "-y", p(&tmp)]);
                let len = cd.length;
                ffmpeg::run(&self.cfg.ffmpeg, &a, cancel, Some(&|t: f64| progress(0.75 + 0.15 * (t / len).min(1.0), "Preparing")))?;
                Ok(std::fs::metadata(&tmp)?.len())
            };
            size = enc(q)?;
            while c.manual_q.is_none() && size > c.budget && q < Q_MAX {
                q += 1;
                size = enc(q)?;
            }
            if c.manual_q.is_none() && size > c.budget {
                let _ = std::fs::remove_file(&tmp);
                return Err(MediaError::TooLong { max_fit_length: Self::max_fit_length(&c, size as f64) });
            }
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
            a.extend(input_args(&p(&c.file), &b, c.corner_file.as_deref().map(p).as_deref()));
            let g = format!(
                "{};{};{};[mc]scale=1200:540:flags=bicubic,format=yuv420p[out]",
                graph::base_graph(&b),
                graph::mask_graph(&c.mask),
                graph::corners_graph(c.corner_file.is_some())
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
        let out = self.cfg.work_dir.join(format!("bps{n}.mp4"));
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
        self.clear_cache();
    }
}
