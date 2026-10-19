//! Pulse: the one sound-reactive effect. Columns of coloured light across a dark sky glow with the music (the 64 loudness bands of what
//! the PC is playing, the same capture the visualizer widget uses; only listened to while this effect is the live background).
//! Loudness changes brightness, never position, so there is no vertical motion; the colours slide slowly sideways, and a quiet
//! shimmer keeps it alive in silence. When no audio is given (editor preview, and the recording that becomes the Embedded loop) a
//! synthetic, exactly periodic "music" is used, so the stored loop is a demo of the effect, not a recording of sound.
use super::aurora::mix_row;
use super::util::*;
use super::{Effect, Params};

const BANDS: usize = 64;

pub struct Pulse {
    p: Params,
    pal: Palette,
    /// Smoothed band levels (live), and the latest raw ones handed in for the next frame.
    sm: Vec<f32>,
    fresh: Option<Vec<f32>>,
    colb: Vec<Vec<u8>>,
    prof8: Vec<Vec<u8>>,
    sky: Vec<u8>,
    sky_k: Vec<u8>,
    noise: Vec<u16>,
    key: (usize, usize, usize, usize),
}

impl Pulse {
    pub fn new(p: Params) -> Pulse {
        Pulse { pal: Palette::new(&p.colors, true), p, sm: vec![0.0; BANDS], fresh: None, colb: Vec::new(), prof8: Vec::new(), sky: Vec::new(), sky_k: Vec::new(), noise: Vec::new(), key: (0, 0, 0, 0) }
    }

    /// Exactly periodic stand-in for music: every band swells and falls a whole number of times per loop.
    fn synthetic(&self, phi: f64) -> Vec<f32> {
        (0..BANDS as u32)
            .map(|b| {
                let m = 1.0 + (hash(b, 51, self.p.seed) * 3.0).floor() as f64;
                let swell = 0.5 + 0.5 * sin_tau(turns(m, phi) + hash(b, 52, self.p.seed));
                let travel = 0.5 + 0.5 * sin_tau(b as f32 / BANDS as f32 * 2.0 - turns(1.0, phi));
                (0.15 + 0.85 * swell * (0.35 + 0.65 * travel)) * (1.0 - 0.45 * b as f32 / BANDS as f32)
            })
            .collect()
    }
}

impl Effect for Pulse {
    fn wants_audio(&self) -> bool {
        true
    }
    fn audio(&mut self, levels: &[f32]) {
        self.fresh = Some(levels.to_vec());
    }

    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]) {
        let n = sp.x1 - sp.x0;
        let key = (sp.x0, sp.x1, sp.y0, sp.y1);
        const LAYERS: usize = 3;
        if self.key != key || self.prof8.len() != LAYERS || self.sky_k.len() != sp.y1 - sp.y0 {
            self.key = key;
            let wscale = 0.6 + 0.9 * self.p.scale;
            // (centre, half width) of the glow bands: sharp columns, a wide halo, a shimmer high up
            let shape = [(0.5f32, 0.16f32), (0.5, 0.40), (0.34, 0.26)];
            self.prof8 = shape.iter().map(|&(cy, w)| (sp.y0..sp.y1).map(|y| (((-((sp.v(y) - cy) / (w * wscale)).powi(2)).exp()) * 255.0 + 0.5) as u8).collect()).collect();
            let c0 = self.pal.at(0.0);
            self.sky = (0..n).flat_map(|_| [(c0[0] * 0.3).max(10.0) as u8, (c0[1] * 0.3).max(10.0) as u8, (c0[2] * 0.3).max(24.0) as u8, 0]).collect();
            self.sky_k = (sp.y0..sp.y1).map(|y| ((0.04 + 0.12 * (1.0 - (sp.v(y) - 0.5).abs() * 2.0).max(0.0)) * 255.0) as u8).collect();
            let dz = dither();
            self.noise = Vec::with_capacity(64 * n * 4);
            for r in 0..64usize {
                for x in sp.x0..sp.x1 {
                    let z = ((dz[(r << 6) | (x & 63)] + 0.5) * 3.999) as u16;
                    self.noise.extend_from_slice(&[z, z, z, 1020]);
                }
            }
        }
        if self.colb.len() != LAYERS || self.colb[0].len() != n * 4 {
            self.colb = (0..LAYERS).map(|_| vec![0u8; n * 4]).collect();
        }
        // levels: live audio (attack fast, fall slowly) or the periodic stand-in
        let raw = match self.fresh.take() {
            Some(l) if l.len() == BANDS => {
                for (s, r) in self.sm.iter_mut().zip(&l) {
                    let r = ((r - 0.04) * 1.15).clamp(0.0, 1.0);
                    *s = if r > *s { *s + (r - *s) * 0.6 } else { *s * 0.9 };
                }
                self.sm.clone()
            }
            _ => self.synthetic(phi),
        };
        // a wide, smooth copy of the levels for the halo
        let halo: Vec<f32> = (0..BANDS)
            .map(|b| {
                let (mut s, mut c) = (0.0, 0.0);
                for d in -5i32..=5 {
                    let i = b as i32 + d;
                    if (0..BANDS as i32).contains(&i) {
                        let wgt = 1.0 - d.abs() as f32 / 6.0;
                        s += raw[i as usize] * wgt;
                        c += wgt;
                    }
                }
                s / c
            })
            .collect();
        let avg = raw.iter().sum::<f32>() / BANDS as f32;
        let gain = self.p.brightness * 1.3;
        let halo_gain = 0.4 + 0.8 * self.p.density;
        let (tc, ts) = (turns(1.0, phi), turns(2.0, phi));
        for i in 0..n {
            let u = sp.u(sp.x0 + i);
            let bx = (u * (BANDS as f32 - 1.0)).clamp(0.0, BANDS as f32 - 1.0);
            let (b0, f) = (bx as usize, bx - bx.floor());
            let b1 = (b0 + 1).min(BANDS - 1);
            let lv = raw[b0] + (raw[b1] - raw[b0]) * f;
            let hv = halo[b0] + (halo[b1] - halo[b0]) * f;
            let col = self.pal.at(u * 0.9 - tc);
            // fine columns: the level is cut into thin bars so it reads as an equalizer, not a smear
            let bar = 0.55 + 0.45 * smoothstep(0.1, 0.9, (sin_tau(u * BANDS as f32 * 0.5)).abs());
            let shimmer = (0.5 + 0.5 * sin_tau(u * 9.0 - ts)) * (0.15 + 0.35 * avg);
            let amp = [lv * bar * gain * 1.6, hv * halo_gain * gain, shimmer * gain];
            for (l, a) in amp.iter().enumerate() {
                let px = &mut self.colb[l][i * 4..i * 4 + 4];
                px.copy_from_slice(&[(col[0] * a).min(255.0) as u8, (col[1] * a).min(255.0) as u8, (col[2] * a).min(255.0) as u8, 0]);
            }
        }
        let (c0, c1, c2, c3) = (&self.colb[0], &self.colb[1], &self.colb[2], &self.sky);
        for y in sp.y0..sp.y1 {
            let j = y - sp.y0;
            let (p0, p1, p2, p3) = (self.prof8[0][j] as u16, self.prof8[1][j] as u16, self.prof8[2][j] as u16, self.sky_k[j] as u16);
            let out = &mut data[(y * sp.cw + sp.x0) * 4..(y * sp.cw + sp.x1) * 4];
            let nz = &self.noise[(y & 63) * n * 4..][..n * 4];
            mix_row(out, c0, c1, c2, c3, nz, [p0, p1, p2, p3]);
        }
    }
}
