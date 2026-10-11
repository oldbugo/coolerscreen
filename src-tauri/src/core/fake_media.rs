//! A stand-in media engine (no ffmpeg) for tests and for running the UI before WS2 lands:
//! `COOLERSCREEN_FAKE_MEDIA=1`. Its device.mp4 files are NOT real videos, so with fake media the device is always
//! the fake device too (enforced in `App::new`): nothing it makes can reach the real screen.
use super::api::{EncodeReq, LoopCardsReq, LoopPointReq, LoopSearchReq, MediaEngine, ProbeInfo, SourceAssets};
use super::error::{CoreError, CoreResult};
use super::js::num;
use super::scheduler::TaskCtx;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

pub struct FakeMedia {
    /// Simulated work time per heavy call.
    pub delay: Duration,
}

impl Default for FakeMedia {
    fn default() -> Self {
        FakeMedia { delay: Duration::from_millis(400) }
    }
}

fn fbox(t: &[u8], body: &[u8]) -> Vec<u8> {
    let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    v.extend_from_slice(t);
    v.extend_from_slice(body);
    v
}

/// A tiny ISO-mp4-shaped file with an MJPEG sample entry (passes the guard; not playable).
pub fn fake_mjpeg_mp4(payload: usize) -> Vec<u8> {
    let mut v = fbox(b"ftyp", b"isom\0\0\x02\0");
    v.extend(fbox(b"mdat", &vec![0u8; payload]));
    v.extend(fbox(b"moov", &fbox(b"trak", b"....jpeg....")));
    v
}

/// A solid-colour PNG (browsers sniff it even when named .jpg).
pub fn png(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity(((w * 3 + 1) * h) as usize);
    for _ in 0..h {
        raw.push(0);
        for _ in 0..w {
            raw.extend_from_slice(&rgb);
        }
    }
    let chunk = |t: &[u8], d: &[u8]| {
        let mut c = (d.len() as u32).to_be_bytes().to_vec();
        let mut td = t.to_vec();
        td.extend_from_slice(d);
        c.extend_from_slice(&td);
        c.extend_from_slice(&super::theme::crc32(&td).to_be_bytes());
        c
    };
    let mut ihdr = w.to_be_bytes().to_vec();
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    out.extend(chunk(b"IHDR", &ihdr));
    out.extend(chunk(b"IDAT", &miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6)));
    out.extend(chunk(b"IEND", &[]));
    out
}

fn colour_of(p: &Path) -> [u8; 3] {
    let h = super::theme::crc32(p.to_string_lossy().as_bytes());
    [(h >> 16) as u8 | 0x40, (h >> 8) as u8 | 0x40, h as u8 | 0x40]
}

fn is_image(p: &Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref(),
        Some("png" | "jpg" | "jpeg" | "bmp" | "webp" | "tif" | "tiff")
    )
}

impl MediaEngine for FakeMedia {
    fn is_fake(&self) -> bool {
        true
    }

    fn probe(&self, _: &TaskCtx, file: &Path) -> CoreResult<ProbeInfo> {
        let ok = std::fs::metadata(file).map(|m| m.len() > 0).unwrap_or(false);
        let img = is_image(file);
        Ok(ProbeInfo {
            duration: if img || !ok { None } else { Some(12.0) },
            width: ok.then_some(1920),
            height: ok.then_some(1080),
            fps: if img || !ok { None } else { Some(25.0) },
            codec: ok.then(|| if img { "png".into() } else { "h264".into() }),
            has_video: ok,
            is_image: img,
            rotated: false,
        })
    }

    fn source_assets(&self, ctx: &TaskCtx, file: &Path, dir: &Path, _: &ProbeInfo) -> CoreResult<SourceAssets> {
        ctx.sleep(self.delay / 4)?;
        std::fs::create_dir_all(dir)?;
        let c = colour_of(file);
        std::fs::write(dir.join("thumb.jpg"), png(240, 108, c))?;
        std::fs::write(dir.join("frame.jpg"), png(1200, 540, c))?;
        Ok(SourceAssets { proxy: None })
    }

