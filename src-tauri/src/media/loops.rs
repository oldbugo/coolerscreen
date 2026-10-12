//! "Find loop points" over the whole source (port of src/loopsearch.js) with the 2.0 performance fixes:
//! - ONE decode pass gives both the 64x36 search frames and the 192x108 frames for the leader re-check (1.2 re-decoded
//!   16 three-frame windows with separate ffmpeg seeks: 3-10 s). On the CPU path the frames are byte-identical to 1.2's
//!   (same decoder, same `scale=WxH:flags=area,format=gray` per output), so the ranking is identical.
//! - NVDEC + scale_cuda only for sources above 2.5K (pre-scaled on the GPU to 768x432, so values differ slightly and
//!   the ranking can differ a little from 1.2's); 1080p HEVC gains nothing on the GPU and stays on the exact CPU path.
//! - The comparison runs in Rust over rayon (1.2: 20-910 ms of JS).
//! - Loop cards: 320x144 previews rendered straight at that size, all in parallel.
//!
//! Seam cost of a loop starting at source frame a with n output frames: j = round(a + n * srcFps / 18) is the frame that
//! would follow the end; cost = (D(j-1, a-1) + 2 D(j, a) + D(j+1, a+1)) / 4 with D the mean absolute difference of small
//! grey frames; the ratio to the usual neighbouring-frame difference says how visible the jump is (1.0 = a normal step).
use super::ffmpeg::{self, p};
use super::graph::{base_graph, Base, Frame, DEVICE_FPS};
use super::{gpu, js_round, r3, CancelToken, MediaError, Progress, Result, SourceInfo};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub const FW: usize = 64;
pub const FH: usize = 36;
const CW: usize = 32;
const CH: usize = 18;
pub const RW: usize = 192;
pub const RH: usize = 108;
/// ~6 min at 25 fps; longer sources are searched in their first MAX_FRAMES frames.
pub const MAX_FRAMES: usize = 9000;
/// The 192x108 frames are cached with the 64x36 ones up to this size; above it a cache hit re-checks with seeks.
const HI_CACHE_MAX: usize = 64 << 20;
const OUT_FPS: f64 = DEVICE_FPS as f64;

