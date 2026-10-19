//! One-frame rendering without a device, for the app's editor preview and for baking `onBase` items into the base.
//! Both use the same widget code as the live renderer, with sample data (no sensors, no media session, no USB).
use super::nowplaying::NowPlaying;
use super::render::{resize_exact, Renderer};
use super::sensors::{Gpu, Sensors};
use super::theme::ThemeLive;
use super::video;
use super::{LH, LW};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// Overlay canvas: the landscape design space the theme boxes are fractions of.
pub const OVERLAY_W: usize = 2400;
pub const OVERLAY_H: usize = 1080;

fn sample_sensors() -> Sensors {
    Sensors {
        cpu_load_pct: 34.0,
        ram_used_gb: 18.4,
        ram_total_gb: 31.9,
        gpu: Some(Gpu { name: "GPU".into(), temp_c: 61, load_pct: 52, clock_mhz: 1950, mem_used_mb: 6200, mem_total_mb: 16000, power_w: Some(182.0), fan_pct: Some(46) }),
        cpu_temp_c: Some(48.0),
    }
}

/// The loop phase the editor preview shows (a pleasing moment, the same on every refresh).
pub const PREVIEW_PHASE: f64 = 0.3;

type FrameKey = (PathBuf, bool, Option<SystemTime>);

/// First frames already decoded (an ffmpeg run each), newest few kept.
fn first_frame_cached(path: &PathBuf, portrait: bool) -> Option<Arc<Vec<u8>>> {
    static CACHE: OnceLock<Mutex<HashMap<FrameKey, Arc<Vec<u8>>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let key: FrameKey = (path.clone(), portrait, std::fs::metadata(path).and_then(|m| m.modified()).ok());
    if let Some(f) = cache.lock().unwrap().get(&key) {
        return Some(f.clone());
    }
    let f = Arc::new(video::first_frame(path, portrait).ok()?);
    let mut g = cache.lock().unwrap();
    if g.len() >= 6 {
        g.clear();
    }
    g.insert(key, f.clone());
    Some(f)
}

/// The still background of the preview: the original clip's first frame when there is one (it has no baked items, so the
/// `onBase` items can be drawn on top), else the base's first frame, else None (the gradient).
fn preview_background(t: &ThemeLive) -> Option<Arc<Vec<u8>>> {
    if let Some(p) = t.fallback_video.as_ref().filter(|p| p.exists()) {
        if let Some(f) = first_frame_cached(p, false) {
            return Some(f);
        }
    }
    let p = t.base_video.as_ref().filter(|p| p.exists())?;
    first_frame_cached(p, true)
}

/// One frame of the theme as PNG at `w` x `h` (landscape; the fractions of the theme boxes apply). `with_live`: all widgets,
/// with sample sensor values and a sample track (the live layer); otherwise the base only: background plus the `onBase` items.
/// No notch mask is drawn (the app overlays its own). Takes well under 150 ms at 1280x576 once the background's first frame
/// is cached (the first call per clip runs ffmpeg once, 100-300 ms).
pub fn render_preview(t: &ThemeLive, w: u32, h: u32, with_live: bool) -> Result<Vec<u8>, String> {
    let (w, h) = (w.clamp(16, 4096), h.clamp(16, 4096));
    let with_live = with_live && t.theme.live_layer;
    let mut theme = t.clone();
    if !with_live {
        theme.theme.widgets.retain(|x| x.on_base && x.can_be_on_base() && t.theme.is_visible(x));
    }
    let mut r = Renderer::new(Arc::new(Mutex::new(sample_sensors())), None);
    r.set_sample_mode(true);
    r.set_theme(theme);
    if t.theme.effect().is_some() {
        r.set_effect_phase(Some(PREVIEW_PHASE)); // the effect draws itself: no first frame to decode
    } else {
        r.set_still_background(preview_background(t));
    }
    let np = NowPlaying::demo("Midnight City", "M83", true, 83.0, 241.0);
    r.set_music_source(Some(Arc::new(Mutex::new(Some(np)))));
    let (rgba, _) = r.render();
    let _ = rgba;
    let mut pm = r.frame_pixmap();
    if (pm.width(), pm.height()) != (w, h) {
        pm = resize_exact(&pm, w, h);
    }
    debug_assert_eq!((LW, LH), (1680, 756));
    pm.encode_png().map_err(|e| format!("PNG encode failed: {e}"))
}

/// The `onBase` items alone as a 2400x1080 RGBA PNG (transparent elsewhere), for baking into the base with an ffmpeg overlay;
/// None when the theme has none.
pub fn render_base_overlay(t: &ThemeLive) -> Result<Option<Vec<u8>>, String> {
    if !t.theme.has_base_items() {
        return Ok(None);
    }
    let mut r = Renderer::new_sized(OVERLAY_W, OVERLAY_H, Arc::new(Mutex::new(Sensors::default())), None);
    r.set_theme(t.clone());
    r.base_overlay_png()
}
