//! The shared metrics sampler for `sensor` and `info` widgets: ONE thread, started the first time a widget asks and ended when
//! nobody has asked for a few seconds. It polls only the metrics some widget currently shows, each at the fastest rate any
//! widget asked for (1, 2 or 4 Hz), and keeps a ring buffer of history per metric for graphs and sparklines.
//!
//! Sources (no admin, no driver): CPU load / per-core load (NtQuerySystemInformation), CPU clock (CallNtPowerInformation),
//! RAM (GlobalMemoryStatusEx), NVIDIA GPU load/temp/clock/VRAM/power/fan (NVML, loaded at run time), network (GetIfTable2),
//! disk space (GetDiskFreeSpaceEx), disk activity (PDH), top processes (toolhelp snapshot, only while shown).
//! CPU temperature has no reliable source without a kernel driver: it reads as missing (None) and widgets show "-".
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// How long a metric stays polled after the last time a widget asked for it.
const KEEP: Duration = Duration::from_secs(6);
/// Samples kept per metric (4 Hz x 300 s).
const RING: usize = 1200;

#[derive(Default)]
struct Slot {
    last_used: Option<Instant>,
    rate: u32,
    last_poll: Option<Instant>,
    value: Option<f32>,
    /// The natural upper end of the metric when it has one (total RAM, total VRAM, disk size) - same unit as `value`.
    total: Option<f32>,
    /// (when, value)
    ring: VecDeque<(Instant, f32)>,
    /// Counter state between polls: (idle, total) for CPU, (in bytes, out bytes) for the network, etc.
    prev: Option<(u64, u64)>,
    prev_net: HashMap<u32, (u64, u64)>,
    prev_at: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
pub struct ProcRow {
    pub name: String,
    pub cpu_pct: f32,
    pub ram_mb: f32,
}

#[derive(Default)]
struct Shared {
    slots: HashMap<String, Slot>,
    procs: Vec<ProcRow>,
    procs_used: Option<Instant>,
    cpu_name: Option<String>,
    gpu_name: Option<String>,
    running: bool,
}

pub struct Hub {
    s: Mutex<Shared>,
    stop: AtomicBool,
}

pub fn hub() -> &'static Arc<Hub> {
    static H: OnceLock<Arc<Hub>> = OnceLock::new();
    H.get_or_init(|| Arc::new(Hub { s: Mutex::new(Shared::default()), stop: AtomicBool::new(false) }))
}

/// The slot key of a sensor: the metric plus what selects within it (core, drive, adapter).
pub fn key(metric: &str, core: u32, drive: &str, adapter: Option<&str>) -> String {
    match metric {
        "cpu.core" => format!("cpu.core:{core}"),
        "disk.used" => format!("disk.used:{}", drive.trim().to_ascii_uppercase().chars().next().unwrap_or('C')),
        "net.down" | "net.up" => format!("{metric}:{}", adapter.unwrap_or("*")),
        m => m.to_string(),
    }
}

impl Hub {
    fn ensure_thread(self: &Arc<Self>, g: &mut Shared) {
        if g.running {
            return;
        }
        g.running = true;
        self.stop.store(false, Ordering::Relaxed);
        let me = self.clone();
        std::thread::spawn(move || me.run());
    }

    /// Ask for a metric (registers the demand): its latest value and total, or None when it cannot be read.
    pub fn read(self: &Arc<Self>, key: &str, rate: u32) -> (Option<f32>, Option<f32>) {
        let mut g = self.s.lock().unwrap();
        let first = !g.slots.contains_key(key);
        let slot = g.slots.entry(key.to_string()).or_default();
        slot.last_used = Some(Instant::now());
        slot.rate = slot.rate.max(rate.clamp(1, 4));
        let r = (slot.value, slot.total);
        let _ = first;
        self.ensure_thread(&mut g);
        r
    }

