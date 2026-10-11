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
