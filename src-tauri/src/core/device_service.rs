//! The screen as the exclusive `device` resource: check (auto-restart once on "no reply"), restart, send
//! (splash to the intro slot if needed → loop → 0x01 restart → wait → info), measured capacity, one-time setup /
//! restore / reconnect through the existing elevated PowerShell scripts, failed-USB detection. Port of 1.2
//! `deviceOps.js`, `send.js`, `setup.js` on top of the Phase 0 Rust device layer (`crate::device`).
//!
//! Fake mode (`COOLERSCREEN_FAKE_DEVICE=1|winusb|serial|none|failedusb|busy|noreply|noreply-stuck`,
//! `COOLERSCREEN_FAKE_FAIL=1`) simulates a screen exactly like 1.2 and never touches USB, the COM port or
//! PowerShell. `COOLERSCREEN_FAKE_SPEED` (default 1) scales the simulated delays (tests use 0.01).
//!
//! Every method here blocks; callers run them inside scheduler tasks holding `Res::Device`.
use super::error::{CoreError, CoreResult};
use crate::device::{self, ops, DevErr, Slot, SERIAL_WARNING};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub type Progress<'a> = &'a mut dyn FnMut(f64, &str);
pub type SendProgress<'a> = &'a mut dyn FnMut(f64, &str, u64, u64);

#[derive(Debug)]
struct Fake {
    transport: String,
    restarted: bool,
    fail_upload: bool,
    speed: f64,
    /// Free blocks the fake screen reports (a test hook plays "another program wrote to the screen").
    free: u64,
}

pub struct DeviceService {
    fake: Option<Mutex<Fake>>,
    scripts: Mutex<PathBuf>,
    /// Folder for the elevated scripts' logs (the app folder, so an administrator approving the prompt from another
    /// account still writes where the app reads; %TEMP% would be the administrator's own).
    logs_dir: PathBuf,
    last_log: Mutex<Option<PathBuf>>,
}

fn friendly_fake(code: &str) -> CoreError {
    if code == "busy" {
        return super::holder::busy_error("fake");
    }
    let m = match code {
        "notFound" => "Screen not found. Check the USB cable and that the cooler is powered.",
        "failedUsb" => "Windows could not connect to the screen at startup (a known quirk). Click Reconnect (needs one administrator approval).",
        "busy" => "Could not open the screen: another program may be using it.",
        "noReply" => "The screen is not responding (it keeps playing, but doesn't answer). Restart the screen to bring it back.",
        _ => "Screen error",
    };
    CoreError::coded(code, m).status(502)
}


/// The PowerShell command that runs `script` (elevated: through a UAC prompt) with `-Log <log>` and `extra` switches.
/// Inner double quotes keep paths with spaces intact through Start-Process's argument joining.
pub fn script_command(script: &Path, log: &Path, extra: &[&str], elevated: bool) -> String {
    let q = |p: &Path| p.display().to_string().replace('\'', "''");
    let extra: String = extra.iter().map(|a| format!(",'{}'", a.replace('\'', "''"))).collect();
    format!(
        "Start-Process powershell{} -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','\"{}\"','-Log','\"{}\"'{extra}",
        if elevated { " -Verb RunAs" } else { "" },
        q(script),
        q(log)
    )
}

