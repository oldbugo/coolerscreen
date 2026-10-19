//! The `sensor` widget (one reading in one of six styles) and the drawing primitives the v1.6 widgets share.
//! Missing sources show a dash, greyed - never 0. Thresholds colour the value by the metric's natural unit:
//! percent (also for RAM / VRAM / disk space: share of the total), degrees Celsius for temperatures (whatever unit is shown),
//! MHz, watts, bytes per second.
use super::{round_rect, Rect, Renderer, Weight};
use crate::live::metrics;
use crate::live::theme::{parse_color, SensorProps};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

pub(super) const GREY: [u8; 3] = [140, 146, 160];
const GREEN: [u8; 3] = [90, 210, 130];
const AMBER: [u8; 3] = [240, 200, 70];
const RED: [u8; 3] = [240, 90, 80];
const CYAN: [u8; 3] = [90, 200, 255];

// ---------------------------------------------------------------- primitives

fn paint(rgba: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
    p.anti_alias = true;
    p
}

pub(super) fn stroke(layer: &mut Pixmap, path: &Path, rgba: [u8; 4], width: f32, round: bool) {
    let st = Stroke { width: width.max(0.5), line_cap: if round { LineCap::Round } else { LineCap::Butt }, line_join: LineJoin::Round, ..Stroke::default() };
    layer.stroke_path(path, &paint(rgba), &st, Transform::identity(), None);
}

pub(super) fn fill(layer: &mut Pixmap, path: &Path, rgba: [u8; 4]) {
    layer.fill_path(path, &paint(rgba), FillRule::Winding, Transform::identity(), None);
}

pub(super) fn fill_even_odd(layer: &mut Pixmap, path: &Path, rgba: [u8; 4]) {
    layer.fill_path(path, &paint(rgba), FillRule::EvenOdd, Transform::identity(), None);
}

pub(super) fn line(layer: &mut Pixmap, x0: f32, y0: f32, x1: f32, y1: f32, rgba: [u8; 4], width: f32, round: bool) {
    let mut pb = PathBuilder::new();
    pb.move_to(x0, y0);
    pb.line_to(x1, y1);
    if let Some(p) = pb.finish() {
        stroke(layer, &p, rgba, width, round);
    }
}

pub(super) fn circle(layer: &mut Pixmap, cx: f32, cy: f32, r: f32, rgba: [u8; 4]) {
    if let Some(p) = PathBuilder::from_circle(cx, cy, r.max(0.3)) {
        fill(layer, &p, rgba);
    }
}

/// An arc as a polyline path. Angles in degrees, 0 = 3 o'clock, clockwise (y down).
pub(super) fn arc_path(cx: f32, cy: f32, r: f32, start_deg: f32, sweep_deg: f32) -> Option<Path> {
    let steps = ((sweep_deg.abs() / 3.0).ceil() as usize).max(2);
    let mut pb = PathBuilder::new();
    for i in 0..=steps {
        let a = (start_deg + sweep_deg * i as f32 / steps as f32).to_radians();
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        if i == 0 {
            pb.move_to(x, y);
        } else {
            pb.line_to(x, y);
        }
    }
    pb.finish()
}

pub(super) fn arc(layer: &mut Pixmap, cx: f32, cy: f32, r: f32, start_deg: f32, sweep_deg: f32, rgba: [u8; 4], width: f32) {
    if sweep_deg.abs() < 0.2 {
        // a dot so a zero value still shows its start
        return;
    }
    if let Some(p) = arc_path(cx, cy, r, start_deg, sweep_deg) {
        stroke(layer, &p, rgba, width, true);
    }
}

pub(super) fn mix(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [0, 1, 2].map(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t) as u8)
}

// ---------------------------------------------------------------- metrics

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Percent,
    Temp,
    Mhz,
    Watts,
    Gb,
    Rate,
}

