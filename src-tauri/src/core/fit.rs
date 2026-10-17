//! Screen fit (3.0 theme format v1.5): how many pixels the panel hides at each edge, measured in the app's Screen fit page.
//! Values are screen pixels (the 2400x1080 landscape space, `top` = top of the picture as you look at the screen).
//! The visible rectangle is the panel inset by these. Every picture sent to the screen is the design canvas scaled to fit that
//! rectangle, with the hidden strips black; the notch band and the corner radius are not part of it (they describe the panel).
use serde_json::{json, Value};

/// Largest value per edge.
pub const MAX: u32 = 200;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Fit {
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
    pub left: u32,
}

fn one(v: Option<&Value>) -> u32 {
    match v.and_then(Value::as_f64) {
        Some(x) if x.is_finite() => x.round().clamp(0.0, MAX as f64) as u32,
        _ => 0,
    }
}

impl Fit {
    pub fn new(top: u32, right: u32, bottom: u32, left: u32) -> Fit {
        Fit { top, right, bottom, left }.clamped()
    }

    /// From `{ top, right, bottom, left }`: whole numbers 0..=200; anything else counts as 0.
    pub fn from_value(v: &Value) -> Fit {
        Fit { top: one(v.get("top")), right: one(v.get("right")), bottom: one(v.get("bottom")), left: one(v.get("left")) }
    }

    /// `base` with the keys present in `patch` replaced (missing keys keep their value).
    pub fn patched(base: Fit, patch: &Value) -> Fit {
        let pick = |k: &str, old: u32| if patch.get(k).is_some() { one(patch.get(k)) } else { old };
        Fit { top: pick("top", base.top), right: pick("right", base.right), bottom: pick("bottom", base.bottom), left: pick("left", base.left) }
    }

    pub fn clamped(self) -> Fit {
        Fit { top: self.top.min(MAX), right: self.right.min(MAX), bottom: self.bottom.min(MAX), left: self.left.min(MAX) }
    }

    pub fn to_value(&self) -> Value {
        json!({ "top": self.top, "right": self.right, "bottom": self.bottom, "left": self.left })
    }

    pub fn is_zero(&self) -> bool {
        *self == Fit::default()
    }

    /// As an array in the order top, right, bottom, left.
    pub fn as_array(&self) -> [u32; 4] {
        [self.top, self.right, self.bottom, self.left]
    }

    /// Part of a version's cache key: empty for no fit, so bases made before the setting existed stay current.
    pub fn key(&self) -> String {
        if self.is_zero() { String::new() } else { format!("+fit:{}.{}.{}.{}", self.top, self.right, self.bottom, self.left) }
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

    /// The visible rectangle `[x0, x1) x [y0, y1)` on a `w` x `h` picture.
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
        assert_eq!(f, Fit { top: 4, right: 0, bottom: 200, left: 0 });
        assert_eq!(Fit::from_value(&json!(null)), Fit::default());
        assert_eq!(Fit::from_value(&json!({ "top": 3.6 })).top, 4);
        let p = Fit::patched(Fit::new(4, 0, 0, 9), &json!({ "left": 10, "right": 300 }));
        assert_eq!(p, Fit { top: 4, right: 200, bottom: 0, left: 10 });
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
