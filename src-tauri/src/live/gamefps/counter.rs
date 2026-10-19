//! Pure frame-time statistics from present events (no Windows calls: unit-testable with synthetic events).
//! STUB: filled in by the ETW worker.

/// Where a present event came from (a process can emit both a D3D9 and a DXGI event for one frame: count one provider per process).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Src {
    Dxgi,
    D3d9,
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

#[derive(Default)]
pub struct Counter {}

impl Counter {
    pub fn new() -> Counter {
        Counter::default()
    }
    /// One present of process `pid` at `t_ns` (nanoseconds on the sampler's clock).
    pub fn feed(&mut self, _pid: u32, _t_ns: u64, _src: Src) {}
    /// Statistics of `pid` as of `now_ns`; None when it has not presented in the last ~2 s.
    pub fn stats(&self, _pid: u32, _now_ns: u64) -> Option<PidStats> {
        None
    }
    /// Processes that presented recently, with their fps (highest first).
    pub fn active(&self, _now_ns: u64) -> Vec<(u32, f32)> {
        Vec::new()
    }
    /// (age in seconds, frame time ms) of `pid` within the last `secs` seconds, oldest first.
    pub fn history(&self, _pid: u32, _now_ns: u64, _secs: f32) -> Vec<(f32, f32)> {
        Vec::new()
    }
}
