// CoolerScreen 2.0 shell: Tauri 2 + system WebView2, seamless frameless window.
// The Rust core (src/core) serves the UI through Tauri commands/events and a local media server.
// 3.0 prototype B (live companion inside the app, off by default): live_app.rs (tray, hide on close), live_measure.rs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod live_app;
mod live_measure;
mod quit_cmd;
mod window;

use cs_core::core::commands::{self as cc, Core, EventRelay, TauriEvents};
use cs_core::core::media_server::MediaServer;
use cs_core::core::App;
use std::sync::{Arc, Mutex};
use tauri::{Manager, RunEvent};


/// Keeps the media server alive for the app's lifetime.
struct Media(#[allow(dead_code)] Mutex<Option<MediaServer>>);
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `--quit`: ask the running app (same app folder) to quit like the tray's Quit; never shows a window or starts an app.
    if args.iter().any(|a| a == "--quit") {
        std::process::exit(quit_cmd::run_quit());
    }
    let measure = live_measure::parse(&args);
    // `--tray` (the Windows start-up entry): no window, live on, icon in the tray.
    let tray_only = args.iter().any(|a| a == cs_core::core::autostart::TRAY_FLAG);
    // `--background`: start without showing or activating anything (a quiet restart after an install). Live on: tray only, as
    // set in Settings. Live off: the window is built minimised without activation (so it stays visible and easy to quit).
    let background = args.iter().any(|a| a == "--background");
    if let Some(m) = &measure {
        // Measuring without the screen must not touch USB at all (the page checks the screen when it starts).
        if m.no_device && std::env::var_os("COOLERSCREEN_FAKE_DEVICE").is_none() {
            std::env::set_var("COOLERSCREEN_FAKE_DEVICE", "winusb");
        }
    }
    let relay = Arc::new(EventRelay::default());
    let core = App::new(cs_core::core::paths::base(), cs_core::backends(), relay.clone(), env!("CARGO_PKG_VERSION"));
    let mut measure = measure;
    let mut builder = tauri::Builder::default();
    // One CoolerScreen at a time, unless COOLERSCREEN_MULTI is set (UI tests next to the installed app, with a temp COOLERSCREEN_HOME).
    // (The override only works in debug builds: an installed build always focuses the running one.)
    if !(cfg!(debug_assertions) && std::env::var_os("COOLERSCREEN_MULTI").is_some()) {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // a second launch brings the window up, except a quiet `--background` restart
            if !args.iter().any(|a| a == "--background") {
                live_app::show_main(app);
            }
        }));
    }
    let app = builder
        .manage(Core(core.clone()))
        .manage(Media(Mutex::new(None)))
        .invoke_handler(tauri::generate_handler![
            window::win_minimize,
            window::win_toggle_maximize,
            window::win_state,
            window::win_close,
            window::win_force_close,
            live_app::live_set,
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
            cc::device_reconnect,
            cc::theme_get,
            cc::theme_set,
            cc::theme_standard,
            cc::theme_preview,
            cc::theme_asset_add,
            cc::theme_apply,
            cc::theme_use,
            cc::theme_set_base,
            cc::calibration_set,
            cc::fit_cal,
            cc::weather_search,
            cc::weather_set_place,
            cc::ext_sensor_status,
            cc::screen_preview,
            cc::screen_preview_end
        ])
        .setup(move |app| {
            cs_core::device::log(&cs_core::core::buildinfo::startup_line(env!("CARGO_PKG_VERSION")));
            let _ = relay.0.set(Arc::new(TauriEvents(app.handle().clone())));
            // One-time driver scripts are bundled as resources (installed build); development keeps the repo's.
            if let Ok(res) = app.path().resource_dir() {
                app.state::<Core>().0.device.set_scripts_dir(res.join("scripts"));
            }
            if background && !tray_only {
                if app.state::<Core>().0.settings.load().live_in_app() {
                    // tray only: the window is built when the user opens it
                } else {
                    live_app::create_window_as(app.handle(), Some(window::Present::MinimizedNoActivate))?;
                }
            } else if !tray_only {
                live_app::create_window(app.handle(), true)?;
            }
            // `CoolerScreen.exe --quit` from an installer or script reaches this app through a named event
            {
                let (h1, h2) = (app.handle().clone(), app.handle().clone());
                quit_cmd::listen(move || h1.state::<Core>().0.busy_for_quit(), move || live_app::quit_app(&h2));
            }
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
            // A fake screen (tests, COOLERSCREEN_FAKE_DEVICE) means live frames are discarded too: never USB.
            if c.device.fake_mode().is_some() {
                c.live.set_no_device(true);
            }
            // 3.0 prototype B: off unless the setting is on (or --live-measure is used).
            if let Some(o) = measure.take() {
                live_app::ensure_tray(app.handle(), false);
                live_measure::spawn(app.handle().clone(), o);
            } else if tray_only || c.settings.load().live_in_app() {
                // Started in the tray without a window the library start-up work waits for the first window; the repairs do not.
                if tray_only || background {
                    let c2 = c.clone();
                    std::thread::spawn(move || {
                        c2.repair_library();
                    });
                }
                live_app::apply(app.handle(), tray_only);
            }
            // Debug builds only: the tray's Quit path on a timer, for the fake-setup tests (a tray menu cannot be clicked from a script).
            #[cfg(debug_assertions)]
            if let Some(ms) = std::env::var("COOLERSCREEN_TEST_TRAY_QUIT_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                    eprintln!("CS_TEST tray quit, tray_on={}", live_app::tray_on());
                    live_app::quit_app(&h);
                });
            }
            // Debug builds only: "name@ms,..." (open, pause, f30, f60, quit; plus hide and close of the window) calls the tray menu handler on a timer, and prints the state after it.
            #[cfg(debug_assertions)]
            if let Ok(plan) = std::env::var("COOLERSCREEN_TEST_TRAY") {
                let h = app.handle().clone();
                std::thread::spawn(move || {
                    let t0 = std::time::Instant::now();
                    for step in plan.split(',') {
                        let Some((name, ms)) = step.split_once('@') else { continue };
                        let at = std::time::Duration::from_millis(ms.parse().unwrap_or(0));
                        std::thread::sleep(at.saturating_sub(t0.elapsed()));
                        match name {
                            "hide" => { if let Some(w) = h.get_webview_window("main") { let _ = w.hide(); } }
                            "close" => { if let Some(w) = h.get_webview_window("main") { let _ = w.close(); } }
                            _ => live_app::tray_menu_click(&h, name),
                        }
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        let st = h.state::<cs_core::core::commands::Core>().0.live.status();
                        eprintln!("CS_TEST after {name}: tray_on={} paused={} fps={} window_visible={:?} menu: {}", live_app::tray_on(), st["paused"], st["fps"], h.get_webview_window("main").map(|w| w.is_visible().unwrap_or(false)), live_app::tray_state());
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building CoolerScreen");

    app.run(|app, event| match event {
        // The last window closed: with the live tray the app stays (it ends only with the tray's Quit).
        RunEvent::ExitRequested { code, api, .. } => {
            if code.is_none() && live_app::keep_running() {
                api.prevent_exit();
            }
        }
        RunEvent::Exit => {
            live_app::arm_exit_watchdog("exit");
            app.state::<Core>().0.live.stop(); // ends the live stream: the screen plays its loop clip again
            app.state::<Core>().0.shutdown();
            // ffmpeg children (Job Object) and the media scratch files (raw intermediates)
            cs_core::media::shutdown();
        }
        _ => {}
    });
}
