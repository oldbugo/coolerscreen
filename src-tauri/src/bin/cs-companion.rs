//! cs-companion: prototype A of the 3.0 live companion. A small separate program (no WebView) with a tray icon that
//! draws a clock and sensor panel on the PC and streams it to the screen as live JPEG frames.
//!   cs-companion                       run (tray icon: Pause/Resume, 30/60 fps, Quit)
//!   cs-companion --no-device            run the tray without touching the screen (frames are discarded)
//!   cs-companion --dry-run [--video f] write a few frames to work/companion-dry/ (no screen needed)
//!   cs-companion --measure 60 [--video f] [--no-device]   CPU / RAM / timings for 30 and 60 fps (with and without video)
//! Settings: %APPDATA%\CoolerScreen\companion.json. Quit, Ctrl+C, closing the console and logoff/shutdown all end the stream
//! properly: the screen plays its loop clip again (the 2.0 behaviour). No startup entry, nothing runs unless started.
use cs_core::live::encode::{frame_budget, rotate_cw, JpegEncoder};
use cs_core::live::pipeline::{Pipeline, Sink, StatsSnap};
use cs_core::live::render::Renderer;
use cs_core::live::sensors::Sampler;
use cs_core::live::{Config, LH, LW, PH, PW};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{HANDLE, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Console::{FreeConsole, GetConsoleProcessList, SetConsoleCtrlHandler};
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, GetCurrentThreadId, GetProcessTimes};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

static PIPELINE: OnceLock<Arc<Pipeline>> = OnceLock::new();
static QUIT: AtomicBool = AtomicBool::new(false);
static MAIN_THREAD: AtomicU32 = AtomicU32::new(0);

fn wake_main() {
    let t = MAIN_THREAD.load(Ordering::SeqCst);
    if t != 0 {
        unsafe { PostThreadMessageW(t, WM_NULL, 0, 0) };
    }
}

/// End the live stream (loop clip plays again) and ask the message loop to finish.
fn stop_now() {
    QUIT.store(true, Ordering::SeqCst);
    if let Some(p) = PIPELINE.get() {
        p.stop();
    }
    wake_main();
}

unsafe extern "system" fn on_ctrl(_kind: u32) -> i32 {
    stop_now();
    1
}

