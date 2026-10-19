//! Which program is in front (no injection). STUB: filled in by the RTSS/foreground worker.

#[derive(Clone, Debug, PartialEq)]
pub struct Fg {
    pub pid: u32,
    /// "MonsterHunterWilds" (image name without .exe)
    pub name: String,
    /// The window covers its whole monitor (fullscreen or borderless fullscreen).
    pub fullscreen: bool,
}

/// The foreground window's process (None for the desktop/shell or when it cannot be read). Never our own pid.
pub fn current() -> Option<Fg> {
    None
}

/// Image name (without .exe) of a process id; None when it cannot be opened.
pub fn process_name(_pid: u32) -> Option<String> {
    None
}

/// true when the current user token is a member of BUILTIN\Performance Log Users (S-1-5-32-559) or of Administrators with an
/// elevated token (i.e. may start event sessions).
pub fn may_start_event_sessions() -> bool {
    false
}

/// true when the current token is a member of "Performance Log Users" (the group the user has to join once).
pub fn in_performance_log_users() -> bool {
    false
}
