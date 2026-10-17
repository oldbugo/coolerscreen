//! `cs2 edge-tune`: temporary tool to find how much of each screen edge is really visible.
//! Streams a plain tuning picture live at the known-good size (landscape 1680x756 = the 2400x1080 design x 0.7; `--scale 1.0` = full 2400x1080, which the screen does NOT show), rotated 90 deg
//! clockwise into a 1080x2400 portrait 4:2:0 JPEG exactly like the real engine (live/encode.rs rotate_cw), no notch or
//! corner mask. Four offsets (top, right, bottom, left, in px) shrink the content rectangle from the full canvas inward.
//! Sends only: FF D9 FF D9, 0x15 [00 0A], 0x11 keepalives, frames, and the normal ending (0x0E [1], 0x14 [1,0]).
//! `--dry-run [--offsets T,R,B,L] [--select top|right|bottom|left] [--grid]` writes work/edge-tune.jpg (+ .png) instead.
use super::live::{encode_fit, glyph, Canvas};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

const MAX_OFFSET: i32 = 200; // screen px
const DEFAULT_SCALE: f64 = 0.7; // 1680x756 landscape = the known-good live size (vendor profile)
fn max_bytes(k: f64) -> usize {
    if k < 1.0 { 200_000 } else { 250_000 }
}
const NAMES: [&str; 4] = ["TOP", "RIGHT", "BOTTOM", "LEFT"];
const KEYS: [&str; 4] = ["top", "right", "bottom", "left"];

static ABORT: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
unsafe extern "system" fn on_ctrl(_t: u32) -> i32 {
    ABORT.store(true, Ordering::SeqCst);
    1
}

#[derive(Clone, PartialEq)]
struct State {
    off: [i32; 4], // top, right, bottom, left
    sel: usize,
    grid: bool,
    k: f64, // sent pixels per screen pixel (offsets in `off` are in SENT pixels)
}

// ------------------------------------------------------------------ drawing

type Rgb = [u8; 3];
const WHITE: Rgb = [255, 255, 255];
const RED: Rgb = [255, 40, 40];
const YELLOW: Rgb = [255, 230, 0];
const CYAN: Rgb = [0, 230, 255];

fn px(c: &mut Canvas, x: i64, y: i64, p: Rgb) {
    if x >= 0 && y >= 0 && (x as usize) < c.w && (y as usize) < c.h {
        c.set(x as usize, y as usize, p);
    }
}

/// Filled rectangle [x0, x1) x [y0, y1), clipped to the canvas.
fn fill(c: &mut Canvas, x0: i64, y0: i64, x1: i64, y1: i64, p: Rgb) {
    for y in y0.max(0)..y1.min(c.h as i64) {
        for x in x0.max(0)..x1.min(c.w as i64) {
            c.set(x as usize, y as usize, p);
        }
    }
}

fn glyph2(ch: char) -> [u8; 7] {
    match ch.to_ascii_uppercase() {
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '/' => [1, 2, 2, 4, 8, 8, 16],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        ',' => [0, 0, 0, 0, 12, 4, 8],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        ch => glyph(ch),
    }
}

fn text_w(s: usize, t: &str) -> i64 {
    (t.chars().count() * 6 * s) as i64
}

/// Text with its top-left at (x, y), `s` px per glyph dot (glyph is 5x7 dots, 6 dots advance).
fn text(c: &mut Canvas, x: i64, y: i64, s: usize, t: &str, p: Rgb) {
    for (k, ch) in t.chars().enumerate() {
        for (ry, row) in glyph2(ch).iter().enumerate() {
            for rx in 0..5 {
                if row & (16 >> rx) != 0 {
                    let (gx, gy) = (x + ((k * 6 + rx) * s) as i64, y + (ry * s) as i64);
                    fill(c, gx, gy, gx + s as i64, gy + s as i64, p);
                }
            }
        }
    }
}

/// The content rectangle [x0, x1) x [y0, y1): the canvas inset by the four offsets.
struct Rect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Rect {
    /// Canvas position of "`d` pixels inward from edge `e`, `along` pixels along the canvas axis of that edge".
    fn at(&self, e: usize, along: i64, d: i64) -> (i64, i64) {
        match e {
            0 => (along, self.y0 + d),
            2 => (along, self.y1 - 1 - d),
            3 => (self.x0 + d, along),
            _ => (self.x1 - 1 - d, along),
        }
    }

