//! Compositor for the live screen: landscape 1680x756 (the 2400x1080 design at 0.7), drawn with tiny-skia.
//! Background (animated gradient, or a decoded clip), big clock, and a sensor panel.
//! The panel is limited by the screen: motion along the landscape vertical axis tears, so nothing here moves up or down;
//! the gradient drifts sideways and the bars grow sideways.
use super::mask::Mask;
use super::sensors::Sensors;
use super::video::VideoBg;
use super::{LH, LW};
use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, Transform};

struct Bmp {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    adv: f32,
    cov: Vec<u8>,
}

/// System fonts (Segoe UI, falling back to Arial/Tahoma), outlines read lazily (ab_glyph keeps just the file in memory;
/// fontdue parsed every glyph up front and cost ~40 MB for two Segoe UI files). Glyph bitmaps are cached per size.
struct Fonts {
    regular: Option<FontVec>,
    bold: Option<FontVec>,
    cache: HashMap<(char, u32, bool), Bmp>,
}

impl Fonts {
    fn load() -> Fonts {
        let dir = std::path::Path::new(r"C:\Windows\Fonts");
        let load = |names: &[&str]| -> Option<FontVec> {
            names.iter().find_map(|n| std::fs::read(dir.join(n)).ok()).and_then(|b| FontVec::try_from_vec(b).ok())
        };
        let regular = load(&["segoeui.ttf", "arial.ttf", "tahoma.ttf"]);
        let bold = load(&["segoeuib.ttf", "arialbd.ttf", "tahomabd.ttf"]).or_else(|| load(&["segoeui.ttf", "arial.ttf"]));
        Fonts { regular, bold, cache: HashMap::new() }
    }

    fn glyph(&mut self, c: char, px: u32, bold: bool) -> Option<&Bmp> {
        let key = (c, px, bold);
        if !self.cache.contains_key(&key) {
            let f = if bold { self.bold.as_ref() } else { self.regular.as_ref() }?;
            let scale = PxScale::from(px as f32);
            let id = f.glyph_id(c);
            let adv = f.as_scaled(scale).h_advance(id);
            let g = id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
            let bmp = match f.outline_glyph(g) {
                Some(og) => {
                    let b = og.px_bounds();
                    let (w, h) = ((b.max.x - b.min.x).ceil() as usize, (b.max.y - b.min.y).ceil() as usize);
                    let mut cov = vec![0u8; w * h];
                    og.draw(|x, y, v| {
                        let i = y as usize * w + x as usize;
                        if i < cov.len() {
                            cov[i] = (v * 255.0) as u8;
                        }
                    });
                    Bmp { x: b.min.x.floor() as i32, y: b.min.y.floor() as i32, w, h, adv, cov }
                }
                None => Bmp { x: 0, y: 0, w: 0, h: 0, adv, cov: Vec::new() },
            };
            self.cache.insert(key, bmp);
        }
        self.cache.get(&key)
    }

    fn width(&mut self, s: &str, px: u32, bold: bool) -> f32 {
        s.chars().map(|c| self.glyph(c, px, bold).map(|g| g.adv).unwrap_or(0.0)).sum()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub weekday: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

pub fn local_time() -> LocalTime {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut s: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut s) };
    LocalTime { year: s.wYear, month: s.wMonth, weekday: s.wDayOfWeek, day: s.wDay, hour: s.wHour, minute: s.wMinute, second: s.wSecond, millis: s.wMilliseconds }
}

pub struct Renderer {
    pm: Pixmap,
    fonts: Fonts,
    sensors: Arc<Mutex<Sensors>>,
    video: Option<VideoBg>,
    t0: Instant,
    /// Smoothed bar values (0..1): cpu, ram, gpu load, gpu temp, gpu clock.
    bars: [f32; 5],
    /// Animation ticks (10 per second): everything that moves on its own is stepped per tick, so a frame is only redrawn when
    /// the tick, the clock second or the background video frame changed.
    tick: u64,
    bars_tick: u64,
    last_key: Option<(u64, usize, bool, u16)>,
    mask: Mask,
    /// Accumulated microseconds per stage (background, clock, panel), for profiling.
    pub timing_us: [u64; 3],
    /// Panel background and bar tracks, drawn once (premultiplied RGBA, PANEL_W x PANEL_H); blended each frame.
    panel_ov: Vec<u8>,
}

