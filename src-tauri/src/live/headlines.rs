//! 4.0 W2 (headlines widget): props, cleaning, the (opt-in, network) data loader and its parsing.
//! The widget shows the latest titles of an RSS or Atom feed whose link the USER pasted. Nothing else is fetched: one GET of that
//! link every 15 minutes, only while a headlines widget is drawn (the loader thread ends 30 s after the last draw).
//! The parser is a small tolerant scanner (no XML crate): RSS 2.0 / RSS 1.0 and Atom, CDATA, entities, HTML inside titles.
use crate::core::sync::MutexExt;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::morewidgets::days_from_civil;
use super::theme::Widget;

const REFRESH: Duration = Duration::from_secs(15 * 60);
/// After a failed fetch the next try comes sooner than a normal refresh.
const RETRY: Duration = Duration::from_secs(5 * 60);
const KEEP: Duration = Duration::from_secs(30);
const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 200;
const MAX_TITLE: usize = 200;
const MAX_TEXT: usize = 16 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HeadlinesProps {
    pub source: String,
    pub count: i64,
    pub show_source: bool,
    pub color: String,
}
impl Default for HeadlinesProps {
    fn default() -> Self {
        HeadlinesProps { source: String::new(), count: 4, show_source: true, color: "#ffffff".into() }
    }
}

