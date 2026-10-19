//! Running the ffmpeg sidecar: cancellable (the child is killed), with progress from `-progress pipe:1`.
use super::{CancelToken, MediaError, Result};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// `CS_FFMPEG`, else `ffmpeg.exe` next to the running executable, else the repo's node_modules/ffmpeg-static.
pub fn locate() -> PathBuf {
    if let Some(p) = std::env::var_os("CS_FFMPEG") {
        return PathBuf::from(p);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in ["ffmpeg.exe", "ffmpeg"] {
                let p = dir.join(name);
                if p.is_file() {
                    return p;
                }
            }
        }
    }
    // Development: <repo>/node_modules/ffmpeg-static/ffmpeg.exe (src-tauri is one level below the repo root).
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for _ in 0..6 {
        let p = d.join("node_modules").join("ffmpeg-static").join("ffmpeg.exe");
        if p.is_file() {
            return p;
        }
        if !d.pop() {
            break;
        }
    }
    PathBuf::from("ffmpeg")
}

fn command(ffmpeg: &Path) -> Command {
    let mut c = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000 | 0x0000_4000); // CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS (WS1: uploads and UI stay smooth)
    }
    c.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
    c.stdin(Stdio::null());
    c
}

fn stderr_tail(s: &str) -> String {
    let lines: Vec<&str> = s.trim().lines().collect();
    let tail = lines[lines.len().saturating_sub(4)..].join(" | ");
    if tail.is_empty() {
        "no output".into()
    } else {
        tail
    }
}

/// Wait for `child`, polling the cancel token; kills the child on cancel.
fn wait(child: &mut Child, cancel: &CancelToken) -> Result<std::process::ExitStatus> {
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(MediaError::Cancelled);
        }
        if let Some(st) = child.try_wait()? {
            return Ok(st);
        }
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// Run ffmpeg with `args`; `on_time` receives the output time (seconds) as it advances.
pub fn run(ffmpeg: &Path, args: &[String], cancel: &CancelToken, on_time: Option<&(dyn Fn(f64) + Sync)>) -> Result<()> {
    cancel.check()?;
    let mut cmd = command(ffmpeg);
    if on_time.is_some() {
        cmd.args(["-progress", "pipe:1"]);
    }
    cmd.args(args).stdout(if on_time.is_some() { Stdio::piped() } else { Stdio::null() }).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| MediaError::Ffmpeg(format!("could not start {}: {e}", ffmpeg.display())))?;
    super::sys::adopt_child(&child); // killed with the app
    let stderr = child.stderr.take().unwrap();
    let stdout = child.stdout.take();
    std::thread::scope(|s| {
        let err_h = s.spawn(move || {
            let mut e = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut e);
            e
        });
        if let (Some(out), Some(cb)) = (stdout, on_time) {
            s.spawn(move || {
                for line in BufReader::new(out).lines().map_while(|l| l.ok()) {
                    if let Some(t) = line.trim().strip_prefix("out_time=") {
                        if let Some(sec) = parse_hms(t) {
                            cb(sec);
                        }
                    }
                }
            });
        }
        let st = wait(&mut child, cancel);
        let err = err_h.join().unwrap_or_default();
        match st {
            Ok(st) if st.success() => Ok(()),
            Ok(st) => Err(MediaError::Ffmpeg(if err.trim().is_empty() { format!("exit {st}") } else { stderr_tail(&err) })),
            Err(e) => Err(e),
        }
    })
}