    /// History within the last `secs` seconds, oldest first, as (age in seconds, value).
    pub fn history(&self, key: &str, secs: f32) -> Vec<(f32, f32)> {
        let g = self.s.lock().unwrap();
        let now = Instant::now();
        g.slots.get(key).map(|s| s.ring.iter().map(|(t, v)| ((now - *t).as_secs_f32(), *v)).filter(|(a, _)| *a <= secs).collect()).unwrap_or_default()
    }

    pub fn top_processes(self: &Arc<Self>, by_ram: bool, count: usize) -> Vec<ProcRow> {
        let mut g = self.s.lock().unwrap();
        g.procs_used = Some(Instant::now());
        self.ensure_thread(&mut g);
        let mut v = g.procs.clone();
        v.sort_by(|a, b| {
            let (x, y) = if by_ram { (a.ram_mb, b.ram_mb) } else { (a.cpu_pct, b.cpu_pct) };
            y.partial_cmp(&x).unwrap_or(std::cmp::Ordering::Equal)
        });
        v.truncate(count);
        v
    }

    pub fn cpu_name(&self) -> Option<String> {
        let mut g = self.s.lock().unwrap();
        if g.cpu_name.is_none() {
            g.cpu_name = Some(read_cpu_name().unwrap_or_default());
        }
        g.cpu_name.clone().filter(|s| !s.is_empty())
    }

    pub fn gpu_name(self: &Arc<Self>) -> Option<String> {
        // the GPU name comes with the first GPU poll
        let _ = self.read("gpu.load", 1);
        self.s.lock().unwrap().gpu_name.clone()
    }

    /// Stop the thread now (it also ends by itself when nothing is asked for).
    pub fn shutdown(&self) {
        self.stop.store(true, Ordering::Relaxed);
    }

    fn run(self: Arc<Self>) {
        let mut src = Sources::new();
        let mut last_procs = Instant::now() - Duration::from_secs(60);
        let mut prev_proc: HashMap<u32, (u64, Instant)> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(125));
            if self.stop.load(Ordering::Relaxed) {
                break;
            }
            let now = Instant::now();
            // which slots are due
            let due: Vec<String> = {
                let g = self.s.lock().unwrap();
                g.slots
                    .iter()
                    .filter(|(_, s)| s.last_used.is_some_and(|t| now - t < KEEP))
                    .filter(|(_, s)| s.last_poll.is_none_or(|t| now - t >= Duration::from_millis(1000 / s.rate.max(1) as u64 - 20)))
                    .map(|(k, _)| k.clone())
                    .collect()
            };
            for k in due {
                // take the slot's counter state out, poll without holding the lock (the polls make system calls)
                let mut slot = {
                    let mut g = self.s.lock().unwrap();
                    match g.slots.get_mut(&k) {
                        Some(s) => std::mem::take(s),
                        None => continue,
                    }
                };
                let gpu_name = src.poll(&k, &mut slot, now);
                slot.last_poll = Some(now);
                if let Some(v) = slot.value {
                    slot.ring.push_back((now, v));
                    while slot.ring.len() > RING {
                        slot.ring.pop_front();
                    }
                }
                let mut g = self.s.lock().unwrap();
                if let Some(n) = gpu_name {
                    g.gpu_name = Some(n);
                }
                if let Some(s) = g.slots.get_mut(&k) {
                    // keep demand info set meanwhile by readers
                    let (lu, rate) = (s.last_used, s.rate);
                    *s = slot;
                    s.last_used = lu.or(s.last_used);
                    s.rate = rate.max(s.rate);
                }
            }
            // top processes: every 2 s while shown
            let procs_wanted = self.s.lock().unwrap().procs_used.is_some_and(|t| now - t < KEEP);
            if procs_wanted && now - last_procs >= Duration::from_secs(2) {
                let rows = sample_processes(&mut prev_proc, now);
                last_procs = now;
                self.s.lock().unwrap().procs = rows;
            }
            // nothing asked for lately: end the thread (a new ask starts it again)
            let mut g = self.s.lock().unwrap();
            let busy = g.slots.values().any(|s| s.last_used.is_some_and(|t| now - t < KEEP)) || procs_wanted;
            if !busy {
                g.running = false;
                // drop stale slots (history of metrics nobody shows any more)
                g.slots.retain(|_, s| s.last_used.is_some_and(|t| now - t < Duration::from_secs(60)));
                break;
            }
        }
        // a stop request also clears the running flag
        self.s.lock().unwrap().running = false;
    }
}

