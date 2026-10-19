//! Drawing of the `gamefps` widget (4.0 G2). STUB: filled in by the widget worker.
//! Reads `crate::live::gamefps::read()` (a `View`, never blocks); `self.sample` = editor preview: draw a fixed sample (e.g. "Monster Hunter
//! Wilds" 87 fps, 1 % low 61, a frame-time graph) and make NO call to gamefps::read in preview. Big number; graph grows SIDEWAYS only.
use super::{LocalTime, Rect, Renderer};
use crate::live::theme::Widget;

impl Renderer {
    pub(super) fn draw_gamefps(&mut self, b: Rect, _w: &Widget, _now: &LocalTime) {
        let _ = self.mcard(b);
    }
}
