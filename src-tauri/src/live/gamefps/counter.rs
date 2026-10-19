//! Pure frame-time statistics from present events (no Windows calls: unit-testable with synthetic events).

use std::collections::{HashMap, VecDeque};

/// Where a present event came from (a process can emit both a D3D9 and a DXGI event for one frame: count one provider per process).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Dxgi,
    D3d9,
}

impl Src {
    fn idx(self) -> usize {
        match self {
            Src::Dxgi => 0,
            Src::D3d9 => 1,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PidStats {
    /// Frames per second over the last second or two.
    pub fps: f32,
    /// 1 % low (fps equivalent of the slowest 1 % of frames in the last several seconds), None until there are enough frames.
    pub low1: Option<f32>,
    /// The most recent frame time (ms).
    pub frame_ms: f32,
}

const SEC: u64 = 1_000_000_000;
const RING_SPAN: u64 = 8 * SEC;
const RING_CAP: usize = 2000;
const PAUSE_GAP: u64 = 2 * SEC;
const STALE_QUERY: u64 = 2 * SEC;
const DROP_IDLE: u64 = 5 * SEC;
const FPS_WINDOW: u64 = SEC;
const LOW_WINDOW: u64 = 5 * SEC;
const PROVIDER_WINDOW: u64 = SEC;
const CLEANUP_EVERY: u32 = 4096;
const HIST_MAX_POINTS: usize = 200;

/// Milliseconds to nanoseconds (test/helper convenience).
pub fn ns(ms: f64) -> u64 {
    if ms <= 0.0 {
        0
    } else {
        (ms * 1e6) as u64
    }
}

struct PidState {
    times: VecDeque<u64>,
    chosen: Src,
    cnt: [u32; 2],
    win_start: u64,
    last: u64,
}

impl PidState {
    fn new(src: Src, t: u64) -> PidState {
        PidState { times: VecDeque::new(), chosen: src, cnt: [0, 0], win_start: t, last: t }
    }

    fn feed(&mut self, t: u64, src: Src) {
        // Provider bookkeeping with hysteresis, evaluated once per window.
        if t.saturating_sub(self.win_start) >= PROVIDER_WINDOW {
            let mine = self.cnt[self.chosen.idx()];
            let other = self.cnt[1 - self.chosen.idx()];
            if other >= 5 && other as u64 * 2 > mine as u64 * 3 {
                self.chosen = if self.chosen == Src::Dxgi { Src::D3d9 } else { Src::Dxgi };
                self.times.clear();
            }
            self.cnt = [0, 0];
            self.win_start = t;
        }
        self.cnt[src.idx()] = self.cnt[src.idx()].saturating_add(1);
        if src != self.chosen {
            return;
        }
        if let Some(&l) = self.times.back() {
            if t <= l {
                return; // duplicate / out of order
            }
            if t - l > PAUSE_GAP {
                self.times.clear(); // pause: restart the series
            }
        }
        self.times.push_back(t);
        self.last = self.last.max(t);
        let cut = t.saturating_sub(RING_SPAN);
        while self.times.front().map_or(false, |&f| f < cut) || self.times.len() > RING_CAP {
            self.times.pop_front();
        }
    }

    /// Frame times (end timestamp, ms) with end >= since.
    fn frames_since(&self, since: u64) -> impl Iterator<Item = (u64, f64)> + '_ {
        self.times
            .iter()
            .zip(self.times.iter().skip(1))
            .filter(move |(_, &b)| b >= since)
            .map(|(&a, &b)| (b, b.saturating_sub(a) as f64 / 1e6))
    }

    fn stats(&self, now: u64) -> Option<PidStats> {
        let last = *self.times.back()?;
        if now.saturating_sub(last) > STALE_QUERY || self.times.len() < 3 {
            return None;
        }
        let since = last.saturating_sub(FPS_WINDOW);
        let first = *self.times.iter().find(|&&t| t >= since)?;
        let n = self.times.iter().filter(|&&t| t >= since).count();
        if n < 2 || last <= first {
            return None;
        }
        let fps = (n - 1) as f64 / ((last - first) as f64 / 1e9);
        let mut ft: Vec<f64> = self.frames_since(last.saturating_sub(LOW_WINDOW)).map(|(_, ms)| ms).collect();
        let frame_ms = self.frames_since(last).last().map(|x| x.1)?;
        let low1 = if ft.len() < 30 {
            None
        } else {
            ft.sort_by(|a, b| b.partial_cmp(a).unwrap_or(std::cmp::Ordering::Equal));
            let k = if ft.len() < 100 { 1 } else { (ft.len() + 99) / 100 };
            let mean = ft[..k].iter().sum::<f64>() / k as f64;
            if mean > 0.0 { Some((1000.0 / mean) as f32) } else { None }
        };
        Some(PidStats { fps: fps as f32, low1, frame_ms: frame_ms as f32 })
    }
}

#[derive(Default)]
pub struct Counter {
    pids: HashMap<u32, PidState>,
    since_cleanup: u32,
}

impl Counter {
    pub fn new() -> Counter {
        Counter::default()
    }

    /// One present of process `pid` at `t_ns` (nanoseconds on the sampler's clock).
    pub fn feed(&mut self, pid: u32, t_ns: u64, src: Src) {
        self.pids.entry(pid).or_insert_with(|| PidState::new(src, t_ns)).feed(t_ns, src);
        self.since_cleanup += 1;
        if self.since_cleanup >= CLEANUP_EVERY {
            self.since_cleanup = 0;
            self.cleanup(t_ns);
        }
    }

    fn cleanup(&mut self, now: u64) {
        self.pids.retain(|_, s| now.saturating_sub(s.last) <= DROP_IDLE);
    }

    /// Statistics of `pid` as of `now_ns`; None when it has not presented in the last ~2 s.
    pub fn stats(&self, pid: u32, now_ns: u64) -> Option<PidStats> {
        self.pids.get(&pid)?.stats(now_ns)
    }

    /// Processes that presented recently, with their fps (highest first).
    pub fn active(&self, now_ns: u64) -> Vec<(u32, f32)> {
        let mut v: Vec<(u32, f32)> = self
            .pids
            .iter()
            .filter_map(|(&p, s)| s.stats(now_ns).map(|st| (p, st.fps)))
            .filter(|(_, f)| *f >= 1.0)
            .collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(a.0.cmp(&b.0)));
        v
    }

    /// (age in seconds, frame time ms) of `pid` within the last `secs` seconds, oldest first.
    pub fn history(&self, pid: u32, now_ns: u64, secs: f32) -> Vec<(f32, f32)> {
        let Some(s) = self.pids.get(&pid) else { return Vec::new() };
        let secs = if secs.is_finite() { secs.clamp(0.0, 120.0) } else { 0.0 };
        let since = now_ns.saturating_sub((secs as f64 * 1e9) as u64);
        let pts: Vec<(f32, f32)> = s
            .frames_since(since)
            .filter(|(t, _)| *t <= now_ns)
            .map(|(t, ms)| ((now_ns - t) as f32 / 1e9, ms as f32))
            .collect();
        if pts.len() <= HIST_MAX_POINTS {
            return pts;
        }
        let bucket = (pts.len() + HIST_MAX_POINTS - 1) / HIST_MAX_POINTS;
        pts.chunks(bucket)
            .map(|c| *c.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal)).unwrap())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steady(c: &mut Counter, pid: u32, start_ms: f64, dur_ms: f64, fps: f64) -> f64 {
        let step = 1000.0 / fps;
        let mut t = start_ms;
        while t < start_ms + dur_ms {
            c.feed(pid, ns(t), Src::Dxgi);
            t += step;
        }
        t
    }

