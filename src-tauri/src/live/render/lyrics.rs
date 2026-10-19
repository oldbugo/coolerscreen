//! Drawing of the `lyrics` widget (4.x E M5): the synced lyrics of the song that plays, as a "karaoke line". The current line is
//! centred, big and bold, drawn dim with a bright (accent coloured) copy revealed from left to right over the line's time; the
//! previous and next line are small and dim at fixed places. Nothing moves up or down (panel rule): on a line change the old set
//! of lines fades out while the new one fades in, in the same places; the instrumental dots only change their opacity.
//!
//! CPU: a line is rendered ONCE into two pictures (dim, bright) per (line, size, colours); a frame is only a blit of the dim
//! picture, a clipped blit of the bright one with a soft edge, and the alpha-blended neighbours. The cover colours (accent,
//! gradient) are worked out once per track; the gradient background is one cached strip that is shifted sideways.
//! The data layer (lookup, cache, offsets, clock) is `live/lyrics.rs`; this file never touches the network itself.
use super::{blend_tile, round_rect, Rect, Renderer, Weight};
use crate::live::lyrics::{self as ly, Lyrics, LyricsClock, LyricsLoader, LyricsProps, Source, State, Status};
use crate::live::nowplaying::{Cover, NowPlaying};
use crate::live::theme::{parse_color, Widget};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tiny_skia::Pixmap;

/// Cross-fade between two sets of lines.
const FADE_S: f64 = 0.25;
/// Alpha of the not yet sung part of the current line, and of the previous / next line.
const DIM_A: f32 = 0.45;
const PREV_A: f32 = 0.35;
const NEXT_A: f32 = 0.45;
/// One cycle of the gradient drift.
const DRIFT_S: f64 = 20.0;
const FALLBACK_ACCENT: [u8; 3] = [90, 200, 255];
const GREY: [u8; 3] = [160, 166, 182];
/// Transparent border around a rendered line (room for the text shadow).
const MARGIN: u32 = 4;
/// Line height as a multiple of the font size.
const LH: f32 = 1.22;

/// The made-up song of the editor preview (my own words).
pub(super) fn demo_lyrics() -> Arc<Lyrics> {
    let l = |s: f64, e: f64, t: &str| ly::Line { start_s: s, end_s: e, text: t.into() };
    Arc::new(Lyrics {
        lines: vec![
            l(0.0, 6.0, "Lights are coming up across the bay"),
            l(6.0, 13.0, "We keep moving till the morning comes"),
            l(13.0, 20.0, "Every window glowing gold"),
            l(20.0, 27.0, "Hold on, we are nearly home"),
        ],
        source: Source::Lrclib,
        lrclib_id: None,
        track_name: "Sample".into(),
        artist_name: "Sample".into(),
    })
}
/// Position in the preview: half way through the second line.
const DEMO_POS: f64 = 9.5;

// ------------------------------------------------------------------ cache

struct Item {
    pm: Pixmap,
    x: i32,
    y: i32,
}

/// The current line: dim and bright copies (same size), where they sit (relative to the box) and the geometry of its text rows.
struct Fill {
    dim: Pixmap,
    bright: Pixmap,
    x: i32,
    y: i32,
    /// Per text row: its width, its left edge inside the picture, and the picture rows it owns.
    widths: Vec<f32>,
    lefts: Vec<f32>,
    rows: Vec<(usize, usize)>,
}

struct Dots {
    cx: f32,
    cy: f32,
    d: f32,
}

/// Everything shown for one thing at a time (a line, a gap, a message). Pictures are relative to the box.
struct Scene {
    id: u64,
    /// Size / colour signature: a different one rebuilds the pictures in place (no fade).
    sig: u64,
    items: Vec<Item>,
    fill: Option<Fill>,
    dots: Option<Dots>,
    /// How much of the line is bright (0..1); frozen on a scene that is fading out.
    prog: f32,
    /// A line or a gap: it changes with time while the song plays.
    live: bool,
}

#[derive(Clone)]
struct Auto {
    tid: u64,
    has: bool,
    accent: [u8; 3],
    cols: [[u8; 3]; 3],
}

#[derive(Default)]
struct Slot {
    cur: Option<Scene>,
    old: Option<Scene>,
    /// Clock value when `cur` replaced `old`.
    changed: f64,
    auto: Option<Auto>,
    grad: Option<((u64, u32, u32), Pixmap)>,
}

#[derive(Default)]
pub(super) struct LyricsCache {
    loader: Option<LyricsLoader>,
    clock: LyricsClock,
    /// A fixed animation clock (seconds): tests and sample pictures.
    pub(super) clock_override: Option<f64>,
    /// A fixed result and position (offset included) instead of the loader and the player: tests and sample pictures.
    pub(super) force: Option<(State, f64)>,
    /// The last draw had something that changes with time (a filling line, dots, a fade): redraw at 30 Hz.
    pub(super) animating: bool,
    now: f64,
    offset: Option<(String, u64, f64)>,
    slots: HashMap<String, Slot>,
    demo: Option<Arc<Lyrics>>,
}

impl LyricsCache {
    /// The animation clock for this frame: the override, else 0 in sample mode (a still preview), else seconds since start.
    pub(super) fn set_clock(&mut self, t: f32, sample: bool) {
        self.now = self.clock_override.unwrap_or(if sample { 0.0 } else { t as f64 });
    }

    /// Changes at 30 Hz while something animates, so the frame is redrawn even without a video.
    pub(super) fn motion_key(&self) -> u64 {
        if self.animating { ((self.now * 30.0) as u64) << 36 } else { 0 }
    }

    /// The stored offset of a track; re-read when an offset was changed anywhere (the editor's buttons).
    fn offset_for(&mut self, key: &str, dir: &Path) -> f64 {
        let g = ly::offset_generation();
        if let Some((k, gg, v)) = &self.offset {
            if k == key && *gg == g {
                return *v;
            }
        }
        let v = ly::get_offset(dir, key);
        self.offset = Some((key.to_string(), g, v));
        v
    }
}

/// What to show right now.
struct Input {
    key: String,
    state: State,
    /// Playback position in seconds, offset applied.
    pos: f64,
    title: String,
    artist: String,
    playing: bool,
}

enum Msg {
    Loading,
    NoSynced,
    Off,
    Instrumental,
    Nothing,
}

enum Content {
    Line { idx: usize, prog: f64 },
    Gap { after: Option<usize> },
    Msg(Msg),
}

struct Geo {
    maxw: f32,
    cur_px: u32,
    small_px: u32,
    cx: f32,
    cy: f32,
}

