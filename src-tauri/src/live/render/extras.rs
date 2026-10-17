//! The v1.6 widgets besides `sensor`: clock styles (analog, minimal), `date`, `weather`, `visualizer`, `slideshow`, `info`.
//! Same look as the existing widgets: dark rounded card, Segoe UI, a missing value is a greyed dash. Nothing moves up and
//! down: hands rotate, bars grow sideways, the spectrogram scrolls sideways, fades only change opacity.
use super::sensors::{circle, fill, fill_even_odd, line, mix, stroke, GREY};
use super::widgets::{decode_image, resolve_image, scale_to_box};
use super::{round_rect, LocalTime, Rect, Renderer, Weight, DAYS, MONTHS};
use crate::live::audio;
use crate::live::metrics;
use crate::live::theme::{parse_color, ClockProps, DateProps, InfoProps, SlideshowProps, VisualizerProps, WeatherProps, Widget};
use crate::live::weather::{self, Icon};
use std::path::PathBuf;
use std::sync::Arc;
use tiny_skia::{Pixmap, PixmapPaint, PathBuilder, Transform};

const LAB: [u8; 3] = [185, 192, 208];

/// Slideshow state per widget: the loaded pictures (all frames of GIFs) and a scaled-copy cache.
pub(super) struct Slides {
    files: Vec<String>,
    dir: PathBuf,
    items: Vec<Option<Item>>,
    scaled: Vec<((usize, usize, u32, u32, bool), Pixmap, i32, i32)>,
}

struct Item {
    frames: Vec<(Arc<Pixmap>, u32)>,
    total_ms: u32,
}

fn civil_days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

impl Renderer {
    fn card(&mut self, b: Rect) {
        let sc = (self.cw as f32 / 1680.0).max(0.1);
        round_rect(&mut self.layer, b.x, b.y, b.w, b.h, (28.0 * sc).min(b.w.min(b.h) / 5.0), [0, 0, 0, 96]);
    }

    /// Centre `s` horizontally in `b`, baseline `base`.
    fn text_c(&mut self, s: &str, b: Rect, base: f32, px: u32, wt: Weight, col: [u8; 3], a: f32) {
        let w = self.fonts.width(s, px, wt);
        self.text(s, b.x + (b.w - w) / 2.0, base, px, wt, col, a);
    }

    // ------------------------------------------------------------ clock styles

    pub(super) fn draw_clock_style(&mut self, b: Rect, p: &ClockProps, n: &LocalTime) {
        let col = parse_color(&p.color, [255, 255, 255]);
        match p.style.as_str() {
            "analog" => self.clock_analog(b, p, n, col),
            _ => self.clock_minimal(b, p, n, col),
        }
    }

    fn time_string(p: &ClockProps, n: &LocalTime) -> String {
        if p.format == "12h" {
            let h = match n.hour % 12 {
                0 => 12,
                h => h,
            };
            format!("{}:{:02}", h, n.minute)
        } else {
            format!("{:02}:{:02}", n.hour, n.minute)
        }
    }

    fn clock_minimal(&mut self, b: Rect, p: &ClockProps, n: &LocalTime, col: [u8; 3]) {
        let t = Self::time_string(p, n);
        let size = self.fit_px(&t, Weight::Bold, b.w * 0.96, b.h * 0.9 / 0.72 * 0.8);
        let w = self.fonts.width(&t, size, Weight::Bold);
        let base = b.y + (b.h + size as f32 * 0.72) / 2.0;
        let frac = (self.tick % 10) as f32 / 10.0;
        let colon_alpha = 0.35 + 0.65 * (0.5 + 0.5 * (frac * std::f32::consts::TAU).cos());
        let mut pen = b.x + (b.w - w) / 2.0;
        for c in t.chars() {
            let adv = self.fonts.glyph(c, size, Weight::Bold).map(|g| g.adv).unwrap_or(0.0);
            self.text(&c.to_string(), pen, base, size, Weight::Bold, col, if c == ':' { colon_alpha } else { 1.0 });
            pen += adv;
        }
    }

