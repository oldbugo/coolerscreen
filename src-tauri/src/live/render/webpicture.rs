//! Drawing of the `webpicture` widget (4.0 W2): one picture from a link, no card behind it (like the picture item). Whole picture
//! or filled box, corners rounded by the item's own radius when it has one, a small dim card with a short message while there is
//! no picture yet (nothing chosen / loading / failed). A refresh that fails keeps the old picture with a short amber note.
//! The editor preview (`self.sample`) draws a built-in landscape and never touches the network.
use super::widgets::scale_to_box;
use super::{LocalTime, Rect, Renderer, Weight};
use crate::live::theme::Widget;
use crate::live::webpicture::{self as wp, PicError, View, WebPictureProps};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tiny_skia::{Color, FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

const DIM: [u8; 3] = [185, 192, 208];
const AMBER: [u8; 3] = [255, 176, 64];

/// The scaled copy of the picture for one widget's box (so a frame does not re-scale it).
struct Scaled {
    pic_id: u64,
    bw: u32,
    bh: u32,
    cover: bool,
    pm: Arc<Pixmap>,
    ox: i32,
    oy: i32,
}

fn cache() -> &'static Mutex<HashMap<String, Scaled>> {
    static C: OnceLock<Mutex<HashMap<String, Scaled>>> = OnceLock::new();
    C.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A pleasant landscape made in code (sky gradient, sun, two ridges, a lake), 960 x 600.
fn sample_picture() -> Arc<Pixmap> {
    static S: OnceLock<Arc<Pixmap>> = OnceLock::new();
    S.get_or_init(|| {
        let (w, h) = (960u32, 600u32);
        let mut pm = Pixmap::new(w, h).unwrap();
        let lerp = |a: [f32; 3], b: [f32; 3], t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
        for y in 0..h {
            let t = y as f32 / (h as f32 * 0.7);
            let c = if t < 1.0 { lerp([40.0, 70.0, 150.0], [250.0, 170.0, 120.0], t.clamp(0.0, 1.0)) } else { lerp([40.0, 90.0, 130.0], [18.0, 40.0, 70.0], ((y as f32 / h as f32 - 0.7) / 0.3).clamp(0.0, 1.0)) };
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                pm.data_mut()[i..i + 4].copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
            }
        }
        let fill = |pm: &mut Pixmap, path: &tiny_skia::Path, rgba: [u8; 4]| {
            let mut p = Paint::default();
            p.set_color(Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]));
            p.anti_alias = true;
            pm.fill_path(path, &p, FillRule::Winding, Transform::identity(), None);
        };
        let sun = PathBuilder::from_circle(660.0, 300.0, 70.0).unwrap();
        fill(&mut pm, &sun, [255, 232, 170, 255]);
        let ridge = |pts: &[(f32, f32)], base: f32| {
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, base);
            for (x, y) in pts {
                pb.line_to(*x, *y);
            }
            pb.line_to(960.0, base);
            pb.close();
            pb.finish().unwrap()
        };
        fill(&mut pm, &ridge(&[(0.0, 330.0), (150.0, 250.0), (300.0, 320.0), (470.0, 210.0), (640.0, 330.0), (800.0, 270.0), (960.0, 340.0)], 420.0), [96, 94, 140, 255]);
        fill(&mut pm, &ridge(&[(0.0, 390.0), (200.0, 330.0), (380.0, 400.0), (560.0, 340.0), (760.0, 410.0), (960.0, 360.0)], 420.0), [46, 58, 96, 255]);
        // the lake: a lighter band with the sun's reflection
        let lake = {
            let mut pb = PathBuilder::new();
            pb.push_rect(tiny_skia::Rect::from_xywh(0.0, 420.0, 960.0, 180.0).unwrap());
            pb.finish().unwrap()
        };
        fill(&mut pm, &lake, [30, 62, 104, 255]);
        for i in 0..7 {
            let wdt = 120.0 - i as f32 * 14.0;
            let r = tiny_skia::Rect::from_xywh(660.0 - wdt / 2.0, 440.0 + i as f32 * 22.0, wdt, 6.0).unwrap();
            let mut pb = PathBuilder::new();
            pb.push_rect(r);
            fill(&mut pm, &pb.finish().unwrap(), [255, 220, 160, 150 - i * 15]);
        }
        Arc::new(pm)
    })
    .clone()
}

