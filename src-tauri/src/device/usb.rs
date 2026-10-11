//! WinUSB transport via nusb (port of src/usbScreen.js). With WinUSB bound (scripts/setup-winusb.ps1)
//! no CDC DTR request is ever sent, so the firmware's 30 s "DTR low -> display off" watchdog never fires.
use super::frame::Parser;
use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, Direction, In, Out, TransferError};
use nusb::MaybeFuture;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub const VID: u16 = 0x33c3;
pub const PIDS: [u16; 2] = [0x7791, 0x7788];

pub struct UsbLink {
    out: Option<nusb::Endpoint<Bulk, Out>>,
    stop: Arc<AtomicBool>,
    reader: Option<JoinHandle<()>>,
    iface: Option<nusb::Interface>,
    _dev: nusb::Device,
}

pub fn find() -> Option<nusb::DeviceInfo> {
    let list: Vec<_> = nusb::list_devices().wait().ok()?.collect();
    for pid in PIDS {
        if let Some(d) = list.iter().find(|d| d.vendor_id() == VID && d.product_id() == pid) {
            return Some(d.clone());
        }
    }
    None
}

impl UsbLink {
    /// Errors use the same wording as the Node version (deviceOps.friendly() keys off it).
    pub fn open(tx: Sender<Value>) -> Result<UsbLink, String> {
        let info = find().ok_or_else(|| "Screen not found (WinUSB)".to_string())?;
        let dev = info.open().wait().map_err(|e| format!("Cannot open the screen over WinUSB: {e}"))?;
        // First interface with a bulk IN and a bulk OUT endpoint (CDC data interface 1: OUT 0x02, IN 0x81).
        let mut found: Option<(u8, u8, u8)> = None;
        if let Ok(cfg) = dev.active_configuration() {
            for alt in cfg.interface_alt_settings() {
                let eps: Vec<_> = alt.endpoints().filter(|e| e.transfer_type() == TransferType::Bulk).collect();
                let o = eps.iter().find(|e| e.direction() == Direction::Out).map(|e| e.address());
                let i = eps.iter().find(|e| e.direction() == Direction::In).map(|e| e.address());
                if let (Some(o), Some(i)) = (o, i) {
                    found = Some((alt.interface_number(), o, i));
                    break;
                }
            }
        }
        let (num, ep_out, ep_in) =
            found.ok_or_else(|| "Screen has no bulk interface (WinUSB driver not bound?)".to_string())?;
        let iface = dev.claim_interface(num).wait().map_err(|e| {
            format!("Cannot claim the screen interface (is another program using it?): {e}")
        })?;
        let out = iface
            .endpoint::<Bulk, Out>(ep_out)
            .map_err(|e| format!("Cannot claim the screen interface (OUT endpoint): {e}"))?;
        let mut inp = iface
            .endpoint::<Bulk, In>(ep_in)
            .map_err(|e| format!("Cannot claim the screen interface (IN endpoint): {e}"))?;
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        // Equivalent of epIn.startPoll(3, 4096): keep three 4 KB IN transfers pending, feed the parser.
        let reader = std::thread::spawn(move || {
            let mut parser = Parser::default();
            while inp.pending() < 3 {
                inp.submit(Buffer::new(4096));
            }
            while !stop2.load(Ordering::Relaxed) {
                let Some(c) = inp.wait_next_complete(Duration::from_millis(100)) else { continue };
                match c.status {
                    Ok(()) => {
                        for m in parser.push(&c.buffer[..]) {
                            let _ = tx.send(m);
                        }
                    }
                    Err(TransferError::Disconnected) => break,
                    Err(_) => std::thread::sleep(Duration::from_millis(50)),
                }
                inp.submit(Buffer::new(4096));
            }
            inp.cancel_all();
            while inp.pending() > 0 {
                if inp.wait_next_complete(Duration::from_secs(1)).is_none() {
                    break;
                }
            }
        });
        Ok(UsbLink { out: Some(out), stop, reader: Some(reader), iface: Some(iface), _dev: dev })
    }

    /// One bulk OUT transfer with a timeout (libusb's default "forever" once turned a stalled screen into a hang).
    pub fn write(&mut self, buf: &[u8], timeout: Duration) -> Result<(), String> {
        let out = self.out.as_mut().ok_or("USB link closed")?;
        let c = out.transfer_blocking(Buffer::from(buf.to_vec()), timeout);
        match c.status {
            Ok(()) if c.actual_len == buf.len() => Ok(()),
            Ok(()) => Err(format!("USB write failed: short write ({} of {} bytes)", c.actual_len, buf.len())),
            Err(TransferError::Cancelled) => Err("TIMEOUT".into()),
            Err(e) => Err(format!("{e}")),
        }
    }

    pub fn close(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        self.out.take();
        if let Some(i) = self.iface.take() {
            let _ = i.release().wait();
        }
    }
}

impl Drop for UsbLink {
    fn drop(&mut self) {
        self.close();
    }
}
