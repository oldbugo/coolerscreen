//! Our own ETW session for present events (Microsoft-Windows-DXGI and -D3D9; the way Intel PresentMon sees frames).
//!
//! Rules: a fixed, descriptive session name; real-time mode; NEVER the NT Kernel Logger and never a system-logger session; the
//! session is created EMPTY (no providers) so it exists before a game with an anti-cheat starts (PresentMon issue #573: after some
//! anti-cheats start, StartTrace fails with ACCESS_DENIED while EnableTraceEx2 on an existing session still works); providers are
//! enabled only on demand; an orphan session of our own name (real-time sessions outlive a crashed process) is stopped before
//! creating; the session is stopped on `stop()` / drop.
//!
//! Providers and ids (verified in PresentMon `PresentData/ETW/Microsoft_Windows_DXGI.h` and `Microsoft_Windows_D3D9.h`):
//! DXGI {CA11C036-0102-4A2D-A6AD-F03CFED5D3C9} Present_Start = 0x2a (42); D3D9 {783ACA0A-790E-4D7F-8451-AA850511C6B9}
//! Present_Start = 0x1. We count the Start (the call), not the Stop.
//!
//! Scope: Vulkan and OpenGL games (which do not present through DXGI/D3D9 user-mode providers; PresentMon sees them through
//! DxgKrnl events) are OUT OF SCOPE for now. DX12 presents through DXGI, so it is covered.
//!
//! Timestamps: the session uses the PerfCounter clock (`ClientContext = 1`) and the consumer asks for raw timestamps, so the event
//! header TimeStamp is QueryPerformanceCounter ticks; converted to ns with QueryPerformanceFrequency (see `now_ns`).
use super::counter::{Counter, Src};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::core::{GUID, PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, ERROR_SUCCESS};
use windows::Win32::System::Diagnostics::Etw::*;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub const SESSION_NAME: &str = "CoolerScreen-GameFPS";

/// Microsoft-Windows-DXGI.
pub const DXGI_GUID: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
/// Microsoft-Windows-D3D9.
pub const D3D9_GUID: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
pub const DXGI_PRESENT_START: u16 = 42;
pub const D3D9_PRESENT_START: u16 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtwError {
    /// ERROR_ACCESS_DENIED (5): not an administrator and not in "Performance Log Users".
    AccessDenied,
    /// Any other Win32 error code.
    Other(u32),
}

fn map_err(code: u32) -> EtwError {
    if code == ERROR_ACCESS_DENIED.0 {
        EtwError::AccessDenied
    } else {
        EtwError::Other(code)
    }
}

// ---------------------------------------------------------------------------------------------------------------------------
// Pure helpers

/// Which counter source a (provider, event id) pair is, if it is a Present_Start we count.
pub fn classify(provider_guid: &GUID, event_id: u16) -> Option<Src> {
    if *provider_guid == DXGI_GUID && event_id == DXGI_PRESENT_START {
        Some(Src::Dxgi)
    } else if *provider_guid == D3D9_GUID && event_id == D3D9_PRESENT_START {
        Some(Src::D3d9)
    } else {
        None
    }
}

/// QPC ticks to nanoseconds without overflow (u128 intermediate). Negative ticks and a zero frequency give 0.
pub fn ticks_to_ns(ticks: i64, freq: u64) -> u64 {
    if ticks <= 0 || freq == 0 {
        return 0;
    }
    let ns = (ticks as u128) * 1_000_000_000u128 / freq as u128;
    ns.min(u64::MAX as u128) as u64
}

/// Turn one raw event into a feed call. Returns true when the event was fed. Ignores other events, pid 0 / 4 (idle / System) and
/// our own process.
pub fn on_event(
    counter: &mut Counter,
    provider_guid: &GUID,
    event_id: u16,
    pid: u32,
    timestamp_ticks: i64,
    freq: u64,
    own_pid: u32,
) -> bool {
    if pid == 0 || pid == 4 || pid == own_pid {
        return false;
    }
    match classify(provider_guid, event_id) {
        Some(src) => {
            counter.feed(pid, ticks_to_ns(timestamp_ticks, freq), src);
            true
        }
        None => false,
    }
}

/// An aligned, zero-initialised buffer holding EVENT_TRACE_PROPERTIES followed by the logger name (UTF-16, NUL terminated).
pub struct PropsBuf {
    buf: Vec<u64>,
    pub name_offset: usize,
    pub total_bytes: usize,
}

impl PropsBuf {
    pub fn as_ptr(&mut self) -> *mut EVENT_TRACE_PROPERTIES {
        self.buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES
    }
}