/// Every source frame as small grey pictures.
pub struct LoopFrames {
    pub n: usize,
    /// n * 64x36
    pub lo: Vec<u8>,
    /// n * 192x108 (empty when loaded from a cache that did not keep them)
    pub hi: Vec<u8>,
    /// Decoded with NVDEC + scale_cuda.
    pub hw: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoopCandidate {
    pub start: f64,
    pub start_frame: usize,
    pub length: f64,
    pub frames: usize,
    pub end: f64,
    pub end_frame: i64,
    pub cost: f64,
    pub ratio: f64,
    pub similarity: f64,
    /// Seconds of crossfade the card and a version use (library.js autoBlend), set by [`auto_blend`] users.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<f64>,
    /// With a cross-fade: the share of the picture that shows a brief double image during it (0..1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ghost: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Searched {
    pub min_len: f64,
    pub max_len: f64,
    pub truncated: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LoopSearch {
    pub frames: usize,
    pub searched: Searched,
    pub candidates: Vec<LoopCandidate>,
}

#[derive(Serialize, Deserialize)]
struct CacheMeta {
    key: String,
    n: usize,
    hi: bool,
    hw: bool,
}

fn source_key(file: &Path) -> String {
    std::fs::metadata(file)
        .map(|m| {
            let ms = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
            format!("{}-{}", m.len(), ms)
        })
        .unwrap_or_default()
}

/// Decode every frame once to 64x36 and 192x108 grey (cached in `cache_dir` per file size + mtime).
pub fn read_frames(ff: &Path, file: &Path, info: &SourceInfo, cache_dir: Option<&Path>, cancel: &CancelToken, progress: Progress) -> Result<LoopFrames> {
    let key = source_key(file);
    let bin = cache_dir.map(|d| d.join("loopframes-v2.bin"));
    let meta = cache_dir.map(|d| d.join("loopframes-v2.json"));
    if let (Some(bin), Some(meta)) = (&bin, &meta) {
        if let Ok(m) = std::fs::read(meta).map_err(|_| ()).and_then(|b| serde_json::from_slice::<CacheMeta>(&b).map_err(|_| ())) {
            if m.key == key {
                if let Ok(mut data) = std::fs::read(bin) {
                    let lo_len = m.n * FW * FH;
                    let hi_len = if m.hi { m.n * RW * RH } else { 0 };
                    if data.len() == lo_len + hi_len {
                        let hi = data.split_off(lo_len);
                        progress(1.0, "Reading the frames");
                        return Ok(LoopFrames { n: m.n, lo: data, hi, hw: m.hw });
                    }
                }
            }
        }
    }
    let expected = (info.duration.unwrap_or(0.0) * info.fps.unwrap_or(25.0)).round().max(1.0) as usize;
    let mut fr = None;
    if gpu::use_nvdec_small(info) {
        match decode_dual(ff, file, true, expected, cancel, progress) {
            Ok(f) => fr = Some(f),
            Err(MediaError::Cancelled) => return Err(MediaError::Cancelled),
            Err(_) => {}
        }
    }
    let fr = match fr {
        Some(f) => f,
        None => decode_dual(ff, file, false, expected, cancel, progress)?,
    };
    if let (Some(dir), Some(bin), Some(meta)) = (cache_dir, &bin, &meta) {
        let keep_hi = fr.hi.len() <= HI_CACHE_MAX;
        let _ = std::fs::create_dir_all(dir);
        let mut data = fr.lo.clone();
        if keep_hi {
            data.extend_from_slice(&fr.hi);
        }
        let m = CacheMeta { key, n: fr.n, hi: keep_hi, hw: fr.hw };
        if std::fs::write(bin, &data).is_ok() {
            let _ = std::fs::write(meta, serde_json::to_vec(&m).unwrap());
        }
    }
    Ok(fr)
}

fn decode_dual(ff: &Path, file: &Path, hw: bool, expected: usize, cancel: &CancelToken, progress: Progress) -> Result<LoopFrames> {
    let n_dir = super::encode::default_run_dir(); // same folder as the engine's scratch (cleaned on start)
    let _ = std::fs::create_dir_all(&n_dir);
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let hi_file = n_dir.join(format!("hi-{}-{}.raw", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed)));
    let (pre, head): (Vec<String>, &str) = if hw {
        (
            crate::ff_args!["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"],
            "[0:v]scale_cuda=768:432:interp_algo=lanczos:format=nv12,hwdownload,format=nv12,format=gray,split[a][b];",
        )
    } else {
        (vec![], "[0:v]split[a][b];")
    };
    let mut a = pre;
    a.extend(crate::ff_args![
        "-i", p(file), "-frames:v", MAX_FRAMES, "-an", "-fps_mode", "passthrough", "-filter_complex",
        format!("{head}[a]scale={FW}:{FH}:flags=area,format=gray[o1];[b]scale={RW}:{RH}:flags=area,format=gray[o2]"),
        "-map", "[o1]", "-f", "rawvideo", "pipe:1", "-map", "[o2]", "-f", "rawvideo", "-y", p(&hi_file)
    ]);
    let on_bytes = |b: usize| progress(((b / (FW * FH)) as f64 / expected as f64).min(1.0), "Reading the frames");
    let lo = ffmpeg::run_capture(ff, &a, cancel, Some(&on_bytes));
    let hi = std::fs::read(&hi_file);
    let _ = std::fs::remove_file(&hi_file);
    let (mut lo, mut hi) = (lo?, hi?);
    let n = (lo.len() / (FW * FH)).min(hi.len() / (RW * RH));
    lo.truncate(n * FW * FH);
    hi.truncate(n * RW * RH);
    Ok(LoopFrames { n, lo, hi, hw })
}

#[inline]
fn sad(a: &[u8], b: &[u8]) -> u32 {
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y) as u32).sum()
}

fn downscale(lo: &[u8], n: usize) -> Vec<u8> {
    let mut out = vec![0u8; n * CW * CH];
    out.par_chunks_mut(CW * CH).enumerate().for_each(|(f, o)| {
        let s = f * FW * FH;
        for y in 0..CH {
            for x in 0..CW {
                let i = s + y * 2 * FW + x * 2;
                o[y * CW + x] = ((lo[i] as u32 + lo[i + 1] as u32 + lo[i + FW] as u32 + lo[i + FW + 1] as u32 + 2) >> 2) as u8;
            }
        }
    });
    out
}

#[derive(Clone, Copy)]
struct Refined {
    a: usize,
    nn: usize,
    j: usize,
    c: f64,
    hc: f64,
    /// How the join looks on the screen (1.0 = a normal frame step), with `bn` frames of cross-fade.
    ratio: f64,
    bn: usize,
    /// The best cross-fade for this pair (frames, ratio), used when the hard cut fails the 192x108 re-check.
    alt: Option<(usize, f64)>,
    /// Share of the picture that differs clearly between the frames the cross-fade mixes (0..1): sparse particles
    /// fade invisibly, a whole scene that moved shows as a brief double image.
    ghost: f64,
}
impl Refined {
    /// Ranking: how the join looks, with a cross-fade's double image counted against it.
    fn rank(&self) -> f64 {
        self.ratio + if self.bn > 0 { GHOST_WEIGHT * self.ghost } else { 0.0 }
    }
}
const GHOST_WEIGHT: f64 = 2.0;
/// Grey levels (of 255) above which a pixel counts as clearly different in [`ghost_area`].
const GHOST_LEVEL: u8 = 12;

