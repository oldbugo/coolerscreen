//! The editor's digital twin: the latest live picture (the JPEG the engine sends, or with Live off the one it would send) for the
//! On screen editor to show, about 15 per second while the editor is open and looking. Nothing is produced unless the page has asked
//! within the last few seconds (`wanted`), so the cost is zero when the editor is closed, hidden or minimised.
//! The picture is the portrait 756x1680 frame the screen gets; the page turns it back with CSS (no work here).
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// How long after the page's last request frames are still produced.
pub const WANT_MS: u64 = 3000;
/// At most one published frame per this many ms (about 15 per second).
pub const MIN_GAP_MS: u64 = 60;

/// The state; one global instance serves the app, tests make their own.
pub struct Twin {
    frame: Mutex<Option<(u64, Arc<Vec<u8>>)>>,
    seq: AtomicU64,
    watched: AtomicU64,
    last_pub: AtomicU64,
}

static TWIN: Twin = Twin::new();

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl Twin {
    pub const fn new() -> Twin {
        Twin { frame: Mutex::new(None), seq: AtomicU64::new(0), watched: AtomicU64::new(0), last_pub: AtomicU64::new(0) }
    }
    pub fn touch(&self) {
        self.watched.store(now_ms(), Relaxed);
    }
    pub fn wanted(&self) -> bool {
        let w = self.watched.load(Relaxed);
        w != 0 && now_ms().saturating_sub(w) < WANT_MS
    }
    pub fn publish(&self, jpeg: &[u8]) {
        let now = now_ms();
        if now.saturating_sub(self.last_pub.load(Relaxed)) < MIN_GAP_MS {
            return;
        }
        self.last_pub.store(now, Relaxed);
        let seq = self.seq.fetch_add(1, Relaxed) + 1;
        *self.frame.lock().unwrap() = Some((seq, Arc::new(jpeg.to_vec())));
    }
    pub fn latest(&self, since: u64) -> Option<(u64, Arc<Vec<u8>>)> {
        self.frame.lock().unwrap().as_ref().filter(|(s, _)| *s > since).cloned()
    }
    pub fn clear(&self) {
        *self.frame.lock().unwrap() = None;
        self.watched.store(0, Relaxed);
    }
}

/// The page is looking (called for every request, and when the editor opens).
pub fn touch() {
    TWIN.touch()
}

/// Has the page asked recently? The sender copies frames only while this is true.
pub fn wanted() -> bool {
    TWIN.wanted()
}

/// Called by the sender for every frame it takes: keeps a copy of at most about 15 per second.
pub fn publish(jpeg: &[u8]) {
    TWIN.publish(jpeg)
}

/// The newest frame when it is newer than `since` (0: any).
pub fn latest(since: u64) -> Option<(u64, Arc<Vec<u8>>)> {
    TWIN.latest(since)
}

/// Forget the picture (the editor closed): the next editor does not start with an old one.
pub fn clear() {
    TWIN.clear()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_flow_only_while_watched_and_are_throttled() {
        let t = Twin::new();
        assert!(!t.wanted());
        t.touch();
        assert!(t.wanted());
        t.publish(&[1, 2, 3]);
        let (s1, f) = t.latest(0).unwrap();
        assert_eq!(&f[..], &[1, 2, 3]);
        t.publish(&[9]); // within the gap: dropped
        assert_eq!(t.latest(0).unwrap().0, s1);
        assert!(t.latest(s1).is_none(), "nothing newer");
        std::thread::sleep(std::time::Duration::from_millis(MIN_GAP_MS + 20));
        t.publish(&[4]);
        assert!(t.latest(s1).is_some());
        t.clear();
        assert!(!t.wanted() && t.latest(0).is_none());
    }
}
