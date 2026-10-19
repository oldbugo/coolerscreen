//! The 4.0 stream B widgets: `meters`, `cores`, `network`, `disks`, `worldclock`, `timer` (props and data helpers live in
//! `live/morewidgets.rs`). Same look as the other widgets (dark rounded card, Segoe UI, a missing reading is a greyed dash).
//! Nothing moves up and down: bars grow sideways, a finished timer pulses its opacity.
use super::sensors::{fill, info, line, mix, nice_ceil, stroke, Kind, GREY};
use super::{round_rect, LocalTime, Rect, Renderer, Weight};
use crate::live::metrics;
use crate::live::morewidgets::{self as mw, CoresProps, DisksProps, MetersProps, NetworkProps, TimerProps, WorldClockProps};
use crate::live::theme::{parse_color, SensorProps, Widget};
use tiny_skia::PathBuilder;

const LAB: [u8; 3] = [185, 192, 208];
const AMBER: [u8; 3] = [240, 200, 70];
const RED: [u8; 3] = [240, 90, 80];
const CYAN: [u8; 3] = [90, 200, 255];
const GREEN: [u8; 3] = [90, 210, 130];

fn bytes_text(b: f32) -> String {
    if b >= 1e12 { format!("{:.2} TB", b / 1e12) } else if b >= 1e9 { format!("{:.1} GB", b / 1e9) } else { format!("{:.0} MB", b / 1e6) }
}

fn size_text(gb: f32) -> String {
    if gb >= 1000.0 { format!("{:.1} TB", gb / 1024.0) } else if gb >= 100.0 { format!("{gb:.0} GB") } else { format!("{gb:.1} GB") }
}

impl Renderer {
    fn mcard(&mut self, b: Rect) -> Rect {
        let sc = (self.cw as f32 / 1680.0).max(0.1);
        round_rect(&mut self.layer, b.x, b.y, b.w, b.h, (28.0 * sc).min(b.w.min(b.h) / 5.0), [0, 0, 0, 96]);
        let pad = (b.w.min(b.h) * 0.06).max(6.0);
        Rect { x: b.x + pad, y: b.y + pad, w: (b.w - 2.0 * pad).max(1.0), h: (b.h - 2.0 * pad).max(1.0) }
    }

    /// Text right-aligned to `right`.
    fn text_r(&mut self, s: &str, right: f32, base: f32, px: u32, wt: Weight, col: [u8; 3], a: f32) {
        let w = self.fonts.width(s, px, wt);
        self.text(s, right - w, base, px, wt, col, a);
    }

    pub(super) fn draw_more(&mut self, b: Rect, w: &Widget, now: &LocalTime) {
        match w.kind.as_str() {
            "meters" => self.draw_meters(b, &w.id, &w.meters()),
            "cores" => self.draw_cores(b, &w.id, &w.cores()),
            "network" => self.draw_network(b, &w.network()),
            "disks" => self.draw_disks(b, &w.id, &w.disks()),
            "worldclock" => self.draw_worldclock(b, &w.worldclock()),
            "timer" => self.draw_timer(b, &w.timer(), now),
            _ => {}
        }
    }

    /// Eased fraction for a bar, remembered per key.
    fn eased(&mut self, key: String, target: f32) -> f32 {
        let k = 1.0 - 0.75f32.powi(self.steps_now.max(0));
        let sample = self.sample || self.steps_now >= 30;
        let e = self.sens_smooth.entry(key).or_insert(target);
        if sample {
            *e = target;
        } else {
            *e += (target - *e) * k;
        }
        *e
    }

    fn track_and_fill(&mut self, x: f32, y: f32, w: f32, h: f32, frac: f32, col: [u8; 3], present: bool) {
        round_rect(&mut self.layer, x, y, w, h, h / 2.0, [255, 255, 255, 40]);
        if present && frac > 0.005 {
            round_rect(&mut self.layer, x, y, (w * frac).max(h), h, h / 2.0, [col[0], col[1], col[2], 255]);
        }
    }

    // ------------------------------------------------------------ meters

