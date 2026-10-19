//! Drift: soft glowing particles (dust, embers, bokeh) that appear, drift sideways while they fade up and down, and vanish.
//! Every particle lives a whole number of times per loop and its place depends only on which life it is in, so the picture
//! after one loop is exactly the picture before (a seamless recording). Near particles are big and quick, far ones small and
//! slow; a few very large dim ones give the depth. Motion is horizontal only; brightness changes are fades.
use super::util::*;
use super::{Effect, Params};
use std::collections::HashMap;

struct Particle {
    /// Lives per loop (1..3), the point in the loop its first life starts, depth 0 (far) .. 1 (near), colour index.
    lives: u32,
    start: f32,
    depth: f32,
    col: usize,
    /// Radius in design pixels (1680 wide), how far it drifts in one life, brightness.
    radius: f32,
    drift: f32,
    bright: f32,
    id: u32,
}

pub struct Drift {
    p: Params,
    parts: Vec<Particle>,
    pal: Vec<[f32; 3]>,
    bg: Vec<u8>,
    key: (usize, usize, usize, usize),
    sprites: HashMap<u32, (usize, Vec<f32>)>,
}

impl Drift {
    pub fn new(p: Params) -> Drift {
        let n = (30.0 + p.density * 330.0) as u32;
        let size = 0.55 + 0.9 * p.scale;
        let mut parts: Vec<Particle> = (0..n)
            .map(|i| {
                let depth = hash(i, 10, p.seed).powf(1.3);
                let lives = if depth < 0.34 { 1 } else if depth < 0.67 { 2 } else { 3 };
                let big = hash(i, 11, p.seed) < 0.06; // a few large dim bokeh discs
                Particle {
                    lives,
                    start: hash(i, 12, p.seed),
                    depth,
                    col: (hash(i, 13, p.seed) * 64.0) as usize,
                    radius: if big { (26.0 + 40.0 * hash(i, 14, p.seed)) * size } else { (2.0 + 9.0 * depth) * size },
                    drift: if big { 70.0 } else { 50.0 + 210.0 * depth },
                    bright: if big { 0.16 } else { 0.4 + 0.6 * depth },
                    id: i,
                }
            })
            .collect();
        // far ones first so near ones draw over them
        parts.sort_by(|a, b| a.depth.partial_cmp(&b.depth).unwrap());
        let pal = p.colors.iter().map(|c| [c[0] as f32, c[1] as f32, c[2] as f32]).collect();
        Drift { p, parts, pal, bg: Vec::new(), key: (0, 0, 0, 0), sprites: HashMap::new() }
    }

    fn sprite(&mut self, r: u32) -> &(usize, Vec<f32>) {
        self.sprites.entry(r).or_insert_with(|| {
            let half = (r as f32 * 1.4).ceil() as i32;
            let side = (2 * half + 1) as usize;
            let mut v = vec![0.0; side * side];
            for y in -half..=half {
                for x in -half..=half {
                    let d = ((x * x + y * y) as f32).sqrt() / r as f32;
                    v[(y + half) as usize * side + (x + half) as usize] = if d > 1.4 { 0.0 } else { (-3.2 * d * d).exp() };
                }
            }
            (half as usize, v)
        })
    }
}

impl Effect for Drift {
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
                let k = 0.05 + 0.13 * v;
                let row = &mut self.bg[y * w * 4..(y + 1) * w * 4];
                for (x, px) in row.chunks_exact_mut(4).enumerate() {
                    put(px, c0[0] * k + 2.0, c0[1] * k + 3.0, c0[2] * k + 6.0, dz[((sp.y0 + y) & 63) << 6 | ((sp.x0 + x) & 63)]);
                }
            }
        }
        for y in 0..h {
            let o = ((sp.y0 + y) * sp.cw + sp.x0) * 4;
            data[o..o + w * 4].copy_from_slice(&self.bg[y * w * 4..(y + 1) * w * 4]);
        }
        let (kx, ky) = (sp.k(), sp.rh / 756.0);
        let gain = self.p.brightness * 255.0;
        let n = self.parts.len();
        for pi in 0..n {
            let (lives, start, radius, drift, bright, col, id) = {
                let q = &self.parts[pi];
                (q.lives, q.start, q.radius, q.drift, q.bright, q.col, q.id)
            };
            let t = phi * lives as f64 + start as f64;
            let life = t.floor() as i64;
            let u = (t - t.floor()) as f32;
            // the place depends on the life number modulo the lives per loop, which is what repeats every loop
            let slot = life.rem_euclid(lives as i64) as u32;
            let x = hash(id, 100 + slot, self.p.seed) * (1680.0 + drift) - drift * 0.5 + drift * (u - 0.5);
            let y = hash(id, 200 + slot, self.p.seed) * 756.0;
            let s = sin_tau(0.5 * u);
            let tw = 0.8 + 0.2 * sin_tau(turns(3.0, phi) + start * 7.0);
            let a = s * s * tw * bright;
            if a < 0.01 {
                continue;
            }
            let r = (radius * kx).clamp(1.0, 90.0);
            let ri = r.round() as u32;
            let (cx, cy) = (sp.rx + x * kx, sp.ry + y * ky);
            let c = self.pal[col % self.pal.len()];
            let (half, spr) = {
                let e = self.sprite(ri);
                (e.0 as i64, &e.1)
            };
            let side = (2 * half + 1) as usize;
            let (px0, py0) = (cx as i64 - half, cy as i64 - half);
            let k = a * gain / 255.0;
            for sy in 0..side as i64 {
                let py = py0 + sy;
                if py < sp.y0 as i64 || py >= sp.y1 as i64 {
                    continue;
                }
                let xs = (sp.x0 as i64 - px0).max(0);
                let xe = (sp.x1 as i64 - px0).min(side as i64);
                if xe <= xs {
                    continue;
                }
                let srow = &spr[sy as usize * side..][..side];
                for sx in xs..xe {
                    let m = srow[sx as usize] * k;
                    if m < 0.004 {
                        continue;
                    }
                    let o = (py as usize * sp.cw + (px0 + sx) as usize) * 4;
                    let d = &mut data[o..o + 3];
                    d[0] = (d[0] as f32 + c[0] * m).min(255.0) as u8;
                    d[1] = (d[1] as f32 + c[1] * m).min(255.0) as u8;
                    d[2] = (d[2] as f32 + c[2] * m).min(255.0) as u8;
                }
            }
        }
    }
}