fn geo(b: Rect, neigh: bool) -> Geo {
    let pad = (b.w.min(b.h) * 0.06).max(6.0);
    let (iw, ih) = ((b.w - 2.0 * pad).max(1.0), (b.h - 2.0 * pad).max(1.0));
    let by_h = if neigh { ih * 0.26 } else { ih * 0.34 };
    let cur = by_h.min(iw * 0.075).floor().max(8.0) as u32;
    Geo { maxw: iw * 0.96, cur_px: cur, small_px: ((cur as f32 * 0.55).round() as u32).max(6), cx: b.w / 2.0, cy: b.h / 2.0 }
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || c.is_whitespace()).take(300).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hash_of<T: Hash>(v: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

// ------------------------------------------------------------------ colours from the cover

fn rgb_to_hsv(c: [f32; 3]) -> (f32, f32, f32) {
    let (r, g, b) = (c[0] / 255.0, c[1] / 255.0, c[2] / 255.0);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let d = mx - mn;
    let h = if d <= 1e-6 {
        0.0
    } else if mx == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (h, if mx <= 1e-6 { 0.0 } else { d / mx }, mx)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> [u8; 3] {
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    [((r + m) * 255.0).round() as u8, ((g + m) * 255.0).round() as u8, ((b + m) * 255.0).round() as u8]
}

/// The most saturated, well-lit colour of a small sample of the cover (the accent) and up to three colours for the gradient.
/// `None` when the cover has no usable colour (grey, black, white, or no pixels).
fn palette(c: &Cover) -> Option<([u8; 3], [[u8; 3]; 3])> {
    if c.w == 0 || c.h == 0 || c.rgba.len() < c.w * c.h * 4 {
        return None;
    }
    const N: usize = 24;
    // six hue buckets: weight and colour sum
    let mut acc = [(0.0f32, [0.0f32; 3]); 6];
    let mut valid = 0;
    for iy in 0..N {
        for ix in 0..N {
            let (x, y) = ((ix * c.w) / N + c.w / (2 * N), (iy * c.h) / N + c.h / (2 * N));
            let p = &c.rgba[(y.min(c.h - 1) * c.w + x.min(c.w - 1)) * 4..][..3];
            let rgb = [p[0] as f32, p[1] as f32, p[2] as f32];
            let (h, s, v) = rgb_to_hsv(rgb);
            if s < 0.25 || v < 0.25 {
                continue;
            }
            valid += 1;
            let k = ((h / 60.0) as usize).min(5);
            let w = s * v;
            acc[k].0 += w;
            for i in 0..3 {
                acc[k].1[i] += rgb[i] * w;
            }
        }
    }
    if valid < 12 {
        return None;
    }
    let mut order: Vec<usize> = (0..6).filter(|&k| acc[k].0 > 0.0).collect();
    order.sort_by(|&a, &b| acc[b].0.partial_cmp(&acc[a].0).unwrap_or(std::cmp::Ordering::Equal));
    let mean = |k: usize| [acc[k].1[0] / acc[k].0, acc[k].1[1] / acc[k].0, acc[k].1[2] / acc[k].0];
    let best = mean(*order.first()?);
    let (h, s, v) = rgb_to_hsv(best);
    let accent = hsv_to_rgb(h, s.max(0.5), v.max(0.88));
    let mut cols: Vec<[u8; 3]> = order.iter().take(3).map(|&k| mean(k).map(|x| x.round() as u8)).collect();
    // a cover of one colour: neighbouring hues of the accent make the drift visible
    for shift in [35.0, -35.0] {
        if cols.len() < 3 {
            cols.push(hsv_to_rgb(h + shift, s.max(0.5), v.max(0.6)));
        }
    }
    Some((accent, [cols[0], cols[1], cols[2]]))
}

/// Gradient strip, `w` x `h`, periodic along x (so a sideways shift never shows a seam), darkened for contrast.
fn gradient_strip(cols: &[[u8; 3]; 3], w: u32, h: u32) -> Pixmap {
    let (w, h) = (w.max(1), h.max(1));
    let mut pm = Pixmap::new(w, h).unwrap();
    let dark = 0.42f32;
    for x in 0..w as usize {
        let t = x as f32 / w as f32 * 3.0;
        let seg = (t.floor() as usize) % 3;
        let f = t - t.floor();
        let s = f * f * (3.0 - 2.0 * f);
        let (a, b) = (cols[seg], cols[(seg + 1) % 3]);
        let base = [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - s) + b[i] as f32 * s) * dark);
        for y in 0..h as usize {
            let k = 1.0 - 0.25 * (y as f32 / h as f32);
            let d = &mut pm.data_mut()[(y * w as usize + x) * 4..][..4];
            d.copy_from_slice(&[(base[0] * k) as u8, (base[1] * k) as u8, (base[2] * k) as u8, 255]);
        }
    }
    pm
}

// ------------------------------------------------------------------ blits

/// Premultiplied source-over of tile pixel `s` scaled by `k`/255 onto `dst[i..i+4]`.
#[inline]
fn over(dst: &mut [u8], i: usize, s: &[u8], k: u32) {
    let sa = s[3] as u32 * k / 255;
    let inv = 255 - sa;
    for c in 0..3 {
        dst[i + c] = ((s[c] as u32 * k / 255) + dst[i + c] as u32 * inv / 255).min(255) as u8;
    }
    dst[i + 3] = (sa + dst[i + 3] as u32 * inv / 255).min(255) as u8;
}

/// Rows `r0..r1` of `tile` at (x0, y0), visible only left of `edge` (tile columns) with a soft edge `sw` px wide.
fn blit_soft(dst: &mut [u8], dw: usize, dh: usize, tile: &Pixmap, x0: i32, y0: i32, r0: usize, r1: usize, edge: f32, sw: f32, fade: u32) {
    let tw = tile.width() as usize;
    let c_end = ((edge + sw / 2.0).ceil().max(0.0) as usize).min(tw);
    if c_end == 0 {
        return;
    }
    let facs: Vec<u32> = (0..c_end)
        .map(|c| {
            let t = ((edge - c as f32) / sw + 0.5).clamp(0.0, 1.0);
            (t * t * (3.0 - 2.0 * t) * 256.0) as u32
        })
        .collect();
    let td = tile.data();
    for row in r0..r1.min(tile.height() as usize) {
        let y = y0 + row as i32;
        if y < 0 || y >= dh as i32 {
            continue;
        }
        for c in 0..c_end {
            let x = x0 + c as i32;
            if x < 0 || x >= dw as i32 {
                continue;
            }
            let sp = &td[(row * tw + c) * 4..][..4];
            if sp[3] == 0 {
                continue;
            }
            let k = fade * facs[c] / 256;
            if k == 0 {
                continue;
            }
            over(dst, (y as usize * dw + x as usize) * 4, sp, k);
        }
    }
}

/// Coverage (0..1) of pixel (x, y) of a `w` x `h` rounded rectangle with corner radius `r`.
fn corner_cov(x: usize, y: usize, w: usize, h: usize, r: f32) -> f32 {
    let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
    let dx = if fx < r { r - fx } else if fx > w as f32 - r { fx - (w as f32 - r) } else { 0.0 };
    let dy = if fy < r { r - fy } else if fy > h as f32 - r { fy - (h as f32 - r) } else { 0.0 };
    if dx <= 0.0 || dy <= 0.0 {
        return 1.0;
    }
    (r + 0.5 - (dx * dx + dy * dy).sqrt()).clamp(0.0, 1.0)
}