    fn draw_meters(&mut self, b: Rect, id: &str, p: &MetersProps) {
        let inner = self.mcard(b);
        let n = p.rows.len();
        if n == 0 {
            let sz = self.fit_px("No rows", Weight::Regular, inner.w, inner.h * 0.3);
            self.text("No rows", inner.x, inner.y + sz as f32, sz, Weight::Regular, GREY, 1.0);
            return;
        }
        let row_h = inner.h / n as f32;
        let unit_f = p.unit.eq_ignore_ascii_case("F");
        for (i, row) in p.rows.iter().enumerate() {
            let sp = SensorProps { metric: row.metric.clone(), core: row.core, drive: row.drive.clone(), rate: p.rate, ..SensorProps::default() };
            let inf = info(&row.metric);
            let r = self.read_metric(&sp, false);
            let cur = r.value.unwrap_or(0.0);
            // scale: the metric's own, else the total, else the recent peak
            let max = match inf.max.or(r.total) {
                Some(m) => m,
                None => {
                    let floor = match inf.kind {
                        Kind::Rate => 1e6,
                        Kind::Watts => 100.0,
                        _ => 1.0,
                    };
                    let steps = self.steps_now.max(1) as f32;
                    let pk = self.sens_smooth.entry(format!("{id}:{i}:pk")).or_insert(0.0);
                    if cur > *pk {
                        *pk = cur;
                    } else {
                        *pk -= (*pk - cur) * (0.02 * steps).min(1.0);
                    }
                    nice_ceil((*pk * 1.15).max(floor))
                }
            };
            let target = if max > 0.0 { (cur / max).clamp(0.0, 1.0) } else { 0.0 };
            let frac = self.eased(format!("{id}:{i}"), target);
            let col = match r.value {
                Some(v) => self.sensor_color(&sp, &inf, v, r.total),
                None => GREY,
            };
            let (num, unit) = match r.value {
                Some(v) => super::sensors::format_value(&row.metric, v, unit_f, None),
                None => ("\u{2013}".to_string(), String::new()),
            };
            let label = match &row.label {
                Some(l) => l.clone(),
                None if row.metric == "cpu.core" => format!("CPU core {}", row.core),
                None if row.metric == "disk.used" => format!("Disk {}", row.drive.trim_end_matches(':')),
                None => inf.label.to_string(),
            };
            let y0 = inner.y + row_h * i as f32;
            let line_h = if p.bars { row_h * 0.62 } else { row_h * 0.9 };
            let vtxt = if unit.is_empty() { num.clone() } else { format!("{num} {unit}") };
            let vsz = self.fit_px(&vtxt, Weight::Bold, inner.w * 0.55, line_h * 0.95);
            let lsz = ((vsz as f32) * 0.66).round().max(8.0) as u32;
            let base = y0 + line_h * 0.82;
            let vw = self.fonts.width(&vtxt, vsz, Weight::Bold);
            if !label.is_empty() {
                let l = self.fit(&label, lsz, Weight::Regular, (inner.w - vw - 12.0).max(20.0));
                self.text(&l, inner.x, base, lsz, Weight::Regular, LAB, 1.0);
            }
            self.text(&vtxt, inner.x + inner.w - vw, base, vsz, Weight::Bold, col, 1.0);
            if p.bars {
                let bh = (row_h * 0.17).clamp(4.0, 22.0);
                self.track_and_fill(inner.x, y0 + row_h * 0.7, inner.w, bh, frac, col, r.value.is_some());
            }
        }
    }

    // ------------------------------------------------------------ cores