unsafe extern "system" fn wndproc(hwnd: *mut core::ffi::c_void, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_QUERYENDSESSION => 1,
        WM_CLOSE => {
            stop_now();
            0
        }
        WM_ENDSESSION => {
            if wp != 0 {
                stop_now();
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// A never-shown top-level window: only top-level windows get WM_ENDSESSION at logoff/shutdown.
fn make_session_window() {
    unsafe {
        let class: Vec<u16> = "CoolerScreenCompanion\0".encode_utf16().collect();
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
    }
}

fn arg_val<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut cfg = Config::load();
    if let Some(v) = arg_val(&args, "--video") {
        cfg.background = Some(v.to_string());
    }
    if let Some(f) = arg_val(&args, "--fps").and_then(|v| v.parse().ok()) {
        cfg.fps = f;
    }
    let code = if args.iter().any(|a| a == "--dry-run") {
        dry_run(&cfg)
    } else if let Some(secs) = arg_val(&args, "--measure").and_then(|v| v.parse::<u64>().ok()) {
        measure(&cfg, secs, !args.iter().any(|a| a == "--no-device"))
    } else {
        run_tray(cfg, !args.iter().any(|a| a == "--no-device"))
    };
    std::process::exit(code);
}

// ------------------------------------------------------------------ dry run

fn work_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("work").join("companion-dry")
}

fn dry_run(cfg: &Config) -> i32 {
    let dir = work_dir();
    let _ = std::fs::create_dir_all(&dir);
    let sampler = Sampler::start();
    std::thread::sleep(Duration::from_millis(1300)); // first sensor sample
    let video = cfg.background.as_deref().and_then(|b| match cs_core::live::video::VideoBg::start(std::path::Path::new(b), cfg.fps) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("video background not used: {e}");
            None
        }
    });
    if video.is_some() {
        std::thread::sleep(Duration::from_millis(1500)); // let ffmpeg deliver its first frames
    }
    let me: HANDLE = unsafe { GetCurrentProcess() };
    let ws0 = mem(me).0;
    let mut r = Renderer::new(sampler.data.clone(), video);
    println!("working set: {:.1} MB before the renderer, {:.1} MB after loading fonts and buffers", ws0, mem(me).0);
    if !r.has_fonts() {
        eprintln!("warning: no system font found, text will be missing");
    }
    let (mut portrait, mut jpeg) = (Vec::new(), Vec::new());
    let mut enc = JpegEncoder::default();
    for i in 0..4 {
        let rgba = r.render(i % 2 == 1).0.to_vec();
        // landscape preview (what the user sees on the mounted screen)
        let mut lj = Vec::new();
        let mut le = jpeg_encoder::Encoder::new(&mut lj, 90);
        le.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
        le.encode(&rgba, LW as u16, LH as u16, jpeg_encoder::ColorType::Rgba).unwrap();
        std::fs::write(dir.join(format!("frame{i}-landscape.jpg")), &lj).unwrap();
        // the exact frame that would be sent
        rotate_cw(&rgba, &mut portrait);
        let n = enc.encode(&portrait, PW, PH, frame_budget(cfg.max_kb, cfg.fps), &mut jpeg).unwrap();
        std::fs::write(dir.join(format!("frame{i}-portrait.jpg")), &jpeg).unwrap();
        println!("frame {i}: landscape {} KB, portrait (as sent) {} KB at quality {}", lj.len() / 1000, n / 1000, enc.quality);
        std::thread::sleep(Duration::from_millis(1100));
    }
    // timing of the stages, single thread, 120 frames
    let (mut tr, mut tt, mut te, mut bytes) = (0u128, 0u128, 0u128, 0usize);
    for _ in 0..120 {
        let t = Instant::now();
        r.invalidate();
        let f = r.render(true).0.to_vec();
        tr += t.elapsed().as_micros();
        let t = Instant::now();
        rotate_cw(&f, &mut portrait);
        tt += t.elapsed().as_micros();
        let t = Instant::now();
        bytes += enc.encode(&portrait, PW, PH, frame_budget(cfg.max_kb, cfg.fps), &mut jpeg).unwrap();
        te += t.elapsed().as_micros();
    }
    println!("render stages avg: background {:.2} ms, clock {:.2} ms, panel {:.2} ms", r.timing_us[0] as f64 / 120000.0, r.timing_us[1] as f64 / 120000.0, r.timing_us[2] as f64 / 120000.0);
    println!("per frame (avg of 120, one thread): render {:.2} ms (includes a 5 MB copy), rotate {:.2} ms, encode {:.2} ms, {:.0} KB, quality {}", tr as f64 / 120000.0, tt as f64 / 120000.0, te as f64 / 120000.0, bytes as f64 / 120000.0, enc.quality);
    println!("wrote frames to {}", dir.display());
    0
}

// ------------------------------------------------------------------ measuring

fn ft(h: HANDLE) -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (z, z, z, z);
    unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) };
    let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    f(k) + f(u)
}

/// (working set, private bytes) in MB.
fn mem(h: HANDLE) -> (f64, f64) {
    let mut m: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    unsafe { K32GetProcessMemoryInfo(h, &mut m, m.cb) };
    (m.WorkingSetSize as f64 / 1048576.0, m.PagefileUsage as f64 / 1048576.0)
}