pub(super) struct Info {
    pub(super) label: &'static str,
    pub(super) kind: Kind,
    /// Default scale max (None: the metric's total, else automatic).
    max: Option<f32>,
    /// (threshold, colour) in the metric's threshold unit, ascending.
    thr: &'static [(f32, [u8; 3])],
    base: [u8; 3],
}

pub(super) fn info(metric: &str) -> Info {
    let thr_load: &'static [(f32, [u8; 3])] = &[(70.0, AMBER), (88.0, RED)];
    match metric {
        "cpu.core" => Info { label: "CPU core", kind: Kind::Percent, max: Some(100.0), thr: thr_load, base: GREEN },
        "cpu.temp" => Info { label: "CPU temp", kind: Kind::Temp, max: Some(100.0), thr: &[(70.0, AMBER), (85.0, RED)], base: GREEN },
        "cpu.clock" => Info { label: "CPU clock", kind: Kind::Mhz, max: Some(6000.0), thr: &[], base: CYAN },
        "gpu.load" => Info { label: "GPU load", kind: Kind::Percent, max: Some(100.0), thr: thr_load, base: GREEN },
        "gpu.temp" => Info { label: "GPU temp", kind: Kind::Temp, max: Some(100.0), thr: &[(70.0, AMBER), (83.0, RED)], base: GREEN },
        "gpu.clock" => Info { label: "GPU clock", kind: Kind::Mhz, max: Some(3200.0), thr: &[], base: CYAN },
        "gpu.vram" => Info { label: "VRAM", kind: Kind::Gb, max: None, thr: &[(80.0, AMBER), (92.0, RED)], base: GREEN },
        "gpu.power" => Info { label: "GPU power", kind: Kind::Watts, max: None, thr: &[], base: CYAN },
        "gpu.fan" => Info { label: "GPU fan", kind: Kind::Percent, max: Some(100.0), thr: &[], base: CYAN },
        "ram.used" => Info { label: "RAM", kind: Kind::Gb, max: None, thr: &[(75.0, AMBER), (90.0, RED)], base: GREEN },
        "net.down" => Info { label: "Download", kind: Kind::Rate, max: None, thr: &[], base: [90, 220, 160] },
        "net.up" => Info { label: "Upload", kind: Kind::Rate, max: None, thr: &[], base: [170, 140, 255] },
        "disk.used" => Info { label: "Disk", kind: Kind::Gb, max: None, thr: &[(85.0, AMBER), (95.0, RED)], base: GREEN },
        "disk.activity" => Info { label: "Disk activity", kind: Kind::Percent, max: Some(100.0), thr: thr_load, base: GREEN },
        _ => Info { label: "CPU load", kind: Kind::Percent, max: Some(100.0), thr: thr_load, base: GREEN },
    }
}

/// Round up to 1, 2 or 5 times a power of ten.
pub(super) fn nice_ceil(v: f32) -> f32 {
    if v <= 0.0 {
        return 1.0;
    }
    let p = 10f32.powf(v.log10().floor());
    for m in [1.0, 2.0, 5.0, 10.0] {
        if v <= m * p {
            return m * p;
        }
    }
    10.0 * p
}

/// The shown number and its unit: ("18.4", "GB"), ("62", "\u{00B0}C"), ("1.2", "MB/s").
pub(super) fn format_value(metric: &str, v: f32, unit_f: bool, decimals: Option<u32>) -> (String, String) {
    let i = info(metric);
    match i.kind {
        Kind::Percent => (format!("{:.*}", decimals.unwrap_or(0) as usize, v), "%".into()),
        Kind::Temp => {
            let t = if unit_f { v * 9.0 / 5.0 + 32.0 } else { v };
            (format!("{:.*}", decimals.unwrap_or(0) as usize, t), if unit_f { "\u{00B0}F" } else { "\u{00B0}C" }.into())
        }
        Kind::Mhz => (format!("{:.*}", decimals.unwrap_or(0) as usize, v), "MHz".into()),
        Kind::Watts => (format!("{:.*}", decimals.unwrap_or(0) as usize, v), "W".into()),
        Kind::Gb => (format!("{:.*}", decimals.unwrap_or(1) as usize, v), "GB".into()),
        Kind::Rate => {
            let d = decimals;
            if v >= 1e9 {
                (format!("{:.*}", d.unwrap_or(1) as usize, v / 1e9), "GB/s".into())
            } else if v >= 1e6 {
                (format!("{:.*}", d.unwrap_or(1) as usize, v / 1e6), "MB/s".into())
            } else if v >= 1e3 {
                (format!("{:.*}", d.unwrap_or(0) as usize, v / 1e3), "KB/s".into())
            } else {
                (format!("{:.0}", v), "B/s".into())
            }
        }
    }
}