/// [`Refined::ghost`] for a cross-fade of `bn` output frames: the pairs a quarter, half and three quarters into it.
fn ghost_area(lo: &[u8], a: usize, nn: usize, bn: usize, rf: f64, n: usize) -> f64 {
    let q = FW * FH;
    let mut s = 0.0;
    let ms = [bn / 4, bn / 2, (3 * bn) / 4];
    for m in ms {
        let x = js_round(a as f64 - (bn - m) as f64 * rf).max(0.0) as usize;
        let y = (js_round(a as f64 + (nn - bn + m) as f64 * rf) as usize).min(n - 1);
        let (fx, fy) = (&lo[x * q..(x + 1) * q], &lo[y * q..(y + 1) * q]);
        s += fx.iter().zip(fy).filter(|(p, r)| p.abs_diff(**r) > GHOST_LEVEL).count() as f64 / q as f64;
    }
    s / ms.len() as f64
}

/// Cross-fade lengths tried, in output frames (1/3 s to 1.5 s, the most a version blends).
const BLENDS: [usize; 5] = [6, 9, 12, 18, 27];
/// A hard cut this close to a normal frame step needs no blend.
const SEAMLESS: f64 = 1.05;

/// The best cross-fade for a loop of `nn` output frames starting at source frame `a`. `mis(bn, bs)` = the mean
/// difference between the frames faded together (bn output frames = bs source frames), in local frame steps. A fade
/// spreads that difference over bn frames, so the join looks like 1 + mis / bn normal steps (plus a small preference
/// for shorter fades, which ghost less when something big moves).
fn best_blend(nn: usize, a: usize, rf: f64, mis: impl Fn(usize, usize) -> f64) -> Option<(usize, f64)> {
    let mut best: Option<(usize, f64, f64)> = None;
    for bn in BLENDS {
        let bs = js_round(bn as f64 * rf).max(1.0) as usize;
        if bn * 2 > nn || bs > a {
            break;
        }
        let eff = 1.0 + mis(bn, bs) / (bn as f64 * rf.max(1.0));
        let score = eff + 0.02 * bn as f64 / OUT_FPS;
        if best.map_or(true, |b| score < b.2) {
            best = Some((bn, eff, score));
        }
    }
    best.map(|b| (b.0, b.1))
}

/// Hard cut or cross-fade, whichever looks smoother: (blend frames, ratio).
fn pick_blend(raw: f64, alt: Option<(usize, f64)>) -> (usize, f64) {
    match alt {
        Some((bn, eff)) if raw > SEAMLESS && eff < raw => (bn, eff),
        _ => (0, raw),
    }
}

/// Search options (seconds; `fps` = source frame rate).
#[derive(Clone, Copy, Debug)]
pub struct LoopOptions {
    pub fps: f64,
    pub min_len: f64,
    pub max_len: f64,
}

