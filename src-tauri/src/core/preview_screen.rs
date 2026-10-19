//! Preview on the real screen: selecting a theme in My themes, or an item in Browse, shows it on the screen at once ("a live
//! demo") and leaving without using it puts the screen back. Nothing is saved (no theme.json, no settings, no stale mark): the
//! preview is only handed to the live engine (`LiveHost::preview_start` / `preview_end`).
//!
//! * My themes: the theme as it is now (its picture, its live layer), streamed even when it would not need streaming.
//! * Browse: the item's own thumbnail, full screen, as a still picture (a source item is not in the library, so there is
//!   nothing else to show before it is added; Wallpaper Engine scenes and anything without a thumbnail show nothing and
//!   say so). Thumbnails of online sources are fetched once over https (at most 6 MB, cached for a day).
use super::app::App;
use super::error::{CoreError, CoreResult};
use crate::live::theme::{Theme, ThemeLive};
use serde_json::{json, Value};
use std::io::Read;
use std::path::PathBuf;

const MAX_PICTURE: u64 = 6_000_000;

fn dir() -> PathBuf {
    super::paths::base().join("preview")
}

fn short(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// The theme of a picture preview: one image filling the canvas, nothing else.
pub fn picture_theme(file: &str) -> Theme {
    let v = json!({ "widgets": [ { "id": "pic", "type": "image", "x": 0.0, "y": 0.0, "w": 1.0, "h": 1.0, "props": { "file": file, "fit": "cover", "opacity": 1.0 } } ] });
    Theme::from_json(&v.to_string()).unwrap_or_default()
}

impl App {
    /// My themes: show this theme on the screen (nothing is saved).
    pub fn preview_theme_on_screen(&self, oid: &str) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(|| CoreError::not_found("That theme is gone."))?;
        let t = self.load_theme(oid);
        let tl = self.theme_live_of(oid, t);
        self.live.preview_start(tl);
        Ok(json!({ "ok": true }))
    }

    /// Browse: show the item's thumbnail on the screen. `url` is the thumbnail address the page shows (https only).
    pub fn preview_picture_on_screen(&self, sid: &str, item: &str, url: Option<&str>) -> CoreResult<Value> {
        let src = self.preview_source(sid, item, url)?;
        let d = dir();
        std::fs::create_dir_all(&d)?;
        // clean up what is older than a day (the previews of earlier sessions)
        if let Ok(rd) = std::fs::read_dir(&d) {
            for e in rd.flatten() {
                let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|m| m.elapsed().ok()).is_some_and(|a| a > std::time::Duration::from_secs(86_400));
                if old {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        let name = format!("{}.img", short(&format!("{sid}/{item}")));
        let dest = d.join(&name);
        if std::fs::copy(&src, &dest).is_err() {
            return Err(CoreError::new("Could not read the picture."));
        }
        let mut tl = ThemeLive::new(picture_theme(&name), d);
        tl.not_embedded = true; // nothing here is on the screen: ending the preview switches back at once
        self.live.preview_start(tl);
        Ok(json!({ "ok": true }))
    }

    fn preview_source(&self, sid: &str, item: &str, url: Option<&str>) -> CoreResult<PathBuf> {
        if let Some(p) = self.source_thumb(sid, item) {
            return Ok(p);
        }
        let Some(u) = url.filter(|u| u.starts_with("https://")) else {
            return Err(CoreError::new("This item has no picture to preview."));
        };
        let file = dir().join(format!("dl-{}.img", short(u)));
        if file.is_file() {
            return Ok(file);
        }
        std::fs::create_dir_all(dir())?;
        let c = crate::sources::http::client(3).map_err(|e| CoreError::new(e.to_string()))?;
        let resp = c
            .get(u)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .map_err(|e| CoreError::new(format!("Could not load the picture: {e}")))?;
        if !resp.status().is_success() {
            return Err(CoreError::new("Could not load the picture."));
        }
        let mut body = Vec::new();
        resp.take(MAX_PICTURE + 1).read_to_end(&mut body).map_err(|e| CoreError::new(format!("Could not load the picture: {e}")))?;
        if body.is_empty() || body.len() as u64 > MAX_PICTURE {
            return Err(CoreError::new("The picture is not usable."));
        }
        std::fs::write(&file, &body)?;
        Ok(file)
    }

    /// Back to what the screen showed before (also when nothing is previewed: then it does nothing).
    pub fn preview_end_on_screen(&self) -> Value {
        json!({ "ended": self.live.preview_end() })
    }

    /// The page still shows the preview (safety timer).
    pub fn preview_keepalive_on_screen(&self) -> Value {
        json!({ "previewing": self.live.preview_keepalive() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_theme_is_one_full_canvas_cover_image() {
        let t = picture_theme("a.img");
        assert_eq!(t.widgets.len(), 1);
        let w = &t.widgets[0];
        assert_eq!((w.kind.as_str(), w.image().file.as_str(), w.image().fit.as_str()), ("image", "a.img", "cover"));
        assert!(w.w > 0.99 && w.h > 0.99);
    }

    #[test]
    fn a_picture_preview_fills_the_whole_canvas() {
        // a red PNG as the thumbnail: every pixel of the frame (cover fit) comes out red
        let d = tempfile::tempdir().unwrap();
        let mut pm = tiny_skia::Pixmap::new(64, 36).unwrap();
        pm.fill(tiny_skia::Color::from_rgba8(255, 0, 0, 255));
        std::fs::write(d.path().join("p.img"), pm.encode_png().unwrap()).unwrap();
        let tl = ThemeLive::new(picture_theme("p.img"), d.path().to_path_buf());
        let mut r = crate::live::render::Renderer::new(std::sync::Arc::new(std::sync::Mutex::new(crate::live::sensors::Sensors::default())), None);
        r.set_theme(tl);
        let f = r.render().0.to_vec();
        let at = |x: usize, y: usize| &f[(y * crate::live::LW + x) * 4..][..3];
        for (x, y) in [(840usize, 378usize), (2, 2), (crate::live::LW - 3, crate::live::LH - 3), (crate::live::LW / 4, 3)] {
            assert!(at(x, y)[0] > 200 && at(x, y)[1] < 60 && at(x, y)[2] < 60, "red at {x},{y}: {:?}", at(x, y));
        }
    }
}
