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
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

const PAUSE_TEXT: &str = "Pause live (screen shows its normal clip)";

/// True while the tray icon exists (closing the window then hides it, and the app does not exit with its last window).
static TRAY_ON: AtomicBool = AtomicBool::new(false);
static HOST: OnceLock<Arc<LiveHost>> = OnceLock::new();
static IDS: Mutex<Option<Ids>> = Mutex::new(None);
/// The live settings as last applied (the session window's procedure has no other way to reach them).
static CFG: Mutex<Option<LiveSettings>> = Mutex::new(None);

fn cfg() -> LiveSettings {
    CFG.lock().unwrap().clone().unwrap_or_default()
}

#[derive(Clone)]
struct Ids {
    open: MenuId,
    pause: MenuId,
    f30: MenuId,
    f60: MenuId,
    quit: MenuId,
}

struct Tray {
    _icon: TrayIcon,
    pause: MenuItem,
    f30: CheckMenuItem,
    f60: CheckMenuItem,
}

thread_local! {
    /// Menu objects are not Send: they live on the main thread.
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
}

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
    ensure_startup(app);
    let win = crate::window::create(app)?;
    attach_close_handler(&win);
    if show {
        win.show()?;
    }
    Ok(win)
}

/// Bring the window up (build it again if it was destroyed). Safe from any thread.
pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        if let Ok(w) = create_window(&app, true) {
            let _ = w.set_focus();
        }
    });
}

/// True when the app must stay up although its last window is gone.
pub fn keep_running() -> bool {
    tray_on()
}

// ---------------------------------------------------------------- tray

fn icon(app: &AppHandle) -> tray_icon::Icon {
    if let Some(i) = app.default_window_icon() {
        if let Ok(t) = tray_icon::Icon::from_rgba(i.rgba().to_vec(), i.width(), i.height()) {
            return t;
        }
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
    tray_icon::Icon::from_rgba(px, n as u32, n as u32).expect("icon")
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
    let menu = Menu::new();
    let open = MenuItem::new("Open CoolerScreen", true, None);
    let pause = MenuItem::new(if h.user_paused() { "Resume live" } else { PAUSE_TEXT }, true, None);
    let f30 = CheckMenuItem::new("30 frames per second", true, fps < 45, None);
    let f60 = CheckMenuItem::new("60 frames per second", true, fps >= 45, None);
    let quit = MenuItem::new("Quit", true, None);
    let _ = menu.append_items(&[&open, &PredefinedMenuItem::separator(), &pause, &f30, &f60, &PredefinedMenuItem::separator(), &quit]);
    let built = TrayIconBuilder::new().with_menu(Box::new(menu)).with_menu_on_left_click(false).with_tooltip("CoolerScreen live").with_icon(icon(app)).build();
    let Ok(ic) = built else {
        eprintln!("could not create the tray icon");
        return;
    };
    *IDS.lock().unwrap() = Some(Ids { open: open.id().clone(), pause: pause.id().clone(), f30: f30.id().clone(), f60: f60.id().clone(), quit: quit.id().clone() });
    TRAY.with(|t| *t.borrow_mut() = Some(Tray { _icon: ic, pause, f30, f60 }));
    TRAY_ON.store(true, Ordering::SeqCst);

    let a = app.clone();
    MenuEvent::set_event_handler(Some(move |ev: MenuEvent| {
        let Some(ids) = IDS.lock().unwrap().clone() else { return };
        let h = host(&a);
        if ev.id == ids.open {
            show_main(&a);
        } else if ev.id == ids.pause {
            h.set_user_paused(!h.user_paused());
            refresh_tray(&a);
        } else if ev.id == ids.f30 || ev.id == ids.f60 {
            h.set_fps(if ev.id == ids.f30 { 30 } else { 60 });
            refresh_tray(&a);
        } else if ev.id == ids.quit {
            quit_app(&a);
        }
    }));
    let a = app.clone();
    TrayIconEvent::set_event_handler(Some(move |ev: TrayIconEvent| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
            show_main(&a);
        }
    }));
}

/// Bring the menu labels in line with the host (after a change from the tray or from the page).
pub fn refresh_tray(app: &AppHandle) {
    let h = host(app);
    let (paused, fps) = (h.user_paused(), h.fps());
    let _ = app.run_on_main_thread(move || {
        TRAY.with(|t| {
            if let Some(t) = t.borrow().as_ref() {
                t.pause.set_text(if paused { "Resume live" } else { PAUSE_TEXT });
                t.f30.set_checked(fps < 45);
                t.f60.set_checked(fps >= 45);
            }
        })
    });
}

fn remove_tray() {
    TRAY.with(|t| *t.borrow_mut() = None);
    MenuEvent::set_event_handler(None::<fn(MenuEvent)>);
    TrayIconEvent::set_event_handler(None::<fn(TrayIconEvent)>);
    TRAY_ON.store(false, Ordering::SeqCst);
}

/// Tray "Quit": end the live stream (loop clip plays again), then exit.
pub fn quit_app(app: &AppHandle) {
    let a = app.clone();
    std::thread::spawn(move || {
        host(&a).stop();
        let b = a.clone();
        let _ = a.run_on_main_thread(move || {
            remove_tray();
            b.exit(0);
        });
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
        core.refresh_live_theme(); // the current theme (or the Standard layout) is what live shows
        core.live.start_with(core.live_config());
    } else {
        let a = app.clone();
        std::thread::spawn(move || {
            host(&a).stop();
            let b = a.clone();
            let _ = a.run_on_main_thread(move || {
                remove_tray();
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
