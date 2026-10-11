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
    // NVENC: upload a generated clip, scale on the GPU, encode with h264_nvenc (this also makes the NVDEC test clip).
    let nvenc = ffmpeg::run(
        ff,
        &crate::ff_args![
            "-f", "lavfi", "-i", "testsrc2=s=320x180:r=25:d=0.4", "-vf", "format=nv12,hwupload_cuda,scale_cuda=256:144",
            "-c:v", "h264_nvenc", "-y", clip_s
        ],
        &c,
        None,
    )
    .is_ok();
    if !nvenc {
        let _ = ffmpeg::run(ff, &crate::ff_args!["-f", "lavfi", "-i", "testsrc2=s=256x144:r=25:d=0.4", "-c:v", "libx264", "-y", clip_s], &c, None);
    }
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

/// Decode on the GPU for this source? HEVC, above 2.5K, or long (> 60 s) sources; never rotated phone videos or images.
pub fn use_nvdec(info: &SourceInfo) -> bool {
    let c = caps();
    c.nvdec && !info.is_image && !info.rotated && wants_gpu(info)
}

pub fn wants_gpu(info: &SourceInfo) -> bool {
    info.codec.as_deref() == Some("hevc") || info.width.unwrap_or(0) > 2560 || info.height.unwrap_or(0) > 1600 || info.duration.unwrap_or(0.0) > 60.0
}
