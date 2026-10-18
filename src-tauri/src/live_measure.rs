//! `coolerscreen.exe --live-measure SECS [--no-device] [--fps 30,60] [--variants visible,hidden,destroyed]
//!                    [--video clip.mp4] [--out result.txt]`
//! Prototype B measuring without any UI click: starts the live engine inside this app in each window variant and prints
//! CPU and memory of this process PLUS its WebView2 child processes (msedgewebview2.exe) for each fps.
//!   visible     window shown
//!   hidden      window hidden to the tray, WebView2 alive
//!   destroyed   window destroyed, process stays in the tray (WebView2 gone)
//! Each variant also gets a "no live" baseline row (engine stopped, same window state) to subtract.
//! With --no-device the frames are discarded and the screen is replaced by the fake device (no USB at all, even the page's
//! own start-up check). Results are written to --out (default %TEMP%\cs-live-measure.txt) and printed.
use cs_core::core::commands::Core;
use cs_core::live::Config;
use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows_sys::Win32::System::Threading::{GetCurrentProcessId, GetProcessTimes, OpenProcess};

pub struct Opts {
    pub secs: u64,
    pub no_device: bool,
    pub fps: Vec<u32>,
    pub variants: Vec<String>,
    pub video: Option<String>,
    pub out: std::path::PathBuf,
}

fn arg_val<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

pub fn parse(args: &[String]) -> Option<Opts> {
    let secs = arg_val(args, "--live-measure")?.parse().ok()?;
    Some(Opts {
        secs,
        no_device: args.iter().any(|a| a == "--no-device"),
        fps: arg_val(args, "--fps").map(|v| v.split(',').filter_map(|x| x.parse().ok()).collect()).filter(|v: &Vec<u32>| !v.is_empty()).unwrap_or(vec![30, 60]),
        variants: arg_val(args, "--variants").map(|v| v.split(',').map(str::to_string).collect()).unwrap_or(vec!["visible".into(), "hidden".into(), "destroyed".into()]),
        video: arg_val(args, "--video").map(str::to_string),
        out: arg_val(args, "--out").map(Into::into).unwrap_or_else(|| std::env::temp_dir().join("cs-live-measure.txt")),
    })
}

// ---------------------------------------------------------------- process sampling

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Kind {
    App,
    WebView,
    Other,
}

struct Proc {
    pid: u32,
    kind: Kind,
}

/// This process and every descendant (WebView2's renderer / GPU / utility processes hang below its browser process).
fn family() -> Vec<Proc> {
    let me = unsafe { GetCurrentProcessId() };
    let mut all: Vec<(u32, u32, String)> = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap as isize == -1 {
            return vec![Proc { pid: me, kind: Kind::App }];
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            all.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase()));
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    let mut out = vec![Proc { pid: me, kind: Kind::App }];
    let mut seen = vec![me];
    let mut i = 0;
    while i < out.len() {
        let parent = out[i].pid;
        for (pid, ppid, name) in &all {
            if *ppid == parent && !seen.contains(pid) {
                seen.push(*pid);
                out.push(Proc { pid: *pid, kind: if name.contains("msedgewebview2") { Kind::WebView } else { Kind::Other } });
            }
        }
        i += 1;
    }
    out
}

fn ft(h: HANDLE) -> u64 {
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (z, z, z, z);
    unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) };
    let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
    f(k) + f(u)
}

/// (cpu in 100 ns units, working set MB, private MB) of one process; None if it cannot be opened.
fn read(pid: u32) -> Option<(u64, f64, f64)> {
    unsafe {
        let h = OpenProcess(0x1000, 0, pid); // PROCESS_QUERY_LIMITED_INFORMATION
        if h.is_null() {
            return None;
        }
        let mut m: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        K32GetProcessMemoryInfo(h, &mut m, m.cb);
        let cpu = ft(h);
        CloseHandle(h);
        Some((cpu, m.WorkingSetSize as f64 / 1048576.0, m.PagefileUsage as f64 / 1048576.0))
    }
}

#[derive(Default, Clone, Copy)]
struct Sums {
    cpu_s: f64,
    ws_sum: f64,
    ws_peak: f64,
    pv_sum: f64,
    pv_peak: f64,
    n_sum: f64,
}

#[derive(Default)]
struct Sampler {
    last_cpu: HashMap<u32, u64>,
    sums: HashMap<Kind, Sums>,
    samples: f64,
}

impl Sampler {
    fn sample(&mut self, count_cpu: bool) {
        let mut ws: HashMap<Kind, (f64, f64, f64)> = HashMap::new();
        for p in family() {
            let Some((cpu, w, pv)) = read(p.pid) else { continue };
            let prev = self.last_cpu.insert(p.pid, cpu);
            if count_cpu {
                // a process that appeared during the run counts from its start
                let d = cpu.saturating_sub(prev.unwrap_or(0));
                self.sums.entry(p.kind).or_default().cpu_s += d as f64 / 1e7;
            }
            let e = ws.entry(p.kind).or_default();
            e.0 += w;
            e.1 += pv;
            e.2 += 1.0;
        }
        if count_cpu {
            self.samples += 1.0;
            for k in [Kind::App, Kind::WebView, Kind::Other] {
                let (w, pv, n) = ws.get(&k).copied().unwrap_or_default();
                let s = self.sums.entry(k).or_default();
                s.ws_sum += w;
                s.ws_peak = s.ws_peak.max(w);
                s.pv_sum += pv;
                s.pv_peak = s.pv_peak.max(pv);
                s.n_sum += n;
            }
        }
    }
}

