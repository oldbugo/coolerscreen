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
mod extras;
mod more;
mod music;
mod progress;
mod sunmoon;
mod todo;
mod headlines;
mod webpicture;
mod sensors;
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

/// A group of widgets whose boxes overlap (so they are drawn together, in order), redrawn when its own time slot,
/// the clock second or the music state changed.
struct Cluster {
    rect: (usize, usize, usize, usize),
    fit: Option<(usize, usize, usize, usize)>,
    widgets: Vec<usize>,
    hz: u32,
    key: Option<(u64, u16, u64)>,
    last_tick: u64,
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
    last_key: Option<(u64, usize, u16, u64, u64)>,
    mask: Mask,
    /// Now-playing source (shared with the watcher thread), the fade of its card, and the per-track cover scaled for the card.
    music: Option<Arc<Mutex<Option<NowPlaying>>>>,
    music_alpha: f32,
    music_tick: u64,
    cover: Option<(u64, usize, Vec<u8>)>,
    /// Caches and clock of the four extra music looks (render/music.rs).
    mus: music::MusicCache,
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
    /// The corner radius (engine px) of the item being drawn: its own, else the theme's; None = the card's built-in radius.
    item_radius: Option<f32>,
    /// Calibration overlay (screen guides) drawn over every frame while Some; `cal_gen` changes with it.
    calib: Option<Guides>,
    cal_gen: u64,
    /// FPS counter drawn in a corner while Some((measured fps x10, target fps)); `fps_gen` changes with the text.
    fps_overlay: Option<(u32, u32)>,
    fps_gen: u64,
    /// Animation ticks since the last frame (for eased values), eased sensor fractions, visualizer history, slideshow state.
    steps_now: i32,
    sens_smooth: HashMap<String, f32>,
    viz_hist: HashMap<String, std::collections::VecDeque<Vec<f32>>>,
    viz_last: HashMap<String, u64>,
    slides: HashMap<String, extras::Slides>,
    /// Screen fit (screen px hidden at each edge). Not zero: the design canvas is scaled into the visible rectangle.
    fit: crate::core::fit::Fit,
    /// The fit the stored base picture was made with (only meaningful while the background is the base).
    bg_fit: crate::core::fit::Fit,
    /// Scratch pictures for the fit (made on first use): the fitted frame and the fitted widget layer.
    fit_pm: Option<Pixmap>,
    fit_layer: Option<Pixmap>,
    /// The widget boxes on the fitted frame (merged like `rects`).
    fit_rects: Vec<(usize, usize, usize, usize)>,
    /// Accumulated microseconds per stage (background, widgets, composite + mask), for profiling.
    pub timing_us: [u64; 3],
    /// Accumulated microseconds per widget id (kind, us), for profiling.
    pub widget_us: HashMap<String, (String, u64)>,
    /// None = every cluster must redraw (set by anything that changes the widgets or what they show).
    layer_key: Option<()>,
    /// Groups of overlapping widgets, each redrawn on its own schedule.
    clusters: Vec<Cluster>,
    /// 4.0: the live effect that replaces the picture as the background (see `effects`), its frame counter and an optional fixed
    /// loop phase (previews and recordings), and a flag to blacken the strips outside the visible rectangle first.
    effect: Option<super::effects::EffectBg>,
    effect_frame: u64,
    effect_phase: Option<f64>,
    effect_clear: bool,
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
            last_key: None,
            mask: Mask::none(),
            music: None,
            music_alpha: 0.0,
            music_tick: 0,
            mus: Default::default(),
            cover: None,
            theme: ThemeLive::new(Theme::standard_preset(), PathBuf::new()),
            images: HashMap::new(),
            rects: Vec::new(),
            live_alpha: 1.0,
            hand: None,
            first: Arc::new(Mutex::new(None)),
            bg_is_base: false,
            sample: false,
            item_radius: None,
            calib: None,
            cal_gen: 0,
            fps_overlay: None,
            fps_gen: 0,
            steps_now: 0,
            sens_smooth: HashMap::new(),
            viz_hist: HashMap::new(),
            viz_last: HashMap::new(),
            slides: HashMap::new(),
            fit: crate::core::fit::Fit::default(),
            bg_fit: crate::core::fit::Fit::default(),
            fit_pm: None,
            fit_layer: None,
            fit_rects: Vec::new(),
            timing_us: [0; 3],
            widget_us: HashMap::new(),
            layer_key: None,
            clusters: Vec::new(),
            effect: None,
            effect_frame: 0,
            effect_phase: None,
            effect_clear: true,
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

    /// Show the live frame rate on the picture: Some((measured fps x10, target fps)) or None (off).
    pub fn set_fps_overlay(&mut self, v: Option<(u32, u32)>) {
        if self.fps_overlay != v {
            self.fps_overlay = v;
            self.fps_gen += 1;
        }
    }

    /// Draw the calibration guides over every frame (None: normal frames).
    pub fn set_calibration(&mut self, g: Option<Guides>) {
        if self.calib != g {
            self.calib = g;
            self.cal_gen += 1;
            self.last_key = None;
            self.layer_key = None;
        }
    }

    /// Where now-playing comes from (None: no music card).
    pub fn set_music_source(&mut self, m: Option<Arc<Mutex<Option<NowPlaying>>>>) {
        self.music = m;
        self.last_key = None;
        self.layer_key = None;
    }

    /// Replace the background clip (None: the animated gradient).
    pub fn set_video(&mut self, v: Option<VideoBg>) {
        self.video = v;
        self.last_key = None;
        self.layer_key = None;
    }

    /// Replace the background with a fixed picture (landscape LW x LH RGBA): used by the preview.
    pub fn set_still_background(&mut self, rgba: Option<Arc<Vec<u8>>>) {
        self.still = rgba;
        self.last_key = None;
        self.layer_key = None;
    }

    /// Swap the theme (widgets); takes effect on the next frame.
    pub fn set_theme(&mut self, t: ThemeLive) {
        self.theme = t;
        self.theme.theme.clamp();
        self.sync_effect();
        // Pictures stay loaded across theme edits unless the file changed.
        self.images.retain(|p, e| std::fs::metadata(p).and_then(|m| m.modified()).ok() == e.mtime);
        self.update_bbox();
        self.last_key = None;
        self.layer_key = None;
    }