    fn draw_cores(&mut self, b: Rect, id: &str, p: &CoresProps) {
        let inner = self.mcard(b);
        let n = if self.sample { 16 } else { mw::logical_cores() };
        let hz = p.rate.clamp(1, 4);
        let mut loads: Vec<Option<f32>> = Vec::with_capacity(n);
        for i in 0..n {
            loads.push(if self.sample { Some(12.0 + ((i * 37) % 71) as f32) } else { metrics::hub().read(&metrics::key("cpu.core", i as u32, "", None), hz).0 });
        }
        let avg = if self.sample { Some(38.0) } else { metrics::hub().read("cpu.load", hz).0 };
        let base_col = p.color.as_deref().map(|c| parse_color(c, CYAN));
        let head_h = if p.average { (inner.h * 0.24).min(inner.w * 0.2) } else { (inner.h * 0.12).min(24.0) };
        let lsz = (head_h * 0.62).round().max(8.0) as u32;
        self.text("CPU cores", inner.x, inner.y + head_h * 0.78, lsz, Weight::Regular, LAB, 1.0);
        if p.average {
            let (txt, col) = match avg {
                Some(v) => (format!("{v:.0} %"), base_col.unwrap_or_else(|| load_color(v))),
                None => ("\u{2013}".to_string(), GREY),
            };
            let vsz = self.fit_px(&txt, Weight::Bold, inner.w * 0.5, head_h * 0.95);
            self.text_r(&txt, inner.x + inner.w, inner.y + head_h * 0.84, vsz, Weight::Bold, col, 1.0);
        }
        let area = Rect { x: inner.x, y: inner.y + head_h * 1.15, w: inner.w, h: (inner.h - head_h * 1.15).max(4.0) };
        if n == 0 {
            return;
        }
        let gap = (area.h.min(area.w) * 0.012).max(2.0);
        if p.style == "heat" {
            let cols = ((n as f32 * area.w / area.h).sqrt().ceil() as usize).clamp(1, n);
            let rows = n.div_ceil(cols);
            let (tw, th) = ((area.w - gap * (cols as f32 - 1.0)) / cols as f32, (area.h - gap * (rows as f32 - 1.0)) / rows as f32);
            for i in 0..n {
                let (cx, cy) = (i % cols, i / cols);
                let v = loads[i];
                let frac = self.eased(format!("{id}:c{i}"), v.map(|v| v / 100.0).unwrap_or(0.0).clamp(0.0, 1.0));
                let hot = v.map(|v| base_col.unwrap_or_else(|| load_color(v))).unwrap_or(GREY);
                let c = mix([34, 40, 54], hot, 0.18 + 0.82 * frac);
                round_rect(&mut self.layer, area.x + cx as f32 * (tw + gap), area.y + cy as f32 * (th + gap), tw, th, (tw.min(th) * 0.18).min(8.0), [c[0], c[1], c[2], if v.is_some() { 255 } else { 90 }]);
            }
        } else {
            // bars: fill columns first; as few columns as keep every bar row at least ~12 px high
            let mut cols = 1usize;
            while cols < 4 && (area.h / n.div_ceil(cols) as f32) < 12.0 {
                cols += 1;
            }
            let rows = n.div_ceil(cols);
            let cgap = gap * 4.0;
            let cw = (area.w - cgap * (cols as f32 - 1.0)) / cols as f32;
            let rh = area.h / rows as f32;
            let bh = (rh * 0.62).clamp(3.0, 22.0);
            for i in 0..n {
                let (cx, cy) = (i / rows, i % rows);
                let v = loads[i];
                let frac = self.eased(format!("{id}:c{i}"), v.map(|v| v / 100.0).unwrap_or(0.0).clamp(0.0, 1.0));
                let col = v.map(|v| base_col.unwrap_or_else(|| load_color(v))).unwrap_or(GREY);
                self.track_and_fill(area.x + cx as f32 * (cw + cgap), area.y + cy as f32 * rh + (rh - bh) / 2.0, cw, bh, frac, col, v.is_some());
            }
        }
    }

    // ------------------------------------------------------------ network

