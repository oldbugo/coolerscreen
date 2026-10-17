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
        if let Some(p) = self.pipeline() {
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
        let idle = !self.tuning() && self.theme.lock().unwrap().as_ref().is_some_and(|t| !t.theme.live_layer || t.nothing_to_stream(fit));
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
            return;
        }
        self.off_stopped.store(false, SeqCst);
        *self.last_cfg.lock().unwrap() = Some(cfg.clone());
        let sink = if self.no_device.load(SeqCst) { Sink::Null } else { Sink::Device };
        hlog(&format!("start (holds: {})", if self.holds.load(SeqCst) == 0 { "none".to_string() } else { reason(self.holds.load(SeqCst)) }));
        let p = Arc::new(Pipeline::start(&cfg, sink));
        if let Some(t) = self.theme.lock().unwrap().clone() {
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
        let p = self.pipeline.lock().unwrap().take();
        if let Some(p) = p {
            hlog("stop (live switched off or the app is closing)");
            p.stop();
        }
    }

    /// Shutdown or logoff: the fast ending (at most 800 ms), loop clip or screen off.
    pub fn stop_fast(&self, screen_off: bool) {
        self.off_stopped.store(false, SeqCst);
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
        let (running, connected, fps) = match self.pipeline() {
            Some(p) => (true, p.shared.connected.load(SeqCst), p.shared.fps.load(SeqCst)),
            None => (false, false, self.quiet_fps()),
        };
        let h = self.holds.load(SeqCst);
        json!({
            "running": running,
            "connected": connected,
            "fps": fps,
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
        std::thread::sleep(Duration::from_millis(700));
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
        std::thread::sleep(Duration::from_millis(300));
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
        std::thread::sleep(Duration::from_millis(700));
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
        let (r60, s60) = rate(&p, 2000);
        assert!(s60 > 5, "frames flow while live runs: {s60}");
        // 30 fps: the render loop (it paces on the fps setting) runs at about half the rate
        h.set_fps(30);
        std::thread::sleep(Duration::from_millis(300));
        let (r30, _) = rate(&p, 2000);
        assert!(r30 as f64 <= r60 as f64 * 0.85 && r30 as f64 >= r60 as f64 * 0.3, "render rate follows fps: 60 -> {r60}, 30 -> {r30} frames in 2 s");
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
}
