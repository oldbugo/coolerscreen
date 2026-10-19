//! Four more looks for the music widget (4.x E M3), chosen with the `style` prop: "cover" (big cover left, text right), "line"
//! (one ticker line), "disc" (a turning vinyl) and "blur" (blurred-cover card). "card" (the default, and anything unknown) is the
//! original card in widgets.rs and is never touched from here.
//!
//! Panel rules kept: nothing moves up or down (the line slides sideways, the disc turns in place, everything else fades);
//! titles are big and bold; the pictures (rounded cover, blurred background, vinyl, ticker line) are built once per track and
//! box size and kept in a per-widget slot, so a frame is only blits and one rotated blit.
use super::widgets::{fmt_time, resize_exact, transport_icon, TransportIcon};
use super::{blend_tile, round_rect, Rect, Renderer, Weight};
use crate::live::nowplaying::{Cover, NowPlaying};
use crate::live::theme::MusicProps;
use std::collections::HashMap;
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

const SCRIM: [u8; 4] = [0, 0, 0, 96];
const GREY: [u8; 3] = [190, 197, 214];
const IDLE_GREY: [u8; 3] = [150, 156, 172];
/// One turn of the disc takes this long.
const TURN_S: f32 = 8.0;
/// Fade between two tracks on the line look.
const SWAP_S: f32 = 0.4;
/// Ticker: wait at the start, hold at the end, fade out / in at the wrap (seconds).
const PAUSE_S: f32 = 2.0;
const HOLD_S: f32 = 1.0;
const WRAP_FADE_S: f32 = 0.4;
/// Ticker speed on the 2400 px design canvas (px/s); scaled to the engine canvas when used.
const TICKER_PX_S: f32 = 60.0;
/// Transparent border around the pre-rendered disc so a rotated, resampled edge never leaves the picture.
const DISC_MARGIN: usize = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Look {
    Card,
    Cover,
    Line,
    Disc,
    Blur,
}

pub(super) fn look(style: &str) -> Look {
    match style {
        "cover" => Look::Cover,
        "line" => Look::Line,
        "disc" => Look::Disc,
        "blur" => Look::Blur,
        _ => Look::Card,
    }
}

/// Redraws per second the cluster needs: the ticker and the turning disc want 30, the rest 10.
pub(super) fn hz(p: &MusicProps) -> u32 {
    if fast_bits(p) != 0 { 30 } else { 10 }
}

/// 1 = a ticker line (moves even when paused), 2 = a disc that turns while playing.
pub(super) fn fast_bits(p: &MusicProps) -> u8 {
    match look(&p.style) {
        Look::Line => 1,
        Look::Disc if p.spin => 2,
        _ => 0,
    }
}

/// A made line of text (Title  ·  Artist) rendered once into a strip, with its ticker clock.
struct Line {
    tid: u64,
    tsz: u32,
    txt: String,
    pm: Pixmap,
    text_w: f32,
    /// Clock value when this line first showed (the ticker starts from here).
    started: f64,
    /// When this line (as a new track) began fading in.
    changed: f64,
    /// Where the ticker was on the last frame (the outgoing line freezes there).
    last_off: f32,
}

/// Everything cached for one music widget (keyed by its position).
#[derive(Default)]
struct Slot {
    /// Rounded cover tile for "cover" / "blur": key (track, side, has cover, radius x4).
    tile: Option<((u64, usize, bool, u32), Pixmap)>,
    /// Blurred background for "blur": key (track, w, h, has cover, radius x4).
    bg: Option<((u64, u32, u32, bool, u32), Pixmap)>,
    /// The vinyl for "disc": key (track, side, has cover).
    disc: Option<((u64, usize, bool), Pixmap)>,
    /// Disc angle in degrees and the clock of the last advance.
    spin: f32,
    spin_at: Option<f64>,
    line: Option<Line>,
    prev: Option<Line>,
}

#[derive(Default)]
pub(super) struct MusicCache {
    /// OR of `fast_bits` of the visible music widgets (set when the theme changes).
    pub(super) fast: u8,
    /// A fixed clock (seconds) instead of the real one: tests and sample pictures.
    pub(super) clock_override: Option<f64>,
    now: f64,
    slots: HashMap<(i32, i32), Slot>,
}

impl MusicCache {
    /// The animation clock for this frame: the override, else 0 in sample mode (a still preview), else seconds since start.
    pub(super) fn set_clock(&mut self, t: f32, sample: bool) {
        self.now = self.clock_override.unwrap_or(if sample { 0.0 } else { t as f64 });
    }

    /// Changes at 30 Hz while a ticker or a turning disc is on show, so the frame is redrawn even without a video.
    pub(super) fn motion_key(&self, playing: bool) -> u64 {
        if self.fast & 1 != 0 || (self.fast & 2 != 0 && playing) { ((self.now * 30.0) as u64) << 40 } else { 0 }
    }
}

/// The cover shrunk to at most 320 px on its long side for the original card (covers are decoded up to 640 px now; the card
/// always scaled from 320). None = already small enough.
pub(super) fn shrink_cover(c: &Cover) -> Option<Cover> {
    if c.w.max(c.h) <= 320 || c.w == 0 || c.h == 0 || c.rgba.len() < c.w * c.h * 4 {
        return None;
    }
    let src = opaque_pixmap(c)?;
    let k = 320.0 / c.w.max(c.h) as f32;
    let (tw, th) = (((c.w as f32 * k).round() as u32).max(1), ((c.h as f32 * k).round() as u32).max(1));
    let out = resize_exact(&src, tw, th);
    Some(Cover { w: tw as usize, h: th as usize, rgba: out.data().to_vec() })
}

/// The cover as an opaque pixmap (alpha ignored, like the card does).
fn opaque_pixmap(c: &Cover) -> Option<Pixmap> {
    if c.w == 0 || c.h == 0 || c.rgba.len() < c.w * c.h * 4 {
        return None;
    }
    let mut pm = Pixmap::new(c.w as u32, c.h as u32)?;
    for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(c.rgba.chunks_exact(4)) {
        d.copy_from_slice(&[s[0], s[1], s[2], 255]);
    }
    Some(pm)
}

