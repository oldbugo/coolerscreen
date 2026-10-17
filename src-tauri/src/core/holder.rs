//! Who is using the screen when opening it fails with "busy": CoolerScreen itself (a live session in this process), one of
//! its own tools (cs2, cs-companion), another CoolerScreen, the vendor's LOVINGCOOL MONITOR, or something unknown.
//! Only asked after a claim failed; the answer goes into the message the page shows and into device.log.
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Holder {
    /// "self" | "tool" | "instance" | "vendor" | "unknown"
    pub kind: &'static str,
    pub name: String,
    pub pid: Option<u32>,
}

impl Holder {
    /// The sentence the page shows.
    pub fn message(&self) -> String {
        match self.kind {
            "self" => "CoolerScreen's live screen is using it. Trying again in a moment.".to_string(),
            "tool" => format!("CoolerScreen's test tool ({}) is using the screen. Close it.", self.name),
            "instance" => "Another CoolerScreen is using the screen.".to_string(),
            "vendor" => "LOVINGCOOL MONITOR is using the screen. Close it.".to_string(),
            _ => "Something else is using the screen.".to_string(),
        }
    }
    pub fn to_value(&self) -> Value {
        json!({ "kind": self.kind, "name": self.name, "pid": self.pid })
    }
}

/// Classify a process list (`(exe name, pid)`), ignoring `me`.
pub fn classify(procs: &[(String, u32)], me: u32) -> Option<Holder> {
    let mk = |kind, name: &str, pid| Some(Holder { kind, name: name.to_string(), pid: Some(pid) });
    let low: Vec<(String, u32)> = procs.iter().filter(|(_, p)| *p != me).map(|(n, p)| (n.to_lowercase(), *p)).collect();
    for (n, p) in &low {
        if n == "cs2.exe" {
            return mk("tool", "cs2", *p);
        }
        if n == "cs-companion.exe" {
            return mk("tool", "cs-companion", *p);
        }
    }
    for (n, p) in &low {
        if n == "coolerscreen.exe" {
            return mk("instance", "CoolerScreen", *p);
        }
    }
    for (n, p) in &low {
        if n.contains("lovingcool") {
            return mk("vendor", "LOVINGCOOL MONITOR", *p);
        }
    }
    None
}

#[cfg(windows)]
fn processes() -> Vec<(String, u32)> {
    use std::os::windows::process::CommandExt;
    let Ok(out) = std::process::Command::new("tasklist").args(["/FO", "CSV", "/NH"]).creation_flags(0x0800_0000).output() else { return vec![] };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.trim().trim_matches('"').split("\",\"");
            let name = f.next()?.to_string();
            let pid = f.next()?.parse().ok()?;
            Some((name, pid))
        })
        .collect()
}
#[cfg(not(windows))]
fn processes() -> Vec<(String, u32)> {
    vec![]
}

/// The most likely holder now. `COOLERSCREEN_FAKE_HOLDER=self|tool|instance|vendor|unknown` forces one (tests, fake screen).
pub fn detect() -> Holder {
    if let Ok(f) = std::env::var("COOLERSCREEN_FAKE_HOLDER") {
        let (kind, name) = match f.as_str() {
            "self" => ("self", "CoolerScreen"),
            "tool" => ("tool", "cs2"),
            "instance" => ("instance", "CoolerScreen"),
            "vendor" => ("vendor", "LOVINGCOOL MONITOR"),
            _ => ("unknown", ""),
        };
        return Holder { kind, name: name.into(), pid: None };
    }
    if crate::device::in_process_open() > 0 {
        return Holder { kind: "self", name: "CoolerScreen".into(), pid: Some(std::process::id()) };
    }
    classify(&processes(), std::process::id()).unwrap_or(Holder { kind: "unknown", name: String::new(), pid: None })
}

/// A busy error with the holder named (and a line in device.log).
pub fn busy_error(detail: &str) -> super::error::CoreError {
    let h = detect();
    crate::device::log(&format!("screen busy: holder={} {} pid={:?} ({})", h.kind, h.name, h.pid, detail));
    super::error::CoreError::coded("busy", h.message()).status(502).with("holder", h.to_value())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(v: &[(&str, u32)]) -> Vec<(String, u32)> {
        v.iter().map(|(n, i)| (n.to_string(), *i)).collect()
    }
    #[test]
    fn names_the_likely_holder() {
        assert!(classify(&p(&[("explorer.exe", 1), ("chrome.exe", 2)]), 9).is_none());
        assert_eq!(classify(&p(&[("CS2.exe", 5)]), 9).unwrap().message(), "CoolerScreen's test tool (cs2) is using the screen. Close it.");
        assert_eq!(classify(&p(&[("cs-companion.exe", 5), ("lovingcool.exe", 6)]), 9).unwrap().kind, "tool");
        assert_eq!(classify(&p(&[("LOVINGCOOL MONITOR.exe", 6)]), 9).unwrap().kind, "vendor");
        assert_eq!(classify(&p(&[("CoolerScreen.exe", 7)]), 9).unwrap().kind, "instance");
        assert!(classify(&p(&[("CoolerScreen.exe", 9)]), 9).is_none(), "ourselves are not a holder");
    }
}
