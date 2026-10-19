//! Which program is in front (read-only queries; no injection, no hooks).

#[derive(Clone, Debug, PartialEq)]
pub struct Fg {
    pub pid: u32,
    /// "MonsterHunterWilds" (image name without .exe)
    pub name: String,
    /// The window covers its whole monitor (fullscreen or borderless fullscreen).
    pub fullscreen: bool,
}

// ---------------------------------------------------------------- pure helpers

/// Image file name without directory and without a trailing ".exe" (case-insensitive).
pub fn strip_exe(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let n = file.len();
    if n >= 4 && file.is_char_boundary(n - 4) && file[n - 4..].eq_ignore_ascii_case(".exe") {
        file[..n - 4].to_string()
    } else {
        file.to_string()
    }
}

/// Desktop / shell window classes that are never "a game in front".
pub fn is_shell_class(class: &str) -> bool {
    matches!(class, "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// `win` = (left, top, right, bottom) covers `mon` within `tol` pixels on every side.
pub fn rect_covers(win: (i32, i32, i32, i32), mon: (i32, i32, i32, i32), tol: i32) -> bool {
    win.0 <= mon.0 + tol && win.1 <= mon.1 + tol && win.2 >= mon.2 - tol && win.3 >= mon.3 - tol
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::Security::{
        AllocateAndInitializeSid, CheckTokenMembership, FreeSid, GetTokenInformation, TokenElevation, SECURITY_NT_AUTHORITY, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetClassNameW, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsIconic, IsWindowVisible};

    pub fn process_name(pid: u32) -> Option<String> {
        if pid == 0 {
            return None;
        }
        unsafe {
            let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len);
            CloseHandle(h);
            if ok == 0 || len == 0 {
                return None;
            }
            Some(super::strip_exe(&String::from_utf16_lossy(&buf[..(len as usize).min(buf.len())])))
        }
    }

    pub fn current() -> Option<super::Fg> {
        unsafe {
            let hwnd: HWND = GetForegroundWindow();
            if hwnd.is_null() || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == 0 || pid == std::process::id() {
                return None;
            }
            let mut cls = [0u16; 128];
            let n = GetClassNameW(hwnd, cls.as_mut_ptr(), cls.len() as i32);
            if n > 0 && super::is_shell_class(&String::from_utf16_lossy(&cls[..n as usize])) {
                return None;
            }
            let name = process_name(pid).unwrap_or_default();
            let mut wr: RECT = std::mem::zeroed();
            let mut fullscreen = false;
            if GetWindowRect(hwnd, &mut wr) != 0 {
                let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                let mut mi: MONITORINFO = std::mem::zeroed();
                mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                if !mon.is_null() && GetMonitorInfoW(mon, &mut mi) != 0 {
                    let m = mi.rcMonitor;
                    fullscreen = super::rect_covers((wr.left, wr.top, wr.right, wr.bottom), (m.left, m.top, m.right, m.bottom), 1);
                }
            }
            Some(super::Fg { pid, name, fullscreen })
        }
    }

    /// Is the current token a member of the built-in group `rid` (S-1-5-32-<rid>)? CheckTokenMembership with a null token
    /// uses the effective (possibly UAC-filtered) token of the thread/process.
    fn member_of_builtin(rid: u32) -> bool {
        unsafe {
            let mut sid = std::ptr::null_mut();
            // SECURITY_BUILTIN_DOMAIN_RID = 32
            if AllocateAndInitializeSid(&SECURITY_NT_AUTHORITY, 2, 32, rid, 0, 0, 0, 0, 0, 0, &mut sid) == 0 || sid.is_null() {
                return false;
            }
            let mut is_member = 0i32;
            let ok = CheckTokenMembership(std::ptr::null_mut(), sid, &mut is_member);
            FreeSid(sid);
            ok != 0 && is_member != 0
        }
    }

    pub fn in_performance_log_users() -> bool {
        member_of_builtin(559)
    }

    fn process_elevated() -> bool {
        unsafe {
            let mut tok: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
                return false;
            }
            let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut ret = 0u32;
            let ok = GetTokenInformation(tok, TokenElevation, &mut e as *mut _ as *mut _, std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut ret);
            CloseHandle(tok);
            ok != 0 && e.TokenIsElevated != 0
        }
    }

    pub fn may_start_event_sessions() -> bool {
        in_performance_log_users() || (member_of_builtin(544) && process_elevated())
    }
}

/// The foreground window's process (None for the desktop/shell, an invisible/minimised window, or no window). Never our own pid.
pub fn current() -> Option<Fg> {
    #[cfg(windows)]
    {
        win::current()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Image name (without .exe) of a process id; None when it cannot be opened.
pub fn process_name(pid: u32) -> Option<String> {
    #[cfg(windows)]
    {
        win::process_name(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        None
    }
}

/// true when the current user token is a member of BUILTIN\Performance Log Users (S-1-5-32-559) or of Administrators with an
/// elevated token (i.e. may start event sessions).
pub fn may_start_event_sessions() -> bool {
    #[cfg(windows)]
    {
        win::may_start_event_sessions()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// true when the current token is a member of "Performance Log Users" (the group the user has to join once).
pub fn in_performance_log_users() -> bool {
    #[cfg(windows)]
    {
        win::in_performance_log_users()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_never_panics_or_returns_us() {
        if let Some(fg) = current() {
            assert_ne!(fg.pid, std::process::id());
            assert_ne!(fg.pid, 0);
        }
    }

    #[cfg(windows)]
    #[test]
    fn own_process_name_is_known() {
        let n = process_name(std::process::id()).expect("own process name");
        assert!(!n.is_empty());
        assert!(!n.to_lowercase().ends_with(".exe"));
        assert_eq!(process_name(0), None);
    }

    #[test]
    fn membership_calls_return() {
        let a = in_performance_log_users();
        let b = may_start_event_sessions();
        assert!(!a || b, "group member implies may start");
    }

    #[test]
    fn helpers() {
        assert_eq!(strip_exe(r"C:\Games\MonsterHunterWilds.EXE"), "MonsterHunterWilds");
        assert_eq!(strip_exe("a.exe"), "a");
        assert_eq!(strip_exe("tool"), "tool");
        assert_eq!(strip_exe(".exe"), "");
        assert_eq!(strip_exe("é.exe"), "é");
        assert!(is_shell_class("Progman") && is_shell_class("WorkerW") && is_shell_class("Shell_TrayWnd"));
        assert!(!is_shell_class("UnrealWindow"));
        let mon = (0, 0, 1920, 1080);
        assert!(rect_covers((0, 0, 1920, 1080), mon, 1));
        assert!(rect_covers((-1, -1, 1921, 1081), mon, 1));
        assert!(rect_covers((1, 1, 1919, 1079), mon, 1));
        assert!(!rect_covers((0, 0, 1920, 1040), mon, 1));
        assert!(!rect_covers((100, 100, 1000, 800), mon, 1));
    }
}