impl Renderer {
    pub(super) fn draw_webpicture(&mut self, b: Rect, w: &Widget, _now: &LocalTime) {
        let p = w.webpicture();
        let valid = wp::valid_source(&p.source);
        let view = if self.sample {
            View { pic: Some(sample_picture()), id: 1, error: None, loaded: true }
        } else if let Some(link) = &valid {
            wp::read(link, p.minutes)
        } else {
            View::default()
        };
        self.draw_wp_view(b, w, &p, valid.is_some(), &view);
    }

    /// Draw one state of the widget (split from `draw_webpicture` so it can be shown without the network).
    pub(super) fn draw_wp_view(&mut self, b: Rect, w: &Widget, p: &WebPictureProps, has_link: bool, view: &View) {
        let Some(pic) = view.pic.clone() else {
            let (msg, amber) = if !has_link {
                ("Add a picture link in Properties", false)
            } else if let Some(e) = view.error {
                (e.message(), true)
            } else {
                ("Loading…", false)
            };
            self.wp_card(b, msg, amber);
            return;
        };
        let cover = p.fit == "cover";
        let (bw, bh) = (b.w.round().max(1.0) as u32, b.h.round().max(1.0) as u32);
        let id = if w.id.is_empty() { "?".to_string() } else { w.id.clone() };
        let (pm, ox, oy) = {
            let mut g = cache().lock().unwrap();
            let fresh = g.get(&id).is_some_and(|s| s.pic_id == view.id && s.bw == bw && s.bh == bh && s.cover == cover);
            if !fresh {
                let (_, _, _, pm, ox, oy) = scale_to_box(&pic, bw, bh, cover);
                g.insert(id.clone(), Scaled { pic_id: view.id, bw, bh, cover, pm: Arc::new(pm), ox, oy });
            }
            let s = &g[&id];
            (s.pm.clone(), s.ox, s.oy)
        };
        let paint = PixmapPaint { opacity: p.opacity.clamp(0.0, 1.0), ..PixmapPaint::default() };
        let (x, y) = (b.x as i32 + ox, b.y as i32 + oy);
        self.layer.draw_pixmap(x, y, (*pm).as_ref(), &paint, Transform::identity(), None);
        if let Some(px) = w.radius {
            // round the picture itself (the box when it is filled)
            let r = Rect { x: x as f32, y: y as f32, w: pm.width() as f32, h: pm.height() as f32 };
            self.clip_rounded(r, px * self.cw as f32 / 2400.0);
        }
        if view.error.is_some() && b.h >= 40.0 {
            let px = (b.h * 0.07).clamp(10.0, 26.0) as u32;
            self.text("(old picture)", b.x + 8.0 + ox.max(0) as f32, b.y + b.h - 8.0 - oy.max(0) as f32, px, Weight::Regular, AMBER, 0.9);
        }
    }

