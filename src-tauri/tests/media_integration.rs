//! Media pipeline integration tests against a real clip (ffmpeg sidecar, no screen).
//! Set CS_TEST_CLIP to a video file (e.g. a copy of the pink-blue original); without it the tests do nothing.
//! `cargo test --release --test media_integration -- --nocapture --test-threads 1`
use cs_core::media::{self, assets, encode::VersionRequest, gpu, graph, loops, verify, CancelToken, Engine, MediaConfig, MediaError};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

fn setup(tag: &str) -> Option<(PathBuf, MediaConfig, PathBuf)> {
    let clip = PathBuf::from(std::env::var_os("CS_TEST_CLIP")?);
    let out = std::env::temp_dir().join(format!("cs-media-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).unwrap();
    let mut cfg = MediaConfig::from_env();
    cfg.base_dir = out.join("home");
    cfg.work_dir = out.join("work");
    gpu::detect(&cfg.ffmpeg);
    Some((clip, cfg, out))
}

fn req(start: f64, length: f64) -> VersionRequest {
    VersionRequest {
        start,
        length: Some(length),
        quality: None,
        detail: 0.0,
        frame: Default::default(),
        mask: graph::Mask::band(graph::Edge::Left, 92.0),
        corners: graph::Corners { on: true, r: 60.0 },
        blend: 0.3,
        budget_bytes: 8_940_000,
        max_seconds: Some(10.0),
    }
}

#[test]
fn estimate_progress_and_cancel() {
    let Some((clip, cfg, out)) = setup("cancel") else { return };
    let eng = Engine::new(cfg.clone());
    // progress: increasing to 1
    let seen = Mutex::new(vec![]);
    let e = eng.estimate(&clip, None, &req(2.0, 6.0), &CancelToken::new(), &|f, m| seen.lock().unwrap().push((f, m.to_string()))).unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(e.fits && e.exact);
    assert_eq!(seen.last().unwrap().0, 1.0);
    assert!(seen.len() > 3, "progress reported: {seen:?}");
    // cancel in the middle of a fresh estimate (different selection = new decode): returns promptly
    let tok = CancelToken::new();
    let t = Instant::now();
    let r = std::thread::scope(|s| {
        let h = s.spawn(|| eng.estimate(&clip, None, &req(1.0, 9.0), &tok, &media::no_progress));
        std::thread::sleep(Duration::from_millis(400));
        tok.cancel();
        let tc = Instant::now();
        let r = h.join().unwrap();
        (r, tc.elapsed())
    });
    assert!(matches!(r.0, Err(MediaError::Cancelled)), "{:?}", r.0.map(|e| e.q));
    assert!(r.1 < Duration::from_millis(300), "cancel took {:?}", r.1);
    println!("cancel returned {:?} after the cancel ({:?} total)", r.1, t.elapsed());
    // after a cancel the same request works (no half-made raw kept)
    let e2 = eng.estimate(&clip, None, &req(1.0, 9.0), &CancelToken::new(), &media::no_progress).unwrap();
    assert!(e2.bytes > 0);
    // convert reuses the estimate's encode and passes the device checks
    let t = Instant::now();
    let c = eng.convert(&clip, None, &req(1.0, 9.0), &out.join("v"), &CancelToken::new(), &media::no_progress).unwrap();
    println!("save after estimate: {:?}", t.elapsed());
    assert_eq!((c.q, c.size), (e2.q, e2.bytes));
    let chk = verify::check_device_file(&cfg.ffmpeg, &out.join("v").join("device.mp4"), &req(1.0, 9.0).mask).unwrap();
    assert!(chk.ok(), "{chk:?}");
    assert_eq!(chk.frames, 162);
    assert!(out.join("v").join("preview.mp4").is_file() && out.join("v").join("thumb.jpg").is_file());
    drop(eng);
    let _ = std::fs::remove_dir_all(&out);
}

/// A lossless clip whose frame k has luma k (k < 250), at `fps` with the given mp4 timescale.
fn numbered_clip(dir: &std::path::Path, fps: u32, timescale: u32) -> PathBuf {
    let f = dir.join(format!("numbered-{fps}-{timescale}.mp4"));
    let ff = MediaConfig::from_env().ffmpeg;
    let a: Vec<String> = [
        "-f", "lavfi", "-i", &format!("nullsrc=s=64x36:r={fps}:d={}", 240.0 / fps as f64), "-vf", "geq=lum='N':cb=128:cr=128,format=yuv420p",
        "-c:v", "libx264", "-qp", "0", "-video_track_timescale", &timescale.to_string(), "-y", &f.to_string_lossy(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    media::ffmpeg::run(&ff, &a, &CancelToken::new(), None).unwrap();
    f
}

/// Source frame index of every output frame of a selection rendered through the version graph (small canvas).
fn rendered_indices(file: &std::path::Path, fps: f64, start: f64, length: f64, blend: f64) -> Vec<u32> {
    let ff = MediaConfig::from_env().ffmpeg;
    let base = graph::Base {
        start,
        length,
        blend,
        fps: graph::DEVICE_FPS,
        src_fps: Some(fps),
        is_image: false,
        frame: Default::default(),
        detail: 0.0,
        canvas: (64, 36),
        flags: "area".into(),
        head: String::new(),
    };
    let mut a = graph::input_args(&file.to_string_lossy(), &base, None);
    a.extend(cs_core::ff_args!["-filter_complex", format!("{};[base]format=yuv420p[out]", graph::base_graph(&base)), "-map", "[out]", "-f", "rawvideo", "pipe:1"]);
    let raw = media::ffmpeg::run_capture(&ff, &a, &CancelToken::new(), None).unwrap();
    raw.chunks_exact(64 * 36 * 3 / 2).map(|f| &f[..64 * 36]).map(|f| (f.iter().map(|&v| v as u32).sum::<u32>() as f64 / f.len() as f64).round() as u32).collect()
}

/// Version start frames are exact: a selection starting at source frame k (start = k/fps rounded to the millisecond,
/// as the library stores it, the same convention as the loop search's startFrame) renders source frame k first, and
/// the whole frame sequence is the start-0 sequence shifted by k (no rounding-dependent drift), also with a blend.
/// (1.2: the loop leader check seeked with +1 ms and read frame k+1 at fine timescales; version starts could land a
/// frame late when the millisecond rounding went up.)
#[test]
fn start_frames_are_exact() {
    let dir = std::env::temp_dir().join(format!("cs-media-frames-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (fps, ts) in [(30u32, 15360u32), (25, 12800), (24, 24), (60, 15360), (50, 90000)] {
        let clip = numbered_clip(&dir, fps, ts);
        let f = fps as f64;
        let reference = rendered_indices(&clip, f, 0.0, 2.0, 0.0);
        assert_eq!(reference.len(), 36, "{fps} fps: {} frames", reference.len());
        // fps=18 (round=near, as 1.2) keeps the last source frame of each output slot: below 36 fps that is the start
        // frame itself; at 50/60 fps frames 0 and 1 share slot 0 and frame 1 (1/60 s later) is shown, as in 1.2.
        assert_eq!(reference[0], if fps < 36 { 0 } else { 1 }, "{fps} fps first frame");
        for k in [1u32, 2, 7, 37, 50, 61, 100] {
            let start = (k as f64 / f * 1000.0).round() / 1000.0;
            let want: Vec<u32> = reference.iter().map(|i| i + k).collect();
            let got = rendered_indices(&clip, f, start, 2.0, 0.0);
            assert_eq!(got, want, "{fps} fps (timescale {ts}) start frame {k} (start {start})");
            if k >= 37 {
                // 5/18 s blend: the frames before the crossfade are unchanged
                let b = (5.0f64 / 18.0 * 1000.0).round() / 1000.0;
                let got = rendered_indices(&clip, f, start, 2.0, b);
                assert_eq!(got.len(), 36);
                assert_eq!(got[..30], want[..30], "{fps} fps start frame {k} with blend");
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// The sampled path (selections over the raw limit, or low disk): estimate from spread segments, then ONE decode
/// feeding three encoders; the result fits, passes the device checks, and nothing big is left in the cache.
#[test]
fn sampled_path_fits_and_cache_is_bounded() {
    let Some((clip, cfg, out)) = setup("sampled") else { return };
    std::env::set_var("CS_MEDIA_RAW_MAX", "1000"); // force the sampled path
    let eng = Engine::new(cfg.clone());
    let r = req(1.0, 6.0);
    let e = eng.estimate(&clip, None, &r, &CancelToken::new(), &media::no_progress).unwrap();
    assert!(!e.exact && e.fits);
    let t = Instant::now();
    let c = eng.convert(&clip, None, &r, &out.join("v"), &CancelToken::new(), &media::no_progress).unwrap();
    println!("sampled: estimate q {} -> convert q {} ({} bytes of {}), convert {:?}", e.q, c.q, c.size, r.budget_bytes, t.elapsed());
    assert!(c.size <= r.budget_bytes && (c.q as i32 - e.q as i32).abs() <= 3);
    let chk = verify::check_device_file(&cfg.ffmpeg, &out.join("v").join("device.mp4"), &r.mask).unwrap();
    assert!(chk.ok() && chk.frames == 108, "{chk:?}");
    assert!(eng.cache_bytes() < 300 << 20, "cache {}", eng.cache_bytes());
    std::env::remove_var("CS_MEDIA_RAW_MAX");
    // shutdown removes the run folder
    let run = eng.run_dir().to_path_buf();
    eng.shutdown();
    assert!(!run.exists());
    drop(eng);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn too_long_reports_max_fit_length() {
    let Some((clip, cfg, out)) = setup("toolong") else { return };
    let eng = Engine::new(cfg);
    let mut r = req(0.0, 8.0);
    r.budget_bytes = 400_000; // far too small even at q=69 with blur
    let e = eng.estimate(&clip, None, &r, &CancelToken::new(), &media::no_progress).unwrap();
    assert!(!e.fits && e.max_fit_length.is_some());
    match eng.convert(&clip, None, &r, &out.join("v"), &CancelToken::new(), &media::no_progress) {
        Err(MediaError::TooLong { max_fit_length: Some(n) }) => assert_eq!(Some(n), e.max_fit_length),
        other => panic!("{:?}", other.map(|c| c.q)),
    }
    drop(eng);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn loop_point_assets_splash() {
    let Some((clip, cfg, out)) = setup("misc") else { return };
    let ff = &cfg.ffmpeg;
    let info = media::probe::probe(ff, &clip).unwrap();
    let lp = loops::loop_point(ff, &clip, 1.0, 5.0, info.duration, info.fps, &CancelToken::new()).unwrap();
    assert!(lp.best.similarity >= lp.current.similarity && lp.alternatives.len() <= 3);
    let t = Instant::now();
    let proxy = assets::make_source_assets(ff, &clip, &out.join("o"), &info, &CancelToken::new(), &media::no_progress).unwrap();
    println!("source assets ({:?}, gpu proxy {}): {:?}", proxy, gpu::use_gpu_proxy(&info), t.elapsed());
    assert!(out.join("o").join("thumb.jpg").is_file() && out.join("o").join("frame.jpg").is_file());
    let (f, size) = assets::splash(ff, &cfg.base_dir, &CancelToken::new()).unwrap();
    assert!(cs_core::device::mjpeg::is_mjpeg_mp4(&std::fs::read(&f).unwrap()));
    assert!(size > 100_000 && size < 1_500_000, "splash {size}");
    let _ = std::fs::remove_dir_all(&out);
}
