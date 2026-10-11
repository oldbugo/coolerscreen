//! Source: "Add from link": download one file from a direct http(s) link (a video, GIF, image, .ht or .zip). It does not
//! read web pages or look for media inside them: the link must point at the file itself. Also used by declarative plug-ins
//! (with `allowed_hosts`).
use super::*;
use std::path::PathBuf;
use std::time::Duration;

pub struct Link;

pub const MAX_BYTES: u64 = 500_000_000;
const EXT: &[&str] = &["mp4", "mov", "mkv", "webm", "avi", "m4v", "gif", "jpg", "jpeg", "png", "bmp", "webp", "tif", "tiff", "ht", "zip"];
const TYPE_EXT: &[(&str, &str)] = &[
    ("video/mp4", "mp4"),
    ("video/webm", "webm"),
    ("video/quicktime", "mov"),
    ("video/x-matroska", "mkv"),
    ("image/gif", "gif"),
    ("image/jpeg", "jpg"),
    ("image/png", "png"),
    ("image/webp", "webp"),
    ("image/bmp", "bmp"),
    ("application/zip", "zip"),
];

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn ext_of(name: &str) -> Option<String> {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// The file name to keep (with a supported extension), or None when the answer is not a supported file.
pub fn file_name_for(url: &reqwest::Url, content_disposition: &str, content_type: &str) -> Option<String> {
    let mut n = String::new();
    let cd = content_disposition;
    if let Some(i) = cd.to_lowercase().find("filename*=utf-8''") {
        let rest = &cd[i + "filename*=utf-8''".len()..];
        n = percent_decode(rest.split(';').next().unwrap_or("").trim());
    } else if let Some(i) = cd.to_lowercase().find("filename=") {
        let rest = cd[i + "filename=".len()..].split(';').next().unwrap_or("").trim().trim_matches('"');
        n = rest.to_string();
    }
    if n.is_empty() {
        n = percent_decode(url.path_segments().and_then(|s| s.last()).unwrap_or(""));
    }
    let n: String = n.chars().map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 32 { '_' } else { c }).take(120).collect();
    let ctype = content_type.split(';').next().unwrap_or("").trim().to_lowercase();
    if ext_of(&n).map_or(false, |e| EXT.contains(&e.as_str())) {
        return Some(n);
    }
    let e = TYPE_EXT.iter().find(|t| t.0 == ctype)?.1;
    let stem = match n.rfind('.') {
        Some(i) if i > 0 => &n[..i],
        _ => n.as_str(),
    };
    Some(format!("{}.{e}", if stem.is_empty() { "download" } else { stem }))
}

