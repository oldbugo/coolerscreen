//! Widget drawing: every widget draws itself inside its box and scales with it. Design numbers below are for the Standard
//! preset's boxes (clock 890x393, music 890x220, stats 592x660 at the 1680x756 canvas); `u` is the box-to-design scale.
use super::{blend_cov, blend_tile, round_rect, ImgEntry, LocalTime, Rect, Renderer, Weight, CLOCK_CAP, DAYS, MONTHS};
use crate::live::nowplaying::NowPlaying;
use crate::live::sensors::Sensors;
use crate::live::theme::{parse_color, ClockProps, MusicProps, StatsProps, TextProps, Widget};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_skia::{FillRule, FilterQuality, Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

const SCRIM: [u8; 4] = [0, 0, 0, 96];

impl Renderer {
    fn scale(&self) -> f32 {
        self.cw as f32 / 1680.0
    }

    /// Text with a soft drop shadow (sideways offset only), alpha-blended onto the widget layer.
    pub(super) fn text(&mut self, s: &str, x: f32, baseline: f32, px: u32, wt: Weight, rgb: [u8; 3], alpha: f32) {
        let shadow = (px as f32 / 40.0).clamp(1.0, 3.0).round() as i32;
        let mut pen = x;
        for c in s.chars() {
            let Renderer { layer, fonts, cw, ch, .. } = self;
            let Some(g) = fonts.glyph(c, px, wt) else { continue };
            let gx = pen.round() as i32 + g.x;
            let gy = baseline.round() as i32 + g.y;
            blend_cov(layer.data_mut(), *cw, *ch, &g.cov, g.w, g.h, gx + shadow, gy, [0, 0, 0], 0.45 * alpha);
            blend_cov(layer.data_mut(), *cw, *ch, &g.cov, g.w, g.h, gx, gy, rgb, alpha);
            pen += g.adv;
        }
    }

    /// Shorten `s` with an ellipsis so it fits `max` px.
    pub(super) fn fit(&mut self, s: &str, px: u32, wt: Weight, max: f32) -> String {
        if self.fonts.width(s, px, wt) <= max {
            return s.to_string();
        }
        let mut out: String = s.to_string();
        while !out.is_empty() && self.fonts.width(&format!("{out}\u{2026}"), px, wt) > max {
            out.pop();
        }
        format!("{}\u{2026}", out.trim_end())
    }

    pub(super) fn fill_path(&mut self, pts: &[(f32, f32)], rgba: [u8; 4]) {
        let mut pb = PathBuilder::new();
        for (i, (x, y)) in pts.iter().enumerate() {
            if i == 0 {
                pb.move_to(*x, *y);
            } else {
                pb.line_to(*x, *y);
            }
        }
        pb.close();
        if let Some(path) = pb.finish() {
            let mut p = Paint::default();
            p.set_color_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]);
            self.layer.fill_path(&path, &p, FillRule::Winding, Transform::identity(), None);
        }
    }

    // ------------------------------------------------------------ clock

    pub(super) fn draw_clock(&mut self, b: Rect, p: &ClockProps, n: &LocalTime) {
        if p.style == "analog" || p.style == "minimal" {
            return self.draw_clock_style(b, p, n);
        }
        let col = parse_color(&p.color, [255, 255, 255]);
        let rad = self.card_radius(b, (28.0 * self.scale()).min(b.w.min(b.h) / 6.0));
        round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, SCRIM);
        let twelve = p.format == "12h";
        let hhmm = if twelve {
            let h = match n.hour % 12 {
                0 => 12,
                h => h,
            };
            format!("{}:{:02}", h, n.minute)
        } else {
            format!("{:02}:{:02}", n.hour, n.minute)
        };
        let suffix = if twelve { Some(if n.hour < 12 { "AM" } else { "PM" }) } else { None };
        // Widths in em at the time's font size S: seconds are 0.32 S, the AM/PM mark 0.22 S, the date 0.173 S.
        let w_time = self.fonts.width(&hhmm, 100, Weight::Bold) / 100.0;
        let w_sec = if p.seconds { 0.04 + self.fonts.width("00", 100, Weight::Bold) / 100.0 * 0.32 } else { 0.0 };
        let w_suf = suffix.map(|s| 0.04 + self.fonts.width(s, 100, Weight::Bold) / 100.0 * 0.22).unwrap_or(0.0);
        let row_w = w_time + w_sec + w_suf;
        let date = format!("{}, {} {}", DAYS[(n.weekday % 7) as usize], n.day, MONTHS[((n.month.max(1) - 1) % 12) as usize]);
        let w_date = if p.date { self.fonts.width(&date, 100, Weight::Regular) / 100.0 * 0.173 } else { 0.0 };
        let block_w = row_w.max(w_date).max(0.1);
        let block_h = CLOCK_CAP + if p.date { 0.35 } else { 0.0 };
        let s = ((b.w * 0.92) / block_w).min((b.h * 0.88) / block_h).max(8.0);
        let top = b.y + (b.h - s * block_h) / 2.0;
        let base = top + CLOCK_CAP * s;
        let row_x = b.x + (b.w - s * row_w) / 2.0;
        // The colon fades in and out instead of blinking hard.
        let frac = (self.tick % 10) as f32 / 10.0;
        let colon_alpha = 0.35 + 0.65 * (0.5 + 0.5 * (frac * std::f32::consts::TAU).cos());
        let size = s.round() as u32;
        let mut pen = row_x;
        for c in hhmm.chars() {
            let adv = self.fonts.glyph(c, size, Weight::Bold).map(|g| g.adv).unwrap_or(0.0);
            self.text(&c.to_string(), pen, base, size, Weight::Bold, col, if c == ':' { colon_alpha } else { 1.0 });
            pen += adv;
        }
        let mut ex = row_x + s * w_time;
        if p.seconds {
            ex += s * 0.04;
            let sec = format!("{:02}", n.second);
            let ssz = (s * 0.32).round().max(6.0) as u32;
            self.text(&sec, ex, base, ssz, Weight::Bold, col, 0.9);
            ex += self.fonts.width("00", ssz, Weight::Bold);
        }
        if let Some(sf) = suffix {
            ex += s * 0.04;
            let ssz = (s * 0.22).round().max(6.0) as u32;
            self.text(sf, ex, base, ssz, Weight::Bold, col, 0.9);
        }
        if p.date {
            let dsz = (s * 0.173).round().max(6.0) as u32;
            let dw = self.fonts.width(&date, dsz, Weight::Regular);
            self.text(&date, b.x + (b.w - dw) / 2.0, base + 0.30 * s, dsz, Weight::Regular, col, 0.88);
        }
    }

    // ------------------------------------------------------------ music

    pub(super) fn draw_music(&mut self, b: Rect, p: &MusicProps, np: Option<&NowPlaying>) {
        let a = self.music_alpha.clamp(0.0, 1.0);
        if a < 0.01 {
            return;
        }
        if p.style != "card" && self.draw_music_style(b, p, np, a) {
            return;
        }
        let u = (b.h / 220.0).min(b.w / 890.0).clamp(0.2, 4.0);
        let rad = self.card_radius(b, (24.0 * u).min(b.h / 4.0));
        round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, [SCRIM[0], SCRIM[1], SCRIM[2], (SCRIM[3] as f32 * a) as u8]);
        round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, [255, 255, 255, (26.0 * a) as u8]);
        let fade = (a * 255.0) as u32;
        let y0 = b.y + (b.h - 220.0 * u) / 2.0;
        let pad = 20.0 * u;
        let side = (180.0 * u).round() as usize;
        let mut tx = b.x + pad + 8.0 * u;
        if p.cover {
            let tid = np.map(|n| n.track_id).unwrap_or(0);
            if self.cover.as_ref().map(|c| (c.0, c.1)) != Some((tid, side)) {
                // covers are decoded up to 640 px; the card scales from at most 320 px, as it always did
                let small = np.and_then(|n| n.cover.as_deref()).and_then(super::music::shrink_cover);
                let src = small.as_ref().or(np.and_then(|n| n.cover.as_deref()));
                self.cover = Some((tid, side, scale_cover(src, side)));
            }
            if let Some((_, _, px)) = &self.cover {
                let Renderer { layer, cw, ch, .. } = self;
                blend_tile(layer.data_mut(), *cw, *ch, px, (b.x + pad).round() as i32, (y0 + pad).round() as i32, side, side, fade);
            }
            tx = b.x + pad + side as f32 + 26.0 * u;
        }
        let tw = (b.x + b.w - tx - 28.0 * u).max(10.0);
        let (title, artist) = match np {
            Some(n) => (n.title.clone(), n.artist.clone()),
            None => ("Nothing playing".to_string(), String::new()),
        };
        let tsz = (38.0 * u).round().max(6.0) as u32;
        let asz = (28.0 * u).round().max(6.0) as u32;
        let title = self.fit(&title, tsz, Weight::Bold, tw);
        self.text(&title, tx, y0 + 66.0 * u, tsz, Weight::Bold, [255, 255, 255], a);
        let artist = self.fit(&artist, asz, Weight::Regular, tw);
        self.text(&artist, tx, y0 + 108.0 * u, asz, Weight::Regular, [190, 197, 214], a);
        let (Some(n), true) = (np, p.progress) else { return };
        let (by, bh) = (y0 + 128.0 * u, 10.0 * u);
        let frac = if n.duration_s > 0.0 { (n.position() / n.duration_s).clamp(0.0, 1.0) as f32 } else { 0.0 };
        round_rect(&mut self.layer, tx, by, tw, bh, bh / 2.0, [255, 255, 255, (60.0 * a) as u8]);
        if frac > 0.0 {
            round_rect(&mut self.layer, tx, by, (tw * frac).max(bh), bh, bh / 2.0, [255, 255, 255, (255.0 * a) as u8]);
        }
        // play / pause icon and times
        let (ix, iy) = (tx, y0 + 156.0 * u);
        let col = [220, 226, 240, (255.0 * a) as u8];
        match transport_icon(n.playing) {
            TransportIcon::Play => self.fill_path(&[(ix, iy), (ix, iy + 20.0 * u), (ix + 18.0 * u, iy + 10.0 * u)], col),
            TransportIcon::Pause => {
                round_rect(&mut self.layer, ix, iy, 6.0 * u, 20.0 * u, 1.0, col);
                round_rect(&mut self.layer, ix + 10.0 * u, iy, 6.0 * u, 20.0 * u, 1.0, col);
            }
        }
        let ksz = (24.0 * u).round().max(6.0) as u32;
        let base = y0 + 176.0 * u;
        let el = fmt_time(n.position());
        self.text(&el, tx + 32.0 * u, base, ksz, Weight::Regular, [190, 197, 214], a);
        if n.duration_s > 0.0 {
            let tot = fmt_time(n.duration_s);
            let ww = self.fonts.width(&tot, ksz, Weight::Regular);
            self.text(&tot, tx + tw - ww, base, ksz, Weight::Regular, [190, 197, 214], a);
        }
    }

    // ------------------------------------------------------------ stats

    /// Move the smoothed bar values toward the sensor readings (`steps` animation ticks have passed).
    pub(super) fn step_bars(&mut self, s: &Sensors, steps: i32) {
        if steps <= 0 {
            return;
        }
        let gpu = s.gpu.as_ref();
        let target = [
            s.cpu_load_pct / 100.0,
            if s.ram_total_gb > 0.0 { s.ram_used_gb / s.ram_total_gb } else { 0.0 },
            gpu.map(|g| g.load_pct as f32 / 100.0).unwrap_or(0.0),
            gpu.map(|g| g.temp_c as f32 / 100.0).unwrap_or(0.0),
            gpu.map(|g| g.clock_mhz as f32 / 3200.0).unwrap_or(0.0),
            s.cpu_temp_c.map(|c| c / 100.0).unwrap_or(0.0),
        ];
        let k = 1.0 - 0.75f32.powi(steps);
        for (b, t) in self.bars.iter_mut().zip(target) {
            *b += (t.clamp(0.0, 1.0) - *b) * k;
        }
    }

    pub(super) fn draw_stats(&mut self, b: Rect, p: &StatsProps, s: &Sensors) {
        const IDS: [&str; 6] = ["cpu", "ram", "gpuLoad", "gpuTemp", "gpuClock", "cpuTemp"];
        let items: Vec<usize> = p.items.iter().filter_map(|i| IDS.iter().position(|k| k == i)).collect();
        let n = items.len().max(1) as f32;
        let u = (b.w / 592.0).min(b.h / (72.0 + 104.0 * n)).clamp(0.2, 4.0);
        let rad = self.card_radius(b, (28.0 * u).min(b.h / 6.0));
        round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, [8, 10, 18, 120]);
        let pad = 36.0 * u;
        let pitch = (b.h - 2.0 * pad) / n;
        let gpu = s.gpu.as_ref();
        let dash = "\u{2014}".to_string();
        for (row, &id) in items.iter().enumerate() {
            let (label, value) = match id {
                0 => ("CPU load", format!("{:.0} %", s.cpu_load_pct)),
                1 => ("RAM", format!("{:.1} / {:.1} GB", s.ram_used_gb, s.ram_total_gb)),
                2 => ("GPU load", gpu.map(|g| format!("{} %", g.load_pct)).unwrap_or(dash.clone())),
                3 => ("GPU temp", gpu.map(|g| format!("{} \u{00B0}C", g.temp_c)).unwrap_or(dash.clone())),
                4 => ("GPU clock", gpu.map(|g| format!("{} MHz", g.clock_mhz)).unwrap_or(dash.clone())),
                _ => ("CPU temp", s.cpu_temp_c.map(|c| format!("{c:.0} \u{00B0}C")).unwrap_or(dash.clone())),
            };
            let y0 = b.y + pad + row as f32 * pitch + (pitch - 104.0 * u) / 2.0;
            let (lsz, vsz) = ((30.0 * u).round().max(6.0) as u32, (40.0 * u).round().max(6.0) as u32);
            self.text(label, b.x + pad, y0 + 34.0 * u, lsz, Weight::Regular, [185, 192, 208], 1.0);
            let vw = self.fonts.width(&value, vsz, Weight::Bold);
            self.text(&value, b.x + b.w - pad - vw, y0 + 38.0 * u, vsz, Weight::Bold, [255, 255, 255], 1.0);
            let (bx, by, bw, bh) = (b.x + pad, y0 + 56.0 * u, b.w - 2.0 * pad, 12.0 * u);
            round_rect(&mut self.layer, bx, by, bw, bh, bh / 2.0, [255, 255, 255, 40]);
            let v = self.bars[id];
            let col = if v < 0.6 { [90, 210, 130, 255] } else if v < 0.85 { [240, 200, 70, 255] } else { [240, 90, 80, 255] };
            if v > 0.01 {
                round_rect(&mut self.layer, bx, by, (bw * v).max(bh), bh, bh / 2.0, col);
            }
        }
    }

    // ------------------------------------------------------------ text

    pub(super) fn draw_text(&mut self, b: Rect, p: &TextProps) {
        if p.text.is_empty() {
            return;
        }
        let col = parse_color(&p.color, [255, 255, 255]);
        let wt = Weight::parse(&p.weight);
        let alpha = p.opacity.clamp(0.0, 1.0);
        let mut px = (p.size.clamp(0.02, 0.3) * self.ch as f32).max(6.0);
        // Shrink to fit the box (wrapping at word boundaries), a few measured iterations.
        let mut lines = Vec::new();
        for _ in 0..8 {
            lines = self.wrap(&p.text, px.round() as u32, wt, b.w);
            let line_h = px * 1.25;
            let total = line_h * lines.len() as f32;
            let widest = lines.iter().map(|l| self.fonts.width(l, px.round() as u32, wt)).fold(0.0f32, f32::max);
            let k = (b.h / total).min(b.w / widest.max(1.0));
            if k >= 1.0 || px <= 6.0 {
                break;
            }
            px = (px * k * 0.98).max(6.0);
        }
        let pxi = px.round() as u32;
        let line_h = px * 1.25;
        let total = line_h * lines.len() as f32;
        let top = b.y + (b.h - total) / 2.0;
        for (i, l) in lines.iter().enumerate() {
            let w = self.fonts.width(l, pxi, wt);
            let x = match p.align.as_str() {
                "center" => b.x + (b.w - w) / 2.0,
                "right" => b.x + b.w - w,
                _ => b.x,
            };
            self.text(l, x, top + line_h * i as f32 + line_h / 2.0 + 0.36 * px, pxi, wt, col, alpha);
        }
    }

    /// Greedy word wrap to `max` px (explicit newlines kept; a word wider than the line is split by characters).
    fn wrap(&mut self, s: &str, px: u32, wt: Weight, max: f32) -> Vec<String> {
        let mut out = Vec::new();
        for para in s.split('\n') {
            let mut cur = String::new();
            for word in para.split(' ') {
                let cand = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                if self.fonts.width(&cand, px, wt) <= max || cur.is_empty() && word.is_empty() {
                    cur = cand;
                    continue;
                }
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                // the word on its own may still be too wide: split it
                let mut piece = String::new();
                for c in word.chars() {
                    piece.push(c);
                    if self.fonts.width(&piece, px, wt) > max && piece.chars().count() > 1 {
                        piece.pop();
                        out.push(std::mem::take(&mut piece));
                        piece.push(c);
                    }
                }
                cur = piece;
            }
            out.push(cur);
        }
        out
    }

    // ------------------------------------------------------------ image

    pub(super) fn draw_image(&mut self, b: Rect, w: &Widget) {
        let p = w.image();
        let Some(path) = resolve_image(&self.theme.theme_dir, &p.file) else { return };
        let cover = p.fit == "cover";
        let (bw, bh) = (b.w.round() as u32, b.h.round() as u32);
        let entry = self.images.entry(path.clone()).or_insert_with(|| ImgEntry {
            mtime: std::fs::metadata(&path).and_then(|m| m.modified()).ok(),
            src: decode_image(&path).map(Arc::new),
            scaled: None,
        });
        let Some(src) = entry.src.clone() else { return };
        if entry.scaled.as_ref().map(|s| (s.0, s.1, s.2)) != Some((bw, bh, cover)) {
            entry.scaled = Some(scale_to_box(&src, bw, bh, cover));
        }
        let Some((_, _, _, pm, ox, oy)) = &entry.scaled else { return };
        let paint = PixmapPaint { opacity: p.opacity.clamp(0.0, 1.0), ..PixmapPaint::default() };
        self.layer.draw_pixmap(b.x as i32 + ox, b.y as i32 + oy, pm.as_ref(), &paint, Transform::identity(), None);
    }
}

