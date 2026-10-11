// CoolerScreen 2.0 spike shell: Tauri 2 + system WebView2, seamless frameless window.
// Phase 0: the existing Node backend runs as a sidecar (`node`, serving public/ and /api on 127.0.0.1),
// the window shows it unchanged; device commands are also available natively (Rust, see src/device).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod window;

use cs_core::device::{self, ops, Slot};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Emitter, Manager, RunEvent, WindowEvent};

static DEVICE_BUSY: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
struct Sidecar {
    child: Mutex<Option<Child>>,
    url: Mutex<Option<String>>,
}

// ---------- device (native) ----------

struct BusyGuard;
impl BusyGuard {
    fn take() -> Result<BusyGuard, String> {
        if DEVICE_BUSY.swap(true, Ordering::SeqCst) {
            return Err("The screen is busy with another operation.".into());
        }
        Ok(BusyGuard)
    }
}
impl Drop for BusyGuard {
    fn drop(&mut self) {
        DEVICE_BUSY.store(false, Ordering::SeqCst);
    }
}

fn err_json(e: device::DevErr) -> Value {
    serde_json::to_value(e).unwrap_or(Value::Null)
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, Value> + Send + 'static) -> Result<T, Value> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| json!({ "message": e.to_string() }))?
}

#[tauri::command]
async fn device_state() -> Result<Value, Value> {
    blocking(|| Ok(serde_json::to_value(ops::state()).unwrap())).await
}

#[tauri::command]
async fn device_info(app: tauri::AppHandle) -> Result<Value, Value> {
    blocking(move || {
        let _g = BusyGuard::take().map_err(|m| json!({ "message": m }))?;
        ops::check_screen(&mut |p, t| {
            let _ = app.emit("device-progress", json!({ "p": p, "text": t }));
        })
        .map_err(err_json)
    })
    .await
}

#[tauri::command]
async fn device_restart(app: tauri::AppHandle) -> Result<Value, Value> {
    blocking(move || {
        let _g = BusyGuard::take().map_err(|m| json!({ "message": m }))?;
        ops::restart_screen(&mut |p, t| {
            let _ = app.emit("device-progress", json!({ "p": p, "text": t }));
        })
        .map_err(err_json)
    })
    .await
}

/// Upload one MJPEG clip into a slot ("loop"/"standby" or "intro"/"startup"); a loop upload ends with a restart.
#[tauri::command]
async fn device_upload(app: tauri::AppHandle, file: String, slot: String) -> Result<Value, Value> {
    let slot = Slot::parse(&slot).ok_or_else(|| json!({ "message": "slot must be loop or intro" }))?;
    blocking(move || {
        let _g = BusyGuard::take().map_err(|m| json!({ "message": m }))?;
        let r = ops::send_files(&[(file.into(), slot)], slot == Slot::Standby, &mut |p, t, a, b| {
            let _ = app.emit("device-progress", json!({ "p": p, "text": t, "sent": a, "total": b }));
        })
        .map_err(err_json)?;
        Ok(serde_json::to_value(r).unwrap())
    })
    .await
}

// ---------- Node sidecar (Phase 0 only) ----------

fn repo_dir() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("COOLERSCREEN_REPO") {
        return p.into();
    }
    // Spike: the repo this exe was built from. Phase 1 replaces the sidecar with Rust commands.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

const SIDECAR_JS: &str = "process.stdin.on('end',()=>process.exit(0));process.stdin.on('error',()=>process.exit(0));process.stdin.resume();\
require('./src/server').start({open:false,port:0}).then(s=>console.log('CS_URL '+s.url)).catch(e=>{console.error(e);process.exit(1)})";

fn start_sidecar(app: &tauri::AppHandle) -> Result<(), String> {
    let mut cmd = Command::new("node");
    cmd.args(["-e", SIDECAR_JS])
        .current_dir(repo_dir())
        .stdin(Stdio::piped()) // closing it (our exit, even a crash) ends the server
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("Cannot start the Node backend: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    app.state::<Sidecar>().child.lock().unwrap().replace(child);
    let app = app.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(url) = line.strip_prefix("CS_URL ") {
                let url = url.trim().to_string();
                app.state::<Sidecar>().url.lock().unwrap().replace(url.clone());
                if let (Some(w), Ok(u)) = (app.get_webview_window("main"), url.parse()) {
                    let _ = w.navigate(u);
                }
            }
        }
    });
    Ok(())
}

/// True while the Node backend is writing to the screen (send or setup job running).
fn sidecar_uploading(url: &str) -> bool {
    let Some(hostport) = url.strip_prefix("http://").and_then(|s| s.split('/').next()) else { return false };
    let Ok(addr) = hostport.parse() else { return false };
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(500)) else { return false };
    let _ = s.set_read_timeout(Some(Duration::from_millis(1500)));
    let _ = write!(s, "GET /api/activity HTTP/1.0\r\nHost: {hostport}\r\n\r\n");
    let mut body = String::new();
    let _ = s.read_to_string(&mut body);
    let Some(json) = body.split("\r\n\r\n").nth(1) else { return false };
    let v: Value = serde_json::from_str(json).unwrap_or(Value::Null);
    v["jobs"].as_array().is_some_and(|jobs| {
        jobs.iter().any(|j| {
            matches!(j["kind"].as_str(), Some("send") | Some("setup")) && j["status"].as_str() == Some("running")
        })
    })
}

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(Sidecar::default())
        .invoke_handler(tauri::generate_handler![
            window::win_minimize,
            window::win_toggle_maximize,
            window::win_state,
            window::win_close,
            window::win_force_close,
            device_state,
            device_info,
            device_restart,
            device_upload
        ])
        .setup(|app| {
            let win = window::create(app)?;
            let w2 = win.clone();
            win.on_window_event(move |e| match e {
                WindowEvent::CloseRequested { api, .. } => {
                    if window::force_close_requested() {
                        return;
                    }
                    let url = w2.app_handle().state::<Sidecar>().url.lock().unwrap().clone();
                    let busy = DEVICE_BUSY.load(Ordering::SeqCst) || url.is_some_and(|u| sidecar_uploading(&u));
                    if busy {
                        api.prevent_close();
                        let _ = w2.emit("close-guard", ());
                    }
                }
                _ => {}
            });
            win.show()?;
            if std::env::var_os("CS_NO_SIDECAR").is_none() {
                if let Err(e) = start_sidecar(app.handle()) {
                    eprintln!("{e}");
                }
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building CoolerScreen");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            if let Some(mut c) = app.state::<Sidecar>().child.lock().unwrap().take() {
                let _ = c.kill();
            }
        }
    });
}
