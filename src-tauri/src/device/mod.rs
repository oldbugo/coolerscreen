//! Rust device layer for the Lovingcool SC70 screen (VID 33C3, PID 7791/7788).
//! Port of src/device.js, src/usbScreen.js and src/deviceOps.js (CoolerScreen 1.2).
pub mod frame;
pub mod log;
pub mod mjpeg;
mod serial;
mod usb;

pub use log::log;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

pub const CHUNK: usize = 40 * 1024;
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(20);
const REPLY_TIMEOUT: Duration = Duration::from_millis(3000);

pub const KEY_RESTART: u8 = 0x01;
pub const KEY_INFO: u8 = 0x06;
pub const KEY_STARTUP: u8 = 0x12; // intro slot
pub const KEY_STANDBY: u8 = 0x13; // loop slot
/// Never sent by this code: delete (0x05), OTA (0x0C), serial number (0x23).
const FORBIDDEN_KEYS: [u8; 3] = [0x05, 0x0c, 0x23];
/// Everything the app is allowed to send.
const ALLOWED_KEYS: [u8; 4] = [KEY_RESTART, KEY_INFO, KEY_STARTUP, KEY_STANDBY];

pub const SERIAL_WARNING: &str = "The screen is still on the standard serial (COM) driver, so it will turn its display off about 30 seconds after any program closes the port. Run the one-time setup (\"Set up screen\") to fix this.";

const MSG_FAILED_USB: &str = "Windows could not connect to the screen at startup (a known quirk). Click Reconnect (needs one administrator approval).";

#[derive(Debug, Clone, Serialize)]
pub struct DevErr {
    pub code: Option<String>,
    pub message: String,
    #[serde(rename = "failedUsb")]
    pub failed_usb: bool,
}

impl DevErr {
    pub fn new(m: impl Into<String>) -> Self {
        DevErr { code: None, message: m.into(), failed_usb: false }
    }
    fn coded(code: &str, m: impl Into<String>) -> Self {
        DevErr { code: Some(code.into()), message: m.into(), failed_usb: code == "failedUsb" }
    }
}

