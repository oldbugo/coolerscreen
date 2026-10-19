//! Drawing of the `sunmoon` widget (4.0 W1). STUB: filled in by the sunmoon worker.
//! Use `self.mcard(b)` (card + inner rect, honours the item's corner radius), `self.text`, `self.fit_px`, `self.fit`,
//! `self.track_and_fill`, `self.eased` ... from `render/more.rs` / `widgets.rs` / `sensors.rs`; `self.sample` = editor preview
//! (fixed sample data, no clock-dependent surprises). Big numbers; nothing moves up and down.
use super::{LocalTime, Rect, Renderer};
use crate::live::theme::Widget;

impl Renderer {
    pub(super) fn draw_sunmoon(&mut self, b: Rect, _w: &Widget, _now: &LocalTime) {
        let _ = self.mcard(b);
    }
}
