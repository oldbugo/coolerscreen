//! Clouds: soft fractal-noise cloud banks (a nebula or smoke) in a four-colour ramp. Two noise layers slide sideways in opposite
//! directions and a third fades in and out in place, so the shapes keep changing but nothing moves up or down. The fields are
//! made once (they wrap sideways); a frame is only shifted lookups, a palette and the upscale.
use super::util::*;
use super::{Effect, Params};

pub struct Clouds {
    p: Params,
    pal: Palette,
    fields: [Vec<f32>; 3],
    grid: Vec<f32>,
    gw: usize,
    gh: usize,
    up: Upscaler,
    key: (usize, usize),
    ix: [Vec<(usize, usize, f32)>; 2],
}

/// Value-noise fbm on a `gw` x `gh` grid that wraps sideways: `cx` cells across at the first octave.
fn fbm_field(gw: usize, gh: usize, cx: usize, seed: u32) -> Vec<f32> {
    let cy = ((cx * gh) as f32 / gw as f32).ceil() as usize + 1;
    let mut out = vec![0.0f32; gw * gh];
    let mut norm = 0.0;
    for o in 0..5u32 {
        let fx = cx << o;
        let amp = 0.5f32.powi(o as i32);
        norm += amp;
        for gy in 0..gh {
            let y = gy as f32 / gh as f32 * (cy << o) as f32 ;
            let (iy, ty) = (y.floor() as u32, y - y.floor());
            let ty = ty * ty * (3.0 - 2.0 * ty);
            for gx in 0..gw {
                let x = gx as f32 / gw as f32 * fx as f32;
                let (ix, tx) = (x.floor() as u32, x - x.floor());
                let tx = tx * tx * (3.0 - 2.0 * tx);
                let (x0, x1) = (ix % fx as u32, (ix + 1) % fx as u32);
                let h = |xx: u32, yy: u32| hash(xx, yy, seed.wrapping_add(o * 977));
                let a = h(x0, iy) + (h(x1, iy) - h(x0, iy)) * tx;
                let b = h(x0, iy + 1) + (h(x1, iy + 1) - h(x0, iy + 1)) * tx;
                out[gy * gw + gx] += amp * (a + (b - a) * ty);
            }
        }
    }
    for v in out.iter_mut() {
        *v /= norm;
    }
    // fbm sits in a narrow band around 0.5: stretch it so thresholds behave
    for v in out.iter_mut() {
        *v = ((*v - 0.5) * 2.1 + 0.5).clamp(0.0, 1.0);
    }
    out
}

impl Clouds {
    pub fn new(p: Params) -> Clouds {
        let pal = Palette::new(&p.colors, false);
        Clouds { p, pal, fields: [Vec::new(), Vec::new(), Vec::new()], grid: Vec::new(), gw: 0, gh: 0, up: Upscaler::new(), key: (0, 0), ix: [Vec::new(), Vec::new()] }
    }
}

impl Effect for Clouds {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let (w, h) = sp.key();
        if self.key != (w, h) || self.grid.is_empty() {
            self.key = (w, h);
            self.gw = (w / 4).max(16);
            self.gh = (h / 4).max(8);
            let cx = (2.0 + 6.0 * (1.0 - self.p.scale)).round() as usize;
            let s = self.p.seed;
            self.fields = [fbm_field(self.gw, self.gh, cx, s), fbm_field(self.gw, self.gh, cx + 1, s ^ 0x5bd1), fbm_field(self.gw, self.gh, cx + 2, s ^ 0x1234)];
            self.grid = vec![0.0; self.gw * self.gh * 3];
        }
        let (gw, gh) = (self.gw, self.gh);
        // sideways lookups: layer 0 slides right one picture width per loop, layer 1 slides left
        for (l, dir) in [1.0f64, -1.0].into_iter().enumerate() {
            let shift = (dir * phi).rem_euclid(1.0) as f32 * gw as f32;
            self.ix[l] = (0..gw)
                .map(|gx| {
                    let s = (gx as f32 - shift).rem_euclid(gw as f32);
                    let i = s.floor() as usize % gw;
                    (i, (i + 1) % gw, s - s.floor())
                })
                .collect();
        }
        let fade = 0.5 + 0.5 * sin_tau(turns(1.0, phi));
        let thr = 0.62 - 0.40 * self.p.density;
        let gain = self.p.brightness;
        let [fa, fb, fc] = &self.fields;
        for gy in 0..gh {
            let row = gy * gw;
            for gx in 0..gw {
                let (a0, a1, af) = self.ix[0][gx];
                let (b0, b1, bf) = self.ix[1][gx];
                let a = fa[row + a0] + (fa[row + a1] - fa[row + a0]) * af;
                let b = fb[row + b0] + (fb[row + b1] - fb[row + b0]) * bf;
                let c = fc[row + gx];
                let v = 0.46 * a + 0.34 * b + 0.2 * (c * fade + (1.0 - c) * (1.0 - fade));
                let t = smoothstep(thr - 0.30, thr + 0.45, v);
                let col = self.pal.at(t);
                let o = (row + gx) * 3;
                self.grid[o] = col[0] * gain;
                self.grid[o + 1] = col[1] * gain;
                self.grid[o + 2] = col[2] * gain;
            }
        }
        self.up.draw(sp, &self.grid, gw, gh, data);
    }
}