    #[test]
    fn steady_60() {
        let mut c = Counter::new();
        let end = steady(&mut c, 1, 1000.0, 3000.0, 60.0);
        let s = c.stats(1, ns(end)).unwrap();
        assert!((s.fps - 60.0).abs() < 0.5, "{}", s.fps);
        assert!((s.frame_ms - 16.67).abs() < 0.1);
        assert!((s.low1.unwrap() - 60.0).abs() < 1.0);
    }

    #[test]
    fn steady_144() {
        let mut c = Counter::new();
        let end = steady(&mut c, 1, 0.0, 3000.0, 144.0);
        let s = c.stats(1, ns(end)).unwrap();
        assert!((s.fps - 144.0).abs() < 0.5, "{}", s.fps);
    }

    #[test]
    fn stutter_lowers_low1() {
        let mut c = Counter::new();
        let mut t = 1000.0;
        for i in 0..300 {
            c.feed(1, ns(t), Src::Dxgi);
            t += if i == 150 { 100.0 } else { 1000.0 / 60.0 };
        }
        let s = c.stats(1, ns(t)).unwrap();
        assert!(s.low1.unwrap() < s.fps * 0.6, "{:?}", s);
    }

    #[test]
    fn pause_restarts() {
        let mut c = Counter::new();
        let e = steady(&mut c, 1, 0.0, 2000.0, 60.0);
        let e2 = steady(&mut c, 1, e + 3000.0, 2000.0, 60.0);
        let s = c.stats(1, ns(e2)).unwrap();
        assert!((s.fps - 60.0).abs() < 0.5);
        assert!(s.low1.unwrap() > 55.0, "{:?}", s);
        assert!(c.history(1, ns(e2), 30.0).iter().all(|p| p.1 < 100.0));
    }

