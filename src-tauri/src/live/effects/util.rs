//! Shared helpers for the generated effects: fast sine, hashing, colour palettes, dithered pixel writes, bilinear upscale.
use std::sync::OnceLock;

/// sin(2*pi*t) for t in turns, accurate to about 0.1 % (parabola with one refinement step); no table, no libm call.
#[inline]
pub fn sin_tau(t: f32) -> f32 {
    let x = t - t.floor() - 0.5;
    let y = 8.0 * x * (1.0 - 2.0 * x.abs());
    let y = y + 0.225 * (y * y.abs() - y);
    -y
}

/// Phase in turns of `m * phi` kept small (phi is the absolute loop phase, which grows for the whole run).
#[inline]
pub fn turns(m: f64, phi: f64) -> f32 {
    (m * phi).fract() as f32
}

#[inline]
pub fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Integer hash -> 0..1.
#[inline]
pub fn hash(a: u32, b: u32, seed: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ seed.wrapping_mul(0xC2B2_AE3D) ^ 0x27D4_EB2F;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / 16_777_216.0
}

/// 64x64 white noise in -0.5..0.5, added before rounding so smooth gradients do not band (and JPEG blocks stay quiet).
pub fn dither() -> &'static [f32; 4096] {
    static D: OnceLock<[f32; 4096]> = OnceLock::new();
    D.get_or_init(|| {
        let mut d = [0f32; 4096];
        for (i, v) in d.iter_mut().enumerate() {
            *v = hash(i as u32, 7, 99) - 0.5;
        }
        d
    })
}

/// Write one opaque pixel from float channels (0..255) with dither `n` (-0.5..0.5).
#[inline]
pub fn put(px: &mut [u8], r: f32, g: f32, b: f32, n: f32) {
    px[0] = (r + 0.5 + n).clamp(0.0, 255.0) as u8;
    px[1] = (g + 0.5 + n).clamp(0.0, 255.0) as u8;
    px[2] = (b + 0.5 + n).clamp(0.0, 255.0) as u8;
    px[3] = 255;
}

/// The part of the canvas an effect fills: the effect rectangle (the visible area, in float pixels, may reach past the canvas)
/// clipped to the canvas. u and v are 0..1 across the effect rectangle, so the picture is the design canvas scaled into it.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub cw: usize,
    pub x0: usize,
    pub x1: usize,
    pub y0: usize,
    pub y1: usize,
    pub rx: f32,
    pub ry: f32,
    pub rw: f32,
    pub rh: f32,
}

impl Span {
    pub fn new(cw: usize, ch: usize, rect: (i64, i64, i64, i64)) -> Option<Span> {
        let (x0, y0, x1, y1) = rect;
        let (cx0, cy0, cx1, cy1) = (x0.max(0), y0.max(0), x1.min(cw as i64), y1.min(ch as i64));
        if x1 <= x0 || y1 <= y0 || cx1 <= cx0 || cy1 <= cy0 {
            return None;
        }
        Some(Span { cw, x0: cx0 as usize, x1: cx1 as usize, y0: cy0 as usize, y1: cy1 as usize, rx: x0 as f32, ry: y0 as f32, rw: (x1 - x0) as f32, rh: (y1 - y0) as f32 })
    }
    #[inline]
    pub fn u(&self, x: usize) -> f32 {
        ((x as f32 + 0.5) - self.rx) / self.rw
    }
    #[inline]
    pub fn v(&self, y: usize) -> f32 {
        ((y as f32 + 0.5) - self.ry) / self.rh
    }
    /// Pixel scale against the 1680-wide design canvas (sprite sizes and speeds follow the picture size).
    pub fn k(&self) -> f32 {
        self.rw / 1680.0
    }
    pub fn key(&self) -> (usize, usize) {
        (self.rw as usize, self.rh as usize)
    }
}

/// A colour ramp as a 256-entry table (channels 0..255).
#[derive(Clone)]
pub struct Palette {
    lut: Vec<[f32; 3]>,
    cyclic: bool,
}

impl Palette {
    pub fn new(colors: &[[u8; 3]], cyclic: bool) -> Palette {
        let cols: Vec<[f32; 3]> = if colors.is_empty() { vec![[0.0; 3]] } else { colors.iter().map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).collect() };
        let n = cols.len();
        let segs = if cyclic { n } else { (n - 1).max(1) };
        let lut = (0..256)
            .map(|i| {
                let p = i as f32 / 255.0 * segs as f32;
                let s = (p.floor() as usize).min(segs - 1);
                let f = p - s as f32;
                let (a, b) = (cols[s % n], cols[(s + 1) % n]);
                // ease inside each segment so the ramp has no visible creases at the stops
                let f = f * f * (3.0 - 2.0 * f);
                [a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
            })
            .collect();
        Palette { lut, cyclic }
    }
    /// Colour at `pos` (0..1; wraps for a cyclic ramp, clamps otherwise).
    #[inline]
    pub fn at(&self, pos: f32) -> [f32; 3] {
        let p = if self.cyclic { pos - pos.floor() } else { pos.clamp(0.0, 1.0) };
        let f = p * 255.0;
        let i = f as usize;
        let j = (i + 1).min(255);
        let t = f - i as f32;
        let (a, b) = (self.lut[i], self.lut[j]);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
    }
}

