//! Screen fit (3.0 theme format v1.5): how many pixels the panel hides at each edge, measured in the app's Screen fit page.
//! Values are screen pixels (the 2400x1080 landscape space, `top` = top of the picture as you look at the screen).
//! The visible rectangle is the panel inset by these. Every picture sent to the screen is the design canvas scaled to fit that
//! rectangle, with the hidden strips black; the notch band and the corner radius are not part of it (they describe the panel).
use serde_json::{json, Value};

/// Largest value per edge.
pub const MAX: u32 = 200;

/// Largest shift per axis (screen px, either direction).
pub const MAX_SHIFT: i32 = 20;

/// Hidden edge strips (`top`..`left`, 0..=200) and a whole-picture shift (`shift_x` right, `shift_y` down, -20..=20), all screen px.
/// The picture is scaled into the inset rectangle first and then moved by the shift; whatever is not covered is black.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Fit {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
    pub shift_x: i32,
    pub shift_y: i32,
}

fn one(v: Option<&Value>) -> u32 {
    match v.and_then(Value::as_f64) {
        Some(x) if x.is_finite() => x.round().clamp(0.0, MAX as f64) as u32,
        _ => 0,
    }
}

fn shift(v: Option<&Value>) -> i32 {
    match v.and_then(Value::as_f64) {
        Some(x) if x.is_finite() => x.round().clamp(-(MAX_SHIFT as f64), MAX_SHIFT as f64) as i32,
        _ => 0,
    }
}

impl Fit {
    pub fn new(top: u32, right: u32, bottom: u32, left: u32) -> Fit {
        Fit { top, right, bottom, left, shift_x: 0, shift_y: 0 }.clamped()
    }

    pub fn with_shift(self, x: i32, y: i32) -> Fit {
        Fit { shift_x: x, shift_y: y, ..self }.clamped()
    }

    /// From `{ top, right, bottom, left }`: whole numbers 0..=200; anything else counts as 0.
    pub fn from_value(v: &Value) -> Fit {
        Fit { top: one(v.get("top")), right: one(v.get("right")), bottom: one(v.get("bottom")), left: one(v.get("left")), shift_x: shift(v.get("shiftX")), shift_y: shift(v.get("shiftY")) }
    }

    /// `base` with the keys present in `patch` replaced (missing keys keep their value).
    pub fn patched(base: Fit, patch: &Value) -> Fit {
        let pick = |k: &str, old: u32| if patch.get(k).is_some() { one(patch.get(k)) } else { old };
        let pick_s = |k: &str, old: i32| if patch.get(k).is_some() { shift(patch.get(k)) } else { old };
        Fit {
            top: pick("top", base.top),
            right: pick("right", base.right),
            bottom: pick("bottom", base.bottom),
            left: pick("left", base.left),
            shift_x: pick_s("shiftX", base.shift_x),
            shift_y: pick_s("shiftY", base.shift_y),
        }
    }

    pub fn clamped(self) -> Fit {
        Fit {
            top: self.top.min(MAX),
            right: self.right.min(MAX),
            bottom: self.bottom.min(MAX),
            left: self.left.min(MAX),
            shift_x: self.shift_x.clamp(-MAX_SHIFT, MAX_SHIFT),
            shift_y: self.shift_y.clamp(-MAX_SHIFT, MAX_SHIFT),
        }
    }

    pub fn to_value(&self) -> Value {
        json!({ "top": self.top, "right": self.right, "bottom": self.bottom, "left": self.left, "shiftX": self.shift_x, "shiftY": self.shift_y })
    }

    pub fn is_zero(&self) -> bool {
        *self == Fit::default()
    }

    /// Part of a version's cache key: empty for no fit, so bases made before the setting existed stay current. The shift is
    /// only in the key when there is one (so a fit without a shift keeps the key it had).
    pub fn key(&self) -> String {
        if self.is_zero() {
            String::new()
        } else if self.shift_x == 0 && self.shift_y == 0 {
            format!("+fit:{}.{}.{}.{}", self.top, self.right, self.bottom, self.left)
        } else {
            format!("+fit:{}.{}.{}.{}.{}.{}", self.top, self.right, self.bottom, self.left, self.shift_x, self.shift_y)
        }
    }

    /// The fit part of a stored key (`notch_key` + `Fit::key`), as `Fit::key` would give it ("" when there is none).
    pub fn key_of(stored: Option<&str>) -> String {
        let s = stored.unwrap_or("");
        s.split('|').next().unwrap_or("").find("+fit:").map(|i| s[i..].split('|').next().unwrap_or("").to_string()).unwrap_or_default()
    }

    /// Hidden pixels at each edge on a `w` x `h` picture (the 2400x1080 values scaled), rounded to whole pixels.
    pub fn px(&self, w: usize, h: usize) -> [usize; 4] {
        let sx = |v: u32| (v as usize * w + 1200) / 2400;
        let sy = |v: u32| (v as usize * h + 540) / 1080;
        [sy(self.top), sx(self.right), sy(self.bottom), sx(self.left)]
    }

