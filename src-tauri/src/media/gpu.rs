//! NVIDIA decode/encode detection (once, at startup) and the policy for when to use it.
//!
//! Perf report (2026-10-11): NVDEC + scale_cuda halves loop-frame extraction for HEVC and 4K sources and frees the CPU;
//! for ~2.4K H.264 there is no gain. D3D11VA with a download was slower than the CPU, so it is never used.
//! Every GPU path falls back to the CPU when ffmpeg fails (driver missing, unsupported profile, ...).
use super::{ffmpeg, CancelToken, SourceInfo};
use serde::Serialize;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, Default, Serialize, PartialEq, Eq)]
pub struct GpuCaps {
    /// CUDA hwaccel decode (NVDEC) with scale_cuda.
    pub nvdec: bool,
    /// h264_nvenc encode (proxies).
    pub nvenc: bool,
}

static CAPS: OnceLock<GpuCaps> = OnceLock::new();

/// Probe the GPU once (about 0.3-0.6 s; call at startup in the background). `CS_NO_GPU=1` disables it.
pub fn detect(ffmpeg_path: &Path) -> GpuCaps {
    *CAPS.get_or_init(|| {
        if std::env::var_os("CS_NO_GPU").is_some_and(|v| v != "0") {
            return GpuCaps::default();
        }
        probe_caps(ffmpeg_path)
    })
}

/// Already-detected caps (CPU only if detect() has not run).
pub fn caps() -> GpuCaps {
    CAPS.get().copied().unwrap_or_default()
}

fn probe_caps(ff: &Path) -> GpuCaps {
    let c = CancelToken::new();
    let dir = std::env::temp_dir().join(format!("cs-gpuprobe-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let clip = dir.join("probe.mp4");
    let clip_s = ffmpeg::p(&clip);
    // NVENC: upload a generated clip, scale on the GPU, encode with h264_nvenc.
    let nvenc = ffmpeg::run(
        ff,
        &crate::ff_args![
            "-f", "lavfi", "-i", "testsrc2=s=320x180:r=25:d=0.2", "-vf", "format=nv12,hwupload_cuda,scale_cuda=256:144",
            "-c:v", "h264_nvenc", "-f", "null", "-"
        ],
        &c,
        None,
    )
    .is_ok();
    // NVDEC test clip: H.264 High (libx264). Measured 2026-10-11 on an RTX 5070 Ti with this ffmpeg 6.1.1: NVDEC
    // refuses some H.264 *Main* profile streams (cuvidCreateDecoder: invalid argument) while High and HEVC decode
    // fine, so every GPU path falls back to the CPU per file anyway.
    let _ = ffmpeg::run(
        ff,
        &crate::ff_args!["-f", "lavfi", "-i", "testsrc2=s=640x360:r=25:d=0.4", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-y", clip_s],
        &c,
        None,
    );
    let nvdec = clip.is_file()
        && ffmpeg::run(
            ff,
            &crate::ff_args![
                "-hwaccel", "cuda", "-hwaccel_output_format", "cuda", "-i", clip_s, "-vf",
                "scale_cuda=64:36:format=nv12,hwdownload,format=nv12", "-f", "null", "-"
            ],
            &c,
            None,
        )
        .is_ok();
    let _ = std::fs::remove_dir_all(&dir);
    GpuCaps { nvdec, nvenc }
}

/// NVDEC + scale_cuda for small frames (loop search, loop cards): only above 2.5K, where it pays off
/// (measured 2026-10-11, 9800X3D / 5070 Ti, idle: 4K H.264 87 s loop frames 6.4 s CPU -> 3.9 s; 1080p HEVC 0.95 s CPU,
/// no gain). The GPU pre-scale changes the small frames slightly (not bit-exact with 1.2); the CPU path is exact.
pub fn use_nvdec_small(info: &SourceInfo) -> bool {
    let c = caps();
    c.nvdec && !info.is_image && !info.rotated && (info.width.unwrap_or(0) > 2560 || info.height.unwrap_or(0) > 1600)
}

/// NVDEC + NVENC for proxies: HEVC, above 2.5K, or long (> 60 s) sources (perf report: HEVC 25 s 2.6 -> 1.3 s,
/// 4K 90 s 11.1 -> 4.0 s).
pub fn use_gpu_proxy(info: &SourceInfo) -> bool {
    let c = caps();
    c.nvdec && c.nvenc && !info.is_image && !info.rotated && wants_gpu(info)
}

/// NVDEC for the full-resolution decode of a version (raw intermediate). Off by default: the decode is bit-exact but
/// measured equal or slower (1080p HEVC 10 s: 1.37 vs 1.46 s; 4K: 1.55 vs 2.45 s: downloading 4K frames costs more
/// than the decode saves). `CS_HW_CONVERT=1` turns it on for experiments.
pub fn use_nvdec_convert(info: &SourceInfo) -> bool {
    std::env::var_os("CS_HW_CONVERT").is_some_and(|v| v == "1") && caps().nvdec && !info.is_image && !info.rotated
}

pub fn wants_gpu(info: &SourceInfo) -> bool {
    info.codec.as_deref() == Some("hevc") || info.width.unwrap_or(0) > 2560 || info.height.unwrap_or(0) > 1600 || info.duration.unwrap_or(0.0) > 60.0
}
