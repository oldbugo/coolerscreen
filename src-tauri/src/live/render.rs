//! Compositor for the live screen: landscape 1680x756 (the 2400x1080 design at 0.7), drawn with tiny-skia.
//! Background (the theme's stored base, the original clip, or an animated gradient), then the theme's widgets drawn into a
//! transparent layer (each inside its box, scaled to it), composited with the live-layer fade, then the notch mask.
//! The panel is limited by the screen: motion along the landscape vertical axis tears, so nothing here moves up or down;
//! the gradient drifts sideways, the bars grow sideways, and fades only change opacity.
use super::mask::Mask;
use super::nowplaying::NowPlaying;
use super::sensors::Sensors;
use super::theme::{Guides, MusicIdle, Theme, ThemeLive, Widget};
use super::video::VideoBg;
use super::{LH, LW};
use ab_glyph::{Font as _, FontVec, PxScale, ScaleFont};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const FALLBACKS: [&str; 4] = ["msyh.ttc", "YuGothM.ttc", "malgun.ttf", "seguisym.ttf"];
mod widgets;
pub(crate) use widgets::resize_exact;
use tiny_skia::{Color, Pixmap};

struct Bmp {
    x: i32,
    y: i32,
    w: usize,
    h: usize,
    adv: f32,
    cov: Vec<u8>,
}

/// Segoe UI weights.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Weight {
    Light,
    Regular,
    Bold,
}

impl Weight {
    pub fn parse(s: &str) -> Weight {
        match s {
            "light" => Weight::Light,
            "bold" => Weight::Bold,
            _ => Weight::Regular,
        }
    }
}

fn read_font(names: &[&str]) -> Option<FontVec> {
    let dir = std::path::Path::new(r"C:\Windows\Fonts");
    names.iter().find_map(|n| std::fs::read(dir.join(n)).ok()).and_then(|b| FontVec::try_from_vec(b).ok())
}

/// System fonts (Segoe UI, falling back to Arial/Tahoma), outlines read lazily (ab_glyph keeps just the file in memory;
/// fontdue parsed every glyph up front and cost ~40 MB for two Segoe UI files). Glyph bitmaps are cached per size.
struct Fonts {
    regular: Option<FontVec>,
    bold: Option<FontVec>,
    /// Segoe UI Light, loaded the first time a light text needs it.
    light: Option<Option<FontVec>>,
    /// For characters Segoe UI lacks (CJK, symbols): loaded the first time one is needed (msyh.ttc alone is ~19 MB).
    fallback: Vec<FontVec>,
    fb_next: usize,
    fb_last_used: Option<Instant>,
    cache: HashMap<(char, u32, Weight), Bmp>,
}

impl Fonts {
    fn load() -> Fonts {
        let regular = read_font(&["segoeui.ttf", "arial.ttf", "tahoma.ttf"]);
        let bold = read_font(&["segoeuib.ttf", "arialbd.ttf", "tahomabd.ttf"]).or_else(|| read_font(&["segoeui.ttf", "arial.ttf"]));
        Fonts { regular, bold, light: None, fallback: Vec::new(), fb_next: 0, fb_last_used: None, cache: HashMap::new() }
    }

    fn primary(&mut self, w: Weight) -> Option<&FontVec> {
        match w {
            Weight::Bold => self.bold.as_ref(),
            Weight::Regular => self.regular.as_ref(),
            Weight::Light => {
                if self.light.is_none() {
                    self.light = Some(read_font(&["segoeuil.ttf", "segoeui.ttf", "arial.ttf"]));
                }
                self.light.as_ref().and_then(|l| l.as_ref()).or(self.regular.as_ref())
            }
        }
    }

    fn glyph(&mut self, c: char, px: u32, wt: Weight) -> Option<&Bmp> {
        let key = (c, px, wt);
        if !self.cache.contains_key(&key) {
            if self.cache.len() > 4000 {
                self.cache.clear();
            }
            let need_fb = {
                let p = self.primary(wt)?;
                c != ' ' && p.glyph_id(c).0 == 0
            };
            let mut fb_at: Option<usize> = None;
            if need_fb {
                self.fb_last_used = Some(Instant::now());
                // Load fallback fonts one at a time, only as far as needed (each is 10-20 MB).
                loop {
                    if let Some(i) = self.fallback.iter().position(|f| f.glyph_id(c).0 != 0) {
                        fb_at = Some(i);
                        break;
                    }
                    let Some(n) = FALLBACKS.get(self.fb_next) else { break };
                    self.fb_next += 1;
                    if let Some(f) = read_font(&[n]) {
                        self.fallback.push(f);
                    }
                }
            }
            let bmp = {
                let f = match fb_at {
                    Some(i) => &self.fallback[i],
                    None => self.primary(wt)?,
                };
                let scale = PxScale::from(px as f32);
                let id = f.glyph_id(c);
                let adv = f.as_scaled(scale).h_advance(id);
                let g = id.with_scale_and_position(scale, ab_glyph::point(0.0, 0.0));
                match f.outline_glyph(g) {
                    Some(og) => {
                        let b = og.px_bounds();
                        let (w, h) = ((b.max.x - b.min.x).ceil() as usize, (b.max.y - b.min.y).ceil() as usize);
                        let mut cov = vec![0u8; w * h];
                        og.draw(|x, y, v| {
                            let i = y as usize * w + x as usize;
                            if i < cov.len() {
                                cov[i] = (v * 255.0) as u8;
                            }
                        });
                        Bmp { x: b.min.x.floor() as i32, y: b.min.y.floor() as i32, w, h, adv, cov }
                    }
                    None => Bmp { x: 0, y: 0, w: 0, h: 0, adv, cov: Vec::new() },
                }
            };
            self.cache.insert(key, bmp);
        }
        self.cache.get(&key)
    }

