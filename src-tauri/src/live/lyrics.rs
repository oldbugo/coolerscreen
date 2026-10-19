//! Lyrics data layer (no drawing): LRC parsing, line lookup, title cleaning, the LRCLIB client, disk cache, local .lrc
//! sidecars, per-track offsets, a background loader and a smooth playback clock.
//!
//! Privacy / network rules: the network is touched ONLY when the loader is `enabled` (opt-in), only after a track change
//! (one lookup per track, results cached on disk), only while requests keep coming from the renderer (the loader thread
//! ends ~30 s after the last `request`), and never more often than the back-off allows. When disabled, no network at all;
//! local sources (a sidecar `.lrc` and already cached results) are still used because they are local.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::nowplaying::NowPlaying;

const KEEP: Duration = Duration::from_secs(30);
const MAX_BYTES: usize = 1024 * 1024;
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);
const BACKOFF_START: u64 = 30;
const BACKOFF_MAX: u64 = 600;
/// A "not found" (or "no synced lyrics") result is asked again after this long; found results never expire.
const NOT_FOUND_TTL: i64 = 7 * 24 * 3600;
const MAX_STATES: usize = 64;
const MAX_PENDING: usize = 4;
const DURATION_TOLERANCE_S: f64 = 3.0;

// ================================================================= LRC

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub start_s: f64,
    pub end_s: f64,
    /// Empty = an instrumental gap marker.
    pub text: String,
}

/// Parse one `mm:ss`, `mm:ss.xx`, `mm:ss.xxx` (or `mm:ss:xx`) time; `None` when it is not a time.
fn parse_time(s: &str) -> Option<f64> {
    let (m, rest) = s.split_once(':')?;
    if m.is_empty() || m.len() > 4 || !m.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (sec, frac) = match rest.find(['.', ':']) {
        Some(i) => (&rest[..i], Some(&rest[i + 1..])),
        None => (rest, None),
    };
    if sec.is_empty() || sec.len() > 2 || !sec.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut t = m.parse::<f64>().ok()? * 60.0 + sec.parse::<f64>().ok()?;
    if let Some(f) = frac {
        if f.is_empty() || f.len() > 3 || !f.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        t += f.parse::<f64>().ok()? / 10f64.powi(f.len() as i32);
    }
    Some(t)
}

/// Remove enhanced word tags `<mm:ss.xx>` and tidy spaces.
fn strip_word_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('<') {
        match rest[i..].find('>') {
            Some(j) if parse_time(&rest[i + 1..i + j]).is_some() => {
                out.push_str(&rest[..i]);
                rest = &rest[i + j + 1..];
            }
            _ => {
                out.push_str(&rest[..i + 1]);
                rest = &rest[i + 1..];
            }
        }
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parse LRC text. `[offset:+500]` (ms) follows the LRC convention: positive = lyrics appear earlier (time minus offset).
/// Tag-less lines and metadata tags are ignored; time-tagged empty lines are kept (empty text). Sorted by time (stable).
/// The end of a line is the next line's start; the last line ends at `duration_s` when that is later than its start,
/// else 5 s after it.
pub fn parse_lrc(text: &str, duration_s: f64) -> Vec<Line> {
    let text = text.trim_start_matches('\u{feff}');
    let mut offset = 0.0f64;
    let mut raw: Vec<(f64, String)> = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut times: Vec<f64> = Vec::new();
        let mut meta_line = false;
        while rest.starts_with('[') {
            let Some(j) = rest.find(']') else { break };
            let tag = &rest[1..j];
            if let Some(t) = parse_time(tag) {
                times.push(t);
            } else if times.is_empty() {
                if let Some(v) = tag.strip_prefix("offset:").or_else(|| tag.strip_prefix("Offset:")) {
                    if let Ok(ms) = v.trim().trim_start_matches('+').parse::<f64>() {
                        offset = ms / 1000.0;
                    }
                }
                meta_line = true;
                break;
            } else {
                break;
            }
            rest = rest[j + 1..].trim_start();
        }
        if meta_line || times.is_empty() {
            continue;
        }
        let t = strip_word_tags(rest);
        for ts in times {
            raw.push((ts, t.clone()));
        }
    }
    raw.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut lines: Vec<Line> = raw.into_iter().map(|(t, text)| Line { start_s: (t - offset).max(0.0), end_s: 0.0, text }).collect();
    let n = lines.len();
    for i in 0..n {
        lines[i].end_s = if i + 1 < n {
            lines[i + 1].start_s
        } else if duration_s > lines[i].start_s {
            duration_s
        } else {
            lines[i].start_s + 5.0
        };
    }
    lines
}

/// Where playback is inside the lyrics.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    /// Index of the current line (`None` before the first line).
    pub index: Option<usize>,
    /// 0..1 through the line's sung time (see [`natural_len`]); 1 once the line has been sung.
    pub progress: f64,
    /// Instrumental: before the first line, on an empty line, or the line was sung long ago and the next one is far off.
    pub gap: bool,
    /// Seconds until the next line starts (`None` after the last).
    pub next_in: Option<f64>,
}

/// How long a line is assumed to take to sing: `max(4 s, chars * 0.35 s)`.
pub fn natural_len(text: &str) -> f64 {
    (text.chars().count() as f64 * 0.35).max(4.0)
}
/// A line counts as followed by an instrumental gap when more than this many seconds lie between its natural end and
/// the next line's start.
pub const GAP_AFTER_NATURAL_S: f64 = 8.0;

/// Current line for `pos_s` (seconds, offset already applied).
pub fn locate(lines: &[Line], pos_s: f64) -> Position {
    let n = lines.partition_point(|l| l.start_s <= pos_s);
    if n == 0 {
        return Position { index: None, progress: 0.0, gap: true, next_in: lines.first().map(|l| l.start_s - pos_s) };
    }
    let i = n - 1;
    let l = &lines[i];
    let next_in = lines.get(i + 1).map(|x| x.start_s - pos_s);
    if l.text.is_empty() {
        return Position { index: Some(i), progress: 0.0, gap: true, next_in };
    }
    let slot = (l.end_s - l.start_s).max(0.001);
    let nat = natural_len(&l.text);
    let sung = slot.min(nat);
    let el = pos_s - l.start_s;
    let progress = (el / sung).clamp(0.0, 1.0);
    let gap = el > nat && (l.end_s - (l.start_s + nat)) > GAP_AFTER_NATURAL_S;
    Position { index: Some(i), progress, gap, next_in }
}

// ================================================================= cleaning

const NOISE: &[&str] = &[
    "official", "video", "audio", "lyric", "lyrics", "visualizer", "visualiser", "hd", "hq", "4k", "8k", "1080p", "mv", "music", "remaster",
    "remastered", "explicit", "clean", "version", "full", "uhd",
];
/// At least one of these must be present for a bracket / suffix to count as decoration.
const STRONG: &[&str] = &["official", "video", "audio", "lyric", "lyrics", "visualizer", "visualiser", "hd", "hq", "4k", "8k", "1080p", "mv", "remaster", "remastered", "explicit"];

