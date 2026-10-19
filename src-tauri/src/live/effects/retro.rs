//! Retro: a synthwave horizon. A striped sun and mountains sit still under a gradient sky with twinkling stars; the neon grid on the
//! ground has fixed horizontal lines and vertical lines that slide sideways in perspective (each line is a straight line to the
//! vanishing point, so every point moves left or right only). One line spacing per loop, so the loop is exact. The still scene
//! is drawn once per picture size; a frame is a copy plus the sliding lines.
use super::util::*;
use super::{Effect, Params};

pub struct Retro {
    p: Params,
    scene: Vec<u8>,
    key: (usize, usize, usize, usize),
    stars: Vec<(f32, f32, f32, f64, f32)>,
}

impl Retro {
    pub fn new(p: Params) -> Retro {
        let stars = (0..90u32).map(|i| (hash(i, 41, p.seed), hash(i, 42, p.seed) * 0.42, 0.3 + 0.7 * hash(i, 43, p.seed), 1.0 + (hash(i, 44, p.seed) * 3.0).floor() as f64, hash(i, 45, p.seed))).collect();
        Retro { p, scene: Vec::new(), key: (0, 0, 0, 0), stars }
    }
}

const HORIZON: f32 = 0.58;

fn mountain(u: f32, seed: u32) -> f32 {
    // a jagged skyline from a few sines (repeats across the picture, 0..1)
    let a = 0.5 + 0.5 * sin_tau(u * 3.0 + hash(seed, 1, 5));
    let b = 0.5 + 0.5 * sin_tau(u * 7.0 + hash(seed, 2, 5));
    let c = 0.5 + 0.5 * sin_tau(u * 17.0 + hash(seed, 3, 5));
    (0.5 * a + 0.3 * b + 0.2 * c).powf(1.4)
}