    /// Start, change or stop the background effect to match the theme (an unchanged effect keeps running).
    fn sync_effect(&mut self) {
        let want = self.theme.theme.effect();
        if want.is_none() && self.effect.is_some() {
            *self.first.lock().unwrap() = None; // no longer an effect theme: drop its hand-over frame
        }
        if want != self.effect.as_ref().map(|e| e.params.clone()) {
            self.effect = want.map(super::effects::EffectBg::new);
            self.effect_clear = true;
        }
        self.fill_effect_first();
    }

    /// The hand-over target of an effect theme whose Embedded loop is a recording of it: the loop starts at phase 0, so the first
    /// frame is the effect at phase 0 (drawn here, no decoder). Cleared when the theme has no such loop.
    fn fill_effect_first(&mut self) {
        let mut slot = self.first.lock().unwrap();
        match (&self.effect, self.theme.effect_stored) {
            (Some(e), true) => {
                let (cw, ch) = (self.cw, self.ch);
                let mut buf = vec![0u8; cw * ch * 4];
                for p in buf.chunks_exact_mut(4) {
                    p.copy_from_slice(&[0, 0, 0, 255]);
                }
                super::effects::EffectBg::new(e.params.clone()).draw_phase(0.0, &mut buf, cw, ch, self.fit.placed(cw, ch));
                *slot = Some(Arc::new(buf));
            }
            _ => {
                if self.effect.is_some() {
                    *slot = None; // an effect without a matching recording has nothing to hand over to
                }
            }
        }
    }

    /// Draw the effect at this loop phase every frame (previews, recordings); None = real time.
    pub fn set_effect_phase(&mut self, phi: Option<f64>) {
        self.effect_phase = phi;
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
            self.layer_key = None;
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
            self.layer_key = None;
        }
    }

    /// Force the next render to redraw (for benchmarks).
    pub fn invalidate(&mut self) {
        self.last_key = None;
        self.layer_key = None;
    }

    pub fn has_fonts(&self) -> bool {
        self.fonts.regular.is_some()
    }

    /// A real background picture is available (a decoded video frame or the preview still).
    pub fn has_background_frame(&self) -> bool {
        self.still.is_some() || self.effect.is_some() || self.video.as_ref().is_some_and(|v| v.frame().is_some())
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
        self.rects = merge_rects(rects);
        // one cluster per merged rectangle: the widgets inside it and the fastest refresh any of them needs
        let mut clusters: Vec<Cluster> = self.rects.iter().map(|&rect| Cluster { rect, fit: None, widgets: Vec::new(), hz: 10, key: None, last_tick: 0 }).collect();
        for (i, w) in self.theme.theme.widgets.iter().enumerate() {
            let r = widget_rect(w, self.cw, self.ch);
            let (cx, cy) = ((r.x + r.w / 2.0) as usize, (r.y + r.h / 2.0) as usize);
            if let Some(c) = clusters.iter_mut().find(|c| cx >= c.rect.0 && cx < c.rect.2 && cy >= c.rect.1 && cy < c.rect.3) {
                c.widgets.push(i);
                if self.theme.theme.is_visible(w) {
                    c.hz = c.hz.max(match w.kind.as_str() {
                        "visualizer" => 20,
                        "slideshow" => 15,
                        "music" => music::hz(&w.music()),
                        "clock" if w.clock().style == "analog" && w.clock().seconds => 15,
                        _ => 10,
                    });
                }
            }
        }
        self.clusters = clusters;
        self.mus.fast = self.theme.theme.widgets.iter().filter(|w| w.kind == "music" && self.theme.theme.is_visible(w)).fold(0, |a, w| a | music::fast_bits(&w.music()));
        self.update_fit_rects();
    }

    /// The screen fit (None hidden = the design canvas as it is).
    pub fn set_fit(&mut self, f: crate::core::fit::Fit) {
        if self.fit != f {
            self.fit = f.clamped();
            self.effect_clear = true;
            self.fill_effect_first();
            self.update_fit_rects();
            self.last_key = None;
            self.layer_key = None;
        }
    }

    /// The fit the stored base was made with. When it differs from the current fit, the base picture is scaled here.
    pub fn set_bg_fit(&mut self, f: crate::core::fit::Fit) {
        if self.bg_fit != f {
            self.bg_fit = f;
            self.last_key = None;
            self.layer_key = None;
        }
    }

    /// The visible rectangle on this canvas.
    fn visible(&self) -> (i64, i64, i64, i64) {
        self.fit.placed(self.cw, self.ch)
    }

    /// Where the widget boxes land on the fitted frame (a pixel wider on each side for the interpolation).
    fn update_fit_rects(&mut self) {
        if self.fit.is_zero() {
            self.fit_rects.clear();
            for c in self.clusters.iter_mut() {
                c.fit = None;
            }
            return;
        }
        let (vx0, vy0, vx1, vy1) = self.visible();
        let (sx, sy) = ((vx1 - vx0) as f32 / self.cw as f32, (vy1 - vy0) as f32 / self.ch as f32);
        let (cw, ch) = (self.cw as i64, self.ch as i64);
        let (lo_x, lo_y, hi_x, hi_y) = (vx0.max(0), vy0.max(0), vx1.min(cw), vy1.min(ch));
        let map = |&(x0, y0, x1, y1): &(usize, usize, usize, usize)| {
            let r = (
                ((vx0 as f32 + x0 as f32 * sx).floor() as i64 - 1).max(lo_x),
                ((vy0 as f32 + y0 as f32 * sy).floor() as i64 - 1).max(lo_y),
                ((vx0 as f32 + x1 as f32 * sx).ceil() as i64 + 1).min(hi_x),
                ((vy0 as f32 + y1 as f32 * sy).ceil() as i64 + 1).min(hi_y),
            );
            (r.2 > r.0 && r.3 > r.1).then_some((r.0 as usize, r.1 as usize, r.2 as usize, r.3 as usize))
        };
        let rects = self.rects.iter().filter_map(map).collect();
        self.fit_rects = merge_rects(rects);
        for c in self.clusters.iter_mut() {
            c.fit = map(&c.rect);
        }
    }
}