    /// Free the fallback fonts when no character has needed them for a while (the glyph bitmaps stay cached).
    fn trim_fallback(&mut self) {
        if !self.fallback.is_empty() && self.fb_last_used.is_some_and(|t| t.elapsed() > Duration::from_secs(10)) {
            self.fallback.clear();
            self.fallback.shrink_to_fit();
            self.fb_next = 0;
        }
    }

    fn width(&mut self, s: &str, px: u32, wt: Weight) -> f32 {
        s.chars().map(|c| self.glyph(c, px, wt).map(|g| g.adv).unwrap_or(0.0)).sum()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LocalTime {
    pub year: u16,
    pub month: u16,
    pub weekday: u16,
    pub day: u16,
    pub hour: u16,
    pub minute: u16,
    pub second: u16,
    pub millis: u16,
}

pub fn local_time() -> LocalTime {
    use windows_sys::Win32::Foundation::SYSTEMTIME;
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut s: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut s) };
    LocalTime { year: s.wYear, month: s.wMonth, weekday: s.wDayOfWeek, day: s.wDay, hour: s.wHour, minute: s.wMinute, second: s.wSecond, millis: s.wMilliseconds }
}

/// A widget box in canvas pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// The widget's box on a `cw` x `ch` canvas (boxes are fractions of the landscape canvas).
pub fn widget_rect(w: &Widget, cw: usize, ch: usize) -> Rect {
    Rect { x: (w.x * cw as f32).round(), y: (w.y * ch as f32).round(), w: (w.w * cw as f32).round().max(1.0), h: (w.h * ch as f32).round().max(1.0) }
}

/// Where the live background goes at the end of a session (hand-over).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum HandTarget {
    /// Cross-fade to the first frame of the stored base (the screen's loop then continues from it).
    FirstFrame,
    /// Fade to black (screen-off ending).
    Black,
}

struct ImgEntry {
    mtime: Option<std::time::SystemTime>,
    src: Option<Arc<Pixmap>>,
    /// Scaled copy for the current box: (box w, box h, cover, pixmap, offset x, offset y).
    scaled: Option<(u32, u32, bool, Pixmap, i32, i32)>,
}

pub struct Renderer {
    cw: usize,
    ch: usize,
    /// The finished opaque frame (live canvas size).
    pm: Pixmap,
    /// Widgets, premultiplied RGBA, transparent where nothing is drawn.
    layer: Pixmap,
    fonts: Fonts,
    sensors: Arc<Mutex<Sensors>>,
    video: Option<VideoBg>,
    /// A fixed background picture (preview); wins over the video.
    still: Option<Arc<Vec<u8>>>,
    t0: Instant,
    /// Smoothed bar values (0..1): cpu, ram, gpu load, gpu temp, gpu clock, cpu temp.
    bars: [f32; 6],
    /// Animation ticks (10 per second): everything that moves on its own is stepped per tick, so a frame is only redrawn when
    /// the tick, the clock second or the background video frame changed.
    tick: u64,
    bars_tick: u64,
    last_key: Option<(u64, usize, u16, u64, u64)>,
    mask: Mask,
    /// Now-playing source (shared with the watcher thread), the fade of its card, and the per-track cover scaled for the card.
    music: Option<Arc<Mutex<Option<NowPlaying>>>>,
    music_alpha: f32,
    music_tick: u64,
    cover: Option<(u64, usize, Vec<u8>)>,
    theme: ThemeLive,
    images: HashMap<PathBuf, ImgEntry>,
    /// Union of the widget boxes (the only part of the layer that is cleared and composited).
    rects: Vec<(usize, usize, usize, usize)>,
    /// Live-layer opacity (fade in at the start, fade out at the hand-over).
    live_alpha: f32,
    /// Hand-over: background blend toward a target (0..1).
    hand: Option<(HandTarget, f32)>,
    /// First frame of the stored base (filled by a helper thread).
    first: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    /// The background is the stored base: its `onBase` items are already in the picture and are not drawn again.
    bg_is_base: bool,
    /// Preview: fixed music state (no fade animation).
    sample: bool,
    /// Calibration overlay (screen guides) drawn over every frame while Some; `cal_gen` changes with it.
    calib: Option<Guides>,
    cal_gen: u64,
    /// Accumulated microseconds per stage (background, widgets, composite + mask), for profiling.
    pub timing_us: [u64; 3],
}

