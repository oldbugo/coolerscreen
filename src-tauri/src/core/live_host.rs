//! 3.0: the live companion engine (`crate::live`) hosted inside the app process.
//! The host owns the pipeline and the rules that keep the screen to one user at a time. Live is paused (the stream ends and the
//! screen plays its loop clip) while any "hold" is set:
//!   USER    the tray's Pause;
//!   DEVICE  a device task (send, restart, check, setup...) is queued or running;
//!   SLEEP / LOCK  the PC sleeps / is locked, when the user chose "show the loop clip" for it.
//! A hold is released after a short delay, so the screen has settled before live reconnects.
//! Nothing here touches the screen except through `Pipeline` / `LiveSession` (0x15 [00 0A], 0x11 and frames; stop sends
//! FF D9 FF D9, 0x0E [1], 0x14 [1,0]).
use crate::live::pipeline::{Pipeline, Sink};
use crate::live::session::Ending;
use crate::live::Config;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const USER: u32 = 1;
pub const DEVICE: u32 = 2;
pub const SLEEP: u32 = 4;
pub const LOCK: u32 = 8;
/// The current theme's "Live layer" switch is off: nothing is streamed, the screen plays its own base.
pub const LAYER: u32 = 16;
/// The Screen fit page has the ruler picture on the screen.
pub const FITCAL: u32 = 32;

pub struct LiveHost {
    pipeline: Mutex<Option<Arc<Pipeline>>>,
    /// Frames are discarded (measuring or testing without the screen).
    no_device: AtomicBool,
    /// Why live is paused (bits above).
    holds: AtomicU32,
    /// Bumped on every hold / release so only the newest delayed resume acts.
    resume_gen: AtomicU64,
    /// Overrides for the saved config (tests, measuring).
    cfg_override: Mutex<Option<Config>>,
    /// The config live was last started or reloaded with (to start again after a screen-off stop).
    last_cfg: Mutex<Option<Config>>,
    /// Live was stopped with the screen off for a sleep or lock hold and starts again when the holds are released.
    off_stopped: AtomicBool,
    /// The theme live shows (3.0): given to every pipeline when it starts and whenever it changes.
    theme: Mutex<Option<crate::live::theme::ThemeLive>>,
    /// Calibration guides drawn on the real screen while the Screen guides page is open (None = off).
    calibration: Mutex<Option<crate::live::theme::Guides>>,
    /// The screen fit live frames are drawn with (given to every pipeline when it starts and whenever it changes).
    fit: Mutex<crate::core::fit::Fit>,
    /// The temporary Screen fit calibration session (ruler picture), while the page is open.
    fit_cal: Mutex<Option<crate::live::fitcal::FitCalibration>>,
    /// The unsaved fit the Screen fit page is trying on the live picture (None: not tuning), and whether tuning started live.
    fit_tune: Mutex<Option<crate::core::fit::Fit>>,
    tune_temp: AtomicBool,
    /// 4.x preview: a theme (or a picture) tried on the real screen from My themes / Browse without being used. It is shown
    /// instead of the live theme and never saved; `preview_temp` says it started the stream (Live off), so ending it stops it.
    preview: Mutex<Option<crate::live::theme::ThemeLive>>,
    preview_temp: AtomicBool,
    /// When the page last asked for the preview (a safety net: a preview nobody looks at ends by itself) and the watcher state.
    preview_touched: Mutex<Option<std::time::Instant>>,
    preview_watching: AtomicBool,
    preview_idle_ms: AtomicU64,
    /// The editor's digital twin started the stream (Live off): frames are rendered and encoded for the editor only, nothing is sent.
    twin_temp: AtomicBool,
    twin_watching: AtomicBool,
    /// Called when something the UI and the tray show changes (theme, idle state).
    on_change: Mutex<Vec<Arc<dyn Fn() + Send + Sync>>>,
    /// How many times live was paused (holds went from none to some) and brought back since the app started (tests, log).
    pauses: AtomicU64,
    resumes: AtomicU64,
}

/// Why a hold bit is set, in words for device.log.
fn reason(bits: u32) -> String {
    let mut v = Vec::new();
    for (b, t) in [
        (USER, "user pause"),
        (DEVICE, "device task"),
        (SLEEP, "sleep"),
        (LOCK, "lock"),
        (LAYER, "no live items or live layer off: the screen plays its own base"),
        (FITCAL, "screen fit rulers"),
    ] {
        if bits & b != 0 {
            v.push(t);
        }
    }
    v.join(", ")
}
fn hlog(msg: &str) {
    crate::device::log(&format!("live: {msg}"));
}

impl Default for LiveHost {
    fn default() -> Self {
        LiveHost {
            pipeline: Mutex::new(None),
            no_device: AtomicBool::new(false),
            holds: AtomicU32::new(0),
            resume_gen: AtomicU64::new(0),
            cfg_override: Mutex::new(None),
            last_cfg: Mutex::new(None),
            off_stopped: AtomicBool::new(false),
            theme: Mutex::new(None),
            calibration: Mutex::new(None),
            fit: Mutex::new(crate::core::fit::Fit::default()),
            fit_cal: Mutex::new(None),
            fit_tune: Mutex::new(None),
            tune_temp: AtomicBool::new(false),
            preview: Mutex::new(None),
            preview_temp: AtomicBool::new(false),
            preview_touched: Mutex::new(None),
            preview_watching: AtomicBool::new(false),
            preview_idle_ms: AtomicU64::new(150_000),
            twin_temp: AtomicBool::new(false),
            twin_watching: AtomicBool::new(false),
            on_change: Mutex::new(Vec::new()),
            pauses: AtomicU64::new(0),
            resumes: AtomicU64::new(0),
        }
    }
}

impl LiveHost {
    pub fn new() -> Arc<LiveHost> {
        Arc::new(LiveHost::default())
    }

    /// Something the tray tooltip or the page shows has changed (live items, idle).
    pub fn set_on_change(&self, f: impl Fn() + Send + Sync + 'static) {
        self.on_change.lock().unwrap().push(Arc::new(f));
    }

    fn changed(&self) {
        let fs = self.on_change.lock().unwrap().clone();
        for f in fs {
            f();
        }
    }