    /// Range of the along-axis of edge `e` (x for top/bottom, y for left/right).
    fn span(&self, e: usize) -> (i64, i64) {
        if e == 0 || e == 2 { (self.x0, self.x1) } else { (self.y0, self.y1) }
    }
}

fn draw(st: &State) -> Canvas {
    let mut c = Canvas::blank(st.k, [0, 0, 0]);
    let (cw, ch) = (c.w as i64, c.h as i64);
    let r = Rect { x0: st.off[3] as i64, y0: st.off[0] as i64, x1: cw - st.off[1] as i64, y1: ch - st.off[2] as i64 };
    if st.grid {
        let gs = ((20.0 * st.k).round() as usize).max(2);
        for x in (0..cw).step_by(gs) {
            fill(&mut c, x, 0, x + 1, ch, if (x / gs as i64) % 5 == 0 { [120, 120, 120] } else { [55, 55, 55] });
        }
        for y in (0..ch).step_by(gs) {
            fill(&mut c, 0, y, cw, y + 1, if (y / gs as i64) % 5 == 0 { [120, 120, 120] } else { [55, 55, 55] });
        }
    }

    // rulers: three per edge (15 %, 50 %, 85 % along it); a tick every 2 px (alternating colours) for 60 px,
    // a long white tick with a number every 10 px. d = pixels inward from the content edge.
    for e in 0..4 {
        let (a0, a1) = r.span(e);
        for frac in if e == 0 || e == 2 { [15i64, 50, 85] } else { [6, 42, 76] } {
            let along = a0 + (a1 - a0) * frac / 100;
            for d in (0..=60i64).step_by(2) {
                let long = d % 10 == 0;
                let lane = 6 - d / 10; // staircase: each label hangs at the end of its own tick, later ticks are shorter and never cross it
                let step = if e == 0 || e == 2 { 30 } else { 18 };
                let len = if long { 24 + lane * step } else { 8 };
                let col = if long { WHITE } else if (d / 2) % 2 == 0 { YELLOW } else { CYAN };
                for b in 0..len {
                    let (x, y) = r.at(e, along + b, d);
                    px(&mut c, x, y, col);
                }
                if long {
                    let label = ((d as f64) / st.k).round().to_string();
                    let lw = text_w(2, &label) - 2;
                    let (tx, ty) = r.at(e, along, d);
                    let (lx, ly) = match e {
                        0 => (tx + 28 + lane * 30, ty),
                        2 => (tx + 28 + lane * 30, ty - 13),
                        3 => (tx, ty + 28 + lane * 18),
                        _ => (tx + 1 - lw, ty + 28 + lane * 18),
                    };
                    text(&mut c, lx, ly, 2, &label, WHITE);
                }
            }
        }
    }

    // edge lines, exactly on the content edge (1 px); the selected one in red, drawn last
    for e in (0..4).filter(|&e| e != st.sel).chain(std::iter::once(st.sel)) {
        let p = if e == st.sel { RED } else { WHITE };
        match e {
            0 => fill(&mut c, r.x0, r.y0, r.x1, r.y0 + 1, p),
            2 => fill(&mut c, r.x0, r.y1 - 1, r.x1, r.y1, p),
            3 => fill(&mut c, r.x0, r.y0, r.x0 + 1, r.y1, p),
            _ => fill(&mut c, r.x1 - 1, r.y0, r.x1, r.y1, p),
        }
    }

    // arrows pointing at the selected edge (30 % and 70 % along it): tip 30 px inside, head 80 px, shaft to 170 px
    let (a0, a1) = r.span(st.sel);
    for frac in [30i64, 70] {
        let along = a0 + (a1 - a0) * frac / 100;
        for d in 30..170i64 {
            let half = if d < 110 { (d - 30) / 2 } else { 12 };
            for b in -half..=half {
                let (x, y) = r.at(st.sel, along + b, d);
                px(&mut c, x, y, RED);
            }
        }
    }

    // big readout in the middle, the selected value on a yellow box
    let s = if st.k < 1.0 { 4usize } else { 6 };
    let hs = if st.k < 1.0 { 2usize } else { 3 };
    let ss = s * 2 / 3;
    let parts: Vec<String> = (0..4).map(|i| format!("{} {}", NAMES[i], scr(st, i))).collect();
    let gap = 3 * 6 * s as i64;
    let total: i64 = parts.iter().map(|p| text_w(s, p)).sum::<i64>() + gap * 3;
    let mut x = (cw - total) / 2;
    let y = ch / 2 - (7 * s as i64) / 2;
    for (i, p) in parts.iter().enumerate() {
        let w = text_w(s, p);
        if i == st.sel {
            fill(&mut c, x - 2 * s as i64, y - 2 * s as i64, x + w + s as i64, y + 9 * s as i64, YELLOW);
            text(&mut c, x, y, s, p, [0, 0, 0]);
        } else {
            text(&mut c, x, y, s, p, WHITE);
        }
        x += w + gap;
    }
    let sel_line = format!("SELECTED: {}   (1-4 OR T R B L)", NAMES[st.sel]);
    text(&mut c, (cw - text_w(ss, &sel_line)) / 2, y - (s as i64 * 10 + 20), ss, &sel_line, RED);
    let help = [
        "UP OR + = EDGE INWARD   DOWN OR - = BACK   (STEPS OF ABOUT 1.4 PX)   SHIFT OR PGUP/PGDN = 10 STEPS   0 = RESET",
        "S = SAVE AND PRINT   G = GRID   Q OR ESC = QUIT",
    ];
    for (i, h) in help.iter().enumerate() {
        text(&mut c, (cw - text_w(hs, h)) / 2, y + 7 * s as i64 + 40 + i as i64 * (hs as i64 * 13), hs, h, WHITE);
    }
    c
}