const CLOCK_CAP: f32 = 0.72;
const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

impl Renderer {
    pub fn new(sensors: Arc<Mutex<Sensors>>, video: Option<VideoBg>) -> Renderer {
        Renderer::new_sized(LW, LH, sensors, video)
    }

    /// A renderer for another canvas size (the 2400x1080 base overlay); all layout comes from the box fractions.
    pub fn new_sized(cw: usize, ch: usize, sensors: Arc<Mutex<Sensors>>, video: Option<VideoBg>) -> Renderer {
        let mut r = Renderer {
            cw,
            ch,
            pm: Pixmap::new(cw as u32, ch as u32).unwrap(),
            layer: Pixmap::new(cw as u32, ch as u32).unwrap(),
            fonts: Fonts::load(),
            sensors,
            video,
            still: None,
            t0: Instant::now(),
            bars: [0.0; 6],
            tick: 0,
            bars_tick: 0,
            last_key: None,
            mask: Mask::none(),
            music: None,
            music_alpha: 0.0,
            music_tick: 0,
            cover: None,
            theme: ThemeLive::new(Theme::standard_preset(), PathBuf::new()),
            images: HashMap::new(),
            rects: Vec::new(),
            live_alpha: 1.0,
            hand: None,
            first: Arc::new(Mutex::new(None)),
            bg_is_base: false,
            sample: false,
            calib: None,
            cal_gen: 0,
            timing_us: [0; 3],
        };
        r.update_bbox();
        r
    }

    /// Use fixed sample data and no fade animation (editor preview).
    pub fn set_sample_mode(&mut self, on: bool) {
        self.sample = on;
        if on {
            self.music_alpha = 1.0;
        }
    }

    /// Draw the calibration guides over every frame (None: normal frames).
    pub fn set_calibration(&mut self, g: Option<Guides>) {
        if self.calib != g {
            self.calib = g;
            self.cal_gen += 1;
            self.last_key = None;
        }
    }

    /// Where now-playing comes from (None: no music card).
    pub fn set_music_source(&mut self, m: Option<Arc<Mutex<Option<NowPlaying>>>>) {
        self.music = m;
        self.last_key = None;
    }

    /// Replace the background clip (None: the animated gradient).
    pub fn set_video(&mut self, v: Option<VideoBg>) {
        self.video = v;
        self.last_key = None;
    }

    /// Replace the background with a fixed picture (landscape LW x LH RGBA): used by the preview.
    pub fn set_still_background(&mut self, rgba: Option<Arc<Vec<u8>>>) {
        self.still = rgba;
        self.last_key = None;
    }

    /// Swap the theme (widgets); takes effect on the next frame.
    pub fn set_theme(&mut self, t: ThemeLive) {
        self.theme = t;
        self.theme.theme.clamp();
        // Pictures stay loaded across theme edits unless the file changed.
        self.images.retain(|p, e| std::fs::metadata(p).and_then(|m| m.modified()).ok() == e.mtime);
        self.update_bbox();
        self.last_key = None;
    }

    pub fn theme(&self) -> &ThemeLive {
        &self.theme
    }

    pub fn has_widget(&self, kind: &str) -> bool {
        self.theme.theme.widgets.iter().any(|w| w.kind == kind && self.theme.theme.is_visible(w))
    }

    /// The background is the stored base (true) or something else (false); the base already contains its `onBase` items.
    pub fn set_bg_is_base(&mut self, v: bool) {
        if self.bg_is_base != v {
            self.bg_is_base = v;
            self.last_key = None;
        }
    }

    /// Live-layer opacity 0..1 (fade in / out).
    pub fn set_live_alpha(&mut self, a: f32) {
        self.live_alpha = a.clamp(0.0, 1.0);
    }

    /// Hand-over blend of the background toward `target` (None: off).
    pub fn set_handover(&mut self, h: Option<(HandTarget, f32)>) {
        self.hand = h.map(|(k, t)| (k, t.clamp(0.0, 1.0)));
    }

    /// Slot the first frame of the stored base is delivered into.
    pub fn first_frame_slot(&self) -> Arc<Mutex<Option<Arc<Vec<u8>>>>> {
        self.first.clone()
    }

    pub fn has_first_frame(&self) -> bool {
        self.first.lock().unwrap().is_some()
    }

