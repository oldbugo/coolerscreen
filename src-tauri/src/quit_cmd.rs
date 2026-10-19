//! `CoolerScreen.exe --quit`: ask the running CoolerScreen (the one that uses the same app folder) to quit the way the tray's Quit
//! does: live stops with the normal hand-back to Embedded, bounded waits, the exit watchdog. For installers and scripts, because a
//! plain `taskkill /PID` (without /F) only closes the window and the app stays in the tray.
//!
//! No window is shown, nothing is started when no app is running. Exit codes of the `--quit` process:
//!   0  the running app quit
//!   1  it was asked but did not exit within 20 s (it has its own 3 s exit watchdog, so this is unusual)
//!   2  it refused: a job is running (upload, conversion, store, recording); nothing was done
//!   3  no running instance
//!
//! How: the running app owns a named event (`Local\CoolerScreenQuit-<hash of its app folder>`, so a test instance with its own
//! COOLERSCREEN_HOME never answers for the installed app). `--quit` opens it (missing = no instance), signals it, and reads the
//! answer from `quit.result` in the app folder ("ok <pid>" or "busy"); then it waits for that process to end.
use std::path::PathBuf;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateEventW, OpenEventW, OpenProcess, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE};
const SYNCHRONIZE: u32 = 0x0010_0000;

pub const EXIT_QUIT: i32 = 0;
pub const EXIT_SLOW: i32 = 1;
pub const EXIT_BUSY: i32 = 2;
pub const EXIT_NONE: i32 = 3;

fn base() -> PathBuf {
    cs_core::core::paths::base()
}

fn result_file() -> PathBuf {
    base().join("quit.result")
}

/// The event's name: one per app folder (case-insensitive path, FNV-1a).
pub fn event_name() -> String {
    // the same scope as the single-instance lock: build kind + app folder (a debug build never answers for a release one)
    let scope = crate::instance_id::scope(cfg!(debug_assertions), Some(&base())).unwrap_or_default();
    format!(r"Local\CoolerScreenQuit-{scope}")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The `--quit` process: returns its exit code.
pub fn run_quit() -> i32 {
    let name = wide(&event_name());
    let ev: HANDLE = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
    if ev.is_null() {
        return EXIT_NONE;
    }
    let _ = std::fs::remove_file(result_file());
    unsafe {
        SetEvent(ev);
        CloseHandle(ev);
    }
    // the answer comes within a moment
    let t0 = Instant::now();
    let answer = loop {
        if let Ok(s) = std::fs::read_to_string(result_file()) {
            if !s.trim().is_empty() {
                break s;
            }
        }
        if t0.elapsed() > Duration::from_secs(8) {
            return EXIT_NONE; // the event exists but nobody answered: treat as not running
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let _ = std::fs::remove_file(result_file());
    let answer = answer.trim();
    if answer == "busy" {
        return EXIT_BUSY;
    }
    let pid: u32 = answer.strip_prefix("ok ").and_then(|p| p.trim().parse().ok()).unwrap_or(0);
    if pid == 0 {
        return EXIT_NONE;
    }
    let h = unsafe { OpenProcess(SYNCHRONIZE, 0, pid) };
    if h.is_null() {
        return EXIT_QUIT; // already gone
    }
    let w = unsafe { WaitForSingleObject(h, 20_000) };
    unsafe { CloseHandle(h) };
    if w == WAIT_OBJECT_0 { EXIT_QUIT } else { EXIT_SLOW }
}

/// In the running app: wait for `--quit` requests on a small thread (blocked on the event, no polling) and answer them. `busy` says
/// whether a job is running; `quit` runs the tray Quit path.
pub fn listen(busy: impl Fn() -> bool + Send + 'static, quit: impl Fn() + Send + 'static) {
    let name = wide(&event_name());
    let ev: HANDLE = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) }; // auto-reset, unsignalled
    if ev.is_null() {
        return;
    }
    struct H(HANDLE);
    unsafe impl Send for H {}
    let ev = H(ev);
    std::thread::spawn(move || {
        let ev = ev;
        loop {
            if unsafe { WaitForSingleObject(ev.0, INFINITE) } != WAIT_OBJECT_0 {
                return;
            }
            let _ = std::fs::create_dir_all(base());
            if busy() {
                cs_core::device::log("quit requested (--quit) but a job is running: refused");
                let _ = std::fs::write(result_file(), "busy");
                continue;
            }
            cs_core::device::log("quit requested (--quit): the tray Quit path");
            let _ = std::fs::write(result_file(), format!("ok {}", std::process::id()));
            quit();
            return;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_event_name_follows_the_app_folder() {
        let a = event_name();
        assert!(a.starts_with("Local\\CoolerScreenQuit-") && a.len() > "Local\\CoolerScreenQuit-".len() + 16);
        assert_eq!(a, event_name());
    }
}