// ------------------------------------------------------------------ saving

fn json_path() -> PathBuf {
    cs_core::core::paths::base().join("edge-offsets.json")
}

/// Offsets in screen px as saved; the caller converts to sent px.
fn load() -> Option<[i32; 4]> {
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(json_path()).ok()?).ok()?;
    let mut o = [0i32; 4];
    for i in 0..4 {
        o[i] = (v[KEYS[i]].as_i64()? as i32).clamp(0, MAX_OFFSET);
    }
    Some(o)
}

fn save(st: &State) -> Result<PathBuf, String> {
    let p = json_path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let v = serde_json::json!({
        "top": scr(st, 0), "right": scr(st, 1), "bottom": scr(st, 2), "left": scr(st, 3),
        "note": "screen pixels (2400x1080 space) hidden at each edge of the landscape picture (top = top as you look at the screen); content rectangle = canvas inset by these"
    });
    std::fs::write(&p, serde_json::to_string_pretty(&v).unwrap()).map_err(|e| e.to_string())?;
    Ok(p)
}

fn describe(st: &State) -> String {
    format!(
        "Top {} | Right {} | Bottom {} | Left {}   (selected: {})",
        scr(st, 0), scr(st, 1), scr(st, 2), scr(st, 3), NAMES[st.sel].to_lowercase()
    )
}

// ------------------------------------------------------------------ input

enum Key {
    Select(usize),
    Step(i32),
    Reset,
    Save,
    Grid,
    Quit,
    Next,
}

#[cfg(windows)]
mod con {
    use super::Key;
    use windows_sys::Win32::System::Console::*;
    use windows_sys::Win32::System::Threading::WaitForSingleObject;

    pub struct Console {
        h: windows_sys::Win32::Foundation::HANDLE,
        old: u32,
        ok: bool,
    }

    impl Console {
        pub fn new() -> Console {
            unsafe {
                let h = GetStdHandle(STD_INPUT_HANDLE);
                let mut old = 0u32;
                let ok = GetConsoleMode(h, &mut old) != 0;
                if ok {
                    // keep ENABLE_PROCESSED_INPUT so Ctrl+C still reaches the handler
                    SetConsoleMode(h, (old & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT)) | ENABLE_PROCESSED_INPUT);
                }
                Console { h, old, ok }
            }
        }

        pub fn is_console(&self) -> bool {
            self.ok
        }

