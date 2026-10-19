//! Drawing of the `sunmoon` widget (4.0 W1): sunrise, sunset and day length, and the moon phase as a picture with its name
//! and lit percentage. Wide box: sun block left, moon right; tall box: stacked. Big numbers, nothing moves.
//! The editor preview (`self.sample`) uses a fixed sample (Sydney, 2026-10-15, a fixed moon age) so it is stable.
use super::sensors::{circle, fill, stroke, GREY};
use super::{LocalTime, Rect, Renderer, Weight};
use crate::live::sunmoon::{self as sm, SunDay};
use crate::live::theme::{parse_color, Widget};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tiny_skia::PathBuilder;

const LAB: [u8; 3] = [185, 192, 208];
const MOON_TXT: [u8; 3] = [236, 239, 246];
const LIT: [u8; 4] = [236, 238, 244, 255];
const DARK: [u8; 4] = [52, 60, 80, 255];

/// The fixed preview sample: Sydney, a date in AEDT (UTC+11), a waxing gibbous moon.
const SAMPLE_PLACE: (f64, f64) = (-33.87, 151.21);
const SAMPLE_DATE: (i32, u32, u32) = (2026, 10, 15);
const SAMPLE_OFFSET_MIN: i32 = 660;
const SAMPLE_MOON_AGE: f64 = 10.5;

/// The chosen city, re-read from the settings file at most every few seconds (not every frame).
fn place() -> Option<(f64, f64)> {
    static CACHE: Mutex<Option<(Instant, Option<(f64, f64)>)>> = Mutex::new(None);
    let mut g = CACHE.lock().ok()?;
    if let Some((t, v)) = g.as_ref() {
        if t.elapsed() < Duration::from_secs(5) {
            return *v;
        }
    }
    let v = crate::live::weather::read_place().map(|p| (p.lat, p.lon));
    *g = Some((Instant::now(), v));
    v
}