fn measure(base: &Config, secs: u64, use_device: bool) -> i32 {
    let cores: f64 = std::env::var("NUMBER_OF_PROCESSORS").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let me: HANDLE = unsafe { GetCurrentProcess() };
    println!("measuring {secs} s per run, {} (cores: {cores})", if use_device { "WITH the screen" } else { "no screen (frames are discarded)" });
    let mut runs: Vec<(u32, bool)> = vec![(30, false), (60, false)];
    if base.background.is_some() {
        runs.extend([(30, true), (60, true)]);
    } else {
        println!("(no --video given: only the gradient background is measured)");
    }
    for (fps, video) in runs {
        let mut cfg = base.clone();
        cfg.fps = fps;
        if !video {
            cfg.background = None;
        }
        let p = Pipeline::start(&cfg, if use_device { Sink::Device } else { Sink::Null });
        std::thread::sleep(Duration::from_secs(4)); // warm up (connect, caches, quality settles)
        let s0 = p.shared.stats.snap();
        let (c0, w0) = (ft(me), Instant::now());
        let (mut ws_sum, mut ws_peak, mut pv_sum, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        let child0 = child_handle(&p).map(|h| (h, ft(h)));
        let mut child_ws = 0.0f64;
        while w0.elapsed().as_secs() < secs {
            std::thread::sleep(Duration::from_secs(1));
            let (ws, pv) = mem(me);
            ws_sum += ws;
            ws_peak = ws_peak.max(ws);
            pv_sum += pv;
            n += 1.0;
            if let Some((h, _)) = child0 {
                child_ws = child_ws.max(mem(h).0);
            }
        }
        let wall = w0.elapsed().as_secs_f64();
        let cpu = (ft(me) - c0) as f64 / 1e7;
        let s1 = p.shared.stats.snap();
        let child_cpu = child0.map(|(h, c)| (ft(h) - c) as f64 / 1e7);
        let d = diff(&s0, &s1);
        println!(
            "\n[{fps} fps, {}] over {wall:.0} s\n  CPU: {:.2} % of one core, {:.2} % of the whole PC (this process{})\n  memory: working set avg {:.1} MB, peak {:.1} MB, private avg {:.1} MB{}\n  rendered {:.1} fps, encoded {:.1} fps, sent {:.1} fps (unchanged frames skipped {}, dropped {} before encode, {} before send, send errors {})\n  per frame: render {:.2} ms, rotate {:.2} ms, encode {:.2} ms, send {:.2} ms; frame {:.0} KB at quality {}",
            if video { "video background" } else { "gradient background" },
            100.0 * cpu / wall,
            100.0 * cpu / wall / cores,
            if video { ", ffmpeg separate below" } else { "" },
            ws_sum / n.max(1.0),
            ws_peak,
            pv_sum / n.max(1.0),
            child_cpu.map(|c| format!("\n  ffmpeg child: CPU {:.2} % of one core, working set peak {:.1} MB", 100.0 * c / wall, child_ws)).unwrap_or_default(),
            d.rendered as f64 / wall,
            d.encoded as f64 / wall,
            d.sent as f64 / wall,
            d.unchanged_skipped,
            d.dropped_before_encode,
            d.dropped_before_send,
            d.send_errors,
            avg_ms(d.render_us, d.rendered),
            avg_ms(d.rotate_us, d.rendered),
            avg_ms(d.encode_us, d.encoded),
            avg_ms(d.send_us, d.sent),
            d.encoded_bytes as f64 / d.encoded.max(1) as f64 / 1000.0,
            s1.quality
        );
        p.stop();
    }
    0
}

fn child_handle(p: &Pipeline) -> Option<HANDLE> {
    std::thread::sleep(Duration::from_millis(500));
    p.video_child.lock().unwrap().filter(|h| *h != 0).map(|h| h as HANDLE)
}

fn avg_ms(us: u64, n: u64) -> f64 {
    us as f64 / n.max(1) as f64 / 1000.0
}

fn diff(a: &StatsSnap, b: &StatsSnap) -> StatsSnap {
    StatsSnap {
        rendered: b.rendered - a.rendered,
        render_us: b.render_us - a.render_us,
        rotate_us: b.rotate_us - a.rotate_us,
        encoded: b.encoded - a.encoded,
        encode_us: b.encode_us - a.encode_us,
        encoded_bytes: b.encoded_bytes - a.encoded_bytes,
        sent: b.sent - a.sent,
        send_us: b.send_us - a.send_us,
        sent_bytes: b.sent_bytes - a.sent_bytes,
        dropped_before_encode: b.dropped_before_encode - a.dropped_before_encode,
        unchanged_skipped: b.unchanged_skipped - a.unchanged_skipped,
        dropped_before_send: b.dropped_before_send - a.dropped_before_send,
        send_errors: b.send_errors - a.send_errors,
        reconnects: b.reconnects - a.reconnects,
        quality: b.quality,
    }
}

// ------------------------------------------------------------------ tray

fn tray_icon_image() -> tray_icon::Icon {
    let n = 32usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let inside = x >= 2 && x < 30 && y >= 2 && y < 30;
            let screen = x >= 9 && x < 23 && y >= 5 && y < 27;
            let c: [u8; 4] = if screen { [70, 160, 255, 255] } else if inside { [28, 32, 48, 255] } else { [0, 0, 0, 0] };
            px[(y * n + x) * 4..(y * n + x) * 4 + 4].copy_from_slice(&c);
        }
    }
    tray_icon::Icon::from_rgba(px, n as u32, n as u32).expect("icon")
}