/// The search proper over decoded frames. `seek_hi` supplies 192x108 frames when `frames.hi` is empty.
pub fn find_loops(
    frames: &LoopFrames,
    opts: LoopOptions,
    seek_hi: &(dyn Fn(usize) -> Result<Vec<Vec<u8>>> + Sync),
    cancel: &CancelToken,
    progress: Progress,
) -> Result<LoopSearch> {
    let n = frames.n;
    let sf = if opts.fps > 0.0 { opts.fps } else { 25.0 };
    if n < 4 {
        return Err(MediaError::Invalid("The clip is too short to search for loop points.".into()));
    }
    let lo = &frames.lo;
    let small = downscale(lo, n);
    let (pp, qq) = (CW * CH, FW * FH);
    let dc = |x: usize, y: usize| sad(&small[x * pp..(x + 1) * pp], &small[y * pp..(y + 1) * pp]) as f64 / pp as f64;
    let df = |x: usize, y: usize| sad(&lo[x * qq..(x + 1) * qq], &lo[y * qq..(y + 1) * qq]) as f64 / qq as f64;
    let steps = |d: &(dyn Fn(usize, usize) -> f64 + Sync)| {
        let mut s: Vec<f64> = (0..n).into_par_iter().map(|i| if i == 0 { 0.0 } else { d(i - 1, i) }).collect();
        s[0] = s[1];
        s
    };
    let step = steps(&dc);

    let min_n = 2i64.max(js_round(opts.min_len * OUT_FPS) as i64) as usize;
    let max_n = (min_n as i64).max(js_round(opts.max_len * OUT_FPS) as i64) as usize;
    let ratio_f = sf / OUT_FPS;
    // Local motion: the neighbouring-frame step averaged over +-4 frames (a jump is compared with the motion around
    // it), at each search resolution; never below a quarter of the clip's median step (still clips).
    let local_of = |step: &[f64]| -> Vec<f64> {
        let mut ps = vec![0f64; n + 1];
        for i in 0..n {
            ps[i + 1] = ps[i] + step[i];
        }
        let mut sorted = step.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let floor = (sorted[n / 2] * 0.25).max(0.02);
        (0..n).map(|i| ((ps[(i + 5).min(n)] - ps[i.saturating_sub(4)]) / ((i + 5).min(n) - i.saturating_sub(4)) as f64).max(floor)).collect()
    };
    let local_c = local_of(&step);
    let local_f = local_of(&steps(&df));
    let cost = |d: &dyn Fn(usize, usize) -> f64, a: usize, nn: usize| -> Option<(usize, f64)> {
        let j = js_round(a as f64 + nn as f64 * ratio_f) as usize;
        if j >= n {
            return None;
        }
        let mut s = 2.0 * d(j, a);
        let mut w = 2.0;
        if a >= 1 {
            s += d(j - 1, a - 1);
            w += 1.0;
        }
        if j + 1 < n {
            s += d(j + 1, a + 1);
            w += 1.0;
        }
        Some((j, s / w))
    };
    // Every start with every length (no sampling: one lag at a time, D(i, i+k) for all i, then prefix sums give any
    // cross-fade window in O(1)). Each pair is scored as it would look on the screen: the hard cut, or the best blend.
    progress(0.5, "Comparing every start with every end");
    cancel.check()?;
    let mut cand: Vec<(f64, usize, usize)> = (min_n..=max_n)
        .into_par_iter()
        .map(|nn| {
            let k = js_round(nn as f64 * ratio_f) as usize;
            let mut v = vec![];
            if k + 1 >= n || cancel.is_cancelled() {
                return v;
            }
            let dl: Vec<f64> = (0..n - k).map(|i| dc(i, i + k)).collect();
            let mut ps = vec![0f64; dl.len() + 1];
            for i in 0..dl.len() {
                ps[i + 1] = ps[i] + dl[i];
            }
            for a in 0..dl.len() {
                let j = js_round(a as f64 + nn as f64 * ratio_f) as usize;
                if j >= n {
                    break;
                }
                let (mut s, mut w) = (2.0 * dl[a], 2.0);
                if a >= 1 {
                    s += dl[a - 1];
                    w += 1.0;
                }
                if a + 1 < dl.len() {
                    s += dl[a + 1];
                    w += 1.0;
                }
                let loc = (local_c[a] + local_c[j]) / 2.0;
                let alt = best_blend(nn, a, ratio_f, |_, bs| (ps[a] - ps[a - bs]) / bs as f64 / loc);
                v.push((pick_blend(s / w / loc, alt).1, a, nn));
            }
            v
        })
        .flatten_iter()
        .collect();
    cancel.check()?;
    if cand.is_empty() {
        return Err(MediaError::Invalid("No loop of that length fits in the clip. Lower the minimum length.".into()));
    }
    cand.par_sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap()); // stable, like Array.prototype.sort
    progress(0.78, "Refining the best matches");
    // The leaders again at 64x36 with the exact frame pairs of the cross-fade.
    let mut refined: Vec<Refined> = cand
        .par_iter()
        .take(400)
        .filter_map(|&(_, a, nn)| {
            let (j, c) = cost(&df, a, nn)?;
            let loc = (local_f[a] + local_f[j]) / 2.0;
            // the exact pairs the version's xfade mixes: output frame nn-bn+m with the frame (bn-m)/18 s before the start
            let alt = best_blend(nn, a, ratio_f, |bn, _| {
                let mut s = 0.0;
                for m in 0..bn {
                    let x = js_round(a as f64 - (bn - m) as f64 * ratio_f).max(0.0) as usize;
                    let y = js_round(a as f64 + (nn - bn + m) as f64 * ratio_f) as usize;
                    s += df(x, y.min(n - 1));
                }
                s / bn as f64 / loc
            });
            let (bn, ratio) = pick_blend(c / loc, alt);
            let ghost = alt.map_or(0.0, |(abn, _)| ghost_area(lo, a, nn, abn, ratio_f, n));
            Some(Refined { a, nn, j, c, hc: 0.0, ratio, bn, alt, ghost })
        })
        .collect();
    refined.sort_by(|x, y| x.rank().partial_cmp(&y.rank()).unwrap().then(x.c.partial_cmp(&y.c).unwrap()));
    let tol = 0.5 * sf;
    let mut picked: Vec<Refined> = vec![];
    for r in refined {
        if picked.iter().any(|p| (p.a as f64 - r.a as f64).abs() <= tol && (p.j as f64 - r.j as f64).abs() <= tol) {
            continue;
        }
        picked.push(r);
        if picked.len() >= 8 {
            break;
        }
    }
    progress(0.85, "Checking the leaders at higher resolution");
    // three frames around each point (f-1, f, f+1; from f when f is the first frame), at 192x108
    let rq = RW * RH;
    let window = |f: usize| -> Result<(usize, Vec<Vec<u8>>)> {
        let s0 = f.saturating_sub(1);
        if frames.hi.is_empty() {
            return Ok((s0, seek_hi(s0)?));
        }
        Ok((s0, (s0..n.min(s0 + 3)).map(|k| frames.hi[k * rq..(k + 1) * rq].to_vec()).collect()))
    };
    let leaders: Vec<Result<(f64, f64)>> = picked
        .par_iter()
        .map(|r| {
            let aw = window(r.a)?;
            let jw = window(r.j)?;
            let get = |w: &(usize, Vec<Vec<u8>>), f: i64| -> Option<Vec<u8>> {
                let i = f - w.0 as i64;
                if i < 0 {
                    None
                } else {
                    w.1.get(i as usize).cloned()
                }
            };
            let mad = |x: &[u8], y: &[u8]| sad(x, y) as f64 / x.len() as f64;
            let (mut s, mut w) = (0.0, 0.0);
            for (wt, k) in [(2.0, 0i64), (1.0, -1), (1.0, 1)] {
                if let (Some(x), Some(y)) = (get(&jw, r.j as i64 + k), get(&aw, r.a as i64 + k)) {
                    s += wt * mad(&x, &y);
                    w += wt;
                }
            }
            let hc = if w > 0.0 { s / w } else { r.c };
            let mut nb = vec![];
            for (wn, f) in [(&aw, r.a as i64), (&jw, r.j as i64)] {
                for k in [0i64, 1] {
                    if let (Some(x), Some(y)) = (get(wn, f - 1 + k), get(wn, f + k)) {
                        nb.push(mad(&x, &y));
                    }
                }
            }
            let loc = if nb.is_empty() {
                (step[r.a] + step[r.j]) / 2.0
            } else {
                nb.iter().fold(0.0, |p, q| p + q) / nb.len() as f64
            };
            Ok((hc, hc / loc.max(0.5)))
        })
        .collect();
    for (r, l) in picked.iter_mut().zip(leaders) {
        let (hc, raw) = l?;
        r.hc = hc;
        // the hard cut as seen at 192x108 decides; a cross-fade keeps its 64x36 estimate
        (r.bn, r.ratio) = pick_blend(raw, r.alt);
    }
    picked.sort_by(|x, y| {
        let d = x.rank() - y.rank();
        let d = if d != 0.0 { d } else { x.hc - y.hc };
        d.partial_cmp(&0.0).unwrap_or(std::cmp::Ordering::Equal)
    });
    progress(0.99, "Checking the leaders at higher resolution");
    Ok(LoopSearch {
        frames: n,
        searched: Searched { min_len: r3(min_n as f64 / OUT_FPS), max_len: r3(max_n as f64 / OUT_FPS), truncated: n >= MAX_FRAMES },
        candidates: picked
            .iter()
            .take(5)
            .map(|r| LoopCandidate {
                start: r3(r.a as f64 / sf),
                start_frame: r.a,
                length: r3(r.nn as f64 / OUT_FPS),
                frames: r.nn,
                end: r3(r.j as f64 / sf),
                end_frame: r.j as i64 - 1,
                cost: js_round(r.hc * 100.0) / 100.0,
                ratio: js_round(r.ratio * 100.0) / 100.0,
                similarity: js_round((100.0 - (r.hc / 255.0) * 100.0) * 10.0) / 10.0,
                blend: Some(r3(r.bn as f64 / OUT_FPS)),
                ghost: if r.bn > 0 { Some(js_round(r.ghost * 1000.0) / 1000.0) } else { None },
            })
            .collect(),
    })
}

