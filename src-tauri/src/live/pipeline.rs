//! The companion's engine: three threads joined by "latest frame" slots, so drawing never waits for USB.
//!   render (paced at the fps setting) -> slot -> encode -> slot -> send (owns the screen link)
//! A slot holds only the newest item: when a stage is slower, older frames are dropped instead of queuing up (a queue would
//! show stale clocks). Buffers are recycled through pools so a 60 fps run does not churn the allocator.
//! The sender connects with backoff, reconnects when the screen disappears, and ends the stream properly (loop clip keeps
//! playing) when paused or stopped. While there is no connection nothing is rendered.
use super::encode::{frame_budget, rotate_cw, JpegEncoder};
use super::mask::Mask;
use super::render::{HandTarget, Renderer};
use super::sensors::Sampler;
use super::nowplaying::MusicWatcher;
use super::session::{Ending, LiveSession};
use super::theme::{Guides, HandoverEnd, Theme, ThemeLive};
use std::path::PathBuf;
use super::video::VideoBg;
use super::{Config, PH, PW};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

pub struct Latest<T> {
    slot: Mutex<Option<T>>,
    cv: Condvar,
}

impl<T> Latest<T> {
    fn new() -> Self {
        Latest { slot: Mutex::new(None), cv: Condvar::new() }
    }
    /// Store `t`; returns the stale item it replaced, if any.
    fn put(&self, t: T) -> Option<T> {
        let old = self.slot.lock().unwrap().replace(t);
        self.cv.notify_one();
        old
    }
    fn take(&self, timeout: Duration) -> Option<T> {
        let mut g = self.slot.lock().unwrap();
        if g.is_none() {
            g = self.cv.wait_timeout(g, timeout).unwrap().0;
        }
        g.take()
    }
    fn clear(&self) {
        self.slot.lock().unwrap().take();
    }
}

