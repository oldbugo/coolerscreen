//! 4.0 W1 (sunmoon widget): props, cleaning and the data logic. STUB: filled in by the sunmoon worker; drawing is in render/sunmoon.rs.
//! Rules: lenient props (`#[serde(default, rename_all = "camelCase")]`), defaults behave sensibly and nothing new appears in
//! existing themes; `clean` bounds every prop (idempotent) using the helpers `word`, `colour`, `number`, `int`, `boolean`, `short`
//! of `super::morewidgets`.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::theme::Widget;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SunMoonProps {
    pub color: String,
}
impl Default for SunMoonProps {
    fn default() -> Self {
        SunMoonProps { color: "#ffffff".into() }
    }
}

impl Widget {
    pub fn sunmoon(&self) -> SunMoonProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`).
pub fn clean(o: &mut Map<String, Value>) {
    super::morewidgets::colour(o, "color", "#ffffff", false);
}
