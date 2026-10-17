//! `core::api::MediaEngine` (WS1 contract, docs/v2/api-contract.md section 5.1) implemented over [`Engine`].
//!
//! Wiring once WS1's `core` is merged: add `pub mod core_engine;` to `media/mod.rs` and pass
//! `Arc::new(media::core_engine::CoreMedia::new(MediaConfig { ffmpeg, base_dir: paths.base, work_dir }))` as
//! `Backends::media`. (Not referenced from mod.rs on the ws2-media branch because `crate::core` does not exist there
//! yet; it compiles against origin/ws1-core a8530fd.)
use super::encode::VersionRequest;
use super::graph::{Band, Circle, Corners, Edge, Frame, Mask, MaskMode};
use super::{assets, gpu, loops, CancelToken, Engine, MediaConfig, MediaError, SourceInfo};
use crate::core::api::{EncodeReq, LoopCardsReq, LoopPointReq, LoopSearchReq, MediaEngine, MediaTask, ProbeInfo, SourceAssets};
use crate::core::error::{CoreError, CoreResult};
use crate::core::scheduler::{Res, Resources, TaskCtx};
use serde_json::Value;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct CoreMedia {
    eng: Engine,
}

impl CoreMedia {
    /// Starts GPU detection in the background (CPU paths are used until it finishes).
    pub fn new(cfg: MediaConfig) -> Self {
        let ff = cfg.ffmpeg.clone();
        std::thread::spawn(move || {
            gpu::detect(&ff);
        });
        CoreMedia { eng: Engine::new(cfg) }
    }
    pub fn engine(&self) -> &Engine {
        &self.eng
    }
}

fn to_core(e: MediaError) -> CoreError {
    match e {
        MediaError::Cancelled => CoreError::cancelled(),
        MediaError::TooLong { max_fit_length } => {
            let msg = MediaError::TooLong { max_fit_length }.to_string();
            let e = CoreError::coded("tooLong", msg);
            match max_fit_length {
                Some(m) => e.with("maxFitLength", m),
                None => e,
            }
        }
        MediaError::Invalid(m) => CoreError::new(m),
        e => CoreError::new(e.to_string()).status(500),
    }
}

/// Run a media operation with a [`CancelToken`] that follows the task's cancellation (ffmpeg children are killed by
/// the media runner within a few ms).
fn bridged<T>(ctx: &TaskCtx, f: impl FnOnce(&CancelToken) -> super::Result<T>) -> CoreResult<T> {
    ctx.check()?;
    let tok = CancelToken::new();
    let done = AtomicBool::new(false);
    let r = std::thread::scope(|s| {
        s.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                if ctx.is_cancelled() {
                    tok.cancel();
                    break;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        });
        let r = f(&tok);
        done.store(true, Ordering::Relaxed);
        r
    });
    match r {
        Err(MediaError::Cancelled) => Err(ctx.check().err().unwrap_or_else(CoreError::cancelled)),
        r => r.map_err(to_core),
    }
}

fn info_from(p: &ProbeInfo) -> SourceInfo {
    SourceInfo {
        duration: p.duration,
        width: p.width,
        height: p.height,
        fps: p.fps,
        codec: p.codec.clone(),
        has_video: p.has_video,
        is_image: p.is_image,
        rotated: p.rotated,
    }
}
fn probe_from(i: &SourceInfo) -> ProbeInfo {
    ProbeInfo {
        duration: i.duration,
        width: i.width,
        height: i.height,
        fps: i.fps,
        codec: i.codec.clone(),
        has_video: i.has_video,
        is_image: i.is_image,
        rotated: i.rotated,
    }
}

