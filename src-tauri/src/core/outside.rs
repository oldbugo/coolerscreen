//! "Screen content changed outside the app": the screen reports its free flash (`i_block_free`) in the same read-only info
//! answer every check already gets. CoolerScreen remembers that number each time it stored something itself (the final check
//! of an upload). If a later check finds a clearly different number, something else wrote to the screen (another program, the
//! vendor tool, a reset) and what the app thinks is Embedded may not be what is stored. Nothing is sent to the screen for this.
//!
//! Settings keys: `screenBaseline` {free, at} (free blocks after our last store or first check) and `screenChanged`
//! {was, now, blockSize, at} while a difference is open (cleared by a new store or by the user accepting it).
use super::app::App;
use super::js;
use serde_json::{json, Value};

/// Free blocks may wobble a little between reads (the screen keeps some bookkeeping): up to 64 blocks (32 KB) is not a change.
pub const TOLERANCE_BLOCKS: f64 = 64.0;

/// (free blocks, block size) from an info answer (nested under `data` on the real screen, top level on the fake one).
pub fn free_blocks(info: &Value) -> Option<(f64, f64)> {
    let d = info.get("data").filter(|d| d.is_object()).unwrap_or(info);
    let free = js::to_num(d.get("i_block_free"));
    if !free.is_finite() || free < 0.0 {
        return None;
    }
    let bs = js::to_num(d.get("i_block_size"));
    Some((free, if bs.is_finite() && bs > 0.0 { bs } else { 512.0 }))
}

/// SHA-1 of a file, read in pieces (what identifies a stored clip).
pub fn file_hash(p: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(p)?;
    let mut h = sha1_smol::Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(h.digest().to_string());
        }
        h.update(&buf[..n]);
    }
}

/// Did the free space move by more than the tolerance since the baseline?
pub fn differs(baseline_free: f64, now_free: f64) -> bool {
    (baseline_free - now_free).abs() > TOLERANCE_BLOCKS
}

impl App {
    /// A check of the screen: remember the answer as the baseline the first time, flag a difference afterwards.
    pub fn note_screen_info(&self, info: &Value) {
        let Some((free, bs)) = free_blocks(info) else { return };
        let s = self.settings.load();
        let base = s.0.get("screenBaseline").and_then(|b| b["free"].as_f64());
        match base {
            None => {
                let _ = self.settings.update(|v| v["screenBaseline"] = json!({ "free": free, "at": js::iso_now() }));
            }
            Some(b) if differs(b, free) => {
                let prev = s.0.get("screenChanged").filter(|c| c.is_object());
                let was = prev.and_then(|c| c["was"].as_f64()).unwrap_or(b);
                let at = prev.and_then(|c| c["at"].as_str().map(String::from)).unwrap_or_else(js::iso_now);
                if prev.is_none() {
                    crate::device::log(&format!("Screen changed outside the app: free space was {b} blocks after the last store, now {free} (block {bs} B)"));
                }
                let _ = self.settings.update(|v| v["screenChanged"] = json!({ "was": was, "now": free, "blockSize": bs, "at": at }));
            }
            Some(_) => {}
        }
    }

    /// Is the loop about to be stored exactly what the screen holds? Some(final info) only when everything agrees: the loop slot's
    /// record is complete, the same size and the same content (hash; records from before hashes existed count only when they name
    /// this very version), no outside change is open, a baseline exists, and a fresh read-only info check shows the same free
    /// space as the baseline taken after our last store. Anything else (or a failed read) is None: send.
    pub(crate) fn already_on_screen(&self, vid: &str, size: u64, hash: &str) -> Option<Value> {
        let s = self.settings.load();
        let slot = s.slot("standby")?;
        if js::truthy(slot.get("incomplete")) || slot["size"].as_u64()? != size {
            return None;
        }
        if s.0.get("screenChanged").is_some_and(|c| c.is_object()) {
            return None;
        }
        let base = s.0.get("screenBaseline")?["free"].as_f64()?;
        match slot["hash"].as_str() {
            Some(h) if h != hash => return None,
            Some(_) => {}
            None if slot["versionId"].as_str() != Some(vid) => return None,
            None => {}
        }
        let info = self.device.read_info().ok()?;
        let (free, _) = free_blocks(&info)?;
        (!differs(base, free)).then_some(info)
    }

    /// The app just stored something and checked the screen afterwards: that is the new baseline and nothing is open.
    /// Without an answer (the final check failed) the baseline is dropped, so the next check starts it again.
    pub fn rebase_screen(&self, info: Option<&Value>) {
        let free = info.and_then(free_blocks).map(|f| f.0);
        let _ = self.settings.update(|v| {
            if let Some(o) = v.as_object_mut() {
                o.remove("screenChanged");
                match free {
                    Some(f) => {
                        o.insert("screenBaseline".into(), json!({ "free": f, "at": js::iso_now() }));
                    }
                    None => {
                        o.remove("screenBaseline");
                    }
                }
            }
        });
    }

    /// The user says the difference is fine (or has looked at it): what the screen reports now becomes the baseline.
    pub fn accept_screen_change(&self) {
        let now = self.settings.load().0.get("screenChanged").and_then(|c| c["now"].as_f64());
        let _ = self.settings.update(|v| {
            if let Some(o) = v.as_object_mut() {
                o.remove("screenChanged");
                if let Some(n) = now {
                    o.insert("screenBaseline".into(), json!({ "free": n, "at": js::iso_now() }));
                }
            }
        });
    }

    /// For the page: the open difference as {wasFree, nowFree (bytes), at} or null.
    pub fn screen_changed_value(&self) -> Value {
        match self.settings.load().0.get("screenChanged") {
            Some(c) if c.is_object() => {
                let bs = c["blockSize"].as_f64().unwrap_or(512.0);
                json!({ "wasFree": js::num(c["was"].as_f64().unwrap_or(0.0) * bs), "nowFree": js::num(c["now"].as_f64().unwrap_or(0.0) * bs), "at": c["at"] })
            }
            _ => Value::Null,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_real_and_the_fake_shape_and_applies_the_tolerance() {
        let real = json!({ "cmd": "info", "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 792 } });
        assert_eq!(free_blocks(&real), Some((792.0, 512.0)));
        assert_eq!(free_blocks(&json!({ "i_block_free": 4880 })), Some((4880.0, 512.0)));
        assert_eq!(free_blocks(&json!({ "data": {} })), None);
        assert!(!differs(792.0, 792.0 + 64.0));
        assert!(differs(792.0, 792.0 + 65.0));
        assert!(differs(4880.0, 100.0));
    }
}
