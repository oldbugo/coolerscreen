// CoolerScreen 2.0 shell: Tauri 2 + system WebView2, seamless frameless window.
// The Rust core (src/core) serves the UI through Tauri commands/events and a local media server.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod window;

use cs_core::core::commands::{self as cc, Core, EventRelay, TauriEvents};
use cs_core::core::media_server::MediaServer;
use cs_core::core::App;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Manager, RunEvent, WindowEvent};


/// Keeps the media server alive for the app's lifetime.
struct Media(#[allow(dead_code)] Mutex<Option<MediaServer>>);
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
        .manage(Core(core.clone()))
        .manage(Media(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            window::win_minimize,
            window::win_toggle_maximize,
            window::win_state,
            window::win_close,
            window::win_force_close,
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
            // One-time driver scripts are bundled as resources (installed build); development keeps the repo's.
            if let Ok(res) = app.path().resource_dir() {
                app.state::<Core>().0.device.set_scripts_dir(res.join("scripts"));
            }
            let win = window::create(app)?;
            let w2 = win.clone();
            win.on_window_event(move |e| {
                if let WindowEvent::CloseRequested { api, .. } = e {
                    if window::force_close_requested() {
                        return;
                    }
                    // An upload in progress: keep the window and let the page ask in its own dialog.
                    if w2.app_handle().state::<Core>().0.uploading() {
                        api.prevent_close();
                        let _ = w2.emit("close-guard", ());
                    }
                }
            });
            win.show()?;
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
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building CoolerScreen");

    app.run(|app, event| {
        if let RunEvent::Exit = event {
            app.state::<Core>().0.shutdown();
            // ffmpeg children (Job Object) and the media scratch files (raw intermediates)
            cs_core::media::shutdown();
        }
    });
}
