//! 3.0 prototype B: the live companion running inside the CoolerScreen app (Settings > Live screen, default off).
//! With it on: a tray icon (Open, Pause/Resume, 30/60 fps, Quit) and closing the window hides it to the tray instead of
//! quitting. With "free the window memory" also on, the window (and its WebView2) is destroyed while hidden and built again
//! when the tray icon is used. Quit ends the live stream properly (the screen plays its loop clip again).
//! Nothing starts with Windows and nothing runs unless the setting is on.
use cs_core::core::commands::Core;
use cs_core::core::error::CoreError;
use cs_core::core::live_host::{LiveHost, LOCK, SLEEP};
use cs_core::core::settings::LiveSettings;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry, WebviewWindow, WindowEvent};

const PAUSE_TEXT: &str = "Pause live (screen shows Embedded)";

/// True while the tray icon exists (closing the window then hides it, and the app does not exit with its last window).
static TRAY_ON: AtomicBool = AtomicBool::new(false);
static HOST: OnceLock<Arc<LiveHost>> = OnceLock::new();
/// The live settings as last applied (the session window's procedure has no other way to reach them).
static CFG: Mutex<Option<LiveSettings>> = Mutex::new(None);

fn cfg() -> LiveSettings {
    CFG.lock().unwrap().clone().unwrap_or_default()
}

const TRAY_ID: &str = "live";

/// The menu items whose labels follow the host (Tauri menu items can be changed from any thread).
struct Tray {
    pause: MenuItem<Wry>,
    f30: CheckMenuItem<Wry>,
    f60: CheckMenuItem<Wry>,
}

static TRAY: Mutex<Option<Tray>> = Mutex::new(None);

pub fn tray_on() -> bool {
    TRAY_ON.load(Ordering::SeqCst)
}

fn host(app: &AppHandle) -> Arc<LiveHost> {
    app.state::<Core>().0.live.clone()
}

// ---------------------------------------------------------------- window

/// Close handling: with the tray on, the window hides (or is destroyed to free the WebView) instead of quitting.
pub fn attach_close_handler(win: &WebviewWindow) {
    let w2 = win.clone();
    win.on_window_event(move |e| {
        if let WindowEvent::CloseRequested { api, .. } = e {
            // a preview on the screen never outlives the window (the page cannot end it any more)
            let live = w2.app_handle().state::<Core>().0.live.clone();
            if live.previewing() {
                std::thread::spawn(move || {
                    live.preview_end();
                });
            }
            if tray_on() {
                crate::window::clear_force_close();
                let destroy = w2.app_handle().state::<Core>().0.settings.load().live_destroy_window();
                if !destroy {
                    api.prevent_close();
                    let _ = w2.hide();
                }
                return; // destroy: let the close happen; the app stays up (see `keep_running`)
            }
            if crate::window::force_close_requested() {
                return;
            }
            // An upload in progress: keep the window and let the page ask in its own dialog.
            if w2.app_handle().state::<Core>().0.uploading() {
                api.prevent_close();
                let _ = tauri::Emitter::emit(&w2, "close-guard", ());
            }
        }
    });
}

/// The library start-up work (gallery scan and watcher), once, when the first window is built. `--tray` skips it until then.
fn ensure_startup(app: &AppHandle) {
    static DONE: AtomicBool = AtomicBool::new(false);
    if !DONE.swap(true, Ordering::SeqCst) {
        let c = app.state::<Core>().0.clone();
        std::thread::spawn(move || c.startup());
    }
}

/// Build the main window (hidden), add the close handler and show it.
pub fn create_window(app: &AppHandle, show: bool) -> tauri::Result<WebviewWindow> {
    create_window_as(app, show.then_some(crate::window::Present::Normal))
}

/// As `create_window`, with how it is shown (None: built hidden).
pub fn create_window_as(app: &AppHandle, how: Option<crate::window::Present>) -> tauri::Result<WebviewWindow> {
    ensure_startup(app);
    let win = crate::window::create(app)?;
    attach_close_handler(&win);
    if let Some(h) = how {
        crate::window::present(&win, h);
    }
    Ok(win)
}

/// Bring the window up (build it again if it was destroyed). Safe from any thread.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        crate::window::present(&w, crate::window::Present::Normal);
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let _ = create_window(&app, true);
    });
}

/// True when the app must stay up although its last window is gone.
pub fn keep_running() -> bool {
    tray_on()
}

// ---------------------------------------------------------------- tray