// ---------------------------------------------------------------- the run

struct Out(std::fs::File);
impl Out {
    fn line(&mut self, s: &str) {
        println!("{s}");
        let _ = writeln!(self.0, "{s}");
        let _ = self.0.flush();
    }
}

fn run_one(out: &mut Out, app: &AppHandle, o: &Opts, cores: f64, variant: &str, fps: Option<u32>) {
    let core = app.state::<Core>().0.clone();
    core.live.stop();
    if let Some(f) = fps {
        let mut c = Config::default();
        c.fps = f;
        c.background = o.video.clone();
        core.live.set_config_override(Some(c));
        core.live.start();
    }
    // settle: connect, caches, quality, WebView2 start-up / shut-down
    std::thread::sleep(Duration::from_secs(if variant == "destroyed" { 9 } else { 6 }));
    let mut sm = Sampler::default();
    sm.sample(false);
    let s0 = core.live.pipeline().map(|p| p.shared.stats.snap());
    let t0 = Instant::now();
    while t0.elapsed().as_secs() < o.secs {
        std::thread::sleep(Duration::from_secs(1));
        sm.sample(true);
    }
    let wall = t0.elapsed().as_secs_f64();
    let rates = match (core.live.pipeline(), s0) {
        (Some(p), Some(a)) => {
            let b = p.shared.stats.snap();
            format!("rendered {:.1} / sent {:.1} fps, unchanged skipped {}, send errors {}", (b.rendered - a.rendered) as f64 / wall, (b.sent - a.sent) as f64 / wall, b.unchanged_skipped - a.unchanged_skipped, b.send_errors - a.send_errors)
        }
        _ => "live engine not running".into(),
    };
    let g = |k: Kind| sm.sums.get(&k).copied().unwrap_or_default();
    let (a, w, x) = (g(Kind::App), g(Kind::WebView), g(Kind::Other));
    let cpu_total = a.cpu_s + w.cpu_s + x.cpu_s;
    let n = sm.samples.max(1.0);
    out.line(&format!(
        "{:<10} {:<8} | CPU one core {:>6.2} % (app {:>5.2}, webview {:>5.2}, other {:>5.2}) | whole PC {:>5.2} % | WS avg {:>6.1} MB (app {:>5.1}, webview {:>6.1}, other {:>5.1}) peak {:>6.1} | private avg {:>6.1} MB (app {:>5.1}, webview {:>6.1}) | webview procs {:.1} | {rates}",
        variant,
        fps.map(|f| format!("{f} fps")).unwrap_or_else(|| "no live".into()),
        100.0 * cpu_total / wall,
        100.0 * a.cpu_s / wall,
        100.0 * w.cpu_s / wall,
        100.0 * x.cpu_s / wall,
        100.0 * cpu_total / wall / cores,
        (a.ws_sum + w.ws_sum + x.ws_sum) / n,
        a.ws_sum / n,
        w.ws_sum / n,
        x.ws_sum / n,
        a.ws_peak + w.ws_peak + x.ws_peak,
        (a.pv_sum + w.pv_sum + x.pv_sum) / n,
        a.pv_sum / n,
        w.pv_sum / n,
        w.n_sum / n,
    ));
}

/// Drives the whole measurement from its own thread (the app's main thread keeps running the window system).
pub fn spawn(app: AppHandle, o: Opts) {
    std::thread::spawn(move || {
        let file = std::fs::File::create(&o.out);
        let Ok(file) = file else { return };
        let mut out = Out(file);
        let cores: f64 = std::env::var("NUMBER_OF_PROCESSORS").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let core = app.state::<Core>().0.clone();
        core.live.set_no_device(o.no_device);
        out.line(&format!(
            "CoolerScreen prototype B: {} s per row, {}, {} cores, video background: {}",
            o.secs,
            if o.no_device { "NO screen (frames discarded, fake device)" } else { "WITH the screen" },
            cores,
            o.video.as_deref().unwrap_or("none (gradient)")
        ));
        // let the window load and the start-up scan finish
        std::thread::sleep(Duration::from_secs(8));
        for v in &o.variants {
            match v.as_str() {
                "visible" => {
                    if let Some(w) = app.get_webview_window("main") {
                        crate::window::present(&w, crate::window::Present::Normal);
                    }
                }
                "hidden" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.hide();
                    }
                }
                "destroyed" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.destroy();
                    }
                    let t = Instant::now();
                    while app.get_webview_window("main").is_some() && t.elapsed() < Duration::from_secs(5) {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                other => {
                    out.line(&format!("unknown variant {other}"));
                    continue;
                }
            }
            for f in &o.fps {
                run_one(&mut out, &app, &o, cores, v, Some(*f));
            }
            run_one(&mut out, &app, &o, cores, v, None);
        }
        out.line("done");
        core.live.stop();
        let a2 = app.clone();
        let _ = app.run_on_main_thread(move || a2.exit(0));
    });
}
