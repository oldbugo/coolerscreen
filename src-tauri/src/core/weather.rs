//! Weather place (docs/v3/theme-format.md, v1.6): the user types a city, Open-Meteo's geocoding answers, the chosen place is
//! stored in settings `weather.place` { name, lat, lon, tz }. Nothing location-related is automatic: the only network call
//! here happens when the user presses Search, and nothing is stored until a place is picked.
use super::app::App;
use super::error::{CoreError, CoreResult};
use serde_json::{json, Value};

const GEOCODING: &str = "https://geocoding-api.open-meteo.com/v1/search";

/// Turn the geocoding answer into `[{ name, country, admin1, lat, lon, tz }]` (rows without a usable position are dropped).
pub fn parse_results(answer: &Value) -> Vec<Value> {
    answer["results"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let (lat, lon) = (r["latitude"].as_f64()?, r["longitude"].as_f64()?);
                    let name = r["name"].as_str().filter(|n| !n.trim().is_empty())?;
                    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                        return None;
                    }
                    Some(json!({
                        "name": name, "country": r["country"].as_str().unwrap_or(""), "admin1": r["admin1"].as_str().unwrap_or(""),
                        "lat": lat, "lon": lon, "tz": r["timezone"].as_str().unwrap_or(""),
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A place from the page, cleaned: `{ name, lat, lon, tz }` or None when it is not usable.
pub fn clean_place(v: &Value) -> Option<Value> {
    let (lat, lon) = (v["lat"].as_f64()?, v["lon"].as_f64()?);
    if !lat.is_finite() || !lon.is_finite() || !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    let name: String = v["name"].as_str()?.trim().chars().take(80).collect();
    if name.is_empty() {
        return None;
    }
    let tz: String = v["tz"].as_str().unwrap_or("").trim().chars().filter(|c| c.is_ascii_alphanumeric() || "/_+-".contains(*c)).take(64).collect();
    let r4 = |x: f64| (x * 10000.0).round() / 10000.0;
    Some(json!({ "name": name, "lat": r4(lat), "lon": r4(lon), "tz": tz }))
}

impl App {
    /// Search cities by name (Open-Meteo geocoding, no key). Called only when the user presses Search.
    pub fn weather_search(&self, query: &str) -> CoreResult<Value> {
        let q = query.trim();
        if q.chars().count() < 2 {
            return Err(CoreError::new("Type at least 2 letters."));
        }
        let q: String = q.chars().take(80).collect();
        let mut url = reqwest::Url::parse(GEOCODING).map_err(|e| CoreError::new(e.to_string()))?;
        url.query_pairs_mut().append_pair("name", &q).append_pair("count", "8").append_pair("language", "en").append_pair("format", "json");
        let answer = crate::sources::http::get_json(url, "the weather service").map_err(|e| CoreError::new(e.0))?;
        Ok(json!({ "results": parse_results(&answer) }))
    }

    /// Store the chosen place (`null` clears it) -> the app state.
    pub fn weather_set_place(&self, place: &Value) -> CoreResult<Value> {
        let clean = if place.is_null() { Value::Null } else { clean_place(place).ok_or_else(|| CoreError::new("That place can't be used."))? };
        self.settings.update(|s| {
            let mut w = s.get("weather").cloned().filter(Value::is_object).unwrap_or_else(|| json!({}));
            if clean.is_null() {
                w.as_object_mut().map(|o| o.remove("place"));
            } else {
                w["place"] = clean.clone();
            }
            s["weather"] = w;
        })?;
        self.lib.bump();
        self.refresh_live_theme();
        Ok(self.app_state())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geocoding_rows() {
        let a = json!({ "results": [
            { "id": 1, "name": "Sydney", "latitude": -33.8678, "longitude": 151.2073, "country": "Australia", "admin1": "New South Wales", "timezone": "Australia/Sydney" },
            { "name": "Nowhere" },
            { "name": "Bad", "latitude": 95.0, "longitude": 0.0 },
            { "name": "", "latitude": 1.0, "longitude": 1.0 } ] });
        let r = parse_results(&a);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0], json!({ "name": "Sydney", "country": "Australia", "admin1": "New South Wales", "lat": -33.8678, "lon": 151.2073, "tz": "Australia/Sydney" }));
        assert!(parse_results(&json!({ "generationtime_ms": 0.3 })).is_empty(), "no match: Open-Meteo sends no results key");
    }

    #[test]
    fn place_is_cleaned() {
        let p = clean_place(&json!({ "name": " Sydney ", "lat": -33.86781234, "lon": 151.2, "tz": "Australia/Sydney", "country": "x" })).unwrap();
        assert_eq!(p, json!({ "name": "Sydney", "lat": -33.8678, "lon": 151.2, "tz": "Australia/Sydney" }));
        assert!(clean_place(&json!({ "name": "A", "lat": 91, "lon": 0 })).is_none());
        assert!(clean_place(&json!({ "name": "", "lat": 1, "lon": 0 })).is_none());
        assert!(clean_place(&json!({ "name": "A", "lat": "1", "lon": 0 })).is_none());
        assert_eq!(clean_place(&json!({ "name": "A", "lat": 1, "lon": 2, "tz": "a b<script>" })).unwrap()["tz"], "abscript");
    }
}
