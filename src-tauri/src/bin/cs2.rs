//! cs2: hardware test CLI for the Rust device layer.
//!   cs2 info                      check the screen (auto-restarts once if it does not answer)
//!   cs2 state                     what is reachable, without sending anything
//!   cs2 upload <file> [--intro]   upload an MJPEG .mp4 to the loop slot (0x13; --intro: 0x12), then restart and check
//!   cs2 restart                   restart the screen and wait for it to come back
//!   cs2 poll <count> <seconds>    info every <seconds>, <count> times
//!   cs2 edge-tune [--dry-run]     temporary: tune the visible area of each screen edge (bin/cs2/edgetune.rs, tune-edges.cmd)
//!   cs2 live-test[--frames N | --seconds S --fps F] [--orient landscape|portrait] [--scale 0.7] [--quality 100] [--set-sleep-delay] [--set-brightness] [--dry-run]   3.0 live-mode spike (bin/cs2/live.rs)
use cs_core::device::{self, ops, Slot};
use std::time::Instant;

#[path = "cs2/trial.rs"]
mod trial;
#[path = "cs2/live.rs"]
mod live;
#[path = "cs2/recovery.rs"]
mod recovery;
#[path = "cs2/edgetune.rs"]
mod edgetune;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        Some("info") => run(|| ops::check_screen(&mut prog).map(|v| v.to_string())),
        Some("check") if args.len() >= 2 => {
            // MJPEG guard only (never touches the screen)
            for f in &args[1..] {
                let ok = std::fs::read(f).map(|d| device::mjpeg::is_mjpeg_mp4(&d)).unwrap_or(false);
                println!("{ok}\t{f}");
            }
            0
        }
        Some("state") => run(|| Ok(serde_json::to_string(&ops::state()).unwrap())),
        Some("restart") => run(|| ops::restart_screen(&mut prog).map(|v| v.to_string())),
        Some("upload") if args.len() >= 2 => {
            let slot = if args.iter().any(|a| a == "--intro") { Slot::Startup } else { Slot::Standby };
            let file = std::path::PathBuf::from(&args[1]);
            run(|| {
                let t0 = Instant::now();
                let mut last_pct = -1i64;
                let mut stream_t0: Option<Instant> = None;
                let r = ops::send_files(&[(file.clone(), slot)], slot == Slot::Standby, &mut |p, text, a, b| {
                    if b > 0 {
                        let st = *stream_t0.get_or_insert_with(Instant::now);
                        let pct = (a * 100 / b) as i64;
                        if pct / 5 != last_pct / 5 || a == b {
                            last_pct = pct;
                            let s = st.elapsed().as_secs_f64().max(0.001);
                            eprintln!("  {pct:3}%  {a}/{b} bytes  {:.1} KB/s  {:.0}s", a as f64 / 1024.0 / s, s);
                        }
                    } else {
                        eprintln!("[{:5.1}s] {:3.0}% {text}", t0.elapsed().as_secs_f64(), p * 100.0);
                    }
                })?;
                eprintln!("total {:.1}s", t0.elapsed().as_secs_f64());
                Ok(serde_json::to_string_pretty(&r).unwrap())
            })
        }
        Some("poll") if args.len() >= 3 => {
            let n: u32 = args[1].parse().unwrap_or(10);
            let secs: u64 = args[2].parse().unwrap_or(30);
            let mut fails = 0;
            for k in 1..=n {
                let t = Instant::now();
                match ops::check_once() {
                    Ok(v) => println!(
                        "{} poll {k}/{n} ok in {} ms: free={} display_on={}",
                        device::log::iso_now(),
                        t.elapsed().as_millis(),
                        v["data"]["i_block_free"],
                        v["data"]["diplay_on"]
                    ),
                    Err(e) => {
                        fails += 1;
                        println!("{} poll {k}/{n} FAILED: {e}", device::log::iso_now())
                    }
                }
                if k < n {
                    std::thread::sleep(std::time::Duration::from_secs(secs));
                }
            }
            if fails == 0 { 0 } else { 1 }
        }
        // The same paths through the 2.0 core (scheduler + device service), for hardware checks.
        Some("core-check") => core_run(|app| app.device_check()),
        Some("core-state") => core_run(|app| app.device_state()),
        Some("core-restart") => core_run(|app| {
            let r = app.device_restart();
            wait_job(app, r["jobId"].as_str().unwrap_or(""))
        }),
        Some("core-send") if args.len() >= 2 => core_send(std::path::Path::new(&args[1])),
        Some("live-test") => live::live_test(&args[1..]),
        Some("wake") => live::wake(),
        Some("edge-tune") => edgetune::edge_tune(&args[1..]),
        Some("trial") => trial::trial(&args[1..]),
        Some("unstick") => recovery::unstick(),
        Some("recover-race-delete") => recovery::race_delete(),
        Some("delete-ota") => recovery::delete_ota(),
        _ => {
            eprintln!("usage: cs2 info | state | restart | upload <file.mp4> [--intro] | poll <count> <seconds> | live-test [--frames N | --seconds S --fps F] [--orient landscape|portrait] [--scale 0.7] [--quality 100] [--set-sleep-delay] [--set-brightness] [--dry-run]");
            eprintln!("       cs2 trial [--dry-run] [--no-poll] | wake   (guided live-mode trials; see test-live.cmd)");
            eprintln!("       cs2 unstick | recover-race-delete | delete-ota   (recovery tools, see tools/README.md)");
            eprintln!("       cs2 core-check | core-state | core-restart [--real] | core-send <library>/<oid>/versions/<vid>  (COOLERSCREEN_HOME = scratch folder, or --real)");
            2
        }
    };
    std::process::exit(code);
}