    fn clock_analog(&mut self, b: Rect, p: &ClockProps, n: &LocalTime, col: [u8; 3]) {
        self.card(b);
        let date_h = if p.date { b.h * 0.15 } else { 0.0 };
        let d = (b.w.min(b.h - date_h) * 0.9).max(10.0);
        let (cx, cy) = (b.x + b.w / 2.0, b.y + (b.h - date_h) / 2.0);
        let r = d / 2.0;
        circle(&mut self.layer, cx, cy, r, [255, 255, 255, 22]);
        if let Some(pp) = PathBuilder::from_circle(cx, cy, r) {
            stroke(&mut self.layer, &pp, [col[0], col[1], col[2], 110], (r * 0.025).max(1.5), false);
        }
        for i in 0..60 {
            let a = (i as f32 * 6.0 - 90.0).to_radians();
            let major = i % 5 == 0;
            let (r0, r1) = (r * if major { 0.84 } else { 0.9 }, r * 0.95);
            line(&mut self.layer, cx + r0 * a.cos(), cy + r0 * a.sin(), cx + r1 * a.cos(), cy + r1 * a.sin(), [col[0], col[1], col[2], if major { 230 } else { 90 }], if major { r * 0.035 } else { r * 0.015 }, false);
        }
        let sec = n.second as f32 + n.millis as f32 / 1000.0;
        let min = n.minute as f32 + sec / 60.0;
        let hour = (n.hour % 12) as f32 + min / 60.0;
        let hand = |s: &mut Renderer, deg: f32, len: f32, w: f32, rgba: [u8; 4]| {
            let a = (deg - 90.0).to_radians();
            let tail = r * 0.12;
            line(&mut s.layer, cx - tail * a.cos(), cy - tail * a.sin(), cx + r * len * a.cos(), cy + r * len * a.sin(), rgba, w, true);
        };
        hand(self, hour * 30.0, 0.5, r * 0.075, [col[0], col[1], col[2], 255]);
        hand(self, min * 6.0, 0.78, r * 0.055, [col[0], col[1], col[2], 255]);
        if p.seconds {
            hand(self, sec * 6.0, 0.88, r * 0.022, [255, 95, 85, 255]);
        }
        circle(&mut self.layer, cx, cy, r * 0.06, [255, 95, 85, 255]);
        if p.date {
            let date = format!("{}, {} {}", DAYS[(n.weekday % 7) as usize], n.day, MONTHS[((n.month.max(1) - 1) % 12) as usize]);
            let sz = self.fit_px(&date, Weight::Regular, b.w * 0.9, date_h * 0.8);
            self.text_c(&date, b, b.y + b.h - date_h * 0.22, sz, Weight::Regular, col, 0.88);
        }
    }

    // ------------------------------------------------------------ date

    pub(super) fn draw_date(&mut self, b: Rect, p: &DateProps, n: &LocalTime) {
        let col = parse_color(&p.color, [255, 255, 255]);
        self.card(b);
        let month = MONTHS[((n.month.max(1) - 1) % 12) as usize];
        match p.style.as_str() {
            "calendar" => {
                let pad = b.w.min(b.h) * 0.06;
                let head_h = b.h * 0.13;
                let title = format!("{} {}", month, n.year);
                let tsz = self.fit_px(&title, Weight::Bold, b.w - 2.0 * pad, head_h * 0.85);
                self.text_c(&title, b, b.y + pad + head_h * 0.78, tsz, Weight::Bold, col, 1.0);
                let grid_y = b.y + pad + head_h * 1.15;
                let cell_w = (b.w - 2.0 * pad) / 7.0;
                let cell_h = ((b.y + b.h - pad - grid_y) / 7.0).max(4.0);
                let csz = (cell_h.min(cell_w) * 0.52).round().max(7.0) as u32;
                for (i, d) in ["M", "T", "W", "T", "F", "S", "S"].iter().enumerate() {
                    let cx = b.x + pad + cell_w * (i as f32 + 0.5);
                    let w = self.fonts.width(d, csz, Weight::Bold);
                    self.text(d, cx - w / 2.0, grid_y + cell_h * 0.7, csz, Weight::Bold, mix(col, LAB, 0.5), 0.9);
                }
                let first = weather::weekday_of(n.year as i32, n.month as u32, 1) as usize;
                let days = civil_days_in_month(n.year as i32, n.month as u32) as usize;
                for day in 1..=days {
                    let idx = first + day - 1;
                    let (row, colm) = (idx / 7 + 1, idx % 7);
                    let cx = b.x + pad + cell_w * (colm as f32 + 0.5);
                    let cy = grid_y + cell_h * (row as f32 + 0.5);
                    let today = day == n.day as usize;
                    if today {
                        circle(&mut self.layer, cx, cy, cell_h.min(cell_w) * 0.46, [90, 200, 255, 235]);
                    }
                    let s = day.to_string();
                    let w = self.fonts.width(&s, csz, if today { Weight::Bold } else { Weight::Regular });
                    self.text(&s, cx - w / 2.0, cy + csz as f32 * 0.36, csz, if today { Weight::Bold } else { Weight::Regular }, if today { [10, 20, 35] } else { col }, if colm >= 5 && !today { 0.75 } else { 1.0 });
                }
            }
            "big" => {
                let pad = b.w.min(b.h) * 0.08;
                let inner = Rect { x: b.x + pad, y: b.y + pad, w: b.w - 2.0 * pad, h: b.h - 2.0 * pad };
                let day = n.day.to_string();
                let dsz = self.fit_px(&day, Weight::Bold, inner.w * 0.5, inner.h / 0.72 * 0.95);
                let dw = self.fonts.width(&day, dsz, Weight::Bold);
                let base = inner.y + (inner.h + dsz as f32 * 0.72) / 2.0;
                self.text(&day, inner.x, base, dsz, Weight::Bold, col, 1.0);
                let rx = inner.x + dw + inner.w * 0.06;
                let rw = (inner.x + inner.w - rx).max(10.0);
                let wd = DAYS[(n.weekday % 7) as usize];
                let wsz = self.fit_px(wd, Weight::Bold, rw, inner.h * 0.3);
                let msz = self.fit_px(&format!("{month} {}", n.year), Weight::Regular, rw, inner.h * 0.22);
                let mid = inner.y + inner.h / 2.0;
                self.text(wd, rx, mid - 2.0, wsz, Weight::Bold, col, 1.0);
                self.text(&format!("{month} {}", n.year), rx, mid + msz as f32 * 1.1, msz, Weight::Regular, LAB, 0.95);
            }
            _ => {
                let date = format!("{}, {} {}", DAYS[(n.weekday % 7) as usize], n.day, month);
                let sz = self.fit_px(&date, Weight::Regular, b.w * 0.92, b.h * 0.5);
                let base = b.y + (b.h + sz as f32 * 0.72) / 2.0;
                self.text_c(&date, b, base, sz, Weight::Regular, col, 0.95);
            }
        }
    }