/// Bilinear upscale of a float RGBA grid (`gw` x `gh`, four floats per cell: channels 0..255 and alpha 255) into the span, dithered.
/// The crate is built for size (opt-level "s"), where the compiler does not vectorise, so the hot loops use SSE2 directly
/// (x86_64 always has it); other targets get the plain loops.
pub struct Upscaler {
    xs: Vec<(usize, usize, f32)>,
    key: (usize, usize, usize, usize),
    row: Vec<f32>,
}

impl Upscaler {
    pub fn new() -> Upscaler {
        Upscaler { xs: Vec::new(), key: (0, 0, 0, 0), row: Vec::new() }
    }
    pub fn draw(&mut self, sp: &Span, grid: &[f32], gw: usize, gh: usize, data: &mut [u8]) {
        let key = (sp.x0, sp.x1, gw, sp.rw as usize);
        if self.key != key || self.xs.len() != sp.x1 - sp.x0 {
            self.key = key;
            self.xs = (sp.x0..sp.x1)
                .map(|x| {
                    let g = (sp.u(x) * gw as f32 - 0.5).clamp(0.0, (gw - 1) as f32);
                    let i = (g as usize).min(gw - 1);
                    (i, (i + 1).min(gw - 1), g - i as f32)
                })
                .collect();
        }
        self.row.resize(gw * 4, 0.0);
        let dz = dither();
        for y in sp.y0..sp.y1 {
            let g = (sp.v(y) * gh as f32 - 0.5).clamp(0.0, (gh - 1) as f32);
            let j0 = (g as usize).min(gh - 1);
            let j1 = (j0 + 1).min(gh - 1);
            let fy = g - j0 as f32;
            lerp_rows(&grid[j0 * gw * 4..][..gw * 4], &grid[j1 * gw * 4..][..gw * 4], fy, &mut self.row);
            let out = &mut data[(y * sp.cw + sp.x0) * 4..(y * sp.cw + sp.x1) * 4];
            let nrow = &dz[(y & 63) << 6..][..64];
            hrow(out, &self.row, &self.xs, nrow, sp.x0);
        }
    }
}

fn lerp_rows(r0: &[f32], r1: &[f32], fy: f32, out: &mut [f32]) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::*;
        let f = _mm_set1_ps(fy);
        let mut i = 0;
        while i + 4 <= out.len() {
            let a = _mm_loadu_ps(r0.as_ptr().add(i));
            let b = _mm_loadu_ps(r1.as_ptr().add(i));
            _mm_storeu_ps(out.as_mut_ptr().add(i), _mm_add_ps(a, _mm_mul_ps(_mm_sub_ps(b, a), f)));
            i += 4;
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    for ((o, a), b) in out.iter_mut().zip(r0).zip(r1) {
        *o = a + (b - a) * fy;
    }
}

/// One output row: horizontal blend of two grid cells per pixel, dither, round to bytes.
fn hrow(out: &mut [u8], row: &[f32], xs: &[(usize, usize, f32)], nrow: &[f32], x0: usize) {
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::*;
        for (k, (px, &(i0, i1, fx))) in out.chunks_exact_mut(4).zip(xs).enumerate() {
            let a = _mm_loadu_ps(row.as_ptr().add(i0 * 4));
            let b = _mm_loadu_ps(row.as_ptr().add(i1 * 4));
            let v = _mm_add_ps(a, _mm_mul_ps(_mm_sub_ps(b, a), _mm_set1_ps(fx)));
            let v = _mm_add_ps(v, _mm_set1_ps(0.5 + nrow[(x0 + k) & 63]));
            let i = _mm_cvttps_epi32(v);
            let p = _mm_packus_epi16(_mm_packs_epi32(i, i), _mm_setzero_si128());
            (px.as_mut_ptr() as *mut i32).write_unaligned(_mm_cvtsi128_si32(p));
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    for (k, (px, &(i0, i1, fx))) in out.chunks_exact_mut(4).zip(xs).enumerate() {
        let (a, b) = (&row[i0 * 4..i0 * 4 + 4], &row[i1 * 4..i1 * 4 + 4]);
        put(px, a[0] + (b[0] - a[0]) * fx, a[1] + (b[1] - a[1]) * fx, a[2] + (b[2] - a[2]) * fx, nrow[(x0 + k) & 63]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_sine_is_close() {
        for i in 0..1000 {
            let t = i as f32 / 137.0 - 3.0;
            assert!((sin_tau(t) - (t * std::f32::consts::TAU).sin()).abs() < 0.003, "t={t}");
        }
    }

    #[test]
    fn hash_is_unit_range_and_varies() {
        let v: Vec<f32> = (0..1000).map(|i| hash(i, 3, 1)).collect();
        assert!(v.iter().all(|x| (0.0..1.0).contains(x)));
        let mean = v.iter().sum::<f32>() / 1000.0;
        assert!((mean - 0.5).abs() < 0.05);
    }

    #[test]
    fn palette_ends_and_wrap() {
        let p = Palette::new(&[[0, 0, 0], [255, 255, 255]], false);
        assert_eq!(p.at(0.0)[0], 0.0);
        assert!((p.at(1.0)[0] - 255.0).abs() < 0.01);
        let c = Palette::new(&[[0, 0, 0], [200, 0, 0]], true);
        assert!((c.at(0.0)[0] - c.at(1.0)[0]).abs() < 0.01);
    }
}