const PANEL_X: usize = 1040;
const PANEL_Y: usize = 48;
const PANEL_W: usize = 592;
const PANEL_H: usize = 660;

fn build_panel_overlay() -> Vec<u8> {
    let mut pm = Pixmap::new(PANEL_W as u32, PANEL_H as u32).unwrap();
    round_rect(&mut pm, 0.0, 0.0, PANEL_W as f32, PANEL_H as f32, 28.0, [8, 10, 18, 120]);
    for i in 0..5 {
        round_rect(&mut pm, 36.0, 36.0 + i as f32 * 104.0 + 56.0, PANEL_W as f32 - 72.0, 12.0, 6.0, [255, 255, 255, 40]);
    }
    pm.data().to_vec()
}

/// Source-over of a premultiplied overlay onto opaque pixels.
fn blend_overlay(dst: &mut [u8], ov: &[u8], x0: usize, y0: usize, w: usize, h: usize) {
    for row in 0..h {
        let d = &mut dst[((y0 + row) * LW + x0) * 4..][..w * 4];
        let s = &ov[row * w * 4..][..w * 4];
        for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
            let inv = 255 - sp[3] as u16;
            dp[0] = sp[0].saturating_add(((dp[0] as u16 * inv) / 255) as u8);
            dp[1] = sp[1].saturating_add(((dp[1] as u16 * inv) / 255) as u8);
            dp[2] = sp[2].saturating_add(((dp[2] as u16 * inv) / 255) as u8);
        }
    }
}

impl Renderer {
    pub fn new(sensors: Arc<Mutex<Sensors>>, video: Option<VideoBg>) -> Renderer {
        Renderer { pm: Pixmap::new(LW as u32, LH as u32).unwrap(), fonts: Fonts::load(), sensors, video, t0: Instant::now(), bars: [0.0; 5], tick: 0, bars_tick: 0, last_key: None, mask: Mask::none(), timing_us: [0; 3], panel_ov: build_panel_overlay() }
    }

    /// Set the notch mask (applied as the last step; the clock keeps clear of a left band).
    pub fn set_mask(&mut self, m: Mask) {
        if m != self.mask {
            self.mask = m;
            self.last_key = None;
        }
    }

    /// Force the next render to redraw (for benchmarks).
    pub fn invalidate(&mut self) {
        self.last_key = None;
    }

    pub fn has_fonts(&self) -> bool {
        self.fonts.regular.is_some()
    }

    pub fn video(&self) -> Option<&VideoBg> {
        self.video.as_ref()
    }

    /// Draw one frame; returns premultiplied RGBA (everything is opaque, so it equals plain RGBA), LW x LH.
    /// Returns the frame and whether it differs from the previous call (when not, nothing was redrawn).
    pub fn render(&mut self, show_seconds: bool) -> (&[u8], bool) {
        let t = self.t0.elapsed().as_secs_f32();
        self.tick = (t * 10.0) as u64;
        let now = local_time();
        let vframe = self.video.as_ref().and_then(|v| v.frame());
        let vkey = vframe.as_ref().map(|a| Arc::as_ptr(a) as usize).unwrap_or(0);
        // With a video the widgets follow the video frames (no separate 10 Hz redraw on top of them).
        let key = (if vkey != 0 { 0 } else { self.tick }, vkey, show_seconds, now.second);
        if self.last_key == Some(key) {
            return (self.pm.data(), false);
        }
        self.last_key = Some(key);
        let s = self.sensors.lock().unwrap().clone();
        let t0 = Instant::now();
        self.background(t, vframe);
        let t1 = Instant::now();
        self.clock(&now, show_seconds);
        let t2 = Instant::now();
        self.panel(&s);
        let t3 = Instant::now();
        self.timing_us[0] += (t1 - t0).as_micros() as u64;
        self.timing_us[1] += (t2 - t1).as_micros() as u64;
        self.timing_us[2] += (t3 - t2).as_micros() as u64;
        if !self.mask.is_none() {
            self.mask.apply(self.pm.data_mut());
        }
        (self.pm.data(), true)
    }