    pub fn pause_count(&self) -> u64 {
        self.pauses.load(SeqCst)
    }
    pub fn resume_count(&self) -> u64 {
        self.resumes.load(SeqCst)
    }

    /// Streaming right now: running, nothing holds it, and (with the real screen) the session is connected. A check or state
    /// request is answered from this instead of opening the screen again.
    pub fn streaming(&self) -> bool {
        let Some(p) = self.pipeline() else { return false };
        self.holds.load(SeqCst) == 0 && !p.shared.paused.load(SeqCst) && (self.no_device.load(SeqCst) || p.shared.connected.load(SeqCst))
    }

    /// Set a hold bit, counting and logging the pause when it is the first one.
    fn add_hold(&self, bit: u32) {
        let was = self.holds.fetch_or(bit, SeqCst);
        if was == 0 {
            self.pauses.fetch_add(1, SeqCst);
        }
        if was & bit == 0 {
            hlog(&format!("pause: {} (holds now: {})", reason(bit), reason(was | bit)));
        }
    }

    pub fn set_no_device(&self, v: bool) {
        self.no_device.store(v, SeqCst);
    }

    pub fn set_config_override(&self, c: Option<Config>) {
        *self.cfg_override.lock().unwrap() = c;
    }

    fn config(&self) -> Config {
        self.cfg_override.lock().unwrap().clone().unwrap_or_else(Config::load)
    }

    /// The saved fps without creating companion.json (the feature may be off).
    fn quiet_fps(&self) -> u32 {
        if let Some(c) = self.cfg_override.lock().unwrap().as_ref() {
            return c.fps;
        }
        std::fs::read_to_string(Config::path()).ok().and_then(|s| serde_json::from_str::<Config>(&s).ok()).map(|c| c.fps).unwrap_or(60)
    }

    /// The screen fit for live frames; a running pipeline switches on its next frame.
    pub fn set_fit(self: &Arc<Self>, f: crate::core::fit::Fit) {
        *self.fit.lock().unwrap() = f;
        self.apply_fit();
    }

    /// The fit live frames use now: the unsaved one while the Screen fit page is tuning, else the saved one.
    pub fn effective_fit(&self) -> crate::core::fit::Fit {
        let tuning = *self.fit_tune.lock().unwrap();
        tuning.unwrap_or(*self.fit.lock().unwrap())
    }

    fn apply_fit(self: &Arc<Self>) {
        if let Some(p) = self.pipeline() {
            p.set_fit(self.effective_fit()); // the renderer redraws and the sender sends at once
        }
        self.evaluate_idle();
    }

    /// Screen fit tuning on the real content ("My theme"): the current theme's live picture goes on the screen with the unsaved
    /// `fit`, and every `tune_set` applies at once. Live on: the running pipeline is simply given the override. Live off: a temporary
    /// pipeline is started and ended again by `tune_end`. A ruler calibration in progress is ended first.
    pub fn tune_start(self: &Arc<Self>, f: crate::core::fit::Fit) {
        if self.calibrating() {
            self.calibrate_end();
        }
        *self.fit_tune.lock().unwrap() = Some(f);
        if self.pipeline().is_none() {
            self.tune_temp.store(true, SeqCst);
            self.start();
        }
        self.apply_fit();
    }

    /// New unsaved fit for the running tuning; false when none is running.
    pub fn tune_set(self: &Arc<Self>, f: crate::core::fit::Fit) -> bool {
        if self.fit_tune.lock().unwrap().is_none() {
            return false;
        }
        *self.fit_tune.lock().unwrap() = Some(f);
        self.apply_fit();
        true
    }

    /// Back to the saved fit; a temporary pipeline ends with the normal hand-over (the screen plays its loop again).
    pub fn tune_end(self: &Arc<Self>) {
        *self.fit_tune.lock().unwrap() = None;
        self.apply_fit();
        if self.tune_temp.swap(false, SeqCst) {
            self.stop();
        }
    }

    pub fn tuning(&self) -> bool {
        self.fit_tune.lock().unwrap().is_some()
    }

    // ---------- preview on the real screen (My themes / Browse) ----------

    /// How long a preview stays up with no sign of life from the page (150 s; the page ends it sooner when the user is idle or leaves).
    pub fn set_preview_idle(&self, d: Duration) {
        self.preview_idle_ms.store(d.as_millis() as u64, SeqCst);
    }

    pub fn previewing(&self) -> bool {
        self.preview.lock().unwrap().is_some()
    }

    fn preview_forget(&self) {
        *self.preview.lock().unwrap() = None;
        self.preview_temp.store(false, SeqCst);
        *self.preview_touched.lock().unwrap() = None;
    }

    /// Show this theme on the real screen instead of the live theme, nothing saved. Live on: the running stream is given the theme
    /// (no reconnect). Live off: a temporary stream starts, and `preview_end` ends it with the normal hand-over. Calling it again
    /// with another theme just swaps the picture. A ruler calibration in progress is ended first.
    pub fn preview_start(self: &Arc<Self>, t: crate::live::theme::ThemeLive) {
        if self.calibrating() {
            self.calibrate_end();
        }
        *self.preview.lock().unwrap() = Some(t.clone());
        *self.preview_touched.lock().unwrap() = Some(std::time::Instant::now());
        match self.pipeline() {
            Some(p) => {
                hlog("preview: another theme on the screen (nothing saved)");
                p.set_theme(t);
            }
            None => {
                hlog("preview: live is off; a temporary stream starts");
                self.preview_temp.store(true, SeqCst);
                self.start();
            }
        }
        self.evaluate_idle();
        self.preview_watch();
    }

    /// The page is still showing the preview (keeps the safety timer from ending it).
    pub fn preview_keepalive(&self) -> bool {
        if !self.previewing() {
            return false;
        }
        *self.preview_touched.lock().unwrap() = Some(std::time::Instant::now());
        true
    }

