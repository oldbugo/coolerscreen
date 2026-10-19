//! Drawing of the `headlines` widget (4.0 W2): the latest titles of the feed the user pointed to. Each row is the title (as large as
//! fits, shortened with an ellipsis; two lines when the box is tall) and a small muted "Source name · 2 h ago" line. The row pitch
//! comes from the number of rows asked for, so nothing moves up and down while the list loads. In the editor preview fixed sample
//! headlines are drawn and the network is never touched.
use super::sensors::{line, GREY};
use super::{LocalTime, Rect, Renderer, Weight};
use crate::live::headlines::{self, Headline, HeadlinesView};
use crate::live::theme::{parse_color, Widget};

const AMBER: [u8; 3] = [240, 200, 70];

impl Renderer {
    /// At most two lines of `s` within `max` px: the first holds as many words as fit, the second is shortened with an ellipsis.
    fn headline_lines(&mut self, s: &str, px: u32, max: f32, lines: usize) -> Vec<String> {
        if lines < 2 || self.fonts.width(s, px, Weight::Regular) <= max {
            return vec![self.fit(s, px, Weight::Regular, max)];
        }
        let words: Vec<&str> = s.split(' ').collect();
        let mut n = 0;
        while n < words.len() && self.fonts.width(&words[..=n].join(" "), px, Weight::Regular) <= max {
            n += 1;
        }
        if n == 0 {
            // one word wider than the line: no point in wrapping
            return vec![self.fit(s, px, Weight::Regular, max)];
        }
        let rest = words[n..].join(" ");
        vec![words[..n].join(" "), self.fit(&rest, px, Weight::Regular, max)]
    }

