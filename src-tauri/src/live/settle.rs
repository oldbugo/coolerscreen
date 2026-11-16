//! Settle wait after a stop (4.23.0). A Live start soon after our own stop has made the screen's firmware hang
//! (6, 11 and 17 Nov 2026: the first connect about 8 s after an update stop). Connects with the stored clip already
//! playing for minutes never did. So the first connect after a stop waits until `settle` has passed since that stop.
//! The stop time is kept in a small file in the app folder, so it also covers a new process after an update or a
//! worker restart. No USB calls here: the commands, their order and their timings are unchanged.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Default wait in seconds (a Live setting, `settleAfterStopSeconds`).
pub const DEFAULT_SECONDS: u32 = 30;
/// Largest allowed wait in seconds.
pub const MAX_SECONDS: u32 = 120;

fn path() -> PathBuf {
    crate::core::paths::base().join("live-last-stop.txt")
}

/// Note that Live has just ended a stream on the screen.
pub fn record_stop() {
    record_stop_at(&path(), SystemTime::now());
}

/// How much longer the next connect must wait (zero when it may start now).
pub fn remaining(settle: Duration) -> Duration {
    remaining_at(&path(), settle, SystemTime::now())
}

fn record_stop_at(p: &Path, now: SystemTime) {
    let Ok(since_epoch) = now.duration_since(UNIX_EPOCH) else { return };
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, since_epoch.as_millis().to_string());
}

fn remaining_at(p: &Path, settle: Duration, now: SystemTime) -> Duration {
    if settle.is_zero() {
        return Duration::ZERO;
    }
    let Ok(text) = std::fs::read_to_string(p) else { return Duration::ZERO };
    let Ok(ms) = text.trim().parse::<u64>() else { return Duration::ZERO };
    let stopped = UNIX_EPOCH + Duration::from_millis(ms);
    match now.duration_since(stopped) {
        Ok(since) => settle.saturating_sub(since),
        // A stop time in the future means the clock was changed: do not wait.
        Err(_) => Duration::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("cs-settle-tests");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(name);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn no_stop_recorded_means_no_wait() {
        let p = temp("none.txt");
        assert_eq!(remaining_at(&p, Duration::from_secs(30), SystemTime::now()), Duration::ZERO);
    }

    #[test]
    fn waits_the_rest_of_the_settle_time_after_a_stop() {
        let p = temp("recent.txt");
        let stop = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        record_stop_at(&p, stop);
        let settle = Duration::from_secs(30);
        assert_eq!(remaining_at(&p, settle, stop + Duration::from_secs(8)), Duration::from_secs(22));
        assert_eq!(remaining_at(&p, settle, stop + Duration::from_secs(30)), Duration::ZERO);
        assert_eq!(remaining_at(&p, settle, stop + Duration::from_secs(600)), Duration::ZERO);
    }

    #[test]
    fn zero_setting_broken_file_or_future_stop_time_never_wait() {
        let p = temp("edge.txt");
        let stop = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
        record_stop_at(&p, stop);
        assert_eq!(remaining_at(&p, Duration::ZERO, stop), Duration::ZERO);
        assert_eq!(remaining_at(&p, Duration::from_secs(30), stop - Duration::from_secs(5)), Duration::ZERO);
        std::fs::write(&p, "not a time").unwrap();
        assert_eq!(remaining_at(&p, Duration::from_secs(30), stop), Duration::ZERO);
    }

    #[test]
    fn production_path_is_in_the_app_folder() {
        assert_eq!(path(), crate::core::paths::base().join("live-last-stop.txt"));
    }
}
