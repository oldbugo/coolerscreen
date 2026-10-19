//! Our own ETW session for present events (Microsoft-Windows-DXGI and -D3D9; the way Intel PresentMon sees frames). STUB: filled in by
//! the ETW worker. Rules: a fixed, descriptive session name; real-time mode; NEVER the NT Kernel Logger; create the session EMPTY (no
//! providers) so it exists before a game with an anti-cheat starts, enable the providers only on demand; clean an orphan session of
//! our own name before creating; stop it on `stop()` / drop.
use super::counter::Counter;
use std::sync::{Arc, Mutex};

pub const SESSION_NAME: &str = "CoolerScreen-GameFPS";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EtwError {
    /// ERROR_ACCESS_DENIED (5): not an administrator and not in "Performance Log Users".
    AccessDenied,
    /// Any other Win32 error code.
    Other(u32),
}

pub struct EtwSession {}

impl EtwSession {
    /// Create (or take over) our named session and start consuming into `sink`. Providers stay disabled.
    pub fn start(_sink: Arc<Mutex<Counter>>) -> Result<EtwSession, EtwError> {
        Err(EtwError::Other(0))
    }
    pub fn enable_providers(&self) -> Result<(), EtwError> {
        Ok(())
    }
    pub fn disable_providers(&self) {}
    /// Stop the session (idempotent).
    pub fn stop(&mut self) {}
    /// The clock the events use, in ns, "now" (so the sampler can ask `Counter` for statistics as of now).
    pub fn now_ns() -> u64 {
        0
    }
}