    /// Number of frames the background decoder delivered so far (the loop position is this modulo the loop length).
    pub fn video_frame_index(&self) -> u64 {
        self.video.as_ref().map(|v| v.index.load(std::sync::atomic::Ordering::Relaxed)).unwrap_or(0)
    }

    /// Set the notch mask (applied as the last step).
    pub fn set_mask(&mut self, m: Mask) {
        if m != self.mask {
            self.mask = m;
            self.last_key = None;
        }
    }

    /// Force the next render to redraw (for benchmarks).
    pub fn invalidate(&mut self) {
        self.last_key = None;
    }

    pub fn has_fonts(&self) -> bool {
        self.fonts.regular.is_some()
    }

    /// A real background picture is available (a decoded video frame or the preview still).
    pub fn has_background_frame(&self) -> bool {
        self.still.is_some() || self.video.as_ref().is_some_and(|v| v.frame().is_some())
    }

    pub fn video(&self) -> Option<&VideoBg> {
        self.video.as_ref()
    }

    fn update_bbox(&mut self) {
        let mut rects: Vec<(usize, usize, usize, usize)> = Vec::new();
        for w in &self.theme.theme.widgets {
            let r = widget_rect(w, self.cw, self.ch);
            let m = 8.0;
            let (x0, y0) = ((r.x - m).max(0.0) as usize, (r.y - m).max(0.0) as usize);
            let (x1, y1) = (((r.x + r.w + m) as usize).min(self.cw), ((r.y + r.h + m) as usize).min(self.ch));
            rects.push((x0, y0, x1, y1));
        }
        // Merge overlapping boxes: the layer is composited once per region (twice would apply it twice).
        loop {
            let mut merged = false;
            'outer: for i in 0..rects.len() {
                for j in i + 1..rects.len() {
                    let (a, b) = (rects[i], rects[j]);
                    if a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3 {
                        rects[i] = (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3));
                        rects.remove(j);
                        merged = true;
                        break 'outer;
                    }
                }
            }
            if !merged {
                break;
            }
        }
        self.rects = rects;
    }
}

impl Renderer {
    /// Draw one frame; returns premultiplied RGBA (everything is opaque, so it equals plain RGBA), LW x LH.
    /// Returns the frame and whether it differs from the previous call (when not, nothing was redrawn).
    pub fn render(&mut self) -> (&[u8], bool) {
        let t = self.t0.elapsed().as_secs_f32();
        self.tick = (t * 10.0) as u64;
        let now = local_time();
        let vframe = if self.still.is_some() { self.still.clone() } else { self.video.as_ref().and_then(|v| v.frame()) };
        let vkey = vframe.as_ref().map(|a| Arc::as_ptr(a) as usize).unwrap_or(0);
        // Now playing: fade the card in and out (stepped per tick), and redraw when track, state, a progress step (3 per
        // second) or the fade changed.
        let wants_music = self.has_widget("music");
        let keep_idle = self.theme.theme.handover.music_idle == MusicIdle::Keep;
        let np = if wants_music { self.music.as_ref().and_then(|m| m.lock().unwrap().clone()) } else { None };
        let want = np.as_ref().is_some_and(|n| n.visible()) || keep_idle;
        if self.sample {
            self.music_alpha = if want { 1.0 } else { 0.0 };
        } else if self.tick != self.music_tick {
            let steps = (self.tick - self.music_tick).min(10) as f32;
            self.music_tick = self.tick;
            let target = if want { 1.0 } else { 0.0 };
            self.music_alpha += (target - self.music_alpha).clamp(-0.2 * steps, 0.2 * steps);
        }
        let np = if np.as_ref().is_some_and(|n| n.visible()) { np } else { None };
        let show_music = self.music_alpha > 0.01;
        let music_key = match &np {
            Some(n) => (n.track_id << 24) ^ ((n.playing as u64) << 20) ^ (((n.position() * 3.0) as u64) << 4) ^ (self.music_alpha * 5.0) as u64,
            None => (self.music_alpha * 5.0) as u64,
        };
        // Fade state (live-layer opacity and hand-over blend in 1/64 steps) is part of the key so fades redraw every frame.
        let hand_key = self.hand.map(|(k, p)| ((p * 64.0) as u64 + 1) | ((k == HandTarget::Black) as u64) << 8).unwrap_or(0);
        let fade_key = (((self.live_alpha * 64.0) as u64) << 12 | hand_key) ^ (self.cal_gen << 40);
        // With a video the widgets follow the video frames (no separate 10 Hz redraw on top of them).
        let key = (if vkey != 0 { 0 } else { self.tick }, vkey, now.second, music_key, fade_key);
        if self.last_key == Some(key) {
            return (self.pm.data(), false);
        }
        self.last_key = Some(key);
        self.fonts.trim_fallback();
        let s = self.sensors.lock().unwrap().clone();
        let t0 = Instant::now();
        self.background(t, vframe);
        self.apply_handover();
        let t1 = Instant::now();
        if self.live_alpha > 0.004 && !self.rects.is_empty() {
            let np_ref = if show_music { np.as_ref() } else { None };
            self.draw_widgets(&now, &s, np_ref, false);
            self.composite(self.live_alpha);
        }
        let t2 = Instant::now();
        if !self.mask.is_none() {
            self.mask.apply(self.pm.data_mut());
        }
        self.draw_calibration();
        let t3 = Instant::now();
        self.timing_us[0] += (t1 - t0).as_micros() as u64;
        self.timing_us[1] += (t2 - t1).as_micros() as u64;
        self.timing_us[2] += (t3 - t2).as_micros() as u64;
        (self.pm.data(), true)
    }

