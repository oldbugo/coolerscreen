//! FFmpeg filter graphs of a version: framing (cover scale/crop, zoom/pan), notch band/circle, rounded corners on the
//! visible rect, loop blend, detail simplification, rotation to the stored portrait frame. Exact port of src/convert.js
//! (frameGraph, bandPx, maskGraph, visibleRect, baseGraph, cornersGraph); public/js/frame.js mirrors the same geometry.
use super::{js_round, r3};
use serde::{Deserialize, Serialize};

/// Landscape canvas (the screen as the user sees it); the stored frame is this rotated 90 deg CW (1080x2400).
pub const W: u32 = 2400;
pub const H: u32 = 1080;
/// Measured on the SC70: it always plays at about 18 frames per second, whatever the file says.
pub const DEVICE_FPS: u32 = 18;
/// A still image becomes a short clip of this length.
pub const IMAGE_SECONDS: f64 = 3.0;
/// MJPEG quantiser range: Q_MIN finest; past ffmpeg's usual 31 up to 69 with -qmax/-lmax (still 8-bit baseline JPEG).
pub const Q_MIN: u8 = 2;
pub const Q_MAX: u8 = 69;

/// Framing of the source inside the canvas. zoom is relative to "cover"; x/y (0..1) align the picture.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Frame {
    pub zoom: f64,
    pub x: f64,
    pub y: f64,
}
impl Default for Frame {
    fn default() -> Self {
        Frame { zoom: 1.0, x: 0.5, y: 0.5 }
    }
}
impl Frame {
    /// convert.js normFrame(): clamp and round to 1/1000.
    pub fn norm(self) -> Frame {
        let n = |v: f64, d: f64, lo: f64, hi: f64| if v.is_finite() { v.clamp(lo, hi) } else { d };
        Frame {
            zoom: js_round(n(self.zoom, 1.0, 0.2, 4.0) * 1000.0) / 1000.0,
            x: js_round(n(self.x, 0.5, 0.0, 1.0) * 1000.0) / 1000.0,
            y: js_round(n(self.y, 0.5, 0.0, 1.0) * 1000.0) / 1000.0,
        }
    }
    pub fn is_default(&self) -> bool {
        self.zoom == 1.0 && self.x == 0.5 && self.y == 0.5
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Band {
    /// "left" | "right" | "top" | "bottom" (landscape canvas edge)
    pub edge: Edge,
    pub px: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Circle {
    pub cx: f64,
    pub cy: f64,
    pub r: f64,
    #[serde(default)]
    pub feather: f64,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MaskMode {
    #[default]
    Off,
    Band,
    Circle,
}
/// The notch mask, same JSON shape as 1.2's settings.mask (corners are separate, see [`Corners`]).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Mask {
    #[serde(default)]
    pub mode: MaskMode,
    #[serde(default)]
    pub band: Option<Band>,
    #[serde(default)]
    pub circle: Option<Circle>,
}
impl Mask {
    pub fn band(edge: Edge, px: f64) -> Mask {
        Mask { mode: MaskMode::Band, band: Some(Band { edge, px }), circle: None }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct Corners {
    pub on: bool,
    pub r: f64,
}
impl Corners {
    pub fn is_on(&self) -> bool {
        self.on && js_round(self.r) > 0.0
    }
}

const fn even(n: i64) -> i64 {
    n - n % 2
}

/// The band rounded up so its edge lands on the 16 px MJPEG block grid of the stored portrait frame (convert.js bandPx).
pub fn band_px(band: &Band) -> u32 {
    let px = js_round(band.px) as i64;
    if px <= 0 {
        return 0;
    }
    let offset = if band.edge == Edge::Top { (1080 % 16) as i64 } else { 0 };
    (((px - offset) as f64 / 16.0).ceil() as i64 * 16 + offset) as u32
}

/// [base] -> [m] (notch applied).
pub fn mask_graph(mask: &Mask) -> String {
    let (w, h) = (W as i64, H as i64);
    match (mask.mode, mask.band, mask.circle) {
        (MaskMode::Band, Some(b), _) => {
            let px = band_px(&b) as i64;
            if px <= 0 {
                return "[base]null[m]".into();
            }
            let bx = match b.edge {
                Edge::Left => [0, 0, px, h],
                Edge::Right => [w - px, 0, px, h],
                Edge::Top => [0, 0, w, px],
                Edge::Bottom => [0, h - px, w, px],
            };
            format!("[base]drawbox=x={}:y={}:w={}:h={}:color=black:t=fill[m]", bx[0], bx[1], bx[2], bx[3])
        }
        (MaskMode::Circle, _, Some(c)) => {
            let (cx, cy, r, feather) = (c.cx, c.cy, c.r, c.feather);
            if r <= 0.0 {
                return "[base]null[m]".into();
            }
            let reach = r + feather + 2.0;
            let x0 = even(0f64.max((cx - reach).floor()) as i64);
            let y0 = even(0f64.max((cy - reach).floor()) as i64);
            let x1 = (w as f64).min((cx + reach).ceil()) as i64;
            let y1 = (h as f64).min((cy + reach).ceil()) as i64;
            let ww = match even(x1 - x0) {
                0 => 2,
                v => v,
            };
            let hh = match even(y1 - y0) {
                0 => 2,
                v => v,
            };
            let dist = format!("hypot(X+{x0}-{cx},Y+{y0}-{cy})");
            let k = if feather > 0.0 { format!("clip(({dist}-{r})/{feather},0,1)") } else { format!("gte({dist},{r})") };
            let ge = |ch: &str| format!("{ch}='{ch}(X,Y)*{k}'");
            format!(
                "[base]split[a][b];[b]crop={ww}:{hh}:{x0}:{y0},format=gbrp,geq={}:{}:{},format=yuv420p[c];[a][c]overlay={x0}:{y0},format=yuv420p[m]",
                ge("r"),
                ge("g"),
                ge("b")
            )
        }
        _ => "[base]null[m]".into(),
    }
}

/// The visible picture: the canvas minus the notch band (convert.js / frame.js visibleRect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}
pub fn visible_rect(mask: &Mask) -> Rect {
    let mut r = Rect { x0: 0, y0: 0, x1: W, y1: H };
    if let (MaskMode::Band, Some(b)) = (mask.mode, mask.band) {
        let px = band_px(&b);
        if px > 0 {
            match b.edge {
                Edge::Left => r.x0 = px,
                Edge::Right => r.x1 = W - px,
                Edge::Top => r.y0 = px,
                Edge::Bottom => r.y1 = H - px,
            }
        }
    }
    r
}

/// Scale flags; swscale slice threads (`threads=0`) are bit-exact and 1.2-1.8x faster (perf report).
pub const LANCZOS: &str = "lanczos:threads=0";

/// Framing on a `cw` x `ch` canvas (convert.js frameGraph for the 2400x1080 canvas; smaller canvases for loop cards).
pub fn frame_graph(frame: &Frame, cw: u32, ch: u32, flags: &str) -> String {
    let f = frame.norm();
    if f.is_default() {
        return format!("scale={cw}:{ch}:force_original_aspect_ratio=increase:flags={flags},crop={cw}:{ch}");
    }
    let s = format!("{}*max({cw}/iw\\,{ch}/ih)", f.zoom);
    format!(
        "scale=w='2*trunc(iw*{s}/2)':h='2*trunc(ih*{s}/2)':flags={flags},\
pad=w='max(iw\\,{cw})':h='max(ih\\,{ch})':x='2*trunc((ow-iw)*{x}/2)':y='2*trunc((oh-ih)*{y}/2)':color=black,\
crop=w={cw}:h={ch}:x='2*trunc((iw-{cw})*{x}/2)':y='2*trunc((ih-{ch})*{y}/2)'",
        x = f.x,
        y = f.y
    )
}

/// "Simplify fine detail" 0-100 -> Gaussian blur sigma 0-3.5 px on the 2400x1080 canvas.
pub fn detail_sigma(level: f64) -> f64 {
    let l = if level.is_finite() { level.clamp(0.0, 100.0) } else { 0.0 };
    js_round(l / 100.0 * 3.5 * 100.0) / 100.0
}

/// Everything baseGraph needs about a selection.
#[derive(Clone, Debug)]
pub struct Base {
    pub start: f64,
    pub length: f64,
    pub blend: f64,
    pub fps: u32,
    pub src_fps: Option<f64>,
    pub is_image: bool,
    pub frame: Frame,
    /// Simplify level 0-100.
    pub detail: f64,
    /// Canvas size and scaler flags for the framing step.
    pub canvas: (u32, u32),
    pub flags: String,
    /// Filters inserted right after [0:v] (e.g. GPU scaling + download for loop cards); empty or ending in ','.
    pub head: String,
}

/// Source -> [base]: frame rate, framing, optional detail simplification, optional loop blend (convert.js baseGraph).
pub fn base_graph(o: &Base) -> String {
    let sigma = detail_sigma(o.detail);
    let x = o.start - o.blend;
    let lead = match (o.is_image, o.src_fps) {
        (false, Some(sf)) if sf > 0.0 => 0f64.max((x * sf - 1e-6).ceil() / sf - x),
        _ => 0.0,
    };
    let shift = if lead > 1e-4 { format!("settb=AVTB,setpts=PTS+{lead:.6}/TB,") } else { String::new() };
    let pre = format!(
        "[0:v]{}{shift}fps={},{},setsar=1,format=yuv420p{}",
        o.head,
        o.fps,
        frame_graph(&o.frame, o.canvas.0, o.canvas.1, &o.flags),
        if sigma > 0.0 { format!(",gblur=sigma={sigma}") } else { String::new() }
    );
    let b = o.blend;
    if b <= 0.0 {
        return format!("{pre}[base]");
    }
    let bn = js_round(b * o.fps as f64) as i64;
    let ln = js_round(o.length * o.fps as f64) as i64;
    format!(
        "{pre},split[ba][bb];[ba]trim=start_frame={bn}:end_frame={},setpts=PTS-STARTPTS[bm];\
[bb]trim=start_frame=0:end_frame={bn},setpts=PTS-STARTPTS[bp];\
[bm][bp]xfade=transition=fade:duration={b}:offset={},format=yuv420p[base]",
        bn + ln,
        r3(o.length - b)
    )
}

/// [m] -> [mc]: the corner PNG (input 1) overlaid.
pub fn corners_graph(has_corner_file: bool) -> &'static str {
    if has_corner_file {
        "[m][1:v]overlay=0:0:format=auto,format=yuv420p[mc]"
    } else {
        "[m]null[mc]"
    }
}

/// The whole device graph ending in [out] (stored portrait 1080x2400 yuvj420p).
pub fn device_graph(base: &Base, mask: &Mask, has_corner_file: bool) -> String {
    format!("{};{};{};[mc]transpose=1,format=yuvj420p[out]", base_graph(base), mask_graph(mask), corners_graph(has_corner_file))
}

/// Input args: `-ss/-t` around the selection (with a blend, from B seconds before the start); images loop.
pub fn input_args(file: &str, base: &Base, corner_file: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = if base.is_image {
        crate::ff_args!["-loop", "1", "-framerate", base.fps, "-t", base.length, "-i", file]
    } else {
        crate::ff_args!["-ss", r3(base.start - base.blend), "-t", r3(base.length + base.blend), "-i", file]
    };
    if let Some(c) = corner_file {
        a.extend(crate::ff_args!["-i", c]);
    }
    a
}

/// mjpeg encoder args for quantiser q (q > 31 needs -qmax 69 -lmax 30000; still 8-bit baseline).
pub fn mjpeg_args(q: u8) -> Vec<String> {
    let mut a = crate::ff_args!["-c:v", "mjpeg", "-q:v", q];
    if q > 31 {
        a.extend(crate::ff_args!["-qmax", Q_MAX, "-lmax", "30000"]);
    }
    a
}

/// Friendly quality 1-100 (higher = better) <-> quantiser 69..2.
pub fn q_from_quality(quality: f64) -> u8 {
    let qq = quality.clamp(1.0, 100.0);
    js_round(Q_MAX as f64 - ((qq - 1.0) * (Q_MAX - Q_MIN) as f64) / 99.0) as u8
}
pub fn quality_from_q(q: u8) -> u8 {
    js_round(1.0 + ((Q_MAX as f64 - q as f64) * 99.0) / (Q_MAX - Q_MIN) as f64).clamp(1.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base(frame: Frame, blend: f64, detail: f64) -> Base {
        Base {
            start: 2.0,
            length: 6.0,
            blend,
            fps: 18,
            src_fps: Some(25.0),
            is_image: false,
            frame,
            detail,
            canvas: (W, H),
            flags: "lanczos".into(),
            head: String::new(),
        }
    }
    #[test]
    fn band_rounding_matches_js() {
        assert_eq!(band_px(&Band { edge: Edge::Left, px: 70.0 }), 80);
        assert_eq!(band_px(&Band { edge: Edge::Left, px: 64.0 }), 64);
        assert_eq!(band_px(&Band { edge: Edge::Top, px: 70.0 }), 72); // 1080 % 16 = 8 offset
        assert_eq!(band_px(&Band { edge: Edge::Right, px: 0.4 }), 0);
    }
    #[test]
    fn graphs_match_js_strings() {
        // Strings produced by src/convert.js (1.2.0) for the same inputs.
        assert_eq!(
            base_graph(&base(Frame::default(), 0.0, 0.0)),
            "[0:v]fps=18,scale=2400:1080:force_original_aspect_ratio=increase:flags=lanczos,crop=2400:1080,setsar=1,format=yuv420p[base]"
        );
        assert_eq!(
            frame_graph(&Frame { zoom: 1.25, x: 0.2, y: 0.7 }, W, H, "lanczos"),
            "scale=w='2*trunc(iw*1.25*max(2400/iw\\,1080/ih)/2)':h='2*trunc(ih*1.25*max(2400/iw\\,1080/ih)/2)':flags=lanczos,pad=w='max(iw\\,2400)':h='max(ih\\,1080)':x='2*trunc((ow-iw)*0.2/2)':y='2*trunc((oh-ih)*0.7/2)':color=black,crop=w=2400:h=1080:x='2*trunc((iw-2400)*0.2/2)':y='2*trunc((ih-1080)*0.7/2)'"
        );
        let circle = Mask { mode: MaskMode::Circle, band: None, circle: Some(Circle { cx: 100.5, cy: 540.0, r: 40.0, feather: 3.0 }) };
        assert_eq!(
            mask_graph(&circle),
            "[base]split[a][b];[b]crop=92:90:54:494,format=gbrp,geq=r='r(X,Y)*clip((hypot(X+54-100.5,Y+494-540)-40)/3,0,1)':g='g(X,Y)*clip((hypot(X+54-100.5,Y+494-540)-40)/3,0,1)':b='b(X,Y)*clip((hypot(X+54-100.5,Y+494-540)-40)/3,0,1)',format=yuv420p[c];[a][c]overlay=54:494,format=yuv420p[m]"
        );
        let mut b = base(Frame::default(), 0.333, 30.0);
        b.start = 2.0;
        assert_eq!(
            base_graph(&b),
            "[0:v]settb=AVTB,setpts=PTS+0.013000/TB,fps=18,scale=2400:1080:force_original_aspect_ratio=increase:flags=lanczos,crop=2400:1080,setsar=1,format=yuv420p,gblur=sigma=1.05,split[ba][bb];[ba]trim=start_frame=6:end_frame=114,setpts=PTS-STARTPTS[bm];[bb]trim=start_frame=0:end_frame=6,setpts=PTS-STARTPTS[bp];[bm][bp]xfade=transition=fade:duration=0.333:offset=5.667,format=yuv420p[base]"
        );
        assert_eq!(
            mask_graph(&Mask::band(Edge::Left, 70.0)),
            "[base]drawbox=x=0:y=0:w=80:h=1080:color=black:t=fill[m]"
        );
    }
    #[test]
    fn quality_mapping() {
        assert_eq!(q_from_quality(100.0), 2);
        assert_eq!(q_from_quality(1.0), 69);
        assert_eq!(q_from_quality(25.0), 53);
        assert_eq!(q_from_quality(45.0), 39);
        assert_eq!(quality_from_q(2), 100);
        assert_eq!(quality_from_q(69), 1);
    }
}
