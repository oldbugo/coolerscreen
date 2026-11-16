//! The sender's current connection state. No USB calls: safe for UI snapshots and deterministic tests.
use crate::device::DevErr;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub struct Connection {
    state: &'static str,
    message: Option<String>,
    attempts: u64,
    retry_at: Option<Instant>,
    delay: u64,
    last_log: Option<(&'static str, String, Instant)>,
    recovery_used: bool,
}

impl Connection {
    pub fn new(connected: bool) -> Self {
        Self { state: if connected { "connected" } else { "connecting" }, message: None, attempts: 0,
            retry_at: None, delay: 1, last_log: None, recovery_used: false }
    }
    pub fn due(&self, now: Instant) -> bool { self.retry_at.map_or(true, |t| now >= t) }
    pub fn begin(&mut self) { self.attempts += 1; }
    /// Return a log line only on a changed failure or a five-minute reminder.
    pub fn failed(&mut self, e: &DevErr, now: Instant) -> Option<String> {
        self.state = match e.code.as_deref() {
            Some("notFound") => "notFound", Some("failedUsb") => "failedUsb", Some("busy") => "busy",
            Some("noReply") => "noReply", _ => "error",
        };
        self.message = Some(e.message.clone());
        self.retry_at = Some(now + Duration::from_secs(self.delay));
        let log = self.last_log.as_ref().map_or(true, |(s, m, t)| *s != self.state || m != &e.message || now.duration_since(*t) >= Duration::from_secs(300));
        let line = log.then(|| format!("cannot connect [{}] ({}); retry in {} s; attempt {}", self.state, e.message, self.delay, self.attempts));
        if log { self.last_log = Some((self.state, e.message.clone(), now)); }
        self.delay = (self.delay * 2).min(15);
        line
    }
    pub fn connected(&mut self) {
        self.state = "connected"; self.message = None; self.attempts = 0;
        self.retry_at = None; self.delay = 1; self.last_log = None;
    }
    pub fn paused(&mut self) {
        self.state = "paused"; self.message = None; self.attempts = 0;
        self.retry_at = None; self.delay = 1; self.last_log = None;
    }
    pub fn resume(&mut self) { if self.state == "paused" { self.state = "connecting"; } }
    /// The last stop was moments ago: wait `left` before the next connect (state stays "connecting"; the countdown shows it).
    pub fn settling(&mut self, now: Instant, left: Duration) {
        self.state = "connecting";
        self.message = Some(format!("Letting the screen settle after the last stop ({} s)", left.as_secs() + u64::from(left.subsec_nanos() > 0)));
        self.retry_at = Some(now + left);
    }
    /// Conservative limit: one firmware restart for this pipeline lifetime, even through pauses/recovery.
    pub fn request_recovery(&mut self, e: &DevErr) -> bool {
        if e.code.as_deref() != Some("noReply") || self.recovery_used { return false; }
        self.recovery_used = true;
        true
    }
    pub fn recovery_deferred(&mut self) { self.recovery_used = false; }
    pub fn snapshot(&self, now: Instant) -> Value {
        let remaining = self.retry_at.map(|t| t.saturating_duration_since(now)).unwrap_or_default();
        json!({ "state": self.state, "message": self.message, "attempts": self.attempts,
            "retryInSeconds": remaining.as_secs() + u64::from(remaining.subsec_nanos() > 0) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn err(code: &str) -> DevErr { DevErr { code: Some(code.into()), message: code.into(), failed_usb: code == "failedUsb" } }
    #[test]
    fn retries_are_bounded_and_countdown_is_current() {
        let mut c = Connection::new(false); let mut now = Instant::now();
        for delay in [1, 2, 4, 8, 15, 15] {
            assert!(c.due(now)); c.begin(); c.failed(&err("notFound"), now);
            assert_eq!(c.snapshot(now)["retryInSeconds"], delay);
            assert!(!c.due(now + Duration::from_millis(1)));
            now += Duration::from_secs(delay); assert!(c.due(now));
        }
        assert_eq!(c.snapshot(now)["attempts"], 6);
    }
    #[test]
    fn repeated_failures_are_quiet_but_transitions_and_reminders_are_logged() {
        let mut c = Connection::new(false); let now = Instant::now();
        assert!(c.failed(&err("notFound"), now).is_some());
        assert!(c.failed(&err("notFound"), now + Duration::from_secs(15)).is_none());
        assert!(c.failed(&err("failedUsb"), now + Duration::from_secs(30)).is_some());
        assert!(c.failed(&err("failedUsb"), now + Duration::from_secs(330)).is_some());
    }
    #[test]
    fn success_and_pause_clear_errors_and_restart_backoff() {
        let mut c = Connection::new(false); let now = Instant::now();
        c.begin(); c.failed(&err("failedUsb"), now); c.connected();
        assert_eq!(c.snapshot(now), json!({"state":"connected","message":null,"attempts":0,"retryInSeconds":0}));
        c.begin(); c.failed(&err("notFound"), now); c.paused();
        assert_eq!(c.snapshot(now), json!({"state":"paused","message":null,"attempts":0,"retryInSeconds":0}));
        c.resume(); assert_eq!(c.snapshot(now)["state"], "connecting");
        c.failed(&err("busy"), now); assert_eq!(c.snapshot(now)["retryInSeconds"], 1);
    }
    #[test]
    fn settling_delays_the_connect_without_counting_a_failure() {
        let mut c = Connection::new(false); let now = Instant::now();
        c.settling(now, Duration::from_millis(21_500));
        let snap = c.snapshot(now);
        assert_eq!(snap["state"], "connecting"); assert_eq!(snap["attempts"], 0); assert_eq!(snap["retryInSeconds"], 22);
        assert!(!c.due(now + Duration::from_secs(21))); assert!(c.due(now + Duration::from_millis(21_500)));
        c.begin(); c.connected(); assert_eq!(c.snapshot(now)["message"], serde_json::Value::Null);
        c.settling(now, Duration::from_secs(5)); c.paused(); assert!(c.due(now));
    }
    #[test]
    fn only_verified_no_reply_can_request_one_restart() {
        let mut c = Connection::new(false);
        for code in ["notFound", "failedUsb", "busy", "stalled", "error"] { assert!(!c.request_recovery(&err(code))); }
        assert!(c.request_recovery(&err("noReply"))); c.paused(); c.connected();
        assert!(!c.request_recovery(&err("noReply")));
    }
    #[test]
    fn a_busy_scheduler_does_not_consume_recovery_eligibility() {
        let mut c = Connection::new(false);
        assert!(c.request_recovery(&err("noReply")));
        c.recovery_deferred();
        assert!(c.request_recovery(&err("noReply")));
        assert!(!c.request_recovery(&err("noReply")));
    }
}