    /// Cross-fade / fade the finished background toward the hand-over target.
    fn apply_handover(&mut self) {
        let Some((target, p)) = self.hand else { return };
        if p <= 0.0 {
            return;
        }
        let k = (p.clamp(0.0, 1.0) * 256.0) as u32;
        let first = self.first.lock().unwrap().clone();
        let d = self.pm.data_mut();
        match (target, first) {
            (HandTarget::FirstFrame, Some(f)) if f.len() == d.len() => {
                if p >= 1.0 {
                    d.copy_from_slice(&f);
                } else {
                    for (dp, fp) in d.chunks_exact_mut(4).zip(f.chunks_exact(4)) {
                        for c in 0..3 {
                            dp[c] = ((dp[c] as u32 * (256 - k) + fp[c] as u32 * k) >> 8) as u8;
                        }
                    }
                }
            }
            (HandTarget::Black, _) => {
                for dp in d.chunks_exact_mut(4) {
                    for c in 0..3 {
                        dp[c] = ((dp[c] as u32 * (256 - k)) >> 8) as u8;
                    }
                }
            }
            _ => {}
        }
    }

    /// Source-over of the widget layer (premultiplied) onto the opaque frame, scaled by `alpha`, inside the widget regions.
    fn composite(&mut self, alpha: f32) {
        let g = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
        let cw = self.cw;
        let src = self.layer.data();
        let dst = self.pm.data_mut();
        for &(x0, y0, x1, y1) in &self.rects {
            let w = x1 - x0;
            for y in y0..y1 {
                let s = &src[(y * cw + x0) * 4..][..w * 4];
                let d = &mut dst[(y * cw + x0) * 4..][..w * 4];
                for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                    let sa = (sp[3] as u32 * g) >> 8;
                    let inv = 255 - sa;
                    for c in 0..3 {
                        let sc = (sp[c] as u32 * g) >> 8;
                        // x / 255 for x < 65536
                        dp[c] = (sc + ((dp[c] as u32 * inv * 32897) >> 23)).min(255) as u8;
                    }
                }
            }
        }
    }

    /// Clear the widget regions of the layer.
    fn clear_layer(&mut self) {
        let cw = self.cw;
        let d = self.layer.data_mut();
        for &(x0, y0, x1, y1) in &self.rects {
            for y in y0..y1 {
                d[(y * cw + x0) * 4..(y * cw + x1) * 4].fill(0);
            }
        }
    }

    /// Draw the widgets into the (cleared) layer. `only_base`: only the `onBase` items (for the base overlay); otherwise all
    /// widgets, except that `onBase` items are skipped when the background is the stored base (they are already in it).
    fn draw_widgets(&mut self, now: &LocalTime, s: &Sensors, np: Option<&NowPlaying>, only_base: bool) {
        self.clear_layer();
        let widgets = std::mem::take(&mut self.theme.theme.widgets);
        // (the preview has no animation history: show the sample values at once)
        let steps = if self.sample { 40 } else { (self.tick - self.bars_tick.min(self.tick)).min(20) as i32 };
        self.step_bars(s, steps);
        if steps > 0 {
            self.bars_tick = self.tick;
        }
        for w in &widgets {
            if !self.theme.theme.is_visible(w) {
                continue;
            }
            let on_base = w.on_base && w.can_be_on_base();
            if only_base && !on_base {
                continue;
            }
            if !only_base && on_base && self.bg_is_base {
                continue;
            }
            let r = widget_rect(w, self.cw, self.ch);
            match w.kind.as_str() {
                "clock" => self.draw_clock(r, &w.clock(), now),
                "music" => self.draw_music(r, &w.music(), np),
                "stats" => self.draw_stats(r, &w.stats(), s),
                "text" => self.draw_text(r, &w.text()),
                "image" => self.draw_image(r, w),
                "weather" => self.draw_weather(r),
                _ => {}
            }
        }
        self.theme.theme.widgets = widgets;
    }

    /// The base overlay: only the `onBase` items, on a transparent layer of this renderer's canvas size, as a PNG
    /// (None when the theme has no such item).
    pub fn base_overlay_png(&mut self) -> Result<Option<Vec<u8>>, String> {
        if !self.theme.theme.has_base_items() {
            return Ok(None);
        }
        // The overlay covers the whole canvas.
        let (cw, ch) = (self.cw, self.ch);
        self.rects = vec![(0, 0, cw, ch)];
        let now = local_time();
        let s = Sensors::default();
        self.draw_widgets(&now, &s, None, true);
        let png = self.layer.encode_png().map_err(|e| format!("PNG encode failed: {e}"));
        self.update_bbox();
        png.map(Some)
    }

    /// The finished frame so far (after `render`), as a pixmap copy for the preview.
    pub fn frame_pixmap(&self) -> Pixmap {
        self.pm.clone()
    }

    /// The widget layer as it stands (premultiplied), for the overlay export.
    pub fn layer_pixmap(&self) -> &Pixmap {
        &self.layer
    }

    fn background(&mut self, t: f32, frame: Option<Arc<Vec<u8>>>) {
        if let Some(f) = frame {
            if f.len() == self.pm.data().len() {
                self.pm.data_mut().copy_from_slice(&f);
                for p in self.pm.data_mut().chunks_exact_mut(4) {
                    p[3] = 255;
                }
                return;
            }
        }
        // Slow colour drift and a gradient sliding sideways (repeats seamlessly: first and last stop are equal).
        // Quantised to 10 steps a second: smooth to the eye, and an unchanged frame is not re-encoded (see pipeline).
        let t = (t * 10.0).floor() / 10.0;
        let hue = (t * 4.0) % 360.0;
        let a = hsv(hue, 0.55, 0.33);
        let b = hsv(hue + 70.0, 0.60, 0.20);
        let (cw, ch) = (self.cw, self.ch);
        let shift = (t * 36.0) % (cw as f32);
        // The gradient only varies along x: compute one row and copy it down (a full tiny-skia gradient fill was ~25 ms).
        let (ca, cb) = ([a.red(), a.green(), a.blue()], [b.red(), b.green(), b.blue()]);
        let data = self.pm.data_mut();
        for x in 0..cw {
            let u = ((x as f32 + shift) / cw as f32).fract();
            let f = 1.0 - (2.0 * u - 1.0).abs();
            for k in 0..3 {
                data[x * 4 + k] = ((ca[k] + (cb[k] - ca[k]) * f) * 255.0) as u8;
            }
            data[x * 4 + 3] = 255;
        }
        for y in 1..ch {
            data.copy_within(0..cw * 4, y * cw * 4);
        }
    }
}

