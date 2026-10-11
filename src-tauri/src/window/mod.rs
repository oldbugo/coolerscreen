//! Seamless frameless window (vault pattern "Seamless frameless window").
//!
//! The window has no native frame; the page draws a transparent 32 px title bar with three 46x32 buttons
//! (public/js/titlebar.js). Windows 11 Snap Layouts (hover over maximise, Win+Z) need the system to see
//! HTMAXBUTTON in WM_NCHITTEST over the maximise button, but WebView2 covers the whole client area and
//! answers hit tests itself. So `snap` puts a small invisible native child window over the maximise
//! button: it answers HTMAXBUTTON (triggering the flyout) and forwards its mouse state to the page as
//! `cs-snap` events, so the HTML button can still draw hover / pressed / click.
//! Size, position and maximised state are remembered in %APPDATA%\CoolerScreen\window.json (`rect`).
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{App, Emitter, WebviewWindow, WebviewWindowBuilder, WindowEvent};

#[cfg(windows)]
mod rect;
#[cfg(windows)]
mod snap;

pub const BG: (u8, u8, u8) = (0x0f, 0x11, 0x15); // public/style.css --bg
pub const TITLEBAR_H: u32 = 32;
pub const BUTTON_W: u32 = 46;
/// Buttons right of maximise (close).
pub const BUTTONS_RIGHT_OF_MAX: u32 = 1;

static FORCE_CLOSE: AtomicBool = AtomicBool::new(false);

/// True after the page confirmed "close anyway" (skips the owner's close guard).
pub fn force_close_requested() -> bool {
    FORCE_CLOSE.load(Ordering::SeqCst)
}

/// Builds the main window (hidden; the caller shows it). The caller adds its own `CloseRequested` guard.
pub fn create(app: &mut App) -> tauri::Result<WebviewWindow> {
    let saved = load_saved(app);
    let win = WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
        .title("CoolerScreen")
        .inner_size(1280.0, 860.0)
        .min_inner_size(900.0, 640.0)
        .decorations(false)
        .shadow(true)
        .background_color(tauri::webview::Color(BG.0, BG.1, BG.2, 255))
        .disable_drag_drop_handler() // HTML5 drop reaches the page, as in Electron
        .visible(false)
        .build()?;
    #[cfg(windows)]
    if let Some(s) = saved {
        // Same logical size on the monitor the window lands on (scale may differ from when it was saved).
        let k = win.scale_factor().unwrap_or(s.scale) / s.scale;
        rect::set(&win, s.x, s.y, (s.w as f64 * k).round() as u32, (s.h as f64 * k).round() as u32);
        if s.maximized {
            let _ = win.maximize(); // un-maximising returns to the rectangle above
        }
    }
    *CURRENT.lock().unwrap() = saved;
    let w2 = win.clone();
    win.on_window_event(move |e| match e {
        WindowEvent::Resized(_) => {
            let _ = w2.emit("window-state", json!({ "maximized": w2.is_maximized().unwrap_or(false) }));
            remember(&w2, false);
        }
        WindowEvent::Moved(_) => remember(&w2, false),
        WindowEvent::CloseRequested { .. } | WindowEvent::Destroyed => remember(&w2, true),
        _ => {}
    });
    #[cfg(windows)]
    snap::install(&win, TITLEBAR_H, BUTTON_W, BUTTONS_RIGHT_OF_MAX);
    Ok(win)
}

// ---------- remembered size / position / maximised state ----------

/// Outer rectangle in physical pixels of the last normal (not maximised, not minimised) window, plus the scale
/// factor it was saved at, so a different monitor scale restores the same logical size.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
struct Saved {
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    scale: f64,
    maximized: bool,
}

static CURRENT: Mutex<Option<Saved>> = Mutex::new(None);
static LAST_WRITE: Mutex<Option<Instant>> = Mutex::new(None);

fn state_path() -> std::path::PathBuf {
    cs_core::core::paths::base().join("window.json")
}

/// The saved rectangle, if enough of its title bar is on a connected monitor.
fn load_saved(app: &App) -> Option<Saved> {
    let s = std::fs::read_to_string(state_path()).ok().and_then(|t| serde_json::from_str::<Saved>(t.trim_start_matches('\u{feff}')).ok())?;
    if s.w < 300 || s.h < 300 || !(s.scale > 0.4 && s.scale < 8.0) {
        return None;
    }
    let mons = app.available_monitors().unwrap_or_default();
    if mons.is_empty() {
        return Some(s); // cannot tell: trust the file
    }
    let visible = mons.iter().any(|m| {
        let (mp, ms) = (m.position(), m.size());
        let ox = (s.x + s.w as i32).min(mp.x + ms.width as i32) - s.x.max(mp.x);
        let oy = (s.y + 80).min(mp.y + ms.height as i32) - s.y.max(mp.y);
        ox >= 160 && oy >= 40
    });
    visible.then_some(s) // off-screen (monitor unplugged): the default size and place
}

fn remember(win: &WebviewWindow, force: bool) {
    if win.is_minimized().unwrap_or(false) {
        return;
    }
    let maximized = win.is_maximized().unwrap_or(false);
    let mut cur = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = cur.unwrap_or(Saved { x: 0, y: 0, w: 0, h: 0, scale: 1.0, maximized });
    s.maximized = maximized;
    if !maximized {
        #[cfg(windows)]
        if let (Some((x, y, w, h)), Ok(f)) = (rect::get(win), win.scale_factor()) {
            (s.x, s.y, s.w, s.h, s.scale) = (x, y, w, h, f);
        }
    }
    if s.w == 0 {
        return; // never saw a normal rectangle yet
    }
    *cur = Some(s);
    drop(cur);
    let mut last = LAST_WRITE.lock().unwrap_or_else(|e| e.into_inner());
    if force || last.is_none_or(|t| t.elapsed() > Duration::from_millis(500)) {
        *last = Some(Instant::now());
        if let Ok(t) = serde_json::to_string(&s) {
            let _ = std::fs::create_dir_all(cs_core::core::paths::base());
            let _ = std::fs::write(state_path(), t);
        }
    }
}

#[tauri::command]
pub fn win_minimize(window: WebviewWindow) {
    let _ = window.minimize();
}

/// Maximise to the work area (taskbar stays visible); returns the new maximised state.
#[tauri::command]
pub fn win_toggle_maximize(window: WebviewWindow) -> bool {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
        false
    } else {
        let _ = window.maximize();
        true
    }
}

#[tauri::command]
pub fn win_state(window: WebviewWindow) -> bool {
    window.is_maximized().unwrap_or(false)
}

#[tauri::command]
pub fn win_close(window: WebviewWindow) {
    let _ = window.close(); // goes through the owner's close guard
}

#[tauri::command]
pub fn win_force_close(window: WebviewWindow) {
    FORCE_CLOSE.store(true, Ordering::SeqCst);
    let _ = window.close();
}
