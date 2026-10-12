//! The live link to the screen over WinUSB (via the device layer). Sequence, as the vendor app does it:
//! connect: FF D9 FF D9 (end any old stream), 0x15 [00 0A] (stored: do not switch the display off after live), 0x11 (enter live).
//! Frames: u32LE length | JPEG | u16LE sum. Keepalive 0x11 every 1.5 s. stop: FF D9 FF D9, display on (0x0E [1]),
//! loop clip (0x14 [1,0]), so the 2.0 behaviour (the loop clip keeps playing) holds when the companion stops.
use crate::device::Screen;
use std::time::{Duration, Instant};

pub const KEEPALIVE: Duration = Duration::from_millis(1500);
const END_OF_STREAM: [u8; 4] = [0xff, 0xd9, 0xff, 0xd9];

/// How a live stream ends.
/// LoopClip (default, the 2.0 behaviour): display on and the stored loop clip plays.
/// ScreenOff (opt-in): the vendor app's shutdown sequence, the screen turns off (0x14 [0,0]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ending {
    #[default]
    LoopClip,
    ScreenOff,
}

pub struct LiveSession {
    scr: Option<Screen>,
    last_keep: Instant,
    wire: Vec<u8>,
}

impl LiveSession {
    pub fn connect() -> Result<LiveSession, String> {
        let mut scr = Screen::open_usb().map_err(|e| e.to_string())?;
        let r = (|| -> Result<(), String> {
            scr.live_write(&END_OF_STREAM).map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(100));
            scr.live_key(0x15, &[0x00, 0x0a]).map_err(|e| e.to_string())?;
            std::thread::sleep(Duration::from_millis(100));
            scr.live_key(0x11, &[]).map_err(|e| e.to_string())
        })();
        if let Err(e) = r {
            scr.close();
            return Err(e);
        }
        Ok(LiveSession { scr: Some(scr), last_keep: Instant::now(), wire: Vec::new() })
    }

    /// 0x11 when 1.5 s have passed since the last one (call between frames, never mid-frame).
    pub fn keepalive_if_due(&mut self) -> Result<(), String> {
        if self.last_keep.elapsed() < KEEPALIVE {
            return Ok(());
        }
        let scr = self.scr.as_mut().ok_or("session closed")?;
        scr.live_key(0x11, &[]).map_err(|e| e.to_string())?;
        scr.live_drain();
        self.last_keep = Instant::now();
        Ok(())
    }

    pub fn send_frame(&mut self, jpeg: &[u8]) -> Result<(), String> {
        self.keepalive_if_due()?;
        wrap_frame_into(jpeg, &mut self.wire);
        let scr = self.scr.as_mut().ok_or("session closed")?;
        scr.live_write(&self.wire).map_err(|e| e.to_string())
    }

    /// Orderly end with the default ending: the loop clip plays, display on.
    pub fn stop(self) {
        self.stop_with(Ending::LoopClip);
    }

    /// Orderly end. FF D9 FF D9 is always its own transfer; the screen-off command follows as a separate transfer a moment
    /// later (the vendor notes the firmware rejects the two when they are coalesced).
    pub fn stop_with(mut self, ending: Ending) {
        if let Some(mut scr) = self.scr.take() {
            let _ = scr.live_write(&END_OF_STREAM);
            match ending {
                Ending::LoopClip => {
                    std::thread::sleep(Duration::from_millis(200));
                    let _ = scr.live_key(0x0e, &[1]);
                    std::thread::sleep(Duration::from_millis(100));
                    let _ = scr.live_key(0x14, &[1, 0]);
                }
                Ending::ScreenOff => {
                    std::thread::sleep(Duration::from_millis(2));
                    let _ = scr.live_screen_off();
                }
            }
            std::thread::sleep(Duration::from_millis(300));
            scr.close();
        }
    }

    /// Shutdown variant: the same writes with only a 2 ms gap, no settling waits. Completes in well under a second.
    pub fn stop_fast(mut self, ending: Ending) {
        if let Some(mut scr) = self.scr.take() {
            let _ = scr.live_write(&END_OF_STREAM);
            std::thread::sleep(Duration::from_millis(2));
            match ending {
                Ending::LoopClip => {
                    let _ = scr.live_key(0x0e, &[1]);
                    let _ = scr.live_key(0x14, &[1, 0]);
                }
                Ending::ScreenOff => {
                    let _ = scr.live_screen_off();
                }
            }
            scr.close();
        }
    }

    /// Drop the link without the end sequence (the device is gone or erroring).
    pub fn abandon(mut self) {
        if let Some(scr) = self.scr.take() {
            scr.close();
        }
    }
}

/// `u32LE len | JPEG | u16LE sum`, sum = (4 length bytes + JPEG bytes) & 0xFFFF.
pub fn wrap_frame_into(jpeg: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(jpeg.len() + 6);
    out.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
    out.extend_from_slice(jpeg);
    let sum = out.iter().fold(0u32, |a, &b| a.wrapping_add(b as u32)) & 0xffff;
    out.extend_from_slice(&(sum as u16).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_layout() {
        let mut v = Vec::new();
        wrap_frame_into(&[0xff, 0xd8, 0xff, 0xd9], &mut v);
        assert_eq!(v, vec![0x04, 0, 0, 0, 0xff, 0xd8, 0xff, 0xd9, 0xb3, 0x03]);
    }
}
