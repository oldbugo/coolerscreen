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
    /// Screen fit: screen px (2400x1080 space) the panel hides at the top, right, bottom and left edge; 0 = none.
    /// The picture is scaled into the visible rectangle before the notch and the corners are applied.
    #[serde(default)]
    pub fit: crate::core::fit::Fit,
}
impl Mask {
    pub fn band(edge: Edge, px: f64) -> Mask {
        Mask { mode: MaskMode::Band, band: Some(Band { edge, px }), circle: None, fit: crate::core::fit::Fit::default() }
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

/// [base] -> [basef]: the screen fit. The picture is scaled into the visible rectangle (the panel minus the hidden edge
/// strips) and padded with black to the full canvas. The padding is done in RGB so the strips are exact to the pixel
/// (4:2:0 video can only pad on even coordinates). None when nothing is hidden.
pub fn fit_graph(fit: crate::core::fit::Fit, flags: &str) -> Option<String> {
    if fit.is_zero() {
        return None;
    }
    let (x0, y0, x1, y1) = fit.rect(W as usize, H as usize);
    let (rw, rh) = ((x1 - x0) as i64, (y1 - y0) as i64);
    // the picture goes where the shift puts it; what falls outside the canvas is cut off first (pad cannot start before 0)
    let (px0, py0, ..) = fit.placed(W as usize, H as usize);
    let (vx0, vy0) = (px0.max(0), py0.max(0));
    let (vx1, vy1) = ((px0 + rw).min(W as i64), (py0 + rh).min(H as i64));
    let (vw, vh) = ((vx1 - vx0).max(1), (vy1 - vy0).max(1));
    let crop = if (vw, vh) != (rw, rh) { format!("crop={vw}:{vh}:{}:{},", vx0 - px0, vy0 - py0) } else { String::new() };
    Some(format!(
        "[base]scale={rw}:{rh}:flags={flags},{crop}format=gbrp,pad={W}:{H}:{vx0}:{vy0}:color=black,format=yuv420p,setsar=1[basef]"
    ))
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

/// Where a video selection starts, by source frame (the loop search's startFrame convention: frame k is at k / srcFps).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    /// Selection start minus the blend (seconds; the time output frame 0 stands for).
    pub x: f64,
    /// First source frame used: the first frame at or after `x`.
    pub f0: i64,
    /// Input seek, on the source frame grid two frames before f0 (the seek only has to land early).
    pub ss: f64,
    /// Input duration from `ss` (one frame more than needed; the trim filter cuts exactly).
    pub t: f64,
    /// Frame-exact cut relative to `ss`: keep frames with trim_start <= ts < trim_end. 0.4-frame margins (trim rounds
    /// to the stream timebase, so exact half frames are ambiguous at timescale = fps), so neither
    /// the container's timescale nor the millisecond rounding of the stored start can move a frame (1.2 seeked to
    /// round(start, ms) and could start a frame late at 30/60 fps or coarse timescales).
    pub trim_start: f64,
    pub trim_end: f64,
}

/// A start stored to the millisecond that is within 1.1 ms of a source frame means exactly that frame.
pub fn snap_to_frame(start: f64, sf: f64) -> f64 {
    let k = (start * sf).round();
    if (start - k / sf).abs() <= 0.0011 {
        k / sf
    } else {
        start
    }
}

pub fn timing(o: &Base) -> Option<Timing> {
    let sf = o.src_fps.filter(|f| *f > 0.0 && !o.is_image)?;
    let fps = o.fps as f64;
    let b = (o.blend * fps).round() / fps; // the blend is whole output frames
    let x = snap_to_frame(o.start, sf) - b;
    let f0 = (x * sf - 1e-6).ceil().max(0.0) as i64;
    let s_idx = (f0 - 2).max(0);
    let ss = s_idx as f64 / sf;
    // the end: the first source frame at or after x + length + blend is not used
    let f_end = ((x + o.length + b) * sf - 1e-6).ceil() as i64;
    let trim_start = ((f0 - s_idx) as f64 - 0.4).max(0.0) / sf;
    let trim_end = ((f_end - s_idx) as f64 - 0.4) / sf;
    Some(Timing { x, f0, ss, t: trim_end + 1.0 / sf, trim_start, trim_end })
}

/// Source -> [base]: frame rate, framing, optional detail simplification, optional loop blend (convert.js baseGraph).
/// Video frames are cut by timestamp with half-frame margins (trim), rebased on the first kept frame and shifted so
/// that 0 is `x`: output frame n is the frame the fps filter picks for x + n/18, whatever the container's timescale or
/// the seek's rounding, and a loop candidate's startFrame k renders frame k first.
pub fn base_graph(o: &Base) -> String {
    let sigma = detail_sigma(o.detail);
    // fps start_time=0: output frame 0 is time x even when the first source frame comes up to a frame later (a blend
    // start off the source grid); without it the fps filter starts at that frame's slot and the stream is one frame
    // short at the front (1.2 behaviour: blended versions began one output frame late).
    let (shift, fps_opts) = match timing(o) {
        Some(t) => {
            // + 0.1 ms: at 24/48/60 fps some source frames fall exactly halfway between two output slots; the bias
            // decides those ties the same way wherever the selection starts (microsecond rounding of the lead used to
            // flip them). Far below the 1 ms+ margin of every non-tie frame, so 25/30 fps output is unchanged.
            let lead = (t.f0 as f64 / o.src_fps.unwrap() - t.x).max(0.0) + 0.0001;
            (
                format!("trim=start={:.6}:end={:.6},setpts=PTS-STARTPTS,settb=AVTB,setpts=PTS+{lead:.6}/TB,", t.trim_start, t.trim_end),
                ":start_time=0",
            )
        }
        None => (String::new(), ""),
    };
    let pre = format!(
        "[0:v]{}{shift}fps={}{fps_opts},{},setsar=1,format=yuv420p{}",
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

/// [m] -> [mc] with the optional theme overlay (3.0: text/picture items baked into the base; input 1, or input 2 when a
/// corner file is input 1) under the corners. Without an overlay this is `corners_graph`.
pub fn finish_graph(has_corner_file: bool, has_overlay: bool) -> String {
    match (has_corner_file, has_overlay) {
        (c, false) => corners_graph(c).into(),
        (false, true) => "[m][1:v]overlay=0:0:format=auto,format=yuv420p[mc]".into(),
        (true, true) => "[m][2:v]overlay=0:0:format=auto[mo];[mo][1:v]overlay=0:0:format=auto,format=yuv420p[mc]".into(),
    }
}

/// The whole device graph ending in [out] (stored portrait 1080x2400 yuvj420p).
pub fn device_graph(base: &Base, mask: &Mask, has_corner_file: bool) -> String {
    device_graph_ov(base, mask, has_corner_file, false)
}

/// As `device_graph`, with the theme overlay PNG as the last input.
pub fn device_graph_ov(base: &Base, mask: &Mask, has_corner_file: bool, has_overlay: bool) -> String {
    match fit_graph(mask.fit, &base.flags) {
        // The notch and the corners belong to the panel: they come after the fit (the mask reads the fitted picture).
        Some(f) => format!(
            "{};{};{};{};[mc]transpose=1,format=yuvj420p[out]",
            base_graph(base),
            f,
            mask_graph(mask).replacen("[base]", "[basef]", 1),
            finish_graph(has_corner_file, has_overlay)
        ),
        None => format!("{};{};{};[mc]transpose=1,format=yuvj420p[out]", base_graph(base), mask_graph(mask), finish_graph(has_corner_file, has_overlay)),
    }
}

/// Input args: `-ss/-t` around the selection (with a blend, from B seconds before the start); images loop.
pub fn input_args(file: &str, base: &Base, corner_file: Option<&str>) -> Vec<String> {
    input_args_ov(file, base, corner_file, None)
}

/// As `input_args`, plus the theme overlay PNG after the corner file.
pub fn input_args_ov(file: &str, base: &Base, corner_file: Option<&str>, overlay: Option<&str>) -> Vec<String> {
    let mut a: Vec<String> = if base.is_image {
        crate::ff_args!["-loop", "1", "-framerate", base.fps, "-t", base.length, "-i", file]
    } else if let Some(t) = timing(base) {
        crate::ff_args!["-ss", format!("{:.6}", t.ss), "-t", format!("{:.6}", t.t), "-i", file]
    } else {
        crate::ff_args!["-ss", r3(base.start - base.blend), "-t", r3(base.length + base.blend), "-i", file]
    };
    if let Some(c) = corner_file {
        a.extend(crate::ff_args!["-i", c]);
    }
    if let Some(o) = overlay {
        a.extend(crate::ff_args!["-i", o]);
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
        // (without a source frame rate there is no frame-exact cut, so the 1.2 string is unchanged)
        let mut b0 = base(Frame::default(), 0.0, 0.0);
        b0.src_fps = None;
        assert_eq!(
            base_graph(&b0),
            "[0:v]fps=18,scale=2400:1080:force_original_aspect_ratio=increase:flags=lanczos,crop=2400:1080,setsar=1,format=yuv420p[base]"
        );
        assert_eq!(
            frame_graph(&Frame { zoom: 1.25, x: 0.2, y: 0.7 }, W, H, "lanczos"),
            "scale=w='2*trunc(iw*1.25*max(2400/iw\\,1080/ih)/2)':h='2*trunc(ih*1.25*max(2400/iw\\,1080/ih)/2)':flags=lanczos,pad=w='max(iw\\,2400)':h='max(ih\\,1080)':x='2*trunc((ow-iw)*0.2/2)':y='2*trunc((oh-ih)*0.7/2)':color=black,crop=w=2400:h=1080:x='2*trunc((iw-2400)*0.2/2)':y='2*trunc((ih-1080)*0.7/2)'"
        );
        let circle = Mask { mode: MaskMode::Circle, band: None, circle: Some(Circle { cx: 100.5, cy: 540.0, r: 40.0, feather: 3.0 }), fit: crate::core::fit::Fit::default() };
        assert_eq!(
            mask_graph(&circle),
            "[base]split[a][b];[b]crop=92:90:54:494,format=gbrp,geq=r='r(X,Y)*clip((hypot(X+54-100.5,Y+494-540)-40)/3,0,1)':g='g(X,Y)*clip((hypot(X+54-100.5,Y+494-540)-40)/3,0,1)':b='b(X,Y)*clip((hypot(X+54-100.5,Y+494-540)-40)/3,0,1)',format=yuv420p[c];[a][c]overlay=54:494,format=yuv420p[m]"
        );
        // 2.0: frame-exact cut (frames 42..199 of a 25 fps source, seek from frame 40), fps slots from x = 2 - 6/18
        let mut b = base(Frame::default(), 0.333, 30.0);
        b.start = 2.0;
        assert_eq!(input_args("f.mp4", &b, None), ["-ss", "1.600000", "-t", "6.424000", "-i", "f.mp4"]);
        assert_eq!(
            base_graph(&b),
            "[0:v]trim=start=0.064000:end=6.384000,setpts=PTS-STARTPTS,settb=AVTB,setpts=PTS+0.013433/TB,fps=18:start_time=0,scale=2400:1080:force_original_aspect_ratio=increase:flags=lanczos,crop=2400:1080,setsar=1,format=yuv420p,gblur=sigma=1.05,split[ba][bb];[ba]trim=start_frame=6:end_frame=114,setpts=PTS-STARTPTS[bm];[bb]trim=start_frame=0:end_frame=6,setpts=PTS-STARTPTS[bp];[bm][bp]xfade=transition=fade:duration=0.333:offset=5.667,format=yuv420p[base]"
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

#[cfg(test)]
mod overlay_tests {
    use super::*;
    #[test]
    fn theme_overlay_goes_over_the_picture_and_under_the_corners() {
        assert_eq!(finish_graph(false, false), corners_graph(false));
        assert_eq!(finish_graph(true, false), corners_graph(true));
        assert_eq!(finish_graph(false, true), "[m][1:v]overlay=0:0:format=auto,format=yuv420p[mc]");
        // corner file is input 1, overlay input 2: the overlay is laid first, the corners last
        assert_eq!(
            finish_graph(true, true),
            "[m][2:v]overlay=0:0:format=auto[mo];[mo][1:v]overlay=0:0:format=auto,format=yuv420p[mc]"
        );
        let b = Base { start: 0.0, length: 2.0, blend: 0.0, fps: 18, src_fps: None, is_image: false, frame: Frame::default(), detail: 0.0, canvas: (W, H), flags: "lanczos".into(), head: String::new() };
        assert_eq!(input_args_ov("f.mp4", &b, Some("c.png"), Some("o.png")), ["-ss", "0", "-t", "2", "-i", "f.mp4", "-i", "c.png", "-i", "o.png"]);
        assert_eq!(input_args_ov("f.mp4", &b, None, Some("o.png")).last().map(String::as_str), Some("o.png"));
        assert_eq!(device_graph(&b, &Mask::band(Edge::Left, 96.0), false), device_graph_ov(&b, &Mask::band(Edge::Left, 96.0), false, false));
        assert!(device_graph_ov(&b, &Mask::band(Edge::Left, 96.0), false, true).contains("[m][1:v]overlay=0:0:format=auto,format=yuv420p[mc]"));
    }
}

#[cfg(test)]
mod fit_tests {
    use super::*;
    fn base() -> Base {
        Base { start: 0.0, length: 2.0, blend: 0.0, fps: 18, src_fps: None, is_image: false, frame: Frame::default(), detail: 0.0, canvas: (W, H), flags: "lanczos".into(), head: String::new() }
    }

    #[test]
    fn no_fit_leaves_the_graph_as_it_was() {
        let m = Mask::band(Edge::Left, 96.0);
        assert_eq!(fit_graph(crate::core::fit::Fit::default(), "lanczos"), None);
        assert!(!device_graph(&base(), &m, true).contains("basef"));
    }

    #[test]
    fn fit_scales_into_the_visible_rectangle_before_the_notch_and_corners() {
        // top 4, right 0, bottom 0, left 9: the visible rectangle is x 9..2400, y 4..1080 = 2391 x 1076
        let mut m = Mask::band(Edge::Left, 96.0);
        m.fit = crate::core::fit::Fit::new(4, 0, 0, 9);
        assert_eq!(
            fit_graph(m.fit, "lanczos").unwrap(),
            "[base]scale=2391:1076:flags=lanczos,format=gbrp,pad=2400:1080:9:4:color=black,format=yuv420p,setsar=1[basef]"
        );
        let g = device_graph(&base(), &m, true);
        let fit_at = g.find("[basef];").unwrap();
        let mask_at = g.find("[basef]drawbox").unwrap();
        let corners_at = g.find("[m][1:v]overlay").unwrap();
        assert!(fit_at < mask_at && mask_at < corners_at, "{g}");
        assert!(g.contains("[basef]drawbox=x=0:y=0:w=96:h=1080:color=black:t=fill[m]"), "the notch band stays where the panel is: {g}");
        assert!(g.ends_with("[mc]transpose=1,format=yuvj420p[out]"));
        // the circle mask reads the fitted picture too
        let c = Mask { mode: MaskMode::Circle, band: None, circle: Some(Circle { cx: 100.0, cy: 540.0, r: 40.0, feather: 0.0 }), fit: crate::core::fit::Fit::new(4, 0, 0, 9) };
        assert!(device_graph(&base(), &c, false).contains("[basef]split[a][b]"));
    }

    #[test]
    fn fit_is_part_of_the_request_key_through_the_mask() {
        let a = Mask::band(Edge::Left, 96.0);
        let mut b = a;
        b.fit = crate::core::fit::Fit::new(4, 0, 0, 9);
        assert_ne!(format!("{a:?}"), format!("{b:?}"));
        let j = serde_json::json!({ "mode": "band", "band": { "edge": "left", "px": 96 }, "fit": { "top": 4, "right": 0, "bottom": 0, "left": 9 } });
        assert_eq!(super::super::core_engine::parse_mask(&j).fit, crate::core::fit::Fit::new(4, 0, 0, 9));
        assert_eq!(super::super::core_engine::parse_mask(&serde_json::json!({ "mode": "off" })).fit, crate::core::fit::Fit::default());
    }
}

#[cfg(test)]
mod fit_ffmpeg_tests {
    use super::*;

    /// A real conversion of a plain white clip with the fit 4/0/0/9: in the stored portrait frame (landscape rotated clockwise,
    /// landscape (x, y) -> row x, column 1079 - y) the first 4 landscape rows are black and so are the first 9 landscape columns.
    #[test]
    fn a_real_conversion_puts_the_hidden_strips_at_the_top_and_left_as_the_user_sees_the_screen() {
        let ff = crate::media::ffmpeg::locate();
        if !ff.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (jpg, raw) = (dir.path().join("f.jpg"), dir.path().join("f.raw"));
        let mut m = Mask::band(Edge::Left, 96.0);
        m.fit = crate::core::fit::Fit::new(4, 0, 0, 9);
        let b = Base { start: 0.0, length: 1.0, blend: 0.0, fps: 18, src_fps: None, is_image: false, frame: Frame::default(), detail: 0.0, canvas: (W, H), flags: LANCZOS.into(), head: String::new() };
        let g = device_graph(&b, &m, false);
        let ok = |o: std::process::Output| o.status.success();
        let made = std::process::Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=white:s=640x360:d=1:r=18", "-filter_complex", &g, "-map", "[out]", "-frames:v", "1", "-c:v", "mjpeg", "-q:v", "2"])
            .arg(&jpg)
            .output()
            .unwrap();
        assert!(ok(made), "conversion failed");
        let dec = std::process::Command::new(&ff).args(["-v", "error", "-y", "-i"]).arg(&jpg).args(["-f", "rawvideo", "-pix_fmt", "gray"]).arg(&raw).output().unwrap();
        assert!(ok(dec));
        let d = std::fs::read(&raw).unwrap();
        assert_eq!(d.len(), 1080 * 2400);
        let at = |row: usize, col: usize| d[row * 1080 + col];
        // the notch band (96 px) covers landscape columns 0..96, so look at a row below it: landscape x = 500
        for (col, want_black) in [(1079usize, true), (1076, true), (1075, false), (1000, false)] {
            let v = at(500, col);
            assert_eq!(v < 40, want_black, "landscape row {} (portrait column {col}) = {v}", 1079 - col);
        }
        // landscape columns: the notch band hides x < 96 anyway, so check the fit on the right side of the band with a no-notch run
        let mut m2 = Mask::default();
        m2.fit = crate::core::fit::Fit::new(4, 0, 0, 9);
        let g2 = device_graph(&b, &m2, false);
        let made = std::process::Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=white:s=640x360:d=1:r=18", "-filter_complex", &g2, "-map", "[out]", "-frames:v", "1", "-c:v", "mjpeg", "-q:v", "2"])
            .arg(&jpg)
            .output()
            .unwrap();
        assert!(ok(made));
        let dec = std::process::Command::new(&ff).args(["-v", "error", "-y", "-i"]).arg(&jpg).args(["-f", "rawvideo", "-pix_fmt", "gray"]).arg(&raw).output().unwrap();
        assert!(ok(dec));
        let d = std::fs::read(&raw).unwrap();
        let at = |row: usize, col: usize| d[row * 1080 + col];
        for (row, want_black) in [(0usize, true), (8, true), (9, false), (500, false)] {
            let v = at(row, 500);
            assert_eq!(v < 40, want_black, "landscape column {row} = {v}");
        }
    }
}

#[cfg(test)]
mod shift_graph_tests {
    use super::*;
    use crate::core::fit::Fit;

    #[test]
    fn a_shift_moves_the_scaled_picture_and_cuts_what_leaves_the_canvas() {
        let f = Fit::new(4, 0, 0, 9).with_shift(8, -6);
        assert_eq!(
            fit_graph(f, "lanczos").unwrap(),
            "[base]scale=2391:1076:flags=lanczos,crop=2383:1074:0:2,format=gbrp,pad=2400:1080:17:0:color=black,format=yuv420p,setsar=1[basef]"
        );
        // a shift without insets
        assert_eq!(
            fit_graph(Fit::default().with_shift(-5, 3), "lanczos").unwrap(),
            "[base]scale=2400:1080:flags=lanczos,crop=2395:1077:5:0,format=gbrp,pad=2400:1080:0:3:color=black,format=yuv420p,setsar=1[basef]"
        );
        // inside the canvas nothing is cut
        assert_eq!(
            fit_graph(Fit::new(10, 10, 10, 10).with_shift(5, 5), "lanczos").unwrap(),
            "[base]scale=2380:1060:flags=lanczos,format=gbrp,pad=2400:1080:15:15:color=black,format=yuv420p,setsar=1[basef]"
        );
        // the fit without a shift is unchanged from before
        assert_eq!(
            fit_graph(Fit::new(4, 0, 0, 9), "lanczos").unwrap(),
            "[base]scale=2391:1076:flags=lanczos,format=gbrp,pad=2400:1080:9:4:color=black,format=yuv420p,setsar=1[basef]"
        );
    }

    /// A real conversion with inset 4/0/0/9 and a shift of 8 right, 6 up: landscape columns 0..16 and the last 6 rows are black,
    /// the top row is covered (stored rotated clockwise: landscape (x, y) -> row x, column 1079 - y).
    #[test]
    fn a_real_conversion_with_a_shift() {
        let ff = crate::media::ffmpeg::locate();
        if !ff.exists() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let (jpg, raw) = (dir.path().join("f.jpg"), dir.path().join("f.raw"));
        let mut m = Mask::default();
        m.fit = Fit::new(4, 0, 0, 9).with_shift(8, -6);
        let b = Base { start: 0.0, length: 1.0, blend: 0.0, fps: 18, src_fps: None, is_image: false, frame: Frame::default(), detail: 0.0, canvas: (W, H), flags: LANCZOS.into(), head: String::new() };
        let g = device_graph(&b, &m, false);
        let made = std::process::Command::new(&ff)
            .args(["-v", "error", "-y", "-f", "lavfi", "-i", "color=white:s=640x360:d=1:r=18", "-filter_complex", &g, "-map", "[out]", "-frames:v", "1", "-c:v", "mjpeg", "-q:v", "2"])
            .arg(&jpg)
            .output()
            .unwrap();
        assert!(made.status.success(), "{}", String::from_utf8_lossy(&made.stderr));
        let dec = std::process::Command::new(&ff).args(["-v", "error", "-y", "-i"]).arg(&jpg).args(["-f", "rawvideo", "-pix_fmt", "gray"]).arg(&raw).output().unwrap();
        assert!(dec.status.success());
        let d = std::fs::read(&raw).unwrap();
        assert_eq!(d.len(), 1080 * 2400);
        let at = |row: usize, col: usize| d[row * 1080 + col];
        let black = |v: u8| v < 40;
        // landscape y = 500 -> column 579; landscape x -> row
        assert!(black(at(16, 579)) && !black(at(17, 579)), "left edge at landscape x = 17: {} {}", at(16, 579), at(17, 579));
        // landscape x = 500 -> row 500; the top covered (y = 0 -> col 1079), bottom rows y = 1074.. black (cols 5..0)
        assert!(!black(at(500, 1079)), "top row covered");
        assert!(!black(at(500, 1079 - 1073)), "last white row y = 1073: {}", at(500, 6));
        assert!(black(at(500, 1079 - 1074)), "first black row y = 1074: {}", at(500, 5));
        assert!(black(at(500, 0)));
        // the right side runs off the canvas: the last landscape column is covered
        assert!(!black(at(2399, 579)));
    }
}