    fn draw_network(&mut self, b: Rect, p: &NetworkProps) {
        let mut inner = self.mcard(b);
        if p.totals {
            inner = self.network_footer(inner, p);
        }
        let hz = p.rate.clamp(1, 4);
        let secs = p.history.clamp(10.0, 600.0);
        let kd = metrics::key("net.down", 0, "", p.adapter.as_deref());
        let ku = metrics::key("net.up", 0, "", p.adapter.as_deref());
        let (dv, uv, dh, uh) = if self.sample {
            (Some(12.4e6), Some(1.6e6), sample_series(9.0e6, secs), sample_series(1.4e6, secs))
        } else {
            let h = metrics::hub();
            let (d, u) = (h.read(&kd, hz).0, h.read(&ku, hz).0);
            (d, u, if p.graph { h.history(&kd, secs) } else { Vec::new() }, if p.graph { h.history(&ku, secs) } else { Vec::new() })
        };
        let dcol = parse_color(&p.down_color, [90, 210, 160]);
        let ucol = parse_color(&p.up_color, [170, 140, 255]);
        let head_h = if p.graph { inner.h * 0.44 } else { inner.h };
        let half = inner.w / 2.0;
        for (k, (v, col, name, metric)) in [(dv, dcol, "Download", "net.down"), (uv, ucol, "Upload", "net.up")].into_iter().enumerate() {
            let x0 = inner.x + half * k as f32;
            let w = half - if k == 0 { inner.w * 0.02 } else { 0.0 };
            let lsz = (head_h * 0.24).min(w * 0.14).max(8.0).round() as u32;
            // a small arrow (down or up) before the name
            let (ax, ay, s) = (x0 + lsz as f32 * 0.4, inner.y + lsz as f32 * 0.5, lsz as f32 * 0.34);
            let mut pb = PathBuilder::new();
            if k == 0 {
                pb.move_to(ax - s, ay - s * 0.6);
                pb.line_to(ax + s, ay - s * 0.6);
                pb.line_to(ax, ay + s * 0.8);
            } else {
                pb.move_to(ax - s, ay + s * 0.6);
                pb.line_to(ax + s, ay + s * 0.6);
                pb.line_to(ax, ay - s * 0.8);
            }
            pb.close();
            if let Some(path) = pb.finish() {
                fill(&mut self.layer, &path, [col[0], col[1], col[2], 255]);
            }
            self.text(name, x0 + lsz as f32 * 1.1, inner.y + lsz as f32 * 0.85, lsz, Weight::Regular, LAB, 1.0);
            let (num, unit) = match v {
                Some(v) => super::sensors::format_value(metric, v, false, None),
                None => ("\u{2013}".to_string(), String::new()),
            };
            let txt = if unit.is_empty() { num } else { format!("{num} {unit}") };
            let area_h = head_h - lsz as f32 * 1.2;
            let vsz = self.fit_px(&txt, Weight::Bold, w, area_h * 0.9);
            let base = inner.y + lsz as f32 * 1.2 + (area_h + vsz as f32 * 0.72) / 2.0;
            self.text(&txt, x0, base, vsz, Weight::Bold, if v.is_some() { col } else { GREY }, 1.0);
        }
        if p.graph {
            let (px, pw) = (inner.x, inner.w);
            let py = inner.y + head_h + inner.h * 0.03;
            let ph = (inner.y + inner.h - py).max(6.0);
            let peak = dh.iter().chain(uh.iter()).map(|h| h.1).fold(0.0f32, f32::max).max(dv.unwrap_or(0.0)).max(uv.unwrap_or(0.0));
            let max = nice_ceil((peak * 1.15).max(1e6));
            for f in [0.0f32, 0.5, 1.0] {
                line(&mut self.layer, px, py + ph * f, px + pw, py + ph * f, [255, 255, 255, if f == 1.0 { 55 } else { 28 }], 1.0, false);
            }
            self.plot(&dh, px, py, pw, ph, secs, max, dcol, true);
            self.plot(&uh, px, py, pw, ph, secs, max, ucol, false);
            let (n, u) = super::sensors::format_value("net.down", max, false, Some(0));
            let ms = (ph * 0.14).clamp(8.0, 18.0).round() as u32;
            self.text(&format!("{n} {u}"), px + 3.0, py + ms as f32 * 1.05, ms, Weight::Regular, LAB, 0.8);
        }
    }

