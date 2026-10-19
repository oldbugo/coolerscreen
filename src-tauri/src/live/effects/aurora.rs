//! Aurora: vertical rays of light hanging from soft glow bands, drifting sideways at different speeds (some left, some right)
//! over a dark night sky with twinkling stars. Each layer is separable (a ray strength per column times a glow profile per
//! row), so a frame is a handful of multiply-adds per pixel and there is no vertical motion at all.
use super::util::*;
use super::{Effect, Params};

struct Layer {
    /// Whole turns of the main wave across the picture, and its sideways speed in turns per loop (sign = direction).
    k: f32,
    m: f64,
    /// Fine ripple (the rays themselves).
    kf: f32,
    mf: f64,
    /// Glow band: centre (0..1 down the picture), half width, strength.
    cy: f32,
    w: f32,
    amp: f32,
    phase: f32,
}

pub struct Aurora {
    p: Params,
    pal: Palette,
    layers: Vec<Layer>,
    /// Per layer: the colour of every column (RGBA bytes, ray strength included); per layer and row: glow strength 0..255;
    /// per row: the sky colour times 4 (RGBA, alpha 1020); dither 0..3 for 64 rows of the span.
    colb: Vec<Vec<u8>>,
    prof8: Vec<Vec<u8>>,
    sky: Vec<u8>,
    sky_k: Vec<u8>,
    noise: Vec<u16>,
    stars: Vec<(f32, f32, f32, f64, f32)>,
    key: (usize, usize, usize, usize),
}

impl Aurora {
    pub fn new(p: Params) -> Aurora {
        let pal = Palette::new(&p.colors, false);
        let fine = 10.0 + (p.density * 22.0).round();
        let layers = vec![
            Layer { k: 3.0, m: 1.0, kf: fine, mf: 2.0, cy: 0.60, w: 0.20, amp: 1.0, phase: 0.1 },
            Layer { k: 5.0, m: -1.0, kf: (fine * 0.6).round(), mf: -1.0, cy: 0.48, w: 0.26, amp: 0.65, phase: 0.43 },
            Layer { k: 2.0, m: 1.0, kf: (fine * 1.4).round(), mf: 3.0, cy: 0.72, w: 0.14, amp: 0.5, phase: 0.77 },
        ];
        let stars = (0..140u32)
            .map(|i| (hash(i, 1, p.seed), hash(i, 2, p.seed) * 0.62, 0.35 + 0.65 * hash(i, 3, p.seed), 1.0 + (hash(i, 4, p.seed) * 3.0).floor() as f64, hash(i, 5, p.seed)))
            .collect();
        Aurora { p, pal, layers, colb: Vec::new(), prof8: Vec::new(), sky: Vec::new(), sky_k: Vec::new(), noise: Vec::new(), stars, key: (0, 0, 0, 0) }
    }
}