/// Download `url` into `tmp_dir`. At most 5 redirects, 500 MB, HTML and unknown types are refused.
/// `allowed_hosts` (when given) must contain the host of the link and of every redirect target.
pub fn download_file(url: &str, tmp_dir: &std::path::Path, allowed_hosts: Option<&[String]>, progress: Progress, cancel: &Cancel) -> Result<(PathBuf, String)> {
    let u = reqwest::Url::parse(url.trim()).map_err(|_| SourceError("That is not a valid link.".into()))?;
    if u.scheme() != "https" && u.scheme() != "http" {
        return err("Only http and https links are supported.");
    }
    let hosts: Option<Vec<String>> = allowed_hosts.map(|h| h.iter().map(|x| x.to_lowercase()).collect());
    let allowed = move |host: Option<&str>| hosts.as_ref().map_or(true, |h| host.map_or(false, |x| h.contains(&x.to_lowercase())));
    if !allowed(u.host_str()) {
        return err("This link points to a web address the source does not list.");
    }
    let policy = reqwest::redirect::Policy::custom(move |a| {
        if a.previous().len() >= 5 {
            a.error("Too many redirects.")
        } else if !allowed(a.url().host_str()) {
            a.error("The link redirects to an address the source does not list.")
        } else {
            a.follow()
        }
    });
    let client = reqwest::blocking::Client::builder()
        .user_agent(http::UA)
        .connect_timeout(Duration::from_secs(30))
        .redirect(policy)
        .build()
        .map_err(|e| SourceError(e.to_string()))?;
    let resp = client.get(u.clone()).send().map_err(|e| {
        let m = e.to_string();
        let chain = std::error::Error::source(&e).map(|s| s.to_string()).unwrap_or_default();
        if m.contains("Too many redirects") || chain.contains("Too many redirects") {
            SourceError("Too many redirects.".into())
        } else if chain.contains("redirects to an address") {
            SourceError(chain)
        } else {
            SourceError(format!("Download failed: {m}"))
        }
    })?;
    if resp.status().as_u16() != 200 {
        return err(format!("The server answered {}.", resp.status().as_u16()));
    }
    let h = |k: &str| resp.headers().get(k).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let ctype = h("content-type").to_lowercase();
    let name = file_name_for(resp.url(), &h("content-disposition"), &ctype);
    let name = match name {
        Some(n) if !ctype.starts_with("text/html") => n,
        _ => return err("That link opens a web page, not a video, GIF, image or theme file. Use a direct link to the file."),
    };
    if resp.content_length().unwrap_or(0) > MAX_BYTES {
        return err("The file is larger than 500 MB.");
    }
    let ext = ext_of(&name).unwrap_or_default();
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    let file = tmp_dir.join(format!("link-{ms}.{ext}"));
    http::stream_to_file(resp, &file, MAX_BYTES, progress, cancel)?;
    Ok((file, name))
}

impl SourceProvider for Link {
    fn id(&self) -> &str {
        "link"
    }
    fn name(&self) -> &str {
        "Add from link"
    }
    fn description(&self) -> &str {
        "Adds a button to Browse for pasting a direct link to a video, GIF, image or theme file (up to 500 MB). Web pages are not supported."
    }
    fn kind(&self) -> Kind {
        Kind::Action
    }
    fn detect(&self, _ctx: &SourceCtx) -> Detect {
        Detect::ok("Ready")
    }
    fn list(&self, _ctx: &SourceCtx, _page: u32, _query: Option<&str>, _refresh: bool) -> Result<ListPage> {
        Ok(ListPage::default())
    }
    /// `item_id` is the link.
    fn add(&self, ctx: &SourceCtx, item_id: &str, _req: &AddRequest, progress: Progress, cancel: &Cancel) -> Result<AddResult> {
        let (file, name) = download_file(item_id, &ctx.tmp_dir, None, progress, cancel)?;
        Ok(AddResult { file, name, temp: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn u(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }
    #[test]
    fn names() {
        assert_eq!(file_name_for(&u("https://x.com/a/b%20c.mp4"), "", "video/mp4").as_deref(), Some("b c.mp4"));
        assert_eq!(file_name_for(&u("https://x.com/get?id=1"), "", "image/jpeg").as_deref(), Some("get.jpg"));
        assert_eq!(file_name_for(&u("https://x.com/get"), "attachment; filename=\"clip.webm\"", "application/octet-stream").as_deref(), Some("clip.webm"));
        assert_eq!(file_name_for(&u("https://x.com/x"), "attachment; filename*=UTF-8''a%20b.gif", "").as_deref(), Some("a b.gif"));
        assert!(file_name_for(&u("https://x.com/page"), "", "text/html").is_none());
        assert!(file_name_for(&u("https://x.com/a.exe"), "", "application/octet-stream").is_none());
    }
    #[test]
    fn rejects_bad_links() {
        let t = std::env::temp_dir();
        let c = Cancel::new();
        assert!(download_file("ftp://x.com/a.mp4", &t, None, &|_, _| {}, &c).is_err());
        assert!(download_file("not a url", &t, None, &|_, _| {}, &c).is_err());
        assert!(download_file("https://evil.com/a.mp4", &t, Some(&["good.com".to_string()]), &|_, _| {}, &c).is_err());
    }
}