// ---------------------------------------------------------------- sources

// SAFETY: the PDH handles are only used from the sampler thread that created them.
unsafe impl Send for Pdh {}

struct Sources {
    nvml: Option<super::sensors::GpuSource>,
    pdh: Option<Option<Pdh>>,
}

impl Sources {
    fn new() -> Sources {
        Sources { nvml: None, pdh: None }
    }

    fn gpu(&mut self) -> Option<super::sensors::Gpu> {
        self.nvml.get_or_insert_with(super::sensors::GpuSource::new).read()
    }

    /// Poll one metric into its slot. Returns the GPU name when a GPU metric was read.
    fn poll(&mut self, key: &str, s: &mut Slot, now: Instant) -> Option<String> {
        let (metric, arg) = key.split_once(':').unwrap_or((key, ""));
        s.value = None;
        match metric {
            "cpu.load" => {
                if let Some((idle, total)) = cpu_times() {
                    if let Some((pi, pt)) = s.prev {
                        let (di, dt) = (idle.wrapping_sub(pi), total.wrapping_sub(pt));
                        if dt > 0 {
                            s.value = Some((100.0 * (1.0 - di as f32 / dt as f32)).clamp(0.0, 100.0));
                        }
                    }
                    s.prev = Some((idle, total));
                }
            }
            "cpu.core" => {
                let core: usize = arg.parse().unwrap_or(0);
                if let Some(c) = core_times().and_then(|v| v.get(core).copied()) {
                    if let Some((pi, pt)) = s.prev {
                        let (di, dt) = (c.0.wrapping_sub(pi), c.1.wrapping_sub(pt));
                        if dt > 0 {
                            s.value = Some((100.0 * (1.0 - di as f32 / dt as f32)).clamp(0.0, 100.0));
                        }
                    }
                    s.prev = Some(c);
                }
            }
            "cpu.clock" => s.value = cpu_clock_mhz(),
            "cpu.temp" => s.value = None, // no source without a kernel driver
            "ram.used" => {
                if let Some((used, total)) = memory_bytes() {
                    s.value = Some(used as f32 / 1073741824.0);
                    s.total = Some(total as f32 / 1073741824.0);
                }
            }
            "gpu.load" | "gpu.temp" | "gpu.clock" | "gpu.vram" | "gpu.power" | "gpu.fan" => {
                let g = self.gpu();
                let name = g.as_ref().map(|g| g.name.clone());
                if let Some(g) = g {
                    match metric {
                        "gpu.load" => s.value = Some(g.load_pct as f32),
                        "gpu.temp" => s.value = Some(g.temp_c as f32),
                        "gpu.clock" => s.value = Some(g.clock_mhz as f32),
                        "gpu.vram" => {
                            s.value = Some(g.mem_used_mb as f32 / 1024.0);
                            s.total = Some(g.mem_total_mb as f32 / 1024.0);
                        }
                        "gpu.power" => s.value = g.power_w,
                        _ => s.value = g.fan_pct.map(|f| f as f32),
                    }
                }
                return name;
            }
            "net.down" | "net.up" => {
                let down = metric == "net.down";
                let rows = net_rows();
                let dt = s.prev_at.map(|t| (now - t).as_secs_f32()).unwrap_or(0.0);
                let mut best: Option<f32> = None;
                let mut next = HashMap::new();
                for r in rows {
                    next.insert(r.index, (r.rx, r.tx));
                    if arg != "*" && !r.alias.eq_ignore_ascii_case(arg) {
                        continue;
                    }
                    if let (Some((prx, ptx)), true) = (s.prev_net.get(&r.index), dt > 0.0) {
                        let d = if down { r.rx.wrapping_sub(*prx) } else { r.tx.wrapping_sub(*ptx) };
                        let rate = d as f32 / dt;
                        best = Some(best.map_or(rate, |b: f32| b.max(rate)));
                    }
                }
                s.prev_net = next;
                s.prev_at = Some(now);
                s.value = best;
            }
            "disk.used" => {
                let drive = arg.chars().next().unwrap_or('C');
                if let Some((used, total)) = disk_space(drive) {
                    s.value = Some(used as f32 / 1073741824.0);
                    s.total = Some(total as f32 / 1073741824.0);
                }
            }
            "disk.activity" => {
                if self.pdh.is_none() {
                    self.pdh = Some(Pdh::open());
                }
                s.value = self.pdh.as_mut().and_then(|p| p.as_mut()).and_then(|p| p.read());
            }
            _ => {}
        }
        None
    }
}