struct Pool<T>(Mutex<Vec<T>>);
impl<T: Default> Pool<T> {
    fn get(&self) -> T {
        self.0.lock().unwrap().pop().unwrap_or_default()
    }
    fn put(&self, t: T) {
        let mut g = self.0.lock().unwrap();
        if g.len() < 3 {
            g.push(t);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sink {
    /// The real screen over WinUSB.
    Device,
    /// Throw frames away (dry run / measuring without the screen).
    Null,
}

#[derive(Default)]
pub struct Stats {
    pub rendered: AtomicU64,
    pub render_us: AtomicU64,
    pub rotate_us: AtomicU64,
    pub encoded: AtomicU64,
    pub encode_us: AtomicU64,
    pub encoded_bytes: AtomicU64,
    pub sent: AtomicU64,
    pub send_us: AtomicU64,
    pub sent_bytes: AtomicU64,
    pub dropped_before_encode: AtomicU64,
    pub unchanged_skipped: AtomicU64,
    pub dropped_before_send: AtomicU64,
    /// Frames drawn on the placeholder gradient although the theme has a clip (only after the start-up wait timed out).
    pub gradient_frames: AtomicU64,
    pub send_errors: AtomicU64,
    pub reconnects: AtomicU64,
    pub quality: AtomicU32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StatsSnap {
    pub rendered: u64,
    pub render_us: u64,
    pub rotate_us: u64,
    pub encoded: u64,
    pub encode_us: u64,
    pub encoded_bytes: u64,
    pub sent: u64,
    pub send_us: u64,
    pub sent_bytes: u64,
    pub dropped_before_encode: u64,
    pub unchanged_skipped: u64,
    pub dropped_before_send: u64,
    pub gradient_frames: u64,
    pub send_errors: u64,
    pub reconnects: u64,
    pub quality: u32,
}

impl Stats {
    pub fn snap(&self) -> StatsSnap {
        StatsSnap {
            rendered: self.rendered.load(Relaxed),
            render_us: self.render_us.load(Relaxed),
            rotate_us: self.rotate_us.load(Relaxed),
            encoded: self.encoded.load(Relaxed),
            encode_us: self.encode_us.load(Relaxed),
            encoded_bytes: self.encoded_bytes.load(Relaxed),
            sent: self.sent.load(Relaxed),
            send_us: self.send_us.load(Relaxed),
            sent_bytes: self.sent_bytes.load(Relaxed),
            dropped_before_encode: self.dropped_before_encode.load(Relaxed),
            unchanged_skipped: self.unchanged_skipped.load(Relaxed),
            dropped_before_send: self.dropped_before_send.load(Relaxed),
            gradient_frames: self.gradient_frames.load(Relaxed),
            send_errors: self.send_errors.load(Relaxed),
            reconnects: self.reconnects.load(Relaxed),
            quality: self.quality.load(Relaxed),
        }
    }
}

pub struct Shared {
    pub quit: AtomicBool,
    pub paused: AtomicBool,
    pub fps: AtomicU32,
    pub show_seconds: AtomicBool,
    pub max_kb: AtomicU32,
    pub connected: AtomicBool,
    /// True only while a live session is open on the screen (false while paused, connecting or disconnected).
    pub session_active: AtomicBool,
    /// How the stream ends on stop: 0 = loop clip, 1 = screen off. Set before `quit`.
    pub ending: AtomicU8,
    /// Shutdown: skip the pauses and do not wait for the other threads.
    pub fast_stop: AtomicBool,
    /// Current config, changed at run time through `reload_config`, `set_background`, `set_fps`.
    pub cfg: Mutex<Config>,
    pub cfg_gen: AtomicU64,
    /// Current theme (set through `set_theme`); the render thread picks it up when `theme_gen` changes.
    pub theme: Mutex<Option<ThemeLive>>,
    pub theme_gen: AtomicU64,
    /// Calibration guides drawn over every live frame while Some (see `set_calibration`).
    pub calibration: Mutex<Option<Guides>>,
    pub calibration_gen: AtomicU64,
    /// Hand-over at stop: 0 = none, 1 = fade onto the loop's first frame, 2 = fade to black. Set by `stop()` while the stream runs.
    pub stop_mode: AtomicU8,
    /// The render thread has put the final hand-over frame in the slot; `hand_sent_mark` is the sent-frame count at that moment.
    pub hand_done: AtomicBool,
    pub hand_sent_mark: AtomicU64,
    pub stats: Stats,
}

pub struct Pipeline {
    pub shared: Arc<Shared>,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
    /// Raw process handle of the background-video ffmpeg, for measuring.
    pub video_child: Arc<Mutex<Option<usize>>>,
    /// How the background clip is decoded ("d3d11va", "software", ...); empty without a clip.
    pub video_decode: Arc<Mutex<String>>,
}

fn log(s: &str) {
    crate::device::log(&format!("companion: {s}"));
}

impl Pipeline {
    pub fn start(cfg: &Config, sink: Sink) -> Pipeline {
        let shared = Arc::new(Shared {
            quit: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            fps: AtomicU32::new(if cfg.fps >= 45 { 60 } else { 30 }),
            show_seconds: AtomicBool::new(cfg.show_seconds),
            max_kb: AtomicU32::new(cfg.max_kb),
            connected: AtomicBool::new(sink == Sink::Null),
            session_active: AtomicBool::new(false),
            ending: AtomicU8::new(0),
            fast_stop: AtomicBool::new(false),
            cfg: Mutex::new(cfg.clone()),
            cfg_gen: AtomicU64::new(0),
            theme: Mutex::new(None),
            theme_gen: AtomicU64::new(0),
            calibration: Mutex::new(None),
            calibration_gen: AtomicU64::new(0),
            stop_mode: AtomicU8::new(0),
            hand_done: AtomicBool::new(false),
            hand_sent_mark: AtomicU64::new(0),
            stats: Stats::default(),
        });
        let rgba_slot: Arc<Latest<Vec<u8>>> = Arc::new(Latest::new());
        let jpeg_slot: Arc<Latest<Vec<u8>>> = Arc::new(Latest::new());
        let rgba_pool: Arc<Pool<Vec<u8>>> = Arc::new(Pool(Mutex::new(Vec::new())));
        let jpeg_pool: Arc<Pool<Vec<u8>>> = Arc::new(Pool(Mutex::new(Vec::new())));
        let video_child = Arc::new(Mutex::new(None));
        let video_decode = Arc::new(Mutex::new(String::new()));
        let mut threads = Vec::new();

        // render
        {
            let (sh, slot, pool) = (shared.clone(), rgba_slot.clone(), rgba_pool.clone());
            let (vc, vd) = (video_child.clone(), video_decode.clone());
            threads.push(std::thread::spawn(move || {
                let sampler = Sampler::start();
                let mut r = Renderer::new(sampler.data.clone(), None);
                if !r.has_fonts() {
                    log("no system font found (Segoe UI / Arial): text is not drawn");
                }
                let mut cfg = sh.cfg.lock().unwrap().clone();
                let mut seen_gen = sh.cfg_gen.load(Relaxed);
                let mut seen_theme_gen = u64::MAX;
                let mut seen_cal_gen = u64::MAX;
                let mut music: Option<MusicWatcher> = None;
                let mut video_on = false;
                // What the background decodes: (file, is the stored portrait base, decode mode).
                let mut applied_src: Option<(PathBuf, bool, String)> = None;
                let mut fade_in_secs = 0.8f32;
                let mut last_mask_check = Instant::now();
                let mut next = Instant::now();
                let mut last_emit = Instant::now();
                let mut force_apply = true;
                let mut was_active = false;
                let mut fade_in_start: Option<Instant> = None;
                let mut fade_pending = false;
                let mut bg_wait: Option<Instant> = None;
                let mut hand_start: Option<Instant> = None;
                while !sh.quit.load(Relaxed) {
                    // Runtime changes (reload_config / set_background / set_fps / set_theme).
                    let g = sh.cfg_gen.load(Relaxed);
                    let tg = sh.theme_gen.load(Relaxed);
                    if g != seen_gen || tg != seen_theme_gen {
                        seen_gen = g;
                        seen_theme_gen = tg;
                        cfg = sh.cfg.lock().unwrap().clone();
                        let eff = effective_theme(&cfg, sh.theme.lock().unwrap().clone());
                        fade_in_secs = eff.theme.handover.fade_in;
                        r.set_theme(eff);
                        force_apply = true;
                    }
                    let cg = sh.calibration_gen.load(Relaxed);
                    if cg != seen_cal_gen {
                        seen_cal_gen = cg;
                        r.set_calibration(sh.calibration.lock().unwrap().clone());
                    }
                    let active = !sh.paused.load(Relaxed) && sh.connected.load(Relaxed);
                    if active && !was_active {
                        fade_pending = true;
                        bg_wait = None;
                    }
                    was_active = active;
                    if force_apply {
                        force_apply = false;
                        // Music: start/stop the watcher, or just change the preferred app. Only when the theme has a music widget.
                        match (cfg.music.enabled && r.has_widget("music"), music.is_some()) {
                            (true, false) => {
                                let m = MusicWatcher::start(cfg.music.preferred_app.clone());
                                r.set_music_source(Some(m.data.clone()));
                                music = Some(m);
                            }
                            (true, true) => music.as_ref().unwrap().set_preferred(cfg.music.preferred_app.clone()),
                            (false, true) => {
                                r.set_music_source(None);
                                music = None;
                            }
                            _ => {}
                        }
                        r.set_mask(if cfg.notch == "off" { Mask::none() } else { Mask::from_app_settings() }.with_corner_override(r.theme().theme.corners));
                        // A changed background source: drop the old decoder now, the block below starts the new one.
                        let src = background_source(r.theme());
                        let key = src.map(|(p, portrait)| (p, portrait, cfg.video_decode.clone()));
                        if key != applied_src {
                            applied_src = key;
                            bg_wait = None;
                            r.set_video(None);
                            video_on = false;
                            let portrait = applied_src.as_ref().is_some_and(|k| k.1);
                            r.set_bg_is_base(portrait);
                            // The first frame of the stored base, for the hand-over (one short ffmpeg run, off this thread).
                            let slot_first = r.first_frame_slot();
                            *slot_first.lock().unwrap() = None;
                            if let Some((p, true, _)) = applied_src.clone() {
                                std::thread::spawn(move || match super::video::first_frame(&p, true) {
                                    Ok(f) => *slot_first.lock().unwrap() = Some(Arc::new(f)),
                                    Err(e) => log(&format!("hand-over: no first frame of the base ({e}); widgets only fade out")),
                                });
                            }
                        }
                    }
                    // The decoder only runs while frames are being streamed (an idle ffmpeg would burn CPU for nothing).
                    let want_video = active && applied_src.is_some();
                    if want_video && !video_on {
                        let (p, portrait, dec) = applied_src.clone().unwrap();
                        let res = if portrait { VideoBg::start_base(&p) } else { VideoBg::start(&p, &dec) };
                        match res {
                            Ok(v) => {
                                publish_video_info(&v, &vc, &vd);
                                r.set_video(Some(v));
                            }
                            Err(e) => log(&format!("background clip not used: {e}")),
                        }
                        video_on = true;
                    } else if !want_video && video_on {
                        r.set_video(None);
                        *vc.lock().unwrap() = None;
                        video_on = false;
                    }
                    if !active {
                        std::thread::sleep(Duration::from_millis(100));
                        next = Instant::now();
                        continue;
                    }
                    // Hand-over finished: the last frame is out, wait for the stop.
                    if sh.hand_done.load(Relaxed) {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    // Never show the placeholder gradient when the theme has a clip: hold until the first real background frame
                    // is decoded (the screen keeps its own loop; keepalives continue), and start the live-layer fade from that
                    // frame. After BG_WAIT the gradient may be used (decoding failed).
                    if applied_src.is_some() && !r.has_background_frame() {
                        let w0 = *bg_wait.get_or_insert_with(Instant::now);
                        if w0.elapsed() < BG_WAIT {
                            std::thread::sleep(Duration::from_millis(10));
                            next = Instant::now();
                            continue;
                        }
                    } else {
                        bg_wait = None;
                    }
                    if fade_pending {
                        fade_pending = false;
                        fade_in_start = Some(Instant::now());
                    }
                    if cfg.notch != "off" && last_mask_check.elapsed() > Duration::from_secs(3) {
                        last_mask_check = Instant::now();
                        r.set_mask(Mask::from_app_settings().with_corner_override(r.theme().theme.corners)); // picks up a notch change made in the app
                    }
                    // Live-layer opacity: fade in after the session starts; at a stop with a hand-over, fade out while the
                    // background cross-fades to the stored loop's first frame (or to black).
                    let mut alpha = match fade_in_start {
                        Some(t0) if fade_in_secs > 0.0 => smooth((t0.elapsed().as_secs_f32() / fade_in_secs).min(1.0)),
                        _ => 1.0,
                    };
                    let mode = sh.stop_mode.load(Relaxed);
                    let mut finishing = false;
                    if mode != 0 {
                        let hs = *hand_start.get_or_insert_with(Instant::now);
                        let p = (hs.elapsed().as_secs_f32() / HANDOVER_FADE_S).min(1.0);
                        alpha *= 1.0 - smooth(p);
                        let target = if mode == 2 {
                            Some(HandTarget::Black)
                        } else if r.has_first_frame() {
                            Some(HandTarget::FirstFrame)
                        } else {
                            None
                        };
                        r.set_handover(target.map(|t| (t, smooth(p))));
                        finishing = p >= 1.0;
                    }
                    r.set_live_alpha(alpha);
                    let period = Duration::from_secs_f64(1.0 / sh.fps.load(Relaxed).max(1) as f64);
                    let t = Instant::now();
                    let on_gradient = applied_src.is_some() && !r.has_background_frame();
                    let (frame, changed) = r.render();
                    let t1 = Instant::now();
                    // Nothing changed since the last frame (the renderer says so without redrawing): skip rotate, encode and send.
                    // The screen keeps the last picture (keepalives continue), and one frame per half second is still sent as a safety net.
                    if !changed && last_emit.elapsed() < Duration::from_millis(500) {
                        sh.stats.rendered.fetch_add(1, Relaxed);
                        sh.stats.render_us.fetch_add((t1 - t).as_micros() as u64, Relaxed);
                        sh.stats.unchanged_skipped.fetch_add(1, Relaxed);
                        next += period;
                        let now = Instant::now();
                        if next > now {
                            std::thread::sleep(next - now);
                        } else if now - next > period * 3 {
                            next = now;
                        }
                        continue;
                    }
                    last_emit = Instant::now();
                    if on_gradient {
                        sh.stats.gradient_frames.fetch_add(1, Relaxed);
                    }
                    let mut buf = pool.get();
                    rotate_cw(frame, &mut buf);
                    let t2 = Instant::now();
                    sh.stats.rendered.fetch_add(1, Relaxed);
                    sh.stats.render_us.fetch_add((t1 - t).as_micros() as u64, Relaxed);
                    sh.stats.rotate_us.fetch_add((t2 - t1).as_micros() as u64, Relaxed);
                    if let Some(old) = slot.put(buf) {
                        sh.stats.dropped_before_encode.fetch_add(1, Relaxed);
                        pool.put(old);
                    }
                    if finishing {
                        // The final frame (the loop's first frame, or black) is in the slot: tell the stopper.
                        sh.hand_sent_mark.store(sh.stats.sent.load(Relaxed), Relaxed);
                        sh.hand_done.store(true, Relaxed);
                        continue;
                    }
                    next += period;
                    let now = Instant::now();
                    if next > now {
                        std::thread::sleep(next - now);
                    } else if now - next > period * 3 {
                        next = now; // far behind: do not try to catch up with a burst
                    }
                }
            }));
        }

        // encode
        {
            let (sh, rin, pin, jout, jpool) = (shared.clone(), rgba_slot.clone(), rgba_pool.clone(), jpeg_slot.clone(), jpeg_pool.clone());
            threads.push(std::thread::spawn(move || {
                let mut enc = JpegEncoder::default();
                while !sh.quit.load(Relaxed) {
                    let Some(rgba) = rin.take(Duration::from_millis(200)) else { continue };
                    let mut out = jpool.get();
                    let budget = frame_budget(sh.max_kb.load(Relaxed), sh.fps.load(Relaxed));
                    let t = Instant::now();
                    match enc.encode(&rgba, PW, PH, budget, &mut out) {
                        Ok(n) => {
                            sh.stats.encoded.fetch_add(1, Relaxed);
                            sh.stats.encode_us.fetch_add(t.elapsed().as_micros() as u64, Relaxed);
                            sh.stats.encoded_bytes.fetch_add(n as u64, Relaxed);
                            sh.stats.quality.store(enc.quality as u32, Relaxed);
                            if let Some(old) = jout.put(out) {
                                sh.stats.dropped_before_send.fetch_add(1, Relaxed);
                                jpool.put(old);
                            }
                        }
                        Err(e) => log(&e),
                    }
                    pin.put(rgba);
                }
            }));
        }

        // send
        {
            let (sh, jin, jpool) = (shared.clone(), jpeg_slot.clone(), jpeg_pool.clone());
            threads.push(std::thread::spawn(move || {
                let mut session: Option<LiveSession> = None;
                let (mut backoff, mut next_try) = (1u64, Instant::now());
                loop {
                    if sh.quit.load(Relaxed) {
                        break;
                    }
                    if sh.paused.load(Relaxed) {
                        if let Some(s) = session.take() {
                            log("paused: ending the live stream (loop clip plays)");
                            s.stop_with(Ending::LoopClip);
                            sh.session_active.store(false, Relaxed);
                            sh.connected.store(false, Relaxed);
                        }
                        jin.clear();
                        std::thread::sleep(Duration::from_millis(100));
                        continue;
                    }
                    if sink == Sink::Device && session.is_none() {
                        if Instant::now() < next_try {
                            std::thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        match LiveSession::connect() {
                            Ok(s) => {
                                log("connected, live stream started");
                                session = Some(s);
                                sh.connected.store(true, Relaxed);
                                sh.session_active.store(true, Relaxed);
                                sh.stats.reconnects.fetch_add(1, Relaxed);
                                backoff = 1;
                            }
                            Err(e) => {
                                log(&format!("cannot connect ({e}); retry in {backoff} s"));
                                next_try = Instant::now() + Duration::from_secs(backoff);
                                backoff = (backoff * 2).min(15);
                                continue;
                            }
                        }
                    }
                    let wait = if sh.fast_stop.load(Relaxed) { 20 } else { 200 };
                    match jin.take(Duration::from_millis(wait)) {
                        Some(jpeg) => {
                            let t = Instant::now();
                            let r = match session.as_mut() {
                                Some(s) => s.send_frame(&jpeg),
                                None => Ok(()), // Null sink
                            };
                            match r {
                                Ok(()) => {
                                    sh.stats.sent.fetch_add(1, Relaxed);
                                    sh.stats.send_us.fetch_add(t.elapsed().as_micros() as u64, Relaxed);
                                    sh.stats.sent_bytes.fetch_add(jpeg.len() as u64, Relaxed);
                                }
                                Err(e) => {
                                    log(&format!("send failed ({e}); reconnecting"));
                                    sh.stats.send_errors.fetch_add(1, Relaxed);
                                    sh.connected.store(false, Relaxed);
                                    sh.session_active.store(false, Relaxed);
                                    if let Some(s) = session.take() {
                                        s.abandon();
                                    }
                                    next_try = Instant::now() + Duration::from_secs(1);
                                }
                            }
                            jpool.put(jpeg);
                        }
                        None => {
                            if let Some(s) = session.as_mut() {
                                if let Err(e) = s.keepalive_if_due() {
                                    log(&format!("keepalive failed ({e}); reconnecting"));
                                    sh.connected.store(false, Relaxed);
                                    sh.session_active.store(false, Relaxed);
                                    if let Some(s) = session.take() {
                                        s.abandon();
                                    }
                                    next_try = Instant::now() + Duration::from_secs(1);
                                }
                            }
                        }
                    }
                }
                if let Some(s) = session.take() {
                    let ending = if sh.ending.load(Relaxed) == 1 { Ending::ScreenOff } else { Ending::LoopClip };
                    sh.session_active.store(false, Relaxed);
                    if sh.fast_stop.load(Relaxed) {
                        log("stopping fast (shutdown)");
                        s.stop_fast(ending);
                    } else {
                        log(match ending {
                            Ending::LoopClip => "stopping: loop clip plays",
                            Ending::ScreenOff => "stopping: screen off",
                        });
                        s.stop_with(ending);
                    }
                }
            }));
        }

        Pipeline { shared, threads: Mutex::new(threads), video_child, video_decode }
    }

    /// True only while a live session is open on the screen (false while paused, connecting, disconnected or stopped).
    pub fn session_active(&self) -> bool {
        self.shared.session_active.load(Relaxed)
    }

    /// Live frame rate, 30 or 60, effective immediately. The background clip decodes at its own rate (and is paced with
    /// -re), so there is no decoder to restart.
    pub fn set_fps(&self, fps: u32) {
        let f = if fps >= 45 { 60 } else { 30 };
        self.shared.fps.store(f, Relaxed);
        self.shared.cfg.lock().unwrap().fps = f;
    }

    /// Swap the theme (widgets, background, hand-over) while running, without restarting the pipeline.
    pub fn set_theme(&self, t: ThemeLive) {
        *self.shared.theme.lock().unwrap() = Some(t);
        self.shared.theme_gen.fetch_add(1, Relaxed);
    }

    /// Screen-guide calibration: while Some, every live frame gets the guide lines, zone tints and labels drawn on top (and a
    /// change redraws at once). None = normal frames. Guides are never drawn otherwise. Temporary: call with None to leave.
    pub fn set_calibration(&self, g: Option<Guides>) {
        *self.shared.calibration.lock().unwrap() = g;
        self.shared.calibration_gen.fetch_add(1, Relaxed);
    }

    /// Change the background clip while running (None: the animated gradient). The decoder is restarted by the render thread.
    pub fn set_background(&self, clip: Option<String>) {
        self.shared.cfg.lock().unwrap().background = clip.filter(|c| !c.is_empty());
        self.shared.cfg_gen.fetch_add(1, Relaxed);
    }

    /// Apply a new config while running: fps, seconds, frame size, background clip, decode mode, notch mode, music.
    pub fn reload_config(&self, cfg: &Config) {
        self.shared.fps.store(if cfg.fps >= 45 { 60 } else { 30 }, Relaxed);
        self.shared.show_seconds.store(cfg.show_seconds, Relaxed);
        self.shared.max_kb.store(cfg.max_kb, Relaxed);
        *self.shared.cfg.lock().unwrap() = cfg.clone();
        self.shared.cfg_gen.fetch_add(1, Relaxed);
    }

    /// Stop the normal way. With a theme set, the theme's hand-over decides how it ends:
    ///   loopStart  the widgets fade out while the background cross-fades to the stored loop's first frame, that frame is sent,
    ///              then the loop clip ending (the screen continues from its first frame); about 1.2 s in all
    ///   screenOff  fade to black, then the screen-off ending
    ///   instant    (and without a theme, or when no session is open) the plain stop: loop clip ending at once
    /// Waits for the threads.
    pub fn stop(&self) {
        let end = self.shared.theme.lock().unwrap().as_ref().map(|t| t.theme.handover.end);
        let mode = match end {
            Some(HandoverEnd::LoopStart) => 1,
            Some(HandoverEnd::ScreenOff) => 2,
            _ => 0,
        };
        if mode != 0 && self.shared.connected.load(Relaxed) && !self.shared.paused.load(Relaxed) && !self.shared.quit.load(Relaxed) {
            self.shared.stop_mode.store(mode, Relaxed);
            let t0 = Instant::now();
            while !self.shared.hand_done.load(Relaxed) && t0.elapsed() < Duration::from_millis(1000) {
                std::thread::sleep(Duration::from_millis(5));
            }
            // Give the sender a moment to put the final frame on the wire before the end-of-stream marker.
            if self.shared.hand_done.load(Relaxed) {
                let mark = self.shared.hand_sent_mark.load(Relaxed);
                let t1 = Instant::now();
                while self.shared.stats.sent.load(Relaxed) <= mark && t1.elapsed() < Duration::from_millis(250) {
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
            self.stop_with(if mode == 2 { Ending::ScreenOff } else { Ending::LoopClip });
        } else {
            self.stop_with(if mode == 2 { Ending::ScreenOff } else { Ending::LoopClip });
        }
    }

    /// Stop and end the stream as `ending` says. Waits for the threads; safe to call more than once from any thread.
    pub fn stop_with(&self, ending: Ending) {
        self.shared.ending.store(if ending == Ending::ScreenOff { 1 } else { 0 }, Relaxed);
        self.shared.quit.store(true, Relaxed);
        let ts: Vec<_> = self.threads.lock().unwrap().drain(..).collect();
        join_bounded(ts, STOP_JOIN_MAX);
    }

    /// Shutdown / end-of-session variant: no pauses between the end-of-stream writes, and it does not wait for the render
    /// and encode threads (they end on their own). Returns after the stream is ended, or after 800 ms at the latest if the
    /// screen does not answer. Typically well under 300 ms.
    pub fn stop_fast(&self, ending: Ending) {
        self.shared.fast_stop.store(true, Relaxed);
        self.shared.ending.store(if ending == Ending::ScreenOff { 1 } else { 0 }, Relaxed);
        self.shared.quit.store(true, Relaxed);
        let mut ts: Vec<_> = self.threads.lock().unwrap().drain(..).collect();
        // threads are [render, encode, send]: only the sender talks to the screen
        let Some(send) = ts.pop() else { return };
        let end = Instant::now() + Duration::from_millis(800);
        while !send.is_finished() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        if send.is_finished() {
            let _ = send.join();
        }
    }
}

/// The longest a normal stop waits for the engine threads. A thread stuck in a USB call (screen held by another program,
/// or not answering) must never keep the app from quitting.
pub const STOP_JOIN_MAX: Duration = Duration::from_millis(2500);

/// Join threads, but give up (and leave them behind) once `max` has passed in total. Returns true if all ended.
pub fn join_bounded(ts: Vec<std::thread::JoinHandle<()>>, max: Duration) -> bool {
    let end = Instant::now() + max;
    let mut all = true;
    for t in ts {
        while !t.is_finished() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(5));
        }
        if t.is_finished() {
            let _ = t.join();
        } else {
            eprintln!("live: an engine thread did not end within {:?}; not waiting for it", max);
            all = false;
        }
    }
    all
}

/// How long live start waits for the first real background frame before it falls back to the gradient.
const BG_WAIT: Duration = Duration::from_secs(3);

/// Length of the hand-over fade at stop: widgets fade out while the background cross-fades (seconds).
const HANDOVER_FADE_S: f32 = 0.45;

fn smooth(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The theme the renderer uses: the one set through `set_theme`, else the Standard layout over the config's background clip
/// (the 2.0-style companion), with the clock seconds taken from the config.
fn effective_theme(cfg: &Config, set: Option<ThemeLive>) -> ThemeLive {
    if let Some(t) = set {
        return t;
    }
    let mut th = Theme::standard_preset();
    if let Some(c) = th.widgets.iter_mut().find(|w| w.kind == "clock") {
        c.props["seconds"] = serde_json::json!(cfg.show_seconds);
    }
    let mut t = ThemeLive::new(th, PathBuf::new());
    t.fallback_video = cfg.background.clone().filter(|b| !b.is_empty()).map(PathBuf::from);
    t
}

/// Where the live background comes from: the stored base (portrait, true), else the original clip (false), else nothing
/// (the animated gradient).
fn background_source(t: &ThemeLive) -> Option<(PathBuf, bool)> {
    if let Some(p) = t.base_video.as_ref().filter(|p| p.exists()) {
        return Some((p.clone(), true));
    }
    t.fallback_video.as_ref().filter(|p| p.exists()).map(|p| (p.clone(), false))
}


/// Mirror the decoder's process handle and method for the measurer once ffmpeg has started.
fn publish_video_info(v: &VideoBg, vc: &Arc<Mutex<Option<usize>>>, vd: &Arc<Mutex<String>>) {
    *vc.lock().unwrap() = Some(0);
    let (h, how) = (v.child_handle.clone(), v.decode.clone());
    let (vc2, vd2) = (vc.clone(), vd.clone());
    std::thread::spawn(move || {
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            if let Some(handle) = *h.lock().unwrap() {
                *vc2.lock().unwrap() = Some(handle);
                *vd2.lock().unwrap() = how.lock().unwrap().clone();
                break;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn join_bounded_does_not_wait_for_a_stuck_thread() {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let stuck = std::thread::spawn(move || { let _ = rx.recv(); });
        let quick = std::thread::spawn(|| {});
        let t0 = std::time::Instant::now();
        assert!(!super::join_bounded(vec![quick, stuck], std::time::Duration::from_millis(300)));
        assert!(t0.elapsed() < std::time::Duration::from_millis(1500));
        drop(tx);
        assert!(super::join_bounded(vec![std::thread::spawn(|| {})], std::time::Duration::from_millis(300)));
    }

    use super::*;

    fn quiet_cfg() -> Config {
        let mut c = Config::default();
        c.music.enabled = false;
        c.notch = "off".into();
        c
    }

    #[test]
    fn null_sink_runtime_apis_and_fast_stop() {
        let p = Pipeline::start(&quiet_cfg(), Sink::Null);
        assert!(!p.session_active()); // no real session on the null sink
        p.set_fps(30);
        assert_eq!(p.shared.fps.load(Relaxed), 30);
        let mut c = quiet_cfg();
        c.fps = 60;
        c.show_seconds = true;
        p.reload_config(&c);
        assert_eq!(p.shared.fps.load(Relaxed), 60);
        assert!(p.shared.show_seconds.load(Relaxed));
        p.set_background(None);
        std::thread::sleep(Duration::from_millis(300));
        assert!(p.shared.stats.snap().rendered > 0);
        let t = Instant::now();
        p.stop_fast(Ending::ScreenOff);
        assert!(t.elapsed() < Duration::from_millis(800), "fast stop took {:?}", t.elapsed());
        assert_eq!(p.shared.ending.load(Relaxed), 1);
    }

    #[test]
    fn normal_stop_defaults_to_loop_clip() {
        let p = Pipeline::start(&quiet_cfg(), Sink::Null);
        p.stop();
        assert_eq!(p.shared.ending.load(Relaxed), 0);
        assert_eq!(Ending::default(), Ending::LoopClip);
    }
}

#[cfg(test)]
mod handover_tests {
    use super::*;

    #[test]
    fn themed_stop_fades_out_within_the_budget() {
        let mut c = Config::default();
        c.music.enabled = false;
        c.notch = "off".into();
        let p = Pipeline::start(&c, Sink::Null);
        p.set_theme(ThemeLive::new(Theme::standard_preset(), PathBuf::new()));
        std::thread::sleep(Duration::from_millis(1200));
        let t = Instant::now();
        p.stop();
        let e = t.elapsed();
        assert!(e >= Duration::from_millis(350) && e < Duration::from_millis(1200), "themed stop took {e:?}");
        assert!(p.shared.hand_done.load(Relaxed));
    }

    #[test]
    fn instant_end_stops_at_once() {
        let mut c = Config::default();
        c.music.enabled = false;
        c.notch = "off".into();
        let p = Pipeline::start(&c, Sink::Null);
        let mut t = ThemeLive::new(Theme::standard_preset(), PathBuf::new());
        t.theme.handover.end = HandoverEnd::Instant;
        p.set_theme(t);
        std::thread::sleep(Duration::from_millis(500));
        let s = Instant::now();
        p.stop();
        assert!(s.elapsed() < Duration::from_millis(300));
    }

    fn quiet() -> Config {
        let mut c = Config::default();
        c.music.enabled = false;
        c.notch = "off".into();
        c
    }

    /// The start never draws the placeholder gradient while the theme has a clip: nothing is sent before the first decoded frame.
    #[test]
    fn no_gradient_frame_before_the_first_real_background_frame() {
        let clip = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("Gallery").join("Ocean Echo [we-1635817562].mp4");
        if !clip.exists() || !crate::media::ffmpeg::locate().exists() {
            return;
        }
        let p = Pipeline::start(&quiet(), Sink::Null);
        let mut t = ThemeLive::new(Theme::standard_preset(), PathBuf::new());
        t.fallback_video = Some(clip);
        p.set_theme(t);
        // sample while the decoder starts: whenever something has been sent, it was a real frame
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(4) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let s = p.shared.stats.snap();
        assert!(s.sent > 0, "frames flow once the clip decodes");
        assert_eq!(s.gradient_frames, 0, "no frame was drawn on the placeholder gradient");
        p.stop_fast(Ending::LoopClip);
    }

    /// A clip that never decodes: nothing is sent during the wait, then the gradient is used as a last resort.
    #[test]
    fn broken_clip_holds_then_times_out_to_the_gradient() {
        let bad = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
        let p = Pipeline::start(&quiet(), Sink::Null);
        let mut t = ThemeLive::new(Theme::standard_preset(), PathBuf::new());
        t.fallback_video = Some(bad);
        p.set_theme(t);
        std::thread::sleep(Duration::from_millis(1800));
        assert_eq!(p.shared.stats.snap().sent, 0, "held while waiting for a real frame");
        std::thread::sleep(Duration::from_millis(2500));
        let s = p.shared.stats.snap();
        assert!(s.sent > 0 && s.gradient_frames > 0, "gradient allowed after the timeout");
        p.stop_fast(Ending::LoopClip);
    }
}