/// Build the properties for a real-time, file-less session: 64 KB buffers, 2-8 of them, 1 s flush timer, PerfCounter clock.
pub fn build_properties(name: &str) -> PropsBuf {
    let name_w: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let base = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
    let total = base + name_w.len() * 2;
    let mut buf = vec![0u64; (total + 7) / 8];
    let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
    // SAFETY: buf is 8-byte aligned (EVENT_TRACE_PROPERTIES needs 8) and at least `total` bytes; the name is copied after the struct.
    unsafe {
        (*p).Wnode.BufferSize = total as u32;
        (*p).Wnode.Flags = WNODE_FLAG_TRACED_GUID;
        (*p).Wnode.ClientContext = 1; // QueryPerformanceCounter
        (*p).BufferSize = 64; // KB
        (*p).MinimumBuffers = 2;
        (*p).MaximumBuffers = 8;
        (*p).FlushTimer = 1; // seconds
        (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
        (*p).LoggerNameOffset = base as u32;
        (*p).LogFileNameOffset = 0;
        let dst = (buf.as_mut_ptr() as *mut u8).add(base) as *mut u16;
        std::ptr::copy_nonoverlapping(name_w.as_ptr(), dst, name_w.len());
    }
    PropsBuf { buf, name_offset: base, total_bytes: total }
}

fn qpc_freq() -> u64 {
    let mut f = 0i64;
    // SAFETY: valid out pointer.
    unsafe {
        let _ = QueryPerformanceFrequency(&mut f);
    }
    f.max(0) as u64
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------------------------------------------------------------------
// Session

struct Ctx {
    sink: Arc<Mutex<Counter>>,
    own_pid: u32,
    freq: u64,
}

pub struct EtwSession {
    /// ControlTrace handle value (for EnableTraceEx2 / stop).
    session: u64,
    /// OpenTrace handle value.
    trace: u64,
    ctx: *mut Ctx,
    thread: Option<std::thread::JoinHandle<()>>,
    done: Arc<AtomicBool>,
    stopped: bool,
}

// SAFETY: `ctx` is only dereferenced by the ETW callback (read-only; the contents are Send+Sync) and freed in stop().
unsafe impl Send for EtwSession {}

unsafe extern "system" fn callback(rec: *mut EVENT_RECORD) {
    let _ = crate::core::sync::guarded("game fps event", || {
        if rec.is_null() {
            return;
        }
        // SAFETY: ETW passes a valid record for the duration of the call; UserContext is our leaked Ctx, alive until the
        // processing thread has exited.
        let r = &*rec;
        let ctx = r.UserContext as *const Ctx;
        if ctx.is_null() {
            return;
        }
        let ctx = &*ctx;
        let id = r.EventHeader.EventDescriptor.Id;
        // Cheap pre-filter before touching the mutex.
        if classify(&r.EventHeader.ProviderId, id).is_none() {
            return;
        }
        if let Ok(mut c) = ctx.sink.lock() {
            on_event(
                &mut c,
                &r.EventHeader.ProviderId,
                id,
                r.EventHeader.ProcessId,
                r.EventHeader.TimeStamp,
                ctx.freq,
                ctx.own_pid,
            );
        }
    });
}

fn stop_by_name() -> u32 {
    let mut props = build_properties(SESSION_NAME);
    let name = wide(SESSION_NAME);
    // SAFETY: props is a valid, aligned properties buffer; name is NUL terminated.
    unsafe { ControlTraceW(CONTROLTRACE_HANDLE { Value: 0 }, PCWSTR(name.as_ptr()), props.as_ptr(), EVENT_TRACE_CONTROL_STOP).0 }
}

impl EtwSession {
    /// Create (or take over) our named session and start consuming into `sink`. Providers stay disabled.
    pub fn start(sink: Arc<Mutex<Counter>>) -> Result<EtwSession, EtwError> {
        let name = wide(SESSION_NAME);
        let mut handle = CONTROLTRACE_HANDLE { Value: 0 };
        let mut attempt = 0;
        loop {
            let mut props = build_properties(SESSION_NAME);
            // SAFETY: valid handle out pointer, NUL-terminated name, aligned properties buffer.
            let rc = unsafe { StartTraceW(&mut handle, PCWSTR(name.as_ptr()), props.as_ptr()).0 };
            if rc == ERROR_SUCCESS.0 {
                break;
            }
            if rc == ERROR_ALREADY_EXISTS.0 && attempt == 0 {
                // Left over from a crash of an earlier run: stop our own session by name and retry once.
                attempt += 1;
                let _ = stop_by_name();
                continue;
            }
            return Err(map_err(rc));
        }
        let session = handle.Value;

        let ctx = Box::into_raw(Box::new(Ctx { sink, own_pid: std::process::id(), freq: qpc_freq() }));

        // Open the consumer side.
        let mut logname = wide(SESSION_NAME);
        // SAFETY: zeroed EVENT_TRACE_LOGFILEW is a valid "empty" value; the name buffer outlives OpenTraceW (which copies it
        // into the trace handle's state).
        let trace = unsafe {
            let mut lf: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
            lf.LoggerName = PWSTR(logname.as_mut_ptr());
            lf.Anonymous1.ProcessTraceMode =
                PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
            lf.Anonymous2.EventRecordCallback = Some(callback);
            lf.Context = ctx as *mut c_void;
            OpenTraceW(&mut lf).Value
        };
        if trace == u64::MAX || trace == 0 {
            let rc = unsafe { windows::Win32::Foundation::GetLastError().0 };
            let _ = stop_by_name();
            // SAFETY: no consumer was started, so nothing else references ctx.
            drop(unsafe { Box::from_raw(ctx) });
            return Err(map_err(rc));
        }

        let done = Arc::new(AtomicBool::new(false));
        let done2 = done.clone();
        let spawned = std::thread::Builder::new().name("etw-gamefps".into()).spawn(move || {
            let h = [PROCESSTRACE_HANDLE { Value: trace }];
            // SAFETY: handle from OpenTraceW; blocks until the session is stopped / the trace closed.
            unsafe {
                let _ = ProcessTrace(&h, None, None);
            }
            done2.store(true, Ordering::SeqCst);
        });
        let thread = match spawned {
            Ok(t) => t,
            Err(_) => {
                unsafe {
                    let _ = CloseTrace(PROCESSTRACE_HANDLE { Value: trace });
                }
                let _ = stop_by_name();
                // SAFETY: ProcessTrace never ran.
                drop(unsafe { Box::from_raw(ctx) });
                return Err(EtwError::Other(0));
            }
        };
        Ok(EtwSession { session, trace, ctx, thread: Some(thread), done, stopped: false })
    }

    /// Enable DXGI and D3D9 (level information, all keywords: what PresentMon does for these two).
    pub fn enable_providers(&self) -> Result<(), EtwError> {
        for g in [DXGI_GUID, D3D9_GUID] {
            // SAFETY: valid session handle and GUID pointer.
            let rc = unsafe {
                EnableTraceEx2(
                    CONTROLTRACE_HANDLE { Value: self.session },
                    &g,
                    EVENT_CONTROL_CODE_ENABLE_PROVIDER.0,
                    TRACE_LEVEL_INFORMATION as u8,
                    u64::MAX,
                    0,
                    0,
                    None,
                )
                .0
            };
            if rc != ERROR_SUCCESS.0 {
                self.disable_providers();
                return Err(map_err(rc));
            }
        }
        Ok(())
    }

    /// Disable both providers again; the session stays.
    pub fn disable_providers(&self) {
        for g in [DXGI_GUID, D3D9_GUID] {
            // SAFETY: as above.
            unsafe {
                let _ = EnableTraceEx2(
                    CONTROLTRACE_HANDLE { Value: self.session },
                    &g,
                    EVENT_CONTROL_CODE_DISABLE_PROVIDER.0,
                    0,
                    0,
                    0,
                    0,
                    None,
                );
            }
        }
    }

    /// Stop the session (idempotent).
    pub fn stop(&mut self) {
        if self.stopped {
            return;
        }
        self.stopped = true;
        self.disable_providers();
        let _ = stop_by_name();
        // SAFETY: handle from OpenTraceW.
        unsafe {
            let _ = CloseTrace(PROCESSTRACE_HANDLE { Value: self.trace });
        }
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.done.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        if self.done.load(Ordering::SeqCst) {
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
            // SAFETY: the processing thread has exited, so the callback can no longer run and nothing references ctx.
            drop(unsafe { Box::from_raw(self.ctx) });
            self.ctx = std::ptr::null_mut();
        }
        // else: the thread is stuck; leak ctx (a few bytes) rather than risk a use-after-free in the callback.
    }

    /// The clock the events use, in ns, "now" (so the sampler can ask `Counter` for statistics as of now).
    pub fn now_ns() -> u64 {
        let mut t = 0i64;
        // SAFETY: valid out pointer.
        unsafe {
            let _ = QueryPerformanceCounter(&mut t);
        }
        ticks_to_ns(t, qpc_freq())
    }
}

impl Drop for EtwSession {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guids_and_ids() {
        assert_eq!(classify(&DXGI_GUID, 42), Some(Src::Dxgi));
        assert_eq!(classify(&D3D9_GUID, 1), Some(Src::D3d9));
        assert_eq!(classify(&DXGI_GUID, 43), None); // Present_Stop
        assert_eq!(classify(&D3D9_GUID, 2), None);
        assert_eq!(classify(&DXGI_GUID, 1), None);
        assert_eq!(classify(&D3D9_GUID, 42), None);
        assert_eq!(classify(&GUID::zeroed(), 42), None);
        assert_eq!(format!("{:?}", DXGI_GUID).to_uppercase(), "CA11C036-0102-4A2D-A6AD-F03CFED5D3C9");
        assert_eq!(format!("{:?}", D3D9_GUID).to_uppercase(), "783ACA0A-790E-4D7F-8451-AA850511C6B9");
    }

    #[test]
    fn tick_conversion() {
        assert_eq!(ticks_to_ns(10_000_000, 10_000_000), 1_000_000_000);
        assert_eq!(ticks_to_ns(5, 10_000_000), 500);
        assert_eq!(ticks_to_ns(-1, 10_000_000), 0);
        assert_eq!(ticks_to_ns(100, 0), 0);
        // ~30 years of uptime at 10 MHz: would overflow a naive u64 multiply.
        let t = 10_000_000i64 * 86_400 * 365 * 30;
        assert_eq!(ticks_to_ns(t, 10_000_000), 86_400u64 * 365 * 30 * 1_000_000_000);
        assert_eq!(ticks_to_ns(i64::MAX, 1), u64::MAX);
    }

    #[test]
    fn on_event_filters() {
        let mut c = Counter::new();
        assert!(on_event(&mut c, &DXGI_GUID, 42, 1234, 10_000_000, 10_000_000, 999));
        assert!(on_event(&mut c, &D3D9_GUID, 1, 1234, 10_000_000, 10_000_000, 999));
        assert!(!on_event(&mut c, &DXGI_GUID, 43, 1234, 1, 10_000_000, 999));
        assert!(!on_event(&mut c, &DXGI_GUID, 42, 0, 1, 10_000_000, 999));
        assert!(!on_event(&mut c, &DXGI_GUID, 42, 4, 1, 10_000_000, 999));
        assert!(!on_event(&mut c, &DXGI_GUID, 42, 999, 1, 10_000_000, 999));
    }

    #[test]
    fn properties_layout() {
        let mut p = build_properties(SESSION_NAME);
        let base = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
        assert_eq!(p.name_offset, base);
        assert_eq!(p.total_bytes, base + (SESSION_NAME.len() + 1) * 2);
        let ptr = p.as_ptr();
        assert_eq!(ptr as usize % std::mem::align_of::<EVENT_TRACE_PROPERTIES>(), 0);
        unsafe {
            assert_eq!((*ptr).Wnode.BufferSize as usize, p.total_bytes);
            assert_eq!((*ptr).Wnode.ClientContext, 1);
            assert_eq!((*ptr).LogFileMode, EVENT_TRACE_REAL_TIME_MODE);
            assert_eq!((*ptr).LoggerNameOffset as usize, base);
            assert_eq!((*ptr).LogFileNameOffset, 0);
            assert_eq!((*ptr).FlushTimer, 1);
            let n = (ptr as *const u8).add(base) as *const u16;
            let s: Vec<u16> = (0..SESSION_NAME.len()).map(|i| *n.add(i)).collect();
            assert_eq!(String::from_utf16(&s).unwrap(), SESSION_NAME);
            assert_eq!(*n.add(SESSION_NAME.len()), 0);
        }
        // No system logger / kernel logger bits.
        assert_eq!(unsafe { (*ptr).LogFileMode } & EVENT_TRACE_SYSTEM_LOGGER_MODE, 0);
    }

    #[test]
    fn now_is_monotonic_and_nonzero() {
        let a = EtwSession::now_ns();
        let b = EtwSession::now_ns();
        assert!(a > 0 && b >= a);
    }

    #[test]
    fn start_is_denied_or_ok_and_stop_is_idempotent() {
        let sink = Arc::new(Mutex::new(Counter::new()));
        match EtwSession::start(sink) {
            Ok(mut s) => {
                s.stop();
                s.stop();
            }
            Err(e) => assert_eq!(e, EtwError::AccessDenied),
        }
    }
}