    /// A small dim card with one short line, centred.
    fn wp_card(&mut self, b: Rect, msg: &str, amber: bool) {
        let inner = self.mcard(b);
        if b.w < 40.0 || b.h < 24.0 {
            return;
        }
        let px = self.fit_px(msg, Weight::Regular, inner.w, (inner.h * 0.4).min(32.0 * self.cw as f32 / 1680.0));
        let width = self.fonts.width(msg, px, Weight::Regular);
        let col = if amber { AMBER } else { DIM };
        self.text(msg, inner.x + (inner.w - width).max(0.0) / 2.0, inner.y + inner.h / 2.0 + px as f32 * 0.35, px, Weight::Regular, col, 1.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::theme::{Theme, ThemeLive};
    use serde_json::json;
    use std::path::PathBuf;

    fn theme(list: &[(f32, f32, f32, f32, Option<f32>, serde_json::Value)]) -> ThemeLive {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        for (i, (x, y, w, h, radius, p)) in list.iter().enumerate() {
            let mut props = p.clone();
            crate::live::morewidgets::clean("webpicture", &mut props);
            t.theme.widgets.push(Widget { id: format!("wp{i}"), kind: "webpicture".into(), x: *x, y: *y, w: *w, h: *h, radius: *radius, props, ..Widget::default() });
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

    fn px(png: &[u8], x: u32, y: u32) -> [u8; 4] {
        let c = Pixmap::decode_png(png).unwrap().pixel(x, y).unwrap();
        [c.red(), c.green(), c.blue(), c.alpha()]
    }

    #[test]
    fn webpicture_sample_contain_cover_radius_and_tiny() {
        // a wide box and a tall box, whole picture and filled, the last with a corner radius
        let t = theme(&[
            (0.02, 0.04, 0.46, 0.30, None, json!({ "fit": "contain" })),
            (0.52, 0.04, 0.46, 0.30, None, json!({ "fit": "cover" })),
            (0.02, 0.42, 0.14, 0.54, None, json!({ "fit": "contain" })),
            (0.20, 0.42, 0.14, 0.54, Some(120.0), json!({ "fit": "cover" })),
            (0.40, 0.42, 0.26, 0.40, Some(160.0), json!({ "fit": "contain", "opacity": 0.6 })),
        ]);
        let png = crate::live::render_preview(&t, 1680, 756, true).unwrap();
        dump("webpicture_sample", &png);
        assert!(png.len() > 1000);
        // radius: the corner of the rounded, filled tall box shows the background, the middle shows the picture
        let sq = theme(&[(0.20, 0.42, 0.14, 0.54, None, json!({ "fit": "cover" }))]);
        let rd = theme(&[(0.20, 0.42, 0.14, 0.54, Some(300.0), json!({ "fit": "cover" }))]);
        let (a, b) = (crate::live::render_preview(&sq, 480, 216, true).unwrap(), crate::live::render_preview(&rd, 480, 216, true).unwrap());
        assert_ne!(px(&a, 97, 91), px(&b, 97, 91), "a big radius rounds the picture's corner");
        assert_eq!(px(&a, 110, 150), px(&b, 110, 150), "the middle is the same");
        // a tiny 2 % box must not panic
        let mut tiny = t.clone();
        tiny.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        let png = crate::live::render_preview(&tiny, 600, 270, true).unwrap();
        dump("webpicture_tiny", &png);
        assert!(png.len() > 500);
    }

    #[test]
    fn webpicture_states_without_the_network() {
        let mut r = Renderer::new(Arc::new(Mutex::new(crate::live::sensors::Sensors::default())), None);
        let pr = WebPictureProps::default();
        let w = Widget { id: "wpx".into(), kind: "webpicture".into(), ..Widget::default() };
        let cases: [(&str, bool, View); 6] = [
            ("nolink", false, View::default()),
            ("loading", true, View::default()),
            ("noanswer", true, View { error: Some(PicError::Unreachable), loaded: true, ..View::default() }),
            ("notpic", true, View { error: Some(PicError::NotPicture), loaded: true, ..View::default() }),
            ("toolarge", true, View { error: Some(PicError::TooLarge), loaded: true, ..View::default() }),
            ("oldpic", true, View { pic: Some(sample_picture()), id: 1, error: Some(PicError::Unreachable), loaded: true }),
        ];
        let mut out = Pixmap::new(1680, 756).unwrap();
        for (i, (_, has, v)) in cases.iter().enumerate() {
            let b = Rect { x: 20.0 + (i % 3) as f32 * 550.0, y: 20.0 + (i / 3) as f32 * 360.0, w: 500.0, h: 300.0 };
            r.draw_wp_view(b, &w, &pr, *has, v);
        }
        // a narrow box and a tiny box too
        r.draw_wp_view(Rect { x: 20.0, y: 700.0, w: 120.0, h: 50.0 }, &w, &pr, false, &View::default());
        r.draw_wp_view(Rect { x: 200.0, y: 700.0, w: 6.0, h: 6.0 }, &w, &pr, false, &View::default());
        out.draw_pixmap(0, 0, r.layer.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
        let png = out.encode_png().unwrap();
        dump("webpicture_states", &png);
        assert!(r.layer.data().iter().any(|v| *v != 0));
    }
}
