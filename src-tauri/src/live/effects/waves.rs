//! Waves: layered ribbons of sea or hills under a sky, each layer a pair of sine waves that slide sideways at its own speed (the far
//! ones slowly, the near ones faster, every other one the other way). Heights are a function of the sideways position, so every part
//! of the picture only moves left or right. Drawn on a coarse grid back to front and scaled up; the sky and the sun glow are made once.
use super::util::*;
use super::{Effect, Params};

const LAYERS: usize = 5;

pub struct Waves {
    p: Params,
    pal_sky: Palette,
    pal_land: Palette,
    sky: Vec<f32>,
    grid: Vec<f32>,
    gw: usize,
    gh: usize,
    up: Upscaler,
    key: (usize, usize),
}

impl Waves {
    pub fn new(p: Params) -> Waves {
        // colours: sky top, sky at the horizon, far water, near water
        let c = p.colors.clone();
        let at = |i: usize| c[i.min(c.len() - 1)];
        Waves { pal_sky: Palette::new(&[at(0), at(1)], false), pal_land: Palette::new(&[at(2), at(3)], false), p, sky: Vec::new(), grid: Vec::new(), gw: 0, gh: 0, up: Upscaler::new(), key: (0, 0) }
    }
}

impl Effect for Waves {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let (w, h) = sp.key();
        if self.key != (w, h) || self.grid.is_empty() {
            self.key = (w, h);
            self.gw = (w / 4).max(16);
            self.gh = (h / 4).max(8);
            self.grid = vec![0.0; self.gw * self.gh * 4];
            // sky gradient with a soft sun low on the horizon (static)
            let (gw, gh) = (self.gw, self.gh);
            self.sky = vec![0.0; gw * gh * 4];
            for gy in 0..gh {
                let v = (gy as f32 + 0.5) / gh as f32;
                for gx in 0..gw {
                    let u = (gx as f32 + 0.5) / gw as f32;
                    let t = smoothstep(0.0, 0.62, v);
                    let mut c = self.pal_sky.at(t);
                    let (dx, dy) = ((u - 0.7) * 2.2, v - 0.5);
                    let glow = (-(dx * dx + dy * dy * 3.0) * 9.0).exp() * 0.55;
                    let hot = self.pal_sky.at(1.0);
                    for k in 0..3 {
                        c[k] += (hot[k] * 1.1 + 40.0 - c[k]) * glow;
                    }
                    let o = (gy * gw + gx) * 4;
                    self.sky[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255.0]);
                }
            }
        }
        let (gw, gh) = (self.gw, self.gh);
        self.grid.copy_from_slice(&self.sky);
        let wl = 0.6 + 1.2 * self.p.scale; // wave length factor
        let amp = 0.03 + 0.07 * self.p.density; // wave height (fraction of the picture)
        let gain = self.p.brightness;
        let mut top = vec![0f32; gw];
        for l in 0..LAYERS {
            let f = l as f32 / (LAYERS - 1) as f32; // 0 far .. 1 near
            let base = 0.50 + 0.40 * f.powf(0.8);
            // sideways speed in loops: far slow, near fast; every other layer goes the other way
            let m1 = (1 + l as i32 / 2) as f64 * if l % 2 == 0 { 1.0 } else { -1.0 };
            let m2 = m1 * 2.0;
            let k1 = ((2.0 + 2.0 * f) / wl).round().max(1.0);
            let k2 = ((5.0 + 4.0 * f) / wl).round().max(2.0);
            let (t1, t2) = (turns(m1, phi), turns(m2, phi));
            let a = amp * (0.5 + f);
            for (gx, t) in top.iter_mut().enumerate() {
                let u = (gx as f32 + 0.5) / gw as f32;
                *t = base + a * (0.65 * sin_tau(k1 * u - t1 + l as f32 * 0.37) + 0.35 * sin_tau(k2 * u - t2 + l as f32 * 0.71));
            }
            // water colour: far = first colour, near = second; the far layers also melt into the sky haze
            let col = self.pal_land.at(f);
            let haze = (1.0 - f) * 0.55;
            for gy in 0..gh {
                let v = (gy as f32 + 0.5) / gh as f32;
                for gx in 0..gw {
                    let cov = ((v - top[gx]) * gh as f32 + 0.5).clamp(0.0, 1.0);
                    if cov <= 0.0 {
                        continue;
                    }
                    let depth = ((v - top[gx]) * 2.2).clamp(0.0, 1.0);
                    let shade = 1.0 - 0.55 * depth;
                    let o = (gy * gw + gx) * 4;
                    for k in 0..3 {
                        let s = self.sky[o + k];
                        let c = col[k] * shade + (s - col[k] * shade) * haze * (1.0 - depth);
                        let g = &mut self.grid[o + k];
                        *g += (c - *g) * cov;
                    }
                }
            }
        }
        for v in self.grid.chunks_exact_mut(4) {
            v[0] *= gain;
            v[1] *= gain;
            v[2] *= gain;
        }
        self.up.draw(sp, &self.grid, gw, gh, data);
    }
}