// ---------------------------------------------------------------- drawing helpers (shared with widgets.rs)

/// Fill a rounded rectangle on a premultiplied pixmap with plain RGBA source-over, with anti-aliased edges. Done by hand: a
/// tiny-skia path fill of a 900x400 card took ~4 ms here, this takes well under 1 ms (solid spans are written directly).
pub(crate) fn round_rect(pm: &mut Pixmap, x: f32, y: f32, w: f32, h: f32, r: f32, rgba: [u8; 4]) {
    if rgba[3] == 0 || w <= 0.0 || h <= 0.0 {
        return;
    }
    let (dw, dh) = (pm.width() as i32, pm.height() as i32);
    let r = r.max(0.0).min(w / 2.0).min(h / 2.0);
    let (x0, x1) = ((x.floor() as i32).max(0), ((x + w).ceil() as i32).min(dw));
    let (y0, y1) = ((y.floor() as i32).max(0), ((y + h).ceil() as i32).min(dh));
    let a = rgba[3] as u32;
    let pre = [(rgba[0] as u32 * a + 127) / 255, (rgba[1] as u32 * a + 127) / 255, (rgba[2] as u32 * a + 127) / 255];
    let data = pm.data_mut();
    // source-over of the colour at coverage `cov` (0..1) onto one pixel
    let put = |px: &mut [u8], cov: f32| {
        let (sa, k) = if cov >= 0.999 { (a, 256u32) } else { ((a as f32 * cov) as u32, (cov * 256.0) as u32) };
        if sa == 0 {
            return;
        }
        if px[3] == 0 {
            px[0] = ((pre[0] * k) >> 8) as u8;
            px[1] = ((pre[1] * k) >> 8) as u8;
            px[2] = ((pre[2] * k) >> 8) as u8;
            px[3] = sa as u8;
        } else {
            let inv = 255 - sa;
            for c in 0..3 {
                px[c] = (((pre[c] * k) >> 8) + px[c] as u32 * inv / 255).min(255) as u8;
            }
            px[3] = (sa + px[3] as u32 * inv / 255).min(255) as u8;
        }
    };
    let (rl, rr) = (x + r, x + w - r);
    let (rt, rb) = (y + r, y + h - r);
    for py in y0..y1 {
        let covy = ((py + 1) as f32 - y).clamp(0.0, 1.0) * ((y + h) - py as f32).clamp(0.0, 1.0);
        let fy = py as f32 + 0.5;
        let corner_row = fy < rt || fy > rb;
        let row = &mut data[(py * dw) as usize * 4..][..dw as usize * 4];
        // columns that need per-pixel coverage: the first and last r+1 pixels
        let (zl, zr) = (((rl.ceil() as i32) + 1).clamp(x0, x1), ((rr.floor() as i32) - 1).clamp(x0, x1));
        let (zl, zr) = if zl > zr { (zl, zl) } else { (zl, zr) };
        for px in x0..zl.min(x1) {
            let cov = edge_cov(px, py, x, y, w, h, r, corner_row, covy);
            put(&mut row[px as usize * 4..][..4], cov);
        }
        for px in zl..zr {
            if covy >= 0.999 {
                // solid middle span
                let p = &mut row[px as usize * 4..][..4];
                if p[3] == 0 {
                    p[0] = pre[0] as u8;
                    p[1] = pre[1] as u8;
                    p[2] = pre[2] as u8;
                    p[3] = a as u8;
                } else {
                    put(p, 1.0);
                }
            } else {
                put(&mut row[px as usize * 4..][..4], covy);
            }
        }
        for px in zr.max(zl)..x1 {
            let cov = edge_cov(px, py, x, y, w, h, r, corner_row, covy);
            put(&mut row[px as usize * 4..][..4], cov);
        }
    }
}

