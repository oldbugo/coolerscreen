//! Transport selection, restart-and-wait and the send flow (port of src/deviceOps.js).
//! Handles are opened only for the duration of a call. Prefers WinUSB; falls back to the COM port.
use super::{friendly, log, serial, DevErr, Screen, Slot, UploadOutcome, MSG_FAILED_USB, SERIAL_WARNING};
use serde::Serialize;
use serde_json::{json, Value};
use std::thread::sleep;
use std::time::{Duration, Instant};

const USB_PORT_MATCH: &str = r#"
function Test-CoolerUsbPort($Parent, $Location, $History) {
  [bool]($Parent -and $Location -and @($History | Where-Object { $_.Parent -eq $Parent -and $_.Location -eq $Location }).Count -gt 0)
}
"#;

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
    Err(friendly(usb_err))
}

/// Live failures must not block ownership release on a slow PowerShell process. Probe at most once per
/// 30 seconds in the background. A missing device is reported as notFound until the first result arrives.
pub fn failed_usb_cached() -> bool {
    #[cfg(test)]
    { return false; }
    #[cfg(not(test))]
    {
        use std::sync::{Arc, Mutex, OnceLock};
        static CACHE: OnceLock<Arc<Mutex<(Option<Instant>, bool)>>> = OnceLock::new();
        let cache = CACHE.get_or_init(|| Arc::new(Mutex::new((None, false))));
        let mut g = cache.lock().unwrap_or_else(|p| p.into_inner());
        if g.0.map_or(true, |t| t.elapsed() >= Duration::from_secs(30)) {
            g.0 = Some(Instant::now());
            let c = cache.clone();
            std::thread::spawn(move || {
                let found = failed_usb_present();
                c.lock().unwrap_or_else(|p| p.into_inner()).1 = found;
            });
        }
        g.1
    }
}

pub fn live_open_error(e: DevErr) -> DevErr {
    let e = friendly(e);
    if e.code.as_deref() == Some("notFound") && failed_usb_cached() {
        DevErr { code: Some("failedUsb".into()), message: MSG_FAILED_USB.into(), failed_usb: true }
    } else { e }
}

/// Enumeration only: never opens or claims the screen. Transport is a driver hint, not a health check.
pub fn presence() -> Value {
    if serial::find_port().is_some() {
        json!({ "present": true, "transport": "serial", "needsSetup": true })
    } else if super::usb::find().is_some() {
        json!({ "present": true, "transport": "winusb", "needsSetup": false })
    } else { json!({ "present": false, "transport": "none", "needsSetup": false }) }
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
        // A failed descriptor has lost its VID/PID. Only attribute it to the screen when BOTH parent
        // and port location match the SAME remembered screen node (also the reconnect script's rule).
        // An unrelated broken USB device must not become a CoolerScreen diagnosis.
        let cmd = format!("{USB_PORT_MATCH}\n{}", r#"
function Prop($id, $key) { try { (Get-PnpDeviceProperty -InstanceId $id -KeyName $key -ErrorAction Stop).Data } catch { $null } }
$all = @(Get-PnpDevice -ErrorAction SilentlyContinue)
$history = @($all | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)\\' -and -not $_.Present } | ForEach-Object {
  [pscustomobject]@{ Parent = Prop $_.InstanceId 'DEVPKEY_Device_Parent'; Location = Prop $_.InstanceId 'DEVPKEY_Device_LocationInfo' }
})
$matchedPorts = @($all | Where-Object { $_.Present -and $_.InstanceId -match '^USB\\VID_0000&PID_000[0-9A-F]\\' } | Where-Object {
  $parent = Prop $_.InstanceId 'DEVPKEY_Device_Parent'; $location = Prop $_.InstanceId 'DEVPKEY_Device_LocationInfo'
  Test-CoolerUsbPort $parent $location $history
})
$matchedPorts.Count
"#);
        let child = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &cmd])
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

/// The most the loop slot can hold according to the screen's own numbers (blocks of the data flash): about 92 % of it is usable
/// (hardware 2026-10-17: 24520 blocks of 512 B, 11.6 MB measured), minus the intro file. None when the info has no block counts.
pub fn loop_limit(info: &Value, intro_bytes: u64) -> Option<u64> {
    let d = info.get("data").filter(|d| d.is_object()).unwrap_or(info);
    let (blocks, bs) = (d.get("i_blocks")?.as_f64()?, d.get("i_block_size").and_then(Value::as_f64).unwrap_or(512.0));
    let usable = blocks * bs * 0.92;
    (usable > intro_bytes as f64).then(|| (usable - intro_bytes as f64) as u64)
}

/// Room for a new loop file from the screen's real state: the free bytes now plus the loop file it will replace (only when that
/// size is known for sure, else nothing), less a 256 KB margin. Without a free-block count: the flash-based `loop_limit`.
pub fn loop_room(info: &Value, known_loop: Option<u64>, intro_bytes: u64) -> Option<u64> {
    let d = info.get("data").filter(|d| d.is_object()).unwrap_or(info);
    match d.get("i_block_free").and_then(Value::as_f64) {
        Some(free) => {
            let bs = d.get("i_block_size").and_then(Value::as_f64).unwrap_or(512.0);
            Some(((free * bs) as u64 + known_loop.unwrap_or(0)).saturating_sub(LOOP_MARGIN))
        }
        None => loop_limit(info, intro_bytes),
    }
}