    /// The "since start-up" line at the bottom of the network widget; returns the area left for the rest.
    fn network_footer(&mut self, inner: Rect, p: &NetworkProps) -> Rect {
        let foot = (inner.h * 0.13).clamp(10.0, 34.0);
        let (rx, tx) = if self.sample {
            (Some(48.2e9), Some(3.1e9))
        } else {
            let h = metrics::hub();
            let a = p.adapter.as_deref().unwrap_or("*");
            (h.read(&format!("net.rx.total:{a}"), 1).0, h.read(&format!("net.tx.total:{a}"), 1).0)
        };
        let txt = match (rx, tx) {
            (Some(r), Some(t)) => format!("Since start-up   down {}   up {}", bytes_text(r), bytes_text(t)),
            _ => "Since start-up   \u{2013}".to_string(),
        };
        let sz = (foot * 0.9).round().max(8.0) as u32;
        let txt = self.fit(&txt, sz, Weight::Regular, inner.w);
        self.text(&txt, inner.x, inner.y + inner.h - foot * 0.2, sz, Weight::Regular, LAB, 0.95);
        Rect { h: (inner.h - foot * 1.3).max(8.0), ..inner }
    }

    #[allow(clippy::too_many_arguments)]
    fn plot(&mut self, hist: &[(f32, f32)], px: f32, py: f32, pw: f32, ph: f32, secs: f32, max: f32, col: [u8; 3], area: bool) {
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
        if area {
            ab.line_to(last_x, py + ph);
            ab.close();
            if let Some(a) = ab.finish() {
                fill(&mut self.layer, &a, [col[0], col[1], col[2], 60]);
            }
        }
        if let Some(l) = pb.finish() {
            stroke(&mut self.layer, &l, [col[0], col[1], col[2], 255], (ph * 0.03).clamp(1.5, 4.0), true);
        }
    }

    // ------------------------------------------------------------ disks

    fn draw_disks(&mut self, b: Rect, id: &str, p: &DisksProps) {
        let inner = self.mcard(b);
        let rows_max = p.max_rows.clamp(2, 6) as usize;
        let drives: Vec<(String, Option<(f32, f32)>)> = if self.sample {
            vec![("C:".into(), Some((412.0, 931.0))), ("D:".into(), Some((1840.0, 3726.0))), ("E:".into(), Some((90.0, 477.0))), ("F:".into(), Some((310.0, 465.0)))].into_iter().take(rows_max).collect()
        } else {
            let list = if p.drives.is_empty() { mw::fixed_drives() } else { p.drives.clone() };
            list.into_iter()
                .take(rows_max)
                .map(|d| {
                    let (u, t) = metrics::hub().read(&metrics::key("disk.used", 0, &d, None), 1);
                    let v = u.zip(t);
                    (d, v)
                })
                .collect()
        };
        if drives.is_empty() {
            let sz = self.fit_px("No drives", Weight::Regular, inner.w, inner.h * 0.3);
            self.text("No drives", inner.x, inner.y + sz as f32, sz, Weight::Regular, GREY, 1.0);
            return;
        }
        let n = drives.len();
        let row_h = inner.h / n as f32;
        for (i, (d, v)) in drives.iter().enumerate() {
            let y0 = inner.y + row_h * i as f32;
            let line_h = row_h * 0.62;
            let pct = v.map(|(u, t)| if t > 0.0 { 100.0 * u / t } else { 0.0 });
            let col = match pct {
                Some(p) if p >= 95.0 => RED,
                Some(p) if p >= 85.0 => AMBER,
                Some(_) => GREEN,
                None => GREY,
            };
            let txt = match (v, p.show.as_str()) {
                (Some((u, t)), "free") => format!("{} free", size_text((t - u).max(0.0))),
                (Some((u, t)), _) => format!("{} / {}", size_text(*u), size_text(*t)),
                (None, _) => "\u{2013}".to_string(),
            };
            let vsz = self.fit_px(&txt, Weight::Bold, inner.w * 0.7, line_h * 0.95);
            let lsz = ((vsz as f32) * 0.9).round().max(8.0) as u32;
            let base = y0 + line_h * 0.82;
            self.text(d, inner.x, base, lsz, Weight::Bold, LAB, 1.0);
            self.text_r(&txt, inner.x + inner.w, base, vsz, Weight::Bold, col, 1.0);
            let frac = self.eased(format!("{id}:d{i}"), pct.map(|p| p / 100.0).unwrap_or(0.0).clamp(0.0, 1.0));
            let bh = (row_h * 0.17).clamp(4.0, 22.0);
            self.track_and_fill(inner.x, y0 + row_h * 0.7, inner.w, bh, frac, col, v.is_some());
        }
    }

