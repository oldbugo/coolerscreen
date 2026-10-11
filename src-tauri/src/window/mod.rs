//! Seamless frameless window (vault pattern "Seamless frameless window").
//!
//! The window has no native frame; the page draws a transparent 32 px title bar with three 46x32 buttons
//! (public/js/titlebar.js). Windows 11 Snap Layouts (hover over maximise, Win+Z) need the system to see
//! HTMAXBUTTON in WM_NCHITTEST over the maximise button, but WebView2 covers the whole client area and
//! answers hit tests itself. So `snap` puts a small invisible native child window over the maximise
//! button: it answers HTMAXBUTTON (triggering the flyout) and forwards its mouse state to the page as
//! `cs-snap` events, so the HTML button can still draw hover / pressed / click.
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{App, Emitter, WebviewWindow, WebviewWindowBuilder, WindowEvent};

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
    let w2 = win.clone();
    win.on_window_event(move |e| {
        if let WindowEvent::Resized(_) = e {
            let _ = w2.emit("window-state", json!({ "maximized": w2.is_maximized().unwrap_or(false) }));
        }
    });
    #[cfg(windows)]
    snap::install(&win, TITLEBAR_H, BUTTON_W, BUTTONS_RIGHT_OF_MAX);
    Ok(win)
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
