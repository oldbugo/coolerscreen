//! One-frame rendering without a device, for the app's editor preview and for baking `onBase` items into the base.
use super::theme::ThemeLive;

/// One frame of the theme as PNG at w x h (landscape). `with_live`: draw all widgets with sample data (live layer);
/// otherwise only the base: background plus the `onBase` items.
pub fn render_preview(_t: &ThemeLive, _w: u32, _h: u32, _with_live: bool) -> Result<Vec<u8>, String> {
    Err("render_preview: not implemented yet".into())
}

/// The `onBase` items alone as a 2400x1080 RGBA PNG (transparent elsewhere); None when the theme has none.
pub fn render_base_overlay(_t: &ThemeLive) -> Result<Option<Vec<u8>>, String> {
    Err("render_base_overlay: not implemented yet".into())
}