impl Effect for Aurora {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let n = sp.x1 - sp.x0;
        let nl = self.layers.len();
        let key = (sp.x0, sp.x1, sp.y0, sp.y1);
        // rows depend only on the picture size: build once per size
        if self.key != key || self.prof8.len() != nl || self.sky_k.len() != sp.y1 - sp.y0 {
            self.key = key;
            let wscale = 0.6 + 0.9 * self.p.scale;
            self.prof8 = self
                .layers
                .iter()
                .map(|l| {
                    (sp.y0..sp.y1)
                        .map(|y| {
                            let d = sp.v(y) - l.cy;
                            let w = l.w * wscale;
                            let z = if d < 0.0 { d / (w * 1.8) } else { d / (w * 0.55) };
                            ((-z * z).exp() * 255.0 + 0.5) as u8
                        })
                        .collect()
                })
                .collect();
            let c0 = self.pal.at(0.0);
            // sky: first colour, 10 % to 40 % brighter towards the bottom (a shade of it, bright enough to keep a faint blue night)
            let base = [c0[0].max(40.0), c0[1].max(40.0), c0[2].max(60.0)];
            self.sky = (0..n).flat_map(|_| [base[0] as u8, base[1] as u8, base[2] as u8, 0]).collect();
            self.sky_k = (sp.y0..sp.y1)
                .map(|y| {
                    let v = sp.v(y).clamp(0.0, 1.0);
                    ((0.04 + 0.16 * v * v) * 255.0) as u8
                })
                .collect();
            let dz = dither();
            self.noise = Vec::with_capacity(64 * n * 4);
            for r in 0..64usize {
                for x in sp.x0..sp.x1 {
                    let z = ((dz[(r << 6) | (x & 63)] + 0.5) * 3.999) as u16;
                    self.noise.extend_from_slice(&[z, z, z, 1020]);
                }
            }
        }
        if self.colb.len() != nl || self.colb[0].len() != n * 4 {
            self.colb = (0..nl).map(|_| vec![0u8; n * 4]).collect();
        }
        // ray strength and colour for every column of every layer
        let expo = 1.4 + 2.2 * (1.0 - self.p.density);
        let gain = self.p.brightness * 1.25;
        for (l, lay) in self.layers.iter().enumerate() {
            let (tm, tf, tc) = (turns(lay.m, phi), turns(lay.mf, phi), turns(1.0, phi));
            for i in 0..n {
                let u = sp.u(sp.x0 + i);
                let main = (0.5 + 0.5 * sin_tau(lay.k * u - tm + lay.phase + 0.16 * sin_tau(2.0 * u + lay.phase))).powf(expo);
                let fine = 0.5 + 0.5 * sin_tau(lay.kf * u - tf + lay.phase * 3.0 + 0.3 * sin_tau(u * 3.0 - tm));
                let ray = main * (0.35 + 0.65 * fine * fine);
                let cpos = 0.5 + 0.5 * sin_tau(0.8 * u - tc + lay.phase * 2.0);
                let c = self.pal.at(cpos);
                let a = ray * lay.amp * gain * 1.3;
                let px = &mut self.colb[l][i * 4..i * 4 + 4];
                px.copy_from_slice(&[(c[0] * a).min(255.0) as u8, (c[1] * a).min(255.0) as u8, (c[2] * a).min(255.0) as u8, 0]);
            }
        }
        // Integer pixel loop on flat RGBA bytes (u16 maths): every term is (colour byte * strength byte) >> 6, i.e. 4x the byte scale;
        // the three layers and the sky (a fourth layer with a constant colour) are added with 0..3 of dither, then two bits drop.
        let (c0, c1, c2, c3) = (&self.colb[0], &self.colb[1], &self.colb[2], &self.sky);
        for y in sp.y0..sp.y1 {
            let j = y - sp.y0;
            let (p0, p1, p2, p3) = (self.prof8[0][j] as u16, self.prof8[1][j] as u16, self.prof8[2][j] as u16, self.sky_k[j] as u16);
            let out = &mut data[(y * sp.cw + sp.x0) * 4..(y * sp.cw + sp.x1) * 4];
            let nz = &self.noise[(y & 63) * n * 4..][..n * 4];
            mix_row(out, c0, c1, c2, c3, nz, [p0, p1, p2, p3]);
        }
        // stars: tiny dots that fade in and out (a fade, never a move)
        let k = sp.k().max(0.5);
        for &(su, sv, sb, sm, sph) in &self.stars {
            let tw = 0.5 + 0.5 * sin_tau(turns(sm, phi) + sph);
            let a = sb * (0.15 + 0.85 * tw * tw) * self.p.brightness.min(1.0);
            let x = (sp.rx + su * sp.rw) as i64;
            let y = (sp.ry + sv * sp.rh) as i64;
            let size = if sb > 0.85 { (2.0 * k).round().max(1.0) as i64 } else { 1 };
            for dy in 0..size {
                for dx in 0..size {
                    let (px, py) = (x + dx, y + dy);
                    if px < sp.x0 as i64 || px >= sp.x1 as i64 || py < sp.y0 as i64 || py >= sp.y1 as i64 {
                        continue;
                    }
                    let o = (py as usize * sp.cw + px as usize) * 4;
                    for c in 0..3 {
                        data[o + c] = (data[o + c] as f32 + 230.0 * a).min(255.0) as u8;
                    }
                }
            }
        }
    }
}

/// One row of the aurora: out = min(255, (sum of colour*strength >> 6 over four layers + dither) >> 2) on flat RGBA bytes.
/// SSE2 by hand (see `util::Upscaler`): 16 bytes per step.
pub(super) fn mix_row(out: &mut [u8], c0: &[u8], c1: &[u8], c2: &[u8], c3: &[u8], nz: &[u16], p: [u16; 4]) {
    let n = out.len();
    let (c0, c1, c2, c3, nz) = (&c0[..n], &c1[..n], &c2[..n], &c3[..n], &nz[..n]);
    let mut i = 0;
    #[cfg(target_arch = "x86_64")]
    unsafe {
        use std::arch::x86_64::*;
        let z = _mm_setzero_si128();
        let pv = [_mm_set1_epi16(p[0] as i16), _mm_set1_epi16(p[1] as i16), _mm_set1_epi16(p[2] as i16), _mm_set1_epi16(p[3] as i16)];
        while i + 16 <= n {
            let (mut lo, mut hi) = (_mm_loadu_si128(nz.as_ptr().add(i) as *const __m128i), _mm_loadu_si128(nz.as_ptr().add(i + 8) as *const __m128i));
            for (c, pvk) in [c0, c1, c2, c3].into_iter().zip(pv) {
                let v = _mm_loadu_si128(c.as_ptr().add(i) as *const __m128i);
                lo = _mm_add_epi16(lo, _mm_srli_epi16(_mm_mullo_epi16(_mm_unpacklo_epi8(v, z), pvk), 6));
                hi = _mm_add_epi16(hi, _mm_srli_epi16(_mm_mullo_epi16(_mm_unpackhi_epi8(v, z), pvk), 6));
            }
            let r = _mm_packus_epi16(_mm_srli_epi16(lo, 2), _mm_srli_epi16(hi, 2));
            _mm_storeu_si128(out.as_mut_ptr().add(i) as *mut __m128i, r);
            i += 16;
        }
    }
    while i < n {
        let sum = ((c0[i] as u16 * p[0]) >> 6) + ((c1[i] as u16 * p[1]) >> 6) + ((c2[i] as u16 * p[2]) >> 6) + ((c3[i] as u16 * p[3]) >> 6) + nz[i];
        out[i] = (sum >> 2).min(255) as u8;
        i += 1;
    }
}
