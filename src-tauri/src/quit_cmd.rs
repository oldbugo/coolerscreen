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
//! How: the running app owns a named event (`Local\CoolerScreenQuit-<scope>`, scope = build kind + app folder, so a test instance
//! never answers for the installed app). `--quit` creates two answer events (`...-ok`, `...-busy`), opens and signals the request
//! event (missing = no instance), and waits for one of the answers; "ok" is followed by waiting until the request event is gone
//! (the app has exited). Only kernel objects are used, never a file: a script started from a packaged app (Claude desktop, MSIX)
//! sees a private, virtualised copy of %APPDATA% and would never see a file the real app writes.
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, INFINITE};

pub const EXIT_QUIT: i32 = 0;
pub const EXIT_SLOW: i32 = 1;
pub const EXIT_BUSY: i32 = 2;
pub const EXIT_NONE: i32 = 3;

/// The request event's name: one per build kind and app folder (see `instance_id::scope`).
pub fn event_name() -> String {
    let scope = crate::instance_id::scope(cfg!(debug_assertions), Some(&cs_core::core::paths::base())).unwrap_or_default();
    format!(r"Local\CoolerScreenQuit-{scope}")
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn open(name: &str) -> HANDLE {
    unsafe { OpenEventW(EVENT_MODIFY_STATE | 0x0010_0000, 0, wide(name).as_ptr()) } // modify + SYNCHRONIZE
}

/// The `--quit` process: returns its exit code.
pub fn run_quit() -> i32 {
    let name = event_name();
    let req = open(&name);
    if req.is_null() {
        return EXIT_NONE;
    }
    // the answer events exist before the request is made
    let mk = |suffix: &str| -> HANDLE { unsafe { CreateEventW(std::ptr::null(), 1, 0, wide(&format!("{name}-{suffix}")).as_ptr()) } };
    let (ok, busy) = (mk("ok"), mk("busy"));
    if ok.is_null() || busy.is_null() {
        unsafe { CloseHandle(req) };
        return EXIT_NONE;
    }
    unsafe {
        SetEvent(req);
        CloseHandle(req);
    }
    // wait up to 8 s for an answer
    let mut code = EXIT_NONE;
    let hs = [ok, busy];
    let w = unsafe { windows_sys::Win32::System::Threading::WaitForMultipleObjects(2, hs.as_ptr(), 0, 8000) };
    if w == WAIT_OBJECT_0 {
        // asked and accepted: wait until the app is gone (its request event no longer exists)
        let t0 = std::time::Instant::now();
        code = loop {
            let h = open(&name);
            if h.is_null() {
                break EXIT_QUIT;
            }
            unsafe { CloseHandle(h) };
            if t0.elapsed() > std::time::Duration::from_secs(20) {
                break EXIT_SLOW;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        };
    } else if w == WAIT_OBJECT_0 + 1 {
        code = EXIT_BUSY;
    }
    unsafe {
        CloseHandle(ok);
        CloseHandle(busy);
    }
    code
}

/// In the running app: wait for `--quit` requests on a small thread (blocked on the event, no polling) and answer them. `busy` says
/// whether a job is running; `quit` runs the tray Quit path.
pub fn listen(busy: impl Fn() -> bool + Send + 'static, quit: impl Fn() + Send + 'static) {
    let name = event_name();
    let ev: HANDLE = unsafe { CreateEventW(std::ptr::null(), 0, 0, wide(&name).as_ptr()) }; // auto-reset, unsignalled
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
            let answer = |suffix: &str| {
                let h = open(&format!("{name}-{suffix}"));
                if !h.is_null() {
                    unsafe {
                        SetEvent(h);
                        CloseHandle(h);
                    }
                }
            };
            if busy() {
                cs_core::device::log("quit requested (--quit) but a job is running: refused");
                answer("busy");
                continue;
            }
            cs_core::device::log("quit requested (--quit): the tray Quit path");
            answer("ok");
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
        assert!(a.starts_with(r"Local\CoolerScreenQuit-") && a.len() > r"Local\CoolerScreenQuit-".len() + 16);
        assert_eq!(a, event_name());
    }
}