/// Coverage of pixel (px, py) by the rounded rectangle, for pixels near its left/right edge.
fn edge_cov(px: i32, py: i32, x: f32, y: f32, w: f32, h: f32, r: f32, corner_row: bool, covy: f32) -> f32 {
    let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
    if r > 0.0 && corner_row && (fx < x + r || fx > x + w - r) {
        // signed distance to the rounded corner
        let cx = if fx < x + r { x + r } else { x + w - r };
        let cy = if fy < y + r { y + r } else { y + h - r };
        let d = ((fx - cx).powi(2) + (fy - cy).powi(2)).sqrt();
        return (r + 0.5 - d).clamp(0.0, 1.0);
    }
    let covx = ((px + 1) as f32 - x).clamp(0.0, 1.0) * ((x + w) - px as f32).clamp(0.0, 1.0);
    covx * covy
}

fn hsv(h: f32, s: f32, v: f32) -> Color {
    let h = h.rem_euclid(360.0) / 60.0;
    let (c, i) = (v * s, h.floor() as i32);
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match i {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    Color::from_rgba(r + m, g + m, b + m, 1.0).unwrap_or(Color::BLACK)
}

/// Blend a coverage bitmap onto a premultiplied RGBA layer.
pub(crate) fn blend_cov(dst: &mut [u8], dw: usize, dh: usize, cov: &[u8], w: usize, h: usize, x0: i32, y0: i32, rgb: [u8; 3], alpha: f32) {
    for row in 0..h {
        let y = y0 + row as i32;
        if y < 0 || y >= dh as i32 {
            continue;
        }
        for col in 0..w {
            let x = x0 + col as i32;
            if x < 0 || x >= dw as i32 {
                continue;
            }
            let a = cov[row * w + col] as f32 / 255.0 * alpha;
            if a <= 0.0 {
                continue;
            }
            let i = (y as usize * dw + x as usize) * 4;
            let inv = 1.0 - a;
            for k in 0..3 {
                dst[i + k] = (rgb[k] as f32 * a + dst[i + k] as f32 * inv).min(255.0) as u8;
            }
            dst[i + 3] = (255.0 * a + dst[i + 3] as f32 * inv).min(255.0) as u8;
        }
    }
}

/// Source-over of a premultiplied tile (w x h) onto the premultiplied layer at (x0, y0) with an extra fade 0..=255.
pub(crate) fn blend_tile(dst: &mut [u8], dw: usize, dh: usize, tile: &[u8], x0: i32, y0: i32, w: usize, h: usize, fade: u32) {
    for row in 0..h {
        let y = y0 + row as i32;
        if y < 0 || y >= dh as i32 {
            continue;
        }
        for col in 0..w {
            let x = x0 + col as i32;
            if x < 0 || x >= dw as i32 {
                continue;
            }
            let sp = &tile[(row * w + col) * 4..][..4];
            if sp[3] == 0 {
                continue;
            }
            let sa = sp[3] as u32 * fade / 255;
            let i = (y as usize * dw + x as usize) * 4;
            let inv = 255 - sa;
            for k in 0..3 {
                let s = sp[k] as u32 * fade / 255;
                dst[i + k] = (s + dst[i + k] as u32 * inv / 255).min(255) as u8;
            }
            dst[i + 3] = (sa + dst[i + 3] as u32 * inv / 255).min(255) as u8;
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::theme::Theme;

    fn png_size(png: &[u8]) -> (u32, u32) {
        let p = Pixmap::decode_png(png).unwrap();
        (p.width(), p.height())
    }

    #[test]
    fn widget_boxes_are_fractions_of_the_canvas() {
        let w = Widget::new("a", "text", 0.25, 0.5, 0.5, 0.25, serde_json::json!({}));
        assert_eq!(widget_rect(&w, 1680, 756), Rect { x: 420.0, y: 378.0, w: 840.0, h: 189.0 });
        assert_eq!(widget_rect(&w, 2400, 1080), Rect { x: 600.0, y: 540.0, w: 1200.0, h: 270.0 });
    }

    #[test]
    fn preview_and_overlay_have_the_requested_sizes() {
        let mut t = ThemeLive::new(Theme::standard_preset(), PathBuf::new());
        t.theme.widgets.push(Widget { id: "t".into(), kind: "text".into(), x: 0.1, y: 0.1, w: 0.5, h: 0.2, on_base: true, props: serde_json::json!({ "text": "Hi" }), ..Widget::default() });
        let p = crate::live::render_preview(&t, 640, 288, true).unwrap();
        assert_eq!(png_size(&p), (640, 288));
        let b = crate::live::render_preview(&t, 640, 288, false).unwrap();
        assert_eq!(png_size(&b), (640, 288));
        let o = crate::live::render_base_overlay(&t).unwrap().unwrap();
        assert_eq!(png_size(&o), (2400, 1080));
        let none = ThemeLive::new(Theme::standard_preset(), PathBuf::new());
        assert!(crate::live::render_base_overlay(&none).unwrap().is_none());
    }

    #[test]
    fn rounded_rect_fill_is_clipped_and_antialiased() {
        let mut pm = Pixmap::new(100, 60).unwrap();
        round_rect(&mut pm, 10.0, 10.0, 50.0, 30.0, 10.0, [255, 0, 0, 255]);
        let px = |x: usize, y: usize| pm.data()[(y * 100 + x) * 4..][..4].to_vec();
        assert_eq!(px(30, 25), vec![255, 0, 0, 255]);
        assert_eq!(px(10, 10)[3], 0); // cut corner
        assert_eq!(px(70, 25)[3], 0);
        let edge = px(11, 14)[3];
        assert!(edge > 0 && edge < 255, "corner is anti-aliased: {edge}");
        // off-canvas rectangles must not panic
        round_rect(&mut pm, -20.0, -20.0, 500.0, 500.0, 30.0, [0, 0, 255, 128]);
    }

    #[test]
    fn handover_black_and_fade() {
        let sensors = Arc::new(Mutex::new(Sensors::default()));
        let mut r = Renderer::new(sensors, None);
        r.set_theme(ThemeLive::new(Theme::default(), PathBuf::new()));
        r.set_handover(Some((HandTarget::Black, 1.0)));
        let f = r.render().0.to_vec();
        assert!(f.chunks_exact(4).all(|p| p[..3] == [0, 0, 0]));
    }

    #[test]
    fn hidden_widgets_and_live_layer_off() {
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        t.theme.widgets.push(Widget { id: "t".into(), kind: "text".into(), on_base: true, hidden: true, props: serde_json::json!({ "text": "x" }), ..Widget::default() });
        assert!(crate::live::render_base_overlay(&t).unwrap().is_none());
        t.theme.widgets.push(Widget { id: "m".into(), kind: "music".into(), ..Widget::default() });
        t.theme.live_layer = false;
        let live = crate::live::render_preview(&t, 320, 144, true).unwrap();
        t.theme.widgets.clear();
        let base = crate::live::render_preview(&t, 320, 144, true).unwrap();
        assert_eq!(png_size(&live), png_size(&base));
    }

    #[test]
    fn calibration_draws_guides_only_while_set() {
        let mut r = Renderer::new(Arc::new(Mutex::new(Sensors::default())), None);
        r.set_theme(ThemeLive::new(Theme::default(), PathBuf::new()));
        let plain = r.render().0.to_vec();
        let g = Guides::from_value(&serde_json::json!({ "curve": { "right": 200 }, "zones": [{ "name": "RAM", "edge": "right", "size": 180 }] }));
        r.set_calibration(Some(g));
        let (f, changed) = r.render();
        assert!(changed, "a change redraws at once");
        // the curve line is yellow at 2400-200 px = canvas x 1540
        let i = (300 * LW + 1540) * 4;
        assert_eq!(&f[i..i + 3], &[255, 235, 0]);
        r.set_calibration(None);
        let back = r.render().0.to_vec();
        assert_eq!(back.len(), plain.len());
        assert!(back.chunks_exact(4).zip(plain.chunks_exact(4)).all(|(a, b)| a[..3].iter().zip(&b[..3]).all(|(x, y)| x.abs_diff(*y) < 8)), "normal frame again");
    }
}