/// Centre square of the cover scaled to `side` x `side` (box-filtered, then bilinear).
fn square_of(c: &Cover, side: u32) -> Option<Pixmap> {
    let full = opaque_pixmap(c)?;
    let m = c.w.min(c.h);
    let (ox, oy) = ((c.w - m) / 2, (c.h - m) / 2);
    let mut sq = Pixmap::new(m as u32, m as u32)?;
    let (fw, sw) = (full.width() as usize, m);
    for y in 0..m {
        let s = ((oy + y) * fw + ox) * 4;
        sq.data_mut()[y * sw * 4..(y + 1) * sw * 4].copy_from_slice(&full.data()[s..s + sw * 4]);
    }
    Some(resize_exact(&sq, side, side))
}

/// A neutral tile with a note, for "no cover".
fn placeholder(side: usize) -> Pixmap {
    let mut pm = Pixmap::new(side as u32, side as u32).unwrap();
    for y in 0..side {
        for x in 0..side {
            let note = (x > side * 2 / 5 && x < side * 3 / 5 && y > side / 4 && y < side * 3 / 4) || (x > side / 3 && x < side * 2 / 5 && y > side * 11 / 20 && y < side * 3 / 4);
            let c: [u8; 3] = if note { [200, 205, 220] } else { [58, 62, 84] };
            pm.data_mut()[(y * side + x) * 4..][..4].copy_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    pm
}

/// Round the corners of a pixmap (premultiplied): coverage falls off over one pixel.
fn round_corners(pm: &mut Pixmap, r: f32) {
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let r = r.clamp(0.0, w.min(h) as f32 / 2.0);
    if r < 0.5 {
        return;
    }
    let ri = r.ceil() as usize;
    for y in 0..ri.min(h) {
        for x in 0..ri.min(w) {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            let d = ((fx - r).powi(2) + (fy - r).powi(2)).sqrt();
            let cov = if fx >= r || fy >= r { 1.0 } else { (r + 0.5 - d).clamp(0.0, 1.0) };
            if cov >= 1.0 {
                continue;
            }
            for (px, py) in [(x, y), (w - 1 - x, y), (x, h - 1 - y), (w - 1 - x, h - 1 - y)] {
                let i = (py * w + px) * 4;
                for k in 0..4 {
                    let v = pm.data()[i + k] as f32 * cov;
                    pm.data_mut()[i + k] = v as u8;
                }
            }
        }
    }
}

/// The cover (or the placeholder) as a square tile with rounded corners.
fn rounded_tile(c: Option<&Cover>, side: usize, radius: f32) -> Pixmap {
    let mut pm = c.and_then(|c| square_of(c, side as u32)).unwrap_or_else(|| placeholder(side));
    round_corners(&mut pm, radius);
    pm
}

/// The blurred, darkened background of the "blur" look: the part of the cover that fills the box, shrunk to ~32 px (that is the
/// blur) and stretched back up smoothly; no cover: a dark neutral gradient. Rounded corners.
fn blur_background(c: Option<&Cover>, w: u32, h: u32, radius: f32) -> Pixmap {
    let (w, h) = (w.max(1), h.max(1));
    let mut out = Pixmap::new(w, h).unwrap();
    let small = c.and_then(|c| {
        let full = opaque_pixmap(c)?;
        let ba = w as f32 / h as f32;
        let ca = c.w as f32 / c.h as f32;
        let (cw, ch) = if ba > ca { (c.w, ((c.w as f32 / ba).round() as usize).clamp(1, c.h)) } else { (((c.h as f32 * ba).round() as usize).clamp(1, c.w), c.h) };
        let (ox, oy) = ((c.w - cw) / 2, (c.h - ch) / 2);
        let mut crop = Pixmap::new(cw as u32, ch as u32)?;
        for y in 0..ch {
            let s = ((oy + y) * c.w + ox) * 4;
            crop.data_mut()[y * cw * 4..(y + 1) * cw * 4].copy_from_slice(&full.data()[s..s + cw * 4]);
        }
        let (sw, sh) = if ba >= 1.0 { (32u32, ((32.0 / ba).round() as u32).max(4)) } else { (((32.0 * ba).round() as u32).max(4), 32u32) };
        Some(resize_exact(&crop, sw, sh))
    });
    match small {
        Some(sm) => {
            let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
            let t = Transform::from_scale(w as f32 / sm.width() as f32, h as f32 / sm.height() as f32);
            out.draw_pixmap(0, 0, sm.as_ref(), &paint, t, None);
            // 50 % black over it so white text reads on any cover
            for px in out.data_mut().chunks_exact_mut(4) {
                for v in px.iter_mut().take(3) {
                    *v = (*v as u32 * 128 / 255) as u8;
                }
            }
        }
        None => {
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let t = (x as f32 / w as f32) * 0.8 + (y as f32 / h as f32) * 0.2;
                    let c = [34.0 - 20.0 * t, 38.0 - 22.0 * t, 60.0 - 32.0 * t];
                    out.data_mut()[(y * w as usize + x) * 4..][..4].copy_from_slice(&[c[0] as u8, c[1] as u8, c[2] as u8, 255]);
                }
            }
        }
    }
    round_corners(&mut out, radius);
    out
}

/// The vinyl: a dark disc with a few faint grooves, the cover (or a neutral label) cut into a circle in the middle and a small
/// centre hole. Premultiplied, `side` x `side`; drawn rotated each frame.
fn build_disc(c: Option<&Cover>, side: usize) -> Pixmap {
    let full = side + 2 * DISC_MARGIN;
    let mut pm = Pixmap::new(full as u32, full as u32).unwrap();
    let r = side as f32 / 2.0;
    let m = DISC_MARGIN as f32;
    let rc = 0.62 * r;
    let hole = (0.05 * r).max(1.5);
    let label = c.and_then(|c| square_of(c, ((2.0 * rc).round() as u32).max(2)));
    for y in 0..full {
        for x in 0..full {
            let (fx, fy) = (x as f32 + 0.5 - r - m, y as f32 + 0.5 - r - m);
            let d = (fx * fx + fy * fy).sqrt();
            let outer = (r + 0.5 - d).clamp(0.0, 1.0) * (d - hole + 0.5).clamp(0.0, 1.0);
            if outer <= 0.0 {
                continue;
            }
            // vinyl: dark, with five faint grooves and a thin bright rim
            let mut v = 16.0;
            for k in 0..5 {
                let rk = r * (0.69 + 0.055 * k as f32);
                v += (1.0 - (d - rk).abs() / 0.9).clamp(0.0, 1.0) * 14.0;
            }
            v += (1.0 - (d - (r - 1.2)).abs() / 1.0).clamp(0.0, 1.0) * 26.0;
            let mut rgb = [v, v, v + 5.0];
            let cvc = (rc + 0.5 - d).clamp(0.0, 1.0);
            if cvc > 0.0 {
                let lab: [f32; 3] = match &label {
                    Some(l) => {
                        let ix = ((fx + rc).floor().max(0.0) as usize).min(l.width() as usize - 1);
                        let iy = ((fy + rc).floor().max(0.0) as usize).min(l.height() as usize - 1);
                        let p = &l.data()[(iy * l.width() as usize + ix) * 4..][..3];
                        [p[0] as f32, p[1] as f32, p[2] as f32]
                    }
                    None => [58.0, 62.0, 84.0],
                };
                for k in 0..3 {
                    rgb[k] = rgb[k] * (1.0 - cvc) + lab[k] * cvc;
                }
            }
            let i = (y * full + x) * 4;
            let d = pm.data_mut();
            d[i] = (rgb[0] * outer) as u8;
            d[i + 1] = (rgb[1] * outer) as u8;
            d[i + 2] = (rgb[2] * outer) as u8;
            d[i + 3] = (255.0 * outer) as u8;
        }
    }
    pm
}