/// Fixed example readings and a made-up history for previews.
fn sample_value(metric: &str, core: u32) -> (Option<f32>, Option<f32>) {
    match metric {
        "cpu.load" => (Some(34.0), None),
        "cpu.core" => (Some(20.0 + (core * 17 % 60) as f32), None),
        "cpu.temp" => (Some(58.0), None),
        "cpu.clock" => (Some(4650.0), None),
        "gpu.load" => (Some(52.0), None),
        "gpu.temp" => (Some(61.0), None),
        "gpu.clock" => (Some(1950.0), None),
        "gpu.vram" => (Some(6.2), Some(16.0)),
        "gpu.power" => (Some(182.0), None),
        "gpu.fan" => (Some(46.0), None),
        "ram.used" => (Some(18.4), Some(31.9)),
        "net.down" => (Some(12.4e6), None),
        "net.up" => (Some(1.6e6), None),
        "disk.used" => (Some(412.0), Some(931.0)),
        "disk.activity" => (Some(14.0), None),
        _ => (None, None),
    }
}

fn sample_history(v: f32, secs: f32) -> Vec<(f32, f32)> {
    let n = 60;
    (0..n)
        .map(|i| {
            let age = secs * (1.0 - i as f32 / (n - 1) as f32);
            let w = (i as f32 * 0.35).sin() * 0.25 + (i as f32 * 0.9).sin() * 0.08;
            (age, (v * (1.0 + w)).max(0.0))
        })
        .collect()
}

pub(super) struct Reading {
    pub(super) value: Option<f32>,
    /// Natural range end of the metric in the same unit as the value (total RAM, ...).
    pub(super) total: Option<f32>,
    pub(super) hist: Vec<(f32, f32)>,
}

impl Renderer {
    pub(super) fn read_metric(&mut self, p: &SensorProps, need_hist: bool) -> Reading {
        if self.sample {
            let (v, total) = sample_value(&p.metric, p.core);
            let hist = if need_hist { sample_history(v.unwrap_or(0.0), p.history_secs()) } else { Vec::new() };
            return Reading { value: v, total, hist };
        }
        let k = metrics::key(&p.metric, p.core, &p.drive, p.adapter.as_deref());
        let h = metrics::hub();
        let (value, total) = h.read(&k, p.rate_hz());
        let hist = if need_hist { h.history(&k, p.history_secs()) } else { Vec::new() };
        Reading { value, total, hist }
    }

    /// The threshold colour of a value: user steps, else the metric's defaults, else the base colour.
    pub(super) fn sensor_color(&self, p: &SensorProps, i: &Info, value: f32, total: Option<f32>) -> [u8; 3] {
        let base = p.color.as_deref().map(|c| parse_color(c, i.base)).unwrap_or(i.base);
        // the value in threshold units
        let tv = match (i.kind, total) {
            (Kind::Gb, Some(t)) if t > 0.0 => 100.0 * value / t,
            _ => value,
        };
        let steps: Vec<(f32, [u8; 3])> = match &p.thresholds {
            Some(t) => t.iter().map(|t| (t.at, parse_color(&t.color, base))).collect(),
            None => i.thr.iter().map(|(a, c)| (*a, if p.color.is_some() && false { base } else { *c })).collect(),
        };
        let mut col = base;
        let mut sorted = steps;
        sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        for (at, c) in sorted {
            if tv >= at {
                col = c;
            }
        }
        col
    }

