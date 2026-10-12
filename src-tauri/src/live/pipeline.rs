//! The companion's engine: three threads joined by "latest frame" slots, so drawing never waits for USB.
//!   render (paced at the fps setting) -> slot -> encode -> slot -> send (owns the screen link)
//! A slot holds only the newest item: when a stage is slower, older frames are dropped instead of queuing up (a queue would
//! show stale clocks). Buffers are recycled through pools so a 60 fps run does not churn the allocator.
//! The sender connects with backoff, reconnects when the screen disappears, and ends the stream properly (loop clip keeps
//! playing) when paused or stopped. While there is no connection nothing is rendered.
use super::encode::{frame_budget, rotate_cw, JpegEncoder};
use super::render::Renderer;
use super::sensors::Sampler;
use super::session::LiveSession;
use super::video::VideoBg;
use super::{Config, PH, PW};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
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
    pub stats: Stats,
}

pub struct Pipeline {
    pub shared: Arc<Shared>,
    threads: Mutex<Vec<std::thread::JoinHandle<()>>>,
    /// Raw process handle of the background-video ffmpeg, for measuring.
    pub video_child: Arc<Mutex<Option<usize>>>,
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
            stats: Stats::default(),
        });
        let rgba_slot: Arc<Latest<Vec<u8>>> = Arc::new(Latest::new());
        let jpeg_slot: Arc<Latest<Vec<u8>>> = Arc::new(Latest::new());
        let rgba_pool: Arc<Pool<Vec<u8>>> = Arc::new(Pool(Mutex::new(Vec::new())));
        let jpeg_pool: Arc<Pool<Vec<u8>>> = Arc::new(Pool(Mutex::new(Vec::new())));
        let video_child = Arc::new(Mutex::new(None));
        let mut threads = Vec::new();

        // render
        {
            let (sh, slot, pool) = (shared.clone(), rgba_slot.clone(), rgba_pool.clone());
            let (bg, vc) = (cfg.background.clone(), video_child.clone());
            let fps0 = sh.fps.load(Relaxed);
            threads.push(std::thread::spawn(move || {
                let sampler = Sampler::start();
                let video = bg.as_deref().filter(|b| !b.is_empty()).and_then(|b| match VideoBg::start(std::path::Path::new(b), fps0) {
                    Ok(v) => Some(v),
                    Err(e) => {
                        log(&format!("background clip not used: {e}"));
                        None
                    }
                });
                if let Some(v) = &video {
                    *vc.lock().unwrap() = Some(0);
                    let v_handle = v.child_handle.clone();
                    // The child handle appears once ffmpeg has started; mirror it for the measurer.
                    let vc2 = vc.clone();
                    std::thread::spawn(move || {
                        for _ in 0..50 {
                            std::thread::sleep(Duration::from_millis(100));
                            if let Some(h) = *v_handle.lock().unwrap() {
                                *vc2.lock().unwrap() = Some(h);
                                break;
                            }
                        }
                    });
                }
                let mut r = Renderer::new(sampler.data.clone(), video);
                if !r.has_fonts() {
                    log("no system font found (Segoe UI / Arial): text is not drawn");
                }
                let mut next = Instant::now();
                let mut last_emit = Instant::now();
                while !sh.quit.load(Relaxed) {
                    if sh.paused.load(Relaxed) || !sh.connected.load(Relaxed) {
                        std::thread::sleep(Duration::from_millis(100));
                        next = Instant::now();
                        continue;
                    }
                    let period = Duration::from_secs_f64(1.0 / sh.fps.load(Relaxed).max(1) as f64);
                    let t = Instant::now();
                    let show_seconds = sh.show_seconds.load(Relaxed);
                    let (frame, changed) = r.render(show_seconds);
                    let t1 = Instant::now();
                    // Nothing changed since the last frame (the renderer says so without redrawing): skip rotate, encode and send. The screen keeps the last picture
                    // (keepalives continue), and one frame per half second is still sent as a safety net.
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
                            s.stop();
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
                    match jin.take(Duration::from_millis(200)) {
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
                    log("stopping: loop clip plays");
                    s.stop();
                }
            }));
        }

        Pipeline { shared, threads: Mutex::new(threads), video_child }
    }

    /// Ask every thread to finish and wait for them; the sender ends the stream (loop clip, display on) first.
    /// Safe to call more than once and from any thread.
    pub fn stop(&self) {
        self.shared.quit.store(true, Relaxed);
        let ts: Vec<_> = self.threads.lock().unwrap().drain(..).collect();
        for t in ts {
            let _ = t.join();
        }
    }
}