fn icon(app: &AppHandle) -> Image<'static> {
    if let Some(i) = app.default_window_icon() {
        return Image::new_owned(i.rgba().to_vec(), i.width(), i.height());
    }
    let n = 32usize;
    let mut px = vec![0u8; n * n * 4];
    for y in 0..n {
        for x in 0..n {
            let screen = (9..23).contains(&x) && (5..27).contains(&y);
            let c: [u8; 4] = if screen { [70, 160, 255, 255] } else { [28, 32, 48, 255] };
            px[(y * n + x) * 4..(y * n + x) * 4 + 4].copy_from_slice(&c);
        }
    }
    Image::new_owned(px, n as u32, n as u32)
}

unsafe extern "system" fn session_proc(hwnd: *mut core::ffi::c_void, msg: u32, wp: usize, lp: isize) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    const WM_WTSSESSION_CHANGE: u32 = 0x02B1;
    match msg {
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            // Logoff / shutdown: end the stream (loop clip by default; screen off if the user chose that).
            if wp != 0 {
                if let Some(h) = HOST.get() {
                    h.stop_fast(cfg().screen_off_on_shutdown);
                }
            }
            0
        }
        WM_POWERBROADCAST => {
            if let Some(h) = HOST.get() {
                match wp as u32 {
                    PBT_APMSUSPEND => match cfg().on_sleep.as_str() {
                        "loop" => h.hold(SLEEP, std::time::Duration::from_millis(2500)),
                        "off" => h.hold_screen_off(SLEEP),
                        _ => {}
                    },
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => h.release(SLEEP, std::time::Duration::from_secs(3)), // the screen may re-enumerate: the engine reconnects with backoff
                    _ => {}
                }
            }
            1
        }
        WM_WTSSESSION_CHANGE => {
            if let Some(h) = HOST.get() {
                let h = h.clone();
                match wp as u32 {
                    7 => match cfg().on_lock.as_str() {
                        "loop" => {
                            std::thread::spawn(move || h.hold(LOCK, std::time::Duration::from_secs(3)));
                        }
                        "off" => {
                            std::thread::spawn(move || h.hold_screen_off(LOCK));
                        }
                        _ => {}
                    },
                    8 => h.release(LOCK, std::time::Duration::from_secs(1)),
                    _ => {}
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// A never-shown top-level window: only top-level windows get WM_ENDSESSION at logoff or shutdown.
fn make_session_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        let class: Vec<u16> = "CoolerScreenLiveHost\0".encode_utf16().collect();
        let wc = WNDCLASSW { lpfnWndProc: Some(session_proc), lpszClassName: class.as_ptr(), ..std::mem::zeroed() };
        RegisterClassW(&wc);
        let hwnd = CreateWindowExW(0, class.as_ptr(), class.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null());
        if !hwnd.is_null() {
            windows_sys::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(hwnd, 0); // lock / unlock of this session
        }
    });
}

fn build_tray(app: &AppHandle) {
    let h = host(app);
    let fps = h.fps();
    let items = (|| -> tauri::Result<(Menu<Wry>, Tray)> {
        let open = MenuItem::with_id(app, "open", "Open CoolerScreen", true, None::<&str>)?;
        let pause = MenuItem::with_id(app, "pause", if h.user_paused() { "Resume live" } else { PAUSE_TEXT }, true, None::<&str>)?;
        let f30 = CheckMenuItem::with_id(app, "f30", "30 frames per second", true, fps < 45, None::<&str>)?;
        let f60 = CheckMenuItem::with_id(app, "f60", "60 frames per second", true, fps >= 45, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
        let sep = PredefinedMenuItem::separator(app)?;
        let sep2 = PredefinedMenuItem::separator(app)?;
        let menu = Menu::with_items(app, &[&open, &sep, &pause, &f30, &f60, &sep2, &quit])?;
        Ok((menu, Tray { pause, f30, f60 }))
    })();
    let Ok((menu, tray)) = items else {
        eprintln!("could not create the tray menu");
        return;
    };
    // Tauri's own tray: its menu and click events are routed by Tauri to these two closures (the `tray-icon` crate's global
    // handlers are a set-once slot that Tauri fills first, so a handler set here never receives anything).
    let built = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("CoolerScreen live")
        .icon(icon(app))
        .on_menu_event(|a: &AppHandle, ev: MenuEvent| tray_menu_click(a, ev.id().as_ref()))
        .on_tray_icon_event(|t, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_main(t.app_handle());
            }
        })
        .build(app);
    if built.is_err() {
        eprintln!("could not create the tray icon");
        return;
    }
    *TRAY.lock().unwrap() = Some(tray);
    TRAY_ON.store(true, Ordering::SeqCst);
    static HOOK: std::sync::Once = std::sync::Once::new();
    HOOK.call_once(|| {
        let a = app.clone();
        h.set_on_change(move || refresh_tray(&a)); // the tooltip follows the theme's live items
    });
    refresh_tray(app);
}

