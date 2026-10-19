//! Warp: a starfield streaming sideways (right to left) with depth. Near stars are thick bright streaks that cross fast, far ones are
//! fine dots that crawl. Every star lives a whole number of times per loop and crosses the whole picture in each life, so the loop is
//! seamless; a star's height is fixed for its whole crossing (new life, new height) and it fades in and out at the picture edges.
use super::util::*;
use super::{Effect, Params};

struct Star {
    lives: u32,
    start: f32,
    depth: f32,
    id: u32,
    col: usize,
}

pub struct Warp {
    p: Params,
    stars: Vec<Star>,
    pal: Vec<[f32; 3]>,
    bg: Vec<u8>,
    key: (usize, usize, usize, usize),
}

impl Warp {
    pub fn new(p: Params) -> Warp {
        let n = (70.0 + p.density * 520.0) as u32;
        let mut stars: Vec<Star> = (0..n)
            .map(|i| {
                let depth = hash(i, 21, p.seed).powf(1.6);
                Star { lives: 1 + (depth * 3.99) as u32, start: hash(i, 22, p.seed), depth, id: i, col: (hash(i, 23, p.seed) * 64.0) as usize }
            })
            .collect();
        stars.sort_by(|a, b| a.depth.partial_cmp(&b.depth).unwrap());
        let pal = p.colors.iter().map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).collect();
        Warp { p, stars, pal, bg: Vec::new(), key: (0, 0, 0, 0) }
    }
}

impl Effect for Warp {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let (w, h) = (sp.x1 - sp.x0, sp.y1 - sp.y0);
        let key = (sp.x0, sp.x1, sp.y0, sp.y1);
        if self.key != key || self.bg.len() != w * h * 4 {
            self.key = key;
            self.bg = vec![0; w * h * 4];
            let c0 = self.pal[0];
            let dz = dither();
            for y in 0..h {
                let v = sp.v(sp.y0 + y).clamp(0.0, 1.0);
                // a faint glow along the middle where the stars rush by
                let g = (-((v - 0.5) / 0.28).powi(2)).exp();
                let k = 0.03 + 0.10 * g;
                for (x, px) in self.bg[y * w * 4..(y + 1) * w * 4].chunks_exact_mut(4).enumerate() {
                    put(px, c0[0] * k + 1.0, c0[1] * k + 2.0, c0[2] * k + 6.0, dz[((sp.y0 + y) & 63) << 6 | ((sp.x0 + x) & 63)]);
                }
            }
        }
        for y in 0..h {
            let o = ((sp.y0 + y) * sp.cw + sp.x0) * 4;
            data[o..o + w * 4].copy_from_slice(&self.bg[y * w * 4..(y + 1) * w * 4]);
        }
        let (kx, ky) = (sp.k(), sp.rh / 756.0);
        let margin = 160.0f32;
        let span = 1680.0 + 2.0 * margin;
        let long = 0.5 + 1.2 * self.p.scale;
        let gain = self.p.brightness;
        for s in &self.stars {
            let t = phi * s.lives as f64 + s.start as f64;
            let life = t.floor() as i64;
            let u = (t - t.floor()) as f32;
            let slot = life.rem_euclid(s.lives as i64) as u32;
            let xd = span * (1.0 - u) - margin; // head position, design px (moves left)
            let yd = hash(s.id, 300 + slot, self.p.seed) * 756.0;
            let d = s.depth;
            let edge = smoothstep(0.0, 0.06, u) * smoothstep(0.0, 0.06, 1.0 - u);
            let a = (0.25 + 0.75 * d) * edge * gain;
            if a < 0.01 {
                continue;
            }
            let len = (3.0 + 150.0 * d * d) * long * kx;
            let th = (0.6 + 2.6 * d) * kx.max(0.6);
            let (x0, y0) = (sp.rx + xd * kx, sp.ry + yd * ky);
            let c = self.pal[s.col % self.pal.len()];
            let rows = th.ceil() as i64 + 1;
            for dy in -rows..=rows {
                let wy = (1.0 - (dy as f32 / (th * 0.5 + 0.5)).abs()).max(0.0);
                if wy <= 0.0 {
                    continue;
                }
                let py = y0 as i64 + dy;
                if py < sp.y0 as i64 || py >= sp.y1 as i64 {
                    continue;
                }
                let xs = (x0 as i64).max(sp.x0 as i64);
                let xe = ((x0 + len) as i64 + 1).min(sp.x1 as i64);
                for px in xs..xe {
                    // brightest at the head (left end), fading along the trail
                    let f = 1.0 - ((px as f32 - x0) / len.max(1.0)).clamp(0.0, 1.0);
                    let m = a * wy * f * f;
                    if m < 0.004 {
                        continue;
                    }
                    let o = (py as usize * sp.cw + px as usize) * 4;
                    let dd = &mut data[o..o + 3];
                    dd[0] = (dd[0] as f32 + c[0] * m).min(255.0) as u8;
                    dd[1] = (dd[1] as f32 + c[1] * m).min(255.0) as u8;
                    dd[2] = (dd[2] as f32 + c[2] * m).min(255.0) as u8;
                }
            }
        }
    }
}
