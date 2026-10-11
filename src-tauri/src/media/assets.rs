//! Small generated files: the rounded-corner mask, a source's app-only assets (thumb, frame, proxy), the splash, the
//! blank clip. Ports of convert.js cornerMaskFile / makeSourceAssets / makeSplash / makeBlank.
use super::ffmpeg::{self, p};
use super::graph::{visible_rect, Corners, Mask, DEVICE_FPS, H, W};
use super::{gpu, js_round, CancelToken, MediaError, Progress, Result, SourceInfo};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQ: AtomicU64 = AtomicU64::new(0);
/// A unique temp name next to `f` (several operations may make the same cached file at once).
pub(crate) fn tmp_next_to(f: &Path, ext: &str) -> PathBuf {
    let n = UNIQ.fetch_add(1, Ordering::Relaxed);
    let mut s = f.as_os_str().to_owned();
    s.push(format!(".{}-{n}.tmp.{ext}", std::process::id()));
    PathBuf::from(s)
}

/// The corner mask PNG for these corners and notch (black outside the visible rect rounded with radius r), cached in
/// `<base>/masks` under the same name 1.2 uses. None when corners are off.
pub fn corner_mask_file(ff: &Path, base_dir: &Path, corners: &Corners, mask: &Mask, cancel: &CancelToken) -> Result<Option<PathBuf>> {
    if !corners.is_on() {
        return Ok(None);
    }
    let r = visible_rect(mask);
    let (x0, y0, x1, y1) = (r.x0 as i64, r.y0 as i64, r.x1 as i64, r.y1 as i64);
    let rr = (js_round(corners.r) as i64).min((x1 - x0).min(y1 - y0) / 2);
    let dir = base_dir.join("masks");
    let f = dir.join(format!("corners-{W}x{H}-r{rr}-{x0}-{y0}-{x1}-{y1}.png"));
    if f.is_file() {
        return Ok(Some(f));
    }
    std::fs::create_dir_all(&dir)?;
    let dx = format!("max(max({}-(X+0.5),(X+0.5)-{}),0)", x0 + rr, x1 - rr);
    let dy = format!("max(max({}-(Y+0.5),(Y+0.5)-{}),0)", y0 + rr, y1 - rr);
    let a = format!("255*clip(hypot({dx},{dy})-{rr}+0.5,0,1)");
    let tmp = tmp_next_to(&f, "png");
    let res = ffmpeg::run(
        ff,
        &crate::ff_args![
            "-f", "lavfi", "-i", format!("color=black:s={W}x{H}:d=1"), "-frames:v", "1", "-vf",
            format!("format=rgba,geq=r='0':g='0':b='0':a='{a}'"), "-y", p(&tmp)
        ],
        cancel,
        None,
    );
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    if std::fs::rename(&tmp, &f).is_err() {
        let _ = std::fs::remove_file(&tmp); // another operation made it first
    }
    Ok(Some(f))
}

/// Browser-playable formats shown without a proxy.
fn can_play_direct(file: &Path, info: &SourceInfo) -> bool {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if info.is_image {
        ["jpg", "jpeg", "png", "webp"].contains(&ext.as_str())
    } else {
        ["mp4", "m4v", "webm"].contains(&ext.as_str())
            && matches!(info.codec.as_deref(), Some("h264" | "vp8" | "vp9" | "av1"))
    }
}
pub const PROXY_MAX_SECONDS: u32 = 600;

/// App-only assets of an original: thumb.jpg (240x108 cover), frame.jpg (1200x540 cover) and, when the browser can't
/// show the source, proxy.mp4 / proxy.jpg (aspect and timing kept; at most 1600 px). Returns the proxy file name.
/// Proxies of HEVC / 4K / long sources use NVDEC + scale_cuda + NVENC when available (CPU fallback).
pub fn make_source_assets(ff: &Path, file: &Path, dir: &Path, info: &SourceInfo, cancel: &CancelToken, progress: Progress) -> Result<Option<String>> {
    std::fs::create_dir_all(dir)?;
    let mut seek: Vec<String> = vec![];
    if let (false, Some(d)) = (info.is_image, info.duration) {
        seek = crate::ff_args!["-ss", (d / 2.0).min(1.0)];
    }
    let cover = |w: u32, h: u32| format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h},setsar=1");
    // thumb + frame from one decode
    let mut a = seek.clone();
    a.extend(crate::ff_args![
        "-i", p(file), "-filter_complex", format!("[0:v]split[a][b];[a]{}[t];[b]{}[f]", cover(240, 108), cover(1200, 540)),
        "-map", "[t]", "-frames:v", "1", "-q:v", "4", "-y", p(&dir.join("thumb.jpg")),
        "-map", "[f]", "-frames:v", "1", "-q:v", "4", "-y", p(&dir.join("frame.jpg"))
    ]);
    ffmpeg::run(ff, &a, cancel, None)?;
    progress(0.1, "Preparing the preview");
    if can_play_direct(file, info) {
        return Ok(None);
    }
    if info.is_image {
        ffmpeg::run(
            ff,
            &crate::ff_args![
                "-i", p(file), "-frames:v", "1", "-vf",
                "scale=w='min(2400,iw)':h='min(2400,ih)':force_original_aspect_ratio=decrease", "-q:v", "3", "-y",
                p(&dir.join("proxy.jpg"))
            ],
            cancel,
            None,
        )?;
        return Ok(Some("proxy.jpg".into()));
    }
    let tmp = dir.join("proxy.tmp.mp4");
    let total = info.duration.unwrap_or(1.0).min(PROXY_MAX_SECONDS as f64).max(0.1);
    let on_time = |t: f64| progress(0.1 + 0.9 * (t / total).min(1.0), "Preparing the preview");
    let mut done = false;
    if gpu::use_gpu_proxy(info) {
        if let (Some(w), Some(h)) = (info.width, info.height) {
            let (pw, ph) = fit_within(w, h, 1600);
            let r = ffmpeg::run(
                ff,
                &crate::ff_args![
                    "-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-i", p(file), "-t", PROXY_MAX_SECONDS, "-an",
                    "-vf", format!("scale_cuda={pw}:{ph}:format=nv12"), "-c:v", "h264_nvenc", "-preset", "p4", "-rc", "vbr",
                    "-cq", "23", "-b:v", "0", "-movflags", "+faststart", "-y", p(&tmp)
                ],
                cancel,
                Some(&on_time),
            );
            match r {
                Ok(()) => done = true,
                Err(MediaError::Cancelled) => return Err(MediaError::Cancelled),
                Err(_) => {} // CPU fallback below
            }
        }
    }
    if !done {
        ffmpeg::run(
            ff,
            &crate::ff_args![
                "-i", p(file), "-t", PROXY_MAX_SECONDS, "-an", "-vf",
                "scale=w='min(1600,iw)':h='min(1600,ih)':force_original_aspect_ratio=decrease,scale=trunc(iw/2)*2:trunc(ih/2)*2,setsar=1,format=yuv420p",
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "23", "-movflags", "+faststart", "-y", p(&tmp)
            ],
            cancel,
            Some(&on_time),
        )?;
    }
    std::fs::rename(&tmp, dir.join("proxy.mp4"))?;
    Ok(Some("proxy.mp4".into()))
}