    fn loop_search(&self, ctx: &TaskCtx, req: &LoopSearchReq) -> CoreResult<Value> {
        for i in 0..4 {
            ctx.progress(i as f64 / 4.0, "Comparing frames");
            ctx.sleep(self.delay / 4)?;
        }
        let mut cs = vec![];
        for i in 0..3 {
            let start = 0.5 + i as f64;
            let length = (req.min_len + i as f64).min(req.max_len);
            cs.push(json!({ "start": num(start), "startFrame": (start * 25.0) as u64, "length": num(length), "frames": (length * 18.0) as u64,
                "end": num(start + length), "endFrame": ((start + length) * 25.0) as u64 - 1, "cost": 3.2, "ratio": num(0.9 + i as f64 * 0.4),
                "similarity": 98.7 }));
        }
        Ok(json!({ "frames": 300, "searched": { "minLen": num(req.min_len), "maxLen": num(req.max_len), "truncated": false }, "candidates": cs }))
    }

    fn loop_cards(&self, ctx: &TaskCtx, req: &LoopCardsReq) -> CoreResult<()> {
        for i in 0..req.candidates.as_array().map(Vec::len).unwrap_or(0) {
            ctx.sleep(self.delay / 8)?;
            let _ = std::fs::copy(&req.file, req.dir.join(format!("loop-{i}.mp4")));
        }
        Ok(())
    }

    fn loop_point(&self, ctx: &TaskCtx, req: &LoopPointReq) -> CoreResult<Value> {
        ctx.sleep(self.delay / 4)?;
        let pick = |end: f64| json!({ "end": num(end), "length": num(((end - req.start) * 18.0).round() / 18.0), "similarity": 97.5 });
        Ok(json!({ "best": pick(req.end), "current": pick(req.end), "alternatives": [pick(req.end - 0.5)], "searched": [req.end - 2.0, req.end + 2.0] }))
    }

    fn bytes_per_second(&self, ctx: &TaskCtx, _: &EncodeReq, _: u32) -> CoreResult<f64> {
        ctx.sleep(self.delay / 4)?;
        Ok(800_000.0)
    }

    fn estimate(&self, ctx: &TaskCtx, req: &EncodeReq) -> CoreResult<Value> {
        ctx.sleep(self.delay)?;
        let manual = req.quality != "auto";
        let q = if manual { 69.0 - ((req.quality.as_f64().unwrap_or(80.0) - 1.0) * 67.0 / 99.0).round() } else { 12.0 };
        let bytes = (req.length * 700_000.0 * (30.0 / q.max(2.0)).min(3.0)).round();
        let fits = bytes <= req.budget_bytes as f64;
        let mut r = json!({ "length": num(req.length), "start": num(req.start), "budget": req.budget_bytes, "duration": req.info.duration,
            "fps": 18, "mode": if manual { "manual" } else { "auto" }, "q": num(q), "detail": req.detail, "bytes": num(bytes), "fits": fits });
        if !fits {
            r["maxFitLength"] = num((req.budget_bytes as f64 / (bytes / req.length) * 10.0).floor() / 10.0);
        }
        Ok(r)
    }

    fn convert(&self, ctx: &TaskCtx, req: &EncodeReq, out_dir: &Path) -> CoreResult<Value> {
        for i in 0..10 {
            ctx.progress(i as f64 / 10.0, "Preparing");
            ctx.sleep(self.delay / 5)?;
        }
        std::fs::create_dir_all(out_dir)?;
        let size = ((req.length * 600_000.0) as u64).min(req.budget_bytes.saturating_sub(1000)).max(1000);
        let data = fake_mjpeg_mp4(size as usize - 40);
        std::fs::write(out_dir.join("device.mp4"), &data)?;
        if req.file.extension().is_some_and(|e| e.eq_ignore_ascii_case("mp4")) {
            let _ = std::fs::copy(&req.file, out_dir.join("preview.mp4"));
        }
        std::fs::write(out_dir.join("thumb.jpg"), png(240, 108, colour_of(&req.file)))?;
        Ok(json!({ "size": data.len(), "q": 12, "detail": req.detail, "fps": 18, "start": num(req.start), "length": num(req.length), "tooBig": false }))
    }

    fn splash(&self, ctx: &TaskCtx, out: &Path) -> CoreResult<()> {
        ctx.sleep(self.delay / 4)?;
        std::fs::write(out, fake_mjpeg_mp4(540_000)).map_err(CoreError::from)
    }
}
