//! OS helpers for the media pipeline: an app-lifetime Job Object that kills child processes when the app exits (even
//! on a crash), free disk space, and best-effort deletion that skips locked files.
use std::path::Path;
use std::process::Child;

/// Put `child` in the app's kill-on-close Job Object: when the app process ends, Windows kills it. Every ffmpeg the
/// media pipeline starts goes through this; other long-lived helpers (e.g. a capture recorder) should call it too.
/// Returns false when the process could not be added (it still runs; it just is not tied to the app's lifetime).
pub fn adopt_child(child: &Child) -> bool {
    imp::adopt(child)
}

/// Kill every process in the app's Job Object now (app exit: before deleting the scratch files they hold open).
pub fn kill_adopted_children() {
    imp::kill_all();
}

/// Is a process with this id running? (Scratch folders of live processes, e.g. cs2 next to the app, are kept.)
pub fn pid_alive(pid: u32) -> bool {
    imp::pid_alive(pid)
}

/// Bytes available to this user on the volume holding `path` (or its nearest existing ancestor).
pub fn free_bytes(path: &Path) -> Option<u64> {
    let mut p = path;
    loop {
        if p.exists() {
            return imp::free_bytes(p);
        }
        p = p.parent()?;
    }
}

/// Delete a directory tree, continuing past files that cannot be removed (locked by a process that is still exiting).
/// Returns true when everything is gone.
pub fn remove_tree_best_effort(dir: &Path) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return !dir.exists() };
    let mut all = true;
    for e in rd.flatten() {
        let p = e.path();
        let ok = if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            remove_tree_best_effort(&p)
        } else {
            std::fs::remove_file(&p).is_ok()
        };
        all &= ok;
    }
    all && std::fs::remove_dir(dir).is_ok()
}

#[cfg(windows)]
mod imp {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::process::Child;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// The job handle (as usize), created once and never closed: the OS closes it when the app exits, which kills
    /// every process in the job.
    fn job() -> Option<HANDLE> {
        static JOB: OnceLock<usize> = OnceLock::new();
        let h = *JOB.get_or_init(|| unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return 0;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return 0;
            }
            h as usize
        });
        (h != 0).then_some(h as HANDLE)
    }

    pub fn adopt(child: &Child) -> bool {
        match job() {
            Some(j) => unsafe { AssignProcessToJobObject(j, child.as_raw_handle() as HANDLE) != 0 },
            None => false,
        }
    }

    pub fn pid_alive(pid: u32) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return false; // gone (or not ours to query: treated as gone; its files are skipped if locked)
            }
            let mut code = 0u32;
            let ok = GetExitCodeProcess(h, &mut code);
            CloseHandle(h);
            ok != 0 && code == STILL_ACTIVE as u32
        }
    }

    pub fn kill_all() {
        if let Some(j) = job() {
            unsafe {
                TerminateJobObject(j, 1);
            }
        }
    }

    pub fn free_bytes(p: &Path) -> Option<u64> {
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut avail = 0u64;
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) };
        (ok != 0).then_some(avail)
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;
    use std::process::Child;
    pub fn adopt(_: &Child) -> bool {
        false
    }
    pub fn kill_all() {}
    pub fn pid_alive(_: u32) -> bool {
        true
    }
    pub fn free_bytes(_: &Path) -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn free_space_and_best_effort_delete() {
        let d = std::env::temp_dir().join(format!("cs-sys-{}", std::process::id()));
        std::fs::create_dir_all(d.join("a")).unwrap();
        std::fs::write(d.join("a").join("x"), b"1").unwrap();
        assert!(free_bytes(&d.join("missing").join("deeper")).unwrap_or(1) > 0);
        assert!(remove_tree_best_effort(&d));
        assert!(!d.exists());
    }

    /// A child adopted into the job dies with its parent: checked by spawning a helper process (this test binary with a
    /// marker env) that adopts a long `ping` and exits; the ping must be gone right after.
    #[cfg(windows)]
    #[test]
    fn adopted_children_die_with_the_app() {
        use std::process::{Command, Stdio};
        if std::env::var_os("CS_SYS_CHILD").is_some() {
            let c = Command::new("ping").args(["-n", "30", "127.0.0.1"]).stdout(Stdio::null()).spawn().unwrap();
            assert!(adopt_child(&c));
            println!("PID={}", c.id());
            std::process::exit(0); // the job handle closes with this process
        }
        let me = std::env::current_exe().unwrap();
        let out = Command::new(me)
            .args(["--exact", "media::sys::tests::adopted_children_die_with_the_app", "--nocapture", "--test-threads", "1"])
            .env("CS_SYS_CHILD", "1")
            .output()
            .unwrap();
        let s = String::from_utf8_lossy(&out.stdout);
        let pid: u32 = s.split("PID=").nth(1).and_then(|r| r.split_whitespace().next()).and_then(|v| v.parse().ok()).expect("child pid");
        // wait (up to 10 s) for the system to end the child when the job handle closed
        let t0 = std::time::Instant::now();
        let running = || String::from_utf8_lossy(&Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output().unwrap().stdout).contains(&pid.to_string());
        while running() && t0.elapsed() < std::time::Duration::from_secs(10) {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert!(!running(), "ping {pid} still running");
    }
}