fn tokens(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).map(str::to_string).collect()
}

/// Bracket content that is only decoration ("Official Video", "Remastered 2011", "feat. X").
fn noisy(content: &str) -> bool {
    let t = tokens(content);
    let Some(first) = t.first() else { return false };
    if matches!(first.as_str(), "feat" | "ft" | "featuring") {
        return true;
    }
    t.iter().all(|w| w.bytes().all(|b| b.is_ascii_digit()) || NOISE.contains(&w.as_str())) && t.iter().any(|w| STRONG.contains(&w.as_str()))
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find(['(', '[', '{', '（', '【']) {
        let open = rest[i..].chars().next().unwrap();
        let close = match open {
            '(' => ')',
            '[' => ']',
            '{' => '}',
            '（' => '）',
            _ => '】',
        };
        out.push_str(&rest[..i]);
        let after = &rest[i + open.len_utf8()..];
        match after.find(close) {
            Some(j) => {
                if !noisy(&after[..j]) {
                    out.push(open);
                    out.push_str(&after[..j]);
                    out.push(close);
                }
                rest = &after[j + close.len_utf8()..];
            }
            None => {
                out.push_str(&rest[i..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Cut at the first " feat. " / " ft. " / " featuring " (outside brackets).
fn cut_feat(s: &str) -> &str {
    let l = s.to_ascii_lowercase();
    let mut cut = s.len();
    for p in [" feat.", " feat ", " ft.", " ft ", " featuring ", " feat:"] {
        if let Some(i) = l.find(p) {
            cut = cut.min(i);
        }
    }
    &s[..cut]
}

fn tidy(s: &str) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    t.trim_matches(|c: char| c == '-' || c == '–' || c == '—' || c == '|' || c.is_whitespace()).to_string()
}

fn norm_dashes(s: &str) -> String {
    s.replace(" – ", " - ").replace(" — ", " - ").replace(" ‐ ", " - ")
}

/// A title without decoration (for matching): no "(Official Video)", "[Lyrics]", remaster tags, "feat." parts, no
/// trailing " - Remastered 2011" or " | Channel" part.
pub fn clean_title(title: &str) -> String {
    let mut t = norm_dashes(title);
    if let Some((a, _)) = t.split_once(" | ") {
        if !a.trim().is_empty() {
            t = a.to_string();
        }
    }
    let t = strip_brackets(&t);
    let t = cut_feat(&t).to_string();
    let mut t = tidy(&t);
    while let Some((a, b)) = t.rsplit_once(" - ") {
        if noisy(b) && !a.trim().is_empty() {
            t = tidy(a);
        } else {
            break;
        }
    }
    t
}

/// (artist without " - Topic" / "VEVO" / feat., whether the artist looked like a YouTube channel)
pub fn clean_artist(artist: &str) -> (String, bool) {
    let mut a = artist.trim().to_string();
    let mut channel = false;
    let low = a.to_ascii_lowercase();
    if low.ends_with(" - topic") {
        a.truncate(a.len() - 8);
        channel = true;
    }
    let low = a.to_ascii_lowercase();
    if low.ends_with("vevo") && a.len() > 4 {
        a.truncate(a.len() - 4);
        channel = true;
    }
    let a = strip_brackets(&a);
    (tidy(cut_feat(&a)), channel)
}

/// The first of several artists ("A, B", "A & B", "A x B", "A and B", "A / B"), if there are several.
pub fn first_artist(artist: &str) -> Option<String> {
    let mut cut = artist.len();
    for p in [", ", " & ", " x ", " X ", " and ", " + ", " / ", "; ", " with "] {
        if let Some(i) = artist.find(p) {
            cut = cut.min(i);
        }
    }
    let f = artist[..cut].trim();
    (cut < artist.len() && !f.is_empty()).then(|| f.to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cleaned {
    pub title: String,
    pub artist: String,
    /// First of several artists, as a fallback attempt.
    pub alt_artist: Option<String>,
}

/// Normalised form for comparing: lowercase, letters and digits only, single spaces.
pub fn norm(s: &str) -> String {
    let m: String = s.to_lowercase().chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect();
    m.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Equal, or one contains the other with a length ratio of at least 0.8.
pub fn titles_match(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (la, lb) = (a.chars().count() as f64, b.chars().count() as f64);
    (a.contains(&b) || b.contains(&a)) && la.min(lb) / la.max(lb) >= 0.8
}

fn artists_related(a: &str, b: &str) -> bool {
    let (a, b) = (norm(a), norm(b));
    !a.is_empty() && !b.is_empty() && (a.contains(&b) || b.contains(&a))
}

/// Clean what a player reports (YouTube Music / browsers included).
pub fn clean_track(title: &str, artist: &str) -> Cleaned {
    let mut t = clean_title(title);
    let (mut a, channel) = clean_artist(artist);
    if let Some((l, r)) = t.split_once(" - ") {
        let (l, r) = (tidy(l), tidy(r));
        if !l.is_empty() && !r.is_empty() && (channel || a.is_empty() || artists_related(&a, &l)) {
            a = l;
            t = r;
        }
    }
    let alt_artist = first_artist(&a);
    Cleaned { title: t, artist: a, alt_artist }
}

// ================================================================= query / key

#[derive(Clone, Debug)]
pub struct Query {
    pub raw_title: String,
    pub raw_artist: String,
    pub cleaned: Cleaned,
    pub duration_s: f64,
    /// Cache / offsets key.
    pub key: String,
}

impl Query {
    pub fn new(title: &str, artist: &str, duration_s: f64) -> Query {
        let cleaned = clean_track(title, artist);
        let d = if duration_s.is_finite() && duration_s > 0.0 { duration_s } else { 0.0 };
        let key = track_key(&cleaned.title, &cleaned.artist, d);
        Query { raw_title: title.to_string(), raw_artist: artist.to_string(), cleaned, duration_s: d, key }
    }
    pub fn from_np(np: &NowPlaying) -> Query {
        Query::new(&np.title, &np.artist, np.duration_s)
    }
}

/// Hash of (cleaned title, cleaned artist, duration rounded to whole seconds).
pub fn track_key(title: &str, artist: &str, duration_s: f64) -> String {
    let mut h = sha1_smol::Sha1::new();
    h.update(format!("{}\u{0}{}\u{0}{}", title.to_lowercase(), artist.to_lowercase(), duration_s.round() as i64).as_bytes());
    h.digest().to_string()[..32].to_string()
}

// ================================================================= HTTP

pub struct HttpReply {
    pub status: u16,
    pub retry_after: Option<u64>,
    pub body: String,
}

/// Injectable GET (real one: [`ReqwestHttp`]).
pub trait Http: Send + Sync {
    fn get(&self, url: &str) -> Result<HttpReply, String>;
}

pub struct ReqwestHttp;

impl Http for ReqwestHttp {
    fn get(&self, url: &str) -> Result<HttpReply, String> {
        use std::io::Read;
        let policy = reqwest::redirect::Policy::custom(|a| {
            let prev_https = a.previous().last().is_some_and(|u| u.scheme() == "https");
            if a.previous().len() >= 3 || (prev_https && a.url().scheme() != "https") || !a.url().username().is_empty() || a.url().password().is_some() {
                a.stop()
            } else {
                a.follow()
            }
        });
        let ua = format!("CoolerScreen/{} (private; lyrics widget)", env!("CARGO_PKG_VERSION"));
        let c = reqwest::blocking::Client::builder().timeout(HTTP_TIMEOUT).user_agent(ua).redirect(policy).build().map_err(|_| "No network client".to_string())?;
        let r = c.get(url).header(reqwest::header::ACCEPT, "application/json").send().map_err(|_| "No answer".to_string())?;
        let status = r.status().as_u16();
        let retry_after = r.headers().get(reqwest::header::RETRY_AFTER).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse::<u64>().ok());
        if r.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
            return Err("Response too large".into());
        }
        let mut buf = Vec::new();
        r.take(MAX_BYTES as u64 + 1).read_to_end(&mut buf).map_err(|_| "Unreadable answer".to_string())?;
        if buf.len() > MAX_BYTES {
            return Err("Response too large".into());
        }
        Ok(HttpReply { status, retry_after, body: String::from_utf8_lossy(&buf).into_owned() })
    }
}

fn pct(s: &str) -> String {
    let mut o = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            o.push(b as char);
        } else {
            o.push_str(&format!("%{b:02X}"));
        }
    }
    o
}

const API: &str = "https://lrclib.net/api";

fn get_url(title: &str, artist: &str, dur: f64) -> String {
    let mut u = format!("{API}/get?track_name={}&artist_name={}", pct(title), pct(artist));
    if dur > 0.0 {
        u.push_str(&format!("&duration={}", dur.round() as i64));
    }
    u
}
fn search_url(title: &str, artist: &str) -> String {
    format!("{API}/search?track_name={}&artist_name={}", pct(title), pct(artist))
}

// ================================================================= lookup

#[derive(Clone, Debug, PartialEq)]
pub enum Net {
    /// Synced lyrics (raw LRC text) of LRCLIB track `id`.
    Found { id: Option<i64>, track: String, artist: String, lrc: String },
    /// The track exists but only has untimed lyrics (or none).
    NoSynced,
    Instrumental { id: Option<i64> },
    NotFound,
    /// Transient failure: try later (seconds from a Retry-After header, if any).
    Retry(Option<u64>),
}

#[derive(Debug)]
struct Cand {
    id: Option<i64>,
    track: String,
    artist: String,
    instrumental: bool,
    synced: Option<String>,
    ddiff: f64,
}

fn cand_from(v: &Value) -> Option<Cand> {
    let o = v.as_object()?;
    let s = |k: &str| o.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let synced = o.get("syncedLyrics").and_then(Value::as_str).filter(|x| !x.trim().is_empty()).map(str::to_string);
    Some(Cand {
        id: o.get("id").and_then(Value::as_i64),
        track: s("trackName"),
        artist: s("artistName"),
        instrumental: o.get("instrumental").and_then(Value::as_bool).unwrap_or(false),
        synced,
        ddiff: o.get("duration").and_then(Value::as_f64).unwrap_or(0.0),
    })
}

/// Is a /api/search result acceptable for the query? Known duration: within 3 s and cleaned titles match (and the
/// artists are related); unknown duration: exact normalised title and artist.
fn accept(q: &Query, artist: &str, c: &mut Cand) -> bool {
    let rt = clean_title(&c.track);
    let (ra, _) = clean_artist(&c.artist);
    if q.duration_s > 0.0 {
        let d = (c.ddiff - q.duration_s).abs();
        c.ddiff = d;
        d <= DURATION_TOLERANCE_S && titles_match(&q.cleaned.title, &rt) && (artists_related(artist, &ra) || artists_related(&q.cleaned.artist, &ra))
    } else {
        c.ddiff = 0.0;
        norm(&q.cleaned.title) == norm(&rt) && (norm(artist) == norm(&ra) || norm(&q.cleaned.artist) == norm(&ra))
    }
}

/// Choose from search results: accepted ones only; synced first, then smallest duration difference.
pub fn pick_search(q: &Query, artist: &str, body: &str) -> Net {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(body) else { return Net::NotFound };
    let mut ok: Vec<Cand> = items.iter().filter_map(cand_from).filter_map(|mut c| accept(q, artist, &mut c).then_some(c)).collect();
    ok.sort_by(|a, b| (b.synced.is_some() as u8).cmp(&(a.synced.is_some() as u8)).then(a.ddiff.partial_cmp(&b.ddiff).unwrap_or(std::cmp::Ordering::Equal)));
    classify(ok.into_iter())
}

fn classify(mut it: impl Iterator<Item = Cand>) -> Net {
    let mut any = false;
    let mut instr = None;
    for c in it.by_ref() {
        any = true;
        if let Some(l) = c.synced {
            return Net::Found { id: c.id, track: c.track, artist: c.artist, lrc: l };
        }
        if c.instrumental && instr.is_none() {
            instr = Some(c.id);
        }
    }
    match instr {
        Some(id) => Net::Instrumental { id },
        None if any => Net::NoSynced,
        None => Net::NotFound,
    }
}

fn retry_for(r: &HttpReply) -> Option<Net> {
    (r.status == 429 || r.status >= 500).then_some(Net::Retry(r.retry_after))
}

/// One lookup for one artist spelling: /api/get, then /api/search.
fn lookup_artist(http: &dyn Http, q: &Query, artist: &str) -> Net {
    let t = &q.cleaned.title;
    let r = match http.get(&get_url(t, artist, q.duration_s)) {
        Ok(r) => r,
        Err(_) => return Net::Retry(None),
    };
    if let Some(n) = retry_for(&r) {
        return n;
    }
    let mut got_but_unsynced = false;
    if r.status == 200 {
        if let Ok(v) = serde_json::from_str::<Value>(&r.body) {
            if let Some(c) = cand_from(&v) {
                match classify(std::iter::once(c)) {
                    n @ (Net::Found { .. } | Net::Instrumental { .. }) => return n,
                    Net::NoSynced => got_but_unsynced = true,
                    _ => {}
                }
            }
        }
    }
    let r = match http.get(&search_url(t, artist)) {
        Ok(r) => r,
        Err(_) => return Net::Retry(None),
    };
    if let Some(n) = retry_for(&r) {
        return n;
    }
    let n = if r.status == 200 { pick_search(q, artist, &r.body) } else { Net::NotFound };
    match n {
        Net::NotFound if got_but_unsynced => Net::NoSynced,
        n => n,
    }
}

/// Full network lookup, trying the first-artist spelling when the full artist string finds nothing.
pub fn lookup(http: &dyn Http, q: &Query) -> Net {
    if q.cleaned.title.is_empty() {
        return Net::NotFound;
    }
    let mut res = lookup_artist(http, q, &q.cleaned.artist);
    if matches!(res, Net::NotFound | Net::NoSynced) {
        if let Some(alt) = &q.cleaned.alt_artist {
            let r2 = lookup_artist(http, q, alt);
            if !matches!(r2, Net::NotFound) {
                res = r2;
            }
        }
    }
    res
}

// ================================================================= cache

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct CacheEntry {
    /// "found" | "notfound" | "nosynced" | "instrumental"
    pub status: String,
    pub saved_at: i64,
    pub lrclib_id: Option<i64>,
    pub track_name: String,
    pub artist_name: String,
    pub lrc: Option<String>,
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `<app folder>\lyrics` (COOLERSCREEN_HOME respected through `core::paths::base`).
pub fn default_dir() -> PathBuf {
    crate::core::paths::base().join("lyrics")
}

fn cache_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.json"))
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

/// A usable cached result ("not found" / "no synced" ones only for 7 days).
pub fn cache_read(dir: &Path, key: &str, now: i64) -> Option<CacheEntry> {
    let e: CacheEntry = serde_json::from_slice(&std::fs::read(cache_path(dir, key)).ok()?).ok()?;
    match e.status.as_str() {
        "found" | "instrumental" => Some(e),
        "notfound" | "nosynced" if now - e.saved_at < NOT_FOUND_TTL => Some(e),
        _ => None,
    }
}

pub fn cache_write(dir: &Path, key: &str, e: &CacheEntry) {
    if let Ok(b) = serde_json::to_vec(e) {
        let _ = write_atomic(&cache_path(dir, key), &b);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Sidecar,
    Lrclib,
}

#[derive(Debug)]
pub struct Lyrics {
    pub lines: Vec<Line>,
    pub source: Source,
    pub lrclib_id: Option<i64>,
    pub track_name: String,
    pub artist_name: String,
}

#[derive(Clone, Debug)]
pub enum State {
    Loading,
    Found(Arc<Lyrics>),
    /// Nothing found (quiet fallback).
    NotFound,
    /// The track exists but has no timed lyrics.
    NoSynced,
    Instrumental,
    /// Lyrics are switched off (and nothing local exists).
    Off,
}

fn entry_to_state(e: &CacheEntry, duration_s: f64) -> State {
    match e.status.as_str() {
        "found" => {
            let lines = parse_lrc(e.lrc.as_deref().unwrap_or(""), duration_s);
            if lines.is_empty() {
                State::NotFound
            } else {
                State::Found(Arc::new(Lyrics { lines, source: Source::Lrclib, lrclib_id: e.lrclib_id, track_name: e.track_name.clone(), artist_name: e.artist_name.clone() }))
            }
        }
        "instrumental" => State::Instrumental,
        "nosynced" => State::NoSynced,
        _ => State::NotFound,
    }
}

fn net_to_entry(n: &Net, now: i64) -> Option<(CacheEntry, ())> {
    let base = CacheEntry { saved_at: now, ..Default::default() };
    Some((
        match n {
            Net::Found { id, track, artist, lrc } => CacheEntry { status: "found".into(), lrclib_id: *id, track_name: track.clone(), artist_name: artist.clone(), lrc: Some(lrc.clone()), ..base },
            Net::NoSynced => CacheEntry { status: "nosynced".into(), ..base },
            Net::Instrumental { id } => CacheEntry { status: "instrumental".into(), lrclib_id: *id, ..base },
            Net::NotFound => CacheEntry { status: "notfound".into(), ..base },
            Net::Retry(_) => return None,
        },
        (),
    ))
}

// ================================================================= sidecar

/// `<artist> - <title>.lrc` or `<title>.lrc` in `dir` (case-insensitive; cleaned and original names).
pub fn find_sidecar(dir: &Path, q: &Query) -> Option<PathBuf> {
    let mut want: Vec<String> = Vec::new();
    for (t, a) in [(&q.cleaned.title, &q.cleaned.artist), (&q.raw_title, &q.raw_artist)] {
        if t.is_empty() {
            continue;
        }
        if !a.is_empty() {
            want.push(format!("{a} - {t}.lrc").to_lowercase());
        }
        want.push(format!("{t}.lrc").to_lowercase());
    }
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let name = e.file_name().to_string_lossy().to_lowercase();
        if want.contains(&name) && e.path().is_file() {
            return Some(e.path());
        }
    }
    None
}

pub fn read_sidecar(path: &Path, duration_s: f64) -> Option<Vec<Line>> {
    let md = std::fs::metadata(path).ok()?;
    if md.len() as usize > MAX_BYTES {
        return None;
    }
    let lines = parse_lrc(&String::from_utf8_lossy(&std::fs::read(path).ok()?), duration_s);
    (!lines.is_empty()).then_some(lines)
}

// ================================================================= offsets

fn offsets_path(dir: &Path) -> PathBuf {
    dir.join("offsets.json")
}

fn read_offsets(dir: &Path) -> HashMap<String, f64> {
    std::fs::read(offsets_path(dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Per-track offset in seconds (added to the playback position before lookup); 0 when none is stored.
pub fn get_offset(dir: &Path, key: &str) -> f64 {
    read_offsets(dir).get(key).copied().unwrap_or(0.0)
}

/// Store a per-track offset (clamped to -10..+10 s, rounded to 10 ms); 0 removes the entry. Returns the stored value.
pub fn set_offset(dir: &Path, key: &str, secs: f64) -> f64 {
    let v = if secs.is_finite() { (secs.clamp(-10.0, 10.0) * 100.0).round() / 100.0 } else { 0.0 };
    let mut m = read_offsets(dir);
    if v == 0.0 {
        m.remove(key);
    } else {
        m.insert(key.to_string(), v);
    }
    if let Ok(b) = serde_json::to_vec(&m) {
        let _ = write_atomic(&offsets_path(dir), &b);
    }
    v
}

// ================================================================= resolve (one track, no threads)

/// Sidecar, then cache. `None` = needs the network.
pub fn resolve_local(dir: &Path, sidecar_dir: Option<&Path>, q: &Query, now: i64) -> Option<State> {
    if let Some(sd) = sidecar_dir {
        if let Some(p) = find_sidecar(sd, q) {
            if let Some(lines) = read_sidecar(&p, q.duration_s) {
                return Some(State::Found(Arc::new(Lyrics { lines, source: Source::Sidecar, lrclib_id: None, track_name: q.cleaned.title.clone(), artist_name: q.cleaned.artist.clone() })));
            }
        }
    }
    cache_read(dir, &q.key, now).map(|e| entry_to_state(&e, q.duration_s))
}

/// Network step with caching of definite answers. Returns `Err(retry_after)` for transient failures.
pub fn resolve_net(dir: &Path, http: &dyn Http, q: &Query, now: i64) -> Result<State, Option<u64>> {
    let n = lookup(http, q);
    if let Net::Retry(r) = n {
        return Err(r);
    }
    if let Some((e, _)) = net_to_entry(&n, now) {
        cache_write(dir, &q.key, &e);
        return Ok(entry_to_state(&e, q.duration_s));
    }
    Ok(State::NotFound)
}

// ================================================================= loader

struct Job {
    q: Query,
    not_before: Instant,
}

struct Inner {
    states: HashMap<String, State>,
    order: Vec<String>,
    pending: Vec<Job>,
    last_used: Instant,
    running: bool,
    backoff: u64,
    blocked_until: Option<Instant>,
    last_sig: Option<((u64, String, String, i64), String)>,
}

struct Shared {
    dir: PathBuf,
    http: Arc<dyn Http>,
    cfg: Mutex<(bool, Option<PathBuf>)>,
    inner: Mutex<Inner>,
}

/// Owned by the renderer side. `request` is cheap and may be called every frame.
#[derive(Clone)]
pub struct LyricsLoader {
    s: Arc<Shared>,
}

impl LyricsLoader {
    pub fn new(dir: PathBuf) -> LyricsLoader {
        LyricsLoader::with_http(dir, Arc::new(ReqwestHttp))
    }
    pub fn with_http(dir: PathBuf, http: Arc<dyn Http>) -> LyricsLoader {
        LyricsLoader {
            s: Arc::new(Shared {
                dir,
                http,
                cfg: Mutex::new((false, None)),
                inner: Mutex::new(Inner { states: HashMap::new(), order: Vec::new(), pending: Vec::new(), last_used: Instant::now(), running: false, backoff: BACKOFF_START, blocked_until: None, last_sig: None }),
            }),
        }
    }
    pub fn dir(&self) -> &Path {
        &self.s.dir
    }

    /// Opt-in switch and optional sidecar folder. Turning things on re-checks tracks that were "off" / "not found".
    pub fn configure(&self, enabled: bool, sidecar_dir: Option<PathBuf>) {
        let changed = {
            let mut c = self.s.cfg.lock().unwrap();
            let ch = *c != (enabled, sidecar_dir.clone());
            *c = (enabled, sidecar_dir);
            ch
        };
        if changed {
            let mut g = self.s.inner.lock().unwrap();
            g.states.retain(|_, s| matches!(s, State::Loading | State::Found(_) | State::Instrumental));
            let Inner { states, order, .. } = &mut *g;
            order.retain(|k| states.contains_key(k));
            g.last_sig = None;
        }
    }

    /// Note the playing track (dedupes by track) and return its key; "" when nothing plays.
    pub fn request(&self, np: &NowPlaying) -> String {
        if np.title.is_empty() {
            return String::new();
        }
        let sig = (np.track_id, np.title.clone(), np.artist.clone(), np.duration_s.round() as i64);
        let mut g = self.s.inner.lock().unwrap();
        g.last_used = Instant::now();
        let key = match &g.last_sig {
            Some((s, k)) if *s == sig => k.clone(),
            _ => {
                let q = Query::from_np(np);
                let k = q.key.clone();
                g.last_sig = Some((sig, k.clone()));
                if !g.states.contains_key(&k) {
                    g.states.insert(k.clone(), State::Loading);
                    g.order.push(k.clone());
                    g.pending.push(Job { q, not_before: Instant::now() });
                    if g.pending.len() > MAX_PENDING {
                        let old = g.pending.remove(0);
                        g.states.remove(&old.q.key);
                        g.order.retain(|x| *x != old.q.key);
                    }
                    while g.order.len() > MAX_STATES {
                        let Some(i) = g.order.iter().position(|k| !matches!(g.states.get(k), Some(State::Loading))) else { break };
                        let k = g.order.remove(i);
                        g.states.remove(&k);
                    }
                }
                k
            }
        };
        if !g.running && !g.pending.is_empty() {
            g.running = true;
            let me = self.clone();
            std::thread::spawn(move || me.run());
        }
        key
    }

    pub fn get(&self, key: &str) -> State {
        match self.s.inner.lock().unwrap().states.get(key) {
            Some(s) => s.clone(),
            None if self.s.cfg.lock().unwrap().0 => State::Loading,
            None => State::Off,
        }
    }

    /// Forget a track's result (memory and disk), e.g. a wrong match the user cleared; it is looked up again on request.
    pub fn clear_track(&self, key: &str) {
        let _ = std::fs::remove_file(cache_path(&self.s.dir, key));
        let mut g = self.s.inner.lock().unwrap();
        g.states.remove(key);
        g.order.retain(|k| k != key);
        g.last_sig = None;
    }

    fn set_state(&self, key: &str, st: State) {
        let mut g = self.s.inner.lock().unwrap();
        if g.states.contains_key(key) {
            g.states.insert(key.to_string(), st);
        }
    }

    fn run(&self) {
        loop {
            let (job, enabled, sidecar) = {
                let mut g = self.s.inner.lock().unwrap();
                if g.last_used.elapsed() > KEEP {
                    g.running = false;
                    return;
                }
                let now = Instant::now();
                let i = g.pending.iter().rposition(|j| j.not_before <= now);
                let cfg = self.s.cfg.lock().unwrap().clone();
                match i {
                    Some(i) => (Some(g.pending.remove(i)), cfg.0, cfg.1),
                    None => (None, cfg.0, cfg.1),
                }
            };
            let Some(job) = job else {
                std::thread::sleep(Duration::from_millis(200));
                continue;
            };
            let now = now_unix();
            if let Some(st) = resolve_local(&self.s.dir, sidecar.as_deref(), &job.q, now) {
                self.set_state(&job.q.key, st);
                continue;
            }
            if !enabled {
                self.set_state(&job.q.key, State::Off);
                continue;
            }
            let blocked = self.s.inner.lock().unwrap().blocked_until.filter(|b| *b > Instant::now());
            if let Some(b) = blocked {
                self.s.inner.lock().unwrap().pending.push(Job { q: job.q, not_before: b });
                continue;
            }
            match resolve_net(&self.s.dir, &*self.s.http, &job.q, now) {
                Ok(st) => {
                    let mut g = self.s.inner.lock().unwrap();
                    g.backoff = BACKOFF_START;
                    drop(g);
                    self.set_state(&job.q.key, st);
                }
                Err(ra) => {
                    let mut g = self.s.inner.lock().unwrap();
                    let wait = match ra {
                        Some(s) => s.clamp(1, BACKOFF_MAX),
                        None => {
                            let w = g.backoff;
                            g.backoff = (g.backoff * 2).min(BACKOFF_MAX);
                            w
                        }
                    };
                    let until = Instant::now() + Duration::from_secs(wait);
                    g.blocked_until = Some(until);
                    g.pending.push(Job { q: job.q, not_before: until });
                }
            }
        }
    }
}

// ================================================================= clock

/// Smooth playback position from irregular player reports.
#[derive(Clone, Debug)]
pub struct LyricsClock {
    anchor_pos: f64,
    anchor_at: Instant,
    playing: bool,
    track_id: u64,
    started: bool,
}

/// Re-anchor when the reported position differs from our extrapolation by more than this (a seek).
pub const SEEK_JUMP_S: f64 = 1.5;

impl Default for LyricsClock {
    fn default() -> Self {
        LyricsClock::new()
    }
}

impl LyricsClock {
    pub fn new() -> LyricsClock {
        LyricsClock { anchor_pos: 0.0, anchor_at: Instant::now(), playing: false, track_id: 0, started: false }
    }

    /// Feed a report at time `now`; returns the smooth position. Re-anchors on a new track, a play/pause change,
    /// any report while paused (a seek while paused), or a jump of more than 1.5 s from our own extrapolation.
    pub fn feed(&mut self, now: Instant, reported_pos: f64, playing: bool, track_id: u64) -> f64 {
        let ours = self.at(now);
        let reanchor = !self.started || track_id != self.track_id || playing != self.playing || !playing || (reported_pos - ours).abs() > SEEK_JUMP_S;
        if reanchor {
            self.anchor_pos = reported_pos;
            self.anchor_at = now;
            self.playing = playing;
            self.track_id = track_id;
            self.started = true;
        }
        self.at(now)
    }

    pub fn feed_np(&mut self, np: &NowPlaying) -> f64 {
        self.feed(Instant::now(), np.position(), np.playing, np.track_id)
    }

    /// Position at `now` without a new report.
    pub fn at(&self, now: Instant) -> f64 {
        if self.playing {
            self.anchor_pos + now.saturating_duration_since(self.anchor_at).as_secs_f64()
        } else {
            self.anchor_pos
        }
    }
}

// ================================================================= tests

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(s: &str) -> Vec<Line> {
        parse_lrc(s, 0.0)
    }

    #[test]
    fn parses_basic_and_multi_tags_sorted() {
        let l = lines("[ti:Song]\n[ar:Me]\n[00:10.50]second\n[00:01.5][00:20.00]chorus\n[00:05]first\nno tag line\n");
        let t: Vec<_> = l.iter().map(|x| (x.start_s, x.text.as_str())).collect();
        assert_eq!(t, vec![(1.5, "chorus"), (5.0, "first"), (10.5, "second"), (20.0, "chorus")]);
        assert_eq!(l[0].end_s, 5.0);
        assert_eq!(l[3].end_s, 25.0);
    }

    #[test]
    fn three_digit_fraction_and_colon_fraction() {
        let l = lines("[01:02.345]a\n[01:03:50]b");
        assert!((l[0].start_s - 62.345).abs() < 1e-9);
        assert!((l[1].start_s - 63.5).abs() < 1e-9);
    }

    #[test]
    fn offset_tag_positive_means_earlier() {
        let l = lines("[offset:+500]\n[00:10.00]a\n[00:20.00]b");
        assert_eq!(l[0].start_s, 9.5);
        let l = lines("[offset:-1000]\n[00:10.00]a");
        assert_eq!(l[0].start_s, 11.0);
    }

    #[test]
    fn word_tags_stripped_blank_lines_kept() {
        let l = lines("[00:01.00]<00:01.00>Hello <00:01.50>world\n[00:04.00]\n[00:09.00]x < 3 y");
        assert_eq!(l[0].text, "Hello world");
        assert_eq!(l[1].text, "");
        assert_eq!(l[2].text, "x < 3 y");
    }

    #[test]
    fn last_line_end_uses_duration() {
        let l = parse_lrc("[00:10.00]a", 200.0);
        assert_eq!(l[0].end_s, 200.0);
        let l = parse_lrc("[00:10.00]a", 5.0);
        assert_eq!(l[0].end_s, 15.0);
        assert!(parse_lrc("garbage\n[ar:x]", 0.0).is_empty());
    }

    #[test]
    fn locate_boundaries() {
        let l = lines("[00:10.00]one\n[00:14.00]two\n[00:18.00]three");
        let p = locate(&l, 5.0);
        assert_eq!(p.index, None);
        assert!(p.gap);
        assert_eq!(p.next_in, Some(5.0));
        let p = locate(&l, 10.0);
        assert_eq!(p.index, Some(0));
        assert_eq!(p.progress, 0.0);
        let p = locate(&l, 12.0);
        assert_eq!(p.index, Some(0));
        assert!((p.progress - 0.5).abs() < 1e-9);
        let p = locate(&l, 14.0);
        assert_eq!(p.index, Some(1));
        let p = locate(&l, 100.0);
        assert_eq!(p.index, Some(2));
        assert_eq!(p.next_in, None);
        assert_eq!(locate(&[], 3.0).index, None);
    }

    #[test]
    fn gap_detection() {
        let l = lines("[00:10.00]short line\n[00:12.00]\n[00:40.00]after long break\n[01:00.00]next");
        // empty line = gap
        assert!(locate(&l, 13.0).gap);
        // line followed by a long silence: sung part (natural 4.2 s), then gap
        let p = locate(&l, 41.0);
        assert!(!p.gap);
        let p = locate(&l, 50.0);
        assert!(p.gap);
        assert_eq!(p.progress, 1.0);
        // short slot: never a gap
        let l = lines("[00:10.00]a\n[00:14.00]b");
        assert!(!locate(&l, 13.9).gap);
    }

    #[test]
    fn title_cleaning() {
        let cases = [
            ("Dave - Raindance (Official Video)", "Dave", "Raindance", "Dave"),
            ("Raindance [Official Audio]", "Dave", "Raindance", "Dave"),
            ("Raindance (Lyrics)", "Dave - Topic", "Raindance", "Dave"),
            ("Raindance (Lyric Video)", "DaveVEVO", "Raindance", "Dave"),
            ("Song (Visualizer)", "Band", "Song", "Band"),
            ("Song (HD)", "Band", "Song", "Band"),
            ("Song (4K)", "Band", "Song", "Band"),
            ("Song (Remastered 2011)", "Band", "Song", "Band"),
            ("Song - 2011 Remaster", "Band", "Song", "Band"),
            ("Song - Remastered", "Band", "Song", "Band"),
            ("Song (feat. Other Guy)", "Band", "Song", "Band"),
            ("Song ft. Other Guy", "Band", "Song", "Band"),
            ("Song (Official Music Video)", "Band", "Song", "Band"),
            ("Band - Song [Official Video]", "BandVEVO", "Song", "Band"),
            ("Some Artist - Great Song | Official Video", "Some Artist - Topic", "Great Song", "Some Artist"),
            ("Song (Acoustic)", "Band", "Song (Acoustic)", "Band"),
            ("Song (Clean Bandit Remix)", "Band", "Song (Clean Bandit Remix)", "Band"),
            ("Song - Live", "Band", "Song - Live", "Band"),
            ("Band – Song", "Some Channel", "Band - Song", "Some Channel"),
        ];
        for (t, a, et, ea) in cases {
            let c = clean_track(t, a);
            assert_eq!((c.title.as_str(), c.artist.as_str()), (et, ea), "{t} / {a}");
        }
    }

    #[test]
    fn packed_title_needs_matching_artist() {
        let c = clean_track("Dave - Raindance", "Dave");
        assert_eq!((c.title.as_str(), c.artist.as_str()), ("Raindance", "Dave"));
        let c = clean_track("Black Sabbath - Paranoid", "Black Sabbath Official");
        assert_eq!((c.title.as_str(), c.artist.as_str()), ("Paranoid", "Black Sabbath"));
    }

    #[test]
    fn artist_fallback() {
        assert_eq!(first_artist("Dave, Fredo"), Some("Dave".into()));
        assert_eq!(first_artist("A & B"), Some("A".into()));
        assert_eq!(first_artist("A x B"), Some("A".into()));
        assert_eq!(first_artist("Dave"), None);
        let c = clean_track("Song", "Dave, Fredo");
        assert_eq!(c.alt_artist.as_deref(), Some("Dave"));
    }

    #[test]
    fn titles_match_rules() {
        assert!(titles_match("Don't Stop", "dont stop!"));
        assert!(titles_match("Song Title Here", "Song Title Here X"));
        assert!(!titles_match("Song", "Song Of The Century"));
        assert!(!titles_match("", ""));
    }

    fn search_body() -> String {
        serde_json::json!([
            {"id": 1, "trackName": "Raindance (Live)", "artistName": "Dave", "duration": 300.0, "instrumental": false, "syncedLyrics": "[00:01.00]live"},
            {"id": 2, "trackName": "Raindance", "artistName": "Dave", "duration": 221.0, "instrumental": false, "plainLyrics": "plain", "syncedLyrics": null},
            {"id": 3, "trackName": "Raindance", "artistName": "Dave, Tems", "duration": 222.5, "instrumental": false, "syncedLyrics": "[00:01.00]hello"},
            {"id": 4, "trackName": "Other Song", "artistName": "Dave", "duration": 221.0, "syncedLyrics": "[00:01.00]no"}
        ])
        .to_string()
    }

    #[test]
    fn search_accepts_and_rejects() {
        let q = Query::new("Raindance (Official Video)", "Dave", 221.0);
        match pick_search(&q, "Dave", &search_body()) {
            Net::Found { id, .. } => assert_eq!(id, Some(3)),
            n => panic!("{n:?}"),
        }
        // duration too far
        let q = Query::new("Raindance", "Dave", 240.0);
        assert_eq!(pick_search(&q, "Dave", &search_body()), Net::NotFound);
        // only unsynced match
        let b = serde_json::json!([{"id": 2, "trackName": "Raindance", "artistName": "Dave", "duration": 221.0, "plainLyrics": "p", "syncedLyrics": ""}]).to_string();
        let q = Query::new("Raindance", "Dave", 220.0);
        assert_eq!(pick_search(&q, "Dave", &b), Net::NoSynced);
        // instrumental
        let b = serde_json::json!([{"id": 9, "trackName": "Raindance", "artistName": "Dave", "duration": 221.0, "instrumental": true}]).to_string();
        assert_eq!(pick_search(&q, "Dave", &b), Net::Instrumental { id: Some(9) });
        // unknown duration: exact title and artist only
        let q0 = Query::new("Raindance", "Dave", 0.0);
        match pick_search(&q0, "Dave", &search_body()) {
            Net::NoSynced => {} // id 2 is the exact one and has no synced lyrics; id 3's artist differs
            n => panic!("{n:?}"),
        }
        let q0 = Query::new("Raindance", "Someone Else", 0.0);
        assert_eq!(pick_search(&q0, "Someone Else", &search_body()), Net::NotFound);
    }

    struct Fake {
        calls: Mutex<Vec<String>>,
        f: Box<dyn Fn(&str) -> Result<HttpReply, String> + Send + Sync>,
    }
    impl Http for Fake {
        fn get(&self, url: &str) -> Result<HttpReply, String> {
            self.calls.lock().unwrap().push(url.to_string());
            (self.f)(url)
        }
    }
    fn fake(f: impl Fn(&str) -> Result<HttpReply, String> + Send + Sync + 'static) -> Arc<Fake> {
        Arc::new(Fake { calls: Mutex::new(Vec::new()), f: Box::new(f) })
    }
    fn reply(status: u16, body: &str) -> Result<HttpReply, String> {
        Ok(HttpReply { status, retry_after: None, body: body.to_string() })
    }

    #[test]
    fn lookup_get_then_search_and_urls() {
        let h = fake(|u| {
            if u.contains("/api/get") {
                reply(404, "{}")
            } else {
                reply(200, &search_body())
            }
        });
        let q = Query::new("Raindance (Official Video)", "Dave - Topic", 221.4);
        let n = lookup(&*h, &q);
        assert!(matches!(n, Net::Found { id: Some(3), .. }), "{n:?}");
        let c = h.calls.lock().unwrap();
        assert_eq!(c[0], "https://lrclib.net/api/get?track_name=Raindance&artist_name=Dave&duration=221");
        assert!(c[1].starts_with("https://lrclib.net/api/search?track_name=Raindance&artist_name=Dave"));
    }

    #[test]
    fn lookup_get_hit_and_instrumental_and_503() {
        let body = r#"{"id":5,"trackName":"A","artistName":"B","duration":100,"instrumental":false,"syncedLyrics":"[00:01.00]x"}"#;
        let h = fake(move |_| reply(200, body));
        assert!(matches!(lookup(&*h, &Query::new("A", "B", 100.0)), Net::Found { id: Some(5), .. }));
        assert_eq!(h.calls.lock().unwrap().len(), 1);
        let h = fake(|_| reply(200, r#"{"id":6,"trackName":"A","artistName":"B","duration":100,"instrumental":true,"syncedLyrics":null}"#));
        assert_eq!(lookup(&*h, &Query::new("A", "B", 100.0)), Net::Instrumental { id: Some(6) });
        let h = fake(|_| Ok(HttpReply { status: 503, retry_after: Some(7), body: String::new() }));
        assert_eq!(lookup(&*h, &Query::new("A", "B", 100.0)), Net::Retry(Some(7)));
        let h = fake(|_| Err("down".into()));
        assert_eq!(lookup(&*h, &Query::new("A", "B", 100.0)), Net::Retry(None));
    }

    #[test]
    fn lookup_tries_first_artist() {
        let h = fake(|u| if u.contains("artist_name=Dave&") || u.ends_with("artist_name=Dave") || u.contains("artist_name=Dave&duration") { reply(200, r#"{"id":7,"trackName":"S","artistName":"Dave","duration":50,"syncedLyrics":"[00:01.00]x"}"#) } else { reply(404, "[]") });
        let q = Query::new("S", "Dave, Fredo", 50.0);
        assert!(matches!(lookup(&*h, &q), Net::Found { id: Some(7), .. }));
    }

    #[test]
    fn cache_hit_and_expiry() {
        let d = tempfile::tempdir().unwrap();
        let h = fake(|_| reply(404, "[]"));
        let q = Query::new("Nothing", "Nobody", 100.0);
        let t0 = 1_000_000;
        assert!(resolve_local(d.path(), None, &q, t0).is_none());
        assert!(matches!(resolve_net(d.path(), &*h, &q, t0), Ok(State::NotFound)));
        let n = h.calls.lock().unwrap().len();
        assert!(matches!(resolve_local(d.path(), None, &q, t0 + 3600), Some(State::NotFound)));
        assert_eq!(h.calls.lock().unwrap().len(), n);
        assert!(resolve_local(d.path(), None, &q, t0 + NOT_FOUND_TTL + 1).is_none());
        // found never expires
        let h = fake(|_| reply(200, r#"{"id":5,"trackName":"A","artistName":"B","duration":100,"syncedLyrics":"[00:01.00]x"}"#));
        let q = Query::new("A", "B", 100.0);
        assert!(matches!(resolve_net(d.path(), &*h, &q, t0), Ok(State::Found(_))));
        match resolve_local(d.path(), None, &q, t0 + 10 * NOT_FOUND_TTL) {
            Some(State::Found(l)) => {
                assert_eq!(l.lrclib_id, Some(5));
                assert_eq!(l.lines[0].text, "x");
            }
            _ => panic!(),
        }
        // transient failure caches nothing
        let h = fake(|_| reply(503, ""));
        let q = Query::new("Z", "Y", 100.0);
        assert!(resolve_net(d.path(), &*h, &q, t0).is_err());
        assert!(resolve_local(d.path(), None, &q, t0).is_none());
    }

    #[test]
    fn sidecar_lookup_wins_over_cache() {
        let d = tempfile::tempdir().unwrap();
        let s = tempfile::tempdir().unwrap();
        std::fs::write(s.path().join("DAVE - Raindance.LRC"), "\u{feff}[00:01.00]local line\n").unwrap();
        let q = Query::new("Raindance (Official Video)", "Dave - Topic", 221.0);
        match resolve_local(d.path(), Some(s.path()), &q, 0) {
            Some(State::Found(l)) => {
                assert_eq!(l.source, Source::Sidecar);
                assert_eq!(l.lines[0].text, "local line");
            }
            _ => panic!(),
        }
        std::fs::write(s.path().join("Raw Title.lrc"), "[00:02.00]raw\n").unwrap();
        let q = Query::new("Raw Title", "", 0.0);
        assert!(find_sidecar(s.path(), &q).is_some());
        assert!(find_sidecar(s.path(), &Query::new("Missing", "X", 0.0)).is_none());
    }

    #[test]
    fn offsets_round_trip() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(get_offset(d.path(), "k"), 0.0);
        assert_eq!(set_offset(d.path(), "k", 1.234), 1.23);
        assert_eq!(set_offset(d.path(), "j", -50.0), -10.0);
        assert_eq!(get_offset(d.path(), "k"), 1.23);
        assert_eq!(get_offset(d.path(), "j"), -10.0);
        set_offset(d.path(), "k", 0.0);
        assert_eq!(get_offset(d.path(), "k"), 0.0);
        assert_eq!(get_offset(d.path(), "j"), -10.0);
    }

    #[test]
    fn clock_extrapolates_seeks_and_pauses() {
        let t0 = Instant::now();
        let at = |s: f64| t0 + Duration::from_secs_f64(s);
        let mut c = LyricsClock::new();
        assert_eq!(c.feed(at(0.0), 10.0, true, 1), 10.0);
        // irregular, slightly-off reports do not move us
        assert!((c.feed(at(2.0), 12.4, true, 1) - 12.0).abs() < 1e-9);
        assert!((c.feed(at(3.0), 12.8, true, 1) - 13.0).abs() < 1e-9);
        // seek
        assert!((c.feed(at(4.0), 60.0, true, 1) - 60.0).abs() < 1e-9);
        assert!((c.feed(at(5.0), 61.0, true, 1) - 61.0).abs() < 1e-9);
        // pause freezes at the reported position
        assert!((c.feed(at(6.0), 62.0, false, 1) - 62.0).abs() < 1e-9);
        assert!((c.feed(at(9.0), 62.0, false, 1) - 62.0).abs() < 1e-9);
        // seek while paused
        assert!((c.feed(at(10.0), 5.0, false, 1) - 5.0).abs() < 1e-9);
        // resume
        assert!((c.feed(at(11.0), 5.0, true, 1) - 5.0).abs() < 1e-9);
        assert!((c.feed(at(12.0), 6.0, true, 1) - 6.0).abs() < 1e-9);
        // new track
        assert!((c.feed(at(13.0), 0.0, true, 2) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn loader_end_to_end_with_fake_http() {
        let d = tempfile::tempdir().unwrap();
        let h = fake(|_| reply(200, r#"{"id":5,"trackName":"A","artistName":"B","duration":100,"syncedLyrics":"[00:01.00]x"}"#));
        let l = LyricsLoader::with_http(d.path().to_path_buf(), h.clone());
        let np = NowPlaying::demo("A", "B", true, 3.0, 100.0);
        // disabled: no network
        let k = l.request(&np);
        let mut st = l.get(&k);
        for _ in 0..50 {
            if !matches!(st, State::Loading) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            st = l.get(&k);
        }
        assert!(matches!(st, State::Off), "{st:?}");
        assert_eq!(h.calls.lock().unwrap().len(), 0);
        // enable: looked up once, found
        l.configure(true, None);
        let k = l.request(&np);
        let mut st = l.get(&k);
        for _ in 0..100 {
            if !matches!(st, State::Loading) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
            st = l.get(&k);
        }
        assert!(matches!(st, State::Found(_)), "{st:?}");
        for _ in 0..20 {
            l.request(&np);
        }
        assert_eq!(h.calls.lock().unwrap().len(), 1);
    }

    /// Run once by hand: `cargo test --release --lib lyrics::tests::live_lrclib -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn live_lrclib() {
        for (t, a, d) in [("Raindance", "Dave", 221.0), ("Blinding Lights", "The Weeknd", 200.0), ("Bohemian Rhapsody", "Queen", 355.0)] {
            let q = Query::new(t, a, d);
            let url = get_url(&q.cleaned.title, &q.cleaned.artist, q.duration_s);
            let r = ReqwestHttp.get(&url).unwrap();
            println!("{t}: status {}", r.status);
            let Ok(v) = serde_json::from_str::<Value>(&r.body) else { continue };
            println!("fields {:?}", v.as_object().map(|o| o.keys().collect::<Vec<_>>()));
            let lines = parse_lrc(v["syncedLyrics"].as_str().unwrap_or(""), d);
            println!("parsed {} lines", lines.len());
            if lines.is_empty() {
                continue;
            }
        for l in lines.iter().take(3) {
            println!("{:.2} -> {:.2}: {}", l.start_s, l.end_s, l.text);
        }
            break;
        }
    }
}