/// The periodic `strip` shifted left by `shift` columns, with rounded corners, at (x0, y0) of the layer.
fn blit_shifted(dst: &mut [u8], dw: usize, dh: usize, strip: &Pixmap, shift: usize, x0: i32, y0: i32, radius: f32, fade: u32) {
    let (w, h) = (strip.width() as usize, strip.height() as usize);
    let r = radius.clamp(0.0, w.min(h) as f32 / 2.0);
    let ri = r.ceil() as usize;
    let sd = strip.data();
    for y in 0..h {
        let ty = y0 + y as i32;
        if ty < 0 || ty >= dh as i32 {
            continue;
        }
        let edge_row = y < ri || y + ri >= h;
        for x in 0..w {
            let tx = x0 + x as i32;
            if tx < 0 || tx >= dw as i32 {
                continue;
            }
            let k = if edge_row && (x < ri || x + ri >= w) { (fade as f32 * corner_cov(x, y, w, h, r)) as u32 } else { fade };
            if k == 0 {
                continue;
            }
            let sx = (x + shift) % w;
            over(dst, (ty as usize * dw + tx as usize) * 4, &sd[(y * w + sx) * 4..][..4], k);
        }
    }
}

// ------------------------------------------------------------------ the renderer side

impl Renderer {
    /// Render into a fresh transparent `w` x `h` picture: the text routine draws into the widget layer, so the layer is swapped for
    /// the picture for the moment.
    fn paint_on(&mut self, w: u32, h: u32, f: impl FnOnce(&mut Renderer)) -> Pixmap {
        let (w, h) = (w.clamp(1, 8000), h.clamp(1, 4000));
        let mut pm = Pixmap::new(w, h).unwrap();
        std::mem::swap(&mut self.layer, &mut pm);
        let (ocw, och) = (self.cw, self.ch);
        self.cw = w as usize;
        self.ch = h as usize;
        f(self);
        self.cw = ocw;
        self.ch = och;
        std::mem::swap(&mut self.layer, &mut pm);
        pm
    }

    /// A single line of text as a picture (shortened with an ellipsis to `maxw`), and its text width.
    fn small_line(&mut self, text: &str, px: u32, maxw: f32, rgb: [u8; 3], alpha: f32) -> Option<(Pixmap, f32)> {
        let s = self.fit(&norm(text), px, Weight::Bold, maxw);
        if s.is_empty() {
            return None;
        }
        let w = self.fonts.width(&s, px, Weight::Bold);
        let lh = (px as f32 * LH).round() as u32;
        let pm = self.paint_on(w.ceil() as u32 + 2 * MARGIN, lh + 2 * MARGIN, |r| {
            r.text(&s, MARGIN as f32, MARGIN as f32 + lh as f32 / 2.0 + 0.36 * px as f32, px, Weight::Bold, rgb, alpha);
        });
        Some((pm, w))
    }

    /// Break `text` into one or two rows no wider than `maxw` at `px`: the break that makes the wider row narrowest. `None` when
    /// it does not fit two rows.
    fn wrap2(&mut self, text: &str, px: u32, maxw: f32) -> Option<Vec<String>> {
        if self.fonts.width(text, px, Weight::Bold) <= maxw {
            return Some(vec![text.to_string()]);
        }
        let spaced = text.contains(' ');
        let units: Vec<&str> = if spaced { text.split(' ').collect() } else { text.char_indices().map(|(i, c)| &text[i..i + c.len_utf8()]).collect() };
        let sep = if spaced { " " } else { "" };
        let mut best: Option<(f32, usize)> = None;
        for k in 1..units.len() {
            let (a, b) = (units[..k].join(sep), units[k..].join(sep));
            let (wa, wb) = (self.fonts.width(&a, px, Weight::Bold), self.fonts.width(&b, px, Weight::Bold));
            if wa <= maxw && wb <= maxw {
                let m = wa.max(wb);
                if best.map_or(true, |(bm, _)| m < bm) {
                    best = Some((m, k));
                }
            }
        }
        best.map(|(_, k)| vec![units[..k].join(sep), units[k..].join(sep)])
    }

    /// The font size and rows for a line: the base size when it fits two rows, else smaller down to 70 %, else two rows with
    /// the end of the second shortened by an ellipsis.
    fn wrap_fit(&mut self, text: &str, base_px: u32, maxw: f32) -> (u32, Vec<String>) {
        let min = ((base_px as f32 * 0.7).round() as u32).max(8).min(base_px);
        let mut px = base_px;
        loop {
            if let Some(r) = self.wrap2(text, px, maxw) {
                return (px, r);
            }
            if px <= min {
                break;
            }
            px = ((px as f32 * 0.94).floor() as u32).max(min);
        }
        let spaced = text.contains(' ');
        let units: Vec<&str> = if spaced { text.split(' ').collect() } else { text.char_indices().map(|(i, c)| &text[i..i + c.len_utf8()]).collect() };
        let sep = if spaced { " " } else { "" };
        let mut k = 0;
        while k < units.len() && self.fonts.width(&units[..=k].join(sep), px, Weight::Bold) <= maxw {
            k += 1;
        }
        if k == 0 {
            return (px, vec![self.fit(text, px, Weight::Bold, maxw)]);
        }
        let rest = units[k..].join(sep);
        let second = self.fit(&rest, px, Weight::Bold, maxw);
        (px, vec![units[..k].join(sep), second])
    }

    /// Where the next / previous small line goes: centre y (relative to the box) for a current block of height `block_h`.
    fn neighbour_y(g: &Geo, block_h: f32, small_h: f32, below: bool) -> f32 {
        let off = block_h / 2.0 + small_h / 2.0 + g.small_px as f32 * 0.15;
        if below { g.cy + off } else { g.cy - off }
    }

    fn neighbour_item(&mut self, g: &Geo, text: &str, block_h: f32, below: bool, alpha: f32) -> Option<Item> {
        let (pm, _) = self.small_line(text, g.small_px, g.maxw, [255, 255, 255], alpha)?;
        let cy = Self::neighbour_y(g, block_h, g.small_px as f32 * LH, below);
        Some(Item { x: (g.cx - pm.width() as f32 / 2.0).round() as i32, y: (cy - pm.height() as f32 / 2.0).round() as i32, pm })
    }

