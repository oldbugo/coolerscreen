//! 4.0 W2 (webpicture widget): props, cleaning, the (opt-in, network) data loader and its parsing. STUB: filled in by the webpicture worker.
//! Pattern to copy: `live/agenda.rs` (demand-driven background fetch per source: starts on first ask, refreshes on its own schedule,
//! ends a while after the last ask; a `read(source) -> View` that never blocks the render thread). Lenient props
//! (`#[serde(default, rename_all = "camelCase")]`); `clean` bounds every prop (idempotent) with the helpers of `super::morewidgets`.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::theme::Widget;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WebPictureProps {
    pub color: String,
}
impl Default for WebPictureProps {
    fn default() -> Self {
        WebPictureProps { color: "#ffffff".into() }
    }
}

impl Widget {
    pub fn webpicture(&self) -> WebPictureProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`).
pub fn clean(o: &mut Map<String, Value>) {
    super::morewidgets::colour(o, "color", "#ffffff", false);
}
