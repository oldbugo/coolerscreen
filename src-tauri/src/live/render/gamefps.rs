//! Drawing of the `gamefps` widget (4.0 G2): the frame rate of the game in front as a very big number, the game's name above it, an
//! optional "1 % low" line and an optional frame-time graph (history grows SIDEWAYS only, nothing moves up/down), a tiny source tag
//! bottom-right. Reads `crate::live::gamefps::read()` (a `View`, never blocks). In the editor preview (`self.sample`) a fixed sample
//! is drawn and `read()` is never called. Text is fitted to the box (`fit_px` / `fit`), so nothing overflows at any size.
use super::sensors::{fill, line, nice_ceil, stroke, GREY};
use super::{LocalTime, Rect, Renderer, Weight};
use crate::live::gamefps::{self, GameFpsProps, State, View};
use crate::live::theme::{parse_color, Widget};
use tiny_skia::PathBuilder;

const LAB: [u8; 3] = [185, 192, 208];
const AMBER: [u8; 3] = [240, 200, 70];
const CYAN: [u8; 3] = [90, 200, 255];

/// The fixed picture for the editor preview.
fn sample_view(secs: f32) -> View {
    let n = 90;
    let hist = (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1) as f32;
            let age = secs * (1.0 - t);
            let mut ms = 11.5 + (i as f32 * 0.9).sin() * 0.7 + (i as f32 * 2.3).sin() * 0.4;
            if i % 23 == 11 {
                ms += 9.0;
            }
            if i % 37 == 20 {
                ms += 17.0;
            }
            (age, ms)
        })
        .collect();
    View { state: State::Ok, game: "Monster Hunter Wilds".into(), fps: Some(87.0), low1: Some(61.0), frame_ms: Some(11.5), hist, source: "etw" }
}

fn source_tag(s: &str) -> &'static str {
    match s {
        "etw" => "ETW",
        "rtss" => "RTSS",
        "hwinfo" => "HWiNFO",
        _ => "",
    }
}

impl Renderer {
    pub(super) fn draw_gamefps(&mut self, b: Rect, w: &Widget, _now: &LocalTime) {
        let p = w.gamefps();
        let view = if self.sample { sample_view(p.history.clamp(10.0, 120.0)) } else { gamefps::read() };
        self.draw_view(b, &view, &p);
    }

    fn gf_text_r(&mut self, s: &str, right: f32, base: f32, px: u32, wt: Weight, col: [u8; 3], a: f32) {
        let w = self.fonts.width(s, px, wt);
        self.text(s, right - w, base, px, wt, col, a);
    }

