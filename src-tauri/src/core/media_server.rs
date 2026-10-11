//! Tiny local HTTP/1.1 server for media the UI shows (thumbnails, previews, proxies, loop cards, source thumbs),
//! with Range support exactly like 1.2's `serveFile` (seeking in <video>). Bound to 127.0.0.1 on a random port;
//! every path starts with a random 128-bit token, so other local programs and web pages cannot read the library.
//! GET/HEAD only, keep-alive, one thread per connection. See docs/v2/api-contract.md section 4.
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Maps a request path (after the token, e.g. "/media/o/<id>/thumb.jpg") and query to a file and whether it may be
/// cached for 60 s.
pub type Resolver = Arc<dyn Fn(&str, &str) -> Option<(PathBuf, bool)> + Send + Sync>;

pub struct MediaServer {
    pub port: u16,
    pub token: String,
    stop: Arc<AtomicBool>,
}

impl MediaServer {
    /// `http://127.0.0.1:PORT/TOKEN`
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}/{}", self.port, self.token)
    }

    pub fn start(resolver: Resolver) -> std::io::Result<MediaServer> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let token = random_token();
        let stop = Arc::new(AtomicBool::new(false));
        let (tok, st) = (token.clone(), stop.clone());
        std::thread::Builder::new().name("media-server".into()).spawn(move || {
            for conn in listener.incoming() {
                if st.load(Ordering::SeqCst) {
                    break;
                }
                let Ok(conn) = conn else { continue };
                let (r, t) = (resolver.clone(), tok.clone());
                let _ = std::thread::Builder::new().name("media-conn".into()).spawn(move || {
                    let _ = serve_conn(conn, &t, &r);
                });
            }
        })?;
        Ok(MediaServer { port, token, stop })
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.port)); // wake accept()
    }
}

impl Drop for MediaServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn random_token() -> String {
    let mut out = String::new();
    let mut seed = super::scheduler::now_ms() ^ ((std::process::id() as u64) << 32);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    seed ^= nanos.rotate_left(17);
    // Mix in the address of a heap allocation (ASLR) and a per-call counter through splitmix64.
    let heap = Box::new(0u8);
    seed ^= (&*heap as *const u8 as u64).rotate_left(41);
    for _ in 0..2 {
        seed = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        out.push_str(&format!("{z:016x}"));
    }
    out
}

fn content_type(p: &std::path::Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("webm") => "video/webm",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Value of `key` in a query string, percent-decoded.
pub fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (percent_decode(k) == key).then(|| percent_decode(v))
    })
}

fn respond(w: &mut TcpStream, code: u16, reason: &str, headers: &[(&str, String)], body: &[u8], keep: bool) -> std::io::Result<()> {
    let mut h = format!("HTTP/1.1 {code} {reason}\r\n");
    for (k, v) in headers {
        h.push_str(&format!("{k}: {v}\r\n"));
    }
    h.push_str(&format!("Connection: {}\r\n\r\n", if keep { "keep-alive" } else { "close" }));
    w.write_all(h.as_bytes())?;
    w.write_all(body)
}