/// 1.2's leader re-check read: three 192x108 grey frames from frame s0 (CPU decode, exact seek).
pub fn seek_hi_frames(ff: &Path, file: &Path, sf: f64, s0: usize, cancel: &CancelToken) -> Result<Vec<Vec<u8>>> {
    // 1.2 seeked to s0/sf + 1 ms, which drops frame s0 at fine timescales (read s0+1 on a 30 fps / 15360 file); seek
    // two frames early and cut by timestamp with a 0.4-frame margin instead (as the version graph does).
    let s_idx = s0.saturating_sub(2);
    let a = crate::ff_args![
        "-ss", format!("{:.6}", s_idx as f64 / sf), "-i", p(file), "-an", "-fps_mode", "passthrough", "-vf",
        format!("trim=start={:.6},scale={RW}:{RH}:flags=area,format=gray", ((s0 - s_idx) as f64 - 0.4).max(0.0) / sf),
        "-frames:v", 3, "-f", "rawvideo", "pipe:1"
    ];
    let b = ffmpeg::run_capture(ff, &a, cancel, None)?;
    Ok(b.chunks_exact(RW * RH).map(|c| c.to_vec()).collect())
}

/// Read frames (cached) and search. `cache_dir` = the clip's library folder.
pub fn search(ff: &Path, file: &Path, info: &SourceInfo, cache_dir: Option<&Path>, min_len: f64, max_len: f64, cancel: &CancelToken, progress: Progress) -> Result<LoopSearch> {
    if info.is_image {
        return Err(MediaError::Invalid("A still image always loops seamlessly.".into()));
    }
    progress(0.01, "Reading the frames");
    let frames = read_frames(ff, file, info, cache_dir, cancel, &|f, m| progress(0.01 + 0.45 * f, m))?;
    let sf = info.fps.unwrap_or(25.0);
    let seek = |s0: usize| seek_hi_frames(ff, file, sf, s0, cancel);
    find_loops(&frames, LoopOptions { fps: sf, min_len, max_len }, &seek, cancel, progress)
}