impl DeviceService {
    /// Real device unless COOLERSCREEN_FAKE_DEVICE is set.
    pub fn from_env(base: &Path) -> Self {
        let mut d = match std::env::var("COOLERSCREEN_FAKE_DEVICE") {
            Ok(v) if !v.is_empty() => {
                let speed = std::env::var("COOLERSCREEN_FAKE_SPEED").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0);
                Self::fake(&v, speed, std::env::var_os("COOLERSCREEN_FAKE_FAIL").is_some())
            }
            _ => DeviceService { fake: None, ..Self::fake("winusb", 1.0, false) },
        };
        d.logs_dir = base.join("logs");
        d
    }

    /// A new log file for one run of an elevated script (old ones beyond the last 10 are removed).
    fn new_setup_log(&self, script: &str) -> PathBuf {
        let _ = std::fs::create_dir_all(&self.logs_dir);
        if let Ok(rd) = std::fs::read_dir(&self.logs_dir) {
            let mut old: Vec<PathBuf> =
                rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("setup-"))).collect();
            old.sort();
            let n = old.len().saturating_sub(9);
            for p in &old[..n] {
                let _ = std::fs::remove_file(p);
            }
        }
        let stem = script.trim_end_matches(".ps1");
        let f = self.logs_dir.join(format!("setup-{}-{stem}.log", super::scheduler::now_ms()));
        *self.last_log.lock().unwrap() = Some(f.clone());
        f
    }

    /// The log of the last elevated script run (for messages).
    pub fn last_setup_log(&self) -> String {
        self.last_log.lock().unwrap().as_ref().map(|p| p.display().to_string()).unwrap_or_default()
    }

    pub fn fake(mode: &str, speed: f64, fail_upload: bool) -> Self {
        let t = if mode == "1" { "winusb".to_string() } else { mode.to_lowercase() };
        DeviceService {
            fake: Some(Mutex::new(Fake { transport: t, restarted: false, fail_upload, speed, free: 4880 })),
            scripts: Mutex::new(super::paths::scripts_dir()),
            logs_dir: std::env::temp_dir().join("CoolerScreen-logs"),
            last_log: Mutex::new(None),
        }
    }

    /// Where scripts/*.ps1 live (the app's bundled resources folder in an installed build).
    pub fn set_scripts_dir(&self, dir: PathBuf) {
        if dir.join("setup-winusb.ps1").exists() {
            *self.scripts.lock().unwrap() = dir;
        }
    }

    pub fn scripts_dir(&self) -> PathBuf {
        self.scripts.lock().unwrap().clone()
    }

    /// Test hook: the free blocks the fake screen reports from now on.
    pub fn fake_set_free(&self, blocks: u64) {
        if let Some(f) = &self.fake {
            f.lock().unwrap().free = blocks;
        }
    }

    /// The fake mode name ("winusb", ...) or None for the real screen (AppState.fake).
    pub fn fake_mode(&self) -> Option<String> {
        self.fake.as_ref().map(|f| f.lock().unwrap().transport.clone())
    }

    fn fsleep(&self, ms: u64) {
        let speed = self.fake.as_ref().map(|f| f.lock().unwrap().speed).unwrap_or(1.0);
        std::thread::sleep(Duration::from_millis((ms as f64 * speed) as u64));
    }

    fn fake_error(&self) -> Option<CoreError> {
        let f = self.fake.as_ref()?.lock().unwrap();
        match f.transport.as_str() {
            "none" => Some(friendly_fake("notFound")),
            "failedusb" => Some(friendly_fake("failedUsb")),
            "busy" => Some(friendly_fake("busy")),
            t if t.starts_with("noreply") && !f.restarted => Some(friendly_fake("noReply")),
            _ => None,
        }
    }

    /// What is reachable right now, without sending anything to the screen.
    pub fn state(&self) -> Value {
        if let Some(f) = &self.fake {
            let t = f.lock().unwrap().transport.clone();
            return if t == "winusb" || t.starts_with("noreply") {
                json!({ "transport": "winusb", "needsSetup": false, "fake": true })
            } else if t == "serial" {
                json!({ "transport": "serial", "needsSetup": true, "warning": SERIAL_WARNING, "fake": true })
            } else {
                json!({ "transport": "none", "needsSetup": false, "failedUsb": t == "failedusb", "fake": true })
            };
        }
        let s = ops::state();
        let mut v = json!({ "transport": s.transport, "needsSetup": s.needs_setup, "failedUsb": s.failed_usb });
        if let Some(w) = s.warning {
            v["warning"] = json!(w);
        }
        v
    }

    /// One read-only info answer, never restarting the screen (the skip-identical-send check: when this fails the caller just sends).
    pub fn read_info(&self) -> CoreResult<Value> {
        self.check_once()
    }

    fn check_once(&self) -> CoreResult<Value> {
        if self.fake.is_some() {
            self.fsleep(600);
            if let Some(e) = self.fake_error() {
                return Err(e);
            }
            let serial = self.fake.as_ref().unwrap().lock().unwrap().transport == "serial";
            let t = if serial { "serial" } else { "winusb" };
            let mut v = json!({ "model": "D215-NOR-JD9365-6.67inch-hor (fake)", "version": "3.6", "i_blocks": 24520, "i_block_size": 512,
                "i_block_free": self.fake.as_ref().unwrap().lock().unwrap().free, "_transport": t, "_needsSetup": serial, "_fake": true });
            if serial {
                v["_warning"] = json!(SERIAL_WARNING);
            }
            return Ok(v);
        }
        ops::check_once().map_err(CoreError::from)
    }

    /// Restart (0x01), wait for the screen to come back and check it answers.
    pub fn restart(&self, progress: Progress) -> CoreResult<Value> {
        if let Some(f) = &self.fake {
            progress(0.1, "Restarting the screen");
            self.fsleep(1500);
            {
                let mut f = f.lock().unwrap();
                if f.transport == "noreply-stuck" {
                    f.transport = "failedusb".into();
                } else {
                    f.restarted = true;
                }
            }
            progress(0.4, "Waiting for the screen to come back");
            let end = Instant::now() + Duration::from_millis((25_000.0 * self.speed()) as u64);
            let mut last = None;
            while Instant::now() < end {
                match self.check_once() {
                    Ok(i) => {
                        progress(1.0, "Done");
                        return Ok(i);
                    }
                    Err(e) => {
                        let stop = e.failed_usb;
                        last = Some(e);
                        if stop {
                            break;
                        }
                        self.fsleep(1500);
                    }
                }
            }
            return Err(match last {
                Some(e) if e.failed_usb => CoreError::coded(
                    "failedUsb",
                    "The screen restarted but Windows could not connect to it again. Click Reconnect (needs one administrator approval).",
                )
                .status(502),
                Some(e) => e,
                None => friendly_fake("notFound"),
            });
        }
        ops::restart_screen(progress).map_err(CoreError::from)
    }

    fn speed(&self) -> f64 {
        self.fake.as_ref().map(|f| f.lock().unwrap().speed).unwrap_or(1.0)
    }

    /// Check the screen; if it doesn't answer, restart it once and check again (`_restarted: true`).
    pub fn check(&self, progress: Progress) -> CoreResult<Value> {
        match self.check_once() {
            Ok(v) => Ok(v),
            Err(e) if e.code_is("noReply") => {
                let mut info = self.restart(progress)?;
                info["_restarted"] = json!(true);
                Ok(info)
            }
            Err(e) => Err(e),
        }
    }

    /// Upload files one after another; with `play_loop` the send ends with a restart (warning, not failure, if
    /// that restart fails). MJPEG guard enforced in the device layer (and here for fake mode).
    pub fn send_files(&self, items: &[(PathBuf, Slot)], play_loop: bool, progress: SendProgress) -> CoreResult<Value> {
        self.send_files_with(items, play_loop, None, progress)
    }

    /// `known_loop`: the size of the loop file on the screen now, when certain (see `ops::send_files_with`).
    pub fn send_files_with(&self, items: &[(PathBuf, Slot)], play_loop: bool, known_loop: Option<u64>, progress: SendProgress) -> CoreResult<Value> {
        for (f, _) in items {
            let d = std::fs::read(f).map_err(|e| CoreError::new(format!("Cannot read {}: {e}", f.display())))?;
            if !device::mjpeg::is_mjpeg_mp4(&d) {
                return Err(CoreError::new("Refusing to upload: the screen can only play MJPEG .mp4 clips made by CoolerScreen."));
            }
        }
        if self.fake.is_none() {
            let r = ops::send_files_with(items, play_loop, known_loop, progress).map_err(CoreError::from)?;
            let mut v = serde_json::to_value(&r).unwrap_or(json!({}));
            if r.transport == "serial" {
                v["serialWarning"] = json!(SERIAL_WARNING);
            }
            return Ok(v);
        }
        let total = items.len() as f64 + if play_loop { 0.1 } else { 0.0 };
        let mut restarted = false;
        for (i, (f, slot)) in items.iter().enumerate() {
            let size = std::fs::metadata(f).map(|m| m.len()).unwrap_or(0);
            progress(i as f64 / total, if i == 0 { "Connecting to screen" } else { "Waiting for screen to come back" }, 0, size);
            let mut err = self.fake_error();
            if err.as_ref().is_some_and(|e| e.code_is("noReply")) && !restarted {
                progress(i as f64 / total, "The screen did not answer: restarting it", 0, size);
                self.restart(&mut |_, _| {})?;
                restarted = true;
                err = self.fake_error();
            }
            if let Some(e) = err {
                return Err(e);
            }
            self.fsleep(500);
            let steps = if *slot == Slot::Standby { 20 } else { 4 };
            let label = if *slot == Slot::Standby { "loop" } else { "intro" };
            for k in 1..=steps {
                let fail = self.fake.as_ref().unwrap().lock().unwrap().fail_upload;
                if fail && *slot == Slot::Standby && k > steps / 2 {
                    return Err(CoreError::coded(
                        "stalled",
                        "The screen stopped responding. Press Reconnect (or unplug and replug its USB cable), then send again. (USB write timed out after 20 s (fake))",
                    )
                    .status(502));
                }
                progress((i as f64 + k as f64 / steps as f64) / total, &format!("Uploading {label} clip"), size * k / steps, size);
                self.fsleep(500);
            }
        }
        let (mut restart_failed, mut final_info) = (Value::Null, Value::Null);
        if play_loop {
            progress(items.len() as f64 / total, "Restarting the screen to show the new clip", 0, 0);
            match self.restart(&mut |_, _| {}) {
                Ok(info) => final_info = info,
                Err(e) => restart_failed = json!({ "message": e.message, "failedUsb": e.failed_usb }),
            }
        }
        let serial = self.fake.as_ref().unwrap().lock().unwrap().transport == "serial";
        Ok(json!({ "ok": true, "transport": "fake", "restarted": restarted, "restartFailed": restart_failed, "finalInfo": final_info,
            "serialWarning": if serial { json!(SERIAL_WARNING) } else { Value::Null } }))
    }

    // ---------- one-time driver scripts (scripts/*.ps1, elevated) ----------

    fn run_elevated(&self, script: &str) -> CoreResult<String> {
        if let Some(f) = &self.fake {
            // test hook: never elevate or change drivers; just switch the simulated state
            self.fsleep(1500);
            f.lock().unwrap().transport = if script == "restore-serial.ps1" { "serial".into() } else { "winusb".into() };
            return Ok("RESULT: OK (fake)".into());
        }
        #[cfg(not(windows))]
        {
            let _ = script;
            return Err(CoreError::new("Driver setup is only needed (and supported) on Windows."));
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let file = self.scripts.lock().unwrap().join(script);
            if !file.exists() {
                return Err(CoreError::new(format!("Setup script not found: {}", file.display())));
            }
            let log = self.new_setup_log(script);
            let ps = script_command(&file, &log, &[], true);
            let out = std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", &ps])
                .creation_flags(0x0800_0000)
                .output()
                .map_err(|e| CoreError::new(format!("Could not start PowerShell: {e}")))?;
            let err = String::from_utf8_lossy(&out.stderr).to_string();
            let text = std::fs::read_to_string(&log).unwrap_or_default();
            if text.contains("RESULT: OK") {
                return Ok(text);
            }
            if let Some(i) = text.find("RESULT: ERROR ") {
                let m = text[i + 14..].lines().next().unwrap_or("").to_string();
                return Err(CoreError::new(format!("Setup failed: {m} (log: {})", log.display())));
            }
            let l = err.to_lowercase();
            Err(CoreError::new(if l.contains("canceled") || l.contains("cancelled") {
                "Setup was cancelled at the administrator prompt.".to_string()
            } else {
                format!("Setup did not run or finished without a result. {} (log: {})", err.trim(), log.display())
            }))
        }
    }

    fn wait_for(&self, pred: impl Fn(&Value) -> bool, seconds: u64) -> bool {
        let end = Instant::now() + Duration::from_millis((seconds as f64 * 1000.0 * self.speed()) as u64);
        while Instant::now() < end {
            if pred(&self.state()) {
                return true;
            }
            self.fsleep(1000);
        }
        false
    }

    /// Install WinUSB, verify by opening over WinUSB and restarting the screen.
    pub fn setup_winusb(&self, progress: Progress) -> CoreResult<Value> {
        let st = self.state();
        if st["transport"] == "winusb" {
            progress(1.0, "Already set up");
            return Ok(json!({ "already": true }));
        }
        if st["transport"] == "none" {
            return Err(CoreError::new("Screen not found. Plug it in and make sure the cooler is powered."));
        }
        progress(0.1, "Waiting for administrator approval");
        self.run_elevated("setup-winusb.ps1")?;
        progress(0.6, "Waiting for the screen to switch driver");
        if !self.wait_for(|s| s["transport"] == "winusb", 25) {
            return Err(CoreError::new(format!(
                "Driver was installed but the screen is not reachable over WinUSB yet. Unplug and replug the screen's USB cable and try \"Check screen\". (log: {})",
                self.last_setup_log()
            )));
        }
        progress(0.85, "Verifying (restarting the screen)");
        if self.fake.is_none() {
            let mut s = ops::open_any().map_err(CoreError::from)?;
            let r = s.info().and_then(|_| s.send_restart());
            s.close();
            r.map_err(|e: DevErr| CoreError::from(e))?;
            std::thread::sleep(Duration::from_millis(500));
        }
        progress(1.0, "Done");
        Ok(json!({ "already": false }))
    }

    pub fn restore_serial(&self, progress: Progress) -> CoreResult<Value> {
        progress(0.1, "Waiting for administrator approval");
        self.run_elevated("restore-serial.ps1")?;
        progress(0.7, "Waiting for the COM port");
        let ok = self.wait_for(|s| s["transport"] == "serial", 25);
        progress(1.0, "Done");
        if !ok {
            return Ok(json!({ "warning": "Original driver restored, but the COM port has not appeared yet. Replug the USB cable if needed." }));
        }
        Ok(json!({}))
    }

    /// Power-cycle the USB hub port of a screen that failed to enumerate at boot.
    pub fn reconnect(&self, progress: Progress) -> CoreResult<Value> {
        let st = self.state();
        if st["transport"] != "none" {
            progress(1.0, "Screen is already connected");
            return Ok(json!({ "already": true }));
        }
        if st["failedUsb"] != true {
            return Err(CoreError::new(
                "Screen not found and no failed USB device was detected. Check the USB cable and that the cooler has power.",
            ));
        }
        progress(0.1, "Waiting for administrator approval");
        self.run_elevated("reconnect.ps1")?;
        progress(0.6, "Waiting for the screen to appear");
        let ok = self.wait_for(|s| s["transport"] != "none", 25);
        progress(1.0, "Done");
        if !ok {
            return Err(CoreError::new(
                "The USB port was reset but the screen did not appear. Try unplugging and replugging the screen USB cable.",
            ));
        }
        Ok(json!({ "already": false }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mjpeg_file(dir: &Path, name: &str) -> PathBuf {
        // Smallest file the MJPEG guard accepts: a top-level moov with an 'mp4v'-less 'jpeg' sample entry.
        let p = dir.join(name);
        std::fs::write(&p, crate::device::mjpeg::tests_support::tiny_mjpeg_mp4()).unwrap();
        p
    }

    #[test]
    fn fake_check_and_states() {
        let d = DeviceService::fake("1", 0.01, false);
        assert_eq!(d.state()["transport"], "winusb");
        let i = d.check(&mut |_, _| {}).unwrap();
        assert_eq!(i["_transport"], "winusb");
        assert_eq!(i["i_block_free"], json!(4880));
        let none = DeviceService::fake("none", 0.01, false);
        assert!(none.check(&mut |_, _| {}).unwrap_err().code_is("notFound"));
        let failed = DeviceService::fake("failedusb", 0.01, false);
        let e = failed.check(&mut |_, _| {}).unwrap_err();
        assert!(e.failed_usb && e.code_is("failedUsb"));
        assert_eq!(failed.state()["failedUsb"], json!(true));
    }

    #[test]
    fn fake_noreply_restarts_once() {
        let d = DeviceService::fake("noreply", 0.01, false);
        let i = d.check(&mut |_, _| {}).unwrap();
        assert_eq!(i["_restarted"], json!(true));
        let stuck = DeviceService::fake("noreply-stuck", 0.01, false);
        let e = stuck.check(&mut |_, _| {}).unwrap_err();
        assert!(e.failed_usb);
    }

    #[test]
    fn fake_send_and_guard() {
        let dir = tempfile::tempdir().unwrap();
        let d = DeviceService::fake("winusb", 0.01, false);
        let bad = dir.path().join("h264.mp4");
        std::fs::write(&bad, b"not an mp4").unwrap();
        assert!(d.send_files(&[(bad, Slot::Standby)], true, &mut |_, _, _, _| {}).is_err(), "MJPEG guard");
        let ok = mjpeg_file(dir.path(), "ok.mp4");
        let mut last = 0.0;
        let r = d.send_files(&[(ok.clone(), Slot::Standby)], true, &mut |p, _, _, _| last = p).unwrap();
        assert_eq!(r["ok"], json!(true));
        assert!(last > 0.9);
        let failing = DeviceService::fake("winusb", 0.01, true);
        assert!(failing.send_files(&[(ok, Slot::Standby)], true, &mut |_, _, _, _| {}).unwrap_err().code_is("stalled"));
    }

    #[test]
    fn fake_send_recovers_from_no_reply() {
        // The upload command gets no reply (nothing streamed yet): restart once, then upload.
        let dir = tempfile::tempdir().unwrap();
        let ok = mjpeg_file(dir.path(), "ok.mp4");
        let d = DeviceService::fake("noreply", 0.01, false);
        let mut msgs = vec![];
        let r = d.send_files(&[(ok.clone(), Slot::Standby)], true, &mut |_, m, _, _| msgs.push(m.to_string())).unwrap();
        assert_eq!(r["restarted"], json!(true));
        assert!(msgs.iter().any(|m| m.contains("did not answer")));
        // ... and when the screen does not come back after the restart: failed USB (Reconnect).
        let stuck = DeviceService::fake("noreply-stuck", 0.01, false);
        let e = stuck.send_files(&[(ok, Slot::Standby)], true, &mut |_, _, _, _| {}).unwrap_err();
        assert!(e.failed_usb, "{e}");
    }

    fn scripts() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("scripts")
    }

    /// Every regex in scripts/*.ps1 (after -match / -notmatch / [regex]::Match) must compile in .NET: an invalid one
    /// throws at run time and the script ends with RESULT: ERROR (reconnect.ps1 had '^USB\VID_...\').
    #[test]
    #[cfg(windows)]
    fn script_regexes_compile_in_dotnet() {
        let mut pats = vec![];
        for e in std::fs::read_dir(scripts()).unwrap().flatten() {
            if e.path().extension().is_some_and(|x| x == "ps1") {
                let s = std::fs::read_to_string(e.path()).unwrap();
                for key in ["-match '", "-notmatch '", "[regex]::Match("] {
                    let mut rest = s.as_str();
                    while let Some(i) = rest.find(key) {
                        rest = &rest[i + key.len()..];
                        let lit = if key.starts_with('[') {
                            // second argument: , '...'
                            match rest.find(", '") {
                                Some(j) if j < 200 => &rest[j + 3..],
                                _ => continue,
                            }
                        } else {
                            rest
                        };
                        // a PowerShell single-quoted literal ('' is an escaped quote)
                        let mut out = String::new();
                        let mut it = lit.chars().peekable();
                        while let Some(c) = it.next() {
                            if c == '\'' {
                                if it.peek() == Some(&'\'') {
                                    it.next();
                                    out.push('\'');
                                } else {
                                    break;
                                }
                            } else {
                                out.push(c);
                            }
                        }
                        pats.push(out);
                    }
                }
            }
        }
        assert!(pats.len() >= 8, "found {} patterns", pats.len());
        let dir = tempfile::tempdir().unwrap();
        let list = dir.path().join("patterns.txt");
        std::fs::write(&list, pats.join("\n")).unwrap();
        let cmd = format!(
            "$bad = 0; Get-Content -LiteralPath '{}' -Encoding UTF8 | ForEach-Object {{ try {{ [void][regex]::new($_) }} catch {{ $bad++; 'BAD ' + $_ }} }}; \"BAD_COUNT $bad\"",
            list.display()
        );
        let out = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &cmd]).output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("BAD_COUNT 0"), "invalid .NET regexes in scripts:\n{text}\npatterns: {pats:?}");
    }

    /// The script command passes -Log (and extra switches) intact, also through paths with spaces: reconnect.ps1
    /// -DryRun only enumerates devices (no admin rights needed, nothing is cycled).
    #[test]
    #[cfg(windows)]
    fn script_command_passes_log_and_dry_run() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("with space");
        std::fs::create_dir_all(&sub).unwrap();
        let script = sub.join("reconnect.ps1");
        std::fs::copy(scripts().join("reconnect.ps1"), &script).unwrap();
        let log = sub.join("run log.txt");
        let cmd = script_command(&script, &log, &["-DryRun"], false);
        let st = std::process::Command::new("powershell").args(["-NoProfile", "-Command", &cmd]).status().unwrap();
        assert!(st.success());
        let text = std::fs::read_to_string(&log).expect("the script wrote the -Log file");
        assert!(text.contains("RESULT: OK"), "{text}");
        assert!(!text.contains("cycling "), "a dry run never cycles a port:\n{text}");
    }

    #[test]
    fn fake_setup_restore_reconnect() {
        let d = DeviceService::fake("serial", 0.01, false);
        assert_eq!(d.state()["needsSetup"], json!(true));
        assert_eq!(d.setup_winusb(&mut |_, _| {}).unwrap()["already"], json!(false));
        assert_eq!(d.state()["transport"], "winusb");
        d.restore_serial(&mut |_, _| {}).unwrap();
        assert_eq!(d.state()["transport"], "serial");
        let f = DeviceService::fake("failedusb", 0.01, false);
        assert_eq!(f.reconnect(&mut |_, _| {}).unwrap()["already"], json!(false));
        let n = DeviceService::fake("none", 0.01, false);
        assert!(n.reconnect(&mut |_, _| {}).is_err());
    }
}
