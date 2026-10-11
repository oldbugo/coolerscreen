//! Notch keys and frame normalisation shared with the media engine (1.2 `convert.bandPx`, `normFrame`,
//! `library.notchKey`). Equal keys = equal pixels on the screen.
use super::js::{fmt, num, round, to_num};
use serde_json::{json, Value};

/// Band width rounded up to the 16 px MJPEG block grid (the top edge is offset by 1080 % 16).
pub fn band_px(band: &Value) -> f64 {
    let px = round(to_num(band.get("px")));
    if !(px > 0.0) {
        return 0.0;
    }
    let offset = if band.get("edge").and_then(Value::as_str) == Some("top") { (1080 % 16) as f64 } else { 0.0 };
    ((px - offset) / 16.0).ceil() * 16.0 + offset
}

/// The notch part of the mask as applied ("off", "band:left:96", "circle:cx:cy:r:feather").
pub fn notch_key(mask: &Value) -> String {
    match mask.get("mode").and_then(Value::as_str) {
        None | Some("off") => "off".into(),
        Some("band") => {
            let band = &mask["band"];
            let px = band_px(band);
            if px > 0.0 {
                format!("band:{}:{}", band["edge"].as_str().unwrap_or("left"), fmt(px))
            } else {
                "off".into()
            }
        }
        Some(_) => {
            let c = &mask["circle"];
            let g = |k: &str| to_num(c.get(k));
            if g("r") > 0.0 {
                format!("circle:{}:{}:{}:{}", fmt(g("cx")), fmt(g("cy")), fmt(g("r")), fmt(g("feather")))
            } else {
                "off".into()
            }
        }
    }
}

/// Versions made while corners were global have "|corners:R" in their key; that part is ignored when comparing.
pub fn base_key(k: Option<&str>) -> &str {
    k.unwrap_or("").split('|').next().unwrap_or("")
}

/// `normFrame`: zoom 0.2..4, x/y 0..1, 3 decimals; defaults 1 / 0.5 / 0.5.
pub fn norm_frame(f: &Value) -> Value {
    let n = |k: &str, d: f64, lo: f64, hi: f64| {
        let v = f.get(k);
        let ok = match v {
            None | Some(Value::Null) => false,
            Some(Value::String(s)) if s.is_empty() => false,
            _ => to_num(v).is_finite(),
        };
        let x = if ok { to_num(v).max(lo).min(hi) } else { d };
        round(x * 1000.0) / 1000.0
    };
    json!({ "zoom": num(n("zoom", 1.0, 0.2, 4.0)), "x": num(n("x", 0.5, 0.0, 1.0)), "y": num(n("y", 0.5, 0.0, 1.0)) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_like_node() {
        assert_eq!(band_px(&json!({ "edge": "left", "px": 92 })), 96.0);
        assert_eq!(band_px(&json!({ "edge": "top", "px": 92 })), 104.0);
        assert_eq!(notch_key(&json!({ "mode": "band", "band": { "edge": "left", "px": 92 } })), "band:left:96");
        assert_eq!(notch_key(&json!({ "mode": "band", "band": { "edge": "left", "px": 0 } })), "off");
        assert_eq!(notch_key(&json!({ "mode": "circle", "circle": { "cx": 45, "cy": 540, "r": 45, "feather": 0 } })), "circle:45:540:45:0");
        assert_eq!(notch_key(&json!({ "mode": "off" })), "off");
        assert_eq!(base_key(Some("band:left:96|corners:80")), "band:left:96");
        assert_eq!(norm_frame(&json!({ "zoom": 9, "x": "0.25" })), json!({ "zoom": 4, "x": 0.25, "y": 0.5 }));
        assert_eq!(norm_frame(&Value::Null), json!({ "zoom": 1, "x": 0.5, "y": 0.5 }));
    }
}