/// A tray menu item was chosen (`id`: open, pause, f30, f60, quit). The tray's menu handler calls this; the debug test hook
/// (`COOLERSCREEN_TEST_TRAY`) calls it too.
pub fn tray_menu_click(a: &AppHandle, id: &str) {
    let h = host(a);
    match id {
        "open" => show_main(a),
        "pause" => {
            // the fade-out hand-over takes about a second: not on the menu's thread
            let a = a.clone();
            std::thread::spawn(move || {
                let h = host(&a);
                h.set_user_paused(!h.user_paused());
                refresh_tray(&a);
                a.state::<Core>().0.settings_changed(); // the Settings page follows
            });
        }
        "f30" | "f60" => {
            h.set_fps(if id == "f30" { 30 } else { 60 });
            refresh_tray(a);
            a.state::<Core>().0.settings_changed();
        }
        "quit" => quit_app(a),
        _ => {}
    }
}

/// The menu labels as they stand (debug test hook).
#[cfg(debug_assertions)]
pub fn tray_state() -> String {
    match TRAY.lock().unwrap().as_ref() {
        Some(t) => format!("pause_text={:?} f30={:?} f60={:?}", t.pause.text().unwrap_or_default(), t.f30.is_checked().ok(), t.f60.is_checked().ok()),
        None => "no tray".into(),
    }
}

/// Bring the menu labels in line with the host (after a change from the tray or from the page).
pub fn refresh_tray(_app: &AppHandle) {
    let h = host(_app);
    let st = h.status();
    let (paused, fps) = (h.user_paused(), h.fps());
    if let Some(t) = _app.tray_by_id(TRAY_ID) {
        let _ = t.set_tooltip(Some(if paused { "CoolerScreen live: paused (the screen shows Embedded)".to_string() } else { if st["liveItems"] == false { "CoolerScreen live: no live items (the screen shows Embedded)".to_string() } else { format!("CoolerScreen live: {fps} fps") } }));
    }
    if let Some(t) = TRAY.lock().unwrap().as_ref() {
        let _ = t.pause.set_text(if paused { "Resume live" } else { PAUSE_TEXT });
        let _ = t.f30.set_checked(fps < 45);
        let _ = t.f60.set_checked(fps >= 45);
    }
}

fn remove_tray(app: &AppHandle) {
    let _ = app.remove_tray_by_id(TRAY_ID);
    *TRAY.lock().unwrap() = None;
    TRAY_ON.store(false, Ordering::SeqCst);
}

/// Tray "Quit": end the live stream (loop clip plays again), then exit.
pub fn quit_app(app: &AppHandle) {
    arm_exit_watchdog("tray Quit");
    let a = app.clone();
    std::thread::spawn(move || {
        host(&a).stop();
        let b = a.clone();
        let _ = a.run_on_main_thread(move || {
            remove_tray(&b);
            b.exit(0);
        });
    });
}

/// Carry out a request from `--quit` / `--restart` / `--update` or the Settings button: leave a detached helper when the app is to
/// come back (restart, update), then quit exactly like the tray's Quit. If the helper cannot be started the app does not quit.
pub fn restart_app(app: &AppHandle, req: crate::quit_cmd::Request) {
    use crate::quit_cmd::Request;
    let pid = std::process::id();
    let started = match &req {
        Request::Quit => Ok(()),
        Request::Restart => crate::relaunch::spawn_helper(pid, true, None),
        Request::Update(p) => crate::relaunch::spawn_helper(pid, true, Some(std::path::Path::new(p))),
    };
    match started {
        Ok(()) => quit_app(app),
        Err(e) => cs_core::device::log(&format!("updater: could not start the helper ({e}); the app stays up")),
    }
}

/// Settings > "Restart CoolerScreen" (troubleshooting): the tray Quit path, then a fresh copy started by the app itself with its
/// window as usual. Refused with a clear message while a job runs.
#[tauri::command]
pub async fn app_restart(app: AppHandle, core: tauri::State<'_, Core>) -> Result<Value, CoreError> {
    if core.0.busy_for_quit() {
        return Err(CoreError::new("Something is still running (an upload, a conversion or a recording). Restart when it has finished."));
    }
    cs_core::device::log("restart requested (Settings): the tray Quit path");
    let pid = std::process::id();
    crate::relaunch::spawn_helper(pid, false, None).map_err(|e| CoreError::new(format!("Could not restart: {e}")))?;
    let a = app.clone();
    quit_app(&a);
    Ok(serde_json::json!({ "restarting": true }))
}