impl std::fmt::Display for DevErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.code {
            Some(c) => write!(f, "[{c}] {}", self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

/// Same mapping as deviceOps.friendly().
pub fn friendly(e: DevErr) -> DevErr {
    if e.failed_usb || e.code.as_deref() == Some("noReply") {
        return e;
    }
    let m = e.message.clone();
    let l = m.to_lowercase();
    let has = |xs: &[&str]| xs.iter().any(|x| l.contains(x));
    if l.contains("no reply") {
        return DevErr::coded("noReply", "The screen is not responding (it keeps playing, but doesn't answer). Restart the screen to bring it back.");
    }
    if l.contains("not found") {
        return DevErr::coded("notFound", "Screen not found. Check the USB cable and that the cooler is powered.");
    }
    if has(&["access denied", "busy", "in use", "ebusy", "eacces", "cannot open", "cannot claim", "opening"]) {
        return DevErr::coded("busy", format!("Could not open the screen. Close \"LOVINGCOOL MONITOR\" (and any other program using the screen), then try again. ({m})"));
    }
    if has(&["timed out", "time out", "timeout", "timedout", "stalled"]) {
        return DevErr::coded("stalled", format!("The screen stopped responding. Press Reconnect (or unplug and replug its USB cable), then send again. ({m})"));
    }
    e
}

enum Link {
    Usb(usb::UsbLink),
    Serial(serial::SerialLink),
}

/// Connections this process holds right now (the live session, a check, a send): when opening fails with "busy" and this is
/// above zero, the holder is CoolerScreen itself.
static OPEN_HERE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub fn in_process_open() -> usize {
    OPEN_HERE.load(std::sync::atomic::Ordering::SeqCst)
}

impl Drop for Screen {
    fn drop(&mut self) {
        OPEN_HERE.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// An open connection to the screen. Opened only for the duration of one operation.
pub struct Screen {
    link: Link,
    rx: Receiver<Value>,
    pub transport: &'static str,
    sent: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Intro (plays once at power-on), key 0x12.
    Startup,
    /// Loop (plays forever after the intro), key 0x13.
    Standby,
}

impl Slot {
    pub fn key(self) -> u8 {
        match self {
            Slot::Startup => KEY_STARTUP,
            Slot::Standby => KEY_STANDBY,
        }
    }
    pub fn parse(s: &str) -> Option<Slot> {
        match s {
            "startup" | "intro" => Some(Slot::Startup),
            "standby" | "loop" => Some(Slot::Standby),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UploadOutcome {
    pub ack: Value,
    pub done: Value,
    pub bytes: u64,
    pub stream_ms: u64,
}

impl Screen {
    fn is_usb(&self) -> bool {
        matches!(self.link, Link::Usb(_))
    }

    pub fn open_usb() -> Result<Screen, DevErr> {
        let (tx, rx) = channel();
        let link = usb::UsbLink::open(tx).map_err(DevErr::new)?;
        log("opened over WinUSB");
        OPEN_HERE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let s = Screen { link: Link::Usb(link), rx, transport: "winusb", sent: 0 };
        std::thread::sleep(Duration::from_millis(200));
        while s.rx.try_recv().is_ok() {}
        Ok(s)
    }

    pub fn open_serial(path: &str) -> Result<Screen, DevErr> {
        let (tx, rx) = channel();
        let link = serial::SerialLink::open(path, tx).map_err(DevErr::new)?;
        OPEN_HERE.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let s = Screen { link: Link::Serial(link), rx, transport: "serial", sent: 0 };
        std::thread::sleep(Duration::from_millis(300));
        while s.rx.try_recv().is_ok() {}
        Ok(s)
    }

    /// Write bytes in 40 KB transfers, each with a 20 s timeout.
    fn raw(&mut self, buf: &[u8]) -> Result<(), DevErr> {
        for chunk in buf.chunks(CHUNK) {
            let r = match &mut self.link {
                Link::Usb(u) => u.write(chunk, WRITE_TIMEOUT),
                Link::Serial(s) => s.write(chunk, WRITE_TIMEOUT),
            };
            if let Err(e) = r {
                if self.is_usb() {
                    log(&format!("write of {} bytes failed: {}", chunk.len(), e));
                }
                return Err(DevErr::new(if e == "TIMEOUT" {
                    "The screen stopped accepting data (USB write timed out). Unplug power to the PC for 15 s to reset it, then try again.".to_string()
                } else {
                    format!("USB write failed: {e}")
                }));
            }
            let before = self.sent;
            self.sent += chunk.len() as u64;
            if self.is_usb() && before / 1_048_576 != self.sent / 1_048_576 {
                log(&format!("sent {:.1} MB", self.sent as f64 / 1_048_576.0));
            }
        }
        Ok(())
    }

    fn reply(&mut self, timeout: Duration) -> Result<Value, DevErr> {
        self.rx.recv_timeout(timeout).map_err(|_| DevErr::new("No reply from screen"))
    }

    fn cmd(&mut self, key: u8, payload: Option<&[u8]>, wait: bool, timeout: Duration) -> Result<Value, DevErr> {
        if FORBIDDEN_KEYS.contains(&key) || !ALLOWED_KEYS.contains(&key) {
            return Err(DevErr::new(format!("Refusing to send key 0x{key:x}")));
        }
        let usb = self.is_usb();
        if usb {
            log(&format!("cmd 0x{key:x} {}", payload.map(|p| format!("{} bytes", p.len())).unwrap_or_default()));
        }
        while self.rx.try_recv().is_ok() {} // replies nobody waited for are dropped, as in Node
        let r = self.raw(&frame::frame(key, payload.unwrap_or(&[]))).and_then(|_| {
            if wait {
                self.reply(timeout)
            } else {
                Ok(Value::Null)
            }
        });
        if usb {
            match &r {
                Ok(v) => log(&format!("cmd 0x{key:x} -> {v}")),
                Err(e) => log(&format!("cmd 0x{key:x} failed: {}", e.message)),
            }
        }
        r
    }

    pub fn info(&mut self) -> Result<Value, DevErr> {
        self.cmd(KEY_INFO, None, true, REPLY_TIMEOUT)
    }

    /// Restart (0x01): the only command that still works when the screen stops answering. No reply.
    pub fn send_restart(&mut self) -> Result<(), DevErr> {
        self.cmd(KEY_RESTART, Some(&[]), false, REPLY_TIMEOUT)?;
        std::thread::sleep(Duration::from_millis(300));
        Ok(())
    }

    /// Upload an MJPEG .mp4 into a slot: size command, then the file in 40 KB bulk writes.
    pub fn upload(&mut self, data: &[u8], slot: Slot, progress: &mut dyn FnMut(u64, u64)) -> Result<UploadOutcome, DevErr> {
        if !mjpeg::is_mjpeg_mp4(data) {
            return Err(DevErr::new("Refusing to upload: the screen can only play MJPEG .mp4 clips made by CoolerScreen."));
        }
        let total = data.len() as u64;
        let ack = self.cmd(slot.key(), Some(&(data.len() as u32).to_le_bytes()), true, REPLY_TIMEOUT)?;
        // Hardware 2026-10-17: the size command is answered {"key":19,"hex":"00"} when the file is accepted; anything else refuses it.
        if ack_refused(&ack) {
            return Err(DevErr::coded("noRoom", format!("The screen refused the file ({total} bytes): {ack}. Nothing was written.")));
        }
        // Node registers a 30 s "done" waiter before streaming; keep the same deadline.
        let start = Instant::now();
        let mut off = 0u64;
        for chunk in data.chunks(CHUNK) {
            self.raw(chunk)?;
            off += chunk.len() as u64;
            progress(off, total);
        }
        let stream_ms = start.elapsed().as_millis() as u64;
        let left = Duration::from_secs(30).saturating_sub(start.elapsed());
        let done = match self.rx.recv_timeout(left.max(Duration::from_millis(1))) {
            Ok(v) => v,
            Err(_) => json!({ "error": "No reply from screen" }),
        };
        if self.is_usb() {
            log(&format!("upload of {total} bytes streamed in {stream_ms} ms, done -> {done}"));
        }
        Ok(UploadOutcome { ack, done, bytes: total, stream_ms })
    }

    pub fn close(mut self) {
        if self.is_usb() {
            log(&format!("close (sent {} bytes total)", self.sent));
        }
        match &mut self.link {
            Link::Usb(u) => u.close(),
            Link::Serial(s) => s.close(),
        }
    }
}

/// Unrestricted access for the `cs2` recovery tools only (unstick, recover-race-delete, delete-ota). The app never
/// calls these: it can only send the allow-listed keys through `cmd`.
impl Screen {
    pub fn recovery_cmd(&mut self, key: u8, payload: &[u8], timeout: Duration) -> Result<Value, DevErr> {
        while self.rx.try_recv().is_ok() {}
        self.raw(&frame::frame(key, payload))?;
        self.reply(timeout)
    }
    pub fn recovery_raw(&mut self, buf: &[u8]) -> Result<(), DevErr> {
        self.raw(buf)
    }
    /// Next reply frame, if one arrives in time.
    pub fn recovery_poll(&mut self, timeout: Duration) -> Option<Value> {
        self.rx.recv_timeout(timeout).ok()
    }
    pub fn recovery_restart_noreply(&mut self) -> Result<(), DevErr> {
        self.raw(&frame::frame(KEY_RESTART, &[]))
    }
}

pub mod ops;

/// A reply of the upload commands that says "no": a `hex` field other than "00".
pub fn ack_refused(v: &Value) -> bool {
    v.get("hex").and_then(Value::as_str).is_some_and(|h| h != "00")
}

/// Live-mode writes for the 3.0 companion (WinUSB or serial). Frames and the end-of-stream marker are not 55 AA frames,
/// and live traffic is far too frequent for the per-MB log line `raw` writes, so this path skips it.
impl Screen {
    pub fn live_write(&mut self, buf: &[u8]) -> Result<(), DevErr> {
        for chunk in buf.chunks(CHUNK) {
            let r = match &mut self.link {
                Link::Usb(u) => u.write(chunk, WRITE_TIMEOUT),
                Link::Serial(s) => s.write(chunk, WRITE_TIMEOUT),
            };
            r.map_err(|e| DevErr::new(if e == "TIMEOUT" { "The screen stopped accepting data (USB write timed out).".to_string() } else { format!("USB write failed: {e}") }))?;
            self.sent += chunk.len() as u64;
        }
        Ok(())
    }

    /// The only framed keys live mode sends: 0x11 enter/keepalive, 0x15 [00 0A] (do not switch the display off after
    /// a live stream, as the vendor app sends on every connect), 0x0E [1] display on, 0x14 [1,0] play the loop clip.
    pub fn live_key(&mut self, key: u8, payload: &[u8]) -> Result<(), DevErr> {
        let ok = matches!((key, payload), (0x11, []) | (0x15, [0x00, 0x0a]) | (0x0e, [1]) | (0x14, [1, 0]));
        if !ok {
            return Err(DevErr::new(format!("Refusing to send key 0x{key:x} in live mode")));
        }
        self.live_write(&frame::frame(key, payload))
    }

    /// Discard replies the screen sent (acks of 0x11 etc.) so the channel does not grow during a long session.
    pub fn live_drain(&mut self) -> usize {
        let mut n = 0;
        while self.rx.try_recv().is_ok() {
            n += 1;
        }
        n
    }
}

impl Screen {
    /// Screen off (0x14 [0,0]): only for the live companion's opt-in "screen off" ending, sent as its own transfer after
    /// FF D9 FF D9. Deliberately not part of `live_key`'s list.
    pub fn live_screen_off(&mut self) -> Result<(), DevErr> {
        self.live_write(&frame::frame(0x14, &[0, 0]))
    }
}