/// Lit part of the moon disc as a closed polyline: the outer limb (a half circle) and the terminator (half an ellipse).
/// `age` in days; `flip` mirrors left/right (southern hemisphere). Waxing = lit on the right in the north.
fn lit_path(cx: f32, cy: f32, r: f32, age: f64, flip: bool) -> Option<tiny_skia::Path> {
    let f = (age / sm::SYNODIC).rem_euclid(1.0);
    let c = (2.0 * std::f64::consts::PI * f).cos() as f32; // 1 new .. -1 full
    let waxing = f < 0.5;
    let side = if waxing != flip { 1.0f32 } else { -1.0 }; // +1 = lit limb on the right
    let n = 90;
    let mut pb = PathBuilder::new();
    // limb: top to bottom on the lit side
    for i in 0..=n {
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::PI * i as f32 / n as f32;
        let (x, y) = (cx + side * r * a.cos(), cy + r * a.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    // terminator: bottom back to top. Its x per row is c times the disc's half width, toward the lit side.
    for i in 0..=n {
        let a = std::f32::consts::FRAC_PI_2 - std::f32::consts::PI * i as f32 / n as f32;
        pb.line_to(cx + side * c * r * a.cos(), cy + r * a.sin());
    }
    pb.close();
    pb.finish()
}

impl Renderer {
    pub(super) fn draw_sunmoon(&mut self, b: Rect, w: &Widget, now: &LocalTime) {
        let inner = self.mcard(b);
        let p = w.sunmoon();
        let col = parse_color(&p.color, [255, 210, 122]);
        let (sun_on, moon_on) = (p.shows("sun"), p.shows("moon"));
        let (place, date, offset, age) = if self.sample {
            // (test only: a widget with this id previews the "no city" message)
            let none = cfg!(test) && w.id == "nocity-test";
            (if none { None } else { Some(SAMPLE_PLACE) }, SAMPLE_DATE, Some(SAMPLE_OFFSET_MIN), SAMPLE_MOON_AGE)
        } else {
            (place(), (now.year as i32, now.month as u32, now.day as u32), None, sm::moon_age_now())
        };
        let flip = place.map(|(lat, _)| lat < 0.0).unwrap_or(false);
        let h12 = p.format == "12h";
        let gap = (inner.w.min(inner.h) * 0.06).max(4.0);
        let (sun_r, moon_r) = match (sun_on, moon_on) {
            (true, true) => {
                if inner.w >= inner.h * 1.15 {
                    let sw = (inner.w - gap) * 0.57;
                    (Some(Rect { w: sw, ..inner }), Some(Rect { x: inner.x + sw + gap, w: inner.w - sw - gap, ..inner }))
                } else {
                    let sh = (inner.h - gap) * 0.5;
                    (Some(Rect { h: sh, ..inner }), Some(Rect { y: inner.y + sh + gap, h: inner.h - sh - gap, ..inner }))
                }
            }
            (true, false) => (Some(inner), None),
            _ => (None, Some(inner)),
        };
        if let Some(r) = sun_r {
            match place {
                Some((lat, lon)) => {
                    let day = sm::sun_day(lat, lon, date.0, date.1, date.2, offset);
                    self.draw_sun_block(r, day, h12, col);
                }
                None => self.draw_note(r, &["Set your city", "in Settings"]),
            }
        }
        if let Some(r) = moon_r {
            self.draw_moon_block(r, age, flip);
        }
    }

    /// A dim centred message (one to three short lines).
    fn draw_note(&mut self, r: Rect, lines: &[&str]) {
        let longest = lines.iter().max_by_key(|s| s.chars().count()).copied().unwrap_or("");
        let n = lines.len() as f32;
        let sz = self.fit_px(longest, Weight::Regular, r.w, r.h / (n * 1.3)).min(((r.w).max(r.h) * 0.2) as u32).max(6);
        let total = sz as f32 * 1.3 * n;
        for (i, l) in lines.iter().enumerate() {
            let t = self.fit(l, sz, Weight::Regular, r.w);
            let tw = self.fonts.width(&t, sz, Weight::Regular);
            let base = r.y + (r.h - total) / 2.0 + sz as f32 * (1.3 * i as f32 + 1.0);
            self.text(&t, r.x + (r.w - tw) / 2.0, base, sz, Weight::Regular, GREY, 0.9);
        }
    }

    fn draw_sun_block(&mut self, r: Rect, day: SunDay, h12: bool, col: [u8; 3]) {
        let (rise, set, len) = match day {
            SunDay::Times { rise, set, len } => (rise, set, len),
            SunDay::AlwaysUp => return self.draw_polar(r, "Sun up all day", "Day 24 h", col),
            SunDay::AlwaysDown => return self.draw_polar(r, "Sun down all day", "Day 0 h", col),
        };
        let (rs, ss) = (sm::fmt_time(rise, h12), sm::fmt_time(set, h12));
        let day_txt = sm::fmt_day_len(len);
        let widest = if self.fonts.width(&rs, 100, Weight::Bold) >= self.fonts.width(&ss, 100, Weight::Bold) { rs.clone() } else { ss.clone() };
        let dsz = self.fit_px(&day_txt, Weight::Regular, r.w, r.h * 0.13);
        let day_h = dsz as f32 * 1.5;
        let cells_h = (r.h - day_h).max(1.0);
        let gap = r.w * 0.04;
        // Side by side or stacked: whichever gives the bigger numbers.
        let side_w = (r.w - gap) / 2.0;
        let px_side = self.fit_px(&widest, Weight::Bold, side_w, cells_h * 0.6);
        let px_stack = self.fit_px(&widest, Weight::Bold, r.w, cells_h / 2.0 * 0.6);
        let side = px_side >= px_stack;
        let tpx = px_side.max(px_stack).min((r.h * 0.5) as u32).max(6);
        let lsz = ((tpx as f32 * 0.4).round() as u32).max(6);
        let cells: [(&str, &String); 2] = [("Sunrise", &rs), ("Sunset", &ss)];
        for (i, (lab, val)) in cells.iter().enumerate() {
            let (cx, cy, cw, ch) = if side {
                (r.x + i as f32 * (side_w + gap), r.y, side_w, cells_h)
            } else {
                (r.x, r.y + i as f32 * cells_h / 2.0, r.w, cells_h / 2.0)
            };
            let content = lsz as f32 * 1.2 + tpx as f32 * 0.74;
            let top = cy + (ch - content) / 2.0;
            let label = self.fit(lab, lsz, Weight::Regular, cw);
            self.text(&label, cx, top + lsz as f32 * 0.85, lsz, Weight::Regular, LAB, 1.0);
            let t = self.fit(val, tpx, Weight::Bold, cw);
            self.text(&t, cx, top + lsz as f32 * 1.2 + tpx as f32 * 0.74, tpx, Weight::Bold, col, 1.0);
        }
        self.text(&day_txt, r.x, r.y + r.h - dsz as f32 * 0.3, dsz, Weight::Regular, LAB, 0.95);
    }

    fn draw_polar(&mut self, r: Rect, msg: &str, day_txt: &str, col: [u8; 3]) {
        let dsz = self.fit_px(day_txt, Weight::Regular, r.w, r.h * 0.13);
        let area = (r.h - dsz as f32 * 1.5).max(1.0);
        let sz = self.fit_px(msg, Weight::Bold, r.w, area * 0.4).max(6);
        let t = self.fit(msg, sz, Weight::Bold, r.w);
        self.text(&t, r.x, r.y + (area + sz as f32 * 0.74) / 2.0, sz, Weight::Bold, col, 1.0);
        self.text(day_txt, r.x, r.y + r.h - dsz as f32 * 0.3, dsz, Weight::Regular, LAB, 0.95);
    }

    fn draw_moon_block(&mut self, r: Rect, age: f64, flip: bool) {
        let name = sm::phase_name(age);
        let lit = format!("{:.0} % lit", sm::illumination(age) * 100.0);
        let nsz = self.fit_px(name, Weight::Bold, r.w, r.h * 0.12);
        let isz = ((nsz as f32 * 0.85).round() as u32).max(6);
        let text_h = nsz as f32 * 1.3 + isz as f32 * 1.25;
        let mut d = r.w.min(r.h - text_h - r.h * 0.04);
        let with_text = d >= r.w.min(r.h) * 0.45 && d >= 12.0;
        if !with_text {
            d = r.w.min(r.h);
        }
        let used = if with_text { d + r.h * 0.04 + text_h } else { d };
        let top = r.y + (r.h - used) / 2.0;
        let (cx, cy, rad) = (r.x + r.w / 2.0, top + d / 2.0, d / 2.0);
        self.draw_moon(cx, cy, rad, age, flip);
        if with_text {
            let ty = top + d + r.h * 0.04;
            let n = self.fit(name, nsz, Weight::Bold, r.w);
            let nw = self.fonts.width(&n, nsz, Weight::Bold);
            self.text(&n, r.x + (r.w - nw) / 2.0, ty + nsz as f32 * 0.95, nsz, Weight::Bold, MOON_TXT, 1.0);
            let lw = self.fonts.width(&lit, isz, Weight::Regular);
            self.text(&lit, r.x + (r.w - lw) / 2.0, ty + nsz as f32 * 1.3 + isz as f32 * 0.95, isz, Weight::Regular, LAB, 1.0);
        }
    }

    fn draw_moon(&mut self, cx: f32, cy: f32, r: f32, age: f64, flip: bool) {
        circle(&mut self.layer, cx, cy, r, DARK);
        if let Some(path) = lit_path(cx, cy, r, age, flip) {
            fill(&mut self.layer, &path, LIT);
        }
        // A few soft maria, over the whole disc (barely visible on the dark side).
        for (dx, dy, k, a) in [(-0.30f32, -0.25f32, 0.22f32, 46u8), (0.22, -0.34, 0.15, 40), (0.12, 0.18, 0.26, 40), (-0.28, 0.38, 0.12, 40), (0.42, 0.30, 0.1, 36)] {
            let (mx, my, mr) = (cx + dx * r, cy + dy * r, k * r);
            if dx * dx + dy * dy < 0.85 {
                circle(&mut self.layer, mx, my, mr, [120, 128, 150, a]);
            }
        }
        if let Some(rim) = PathBuilder::from_circle(cx, cy, r) {
            stroke(&mut self.layer, &rim, [255, 255, 255, 60], (r * 0.03).max(1.0), false);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::live::theme::{Theme, ThemeLive, Widget};
    use serde_json::json;
    use std::path::PathBuf;

    fn theme(list: &[(&str, f32, f32, f32, f32, serde_json::Value)]) -> ThemeLive {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        for (i, (k, x, y, w, h, p)) in list.iter().enumerate() {
            let mut props = p.clone();
            crate::live::morewidgets::clean(k, &mut props);
            t.theme.widgets.push(Widget { id: format!("w{i}"), kind: (*k).into(), x: *x, y: *y, w: *w, h: *h, props, ..Widget::default() });
        }
        t
    }

    fn dump(name: &str, png: &[u8]) {
        if let Some(d) = std::env::var_os("COOLERSCREEN_B_DUMP") {
            let d = PathBuf::from(d);
            let _ = std::fs::create_dir_all(&d);
            std::fs::write(d.join(format!("{name}.png")), png).unwrap();
        }
    }

    #[test]
    fn terminator_shape_follows_the_phase() {
        // Waxing crescent: lit sliver on the right in the north, on the left when flipped.
        let px = |age: f64, flip: bool| {
            let mut pm = tiny_skia::Pixmap::new(100, 100).unwrap();
            let path = super::lit_path(50.0, 50.0, 40.0, age, flip).unwrap();
            super::fill(&mut pm, &path, [255, 255, 255, 255]);
            let a = |x: usize, y: usize| pm.data()[(y * 100 + x) * 4 + 3];
            (a(87, 50), a(13, 50), a(52, 50))
        };
        let (r, l, _) = px(3.0, false);
        assert!(r > 200 && l < 20, "waxing crescent north: right lit {r}, left {l}");
        let (r, l, _) = px(3.0, true);
        assert!(l > 200 && r < 20, "waxing crescent south: left lit {l}, right {r}");
        let (r, l, c) = px(26.0, false);
        assert!(l > 200 && r < 20 && c < 20, "waning crescent north: left lit");
        let (r, l, c) = px(11.0, false);
        assert!(r > 200 && l < 20 && c > 200, "gibbous: dark sliver on the left edge, centre lit {r} {l} {c}");
        let _ = (r, l, c);
        let g = |x: usize| { let mut pm = tiny_skia::Pixmap::new(100, 100).unwrap(); let path = super::lit_path(50.0, 50.0, 40.0, 11.0, false).unwrap(); super::fill(&mut pm, &path, [255, 255, 255, 255]); pm.data()[(50 * 100 + x) * 4 + 3] };
        assert!(g(25) > 200 && g(87) > 200 && g(13) < 20, "gibbous: both sides and centre lit");
        let (r, l, c) = px(7.4, false);
        assert!(r > 200 && l < 20 && c > 100, "first quarter: right half");
    }

    #[test]
    fn sunmoon_draws_in_every_layout_and_tiny_boxes() {
        let wide = theme(&[("sunmoon", 0.04, 0.2, 0.5, 0.35, json!({}))]);
        dump("sunmoon_wide", &crate::live::render_preview(&wide, 1200, 540, true).unwrap());
        let tall = theme(&[("sunmoon", 0.04, 0.04, 0.26, 0.9, json!({ "format": "12h", "color": "#7ad2ff" }))]);
        dump("sunmoon_tall", &crate::live::render_preview(&tall, 1200, 540, true).unwrap());
        let sun = theme(&[("sunmoon", 0.04, 0.2, 0.3, 0.3, json!({ "show": ["sun"] })), ("sunmoon", 0.4, 0.2, 0.3, 0.3, json!({ "show": ["sun"], "format": "12h" }))]);
        dump("sunmoon_sun", &crate::live::render_preview(&sun, 1200, 540, true).unwrap());
        let moon = theme(&[("sunmoon", 0.04, 0.1, 0.2, 0.5, json!({ "show": ["moon"] })), ("sunmoon", 0.3, 0.3, 0.4, 0.2, json!({ "show": ["moon"] }))]);
        dump("sunmoon_moon", &crate::live::render_preview(&moon, 1200, 540, true).unwrap());
        let mut tiny = wide.clone();
        tiny.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        crate::live::render_preview(&tiny, 320, 144, true).unwrap();
        let mut tiny2 = tall.clone();
        tiny2.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.9;
        });
        crate::live::render_preview(&tiny2, 1200, 540, true).unwrap();
        let mut nc = theme(&[("sunmoon", 0.04, 0.2, 0.5, 0.35, json!({}))]);
        nc.theme.widgets[0].id = "nocity-test".into();
        dump("sunmoon_nocity", &crate::live::render_preview(&nc, 1200, 540, true).unwrap());
        // Garbage props never break the drawing.
        let mut bad = wide.clone();
        bad.theme.widgets[0].props = json!("garbage");
        crate::live::render_preview(&bad, 640, 288, true).unwrap();
    }

    /// Live (not sample) drawing with whatever city the settings hold (none on a clean machine): must not panic, and
    /// without a city the sun block is a message, not times.
    #[test]
    fn sunmoon_live_draws_without_a_city() {
        use std::sync::{Arc, Mutex};
        let mut r = crate::live::render::Renderer::new(Arc::new(Mutex::new(Default::default())), None);
        let mut t = theme(&[("sunmoon", 0.04, 0.2, 0.5, 0.35, json!({}))]);
        t.theme.widgets[0].id = "live-sunmoon".into();
        r.set_theme(t);
        let _ = r.render();
        dump("sunmoon_live", &crate::live::render_preview(&theme(&[("sunmoon", 0.04, 0.2, 0.5, 0.35, json!({ "show": ["sun"] }))]), 1200, 540, false).unwrap());
    }
}