/// Largest even size within max x max keeping the aspect (never upscaling), like the CPU proxy filter.
pub fn fit_within(w: u32, h: u32, max: u32) -> (u32, u32) {
    let s = (max as f64 / w as f64).min(max as f64 / h as f64).min(1.0);
    let ew = ((w as f64 * s).round() as u32) & !1;
    let eh = ((h as f64 * s).round() as u32) & !1;
    (ew.max(2), eh.max(2))
}

/// Tiny 1-frame black 1080x2400 MJPEG mp4 (CLI "clear").
pub fn make_blank(ff: &Path, out: &Path, cancel: &CancelToken) -> Result<u64> {
    ffmpeg::run(
        ff,
        &crate::ff_args![
            "-f", "lavfi", "-i", "color=black:s=1080x2400:r=1", "-frames:v", "1", "-c:v", "mjpeg", "-pix_fmt", "yuvj420p",
            "-q:v", "31", "-movflags", "+faststart", "-y", p(out)
        ],
        cancel,
        None,
    )?;
    Ok(std::fs::metadata(out)?.len())
}

/// The built-in intro (startup slot): black, small centred "CoolerScreen" wordmark fading in and out (splash.js).
pub const SPLASH_VERSION: u32 = 1;
pub const SPLASH_SECONDS: f64 = 1.5;
pub const SPLASH_TEXT: &str = "CoolerScreen";
/// Size before the file has been made once (measured: about 0.54 MB).
pub const SPLASH_SIZE_ESTIMATE: u64 = 560_000;

fn splash_font() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into())).join("Fonts");
    ["segoeuisl.ttf", "segoeui.ttf", "arial.ttf"].iter().map(|f| dir.join(f)).find(|p| p.is_file())
}

pub fn make_splash(ff: &Path, out: &Path, cancel: &CancelToken) -> Result<u64> {
    let font = splash_font()
        .map(|f| format!("fontfile='{}':", f.to_string_lossy().replace('\\', "/").replace(':', "\\:")))
        .unwrap_or_default();
    let (t, fade) = (SPLASH_SECONDS, 0.4);
    let alpha = format!("if(lt(t\\,{fade})\\,t/{fade}\\,if(gt(t\\,{})\\,max(0\\,({t}-t)/{fade})\\,1))", t - fade);
    let g = format!(
        "color=black:s={W}x{H}:r={DEVICE_FPS}:d={t},drawtext={font}text='{SPLASH_TEXT}':fontsize=84:fontcolor=0xE6E8EC:alpha='{alpha}':x=(w-text_w)/2:y=(h-text_h)/2,transpose=1,format=yuvj420p"
    );
    ffmpeg::run(
        ff,
        &crate::ff_args!["-f", "lavfi", "-i", g, "-an", "-c:v", "mjpeg", "-q:v", "8", "-r", DEVICE_FPS, "-movflags", "+faststart", "-y", p(out)],
        cancel,
        None,
    )?;
    Ok(std::fs::metadata(out)?.len())
}

/// The cached splash `<base>/splash-v1.mp4` (made once), as splash.js get(): (file, size).
pub fn splash(ff: &Path, base_dir: &Path, cancel: &CancelToken) -> Result<(PathBuf, u64)> {
    let f = base_dir.join(format!("splash-v{SPLASH_VERSION}.mp4"));
    if !f.is_file() {
        std::fs::create_dir_all(base_dir)?;
        let tmp = tmp_next_to(&f, "mp4");
        make_splash(ff, &tmp, cancel)?;
        if std::fs::rename(&tmp, &f).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
    let size = std::fs::metadata(&f)?.len();
    Ok((f, size))
}

#[cfg(test)]
mod tests {
    #[test]
    fn fit() {
        assert_eq!(super::fit_within(3840, 2160, 1600), (1600, 900));
        assert_eq!(super::fit_within(1080, 1920, 1600), (900, 1600));
        assert_eq!(super::fit_within(640, 360, 1600), (640, 360));
    }
}