pub(super) fn fmt_time(s: f64) -> String {
    let t = s.max(0.0) as u64;
    if t >= 3600 { format!("{}:{:02}:{:02}", t / 3600, (t / 60) % 60, t % 60) } else { format!("{}:{:02}", t / 60, t % 60) }
}

/// `file` relative to the theme folder; absolute paths and `..` are refused.
pub(crate) fn resolve_image(dir: &Path, file: &str) -> Option<PathBuf> {
    let f = Path::new(file.trim());
    if file.trim().is_empty() || f.is_absolute() || f.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return None;
    }
    Some(dir.join(f))
}

/// PNG directly; anything else (jpg, webp, gif, bmp) through the bundled ffmpeg once.
pub(super) fn decode_image(path: &Path) -> Option<Pixmap> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        if let Ok(p) = Pixmap::decode_png(&bytes) {
            return Some(p);
        }
    }
    let ffmpeg = crate::media::ffmpeg::locate();
    if !ffmpeg.exists() {
        return None;
    }
    let mut c = std::process::Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i"]).arg(path).args(["-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let out = c.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    Pixmap::decode_png(&out.stdout).ok()
}

/// 2x2 box average of a premultiplied pixmap.
fn half(src: &Pixmap) -> Pixmap {
    let (w, h) = ((src.width() / 2).max(1), (src.height() / 2).max(1));
    let mut out = Pixmap::new(w, h).unwrap();
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let s = src.data();
    let d = out.data_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (x0, y0) = ((x * 2).min(sw - 1), (y * 2).min(sh - 1));
            let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
            for c in 0..4 {
                let v = s[(y0 * sw + x0) * 4 + c] as u32 + s[(y0 * sw + x1) * 4 + c] as u32 + s[(y1 * sw + x0) * 4 + c] as u32 + s[(y1 * sw + x1) * 4 + c] as u32;
                d[(y * w as usize + x) * 4 + c] = (v / 4) as u8;
            }
        }
    }
    out
}

