//! Screen fit calibration: the ruler picture the Screen fit page puts on the real screen, and the small session that keeps it
//! there. The picture is the known-good live size (landscape 1680x756, one sent pixel = 1.43 screen pixels), black with a thin
//! line on each edge of the visible rectangle and rulers going inward; moving an edge inward until its line is just visible
//! finds how many pixels the panel hides. The device shows a live picture only while pictures keep coming, so the session
//! re-sends the current one about five times a second. Ending it uses the normal live ending (the stored loop clip plays).
use super::encode::{frame_budget, rotate_cw, JpegEncoder};
use super::pipeline::{join_bounded, Sink};
use super::session::{Ending, LiveSession};
use super::{LH, LW, PH, PW};
use crate::core::fit::Fit;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Sent pixels per screen pixel.
const K: f64 = 0.7;
const NAMES: [&str; 4] = ["TOP", "RIGHT", "BOTTOM", "LEFT"];
/// How often the picture is sent again.
const RESEND: Duration = Duration::from_millis(200);

type Rgb = [u8; 3];
const WHITE: Rgb = [255, 255, 255];
const RED: Rgb = [255, 40, 40];
const YELLOW: Rgb = [255, 230, 0];
const CYAN: Rgb = [0, 230, 255];

/// 5x7 glyphs, one row per byte (bit 4 = leftmost column).
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [31, 2, 4, 2, 1, 17, 14],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 30, 1, 1, 17, 14],
        '6' => [6, 8, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 2, 12],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        _ => [0; 7],
    }
}

/// The picture being drawn: RGBA, opaque.
struct Pic {
    d: Vec<u8>,
}

impl Pic {
    fn new() -> Pic {
        let mut d = vec![0u8; LW * LH * 4];
        d.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        Pic { d }
    }
    fn px(&mut self, x: i64, y: i64, c: Rgb) {
        if x >= 0 && y >= 0 && (x as usize) < LW && (y as usize) < LH {
            let i = (y as usize * LW + x as usize) * 4;
            self.d[i..i + 3].copy_from_slice(&c);
        }
    }
    fn fill(&mut self, x0: i64, y0: i64, x1: i64, y1: i64, c: Rgb) {
        for y in y0.max(0)..y1.min(LH as i64) {
            for x in x0.max(0)..x1.min(LW as i64) {
                self.px(x, y, c);
            }
        }
    }
    /// Text with its top-left at (x, y), `s` px per glyph dot (5x7 dots, 6 dots advance).
    fn text(&mut self, x: i64, y: i64, s: usize, t: &str, c: Rgb) {
        for (k, ch) in t.chars().enumerate() {
            for (ry, row) in glyph(ch).iter().enumerate() {
                for rx in 0..5 {
                    if row & (16 >> rx) != 0 {
                        let (gx, gy) = (x + ((k * 6 + rx) * s) as i64, y + (ry * s) as i64);
                        self.fill(gx, gy, gx + s as i64, gy + s as i64, c);
                    }
                }
            }
        }
    }
}

fn text_w(s: usize, t: &str) -> i64 {
    (t.chars().count() * 6 * s) as i64
}

/// The content rectangle [x0, x1) x [y0, y1) in sent pixels.
struct Rect {
    x0: i64,
    y0: i64,
    x1: i64,
    y1: i64,
}

impl Rect {
    /// Picture position of "`d` pixels inward from edge `e`, `along` pixels along that edge".
    fn at(&self, e: usize, along: i64, d: i64) -> (i64, i64) {
        match e {
            0 => (along, self.y0 + d),
            2 => (along, self.y1 - 1 - d),
            3 => (self.x0 + d, along),
            _ => (self.x1 - 1 - d, along),
        }
    }
    fn span(&self, e: usize) -> (i64, i64) {
        if e == 0 || e == 2 { (self.x0, self.x1) } else { (self.y0, self.y1) }
    }
}