fn core_app() -> std::sync::Arc<cs_core::core::App> {
    struct Log;
    impl cs_core::core::Events for Log {
        fn job(&self, j: &cs_core::core::scheduler::Job) {
            eprintln!("  job {} {} {:3.0}% {}", j.kind, j.status, j.progress * 100.0, j.message);
        }
    }
    cs_core::core::App::new(cs_core::core::paths::base(), cs_core::backends_with_work_dir(std::env::temp_dir().join("CoolerScreen-media-cs2")), std::sync::Arc::new(Log), env!("CARGO_PKG_VERSION"))
}

fn core_run(f: impl FnOnce(&cs_core::core::App) -> Result<serde_json::Value, cs_core::core::CoreError>) -> i32 {
    // The core writes settings (capacity, slot records): never the real app folder by accident.
    if std::env::var_os("COOLERSCREEN_HOME").is_none() && !std::env::args().any(|a| a == "--real") {
        eprintln!("core-* commands use an app folder: set COOLERSCREEN_HOME to a scratch folder, or pass --real to use the real one (%APPDATA%\\CoolerScreen).");
        return 2;
    }
    let t = Instant::now();
    let app = core_app();
    match f(&app) {
        Ok(v) => {
            println!("{}", serde_json::to_string_pretty(&v).unwrap());
            eprintln!("ok in {} ms", t.elapsed().as_millis());
            0
        }
        Err(e) => {
            eprintln!("ERROR {e} ({} ms)", t.elapsed().as_millis());
            1
        }
    }
}

fn wait_job(app: &cs_core::core::App, id: &str) -> Result<serde_json::Value, cs_core::core::CoreError> {
    loop {
        let j = app.job_get(id)?;
        match j["state"].as_str() {
            Some("done") => return Ok(j),
            Some("failed") | Some("cancelled") => {
                return Err(cs_core::core::CoreError::new(format!("job {}: {}", j["state"], j["error"])))
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(200)),
        }
    }
}

/// Send an existing version through the core's send flow, from a scratch app folder (COOLERSCREEN_HOME) holding
/// copies of its original.json and version folder, so the real library and settings are never written.
fn core_send(vdir: &std::path::Path) -> i32 {
    let Some(home) = std::env::var_os("COOLERSCREEN_HOME") else {
        eprintln!("Set COOLERSCREEN_HOME to a scratch folder first (core-send never writes the real library).");
        return 2;
    };
    let vid = vdir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let odir = vdir.parent().and_then(|p| p.parent()).expect("<oid>/versions/<vid>");
    let oid = odir.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
    let lib = std::path::PathBuf::from(home).join("library");
    let to = lib.join(&oid).join("versions").join(&vid);
    std::fs::create_dir_all(&to).unwrap();
    std::fs::copy(odir.join("original.json"), lib.join(&oid).join("original.json")).unwrap();
    for f in ["version.json", "device.mp4", "thumb.jpg"] {
        if vdir.join(f).exists() {
            std::fs::copy(vdir.join(f), to.join(f)).unwrap();
        }
    }
    core_run(|app| {
        let r = app.version_send(&vid)?;
        let j = wait_job(app, r["jobId"].as_str().unwrap_or(""))?;
        Ok(serde_json::json!({ "job": j, "slots": app.settings.load().0["slots"], "onScreen": app.app_state()["onScreen"] }))
    })
}

fn prog(p: f64, text: &str) {
    eprintln!("  {:3.0}% {text}", p * 100.0);
}

fn run(f: impl FnOnce() -> Result<String, device::DevErr>) -> i32 {
    let t = Instant::now();
    match f() {
        Ok(s) => {
            println!("{s}");
            eprintln!("ok in {} ms", t.elapsed().as_millis());
            0
        }
        Err(e) => {
            eprintln!("ERROR {e} ({} ms)", t.elapsed().as_millis());
            1
        }
    }
}