use std::ffi::c_void;

/// (idle, kernel+user total) in 100 ns units. Kernel time includes idle time.
fn cpu_times() -> Option<(u64, u64)> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut i, mut k, mut u) = (z, z, z);
    if unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } == 0 {
        return None;
    }
    let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Some((f(i), f(k) + f(u)))
}

#[repr(C)]
struct ProcPerf {
    idle: i64,
    kernel: i64,
    user: i64,
    dpc: i64,
    interrupt: i64,
    interrupt_count: u32,
    _pad: u32,
}

#[link(name = "ntdll", kind = "raw-dylib")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct ProcPower {
    number: u32,
    max_mhz: u32,
    current_mhz: u32,
    mhz_limit: u32,
    max_idle: u32,
    cur_idle: u32,
}

#[link(name = "powrprof", kind = "raw-dylib")]
extern "system" {
    fn CallNtPowerInformation(level: i32, input: *const c_void, in_len: u32, out: *mut c_void, out_len: u32) -> i32;
}

/// Per logical core (idle, total) in 100 ns units.
fn core_times() -> Option<Vec<(u64, u64)>> {
    let mut buf: Vec<ProcPerf> = (0..256).map(|_| ProcPerf { idle: 0, kernel: 0, user: 0, dpc: 0, interrupt: 0, interrupt_count: 0, _pad: 0 }).collect();
    let mut ret = 0u32;
    let st = unsafe { NtQuerySystemInformation(8, buf.as_mut_ptr() as *mut c_void, (buf.len() * std::mem::size_of::<ProcPerf>()) as u32, &mut ret) };
    if st != 0 {
        return None;
    }
    let n = ret as usize / std::mem::size_of::<ProcPerf>();
    Some(buf[..n].iter().map(|p| (p.idle as u64, (p.kernel + p.user) as u64)).collect())
}

/// Average current clock over the logical processors (MHz). Windows reports the P-state it manages, which on modern CPUs can
/// sit below the real boost clock.
fn cpu_clock_mhz() -> Option<f32> {
    let n = logical_processors();
    let mut buf = vec![ProcPower::default(); n.max(1)];
    let st = unsafe { CallNtPowerInformation(11, std::ptr::null(), 0, buf.as_mut_ptr() as *mut c_void, (buf.len() * std::mem::size_of::<ProcPower>()) as u32) };
    if st != 0 {
        return None;
    }
    let sum: u64 = buf.iter().map(|p| p.current_mhz as u64).sum();
    let v = sum as f32 / buf.len() as f32;
    if v > 0.0 { Some(v) } else { None }
}