/// Safety net: if quitting has not finished within ~3 s (a stuck USB call, a lock, a thread that will not end), leave anyway.
/// The screen then keeps playing whatever it has (its stored loop after the stream times out).
pub fn arm_exit_watchdog(why: &'static str) {
    static ARMED: AtomicBool = AtomicBool::new(false);
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(3000));
        eprintln!("quit ({why}) took more than 3 s: exiting anyway");
        std::process::exit(0);
    });
}

/// Make the running app match the settings: on = live engine + tray; off = both gone and the window brought back.
/// Starting live does not block (the engine connects on its own threads); stopping waits for the stream to end.
pub fn apply(app: &AppHandle, force: bool) {
    let core = app.state::<Core>().0.clone();
    let l = core.settings.load().live();
    *CFG.lock().unwrap() = Some(l.clone());
    let _ = HOST.set(core.live.clone());
    if l.in_app || force {
        ensure_tray(app, true);
        core.prepare_live(); // the saved screen fit and the live theme (a window-less start never ran the start-up work)
        core.live.start_with(core.live_config());
    } else {
        let a = app.clone();
        std::thread::spawn(move || {
            host(&a).stop();
            let b = a.clone();
            let _ = a.run_on_main_thread(move || {
                remove_tray(&b);
                show_main(&b);
            });
        });
    }
}

/// Create the tray icon (once) on the main thread. `wait`: block until it exists.
pub fn ensure_tray(app: &AppHandle, wait: bool) {
    let (tx, rx) = std::sync::mpsc::channel();
    let a = app.clone();
    let _ = app.run_on_main_thread(move || {
        if !tray_on() {
            make_session_window();
            build_tray(&a);
        }
        let _ = tx.send(());
    });
    if wait && std::thread::current().name() != Some("main") {
        let _ = rx.recv_timeout(std::time::Duration::from_secs(5));
    }
}

// ---------------------------------------------------------------- commands (Settings page)

/// Any of: inApp, destroyWindow, background (Gallery clip id or ""), showSeconds, music, screenOffOnShutdown,
/// onSleep / onLock ("loop" | "keep" | "off"), fps (30 | 60), paused, autostart -> app_state (with `live`).
#[tauri::command]
pub async fn live_set(app: AppHandle, core: tauri::State<'_, Core>, patch: Value) -> Result<Value, CoreError> {
    let c = core.0.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let before = c.settings.load().live();
        let mut now = before.clone();
        let b = |k: &str| patch.get(k).and_then(Value::as_bool);
        let s = |k: &str| patch.get(k).and_then(Value::as_str);
        if let Some(v) = b("inApp") { now.in_app = v; }
        if let Some(v) = b("destroyWindow") { now.destroy_window = v; }
        if let Some(v) = b("showSeconds") { now.show_seconds = v; }
        if let Some(v) = b("fpsOverlay") { now.fps_overlay = v; }
        if let Some(v) = b("music") { now.music = v; }
        if let Some(v) = b("screenOffOnShutdown") { now.screen_off_on_shutdown = v; }
        if let Some(v) = s("background") { now.background = v.chars().take(80).collect(); }
        if let Some(v) = s("onSleep").filter(|v| ["loop", "keep", "off"].contains(v)) { now.on_sleep = v.into(); }
        if let Some(v) = s("onLock").filter(|v| ["loop", "keep", "off"].contains(v)) { now.on_lock = v.into(); }
        let full = now.to_full_value();
        c.settings.update(|st| st["live"] = full).map_err(|e| CoreError::new(e.to_string()))?;
        *CFG.lock().unwrap() = Some(now.clone());
        if let Some(f) = patch.get("fps").and_then(Value::as_u64) {
            c.live.set_fps(f as u32);
        }
        if let Some(p) = b("paused") {
            c.live.set_user_paused(p);
        }
        if let Some(on) = b("autostart") {
            if !cs_core::core::autostart::set(on) {
                return Err(CoreError::new("Windows would not let CoolerScreen change its start-up entry."));
            }
        }
        if now.in_app != before.in_app {
            apply(&app, false);
        } else if c.live.running() {
            c.live.reload(&c.live_config());
        }
        refresh_tray(&app);
        Ok(c.app_state())
    })
    .await
    .map_err(|e| CoreError::new(e.to_string()))?
}
