//! COM-port fallback (port of the Screen class in src/device.js). Used only when WinUSB is not bound:
//! the firmware then turns the display off ~30 s after the port closes (DTR low), so the app warns.
use super::frame::Parser;
use serde_json::Value;
use serialport::{SerialPort, SerialPortType};
use std::io::{ErrorKind, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

const BAUD: u32 = 2_000_000;

pub fn find_port() -> Option<String> {
    let ports = serialport::available_ports().ok()?;
    ports.into_iter().find_map(|p| match p.port_type {
        SerialPortType::UsbPort(u) if u.vid == super::usb::VID && super::usb::PIDS.contains(&u.pid) => {
            Some(p.port_name)
        }
        _ => None,
    })
}

pub struct SerialLink {
    port: Box<dyn SerialPort>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
}

impl SerialLink {
    pub fn open(path: &str, tx: Sender<Value>) -> Result<SerialLink, String> {
        let mut port = serialport::new(path, BAUD)
            .timeout(Duration::from_millis(100))
            .open()
            .map_err(|e| format!("Opening {path}: {e}"))?;
        let _ = port.write_data_terminal_ready(true); // node-serialport opens with DTR on
        let mut rd = port.try_clone().map_err(|e| format!("Opening {path}: {e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let reader = std::thread::spawn(move || {
            let mut parser = Parser::default();
            let mut buf = [0u8; 4096];
            while !stop2.load(Ordering::Relaxed) {
                match rd.read(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => {
                        for m in parser.push(&buf[..n]) {
                            let _ = tx.send(m);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        });
        let mut link = SerialLink { port, stop, reader: Some(reader) };
        // Same reset the vendor app sends on connect: end any live JPEG stream.
        link.write(&[0xff, 0xd9, 0xff, 0xd9], Duration::from_secs(5))?;
        link.write(&[0, 0, 0, 0], Duration::from_secs(5))?;
        Ok(link)
    }

    pub fn write(&mut self, mut buf: &[u8], timeout: Duration) -> Result<(), String> {
        let end = Instant::now() + timeout;
        while !buf.is_empty() {
            match self.port.write(buf) {
                Ok(n) => buf = &buf[n..],
                Err(e) if e.kind() == ErrorKind::TimedOut || e.kind() == ErrorKind::Interrupted => {
                    if Instant::now() > end {
                        return Err("TIMEOUT".into());
                    }
                }
                Err(e) => return Err(format!("{e}")),
            }
        }
        self.port.flush().map_err(|e| format!("{e}"))
    }

    pub fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
    }
}

impl Drop for SerialLink {
    fn drop(&mut self) {
        self.close();
    }
}