    /// Back to what the screen showed before: live on gets its own theme again; a temporary stream ends with the normal hand-over
    /// (instant when the previewed picture is not the Embedded one). False when no preview was running.
    pub fn preview_end(self: &Arc<Self>) -> bool {
        if self.preview.lock().unwrap().is_none() {
            return false;
        }
        let temp = self.preview_temp.swap(false, SeqCst);
        if temp {
            // stop() reads the previewed theme from the pipeline for the ending, then forgets the preview
            hlog("preview: ended, the temporary stream stops");
            self.stop();
        } else {
            *self.preview.lock().unwrap() = None;
            *self.preview_touched.lock().unwrap() = None;
            hlog("preview: ended, live shows its own theme again");
            if let Some(p) = self.pipeline() {
                if let Some(t) = self.theme.lock().unwrap().clone() {
                    p.set_theme(t);
                }
            }
        }
        self.evaluate_idle();
        true
    }

    /// One small watcher per preview: ends it when the page has not asked for it in `PREVIEW_IDLE` (window closed, page gone).
    fn preview_watch(self: &Arc<Self>) {
        if self.preview_watching.swap(true, SeqCst) {
            return;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                let last = *me.preview_touched.lock().unwrap();
                match last {
                    None => break,
                    Some(t) if t.elapsed() >= Duration::from_millis(me.preview_idle_ms.load(SeqCst)) => {
                        hlog("preview: nobody asked for it for a while");
                        me.preview_end();
                        break;
                    }
                    _ => {}
                }
            }
            me.preview_watching.store(false, SeqCst);
            if me.previewing() {
                me.preview_watch(); // a new preview began while this watcher was ending
            }
        });
    }

    // ---------- the editor's digital twin ----------

    /// The editor is open and looking: with Live on the frames the engine sends are copied for it (no second render); with Live
    /// off a stream is started for the editor only (15 per second, a null sink: nothing goes to the screen). Safe to call again.
    pub fn twin_start(self: &Arc<Self>) {
        crate::live::twin::touch();
        if self.pipeline().is_none() {
            hlog("twin: live is off; a stream for the editor only starts (nothing is sent)");
            self.twin_temp.store(true, SeqCst);
            self.start();
            self.evaluate_idle();
        }
        self.twin_watch();
    }

    /// The editor closed or is hidden: a stream started for the twin ends; live's own stream is left alone.
    pub fn twin_stop(self: &Arc<Self>) {
        crate::live::twin::clear();
        if self.twin_temp.swap(false, SeqCst) {
            hlog("twin: the editor closed; its stream stops");
            self.stop();
            self.evaluate_idle();
        }
    }

    pub fn twinning(&self) -> bool {
        self.twin_temp.load(SeqCst)
    }

    /// Ends the editor-only stream when the page has not asked for frames for 10 s (window closed, crashed, hidden).
    fn twin_watch(self: &Arc<Self>) {
        if self.twin_watching.swap(true, SeqCst) {
            return;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            while me.twin_temp.load(SeqCst) {
                std::thread::sleep(Duration::from_millis(1000));
                if !crate::live::twin::wanted() {
                    std::thread::sleep(Duration::from_secs(7)); // the page may be between two requests; wanted() already allows 3 s
                    if !crate::live::twin::wanted() {
                        me.twin_stop();
                        break;
                    }
                }
            }
            me.twin_watching.store(false, SeqCst);
        });
    }

    /// Start the Screen fit calibration: the ruler picture for `fit` goes on the screen, whether live is on or not. Live (when on)
    /// steps aside first and comes back after `calibrate_end`. Already running: just updates the picture.
    pub fn calibrate_start(self: &Arc<Self>, fit: crate::core::fit::Fit, sel: usize) {
        if let Some(c) = self.fit_cal.lock().unwrap().as_ref() {
            c.set(fit, sel);
            return;
        }
        self.hold(FITCAL, Duration::from_secs(6));
        let sink = if self.no_device.load(SeqCst) { Sink::Null } else { Sink::Device };
        let mut g = self.fit_cal.lock().unwrap();
        if g.is_none() {
            *g = Some(crate::live::fitcal::FitCalibration::start(fit, sel, sink));
        }
    }

    /// New offsets (screen px) and selected edge for the running calibration; false when none is running.
    pub fn calibrate_set(&self, fit: crate::core::fit::Fit, sel: usize) -> bool {
        match self.fit_cal.lock().unwrap().as_ref() {
            Some(c) => {
                c.set(fit, sel);
                true
            }
            None => false,
        }
    }

    /// End the calibration: the screen plays its loop clip again and live (when it was on) resumes.
    pub fn calibrate_end(self: &Arc<Self>) {
        let c = self.fit_cal.lock().unwrap().take();
        if let Some(c) = c {
            c.end();
        }
        if self.holds.load(SeqCst) & FITCAL != 0 {
            self.release(FITCAL, Duration::from_millis(800));
        }
    }

    fn end_calibration_now(&self) {
        let c = self.fit_cal.lock().unwrap().take();
        if let Some(c) = c {
            c.end();
        }
    }

    /// The ruler picture is on the screen.
    pub fn calibration_connected(&self) -> bool {
        self.fit_cal.lock().unwrap().as_ref().is_some_and(|c| c.connected())
    }

    pub fn calibrating(&self) -> bool {
        self.fit_cal.lock().unwrap().is_some()
    }

    /// The theme live shows: kept for pipelines that start later, and given to the running one without ending the stream.
    /// Draw the guide lines on the real screen (live must be on); None clears them. Temporary: never saved.
    pub fn set_calibration(&self, g: Option<crate::live::theme::Guides>) {
        *self.calibration.lock().unwrap() = g.clone();
        if let Some(p) = self.pipeline() {
            p.set_calibration(g);
        }
    }

    /// A theme whose live layer is off holds live (like a device task does) until another theme, or the switch, brings it back.
    pub fn set_theme(self: &Arc<Self>, t: crate::live::theme::ThemeLive) {
        *self.theme.lock().unwrap() = Some(t.clone());
        if let Some(p) = self.pipeline().filter(|_| !self.previewing()) {
            hlog("theme changed: reloaded in place, the stream goes on");
            p.set_theme(t);
        }
        self.evaluate_idle();
    }

    /// Hold live (no stream, the screen plays its own base) while the theme has nothing to add: its live layer is off, or it has
    /// no live items and its base already has the current screen fit. Looked at again whenever the theme or the fit changes.
    fn evaluate_idle(self: &Arc<Self>) {
        let fit = self.effective_fit();
        // while the Screen fit page is tuning, the live picture is what the user is looking at: never idle
        let idle = !self.tuning() && !self.previewing() && !self.twin_temp.load(SeqCst) && self.theme.lock().unwrap().as_ref().is_some_and(|t| !t.theme.live_layer || t.nothing_to_stream(fit));
        let layer_off = idle;
        let held = self.holds.load(SeqCst) & LAYER != 0;
        if layer_off && !held {
            self.hold(LAYER, Duration::from_secs(6));
        } else if !layer_off && held {
            self.release(LAYER, Duration::from_millis(300));
        }
        self.changed();
    }

    pub fn running(&self) -> bool {
        self.pipeline.lock().unwrap().is_some()
    }

    pub fn pipeline(&self) -> Option<Arc<Pipeline>> {
        self.pipeline.lock().unwrap().clone()
    }

    fn apply_pause(&self) {
        if let Some(p) = self.pipeline() {
            let hold = self.holds.load(SeqCst) != 0;
            if !hold && p.shared.paused.load(SeqCst) {
                self.resumes.fetch_add(1, SeqCst);
                hlog("resume");
            }
            p.shared.paused.store(hold, SeqCst);
        } else if self.holds.load(SeqCst) == 0 && self.off_stopped.swap(false, SeqCst) {
            let cfg = self.last_cfg.lock().unwrap().clone();
            if let Some(c) = cfg {
                self.start_with(c);
            }
        }
    }

    /// Start live from the saved config (no-op when already running).
    pub fn start(&self) {
        let cfg = self.config();
        self.start_with(cfg);
    }

    /// Start live with this config (no-op when already running). Returns quickly; the pipeline connects on its own threads.
    pub fn start_with(&self, cfg: Config) {
        let mut g = self.pipeline.lock().unwrap();
        if g.is_some() {
            self.preview_temp.store(false, SeqCst); // live was switched on during a preview: the stream is live's now
            if self.twin_temp.swap(false, SeqCst) {
                // live was switched on while the editor's twin ran its own stream: the real stream (live's) is what is started now
                drop(g);
                self.stop();
                self.start_with(cfg);
            }
            return;
        }
        self.off_stopped.store(false, SeqCst);
        *self.last_cfg.lock().unwrap() = Some(cfg.clone());
        let sink = if self.no_device.load(SeqCst) || self.twin_temp.load(SeqCst) { Sink::Null } else { Sink::Device };
        hlog(&format!("start (holds: {})", if self.holds.load(SeqCst) == 0 { "none".to_string() } else { reason(self.holds.load(SeqCst)) }));
        let p = Arc::new(Pipeline::start(&cfg, sink));
        if self.twin_temp.load(SeqCst) {
            p.lock_rate(15);
        }
        if let Some(t) = self.preview.lock().unwrap().clone().or_else(|| self.theme.lock().unwrap().clone()) {
            p.set_theme(t);
        }
        if let Some(g) = self.calibration.lock().unwrap().clone() {
            p.set_calibration(Some(g));
        }
        p.set_fit(self.effective_fit());
        // Started while a hold is set (a device task is running): stay paused until it is done.
        p.shared.paused.store(self.holds.load(SeqCst) != 0, SeqCst);
        *g = Some(p);
    }

    /// Stop live and end the stream properly (the screen plays its loop clip again). Blocks until the threads are done.
    pub fn stop(&self) {
        self.off_stopped.store(false, SeqCst);
        self.preview_forget();
        self.twin_temp.store(false, SeqCst);
        let p = self.pipeline.lock().unwrap().take();
        if let Some(p) = p {
            hlog("stop (live switched off or the app is closing)");
            p.stop();
        }
    }

    /// Shutdown or logoff: the fast ending (at most 800 ms), loop clip or screen off.
    pub fn stop_fast(&self, screen_off: bool) {
        self.off_stopped.store(false, SeqCst);
        self.preview_forget();
        self.twin_temp.store(false, SeqCst);
        self.end_calibration_now();
        let p = self.pipeline.lock().unwrap().take();
        if let Some(p) = p {
            p.stop_fast(if screen_off { Ending::ScreenOff } else { Ending::LoopClip });
        }
    }

    /// Apply a new config while running (fps, seconds, background, music, ...) without ending the stream.
    pub fn reload(&self, cfg: &Config) {
        *self.last_cfg.lock().unwrap() = Some(cfg.clone());
        if let Some(p) = self.pipeline() {
            p.reload_config(cfg);
        }
    }

    /// The notch or the default corner radius changed: the running picture follows at once.
    pub fn mask_changed(&self) {
        if let Some(p) = self.pipeline() {
            p.mask_changed();
        }
    }

    pub fn set_user_paused(&self, paused: bool) {
        if paused {
            if self.holds.load(SeqCst) == 0 {
                if let Some(p) = self.pipeline() {
                    p.pause_with_handover(); // blocks up to ~1.3 s: call from a worker thread
                }
            }
            self.add_hold(USER);
        } else {
            if self.holds.fetch_and(!USER, SeqCst) & USER != 0 {
                hlog("release: user pause");
            }
        }
        self.apply_pause();
    }

    pub fn user_paused(&self) -> bool {
        self.holds.load(SeqCst) & USER != 0
    }

    /// 30 or 60 (saved in companion.json, like the separate companion).
    pub fn set_fps(&self, fps: u32) {
        let fps = if fps >= 45 { 60 } else { 30 };
        if let Some(p) = self.pipeline() {
            p.set_fps(fps);
        }
        let mut c = Config::load();
        c.fps = fps;
        c.save();
        if let Some(o) = self.cfg_override.lock().unwrap().as_mut() {
            o.fps = fps;
        }
    }

    pub fn fps(&self) -> u32 {
        match self.pipeline() {
            Some(p) => p.shared.fps.load(SeqCst),
            None => self.quiet_fps(),
        }
    }

    /// Wait (at most `max`) until no session is open on the screen. A connect in progress is not open yet, so the state
    /// must stay closed for a moment.
    fn wait_stream_ended(&self, p: &Pipeline, max: Duration) {
        if self.no_device.load(SeqCst) {
            return;
        }
        let end = Instant::now() + max;
        let mut closed_since: Option<Instant> = None;
        while Instant::now() < end {
            if p.session_active() {
                closed_since = None;
            } else if closed_since.get_or_insert_with(Instant::now).elapsed() >= Duration::from_millis(250) {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Like `hold`, but ends the stream with the screen off (sleep or lock set to turn the screen off). Live is started again
    /// when all holds are released.
    pub fn hold_screen_off(&self, bit: u32) {
        self.resume_gen.fetch_add(1, SeqCst);
        self.add_hold(bit);
        let p = self.pipeline.lock().unwrap().take();
        if let Some(p) = p {
            hlog("stop with the screen off (sleep or lock)");
            self.off_stopped.store(true, SeqCst);
            p.stop_with(Ending::ScreenOff);
        }
    }

    /// Pause live for a reason (DEVICE, SLEEP, LOCK) and wait until the stream has ended and the screen is free.
    pub fn hold(&self, bit: u32, max: Duration) {
        self.resume_gen.fetch_add(1, SeqCst);
        self.add_hold(bit);
        if let Some(p) = self.pipeline() {
            p.shared.paused.store(true, SeqCst);
            self.wait_stream_ended(&p, max);
        }
    }

    /// End a hold; live resumes after `delay` unless another reason still holds it.
    pub fn release(self: &Arc<Self>, bit: u32, delay: Duration) {
        let was = self.holds.fetch_and(!bit, SeqCst);
        if was & bit != 0 {
            hlog(&format!("release: {} (still held: {}); resume in {} ms", reason(bit), if was & !bit == 0 { "nothing".to_string() } else { reason(was & !bit) }, delay.as_millis()));
        }
        let gen = self.resume_gen.fetch_add(1, SeqCst) + 1;
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            if me.resume_gen.load(SeqCst) == gen {
                me.apply_pause();
            }
        });
    }

    /// From the scheduler: a device task is queued or running (true) or none is any more (false).
    /// `true` blocks until the stream has ended (so the task finds the screen free); `false` resumes after a short delay.
    pub fn device_busy(self: &Arc<Self>, busy: bool) {
        if busy {
            self.calibrate_end(); // the screen is needed by the task
            self.hold(DEVICE, Duration::from_secs(6));
        } else {
            self.release(DEVICE, Duration::from_millis(1200));
        }
    }

    /// For the UI and the tray.
    pub fn status(&self) -> Value {
        let (running, connected, fps, measured) = match self.pipeline() {
            Some(p) => (true, p.shared.connected.load(SeqCst), p.shared.fps.load(SeqCst), p.shared.stats.sent_fps_x10.load(SeqCst) as f64 / 10.0),
            None => (false, false, self.quiet_fps(), 0.0),
        };
        let h = self.holds.load(SeqCst);
        json!({
            "running": running,
            "connected": connected,
            "fps": fps,
            "previewing": self.previewing(),
            "twinning": self.twinning(),
            "measuredFps": measured,
            "paused": h & USER != 0,
            "deviceHold": h & DEVICE != 0,
            "systemHold": h & (SLEEP | LOCK) != 0,
            "layerOff": self.theme.lock().unwrap().as_ref().is_some_and(|t| !t.theme.live_layer),
            "idle": h & LAYER != 0,
            "liveItems": self.theme.lock().unwrap().as_ref().is_none_or(|t| t.has_live_items()),
            "calibrating": self.calibrating(),
            "tuning": self.tuning(),
            "noDevice": self.no_device.load(SeqCst),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wait (up to 10 s) for a state the host reaches on its own threads; other tests share the CPU, so a fixed sleep is not
    /// a reliable bound. The assertion after it still checks the state.
    fn eventually(mut f: impl FnMut() -> bool) {
        let t0 = std::time::Instant::now();
        while !f() && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn holds_pause_live_and_it_resumes_when_all_are_released() {
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.start();
        let p = h.pipeline().expect("running");
        assert!(!p.shared.paused.load(SeqCst));
        h.device_busy(true);
        assert!(p.shared.paused.load(SeqCst));
        h.device_busy(false);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(!p.shared.paused.load(SeqCst), "live resumes after the device task");
        // a user pause survives a device task
        h.set_user_paused(true);
        h.device_busy(true);
        h.device_busy(false);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(p.shared.paused.load(SeqCst));
        // sleep and device holds are independent: releasing one keeps the other
        h.set_user_paused(false);
        h.hold(SLEEP, Duration::from_secs(1));
        h.device_busy(true);
        h.device_busy(false);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(p.shared.paused.load(SeqCst), "still asleep");
        h.release(SLEEP, Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(500));
        assert!(!p.shared.paused.load(SeqCst));
        // a new device task during the resume delay cancels the resume
        h.device_busy(true);
        h.device_busy(false);
        h.device_busy(true);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(p.shared.paused.load(SeqCst));
        h.stop();
        assert!(!h.running());
    }

    #[test]
    fn live_layer_off_stops_streaming_and_on_resumes() {
        use crate::live::theme::{Theme, ThemeLive};
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.start();
        let p = h.pipeline().expect("running");
        let theme = |layer: bool| ThemeLive::new(Theme { live_layer: layer, ..Theme::default() }, std::path::PathBuf::new());
        h.set_theme(theme(true));
        assert!(!p.shared.paused.load(SeqCst));
        h.set_theme(theme(false));
        assert!(p.shared.paused.load(SeqCst), "no frames while the live layer is off");
        assert_eq!(h.status()["layerOff"], json!(true));
        // a device task coming and going does not bring it back
        h.device_busy(true);
        h.device_busy(false);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(p.shared.paused.load(SeqCst));
        h.set_theme(theme(true));
        eventually(|| !p.shared.paused.load(SeqCst));
        assert!(!p.shared.paused.load(SeqCst), "turning it on again resumes");
        assert_eq!(h.status()["layerOff"], json!(false));
        // live started while the layer is off stays quiet
        h.stop();
        h.set_theme(theme(false));
        h.start();
        assert!(h.pipeline().unwrap().shared.paused.load(SeqCst));
        h.stop();
    }

    #[test]
    fn fit_calibration_pauses_live_and_hands_back_and_works_with_live_off() {
        use crate::core::fit::Fit;
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        // live off: a temporary session for the page
        assert!(!h.calibrating());
        h.calibrate_start(Fit::new(4, 0, 0, 9), 0);
        assert!(h.calibrating() && h.calibration_connected());
        assert_eq!(h.status()["calibrating"], json!(true));
        assert!(h.calibrate_set(Fit::new(5, 0, 0, 9), 3));
        h.calibrate_end();
        assert!(!h.calibrating());
        assert!(!h.calibrate_set(Fit::default(), 0), "nothing to update once it ended");
        // live on: it steps aside, then resumes
        h.start();
        let p = h.pipeline().expect("running");
        h.calibrate_start(Fit::new(4, 0, 0, 9), 1);
        assert!(p.shared.paused.load(SeqCst), "live is paused while the ruler is on the screen");
        h.calibrate_start(Fit::new(6, 0, 0, 9), 1); // a second start only updates the picture
        h.calibrate_end();
        std::thread::sleep(Duration::from_millis(1300));
        assert!(!p.shared.paused.load(SeqCst), "live is back after the page is left");
        // a device task ends the calibration (the screen is needed)
        h.calibrate_start(Fit::default(), 0);
        h.device_busy(true);
        assert!(!h.calibrating());
        h.device_busy(false);
        h.stop();
    }

    #[test]
    fn tuning_on_my_theme_applies_each_change_at_once_with_live_off_and_on() {
        use crate::core::fit::Fit;
        use crate::live::theme::{Theme, ThemeLive};
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_fit(Fit::new(1, 1, 1, 1)); // the saved fit
        // live off: a temporary pipeline for the page
        assert!(!h.running());
        h.tune_start(Fit::new(4, 0, 0, 9));
        let p = h.pipeline().expect("a temporary live session");
        assert_eq!(*p.shared.fit.lock().unwrap(), Fit::new(4, 0, 0, 9), "the unsaved fit, not the saved one");
        assert!(h.tune_set(Fit::new(5, 0, 0, 9).with_shift(3, -2)));
        assert_eq!(*p.shared.fit.lock().unwrap(), Fit::new(5, 0, 0, 9).with_shift(3, -2), "applied at once, no settings re-read");
        assert_eq!(h.status()["tuning"], json!(true));
        eventually(|| p.shared.stats.snap().rendered > 0);
        assert!(p.shared.stats.snap().rendered > 0);
        h.tune_end();
        assert!(!h.running() && !h.tuning(), "the temporary session ends with the page");
        assert!(!h.tune_set(Fit::default()), "nothing to update once it ended");
        assert_eq!(h.effective_fit(), Fit::new(1, 1, 1, 1), "back to the saved fit");
        // live on with a theme that has nothing live: normally quiet, but streaming while the page tunes
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("device.mp4");
        std::fs::write(&base, b"x").unwrap();
        let mut t = ThemeLive::new(Theme::default(), std::path::PathBuf::new());
        t.base_video = Some(base);
        t.base_fit = Some(Fit::new(1, 1, 1, 1));
        h.set_theme(t);
        h.start();
        let p = h.pipeline().unwrap();
        assert!(p.shared.paused.load(SeqCst), "idle: the base already has the saved fit and nothing is live");
        h.tune_start(Fit::new(4, 0, 0, 9));
        eventually(|| !p.shared.paused.load(SeqCst));
        assert!(!p.shared.paused.load(SeqCst), "tuning streams so the user sees the effect");
        assert_eq!(*p.shared.fit.lock().unwrap(), Fit::new(4, 0, 0, 9));
        h.tune_end();
        std::thread::sleep(Duration::from_millis(500));
        assert!(h.running(), "live was on before: it stays on");
        assert_eq!(*p.shared.fit.lock().unwrap(), Fit::new(1, 1, 1, 1));
        assert!(p.shared.paused.load(SeqCst), "and quiet again");
        h.stop();
    }

    #[test]
    fn the_fit_reaches_a_running_and_a_later_pipeline() {
        use crate::core::fit::Fit;
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_fit(Fit::new(4, 0, 0, 9));
        h.start();
        let p = h.pipeline().unwrap();
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(*p.shared.fit.lock().unwrap(), Fit::new(4, 0, 0, 9));
        h.set_fit(Fit::new(1, 2, 3, 4));
        assert_eq!(*p.shared.fit.lock().unwrap(), Fit::new(1, 2, 3, 4));
        h.stop();
    }

    /// What the sink counters say: pause really stops the frames, resume brings them back, and the fps setting changes the rate.
    #[test]
    fn pause_stops_the_frames_and_fps_changes_the_rate() {
        let h = LiveHost::new();
        h.set_no_device(true);
        let mut c = Config::default();
        c.fps = 60;
        h.set_config_override(Some(c));
        h.start();
        let p = h.pipeline().expect("running");
        // an animated background (the gradient) so every tick is a new picture
        let sent = |p: &Arc<Pipeline>| p.shared.stats.snap().sent;
        let rate = |p: &Arc<Pipeline>, ms: u64| {
            let a = p.shared.stats.snap();
            std::thread::sleep(Duration::from_millis(ms));
            let b = p.shared.stats.snap();
            (b.rendered - a.rendered, b.sent - a.sent)
        };
        std::thread::sleep(Duration::from_millis(500));
        // (a debug build next to a busy test run renders slowly: wait for frames instead of counting how many)
        let (mut r60, mut s60) = rate(&p, 2000);
        for _ in 0..5 {
            if s60 >= 1 && r60 >= 12 {
                break;
            }
            let (r, s2) = rate(&p, 2000);
            r60 = r;
            s60 = s2;
        }
        assert!(s60 >= 1, "frames flow while live runs: {s60}");
        // 30 fps: the render loop (it paces on the fps setting) runs at about half the rate. Other tests share the CPU, so a
        // starved window can distort one measurement: the ratio is measured again (60, then 30) up to four times and must hold once.
        let mut last = (r60, 0u64);
        let mut follows = false;
        for _ in 0..4 {
            h.set_fps(60);
            std::thread::sleep(Duration::from_millis(300));
            let (r60, _) = rate(&p, 2000);
            h.set_fps(30);
            std::thread::sleep(Duration::from_millis(300));
            let (r30, _) = rate(&p, 2000);
            last = (r60, r30);
            // the ratio only means something when the machine kept up at 60 (not starved)
            if r60 >= 40 && r30 as f64 <= r60 as f64 * 0.85 && r30 as f64 >= r60 as f64 * 0.3 {
                follows = true;
                break;
            }
        }
        assert!(follows, "render rate follows fps (4 tries; 60 -> 30 in 2 s each), last: {last:?}");
        assert_eq!(h.fps(), 30);
        h.set_fps(60);
        // pause: nothing is rendered or sent any more
        h.set_user_paused(true);
        std::thread::sleep(Duration::from_millis(800));
        let before = p.shared.stats.snap();
        std::thread::sleep(Duration::from_millis(1000));
        let after = p.shared.stats.snap();
        assert_eq!((after.rendered, after.sent), (before.rendered, before.sent), "no frames while paused");
        assert!(!p.session_active() && h.status()["paused"] == json!(true));
        // resume
        h.set_user_paused(false);
        std::thread::sleep(Duration::from_millis(800));
        assert!(sent(&p) > after.sent, "frames flow again after Resume");
        h.stop();
    }

    #[test]
    fn a_themed_pause_hands_over_before_it_stops_and_resume_starts_again() {
        use crate::live::theme::{Theme, ThemeLive};
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_theme(ThemeLive::new(Theme::standard_preset(), std::path::PathBuf::new()));
        h.start();
        let p = h.pipeline().unwrap();
        std::thread::sleep(Duration::from_millis(1200));
        let t = Instant::now();
        h.set_user_paused(true);
        let took = t.elapsed();
        assert!(took >= Duration::from_millis(300) && took < Duration::from_millis(1600), "the fade-out takes its time: {took:?}");
        assert!(p.shared.paused.load(SeqCst));
        assert!(!p.shared.hand_done.load(SeqCst) && p.shared.stop_mode.load(SeqCst) == 0, "the hand-over state is cleared for the next start");
        std::thread::sleep(Duration::from_millis(500));
        let n = p.shared.stats.snap().sent;
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(p.shared.stats.snap().sent, n, "nothing is sent while paused");
        h.set_user_paused(false);
        std::thread::sleep(Duration::from_millis(1000));
        assert!(p.shared.stats.snap().sent > n, "Resume starts the frames again");
        assert!(!p.shared.hand_done.load(SeqCst), "and the render loop is not stuck in the old hand-over");
        h.stop();
    }

    #[test]
    fn a_theme_with_nothing_live_does_not_stream_unless_the_base_needs_the_fit() {
        use crate::core::fit::Fit;
        use crate::live::theme::{Theme, ThemeLive, Widget};
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("device.mp4");
        std::fs::write(&base, b"x").unwrap();
        let mut t = ThemeLive::new(Theme::default(), std::path::PathBuf::new());
        t.base_video = Some(base);
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.start();
        let p = h.pipeline().unwrap();
        h.set_theme(t.clone());
        assert_eq!(h.status()["liveItems"], json!(false));
        assert!(p.shared.paused.load(SeqCst) && h.status()["idle"] == json!(true), "no live items on a fitted base: the screen plays its own loop");
        // a fit the base was not made with: streaming is what applies it
        h.set_fit(Fit::new(4, 0, 0, 9));
        std::thread::sleep(Duration::from_millis(700));
        assert!(!p.shared.paused.load(SeqCst), "an unfitted base keeps streaming");
        // the base made with that fit: quiet again
        t.base_fit = Some(Fit::new(4, 0, 0, 9));
        h.set_theme(t.clone());
        assert!(p.shared.paused.load(SeqCst));
        // an item brings it back
        t.theme.widgets.push(Widget { id: "t".into(), kind: "text".into(), props: json!({ "text": "Hi" }), ..Widget::default() });
        h.set_theme(t.clone());
        assert_eq!(h.status()["liveItems"], json!(true));
        std::thread::sleep(Duration::from_millis(700));
        assert!(!p.shared.paused.load(SeqCst));
        // an item that is only part of the stored base is not a live item
        t.theme.widgets[0].on_base = true;
        h.set_theme(t);
        assert_eq!(h.status()["liveItems"], json!(false));
        assert!(p.shared.paused.load(SeqCst));
        h.stop();
    }

    fn marked(n: usize) -> crate::live::theme::ThemeLive {
        use crate::live::theme::{Theme, ThemeLive, Widget};
        let mut t = Theme::default();
        t.widgets = (0..n).map(|i| Widget::new(&format!("w{i}"), "clock", 0.1, 0.1, 0.3, 0.3, json!({}))).collect();
        ThemeLive::new(t, std::path::PathBuf::new())
    }
    fn shown(p: &Arc<Pipeline>) -> usize {
        p.shared.theme.lock().unwrap().as_ref().map(|t| t.theme.widgets.len()).unwrap_or(99)
    }

    /// Preview with Live off: a temporary stream that follows the clicks without reconnecting, and ends when asked.
    #[test]
    fn preview_with_live_off_starts_a_temporary_stream_swaps_in_place_and_ends() {
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_theme(marked(1)); // the live theme (not shown)
        assert!(!h.running() && !h.previewing());
        h.preview_start(marked(2));
        let p = h.pipeline().expect("a temporary stream");
        assert!(h.previewing() && h.status()["previewing"] == json!(true));
        assert_eq!(shown(&p), 2);
        // another click: same stream, other picture
        h.preview_start(marked(3));
        assert!(Arc::ptr_eq(&p, &h.pipeline().unwrap()), "no reconnect per click");
        assert_eq!(shown(&p), 3);
        // the live theme changing meanwhile (a file refresh) must not replace the preview
        h.set_theme(marked(4));
        assert_eq!(shown(&p), 3);
        eventually(|| p.shared.stats.snap().rendered > 0);
        assert!(p.shared.stats.snap().rendered > 0, "frames flow");
        assert!(h.preview_keepalive());
        assert!(h.preview_end());
        assert!(!h.running() && !h.previewing(), "the temporary stream is gone");
        assert!(!h.preview_end(), "ending twice does nothing");
        assert!(!h.preview_keepalive());
    }

    /// Preview with Live on: the running stream shows it and gets its own theme back afterwards.
    #[test]
    fn preview_with_live_on_restores_the_live_theme() {
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_theme(marked(1));
        h.start();
        let p = h.pipeline().unwrap();
        assert_eq!(shown(&p), 1);
        h.preview_start(marked(2));
        assert!(Arc::ptr_eq(&p, &h.pipeline().unwrap()));
        assert_eq!(shown(&p), 2);
        assert!(h.preview_end());
        assert!(h.running() && Arc::ptr_eq(&p, &h.pipeline().unwrap()), "live keeps running");
        assert_eq!(shown(&p), 1, "back to the live theme");
        h.stop();
    }

    /// A device task (upload, check) pauses a preview like it pauses live, and it carries on afterwards.
    #[test]
    fn a_device_task_pauses_a_preview_like_live() {
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.preview_start(marked(2));
        let p = h.pipeline().unwrap();
        eventually(|| !p.shared.paused.load(SeqCst));
        h.device_busy(true);
        assert!(p.shared.paused.load(SeqCst), "yields the screen to the task");
        h.device_busy(false);
        eventually(|| !p.shared.paused.load(SeqCst));
        assert!(!p.shared.paused.load(SeqCst));
        h.preview_end();
        assert!(!h.running());
    }

    /// Nobody asks for it any more (window closed, page gone): it ends by itself; quitting forgets it too.
    #[test]
    fn a_forgotten_preview_ends_by_itself_and_stop_forgets_it() {
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_preview_idle(Duration::from_millis(1200));
        h.preview_start(marked(2));
        assert!(h.previewing());
        eventually(|| !h.previewing());
        assert!(!h.previewing() && !h.running(), "ended by the safety timer");
        h.preview_start(marked(2));
        h.stop();
        assert!(!h.previewing() && !h.running());
    }

    /// The editor's twin with Live off: a stream for the editor only (15 per second, nothing sent), frames available while the
    /// page asks, and it ends when the editor closes.
    #[test]
    fn the_twin_with_live_off_renders_for_the_editor_only_and_ends() {
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_theme(marked(1));
        assert!(!h.running());
        h.twin_start();
        let p = h.pipeline().expect("a stream for the editor");
        assert!(h.twinning() && h.status()["twinning"] == json!(true));
        assert_eq!(p.shared.fps.load(SeqCst), 15);
        // frames appear while the page keeps asking (touch stands for the page's requests)
        let mut first = None;
        eventually(|| {
            crate::live::twin::touch();
            first = crate::live::twin::latest(0);
            first.is_some()
        });
        let (s0, jpeg) = first.expect("a frame");
        assert!(jpeg.starts_with(&[0xFF, 0xD8]), "a JPEG");
        let t0 = std::time::Instant::now();
        let mut last = s0;
        while t0.elapsed() < Duration::from_millis(2000) {
            crate::live::twin::touch();
            if let Some((s, _)) = crate::live::twin::latest(last) {
                last = s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let n = last - s0;
        assert!((8..=40).contains(&n), "about 15 frames per second, got {n} in 2 s");
        // a settings change keeps the twin's rate
        h.set_fps(60);
        assert_eq!(p.shared.fps.load(SeqCst), 15);
        h.twin_stop();
        assert!(!h.running() && !h.twinning(), "the editor-only stream is gone");
    }

    /// With Live on the editor gets the frames the engine sends (no second stream), and closing the editor leaves live alone.
    #[test]
    fn the_twin_with_live_on_copies_the_sent_frames_and_leaves_live_running() {
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_theme(marked(1));
        h.start();
        let p = h.pipeline().unwrap();
        h.twin_start();
        assert!(Arc::ptr_eq(&p, &h.pipeline().unwrap()) && !h.twinning(), "no second stream");
        eventually(|| {
            crate::live::twin::touch();
            crate::live::twin::latest(0).is_some()
        });
        assert!(crate::live::twin::latest(0).is_some());
        h.twin_stop();
        assert!(h.running(), "live keeps running");
        h.stop();
    }

    /// Measuring aid (run by hand in a release build): CPU used by the editor-only twin stream, as a share of one core and of the PC.
    /// cargo test --release --lib -- twin_cost --ignored --nocapture
    #[test]
    #[ignore = "measuring aid"]
    fn twin_cost() {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let cpu = || unsafe {
            let (mut c, mut e, mut k, mut u) = (FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 }, FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 });
            GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
            let f = |t: FILETIME| (((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64) as f64 / 1e7;
            f(k) + f(u)
        };
        let h = LiveHost::new();
        h.set_no_device(true);
        h.set_config_override(Some(Config::default()));
        h.set_theme(crate::live::theme::ThemeLive::new(crate::live::theme::Theme::standard_preset(), std::path::PathBuf::new()));
        h.twin_start();
        std::thread::sleep(Duration::from_millis(1500));
        let p = h.pipeline().unwrap();
        let (a, c0) = (p.shared.stats.snap(), cpu());
        let t0 = std::time::Instant::now();
        while t0.elapsed() < Duration::from_secs(8) {
            crate::live::twin::touch();
            std::thread::sleep(Duration::from_millis(100));
        }
        let (b, c1) = (p.shared.stats.snap(), cpu());
        let secs = t0.elapsed().as_secs_f64();
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
        let frames = (b.encoded - a.encoded) as f64;
        println!("twin: {:.1} frames/s encoded, render {:.1} ms + encode {:.1} ms per frame, process CPU {:.1}% of one core = {:.2}% of the PC ({} logical cores)",
            frames / secs, (b.render_us - a.render_us) as f64 / (b.rendered - a.rendered).max(1) as f64 / 1000.0, (b.encode_us - a.encode_us) as f64 / frames.max(1.0) / 1000.0,
            (c1 - c0) / secs * 100.0, (c1 - c0) / secs * 100.0 / cores, cores);
        h.twin_stop();
    }
}