/// Scale `src` for a box: contain = fit inside (centred), cover = fill the box (centre crop). Returns
/// (box w, box h, cover, pixmap, offset x, offset y) where the offset places the pixmap relative to the box origin.
pub(super) fn scale_to_box(src: &Pixmap, bw: u32, bh: u32, cover: bool) -> (u32, u32, bool, Pixmap, i32, i32) {
    let (sw, sh) = (src.width() as f32, src.height() as f32);
    let k = if cover { (bw as f32 / sw).max(bh as f32 / sh) } else { (bw as f32 / sw).min(bh as f32 / sh) };
    let (tw, th) = ((sw * k).round().max(1.0) as u32, (sh * k).round().max(1.0) as u32);
    let mut cur = src.clone();
    while cur.width() >= tw * 2 && cur.height() >= th * 2 {
        cur = half(&cur);
    }
    let (ow, oh) = if cover { (bw.max(1), bh.max(1)) } else { (tw, th) };
    let mut out = Pixmap::new(ow, oh).unwrap();
    let (ox, oy) = (((ow as f32 - tw as f32) / 2.0), ((oh as f32 - th as f32) / 2.0));
    let t = Transform::from_row(tw as f32 / cur.width() as f32, 0.0, 0.0, th as f32 / cur.height() as f32, ox, oy);
    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
    out.draw_pixmap(0, 0, cur.as_ref(), &paint, t, None);
    let (px, py) = if cover { (0, 0) } else { (((bw as f32 - tw as f32) / 2.0).round() as i32, ((bh as f32 - th as f32) / 2.0).round() as i32) };
    (bw, bh, cover, out, px, py)
}