        /// Wait up to `ms` for keys; returns the commands read.
        pub fn poll(&self, ms: u32) -> Vec<Key> {
            let mut out = Vec::new();
            unsafe {
                if WaitForSingleObject(self.h, ms) != 0 {
                    return out;
                }
                let mut n = 0u32;
                if GetNumberOfConsoleInputEvents(self.h, &mut n) == 0 || n == 0 {
                    return out;
                }
                let mut recs: Vec<INPUT_RECORD> = vec![std::mem::zeroed(); n.min(64) as usize];
                let mut got = 0u32;
                if ReadConsoleInputW(self.h, recs.as_mut_ptr(), recs.len() as u32, &mut got) == 0 {
                    return out;
                }
                for r in &recs[..got as usize] {
                    if r.EventType != KEY_EVENT as u16 {
                        continue;
                    }
                    let k = r.Event.KeyEvent;
                    if k.bKeyDown == 0 {
                        continue;
                    }
                    let shift = k.dwControlKeyState & SHIFT_PRESSED != 0;
                    let ch = char::from_u32(k.uChar.UnicodeChar as u32).unwrap_or('\0').to_ascii_lowercase();
                    let key = match (k.wVirtualKeyCode, ch) {
                        (0x1B, _) => Key::Quit,
                        (0x26, _) => Key::Step(if shift { 10 } else { 1 }),
                        (0x28, _) => Key::Step(if shift { -10 } else { -1 }),
                        (0x21, _) => Key::Step(10),
                        (0x22, _) => Key::Step(-10),
                        (0x09, _) => Key::Next,
                        (_, '1') | (_, 't') => Key::Select(0),
                        (_, '2') | (_, 'r') => Key::Select(1),
                        (_, '3') | (_, 'b') => Key::Select(2),
                        (_, '4') | (_, 'l') => Key::Select(3),
                        (_, '+') | (_, '=') => Key::Step(1),
                        (_, '-') | (_, '_') => Key::Step(-1),
                        (_, '0') => Key::Reset,
                        (_, 's') => Key::Save,
                        (_, 'g') => Key::Grid,
                        (_, 'q') => Key::Quit,
                        _ => continue,
                    };
                    out.push(key);
                }
            }
            out
        }
    }

    impl Drop for Console {
        fn drop(&mut self) {
            if self.ok {
                unsafe { SetConsoleMode(self.h, self.old) };
            }
        }
    }
}

// ------------------------------------------------------------------ main

fn arg_val<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn dry_run(args: &[String], mut st: State) -> i32 {
    if let Some(o) = arg_val(args, "--offsets") {
        let v: Vec<i32> = o.split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if v.len() != 4 {
            eprintln!("--offsets needs four numbers: top,right,bottom,left");
            return 2;
        }
        for i in 0..4 {
            st.off[i] = ((v[i].clamp(0, MAX_OFFSET) as f64) * st.k).round() as i32;
        }
    }
    if let Some(s) = arg_val(args, "--select") {
        match KEYS.iter().position(|k| *k == s) {
            Some(i) => st.sel = i,
            None => {
                eprintln!("--select: top, right, bottom or left");
                return 2;
            }
        }
    }
    st.grid = args.iter().any(|a| a == "--grid");
    let c = draw(&st);
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("work");
    let _ = std::fs::create_dir_all(&dir);
    match encode_fit(c.w, c.h, &c.rgb, 95, max_bytes(st.k)) {
        Ok((j, _)) => {
            let p = dir.join("edge-tune.jpg");
            std::fs::write(&p, &j).unwrap();
            println!("Wrote {} ({} KB, landscape {}x{}). {}", p.display(), j.len() / 1000, c.w, c.h, describe(&st));
        }
        Err(e) => {
            eprintln!("{e}");
            return 1;
        }
    }
    // lossless copy for pixel checks
    let mut rgba = Vec::with_capacity(c.w * c.h * 4);
    for p in c.rgb.chunks_exact(3) {
        rgba.extend_from_slice(&[p[0], p[1], p[2], 255]);
    }
    if let Some(pm) = tiny_skia::IntSize::from_wh(c.w as u32, c.h as u32).and_then(|s| tiny_skia::Pixmap::from_vec(rgba, s)) {
        let _ = pm.save_png(dir.join("edge-tune.png"));
    }
    // the portrait frame as the screen would get it
    let (pw, ph, rgb) = c.pixels(true);
    match encode_fit(pw, ph, &rgb, 95, max_bytes(st.k)) {
        Ok((j, q)) => println!("Portrait frame {pw}x{ph}: {} KB at quality {q}.", j.len() / 1000),
        Err(e) => eprintln!("{e}"),
    }
    0
}

