//! Shared HTTP helpers (reqwest blocking + rustls, honest User-Agent).
use super::{err, Cancel, Progress, Result, SourceError};
use reqwest::blocking::{Client, Response};
use reqwest::redirect::Policy;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

pub const UA: &str = "CoolerScreen/2.x (+github.com/oldbugo/coolerscreen)";

/// A client following at most `redirects` redirects (0 = none).
pub fn client(redirects: usize) -> Result<Client> {
    Client::builder()
        .user_agent(UA)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(3600))
        .redirect(if redirects == 0 { Policy::none() } else { Policy::limited(redirects) })
        .build()
        .map_err(|e| SourceError(format!("Could not start the network client: {e}")))
}

/// GET a JSON document (response capped at 5 MB).
pub fn get_json(url: reqwest::Url, what: &str) -> Result<serde_json::Value> {
    let c = client(0)?;
    let resp = c
        .get(url)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(20))
        .send()
        .map_err(|e| SourceError(format!("Could not reach {what}: {e}")))?;
    if !resp.status().is_success() {
        return err(format!("{what} answered {}.", resp.status().as_u16()));
    }
    let mut body = Vec::new();
    resp.take(5_000_001).read_to_end(&mut body).map_err(|e| SourceError(format!("Could not read the answer of {what}: {e}")))?;
    if body.len() > 5_000_000 {
        return err(format!("The answer of {what} is too large."));
    }
    serde_json::from_slice(&body).map_err(|_| SourceError(format!("{what} sent something unexpected.")))
}

/// Stream a response body to `dest` (via `dest.part`), refusing more than `max` bytes. Reports progress when the length is known.
pub fn stream_to_file(resp: Response, dest: &Path, max: u64, progress: Progress, cancel: &Cancel) -> Result<()> {
    let total = resp.content_length().unwrap_or(0);
    if total > max {
        return err(format!("The file is larger than {} MB.", max / 1_000_000));
    }
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = std::path::PathBuf::from(part);
    let res = (|| -> Result<()> {
        let mut out = std::fs::File::create(&part)?;
        let mut r = resp;
        let mut buf = vec![0u8; 64 * 1024];
        let mut got: u64 = 0;
        loop {
            cancel.check()?;
            let n = r.read(&mut buf).map_err(|e| SourceError(format!("Download failed: {e}")))?;
            if n == 0 {
                break;
            }
            got += n as u64;
            if got > max {
                return err(format!("The file is larger than {} MB.", max / 1_000_000));
            }
            out.write_all(&buf[..n])?;
            if total > 0 {
                progress(got as f32 / total as f32, "");
            }
        }
        out.flush()?;
        Ok(())
    })();
    match res {
        Ok(()) => std::fs::rename(&part, dest).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            e.into()
        }),
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}
