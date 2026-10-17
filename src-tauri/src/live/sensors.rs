//! Sensor readings for the sensor panel, sampled about once a second on their own thread.
//! CPU load: GetSystemTimes. RAM: GlobalMemoryStatusEx. NVIDIA GPU: NVML, loaded at run time from nvml.dll (it ships with the
//! driver; no admin, nothing to install). CPU temperature needs a kernel driver or admin tooling, so it is None here.
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Debug, Default)]
pub struct Gpu {
    pub name: String,
    pub temp_c: u32,
    pub load_pct: u32,
    pub clock_mhz: u32,
    pub mem_used_mb: u64,
    pub mem_total_mb: u64,
    /// Board power draw in watts / fan speed in percent, when the driver reports them.
    pub power_w: Option<f32>,
    pub fan_pct: Option<u32>,
}

#[derive(Clone, Debug, Default)]
pub struct Sensors {
    pub cpu_load_pct: f32,
    pub ram_used_gb: f32,
    pub ram_total_gb: f32,
    pub gpu: Option<Gpu>,
    /// None: not available without a driver or admin rights.
    pub cpu_temp_c: Option<f32>,
}

pub struct Sampler {
    pub data: Arc<Mutex<Sensors>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    pub fn start() -> Sampler {
        let data = Arc::new(Mutex::new(Sensors::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (d2, s2) = (data.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            let mut nvml = Nvml::load();
            let mut prev = cpu_times();
            while !s2.load(Ordering::Relaxed) {
                let now = cpu_times();
                let mut s = Sensors::default();
                if let (Some(a), Some(b)) = (prev, now) {
                    let (idle, total) = (b.0.wrapping_sub(a.0), b.1.wrapping_sub(a.1));
                    if total > 0 {
                        s.cpu_load_pct = 100.0 * (1.0 - idle as f32 / total as f32);
                    }
                }
                prev = now;
                if let Some((used, total)) = memory() {
                    s.ram_used_gb = used as f32 / 1073741824.0;
                    s.ram_total_gb = total as f32 / 1073741824.0;
                }
                s.gpu = nvml.as_mut().and_then(|n| n.read());
                *d2.lock().unwrap() = s;
                // sleep in small steps so stop is quick
                for _ in 0..10 {
                    if s2.load(Ordering::Relaxed) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        });
        Sampler { data, stop, thread: Some(thread) }
    }

    pub fn get(&self) -> Sensors {
        self.data.lock().unwrap().clone()
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// (idle, kernel+user total) in 100 ns units. Kernel time includes idle time.
fn cpu_times() -> Option<(u64, u64)> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetSystemTimes;
    let (mut i, mut k, mut u) = (FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 });
    let ok = unsafe { GetSystemTimes(&mut i, &mut k, &mut u) };
    if ok == 0 {
        return None;
    }
    let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    Some((f(i), f(k) + f(u)))
}

/// (used bytes, total bytes) of physical memory.
fn memory() -> Option<(u64, u64)> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 {
        return None;
    }
    Some((m.ullTotalPhys - m.ullAvailPhys, m.ullTotalPhys))
}

// ---- NVML, loaded dynamically ----

#[repr(C)]
struct Util {
    gpu: u32,
    memory: u32,
}
#[repr(C)]
struct MemInfo {
    total: u64,
    free: u64,
    used: u64,
}

pub struct Nvml {
    dev: *mut c_void,
    name: String,
    temp: unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32,
    util: unsafe extern "C" fn(*mut c_void, *mut Util) -> i32,
    clock: unsafe extern "C" fn(*mut c_void, u32, *mut u32) -> i32,
    mem: unsafe extern "C" fn(*mut c_void, *mut MemInfo) -> i32,
    power: Option<unsafe extern "C" fn(*mut c_void, *mut u32) -> i32>,
    fan: Option<unsafe extern "C" fn(*mut c_void, *mut u32) -> i32>,
}

// The handle is only used from the sampler thread that created it.
unsafe impl Send for Nvml {}

impl Nvml {
    pub fn load() -> Option<Nvml> {
        use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        unsafe {
            let mut lib = LoadLibraryW(wide("nvml.dll").as_ptr());
            if lib.is_null() {
                lib = LoadLibraryW(wide("C:\\Program Files\\NVIDIA Corporation\\NVSMI\\nvml.dll").as_ptr());
            }
            if lib.is_null() {
                return None;
            }
            macro_rules! sym {
                ($n:expr) => {
                    std::mem::transmute(GetProcAddress(lib, concat!($n, "\0").as_ptr())?)
                };
            }
            let init: unsafe extern "C" fn() -> i32 = sym!("nvmlInit_v2");
            let by_index: unsafe extern "C" fn(u32, *mut *mut c_void) -> i32 = sym!("nvmlDeviceGetHandleByIndex_v2");
            let get_name: unsafe extern "C" fn(*mut c_void, *mut u8, u32) -> i32 = sym!("nvmlDeviceGetName");
            let temp = sym!("nvmlDeviceGetTemperature");
            let util = sym!("nvmlDeviceGetUtilizationRates");
            let clock = sym!("nvmlDeviceGetClockInfo");
            let mem = sym!("nvmlDeviceGetMemoryInfo");
            if init() != 0 {
                return None;
            }
            let mut dev: *mut c_void = std::ptr::null_mut();
            if by_index(0, &mut dev) != 0 || dev.is_null() {
                return None;
            }
            let mut buf = [0u8; 96];
            let name = if get_name(dev, buf.as_mut_ptr(), buf.len() as u32) == 0 {
                let n = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
                String::from_utf8_lossy(&buf[..n]).into_owned()
            } else {
                "GPU".into()
            };
            let power = GetProcAddress(lib, "nvmlDeviceGetPowerUsage            Some(Nvml { dev, name, temp, util, clock, mem })".as_ptr()).map(|p| std::mem::transmute::<_, unsafe extern "C" fn(*mut c_void, *mut u32) -> i32>(p));
            let fan = GetProcAddress(lib, "nvmlDeviceGetFanSpeed            Some(Nvml { dev, name, temp, util, clock, mem })".as_ptr()).map(|p| std::mem::transmute::<_, unsafe extern "C" fn(*mut c_void, *mut u32) -> i32>(p));
            Some(Nvml { dev, name, temp, util, clock, mem, power, fan })
        }
    }

    pub fn read(&mut self) -> Option<Gpu> {
        unsafe {
            let mut t = 0u32;
            let mut c = 0u32;
            let mut u = Util { gpu: 0, memory: 0 };
            let mut m = MemInfo { total: 0, free: 0, used: 0 };
            if (self.temp)(self.dev, 0, &mut t) != 0 {
                return None;
            }
            let _ = (self.util)(self.dev, &mut u);
            let _ = (self.clock)(self.dev, 0, &mut c); // 0 = graphics clock
            let _ = (self.mem)(self.dev, &mut m);
            let mut mw = 0u32;
            let power_w = self.power.and_then(|f| if f(self.dev, &mut mw) == 0 { Some(mw as f32 / 1000.0) } else { None });
            let mut fp = 0u32;
            let fan_pct = self.fan.and_then(|f| if f(self.dev, &mut fp) == 0 { Some(fp) } else { None });
            Some(Gpu { name: self.name.clone(), temp_c: t, load_pct: u.gpu, clock_mhz: c, mem_used_mb: m.used / 1048576, mem_total_mb: m.total / 1048576, power_w, fan_pct })
        }
    }
}
