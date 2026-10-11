//! Transport selection, restart-and-wait and the send flow (port of src/deviceOps.js).
//! Handles are opened only for the duration of a call. Prefers WinUSB; falls back to the COM port.
use super::{friendly, log, serial, DevErr, Screen, Slot, UploadOutcome, MSG_FAILED_USB, SERIAL_WARNING};
use serde::Serialize;
use serde_json::{json, Value};
use std::thread::sleep;
use std::time::{Duration, Instant};

pub fn open_any() -> Result<Screen, DevErr> {
    let usb_err = match Screen::open_usb() {
        Ok(s) => return Ok(s),
        Err(e) => e,
    };
    if let Some(path) = serial::find_port() {
        return Screen::open_serial(&path).map_err(friendly);
    }
    let claim = usb_err.message.contains("claim");
    if !claim && failed_usb_present() {
        return Err(DevErr::coded("failedUsb", MSG_FAILED_USB));
    }
    Err(friendly(if claim { usb_err } else { DevErr::new("Screen not found") }))
}

/// Open with retries (the screen can restart and re-enumerate after an upload).
pub fn open_retry(seconds: u64) -> Result<Screen, DevErr> {
    let end = Instant::now() + Duration::from_secs(seconds);
    loop {
        match open_any() {
            Ok(s) => return Ok(s),
            Err(e) if Instant::now() > end => return Err(e),
            Err(_) => sleep(Duration::from_secs(1)),
        }
    }
}

/// Windows sometimes fails to enumerate the screen at boot: "Unknown USB Device (Device Descriptor Request Failed)".
pub fn failed_usb_present() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let cmd = "@(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\\\VID_0000&PID_000' }).Count";
        let child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", cmd])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return false };
        let end = Instant::now() + Duration::from_secs(15);
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < end => sleep(Duration::from_millis(100)),
                _ => {
                    let _ = child.kill();
                    return false;
                }
            }
        }
        let mut out = String::new();
        if let Some(mut s) = child.stdout.take() {
            use std::io::Read;
            let _ = s.read_to_string(&mut out);
        }
        out.trim().parse::<i64>().map(|n| n > 0).unwrap_or(false)
    }
    #[cfg(not(windows))]
    false
}

fn with_transport(info: Value, transport: &str) -> Value {
    let mut v = match info {
        Value::Object(m) => Value::Object(m),
        other => json!({ "reply": other }),
    };
    let serial = transport == "serial";
    v["_transport"] = json!(transport);
    v["_needsSetup"] = json!(serial);
    if serial {
        v["_warning"] = json!(SERIAL_WARNING);
    }
    v
}

pub fn check_once() -> Result<Value, DevErr> {
    let mut s = open_any()?;
    let t = s.transport;
    let r = s.info().map_err(friendly);
    s.close();
    r.map(|i| with_transport(i, t))
}

/// Restart the screen (0x01), wait for it to re-enumerate (up to 25 s) and check that it answers.
/// Never call this while file data is being streamed.
pub fn restart_screen(progress: &mut dyn FnMut(f64, &str)) -> Result<Value, DevErr> {
    progress(0.1, "Restarting the screen");
    {
        let mut s = open_retry(25)?; // right after an upload it may still be re-enumerating
        let r = s.send_restart();
        s.close();
        r?;
    }
    sleep(Duration::from_millis(3000)); // it drops off USB while rebooting
    progress(0.4, "Waiting for the screen to come back");
    let end = Instant::now() + Duration::from_secs(25);
    let mut last: Option<DevErr> = None;
    while Instant::now() < end {
        match check_once() {
            Ok(info) => {
                progress(1.0, "Done");
                return Ok(info);
            }
            Err(e) => {
                let f = e.failed_usb;
                last = Some(e);
                if f {
                    break;
                }
                sleep(Duration::from_millis(1500));
            }
        }
    }
    if let Some(e) = &last {
        if e.failed_usb {
            return Err(e.clone());
        }
    }
    if failed_usb_present() {
        return Err(DevErr::coded("failedUsb", "The screen restarted but Windows could not connect to it again. Click Reconnect (needs one administrator approval)."));
    }
    Err(friendly(last.unwrap_or_else(|| DevErr::new("Screen not found"))))
}