    // ------------------------------------------------------------ weather

    pub(super) fn draw_icon(&mut self, kind: Icon, cx: f32, cy: f32, s: f32, a: f32) {
        let al = |v: u8| (v as f32 * a) as u8;
        let sun = [255, 205, 70, al(255)];
        let cloud = [225, 230, 242, al(255)];
        let dark = [150, 158, 178, al(255)];
        let sun_at = |sl: &mut Renderer, x: f32, y: f32, r: f32| {
            circle(&mut sl.layer, x, y, r, sun);
            for i in 0..8 {
                let ang = (i as f32 * 45.0).to_radians();
                line(&mut sl.layer, x + r * 1.35 * ang.cos(), y + r * 1.35 * ang.sin(), x + r * 1.75 * ang.cos(), y + r * 1.75 * ang.sin(), sun, r * 0.2, true);
            }
        };
        let cloud_at = |sl: &mut Renderer, x: f32, y: f32, sz: f32, c: [u8; 4]| {
            for (dx, dy, r) in [(-0.22, 0.06, 0.2), (0.0, -0.08, 0.27), (0.24, 0.04, 0.2)] {
                circle(&mut sl.layer, x + dx * sz, y + dy * sz, r * sz, c);
            }
            round_rect(&mut sl.layer, x - 0.42 * sz, y + 0.02 * sz, 0.84 * sz, 0.24 * sz, 0.12 * sz, c);
        };
        match kind {
            Icon::Sun => sun_at(self, cx, cy, s * 0.24),
            Icon::Moon => {
                let r = s * 0.3;
                let mut pb = PathBuilder::new();
                pb.push_circle(cx, cy, r);
                pb.push_circle(cx + r * 0.55, cy - r * 0.25, r * 0.85);
                if let Some(p) = pb.finish() {
                    fill_even_odd(&mut self.layer, &p, [235, 235, 200, al(255)]);
                }
            }
            Icon::PartlyDay => {
                sun_at(self, cx - s * 0.16, cy - s * 0.16, s * 0.17);
                cloud_at(self, cx + s * 0.06, cy + s * 0.1, s * 0.8, cloud);
            }
            Icon::PartlyNight => {
                let r = s * 0.2;
                let mut pb = PathBuilder::new();
                pb.push_circle(cx - s * 0.16, cy - s * 0.14, r);
                pb.push_circle(cx - s * 0.16 + r * 0.55, cy - s * 0.14 - r * 0.25, r * 0.85);
                if let Some(p) = pb.finish() {
                    fill_even_odd(&mut self.layer, &p, [235, 235, 200, al(255)]);
                }
                cloud_at(self, cx + s * 0.06, cy + s * 0.1, s * 0.8, cloud);
            }
            Icon::Cloud => cloud_at(self, cx, cy + s * 0.04, s * 0.95, cloud),
            Icon::Fog => {
                cloud_at(self, cx, cy - s * 0.1, s * 0.85, dark);
                for i in 0..3 {
                    let y = cy + s * (0.2 + 0.13 * i as f32);
                    line(&mut self.layer, cx - s * 0.36, y, cx + s * 0.36, y, [200, 206, 220, al(220)], s * 0.06, true);
                }
            }
            Icon::Drizzle | Icon::Rain => {
                cloud_at(self, cx, cy - s * 0.12, s * 0.85, dark);
                let n = if kind == Icon::Rain { 3 } else { 2 };
                for i in 0..n {
                    let x = cx - s * 0.2 + s * (0.2 * i as f32) / (n as f32 - 1.0).max(1.0) * (if n == 3 { 2.0 } else { 1.0 });
                    line(&mut self.layer, x + s * 0.04, cy + s * 0.2, x - s * 0.04, cy + s * 0.4, [90, 170, 255, al(255)], s * 0.065, true);
                }
            }
            Icon::Snow => {
                cloud_at(self, cx, cy - s * 0.12, s * 0.85, dark);
                for (dx, dy) in [(-0.22, 0.28), (0.0, 0.4), (0.22, 0.28)] {
                    circle(&mut self.layer, cx + dx * s, cy + dy * s, s * 0.05, [255, 255, 255, al(255)]);
                }
            }
            Icon::Thunder => {
                cloud_at(self, cx, cy - s * 0.12, s * 0.85, [120, 128, 150, al(255)]);
                let mut pb = PathBuilder::new();
                for (i, (x, y)) in [(0.06, 0.12), (-0.12, 0.34), (-0.01, 0.34), (-0.1, 0.52), (0.16, 0.26), (0.04, 0.26), (0.14, 0.12)].iter().enumerate() {
                    if i == 0 {
                        pb.move_to(cx + x * s, cy + y * s);
                    } else {
                        pb.line_to(cx + x * s, cy + y * s);
                    }
                }
                pb.close();
                if let Some(p) = pb.finish() {
                    fill(&mut self.layer, &p, [255, 214, 60, al(255)]);
                }
            }
        }
    }