    /// One centred-ish message line (+ optional muted second line) inside the card.
    fn gf_message(&mut self, inner: Rect, first: &str, col: [u8; 3], second: Option<&str>) {
        let h1 = if second.is_some() { inner.h * 0.3 } else { inner.h * 0.4 };
        let sz = self.fit_px(first, Weight::Bold, inner.w, h1);
        let t = self.fit(first, sz, Weight::Bold, inner.w);
        let total = sz as f32 * 1.0 + second.map_or(0.0, |_| sz as f32 * 0.75);
        let top = inner.y + ((inner.h - total) / 2.0).max(0.0);
        self.text(&t, inner.x, top + sz as f32 * 0.8, sz, Weight::Bold, col, 1.0);
        if let Some(s2) = second {
            let sz2 = self.fit_px(s2, Weight::Regular, inner.w, (sz as f32 * 0.6).max(8.0));
            let t2 = self.fit(s2, sz2, Weight::Regular, inner.w);
            self.text(&t2, inner.x, top + sz as f32 * 1.0 + sz2 as f32 * 0.95, sz2, Weight::Regular, LAB, 0.9);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn gf_plot(&mut self, hist: &[(f32, f32)], px: f32, py: f32, pw: f32, ph: f32, secs: f32, max: f32, col: [u8; 3]) {
        if hist.len() < 2 {
            return;
        }
        let mut pb = PathBuilder::new();
        let mut ab = PathBuilder::new();
        let mut last_x = px;
        for (i, (age, v)) in hist.iter().enumerate() {
            let x = px + pw * (1.0 - (age / secs).clamp(0.0, 1.0));
            let y = py + ph * (1.0 - (v / max).clamp(0.0, 1.0));
            if i == 0 {
                pb.move_to(x, y);
                ab.move_to(x, py + ph);
                ab.line_to(x, y);
            } else {
                pb.line_to(x, y);
                ab.line_to(x, y);
            }
            last_x = x;
        }
        ab.line_to(last_x, py + ph);
        ab.close();
        if let Some(a) = ab.finish() {
            fill(&mut self.layer, &a, [col[0], col[1], col[2], 60]);
        }
        if let Some(l) = pb.finish() {
            stroke(&mut self.layer, &l, [col[0], col[1], col[2], 255], (ph * 0.03).clamp(1.5, 3.0), true);
        }
    }

    pub(super) fn draw_view(&mut self, b: Rect, view: &View, p: &GameFpsProps) {
        if view.state == State::NoGame && p.hide_idle && !self.sample {
            return;
        }
        let inner = self.mcard(b);
        match view.state {
            State::NoGame => return self.gf_message(inner, "No game in front", GREY, None),
            State::NeedsGroup => return self.gf_message(inner, "Needs a one-time Windows setting", AMBER, Some("See the item's Properties")),
            State::Blocked => return self.gf_message(inner, "Blocked while the game runs", AMBER, Some("Restart CoolerScreen before the game")),
            State::Unavailable => return self.gf_message(inner, "Frame rate unavailable", GREY, None),
            State::Ok => {}
        }
        let col = parse_color(&p.color, CYAN);
        let secs = p.history.clamp(10.0, 120.0);
        let tiny = inner.h < 30.0;
        // name row (top)
        let lsz = (inner.h * 0.12).min(inner.w * 0.07).max(8.0).round() as u32;
        let name_h = if tiny { 0.0 } else { lsz as f32 * 1.25 };
        if !tiny && !view.game.is_empty() {
            let reserve = if source_tag(view.source).is_empty() { 0.0 } else { 0.0 };
            let t = self.fit(&view.game, lsz, Weight::Regular, inner.w - reserve);
            self.text(&t, inner.x, inner.y + lsz as f32 * 0.9, lsz, Weight::Regular, LAB, 0.9);
        }
        // footer row (source tag, bottom-right)
        let tag = source_tag(view.source);
        let tsz = ((lsz as f32 * 0.8).round() as u32).max(7);
        let foot = if tag.is_empty() || tiny { 0.0 } else { tsz as f32 * 1.3 };
        if foot > 0.0 {
            self.gf_text_r(tag, inner.x + inner.w, inner.y + inner.h - tsz as f32 * 0.2, tsz, Weight::Regular, LAB, 0.6);
        }
        let avail_y = inner.y + name_h;
        let avail_h = (inner.h - name_h - foot).max(4.0);
        let graph_h = if p.graph { avail_h * 0.42 } else { 0.0 };
        let head_h = (avail_h - graph_h).max(4.0);
        let low_h = if p.low && view.low1.is_some() && head_h > 28.0 { head_h * 0.22 } else { 0.0 };
        let num_h = head_h - low_h;

        // the big number + "FPS"
        let txt = view.fps.map_or("\u{2013}".to_string(), |f| format!("{:.0}", f));
        let lab_w_frac = 0.28;
        let vsz = self.fit_px(&txt, Weight::Bold, inner.w * (1.0 - lab_w_frac), num_h * 0.95);
        let nw = self.fonts.width(&txt, vsz, Weight::Bold);
        let base = avail_y + (num_h + vsz as f32 * 0.72) / 2.0;
        let ncol = if view.fps.is_some() { col } else { GREY };
        self.text(&txt, inner.x, base, vsz, Weight::Bold, ncol, 1.0);
        let fsz = ((vsz as f32 * 0.3).round() as u32).max(7);
        let fx = inner.x + nw + fsz as f32 * 0.4;
        if fx + self.fonts.width("FPS", fsz, Weight::Regular) <= inner.x + inner.w + 0.5 {
            self.text("FPS", fx, base, fsz, Weight::Regular, LAB, 0.9);
        }
        // 1 % low
        if low_h > 0.0 {
            if let Some(l) = view.low1 {
                let s = format!("1 % low {:.0}", l);
                let sz = self.fit_px(&s, Weight::Regular, inner.w, low_h * 0.85);
                self.text(&s, inner.x, avail_y + num_h + low_h * 0.8, sz, Weight::Regular, LAB, 0.9);
            }
        }
        // frame-time graph
        if p.graph && graph_h > 6.0 {
            let (px, pw) = (inner.x, inner.w);
            let py = avail_y + head_h + graph_h * 0.04;
            let ph = (graph_h * 0.96).max(4.0);
            let peak = view.hist.iter().map(|h| h.1).fold(0.0f32, f32::max);
            let max = nice_ceil((peak * 1.1).max(33.0));
            line(&mut self.layer, px, py + ph, px + pw, py + ph, [255, 255, 255, 55], 1.0, false);
            let ry = py + ph * (1.0 - 16.7 / max);
            line(&mut self.layer, px, ry, px + pw, ry, [255, 255, 255, 28], 1.0, false);
            self.gf_plot(&view.hist, px, py, pw, ph, secs, max, col);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::live::gamefps::{GameFpsProps, State, View};
    use crate::live::render::{Rect, Renderer};
    use crate::live::theme::{Theme, ThemeLive, Widget};
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    fn dump(name: &str, png: &[u8]) {
        if let Some(d) = std::env::var_os("COOLERSCREEN_B_DUMP") {
            let d = PathBuf::from(d);
            let _ = std::fs::create_dir_all(&d);
            std::fs::write(d.join(format!("{name}.png")), png).unwrap();
        }
    }

    fn theme(list: &[(f32, f32, f32, f32, serde_json::Value)]) -> ThemeLive {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        for (i, (x, y, w, h, p)) in list.iter().enumerate() {
            let mut props = p.clone();
            crate::live::morewidgets::clean("gamefps", &mut props);
            t.theme.widgets.push(Widget { id: format!("w{i}"), kind: "gamefps".into(), x: *x, y: *y, w: *w, h: *h, props, ..Widget::default() });
        }
        t
    }

    fn ok_view() -> View {
        let mut v = super::sample_view(30.0);
        v.game = "A very long game title that certainly will not fit into a small box".into();
        v.source = "hwinfo";
        v
    }

    #[test]
    fn preview_sample_draws_in_sizes_and_tiny() {
        let a = theme(&[
            (0.02, 0.03, 0.3, 0.36, json!({})),
            (0.35, 0.03, 0.3, 0.2, json!({ "graph": false })),
            (0.68, 0.03, 0.3, 0.5, json!({ "color": "#ffd070", "history": 60, "hideIdle": true })),
            (0.02, 0.55, 0.15, 0.2, json!({ "low": false })),
            (0.35, 0.6, 0.3, 0.3, json!({ "low": false, "graph": false })),
            (0.68, 0.6, 0.02, 0.02, json!({})),
            (0.8, 0.8, 0.005, 0.5, json!({})),
        ]);
        let png = crate::live::render_preview(&a, 1200, 540, true).unwrap();
        dump("gamefps_preview", &png);
        assert!(png.len() > 100);
        let png = crate::live::render_preview(&a, 320, 144, true).unwrap();
        dump("gamefps_preview_small", &png);
        let mut c = a.clone();
        c.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        crate::live::render_preview(&c, 1200, 540, true).unwrap();
    }

    #[test]
    fn every_state_draws() {
        let (cw, ch) = (1200usize, 540usize);
        let mut r = Renderer::new_sized(cw, ch, Arc::new(Mutex::new(Default::default())), None);
        r.layer.fill(tiny_skia::Color::from_rgba8(20, 22, 30, 255));
        let p = GameFpsProps::default();
        let hide = GameFpsProps { hide_idle: true, ..GameFpsProps::default() };
        let views = [
            ok_view(),
            View::idle(State::NoGame),
            View::idle(State::NeedsGroup),
            View::idle(State::Blocked),
            View::idle(State::Unavailable),
            View::idle(State::NoGame),
        ];
        for (i, v) in views.iter().enumerate() {
            let pp = if i == 5 { &hide } else { &p };
            let b = Rect { x: 20.0 + (i % 3) as f32 * 390.0, y: 20.0 + (i / 3) as f32 * 260.0, w: 360.0, h: 240.0 };
            r.draw_view(b, v, pp);
        }
        // tiny boxes in every state, no panic
        for v in &views {
            for (w, h) in [(6.0, 6.0), (20.0, 10.0), (60.0, 30.0), (3.0, 200.0)] {
                r.draw_view(Rect { x: 5.0, y: 5.0, w, h }, v, &p);
            }
        }
        // an Ok view with no numbers and no history
        let mut e = View::idle(State::Ok);
        e.source = "etw";
        r.draw_view(Rect { x: 700.0, y: 300.0, w: 300.0, h: 150.0 }, &e, &p);
        let png = r.layer.encode_png().unwrap();
        dump("gamefps_states", &png);
        // a hidden idle widget draws nothing at all
        let mut r2 = Renderer::new_sized(200, 100, Arc::new(Mutex::new(Default::default())), None);
        r2.draw_view(Rect { x: 0.0, y: 0.0, w: 200.0, h: 100.0 }, &View::idle(State::NoGame), &hide);
        assert!(r2.layer.data().iter().all(|&b| b == 0));
    }
}