fn serve_conn(stream: TcpStream, token: &str, resolver: &Resolver) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut w = stream.try_clone()?;
    let mut r = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if r.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let (method, target, ver) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or("HTTP/1.0"));
        let mut range = None;
        let mut host = String::new();
        let mut conn_close = ver == "HTTP/1.0";
        loop {
            let mut h = String::new();
            if r.read_line(&mut h)? == 0 {
                return Ok(());
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                let v = v.trim();
                match k.trim().to_ascii_lowercase().as_str() {
                    "range" => range = Some(v.to_string()),
                    "host" => host = v.to_string(),
                    "connection" => conn_close = v.eq_ignore_ascii_case("close"),
                    _ => {}
                }
            }
        }
        let keep = !conn_close;
        let cors = ("Access-Control-Allow-Origin", "*".to_string());
        // DNS-rebinding protection, as 1.2: only our own host names.
        let host_ok = {
            let hn = host.rsplit_once(':').map(|x| x.0).unwrap_or(&host);
            hn == "127.0.0.1" || hn == "localhost"
        };
        if method != "GET" && method != "HEAD" {
            respond(&mut w, 405, "Method Not Allowed", &[cors, ("Content-Length", "0".into())], b"", keep)?;
            if !keep {
                return Ok(());
            }
            continue;
        }
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        let rest = path.strip_prefix('/').and_then(|p| p.strip_prefix(token)).filter(|p| p.starts_with('/'));
        let resolved = if host_ok { rest.and_then(|p| resolver(&percent_decode(p), query)) } else { None };
        let Some((file, cache)) = resolved else {
            respond(&mut w, 404, "Not Found", &[cors, ("Content-Length", "0".into())], b"", keep)?;
            if !keep {
                return Ok(());
            }
            continue;
        };
        let Ok(mut f) = std::fs::File::open(&file) else {
            respond(&mut w, 404, "Not Found", &[cors, ("Content-Length", "0".into())], b"", keep)?;
            if !keep {
                return Ok(());
            }
            continue;
        };
        let size = f.metadata()?.len();
        let mut headers = vec![
            cors,
            ("Content-Type", content_type(&file).to_string()),
            ("Accept-Ranges", "bytes".into()),
            ("Cache-Control", if cache { "max-age=60".into() } else { "no-cache".into() }),
        ];
        let (code, reason, start, end) = match range.as_deref().and_then(parse_range) {
            Some((a, b)) => {
                let (mut start, mut end) = (a.unwrap_or(0), b.unwrap_or(size.saturating_sub(1)));
                if a.is_none() {
                    if let Some(n) = b {
                        start = size.saturating_sub(n);
                        end = size.saturating_sub(1);
                    }
                }
                end = end.min(size.saturating_sub(1));
                if size == 0 || start > end {
                    headers.push(("Content-Range", format!("bytes */{size}")));
                    headers.push(("Content-Length", "0".into()));
                    respond(&mut w, 416, "Range Not Satisfiable", &headers, b"", keep)?;
                    if !keep {
                        return Ok(());
                    }
                    continue;
                }
                headers.push(("Content-Range", format!("bytes {start}-{end}/{size}")));
                (206, "Partial Content", start, end)
            }
            None => (200, "OK", 0, size.saturating_sub(1)),
        };
        let len = if size == 0 { 0 } else { end - start + 1 };
        headers.push(("Content-Length", len.to_string()));
        respond(&mut w, code, reason, &headers, b"", keep)?;
        if method == "GET" && len > 0 {
            f.seek(SeekFrom::Start(start))?;
            let mut left = len;
            let mut buf = vec![0u8; 256 * 1024];
            while left > 0 {
                let cap = left.min(buf.len() as u64) as usize;
                let n = f.read(&mut buf[..cap])?;
                if n == 0 {
                    break;
                }
                w.write_all(&buf[..n])?;
                left -= n as u64;
            }
        }
        if !keep {
            return Ok(());
        }
    }
}

