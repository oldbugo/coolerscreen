//! 3.0 prototype B: the live companion engine (`crate::live`) hosted inside the app process.
//! The host owns the pipeline and the rules that keep the screen to one user at a time:
//!   - the user can pause live (tray "Pause");
//!   - while any device task (send, restart, check, setup...) is queued or running, live is paused and the stream is ended
//!     (the screen plays its loop clip), and live resumes about a second after the last device task finished.
//! Nothing here touches the screen except through `Pipeline` / `LiveSession` (0x15 [00 0A], 0x11 and frames; stop sends
//! FF D9 FF D9, 0x0E [1], 0x14 [1,0]).
use crate::live::pipeline::{Pipeline, Sink};
use crate::live::Config;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub struct LiveHost {
    pipeline: Mutex<Option<Arc<Pipeline>>>,
    /// Frames are discarded (measuring or testing without the screen).
    no_device: AtomicBool,
    user_paused: AtomicBool,
    /// A device task is queued or running.
    device_hold: AtomicBool,
    /// Bumped on every device_busy(false) so only the newest delayed resume acts.
    resume_gen: AtomicU64,
    /// Overrides for the saved config (tests, measuring): fps and background clip.
    cfg_override: Mutex<Option<Config>>,
}

impl Default for LiveHost {
    fn default() -> Self {
        LiveHost {
            pipeline: Mutex::new(None),
            no_device: AtomicBool::new(false),
            user_paused: AtomicBool::new(false),
            device_hold: AtomicBool::new(false),
            resume_gen: AtomicU64::new(0),
            cfg_override: Mutex::new(None),
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
            p.shared.paused.store(self.user_paused.load(SeqCst) || self.device_hold.load(SeqCst), SeqCst);
        }
    }

    /// Start live (no-op when already running). Returns quickly; the pipeline connects on its own threads.
    pub fn start(&self) {
        let mut g = self.pipeline.lock().unwrap();
        if g.is_some() {
            return;
        }
        let cfg = self.config();
        let sink = if self.no_device.load(SeqCst) { Sink::Null } else { Sink::Device };
        let p = Arc::new(Pipeline::start(&cfg, sink));
        // Started while a device task is already running: stay paused until it is done.
        p.shared.paused.store(self.user_paused.load(SeqCst) || self.device_hold.load(SeqCst), SeqCst);
        *g = Some(p);
    }

    /// Stop live and end the stream properly (the screen plays its loop clip again). Blocks until the threads are done.
    pub fn stop(&self) {
        let p = self.pipeline.lock().unwrap().take();
        if let Some(p) = p {
            p.stop();
        }
    }

    pub fn set_user_paused(&self, paused: bool) {
        self.user_paused.store(paused, SeqCst);
        self.apply_pause();
    }

    pub fn user_paused(&self) -> bool {
        self.user_paused.load(SeqCst)
    }

    /// 30 or 60 (saved in companion.json, like the separate companion).
    pub fn set_fps(&self, fps: u32) {
        let fps = if fps >= 45 { 60 } else { 30 };
        if let Some(p) = self.pipeline() {
            p.shared.fps.store(fps, SeqCst);
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

    /// From the scheduler: a device task is queued or running (true) or none is any more (false).
    /// `true` blocks until the stream has ended (so the task finds the screen free); `false` resumes after a short delay.
    pub fn device_busy(self: &Arc<Self>, busy: bool) {
        let Some(p) = self.pipeline() else {
            self.device_hold.store(busy, SeqCst);
            return;
        };
        if busy {
            self.resume_gen.fetch_add(1, SeqCst);
            self.device_hold.store(true, SeqCst);
            p.shared.paused.store(true, SeqCst);
            if !self.no_device.load(SeqCst) {
                // The sender ends the stream (about 0.7 s) and then clears `connected`. A connect in progress also reads
                // "not connected", so require it to stay false for a while.
                let end = Instant::now() + Duration::from_secs(6);
                let mut stable_since: Option<Instant> = None;
                while Instant::now() < end {
                    if p.shared.connected.load(SeqCst) {
                        stable_since = None;
                    } else if stable_since.get_or_insert_with(Instant::now).elapsed() >= Duration::from_millis(600) {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
        } else {
            self.device_hold.store(false, SeqCst);
            let gen = self.resume_gen.fetch_add(1, SeqCst) + 1;
            let me = self.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(1200));
                if me.resume_gen.load(SeqCst) == gen && !me.device_hold.load(SeqCst) {
                    me.apply_pause();
                }
            });
        }
    }

    /// For the UI and the tray.
    pub fn status(&self) -> Value {
        let (running, connected, fps) = match self.pipeline() {
            Some(p) => (true, p.shared.connected.load(SeqCst), p.shared.fps.load(SeqCst)),
            None => (false, false, self.quiet_fps()),
        };
        json!({
            "running": running,
            "connected": connected,
            "fps": fps,
            "paused": self.user_paused(),
            "deviceHold": self.device_hold.load(SeqCst),
            "noDevice": self.no_device.load(SeqCst),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    #[test]
    fn device_tasks_pause_live_and_it_resumes_unless_the_user_paused() {
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
        // a new device task during the resume delay cancels the resume
        h.set_user_paused(false);
        h.device_busy(true);
        h.device_busy(false);
        h.device_busy(true);
        std::thread::sleep(Duration::from_millis(1600));
        assert!(p.shared.paused.load(SeqCst));
        h.stop();
        assert!(!h.running());
    }
}