    /// Largest font size (px, at most `max_h`) at which `s` fits `max_w`.
    pub(super) fn fit_px(&mut self, s: &str, wt: Weight, max_w: f32, max_h: f32) -> u32 {
        let w100 = self.fonts.width(s, 100, wt).max(1.0) / 100.0;
        (max_h.min(max_w / w100)).floor().max(6.0) as u32
    }

    pub(super) fn draw_sensor(&mut self, b: Rect, id: &str, p: &SensorProps) {
        let i = info(&p.metric);
        let style = p.style.as_str();
        let need_hist = style == "graph" || style == "sparkline";
        let r = self.read_metric(p, need_hist);
        let unit_f = p.unit.eq_ignore_ascii_case("F");
        // scale
        let hist_peak = r.hist.iter().map(|h| h.1).fold(0.0f32, f32::max);
        let cur = r.value.unwrap_or(0.0);
        let min = p.min.unwrap_or(0.0);
        let max = p.max.or(i.max).or(r.total).unwrap_or_else(|| {
            let floor = match i.kind {
                Kind::Rate => 1e6,
                Kind::Watts => 100.0,
                _ => 1.0,
            };
            nice_ceil((hist_peak.max(cur) * 1.15).max(floor))
        });
        let max = if max > min { max } else { min + 1.0 };
        let target = ((cur - min) / (max - min)).clamp(0.0, 1.0);
        // displayed fraction eases toward the target (a few ticks), as the stats bars do
        let k = 1.0 - 0.75f32.powi(self.steps_now.max(0));
        let shown = {
            let e = self.sens_smooth.entry(id.to_string()).or_insert(target);
            if self.sample || self.steps_now >= 30 {
                *e = target;
            } else {
                *e += (target - *e) * k;
            }
            *e
        };
        let col = match r.value {
            Some(v) => self.sensor_color(p, &i, v, r.total),
            None => GREY,
        };
        let label = match &p.label {
            Some(l) => l.clone(),
            None if p.metric == "cpu.core" => format!("CPU core {}", p.core),
            None if p.metric == "disk.used" => format!("Disk {}", p.drive.trim().trim_end_matches('\\')),
            None => i.label.to_string(),
        };
        let (num, unit) = match r.value {
            Some(v) => format_value(&p.metric, v, unit_f, p.decimals),
            None => ("\u{2013}".to_string(), String::new()),
        };
        // sub text for sizes (RAM "of 31.9 GB")
        let sub = match (r.value, r.total, i.kind) {
            (Some(_), Some(t), Kind::Gb) => Some(format!("of {:.0} GB", t)),
            _ => None,
        };
        let sc = (self.cw as f32 / 1680.0).max(0.1);
        round_rect(&mut self.layer, b.x, b.y, b.w, b.h, (28.0 * sc).min(b.w.min(b.h) / 5.0), [0, 0, 0, 96]);
        let pad = (b.w.min(b.h) * 0.07).max(6.0);
        let inner = Rect { x: b.x + pad, y: b.y + pad, w: b.w - 2.0 * pad, h: b.h - 2.0 * pad };
        let lab_col = [185, 192, 208];
        match style {
            "bar" => self.sensor_bar(inner, &label, &num, &unit, shown, col, lab_col, r.value.is_some()),
            "ring" => self.sensor_dial(inner, &label, &num, &unit, shown, col, lab_col, false, r.value.is_some()),
            "gauge" => self.sensor_dial(inner, &label, &num, &unit, shown, col, lab_col, true, r.value.is_some()),
            "graph" => self.sensor_graph(inner, &label, &num, &unit, &r.hist, min, max, p.history_secs(), col, lab_col, true),
            "sparkline" => self.sensor_graph(inner, &label, &num, &unit, &r.hist, min, max, p.history_secs(), col, lab_col, false),
            _ => self.sensor_number(inner, &label, &num, &unit, sub.as_deref(), col, lab_col),
        }
    }