/// Kept free when a loop file is stored (the screen stores some bookkeeping of its own).
pub const LOOP_MARGIN: u64 = 256_000;

/// items uploaded one after another; with play_loop the send ends with a restart (0x01) and a final check
/// (hardware 2026-10-11: after 0x14 the screen later stopped answering; after a restart it stayed responsive).
/// progress(0..1 overall, text, bytes sent of current file, file size)
pub fn send_files(
    items: &[(std::path::PathBuf, Slot)],
    play_loop: bool,
    progress: &mut dyn FnMut(f64, &str, u64, u64),
) -> Result<SendResult, DevErr> {
    send_files_with(items, play_loop, None, progress)
}

/// `known_loop`: the size of the loop file the screen holds now, when that is certain (the slot record, not flagged incomplete).
/// The new loop file may use the free space plus that much (the old file is replaced), less a margin.
pub fn send_files_with(
    items: &[(std::path::PathBuf, Slot)],
    play_loop: bool,
    known_loop: Option<u64>,
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
        if slot == Slot::Standby {
            if let Ok(info) = s.info() {
                let other = items.iter().position(|(_, sl)| *sl == Slot::Startup).map(|k| datas[k].len() as u64).unwrap_or(560_000);
                if let Some(max) = loop_room(&info, known_loop, other) {
                    if datas[i].len() as u64 > max {
                        s.close();
                        return Err(DevErr::coded("noRoom", format!("This file is {:.2} MB but the screen can store at most about {:.2} MB for the loop clip. Nothing was written.", datas[i].len() as f64 / 1e6, max as f64 / 1e6)));
                    }
                }
            }
        }
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
            Ok(o) if super::ack_refused(&o.done) => {
                // The screen finished writing and said no ({"key":19,"hex":"01"} = it ran out of room): the stored file is cut short.
                return Err(DevErr::coded("noRoom", format!("The screen ran out of room while storing the file and answered {}. The clip on the screen is incomplete: send a smaller one.", o.done)));
            }
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

#[cfg(test)]
mod guard_tests {
    use super::*;

    #[test]
    #[cfg(windows)]
    fn failed_usb_diagnosis_matches_one_complete_remembered_port_not_unrelated_or_crossed_nodes() {
        use std::os::windows::process::CommandExt;
        let fixtures = r#"
$h = @([pscustomobject]@{Parent='hubA';Location='port1'},[pscustomobject]@{Parent='hubB';Location='port2'})
if (-not (Test-CoolerUsbPort 'hubA' 'port1' $h)) { throw 'exact match rejected' }
if (Test-CoolerUsbPort 'hubA' 'port2' $h) { throw 'crossed pair accepted' }
if (Test-CoolerUsbPort 'hubC' 'port1' $h) { throw 'unrelated hub accepted' }
if (Test-CoolerUsbPort 'hubA' '' $h) { throw 'missing location accepted' }
if (Test-CoolerUsbPort 'hubA' 'port1' @()) { throw 'no history accepted' }
'OK'
"#;
        let command = format!("{USB_PORT_MATCH}\n{fixtures}");
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &command]).creation_flags(0x0800_0000).output().unwrap();
        assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("OK"), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn the_loop_limit_follows_the_flash_and_refuses_what_broke_the_screen() {
        let info = json!({ "cmd": "info", "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 792 } });
        let max = loop_limit(&info, 537_054).unwrap();
        assert!(10_662_507 <= max, "the known good version fits: {max}");
        assert!(12_855_458 > max, "the version that filled the flash does not: {max}");
        assert!(loop_limit(&json!({ "data": {} }), 537_054).is_none());
    }

    #[test]
    fn the_room_for_a_loop_file_is_what_the_screen_has_free_plus_the_file_it_replaces() {
        // hardware 2026-10-17: 792 blocks free with the 10,662,507 byte loop file and the intro on the screen
        let info = json!({ "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 792 } });
        let room = loop_room(&info, Some(10_662_507), 537_054).unwrap();
        assert_eq!(room, 792 * 512 + 10_662_507 - LOOP_MARGIN);
        assert!(10_662_507 <= room, "replacing the loop with the same file fits");
        assert!(11_000_000 > room, "11.0 MB does not");
        // the size of the current file is not known for sure (incomplete record): nothing is counted back
        assert_eq!(loop_room(&info, None, 537_054).unwrap(), 792 * 512 - LOOP_MARGIN);
        // a full flash: nothing fits
        let full = json!({ "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 0 } });
        assert_eq!(loop_room(&full, None, 537_054), Some(0));
        // no free count: the flash estimate
        assert_eq!(loop_room(&json!({ "data": { "i_blocks": 24520, "i_block_size": 512 } }), None, 537_054), loop_limit(&json!({ "data": { "i_blocks": 24520, "i_block_size": 512 } }), 537_054));
    }

    #[test]
    fn a_no_from_the_screen_is_a_refusal() {
        assert!(crate::device::ack_refused(&json!({ "key": 19, "hex": "01" })));
        assert!(!crate::device::ack_refused(&json!({ "key": 19, "hex": "00" })));
        assert!(!crate::device::ack_refused(&json!({ "error": "No reply from screen" })));
    }
}