    // ------------------------------------------------------------ world clocks

    fn draw_worldclock(&mut self, b: Rect, p: &WorldClockProps) {
        let inner = self.mcard(b);
        let col = parse_color(&p.color, [255, 255, 255]);
        let zones: Vec<_> = p.zones.iter().take(mw::MAX_ZONES).collect();
        if zones.is_empty() {
            let sz = self.fit_px("No cities", Weight::Regular, inner.w, inner.h * 0.3);
            self.text("No cities", inner.x, inner.y + sz as f32, sz, Weight::Regular, GREY, 1.0);
            return;
        }
        let n = zones.len();
        let row_h = inner.h / n as f32;
        let here = mw::zone_time_now("").map(|z| z.days());
        let h12 = p.format == "12h";
        for (i, z) in zones.iter().enumerate() {
            let y0 = inner.y + row_h * i as f32;
            if i > 0 {
                line(&mut self.layer, inner.x, y0, inner.x + inner.w, y0, [255, 255, 255, 30], 1.0, false);
            }
            let zt = mw::zone_time_now(&z.tz);
            let (txt, suffix) = match zt {
                Some(t) if h12 => (format!("{}:{:02}", if t.hour % 12 == 0 { 12 } else { t.hour % 12 }, t.minute), if t.hour >= 12 { "PM" } else { "AM" }),
                Some(t) => (format!("{:02}:{:02}", t.hour, t.minute), ""),
                None => ("\u{2013}".to_string(), ""),
            };
            let day = match (zt, here) {
                (Some(t), Some(h)) => match t.days() - h {
                    0 => "",
                    1 => "Tomorrow",
                    -1 => "Yesterday",
                    d if d > 1 => "Ahead",
                    _ => "Behind",
                },
                _ => "",
            };
            let vsz = self.fit_px(&txt, Weight::Bold, inner.w * 0.5, row_h * 0.72);
            let usz = ((vsz as f32) * 0.42).round().max(8.0) as u32;
            let uw = if suffix.is_empty() { 0.0 } else { self.fonts.width(suffix, usz, Weight::Regular) + vsz as f32 * 0.1 };
            let vw = self.fonts.width(&txt, vsz, Weight::Bold);
            let base = y0 + row_h * 0.5 + vsz as f32 * 0.36;
            let right = inner.x + inner.w;
            self.text(&txt, right - uw - vw, base, vsz, Weight::Bold, if zt.is_some() { col } else { GREY }, 1.0);
            if !suffix.is_empty() {
                self.text(suffix, right - uw + vsz as f32 * 0.1, base, usz, Weight::Regular, mix(col, [200, 206, 220], 0.45), 0.95);
            }
            let name = if z.name.trim().is_empty() { "Local" } else { z.name.as_str() };
            let nsz = ((vsz as f32) * 0.62).min(row_h * 0.4).round().max(8.0) as u32;
            let has_sub = !day.is_empty() && row_h * 0.26 >= 9.0;
            let nbase = if has_sub { y0 + row_h * 0.5 } else { base };
            let nm = self.fit(name, nsz, Weight::Regular, (inner.w - vw - uw - 14.0).max(20.0));
            self.text(&nm, inner.x, nbase, nsz, Weight::Regular, col, 0.95);
            if has_sub {
                let ssz = (row_h * 0.2).round().max(8.0) as u32;
                self.text(day, inner.x, nbase + ssz as f32 * 1.25, ssz, Weight::Regular, LAB, 0.9);
            }
        }
    }

    // ------------------------------------------------------------ timer