    /// Build the pictures of a scene for the box `b`.
    #[allow(clippy::too_many_arguments)]
    fn build_scene(&mut self, b: Rect, g: &Geo, input: Option<&Input>, content: &Content, lyrics: Option<&Lyrics>, accent: [u8; 3], neigh: bool, id: u64, sig: u64) -> Scene {
        let mut sc = Scene { id, sig, items: Vec::new(), fill: None, dots: None, prog: 0.0, live: false };
        let first_line = || match input {
            Some(i) if !i.title.is_empty() && !i.artist.is_empty() => format!("{} \u{b7} {}", i.title, i.artist),
            Some(i) => i.title.clone(),
            None => "Nothing playing".to_string(),
        };
        match content {
            Content::Line { idx, prog } => {
                let l = lyrics.expect("a line needs lyrics");
                let text = norm(&l.lines[*idx].text);
                let (px, rows) = self.wrap_fit(&text, g.cur_px, g.maxw);
                let widths: Vec<f32> = rows.iter().map(|s| self.fonts.width(s, px, Weight::Bold)).collect();
                let wmax = widths.iter().cloned().fold(1.0f32, f32::max);
                let lh = (px as f32 * LH).round() as u32;
                let (bw, bh) = (wmax.ceil() as u32 + 2 * MARGIN, lh * rows.len() as u32 + 2 * MARGIN);
                let paint = |this: &mut Renderer, rgb: [u8; 3], al: f32| {
                    this.paint_on(bw, bh, |r| {
                        for (i, s) in rows.iter().enumerate() {
                            let x = MARGIN as f32 + (wmax - widths[i]) / 2.0;
                            r.text(s, x, MARGIN as f32 + (i as u32 * lh) as f32 + lh as f32 / 2.0 + 0.36 * px as f32, px, Weight::Bold, rgb, al);
                        }
                    })
                };
                let dim = paint(self, [255, 255, 255], DIM_A);
                let bright = paint(self, accent, 1.0);
                let n = rows.len();
                let spans = (0..n)
                    .map(|i| {
                        let r0 = if i == 0 { 0 } else { (MARGIN + i as u32 * lh) as usize };
                        let r1 = if i + 1 == n { bh as usize } else { (MARGIN + (i as u32 + 1) * lh) as usize };
                        (r0, r1)
                    })
                    .collect();
                let block_h = (lh * n as u32) as f32;
                sc.fill = Some(Fill {
                    x: (g.cx - bw as f32 / 2.0).round() as i32,
                    y: (g.cy - bh as f32 / 2.0).round() as i32,
                    lefts: widths.iter().map(|w| MARGIN as f32 + (wmax - w) / 2.0).collect(),
                    widths,
                    rows: spans,
                    dim,
                    bright,
                });
                sc.prog = *prog as f32;
                sc.live = true;
                if neigh {
                    let prev = l.lines[..*idx].iter().rev().find(|x| !ly::is_gap_text(&x.text)).map(|x| x.text.clone());
                    let next = l.lines[*idx + 1..].iter().find(|x| !ly::is_gap_text(&x.text)).map(|x| x.text.clone());
                    if let Some(t) = prev {
                        sc.items.extend(self.neighbour_item(g, &t, block_h, false, PREV_A));
                    }
                    if let Some(t) = next {
                        sc.items.extend(self.neighbour_item(g, &t, block_h, true, NEXT_A));
                    }
                }
            }
            Content::Gap { after } => {
                sc.live = true;
                let d = (g.cur_px as f32 * 0.3).round().max(3.0);
                sc.dots = Some(Dots { cx: g.cx, cy: g.cy, d });
                if neigh {
                    if let Some(l) = lyrics {
                        let from = after.map(|i| i + 1).unwrap_or(0);
                        let next = l.lines.get(from..).and_then(|s| s.iter().find(|x| !ly::is_gap_text(&x.text))).map(|x| x.text.clone());
                        if let Some(t) = next {
                            sc.items.extend(self.neighbour_item(g, &t, g.cur_px as f32 * LH, true, NEXT_A));
                        }
                    }
                }
            }
            Content::Msg(m) => {
                let (s1, s2): (u32, u32) = (((g.cur_px as f32 * 0.55).round() as u32).max(6), ((g.cur_px as f32 * 0.42).round() as u32).max(6));
                let rows: Vec<(String, u32)> = match m {
                    Msg::Loading => vec![(first_line(), s1)],
                    Msg::NoSynced => vec![(first_line(), s1), ("No synced lyrics".into(), s2)],
                    Msg::Off => vec![(first_line(), s1), ("Lyrics lookup is off".into(), s2)],
                    Msg::Instrumental => vec![("Instrumental".into(), ((g.cur_px as f32 * 0.8).round() as u32).max(6))],
                    Msg::Nothing => vec![("Nothing playing".into(), s1)],
                };
                let gap = s1 as f32 * 0.25;
                let heights: Vec<f32> = rows.iter().map(|r| r.1 as f32 * LH).collect();
                let total = heights.iter().sum::<f32>() + gap * (rows.len() - 1) as f32;
                let mut y = g.cy - total / 2.0;
                for ((t, px), h) in rows.iter().zip(heights) {
                    if let Some((pm, _)) = self.small_line(t, *px, g.maxw, GREY, 1.0) {
                        sc.items.push(Item { x: (g.cx - pm.width() as f32 / 2.0).round() as i32, y: (y + h / 2.0 - pm.height() as f32 / 2.0).round() as i32, pm });
                    }
                    y += h + gap;
                }
            }
        }
        let _ = b;
        sc
    }

    /// Draw one scene at `alpha`.
    fn draw_scene(&mut self, sc: &Scene, b: Rect, alpha: f32, now: f64) {
        let fade = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
        if fade == 0 {
            return;
        }
        let (bx, by) = (b.x.round() as i32, b.y.round() as i32);
        let Renderer { layer, cw, ch, .. } = self;
        let (cw, ch) = (*cw, *ch);
        for it in &sc.items {
            blend_tile(layer.data_mut(), cw, ch, it.pm.data(), bx + it.x, by + it.y, it.pm.width() as usize, it.pm.height() as usize, fade);
        }
        if let Some(f) = &sc.fill {
            let (x0, y0) = (bx + f.x, by + f.y);
            blend_tile(layer.data_mut(), cw, ch, f.dim.data(), x0, y0, f.dim.width() as usize, f.dim.height() as usize, fade);
            let total: f32 = f.widths.iter().sum();
            let mut d = sc.prog.clamp(0.0, 1.0) * total;
            for i in 0..f.widths.len() {
                let wi = f.widths[i].max(1.0);
                let frac = (d / wi).clamp(0.0, 1.0);
                d = (d - wi).max(0.0);
                if frac <= 0.0 {
                    continue;
                }
                let sw = (wi * 0.04).max(6.0);
                let edge = if frac >= 0.9999 { f32::INFINITY } else { f.lefts[i] - sw / 2.0 + frac * (wi + sw) };
                blit_soft(layer.data_mut(), cw, ch, &f.bright, x0, y0, f.rows[i].0, f.rows[i].1, edge, sw, fade);
            }
        }
        if let Some(dt) = &sc.dots {
            // three dots, each lit in turn: opacity only
            for k in 0..3 {
                let u = ((now - k as f64 * 0.3).rem_euclid(1.5) / 1.5) as f32;
                let lit = if u < 0.5 { (u / 0.5 * std::f32::consts::PI).sin() } else { 0.0 };
                let al = (0.25 + 0.75 * lit) * alpha;
                let x = b.x + dt.cx + (k as f32 - 1.0) * dt.d * 2.2 - dt.d / 2.0;
                round_rect(layer, x, b.y + dt.cy - dt.d / 2.0, dt.d, dt.d, dt.d / 2.0, [255, 255, 255, (al.clamp(0.0, 1.0) * 255.0) as u8]);
            }
        }
    }

