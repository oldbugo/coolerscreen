//! Lava: glowing metaballs that sway sideways and merge and part like a lava lamp. Each blob keeps its height and sloshes left and
//! right on a sine of the loop phase (whole turns per loop), so the picture repeats exactly and nothing moves up or down. The
//! field is summed on a coarse grid and run through a colour ramp (dark background, rim, body, hot core).
use super::util::*;
use super::{Effect, Params};

struct Blob {
    x0: f32,
    ax: f32,
    y: f32,
    r: f32,
    m: f64,
    ph: f32,
}

pub struct Lava {
    p: Params,
    pal: Palette,
    blobs: Vec<Blob>,
    grid: Vec<f32>,
    gw: usize,
    gh: usize,
    up: Upscaler,
    key: (usize, usize),
}

impl Lava {
    pub fn new(p: Params) -> Lava {
        let n = (5.0 + p.density * 9.0) as u32;
        let size = 0.6 + 0.9 * p.scale;
        let blobs = (0..n)
            .map(|i| Blob {
                x0: 0.1 + 0.8 * hash(i, 31, p.seed),
                ax: 0.08 + 0.22 * hash(i, 32, p.seed),
                y: 0.12 + 0.76 * hash(i, 33, p.seed),
                r: (0.07 + 0.12 * hash(i, 34, p.seed)) * size,
                m: if hash(i, 35, p.seed) < 0.5 { 1.0 } else { 2.0 } * if hash(i, 36, p.seed) < 0.5 { 1.0 } else { -1.0 },
                ph: hash(i, 37, p.seed),
            })
            .collect();
        Lava { pal: Palette::new(&p.colors, false), p, blobs, grid: Vec::new(), gw: 0, gh: 0, up: Upscaler::new(), key: (0, 0) }
    }
}

impl Effect for Lava {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let (w, h) = sp.key();
        if self.key != (w, h) || self.grid.is_empty() {
            self.key = (w, h);
            self.gw = (w / 5).max(16);
            self.gh = (h / 5).max(8);
            self.grid = vec![0.0; self.gw * self.gh * 4];
        }
        let (gw, gh) = (self.gw, self.gh);
        let aspect = w as f32 / h as f32;
        let gain = self.p.brightness;
        // blob centres for this phase (x in units of the picture height, so blobs are round)
        let pos: Vec<(f32, f32, f32)> = self.blobs.iter().map(|b| ((b.x0 + b.ax * sin_tau(turns(b.m, phi) + b.ph)) * aspect, b.y, b.r * b.r)).collect();
        let reach: Vec<f32> = self.blobs.iter().map(|b| (b.r * 7.0).powi(2)).collect();
        for gy in 0..gh {
            let y = (gy as f32 + 0.5) / gh as f32;
            for gx in 0..gw {
                let x = (gx as f32 + 0.5) / gw as f32 * aspect;
                let mut f = 0.0;
                for (i, &(bx, by, r2)) in pos.iter().enumerate() {
                    let (dx, dy) = (x - bx, y - by);
                    let d2 = dx * dx + dy * dy;
                    if d2 < reach[i] {
                        f += r2 / (d2 + 1e-4);
                    }
                }
                let t = smoothstep(0.25, 2.4, f);
                let bgk = 0.75 + 0.5 * y;
                let c = self.pal.at(t);
                let o = (gy * gw + gx) * 4;
                let k = if t < 0.02 { bgk } else { 1.0 };
                self.grid[o..o + 4].copy_from_slice(&[c[0] * gain * k, c[1] * gain * k, c[2] * gain * k, 255.0]);
            }
        }
        self.up.draw(sp, &self.grid, gw, gh, data);
    }
}
