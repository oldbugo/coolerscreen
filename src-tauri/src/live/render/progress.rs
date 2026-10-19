//! Drawing of the `progress` widget (4.0 W1): up to four rows (Day / Week / Month / Year), each a label on the left, the
//! percent as a big bold number on the right and a bar under it that grows sideways. Nothing moves up and down.
use super::{round_rect, LocalTime, Rect, Renderer, Weight};
use crate::live::progress::{self as pg, fractions, percent_text};
use crate::live::theme::{parse_color, Widget};

const LAB: [u8; 3] = [185, 192, 208];
const CYAN: [u8; 3] = [90, 200, 255];

/// The fixed instant the editor preview shows (Thursday 15 October 2026, 14:30), with a Monday week.
fn sample_time() -> LocalTime {
    LocalTime { year: 2026, month: 10, weekday: 4, day: 15, hour: 14, minute: 30, second: 0, millis: 0 }
}

impl Renderer {
    pub(super) fn draw_progress(&mut self, b: Rect, w: &Widget, now: &LocalTime) {
        let p = w.progress();
        let inner = self.mcard(b);
        let col = parse_color(&p.color, CYAN);
        let (t, start) = if self.sample { (sample_time(), 1) } else { (*now, pg::week_start_day(&p.week_start)) };
        let fr = fractions(&t, start);
        let n = p.items.len().clamp(1, 4);
        let row_h = inner.h / n as f32;
        for (i, item) in p.items.iter().take(n).enumerate() {
            let frac = fr.of(item);
            let txt = percent_text(frac, p.decimals);
            let y0 = inner.y + row_h * i as f32;
            let line_h = row_h * 0.62;
            // the widest the number can get ("100 %" / "99.99 %") keeps the size steady while it counts up
            let widest = if p.decimals == 0 { "100 %" } else { "99.99 %" };
            let vsz = self.fit_px(widest, Weight::Bold, inner.w * if p.labels { 0.55 } else { 0.9 }, line_h * 0.95);
            let lsz = ((vsz as f32) * 0.66).round().max(8.0) as u32;
            let base = y0 + line_h * 0.82;
            let vw = self.fonts.width(&txt, vsz, Weight::Bold);
            if p.labels {
                let label = self.fit(pg::label_of(item), lsz, Weight::Regular, (inner.w - vw - 12.0).max(20.0));
                self.text(&label, inner.x, base, lsz, Weight::Regular, LAB, 1.0);
            }
            self.text(&txt, inner.x + inner.w - vw, base, vsz, Weight::Bold, col, 1.0);
            let bh = (row_h * 0.17).clamp(4.0, 22.0);
            // same track and fill as the other bars (`track_and_fill` in more.rs is private to that file)
            let (by, f) = (y0 + row_h * 0.7, frac as f32);
            round_rect(&mut self.layer, inner.x, by, inner.w, bh, bh / 2.0, [255, 255, 255, 40]);
            if f > 0.005 {
                round_rect(&mut self.layer, inner.x, by, (inner.w * f).max(bh), bh, bh / 2.0, [col[0], col[1], col[2], 255]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::live::theme::{Theme, ThemeLive, Widget};
    use serde_json::json;
    use std::path::PathBuf;

    fn theme(list: &[(f32, f32, f32, f32, serde_json::Value)]) -> ThemeLive {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        for (i, (x, y, w, h, p)) in list.iter().enumerate() {
            let mut props = p.clone();
            crate::live::morewidgets::clean("progress", &mut props);
            t.theme.widgets.push(Widget { id: format!("w{i}"), kind: "progress".into(), x: *x, y: *y, w: *w, h: *h, props, ..Widget::default() });
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
    fn progress_draws_four_one_and_tiny() {
        let a = theme(&[
            (0.02, 0.04, 0.26, 0.34, json!({})),
            (0.32, 0.04, 0.26, 0.16, json!({ "items": ["year"], "color": "#ff8800", "decimals": 1 })),
            (0.62, 0.04, 0.26, 0.28, json!({ "items": ["week", "day"], "labels": false, "decimals": 2, "weekStart": "sunday" })),
            (0.32, 0.40, 0.26, 0.34, json!({ "items": ["month", "year", "day"], "color": "#5ad2a0" })),
        ]);
        let png = crate::live::render_preview(&a, 1200, 540, true).unwrap();
        dump("progress", &png);
        assert!(png.len() > 1000);
        // a tiny 2 % box must not panic or overflow
        let mut c = a.clone();
        c.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        dump("progress_tiny", &crate::live::render_preview(&c, 320, 144, true).unwrap());
        // garbage props fall back to the defaults
        let mut d = theme(&[(0.1, 0.1, 0.3, 0.3, json!({}))]);
        d.theme.widgets[0].props = json!("garbage");
        crate::live::render_preview(&d, 640, 288, true).unwrap();
    }
}