    fn sensor_number(&mut self, b: Rect, label: &str, num: &str, unit: &str, sub: Option<&str>, col: [u8; 3], lab_col: [u8; 3]) {
        let has_label = !label.is_empty();
        let lsz = if has_label { (b.h * 0.19).min(b.w * 0.2).max(8.0).round() as u32 } else { 0 };
        if has_label {
            let l = self.fit(label, lsz, Weight::Regular, b.w);
            self.text(&l, b.x, b.y + lsz as f32 * 0.85, lsz, Weight::Regular, lab_col, 1.0);
        }
        let top = b.y + if has_label { lsz as f32 * 1.1 } else { 0.0 };
        let area_h = b.y + b.h - top;
        // number plus a smaller unit
        let unit_k = 0.42;
        let w100 = (self.fonts.width(num, 100, Weight::Bold) + self.fonts.width(unit, 100, Weight::Regular) * unit_k + if unit.is_empty() { 0.0 } else { 14.0 }) / 100.0;
        let sub_h = if sub.is_some() { 0.18 } else { 0.0 };
        let size = ((area_h / (0.82 + sub_h)).min(b.w / w100.max(0.1))).floor().max(8.0);
        let total_w = w100 * size;
        let x = b.x + (b.w - total_w) / 2.0;
        let content_h = size * (0.72 + sub_h);
        let base = top + (area_h - content_h) / 2.0 + size * 0.72;
        let px = size as u32;
        self.text(num, x, base, px, Weight::Bold, col, 1.0);
        if !unit.is_empty() {
            let nw = self.fonts.width(num, px, Weight::Bold);
            let usz = (size * unit_k).round().max(6.0) as u32;
            self.text(unit, x + nw + size * 0.08, base, usz, Weight::Regular, mix(col, [200, 206, 220], 0.45), 0.95);
        }
        if let Some(s) = sub {
            let ssz = (size * 0.2).round().max(6.0) as u32;
            let sw = self.fonts.width(s, ssz, Weight::Regular);
            self.text(s, b.x + (b.w - sw) / 2.0, base + size * 0.24, ssz, Weight::Regular, lab_col, 0.9);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sensor_bar(&mut self, b: Rect, label: &str, num: &str, unit: &str, frac: f32, col: [u8; 3], lab_col: [u8; 3], present: bool) {
        let row_h = b.h * 0.5;
        let vsz = self.fit_px(&format!("{num} {unit}"), Weight::Bold, b.w * 0.6, row_h * 0.95);
        let lsz = ((vsz as f32) * 0.62).round().max(8.0) as u32;
        let text_base = b.y + row_h * 0.78;
        let vtxt = if unit.is_empty() { num.to_string() } else { format!("{num} {unit}") };
        let vw = self.fonts.width(&vtxt, vsz, Weight::Bold);
        if !label.is_empty() {
            let l = self.fit(label, lsz, Weight::Regular, (b.w - vw - 10.0).max(20.0));
            self.text(&l, b.x, text_base, lsz, Weight::Regular, lab_col, 1.0);
        }
        self.text(&vtxt, b.x + b.w - vw, text_base, vsz, Weight::Bold, col, 1.0);
        let bh = (b.h * 0.22).clamp(6.0, 40.0);
        let by = b.y + b.h - bh - b.h * 0.06;
        round_rect(&mut self.layer, b.x, by, b.w, bh, bh / 2.0, [255, 255, 255, 40]);
        if present && frac > 0.005 {
            round_rect(&mut self.layer, b.x, by, (b.w * frac).max(bh), bh, bh / 2.0, [col[0], col[1], col[2], 255]);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sensor_dial(&mut self, b: Rect, label: &str, num: &str, unit: &str, frac: f32, col: [u8; 3], lab_col: [u8; 3], gauge: bool, present: bool) {
        let has_label = !label.is_empty();
        let lab_h = if has_label { b.h * 0.14 } else { 0.0 };
        let lab_h = lab_h * 1.35;
        let d = (b.w.min(b.h - lab_h)).max(10.0);
        let (cx, cy) = (b.x + b.w / 2.0, b.y + (b.h - lab_h) / 2.0);
        let width = d * 0.11;
        let r = (d - width) / 2.0;
        let (start, sweep) = if gauge { (135.0, 270.0) } else { (-90.0, 360.0) };
        // track
        if gauge {
            arc(&mut self.layer, cx, cy, r, start, sweep, [255, 255, 255, 40], width);
        } else if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
            stroke(&mut self.layer, &p, [255, 255, 255, 40], width, false);
        }
        if present && frac > 0.004 {
            if !gauge && frac > 0.995 {
                if let Some(p) = PathBuilder::from_circle(cx, cy, r) {
                    stroke(&mut self.layer, &p, [col[0], col[1], col[2], 255], width, false);
                }
            } else {
                arc(&mut self.layer, cx, cy, r, start, sweep * frac, [col[0], col[1], col[2], 255], width);
            }
        }
        // value in the middle
        let inner_w = d * 0.62;
        let size = self.fit_px(num, Weight::Bold, inner_w, d * 0.30);
        let nw = self.fonts.width(num, size, Weight::Bold);
        let usz = ((size as f32) * 0.4).round().max(6.0) as u32;
        let uw = if unit.is_empty() { 0.0 } else { self.fonts.width(unit, usz, Weight::Regular) + size as f32 * 0.08 };
        let x = cx - (nw + uw) / 2.0;
        let base = cy + size as f32 * if gauge { 0.2 } else { 0.36 };
        self.text(num, x, base, size, Weight::Bold, col, 1.0);
        if !unit.is_empty() {
            self.text(unit, x + nw + size as f32 * 0.08, base, usz, Weight::Regular, mix(col, [200, 206, 220], 0.45), 0.95);
        }
        if has_label {
            let lsz = (lab_h * 0.85).round().max(8.0) as u32;
            let l = self.fit(label, lsz, Weight::Regular, b.w);
            let lw = self.fonts.width(&l, lsz, Weight::Regular);
            self.text(&l, b.x + (b.w - lw) / 2.0, b.y + b.h - lab_h * 0.15, lsz, Weight::Regular, lab_col, 1.0);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sensor_graph(&mut self, b: Rect, label: &str, num: &str, unit: &str, hist: &[(f32, f32)], min: f32, max: f32, secs: f32, col: [u8; 3], lab_col: [u8; 3], full: bool) {
        let wide_row = !full && b.w / b.h > 3.2;
        let vtxt = if unit.is_empty() { num.to_string() } else { format!("{num} {unit}") };
        let head_h = if wide_row { b.h } else { b.h * if full { 0.30 } else { 0.38 } };
        let vsz = self.fit_px(&vtxt, Weight::Bold, if wide_row { b.w * 0.26 } else { b.w * 0.5 }, head_h * 0.92);
        let vw = self.fonts.width(&vtxt, vsz, Weight::Bold);
        let lsz = ((vsz as f32) * 0.6).round().max(8.0) as u32;
        let base = b.y + head_h * if wide_row { 0.5 } else { 0.8 } + vsz as f32 * if wide_row { 0.36 } else { 0.0 } - if wide_row { 0.0 } else { vsz as f32 * 0.16 };
        self.text(&vtxt, b.x + b.w - vw, base, vsz, Weight::Bold, col, 1.0);
        let lab_w = if wide_row { b.w * 0.22 } else { b.w - vw - 10.0 };
        if !label.is_empty() {
            let l = self.fit(label, lsz, Weight::Regular, lab_w.max(20.0));
            self.text(&l, b.x, base, lsz, Weight::Regular, lab_col, 1.0);
        }
        // plot area
        let (px, pw, py, ph) = if wide_row {
            let x0 = b.x + b.w * 0.24;
            (x0, (b.x + b.w - vw - 14.0) - x0, b.y + b.h * 0.12, b.h * 0.76)
        } else {
            let y0 = b.y + head_h + b.h * 0.04;
            (b.x, b.w, y0, (b.y + b.h - y0).max(6.0))
        };
        if pw < 10.0 || ph < 6.0 {
            return;
        }
        if full {
            for f in [0.0f32, 0.5, 1.0] {
                let y = py + ph * f;
                line(&mut self.layer, px, y, px + pw, y, [255, 255, 255, if f == 1.0 { 55 } else { 28 }], 1.0, false);
            }
        }
        if hist.len() >= 2 {
            let mut pb = PathBuilder::new();
            let mut area = PathBuilder::new();
            let mut first = true;
            let (mut last_x, mut first_x) = (0.0, 0.0);
            for (age, v) in hist {
                let x = px + pw * (1.0 - (age / secs).clamp(0.0, 1.0));
                let y = py + ph * (1.0 - ((v - min) / (max - min)).clamp(0.0, 1.0));
                if first {
                    pb.move_to(x, y);
                    area.move_to(x, py + ph);
                    area.line_to(x, y);
                    first_x = x;
                    first = false;
                } else {
                    pb.line_to(x, y);
                    area.line_to(x, y);
                }
                last_x = x;
            }
            let _ = first_x;
            if full {
                area.line_to(last_x, py + ph);
                area.close();
                if let Some(a) = area.finish() {
                    fill(&mut self.layer, &a, [col[0], col[1], col[2], 60]);
                }
            }
            if let Some(l) = pb.finish() {
                stroke(&mut self.layer, &l, [col[0], col[1], col[2], 255], (ph * 0.03).clamp(1.5, 4.0), true);
            }
        } else {
            line(&mut self.layer, px, py + ph - 1.0, px + pw, py + ph - 1.0, [255, 255, 255, 50], 1.5, false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_formatted_with_units_and_scaling() {
        assert_eq!(format_value("cpu.load", 34.4, false, None), ("34".into(), "%".into()));
        assert_eq!(format_value("gpu.temp", 61.0, true, None), ("142".into(), "\u{00B0}F".into()));
        assert_eq!(format_value("gpu.temp", 61.0, false, None).1, "\u{00B0}C");
        assert_eq!(format_value("ram.used", 18.44, false, None), ("18.4".into(), "GB".into()));
        assert_eq!(format_value("net.down", 12.4e6, false, None), ("12.4".into(), "MB/s".into()));
        assert_eq!(format_value("net.down", 53_000.0, false, None), ("53".into(), "KB/s".into()));
        assert_eq!(format_value("net.up", 800.0, false, None), ("800".into(), "B/s".into()));
        assert_eq!(format_value("net.down", 2.5e9, false, Some(2)), ("2.50".into(), "GB/s".into()));
        assert_eq!(nice_ceil(1.3), 2.0);
        assert_eq!(nice_ceil(130.0), 200.0);
        assert_eq!(nice_ceil(4_900_000.0), 5_000_000.0);
    }

    #[test]
    fn default_thresholds_colour_by_share_or_value() {
        let r = Renderer::new(std::sync::Arc::new(std::sync::Mutex::new(Default::default())), None);
        let p = SensorProps { metric: "ram.used".into(), ..SensorProps::default() };
        let i = info("ram.used");
        assert_eq!(r.sensor_color(&p, &i, 10.0, Some(32.0)), GREEN);
        assert_eq!(r.sensor_color(&p, &i, 26.0, Some(32.0)), AMBER);
        assert_eq!(r.sensor_color(&p, &i, 31.0, Some(32.0)), RED);
        let t = SensorProps { metric: "gpu.temp".into(), thresholds: Some(vec![crate::live::theme::Threshold { at: 50.0, color: "#0000ff".into() }]), ..SensorProps::default() };
        assert_eq!(r.sensor_color(&t, &info("gpu.temp"), 55.0, None), [0, 0, 255]);
        assert_eq!(r.sensor_color(&t, &info("gpu.temp"), 40.0, None), GREEN);
    }
}