fn num(v: &Value, k: &str) -> Option<f64> {
    match v.get(k) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Lenient parse of 1.2's mask JSON (`{ mode, band: { edge, px }, circle: { cx, cy, r, feather } }`).
pub fn parse_mask(v: &Value) -> Mask {
    let band = v.get("band").filter(|b| b.is_object()).map(|b| Band {
        edge: match b.get("edge").and_then(Value::as_str) {
            Some("right") => Edge::Right,
            Some("top") => Edge::Top,
            Some("bottom") => Edge::Bottom,
            _ => Edge::Left,
        },
        px: num(b, "px").unwrap_or(0.0),
    });
    let circle = v.get("circle").filter(|c| c.is_object()).map(|c| Circle {
        cx: num(c, "cx").unwrap_or(0.0),
        cy: num(c, "cy").unwrap_or(0.0),
        r: num(c, "r").unwrap_or(0.0),
        feather: num(c, "feather").unwrap_or(0.0),
    });
    let mode = match v.get("mode").and_then(Value::as_str) {
        Some("band") => MaskMode::Band,
        Some("circle") => MaskMode::Circle,
        _ => MaskMode::Off,
    };
    let fit = crate::core::fit::Fit::from_value(v.get("fit").unwrap_or(&Value::Null)).as_array();
    Mask { mode, band, circle, fit }
}
fn parse_frame(v: &Value) -> Frame {
    Frame { zoom: num(v, "zoom").unwrap_or(1.0), x: num(v, "x").unwrap_or(0.5), y: num(v, "y").unwrap_or(0.5) }.norm()
}
fn parse_corners(v: &Value) -> Corners {
    Corners { on: v.get("on").and_then(Value::as_bool).unwrap_or(false), r: num(v, "r").unwrap_or(0.0) }
}

/// EncodeReq -> the media request.
pub fn version_request(r: &EncodeReq) -> VersionRequest {
    let quality = match &r.quality {
        Value::Number(n) => n.as_f64(),
        Value::String(s) if s != "auto" => s.parse().ok(),
        _ => None,
    };
    VersionRequest {
        start: r.start,
        length: Some(r.length),
        quality,
        detail: r.detail as f64,
        frame: parse_frame(&r.frame),
        mask: parse_mask(&r.mask),
        corners: parse_corners(&r.corners),
        blend: r.blend,
        budget_bytes: r.budget_bytes,
        max_seconds: Some(r.max_seconds),
        overlay: r.overlay.clone(),
    }
}

impl MediaEngine for CoreMedia {
    fn probe(&self, ctx: &TaskCtx, file: &Path) -> CoreResult<ProbeInfo> {
        ctx.check()?;
        self.eng.probe(file).map(|i| probe_from(&i)).map_err(to_core)
    }

    fn source_assets(&self, ctx: &TaskCtx, file: &Path, dir: &Path, info: &ProbeInfo) -> CoreResult<SourceAssets> {
        let mut si = info_from(info);
        si.rotated = self.eng.probe(file).map(|i| i.rotated).unwrap_or(false);
        let proxy = bridged(ctx, |tok| assets::make_source_assets(self.eng.ffmpeg(), file, dir, &si, tok, &|f, m| ctx.progress(f, m)))?;
        Ok(SourceAssets { proxy })
    }

    fn loop_search(&self, ctx: &TaskCtx, req: &LoopSearchReq) -> CoreResult<Value> {
        let mut info = self.eng.probe(&req.file).map_err(to_core)?;
        if info.fps.is_none() {
            info.fps = req.fps;
        }
        if info.duration.is_none() {
            info.duration = Some(req.duration);
        }
        bridged(ctx, |tok| {
            let mut r = loops::search(self.eng.ffmpeg(), &req.file, &info, Some(&req.dir), req.min_len, req.max_len, tok, &|f, m| {
                ctx.progress(f, m)
            })?;
            // The search picks each candidate's cross-fade (the length that makes the join smoothest).
            for c in r.candidates.iter_mut().filter(|c| c.blend.is_none()) {
                c.blend = Some(loops::auto_blend(c));
            }
            // Cards are rendered by core as a separate `cards` job (loop_cards), so the meter can run alongside.
            Ok(serde_json::to_value(&r).unwrap())
        })
    }

    fn loop_point(&self, ctx: &TaskCtx, req: &LoopPointReq) -> CoreResult<Value> {
        bridged(ctx, |tok| loops::loop_point(self.eng.ffmpeg(), &req.file, req.start, req.end, req.duration, req.fps, tok))
            .map(|r| serde_json::to_value(r).unwrap())
    }

    fn bytes_per_second(&self, ctx: &TaskCtx, req: &EncodeReq, quality: u32) -> CoreResult<f64> {
        let vr = version_request(req);
        let info = info_from(&req.info);
        bridged(ctx, |tok| self.eng.sample_bytes_per_second(&req.file, Some(&info), &vr, quality as f64, vr.detail, tok))
    }

    fn estimate(&self, ctx: &TaskCtx, req: &EncodeReq) -> CoreResult<Value> {
        let vr = version_request(req);
        let info = info_from(&req.info);
        bridged(ctx, |tok| self.eng.estimate(&req.file, Some(&info), &vr, tok, &|f, m| ctx.progress(f, m))).map(|r| {
            let mut v = serde_json::to_value(r).unwrap();
            let o = v.as_object_mut().unwrap();
            if o.get("maxFitLength").is_some_and(Value::is_null) {
                o.remove("maxFitLength"); // 1.2 only sets it when nothing fits
            }
            v
        })
    }

    fn convert(&self, ctx: &TaskCtx, req: &EncodeReq, out_dir: &Path) -> CoreResult<Value> {
        let vr = version_request(req);
        let info = info_from(&req.info);
        bridged(ctx, |tok| self.eng.convert(&req.file, Some(&info), &vr, out_dir, tok, &|f, m| ctx.progress(f, m)))
            .map(|r| serde_json::to_value(r).unwrap())
    }

    fn splash(&self, ctx: &TaskCtx, out: &Path) -> CoreResult<()> {
        bridged(ctx, |tok| assets::make_splash(self.eng.ffmpeg(), out, tok).map(|_| ()))
    }

    fn loop_cards(&self, ctx: &TaskCtx, req: &LoopCardsReq) -> CoreResult<()> {
        let cands: Vec<loops::LoopCandidate> =
            serde_json::from_value(req.candidates.clone()).map_err(|e| CoreError::new(format!("Bad loop candidates: {e}")))?;
        let mut info = self.eng.probe(&req.file).map_err(to_core)?;
        if info.fps.is_none() {
            info.fps = req.fps;
        }
        if info.duration.is_none() {
            info.duration = Some(req.duration);
        }
        bridged(ctx, |tok| loops::render_cards(self.eng.ffmpeg(), &req.file, &info, &cands, &req.dir, tok, &|f, m| ctx.progress(f, m)).map(|_| ()))
    }

    fn splash_version(&self) -> u32 {
        assets::SPLASH_VERSION
    }

    fn resources(&self, task: MediaTask, info: Option<&ProbeInfo>) -> Resources {
        let si = info.map(info_from);
        let gpu_small = si.as_ref().is_some_and(gpu::use_nvdec_small);
        let gpu_proxy = si.as_ref().is_some_and(gpu::use_gpu_proxy);
        match task {
            MediaTask::Probe => Resources::one(Res::Io),
            MediaTask::Assets if gpu_proxy => Resources::one(Res::Io).with(Res::Gpu, 1),
            MediaTask::Assets => Resources::one(Res::Cpu),
            MediaTask::LoopSearch if gpu_small => Resources::one(Res::Cpu).with(Res::Gpu, 1),
            MediaTask::Cards if gpu_small => Resources::one(Res::Cpu).with(Res::Gpu, 1),
            MediaTask::Cards | MediaTask::LoopSearch | MediaTask::LoopPoint | MediaTask::BytesPerSecond | MediaTask::Splash => Resources::one(Res::Cpu),
            // decode once + up to 3 parallel trial encodes
            MediaTask::Estimate | MediaTask::Convert => Resources::none().with(Res::Cpu, 2),
        }
    }
}
