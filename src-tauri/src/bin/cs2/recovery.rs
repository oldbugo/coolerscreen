//! Recovery tools for a screen that an interrupted or bad upload left stuck (ports of tools/*.js from 1.2).
//! Close CoolerScreen first: only one program can talk to the screen at a time.
use cs_core::device::ops;
use cs_core::device::{DevErr, Screen};
use std::thread::sleep;
use std::time::{Duration, Instant};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn fail(e: impl std::fmt::Display) -> i32 {
    eprintln!("ERROR {e}");
    1
}

/// Finish an interrupted upload: the screen keeps counting bytes until it has the size it was told, so feed zero
/// filler until it acknowledges (a 0x12 / 0x13 reply), then report.
pub fn unstick() -> i32 {
    let mut s = match ops::open_any() {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let zero = vec![0u8; 40 * 1024];
    let mut sent = 0usize;
    let t0 = Instant::now();
    let mut acked = None;
    while acked.is_none() && sent < 10 * 1024 * 1024 {
        if let Err(e) = s.recovery_raw(&zero) {
            eprintln!("write failed after {sent} bytes: {e}");
            s.close();
            return 1;
        }
        sent += zero.len();
        if sent % (1024 * 1024) < zero.len() {
            println!("{:.1} MB filler, {:.0} s", sent as f64 / 1_048_576.0, t0.elapsed().as_secs_f64());
        }
        acked = s.recovery_poll(ms(5));
    }
    if acked.is_none() {
        acked = s.recovery_poll(ms(3000));
    }
    println!("ack: {} after {sent} bytes", acked.map(|v| v.to_string()).unwrap_or_else(|| "none".into()));
    s.close();
    0
}

fn open_usb_quiet() -> Option<Screen> {
    Screen::open_usb().ok()
}

/// Restart the screen, then send "delete /data/video/sleep_video.mp4" the moment it is reachable again, before the
/// player opens the unplayable clip (an open file cannot be deleted).
pub fn race_delete() -> i32 {
    let mut s = match ops::open_any() {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let _ = s.recovery_restart_noreply();
    s.close();
    let t0 = Instant::now();
    // wait for it to drop off USB
    while open_usb_quiet().map(|s| s.close()).is_some() && t0.elapsed() < Duration::from_secs(5) {
        sleep(ms(20));
    }
    while t0.elapsed() < Duration::from_secs(30) {
        let Some(mut s) = open_usb_quiet() else {
            sleep(ms(30));
            continue;
        };
        let r = s.recovery_cmd(0x05, b"/data/video/sleep_video.mp4", ms(1500));
        s.close();
        println!("{} ms: delete reply {}", t0.elapsed().as_millis(), r.as_ref().map(|v| v.to_string()).unwrap_or_else(|e| format!("error: {e}")));
        if let Ok(v) = &r {
            if v["key"] == 5 && v["hex"] == "00" {
                println!("Deleted. The screen should now be usable. Send a clip with CoolerScreen.");
                return 0;
            }
        }
        sleep(ms(100));
    }
    println!("Did not get a successful delete within 30 s. Copy what was printed into a bug report.");
    1
}

fn dir_list(s: &mut Screen, path: &str) -> Result<Vec<String>, DevErr> {
    let v = s.recovery_cmd(0x07, path.as_bytes(), ms(5000))?;
    Ok(v["data"]["dir_list"]
        .as_array()
        .map(|a| a.iter().filter_map(|f| f["filename"].as_str().map(String::from)).collect())
        .unwrap_or_default())
}

fn free_bytes(s: &mut Screen) -> Result<f64, DevErr> {
    Ok(s.info()?["data"]["i_block_free"].as_f64().unwrap_or(0.0) * 512.0)
}

/// Deletes the leftover firmware-update package /data/ota.cpio (about 0.68 MB). Only this exact path is touched.
/// Keep a copy of the vendor's APP_OTA_*.bin first (it is the same file).
pub fn delete_ota() -> i32 {
    let mut s = match ops::open_any() {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let r = (|| -> Result<(), DevErr> {
        let before = free_bytes(&mut s)?;
        if !dir_list(&mut s, "/data")?.iter().any(|f| f == "ota.cpio") {
            println!("ota.cpio is not on the screen; nothing to do.");
            return Ok(());
        }
        let r = s.recovery_cmd(0x05, b"/data/ota.cpio", ms(8000));
        println!("delete reply: {}", r.map(|v| v.to_string()).unwrap_or_else(|e| format!("error: {e}")));
        let after = free_bytes(&mut s)?;
        println!("files now in /data: {}", dir_list(&mut s, "/data")?.join(", "));
        println!("free space: {:.2} MB -> {:.2} MB", before / 1_048_576.0, after / 1_048_576.0);
        Ok(())
    })();
    s.close();
    match r {
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}