impl Renderer {
    /// Draw one frame; returns premultiplied RGBA (everything is opaque, so it equals plain RGBA), LW x LH.
    /// Returns the frame and whether it differs from the previous call (when not, nothing was redrawn).
    pub fn render(&mut self) -> (&[u8], bool) {
        let t = self.t0.elapsed().as_secs_f32();
        self.tick = (t * 10.0) as u64;
        self.mus.set_clock(t, self.sample);
        let now = local_time();
        let vframe = if self.still.is_some() { self.still.clone() } else { self.video.as_ref().and_then(|v| v.frame()) };
        let mut vkey = vframe.as_ref().map(|a| Arc::as_ptr(a) as usize).unwrap_or(0);
        if self.effect.is_some() {
            // an effect changes every frame (a fixed phase draws once)
            self.effect_frame += 1;
            vkey = if self.effect_phase.is_some() { 1 } else { self.effect_frame as usize };
        }
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
            Some(n) => (n.track_id << 24) ^ ((n.playing as u64) << 20) ^ (((n.position() * 3.0) as u64) << 4) ^ (self.music_alpha * 5.0) as u64 ^ self.mus.motion_key(n.playing),
            None => (self.music_alpha * 5.0) as u64,
        };
        // Fade state (live-layer opacity and hand-over blend in 1/64 steps) is part of the key so fades redraw every frame.
        let hand_key = self.hand.map(|(k, p)| ((p * 64.0) as u64 + 1) | ((k == HandTarget::Black) as u64) << 8).unwrap_or(0);
        let fade_key = (((self.live_alpha * 64.0) as u64) << 12 | hand_key) ^ (self.cal_gen << 40) ^ (self.fps_gen << 52);
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
        // The stored base is already fitted the way it was made; anything else (or a base made with another fit) is scaled here.
        if self.effect.is_none() && self.bg_src_fit() != self.fit {
            self.fit_background();
        }
        self.apply_handover();
        let t1 = Instant::now();
        if self.live_alpha > 0.004 && !self.rects.is_empty() {
            // Each cluster of widgets is redrawn only when its own time slot (or the clock second / music state) changed; in
            // between, the finished layer is just composited again (a fade or a new background frame needs no widget redraw).
            if self.layer_key.is_none() {
                for c in self.clusters.iter_mut() {
                    c.key = None;
                }
                self.layer_key = Some(());
            }
            let np_ref = if show_music { np.as_ref() } else { None };
            for ci in 0..self.clusters.len() {
                let lk = ((t * self.clusters[ci].hz as f32) as u64, now.second, music_key);
                if self.clusters[ci].key != Some(lk) {
                    self.draw_cluster(ci, &now, &s, np_ref);
                    self.clusters[ci].key = Some(lk);
                }
            }
            if self.fit.is_zero() {
                self.composite(self.live_alpha);
            } else {
                self.composite_fitted(self.live_alpha);
            }
        }
        let t2 = Instant::now();
        if !self.mask.is_none() {
            let fit = self.fit;
            self.mask.apply_fit(self.pm.data_mut(), &fit);
        }
        self.draw_calibration();
        self.draw_fps_overlay();
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
        let cw = self.cw;
        composite_rects(self.pm.data_mut(), self.layer.data(), cw, &self.rects, alpha);
    }

    /// Scale the opaque frame in `pm` into the visible rectangle (hidden strips black).
    /// The fit the background picture has now.
    fn bg_src_fit(&self) -> crate::core::fit::Fit {
        if self.bg_is_base { self.bg_fit } else { crate::core::fit::Fit::default() }
    }

    fn fit_background(&mut self) {
        let (cw, ch) = (self.cw, self.ch);
        let vis = self.visible();
        let src = self.bg_src_fit().placed(cw, ch);
        let mut out = self.fit_pm.take().unwrap_or_else(|| Pixmap::new(cw as u32, ch as u32).unwrap());
        out.data_mut().fill(0);
        for p in out.data_mut().chunks_exact_mut(4) {
            p[3] = 255;
        }
        let region = (vis.0.max(0) as usize, vis.1.max(0) as usize, vis.2.clamp(0, cw as i64) as usize, vis.3.clamp(0, ch as i64) as usize);
        resample_into(self.pm.data(), out.data_mut(), cw, ch, src, vis, region);
        std::mem::swap(&mut self.pm, &mut out);
        self.fit_pm = Some(out);
    }


    /// `composite` for the fitted layer and boxes.
    fn composite_fitted(&mut self, alpha: f32) {
        let cw = self.cw;
        if let Some(src) = &self.fit_layer {
            composite_rects(self.pm.data_mut(), src.data(), cw, &self.fit_rects, alpha);
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

    /// The corner radius of a card of box `b`: the item's own or the theme's, else `dflt` (the card's built-in radius); at most
    /// half the short side.
    pub(super) fn card_radius(&self, b: Rect, dflt: f32) -> f32 {
        self.item_radius.unwrap_or(dflt).clamp(0.0, b.w.min(b.h) / 2.0)
    }

    /// Round the corners of what is drawn inside `b` (a picture): pixels outside the rounded rectangle fade to transparent.
    fn clip_rounded(&mut self, b: Rect, r: f32) {
        let r = r.clamp(0.0, b.w.min(b.h) / 2.0);
        if r < 0.5 {
            return;
        }
        let (cw, ch) = (self.cw as i32, self.ch as i32);
        let (x0, y0, x1, y1) = (b.x as i32, b.y as i32, (b.x + b.w) as i32, (b.y + b.h) as i32);
        let ri = r.ceil() as i32;
        let d = self.layer.data_mut();
        // (arc centre x, y, left?, top?, corner square origin)
        let corners = [(x0 as f32 + r, y0 as f32 + r, true, true, x0, y0), (x1 as f32 - r, y0 as f32 + r, false, true, x1 - ri, y0), (x0 as f32 + r, y1 as f32 - r, true, false, x0, y1 - ri), (x1 as f32 - r, y1 as f32 - r, false, false, x1 - ri, y1 - ri)];
        for (cx, cy, left, top, sx, sy) in corners {
            for y in sy.max(0)..(sy + ri).min(ch) {
                for x in sx.max(0)..(sx + ri).min(cw) {
                    let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                    // only the quarter next to the outer corner lies outside the arc
                    if (if left { px >= cx } else { px <= cx }) || (if top { py >= cy } else { py <= cy }) {
                        continue;
                    }
                    let (dx, dy) = (px - cx, py - cy);
                    let cov = (r - (dx * dx + dy * dy).sqrt() + 0.5).clamp(0.0, 1.0);
                    if cov < 1.0 {
                        let i = (y as usize * self.cw + x as usize) * 4;
                        for c in 0..4 {
                            d[i + c] = (d[i + c] as f32 * cov) as u8;
                        }
                    }
                }
            }
        }
    }

    /// Draw one widget into the layer (and note how long it took).
    fn draw_one(&mut self, w: &Widget, now: &LocalTime, s: &Sensors, np: Option<&NowPlaying>) {
        let r = widget_rect(w, self.cw, self.ch);
        let tw0 = Instant::now();
        // 4.0 E2: the item's own radius (canvas px) wins; else the theme's corner radius when it has one
        self.item_radius = w.radius.map(|px| px * self.cw as f32 / 2400.0).or_else(|| self.mask.corner_radius().map(|c| c * self.cw as f32 / LW as f32));
        match w.kind.as_str() {
            "clock" => self.draw_clock(r, &w.clock(), now),
            "music" => self.draw_music(r, &w.music(), np),
            "stats" => self.draw_stats(r, &w.stats(), s),
            "text" => self.draw_text(r, &w.text()),
            "image" => self.draw_image(r, w),
            "weather" => self.draw_weather_widget(r, &w.weather()),
            "sensor" => self.draw_sensor(r, &w.id, &w.sensor()),
            "date" => self.draw_date(r, &w.date(), now),
            "visualizer" => self.draw_visualizer(r, &w.id, &w.visualizer()),
            "slideshow" => self.draw_slideshow(r, &w.id, &w.slideshow(), w),
            "info" => self.draw_info(r, &w.info()),
            _ => self.draw_more(r, w, now), // the 4.0 widget types (render/more.rs)
        }
        if let (Some(px), "image" | "slideshow") = (w.radius, w.kind.as_str()) {
            self.clip_rounded(r, px * self.cw as f32 / 2400.0);
        }
        self.item_radius = None;
        self.widget_us.entry(w.id.clone()).or_insert_with(|| (w.kind.clone(), 0)).1 += tw0.elapsed().as_micros() as u64;
    }

    /// Redraw one cluster: clear its rectangle of the layer and draw its widgets in order (live: `onBase` items are skipped while
    /// the background is the stored base).
    fn draw_cluster(&mut self, ci: usize, now: &LocalTime, s: &Sensors, np: Option<&NowPlaying>) {
        let (x0, y0, x1, y1) = self.clusters[ci].rect;
        let cw = self.cw;
        for y in y0..y1 {
            self.layer.data_mut()[(y * cw + x0) * 4..(y * cw + x1) * 4].fill(0);
        }
        // ticks since this cluster was last drawn (eases bars and sensor fills)
        let steps = if self.sample { 40 } else { (self.tick - self.clusters[ci].last_tick.min(self.tick)).min(20) as i32 };
        self.clusters[ci].last_tick = self.tick;
        self.steps_now = steps;
        self.step_bars(s, steps);
        let idx = self.clusters[ci].widgets.clone();
        for i in idx {
            let w = self.theme.theme.widgets[i].clone();
            if !self.theme.theme.is_visible(&w) || (w.on_base && w.can_be_on_base() && self.bg_is_base) {
                continue;
            }
            self.draw_one(&w, now, s, np);
        }
        if !self.fit.is_zero() {
            if let Some(reg) = self.clusters[ci].fit {
                let vis = self.visible();
                let mut out = self.fit_layer.take().unwrap_or_else(|| Pixmap::new(cw as u32, self.ch as u32).unwrap());
                for y in reg.1..reg.3 {
                    out.data_mut()[(y * cw + reg.0) * 4..(y * cw + reg.2) * 4].fill(0);
                }
                resample_into(self.layer.data(), out.data_mut(), cw, self.ch, (0, 0, cw as i64, self.ch as i64), vis, reg);
                self.fit_layer = Some(out);
            }
        }
    }

    /// Draw the `onBase` widgets (the base overlay) into the layer.
    fn draw_widgets(&mut self, now: &LocalTime, s: &Sensors, np: Option<&NowPlaying>, only_base: bool) {
        self.clear_layer();
        self.steps_now = 40;
        self.step_bars(s, 40);
        let widgets = self.theme.theme.widgets.clone();
        for w in &widgets {
            if !self.theme.theme.is_visible(w) {
                continue;
            }
            let on_base = w.on_base && w.can_be_on_base();
            if only_base && !on_base {
                continue;
            }
            self.draw_one(w, now, s, np);
        }
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

    /// The effect fills the visible rectangle directly (no picture to scale afterwards); the strips outside stay black.
    fn draw_effect(&mut self, t: f32) {
        let (cw, ch) = (self.cw, self.ch);
        if self.effect_clear {
            self.effect_clear = false;
            for p in self.pm.data_mut().chunks_exact_mut(4) {
                p.copy_from_slice(&[0, 0, 0, 255]);
            }
        }
        let rect = self.visible();
        let phase = self.effect_phase;
        let sample = self.sample;
        if let Some(e) = self.effect.as_mut() {
            if e.wants_audio() && phase.is_none() && !sample {
                // the sound-reactive effect listens (this starts the audio capture; it ends by itself when nobody asks)
                let levels = super::audio::audio().levels();
                e.feed_audio(&levels);
            }
            match phase {
                Some(phi) => e.draw_phase(phi, self.pm.data_mut(), cw, ch, rect),
                None => e.draw_at(t as f64, self.pm.data_mut(), cw, ch, rect),
            }
        }
    }

    fn background(&mut self, t: f32, frame: Option<Arc<Vec<u8>>>) {
        if self.effect.is_some() {
            return self.draw_effect(t);
        }
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

/// Source-over of a premultiplied layer onto an opaque frame, inside the given boxes, scaled by `alpha`.
fn composite_rects(dst: &mut [u8], src: &[u8], cw: usize, rects: &[(usize, usize, usize, usize)], alpha: f32) {
    let g = (alpha.clamp(0.0, 1.0) * 256.0) as u32;
    for &(x0, y0, x1, y1) in rects {
        let w = x1 - x0;
        for y in y0..y1 {
            let s = &src[(y * cw + x0) * 4..][..w * 4];
            let d = &mut dst[(y * cw + x0) * 4..][..w * 4];
            for (dp, sp) in d.chunks_exact_mut(4).zip(s.chunks_exact(4)) {
                if sp[3] == 0 {
                    continue;
                }
                if g >= 256 && sp[3] == 255 {
                    dp[..3].copy_from_slice(&sp[..3]);
                    continue;
                }
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

/// Merge overlapping boxes: the layer is composited once per region (twice would apply it twice).
fn merge_rects(mut rects: Vec<(usize, usize, usize, usize)>) -> Vec<(usize, usize, usize, usize)> {
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
            return rects;
        }
    }
}

/// Bilinear resample of the `content` rectangle of a `cw` x `ch` picture so that it fills the rectangle `vis` of `dst` (same size); only the pixels
/// of `region` (inside `vis`) are written. Pixel centres map to pixel centres, the source edges are clamped.
fn resample_into(src: &[u8], dst: &mut [u8], cw: usize, ch: usize, content: (i64, i64, i64, i64), vis: (i64, i64, i64, i64), region: (usize, usize, usize, usize)) {
    // `content` and `vis` are signed: a shifted picture may reach beyond the canvas (that part is simply not written / not read)
    let (vx0, vy0, vx1, vy1) = vis;
    let (sx0, sy0, sx1, sy1) = content;
    let (kx, ky) = ((sx1 - sx0) as f32 / (vx1 - vx0) as f32, (sy1 - sy0) as f32 / (vy1 - vy0) as f32);
    let (rx0, ry0) = ((region.0 as i64).max(vx0).max(0) as usize, (region.1 as i64).max(vy0).max(0) as usize);
    let (rx1, ry1) = ((region.2 as i64).min(vx1).min(cw as i64) as usize, (region.3 as i64).min(vy1).min(ch as i64) as usize);
    if rx1 <= rx0 || ry1 <= ry0 {
        return;
    }
    // the source pixels that hold picture content
    let (lo_x, hi_x) = (sx0.max(0) as f32, (sx1.min(cw as i64) - 1) as f32);
    let (lo_y, hi_y) = (sy0.max(0) as f32, (sy1.min(ch as i64) - 1) as f32);
    // per-column source index and weight (computed once)
    let cols: Vec<(usize, usize, u32)> = (rx0..rx1)
        .map(|x| {
            let u = ((x as f32 + 0.5 - vx0 as f32) * kx - 0.5 + sx0 as f32).clamp(lo_x, hi_x);
            let i = u.floor() as usize;
            (i, (i + 1).min(hi_x as usize), ((u - i as f32) * 256.0) as u32)
        })
        .collect();
    for y in ry0..ry1 {
        let v = ((y as f32 + 0.5 - vy0 as f32) * ky - 0.5 + sy0 as f32).clamp(lo_y, hi_y);
        let j = v.floor() as usize;
        let (j1, wy) = ((j + 1).min(hi_y as usize), ((v - j as f32) * 256.0) as u32);
        let (r0, r1) = (&src[j * cw * 4..(j + 1) * cw * 4], &src[j1 * cw * 4..(j1 + 1) * cw * 4]);
        let row = &mut dst[(y * cw + rx0) * 4..(y * cw + rx1) * 4];
        for (o, &(i0, i1, wx)) in row.chunks_exact_mut(4).zip(&cols) {
            for c in 0..4 {
                let a = r0[i0 * 4 + c] as u32 * (256 - wx) + r0[i1 * 4 + c] as u32 * wx;
                let b = r1[i0 * 4 + c] as u32 * (256 - wx) + r1[i1 * 4 + c] as u32 * wx;
                o[c] = ((a * (256 - wy) + b * wy + 32768) >> 16) as u8;
            }
        }
    }
}

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
        let g = Guides::from_value(&serde_json::json!({ "left": { "size": 100 }, "right": { "size": 180 }, "curve": { "offset": 0, "left": 60, "right": 140 } }));
        r.set_calibration(Some(g));
        let (f, changed) = r.render();
        assert!(changed, "a change redraws at once");
        // the curve band edge line is yellow at canvas x 1200+140 = 1340 (engine x 938); the right band tint changes the frame
        let i = (300 * LW + 938) * 4;
        assert_eq!(&f[i..i + 3], &[255, 235, 0]);
        let j = (300 * LW + LW - 20) * 4;
        assert_ne!(&f[j..j + 3], &plain[j..j + 3]);
        r.set_calibration(None);
        let back = r.render().0.to_vec();
        assert_eq!(back.len(), plain.len());
        assert!(back.chunks_exact(4).zip(plain.chunks_exact(4)).all(|(a, b)| a[..3].iter().zip(&b[..3]).all(|(x, y)| x.abs_diff(*y) < 8)), "normal frame again");
    }

    #[test]
    fn the_fps_overlay_draws_a_chip_only_while_set_and_leaves_the_rest_alone() {
        let mut r = Renderer::new(Arc::new(Mutex::new(Sensors::default())), None);
        r.set_theme(ThemeLive::new(Theme::default(), PathBuf::new()));
        let plain = r.render().0.to_vec();
        r.set_fps_overlay(Some((598, 60)));
        let (f, changed) = r.render();
        assert!(changed, "turning it on redraws at once");
        let diff: Vec<usize> = (0..plain.len() / 4).filter(|i| f[i * 4..i * 4 + 3] != plain[i * 4..i * 4 + 3]).collect();
        assert!(diff.len() > 500, "a visible chip: {}", diff.len());
        // only the top-left chip changes (inside 600x120 px from 10% / 6% of the canvas), the rest of the picture is untouched
        assert!(diff.iter().all(|i| (i % LW) >= LW / 10 && (i % LW) < LW / 10 + 600 && (i / LW) < LH * 6 / 100 + 120), "chip stays small");
        // a new number redraws; the same number does not
        r.set_fps_overlay(Some((600, 60)));
        assert!(r.render().1);
        r.set_fps_overlay(Some((600, 60)));
        assert!(!r.render().1, "unchanged: no redraw");
        r.set_fps_overlay(None);
        let back = r.render().0.to_vec();
        assert!(back.chunks_exact(4).zip(plain.chunks_exact(4)).all(|(a, b)| a[..3].iter().zip(&b[..3]).all(|(x, y)| x.abs_diff(*y) < 8)), "normal frame again");
    }

    fn px(f: &[u8], w: usize, x: usize, y: usize) -> [u8; 3] {
        let i = (y * w + x) * 4;
        [f[i], f[i + 1], f[i + 2]]
    }

    /// A renderer on a `w` x `h` canvas with a plain white picture as background and no widgets.
    fn white_renderer(w: usize, h: usize) -> Renderer {
        let mut r = Renderer::new_sized(w, h, Arc::new(Mutex::new(Sensors::default())), None);
        r.set_theme(ThemeLive::new(Theme::default(), PathBuf::new()));
        r.set_still_background(Some(Arc::new(vec![255u8; w * h * 4])));
        r
    }

    /// The screen fit puts the design canvas exactly into the visible rectangle: 4 px hidden at the top and 9 at the left, in
    /// screen px, are 3 and 6 on the 1680x756 live picture and 4 and 9 on the 2400x1080 base picture.
    #[test]
    fn fit_content_rectangle_is_exact_at_both_sizes() {
        for (w, h, x0, y0) in [(1680usize, 756usize, 6usize, 3usize), (2400, 1080, 9, 4)] {
            let mut r = white_renderer(w, h);
            r.set_fit(crate::core::fit::Fit::new(4, 0, 0, 9));
            let f = r.render().0.to_vec();
            let (black, white) = ([0u8; 3], [255u8; 3]);
            assert_eq!(px(&f, w, x0 - 1, h / 2), black, "{w}: last hidden column");
            assert_eq!(px(&f, w, x0, h / 2), white, "{w}: first visible column");
            assert_eq!(px(&f, w, w / 2, y0 - 1), black, "{w}: last hidden row");
            assert_eq!(px(&f, w, w / 2, y0), white, "{w}: first visible row");
            assert_eq!(px(&f, w, w - 1, h - 1), white, "{w}: right and bottom are not hidden");
            assert_eq!(px(&f, w, x0, y0), white);
            assert_eq!(px(&f, w, x0 - 1, y0 - 1), black);
            let hidden = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|&(x, y)| x < x0 || y < y0).all(|(x, y)| px(&f, w, x, y) == black);
            assert!(hidden, "{w}: every hidden pixel is black");
            let seen = (y0..h).flat_map(|y| (x0..w).map(move |x| (x, y))).all(|(x, y)| px(&f, w, x, y) == white);
            assert!(seen, "{w}: every visible pixel is white");
            // no fit again: the whole canvas
            r.set_fit(crate::core::fit::Fit::default());
            let g = r.render().0.to_vec();
            assert_eq!(px(&g, w, 0, 0), white);
        }
    }

    #[test]
    fn fit_scales_the_picture_instead_of_cutting_it() {
        let (w, h) = (1680usize, 756usize);
        let mut ramp = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let v = (x * 255 / (w - 1)) as u8;
                ramp[(y * w + x) * 4..][..3].copy_from_slice(&[v, v, v]);
            }
        }
        let mut r = white_renderer(w, h);
        r.set_still_background(Some(Arc::new(ramp)));
        r.set_fit(crate::core::fit::Fit::new(0, 20, 0, 40)); // left 28, right 14 sent px
        let f = r.render().0.to_vec();
        assert!(px(&f, w, 28, 300)[0] <= 2, "the left end of the picture is at the first visible column: {:?}", px(&f, w, 28, 300));
        assert!(px(&f, w, w - 15, 300)[0] >= 253, "the right end is at the last visible column: {:?}", px(&f, w, w - 15, 300));
        assert_eq!(px(&f, w, w - 14, 300), [0, 0, 0]);
        assert_eq!(px(&f, w, 27, 300), [0, 0, 0]);
    }

    #[test]
    fn fit_leaves_the_notch_band_and_the_corners_on_the_panel() {
        use serde_json::json;
        let mask = Mask::from_value(&json!({ "mode": "band", "band": { "edge": "left", "px": 92 }, "corners": { "on": true, "r": 80 } }));
        let mut a = white_renderer(LW, LH);
        a.set_mask(mask.clone());
        let plain = a.render().0.to_vec();
        let mut b = white_renderer(LW, LH);
        b.set_mask(mask);
        b.set_fit(crate::core::fit::Fit::new(4, 0, 0, 9));
        let fitted = b.render().0.to_vec();
        // the band (67 px on the live picture) is black in both, and starts at the same column
        for x in [0usize, 30, 66] {
            assert_eq!(px(&plain, LW, x, 400), [0, 0, 0]);
            assert_eq!(px(&fitted, LW, x, 400), [0, 0, 0]);
        }
        assert_eq!(px(&plain, LW, 67, 400), [255, 255, 255]);
        assert_eq!(px(&fitted, LW, 67, 400), [255, 255, 255], "the band is not moved or widened by the fit");
        // the rounded corners sit at the panel corners of the visible area (right side shows it)
        assert_eq!(px(&plain, LW, LW - 1, LH - 1), [0, 0, 0]);
        assert_eq!(px(&fitted, LW, LW - 1, LH - 1), [0, 0, 0]);
        assert_eq!(px(&fitted, LW, LW - 1, 300), [255, 255, 255]);
    }

    #[test]
    fn fit_keeps_widgets_inside_the_visible_rectangle() {
        let mut r = Renderer::new(Arc::new(Mutex::new(Sensors::default())), None);
        r.set_theme(ThemeLive::new(Theme::standard_preset(), PathBuf::new()));
        r.set_sample_mode(true);
        r.set_still_background(Some(Arc::new(vec![40u8; LW * LH * 4])));
        r.set_fit(crate::core::fit::Fit::new(40, 30, 20, 60));
        let f = r.render().0.to_vec();
        let (x0, y0, x1, y1) = crate::core::fit::Fit::new(40, 30, 20, 60).rect(LW, LH);
        let outside_is_black = (0..LH).flat_map(|y| (0..LW).map(move |x| (x, y))).filter(|&(x, y)| x < x0 || x >= x1 || y < y0 || y >= y1).all(|(x, y)| px(&f, LW, x, y) == [0, 0, 0]);
        assert!(outside_is_black);
        assert_ne!(px(&f, LW, x0 + 5, y0 + 5), [0, 0, 0], "the picture is inside");
        let widgets_drawn = f.chunks_exact(4).any(|p| p[0] > 120 || p[1] > 120);
        assert!(widgets_drawn, "the clock and the stats still draw (bright text on the dark picture)");
    }

    /// A background that is not the stored base (the original clip, or the gradient when the base could not be decoded) always gets
    /// the screen fit from the engine: black strips on the hidden edges, the picture inside the visible rectangle.
    #[test]
    fn a_fallback_background_gets_the_fit() {
        use crate::core::fit::Fit;
        let f = Fit::new(4, 0, 0, 9);
        let mut r = white_renderer(LW, LH);
        r.set_bg_is_base(false); // what the pipeline does when the base does not decode
        r.set_bg_fit(f); // a stale base fit must not matter
        r.set_fit(f);
        let out = r.render().0.to_vec();
        assert_eq!((px(&out, LW, 100, 2), px(&out, LW, 100, 3), px(&out, LW, 5, 400), px(&out, LW, 6, 400)), ([0; 3], [255; 3], [0; 3], [255; 3]));
    }

    /// With the fit never given to the engine (the old window-less start) the fitted base is stretched over the panel: the top strip is
    /// gone. Given the saved fit, the base made with it is shown as it is (rows 0-3 stay black).
    #[test]
    fn a_base_made_with_the_saved_fit_keeps_its_strips() {
        use crate::core::fit::Fit;
        let f = Fit::new(4, 0, 0, 9);
        let mut pic = vec![255u8; LW * LH * 4];
        let (x0, y0, ..) = f.rect(LW, LH);
        for y in 0..LH {
            for x in 0..LW {
                if x < x0 || y < y0 {
                    pic[(y * LW + x) * 4..][..3].fill(0);
                }
            }
        }
        let mut r = white_renderer(LW, LH);
        r.set_still_background(Some(Arc::new(pic.clone())));
        r.set_bg_is_base(true);
        r.set_bg_fit(f);
        let no_fit = r.render().0.to_vec();
        assert_eq!(px(&no_fit, LW, 100, 0), [255; 3], "the bug: no fit given, the picture is stretched and the strip is gone");
        r.set_fit(f);
        let out = r.render().0.to_vec();
        assert_eq!((px(&out, LW, 100, 0), px(&out, LW, 100, 2), px(&out, LW, 100, 3)), ([0; 3], [0; 3], [255; 3]));
    }

    /// A stored base made before the fit existed is scaled into the visible rectangle by the engine; one made with the current fit is
    /// shown as it is; one made with another fit is re-fitted from its own visible rectangle.
    #[test]
    fn a_base_made_with_another_fit_is_refitted() {
        use crate::core::fit::Fit;
        let f = Fit::new(4, 0, 0, 9);
        let (black, white) = ([0u8; 3], [255u8; 3]);
        let base = |r: &mut Renderer| r.set_bg_is_base(true);
        // made without a fit (white everywhere): the live frame gets the black strips
        let mut r = white_renderer(LW, LH);
        base(&mut r);
        r.set_fit(f);
        let out = r.render().0.to_vec();
        assert_eq!((px(&out, LW, 100, 2), px(&out, LW, 100, 3), px(&out, LW, 5, 400), px(&out, LW, 6, 400)), (black, white, black, white));
        // made with exactly this fit (strips already in the picture): shown as it is
        let mut pic = vec![255u8; LW * LH * 4];
        let (x0, y0, ..) = f.rect(LW, LH);
        for y in 0..LH {
            for x in 0..LW {
                if x < x0 || y < y0 {
                    pic[(y * LW + x) * 4..][..3].fill(0);
                }
            }
        }
        let mut r = white_renderer(LW, LH);
        r.set_still_background(Some(Arc::new(pic.clone())));
        base(&mut r);
        r.set_bg_fit(f);
        r.set_fit(f);
        assert_eq!(r.render().0, &pic[..]);
        // made with the old fit, the fit now removed: the picture is stretched back over the whole panel
        let mut r = white_renderer(LW, LH);
        r.set_still_background(Some(Arc::new(pic)));
        base(&mut r);
        r.set_bg_fit(f);
        let out = r.render().0.to_vec();
        assert_eq!((px(&out, LW, 0, 0), px(&out, LW, 3, 1), px(&out, LW, LW - 1, LH - 1)), (white, white, white));
    }
}

#[cfg(test)]
mod shift_tests {
    use super::*;
    use crate::core::fit::Fit;

    fn px(f: &[u8], w: usize, x: usize, y: usize) -> [u8; 3] {
        let i = (y * w + x) * 4;
        [f[i], f[i + 1], f[i + 2]]
    }

    fn white(w: usize, h: usize) -> Renderer {
        let mut r = Renderer::new_sized(w, h, Arc::new(Mutex::new(Sensors::default())), None);
        r.set_theme(ThemeLive::new(Theme::default(), PathBuf::new()));
        r.set_still_background(Some(Arc::new(vec![255u8; w * h * 4])));
        r
    }

    /// The picture is scaled into the inset rectangle and then moved: black where it no longer covers, cut off past the edge.
    #[test]
    fn the_shift_moves_the_whole_picture_at_both_sizes() {
        for (w, h) in [(1680usize, 756usize), (2400, 1080)] {
            let f = Fit::new(4, 0, 0, 9).with_shift(8, -6);
            let (x0, y0, x1, y1) = f.placed(w, h);
            let mut r = white(w, h);
            r.set_fit(f);
            let out = r.render().0.to_vec();
            let (b, wh) = ([0u8; 3], [255u8; 3]);
            assert!(x0 > 0 && y0 <= 0, "{w}: the shift is small ({x0},{y0})");
            // left strip (inset 9 + shift 8), top starts at row y0 (inset 4 - shift 6 = 0 after clipping)
            assert_eq!(px(&out, w, x0 as usize - 1, h / 2), b, "{w}: last black column");
            assert_eq!(px(&out, w, x0 as usize, h / 2), wh, "{w}: first white column");
            assert_eq!(px(&out, w, w / 2, 0), wh, "{w}: the top row is covered (moved up past the edge)");
            // the bottom now stops short: the last rows are black
            assert_eq!(px(&out, w, w / 2, y1 as usize), b, "{w}: first black row at the bottom");
            assert_eq!(px(&out, w, w / 2, y1 as usize - 1), wh, "{w}: last white row");
            assert_eq!(px(&out, w, w - 1, h / 2), wh, "{w}: shifted right past the edge: the last column is covered");
            assert!(x1 as usize >= w);
            // a shift alone
            let mut r = white(w, h);
            r.set_fit(Fit::default().with_shift(-10, 10));
            let out = r.render().0.to_vec();
            let (dx, dy) = Fit::default().with_shift(-10, 10).shift_px(w, h);
            assert_eq!(px(&out, w, (w as i64 + dx) as usize, h / 2), b, "{w}: black strip on the right where it moved away");
            assert_eq!(px(&out, w, (w as i64 + dx) as usize - 1, h / 2), wh);
            assert_eq!(px(&out, w, w / 2, dy as usize - 1), b, "{w}: black strip on top");
            assert_eq!(px(&out, w, w / 2, dy as usize), wh);
        }
    }

    #[test]
    fn the_notch_band_stays_put_when_the_picture_moves() {
        use serde_json::json;
        let mask = Mask::from_value(&json!({ "mode": "band", "band": { "edge": "left", "px": 92 }, "corners": { "on": false, "r": 0 } }));
        let mut r = white(LW, LH);
        r.set_mask(mask);
        r.set_fit(Fit::default().with_shift(15, 0)); // the picture moves right by 11 px: the strip on its left is black anyway
        let out = r.render().0.to_vec();
        assert_eq!(px(&out, LW, 66, 300), [0, 0, 0]);
        assert_eq!(px(&out, LW, 67, 300), [255, 255, 255], "the band is where the panel is, not where the picture is");
    }

    #[test]
    fn a_base_made_with_a_shift_is_refitted_to_a_new_one() {
        let old = Fit::default().with_shift(10, 0);
        let mut pic = vec![255u8; LW * LH * 4];
        let (dx, _) = old.shift_px(LW, LH);
        for y in 0..LH {
            for x in 0..(dx as usize) {
                pic[(y * LW + x) * 4..][..3].fill(0);
            }
        }
        // the base was made with +7 sent px right; now the fit is none: the picture is moved back to cover the panel again
        let mut r = white(LW, LH);
        r.set_still_background(Some(Arc::new(pic)));
        r.set_bg_is_base(true);
        r.set_bg_fit(old);
        r.set_fit(Fit::default().with_shift(0, 0));
        // zero fit and a base with a shift: render the base as it is, then compare by forcing the refit path with another shift
        r.set_fit(Fit::default().with_shift(0, 5));
        let out = r.render().0.to_vec();
        assert_eq!(px(&out, LW, LW / 2, 0), [0, 0, 0], "moved down: black on top");
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        let p = Pixmap::decode_png(png).unwrap();
        (p.width(), p.height())
    }

    fn w(id: &str, kind: &str, props: serde_json::Value) -> Widget {
        Widget { id: id.into(), kind: kind.into(), x: 0.1, y: 0.1, w: 0.4, h: 0.4, props, ..Widget::default() }
    }

    #[test]
    fn every_widget_type_and_style_renders_in_the_preview() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.gif"), crate::live::gif::tests::tiny()).unwrap();
        let mut widgets = Vec::new();
        for style in ["number", "bar", "ring", "gauge", "graph", "sparkline"] {
            for metric in ["cpu.load", "cpu.core", "cpu.temp", "cpu.clock", "gpu.load", "gpu.temp", "gpu.clock", "gpu.vram", "gpu.power", "gpu.fan", "ram.used", "net.down", "net.up", "disk.used", "disk.activity"] {
                widgets.push(w("s", "sensor", serde_json::json!({ "metric": metric, "style": style })));
            }
        }
        for s in ["digital", "analog", "minimal"] {
            widgets.push(w("c", "clock", serde_json::json!({ "style": s, "seconds": true, "format": "12h" })));
        }
        for s in ["line", "calendar", "big"] {
            widgets.push(w("d", "date", serde_json::json!({ "style": s })));
        }
        for (s, sp) in [("now", "days"), ("forecast", "days"), ("forecast", "hours")] {
            widgets.push(w("w", "weather", serde_json::json!({ "style": s, "span": sp, "unit": "F" })));
        }
        for s in ["bars-h", "ring", "wave-fade"] {
            widgets.push(w("v", "visualizer", serde_json::json!({ "style": s, "bands": 64 })));
        }
        widgets.push(w("p", "slideshow", serde_json::json!({ "files": ["a.gif", "../escape.png", "missing.png"] })));
        for k in ["uptime", "cpuName", "gpuName", "countdown", "topProcesses"] {
            widgets.push(w("i", "info", serde_json::json!({ "kind": k, "target": "2030-01-01T00:00" })));
        }
        let mut t = ThemeLive::new(Theme::default(), dir.path().to_path_buf());
        for (i, mut x) in widgets.into_iter().enumerate() {
            x.id = format!("{}{i}", x.id);
            t.theme.widgets.push(x);
        }
        let png = crate::live::render_preview(&t, 320, 144, true).unwrap();
        assert_eq!(png_size(&png), (320, 144));
        // tiny boxes and an empty slideshow must not panic either
        t.theme.widgets.iter_mut().for_each(|x| {
            x.w = 0.02;
            x.h = 0.02;
        });
        crate::live::render_preview(&t, 320, 144, true).unwrap();
    }

    #[test]
    fn slideshow_shows_the_first_picture_and_gif_colours() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.gif"), crate::live::gif::tests::tiny()).unwrap();
        let mut t = ThemeLive::new(Theme::default(), dir.path().to_path_buf());
        t.theme.widgets.push(Widget { id: "p".into(), kind: "slideshow".into(), x: 0.0, y: 0.0, w: 1.0, h: 1.0, props: serde_json::json!({ "files": ["a.gif"], "fit": "cover" }), ..Widget::default() });
        let png = crate::live::render_preview(&t, 168, 76, true).unwrap();
        let pm = Pixmap::decode_png(&png).unwrap();
        let px = |x: u32, y: u32| pm.pixel(x, y).unwrap();
        // 2x2 GIF frame 1: red, green / blue, white (first frame at t = 0)
        assert!(px(5, 5).red() > 200 && px(5, 5).green() < 60, "top-left is red");
        assert!(px(160, 70).red() > 200 && px(160, 70).green() > 200, "bottom-right is white");
    }

    #[test]
    fn live_sensor_widgets_draw_from_the_shared_sampler() {
        let sensors = Arc::new(Mutex::new(Sensors::default()));
        let mut r = Renderer::new(sensors, None);
        let mut t = ThemeLive::new(Theme::default(), PathBuf::new());
        for (i, m) in ["cpu.load", "ram.used", "cpu.temp"].iter().enumerate() {
            t.theme.widgets.push(Widget { id: format!("s{i}"), kind: "sensor".into(), x: 0.05 + 0.3 * i as f32, y: 0.1, w: 0.25, h: 0.4, props: serde_json::json!({ "metric": m, "style": "ring" }), ..Widget::default() });
        }
        r.set_theme(t);
        let first = r.render().0.to_vec();
        std::thread::sleep(Duration::from_millis(1400));
        r.invalidate();
        let second = r.render().0.to_vec();
        // after the sampler has polled, RAM (always readable) has a value: the picture differs from the "no data yet" one
        assert_ne!(first, second);
    }
}