    #[test]
    fn both_providers_count_once() {
        let mut c = Counter::new();
        let mut t = 0.0;
        while t < 4000.0 {
            c.feed(1, ns(t), Src::Dxgi);
            c.feed(1, ns(t + 0.5), Src::D3d9);
            t += 1000.0 / 60.0;
        }
        let s = c.stats(1, ns(t)).unwrap();
        assert!((s.fps - 60.0).abs() < 0.5, "{}", s.fps);
    }

    #[test]
    fn provider_switch() {
        let mut c = Counter::new();
        // first a few stray DXGI events, then the real D3D9 stream
        for i in 0..3 {
            c.feed(1, ns(i as f64 * 100.0), Src::Dxgi);
        }
        let mut t = 300.0;
        while t < 6000.0 {
            c.feed(1, ns(t), Src::D3d9);
            t += 1000.0 / 60.0;
        }
        let s = c.stats(1, ns(t)).unwrap();
        assert!((s.fps - 60.0).abs() < 1.0, "{}", s.fps);
    }

    #[test]
    fn two_pids_sorted_and_idle_dropped() {
        let mut c = Counter::new();
        let mut t = 0.0;
        while t < 3000.0 {
            c.feed(10, ns(t), Src::Dxgi);
            t += 1000.0 / 60.0;
        }
        let mut t = 0.0;
        while t < 3000.0 {
            c.feed(20, ns(t), Src::Dxgi);
            t += 1000.0 / 144.0;
        }
        let a = c.active(ns(3000.0));
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].0, 20);
        assert_eq!(a[1].0, 10);
        assert!(c.active(ns(6000.0)).is_empty());
        assert!(c.stats(10, ns(6000.0)).is_none());
        assert!(c.active(ns(20000.0)).is_empty());
        c.cleanup(ns(20000.0));
        assert!(c.pids.is_empty());
    }

    #[test]
    fn history_bounded_ordered() {
        let mut c = Counter::new();
        let end = steady(&mut c, 1, 0.0, 8000.0, 240.0);
        let h = c.history(1, ns(end), 120.0);
        assert!(!h.is_empty() && h.len() <= 200);
        assert!(h.windows(2).all(|w| w[0].0 >= w[1].0));
        let h2 = c.history(1, ns(end), 2.0);
        assert!(h2.iter().all(|p| p.0 <= 2.01));
    }

    #[test]
    fn history_keeps_stutter() {
        let mut c = Counter::new();
        let mut t = 0.0;
        for i in 0..600 {
            c.feed(1, ns(t), Src::Dxgi);
            t += if i == 300 { 150.0 } else { 1000.0 / 144.0 };
        }
        let h = c.history(1, ns(t), 10.0);
        assert!(h.len() <= 200);
        assert!(h.iter().any(|p| p.1 > 140.0));
    }

    #[test]
    fn memory_bounded() {
        let mut c = Counter::new();
        let step = 1000.0 / 240.0;
        let mut t = 0.0;
        while t < 600_000.0 {
            for pid in 0..50u32 {
                // half of the processes die after 20 s
                if pid % 2 == 0 || t < 20_000.0 {
                    c.feed(pid, ns(t), Src::Dxgi);
                }
            }
            t += step * 10.0; // coarser stepping keeps the test quick
        }
        assert!(c.pids.len() <= 50);
        assert!(c.pids.values().all(|s| s.times.len() <= RING_CAP));
        let mut c2 = Counter::new();
        for i in 0..100_000u64 {
            c2.feed(1, i * 100_000, Src::Dxgi); // 10 kHz
        }
        assert!(c2.pids[&1].times.len() <= RING_CAP);
    }

    #[test]
    fn odd_timestamps_do_not_panic() {
        let mut c = Counter::new();
        for &t in &[0u64, 0, 5, 3, 3, u64::MAX, u64::MAX, u64::MAX - 1, 10, 0] {
            c.feed(1, t, Src::Dxgi);
            c.feed(1, t, Src::D3d9);
        }
        for &n in &[0u64, 1, u64::MAX] {
            let _ = c.stats(1, n);
            let _ = c.history(1, n, 120.0);
            let _ = c.history(1, n, f32::NAN);
            let _ = c.active(n);
        }
        let mut c = Counter::new();
        let base = u64::MAX - 5 * SEC;
        for i in 0..200u64 {
            c.feed(2, base + i * 16_666_666, Src::Dxgi);
        }
        let _ = c.stats(2, u64::MAX);
        let _ = c.history(2, u64::MAX, 10.0);
    }
}