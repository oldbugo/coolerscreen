//! 4.0 W2: the `webpicture` widget: ONE picture from a link the USER pasted, fetched again every N minutes while the widget is shown.
//! Nothing else is sent: a plain GET of that link (`Accept: image/*`), at most 3 redirects (never https -> http), 10 s to connect and
//! 30 s in all, at most 8 MB of body. Only real picture bytes are accepted (the magic bytes decide, never the Content-Type), the
//! declared size is checked before anything is decoded, and the picture is kept at most 1680 px on its long side.
//!
//! Same shape as `agenda.rs`: a demand-driven loader per (link, minutes), started by the first `read`, ending about 30 s after the
//! last one (so a hidden widget costs nothing); `read` never blocks the render thread; a failed refresh keeps the old picture and
//! reports the error next to it.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

use super::theme::Widget;

pub const MAX_LINK: usize = 500;
pub const MAX_BODY: usize = 8 * 1024 * 1024;
/// Declared picture sizes above these are refused before decoding.
pub const MAX_SIDE: u32 = 6000;
pub const MAX_PIXELS: u64 = 36_000_000;
/// Animated GIFs are decoded whole by the small decoder (every frame in memory): keep them modest.
const MAX_GIF_PIXELS: u64 = 4_000_000;
/// The live canvas is 1680 x 756: a bigger picture is wasted memory.
pub const KEEP_SIDE: u32 = 1680;
const KEEP: Duration = Duration::from_secs(30);
const RETRY: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebPictureProps {
    /// The picture link ("" = nothing chosen).
    pub source: String,
    /// Refresh every N minutes (1..1440).
    pub minutes: u32,
    /// "contain" (whole picture) | "cover" (fill).
    pub fit: String,
    pub opacity: f32,
}
impl Default for WebPictureProps {
    fn default() -> Self {
        WebPictureProps { source: String::new(), minutes: 15, fit: "contain".into(), opacity: 1.0 }
    }
}