/// The ruler picture for `fit` (screen px) with edge `sel` (0 top, 1 right, 2 bottom, 3 left) marked: landscape 1680x756 RGBA.
pub fn draw(fit: Fit, sel: usize) -> Vec<u8> {
    let sel = sel.min(3);
    let mut c = Pic::new();
    let (x0, y0, x1, y1) = fit.rect(LW, LH);
    let r = Rect { x0: x0 as i64, y0: y0 as i64, x1: x1 as i64, y1: y1 as i64 };
    // rulers: three per edge; a tick every 2 sent px (alternating colours) for 60 px, a long white tick with a number every 10
    for e in 0..4 {
        let (a0, a1) = r.span(e);
        for frac in if e == 0 || e == 2 { [15i64, 50, 85] } else { [6, 42, 76] } {
            let along = a0 + (a1 - a0) * frac / 100;
            for d in (0..=60i64).step_by(2) {
                let long = d % 10 == 0;
                let lane = 6 - d / 10; // each label hangs at the end of its own tick; later ticks are shorter and never cross it
                let step = if e == 0 || e == 2 { 30 } else { 18 };
                let len = if long { 24 + lane * step } else { 8 };
                let col = if long { WHITE } else if (d / 2) % 2 == 0 { YELLOW } else { CYAN };
                for b in 0..len {
                    let (x, y) = r.at(e, along + b, d);
                    c.px(x, y, col);
                }
                if long {
                    let label = ((d as f64) / K).round().to_string();
                    let lw = text_w(2, &label) - 2;
                    let (tx, ty) = r.at(e, along, d);
                    let (lx, ly) = match e {
                        0 => (tx + 28 + lane * 30, ty),
                        2 => (tx + 28 + lane * 30, ty - 13),
                        3 => (tx, ty + 28 + lane * 18),
                        _ => (tx + 1 - lw, ty + 28 + lane * 18),
                    };
                    c.text(lx, ly, 2, &label, WHITE);
                }
            }
        }
    }
    // edge lines exactly on the content edge (1 px); the selected one in red, drawn last
    for e in (0..4).filter(|&e| e != sel).chain(std::iter::once(sel)) {
        let p = if e == sel { RED } else { WHITE };
        match e {
            0 => c.fill(r.x0, r.y0, r.x1, r.y0 + 1, p),
            2 => c.fill(r.x0, r.y1 - 1, r.x1, r.y1, p),
            3 => c.fill(r.x0, r.y0, r.x0 + 1, r.y1, p),
            _ => c.fill(r.x1 - 1, r.y0, r.x1, r.y1, p),
        }
    }
    // arrows pointing at the selected edge (30 % and 70 % along it)
    let (a0, a1) = r.span(sel);
    for frac in [30i64, 70] {
        let along = a0 + (a1 - a0) * frac / 100;
        for d in 30..170i64 {
            let half = if d < 110 { (d - 30) / 2 } else { 12 };
            for b in -half..=half {
                let (x, y) = r.at(sel, along + b, d);
                c.px(x, y, RED);
            }
        }
    }
    // big readout in the middle, the selected value on a yellow box (screen px)
    let vals = [fit.top, fit.right, fit.bottom, fit.left];
    let (s, gap) = (4usize, 3 * 6 * 4);
    let parts: Vec<String> = (0..4).map(|i| format!("{} {}", NAMES[i], vals[i])).collect();
    let total: i64 = parts.iter().map(|p| text_w(s, p)).sum::<i64>() + gap * 3;
    let (mut x, y) = ((LW as i64 - total) / 2, LH as i64 / 2 - 14);
    for (i, p) in parts.iter().enumerate() {
        let w = text_w(s, p);
        if i == sel {
            c.fill(x - 8, y - 8, x + w + 4, y + 36, YELLOW);
            c.text(x, y, s, p, [0, 0, 0]);
        } else {
            c.text(x, y, s, p, WHITE);
        }
        x += w + gap;
    }
    c.d
}

struct CalShared {
    quit: AtomicBool,
    /// Bumped whenever the picture changes.
    gen: AtomicU64,
    state: Mutex<(Fit, usize)>,
    connected: AtomicBool,
}

/// A running calibration session. Dropping it without `end` leaves the thread to finish on its own (it is told to quit).
pub struct FitCalibration {
    shared: Arc<CalShared>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
}

fn log(s: &str) {
    crate::device::log(&format!("fit-cal: {s}"));
}