/// The ticker position at `t` seconds after the line first showed: (pixels scrolled, opacity). Wait `PAUSE_S`, slide to the end at
/// `speed` px/s, hold, fade out, (jump back while invisible), fade in. A line that fits (`travel` <= 0) never moves.
pub(super) fn ticker(t: f32, travel: f32, speed: f32) -> (f32, f32) {
    if travel <= 0.5 || speed <= 0.0 {
        return (0.0, 1.0);
    }
    let slide = travel / speed;
    let total = PAUSE_S + slide + HOLD_S + WRAP_FADE_S;
    let first = t < total; // the first round starts fully visible (a new track fades in on its own)
    let t = t.max(0.0) % total;
    if t < PAUSE_S {
        (0.0, if first { 1.0 } else { (t / WRAP_FADE_S).min(1.0) })
    } else if t < PAUSE_S + slide {
        ((t - PAUSE_S) * speed, 1.0)
    } else if t < PAUSE_S + slide + HOLD_S {
        (travel, 1.0)
    } else {
        (travel, (1.0 - (t - PAUSE_S - slide - HOLD_S) / WRAP_FADE_S).clamp(0.0, 1.0))
    }
}

/// Draw `strip` (premultiplied) with its column `off` at layer x `x0`, `vw` columns wide, top at `y0`; sub-pixel scroll by linear
/// interpolation, the edges fade over `fw` px where text is cut off (`lead`: text is hidden on the left, `trail`: on the right).
fn blit_ticker(layer: &mut Pixmap, strip: &Pixmap, off: f32, x0: i32, y0: i32, vw: i32, alpha: f32, fw: f32, lead: f32, trail: f32) {
    let (lw, lh) = (layer.width() as i32, layer.height() as i32);
    let (sw, sh) = (strip.width() as i32, strip.height() as i32);
    let o0 = off.floor();
    let fr = off - o0;
    let o0 = o0 as i32;
    let fw = fw.max(1.0);
    for c in 0..vw {
        let dx = x0 + c;
        if dx < 0 || dx >= lw {
            continue;
        }
        let (sx0, sx1) = (c + o0, c + o0 + 1);
        if sx0 >= sw {
            break;
        }
        let cf = c as f32 + 0.5;
        let mut wgt = alpha;
        if lead > 0.0 && cf < fw {
            wgt *= 1.0 - lead * (1.0 - cf / fw);
        }
        let rem = vw as f32 - cf;
        if trail > 0.0 && rem < fw {
            wgt *= 1.0 - trail * (1.0 - rem / fw);
        }
        if wgt <= 0.002 {
            continue;
        }
        for row in 0..sh {
            let dy = y0 + row;
            if dy < 0 || dy >= lh {
                continue;
            }
            let a = &strip.data()[(row * sw + sx0.max(0)) as usize * 4..][..4];
            let b: &[u8] = if sx1 < sw { &strip.data()[(row * sw + sx1) as usize * 4..][..4] } else { &[0, 0, 0, 0] };
            let mut s = [0f32; 4];
            for k in 0..4 {
                s[k] = (a[k] as f32 * (1.0 - fr) + b[k] as f32 * fr) * wgt;
            }
            if s[3] < 0.5 {
                continue;
            }
            let i = (dy * lw + dx) as usize * 4;
            let inv = 1.0 - s[3] / 255.0;
            let d = layer.data_mut();
            for k in 0..4 {
                d[i + k] = (s[k] + d[i + k] as f32 * inv).min(255.0) as u8;
            }
        }
    }
}

impl Renderer {
    /// The extra looks. Returns false for "card" / unknown (the caller draws the original card).
    pub(super) fn draw_music_style(&mut self, b: Rect, p: &MusicProps, np: Option<&NowPlaying>, a: f32) -> bool {
        let lk = look(&p.style);
        if lk == Look::Card || b.w < 4.0 || b.h < 4.0 {
            return false;
        }
        if self.mus.slots.len() > 8 {
            self.mus.slots.clear();
        }
        let key = (b.x.round() as i32, b.y.round() as i32);
        let mut slot = self.mus.slots.remove(&key).unwrap_or_default();
        if lk == Look::Line {
            self.draw_line(&mut slot, b, p, np, a);
        } else {
            self.draw_media(&mut slot, lk, b, p, np, a);
        }
        self.mus.slots.insert(key, slot);
        true
    }

