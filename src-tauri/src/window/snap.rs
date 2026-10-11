//! Invisible native child window over the maximise button: answers WM_NCHITTEST with HTMAXBUTTON so
//! Windows 11 shows Snap Layouts, and reports mouse state to the page (`cs-snap`: enter/leave/down/click/up).
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Mutex;
use tauri::{Emitter, WebviewWindow};
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, LRESULT, WPARAM},
    Graphics::Gdi::{GetStockObject, HBRUSH, NULL_BRUSH},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::{TrackMouseEvent, TME_LEAVE, TME_NONCLIENT, TRACKMOUSEEVENT},
        Shell::{DefSubclassProc, SetWindowSubclass},
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, GetClientRect, RegisterClassExW, SetWindowPos, CS_HREDRAW, CS_VREDRAW,
            HTMAXBUTTON, HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW, WM_DPICHANGED, WM_NCHITTEST, WM_NCLBUTTONDOWN,
            WM_NCLBUTTONUP, WM_NCMOUSELEAVE, WM_NCMOUSEMOVE, WM_SIZE, WNDCLASSEXW, WS_CHILD, WS_CLIPSIBLINGS,
            WS_VISIBLE,
        },
    },
};

const CLASS: &[u16] = &[
    b'C' as u16, b'S' as u16, b'S' as u16, b'n' as u16, b'a' as u16, b'p' as u16, b'M' as u16, b'a' as u16, b'x' as u16, 0,
];
const SUBCLASS_ID: usize = 0x4353_534e; // "CSSN"

struct State {
    parent: isize,
    overlay: isize,
    height: u32,
    width: u32,
    right: u32,
    pressing: bool,
    emit: Box<dyn Fn(&'static str) + Send>,
}
static STATE: Mutex<Option<State>> = Mutex::new(None); // one main window

pub fn install(win: &WebviewWindow, height: u32, width: u32, right: u32) {
    let Ok(h) = win.window_handle() else { return };
    let RawWindowHandle::Win32(h) = h.as_raw() else { return };
    let parent = h.hwnd.get();
    let w = win.clone();
    // Window creation and subclassing must happen on the thread that owns the window.
    let _ = win.run_on_main_thread(move || unsafe {
        register_class();
        let overlay = CreateWindowExW(
            0, CLASS.as_ptr(), CLASS.as_ptr(), WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS, 0, 0, 0, 0,
            parent as HWND, std::ptr::null_mut(), GetModuleHandleW(std::ptr::null()), std::ptr::null(),
        );
        if overlay.is_null() {
            return;
        }
        *STATE.lock().unwrap() = Some(State {
            parent, overlay: overlay as isize, height, width, right, pressing: false,
            emit: Box::new(move |ev| {
                let _ = w.emit("cs-snap", ev);
            }),
        });
        SetWindowSubclass(parent as HWND, Some(parent_proc), SUBCLASS_ID, 0);
        position();
    });
}

unsafe fn register_class() {
    let class = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(overlay_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: GetModuleHandleW(std::ptr::null()),
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: GetStockObject(NULL_BRUSH) as HBRUSH,
        lpszMenuName: std::ptr::null(),
        lpszClassName: CLASS.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    RegisterClassExW(&class);
}

/// Maximise button = the button just left of the `right` buttons, measured from the right client edge.
unsafe fn position() {
    let g = STATE.lock().unwrap();
    let Some(s) = g.as_ref() else { return };
    let mut rc = std::mem::zeroed();
    if GetClientRect(s.parent as HWND, &mut rc) == 0 {
        return;
    }
    let dpi = GetDpiForWindow(s.parent as HWND).max(96) as u64;
    let sc = |v: u32| ((v as u64 * dpi + 48) / 96) as i32;
    let (w, h) = (sc(s.width), sc(s.height));
    let x = rc.right - w * (s.right as i32 + 1);
    SetWindowPos(s.overlay as HWND, HWND_TOP, x, 0, w, h, SWP_SHOWWINDOW | SWP_NOACTIVATE);
}

unsafe extern "system" fn parent_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _r: usize) -> LRESULT {
    if msg == WM_SIZE || msg == WM_DPICHANGED {
        position();
    }
    DefSubclassProc(hwnd, msg, wp, lp)
}

fn notify(ev: &'static str) {
    if let Some(s) = STATE.lock().unwrap().as_ref() {
        (s.emit)(ev);
    }
}

unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCHITTEST => return HTMAXBUTTON as LRESULT,
        WM_NCMOUSEMOVE => {
            let mut t = TRACKMOUSEEVENT {
                cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                dwFlags: TME_LEAVE | TME_NONCLIENT,
                hwndTrack: hwnd,
                dwHoverTime: 0,
            };
            TrackMouseEvent(&mut t);
            notify("enter");
            return 0;
        }
        WM_NCMOUSELEAVE => {
            if let Some(s) = STATE.lock().unwrap().as_mut() {
                s.pressing = false;
            }
            notify("leave");
            return 0;
        }
        WM_NCLBUTTONDOWN => {
            if let Some(s) = STATE.lock().unwrap().as_mut() {
                s.pressing = true;
            }
            notify("down");
            return 0;
        }
        WM_NCLBUTTONUP => {
            let click = STATE.lock().unwrap().as_mut().map(|s| std::mem::take(&mut s.pressing)).unwrap_or(false);
            notify(if click { "click" } else { "up" });
            return 0;
        }
        _ => {}
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}