/// Check the screen; if it doesn't answer, restart it once and check again (result has _restarted: true).
pub fn check_screen(progress: &mut dyn FnMut(f64, &str)) -> Result<Value, DevErr> {
    match check_once() {
        Ok(v) => Ok(v),
        Err(e) if e.code.as_deref() == Some("noReply") => {
            let mut info = restart_screen(progress)?;
            info["_restarted"] = json!(true);
            Ok(info)
        }
        Err(e) => Err(e),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SendResult {
    pub ok: bool,
    pub warning: Option<String>,
    pub transport: String,
    pub restarted: bool,
    #[serde(rename = "restartFailed")]
    pub restart_failed: Option<DevErr>,
    #[serde(rename = "finalInfo")]
    pub final_info: Option<Value>,
    pub uploads: Vec<UploadOutcome>,
}

/// items uploaded one after another; with play_loop the send ends with a restart (0x01) and a final check
/// (hardware 2026-10-11: after 0x14 the screen later stopped answering; after a restart it stayed responsive).
/// progress(0..1 overall, text, bytes sent of current file, file size)
pub fn send_files(
    items: &[(std::path::PathBuf, Slot)],
    play_loop: bool,
    progress: &mut dyn FnMut(f64, &str, u64, u64),
) -> Result<SendResult, DevErr> {
    // Read and check every file before touching the screen.
    let mut datas = Vec::new();
    for (f, _) in items {
        let d = std::fs::read(f).map_err(|e| DevErr::new(format!("Cannot read {}: {e}", f.display())))?;
        if !super::mjpeg::is_mjpeg_mp4(&d) {
            return Err(DevErr::new("Refusing to upload: the screen can only play MJPEG .mp4 clips made by CoolerScreen."));
        }
        datas.push(d);
    }
    let total = items.len() as f64 + if play_loop { 0.1 } else { 0.0 };
    let (mut warning, mut transport, mut restarted, mut restart_failed, mut final_info) =
        (None, String::new(), false, None, None);
    let mut uploads = Vec::new();
    let mut i = 0usize;
    while i < items.len() {
        let slot = items[i].1;
        let label = if slot == Slot::Standby { "loop" } else { "intro" };
        progress(i as f64 / total, if i == 0 { "Connecting to screen" } else { "Waiting for screen to come back" }, 0, 0);
        let mut s = if i == 0 && !restarted { open_any()? } else { open_retry(25)? };
        transport = s.transport.to_string();
        let mut streaming = false;
        let mut sent_all = false;
        let text = format!("Uploading {label} clip (the screen writes slowly: ~3 min for 9 MB)");
        let r = s.upload(&datas[i], slot, &mut |a, b| {
            streaming = true;
            if a >= b {
                sent_all = true;
            }
            progress((i as f64 + a as f64 / b as f64) / total, &text, a, b);
        });
        s.close();
        match r {
            Ok(o) => uploads.push(o),
            Err(e) if sent_all => warning = Some(e.message), // the screen may reset right after the last byte
            Err(e) => {
                let f = friendly(e);
                if f.code.as_deref() == Some("noReply") && !streaming && !restarted {
                    // The upload command got no reply (no file data sent yet): restart once and retry this file.
                    progress(i as f64 / total, "The screen did not answer: restarting it", 0, 0);
                    restart_screen(&mut |_, _| {})?;
                    restarted = true;
                    continue;
                }
                return Err(f);
            }
        }
        if i < items.len() - 1 || play_loop {
            sleep(Duration::from_millis(2500));
        }
        i += 1;
    }
    if play_loop {
        progress(items.len() as f64 / total, "Restarting the screen to show the new clip", 0, 0);
        match restart_screen(&mut |_, _| {}) {
            Ok(info) => {
                if let Some(t) = info["_transport"].as_str() {
                    transport = t.to_string();
                }
                final_info = Some(info);
            }
            Err(e) => restart_failed = Some(e),
        }
    }
    log(&format!("send finished ({} file(s), transport {transport})", items.len()));
    Ok(SendResult { ok: true, warning, transport, restarted, restart_failed, final_info, uploads })
}

#[derive(Debug, Clone, Serialize)]
pub struct State {
    pub transport: &'static str,
    #[serde(rename = "needsSetup")]
    pub needs_setup: bool,
    pub warning: Option<&'static str>,
    #[serde(rename = "failedUsb")]
    pub failed_usb: bool,
}

/// What is reachable right now, without sending anything to the screen.
pub fn state() -> State {
    if let Ok(s) = Screen::open_usb() {
        s.close();
        return State { transport: "winusb", needs_setup: false, warning: None, failed_usb: false };
    }
    if serial::find_port().is_some() {
        return State { transport: "serial", needs_setup: true, warning: Some(SERIAL_WARNING), failed_usb: false };
    }
    State { transport: "none", needs_setup: false, warning: None, failed_usb: failed_usb_present() }
}