    /// "cover", "disc" and "blur": a picture on the left (cover tile, vinyl, or tile on a blurred card), text on the right.
    fn draw_media(&mut self, slot: &mut Slot, lk: Look, b: Rect, p: &MusicProps, np: Option<&NowPlaying>, a: f32) {
        let h = b.h;
        let pad = (0.09 * h).max(2.0);
        let rad = self.card_radius(b, 0.11 * h);
        let fade = (a * 255.0) as u32;
        // `cover` off (or no cover from the player): a neutral tile / label instead, the layout stays
        let cover = if p.cover { np.and_then(|n| n.cover.as_deref()) } else { None };
        let have = cover.is_some();
        let tid = np.map(|n| n.track_id).unwrap_or(0);
        if lk == Look::Blur {
            let (bw, bh) = (b.w.round().max(1.0) as u32, b.h.round().max(1.0) as u32);
            let k = (tid, bw, bh, have, (rad * 4.0) as u32);
            if slot.bg.as_ref().map(|x| x.0) != Some(k) {
                slot.bg = Some((k, blur_background(cover, bw, bh, rad)));
            }
            let Renderer { layer, cw, ch, .. } = self;
            blend_tile(layer.data_mut(), *cw, *ch, slot.bg.as_ref().unwrap().1.data(), b.x.round() as i32, b.y.round() as i32, bw as usize, bh as usize, fade);
        } else {
            round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, [SCRIM[0], SCRIM[1], SCRIM[2], (SCRIM[3] as f32 * a) as u8]);
            round_rect(&mut self.layer, b.x, b.y, b.w, b.h, rad, [255, 255, 255, (26.0 * a) as u8]);
        }
        let side = ((h - 2.0 * pad).round() as usize).max(4);
        let (px, py) = ((b.x + pad).round() as i32, (b.y + pad).round() as i32);
        if lk == Look::Disc {
            let k = (tid, side, have);
            if slot.disc.as_ref().map(|x| x.0) != Some(k) {
                slot.disc = Some((k, build_disc(cover, side)));
            }
            // advance by elapsed time, only while playing
            let now = self.mus.now;
            let dt = slot.spin_at.map(|t| (now - t).clamp(0.0, 0.25)).unwrap_or(0.0) as f32;
            slot.spin_at = Some(now);
            if p.spin && np.is_some_and(|n| n.playing) {
                slot.spin = (slot.spin + dt * 360.0 / TURN_S) % 360.0;
            }
            let (cx, cy) = (px as f32 + side as f32 / 2.0, py as f32 + side as f32 / 2.0);
            let paint = PixmapPaint { opacity: a, quality: FilterQuality::Bilinear, ..PixmapPaint::default() };
            let tf = if slot.spin.abs() < 0.01 { Transform::identity() } else { Transform::from_rotate_at(slot.spin, cx, cy) };
            let mg = DISC_MARGIN as i32;
            self.layer.draw_pixmap(px - mg, py - mg, slot.disc.as_ref().unwrap().1.as_ref(), &paint, tf, None);
        } else {
            let tr = self.card_radius(Rect { x: px as f32, y: py as f32, w: side as f32, h: side as f32 }, 0.1 * side as f32);
            let k = (tid, side, have, (tr * 4.0) as u32);
            if slot.tile.as_ref().map(|x| x.0) != Some(k) {
                slot.tile = Some((k, rounded_tile(cover, side, tr)));
            }
            let Renderer { layer, cw, ch, .. } = self;
            blend_tile(layer.data_mut(), *cw, *ch, slot.tile.as_ref().unwrap().1.data(), px, py, side, side, fade);
        }
        let tx = b.x + pad + side as f32 + 0.08 * h;
        let tw = (b.x + b.w - pad - tx).max(10.0);
        self.media_text(b, np, p.progress, tx, tw, a);
    }

    /// Title, artist, progress bar, play/pause sign and times in the column right of the picture. Sizes scale with the box height.
    fn media_text(&mut self, b: Rect, np: Option<&NowPlaying>, progress: bool, tx: f32, tw: f32, a: f32) {
        let h = b.h;
        let tsz = (0.21 * h).round().max(6.0) as u32;
        let asz = (0.15 * h).round().max(6.0) as u32;
        let Some(n) = np else {
            let s = self.fit("Nothing playing", tsz, Weight::Bold, tw);
            self.text(&s, tx, b.y + 0.57 * h, tsz, Weight::Bold, IDLE_GREY, a);
            return;
        };
        let (tb, ab) = if progress { (0.32, 0.50) } else { (0.46, 0.66) };
        let title = self.fit(&n.title, tsz, Weight::Bold, tw);
        self.text(&title, tx, b.y + tb * h, tsz, Weight::Bold, [255, 255, 255], a);
        if !n.artist.is_empty() {
            let artist = self.fit(&n.artist, asz, Weight::Regular, tw);
            self.text(&artist, tx, b.y + ab * h, asz, Weight::Regular, GREY, a);
        }
        if !progress {
            return;
        }
        let (by, bh) = (b.y + 0.62 * h, (0.05 * h).max(2.0));
        self.progress_bar(tx, by, tw, bh, n, a);
        let ih = 0.14 * h;
        self.sign(tx, b.y + 0.73 * h, ih, n.playing, a);
        let ksz = (0.115 * h).round().max(6.0) as u32;
        let base = b.y + 0.845 * h;
        self.text(&fmt_time(n.position()), tx + ih * 1.35, base, ksz, Weight::Regular, GREY, a);
        if n.duration_s > 0.0 {
            let tot = fmt_time(n.duration_s);
            let ww = self.fonts.width(&tot, ksz, Weight::Regular);
            self.text(&tot, tx + tw - ww, base, ksz, Weight::Regular, GREY, a);
        }
    }

    fn progress_bar(&mut self, x: f32, y: f32, w: f32, h: f32, n: &NowPlaying, a: f32) {
        let frac = if n.duration_s > 0.0 { (n.position() / n.duration_s).clamp(0.0, 1.0) as f32 } else { 0.0 };
        round_rect(&mut self.layer, x, y, w, h, h / 2.0, [255, 255, 255, (60.0 * a) as u8]);
        if frac > 0.0 {
            round_rect(&mut self.layer, x, y, (w * frac).max(h), h, h / 2.0, [255, 255, 255, (255.0 * a) as u8]);
        }
    }

    /// The transport sign: bars while playing, a triangle while paused (same convention as the card), `ih` high at (x, y).
    fn sign(&mut self, x: f32, y: f32, ih: f32, playing: bool, a: f32) {
        let col = [220, 226, 240, (255.0 * a) as u8];
        match transport_icon(playing) {
            TransportIcon::Play => self.fill_path(&[(x, y), (x, y + ih), (x + 0.9 * ih, y + ih / 2.0)], col),
            TransportIcon::Pause => {
                round_rect(&mut self.layer, x, y, 0.3 * ih, ih, 1.0, col);
                round_rect(&mut self.layer, x + 0.5 * ih, y, 0.3 * ih, ih, 1.0, col);
            }
        }
    }

    /// Render "Title  ·  Artist" once into a strip (the text routine draws into the widget layer, so the layer is swapped for the
    /// strip for the moment).
    fn build_line(&mut self, n: &NowPlaying, tsz: u32, started: f64, changed: f64) -> Line {
        let clip = |s: &str| s.chars().take(300).collect::<String>();
        let (title, artist) = (clip(&n.title), clip(&n.artist));
        let sep = if artist.is_empty() { "" } else { "  \u{00B7}  " };
        let (w1, w2, w3) = (self.fonts.width(&title, tsz, Weight::Bold), self.fonts.width(sep, tsz, Weight::Regular), self.fonts.width(&artist, tsz, Weight::Regular));
        let text_w = w1 + w2 + w3;
        let (sw, sh) = ((text_w.ceil() as u32 + 8).clamp(8, 16000), (tsz as f32 * 1.5).ceil() as u32 + 2);
        let mut strip = Pixmap::new(sw, sh).unwrap();
        std::mem::swap(&mut self.layer, &mut strip);
        let (ocw, och) = (self.cw, self.ch);
        self.cw = sw as usize;
        self.ch = sh as usize;
        let base = tsz as f32 * 1.15;
        self.text(&title, 2.0, base, tsz, Weight::Bold, [255, 255, 255], 1.0);
        if !artist.is_empty() {
            self.text(sep, 2.0 + w1, base, tsz, Weight::Regular, GREY, 1.0);
            self.text(&artist, 2.0 + w1 + w2, base, tsz, Weight::Regular, GREY, 1.0);
        }
        self.cw = ocw;
        self.ch = och;
        std::mem::swap(&mut self.layer, &mut strip);
        Line { tid: n.track_id, tsz, txt: format!("{}\u{1}{}", n.title, n.artist), pm: strip, text_w: text_w + 4.0, started, changed, last_off: 0.0 }
    }

    /// "line": a slim scrim band with the transport sign, one line of text that slides sideways when it is too long, and a thin
    /// progress line.
    fn draw_line(&mut self, slot: &mut Slot, b: Rect, p: &MusicProps, np: Option<&NowPlaying>, a: f32) {
        let h = b.h;
        let tsz = ((0.22 * h).round() as u32).max(8);
        let band_h = (tsz as f32 * 2.5).min(h);
        let band = Rect { x: b.x, y: b.y + (h - band_h) / 2.0, w: b.w, h: band_h };
        let rad = self.card_radius(band, 0.28 * band_h);
        round_rect(&mut self.layer, band.x, band.y, band.w, band.h, rad, [SCRIM[0], SCRIM[1], SCRIM[2], (SCRIM[3] as f32 * a) as u8]);
        round_rect(&mut self.layer, band.x, band.y, band.w, band.h, rad, [255, 255, 255, (26.0 * a) as u8]);
        let pad = (tsz as f32 * 0.7).min(band.w / 4.0);
        let cy = band.y + band.h / 2.0 - if p.progress { 0.12 * tsz as f32 } else { 0.0 };
        let base = cy + 0.36 * tsz as f32;
        let Some(n) = np else {
            let s = self.fit("Nothing playing", tsz, Weight::Bold, (b.w - 2.0 * pad).max(10.0));
            self.text(&s, b.x + pad, base, tsz, Weight::Bold, IDLE_GREY, a);
            return;
        };
        let now = self.mus.now;
        let still = self.sample;
        // (re)build the line: a new track fades the old line out and the new one in; a new size keeps the ticker running
        let txt = format!("{}\u{1}{}", n.title, n.artist);
        if !slot.line.as_ref().is_some_and(|l| l.tid == n.track_id && l.tsz == tsz && l.txt == txt) {
            let old = slot.line.take();
            let same_track = old.as_ref().is_some_and(|o| o.tid == n.track_id && o.txt == txt);
            let (started, changed) = match &old {
                Some(o) if same_track => (o.started, o.changed),
                _ => (now, now),
            };
            slot.line = Some(self.build_line(n, tsz, started, changed));
            if !same_track {
                slot.prev = old;
            }
        }
        let ih = 0.7 * tsz as f32;
        let sx = b.x + pad;
        self.sign(sx, cy - ih / 2.0, ih, n.playing, a);
        let vx0 = sx + ih + 0.5 * tsz as f32;
        let vw = (b.x + b.w - pad - vx0).max(8.0);
        let fw = (0.9 * tsz as f32).min(vw / 4.0);
        let speed = TICKER_PX_S * self.cw as f32 / 2400.0;
        let y_of = |tsz: u32| (base - tsz as f32 * 1.15).round() as i32;
        let swap_f = |changed: f64| if still { 1.0 } else { ((now - changed) as f32 / SWAP_S).clamp(0.0, 1.0) };
        // the outgoing line, frozen where it was
        let f_new = swap_f(slot.line.as_ref().map(|l| l.changed).unwrap_or(now));
        if f_new >= 1.0 {
            slot.prev = None;
        }
        if let Some(pv) = slot.prev.as_ref() {
            let f = f_new;
            if f < 1.0 {
                let travel = (pv.text_w - vw).max(0.0);
                let off = pv.last_off.min(travel);
                let (lead, trail) = (if off > 0.0 { (off / fw).min(1.0) } else { 0.0 }, if travel - off > 0.0 { ((travel - off) / fw).min(1.0) } else { 0.0 });
                blit_ticker(&mut self.layer, &pv.pm, off, vx0.round() as i32, y_of(pv.tsz), vw.round() as i32, a * (1.0 - f), fw, lead, trail);
            }
        }
        let l = slot.line.as_mut().unwrap();
        let travel = (l.text_w - vw).max(0.0);
        let (off, ta) = ticker((now - l.started) as f32, travel, speed);
        l.last_off = off;
        let f = swap_f(l.changed);
        let (lead, trail) = (if off > 0.0 { (off / fw).min(1.0) } else { 0.0 }, if travel - off > 0.0 { ((travel - off) / fw).min(1.0) } else { 0.0 });
        blit_ticker(&mut self.layer, &l.pm, off, vx0.round() as i32, y_of(l.tsz), vw.round() as i32, a * ta * f, fw, lead, trail);
        if p.progress {
            let lh = (0.07 * tsz as f32).max(2.0);
            let ly = band.y + band.h - 0.30 * tsz as f32 - lh;
            let lx = b.x + pad;
            self.progress_bar(lx, ly, (b.x + b.w - pad - lx).max(4.0), lh, n, a);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::render::widget_rect;
    use crate::live::sensors::Sensors;
    use crate::live::theme::{MusicIdle, Theme, ThemeLive, Widget};
    use crate::live::{LH, LW};
    use serde_json::{json, Value};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    const LONG: &str = "gjpqy Hl Quickly Jumping Foggy Typhoons gjpqy Hl Quickly Jumping Foggy Typhoons gjpqy Hl";

    fn backdrop() -> Arc<Vec<u8>> {
        let mut v = vec![255u8; LW * LH * 4];
        for y in 0..LH {
            for x in 0..LW {
                let i = (y * LW + x) * 4;
                let c = ((x / 48 + y / 48) % 2) as usize * 14;
                v[i] = (34 + x * 70 / LW + c) as u8;
                v[i + 1] = (54 + y * 70 / LH + c) as u8;
                v[i + 2] = (110 + c) as u8;
            }
        }
        Arc::new(v)
    }

    type Boxed = (f32, f32, f32, f32, Value);

    fn rig(widgets: &[Boxed], np: Option<NowPlaying>, keep_idle: bool) -> Renderer {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        t.theme.widgets.clear();
        for (i, (x, y, w, h, props)) in widgets.iter().enumerate() {
            t.theme.widgets.push(Widget::new(&format!("m{i}"), "music", *x, *y, *w, *h, props.clone()));
        }
        if keep_idle {
            t.theme.handover.music_idle = MusicIdle::Keep;
        }
        let mut r = Renderer::new(Arc::new(Mutex::new(Sensors::default())), None);
        r.set_sample_mode(true);
        r.set_theme(t);
        r.set_still_background(Some(backdrop()));
        r.set_music_source(Some(Arc::new(Mutex::new(np))));
        r
    }

    fn frame(r: &mut Renderer, clock: f64) -> Vec<u8> {
        r.mus.clock_override = Some(clock);
        r.render().0.to_vec()
    }

    /// Bounding box (x0, y0, x1, y1 inclusive) of the pixels that differ.
    fn diff_box(a: &[u8], b: &[u8]) -> Option<(usize, usize, usize, usize)> {
        let mut bb: Option<(usize, usize, usize, usize)> = None;
        for i in 0..LW * LH {
            if a[i * 4..i * 4 + 3] != b[i * 4..i * 4 + 3] {
                let (x, y) = (i % LW, i / LW);
                bb = Some(match bb {
                    None => (x, y, x, y),
                    Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
                });
            }
        }
        bb
    }

    const BOX: Boxed = (0.10, 0.30, 0.34, 0.22, Value::Null);
    fn boxed(props: Value) -> Boxed {
        (BOX.0, BOX.1, BOX.2, BOX.3, props)
    }
    fn demo(playing: bool) -> NowPlaying {
        NowPlaying::demo("Midnight City", "M83", playing, 241.0, 241.0)
    }
    fn empty_frame() -> Vec<u8> {
        let mut r = rig(&[], None, false);
        frame(&mut r, 0.0)
    }

    fn inside(bb: (usize, usize, usize, usize), w: &Boxed) -> bool {
        let r = widget_rect(&Widget::new("x", "music", w.0, w.1, w.2, w.3, json!({})), LW, LH);
        bb.0 as f32 >= r.x && bb.1 as f32 >= r.y && (bb.2 as f32) < r.x + r.w && (bb.3 as f32) < r.y + r.h
    }

    #[test]
    fn every_new_look_draws_inside_its_box() {
        let bg = empty_frame();
        for style in ["cover", "line", "disc", "blur"] {
            for playing in [true, false] {
                let b = boxed(json!({ "style": style }));
                let mut r = rig(&[b.clone()], Some(demo(playing)), false);
                let f = frame(&mut r, 0.0);
                let bb = diff_box(&f, &bg).unwrap_or_else(|| panic!("{style} playing={playing} drew nothing"));
                assert!(inside(bb, &b), "{style} playing={playing} draws outside its box: {bb:?}");
                let (w, h) = (bb.2 - bb.0, bb.3 - bb.1);
                assert!(w > 40 && h > 20, "{style}: something visible, got {w}x{h}");
            }
        }
    }

    #[test]
    fn a_theme_without_style_is_the_card() {
        let plain = {
            let mut r = rig(&[boxed(json!({ "cover": true, "progress": true }))], Some(demo(true)), false);
            frame(&mut r, 0.0)
        };
        for props in [json!({ "style": "card" }), json!({ "style": "nonsense" }), json!({ "style": 7 }), json!({ "spin": false })] {
            let mut r = rig(&[boxed(props.clone())], Some(demo(true)), false);
            assert!(frame(&mut r, 0.0) == plain, "{props} must look like the card");
        }
        let bg = empty_frame();
        assert!(plain != bg, "the card draws");
        // a new look is not the card
        let mut r = rig(&[boxed(json!({ "style": "cover" }))], Some(demo(true)), false);
        assert!(frame(&mut r, 0.0) != plain);
    }

    #[test]
    fn a_short_line_stays_put() {
        let np = NowPlaying::demo("Hi", "Me", true, 241.0, 241.0);
        let mut r = rig(&[boxed(json!({ "style": "line" }))], Some(np), false);
        let (a, b) = (frame(&mut r, 0.0), frame(&mut r, 5.3));
        assert!(a == b, "nothing moves when the line fits");
    }

    #[test]
    fn a_long_line_slides_only_sideways() {
        let np = NowPlaying::demo(LONG, "Some Artist gjpqy", false, 241.0, 241.0);
        let bg = empty_frame();
        let mut r = rig(&[boxed(json!({ "style": "line", "progress": false }))], Some(np), false);
        let (a, b, c) = (frame(&mut r, 0.5), frame(&mut r, 4.0), frame(&mut r, 7.0));
        assert!(a != b && b != c, "the text moves");
        let _ = &bg;
        // rows holding bright ink (white title, grey artist, the sign); the dark scrim and the backdrop are not that bright
        let rows = |f: &[u8]| {
            let ys: Vec<usize> = (0..LW * LH).filter(|i| f[i * 4] > 170 && f[i * 4 + 1] > 170).map(|i| i / LW).collect();
            (*ys.iter().min().unwrap(), *ys.iter().max().unwrap())
        };
        assert_eq!(rows(&a), rows(&b), "same rows of ink while it slides");
        assert_eq!(rows(&b), rows(&c));
        // the pixels that changed between the frames are inside the ink rows
        let (y0, y1) = rows(&a);
        let d = diff_box(&a, &b).unwrap();
        assert!(d.1 + 1 >= y0 && d.3 <= y1 + 1, "changed rows {}..{} vs ink rows {y0}..{y1}", d.1, d.3);
    }

    #[test]
    fn the_ticker_waits_slides_holds_fades_and_fits() {
        assert_eq!(ticker(3.0, 0.0, 42.0), (0.0, 1.0), "fits: never moves");
        let (o, a) = ticker(1.0, 200.0, 40.0);
        assert_eq!((o, a), (0.0, 1.0), "waits at the start");
        let (o, _) = ticker(2.0 + 2.5, 200.0, 40.0);
        assert!((o - 100.0).abs() < 0.01, "slides at the speed: {o}");
        let (o, a) = ticker(2.0 + 5.0 + 0.5, 200.0, 40.0);
        assert_eq!((o, a), (200.0, 1.0), "holds at the end");
        let (o, a) = ticker(2.0 + 5.0 + 1.0 + 0.2, 200.0, 40.0);
        assert!(o == 200.0 && a > 0.4 && a < 0.6, "fades out at the end");
        let total = 2.0 + 5.0 + 1.0 + 0.4;
        let (o, a) = ticker(total + 0.2, 200.0, 40.0);
        assert!(o == 0.0 && a > 0.4 && a < 0.6, "back at the start, fading in: {o} {a}");
        // continuous: the opacity is 0 at the wrap and the offset jumps only there
        let (_, a0) = ticker(total - 0.001, 200.0, 40.0);
        assert!(a0 < 0.01);
    }

    #[test]
    fn a_paused_disc_stands_still_and_a_playing_one_turns_inside_its_square() {
        let b = boxed(json!({ "style": "disc" }));
        let mut r = rig(&[b.clone()], Some(demo(false)), false);
        let (p0, p1, p2) = (frame(&mut r, 0.0), frame(&mut r, 0.1), frame(&mut r, 3.0));
        assert!(p0 == p1 && p1 == p2, "paused: identical");
        let mut r = rig(&[b.clone()], Some(demo(true)), false);
        let f0 = frame(&mut r, 0.0);
        let f1 = frame(&mut r, 0.1);
        let f2 = frame(&mut r, 0.2);
        assert!(f0 != f1 && f1 != f2, "playing: it turns");
        // its square: the box minus the padding, a square of (h - 2 pad) at the left
        let rc = widget_rect(&Widget::new("x", "music", b.0, b.1, b.2, b.3, json!({})), LW, LH);
        let pad = 0.09 * rc.h;
        let side = (rc.h - 2.0 * pad).round();
        let (sx, sy) = ((rc.x + pad).round() as usize, (rc.y + pad).round() as usize);
        let bb = diff_box(&f0, &f2).unwrap();
        let m = DISC_MARGIN;
        assert!(bb.0 + m >= sx && bb.1 + m >= sy && bb.2 < sx + side as usize + m && bb.3 < sy + side as usize + m, "only the disc changes: {bb:?} vs square {sx},{sy},{side}");
    }

    #[test]
    fn the_disc_turns_by_elapsed_time_not_frames() {
        let mut r = rig(&[boxed(json!({ "style": "disc" }))], Some(demo(true)), false);
        // 40 frames of 0.1 s = 4 s = half a turn; then the same 4 s in 8 big frames of 0.5 s are capped like a hiccup
        for i in 0..=40 {
            frame(&mut r, i as f64 * 0.1);
        }
        let ang = r.mus.slots.values().next().unwrap().spin;
        assert!((ang - 180.0).abs() < 1.0, "half a turn after 4 s: {ang}");
        // spin off: stays at 0
        let mut r = rig(&[boxed(json!({ "style": "disc", "spin": false }))], Some(demo(true)), false);
        for i in 0..=20 {
            frame(&mut r, i as f64 * 0.1);
        }
        assert_eq!(r.mus.slots.values().next().unwrap().spin, 0.0);
    }

    #[test]
    fn missing_pieces_never_panic() {
        let bg = empty_frame();
        for style in ["cover", "line", "disc", "blur"] {
            // no cover from the player
            let mut np = demo(true);
            np.cover = None;
            let b = boxed(json!({ "style": style }));
            let mut r = rig(&[b.clone()], Some(np.clone()), false);
            let f = frame(&mut r, 0.0);
            assert!(inside(diff_box(&f, &bg).expect("draws"), &b), "{style} without a cover");
            // the cover switch off
            let b = boxed(json!({ "style": style, "cover": false, "progress": false }));
            let mut r = rig(&[b.clone()], Some(demo(false)), false);
            let f = frame(&mut r, 0.0);
            assert!(inside(diff_box(&f, &bg).expect("draws"), &b), "{style} with cover and progress off");
            // nothing playing, "keep an idle card"
            let mut r = rig(&[b.clone()], None, true);
            let f = frame(&mut r, 0.0);
            assert!(inside(diff_box(&f, &bg).expect("an idle card"), &b), "{style} idle");
            // nothing playing, hidden
            let mut r = rig(&[b], None, false);
            assert!(frame(&mut r, 0.0) == bg, "{style}: hidden when nothing plays");
            // a tiny and a very long, thin box
            let mut r = rig(&[(0.1, 0.1, 0.02, 0.02, json!({ "style": style })), (0.2, 0.5, 0.7, 0.05, json!({ "style": style }))], Some(demo(true)), false);
            frame(&mut r, 0.0);
            frame(&mut r, 1.0);
        }
    }

    #[test]
    fn a_new_track_swaps_the_line_by_fading() {
        let mut r = rig(&[boxed(json!({ "style": "line" }))], Some(NowPlaying::demo(LONG, "A", true, 10.0, 241.0)), false);
        r.set_sample_mode(false);
        r.music_alpha = 1.0;
        frame(&mut r, 0.0);
        let a = frame(&mut r, 1.0);
        let mut n2 = NowPlaying::demo("Another title", "B", true, 10.0, 241.0);
        n2.track_id = 2;
        r.set_music_source(Some(Arc::new(Mutex::new(Some(n2)))));
        frame(&mut r, 1.2);
        let mid = frame(&mut r, 1.4);
        let done = frame(&mut r, 2.0);
        assert!(a != mid && mid != done, "old fades out, new fades in");
    }

    #[test]
    fn props_keep_old_themes_stable() {
        let old: MusicProps = serde_json::from_str(r#"{"cover":true,"progress":true}"#).unwrap();
        assert_eq!((old.style.as_str(), old.spin), ("card", true));
        assert_eq!(serde_json::to_string(&old).unwrap(), r#"{"cover":true,"progress":true}"#, "no new fields written for the defaults");
        let new: MusicProps = serde_json::from_str(r#"{"style":"disc","spin":false}"#).unwrap();
        assert_eq!((new.style.as_str(), new.spin, new.cover), ("disc", false, true));
        assert_eq!(serde_json::to_string(&new).unwrap(), r#"{"cover":true,"progress":true,"style":"disc","spin":false}"#);
        for bad in [r#"{"style":"fancy"}"#, r#"{"style":3}"#, r#"{"style":null}"#, r#"{"style":""}"#] {
            let p: MusicProps = serde_json::from_str(bad).unwrap();
            assert_eq!(p.style, "card", "{bad}");
        }
        assert_eq!(look("fancy"), Look::Card);
        // widget props that are not an object give the defaults
        let mut w = Widget::new("m", "music", 0.0, 0.0, 0.3, 0.2, json!("junk"));
        assert_eq!(w.music().style, "card");
        w.props = json!({ "style": "blur", "cover": false });
        assert_eq!((w.music().style.as_str(), w.music().cover), ("blur", false));
        // refresh rates: the ticker and a turning disc 30, the rest 10
        let hz_of = |s: &str| hz(&serde_json::from_str::<MusicProps>(s).unwrap());
        assert_eq!(
            [hz_of(r#"{}"#), hz_of(r#"{"style":"cover"}"#), hz_of(r#"{"style":"blur"}"#), hz_of(r#"{"style":"line"}"#), hz_of(r#"{"style":"disc"}"#), hz_of(r#"{"style":"disc","spin":false}"#)],
            [10, 10, 10, 30, 30, 10]
        );
    }

    #[test]
    fn a_big_cover_is_shrunk_for_the_card_only() {
        let big = Cover { w: 640, h: 640, rgba: vec![200u8; 640 * 640 * 4] };
        let s = shrink_cover(&big).unwrap();
        assert_eq!((s.w, s.h, s.rgba.len()), (320, 320, 320 * 320 * 4));
        let small = Cover { w: 320, h: 300, rgba: vec![0u8; 320 * 300 * 4] };
        assert!(shrink_cover(&small).is_none());
    }

    /// Sample pictures and timings. `E_MUSIC_SAMPLES=<dir> cargo test --release --lib music_samples -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn music_samples_and_timings() {
        let styles = ["card", "cover", "line", "disc", "blur"];
        let long = NowPlaying::demo("Midnight City (Extended Remastered Version)", "M83", true, 83.0, 241.0);
        let save = |dir: &std::path::Path, name: &str, r: &mut Renderer, clock: f64| {
            let _ = frame(r, clock);
            std::fs::write(dir.join(format!("{name}.png")), r.frame_pixmap().encode_png().unwrap()).unwrap();
        };
        if let Some(dir) = std::env::var_os("E_MUSIC_SAMPLES") {
            let dir = PathBuf::from(dir);
            std::fs::create_dir_all(&dir).unwrap();
            for s in styles {
                let mut r = rig(&[(0.1, 0.35, 0.6, 0.3, json!({ "style": s }))], Some(long.clone()), false);
                save(&dir, &format!("music_{s}"), &mut r, 0.0);
            }
            let mut r = rig(&[(0.1, 0.35, 0.6, 0.3, json!({ "style": "line" }))], Some(long.clone()), false);
            r.set_theme({
                let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
                t.theme.widgets.clear();
                t.theme.widgets.push(Widget::new("m0", "music", 0.1, 0.35, 0.34, 0.22, json!({ "style": "line" })));
                t
            });
            save(&dir, "music_line_scrolling_default_size", &mut r, 5.0);
            let mut r = rig(&[(0.1, 0.35, 0.6, 0.3, json!({ "style": "disc" }))], Some(long.clone()), false);
            for i in 0..=24 {
                frame(&mut r, i as f64 * 0.1);
            }
            save(&dir, "music_disc_turned", &mut r, 2.5);
            let stacked: Vec<Boxed> = styles.iter().enumerate().map(|(i, s)| (0.05, 0.012 + 0.197 * i as f32, 0.6, 0.185, json!({ "style": s }))).collect();
            let mut r = rig(&stacked, Some(long.clone()), false);
            save(&dir, "music_all_stacked", &mut r, 0.0);
            let grid: Vec<Boxed> = styles.iter().enumerate().map(|(i, s)| (0.02 + 0.33 * (i % 3) as f32, 0.08 + 0.4 * (i / 3) as f32, 0.31, 0.22, json!({ "style": s }))).collect();
            let mut r = rig(&grid, Some(long.clone()), false);
            save(&dir, "music_all_default_size", &mut r, 0.0);
            let mut r = rig(&grid, None, true);
            save(&dir, "music_all_idle", &mut r, 0.0);
            println!("pictures in {}", dir.display());
        }
        // time per frame: a full render() with the widget minus the same frame without it, 100 distinct frames each
        let run = |props: Option<Value>| {
            let widgets: Vec<Boxed> = props.map(|p| vec![(0.1, 0.3, 0.34, 0.22, p)]).unwrap_or_default();
            let mut r = rig(&widgets, Some(long.clone()), false);
            let forced = |r: &mut Renderer, c: f64| {
                r.last_key = None; // redraw the widget layer and the frame every time, whatever the style
                r.layer_key = None;
                frame(r, c)
            };
            for i in 0..10 {
                forced(&mut r, 10.0 + i as f64 / 30.0);
            }
            let t0 = std::time::Instant::now();
            for i in 0..100 {
                forced(&mut r, 20.0 + i as f64 / 30.0);
            }
            t0.elapsed().as_secs_f64() * 10.0
        };
        let base = run(None);
        println!("empty theme: {base:.3} ms/frame (backdrop copy)");
        for s in styles {
            let ms = run(Some(json!({ "style": s })));
            println!("style {s}: {ms:.3} ms/frame total, {:.3} ms for the widget", ms - base);
        }
    }
}