impl Widget {
    pub fn webpicture(&self) -> WebPictureProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`).
pub fn clean(o: &mut Map<String, Value>) {
    use super::morewidgets::{int, number, word};
    let src = o.get("source").and_then(Value::as_str).and_then(valid_source).unwrap_or_default();
    o.insert("source".into(), Value::String(src));
    int(o, "minutes", 15, 1, 1440);
    word(o, "fit", &["contain", "cover"], "contain");
    number(o, "opacity", 1.0, 0.0, 1.0);
}

/// The link when it is an http:// or https:// address of at most 500 characters without credentials; else None.
pub fn valid_source(s: &str) -> Option<String> {
    let s = s.trim();
    if s.is_empty() || s.chars().count() > MAX_LINK || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if !(lower.starts_with("http://") || lower.starts_with("https://")) {
        return None;
    }
    let u = reqwest::Url::parse(s).ok()?;
    if !matches!(u.scheme(), "http" | "https") || !u.username().is_empty() || u.password().is_some() || u.host_str().is_none_or(str::is_empty) {
        return None;
    }
    Some(s.to_string())
}

// ---------------------------------------------------------------- errors

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PicError {
    Unreachable,
    NotPicture,
    TooLarge,
    Unsupported,
}
impl PicError {
    pub fn message(self) -> &'static str {
        match self {
            PicError::Unreachable => "The link did not answer",
            PicError::NotPicture => "Not a picture",
            PicError::TooLarge => "Picture too large",
            PicError::Unsupported => "Unsupported picture type",
        }
    }
}

// ---------------------------------------------------------------- sniffing and decoding

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    WebP,
    Bmp,
}

/// What the bytes are, judged by their first bytes only. Known picture formats the app cannot show (TIFF, ICO, AVIF/HEIC, SVG)
/// are `Unsupported`; anything else (html, text, empty) is `NotPicture`.
pub fn sniff(b: &[u8]) -> Result<Kind, PicError> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return Ok(Kind::Png);
    }
    if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(Kind::Jpeg);
    }
    if super::gif::is_gif(b) {
        return Ok(Kind::Gif);
    }
    if b.len() >= 16 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Ok(Kind::WebP);
    }
    if b.len() >= 30 && &b[0..2] == b"BM" {
        return Ok(Kind::Bmp);
    }
    if b.starts_with(b"II*\0") || b.starts_with(b"MM\0*") || b.starts_with(&[0, 0, 1, 0]) || (b.len() >= 12 && &b[4..8] == b"ftyp") {
        return Err(PicError::Unsupported);
    }
    let head = String::from_utf8_lossy(&b[..b.len().min(512)]).to_ascii_lowercase();
    if head.contains("<svg") {
        return Err(PicError::Unsupported);
    }
    Err(PicError::NotPicture)
}

fn be32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(i..i + 4)?.try_into().ok()?))
}
fn le16(b: &[u8], i: usize) -> Option<u32> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?) as u32)
}
fn le32(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// The size the header declares (read without decoding anything).
pub fn header_size(b: &[u8], kind: Kind) -> Option<(u32, u32)> {
    match kind {
        Kind::Png => Some((be32(b, 16)?, be32(b, 20)?)),
        Kind::Gif => Some((le16(b, 6)?, le16(b, 8)?)),
        Kind::Bmp => {
            let (w, h) = (le32(b, 18)? as i32, le32(b, 22)? as i32);
            Some((w.unsigned_abs(), h.unsigned_abs()))
        }
        Kind::Jpeg => {
            let mut i = 2;
            while i + 9 < b.len() {
                if b[i] != 0xFF {
                    i += 1;
                    continue;
                }
                let m = b[i + 1];
                if m == 0xFF {
                    i += 1;
                    continue;
                }
                if (0xD0..=0xD9).contains(&m) || m == 0x01 || m == 0x00 {
                    i += 2;
                    continue;
                }
                if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
                    let h = u16::from_be_bytes([b[i + 5], b[i + 6]]) as u32;
                    let w = u16::from_be_bytes([b[i + 7], b[i + 8]]) as u32;
                    return Some((w, h));
                }
                let len = u16::from_be_bytes([b[i + 2], b[i + 3]]) as usize;
                i += 2 + len.max(2);
            }
            None
        }
        Kind::WebP => match b.get(12..16)? {
            b"VP8 " => {
                if b.get(23..26)? != [0x9D, 0x01, 0x2A] {
                    return None;
                }
                Some((le16(b, 26)? & 0x3FFF, le16(b, 28)? & 0x3FFF))
            }
            b"VP8L" => {
                if *b.get(20)? != 0x2F {
                    return None;
                }
                let (b1, b2, b3, b4) = (*b.get(21)? as u32, *b.get(22)? as u32, *b.get(23)? as u32, *b.get(24)? as u32);
                Some((1 + (b1 | (b2 & 0x3F) << 8), 1 + ((b2 >> 6) | b3 << 2 | (b4 & 0x0F) << 10)))
            }
            b"VP8X" => {
                let w = 1 + (*b.get(24)? as u32 | (*b.get(25)? as u32) << 8 | (*b.get(26)? as u32) << 16);
                let h = 1 + (*b.get(27)? as u32 | (*b.get(28)? as u32) << 8 | (*b.get(29)? as u32) << 16);
                Some((w, h))
            }
            _ => None,
        },
    }
}

/// Refuse sizes that are empty, wider/taller than 6000 px or over 36 million pixels.
pub fn check_size(w: u32, h: u32, kind: Kind) -> Result<(), PicError> {
    if w == 0 || h == 0 {
        return Err(PicError::NotPicture);
    }
    let px = w as u64 * h as u64;
    if w > MAX_SIDE || h > MAX_SIDE || px > MAX_PIXELS || (kind == Kind::Gif && px > MAX_GIF_PIXELS) {
        return Err(PicError::TooLarge);
    }
    Ok(())
}

/// Decode the first (or only) picture of `bytes` to a premultiplied pixmap of at most 1680 px on the long side.
/// PNG and GIF with the crate's own decoders; JPEG, WebP and BMP through the bundled ffmpeg (as the image item does).
pub fn decode(bytes: &[u8]) -> Result<Pixmap, PicError> {
    let kind = sniff(bytes)?;
    let (w, h) = header_size(bytes, kind).ok_or(PicError::NotPicture)?;
    check_size(w, h, kind)?;
    let pm = match kind {
        Kind::Png => Pixmap::decode_png(bytes).map_err(|_| PicError::NotPicture)?,
        Kind::Gif => {
            let g = super::gif::decode(bytes).map_err(|_| PicError::NotPicture)?;
            let f = g.frames.into_iter().next().ok_or(PicError::NotPicture)?;
            let mut pm = Pixmap::new(g.w as u32, g.h as u32).ok_or(PicError::NotPicture)?;
            if f.rgba.len() != pm.data().len() {
                return Err(PicError::NotPicture);
            }
            pm.data_mut().copy_from_slice(&f.rgba);
            pm
        }
        _ => via_ffmpeg(bytes)?,
    };
    check_size(pm.width(), pm.height(), Kind::Png)?;
    Ok(downscale(pm, KEEP_SIDE))
}

fn via_ffmpeg(bytes: &[u8]) -> Result<Pixmap, PicError> {
    let ffmpeg = crate::media::ffmpeg::locate();
    if !ffmpeg.exists() {
        return Err(PicError::Unsupported);
    }
    let mut c = std::process::Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin", "-i", "pipe:0", "-frames:v", "1", "-f", "image2pipe", "-c:v", "png", "-"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let mut child = c.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null()).spawn().map_err(|_| PicError::Unsupported)?;
    let mut stdin = child.stdin.take().ok_or(PicError::Unsupported)?;
    let data = bytes.to_vec();
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        let _ = stdin.write_all(&data);
    });
    let out = child.wait_with_output().map_err(|_| PicError::NotPicture)?;
    let _ = writer.join();
    Pixmap::decode_png(&out.stdout).map_err(|_| PicError::NotPicture)
}

/// Shrink so the long side is at most `max` (box-average halving while far larger, then bilinear). Smaller pictures are untouched.
pub fn downscale(src: Pixmap, max: u32) -> Pixmap {
    let (w, h) = (src.width(), src.height());
    let long = w.max(h);
    if long <= max {
        return src;
    }
    let k = max as f32 / long as f32;
    let (tw, th) = (((w as f32 * k).round() as u32).max(1), ((h as f32 * k).round() as u32).max(1));
    let mut cur = src;
    while cur.width() >= tw * 2 && cur.height() >= th * 2 {
        cur = half(&cur);
    }
    let mut out = Pixmap::new(tw, th).unwrap();
    let t = Transform::from_scale(tw as f32 / cur.width() as f32, th as f32 / cur.height() as f32);
    out.draw_pixmap(0, 0, cur.as_ref(), &PixmapPaint { quality: FilterQuality::Bilinear, ..PixmapPaint::default() }, t, None);
    out
}

fn half(src: &Pixmap) -> Pixmap {
    let (w, h) = ((src.width() / 2).max(1), (src.height() / 2).max(1));
    let mut out = Pixmap::new(w, h).unwrap();
    let (sw, sh) = (src.width() as usize, src.height() as usize);
    let s = src.data();
    let d = out.data_mut();
    for y in 0..h as usize {
        for x in 0..w as usize {
            let (x0, y0) = ((x * 2).min(sw - 1), (y * 2).min(sh - 1));
            let (x1, y1) = ((x0 + 1).min(sw - 1), (y0 + 1).min(sh - 1));
            for c in 0..4 {
                let v = s[(y0 * sw + x0) * 4 + c] as u32 + s[(y0 * sw + x1) * 4 + c] as u32 + s[(y1 * sw + x0) * 4 + c] as u32 + s[(y1 * sw + x1) * 4 + c] as u32;
                d[(y * w as usize + x) * 4 + c] = (v / 4) as u8;
            }
        }
    }
    out
}

// ---------------------------------------------------------------- the loader

#[derive(Clone, Debug, Default)]
pub struct View {
    /// The latest good picture.
    pub pic: Option<Arc<Pixmap>>,
    /// Identifies `pic` (changes when the picture changes; used by the scaled-copy cache).
    pub id: u64,
    /// The last fetch failed (the old picture, if any, is still in `pic`).
    pub error: Option<PicError>,
    /// At least one fetch has finished.
    pub loaded: bool,
}

struct Slot {
    last_used: Instant,
    running: bool,
    view: View,
}

fn slots() -> &'static Mutex<HashMap<String, Slot>> {
    static S: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The current picture state for `source` (a valid link) refreshed every `minutes`: starts the loader if it is not running.
/// Never blocks; the first call returns `loaded: false`.
pub fn read(source: &str, minutes: u32) -> View {
    let Some(link) = valid_source(source) else {
        return View { loaded: true, error: Some(PicError::NotPicture), ..View::default() };
    };
    let minutes = minutes.clamp(1, 1440);
    let key = format!("{minutes}|{link}");
    let mut g = slots().lock().unwrap();
    let s = g.entry(key.clone()).or_insert_with(|| Slot { last_used: Instant::now(), running: false, view: View::default() });
    s.last_used = Instant::now();
    if !s.running {
        s.running = true;
        std::thread::spawn(move || run(key, link, minutes));
    }
    s.view.clone()
}

fn hash_of(b: &[u8]) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    b.hash(&mut h);
    h.finish() | 1
}

fn client() -> Result<reqwest::blocking::Client, PicError> {
    let policy = reqwest::redirect::Policy::custom(|a| {
        if a.previous().len() >= 3 {
            a.error("too many redirects")
        } else if a.previous().last().is_some_and(|u| u.scheme() == "https") && a.url().scheme() != "https" {
            a.error("redirect from https to http")
        } else if !a.url().username().is_empty() || a.url().password().is_some() {
            a.error("redirect with credentials")
        } else {
            a.follow()
        }
    });
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .user_agent(crate::sources::http::UA)
        .redirect(policy)
        .build()
        .map_err(|_| PicError::Unreachable)
}

/// GET the link and return at most `MAX_BODY` bytes (reading stops beyond the cap).
fn fetch(link: &str) -> Result<Vec<u8>, PicError> {
    let r = client()?.get(link).header(reqwest::header::ACCEPT, "image/*").send().map_err(|_| PicError::Unreachable)?;
    if !r.status().is_success() {
        return Err(PicError::Unreachable);
    }
    if r.content_length().is_some_and(|n| n as usize > MAX_BODY) {
        return Err(PicError::TooLarge);
    }
    let mut body = Vec::new();
    r.take(MAX_BODY as u64 + 1).read_to_end(&mut body).map_err(|_| PicError::Unreachable)?;
    if body.len() > MAX_BODY {
        return Err(PicError::TooLarge);
    }
    Ok(body)
}

fn run(key: String, link: String, minutes: u32) {
    let every = Duration::from_secs(minutes as u64 * 60);
    let mut next = Instant::now();
    let mut last_hash = 0u64;
    loop {
        {
            let mut g = slots().lock().unwrap();
            match g.get(&key) {
                // gone quiet: end, and let go of the picture
                Some(s) if s.last_used.elapsed() > KEEP => {
                    g.remove(&key);
                    return;
                }
                None => return,
                _ => {}
            }
        }
        if Instant::now() >= next {
            let r = fetch(&link).and_then(|b| {
                let h = hash_of(&b);
                if h == last_hash {
                    Ok((h, None)) // the same bytes: nothing to decode
                } else {
                    decode(&b).map(|pm| (h, Some(pm)))
                }
            });
            let mut g = slots().lock().unwrap();
            let Some(s) = g.get_mut(&key) else { return };
            s.view.loaded = true;
            match r {
                Ok((h, pm)) => {
                    last_hash = h;
                    if let Some(pm) = pm {
                        s.view.pic = Some(Arc::new(pm));
                        s.view.id = h;
                    }
                    s.view.error = None;
                    next = Instant::now() + every;
                }
                Err(e) => {
                    // the old picture stays
                    s.view.error = Some(e);
                    next = Instant::now() + every.min(RETRY);
                }
            }
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

// ---------------------------------------------------------------- fit geometry

/// The size a picture of `sw` x `sh` takes in a box of `bw` x `bh`: whole picture (contain) or filling the box (cover, cropped).
pub fn fit_size(sw: f32, sh: f32, bw: f32, bh: f32, cover: bool) -> (f32, f32) {
    if sw <= 0.0 || sh <= 0.0 {
        return (0.0, 0.0);
    }
    let k = if cover { (bw / sw).max(bh / sh) } else { (bw / sw).min(bh / sh) };
    (sw * k, sh * k)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub(crate) fn png(w: u32, h: u32) -> Vec<u8> {
        let mut pm = Pixmap::new(w, h).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(200, 80, 40, 255));
        pm.encode_png().unwrap()
    }

    #[test]
    fn source_validation() {
        assert_eq!(valid_source(" https://example.com/a.jpg "), Some("https://example.com/a.jpg".into()));
        assert!(valid_source("http://192.168.1.5:8080/cam.png?x=1").is_some());
        assert!(valid_source("HTTPS://Example.com/a").is_some());
        for bad in ["", "file:///C:/a.png", "data:image/png;base64,AAAA", "ftp://example.com/a.png", "javascript:alert(1)", "https://user:pass@example.com/a.png", "https://user@example.com/a.png", "example.com/a.png", "https://", "https://exa mple.com/a", "C:\\a.png"] {
            assert_eq!(valid_source(bad), None, "{bad}");
        }
        let ok = format!("https://example.com/{}", "a".repeat(MAX_LINK - 20));
        assert_eq!(ok.chars().count(), MAX_LINK - 20 + 20);
        assert!(valid_source(&ok).is_some());
        assert_eq!(valid_source(&format!("{ok}b")), None, "501 characters");
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff(&png(2, 2)), Ok(Kind::Png));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]), Ok(Kind::Jpeg));
        assert_eq!(sniff(&super::super::gif::tests::tiny()), Ok(Kind::Gif));
        let mut bmp = b"BM".to_vec();
        bmp.resize(54, 0);
        assert_eq!(sniff(&bmp), Ok(Kind::Bmp));
        let mut webp = b"RIFF\0\0\0\0WEBPVP8 ".to_vec();
        webp.resize(40, 0);
        assert_eq!(sniff(&webp), Ok(Kind::WebP));
        assert_eq!(sniff(b"<!DOCTYPE html><html></html>"), Err(PicError::NotPicture));
        assert_eq!(sniff(b"hello, this is text"), Err(PicError::NotPicture));
        assert_eq!(sniff(b""), Err(PicError::NotPicture));
        assert_eq!(sniff(b"<?xml version=\"1.0\"?><svg xmlns=\"http://www.w3.org/2000/svg\"/>"), Err(PicError::Unsupported));
        assert_eq!(sniff(b"<svg width=\"2\"></svg>"), Err(PicError::Unsupported));
        assert_eq!(sniff(b"II*\0\x08\0\0\0"), Err(PicError::Unsupported));
    }

    #[test]
    fn header_sizes() {
        assert_eq!(header_size(&png(37, 11), Kind::Png), Some((37, 11)));
        assert_eq!(header_size(&super::super::gif::tests::tiny(), Kind::Gif), Some((2, 2)));
        // a JPEG header: SOI, an APP0 segment, then SOF0 (height 300, width 500)
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 11, 8, 0x01, 0x2C, 0x01, 0xF4, 3, 1, 0x11, 0];
        assert_eq!(header_size(&j, Kind::Jpeg), Some((500, 300)));
        j.truncate(12);
        assert_eq!(header_size(&j, Kind::Jpeg), None, "truncated");
        let mut bmp = b"BM".to_vec();
        bmp.resize(54, 0);
        bmp[18..22].copy_from_slice(&640i32.to_le_bytes());
        bmp[22..26].copy_from_slice(&(-480i32).to_le_bytes()); // top-down
        assert_eq!(header_size(&bmp, Kind::Bmp), Some((640, 480)));
        let mut x = b"RIFF\0\0\0\0WEBPVP8X".to_vec();
        x.resize(40, 0);
        x[24..27].copy_from_slice(&[0xFF, 0x03, 0]); // 1024 wide
        x[27..30].copy_from_slice(&[0xFF, 0x01, 0]); // 512 high
        assert_eq!(header_size(&x, Kind::WebP), Some((1024, 512)));
    }

    #[test]
    fn decode_limits_reject_before_allocating() {
        // a PNG header that declares 50000 x 50000 (the data after it is missing): refused on the header alone
        let mut p = png(2, 2);
        p[16..20].copy_from_slice(&50000u32.to_be_bytes());
        p[20..24].copy_from_slice(&50000u32.to_be_bytes());
        assert_eq!(decode(&p).err(), Some(PicError::TooLarge));
        // 6000 x 6000 = 36 million pixels is allowed by the side rule, 6000 x 6001 is not; 5000 x 7400 breaks the side rule
        assert_eq!(check_size(6000, 6000, Kind::Png), Ok(()));
        assert_eq!(check_size(6001, 100, Kind::Png), Err(PicError::TooLarge));
        assert_eq!(check_size(6000, 6001, Kind::Jpeg), Err(PicError::TooLarge));
        assert_eq!(check_size(0, 10, Kind::Png), Err(PicError::NotPicture));
        assert_eq!(check_size(2500, 2500, Kind::Gif), Err(PicError::TooLarge));
        assert_eq!(decode(b"<html>nope</html>").err(), Some(PicError::NotPicture));
        assert_eq!(decode(b"").err(), Some(PicError::NotPicture));
        let mut g = super::super::gif::tests::tiny();
        g[6..8].copy_from_slice(&5000u16.to_le_bytes());
        g[8..10].copy_from_slice(&5000u16.to_le_bytes());
        assert_eq!(decode(&g).err(), Some(PicError::TooLarge));
    }

    #[test]
    fn decodes_png_and_first_gif_frame() {
        let pm = decode(&png(40, 20)).unwrap();
        assert_eq!((pm.width(), pm.height()), (40, 20));
        let c = pm.pixel(5, 5).unwrap();
        assert_eq!((c.red(), c.green(), c.blue()), (200, 80, 40));
        let g = decode(&super::super::gif::tests::tiny()).unwrap();
        assert_eq!((g.width(), g.height()), (2, 2));
        let c = g.pixel(0, 0).unwrap();
        assert_eq!((c.red(), c.green(), c.blue()), (255, 0, 0), "first frame only");
    }

    #[test]
    fn decodes_a_jpeg_when_ffmpeg_is_there() {
        if !crate::media::ffmpeg::locate().exists() {
            return;
        }
        let rgb: Vec<u8> = (0..32 * 16).flat_map(|_| [30u8, 120, 220]).collect();
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, 90).encode(&rgb, 32, 16, jpeg_encoder::ColorType::Rgb).unwrap();
        assert_eq!(sniff(&out), Ok(Kind::Jpeg));
        assert_eq!(header_size(&out, Kind::Jpeg), Some((32, 16)));
        let pm = decode(&out).unwrap();
        assert_eq!((pm.width(), pm.height()), (32, 16));
        let c = pm.pixel(16, 8).unwrap();
        assert!((c.blue() as i32 - 220).abs() < 20 && (c.red() as i32 - 30).abs() < 20);
    }

    #[test]
    fn downscale_keeps_long_side_and_aspect() {
        let big = Pixmap::new(3400, 1700).unwrap();
        let s = downscale(big, KEEP_SIDE);
        assert_eq!((s.width(), s.height()), (1680, 840));
        let tall = downscale(Pixmap::new(900, 4000).unwrap(), KEEP_SIDE);
        assert_eq!((tall.width(), tall.height()), (378, 1680));
        let small = downscale(Pixmap::new(100, 50).unwrap(), KEEP_SIDE);
        assert_eq!((small.width(), small.height()), (100, 50));
    }

    #[test]
    fn fit_geometry() {
        assert_eq!(fit_size(200.0, 100.0, 100.0, 100.0, false), (100.0, 50.0));
        assert_eq!(fit_size(200.0, 100.0, 100.0, 100.0, true), (200.0, 100.0));
        assert_eq!(fit_size(100.0, 200.0, 100.0, 100.0, false), (50.0, 100.0));
        assert_eq!(fit_size(0.0, 5.0, 100.0, 100.0, false), (0.0, 0.0));
    }

    #[test]
    fn clean_is_idempotent_and_bounded() {
        let run = |v: Value| {
            let mut p = v;
            crate::live::morewidgets::clean("webpicture", &mut p);
            p
        };
        let a = run(json!({ "source": "https://example.com/a.png", "minutes": 0, "fit": "weird", "opacity": 7 }));
        assert_eq!(a["minutes"], 1);
        assert_eq!(a["fit"], "contain");
        assert_eq!(a["opacity"], 1.0);
        assert_eq!(a["source"], "https://example.com/a.png");
        assert_eq!(run(json!({ "minutes": 99999 }))["minutes"], 1440);
        assert_eq!(run(json!({ "minutes": -4, "opacity": -1, "fit": "cover" }))["minutes"], 1);
        assert_eq!(run(json!({ "source": "file:///x.png" }))["source"], "");
        assert_eq!(run(json!({ "source": "https://u:p@example.com/x.png" }))["source"], "");
        assert_eq!(run(json!({ "source": 5 }))["source"], "");
        let d = run(json!({}));
        assert_eq!((d["minutes"].clone(), d["fit"].clone(), d["opacity"].clone()), (json!(15), json!("contain"), json!(1.0)));
        assert_eq!(run(a.clone()), a);
        let w = Widget { kind: "webpicture".into(), props: a, ..Widget::default() };
        assert_eq!(w.webpicture().minutes, 1);
    }

    #[test]
    fn read_without_a_valid_link_starts_nothing() {
        let v = read("file:///C:/a.png", 5);
        assert!(v.pic.is_none() && v.loaded && v.error.is_some());
        assert!(slots().lock().unwrap().keys().all(|k| !k.contains("file:")));
    }
}