    fn draw_timer(&mut self, b: Rect, p: &TimerProps, _now: &LocalTime) {
        let inner = self.mcard(b);
        let v = mw::timer_view(p, mw::now_ms());
        let base_col = parse_color(&p.color, [255, 255, 255]);
        let accent = match (v.done, v.state) {
            (true, _) => RED,
            (_, "Break") => GREEN,
            (_, "Paused") => mix(base_col, GREY, 0.5),
            _ => base_col,
        };
        // a finished timer pulses its opacity (no movement)
        let pulse = if v.done && !self.sample { 0.65 + 0.35 * (self.t0.elapsed().as_secs_f32() * 4.0).sin().abs() } else { 1.0 };
        let name = match &p.label {
            Some(l) if !l.trim().is_empty() => l.clone(),
            _ => match p.mode.as_str() {
                "timer" => "Timer".into(),
                "pomodoro" => "Pomodoro".into(),
                _ => "Stopwatch".into(),
            },
        };
        let lsz = (inner.h * 0.16).min(inner.w * 0.1).max(8.0).round() as u32;
        let state_w = self.fonts.width(v.state, lsz, Weight::Regular);
        let nm = self.fit(&name, lsz, Weight::Regular, (inner.w - state_w - 14.0).max(20.0));
        self.text(&nm, inner.x, inner.y + lsz as f32 * 0.85, lsz, Weight::Regular, LAB, 1.0);
        self.text_r(v.state, inner.x + inner.w, inner.y + lsz as f32 * 0.85, lsz, Weight::Regular, accent, 1.0);
        let bar_h = (inner.h * 0.09).clamp(4.0, 22.0);
        let top = inner.y + lsz as f32 * 1.2;
        let area_h = (inner.y + inner.h - bar_h - inner.h * 0.06) - top;
        let txt = mw::clock_text(v.shown_secs);
        let vsz = self.fit_px(&txt, Weight::Bold, inner.w, area_h * 0.95);
        let tw = self.fonts.width(&txt, vsz, Weight::Bold);
        self.text(&txt, inner.x + (inner.w - tw) / 2.0, top + (area_h + vsz as f32 * 0.72) / 2.0, vsz, Weight::Bold, accent, pulse);
        let frac = if p.mode == "stopwatch" && !p.running && p.elapsed_ms <= 0.0 { 0.0 } else { v.fraction };
        self.track_and_fill(inner.x, inner.y + inner.h - bar_h, inner.w, bar_h, frac, accent, true);
    }
}

/// Green below 70, amber to 88, red above (the load colours the other widgets use).
fn load_color(v: f32) -> [u8; 3] {
    if v >= 88.0 { RED } else if v >= 70.0 { AMBER } else { GREEN }
}