fn run_tray(mut cfg: Config, use_device: bool) -> i32 {
    use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    // One companion at a time.
    let name: Vec<u16> = "Local\\CoolerScreenCompanion\0".encode_utf16().collect();
    let m = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if m.is_null() || unsafe { windows_sys::Win32::Foundation::GetLastError() } == 183 {
        eprintln!("The CoolerScreen companion is already running (look for its icon in the tray).");
        return 1;
    }
    // Started by double click: drop the console window (the tray icon is the interface). From a terminal it stays.
    let mut list = [0u32; 2];
    if unsafe { GetConsoleProcessList(list.as_mut_ptr(), 2) } <= 1 {
        unsafe { FreeConsole() };
    }
    MAIN_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);
    unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
    make_session_window();

    let pipeline = Arc::new(Pipeline::start(&cfg, if use_device { Sink::Device } else { Sink::Null }));
    let _ = PIPELINE.set(pipeline.clone());

    let menu = Menu::new();
    let pause = MenuItem::new("Pause (screen shows its normal clip)", true, None);
    let f30 = CheckMenuItem::new("30 frames per second", true, cfg.fps < 45, None);
    let f60 = CheckMenuItem::new("60 frames per second", true, cfg.fps >= 45, None);
    let quit = MenuItem::new("Quit", true, None);
    let _ = menu.append_items(&[&pause, &PredefinedMenuItem::separator(), &f30, &f60, &PredefinedMenuItem::separator(), &quit]);
    let _tray = tray_icon::TrayIconBuilder::new().with_menu(Box::new(menu)).with_tooltip("CoolerScreen companion").with_icon(tray_icon_image()).build();
    if _tray.is_err() {
        eprintln!("could not create the tray icon");
    }

    let rx = MenuEvent::receiver();
    let mut msg: MSG = unsafe { std::mem::zeroed() };
    while !QUIT.load(Ordering::SeqCst) && unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        while let Ok(ev) = rx.try_recv() {
            if ev.id == *quit.id() {
                QUIT.store(true, Ordering::SeqCst);
            } else if ev.id == *pause.id() {
                let now = !pipeline.shared.paused.load(Ordering::Relaxed);
                pipeline.shared.paused.store(now, Ordering::Relaxed);
                pause.set_text(if now { "Resume" } else { "Pause (screen shows its normal clip)" });
            } else if ev.id == *f30.id() || ev.id == *f60.id() {
                let fps = if ev.id == *f30.id() { 30 } else { 60 };
                f30.set_checked(fps == 30);
                f60.set_checked(fps == 60);
                pipeline.shared.fps.store(fps, Ordering::Relaxed);
                cfg.fps = fps;
                cfg.save();
            }
        }
    }
    stop_now();
    0
}
