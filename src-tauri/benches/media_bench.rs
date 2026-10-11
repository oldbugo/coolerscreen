//! Before/after benchmark and golden comparison against 1.2 (tests/golden/ref12.js output).
//!
//! usage: cargo bench --bench media_bench -- <clip> <outdir>
//!   <outdir>/ref12.json and <outdir>/v12-{a,b}/device.mp4 must exist (run ref12.js first with the same outdir).
//! Prints a JSON report: timings (ms) of the 2.0 pipeline for the same user actions, the loop ranking against 1.2, the
//! estimate/convert q and sizes against 1.2, and PSNR/SSIM + device checks of the converted files.
use cs_core::media::{self, encode::VersionRequest, gpu, loops, verify, CancelToken, Engine, MediaConfig};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Instant;

fn req_from(v: &Value) -> VersionRequest {
    VersionRequest {
        start: v["start"].as_f64().unwrap(),
        length: v["length"].as_f64(),
        quality: v["quality"].as_f64(),
        detail: v["detail"].as_f64().unwrap_or(0.0),
        frame: serde_json::from_value(v["frame"].clone()).unwrap_or_default(),
        mask: serde_json::from_value(v["mask"].clone()).unwrap(),
        corners: serde_json::from_value(v["corners"].clone()).unwrap_or_default(),
        blend: v["blend"].as_f64().unwrap_or(0.0),
        budget_bytes: v["budgetBytes"].as_u64().unwrap(),
        max_seconds: v["maxSeconds"].as_f64(),
    }
}

/// Every candidate field equal as numbers (1.2's JSON writes 3 where Rust writes 3.0).
fn same_candidates(a: &Value, b: &Value) -> bool {
    let (a, b) = (a.as_array().unwrap(), b.as_array().unwrap());
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            ["start", "startFrame", "length", "frames", "end", "endFrame", "cost", "ratio", "similarity"].iter().all(|k| x[k].as_f64() == y[k].as_f64())
        })
}