    fn background(&mut self, t: f32, frame: Option<Arc<Vec<u8>>>) {
        if let Some(f) = frame {
            self.pm.data_mut().copy_from_slice(&f);
            for p in self.pm.data_mut().chunks_exact_mut(4) {
                // dim to 65 % for legibility
                p[0] = ((p[0] as u16 * 166) >> 8) as u8;
                p[1] = ((p[1] as u16 * 166) >> 8) as u8;
                p[2] = ((p[2] as u16 * 166) >> 8) as u8;
                p[3] = 255;
            }
            return;
        }
        // Slow colour drift and a gradient sliding sideways (repeats seamlessly: first and last stop are equal).
        // Quantised to 10 steps a second: smooth to the eye, and an unchanged frame is not re-encoded (see pipeline).
        let t = (t * 10.0).floor() / 10.0;
        let hue = (t * 4.0) % 360.0;
        let a = hsv(hue, 0.55, 0.33);
        let b = hsv(hue + 70.0, 0.60, 0.20);
        let shift = (t * 36.0) % (LW as f32);
        // The gradient only varies along x: compute one row and copy it down (a full tiny-skia gradient fill was ~25 ms).
        let (ca, cb) = ([a.red(), a.green(), a.blue()], [b.red(), b.green(), b.blue()]);
        let data = self.pm.data_mut();
        for x in 0..LW {
            let u = ((x as f32 + shift) / LW as f32).fract();
            let f = 1.0 - (2.0 * u - 1.0).abs();
            for k in 0..3 {
                data[x * 4 + k] = ((ca[k] + (cb[k] - ca[k]) * f) * 255.0) as u8;
            }
            data[x * 4 + 3] = 255;
        }
        for y in 1..LH {
            data.copy_within(0..LW * 4, y * LW * 4);
        }
    }

    fn clock(&mut self, n: &LocalTime, show_seconds: bool) {
        let hhmm = format!("{:02}:{:02}", n.hour, n.minute);
        let size = 300;
        let w = self.fonts.width(&hhmm, size, true);
        let cx = (if show_seconds { 470.0 } else { 520.0 }) + self.mask.left_inset() / 2.0;
        let x = cx - w / 2.0;
        // The colon fades in and out instead of blinking hard.
        let frac = (self.tick % 10) as f32 / 10.0;
        let colon_alpha = 0.35 + 0.65 * (0.5 + 0.5 * (frac * std::f32::consts::TAU).cos());
        self.text_with_colon(&hhmm, x, 430.0, size, colon_alpha);
        let days = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
        let months = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
        let date = format!("{}, {} {}", days[(n.weekday % 7) as usize], n.day, months[((n.month.max(1) - 1) % 12) as usize]);
        let dw = self.fonts.width(&date, 52, false);
        self.text(&date, 520.0 + self.mask.left_inset() / 2.0 - dw / 2.0, 520.0, 52, false, [215, 220, 232], 1.0);
        if show_seconds {
            let sec = format!("{:02}", n.second);
            self.text(&sec, x + w + 10.0, 430.0, 96, true, [200, 210, 235], 0.9);
        }
    }

    fn text_with_colon(&mut self, s: &str, x: f32, base: f32, size: u32, colon_alpha: f32) {
        let mut pen = x;
        for c in s.chars() {
            let adv = self.fonts.glyph(c, size, true).map(|g| g.adv).unwrap_or(0.0);
            let a = if c == ':' { colon_alpha } else { 1.0 };
            self.text(&c.to_string(), pen, base, size, true, [255, 255, 255], a);
            pen += adv;
        }
    }