impl FitCalibration {
    /// Open the screen (retrying while it is busy) and show the ruler picture for `fit`. With `Sink::Null` nothing is sent.
    pub fn start(fit: Fit, sel: usize, sink: Sink) -> FitCalibration {
        let shared = Arc::new(CalShared { quit: AtomicBool::new(false), gen: AtomicU64::new(1), state: Mutex::new((fit, sel.min(3))), connected: AtomicBool::new(sink == Sink::Null) });
        let sh = shared.clone();
        let t = std::thread::spawn(move || {
            let mut session: Option<LiveSession> = None;
            let mut enc = JpegEncoder::default();
            let (mut portrait, mut jpeg) = (Vec::new(), Vec::new());
            let (mut seen, mut last_sent, mut next_try) = (0u64, None::<Instant>, Instant::now());
            let mut backoff = 1u64;
            while !sh.quit.load(Relaxed) {
                if sink == Sink::Device && session.is_none() && Instant::now() >= next_try {
                    match LiveSession::connect() {
                        Ok(s) => {
                            log("connected");
                            session = Some(s);
                            sh.connected.store(true, Relaxed);
                            backoff = 1;
                            last_sent = None;
                        }
                        Err(e) => {
                            log(&format!("cannot connect ({e}); retry in {backoff} s"));
                            next_try = Instant::now() + Duration::from_secs(backoff);
                            backoff = (backoff * 2).min(8);
                        }
                    }
                }
                let g = sh.gen.load(Relaxed);
                if g != seen {
                    seen = g;
                    let (fit, sel) = *sh.state.lock().unwrap();
                    let rgba = draw(fit, sel);
                    rotate_cw(&rgba, &mut portrait);
                    if let Err(e) = enc.encode(&portrait, PW, PH, frame_budget(200, 5), &mut jpeg) {
                        log(&e);
                        jpeg.clear();
                    }
                    last_sent = None;
                }
                if let Some(s) = session.as_mut() {
                    if !jpeg.is_empty() && last_sent.map_or(true, |t| t.elapsed() >= RESEND) {
                        if let Err(e) = s.send_frame(&jpeg) {
                            log(&format!("send failed ({e})"));
                            if let Some(s) = session.take() {
                                s.abandon();
                            }
                            sh.connected.store(false, Relaxed);
                            next_try = Instant::now() + Duration::from_secs(1);
                            continue;
                        }
                        last_sent = Some(Instant::now());
                    }
                    if let Err(e) = s.keepalive_if_due() {
                        log(&format!("lost the screen ({e})"));
                        if let Some(s) = session.take() {
                            s.abandon();
                        }
                        sh.connected.store(false, Relaxed);
                        next_try = Instant::now() + Duration::from_secs(1);
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            if let Some(s) = session.take() {
                s.stop_with(Ending::LoopClip); // the stored loop clip plays again
            }
            sh.connected.store(false, Relaxed);
        });
        FitCalibration { shared, thread: Mutex::new(Some(t)) }
    }

    /// New offsets (screen px) and selected edge; the picture changes at once.
    pub fn set(&self, fit: Fit, sel: usize) {
        *self.shared.state.lock().unwrap() = (fit, sel.min(3));
        self.shared.gen.fetch_add(1, Relaxed);
    }

    /// The ruler picture is on the screen (a connection is open); always true for the null sink.
    pub fn connected(&self) -> bool {
        self.shared.connected.load(Relaxed)
    }

    /// End the session: the screen plays its stored loop clip again. Waits a bounded time for the screen link to close.
    pub fn end(&self) {
        self.shared.quit.store(true, Relaxed);
        let t = self.thread.lock().unwrap().take();
        if let Some(t) = t {
            join_bounded(vec![t], Duration::from_millis(2500));
        }
    }
}

impl Drop for FitCalibration {
    fn drop(&mut self) {
        self.shared.quit.store(true, Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(d: &[u8], x: usize, y: usize) -> [u8; 3] {
        let i = (y * LW + x) * 4;
        [d[i], d[i + 1], d[i + 2]]
    }

    #[test]
    fn the_edge_lines_sit_on_the_visible_rectangle() {
        let f = Fit::new(4, 0, 0, 9); // sent: top 3, left 6
        let d = draw(f, 0);
        assert_eq!(d.len(), LW * LH * 4);
        assert_eq!(at(&d, 800, 3), RED, "selected top edge on row 3");
        assert_eq!(at(&d, 6, 400), WHITE, "left edge on column 6");
        assert_eq!(at(&d, 800, 2), [0, 0, 0], "above the top edge: black");
        assert_eq!(at(&d, 3, 400), [0, 0, 0], "left of the left edge: black");
        assert_eq!(at(&d, LW - 1, 300), WHITE, "right edge on the last column");
        assert_eq!(at(&d, 900, LH - 1), WHITE, "bottom edge on the last row");
        // no offsets: the lines are on the picture's own edges
        let z = draw(Fit::default(), 3);
        assert_eq!(at(&z, 0, 400), RED);
        assert_eq!(at(&z, 800, 0), WHITE);
    }

    #[test]
    fn a_null_session_starts_follows_changes_and_ends_quickly() {
        let c = FitCalibration::start(Fit::default(), 0, Sink::Null);
        assert!(c.connected());
        c.set(Fit::new(4, 0, 0, 9), 3);
        std::thread::sleep(Duration::from_millis(100));
        let t = Instant::now();
        c.end();
        assert!(t.elapsed() < Duration::from_millis(1000));
        assert!(!c.connected());
    }
}
