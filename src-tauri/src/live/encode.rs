//! Landscape RGBA -> portrait (rotated 90 deg clockwise, like the clips' transpose=1) -> 4:2:0 JPEG within a size budget.
use super::{LH, LW, PH, PW};
use jpeg_encoder::{ColorType, Encoder, SamplingFactor};

/// Rotate a landscape RGBA image (LW x LH) 90 degrees clockwise into `dst` (PW x PH, RGBA). Landscape (x, y) goes to
/// portrait row x, column LH-1-y. Tiled so both sides stay cache friendly.
pub fn rotate_cw(src: &[u8], dst: &mut Vec<u8>) {
    dst.resize(PW * PH * 4, 0);
    const T: usize = 32;
    for y0 in (0..LH).step_by(T) {
        for x0 in (0..LW).step_by(T) {
            for y in y0..(y0 + T).min(LH) {
                for x in x0..(x0 + T).min(LW) {
                    let s = (y * LW + x) * 4;
                    let d = (x * PW + (PW - 1 - y)) * 4;
                    dst[d..d + 4].copy_from_slice(&src[s..s + 4]);
                }
            }
        }
    }
}

/// Adaptive-quality encoder: keeps the last quality that fitted, backs off when a frame is over budget (re-encoding that
/// frame), and creeps up when there is room.
pub struct JpegEncoder {
    pub quality: u8,
    calm: u32,
}

impl Default for JpegEncoder {
    fn default() -> Self {
        JpegEncoder { quality: 85, calm: 0 }
    }
}

impl JpegEncoder {
    /// Encode portrait RGBA (alpha ignored) into `out`; returns the size in bytes.
    pub fn encode(&mut self, rgba: &[u8], w: usize, h: usize, max_bytes: usize, out: &mut Vec<u8>) -> Result<usize, String> {
        loop {
            out.clear();
            let mut enc = Encoder::new(&mut *out, self.quality);
            // The screen only shows 4:2:0 (4:4:4, which jpeg-encoder picks at q >= 90, is silently ignored).
            enc.set_sampling_factor(SamplingFactor::F_2_2);
            enc.encode(rgba, w as u16, h as u16, ColorType::Rgba).map_err(|e| format!("JPEG encode failed: {e}"))?;
            if out.len() > max_bytes && self.quality > 20 {
                self.quality = self.quality.saturating_sub(4).max(20);
                self.calm = 0;
                continue;
            }
            if out.len() * 10 < max_bytes * 8 && self.quality < 92 {
                self.calm += 1;
                if self.calm >= 20 {
                    self.quality += 1;
                    self.calm = 0;
                }
            } else {
                self.calm = 0;
            }
            return Ok(out.len());
        }
    }
}

/// Frame budget: the vendor's 200 KB at most, and what the ~9 MB/s link can carry at this frame rate.
pub fn frame_budget(max_kb: u32, fps: u32) -> usize {
    ((max_kb as usize) * 1000).min(9_000_000 / fps.max(1) as usize)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_maps_corners_clockwise() {
        let mut src = vec![0u8; LW * LH * 4];
        // landscape top-left pixel -> portrait top-right; top-right -> bottom-right
        src[0] = 1;
        src[(LW - 1) * 4] = 2;
        let mut dst = Vec::new();
        rotate_cw(&src, &mut dst);
        assert_eq!(dst[(PW - 1) * 4], 1);
        assert_eq!(dst[((PH - 1) * PW + PW - 1) * 4], 2);
    }
}