fn ms(t: Instant) -> u64 {
    t.elapsed().as_millis() as u64
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).filter(|a| a != "--bench").collect();
    let clip = PathBuf::from(&args[0]);
    let out = PathBuf::from(&args[1]);
    let r12: Value = serde_json::from_slice(&std::fs::read(out.join("ref12.json")).expect("run ref12.js first")).unwrap();
    let mut cfg = MediaConfig::from_env();
    cfg.base_dir = out.join("home12"); // share the corner mask PNG made by ref12.js
    cfg.work_dir = out.join("work20");
    let cancel = CancelToken::new();
    let quiet = |_: f64, _: &str| {};
    let mut t = serde_json::Map::new();

    let t0 = Instant::now();
    let caps = gpu::detect(&cfg.ffmpeg);
    t.insert("gpuDetect".into(), json!(ms(t0)));
    let eng = Engine::new(cfg.clone());
    let info = eng.probe(&clip).unwrap();
    let sel = req_from(&r12["sel"]);
    let sel2 = req_from(&r12["sel2"]);
    let budget = sel.budget_bytes as f64;
    let dur = info.duration.unwrap();
    let odir = out.join("o20");
    let _ = std::fs::remove_dir_all(&odir);

    // ---- open a clip: frames || loop-length sample, then search, then cards || estimate of the top loop
    let t_open = Instant::now();
    let (frames, bps) = std::thread::scope(|s| {
        let f = s.spawn(|| loops::read_frames(eng.ffmpeg(), &clip, &info, Some(&odir), &cancel, &quiet));
        let mut r = sel.clone();
        r.start = 0.0;
        r.length = Some(2f64.min(dur));
        r.corners = Default::default();
        r.frame = Default::default();
        let b = eng.sample_bytes_per_second(&clip, Some(&info), &r, 25.0, 0.0, &cancel).unwrap();
        (f.join().unwrap().unwrap(), b)
    });
    let t_frames = ms(t_open);
    let max_len = 3f64.max(1f64.max((budget * 0.94 / bps * 2.0).floor() / 2.0).min(dur - 0.2).min(30.0));
    let sf = info.fps.unwrap_or(25.0);
    let seek = |s0: usize| loops::seek_hi_frames(eng.ffmpeg(), &clip, sf, s0, &cancel);
    let tc = Instant::now();
    let mut found = loops::find_loops(&frames, loops::LoopOptions { fps: sf, min_len: 3.0, max_len }, &seek, &cancel, &quiet).unwrap();
    let t_compute = ms(tc);
    for c in found.candidates.iter_mut() {
        c.blend = Some(loops::auto_blend(c));
    }
    let t_loops = ms(t_open);
    // the meter for the top loop (2.0's own pick) and the cards, concurrently
    let mut top = sel.clone();
    top.start = found.candidates[0].start;
    top.length = Some(found.candidates[0].length);
    top.blend = found.candidates[0].blend.unwrap();
    let (est_top, t_meter) = std::thread::scope(|s| {
        let cards = s.spawn(|| loops::render_cards(eng.ffmpeg(), &clip, &info, &found.candidates, &odir, &cancel, &quiet));
        let tm = Instant::now();
        let e = eng.estimate(&clip, Some(&info), &top, &cancel, &quiet).unwrap();
        let tmeter = ms(t_open);
        let _ = tm;
        cards.join().unwrap().unwrap();
        (e, tmeter)
    });
    t.insert("openReady".into(), json!(ms(t_open)));
    t.insert("openMeter".into(), json!(t_meter));
    t.insert("openLoops".into(), json!(t_loops));
    t.insert("loopFramesAndBps".into(), json!(t_frames));
    t.insert("loopCompute".into(), json!(t_compute));

    // ---- estimate for 1.2's selection (same q expected), then a tuning change
    let te = Instant::now();
    let est = eng.estimate(&clip, Some(&info), &sel, &cancel, &quiet).unwrap();
    t.insert("estimate".into(), json!(ms(te)));
    let te = Instant::now();
    let est2 = eng.estimate(&clip, Some(&info), &sel2, &cancel, &quiet).unwrap();
    t.insert("estimateAfterChange".into(), json!(ms(te)));
    // ---- save as version: warm (estimate done) and cold (new engine, empty cache)
    let ts = Instant::now();
    let cv = eng.convert(&clip, Some(&info), &sel, &out.join("v20-a"), &cancel, &quiet).unwrap();
    t.insert("saveWarm".into(), json!(ms(ts)));
    let ts = Instant::now();
    let cv2 = eng.convert(&clip, Some(&info), &sel2, &out.join("v20-b"), &cancel, &quiet).unwrap();
    t.insert("save2Warm".into(), json!(ms(ts)));
    let mut cfg2 = cfg.clone();
    cfg2.work_dir = out.join("work20cold");
    let eng2 = Engine::new(cfg2);
    let ts = Instant::now();
    let cold = eng2.convert(&clip, Some(&info), &sel, &out.join("v20-c"), &cancel, &quiet).unwrap();
    t.insert("saveCold".into(), json!(ms(ts)));
    drop(eng2);

    // ---- loop port check: same frames, but the leader re-check done 1.2's way (seeks) -> must equal 1.2 exactly
    let seek_mode = if !frames.hw {
        let lo_only = loops::LoopFrames { n: frames.n, lo: frames.lo.clone(), hi: vec![], hw: false };
        let r = loops::find_loops(&lo_only, loops::LoopOptions { fps: sf, min_len: 3.0, max_len }, &seek, &cancel, &quiet).unwrap();
        Some(serde_json::to_value(&r).unwrap())
    } else {
        None
    };

    // ---- golden comparisons
    let ff = &cfg.ffmpeg;
    let cmp = |a: &str, b: &str, mask| {
        let (fa, fb) = (out.join(a).join("device.mp4"), out.join(b).join("device.mp4"));
        let s = verify::compare(ff, &fa, &fb).unwrap();
        let chk = verify::check_device_file(ff, &fa, mask).unwrap();
        json!({ "vs": b, "psnr": s.psnr, "ssim": s.ssim, "identicalBytes": s.identical_bytes, "check": chk, "ok": chk.ok(),
                "size": std::fs::metadata(&fa).unwrap().len(), "size12": std::fs::metadata(&fb).unwrap().len() })
    };
    let ranking = |l: &Value| l["candidates"].as_array().unwrap().iter().map(|c| json!([c["startFrame"], c["frames"], c["ratio"]])).collect::<Vec<_>>();
    let ours = serde_json::to_value(&found).unwrap();
    let report = json!({
        "clip": clip.file_name().unwrap().to_string_lossy(), "gpu": caps, "nvdecUsed": frames.hw, "info": info,
        "t20": t, "t12": r12["t"],
        "loops": { "same": ranking(&ours) == ranking(&r12["loops"]), "ours": ranking(&ours), "v12": ranking(&r12["loops"]),
                   "seekModeSameAs12": seek_mode.as_ref().map(|s| same_candidates(&s["candidates"], &r12["loops"]["candidates"])),
                   "oursSameValuesAs12": same_candidates(&ours["candidates"], &r12["loops"]["candidates"]) },
        "topEstimate": { "q": est_top.q, "bytes": est_top.bytes },
        "estimate": { "q20": est.q, "q12": r12["estimate"]["q"], "bytes20": est.bytes, "fits": est.fits, "convertQ12": r12["convert"]["q"], "convertSize12": r12["convert"]["size"] },
        "estimate2": { "q20": est2.q, "q12": r12["estimate2"]["q"], "bytes20": est2.bytes, "convertQ12": r12["convert2"]["q"], "convertSize12": r12["convert2"]["size"] },
        "convert": { "q": cv.q, "size": cv.size, "q2": cv2.q, "size2": cv2.size, "coldQ": cold.q, "budget": sel.budget_bytes },
        "golden": [cmp("v20-a", "v12-a", &sel.mask), cmp("v20-b", "v12-b", &sel2.mask), cmp("v20-c", "v12-a", &sel.mask)],
    });
    let s = serde_json::to_string_pretty(&report).unwrap();
    std::fs::write(out.join("report20.json"), &s).unwrap();
    println!("{s}");
    let _ = media::no_progress;
    let _: &Path = &out;
}
