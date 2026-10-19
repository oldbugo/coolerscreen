//! RivaTuner Statistics Server shared memory (`RTSSSharedMemoryV2`), read-only. STUB: filled in by the RTSS worker.
//! Only used when the user already runs RTSS (we never start or install it).

#[derive(Clone, Debug, PartialEq)]
pub struct RtssFrame {
    pub pid: u32,
    /// Program name without path and extension.
    pub name: String,
    pub fps: f32,
    pub frame_ms: f32,
}

/// The entry of the foreground program (or, when `foreground_pid` is None / has none, the freshest entry), None when RTSS is not running
/// or the entry is stale.
pub fn read(_foreground_pid: Option<u32>) -> Option<RtssFrame> {
    None
}

/// Pure parser for tests: `buf` = the mapped bytes, `tick_ms` = GetTickCount() now.
pub fn parse(_buf: &[u8], _foreground_pid: Option<u32>, _tick_ms: u32) -> Option<RtssFrame> {
    None
}
