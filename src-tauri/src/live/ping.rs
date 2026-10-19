//! 4.0 stream B2: the `ping` widget's data. One ICMP echo per second to a host the USER typed into the widget (default 1.1.1.1), only
//! while a ping widget is shown (a thread per host, ending a few seconds after the last ask). Windows' own ICMP API (IcmpSendEcho): no
//! admin, no driver. Nothing else is sent.
use std::collections::{HashMap, VecDeque};
use std::net::{Ipv4Addr, ToSocketAddrs};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const KEEP: Duration = Duration::from_secs(5);
const RING: usize = 600;
const MAX_HOSTS: usize = 3;

#[derive(Default)]
struct Slot {
    last_used: Option<Instant>,
    running: bool,
    /// (when, round trip in ms; None = lost / unreachable)
    ring: VecDeque<(Instant, Option<f32>)>,
    resolved: Option<(Instant, Option<Ipv4Addr>)>,
}

fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static S: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Clone, Debug, PartialEq)]
pub struct PingView {
    /// The last answer in ms; None = no answer yet or the last one was lost.
    pub last: Option<f32>,
    /// Some answer has been tried (so None above means "lost", not "waiting").
    pub tried: bool,
    /// Lost share of the last 20 tries, 0..1.
    pub loss: f32,
    /// (age in seconds, ms) of the answers within `secs`, oldest first.
    pub hist: Vec<(f32, f32)>,
}

/// A host the widget may use: letters, digits, dots and dashes only (also an IPv4 address).
pub fn clean_host(s: &str) -> Option<String> {
    let t = s.trim().trim_matches('.');
    (!t.is_empty() && t.len() <= 60 && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')).then(|| t.to_string())
}

/// The latest numbers for `host` (registers the demand and starts its thread when needed). None: not an acceptable host.
pub fn read(host: &str, secs: f32) -> Option<PingView> {
    let host = clean_host(host)?;
    let mut g = slots().lock().unwrap();
    if !g.contains_key(&host) && g.values().filter(|s| s.running).count() >= MAX_HOSTS {
        return None;
    }
    let s = g.entry(host.clone()).or_default();
    s.last_used = Some(Instant::now());
    if !s.running {
        s.running = true;
        let h = host.clone();
        std::thread::spawn(move || run(h));
    }
    let now = Instant::now();
    let hist = s.ring.iter().filter_map(|(t, v)| v.map(|v| ((now - *t).as_secs_f32(), v))).filter(|(a, _)| *a <= secs).collect();
    let recent: Vec<_> = s.ring.iter().rev().take(20).collect();
    let loss = if recent.is_empty() { 0.0 } else { recent.iter().filter(|(_, v)| v.is_none()).count() as f32 / recent.len() as f32 };
    Some(PingView { last: s.ring.back().and_then(|(_, v)| *v), tried: !s.ring.is_empty(), loss, hist })
}

fn resolve(host: &str) -> Option<Ipv4Addr> {
    if let Ok(a) = host.parse::<Ipv4Addr>() {
        return Some(a);
    }
    (host, 0u16).to_socket_addrs().ok()?.find_map(|a| match a.ip() {
        std::net::IpAddr::V4(v) => Some(v),
        _ => None,
    })
}

fn run(host: String) {
    use windows_sys::Win32::NetworkManagement::IpHelper::{IcmpCloseHandle, IcmpCreateFile, IcmpSendEcho};
    unsafe {
        let h = IcmpCreateFile();
        let mut next = Instant::now();
        loop {
            // stop when nobody asked lately
            {
                let g = slots().lock().unwrap();
                if !g.get(&host).and_then(|s| s.last_used).is_some_and(|t| t.elapsed() < KEEP) {
                    break;
                }
            }
            let dest = {
                let cached = slots().lock().unwrap().get(&host).and_then(|s| s.resolved).filter(|(t, ip)| t.elapsed() < Duration::from_secs(if ip.is_some() { 300 } else { 10 }));
                match cached {
                    Some((_, ip)) => ip,
                    None => {
                        let ip = resolve(&host);
                        if let Some(s) = slots().lock().unwrap().get_mut(&host) {
                            s.resolved = Some((Instant::now(), ip));
                        }
                        ip
                    }
                }
            };
            let rtt: Option<f32> = match (dest, h as isize != -1 && !h.is_null()) {
                (Some(ip), true) => {
                    let payload = *b"CoolerScreen   ";
                    let mut reply = [0u8; 160];
                    let n = IcmpSendEcho(h, u32::from_ne_bytes(ip.octets()), payload.as_ptr() as *const _, payload.len() as u16, std::ptr::null(), reply.as_mut_ptr() as *mut _, reply.len() as u32, 1000);
                    // ICMP_ECHO_REPLY: Address u32, Status u32, RoundTripTime u32
                    let status = u32::from_le_bytes(reply[4..8].try_into().unwrap());
                    (n > 0 && status == 0).then(|| u32::from_le_bytes(reply[8..12].try_into().unwrap()) as f32)
                }
                _ => None,
            };
            {
                let mut g = slots().lock().unwrap();
                if let Some(s) = g.get_mut(&host) {
                    s.ring.push_back((Instant::now(), rtt));
                    while s.ring.len() > RING {
                        s.ring.pop_front();
                    }
                }
            }
            next += Duration::from_secs(1);
            let now = Instant::now();
            if next > now {
                std::thread::sleep(next - now);
            } else {
                next = now;
            }
        }
        if h as isize != -1 && !h.is_null() {
            IcmpCloseHandle(h);
        }
    }
    if let Some(s) = slots().lock().unwrap().get_mut(&host) {
        s.running = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_checked() {
        assert_eq!(clean_host(" 1.1.1.1 ").as_deref(), Some("1.1.1.1"));
        assert_eq!(clean_host("example.com").as_deref(), Some("example.com"));
        assert!(clean_host("a b").is_none() && clean_host("").is_none() && clean_host("x/y").is_none() && clean_host("http://a").is_none());
    }

    #[test]
    fn loopback_answers() {
        let _ = read("127.0.0.1", 30.0);
        let mut v = None;
        for _ in 0..30 {
            std::thread::sleep(Duration::from_millis(200));
            v = read("127.0.0.1", 30.0);
            if v.as_ref().is_some_and(|v| v.last.is_some()) {
                break;
            }
        }
        let v = v.unwrap();
        assert!(v.last.is_some_and(|ms| ms < 200.0), "{v:?}");
        assert!(read("not a host", 30.0).is_none());
    }
}