    /// The result and position to show, from the loader and the player (live), from a made-up song (preview), or forced (tests).
    fn lyrics_input(&mut self, np: Option<&NowPlaying>, p: &LyricsProps) -> Option<Input> {
        let n = np?;
        let (title, artist) = (n.title.clone(), n.artist.clone());
        if let Some((st, pos)) = self.lyr.force.clone() {
            return Some(Input { key: "forced".into(), state: st, pos, title, artist, playing: n.playing });
        }
        if self.sample {
            let l = self.lyr.demo.get_or_insert_with(demo_lyrics).clone();
            return Some(Input { key: "demo".into(), state: State::Found(l), pos: DEMO_POS, title, artist, playing: false });
        }
        let loader = self.lyr.loader.get_or_insert_with(|| LyricsLoader::new(ly::default_dir())).clone();
        let folder = p.folder.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from);
        loader.configure(p.online, folder);
        let key = loader.request(n);
        if key.is_empty() {
            return None;
        }
        let state = loader.get(&key);
        let off = self.lyr.offset_for(&key, loader.dir());
        let pos = self.lyr.clock.feed_np(n) + off;
        let source = match &state {
            State::Found(l) => Some(if l.source == Source::Sidecar { "file" } else { "lrclib" }),
            _ => None,
        };
        ly::publish_status(Status {
            track: Some(if artist.is_empty() { title.clone() } else { format!("{title} \u{b7} {artist}") }),
            key: key.clone(),
            state: ly::state_name(&state),
            source,
            offset: off,
        });
        Some(Input { key, state, pos, title, artist, playing: n.playing })
    }

    /// The cover colours of the track, worked out once per track.
    fn lyrics_look(&mut self, slot: &mut Slot, np: Option<&NowPlaying>) {
        if np.is_none() && slot.auto.is_some() {
            return;
        }
        let tid = np.map(|n| n.track_id).unwrap_or(0);
        let cover = np.and_then(|n| n.cover.as_deref());
        let has = cover.is_some();
        if slot.auto.as_ref().is_some_and(|a| a.tid == tid && a.has == has) {
            return;
        }
        let (accent, cols) = cover.and_then(palette).unwrap_or((FALLBACK_ACCENT, [[40, 60, 130], [90, 50, 130], [30, 100, 120]]));
        slot.auto = Some(Auto { tid, has, accent, cols });
        slot.grad = None;
    }

    pub(super) fn draw_lyrics(&mut self, b: Rect, w: &Widget, np: Option<&NowPlaying>) {
        let a = self.music_alpha.clamp(0.0, 1.0);
        if a < 0.01 {
            return;
        }
        let p = w.lyrics();
        let input = self.lyrics_input(np, &p);
        if np.is_none() && !self.sample && self.lyr.force.is_none() {
            ly::publish_status(Status::none());
        }
        let mut slot = self.lyr.slots.remove(&w.id).unwrap_or_default();
        self.lyrics_frame(&mut slot, b, &p, np, input, a);
        self.lyr.slots.insert(w.id.clone(), slot);
    }

    fn lyrics_frame(&mut self, slot: &mut Slot, b: Rect, p: &LyricsProps, np: Option<&NowPlaying>, input: Option<Input>, a: f32) {
        let now = self.lyr.now;
        self.lyrics_look(slot, np);
        let auto = slot.auto.clone().unwrap();
        let accent = p.accent.as_deref().map(|s| parse_color(s, FALLBACK_ACCENT)).unwrap_or(auto.accent);
        let g = geo(b, p.neighbours);

        // ---- background
        let sc = (self.cw as f32 / 1680.0).max(0.1);
        let rad = self.card_radius(b, (24.0 * sc).min(b.h / 4.0));
        match p.background.as_str() {
            "none" => {}
            "gradient" => {
                let (bw, bh) = (b.w.round().max(1.0) as u32, b.h.round().max(1.0) as u32);
                let k = (auto.tid, bw, bh);
                if slot.grad.as_ref().map(|x| x.0) != Some(k) {
                    slot.grad = Some((k, gradient_strip(&auto.cols, bw, bh)));
                }
                let strip = &slot.grad.as_ref().unwrap().1;
                let shift = ((now / DRIFT_S).rem_euclid(1.0) * bw as f64) as usize % bw as usize;
                let Renderer { layer, cw, ch, .. } = self;
                blit_shifted(layer.data_mut(), *cw, *ch, strip, shift, b.x.round() as i32, b.y.round() as i32, rad, (a * 255.0 * 0.92) as u32);
            }
            _ => {
                round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, [0, 0, 0, (96.0 * a) as u8]);
                round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, [255, 255, 255, (26.0 * a) as u8]);
            }
        }

        // ---- what to show; keep the last picture while the song is gone and the item fades out
        let keep = input.is_none() && slot.cur.is_some();
        if !keep {
            let lyrics: Option<Arc<Lyrics>> = match input.as_ref().map(|i| &i.state) {
                Some(State::Found(l)) => Some(l.clone()),
                _ => None,
            };
            let content = match input.as_ref().map(|i| &i.state) {
                Some(State::Found(l)) => {
                    let loc = ly::locate(&l.lines, input.as_ref().unwrap().pos);
                    match loc.index {
                        None => Content::Gap { after: None },
                        Some(i) if loc.gap => Content::Gap { after: Some(i) },
                        Some(i) => Content::Line { idx: i, prog: loc.progress },
                    }
                }
                Some(State::Loading) => Content::Msg(Msg::Loading),
                Some(State::NotFound) | Some(State::NoSynced) => Content::Msg(Msg::NoSynced),
                Some(State::Instrumental) => Content::Msg(Msg::Instrumental),
                Some(State::Off) => Content::Msg(Msg::Off),
                None => Content::Msg(Msg::Nothing),
            };
            let key = input.as_ref().map(|i| i.key.as_str()).unwrap_or("");
            let id = match &content {
                Content::Line { idx, .. } => hash_of(&(key, 1u8, *idx)),
                Content::Gap { after } => hash_of(&(key, 2u8, after.map(|x| x as i64).unwrap_or(-1))),
                Content::Msg(m) => hash_of(&(key, 3u8, std::mem::discriminant(m))),
            };
            let sig = hash_of(&(b.w.to_bits(), b.h.to_bits(), p.neighbours, accent, self.cw));
            if slot.cur.as_ref().map(|s| (s.id, s.sig)) != Some((id, sig)) {
                let scene = self.build_scene(b, &g, input.as_ref(), &content, lyrics.as_deref(), accent, p.neighbours, id, sig);
                match slot.cur.take() {
                    Some(old) if old.id != id => {
                        slot.old = Some(old);
                        slot.changed = now;
                    }
                    Some(_) => {}
                    None => slot.changed = f64::NEG_INFINITY,
                }
                slot.cur = Some(scene);
            }
            if let (Some(sc), Content::Line { prog, .. }) = (slot.cur.as_mut(), &content) {
                sc.prog = *prog as f32;
            }
        }

        // ---- cross-fade (a still preview never fades)
        let fading_on = !self.sample || self.lyr.clock_override.is_some();
        let f = if !fading_on || slot.old.is_none() { 1.0 } else { ((now - slot.changed) / FADE_S).clamp(0.0, 1.0) as f32 };
        let fe = f * f * (3.0 - 2.0 * f);
        if f >= 1.0 {
            slot.old = None;
        }
        if let Some(old) = &slot.old {
            self.draw_scene(old, b, a * (1.0 - fe), now);
        }
        if let Some(cur) = &slot.cur {
            self.draw_scene(cur, b, a * fe, now);
        }
        let playing = input.as_ref().is_some_and(|i| i.playing) && !self.sample;
        self.lyr.animating = slot.old.is_some() || (playing && slot.cur.as_ref().is_some_and(|s| s.live));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::lyrics::Line;
    use crate::live::render::widget_rect;
    use crate::live::sensors::Sensors;
    use crate::live::theme::{Theme, ThemeLive};
    use crate::live::{LH as CH, LW as CW};
    use serde_json::{json, Value};
    use std::sync::Mutex;
    use std::time::Instant;

    fn backdrop() -> Arc<Vec<u8>> {
        let mut v = vec![255u8; CW * CH * 4];
        for y in 0..CH {
            for x in 0..CW {
                let i = (y * CW + x) * 4;
                let c = ((x / 48 + y / 48) % 2) as usize * 14;
                v[i] = (34 + x * 70 / CW + c) as u8;
                v[i + 1] = (54 + y * 70 / CH + c) as u8;
                v[i + 2] = (110 + c) as u8;
            }
        }
        Arc::new(v)
    }

    const BOX: (f32, f32, f32, f32) = (0.15, 0.35, 0.70, 0.30);

    fn rig(props: Value, with_widget: bool, np: Option<NowPlaying>) -> Renderer {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        t.theme.widgets.clear();
        if with_widget {
            t.theme.widgets.push(Widget::new("l0", "lyrics", BOX.0, BOX.1, BOX.2, BOX.3, props));
        }
        let mut r = Renderer::new(Arc::new(Mutex::new(Sensors::default())), None);
        r.set_sample_mode(true);
        r.set_theme(t);
        r.set_still_background(Some(backdrop()));
        r.set_music_source(Some(Arc::new(Mutex::new(np))));
        r
    }

    fn demo_np() -> NowPlaying {
        NowPlaying::demo("Neon Skyline", "Sample Artist", true, 10.0, 200.0)
    }

    fn frame(r: &mut Renderer) -> Vec<u8> {
        r.last_key = None;
        r.layer_key = None;
        r.render().0.to_vec()
    }

    fn empty() -> Vec<u8> {
        let mut r = rig(json!({}), false, None);
        frame(&mut r)
    }

    /// Bounding box (x0, y0, x1, y1 inclusive) of the pixels of `a` that differ from `b`.
    fn diff_box(a: &[u8], b: &[u8]) -> Option<(usize, usize, usize, usize)> {
        let mut bb: Option<(usize, usize, usize, usize)> = None;
        for i in 0..CW * CH {
            if a[i * 4..i * 4 + 3] != b[i * 4..i * 4 + 3] {
                let (x, y) = (i % CW, i / CW);
                bb = Some(match bb {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
        bb
    }

    fn box_rect() -> Rect {
        widget_rect(&Widget::new("x", "lyrics", BOX.0, BOX.1, BOX.2, BOX.3, json!({})), CW, CH)
    }

    fn inside(bb: (usize, usize, usize, usize)) -> bool {
        let r = box_rect();
        bb.0 as f32 >= r.x && bb.1 as f32 >= r.y && (bb.2 as f32) < r.x + r.w && (bb.3 as f32) < r.y + r.h
    }

    fn lyr(lines: &[(f64, f64, &str)]) -> Arc<Lyrics> {
        Arc::new(Lyrics {
            lines: lines.iter().map(|(s, e, t)| Line { start_s: *s, end_s: *e, text: (*t).into() }).collect(),
            source: Source::Lrclib,
            lrclib_id: None,
            track_name: "T".into(),
            artist_name: "A".into(),
        })
    }

    /// Position `frac` through demo line `i`.
    fn at(l: &Lyrics, i: usize, frac: f64) -> f64 {
        let ln = &l.lines[i];
        ln.start_s + frac * (ln.end_s - ln.start_s).min(ly::natural_len(&ln.text))
    }

    /// Pixels that are strongly red (the bright fill when the accent is #ff0000; the backdrop is blue/green).
    fn red(f: &[u8]) -> (usize, usize, usize) {
        let (mut n, mut x0, mut x1) = (0, usize::MAX, 0);
        for i in 0..CW * CH {
            let p = &f[i * 4..i * 4 + 3];
            if p[0] > 200 && p[1] < 90 && p[2] < 90 {
                n += 1;
                x0 = x0.min(i % CW);
                x1 = x1.max(i % CW);
            }
        }
        (n, x0, x1)
    }

    fn with(r: &mut Renderer, st: State, pos: f64, clock: f64) {
        r.lyr.force = Some((st, pos));
        r.lyr.clock_override = Some(clock);
    }

    #[test]
    fn found_state_draws_inside_its_box_and_is_centred() {
        let bg = empty();
        for background in ["card", "gradient", "none"] {
            let mut r = rig(json!({ "background": background }), true, Some(demo_np()));
            let f = frame(&mut r);
            let bb = diff_box(&f, &bg).unwrap_or_else(|| panic!("{background}: drew nothing"));
            assert!(inside(bb), "{background}: draws outside its box: {bb:?}");
        }
        let mut r = rig(json!({ "background": "none" }), true, Some(demo_np()));
        let bb = diff_box(&frame(&mut r), &bg).unwrap();
        let bx = box_rect();
        let mid = (bb.0 + bb.2) as f32 / 2.0;
        assert!((mid - (bx.x + bx.w / 2.0)).abs() <= 8.0, "text is centred: ink middle {mid}, box middle {}", bx.x + bx.w / 2.0);
        let (w, h) = (bb.2 - bb.0, bb.3 - bb.1);
        assert!(w > 200 && h > 60, "something readable: {w}x{h}");
    }

    #[test]
    fn current_line_is_big_enough() {
        // >= 72 px on the 2400 canvas = 50.4 px on the live canvas: the ink of the current line alone (no neighbours) is taller
        let bg = empty();
        let mut r = rig(json!({ "background": "none", "neighbours": false }), true, Some(demo_np()));
        let l = demo_lyrics();
        with(&mut r, State::Found(l.clone()), at(&l, 1, 0.5), 0.0);
        let bb = diff_box(&frame(&mut r), &bg).unwrap();
        // ascender-to-descender of the text is at least ~0.9 of the font size
        assert!((bb.3 - bb.1) as f32 >= 0.9 * 50.0, "current line ink height {}", bb.3 - bb.1);
    }

    #[test]
    fn the_fill_grows_left_to_right_and_nothing_moves_vertically() {
        let bg = empty();
        let l = demo_lyrics();
        let mut shots = Vec::new();
        for frac in [0.2, 0.8] {
            let mut r = rig(json!({ "background": "none", "accent": "#ff0000" }), true, Some(demo_np()));
            with(&mut r, State::Found(l.clone()), at(&l, 1, frac), 0.0);
            let f = frame(&mut r);
            shots.push((red(&f), diff_box(&f, &bg).unwrap()));
        }
        let ((n0, a0, b0), bb0) = shots[0];
        let ((n1, a1, b1), bb1) = shots[1];
        assert!(n0 > 100, "some bright ink at 20 %: {n0}");
        assert!(n1 > n0 * 2, "the bright area grows: {n0} -> {n1}");
        assert!(b1 > b0 + 100, "the edge moved right: {b0} -> {b1}");
        assert!((a0 as i64 - a1 as i64).abs() <= 3, "the fill starts at the same left edge: {a0} vs {a1}");
        assert_eq!((bb0.1, bb0.3), (bb1.1, bb1.3), "ink rows are identical (no vertical motion)");
        assert_eq!((bb0.0, bb0.2), (bb1.0, bb1.2), "ink columns are identical too");
    }

    #[test]
    fn a_long_line_wraps_to_two_rows_and_the_fill_runs_through_both() {
        let bg = empty();
        let long = "We keep moving till the morning comes and the whole wide city wakes up under a sky full of colours";
        let l = lyr(&[(0.0, 10.0, long), (10.0, 20.0, "next")]);
        let mut rows = Vec::new();
        for frac in [0.25, 0.75] {
            let mut r = rig(json!({ "background": "none", "accent": "#ff0000", "neighbours": false }), true, Some(demo_np()));
            with(&mut r, State::Found(l.clone()), 4.0 * 0.0 + frac * 10.0, 0.0);
            let f = frame(&mut r);
            let bb = diff_box(&f, &bg).unwrap();
            assert!(inside(bb), "{bb:?}");
            // red pixels' vertical extent
            let (mut y0, mut y1) = (usize::MAX, 0);
            for y in 0..CH {
                for x in 0..CW {
                    let p = &f[(y * CW + x) * 4..][..3];
                    if p[0] > 200 && p[1] < 90 && p[2] < 90 {
                        y0 = y0.min(y);
                        y1 = y1.max(y);
                    }
                }
            }
            rows.push((bb, y0, y1));
        }
        let (bb_a, ra0, ra1) = rows[0];
        let (bb_b, rb0, rb1) = rows[1];
        assert_eq!((bb_a.1, bb_a.3), (bb_b.1, bb_b.3), "same ink rows");
        // two rows of text: the ink is taller than one row of 50 px text would be
        assert!(bb_a.3 - bb_a.1 > 85, "two rows: {}", bb_a.3 - bb_a.1);
        // 25 %: only the first row is bright; 75 %: the second row is bright as well
        assert!(ra1 < rb1, "the second row fills later: {ra1} < {rb1}");
        assert!(rb0 <= ra0 + 2);
    }

    /// Rows (of the whole frame) that carry ink, i.e. differ from the empty frame.
    fn ink_rows(f: &[u8], bg: &[u8]) -> Vec<bool> {
        (0..CH).map(|y| (0..CW).any(|x| f[(y * CW + x) * 4..][..3] != bg[(y * CW + x) * 4..][..3])).collect()
    }

    #[test]
    fn a_line_change_cross_fades_in_place() {
        let bg = empty();
        let l = demo_lyrics();
        let mut r = rig(json!({ "background": "none" }), true, Some(demo_np()));
        with(&mut r, State::Found(l.clone()), at(&l, 1, 0.9), 0.0);
        let first = ink_rows(&frame(&mut r), &bg);
        // the next line starts at clock 1.0
        r.lyr.force = Some((State::Found(l.clone()), at(&l, 2, 0.1)));
        let mut mids = Vec::new();
        for c in [1.0, 1.06, 1.12, 1.18, 1.24] {
            r.lyr.clock_override = Some(c);
            let f = frame(&mut r);
            assert!(r.lyr.animating, "a running fade asks for 30 Hz");
            mids.push(f);
        }
        r.lyr.clock_override = Some(2.0);
        let last = ink_rows(&frame(&mut r), &bg);
        // the first frame of the change still shows the old lines only; later ones blend the two sets
        assert_eq!(ink_rows(&mids[0], &bg), first, "the old set stays where it was");
        for (i, f) in mids.iter().enumerate() {
            let rows = ink_rows(f, &bg);
            for y in 0..CH {
                assert!(!rows[y] || first[y] || last[y], "frame {i}: ink on row {y} that neither the old nor the new set uses");
            }
            let bb = diff_box(f, &bg).unwrap();
            assert!(inside(bb));
        }
        // something really changes between the first and the last mid frame
        assert!(mids[0] != mids[3]);
    }

    #[test]
    fn the_gap_draws_three_dots_that_only_change_opacity() {
        let bg = empty();
        let l = lyr(&[(10.0, 20.0, "First line"), (20.0, 30.0, "Second line")]);
        let mut shots = Vec::new();
        for clock in [0.0, 0.45, 0.9] {
            let mut r = rig(json!({ "background": "none", "neighbours": false }), true, Some(demo_np()));
            with(&mut r, State::Found(l.clone()), 3.0, clock);
            let f = frame(&mut r);
            shots.push((diff_box(&f, &bg).expect("the dots are drawn"), f));
        }
        let bx = box_rect();
        let b0 = shots[0].0;
        assert!(inside(b0));
        assert!(b0.2 - b0.0 < 200 && b0.3 - b0.1 < 60, "just dots: {b0:?}");
        assert!(((b0.0 + b0.2) as f32 / 2.0 - (bx.x + bx.w / 2.0)).abs() <= 3.0, "dots centred");
        assert!(shots[0].1 != shots[1].1, "the dots change with time");
        for s in &shots {
            assert_eq!((s.0 .1, s.0 .3), (b0.1, b0.3), "dots never move vertically");
        }
        // a note line is a gap as well
        let n = lyr(&[(0.0, 10.0, "Words"), (10.0, 20.0, "\u{266a}"), (20.0, 30.0, "More words")]);
        let mut r = rig(json!({ "background": "none", "neighbours": true }), true, Some(demo_np()));
        with(&mut r, State::Found(n.clone()), 12.0, 0.0);
        let bb = diff_box(&frame(&mut r), &bg).unwrap();
        assert!(inside(bb));
    }

    #[test]
    fn states_without_lyrics_draw_something_calm() {
        let bg = empty();
        for (name, st) in [
            ("loading", State::Loading),
            ("notfound", State::NotFound),
            ("nosynced", State::NoSynced),
            ("instrumental", State::Instrumental),
            ("off", State::Off),
        ] {
            for background in ["card", "gradient", "none"] {
                let mut r = rig(json!({ "background": background }), true, Some(demo_np()));
                with(&mut r, st.clone(), 5.0, 0.0);
                let f = frame(&mut r);
                let bb = diff_box(&f, &bg).unwrap_or_else(|| panic!("{name}/{background} drew nothing"));
                assert!(inside(bb), "{name}/{background}: {bb:?}");
            }
        }
        // an empty song title and a very long one never panic
        let weird = NowPlaying::demo(&"x".repeat(900), "", true, 1.0, 10.0);
        let mut r = rig(json!({}), true, Some(weird));
        with(&mut r, State::NoSynced, 5.0, 0.0);
        let bb = diff_box(&frame(&mut r), &bg).unwrap();
        assert!(inside(bb));
    }

    #[test]
    fn text_without_spaces_and_huge_lines_stay_inside() {
        let bg = empty();
        let cjk = "\u{3042}".repeat(70);
        let huge = "word ".repeat(120);
        for text in [cjk.as_str(), huge.as_str(), "Supercalifragilisticexpialidocious".repeat(6).as_str()] {
            let l = lyr(&[(0.0, 10.0, text)]);
            let mut r = rig(json!({ "background": "none" }), true, Some(demo_np()));
            with(&mut r, State::Found(l), 5.0, 0.0);
            let bb = diff_box(&frame(&mut r), &bg).unwrap_or_else(|| panic!("nothing for a {}-char line", text.len()));
            assert!(inside(bb), "{bb:?}");
        }
    }

    #[test]
    fn nothing_playing_hides_like_the_music_card() {
        let bg = empty();
        let mut r = rig(json!({}), true, None);
        let f = frame(&mut r);
        assert!(diff_box(&f, &bg).is_none(), "no song: nothing drawn");
    }

    #[test]
    fn preview_shows_the_made_up_song_without_a_loader() {
        let mut r = rig(json!({}), true, Some(demo_np()));
        let _ = frame(&mut r);
        assert!(r.lyr.loader.is_none(), "the preview never creates the network loader");
        assert!(r.lyr.slots.contains_key("l0"));
    }

    #[test]
    fn props_defaults_and_clean() {
        let d = LyricsProps::default();
        assert!(!d.online && d.neighbours && d.folder.is_none() && d.accent.is_none() && d.background == "card");
        let w = Widget { props: json!({ "online": true, "extra": 1 }), ..Widget::default() };
        let p = w.lyrics();
        assert!(p.online && p.neighbours && p.background == "card");
        assert_eq!(Widget { props: json!("junk"), ..Widget::default() }.lyrics(), LyricsProps::default());
        let run = |v: Value| {
            let mut o = v.as_object().cloned().unwrap();
            ly::clean(&mut o);
            o
        };
        let a = run(json!({ "online": "yes", "folder": "  C:\\Lyrics  ", "background": "plasma", "accent": "red", "neighbours": 3 }));
        assert_eq!(a["online"], json!(false));
        assert_eq!(a["folder"], json!("C:\\Lyrics"));
        assert_eq!(a["background"], json!("card"));
        assert_eq!(a["accent"], Value::Null);
        assert_eq!(a["neighbours"], json!(true));
        assert_eq!(run(json!({ "folder": "   " }))["folder"], Value::Null);
        assert_eq!(run(json!({ "accent": "#ABCDEF", "background": "gradient" }))["accent"], json!("#abcdef"));
        for v in [json!({}), json!({ "online": true, "folder": "D:\\x", "accent": "#00ff00", "background": "none", "neighbours": false })] {
            let once = run(v);
            assert_eq!(once, run(Value::Object(once.clone())));
        }
        let mut full = json!({ "online": true });
        crate::live::morewidgets::clean("lyrics", &mut full);
        assert_eq!(full["background"], json!("card"));
    }

    #[test]
    fn palette_finds_a_colour_or_gives_up() {
        let mut rgba = Vec::new();
        for _ in 0..(64 * 64) {
            rgba.extend_from_slice(&[200, 30, 40, 255]);
        }
        let (acc, cols) = palette(&Cover { w: 64, h: 64, rgba }).expect("a red cover has a colour");
        assert!(acc[0] > 200 && acc[1] < 120, "{acc:?}");
        assert_eq!(cols.len(), 3);
        let grey: Vec<u8> = (0..64 * 64).flat_map(|_| [128u8, 128, 128, 255]).collect();
        assert!(palette(&Cover { w: 64, h: 64, rgba: grey }).is_none(), "a grey cover gives the fallback");
        assert!(palette(&Cover { w: 0, h: 0, rgba: Vec::new() }).is_none());
    }

    /// Writes sample pictures when E_LYRICS_SAMPLES=<dir> is set. Run with --ignored.
    #[test]
    #[ignore]
    fn write_samples() {
        let Ok(dir) = std::env::var("E_LYRICS_SAMPLES") else { return };
        std::fs::create_dir_all(&dir).unwrap();
        let save = |r: &mut Renderer, name: &str| {
            let _ = frame(r);
            std::fs::write(std::path::Path::new(&dir).join(name), r.frame_pixmap().encode_png().unwrap()).unwrap();
        };
        let l = demo_lyrics();
        for bgk in ["card", "gradient", "none"] {
            let mut r = rig(json!({ "background": bgk }), true, Some(demo_np()));
            with(&mut r, State::Found(l.clone()), at(&l, 1, 0.5), 0.0);
            save(&mut r, &format!("lyrics-{bgk}.png"));
        }
        let long = lyr(&[
            (0.0, 6.0, "Lights are coming up across the bay"),
            (6.0, 16.0, "We keep moving till the morning comes and the whole wide city wakes up under a sky full of colours"),
            (16.0, 22.0, "Hold on, we are nearly home"),
        ]);
        let mut r = rig(json!({ "background": "gradient" }), true, Some(demo_np()));
        with(&mut r, State::Found(long.clone()), at(&long, 1, 0.62), 0.0);
        save(&mut r, "lyrics-two-lines.png");
        let gap = lyr(&[(14.0, 20.0, "Hold on, we are nearly home"), (20.0, 26.0, "Every window glowing gold")]);
        let mut r = rig(json!({}), true, Some(demo_np()));
        with(&mut r, State::Found(gap), 6.0, 0.2);
        save(&mut r, "lyrics-gap.png");
        let mut r = rig(json!({}), true, Some(demo_np()));
        with(&mut r, State::NoSynced, 6.0, 0.0);
        save(&mut r, "lyrics-no-synced.png");
        let mut r = rig(json!({ "background": "gradient" }), true, Some(demo_np()));
        with(&mut r, State::Found(l.clone()), at(&l, 1, 0.95), 0.0);
        let _ = frame(&mut r);
        r.lyr.force = Some((State::Found(l.clone()), at(&l, 2, 0.05)));
        r.lyr.clock_override = Some(1.0);
        let _ = frame(&mut r);
        r.lyr.clock_override = Some(1.12);
        save(&mut r, "lyrics-cross-fade.png");
    }

    /// Prints the cost of a redraw (release build): `cargo test --release --lib lyrics_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn lyrics_cost() {
        let l = lyr(&[(0.0, 100.0, "We keep moving till the morning comes and the sky is full of colours")]);
        for bgk in ["card", "gradient"] {
            let mut r = rig(json!({ "background": bgk }), true, Some(demo_np()));
            with(&mut r, State::Found(l.clone()), 10.0, 0.0);
            let _ = frame(&mut r);
            r.widget_us.clear();
            let t0 = Instant::now();
            for i in 0..100 {
                r.lyr.force = Some((State::Found(l.clone()), 10.0 + i as f64 / 30.0));
                r.lyr.clock_override = Some(i as f64 / 30.0);
                let _ = frame(&mut r);
            }
            let total = t0.elapsed().as_secs_f64() * 10.0;
            let wid = r.widget_us.get("l0").map(|x| x.1 as f64 / 100.0 / 1000.0).unwrap_or(0.0);
            println!("lyrics cost {bgk}: {total:.2} ms per full frame, {wid:.2} ms in the lyrics item");
        }
    }
}
