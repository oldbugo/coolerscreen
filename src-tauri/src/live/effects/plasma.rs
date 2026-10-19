//! Plasma: layers of warped colour bands sliding sideways at different speeds, mixed through a cyclic palette. Each layer is a
//! wave across the picture whose crests are bent by a fixed shape down the picture, so patterns slide left and right and
//! slowly blend into each other but never move vertically. Computed on a coarse grid and scaled up (it is all smooth).
use super::util::*;
use super::{Effect, Params};

struct Layer {
    k: f32,
    m: f64,
    /// Warp: shape down the picture (turns across the height), strength, offset.
    j: f32,
    b: f32,
    c: f32,
    amp: f32,
}

pub struct Plasma {
    p: Params,
    pal: Palette,
    layers: Vec<Layer>,
    grid: Vec<f32>,
    gw: usize,
    gh: usize,
    up: Upscaler,
    /// Per grid row warp shapes, per layer (they do not depend on time).
    warp: Vec<Vec<f32>>,
    key: (usize, usize),
}

impl Plasma {
    pub fn new(p: Params) -> Plasma {
        let pal = Palette::new(&p.colors, true);
        let s = 1.0 + 3.0 * p.scale; // feature size: bigger scale = fewer, wider bands
        let k = |a: f32| (a * (4.0 - s) * 0.9).round().max(1.0);
        let layers = vec![
            Layer { k: k(3.0), m: 1.0, j: 1.0, b: 0.55, c: 0.0, amp: 1.0 },
            Layer { k: k(5.0), m: -1.0, j: 2.0, b: 0.4, c: 0.3, amp: 0.8 },
            Layer { k: k(2.0), m: 2.0, j: 1.0, b: 0.8, c: 0.6, amp: 0.7 },
            Layer { k: k(7.0), m: -2.0, j: 3.0, b: 0.25, c: 0.1, amp: 0.4 },
        ];
        Plasma { p, pal, layers, grid: Vec::new(), gw: 0, gh: 0, up: Upscaler::new(), warp: Vec::new(), key: (0, 0) }
    }
}

impl Effect for Plasma {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let (w, h) = sp.key();
        if self.key != (w, h) || self.grid.is_empty() {
            self.key = (w, h);
            self.gw = (w / 5).max(8);
            self.gh = (h / 5).max(8);
            self.grid = vec![0.0; self.gw * self.gh * 3];
            self.warp = self
                .layers
                .iter()
                .map(|l| (0..self.gh).map(|y| l.b * sin_tau(l.j * (y as f32 + 0.5) / self.gh as f32 + l.c)).collect())
                .collect();
        }
        let (gw, gh) = (self.gw, self.gh);
        let total: f32 = self.layers.iter().map(|l| l.amp).sum();
        let tm: Vec<f32> = self.layers.iter().map(|l| turns(l.m, phi)).collect();
        // the mix weights breathe once per loop (a fade between layers, not a move)
        let breathe = [0.5 + 0.5 * sin_tau(turns(1.0, phi)), 0.5 + 0.5 * sin_tau(turns(1.0, phi) + 0.5)];
        let bands = 0.6 + 0.9 * self.p.density;
        let gain = self.p.brightness;
        // columns of the grid are the picture columns scaled the same way as the final upscale (u = (g + 0.5) / gw)
        for gx in 0..gw {
            let u = (gx as f32 + 0.5) / gw as f32;
            for gy in 0..gh {
                let mut s = 0.0;
                for (i, l) in self.layers.iter().enumerate() {
                    let wgt = match i {
                        2 => 0.35 + 0.65 * breathe[0],
                        3 => 0.35 + 0.65 * breathe[1],
                        _ => 1.0,
                    };
                    s += l.amp * wgt * sin_tau(l.k * u - tm[i] + self.warp[i][gy]);
                }
                let pos = 0.5 + 0.5 * s / total * 1.1;
                let c = self.pal.at(pos * bands);
                let o = (gy * gw + gx) * 3;
                self.grid[o] = c[0] * gain;
                self.grid[o + 1] = c[1] * gain;
                self.grid[o + 2] = c[2] * gain;
            }
        }
        self.up.draw(sp, &self.grid, gw, gh, data);
    }
}