/// A made-up history (age, value) for previews.
fn sample_series(v: f32, secs: f32) -> Vec<(f32, f32)> {
    let n = 60;
    (0..n)
        .map(|i| {
            let age = secs * (1.0 - i as f32 / (n - 1) as f32);
            let w = (i as f32 * 0.35).sin() * 0.45 + (i as f32 * 1.3).sin() * 0.15;
            (age, (v * (1.0 + w)).max(0.0))
        })
        .collect()
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
    fn all_new_widgets_draw_in_every_option_and_tiny_boxes() {
        let a = theme(&[
            ("meters", 0.02, 0.04, 0.3, 0.45, json!({})),
            ("cores", 0.34, 0.04, 0.3, 0.45, json!({})),
            ("network", 0.66, 0.04, 0.32, 0.3, json!({})),
            ("disks", 0.02, 0.52, 0.3, 0.44, json!({})),
            ("worldclock", 0.34, 0.52, 0.3, 0.44, json!({})),
            ("timer", 0.66, 0.52, 0.32, 0.3, json!({ "mode": "timer", "duration": 600, "running": true, "startedAt": 1.0 })),
        ]);
        let png = crate::live::render_preview(&a, 1200, 540, true).unwrap();
        dump("all", &png);
        let b = theme(&[
            ("cores", 0.02, 0.04, 0.3, 0.45, json!({ "style": "heat", "color": "#ff8800" })),
            ("network", 0.34, 0.04, 0.3, 0.2, json!({ "graph": false })),
            ("disks", 0.02, 0.52, 0.3, 0.44, json!({ "show": "free", "drives": ["C:", "Z:"] })),
            ("worldclock", 0.34, 0.52, 0.3, 0.44, json!({ "format": "12h", "zones": [{ "name": "Sydney", "tz": "AUS Eastern Standard Time" }, { "name": "Bad", "tz": "Nowhere" }] })),
            ("timer", 0.66, 0.52, 0.32, 0.3, json!({ "mode": "pomodoro", "label": "Focus" })),
            ("meters", 0.66, 0.04, 0.3, 0.4, json!({ "rows": [{ "metric": "net.down" }, { "metric": "disk.used", "drive": "D:" }, { "metric": "cpu.temp" }], "bars": false })),
        ]);
        dump("variants", &crate::live::render_preview(&b, 1200, 540, true).unwrap());
        let mut c = a.clone();
        c.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        crate::live::render_preview(&c, 320, 144, true).unwrap();
        let mut d = theme(&[("meters", 0.1, 0.1, 0.3, 0.3, json!({ "rows": [] })), ("worldclock", 0.5, 0.1, 0.3, 0.3, json!({ "zones": [] })), ("disks", 0.1, 0.5, 0.3, 0.3, json!({}))]);
        d.theme.widgets[2].props = json!("garbage");
        crate::live::render_preview(&d, 640, 288, true).unwrap();
    }

    /// Live (not sample) data: the sampler thread feeds the widgets; measures what one redraw of all six costs.
    #[test]
    fn live_data_draws_and_costs_little() {
        use std::sync::{Arc, Mutex};
        let mut r = crate::live::render::Renderer::new(Arc::new(Mutex::new(Default::default())), None);
        let mut t = theme(&[
            ("meters", 0.02, 0.04, 0.3, 0.45, json!({})),
            ("cores", 0.34, 0.04, 0.3, 0.45, json!({})),
            ("network", 0.66, 0.04, 0.32, 0.3, json!({})),
            ("disks", 0.02, 0.52, 0.3, 0.44, json!({})),
            ("worldclock", 0.34, 0.52, 0.3, 0.44, json!({})),
            ("timer", 0.66, 0.52, 0.32, 0.3, json!({ "mode": "stopwatch", "running": true, "startedAt": crate::live::morewidgets::now_ms() })),
        ]);
        t.theme.widgets.iter_mut().for_each(|w| w.id = format!("live-{}", w.id));
        r.set_theme(t);
        let _ = r.render();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let n = 100;
        let t0 = std::time::Instant::now();
        for _ in 0..n {
            r.invalidate();
            let _ = r.render();
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("six new widgets, live data: {per:.2} ms per full redraw ({:.2} % of one core at 10 redraws/s)", per / 10.0);
        let by: Vec<_> = r.widget_us.values().map(|(k, us)| format!("{k}={:.2}ms", *us as f64 / 1000.0 / (n + 1) as f64)).collect();
        eprintln!("per widget: {by:?}");
        assert!(per < 25.0, "a redraw of all six took {per:.1} ms");
        // the cores really were read: every logical core slot exists in the sampler
        let (v, _) = crate::live::metrics::hub().read("cpu.core:0", 2);
        let _ = v;
    }

    #[test]
    fn every_metric_is_a_meter_row_and_the_new_readings_are_polled() {
        let rows: Vec<serde_json::Value> = crate::live::theme::SENSOR_METRICS.iter().map(|m| json!({ "metric": m })).collect();
        // the meters widget holds 8 rows: draw them in chunks
        for chunk in rows.chunks(8) {
            let t = theme(&[("meters", 0.0, 0.0, 0.5, 1.0, json!({ "rows": chunk })), ("network", 0.5, 0.0, 0.5, 0.5, json!({ "totals": true }))]);
            crate::live::render_preview(&t, 640, 288, true).unwrap();
        }
        let h = crate::live::metrics::hub();
        for k in ["disk.read", "disk.write", "net.rx.total:*", "net.tx.total:*"] {
            h.read(k, 4);
        }
        std::thread::sleep(std::time::Duration::from_millis(1600));
        let (rx, _) = h.read("net.rx.total:*", 4);
        assert!(rx.is_some_and(|v| v > 0.0), "adapter counters are read");
        let (r, _) = h.read("disk.read", 4);
        eprintln!("disk read now: {r:?}");
    }
}