impl Effect for Retro {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let (w, h) = (sp.x1 - sp.x0, sp.y1 - sp.y0);
        let key = (sp.x0, sp.x1, sp.y0, sp.y1);
        let pal = |i: usize| {
            let c = self.p.colors[i.min(self.p.colors.len() - 1)];
            [c[0] as f32, c[1] as f32, c[2] as f32]
        };
        let (top, hor, sun, grid) = (pal(0), pal(1), pal(2), pal(3));
        if self.key != key || self.scene.len() != w * h * 4 {
            self.key = key;
            self.scene = vec![0; w * h * 4];
            let dz = dither();
            let (cx, cy) = (0.5f32, HORIZON * 0.64);
            let rad = 0.16 + 0.18 * self.p.scale; // sun radius as a fraction of the picture height
            let aspect = sp.rw / sp.rh;
            let ground_h = 1.0 - HORIZON;
            // horizontal ground lines: fixed rows, packed towards the horizon
            let lines: Vec<f32> = (1..=14).map(|k| HORIZON + ground_h * (k as f32 / 14.0).powf(2.2)).collect();
            for y in 0..h {
                let v = sp.v(sp.y0 + y);
                for x in 0..w {
                    let u = sp.u(sp.x0 + x);
                    let o = (y * w + x) * 4;
                    let mut c;
                    if v < HORIZON {
                        let t = smoothstep(0.0, HORIZON, v);
                        c = [top[0] + (hor[0] - top[0]) * t, top[1] + (hor[1] - top[1]) * t, top[2] + (hor[2] - top[2]) * t];
                        // sun: disc with gaps cut in its lower half, and a glow around it
                        let (dx, dy) = ((u - cx) * aspect, v - cy);
                        let d = (dx * dx + dy * dy).sqrt() / rad;
                        let glow = (-(d * d) * 0.55).exp() * 0.45;
                        for k in 0..3 {
                            c[k] += (hor[k] * 1.2 + 30.0 - c[k]) * glow;
                        }
                        if d < 1.0 {
                            let s = dy / rad; // -1 top .. 1 bottom
                            let gap = s > 0.0 && (s * 7.0).fract() < 0.12 + 0.5 * s;
                            if !gap {
                                let e = smoothstep(1.0, 0.96, d);
                                let g = (s * 0.5 + 0.5).clamp(0.0, 1.0);
                                for k in 0..3 {
                                    let body = sun[k] + (hor[k] - sun[k]) * g;
                                    c[k] += (body - c[k]) * e;
                                }
                            }
                        }
                        // mountains in front of the sun, dark against the glow
                        let m = HORIZON - 0.17 * mountain(u, self.p.seed);
                        if v > m {
                            for k in 0..3 {
                                c[k] = top[k] * 0.35 + hor[k] * 0.06;
                            }
                        }
                    } else {
                        let z = (v - HORIZON) / ground_h;
                        c = [top[0] * 0.28 + hor[0] * 0.04, top[1] * 0.28 + hor[1] * 0.04, top[2] * 0.28 + hor[2] * 0.04];
                        let band = (1.0 - z) * 0.18;
                        for k in 0..3 {
                            c[k] += hor[k] * band * 0.35;
                        }
                        for &ly in &lines {
                            let dd = (v - ly).abs() * sp.rh;
                            let lw = (0.8 + 1.8 * z) * sp.k().max(0.6);
                            if dd < lw {
                                let m = (1.0 - dd / lw) * (0.35 + 0.65 * z);
                                for k in 0..3 {
                                    c[k] += grid[k] * m;
                                }
                            }
                        }
                    }
                    put(&mut self.scene[o..o + 4], c[0], c[1], c[2], dz[((sp.y0 + y) & 63) << 6 | ((sp.x0 + x) & 63)]);
                }
            }
        }
        for y in 0..h {
            let o = ((sp.y0 + y) * sp.cw + sp.x0) * 4;
            data[o..o + w * 4].copy_from_slice(&self.scene[y * w * 4..(y + 1) * w * 4]);
        }
        // sliding vertical lines on the ground
        let n_lines = 6.0 + (self.p.density * 14.0).round();
        let spacing = sp.rw / n_lines; // at the bottom of the picture
        let shift = turns(1.0, phi);
        let gain = self.p.brightness * (1.0 + 0.2 * sin_tau(turns(1.0, phi)));
        let cxp = sp.rx + sp.rw * 0.5;
        let y_start = (sp.ry + sp.rh * HORIZON).ceil() as usize;
        for y in y_start.max(sp.y0)..sp.y1 {
            let z = (sp.v(y) - HORIZON) / (1.0 - HORIZON);
            if z <= 0.01 {
                continue;
            }
            let s = spacing * z;
            let lw = (1.0 + 2.4 * z) * sp.k().max(0.6);
            let fade = z.powf(0.6) * gain * smoothstep(3.0, 12.0, s); // far lines are too fine to draw: they fade out instead of shimmering
            let row = &mut data[(y * sp.cw) * 4..(y * sp.cw + sp.cw) * 4];
            let inv = 1.0 / s;
            for x in sp.x0..sp.x1 {
                let t = (x as f32 - cxp) * inv - shift;
                let nearest = (t + 1024.5) as i32 as f32 - 1024.0;
                let d = (t - nearest).abs() * s;
                if d < lw {
                    let m = (1.0 - d / lw) * fade;
                    let px = &mut row[x * 4..x * 4 + 3];
                    px[0] = (px[0] as f32 + grid[0] * m).min(255.0) as u8;
                    px[1] = (px[1] as f32 + grid[1] * m).min(255.0) as u8;
                    px[2] = (px[2] as f32 + grid[2] * m).min(255.0) as u8;
                }
            }
        }
        // stars in the sky
        for &(su, sv, sb, sm, sph) in &self.stars {
            let tw = 0.5 + 0.5 * sin_tau(turns(sm, phi) + sph);
            let a = sb * (0.15 + 0.85 * tw * tw) * self.p.brightness.min(1.0);
            let (x, y) = ((sp.rx + su * sp.rw) as i64, (sp.ry + sv * sp.rh) as i64);
            if x < sp.x0 as i64 || x >= sp.x1 as i64 || y < sp.y0 as i64 || y >= sp.y1 as i64 {
                continue;
            }
            let o = (y as usize * sp.cw + x as usize) * 4;
            for c in 0..3 {
                data[o + c] = (data[o + c] as f32 + 230.0 * a).min(255.0) as u8;
            }
        }
    }
}