    pub(super) fn draw_headlines(&mut self, b: Rect, w: &Widget, _now: &LocalTime) {
        let inner = self.mcard(b);
        if inner.h < 18.0 || inner.w < 40.0 {
            return; // too small to read anything: just the card
        }
        let p = w.headlines();
        let col = parse_color(&p.color, [255, 255, 255]);
        let count = p.count.clamp(3, 5) as usize;
        let now = headlines::now_utc();
        let has_source = !p.source.trim().is_empty();

        let view = if self.sample {
            let ago = |s: i64| Some(now - s);
            let h = |t: &str, ts: Option<i64>| Headline { title: t.into(), ts };
            HeadlinesView {
                loaded: true,
                error: None,
                title: Some("Daily Example".into()),
                entries: vec![
                    h("Council approves new cycle path along the river", ago(25 * 60)),
                    h("Local team wins the regional final in extra time", ago(3 * 3600)),
                    h("Heavy rain expected across the north this weekend", ago(7 * 3600)),
                    h("New library opens its doors to the public", ago(26 * 3600)),
                    h("Ferry timetable changes from next month", ago(2 * 86400 + 3600)),
                ],
            }
        } else if has_source {
            headlines::read(&p.source)
        } else {
            HeadlinesView { loaded: true, ..HeadlinesView::default() }
        };

        // a single message instead of rows
        if view.entries.is_empty() {
            let (msg, bad) = if !has_source && !self.sample {
                ("Add a feed link in Properties".to_string(), false)
            } else if let Some(e) = &view.error {
                (e.clone(), true)
            } else if !view.loaded {
                ("Loading\u{2026}".to_string(), false)
            } else {
                ("No headlines".to_string(), false)
            };
            let sz = self.fit_px(&msg, Weight::Regular, inner.w, inner.h * 0.2);
            let m = self.fit(&msg, sz, Weight::Regular, inner.w);
            self.text(&m, inner.x, inner.y + inner.h / 2.0 + sz as f32 * 0.3, sz, Weight::Regular, if bad { AMBER } else { GREY }, 1.0);
            return;
        }

        // an old list with a fresh error: the error takes a thin strip at the bottom
        let mut area = inner;
        if let Some(e) = &view.error {
            let epx = ((inner.h * 0.07).max(7.0).min(inner.w * 0.05)).round() as u32;
            let strip = epx as f32 * 1.5;
            if inner.h > strip * 3.0 {
                area.h -= strip;
                let m = self.fit(e, epx, Weight::Regular, inner.w);
                self.text(&m, inner.x, inner.y + inner.h - epx as f32 * 0.25, epx, Weight::Regular, AMBER, 1.0);
            }
        }

        let shown: Vec<&Headline> = view.entries.iter().take(count).collect();
        let row_h = area.h / count as f32;
        let meta_k = 0.62f32; // meta text size relative to the title
        let with_meta = p.show_source;
        // one title line plus the meta line must fit the row; the width caps the size so a tall box has room for a second line
        let per_px = 1.2 + if with_meta { meta_k * 1.2 + 0.2 } else { 0.0 };
        let mut tpx = ((row_h * 0.92 / per_px).min(inner.w * 0.075)).floor().max(6.0) as u32;
        // when some title does not fit on one line and the row has room, wrap to two lines (a slightly smaller size is fine)
        let per_px2 = 2.4 + if with_meta { meta_k * 1.2 + 0.2 } else { 0.0 };
        let tpx2 = ((row_h * 0.92 / per_px2).min(inner.w * 0.075)).floor().max(6.0) as u32;
        let need_wrap = shown.iter().any(|h| self.fonts.width(&h.title, tpx, Weight::Regular) > area.w);
        let two = need_wrap && tpx2 as f32 >= tpx as f32 * 0.72 && tpx2 >= 9;
        if two {
            tpx = tpx2;
        }
        let mpx = ((tpx as f32 * meta_k).round() as u32).max(6);
        let name = view.title.clone().unwrap_or_else(|| headlines::host_of(&p.source));
        let name = if name.is_empty() && self.sample { "Daily Example".to_string() } else { name };

        for (i, h) in shown.iter().enumerate() {
            let ry = area.y + row_h * i as f32;
            let lines = self.headline_lines(&h.title, tpx, area.w, if two { 2 } else { 1 });
            let meta = if with_meta {
                let s = match h.ts {
                    Some(ts) if !name.is_empty() => format!("{} \u{b7} {}", name, headlines::age_text(now, ts)),
                    Some(ts) => headlines::age_text(now, ts),
                    None => name.clone(),
                };
                (!s.is_empty()).then(|| self.fit(&s, mpx, Weight::Regular, area.w))
            } else {
                None
            };
            let lh = tpx as f32 * 1.2;
            let block = lh * lines.len() as f32 + meta.as_ref().map_or(0.0, |_| mpx as f32 * 1.2 + tpx as f32 * 0.1);
            let mut y = ry + (row_h - block).max(0.0) / 2.0;
            for l in &lines {
                self.text(l, area.x, y + tpx as f32 * 0.95, tpx, Weight::Regular, col, 1.0);
                y += lh;
            }
            if let Some(m) = meta {
                y += tpx as f32 * 0.1;
                self.text(&m, area.x, y + mpx as f32 * 0.95, mpx, Weight::Regular, col, 0.55);
            }
            if i + 1 < shown.len() && row_h > 12.0 {
                let ly = ry + row_h;
                line(&mut self.layer, area.x, ly, area.x + area.w, ly, [col[0], col[1], col[2], 28], 1.0, false);
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
            crate::live::morewidgets::clean("headlines", &mut props);
            t.theme.widgets.push(Widget { id: format!("w{i}"), kind: "headlines".into(), x: *x, y: *y, w: *w, h: *h, props, ..Widget::default() });
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
    fn headlines_draws_every_state() {
        let src = "https://example.com/feed.xml";
        let a = theme(&[
            (0.02, 0.03, 0.3, 0.3, json!({ "source": src, "count": 3 })),
            (0.35, 0.03, 0.3, 0.38, json!({ "source": src, "count": 4 })),
            (0.68, 0.03, 0.3, 0.45, json!({ "source": src, "count": 5, "color": "#ffd070" })),
            (0.02, 0.4, 0.3, 0.55, json!({ "source": src, "count": 3 })),
            (0.35, 0.45, 0.3, 0.25, json!({ "source": src, "count": 4, "showSource": false })),
            (0.68, 0.52, 0.3, 0.14, json!({})),
            (0.35, 0.75, 0.3, 0.02, json!({ "source": src })),
            (0.68, 0.72, 0.3, 0.2, json!({ "source": src, "count": 3 })),
        ]);
        // the preview uses sample headlines and never touches the network
        let png = crate::live::render_preview(&a, 1200, 540, true).unwrap();
        dump("headlines", &png);
        assert!(png.len() > 100);
        // live with no link / an invalid link: a message, no loader
        let mut b = theme(&[(0.02, 0.03, 0.3, 0.3, json!({})), (0.35, 0.03, 0.3, 0.3, json!({ "source": "file:///x" }))]);
        let png = crate::live::render_preview(&b, 1200, 540, false).unwrap();
        dump("headlines_nosource", &png);
        // junk props
        b.theme.widgets[0].props = json!("garbage");
        crate::live::render_preview(&b, 640, 288, false).unwrap();
        // tiny boxes
        let mut c = a.clone();
        c.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        let png = crate::live::render_preview(&c, 1200, 540, true).unwrap();
        dump("headlines_tiny", &png);
    }

    #[test]
    fn headlines_error_states_draw() {
        use crate::live::headlines::{Headline, HeadlinesView};
        // drive the drawing code with a finished view through the slot-less path: error text alone and error beside old titles
        let mk = |entries: Vec<Headline>, error: Option<&str>| HeadlinesView { entries, title: Some("Feed".into()), loaded: true, error: error.map(String::from) };
        let h = |t: &str| Headline { title: t.into(), ts: Some(crate::live::headlines::now_utc() - 7200) };
        let views = [mk(vec![], Some("The feed did not answer")), mk(vec![h("An old headline"), h("Another old one")], Some("Feed too large"))];
        for (i, v) in views.iter().enumerate() {
            crate::live::headlines::test_inject("https://example.com/err".to_string() + &i.to_string(), v.clone());
        }
        let a = theme(&[
            (0.02, 0.03, 0.3, 0.3, json!({ "source": "https://example.com/err0" })),
            (0.35, 0.03, 0.3, 0.4, json!({ "source": "https://example.com/err1" })),
            (0.68, 0.03, 0.3, 0.3, json!({})),
            (0.02, 0.45, 0.3, 0.3, json!({ "source": "https://example.com/err2" })),
        ]);
        crate::live::headlines::test_inject("https://example.com/err2".into(), HeadlinesView { loaded: false, ..HeadlinesView::default() });
        // the preview renderer is always in sample mode, so build a live one
        let mut r = crate::live::render::Renderer::new(std::sync::Arc::new(std::sync::Mutex::new(crate::live::sensors::Sensors::default())), None);
        r.set_theme(a);
        r.render();
        let png = r.frame_pixmap().encode_png().unwrap();
        dump("headlines_errors", &png);
    }
}