/// Scale a cover to a `side` x `side` square (centre crop, bilinear), rounded corners, premultiplied RGBA.
/// No cover: a neutral tile with a note-like block.
fn scale_cover(c: Option<&crate::live::nowplaying::Cover>, side: usize) -> Vec<u8> {
    let mut out = vec![0u8; side * side * 4];
    let r = (16.0 * side as f32 / 180.0).max(3.0);
    for y in 0..side {
        for x in 0..side {
            let i = (y * side + x) * 4;
            // rounded-corner coverage
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let cx = fx.clamp(r, side as f32 - r);
            let cy = fy.clamp(r, side as f32 - r);
            let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
            let cov = (r + 0.5 - d).clamp(0.0, 1.0);
            if cov <= 0.0 {
                continue;
            }
            let rgb: [f32; 3] = match c {
                Some(c) if c.w > 0 && c.h > 0 => {
                    let m = c.w.min(c.h) as f32;
                    let (ox, oy) = ((c.w as f32 - m) / 2.0, (c.h as f32 - m) / 2.0);
                    let (sx, sy) = (ox + fx / side as f32 * m - 0.5, oy + fy / side as f32 * m - 0.5);
                    let (x0, y0) = (sx.floor().clamp(0.0, c.w as f32 - 1.0) as usize, sy.floor().clamp(0.0, c.h as f32 - 1.0) as usize);
                    let (x1, y1) = ((x0 + 1).min(c.w - 1), (y0 + 1).min(c.h - 1));
                    let (tx, ty) = ((sx - x0 as f32).clamp(0.0, 1.0), (sy - y0 as f32).clamp(0.0, 1.0));
                    let px = |xx: usize, yy: usize, k: usize| c.rgba[(yy * c.w + xx) * 4 + k] as f32;
                    let mut o = [0.0; 3];
                    for k in 0..3 {
                        let top = px(x0, y0, k) * (1.0 - tx) + px(x1, y0, k) * tx;
                        let bot = px(x0, y1, k) * (1.0 - tx) + px(x1, y1, k) * tx;
                        o[k] = top * (1.0 - ty) + bot * ty;
                    }
                    o
                }
                _ => {
                    let note = (x > side * 2 / 5 && x < side * 3 / 5 && y > side / 4 && y < side * 3 / 4) || (x > side / 3 && x < side * 2 / 5 && y > side * 11 / 20 && y < side * 3 / 4);
                    if note { [200.0, 205.0, 220.0] } else { [58.0, 62.0, 84.0] }
                }
            };
            for k in 0..3 {
                out[i + k] = (rgb[k] * cov) as u8;
            }
            out[i + 3] = (255.0 * cov) as u8;
        }
    }
    out
}

