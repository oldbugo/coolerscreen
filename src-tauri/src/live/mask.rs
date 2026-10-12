//! The screen's notch mask, as in 2.0 (settings.json "mask": a black band on one edge, or a feathered circle, plus optional
//! rounded corners). The 2.0 values are in the 2400x1080 design space; the live canvas is 1680x756, so they are scaled by 0.7.
//! Applied as the last render step. The band width is rounded the same way 2.0 does it (core::mask::band_px).
use super::{LH, LW};
use serde_json::Value;

const K: f32 = 0.7;

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Mask {
    /// (edge, width in canvas px)
    band: Option<(String, usize)>,
    /// centre x, centre y, radius, feather (canvas px)
    circle: Option<(f32, f32, f32, f32)>,
    /// corner radius (canvas px)
    corners: Option<f32>,
}

impl Mask {
    pub fn none() -> Mask {
        Mask::default()
    }

    /// From the settings.json "mask" object (same shape the 2.0 app writes).
    pub fn from_value(mask: &Value) -> Mask {
        let mut m = Mask::default();
        let num = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        match mask.get("mode").and_then(Value::as_str) {
            Some("band") => {
                let band = &mask["band"];
                let px = crate::core::mask::band_px(band) as f32;
                if px > 0.0 {
                    m.band = Some((band["edge"].as_str().unwrap_or("left").to_string(), (px * K).round() as usize));
                }
            }
            Some("circle") => {
                let c = &mask["circle"];
                if num(c, "r") > 0.0 {
                    m.circle = Some((num(c, "cx") * K, num(c, "cy") * K, num(c, "r") * K, num(c, "feather") * K));
                }
            }
            _ => {}
        }
        let c = &mask["corners"];
        if c.get("on").and_then(Value::as_bool) == Some(true) && num(c, "r") > 0.0 {
            m.corners = Some(num(c, "r") * K);
        }
        m
    }

    /// The mask the 2.0 app has saved (settings.json in the app folder); none when it cannot be read.
    pub fn from_app_settings() -> Mask {
        let s = crate::core::settings::SettingsStore::new(crate::core::paths::settings()).load();
        Mask::from_value(s.mask())
    }

    /// Canvas px the left edge is covered by a band (content is kept clear of it).
    pub fn left_inset(&self) -> f32 {
        match &self.band {
            Some((e, px)) if e == "left" => *px as f32,
            _ => 0.0,
        }
    }

    pub fn is_none(&self) -> bool {
        self.band.is_none() && self.circle.is_none() && self.corners.is_none()
    }

    /// Black out the masked areas of an LW x LH RGBA frame.
    pub fn apply(&self, d: &mut [u8]) {
        if let Some((edge, px)) = &self.band {
            let px = (*px).min(LW);
            let black = |row: &mut [u8]| row.chunks_exact_mut(4).for_each(|p| p[..3].fill(0));
            for y in 0..LH {
                let row = &mut d[y * LW * 4..(y + 1) * LW * 4];
                match edge.as_str() {
                    "right" => black(&mut row[(LW - px) * 4..]),
                    "top" if y < px => black(row),
                    "bottom" if y >= LH.saturating_sub(px) => black(row),
                    "top" | "bottom" => {}
                    _ => black(&mut row[..px * 4]),
                }
            }
        }
        if let Some((cx, cy, r, f)) = self.circle {
            let reach = r + f + 2.0;
            let (x0, x1) = (((cx - reach).floor().max(0.0)) as usize, ((cx + reach).ceil() as usize).min(LW));
            let (y0, y1) = (((cy - reach).floor().max(0.0)) as usize, ((cy + reach).ceil() as usize).min(LH));
            for y in y0..y1 {
                for x in x0..x1 {
                    let dist = (((x as f32 + 0.5 - cx).powi(2)) + ((y as f32 + 0.5 - cy).powi(2))).sqrt();
                    let keep = if f > 0.0 { ((dist - r) / f).clamp(0.0, 1.0) } else if dist >= r { 1.0 } else { 0.0 };
                    if keep < 1.0 {
                        let i = (y * LW + x) * 4;
                        for k in 0..3 {
                            d[i + k] = (d[i + k] as f32 * keep) as u8;
                        }
                    }
                }
            }
        }
        if let Some(r) = self.corners {
            let ri = r.ceil() as usize;
            for (ox, oy, sx, sy) in [(0usize, 0usize, 1.0f32, 1.0f32), (LW - ri, 0, -1.0, 1.0), (0, LH - ri, 1.0, -1.0), (LW - ri, LH - ri, -1.0, -1.0)] {
                for y in 0..ri {
                    for x in 0..ri {
                        // distance from the arc centre of this corner, in the local square
                        let (lx, ly) = (if sx > 0.0 { x as f32 + 0.5 } else { ri as f32 - x as f32 - 0.5 }, if sy > 0.0 { y as f32 + 0.5 } else { ri as f32 - y as f32 - 0.5 });
                        let dist = ((lx - r).powi(2) + (ly - r).powi(2)).sqrt();
                        if lx < r && ly < r && dist > r - 0.5 {
                            let keep = (r + 0.5 - dist).clamp(0.0, 1.0);
                            let i = ((oy + y) * LW + ox + x) * 4;
                            for k in 0..3 {
                                d[i + k] = (d[i + k] as f32 * keep) as u8;
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn left_band_scales_from_the_2_0_setting() {
        // 92 px at 2400 wide is rounded to 96 by 2.0, 96 * 0.7 = 67
        let m = Mask::from_value(&json!({ "mode": "band", "band": { "edge": "left", "px": 92 }, "corners": { "on": false, "r": 80 } }));
        assert_eq!(m.left_inset(), 67.0);
        let mut f = vec![255u8; LW * LH * 4];
        m.apply(&mut f);
        assert_eq!(f[0], 0);
        assert_eq!(f[66 * 4], 0);
        assert_eq!(f[67 * 4], 255);
        assert_eq!(f[((LH - 1) * LW + 10) * 4], 0);
    }

    #[test]
    fn off_and_corners() {
        assert!(Mask::from_value(&json!({ "mode": "off" })).is_none());
        let m = Mask::from_value(&json!({ "mode": "off", "corners": { "on": true, "r": 80 } }));
        let mut f = vec![255u8; LW * LH * 4];
        m.apply(&mut f);
        assert_eq!(f[0], 0); // outer corner pixel black
        assert_eq!(f[(LW * 100 + 100) * 4], 255); // inside untouched
    }
}