/// Blend length for a loop candidate: none when the join already looks like a normal frame step (library.js autoBlend).
pub fn auto_blend(c: &LoopCandidate) -> f64 {
    if c.ratio <= 1.0 {
        return 0.0;
    }
    let want: f64 = if c.ratio <= 1.6 { 0.33 } else { 0.5 };
    let b = want.min(c.start).min(c.length / 2.0);
    js_round((b * OUT_FPS + 1e-6).floor() / OUT_FPS * 1000.0) / 1000.0
}

/// Small grey frames (source rate) from `start`: 1.2 grayFrames.
fn gray_frames(ff: &Path, file: &Path, start: f64, dur: Option<f64>, max_frames: Option<usize>, w: usize, h: usize, cancel: &CancelToken) -> Result<Vec<Vec<u8>>> {
    let mut a = crate::ff_args!["-ss", start.max(0.0), "-i", p(file)];
    if let Some(d) = dur {
        a.extend(crate::ff_args!["-t", d]);
    }
    if let Some(m) = max_frames {
        a.extend(crate::ff_args!["-frames:v", m]);
    }
    a.extend(crate::ff_args!["-an", "-fps_mode", "passthrough", "-vf", format!("scale={w}:{h}:flags=area,format=gray"), "-f", "rawvideo", "pipe:1"]);
    let b = ffmpeg::run_capture(ff, &a, cancel, None)?;
    Ok(b.chunks_exact(w * h).map(|c| c.to_vec()).collect())
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct LoopPointPick {
    pub end: f64,
    pub length: f64,
    pub similarity: f64,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct LoopPoint {
    pub best: LoopPointPick,
    pub current: LoopPointPick,
    pub alternatives: Vec<LoopPointPick>,
    pub searched: [f64; 2],
}

/// "Find best loop point" near the selection's end (convert.js findLoopPoint): the source frame within +-2 s of `end`
/// most like the start frame (96x54 grey mean absolute difference), plus the current end and a few alternatives.
pub fn loop_point(ff: &Path, file: &Path, start: f64, end: f64, duration: Option<f64>, fps: Option<f64>, cancel: &CancelToken) -> Result<LoopPoint> {
    let sf = fps.filter(|f| *f > 0.0).unwrap_or(25.0);
    let lo = (start + 0.5).max(end - 2.0);
    let hi = duration.filter(|d| *d > 0.0).unwrap_or(end + 2.0).min(end + 2.0);
    if !(hi > lo) {
        return Err(MediaError::Invalid("The clip is too short to search for a loop point.".into()));
    }
    let (first, frames) = std::thread::scope(|s| {
        let f = s.spawn(|| gray_frames(ff, file, start, None, Some(1), 96, 54, cancel));
        let fr = gray_frames(ff, file, lo, Some(hi - lo), None, 96, 54, cancel);
        (f.join().unwrap(), fr)
    });
    let first = first?.into_iter().next().ok_or_else(|| MediaError::Invalid("Could not read the start frame.".into()))?;
    let frames = frames?;
    if frames.is_empty() {
        return Err(MediaError::Invalid("The clip is too short to search for a loop point.".into()));
    }
    let t0 = (lo * sf - 1e-6).ceil() / sf;
    let mad = |a: &[u8], b: &[u8]| sad(a, b) as f64 / a.len() as f64;
    let cands: Vec<(f64, f64)> = frames.iter().enumerate().map(|(i, f)| (js_round((t0 + i as f64 / sf) * 1000.0) / 1000.0, mad(&first, f))).collect();
    let mut cur = cands[0];
    for c in &cands {
        if (c.0 - end).abs() < (cur.0 - end).abs() {
            cur = *c;
        }
    }
    let mut sorted = cands.clone();
    sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
    let sim = |d: f64| js_round((100.0 - (d / 255.0) * 100.0) * 10.0) / 10.0;
    let pick = |c: (f64, f64)| LoopPointPick {
        end: c.0,
        length: r3(js_round((c.0 - start) * OUT_FPS) / OUT_FPS),
        similarity: sim(c.1),
    };
    let mut alts: Vec<(f64, f64)> = vec![];
    for c in &sorted {
        if alts.iter().all(|a| (a.0 - c.0).abs() > 0.3) {
            alts.push(*c);
        }
        if alts.len() >= 4 {
            break;
        }
    }
    Ok(LoopPoint {
        best: pick(sorted[0]),
        current: pick(cur),
        alternatives: alts.iter().skip(1).map(|c| pick(*c)).collect(),
        searched: [lo, hi],
    })
}

/// Card preview size.
pub const CARD_W: u32 = 320;
pub const CARD_H: u32 = 144;

/// 320x144 H.264 previews of loop candidates (made like a version: 18 fps, blend), `out_dir/loop-{i}.mp4`, rendered
/// in parallel straight at card size (1.2 rendered 2400x1080 then scaled, one at a time). Uses `c.blend` (0 if None).
pub fn render_cards(ff: &Path, file: &Path, info: &SourceInfo, cands: &[LoopCandidate], out_dir: &Path, cancel: &CancelToken, progress: Progress) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(out_dir)?;
    let hw = gpu::use_nvdec_small(info);
    let done = AtomicU32::new(0);
    let one = |i: usize, c: &LoopCandidate, hw: bool| -> Result<PathBuf> {
        let out = out_dir.join(format!("loop-{i}.mp4"));
        let base = Base {
            start: c.start,
            length: c.length,
            blend: c.blend.unwrap_or(0.0),
            fps: DEVICE_FPS,
            src_fps: info.fps,
            is_image: false,
            frame: Frame::default(),
            detail: 0.0,
            canvas: (CARD_W, CARD_H),
            flags: "area".into(),
            head: if hw {
                format!(
                    "scale_cuda={}:{}:force_original_aspect_ratio=increase:force_divisible_by=2:interp_algo=lanczos:format=nv12,hwdownload,format=nv12,",
                    CARD_W * 2,
                    CARD_H * 2
                )
            } else {
                String::new()
            },
        };
        let mut a: Vec<String> = if hw { crate::ff_args!["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"] } else { vec![] };
        a.extend(super::graph::input_args(&p(file), &base, None));
        a.extend(crate::ff_args![
            "-filter_complex", format!("{};[base]format=yuv420p[out]", base_graph(&base)), "-map", "[out]", "-an",
            "-c:v", "libx264", "-preset", "veryfast", "-crf", "26", "-threads", "2", "-movflags", "+faststart", "-y", p(&out)
        ]);
        ffmpeg::run(ff, &a, cancel, None)?;
        Ok(out)
    };
    let res: Vec<Result<PathBuf>> = cands
        .par_iter()
        .enumerate()
        .map(|(i, c)| {
            let r = match one(i, c, hw) {
                Err(MediaError::Ffmpeg(_)) if hw => one(i, c, false),
                r => r,
            };
            let k = done.fetch_add(1, Ordering::Relaxed) + 1;
            progress(k as f64 / cands.len() as f64, "Making the loop previews");
            r
        })
        .collect();
    res.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    /// Loop search over real files: `CS_LOOP_FILES` (paths separated by `|`), `CS_LOOP_MAX` (seconds, default 12).
    /// cargo test --release --lib media::loops::tests::real_files -- --ignored --nocapture
    #[test]
    #[ignore]
    fn real_files() {
        let ff = PathBuf::from(std::env::var("COOLERSCREEN_FFMPEG").unwrap_or_else(|_| "C:\\Users\\pc\\AppData\\Local\\CoolerScreen\\ffmpeg.exe".into()));
        let max: f64 = std::env::var("CS_LOOP_MAX").ok().and_then(|s| s.parse().ok()).unwrap_or(12.0);
        for f in std::env::var("CS_LOOP_FILES").unwrap().split('|') {
            let file = PathBuf::from(f);
            let info = super::super::probe::probe(&ff, &file).unwrap();
            let t = std::time::Instant::now();
            let r = search(&ff, &file, &info, None, 3.0, max, &CancelToken::new(), &|_, _| {}).unwrap();
            println!("{} ({} frames, {:.1}s search)", file.file_name().unwrap().to_string_lossy(), r.frames, t.elapsed().as_secs_f64());
            for c in &r.candidates {
                println!("  start {:6.2}s length {:5.2}s blend {:.2}s ratio {:.2} ghost {:.3}", c.start, c.length, c.blend.unwrap_or(0.0), c.ratio, c.ghost.unwrap_or(0.0));
            }
        }
    }
    #[test]
    fn auto_blend_matches_library_js() {
        let c = |ratio, start, length| LoopCandidate {
            start,
            start_frame: 0,
            length,
            frames: 0,
            end: 0.0,
            end_frame: 0,
            cost: 0.0,
            ratio,
            similarity: 0.0,
            blend: None,
            ghost: None,
        };
        assert_eq!(auto_blend(&c(0.9, 3.0, 6.0)), 0.0);
        assert_eq!(auto_blend(&c(1.3, 3.0, 6.0)), 0.278);
        assert_eq!(auto_blend(&c(2.0, 3.0, 6.0)), 0.5);
        assert_eq!(auto_blend(&c(2.0, 0.2, 6.0)), 0.167);
    }
    /// A still scene with random particles (like Wallpaper Engine fireflies) never repeats: the search must answer
    /// with a cross-fade that hides the join, and see that only a small part of the picture is faded.
    #[test]
    fn particles_get_a_clean_cross_fade() {
        let n = 540;
        let (mut lo, mut hi) = (vec![0u8; n * FW * FH], vec![0u8; n * RW * RH]);
        let mut seed = 12345u64;
        let mut rnd = |m: usize| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize % m
        };
        // 30 slow particles (one pixel every 6 frames), each living 120 frames, respawning at random places
        let mut parts: Vec<(usize, usize, usize)> = (0..30).map(|_| (rnd(FW), rnd(FH), rnd(120))).collect();
        for k in 0..n {
            let f = &mut lo[k * FW * FH..(k + 1) * FW * FH];
            f.iter_mut().enumerate().for_each(|(i, p)| *p = 60 + (i % 13) as u8 * 5);
            for p in parts.iter_mut() {
                p.2 += 1;
                if p.2 >= 120 {
                    *p = (rnd(FW), rnd(FH), 0);
                }
                if k % 6 == 0 {
                    p.0 = (p.0 + 1) % FW;
                }
                f[p.1 * FW + p.0] = 250;
            }
            let lk = lo[k * FW * FH..(k + 1) * FW * FH].to_vec();
            let h = &mut hi[k * RW * RH..(k + 1) * RW * RH];
            for y in 0..RH {
                for x in 0..RW {
                    h[y * RW + x] = lk[(y / 3) * FW + x / 3];
                }
            }
        }
        let f = LoopFrames { n, lo, hi, hw: false };
        let none = |_s0: usize| -> Result<Vec<Vec<u8>>> { unreachable!() };
        let r = find_loops(&f, LoopOptions { fps: 18.0, min_len: 3.0, max_len: 10.0 }, &none, &CancelToken::new(), &|_, _| {}).unwrap();
        let best = &r.candidates[0];
        assert!(best.blend.unwrap() >= 0.5, "a cross-fade: {best:?}");
        assert!(best.ratio < 1.4, "the fade hides the join: {best:?}");
        assert!(best.ghost.unwrap() < 0.03, "only the particles fade: {best:?}");
    }
    #[test]
    fn finds_a_planted_loop() {
        // a sawtooth "video": frame k looks like k % 50, so loops of 50 source frames are perfect
        let n = 400;
        let mut lo = vec![0u8; n * FW * FH];
        let mut hi = vec![0u8; n * RW * RH];
        for k in 0..n {
            let v = ((k % 50) * 5) as u8;
            lo[k * FW * FH..(k + 1) * FW * FH].iter_mut().enumerate().for_each(|(i, p)| *p = v.wrapping_add((i % 7) as u8));
            hi[k * RW * RH..(k + 1) * RW * RH].iter_mut().enumerate().for_each(|(i, p)| *p = v.wrapping_add((i % 7) as u8));
        }
        let f = LoopFrames { n, lo, hi, hw: false };
        let none = |_s0: usize| -> Result<Vec<Vec<u8>>> { unreachable!() };
        let r = find_loops(&f, LoopOptions { fps: 25.0, min_len: 1.0, max_len: 5.0 }, &none, &CancelToken::new(), &|_, _| {}).unwrap();
        let best = &r.candidates[0];
        assert_eq!(best.cost, 0.0);
        assert_eq!((best.frames as f64 * 25.0 / 18.0).round() as usize % 50, 0);
    }
}
