//! Send 0x15 [00 0A] only once (4.24.0). The screen stores this setting ("keep the display on after Live") in its own
//! memory, so sending it at every Live start is not needed. Every screen hang seen (6, 11, 14 and 17 Nov 2026) started
//! at enter-Live, the step right after this setting, and the screen is busy for about 200 ms after it even on good
//! starts. So after one start where the screen took it, the engine leaves it out. A small file in the app folder keeps
//! that fact, so it also holds after an update or a worker restart. Delete the file to send the setting once more.
use std::path::{Path, PathBuf};

/// The setting the engine sends (0x15 payload).
pub const PAYLOAD: [u8; 2] = [0x00, 0x0a];

fn path() -> PathBuf {
    crate::core::paths::base().join("live-ending-stored.txt")
}

/// True when the screen has already taken this setting (it is stored, so it need not be sent again).
pub fn stored() -> bool {
    stored_at(&path())
}

/// Note that the screen took the setting.
pub fn record_stored() {
    record_at(&path());
}

fn text() -> String {
    format!("0x15 {:02x} {:02x}", PAYLOAD[0], PAYLOAD[1])
}

fn stored_at(p: &Path) -> bool {
    std::fs::read_to_string(p).is_ok_and(|t| t.trim() == text())
}

fn record_at(p: &Path) {
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(p, text());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("cs-preserve-tests");
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join(name);
        let _ = std::fs::remove_file(&p);
        p
    }

    #[test]
    fn not_stored_until_recorded_then_stored() {
        let p = temp("record.txt");
        assert!(!stored_at(&p));
        record_at(&p);
        assert!(stored_at(&p));
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "0x15 00 0a");
    }

    #[test]
    fn a_different_or_broken_record_means_send_again() {
        let p = temp("other.txt");
        std::fs::write(&p, "0x15 00 05").unwrap();
        assert!(!stored_at(&p));
        std::fs::write(&p, "").unwrap();
        assert!(!stored_at(&p));
    }

    #[test]
    fn production_path_is_in_the_app_folder() {
        assert_eq!(path(), crate::core::paths::base().join("live-ending-stored.txt"));
    }
}
