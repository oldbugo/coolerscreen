//! Which build this is: git short hash (a "+" when built with uncommitted changes), commit date and build time
//! (embedded by build.rs; "unknown" when git was not available).
pub const HASH: &str = env!("CS_GIT_HASH");
pub const COMMIT_DATE: &str = env!("CS_GIT_DATE");
/// Seconds since 1970 when the build script ran.
pub fn built_at() -> u64 {
    env!("CS_BUILT_AT").parse().unwrap_or(0)
}
pub fn title(version: &str) -> String {
    format!("CoolerScreen {version} ({HASH}){}", screen_note())
}
/// Debug builds only: which screen this process uses, for the window title and the log (release builds: nothing, as before).
pub fn screen_note() -> String {
    if !cfg!(debug_assertions) {
        return String::new();
    }
    match super::device_service::fake_mode_from_env() {
        Some(m) => format!(" (debug, fake screen: {m})"),
        None => " (debug, REAL screen)".into(),
    }
}
/// First line of device.log at start-up, to match what the user runs.
pub fn startup_line(version: &str) -> String {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    format!("CoolerScreen {version} build {HASH} started, pid {}, exe {exe}{}", std::process::id(), screen_note())
}
