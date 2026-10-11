//! cs2: hardware test CLI for the Rust device layer.
//!   cs2 info                      check the screen (auto-restarts once if it does not answer)
//!   cs2 state                     what is reachable, without sending anything
//!   cs2 upload <file> [--intro]   upload an MJPEG .mp4 to the loop slot (0x13; --intro: 0x12), then restart and check
//!   cs2 restart                   restart the screen and wait for it to come back
//!   cs2 poll <count> <seconds>    info every <seconds>, <count> times
use cs_core::device::{self, ops, Slot};
use std::time::Instant;

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
        _ => {
            eprintln!("usage: cs2 info | state | restart | upload <file.mp4> [--intro] | poll <count> <seconds>");
            2
        }
    };
    std::process::exit(code);
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