impl Widget {
    pub fn headlines(&self) -> HeadlinesProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`).
pub fn clean(o: &mut Map<String, Value>) {
    use super::morewidgets::{boolean, colour, int, short};
    short(o, "source", 500, false);
    let s = o.get("source").and_then(Value::as_str).unwrap_or("").to_string();
    o.insert("source".into(), Value::String(valid_source(&s).unwrap_or_default()));
    int(o, "count", 4, 3, 5);
    boolean(o, "showSource", true);
    colour(o, "color", "#ffffff", false);
}

/// The feed link when it is usable: http or https only, no credentials, no spaces, at most 500 characters.
pub fn valid_source(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > 500 || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    let rest = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://"))?;
    let auth_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let auth = &rest[..auth_end];
    if auth.is_empty() || auth.contains('@') || auth.contains('\\') {
        return None;
    }
    let host = host_part(auth);
    if host.is_empty() || host.starts_with('.') {
        return None;
    }
    if let Some((_, port)) = auth.rsplit_once(':') {
        if !auth.ends_with(']') && !port.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
    }
    Some(s.to_string())
}

fn host_part(auth: &str) -> &str {
    if auth.starts_with('[') {
        return auth.split(']').next().unwrap_or("");
    }
    auth.split(':').next().unwrap_or("")
}

/// The host name of a feed link (fallback for the "source name" when the feed has no title).
pub fn host_of(source: &str) -> String {
    let s = source.trim();
    let rest = s.split_once("://").map(|x| x.1).unwrap_or(s);
    let auth = rest.split(['/', '?', '#']).next().unwrap_or("");
    let h = host_part(auth).to_string();
    h.strip_prefix("www.").map(str::to_string).unwrap_or(h)
}

// ---------------------------------------------------------------- data

#[derive(Clone, Debug, PartialEq)]
pub struct Headline {
    pub title: String,
    /// UTC seconds since 1970, when the entry has a readable date.
    pub ts: Option<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Feed {
    pub title: Option<String>,
    pub entries: Vec<Headline>,
    /// A feed element (rss / feed / channel / rdf) was seen.
    pub is_feed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeadlinesView {
    pub entries: Vec<Headline>,
    pub title: Option<String>,
    /// Loaded at least once (successfully or not).
    pub loaded: bool,
    pub error: Option<String>,
}

// ---------------------------------------------------------------- dates

/// Seconds since 1970 (UTC) now.
pub fn now_utc() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn month_num(s: &str) -> Option<i64> {
    let l = s.to_ascii_lowercase();
    ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"].iter().position(|m| l.starts_with(m)).map(|i| i as i64 + 1)
}

fn to_utc(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, offset_min: i64) -> Option<i64> {
    if !(1970..=2200).contains(&y) || !(1..=12).contains(&mo) || !(1..=31).contains(&d) || !(0..=23).contains(&h) || !(0..=59).contains(&mi) || !(0..=60).contains(&s) {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s - offset_min * 60)
}

/// "+1100", "-0430", "GMT", "UT", "Z", "EST" ... as minutes east of UTC.
fn zone_minutes(z: &str) -> Option<i64> {
    let z = z.trim();
    let up = z.to_ascii_uppercase();
    match up.as_str() {
        "" | "GMT" | "UT" | "UTC" | "Z" => return Some(0),
        "EST" => return Some(-300),
        "EDT" => return Some(-240),
        "CST" => return Some(-360),
        "CDT" => return Some(-300),
        "MST" => return Some(-420),
        "MDT" => return Some(-360),
        "PST" => return Some(-480),
        "PDT" => return Some(-420),
        _ => {}
    }
    let (sign, digits) = match z.as_bytes().first()? {
        b'+' => (1, &z[1..]),
        b'-' => (-1, &z[1..]),
        _ => return None,
    };
    let digits: String = digits.chars().filter(|c| *c != ':').collect();
    if digits.len() != 4 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m): (i64, i64) = (digits[..2].parse().ok()?, digits[2..].parse().ok()?);
    if h > 14 || m > 59 {
        return None;
    }
    Some(sign * (h * 60 + m))
}

/// RFC 2822: "Mon, 19 Oct 2026 08:30:00 +1100" (weekday and seconds optional).
pub fn parse_rfc2822(s: &str) -> Option<i64> {
    let s = s.trim();
    let s = s.split_once(',').map(|x| x.1).unwrap_or(s);
    let mut it = s.split_whitespace();
    let d: i64 = it.next()?.parse().ok()?;
    let mo = month_num(it.next()?)?;
    let mut y: i64 = it.next()?.parse().ok()?;
    if y < 50 {
        y += 2000;
    } else if y < 1000 {
        y += 1900;
    }
    let t = it.next().unwrap_or("00:00:00");
    let mut tp = t.split(':');
    let h: i64 = tp.next()?.parse().ok()?;
    let mi: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let sec: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let off = zone_minutes(it.next().unwrap_or("GMT"))?;
    to_utc(y, mo, d, h, mi, sec, off)
}

/// RFC 3339 / ISO 8601: "2026-10-19T08:30:00Z", "...+11:00", fractions allowed; a bare date means midnight UTC.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.len() < 10 || !s.is_char_boundary(10) {
        return None;
    }
    let (date, rest) = s.split_at(10);
    let mut dp = date.split('-');
    let y: i64 = dp.next()?.parse().ok()?;
    let mo: i64 = dp.next()?.parse().ok()?;
    let d: i64 = dp.next()?.parse().ok()?;
    if dp.next().is_some() {
        return None;
    }
    let rest = rest.trim_start_matches(['T', 't', ' ']);
    if rest.is_empty() {
        return to_utc(y, mo, d, 0, 0, 0, 0);
    }
    let split_at = rest.find(['Z', 'z', '+', '-']).unwrap_or(rest.len());
    let (time, zone) = rest.split_at(split_at);
    let time = time.split('.').next().unwrap_or("");
    let mut tp = time.split(':');
    let h: i64 = tp.next()?.parse().ok()?;
    let mi: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let sec: i64 = tp.next().unwrap_or("0").parse().ok()?;
    let off = zone_minutes(zone)?;
    to_utc(y, mo, d, h, mi, sec, off)
}

/// A feed date in either format.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.as_bytes().get(4) == Some(&b'-') {
        parse_rfc3339(s)
    } else {
        parse_rfc2822(s).or_else(|| parse_rfc3339(s))
    }
}

/// "just now", "5 min ago", "3 h ago", "2 d ago" (a time in the future counts as "just now").
pub fn age_text(now: i64, ts: i64) -> String {
    let d = now - ts;
    if d < 60 {
        "just now".into()
    } else if d < 3600 {
        format!("{} min ago", d / 60)
    } else if d < 86400 {
        format!("{} h ago", d / 3600)
    } else {
        format!("{} d ago", d / 86400)
    }
}

// ---------------------------------------------------------------- parsing

enum Tok<'a> {
    Start { name: String, empty: bool },
    End(String),
    Text(&'a str),
    Cdata(&'a str),
}

fn local(name: &str) -> String {
    name.rsplit(':').next().unwrap_or("").to_ascii_lowercase()
}

/// Index of the `>` that closes the tag at the start of `rest` (quotes respected; falls back to the first `>`).
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote: Option<u8> = None;
    for (i, c) in rest.bytes().enumerate() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, b'"') | (None, b'\'') => quote = Some(c),
            (None, b'>') => return Some(i),
            _ => {}
        }
    }
    rest.find('>')
}

fn tokens<'a>(xml: &'a str, mut f: impl FnMut(Tok<'a>) -> bool) {
    let mut i = 0usize;
    while i < xml.len() {
        match xml[i..].find('<') {
            None => {
                f(Tok::Text(&xml[i..]));
                return;
            }
            Some(off) => {
                if off > 0 && !f(Tok::Text(&xml[i..i + off])) {
                    return;
                }
                i += off;
            }
        }
        let rest = &xml[i..];
        if rest.starts_with("<!--") {
            match rest.find("-->") {
                Some(e) => i += e + 3,
                None => return,
            }
            continue;
        }
        if let Some(body) = rest.strip_prefix("<![CDATA[") {
            match body.find("]]>") {
                Some(e) => {
                    if !f(Tok::Cdata(&body[..e])) {
                        return;
                    }
                    i += 9 + e + 3;
                }
                None => {
                    f(Tok::Cdata(body));
                    return;
                }
            }
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") {
            match rest.find('>') {
                Some(e) => i += e + 1,
                None => return,
            }
            continue;
        }
        let next = rest[1..].chars().next();
        if !next.is_some_and(|c| c.is_alphabetic() || c == '/' || c == '_') {
            // a stray "<" in text
            if !f(Tok::Text("<")) {
                return;
            }
            i += 1;
            continue;
        }
        let Some(end) = tag_end(rest) else { return };
        let inner = &rest[1..end];
        let ok = if let Some(n) = inner.strip_prefix('/') {
            f(Tok::End(local(n.split_whitespace().next().unwrap_or(""))))
        } else {
            let empty = inner.ends_with('/');
            let inner = inner.trim_end_matches('/');
            let name = inner.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
            f(Tok::Start { name: local(name), empty })
        };
        if !ok {
            return;
        }
        i += end + 1;
    }
}

/// &amp; &lt; &gt; &quot; &apos; &nbsp; &#N; &#xH; ; anything else is left as it is.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let semi = rest.bytes().take(12).position(|b| b == b';');
        let rep = semi.and_then(|e| {
            let name = &rest[1..e];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => {
                    let num = name.strip_prefix('#')?;
                    let code = if let Some(h) = num.strip_prefix(['x', 'X']) { u32::from_str_radix(h, 16).ok()? } else { num.parse::<u32>().ok()? };
                    char::from_u32(code)
                }
            };
            ch.map(|c| (c, e))
        });
        match rep {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Remove `<tag ...>` runs (a "<" followed by a letter, "/" or "!").
fn strip_tags(s: &str) -> String {
    if !s.contains('<') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('<') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let tag_like = rest[1..].chars().next().is_some_and(|c| c.is_alphabetic() || c == '/' || c == '!');
        match (tag_like, rest.find('>')) {
            (true, Some(e)) => {
                out.push(' ');
                rest = &rest[e + 1..];
            }
            _ => {
                out.push('<');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Plain one-line text of a captured element: tags stripped, entities decoded, whitespace collapsed, bounded.
fn clean_title(s: &str, cdata: bool) -> String {
    let stripped = strip_tags(s);
    let decoded = if cdata { decode_entities(&stripped) } else { stripped };
    let one: String = decoded.chars().filter(|c| !c.is_control() || c.is_whitespace()).collect();
    let words = one.split_whitespace().collect::<Vec<_>>().join(" ");
    if words.chars().count() > MAX_TITLE {
        words.chars().take(MAX_TITLE).collect::<String>().trim_end().to_string()
    } else {
        words
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Target {
    FeedTitle,
    ItemTitle,
    Date(usize),
}

struct Cap {
    target: Target,
    /// stack length including the captured element
    depth: usize,
    text: String,
    cdata: bool,
}

#[derive(Default)]
struct Cur {
    title: Option<String>,
    dates: [Option<i64>; 4],
}

/// Parse an RSS / Atom document. Never panics; bounded (200 entries, 200-character titles).
pub fn parse(xml: &str) -> Feed {
    let mut feed = Feed::default();
    let mut stack: Vec<String> = Vec::new();
    let mut cur: Option<Cur> = None;
    let mut cap: Option<Cap> = None;
    let mut dated: Vec<Option<i64>> = Vec::new();

    fn finish_cap(c: Cap, feed: &mut Feed, cur: &mut Option<Cur>) {
        match c.target {
            Target::FeedTitle => {
                let t = clean_title(&c.text, c.cdata);
                if !t.is_empty() && feed.title.is_none() {
                    feed.title = Some(t);
                }
            }
            Target::ItemTitle => {
                if let Some(it) = cur.as_mut() {
                    let t = clean_title(&c.text, c.cdata);
                    if !t.is_empty() && it.title.is_none() {
                        it.title = Some(t);
                    }
                }
            }
            Target::Date(i) => {
                if let Some(it) = cur.as_mut() {
                    if it.dates[i].is_none() {
                        it.dates[i] = parse_date(&decode_entities(c.text.trim()));
                    }
                }
            }
        }
    }
    fn finish_item(cur: &mut Option<Cur>, feed: &mut Feed, dated: &mut Vec<Option<i64>>) {
        if let Some(it) = cur.take() {
            if let Some(title) = it.title {
                if feed.entries.len() < MAX_ENTRIES {
                    let ts = it.dates.iter().flatten().next().copied();
                    feed.entries.push(Headline { title, ts });
                    dated.push(ts);
                }
            }
        }
    }

    tokens(xml, |tok| {
        if feed.entries.len() >= MAX_ENTRIES {
            return false;
        }
        match tok {
            Tok::Text(t) => {
                if let Some(c) = cap.as_mut() {
                    if c.text.len() < MAX_TEXT {
                        c.text.push_str(&decode_entities(t));
                    }
                }
            }
            Tok::Cdata(t) => {
                if let Some(c) = cap.as_mut() {
                    if c.text.len() < MAX_TEXT {
                        c.text.push_str(t);
                        c.cdata = true;
                    }
                }
            }
            Tok::Start { name, empty } => {
                if matches!(name.as_str(), "rss" | "rdf" | "feed" | "channel") {
                    feed.is_feed = true;
                }
                let parent = stack.last().map(String::as_str).unwrap_or("");
                if let Some(c) = cap.as_mut() {
                    // markup inside a title separates words ("one<br/>two")
                    c.text.push(' ');
                }
                if cap.is_none() && !empty {
                    let in_item = cur.is_some() && matches!(parent, "item" | "entry");
                    let target = match name.as_str() {
                        "title" if in_item => Some(Target::ItemTitle),
                        "title" if cur.is_none() && matches!(parent, "channel" | "feed") => Some(Target::FeedTitle),
                        "pubdate" if in_item => Some(Target::Date(0)),
                        "published" if in_item => Some(Target::Date(1)),
                        "updated" if in_item => Some(Target::Date(2)),
                        "date" if in_item => Some(Target::Date(3)),
                        _ => None,
                    };
                    if let Some(target) = target {
                        cap = Some(Cap { target, depth: stack.len() + 1, text: String::new(), cdata: false });
                    }
                }
                if matches!(name.as_str(), "item" | "entry") && !empty {
                    finish_item(&mut cur, &mut feed, &mut dated);
                    cur = Some(Cur::default());
                }
                if !empty {
                    stack.push(name);
                }
            }
            Tok::End(name) => {
                let Some(idx) = stack.iter().rposition(|n| *n == name) else { return true };
                if let Some(c) = cap.as_mut() {
                    c.text.push(' ');
                }
                if let Some(c) = cap.take() {
                    if c.depth > idx {
                        finish_cap(c, &mut feed, &mut cur);
                    } else {
                        cap = Some(c);
                    }
                }
                if matches!(name.as_str(), "item" | "entry") {
                    finish_item(&mut cur, &mut feed, &mut dated);
                }
                stack.truncate(idx);
            }
        }
        true
    });
    if let Some(c) = cap.take() {
        finish_cap(c, &mut feed, &mut cur);
    }
    finish_item(&mut cur, &mut feed, &mut dated);

    // when every entry has a date, newest first; otherwise the feed's own order
    if !feed.entries.is_empty() && feed.entries.iter().all(|e| e.ts.is_some()) {
        feed.entries.sort_by_key(|e| std::cmp::Reverse(e.ts));
    }
    feed
}

// ---------------------------------------------------------------- loading

struct Slot {
    last_used: Instant,
    running: bool,
    view: HeadlinesView,
}

fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static S: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The latest headlines of `source` (starts or refreshes the loader; never blocks; the first call returns `loaded: false`).
pub fn read(source: &str) -> HeadlinesView {
    let Some(url) = valid_source(source) else {
        return HeadlinesView { entries: Vec::new(), title: None, loaded: true, error: Some("Not a feed link".into()) };
    };
    let mut g = slots().lock_or_recover();
    let s = g.entry(url.clone()).or_insert_with(|| Slot { last_used: Instant::now(), running: false, view: HeadlinesView::default() });
    s.last_used = Instant::now();
    if !s.running {
        s.running = true;
        // A crash starts this feed's fetcher again (1 s, doubling to 30 s) while it is still asked for.
        std::thread::spawn(move || {
            let quiet = || slots().lock_or_recover().get(&url).is_none_or(|s| s.last_used.elapsed() > KEEP);
            if crate::core::sync::restarting("headlines fetch", quiet, || run(url.clone())).is_none() {
                if let Some(s) = slots().lock_or_recover().get_mut(&url) {
                    s.running = false;
                }
            }
        });
    }
    s.view.clone()
}

fn fetch(url: &str) -> Result<Feed, String> {
    use std::io::Read;
    let policy = reqwest::redirect::Policy::custom(|attempt| {
        let prev_https = attempt.previous().last().is_some_and(|u| u.scheme() == "https");
        if attempt.previous().len() >= 3 || (prev_https && attempt.url().scheme() != "https") || !attempt.url().username().is_empty() || attempt.url().password().is_some() {
            attempt.stop()
        } else {
            attempt.follow()
        }
    });
    let c = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).user_agent(crate::sources::http::UA).redirect(policy).build().map_err(|_| "No network client".to_string())?;
    let r = c
        .get(url)
        .header(reqwest::header::ACCEPT, "application/rss+xml, application/atom+xml, application/xml, text/xml;q=0.9, */*;q=0.5")
        .send()
        .map_err(|_| "The feed did not answer".to_string())?;
    if !r.status().is_success() {
        return Err(format!("The feed answered {}", r.status().as_u16()));
    }
    if r.content_length().is_some_and(|n| n as usize > MAX_BYTES) {
        return Err("Feed too large".into());
    }
    let mut buf = Vec::new();
    r.take(MAX_BYTES as u64 + 1).read_to_end(&mut buf).map_err(|_| "The feed could not be read".to_string())?;
    if buf.len() > MAX_BYTES {
        return Err("Feed too large".into());
    }
    if buf.iter().take(1024).any(|b| *b == 0) {
        return Err("Not a feed".into());
    }
    let feed = parse(&String::from_utf8_lossy(&buf));
    if feed.entries.is_empty() {
        return Err(if feed.is_feed { "The feed has no headlines".into() } else { "Not a feed".into() });
    }
    Ok(feed)
}

fn run(key: String) {
    let mut next = Instant::now();
    loop {
        let idle = match slots().lock_or_recover().get(&key) {
            Some(s) => s.last_used.elapsed() > KEEP,
            None => true,
        };
        if idle {
            break;
        }
        if Instant::now() >= next {
            let r = fetch(&key);
            next = Instant::now() + if r.is_ok() { REFRESH } else { RETRY };
            if let Some(s) = slots().lock_or_recover().get_mut(&key) {
                s.view.loaded = true;
                match r {
                    Ok(f) => {
                        s.view.entries = f.entries;
                        s.view.title = f.title;
                        s.view.error = None;
                    }
                    // keep the old titles when a refresh fails
                    Err(e) => s.view.error = Some(e),
                }
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    if let Some(s) = slots().lock_or_recover().get_mut(&key) {
        s.running = false;
    }
}

/// Test helper: a ready-made view for a link, with no loader thread (so no network).
#[cfg(test)]
pub fn test_inject(url: String, view: HeadlinesView) {
    slots().lock_or_recover().insert(url, Slot { last_used: Instant::now(), running: true, view });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/"><channel>
<title>Example News</title><link>https://example.com/</link>
<item><title>First story</title><link>https://example.com/1</link><pubDate>Mon, 19 Oct 2026 08:30:00 +1100</pubDate></item>
<item><title>Second story</title><pubDate>Mon, 19 Oct 2026 10:00:00 GMT</pubDate></item>
<item><title>   </title><pubDate>Mon, 19 Oct 2026 11:00:00 GMT</pubDate></item>
<item><title>Third story</title><dc:date>2026-10-18T09:00:00Z</dc:date></item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>Atom Blog</title>
<entry><title type="html">Hello &lt;b&gt;bold&lt;/b&gt; world</title><updated>2026-10-19T08:00:00+11:00</updated><published>2026-10-18T08:00:00Z</published>
<author><name>Someone</name></author></entry>
<entry><title type="text">Plain &amp; simple</title><updated>2026-10-17T00:00:00.123Z</updated></entry>
</feed>"#;

    fn utc(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64) -> i64 {
        days_from_civil(y, mo, d) * 86400 + h * 3600 + mi * 60 + s
    }

    #[test]
    fn rss_sample() {
        let f = parse(RSS);
        assert!(f.is_feed);
        assert_eq!(f.title.as_deref(), Some("Example News"));
        // the empty title is dropped; the last one has a dc:date; all dated, so newest first
        let t: Vec<&str> = f.entries.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(t, ["Second story", "First story", "Third story"]);
        assert_eq!(f.entries[1].ts, Some(utc(2026, 10, 18, 21, 30, 0)));
        assert_eq!(f.entries[2].ts, Some(utc(2026, 10, 18, 9, 0, 0)));
    }

    #[test]
    fn feed_order_is_kept_when_a_date_is_missing() {
        let x = "<rss><channel><item><title>A</title></item><item><title>B</title><pubDate>Mon, 19 Oct 2026 10:00:00 GMT</pubDate></item><item><title>C</title><pubDate>Tue, 20 Oct 2026 10:00:00 GMT</pubDate></item></channel></rss>";
        let t: Vec<String> = parse(x).entries.into_iter().map(|e| e.title).collect();
        assert_eq!(t, ["A", "B", "C"]);
    }

    #[test]
    fn atom_sample() {
        let f = parse(ATOM);
        assert_eq!(f.title.as_deref(), Some("Atom Blog"));
        assert_eq!(f.entries.len(), 2);
        assert_eq!(f.entries[0].title, "Hello bold world");
        // published wins over updated
        assert_eq!(f.entries[0].ts, Some(utc(2026, 10, 18, 8, 0, 0)));
        assert_eq!(f.entries[1].title, "Plain & simple");
        assert_eq!(f.entries[1].ts, Some(utc(2026, 10, 17, 0, 0, 0)));
    }

    #[test]
    fn cdata_entities_and_html_in_titles() {
        let x = r#"<rss><channel><title><![CDATA[My <Feed>]]></title>
<item><title><![CDATA[Tom &amp; Jerry <i>live</i>
   tonight]]></title></item>
<item><title>Caf&#233; &#x1F600; &quot;quoted&quot; &apos;x&apos; &lt;3 &unknown; &#99999999;</title></item>
<item><title>Rock <b>&amp;</b> roll<br/>again</title></item>
</channel></rss>"#;
        let f = parse(x);
        assert_eq!(f.entries[0].title, "Tom & Jerry live tonight");
        assert_eq!(f.entries[1].title, "Caf\u{e9} \u{1F600} \"quoted\" 'x' <3 &unknown; &#99999999;");
        assert_eq!(f.entries[2].title, "Rock & roll again");
    }

    #[test]
    fn malformed_xml_does_not_panic() {
        for x in [
            "",
            "<",
            "<<<<>>>>",
            "<rss><channel><item><title>Unclosed",
            "<rss><channel><item><title>A</item><item><title>B</title>",
            "<item><title attr=\"unterminated>X</title></item>",
            "<![CDATA[ never closed",
            "<!-- never closed",
            "</title></item></rss>",
            "<feed><entry><title>x</title><updated>not a date</updated></entry>",
            "just some text & more < text",
            "<rss><channel><item><title>\u{e9}\u{4e2d}\u{6587}<</title></item></channel></rss>",
        ] {
            let _ = parse(x);
        }
        let f = parse("<rss><channel><item><title>A</item><item><title>B</title>");
        let t: Vec<&str> = f.entries.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(t, ["A", "B"]);
        assert!(!parse("<html><body>hello</body></html>").is_feed);
    }

    #[test]
    fn huge_and_odd_input_is_bounded() {
        let mut x = String::from("<rss><channel>");
        for i in 0..1000 {
            x.push_str(&format!("<item><title>{}{}</title></item>", "long ".repeat(200), i));
        }
        x.push_str("</channel></rss>");
        let f = parse(&x);
        assert_eq!(f.entries.len(), MAX_ENTRIES);
        assert!(f.entries.iter().all(|e| e.title.chars().count() <= MAX_TITLE));
        // deep nesting and a very long run of tags
        let deep = "<a>".repeat(50_000) + "<item><title>x</title></item>";
        let _ = parse(&deep);
        let _ = parse(&"<".repeat(100_000));
        let _ = parse(&"&".repeat(100_000));
    }

    #[test]
    fn dates_in_both_formats() {
        assert_eq!(parse_rfc2822("Mon, 19 Oct 2026 08:30:00 +1100"), Some(utc(2026, 10, 18, 21, 30, 0)));
        assert_eq!(parse_rfc2822("Mon, 19 Oct 2026 08:30:00 GMT"), Some(utc(2026, 10, 19, 8, 30, 0)));
        assert_eq!(parse_rfc2822("19 Oct 2026 08:30 Z"), Some(utc(2026, 10, 19, 8, 30, 0)));
        assert_eq!(parse_rfc2822("Mon, 19 Oct 2026 08:30:00 -0430"), Some(utc(2026, 10, 19, 13, 0, 0)));
        assert_eq!(parse_rfc2822("Mon, 19 Oct 26 08:30:00 EST"), Some(utc(2026, 10, 19, 13, 30, 0)));
        assert_eq!(parse_rfc3339("2026-10-19T08:30:00Z"), Some(utc(2026, 10, 19, 8, 30, 0)));
        assert_eq!(parse_rfc3339("2026-10-19T08:30:00+11:00"), Some(utc(2026, 10, 18, 21, 30, 0)));
        assert_eq!(parse_rfc3339("2026-10-19T08:30:00.250-05:00"), Some(utc(2026, 10, 19, 13, 30, 0)));
        assert_eq!(parse_rfc3339("2026-10-19"), Some(utc(2026, 10, 19, 0, 0, 0)));
        assert_eq!(parse_date("2026-10-19T08:30:00Z"), parse_date("Mon, 19 Oct 2026 08:30:00 GMT"));
        for bad in ["", "yesterday", "2026-13-40T00:00:00Z", "Mon, 32 Oct 2026 00:00:00 GMT", "Mon, 19 Foo 2026 00:00:00 GMT", "2026-10-19T25:00:00Z", "99999-99-99"] {
            assert_eq!(parse_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn age_text_steps() {
        let now = 1_800_000_000;
        assert_eq!(age_text(now, now - 5), "just now");
        assert_eq!(age_text(now, now + 500), "just now");
        assert_eq!(age_text(now, now - 60), "1 min ago");
        assert_eq!(age_text(now, now - 5 * 60 - 3), "5 min ago");
        assert_eq!(age_text(now, now - 3600), "1 h ago");
        assert_eq!(age_text(now, now - 3 * 3600 - 100), "3 h ago");
        assert_eq!(age_text(now, now - 86400), "1 d ago");
        assert_eq!(age_text(now, now - 2 * 86400 - 5), "2 d ago");
    }

    #[test]
    fn source_validation() {
        for ok in ["https://example.com/feed.xml", "http://example.com/rss", "HTTPS://Example.com:8080/a?b=c#d", "https://[::1]:81/x", " https://example.com/f "] {
            assert!(valid_source(ok).is_some(), "{ok}");
        }
        assert_eq!(valid_source(" https://example.com/f ").as_deref(), Some("https://example.com/f"));
        for bad in [
            "", "file:///C:/x.xml", "javascript:alert(1)", "ftp://example.com/feed", "data:text/xml,<rss/>", "feed://example.com/rss", "example.com/feed",
            "https://user:pass@example.com/feed", "https://user@example.com/", "https://", "https:///x", "https://exa mple.com/", "https://example.com:abc/",
            "https://example.com\\@evil.com/",
        ] {
            assert!(valid_source(bad).is_none(), "{bad}");
        }
        assert!(valid_source(&format!("https://example.com/{}", "a".repeat(500))).is_none());
        assert_eq!(host_of("https://www.example.com/feed.xml"), "example.com");
        assert_eq!(host_of("http://news.example.org:8080/x"), "news.example.org");
    }

    #[test]
    fn clean_keeps_a_valid_source_and_is_idempotent() {
        let run = |v: Value| {
            let mut o = v.as_object().cloned().unwrap();
            clean(&mut o);
            o
        };
        let a = run(json!({ "source": "  https://example.com/feed.xml ", "count": 99, "color": "nope" }));
        assert_eq!(a["source"], json!("https://example.com/feed.xml"));
        assert_eq!(a["count"], json!(5));
        assert_eq!(a["showSource"], json!(true));
        assert_eq!(a["color"], json!("#ffffff"));
        assert_eq!(run(json!({ "source": "file:///x", "count": 1 }))["source"], json!(""));
        assert_eq!(run(json!({ "source": "file:///x", "count": 1 }))["count"], json!(3));
        assert_eq!(run(json!({ "source": "https://u:p@example.com/" }))["source"], json!(""));
        assert_eq!(run(json!({ "source": 5 }))["source"], json!(""));
        for v in [json!({}), json!({ "source": "http://example.com/x", "count": 4.6, "showSource": false, "color": "#ABCDEF" }), json!({ "source": "javascript:1" })] {
            let once = run(v);
            let twice = run(Value::Object(once.clone()));
            assert_eq!(once, twice);
        }
        let w = Widget { props: json!({ "source": "https://example.com/f", "count": 3 }), ..Widget::default() };
        assert_eq!(w.headlines().count, 3);
        assert_eq!(Widget { props: json!("junk"), ..Widget::default() }.headlines(), HeadlinesProps::default());
    }

    #[test]
    fn read_with_a_bad_link_never_starts_a_loader() {
        let v = read("file:///C:/feed.xml");
        assert!(v.loaded && v.entries.is_empty() && v.error.is_some());
    }
}