/// `bytes=(\d*)-(\d*)` → (start, end); None if absent or malformed (then the whole file is sent, as 1.2).
fn parse_range(v: &str) -> Option<(Option<u64>, Option<u64>)> {
    let i = v.find("bytes=")?;
    let spec = &v[i + 6..];
    let (a, rest) = spec.split_once('-')?;
    let b: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    let a = if a.is_empty() { None } else { Some(a.trim().parse().ok()?) };
    let b = if b.is_empty() { None } else { Some(b.parse().ok()?) };
    Some((a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(port: u16, path: &str, range: Option<&str>) -> (u16, Vec<(String, String)>, Vec<u8>) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n");
        if let Some(r) = range {
            req.push_str(&format!("Range: {r}\r\n"));
        }
        req.push_str("\r\n");
        s.write_all(req.as_bytes()).unwrap();
        let mut all = vec![];
        s.read_to_end(&mut all).unwrap();
        let split = all.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let head = String::from_utf8_lossy(&all[..split]).to_string();
        let mut lines = head.lines();
        let code = lines.next().unwrap().split_whitespace().nth(1).unwrap().parse().unwrap();
        let hs = lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.to_lowercase(), v.trim().to_string())).collect();
        (code, hs, all[split + 4..].to_vec())
    }

    fn header<'a>(hs: &'a [(String, String)], k: &str) -> Option<&'a str> {
        hs.iter().find(|(a, _)| a == k).map(|(_, v)| v.as_str())
    }

    #[test]
    fn serves_with_ranges_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("clip.mp4");
        let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&f, &data).unwrap();
        let ff = f.clone();
        let srv = MediaServer::start(Arc::new(move |p: &str, q: &str| {
            if p == "/media/o/abcdefgh/clip.mp4" {
                Some((ff.clone(), false))
            } else if p == "/sources/we/thumb" && query_param(q, "item").as_deref() == Some("a b/c") {
                Some((ff.clone(), true))
            } else {
                None
            }
        }))
        .unwrap();
        let tok = srv.token.clone();
        let (c, hs, body) = get(srv.port, &format!("/{tok}/media/o/abcdefgh/clip.mp4"), None);
        assert_eq!((c, body.len()), (200, 10_000));
        assert_eq!(header(&hs, "content-type"), Some("video/mp4"));
        assert_eq!(header(&hs, "accept-ranges"), Some("bytes"));
        let (c, hs, body) = get(srv.port, &format!("/{tok}/media/o/abcdefgh/clip.mp4"), Some("bytes=100-199"));
        assert_eq!(c, 206);
        assert_eq!(body, &data[100..200]);
        assert_eq!(header(&hs, "content-range"), Some("bytes 100-199/10000"));
        let (c, _, body) = get(srv.port, &format!("/{tok}/media/o/abcdefgh/clip.mp4"), Some("bytes=9990-"));
        assert_eq!((c, body), (206, data[9990..].to_vec()));
        let (c, _, body) = get(srv.port, &format!("/{tok}/media/o/abcdefgh/clip.mp4"), Some("bytes=-10"));
        assert_eq!((c, body), (206, data[9990..].to_vec()));
        let (c, hs, _) = get(srv.port, &format!("/{tok}/media/o/abcdefgh/clip.mp4"), Some("bytes=20000-"));
        assert_eq!(c, 416);
        assert_eq!(header(&hs, "content-range"), Some("bytes */10000"));
        // wrong token / unknown path
        assert_eq!(get(srv.port, "/nottoken/media/o/abcdefgh/clip.mp4", None).0, 404);
        assert_eq!(get(srv.port, &format!("/{tok}/media/o/abcdefgh/other.mp4"), None).0, 404);
        // source thumb with an encoded query
        let (c, hs, _) = get(srv.port, &format!("/{tok}/sources/we/thumb?item=a%20b%2Fc"), None);
        assert_eq!(c, 200);
        assert_eq!(header(&hs, "cache-control"), Some("max-age=60"));
    }

    #[test]
    fn keep_alive_two_requests() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("t.jpg");
        std::fs::write(&f, b"0123456789").unwrap();
        let srv = MediaServer::start(Arc::new(move |_: &str, _: &str| Some((f.clone(), false)))).unwrap();
        let mut s = TcpStream::connect(("127.0.0.1", srv.port)).unwrap();
        let req = format!("GET /{}/x HTTP/1.1\r\nHost: localhost\r\nRange: bytes=0-1\r\n\r\n", srv.token);
        s.write_all(req.as_bytes()).unwrap();
        s.write_all(req.as_bytes()).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut buf = vec![0u8; 4096];
        let mut got = vec![];
        while got.windows(2).filter(|w| *w == b"01").count() < 2 {
            let n = s.read(&mut buf).unwrap();
            assert!(n > 0);
            got.extend_from_slice(&buf[..n]);
        }
        assert_eq!(String::from_utf8_lossy(&got).matches("206 Partial Content").count(), 2);
    }
}