    /// Shift in pixels of a `w` x `h` picture (the 2400x1080 values scaled, rounded away from zero).
    pub fn shift_px(&self, w: usize, h: usize) -> (i64, i64) {
        let s = |v: i32, extent: usize, design: usize| {
            let m = (v.unsigned_abs() as usize * extent + design / 2) / design;
            if v < 0 { -(m as i64) } else { m as i64 }
        };
        (s(self.shift_x, w, 2400), s(self.shift_y, h, 1080))
    }

    /// Where the picture goes: the inset rectangle moved by the shift, `[x0, x1) x [y0, y1)`. It may reach beyond the picture
    /// (the part outside is cut off).
    pub fn placed(&self, w: usize, h: usize) -> (i64, i64, i64, i64) {
        let (x0, y0, x1, y1) = self.rect(w, h);
        let (dx, dy) = self.shift_px(w, h);
        (x0 as i64 + dx, y0 as i64 + dy, x1 as i64 + dx, y1 as i64 + dy)
    }

    /// The inset rectangle `[x0, x1) x [y0, y1)` on a `w` x `h` picture (before the shift).
    pub fn rect(&self, w: usize, h: usize) -> (usize, usize, usize, usize) {
        let [t, r, b, l] = self.px(w, h);
        let (x0, y0) = (l.min(w.saturating_sub(2)), t.min(h.saturating_sub(2)));
        let (x1, y1) = (w.saturating_sub(r).max(x0 + 1), h.saturating_sub(b).max(y0 + 1));
        (x0, y0, x1, y1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_are_clamped_and_garbage_is_zero() {
        let f = Fit::from_value(&json!({ "top": 4, "right": -3, "bottom": 999, "left": "9" }));
        assert_eq!(f, Fit::new(4, 0, 200, 0));
        assert_eq!(Fit::from_value(&json!(null)), Fit::default());
        assert_eq!(Fit::from_value(&json!({ "top": 3.6 })).top, 4);
        let p = Fit::patched(Fit::new(4, 0, 0, 9), &json!({ "left": 10, "right": 300 }));
        assert_eq!(p, Fit::new(4, 200, 0, 10));
    }

    #[test]
    fn the_visible_rectangle_for_the_users_screen() {
        let f = Fit::new(4, 0, 0, 9);
        assert_eq!(f.rect(2400, 1080), (9, 4, 2400, 1080));
        // the live picture is 1680x756: one sent pixel is 1.43 screen pixels
        assert_eq!(f.rect(1680, 756), (6, 3, 1680, 756));
        assert_eq!(Fit::default().rect(1680, 756), (0, 0, 1680, 756));
    }

    #[test]
    fn keys_are_empty_without_a_fit_and_readable_back() {
        assert_eq!(Fit::default().key(), "");
        let k = Fit::new(4, 0, 0, 9).key();
        assert_eq!(k, "+fit:4.0.0.9");
        assert_eq!(Fit::key_of(Some(&format!("band:left:96{k}"))), k);
        assert_eq!(Fit::key_of(Some("band:left:96|corners:80")), "");
        assert_eq!(Fit::key_of(None), "");
    }
}

#[cfg(test)]
mod shift_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shift_is_validated_scaled_and_part_of_the_key() {
        let f = Fit::from_value(&json!({ "top": 4, "left": 9, "shiftX": -3, "shiftY": 99 }));
        assert_eq!((f.shift_x, f.shift_y), (-3, 20), "-20..20");
        assert_eq!(Fit::from_value(&json!({ "shiftX": "x" })).shift_x, 0);
        assert_eq!(f.to_value()["shiftX"], json!(-3));
        assert_eq!(Fit::patched(f, &json!({ "shiftY": -7 })).shift_y, -7);
        assert_eq!(Fit::patched(f, &json!({ "top": 1 })).shift_x, -3, "keys left out keep their value");
        // 4/0/0/9 without a shift keeps its earlier key; with one the shift is added
        assert_eq!(Fit::new(4, 0, 0, 9).key(), "+fit:4.0.0.9");
        assert_eq!(Fit::new(4, 0, 0, 9).with_shift(2, -1).key(), "+fit:4.0.0.9.2.-1");
        assert_eq!(Fit::default().with_shift(3, 0).key(), "+fit:0.0.0.0.3.0", "a shift alone is a fit");
        assert!(!Fit::default().with_shift(3, 0).is_zero());
        assert_eq!(Fit::key_of(Some("band:left:96+fit:4.0.0.9.2.-1")), "+fit:4.0.0.9.2.-1");
    }

    #[test]
    fn the_picture_is_placed_by_inset_then_shift() {
        let f = Fit::new(4, 0, 0, 9).with_shift(7, -3);
        // 1680x756: shift 7 -> 5 sent px right (4.9), -3 -> -2 down; inset (6, 3)
        assert_eq!(f.shift_px(1680, 756), (5, -2));
        assert_eq!(f.placed(1680, 756), (6 + 5, 3 - 2, 1680 + 5, 756 - 2));
        // 2400x1080 is exact
        assert_eq!(f.placed(2400, 1080), (9 + 7, 4 - 3, 2400 + 7, 1080 - 3));
        assert_eq!(Fit::default().placed(1680, 756), (0, 0, 1680, 756));
        assert_eq!(Fit::default().with_shift(-20, 20).shift_px(2400, 1080), (-20, 20));
    }
}