    fn panel(&mut self, s: &Sensors) {
        let (px, py, pw) = (PANEL_X as f32, PANEL_Y as f32, PANEL_W as f32);
        blend_overlay(self.pm.data_mut(), &self.panel_ov, PANEL_X, PANEL_Y, PANEL_W, PANEL_H);
        let gpu = s.gpu.as_ref();
        let target = [
            s.cpu_load_pct / 100.0,
            if s.ram_total_gb > 0.0 { s.ram_used_gb / s.ram_total_gb } else { 0.0 },
            gpu.map(|g| g.load_pct as f32 / 100.0).unwrap_or(0.0),
            gpu.map(|g| g.temp_c as f32 / 100.0).unwrap_or(0.0),
            gpu.map(|g| g.clock_mhz as f32 / 3200.0).unwrap_or(0.0),
        ];
        let steps = (self.tick - self.bars_tick).min(20) as i32;
        if steps > 0 {
            self.bars_tick = self.tick;
            let k = 1.0 - 0.75f32.powi(steps);
            for (b, t) in self.bars.iter_mut().zip(target) {
                *b += (t.clamp(0.0, 1.0) - *b) * k;
            }
        }
        let rows: [(&str, String, Option<usize>); 6] = [
            ("CPU load", format!("{:.0} %", s.cpu_load_pct), Some(0)),
            ("RAM", format!("{:.1} / {:.1} GB", s.ram_used_gb, s.ram_total_gb), Some(1)),
            ("GPU load", gpu.map(|g| format!("{} %", g.load_pct)).unwrap_or("\u{2014}".into()), Some(2)),
            ("GPU temp", gpu.map(|g| format!("{} \u{00B0}C", g.temp_c)).unwrap_or("\u{2014}".into()), Some(3)),
            ("GPU clock", gpu.map(|g| format!("{} MHz", g.clock_mhz)).unwrap_or("\u{2014}".into()), Some(4)),
            ("CPU temp", s.cpu_temp_c.map(|c| format!("{c:.0} \u{00B0}C")).unwrap_or("\u{2014}".into()), None),
        ];
        for (i, (label, value, bar)) in rows.iter().enumerate() {
            let y0 = py + 36.0 + i as f32 * 104.0;
            self.text(label, px + 36.0, y0 + 34.0, 30, false, [185, 192, 208], 1.0);
            let vw = self.fonts.width(value, 40, true);
            self.text(value, px + pw - 36.0 - vw, y0 + 38.0, 40, true, [255, 255, 255], 1.0);
            if let Some(k) = bar {
                let v = self.bars[*k];
                let col = if v < 0.6 { [90, 210, 130, 255] } else if v < 0.85 { [240, 200, 70, 255] } else { [240, 90, 80, 255] };
                if v > 0.01 {
                    round_rect(&mut self.pm, px + 36.0, y0 + 56.0, ((pw - 72.0) * v).max(12.0), 12.0, 6.0, col);
                }
            }
        }
    }

    /// Text with a soft drop shadow (sideways offset only), alpha-blended onto the opaque canvas.
    fn text(&mut self, s: &str, x: f32, baseline: f32, px: u32, bold: bool, rgb: [u8; 3], alpha: f32) {
        let mut pen = x;
        for c in s.chars() {
            let Renderer { pm, fonts, .. } = self;
            let Some(g) = fonts.glyph(c, px, bold) else { continue };
            let gx = pen.round() as i32 + g.x;
            let gy = baseline.round() as i32 + g.y;
            blend(pm.data_mut(), &g.cov, g.w, g.h, gx + 2, gy, [0, 0, 0], 0.45 * alpha);
            blend(pm.data_mut(), &g.cov, g.w, g.h, gx, gy, rgb, alpha);
            pen += g.adv;
        }
    }
}

/// Blend a coverage bitmap onto RGBA pixels (opaque destination).
fn blend(dst: &mut [u8], cov: &[u8], w: usize, h: usize, x0: i32, y0: i32, rgb: [u8; 3], alpha: f32) {
    for row in 0..h {
        let y = y0 + row as i32;
        if y < 0 || y >= LH as i32 {
            continue;
        }
        for col in 0..w {
            let x = x0 + col as i32;
            if x < 0 || x >= LW as i32 {
                continue;
            }
            let a = cov[row * w + col] as f32 / 255.0 * alpha;
            if a <= 0.0 {
                continue;
            }
            let i = (y as usize * LW + x as usize) * 4;
            for k in 0..3 {
                let d = dst[i + k] as f32;
                dst[i + k] = (d + (rgb[k] as f32 - d) * a) as u8;
            }
        }
    }
}

fn round_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, rgba: [u8; 4]) {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + w - r, y);
    pb.quad_to(x + w, y, x + w, y + r);
    pb.line_to(x + w, y + h - r);
    pb.quad_to(x + w, y + h, x + w - r, y + h);
    pb.line_to(x + r, y + h);
    pb.quad_to(x, y + h, x, y + h - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    let Some(path) = pb.finish() else { return };
    let mut p = Paint::default();
    p.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
    pm.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(360.0) / 60.0;
    let (c, i) = (v * s, h.floor() as i32);
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match i {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color::from_rgba(r + m, g + m, b + m, 1.0).unwrap_or(Color::BLACK)
}