fn logical_processors() -> usize {
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    let mut si: SYSTEM_INFO = unsafe { std::mem::zeroed() };
    unsafe { GetSystemInfo(&mut si) };
    si.dwNumberOfProcessors as usize
}

fn memory_bytes() -> Option<(u64, u64)> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return None;
    }
    Some((m.ullTotalPhys - m.ullAvailPhys, m.ullTotalPhys))
}

fn disk_space(drive: char) -> Option<(u64, u64)> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let root: Vec<u16> = format!("{}:\\", drive.to_ascii_uppercase()).encode_utf16().chain(std::iter::once(0)).collect();
    let (mut free, mut total, mut tfree) = (0u64, 0u64, 0u64);
    if unsafe { GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, &mut tfree) } == 0 || total == 0 {
        return None;
    }
    Some((total - tfree, total))
}

struct NetRow {
    index: u32,
    alias: String,
    rx: u64,
    tx: u64,
}

/// Up, non-loopback, non-tunnel interfaces with their octet counters.
fn net_rows() -> Vec<NetRow> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    let mut out = Vec::new();
    let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
    unsafe {
        if GetIfTable2(&mut table) != 0 || table.is_null() {
            return out;
        }
        let t = &*table;
        let rows = std::slice::from_raw_parts(t.Table.as_ptr(), t.NumEntries as usize);
        for r in rows {
            // 1 = IfOperStatusUp; type 24 = loopback, 131 = tunnel
            if r.OperStatus != 1 || r.Type == 24 || r.Type == 131 {
                continue;
            }
            let n = r.Alias.iter().position(|c| *c == 0).unwrap_or(r.Alias.len());
            out.push(NetRow { index: r.InterfaceIndex, alias: String::from_utf16_lossy(&r.Alias[..n]), rx: r.InOctets, tx: r.OutOctets });
        }
        FreeMibTable(table as *const c_void);
    }
    out
}

/// PDH "% Disk Time" of all physical disks (clamped to 100).
struct Pdh {
    query: *mut c_void,
    counter: *mut c_void,
}

impl Pdh {
    fn open() -> Option<Pdh> {
        use windows_sys::Win32::System::Performance::{PdhAddEnglishCounterW, PdhCollectQueryData, PdhOpenQueryW};
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        unsafe {
            let mut q: *mut c_void = std::ptr::null_mut();
            if PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 {
                return None;
            }
            let mut c: *mut c_void = std::ptr::null_mut();
            if PdhAddEnglishCounterW(q, wide("\\PhysicalDisk(_Total)\\% Disk Time").as_ptr(), 0, &mut c) != 0 {
                return None;
            }
            PdhCollectQueryData(q);
            Some(Pdh { query: q, counter: c })
        }
    }

    fn read(&mut self) -> Option<f32> {
        use windows_sys::Win32::System::Performance::{PdhCollectQueryData, PdhGetFormattedCounterValue, PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE};
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let mut v: PDH_FMT_COUNTERVALUE = std::mem::zeroed();
            if PdhGetFormattedCounterValue(self.counter, PDH_FMT_DOUBLE, std::ptr::null_mut(), &mut v) != 0 || v.CStatus != 0 {
                return None;
            }
            Some((v.Anonymous.doubleValue as f32).clamp(0.0, 100.0))
        }
    }
}

impl Drop for Pdh {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::System::Performance::PdhCloseQuery(self.query) };
    }
}

fn read_cpu_name() -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ};
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let mut buf = [0u16; 256];
    let mut len = (buf.len() * 2) as u32;
    let st = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            wide("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0").as_ptr(),
            wide("ProcessorNameString").as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
        )
    };
    if st != 0 {
        return None;
    }
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]).split_whitespace().collect::<Vec<_>>().join(" "))
}

