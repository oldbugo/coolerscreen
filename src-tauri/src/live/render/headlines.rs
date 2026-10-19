//! Drawing of the `headlines` widget (4.0 W2). STUB: filled in by the headlines worker.
//! Use `self.mcard(b)` (card + inner rect, honours the item's corner radius), `self.text`, `self.fit_px`, `self.fit`,
//! `self.card_radius(...)`, `self.clip_rounded(...)` ... from `render.rs` / `render/more.rs` / `widgets.rs`; `self.sample` = editor
//! preview (fixed sample data, NO network access in preview). Big readable text; nothing moves up and down.
use super::{LocalTime, Rect, Renderer};
use crate::live::theme::Widget;

impl Renderer {
    pub(super) fn draw_headlines(&mut self, b: Rect, _w: &Widget, _now: &LocalTime) {
        let _ = self.mcard(b);
    }
}
