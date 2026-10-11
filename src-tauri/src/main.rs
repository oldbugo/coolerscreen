// CoolerScreen 2.0 shell: Tauri 2 + system WebView2, seamless frameless window.
// Backend (COOLERSCREEN_BACKEND):
//   node (default until the Phase 1 workstreams land): the 1.2 Node backend runs as a sidecar and serves public/
//        and /api on 127.0.0.1; the Rust core is created but not started (no watcher, one writer only).
//   rust: the Rust core (src/core) serves the UI through Tauri commands/events and a local media server.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use cs_core::core::commands::{self as cc, Core, EventRelay, TauriEvents};
use cs_core::core::media_server::MediaServer;
use cs_core::core::App;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{Emitter, Manager, RunEvent, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

static FORCE_CLOSE: AtomicBool = AtomicBool::new(false);
const BG: (u8, u8, u8) = (0x0f, 0x11, 0x15); // public/style.css --bg

#[derive(Default)]
struct Sidecar {
    child: Mutex<Option<Child>>,
    url: Mutex<Option<String>>,
}

/// Keeps the media server alive for the app's lifetime.
struct Media(#[allow(dead_code)] Mutex<Option<MediaServer>>);

fn rust_backend() -> bool {
    std::env::var("COOLERSCREEN_BACKEND").map(|v| v.eq_ignore_ascii_case("rust")).unwrap_or(false)
}

// ---------- window chrome ----------

#[tauri::command]
fn win_minimize(window: WebviewWindow) {
    let _ = window.minimize();
}

/// Maximise to the work area (taskbar stays visible); returns the new maximised state.
#[tauri::command]
fn win_toggle_maximize(window: WebviewWindow) -> bool {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
        false
    } else {
        let _ = window.maximize();
        true
    }
}

#[tauri::command]
fn win_state(window: WebviewWindow) -> bool {
    window.is_maximized().unwrap_or(false)
}

#[tauri::command]
fn win_close(window: WebviewWindow) {
    let _ = window.close(); // goes through the close guard
}

#[tauri::command]
fn win_force_close(window: WebviewWindow) {
    FORCE_CLOSE.store(true, Ordering::SeqCst);
    let _ = window.close();
}

// ---------- Node sidecar (COOLERSCREEN_BACKEND=node) ----------

fn repo_dir() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("COOLERSCREEN_REPO") {
        return p.into();
    }
    // The repo this exe was built from (development); the Rust backend replaces the sidecar.
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
    let relay = Arc::new(EventRelay::default());
    let core = App::new(cs_core::core::paths::base(), cs_core::backends(), relay.clone(), env!("CARGO_PKG_VERSION"));
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(Sidecar::default())
        .manage(Core(core.clone()))
        .manage(Media(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            win_minimize,
            win_toggle_maximize,
            win_state,
            win_close,
            win_force_close,
            cc::app_state,
            cc::settings_update,
            cc::activity,
            cc::job_get,
            cc::job_cancel,
            cc::set_focus,
            cc::originals_list,
            cc::import_bytes,
            cc::import_path,
            cc::missing_delete,
            cc::original_get,
            cc::original_patch,
            cc::original_delete,
            cc::original_reveal,
            cc::estimate,
            cc::loop_search,
            cc::loop_point,
            cc::version_create,
            cc::version_patch,
            cc::version_delete,
            cc::version_send,
            cc::version_remake,
            cc::version_retry,
            cc::sources_list,
            cc::browse_set,
            cc::link_add,
            cc::source_set,
            cc::source_items,
            cc::source_add,
            cc::gallery_set_dir,
            cc::gallery_open,
            cc::gallery_sync,
            cc::device_state,
            cc::device_check,
            cc::device_restart,
            cc::device_setup,
            cc::device_restore,
            cc::device_reconnect
        ])
        .setup(move |app| {
            let _ = relay.0.set(Arc::new(TauriEvents(app.handle().clone())));
            let win = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title("CoolerScreen")
                .inner_size(1280.0, 860.0)
                .min_inner_size(900.0, 640.0)
                .decorations(false)
                .shadow(true)
                .background_color(tauri::webview::Color(BG.0, BG.1, BG.2, 255))
                .disable_drag_drop_handler() // HTML5 drop reaches the page, as in Electron
                .initialization_script(include_str!("../shell/titlebar.js"))
                .visible(false)
                .build()?;
            let w2 = win.clone();
            win.on_window_event(move |e| match e {
                WindowEvent::Resized(_) => {
                    let _ = w2.emit("window-state", json!({ "maximized": w2.is_maximized().unwrap_or(false) }));
                }
                WindowEvent::CloseRequested { api, .. } => {
                    if FORCE_CLOSE.load(Ordering::SeqCst) {
                        return;
                    }
                    let h = w2.app_handle();
                    let url = h.state::<Sidecar>().url.lock().unwrap().clone();
                    let busy = h.state::<Core>().0.uploading() || url.is_some_and(|u| sidecar_uploading(&u));
                    if busy {
                        api.prevent_close();
                        let _ = w2.emit("close-guard", ());
                    }
                }
                _ => {}
            });
            win.show()?;
            if rust_backend() {
                let c = app.state::<Core>().0.clone();
                let resolver_app = c.clone();
                match MediaServer::start(Arc::new(move |p: &str, q: &str| resolver_app.resolve_media(p, q))) {
                    Ok(srv) => {
                        #[cfg(debug_assertions)]
                        eprintln!("CS_MEDIA {}", srv.base_url());
                        c.set_media_base(&srv.base_url());
                        app.state::<Media>().0.lock().unwrap().replace(srv);
                    }
                    Err(e) => eprintln!("Media server failed to start: {e}"),
                }
                std::thread::spawn(move || c.startup());
            } else if let Err(e) = start_sidecar(app.handle()) {
                eprintln!("{e}");
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