    pub(super) fn draw_weather_widget(&mut self, b: Rect, p: &WeatherProps) {
        self.card(b);
        let col = parse_color(&p.color, [255, 255, 255]);
        let data = if self.sample { Ok(weather::sample()) } else { weather::get() };
        let d = match data {
            Ok(d) => d,
            Err(why) => {
                let msg = if why == "no place" { "Weather: choose a place in Settings" } else { "Weather: loading" };
                let sz = self.fit_px(msg, Weight::Regular, b.w * 0.9, b.h * 0.2);
                let base = b.y + (b.h + sz as f32 * 0.72) / 2.0;
                self.text_c(msg, b, base, sz, Weight::Regular, GREY, 0.9);
                return;
            }
        };
        let al = if d.stale { 0.55 } else { 1.0 };
        let f = p.unit.eq_ignore_ascii_case("F");
        let conv = |c: f32| if f { c * 9.0 / 5.0 + 32.0 } else { c };
        let (icon, text) = weather::describe(d.code, d.is_day);
        let pad = b.w.min(b.h) * 0.07;
        let inner = Rect { x: b.x + pad, y: b.y + pad, w: b.w - 2.0 * pad, h: b.h - 2.0 * pad };
        let forecast = p.style == "forecast";
        let top_h = if forecast { inner.h * 0.5 } else { inner.h };
        // "now": icon left, texts right
        let isz = (top_h.min(inner.w * 0.42)) * 0.92;
        self.draw_icon(icon, inner.x + isz / 2.0, inner.y + top_h / 2.0, isz, al);
        let tx = inner.x + isz + inner.w * 0.04;
        let tw = (inner.x + inner.w - tx).max(10.0);
        let temp = format!("{:.0}\u{00B0}", conv(d.temp_c));
        let tsz = self.fit_px(&temp, Weight::Bold, tw * 0.7, top_h * 0.5);
        let base = inner.y + top_h * 0.5 + tsz as f32 * 0.2 - if forecast { 0.0 } else { top_h * 0.06 };
        self.text(&temp, tx, base, tsz, Weight::Bold, col, al);
        let csz = self.fit_px(text, Weight::Regular, tw, top_h * 0.17).min((tsz as f32 * 0.42) as u32).max(8);
        self.text(text, tx, base + csz as f32 * 1.25, csz, Weight::Regular, mix(col, LAB, 0.35), 0.95 * al);
        if !d.place.is_empty() {
            let psz = (csz as f32 * 0.82).round().max(7.0) as u32;
            let place = self.fit(&d.place, psz, Weight::Regular, tw);
            self.text(&place, tx, base - tsz as f32 * 0.78 - psz as f32 * 0.35, psz, Weight::Regular, LAB, 0.85 * al);
        }
        if forecast {
            let count = if p.span == "hours" { (p.count as usize).clamp(3, 6) } else { (p.count as usize).clamp(3, 5) };
            let y0 = inner.y + top_h + inner.h * 0.03;
            let row_h = inner.y + inner.h - y0;
            let cell_w = inner.w / count as f32;
            line(&mut self.layer, inner.x, y0 - inner.h * 0.015, inner.x + inner.w, y0 - inner.h * 0.015, [255, 255, 255, 36], 1.0, false);
            for i in 0..count {
                let cx = inner.x + cell_w * (i as f32 + 0.5);
                let (lab, ic, top, bot): (String, Icon, String, Option<String>) = if p.span == "hours" {
                    match d.hours.get(1 + i * 2) {
                        Some(h) => (format!("{:02}h", h.hour), weather::describe(h.code, h.hour >= 7 && h.hour < 20).0, format!("{:.0}\u{00B0}", conv(h.temp_c)), None),
                        None => continue,
                    }
                } else {
                    match d.days.get(1 + i) {
                        Some(dy) => (["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][(dy.weekday % 7) as usize].to_string(), weather::describe(dy.code, true).0, format!("{:.0}\u{00B0}", conv(dy.hi_c)), Some(format!("{:.0}\u{00B0}", conv(dy.lo_c)))),
                        None => continue,
                    }
                };
                let lsz = ((row_h * 0.18).min(cell_w * 0.3)).round().max(7.0) as u32;
                self.text_c(&lab, Rect { x: cx - cell_w / 2.0, y: 0.0, w: cell_w, h: 0.0 }, y0 + lsz as f32 * 1.0, lsz, Weight::Regular, LAB, 0.95 * al);
                self.draw_icon(ic, cx, y0 + row_h * 0.46, (row_h * 0.42).min(cell_w * 0.7), al);
                let vsz = ((row_h * 0.2).min(cell_w * 0.3)).round().max(7.0) as u32;
                let tw2 = self.fonts.width(&top, vsz, Weight::Bold);
                match bot {
                    Some(lo) => {
                        let lw = self.fonts.width(&lo, vsz, Weight::Regular);
                        let gap = vsz as f32 * 0.4;
                        let x0 = cx - (tw2 + gap + lw) / 2.0;
                        self.text(&top, x0, y0 + row_h * 0.93, vsz, Weight::Bold, col, al);
                        self.text(&lo, x0 + tw2 + gap, y0 + row_h * 0.93, vsz, Weight::Regular, LAB, 0.9 * al);
                    }
                    None => self.text(&top, cx - tw2 / 2.0, y0 + row_h * 0.93, vsz, Weight::Bold, col, al),
                }
            }
        }
    }

    // ------------------------------------------------------------ visualizer

    fn viz_levels(&mut self, n: usize, sens: f32) -> Vec<f32> {
        let raw: Vec<f32> = if self.sample {
            (0..audio::BANDS).map(|i| {
                let x = i as f32 / audio::BANDS as f32;
                (0.78 * (1.0 - x * 0.55) * (0.55 + 0.45 * (i as f32 * 0.9).sin().abs()) + 0.1 * (i as f32 * 2.3).cos()).clamp(0.02, 1.0)
            }).collect()
        } else {
            audio::audio().levels()
        };
        // group the 64 bands into n
        let n = n.clamp(4, audio::BANDS);
        (0..n)
            .map(|i| {
                let (a, z) = (i * audio::BANDS / n, ((i + 1) * audio::BANDS / n).max(i * audio::BANDS / n + 1));
                let v = raw[a..z.min(raw.len())].iter().cloned().fold(0.0f32, f32::max);
                (v * sens).clamp(0.0, 1.0)
            })
            .collect()
    }

    pub(super) fn draw_visualizer(&mut self, b: Rect, id: &str, p: &VisualizerProps) {
        let col = parse_color(&p.color, [90, 200, 255]);
        let n = (p.bands as usize).clamp(16, 64);
        let sens = if p.sensitivity.is_finite() { p.sensitivity.clamp(0.2, 4.0) } else { 1.0 };
        let lv = self.viz_levels(n, sens);
        match p.style.as_str() {
            "ring" => {
                let d = b.w.min(b.h);
                let (cx, cy) = (b.x + b.w / 2.0, b.y + b.h / 2.0);
                let r0 = d * 0.26;
                let maxlen = d * 0.24;
                let width = (std::f32::consts::TAU * r0 / n as f32 * 0.55).max(2.0);
                for (i, v) in lv.iter().enumerate() {
                    let a = (i as f32 / n as f32 * 360.0 - 90.0).to_radians();
                    let len = maxlen * (0.06 + 0.94 * v);
                    let c = mix(col, [255, 255, 255], v * 0.5);
                    line(&mut self.layer, cx + r0 * a.cos(), cy + r0 * a.sin(), cx + (r0 + len) * a.cos(), cy + (r0 + len) * a.sin(), [c[0], c[1], c[2], (110.0 + 145.0 * v) as u8], width, true);
                }
                if let Some(pp) = PathBuilder::from_circle(cx, cy, r0 * 0.94) {
                    stroke(&mut self.layer, &pp, [col[0], col[1], col[2], 70], (d * 0.008).max(1.0), false);
                }
            }
            "wave-fade" => {
                // scrolling spectrogram: columns of the last seconds, brightness = level, scrolling sideways, frequencies on fixed rows
                let cols = ((b.w / 8.0) as usize).clamp(24, 200);
                let sample = self.sample;
                let hist = self.viz_hist.entry(id.to_string()).or_default();
                if sample && hist.is_empty() {
                    for c in 0..cols {
                        hist.push_back((0..lv.len()).map(|i| (0.5 + 0.5 * (c as f32 * 0.22 + i as f32 * 0.45).sin()).powf(1.6) * (1.0 - i as f32 / lv.len() as f32 * 0.55)).collect());
                    }
                }
                let last = self.viz_last.entry(id.to_string()).or_insert(u64::MAX);
                // a new column about every 70 ms
                let step = (self.tick as f32 * 10.0 / 7.0) as u64;
                if *last != step {
                    *last = step;
                    hist.push_back(lv.clone());
                    while hist.len() > cols {
                        hist.pop_front();
                    }
                }
                let rows = lv.len();
                let mut img = Pixmap::new(cols as u32, rows as u32).unwrap();
                {
                    let d = img.data_mut();
                    let offset = cols.saturating_sub(hist.len());
                    for (x, colv) in hist.iter().enumerate() {
                        for (r, v) in colv.iter().enumerate() {
                            let y = rows - 1 - r; // low frequencies at the bottom
                            let a = (v.powf(1.3) * 255.0) as u32;
                            let o = (y * cols + x + offset) * 4;
                            d[o] = (col[0] as u32 * a / 255) as u8;
                            d[o + 1] = (col[1] as u32 * a / 255) as u8;
                            d[o + 2] = (col[2] as u32 * a / 255) as u8;
                            d[o + 3] = a as u8;
                        }
                    }
                }
                let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..PixmapPaint::default() };
                let t = Transform::from_row(b.w / cols as f32, 0.0, 0.0, b.h / rows as f32, b.x, b.y);
                self.layer.draw_pixmap(0, 0, img.as_ref(), &paint, t, None);
            }
            _ => {
                // horizontal bars from the left, one row per band
                let gap = (b.h / n as f32 * 0.25).max(0.5);
                let rh = (b.h - gap * (n as f32 - 1.0)) / n as f32;
                for (i, v) in lv.iter().enumerate() {
                    let y = b.y + i as f32 * (rh + gap);
                    round_rect(&mut self.layer, b.x, y, b.w, rh, rh / 2.0, [255, 255, 255, 22]);
                    let c = mix(col, [255, 255, 255], v * 0.45);
                    round_rect(&mut self.layer, b.x, y, (b.w * (0.015 + 0.985 * v)).max(rh), rh, rh / 2.0, [c[0], c[1], c[2], (120.0 + 135.0 * v) as u8]);
                }
            }
        }
    }

    // ------------------------------------------------------------ slideshow

    fn load_item(dir: &std::path::Path, file: &str) -> Option<Item> {
        let path = resolve_image(dir, file)?;
        let bytes = std::fs::read(&path).ok()?;
        if crate::live::gif::is_gif(&bytes) {
            if let Ok(g) = crate::live::gif::decode(&bytes) {
                let mut frames = Vec::new();
                let mut total = 0;
                for f in g.frames {
                    let pm = Pixmap::from_vec(f.rgba, tiny_skia::IntSize::from_wh(g.w as u32, g.h as u32)?)?;
                    total += f.delay_ms;
                    frames.push((Arc::new(pm), f.delay_ms));
                }
                return Some(Item { frames, total_ms: total.max(1) });
            }
        }
        let pm = decode_image(&path)?;
        Some(Item { frames: vec![(Arc::new(pm), 1000)], total_ms: 1000 })
    }

    pub(super) fn draw_slideshow(&mut self, b: Rect, id: &str, p: &SlideshowProps, w: &Widget) {
        let _ = w;
        let files: Vec<String> = p.files.iter().filter(|f| !f.trim().is_empty()).cloned().collect();
        let dir = self.theme.theme_dir.clone();
        let st = self.slides.entry(id.to_string()).or_insert_with(|| Slides { files: Vec::new(), dir: PathBuf::new(), items: Vec::new(), scaled: Vec::new() });
        if st.files != files || st.dir != dir {
            *st = Slides { files: files.clone(), dir: dir.clone(), items: files.iter().map(|_| None).collect(), scaled: Vec::new() };
        }
        if files.is_empty() {
            let msg = "Slideshow: add pictures";
            let sz = self.fit_px(msg, Weight::Regular, b.w * 0.9, b.h * 0.18);
            self.card(b);
            let base = b.y + (b.h + sz as f32 * 0.72) / 2.0;
            self.text_c(msg, b, base, sz, Weight::Regular, GREY, 0.9);
            return;
        }
        let secs = p.seconds.clamp(1.0, 3600.0);
        let fade = p.fade.clamp(0.0, 5.0).min(secs * 0.9);
        let t = self.t0.elapsed().as_secs_f32();
        let n = files.len();
        let (idx, into) = (((t / secs) as usize) % n, t % secs);
        let (mut cur_a, mut next_a) = (1.0f32, 0.0f32);
        if n > 1 && fade > 0.0 && into > secs - fade {
            next_a = (into - (secs - fade)) / fade;
            cur_a = 1.0 - next_a;
        }
        let cover = p.fit == "cover";
        let (bw, bh) = (b.w.round() as u32, b.h.round() as u32);
        let mut order = vec![(idx, cur_a)];
        if next_a > 0.0 {
            order.push(((idx + 1) % n, next_a));
        }
        for (fi, a) in order {
            // load on demand
            if self.slides.get(id).and_then(|s| s.items.get(fi)).is_some_and(|i| i.is_none()) {
                let item = Self::load_item(&dir, &files[fi]);
                if let Some(s) = self.slides.get_mut(id) {
                    s.items[fi] = Some(item.unwrap_or(Item { frames: Vec::new(), total_ms: 1 }));
                }
            }
            let Some(st) = self.slides.get_mut(id) else { return };
            let Some(Some(item)) = st.items.get(fi) else { continue };
            if item.frames.is_empty() {
                continue;
            }
            // GIF frame by time
            let mut fr = 0;
            if item.frames.len() > 1 {
                let tm = (t * 1000.0) as u32 % item.total_ms;
                let mut acc = 0;
                for (k, (_, d)) in item.frames.iter().enumerate() {
                    acc += d;
                    if tm < acc {
                        fr = k;
                        break;
                    }
                }
            }
            let key = (fi, fr, bw, bh, cover);
            if !st.scaled.iter().any(|s| s.0 == key) {
                let src = item.frames[fr].0.clone();
                let (_, _, _, pm, ox, oy) = scale_to_box(&src, bw, bh, cover);
                if st.scaled.len() > 48 {
                    st.scaled.clear();
                }
                st.scaled.push((key, pm, ox, oy));
            }
            let Some((_, pm, ox, oy)) = st.scaled.iter().find(|s| s.0 == key) else { continue };
            let paint = PixmapPaint { opacity: a, ..PixmapPaint::default() };
            self.layer.draw_pixmap(b.x as i32 + ox, b.y as i32 + oy, pm.as_ref(), &paint, Transform::identity(), None);
        }
    }

    // ------------------------------------------------------------ info

    pub(super) fn draw_info(&mut self, b: Rect, p: &InfoProps) {
        let col = parse_color(&p.color, [255, 255, 255]);
        self.card(b);
        let pad = b.w.min(b.h) * 0.07;
        let inner = Rect { x: b.x + pad, y: b.y + pad, w: b.w - 2.0 * pad, h: b.h - 2.0 * pad };
        match p.kind.as_str() {
            "topProcesses" => self.info_processes(inner, p, col),
            kind => {
                let (label, value): (String, String) = match kind {
                    "cpuName" => ("CPU".into(), if self.sample { "AMD Ryzen 7 9800X3D 8-Core Processor".to_string() } else { metrics::hub().cpu_name().unwrap_or_else(|| "\u{2013}".into()) }),
                    "gpuName" => ("GPU".into(), if self.sample { "NVIDIA GeForce RTX 4080".to_string() } else { metrics::hub().gpu_name().unwrap_or_else(|| "\u{2013}".into()) }),
                    "countdown" => {
                        let label = p.label.clone().unwrap_or_else(|| "Countdown".into());
                        let now = crate::live::render::local_time();
                        (label, countdown_text(p.target.as_deref(), &now))
                    }
                    _ => ("Uptime".into(), uptime_text(if self.sample { 3 * 86400 + 4 * 3600 + 12 * 60 } else { uptime_secs() })),
                };
                let lsz = (inner.h * 0.2).min(inner.w * 0.12).max(8.0).round() as u32;
                if !label.is_empty() {
                    let l = self.fit(&label, lsz, Weight::Regular, inner.w);
                    self.text(&l, inner.x, inner.y + lsz as f32 * 0.85, lsz, Weight::Regular, LAB, 1.0);
                }
                let top = inner.y + if label.is_empty() { 0.0 } else { lsz as f32 * 1.15 };
                let area_h = inner.y + inner.h - top;
                let multi_line = kind == "cpuName" || kind == "gpuName";
                let dim = value == "\u{2013}";
                let vcol = if dim { GREY } else { col };
                if multi_line && self.fonts.width(&value, 100, Weight::Bold) / 100.0 * area_h * 0.4 > inner.w {
                    // long names: wrap onto two lines
                    let words: Vec<&str> = value.split(' ').collect();
                    let half = words.len().div_ceil(2);
                    let (l1, l2) = (words[..half].join(" "), words[half..].join(" "));
                    let sz = self.fit_px(&l1, Weight::Bold, inner.w, area_h * 0.4).min(self.fit_px(&l2, Weight::Bold, inner.w, area_h * 0.4));
                    let base = top + (area_h - sz as f32 * 1.45) / 2.0 + sz as f32 * 0.8;
                    self.text(&l1, inner.x, base, sz, Weight::Bold, vcol, 1.0);
                    self.text(&l2, inner.x, base + sz as f32 * 1.25, sz, Weight::Bold, vcol, 1.0);
                } else {
                    let sz = self.fit_px(&value, Weight::Bold, inner.w, area_h * 0.7);
                    let base = top + (area_h + sz as f32 * 0.72) / 2.0;
                    self.text(&value, inner.x, base, sz, Weight::Bold, vcol, 1.0);
                }
            }
        }
    }

    fn info_processes(&mut self, b: Rect, p: &InfoProps, col: [u8; 3]) {
        let by_ram = p.by == "ram";
        let count = (p.count as usize).clamp(3, 5);
        let rows: Vec<(String, f32, f32)> = if self.sample {
            vec![("chrome".into(), 14.0, 2400.0), ("CoolerScreen".into(), 6.0, 160.0), ("Code".into(), 4.0, 980.0), ("Spotify".into(), 2.0, 420.0), ("explorer".into(), 1.0, 210.0)]
                .into_iter().take(count).collect()
        } else {
            metrics::hub().top_processes(by_ram, count).into_iter().map(|r| (r.name, r.cpu_pct, r.ram_mb)).collect()
        };
        let head = format!("Top processes by {}", if by_ram { "RAM" } else { "CPU" });
        let hsz = (b.h * 0.15).min(b.w * 0.07).max(8.0).round() as u32;
        let hh = self.fit(&head, hsz, Weight::Regular, b.w);
        self.text(&hh, b.x, b.y + hsz as f32 * 0.85, hsz, Weight::Regular, LAB, 1.0);
        let top = b.y + hsz as f32 * 1.35;
        let row_h = (b.y + b.h - top) / count as f32;
        let sz = (row_h * 0.62).round().max(8.0) as u32;
        for (i, (name, cpu, ram)) in rows.iter().enumerate() {
            let val = if by_ram {
                if *ram >= 1024.0 { format!("{:.1} GB", ram / 1024.0) } else { format!("{:.0} MB", ram) }
            } else {
                format!("{:.0} %", cpu)
            };
            let vw = self.fonts.width(&val, sz, Weight::Bold);
            let base = top + row_h * (i as f32 + 0.5) + sz as f32 * 0.36;
            let nm = self.fit(name, sz, Weight::Regular, (b.w - vw - 12.0).max(20.0));
            self.text(&nm, b.x, base, sz, Weight::Regular, col, 0.95);
            self.text(&val, b.x + b.w - vw, base, sz, Weight::Bold, col, 1.0);
        }
        if rows.is_empty() {
            self.text("\u{2013}", b.x, top + row_h * 0.7, sz, Weight::Bold, GREY, 1.0);
        }
    }
}

fn uptime_secs() -> u64 {
    use windows_sys::Win32::System::SystemInformation::GetTickCount64;
    unsafe { GetTickCount64() / 1000 }
}

pub(crate) fn uptime_text(s: u64) -> String {
    let (d, h, m) = (s / 86400, (s / 3600) % 24, (s / 60) % 60);
    if d > 0 {
        format!("{d}d {h}h {m:02}m")
    } else {
        format!("{h}:{m:02}:{:02}", s % 60)
    }
}

/// "YYYY-MM-DDTHH:MM[:SS]" (local) -> seconds since 1970 (no time zone applied: the clock is local too).
pub(crate) fn parse_local_iso(s: &str) -> Option<i64> {
    let s = s.trim().replace(' ', "T");
    let (date, time) = s.split_once('T').unwrap_or((&s, "00:00"));
    let mut dp = date.split('-');
    let (y, m, d): (i32, u32, u32) = (dp.next()?.parse().ok()?, dp.next()?.parse().ok()?, dp.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let mut tp = time.split(':');
    let (hh, mm, ss): (i64, i64, i64) = (tp.next()?.parse().ok()?, tp.next().unwrap_or("0").parse().ok()?, tp.next().unwrap_or("0").parse().ok()?);
    Some(weather::days_from_civil(y, m, d) * 86400 + hh * 3600 + mm * 60 + ss)
}

fn countdown_text(target: Option<&str>, now: &LocalTime) -> String {
    let Some(t) = target.and_then(parse_local_iso) else { return "\u{2013}".into() };
    let n = weather::days_from_civil(now.year as i32, now.month as u32, now.day as u32) * 86400 + now.hour as i64 * 3600 + now.minute as i64 * 60 + now.second as i64;
    let left = t - n;
    if left <= 0 {
        return "Now".into();
    }
    let (d, h, m, s) = (left / 86400, (left / 3600) % 24, (left / 60) % 60, left % 60);
    if d > 0 { format!("{d}d {h:02}:{m:02}:{s:02}") } else { format!("{h:02}:{m:02}:{s:02}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn countdown_and_uptime_text() {
        let now = LocalTime { year: 2026, month: 10, weekday: 6, day: 18, hour: 12, minute: 0, second: 0, millis: 0 };
        assert_eq!(countdown_text(Some("2026-10-20T14:30:15"), &now), "2d 02:30:15");
        assert_eq!(countdown_text(Some("2026-10-18T13:00"), &now), "01:00:00");
        assert_eq!(countdown_text(Some("2026-10-01T00:00"), &now), "Now");
        assert_eq!(countdown_text(Some("rubbish"), &now), "\u{2013}");
        assert_eq!(countdown_text(None, &now), "\u{2013}");
        assert_eq!(uptime_text(3 * 86400 + 4 * 3600 + 12 * 60), "3d 4h 12m");
        assert_eq!(uptime_text(3725), "1:02:05");
        assert_eq!(civil_days_in_month(2024, 2), 29);
        assert_eq!(civil_days_in_month(2026, 2), 28);
    }
}
