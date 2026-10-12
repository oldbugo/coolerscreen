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
        }
    }
}

impl LiveHost {
    pub fn new() -> Arc<LiveHost> {
        Arc::new(LiveHost::default())
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

    pub fn running(&self) -> bool {
        self.pipeline.lock().unwrap().is_some()
    }

    pub fn pipeline(&self) -> Option<Arc<Pipeline>> {
        self.pipeline.lock().unwrap().clone()
    }

    fn apply_pause(&self) {
        if let Some(p) = self.pipeline() {
            p.shared.paused.store(self.holds.load(SeqCst) != 0, SeqCst);
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
        let p = Arc::new(Pipeline::start(&cfg, sink));
        // Started while a hold is set (a device task is running): stay paused until it is done.
        p.shared.paused.store(self.holds.load(SeqCst) != 0, SeqCst);
        *g = Some(p);
    }

    /// Stop live and end the stream properly (the screen plays its loop clip again). Blocks until the threads are done.
    pub fn stop(&self) {
        self.off_stopped.store(false, SeqCst);
        let p = self.pipeline.lock().unwrap().take();
        if let Some(p) = p {
            p.stop();
        }
    }

    /// Shutdown or logoff: the fast ending (at most 800 ms), loop clip or screen off.
    pub fn stop_fast(&self, screen_off: bool) {
        self.off_stopped.store(false, SeqCst);
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
            self.holds.fetch_or(USER, SeqCst);
        } else {
            self.holds.fetch_and(!USER, SeqCst);
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
        self.holds.fetch_or(bit, SeqCst);
        let p = self.pipeline.lock().unwrap().take();
        if let Some(p) = p {
            self.off_stopped.store(true, SeqCst);
            p.stop_with(Ending::ScreenOff);
        }
    }

    /// Pause live for a reason (DEVICE, SLEEP, LOCK) and wait until the stream has ended and the screen is free.
    pub fn hold(&self, bit: u32, max: Duration) {
        self.resume_gen.fetch_add(1, SeqCst);
        self.holds.fetch_or(bit, SeqCst);
        if let Some(p) = self.pipeline() {
            p.shared.paused.store(true, SeqCst);
            self.wait_stream_ended(&p, max);
        }
    }

    /// End a hold; live resumes after `delay` unless another reason still holds it.
    pub fn release(self: &Arc<Self>, bit: u32, delay: Duration) {
        self.holds.fetch_and(!bit, SeqCst);
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
}