/// Per-process CPU % (of the whole PC) and working set, grouped by executable name.
fn sample_processes(prev: &mut HashMap<u32, (u64, Instant)>, now: Instant) -> Vec<ProcRow> {
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let cores = logical_processors().max(1) as f32;
    let mut groups: HashMap<String, ProcRow> = HashMap::new();
    let mut next: HashMap<u32, (u64, Instant)> = HashMap::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let pid = e.th32ProcessID;
            if pid != 0 {
                let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                if !h.is_null() {
                    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
                    let (mut c, mut x, mut k, mut u) = (z, z, z, z);
                    let mut mem: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                    mem.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                    let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
                    if GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u) != 0 {
                        let cpu_total = f(k) + f(u);
                        next.insert(pid, (cpu_total, now));
                        let mut pct = 0.0;
                        if let Some((pt, at)) = prev.get(&pid) {
                            let dt = (now - *at).as_secs_f32();
                            if dt > 0.0 {
                                pct = (cpu_total.saturating_sub(*pt) as f32 / 1e7) / dt / cores * 100.0;
                            }
                        }
                        let ram = if K32GetProcessMemoryInfo(h, &mut mem, mem.cb) != 0 { mem.WorkingSetSize as f32 / 1048576.0 } else { 0.0 };
                        let n = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                        let name = String::from_utf16_lossy(&e.szExeFile[..n]);
                        let name = name.strip_suffix(".exe").unwrap_or(&name).to_string();
                        let row = groups.entry(name.to_ascii_lowercase()).or_insert(ProcRow { name, cpu_pct: 0.0, ram_mb: 0.0 });
                        row.cpu_pct += pct;
                        row.ram_mb += ram;
                    }
                    CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    *prev = next;
    groups.into_values().filter(|r| r.name != "System Idle Process" && r.name != "Idle").collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_select_core_drive_and_adapter() {
        assert_eq!(key("cpu.core", 3, "C:", None), "cpu.core:3");
        assert_eq!(key("disk.used", 0, "d:", None), "disk.used:D");
        assert_eq!(key("net.down", 0, "C:", None), "net.down:*");
        assert_eq!(key("net.up", 0, "C:", Some("Ethernet")), "net.up:Ethernet");
        assert_eq!(key("gpu.temp", 0, "C:", None), "gpu.temp");
    }

    #[test]
    fn the_sampler_polls_what_is_asked_and_ends_when_idle() {
        let h = hub();
        // RAM is always readable; CPU load needs two polls for a delta
        let (v0, _) = h.read("ram.used", 4);
        let _ = (v0, h.read("cpu.load", 4));
        std::thread::sleep(Duration::from_millis(1200));
        let (ram, total) = h.read("ram.used", 4);
        assert!(ram.unwrap_or(0.0) > 0.1 && total.unwrap_or(0.0) >= ram.unwrap());
        // the load needs a time delta between two polls; on a starved machine the first one can be empty, so wait for a value
        let mut cpu = None;
        for _ in 0..30 {
            cpu = h.read("cpu.load", 4).0;
            if cpu.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        assert!(cpu.is_some_and(|c| (0.0..=100.0).contains(&c)), "{cpu:?}");
        assert!(h.history("ram.used", 10.0).len() >= 2);
        // a metric that cannot be read stays None, never 0
        let (t, _) = h.read("cpu.temp", 1);
        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(h.read("cpu.temp", 1).0, t);
        assert!(h.read("cpu.temp", 1).0.is_none());
        let (n, _) = h.read("disk.used:C", 1);
        let _ = n;
        std::thread::sleep(Duration::from_millis(500));
        assert!(h.read("disk.used:C", 1).0.is_some());
        assert!(h.cpu_name().is_some());
    }

    #[test]
    fn top_processes_lists_this_test_binary() {
        let h = hub();
        let _ = h.top_processes(true, 5);
        std::thread::sleep(Duration::from_millis(2600));
        let rows = h.top_processes(true, 5);
        assert!(!rows.is_empty() && rows[0].ram_mb > 1.0, "{rows:?}");
    }
}