pub fn edge_tune(args: &[String]) -> i32 {
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }
    let k = scale_arg(args);
    let off = load().map(|o| o.map(|v| ((v as f64) * k).round() as i32)).unwrap_or([0; 4]);
    let mut st = State { off, sel: 0, grid: false, k };
    if args.iter().any(|a| a == "--dry-run") {
        return dry_run(args, st);
    }
    println!("CoolerScreen edge tuning (temporary tool)");
    println!("Find how much of each edge of the screen is really visible. The screen shows a black picture with a thin");
    println!("line on each edge of a 'content rectangle' and rulers going inward. Move an edge inward until its line");
    println!("(and the 0 of its ruler) is just visible: that offset is the number of hidden pixels on that edge.");
    println!("Top means the top of the picture as you look at the screen. The picture is sent at 1680x756, so one key press");
    println!("is one sent pixel = about 1.4 screen pixels; all printed and saved values are in screen pixels (2400x1080 space).");
    println!("If the screen stays black, press Q and tell Claude.");
    println!();
    println!("  1 2 3 4 or T R B L         choose Top, Right, Bottom, Left (Tab also cycles)");
    println!("  Up or +                    move the chosen edge inward one step (about 1.4 screen pixels)");
    println!("  Down or -                  move it back out one step (lowest value is 0)");
    println!("  Shift+Up/Down, PgUp/PgDn   the same in steps of 10");
    println!("  0                          reset the chosen edge to 0");
    println!("  S                          save to {} and print the values", json_path().display());
    println!("  G                          show or hide a grid (every 20 screen pixels) over the whole screen");
    println!("  Q, Esc or Ctrl+C           quit (the screen goes back to its stored loop clip)");
    println!();
    let mut session = match cs_core::live::session::LiveSession::connect() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Could not open the screen: {e}");
            eprintln!("Quit CoolerScreen from the tray first (only one program can use the screen), then run this again.");
            return 1;
        }
    };
    let con = con::Console::new();
    if !con.is_console() {
        eprintln!("Keyboard input needs a normal console window (not a pipe).");
        session.stop();
        return 1;
    }
    println!("Connected. {}", describe(&st));
    let mut dirty = true;
    let mut code = 0;
    loop {
        if ABORT.load(Ordering::SeqCst) {
            break;
        }
        let mut quit = false;
        let before = st.clone();
        for k in con.poll(80) {
            match k {
                Key::Select(i) => st.sel = i,
                Key::Next => st.sel = (st.sel + 1) % 4,
                Key::Step(d) => st.off[st.sel] = (st.off[st.sel] + d).clamp(0, max_sent(st.k)),
                Key::Reset => st.off[st.sel] = 0,
                Key::Grid => st.grid = !st.grid,
                Key::Save => match save(&st) {
                    Ok(p) => println!("SAVED to {}: top {}, right {}, bottom {}, left {}", p.display(), scr(&st, 0), scr(&st, 1), scr(&st, 2), scr(&st, 3)),
                    Err(e) => println!("Could not save: {e}"),
                },
                Key::Quit => quit = true,
            }
        }
        if st != before {
            dirty = true;
            if st.grid != before.grid {
                println!("Grid {}", if st.grid { "on" } else { "off" });
            } else {
                println!("{}", describe(&st));
            }
        }
        if quit {
            break;
        }
        if dirty {
            let (pw, ph, rgb) = draw(&st).pixels(true);
            let r = encode_fit(pw, ph, &rgb, 95, max_bytes(st.k)).and_then(|(j, _)| session.send_frame(&j));
            if let Err(e) = r {
                eprintln!("Error: {e}");
                code = 1;
                break;
            }
            dirty = false;
        } else if let Err(e) = session.keepalive_if_due() {
            eprintln!("Lost the screen: {e}");
            code = 1;
            break;
        }
    }
    println!("Final values: {}", describe(&st));
    println!("(Press S next time to save them; nothing is saved automatically.)");
    session.stop();
    println!("Screen restored to the loop clip.");
    code
}

/// An offset in screen pixels (2400x1080 space), rounded, from the sent-pixel value kept in the state.
fn scr(st: &State, i: usize) -> i32 {
    (st.off[i] as f64 / st.k).round() as i32
}

fn max_sent(k: f64) -> i32 {
    (MAX_OFFSET as f64 * k).round() as i32
}

fn scale_arg(args: &[String]) -> f64 {
    arg_val(args, "--scale").and_then(|s| s.parse().ok()).filter(|k: &f64| *k > 0.2 && *k <= 1.0).unwrap_or(DEFAULT_SCALE)
}
