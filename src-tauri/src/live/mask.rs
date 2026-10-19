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
        if c.get("on").and_then(Value::as_bool) != Some(false) && num(c, "r") > 0.0 {
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

    /// Black out the masked areas of an LW x LH RGBA frame (no screen fit).
    pub fn apply(&self, d: &mut [u8]) {
        self.apply_fit(d, &crate::core::fit::Fit::default());
    }

    /// As `apply`, with the screen fit: the rounded corners are drawn on the VISIBLE rectangle (the panel inset by the fit,
    /// not moved by the shift: the bezel is physical), and on the left after the notch band. The band itself stays measured
    /// from the panel's edge.
    pub fn apply_fit(&self, d: &mut [u8], fit: &crate::core::fit::Fit) {
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
            // Same geometry as the base's corner PNG (media::assets::corner_mask_file): black outside the visible rectangle
            // (the canvas minus a notch band) rounded with radius r (at most half its short side), anti-aliased.
            let (fx0, fy0, fx1, fy1) = fit.rect(LW, LH);
            let (mut x0, mut y0, mut x1, mut y1) = (fx0 as f32, fy0 as f32, fx1 as f32, fy1 as f32);
            if let Some((edge, px)) = &self.band {
                let px = *px as f32;
                match edge.as_str() {
                    "right" => x1 = x1.min(LW as f32 - px),
                    "top" => y0 = y0.max(px),
                    "bottom" => y1 = y1.min(LH as f32 - px),
                    _ => x0 = x0.max(px),
                }
            }
            let rr = r.round().min(((x1 - x0).min(y1 - y0)) / 2.0).max(0.0);
            let ri = rr.ceil() as usize + 1;
            let (cx0, cy0, cx1, cy1) = (x0 as usize, y0 as usize, x1 as usize, y1 as usize);
            for (ox, oy) in [(cx0, cy0), (cx1.saturating_sub(ri), cy0), (cx0, cy1.saturating_sub(ri)), (cx1.saturating_sub(ri), cy1.saturating_sub(ri))] {
                for y in oy..(oy + ri).min(LH) {
                    for x in ox..(ox + ri).min(LW) {
                        let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                        let dx = (x0 + rr - fx).max(fx - (x1 - rr)).max(0.0);
                        let dy = (y0 + rr - fy).max(fy - (y1 - rr)).max(0.0);
                        let a = (dx.hypot(dy) - rr + 0.5).clamp(0.0, 1.0);
                        if a > 0.0 {
                            let i = (y * LW + x) * 4;
                            for k in 0..3 {
                                d[i + k] = (d[i + k] as f32 * (1.0 - a)) as u8;
                            }
                        }
                    }
                }
            }
        }
    }

    /// The rounded-corner radius in canvas px (None = square corners).
    pub fn corner_radius(&self) -> Option<f32> {
        self.corners
    }

    /// The effective corner radius: a theme override (px on 2400x1080, 0 = none) wins over the settings default.
    pub fn with_corner_override(mut self, theme_px: Option<f32>) -> Mask {
        if let Some(v) = theme_px {
            self.corners = if v > 0.0 { Some(v * K) } else { None };
        }
        self
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
    fn corners_are_tangent_to_the_visible_rectangle_with_the_fit_and_the_notch() {
        use crate::core::fit::Fit;
        // fit 4/0/0/9 and notch 92 at 2400x1080 are 3/0/0/6 and a 67 px band at 1680x756; radius 80 is 56
        let m = Mask::from_value(&json!({ "mode": "band", "band": { "edge": "left", "px": 92 }, "corners": { "on": true, "r": 80 } }));
        let fit = Fit::new(4, 0, 0, 9);
        let (_, y0, x1, y1) = fit.rect(LW, LH);
        assert_eq!((y0, x1, y1), (3, LW, LH));
        let mut f = vec![255u8; LW * LH * 4];
        m.apply_fit(&mut f, &fit);
        let at = |x: usize, y: usize| f[(y * LW + x) * 4];
        let rr = 56;
        assert_eq!(at(x1 - rr - 4, y0), 255, "top edge row y0 is untouched before the arc: tangent there");
        assert!(at(x1 - 1, y0) < 20, "the corner pixel is black");
        assert!(at(x1 - 1, y0 + rr / 2) < 20 && at(x1 - 1, y0 + rr + 3) == 255, "the arc ends rr rows down");
        assert_eq!(at(67 + rr + 4, y0), 255, "the top-left curve starts after the notch band (67)");
        assert!(at(67, y0) < 20 && at(67 + 3, y0) < 60);
        // without the fit the curve starts at row 0 (as before)
        let mut g = vec![255u8; LW * LH * 4];
        m.apply(&mut g);
        assert_eq!(g[(0 * LW + LW - rr - 4) * 4], 255);
        assert!(g[(LW - 1) * 4] < 20);
        // at the full design size: tangent at y = fit.top
        let big = Fit::new(4, 0, 0, 9).rect(2400, 1080);
        assert_eq!(big, (9, 4, 2400, 1080));
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

    #[test]
    fn corners_follow_the_visible_rect_and_theme_override() {
        // default radius 80 design px (56 canvas px) with a left band: the corners sit at the band's inner edge
        let m = Mask::from_value(&json!({ "mode": "band", "band": { "edge": "left", "px": 92 }, "corners": { "r": 80 } }));
        let mut f = vec![255u8; LW * LH * 4];
        m.apply(&mut f);
        let px = |x: usize, y: usize| f[(y * LW + x) * 4];
        assert_eq!(px(67, 0), 0, "top-left corner of the visible area is rounded");
        assert_eq!(px(67 + 100, 100), 255);
        assert_eq!(px(LW - 1, LH - 1), 0);
        // theme override: 0 = none, a number replaces the default
        let none = m.clone().with_corner_override(Some(0.0));
        let mut g = vec![255u8; LW * LH * 4];
        none.apply(&mut g);
        assert_eq!(g[(0 * LW + 67) * 4], 255);
        let big = m.clone().with_corner_override(Some(200.0));
        assert_eq!(m.with_corner_override(None), Mask::from_value(&json!({ "mode": "band", "band": { "edge": "left", "px": 92 }, "corners": { "r": 80 } })));
        let mut h = vec![255u8; LW * LH * 4];
        big.apply(&mut h);
        assert_eq!(h[(40 * LW + 67 + 40) * 4], 0, "larger radius cuts deeper");
        // 1.2 settings with the toggle off still mean no corners
        assert!(Mask::from_value(&json!({ "mode": "off", "corners": { "on": false, "r": 80 } })).is_none());
    }
}