/// Run ffmpeg writing raw bytes to stdout (`pipe:1` in args) and return them; `on_bytes` gets the running total.
pub fn run_capture(ffmpeg: &Path, args: &[String], cancel: &CancelToken, on_bytes: Option<&(dyn Fn(usize) + Sync)>) -> Result<Vec<u8>> {
    cancel.check()?;
    let mut cmd = command(ffmpeg);
    cmd.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| MediaError::Ffmpeg(format!("could not start {}: {e}", ffmpeg.display())))?;
    super::sys::adopt_child(&child); // killed with the app
    let stderr = child.stderr.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    std::thread::scope(|s| {
        let err_h = s.spawn(move || {
            let mut e = String::new();
            let _ = BufReader::new(stderr).read_to_string(&mut e);
            e
        });
        let out_h = s.spawn(move || {
            let mut buf = Vec::with_capacity(1 << 20);
            let mut chunk = vec![0u8; 1 << 16];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) | Err(_) => break,
                    Ok(k) => {
                        buf.extend_from_slice(&chunk[..k]);
                        if let Some(cb) = on_bytes {
                            cb(buf.len());
                        }
                    }
                }
            }
            buf
        });
        let st = wait(&mut child, cancel);
        let out = out_h.join().unwrap_or_default();
        let err = err_h.join().unwrap_or_default();
        match st {
            Ok(st) if st.success() => Ok(out),
            Ok(st) => Err(MediaError::Ffmpeg(if err.trim().is_empty() { format!("exit {st}") } else { stderr_tail(&err) })),
            Err(e) => Err(e),
        }
    })
}

/// How long a probe may take before it is killed (a hung network share or a stalled device read would otherwise
/// block the probe task forever). A normal probe takes 30-100 ms.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(15);

/// `ffmpeg -i file` stderr (ffprobe is not bundled), killed after [`PROBE_TIMEOUT`].
pub fn probe_text(ffmpeg: &Path, file: &Path) -> Result<String> {
    probe_text_timeout(ffmpeg, file, PROBE_TIMEOUT)
}

pub fn probe_text_timeout(ffmpeg: &Path, file: &Path, timeout: Duration) -> Result<String> {
    let mut c = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000 | 0x0000_4000);
    }
    let mut child = c
        .args(["-hide_banner", "-nostdin", "-i"])
        .arg(file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| MediaError::Ffmpeg(format!("could not start {}: {e}", ffmpeg.display())))?;
    super::sys::adopt_child(&child);
    let stderr = child.stderr.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut e = String::new();
        let _ = BufReader::new(stderr).read_to_string(&mut e);
        e
    });
    let t0 = std::time::Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if t0.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            // the reader ends when the pipe closes with the process
            let _ = reader.join();
            return Err(MediaError::Ffmpeg(format!("reading {} took longer than {} s", file.display(), timeout.as_secs())));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(reader.join().unwrap_or_default())
}

fn parse_hms(t: &str) -> Option<f64> {
    let mut it = t.split(':');
    let h: f64 = it.next()?.parse().ok()?;
    let m: f64 = it.next()?.parse().ok()?;
    let s: f64 = it.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Build an args vector from string-likes.
#[macro_export]
macro_rules! ff_args {
    ($($a:expr),* $(,)?) => { vec![$(::std::string::ToString::to_string(&$a)),*] };
}

/// A path as an ffmpeg argument.
pub fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hms() {
        assert_eq!(parse_hms("00:01:02.500000"), Some(62.5));
        assert_eq!(parse_hms("N/A"), None);
    }

    /// A probe that never gets data (ffmpeg waiting for a TCP client, like a hung share) is killed at the timeout.
    #[test]
    fn probe_times_out() {
        let ff = locate();
        if !ff.is_file() {
            return;
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let t0 = std::time::Instant::now();
        let r = probe_text_timeout(&ff, Path::new(&format!("tcp://127.0.0.1:{port}?listen=1")), Duration::from_millis(800));
        assert!(matches!(r, Err(MediaError::Ffmpeg(_))), "{r:?}");
        // killed at the timeout, not left hanging (the kill and the wait can be slow while other tests use the CPU)
        assert!(t0.elapsed() >= Duration::from_millis(700) && t0.elapsed() < Duration::from_secs(10), "{:?}", t0.elapsed());
    }
}