/// Scale a pixmap to exactly `tw` x `th` (box-filter halving while far larger, then bilinear).
pub(crate) fn resize_exact(src: &Pixmap, tw: u32, th: u32) -> Pixmap {
    let (tw, th) = (tw.max(1), th.max(1));
    if src.width() == tw && src.height() == th {
        return src.clone();
    }
    let mut cur = src.clone();
    while cur.width() >= tw * 2 && cur.height() >= th * 2 {
        cur = half(&cur);
    }
    let mut out = Pixmap::new(tw, th).unwrap();
    let t = Transform::from_scale(tw as f32 / cur.width() as f32, th as f32 / cur.height() as f32);
    let paint = PixmapPaint { quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
    out.draw_pixmap(0, 0, cur.as_ref(), &paint, t, None);
    out
}

// ---------------------------------------------------------------- calibration overlay (screen guides)

impl Renderer {
    /// Draw the screen guides over the finished frame: hatched red bands for the left and right edge, a yellow hatched
    /// vertical band for the curve with its centre line and both edge lines, and short labels with px values.
    /// Static vertical lines only (the panel tears on vertical motion). Guide px are on the 2400x1080 canvas.
    /// Temporary: only while calibrating, never part of a normal frame.
    /// The FPS counter: "59.8 / 60 fps" on a dark chip near the top left of the visible picture, drawn straight onto the
    /// finished frame (a few hundred pixels, no layer redraw).
    pub(super) fn draw_fps_overlay(&mut self) {
        let Some((x10, target)) = self.fps_overlay else { return };
        let text = format!("{}.{} / {} fps", x10 / 10, x10 % 10, target);
        let (cw, ch) = (self.cw, self.ch);
        let px = (cw as f32 / 1680.0 * 34.0).round().max(12.0) as u32;
        let vis = self.visible();
        let tw = self.fonts.width(&text, px, Weight::Bold);
        let (x0, y0) = ((vis.0.max(0) as f32 + cw as f32 * 0.10).round() as i64, (vis.1.max(0) as f32 + ch as f32 * 0.06).round() as i64);
        let (w, h) = (tw.ceil() as i64 + 24, px as i64 + 20);
        let d = self.pm.data_mut();
        for y in y0.max(0)..(y0 + h).min(ch as i64) {
            for x in x0.max(0)..(x0 + w).min(cw as i64) {
                let i = (y as usize * cw + x as usize) * 4;
                for c in 0..3 {
                    d[i + c] = (d[i + c] as u32 * 80 / 256) as u8;
                }
            }
        }
        let mut pen = (x0 + 12) as f32;
        let base = (y0 + 10 + px as i64) as f32 - px as f32 * 0.2;
        for c in text.chars() {
            let Renderer { pm, fonts, .. } = self;
            let Some(g) = fonts.glyph(c, px, Weight::Bold) else { continue };
            blend_cov(pm.data_mut(), cw, ch, &g.cov, g.w, g.h, pen.round() as i32 + g.x, base.round() as i32 + g.y, [120, 255, 140], 1.0);
            pen += g.adv;
        }
    }

    pub(super) fn draw_calibration(&mut self) {
        let Some(g) = self.calib.clone() else { return };
        let (cw, ch) = (self.cw, self.ch);
        let k = cw as f32 / crate::live::theme::GUIDE_W;
        let saved = std::mem::replace(&mut self.rects, vec![(0, 0, cw, ch)]);
        self.layer.data_mut().fill(0);
        let (fw, fh) = (cw as f32, ch as f32);
        let side_col = [255u8, 70, 70];
        let curve_col = [255u8, 235, 0];
        let lsz = (26.0 * k / 0.7).round().max(10.0) as u32;
        let mut chips: Vec<(String, f32, f32, [u8; 3])> = Vec::new(); // text, x, baseline, colour
        let band = |layer: &mut tiny_skia::Pixmap, x: f32, w: f32, col: [u8; 3], tint: u8| {
            round_rect(layer, x, 0.0, w, fh, 0.0, [col[0], col[1], col[2], tint]);
            let mut sx = x;
            while sx < x + w {
                round_rect(layer, sx, 0.0, 3.0f32.min(x + w - sx), fh, 0.0, [col[0], col[1], col[2], 70]);
                sx += 14.0;
            }
        };
        // left and right bands
        let s = g.left.size * k;
        if s >= 0.5 {
            band(&mut self.layer, 0.0, s, side_col, 60);
            round_rect(&mut self.layer, s - 3.0, 0.0, 3.0, fh, 0.0, [side_col[0], side_col[1], side_col[2], 255]);
            chips.push((format!("Left  {:.0}", g.left.size), s + 10.0, fh * 0.5, side_col));
        }
        let s = g.right.size * k;
        if s >= 0.5 {
            let label = format!("Right  {:.0}", g.right.size);
            let tw = self.fonts.width(&label, lsz, Weight::Bold);
            band(&mut self.layer, fw - s, s, side_col, 60);
            round_rect(&mut self.layer, fw - s, 0.0, 3.0, fh, 0.0, [side_col[0], side_col[1], side_col[2], 255]);
            chips.push((label, (fw - s - tw - 10.0).max(4.0), fh * 0.5, side_col));
        }
        // curve band: tint + hatch, edge lines, centre line
        let (el, er) = g.curve_edges();
        let (xl, xr, xc) = (el * k, er * k, (crate::live::theme::GUIDE_W / 2.0 + g.curve.offset) * k);
        if xr - xl >= 0.5 {
            band(&mut self.layer, xl, xr - xl, curve_col, 40);
            round_rect(&mut self.layer, xl - 1.5, 0.0, 3.0, fh, 0.0, [curve_col[0], curve_col[1], curve_col[2], 255]);
            round_rect(&mut self.layer, xr - 1.5, 0.0, 3.0, fh, 0.0, [curve_col[0], curve_col[1], curve_col[2], 255]);
        }
        round_rect(&mut self.layer, xc - 2.0, 0.0, 4.0, fh, 0.0, [255, 255, 255, 255]);
        let top = lsz as f32 + 14.0;
        let label = format!("Curve  {:+.0}", g.curve.offset);
        let tw = self.fonts.width(&label, lsz, Weight::Bold);
        chips.push((label, (xc - tw / 2.0).clamp(4.0, fw - tw - 4.0), top, curve_col));
        if g.curve.left >= 1.0 {
            let l = format!("{:.0}", g.curve.left);
            let tw = self.fonts.width(&l, lsz, Weight::Bold);
            chips.push((l, (xl - tw - 10.0).max(4.0), fh * 0.5, curve_col));
        }
        if g.curve.right >= 1.0 {
            let l = format!("{:.0}", g.curve.right);
            let tw = self.fonts.width(&l, lsz, Weight::Bold);
            chips.push((l, (xr + 10.0).min(fw - tw - 4.0), fh * 0.5, curve_col));
        }
        for (text, x, base, col) in chips {
            let tw = self.fonts.width(&text, lsz, Weight::Bold);
            round_rect(&mut self.layer, x - 8.0, base - lsz as f32 - 2.0, tw + 16.0, lsz as f32 + 14.0, 6.0, [0, 0, 0, 190]);
            self.text(&text, x, base, lsz, Weight::Bold, col, 1.0);
        }
        self.composite(1.0);
        self.rects = saved;
        self.layer_key = None;
    }
}

/// Which transport glyph the music card shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransportIcon {
    Play,
    Pause,
}

