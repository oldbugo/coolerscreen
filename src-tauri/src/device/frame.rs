//! 55 AA framing, ported from src/device.js and src/usbScreen.js.
//! Frame: 55 AA | len16 LE (payload + 7) | key | payload | sum16 LE (sum of all previous bytes).
use serde_json::{json, Value};

/// Unsolicited status frames (every ~2 s over WinUSB); never a reply.
pub const STATUS_KEY: u8 = 0x22;

pub fn frame(key: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() + 7;
    let mut v = Vec::with_capacity(len);
    v.extend_from_slice(&[0x55, 0xaa, (len & 0xff) as u8, ((len >> 8) & 0xff) as u8, key]);
    v.extend_from_slice(payload);
    let sum = v.iter().fold(0u32, |a, &b| a + b as u32) & 0xffff;
    v.push((sum & 0xff) as u8);
    v.push((sum >> 8) as u8);
    v
}

fn find_marker(b: &[u8]) -> Option<usize> {
    b.windows(2).position(|w| w == [0x55, 0xaa])
}

/// Incremental frame extractor (same rules as UsbScreen._onData: resync on 55 AA, drop
/// frames shorter than 7 bytes, skip 0x22 status frames, JSON body or {key, hex}).
#[derive(Default)]
pub struct Parser {
    rx: Vec<u8>,
}

impl Parser {
    pub fn push(&mut self, d: &[u8]) -> Vec<Value> {
        self.rx.extend_from_slice(d);
        let mut out = Vec::new();
        loop {
            let Some(i) = find_marker(&self.rx) else {
                self.rx.clear();
                return out;
            };
            if i > 0 {
                self.rx.drain(..i);
            }
            if self.rx.len() < 4 {
                return out;
            }
            let len = u16::from_le_bytes([self.rx[2], self.rx[3]]) as usize;
            if len < 7 {
                self.rx.drain(..2);
                continue;
            }
            if self.rx.len() < len {
                return out;
            }
            let f: Vec<u8> = self.rx.drain(..len).collect();
            if f[4] == STATUS_KEY {
                continue;
            }
            let body = &f[5..len - 2];
            let msg = std::str::from_utf8(body)
                .ok()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .unwrap_or_else(|| json!({ "key": f[4], "hex": hex(body) }));
            out.push(msg);
        }
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_matches_node() {
        // Node: frame(0x06) -> 55 aa 07 00 06 0c 01
        assert_eq!(frame(0x06, &[]), vec![0x55, 0xaa, 0x07, 0x00, 0x06, 0x0c, 0x01]);
        // frame(0x13, size 0x00879b36 LE)
        let f = frame(0x13, &8_887_094u32.to_le_bytes());
        assert_eq!(&f[..5], &[0x55, 0xaa, 0x0b, 0x00, 0x13]);
        let sum: u32 = f[..f.len() - 2].iter().map(|&b| b as u32).sum();
        assert_eq!(u16::from_le_bytes([f[f.len() - 2], f[f.len() - 1]]) as u32, sum & 0xffff);
    }

    #[test]
    fn parser_skips_status_and_resyncs() {
        let mut p = Parser::default();
        let mut data = vec![0x00, 0x13];
        data.extend(frame(STATUS_KEY, b"{\"x\":1}"));
        data.extend(frame(0x13, &[0x00]));
        let info = frame(0x06, br#"{"cmd":"info","data":{"version":"3.6"}}"#);
        data.extend(&info[..6]);
        let out = p.push(&data);
        assert_eq!(out, vec![json!({"key": 19, "hex": "00"})]);
        let out = p.push(&info[6..]);
        assert_eq!(out[0]["data"]["version"], "3.6");
    }
}
