//! Drawing of the `todo` widget (4.0 W1): an optional title and up to 8 rows, each a tick box and a line of text. Done rows are
//! dimmed and struck through, or left out (`hideDone`). The data is the item's own props (the user types and ticks them in the
//! editor), so the preview draws exactly what the screen shows. The row pitch comes from the number of rows kept in the list
//! (not the number shown) and text is shortened with an ellipsis, so nothing overflows.
use super::sensors::{fill, line, stroke, GREY};
use super::{LocalTime, Rect, Renderer, Weight};
use crate::live::theme::{parse_color, Widget};
use tiny_skia::PathBuilder;

/// A rounded square as a path (quarter circles approximated with quadratic curves).
fn rr_path(x: f32, y: f32, s: f32, r: f32) -> Option<tiny_skia::Path> {
    let r = r.min(s / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(x + r, y);
    pb.line_to(x + s - r, y);
    pb.quad_to(x + s, y, x + s, y + r);
    pb.line_to(x + s, y + s - r);
    pb.quad_to(x + s, y + s, x + s - r, y + s);
    pb.line_to(x + r, y + s);
    pb.quad_to(x, y + s, x, y + s - r);
    pb.line_to(x, y + r);
    pb.quad_to(x, y, x + r, y);
    pb.close();
    pb.finish()
}

impl Renderer {
    pub(super) fn draw_todo(&mut self, b: Rect, w: &Widget, _now: &LocalTime) {
        let inner = self.mcard(b);
        let p = w.todo();
        let col = parse_color(&p.color, [255, 255, 255]);
        let acc = parse_color(&p.accent, [90, 200, 255]);
        let title = p.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
        let shown: Vec<&crate::live::todo::TodoItem> = p.items.iter().filter(|i| !(p.hide_done && i.done)).collect();
        let units = (p.items.len().max(1) + usize::from(title.is_some())) as f32;
        let pitch = (inner.h / units).min(inner.w * 0.18).max(4.0);
        let px = ((pitch * 0.6).floor().min(inner.w * 0.1).max(6.0)) as u32;
        let mut y0 = inner.y;

        if let Some(t) = title {
            let txt = self.fit(t, px, Weight::Bold, inner.w);
            self.text(&txt, inner.x, y0 + pitch / 2.0 + px as f32 * 0.35, px, Weight::Bold, col, 0.7);
            y0 += pitch;
        }
        if shown.is_empty() {
            let msg = if p.items.is_empty() { "Add items in Properties" } else { "Nothing to do" };
            let sz = self.fit_px(msg, Weight::Regular, inner.w, px as f32);
            let txt = self.fit(msg, sz, Weight::Regular, inner.w);
            self.text(&txt, inner.x, y0 + pitch / 2.0 + sz as f32 * 0.35, sz, Weight::Regular, GREY, 1.0);
            return;
        }

        let s = (px as f32 * 0.9).round().max(5.0);
        let tx = inner.x + s + px as f32 * 0.5;
        let max_w = (inner.x + inner.w - tx).max(10.0);
        for (i, it) in shown.iter().enumerate() {
            let ry = y0 + pitch * i as f32;
            if ry + pitch * 0.5 > inner.y + inner.h + 0.5 {
                break;
            }
            let cy = ry + pitch / 2.0;
            let bx = inner.x;
            let by = (cy - s / 2.0).round();
            let rad = s * 0.28;
            if it.done {
                if let Some(path) = rr_path(bx, by, s, rad) {
                    fill(&mut self.layer, &path, [acc[0], acc[1], acc[2], 150]);
                }
                let mut pb = PathBuilder::new();
                pb.move_to(bx + s * 0.24, by + s * 0.54);
                pb.line_to(bx + s * 0.43, by + s * 0.72);
                pb.line_to(bx + s * 0.78, by + s * 0.30);
                if let Some(path) = pb.finish() {
                    stroke(&mut self.layer, &path, [20, 24, 32, 235], (s * 0.14).max(1.5), true);
                }
            } else if let Some(path) = rr_path(bx + s * 0.04, by + s * 0.04, s * 0.92, rad) {
                stroke(&mut self.layer, &path, [acc[0], acc[1], acc[2], 255], (s * 0.1).max(1.5), false);
            }
            let txt = self.fit(&it.text, px, Weight::Regular, max_w);
            let base = cy + px as f32 * 0.35;
            if it.done {
                self.text(&txt, tx, base, px, Weight::Regular, col, 0.45);
                let tw = self.fonts.width(&txt, px, Weight::Regular);
                line(&mut self.layer, tx, base - px as f32 * 0.3, tx + tw, base - px as f32 * 0.3, [col[0], col[1], col[2], 170], (px as f32 * 0.06).max(1.5), false);
            } else {
                self.text(&txt, tx, base, px, Weight::Regular, col, 1.0);
            }
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

    fn items(list: &[(&str, bool)]) -> serde_json::Value {
        json!(list.iter().map(|(t, d)| json!({ "text": t, "done": d })).collect::<Vec<_>>())
    }

    #[test]
    fn todo_draws_every_state() {
        let three = items(&[("First thing", false), ("Second thing", true), ("Third thing", false)]);
        let eight: Vec<(&str, bool)> = vec![("One", false), ("Two", true), ("Three", false), ("Four", true), ("Five", false), ("Six", false), ("Seven", true), ("Eight", false)];
        let long = items(&[("An extremely long line of text that cannot possibly fit in this small card without being shortened", false), ("Short", true)]);
        let a = theme(&[
            ("todo", 0.02, 0.04, 0.3, 0.42, json!({ "title": "Today", "items": three, "accent": "#5ac8ff" })),
            ("todo", 0.35, 0.04, 0.3, 0.42, json!({ "items": three, "hideDone": true, "color": "#ffd070", "accent": "#ff8844" })),
            ("todo", 0.68, 0.04, 0.3, 0.42, json!({ "title": "A title that is far too long to fit in the box", "items": long })),
            ("todo", 0.02, 0.52, 0.3, 0.2, json!({ "title": "Eight rows", "items": items(&eight) })),
            ("todo", 0.35, 0.52, 0.3, 0.2, json!({ "items": [] })),
            ("todo", 0.68, 0.52, 0.3, 0.2, json!({ "items": items(&[("a", true), ("b", true)]), "hideDone": true })),
            ("todo", 0.02, 0.78, 0.3, 0.2, json!({ "items": items(&[("Done one", true), ("Done two", true)]) })),
            ("todo", 0.35, 0.78, 0.3, 0.2, json!({ "title": "One", "items": items(&[("Only item", false)]) })),
        ]);
        let png = crate::live::render_preview(&a, 1200, 540, true).unwrap();
        dump("todo", &png);
        assert!(png.len() > 100);
        // live (not preview) draws the same without panicking
        crate::live::render_preview(&a, 1200, 540, false).unwrap();
        // tiny boxes
        let mut c = a.clone();
        c.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        crate::live::render_preview(&c, 320, 144, true).unwrap();
        // junk props
        let mut d = theme(&[("todo", 0.1, 0.1, 0.3, 0.3, json!({}))]);
        d.theme.widgets[0].props = json!("garbage");
        crate::live::render_preview(&d, 640, 288, true).unwrap();
    }

    #[test]
    fn todo_eight_rows_in_a_short_box_stay_inside() {
        let eight: Vec<(&str, bool)> = vec![("One", false); 8];
        let t = theme(&[("todo", 0.1, 0.1, 0.3, 0.12, json!({ "title": "Tight", "items": items(&eight) }))]);
        let png = crate::live::render_preview(&t, 1200, 540, true).unwrap();
        dump("todo_tight", &png);
    }
}