/// Player-button convention, like every player: two bars while playing, a triangle while paused.
pub(super) fn transport_icon(playing: bool) -> TransportIcon {
    if playing { TransportIcon::Pause } else { TransportIcon::Play }
}

#[cfg(test)]
mod music_icon_tests {
    use super::*;
    use crate::live::{LH, LW};
    use crate::live::sensors::Sensors;
    use crate::live::theme::{Theme, ThemeLive};
    use std::sync::Mutex;

    #[test]
    fn bars_while_playing_triangle_while_paused() {
        assert_eq!(transport_icon(true), TransportIcon::Pause);
        assert_eq!(transport_icon(false), TransportIcon::Play);
    }

    fn frame(playing: bool) -> Vec<u8> {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        t.theme.widgets.push(Widget { id: "m".into(), kind: "music".into(), ..Widget::default() });
        let mut r = Renderer::new(Arc::new(Mutex::new(Sensors::default())), None);
        r.set_sample_mode(true);
        r.set_theme(t);
        let np = NowPlaying::demo("Midnight City", "M83", playing, 241.0, 241.0); // at the end: the elapsed time reads the same in both states
        r.set_music_source(Some(Arc::new(Mutex::new(Some(np)))));
        r.render().0.to_vec()
    }

    /// Runs of bright icon-coloured pixels along the middle row of the changed area.
    fn runs(f: &[u8], rows: (usize, usize), cols: (usize, usize)) -> usize {
        let y = (rows.0 + rows.1) / 2;
        let mut n = 0;
        let mut prev = false;
        for x in cols.0..=cols.1 {
            let i = (y * LW + x) * 4;
            let on = f[i] > 200 && f[i + 1] > 200 && f[i + 2] > 215;
            if on && !prev {
                n += 1;
            }
            prev = on;
        }
        n
    }

    #[test]
    fn the_card_draws_bars_when_playing_and_a_triangle_when_paused() {
        let (a, b) = (frame(true), frame(false));
        let diff: Vec<usize> = (0..LW * LH).filter(|i| a[i * 4..i * 4 + 3] != b[i * 4..i * 4 + 3]).collect();
        assert!(!diff.is_empty(), "the two states differ");
        let (x0, x1) = (diff.iter().map(|i| i % LW).min().unwrap(), diff.iter().map(|i| i % LW).max().unwrap());
        let (y0, y1) = (diff.iter().map(|i| i / LW).min().unwrap(), diff.iter().map(|i| i / LW).max().unwrap());
        assert!(x1 - x0 < 120 && y1 - y0 < 120, "only the icon changes: {x0}-{x1} x {y0}-{y1}");
        assert_eq!(runs(&a, (y0, y1), (x0, x1)), 2, "playing: two bars with a gap");
        assert_eq!(runs(&b, (y0, y1), (x0, x1)), 1, "paused: one solid triangle row");
    }
}
