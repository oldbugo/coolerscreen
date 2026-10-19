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
    /// Per column and layer: premultiplied colour (r, g, b planes).
    cols: Vec<[Vec<f32>; 3]>,
    prof: Vec<Vec<f32>>,
    bg: Vec<[f32; 3]>,
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
        Aurora { p, pal, layers, cols: Vec::new(), prof: Vec::new(), bg: Vec::new(), stars, key: (0, 0, 0, 0) }
    }
}

impl Effect for Aurora {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let n = sp.x1 - sp.x0;
        let nl = self.layers.len();
        let key = (sp.x0, sp.x1, sp.y0, sp.y1);
        // rows depend only on the picture size: build once per size
        if self.key != key || self.prof.len() != nl || self.bg.len() != sp.y1 - sp.y0 {
            self.key = key;
            let wscale = 0.6 + 0.9 * self.p.scale;
            self.prof = self
                .layers
                .iter()
                .map(|l| {
                    (sp.y0..sp.y1)
                        .map(|y| {
                            let v = sp.v(y);
                            let d = v - l.cy;
                            let w = l.w * wscale;
                            let z = if d < 0.0 { d / (w * 1.8) } else { d / (w * 0.55) };
                            (-z * z).exp()
                        })
                        .collect()
                })
                .collect();
            let c0 = self.pal.at(0.0);
            self.bg = (sp.y0..sp.y1)
                .map(|y| {
                    let v = sp.v(y).clamp(0.0, 1.0);
                    let k = 0.03 + 0.12 * v * v;
                    [c0[0] * k + 2.0, c0[1] * k + 3.0, c0[2] * k + 7.0 * (1.0 - v) + 5.0]
                })
                .collect();
        }
        if self.cols.len() != nl || self.cols[0][0].len() != n {
            self.cols = (0..nl).map(|_| [vec![0.0; n], vec![0.0; n], vec![0.0; n]]).collect();
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
                self.cols[l][0][i] = c[0] * a;
                self.cols[l][1][i] = c[1] * a;
                self.cols[l][2][i] = c[2] * a;
            }
        }
        let dz = dither();
        let (c0, c1, c2) = (&self.cols[0], &self.cols[1], &self.cols[2]);
        for y in sp.y0..sp.y1 {
            let j = y - sp.y0;
            let (p0, p1, p2) = (self.prof[0][j], self.prof[1][j], self.prof[2][j]);
            let bg = self.bg[j];
            let out = &mut data[(y * sp.cw + sp.x0) * 4..(y * sp.cw + sp.x1) * 4];
            let nrow = &dz[(y & 63) << 6..][..64];
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                let r = bg[0] + c0[0][i] * p0 + c1[0][i] * p1 + c2[0][i] * p2;
                let g = bg[1] + c0[1][i] * p0 + c1[1][i] * p1 + c2[1][i] * p2;
                let b = bg[2] + c0[2][i] * p0 + c1[2][i] * p1 + c2[2][i] * p2;
                put(px, r, g, b, nrow[(sp.x0 + i) & 63]);
            }
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
