//! 4.0 live effects: backgrounds the engine draws itself (no video file), chosen per theme with `background.kind = "effect"`
//! (docs/v3/theme-format.md, "v4 effects"). Drawn on the CPU straight into the frame, so they cost a few percent of one core.
//!
//! Rules every effect follows:
//!  * motion is sideways or a fade, never along the landscape vertical axis (the panel tears there): a pixel's colour may
//!    change, and patterns may slide left or right, but nothing slides up or down;
//!  * an effect is a pure function of its settings and the loop phase `phi = seconds * speed / LOOP_BASE_S`, and it is periodic in
//!    `phi` with period 1 (every sideways speed is a whole number of turns per loop). So `loop_seconds()` of video rendered from
//!    the same theme loops seamlessly (the Embedded face), and the preview can show any phase;
//!  * it fills only the rectangle it is given (the visible area, see the screen fit): the picture is the design canvas scaled into
//!    that rectangle, with no resampling afterwards.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod aurora;
pub mod clouds;
pub mod drift;
pub mod plasma;
pub mod util;

use util::Span;

/// Seconds one loop lasts at speed 1.
pub const LOOP_BASE_S: f32 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Aurora,
    Plasma,
    Drift,
    Clouds,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Aurora, Kind::Plasma, Kind::Drift, Kind::Clouds];
    pub fn name(self) -> &'static str {
        match self {
            Kind::Aurora => "aurora",
            Kind::Plasma => "plasma",
            Kind::Drift => "drift",
            Kind::Clouds => "clouds",
        }
    }
    pub fn parse(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == s)
    }
    /// The settings an effect starts with (what the editor shows before anything is changed).
    pub fn defaults(self) -> Params {
        let c = |v: &[&str]| v.iter().map(|s| crate::live::theme::parse_color(s, [255, 255, 255])).collect::<Vec<_>>();
        match self {
            Kind::Aurora => Params { kind: self, speed: 1.0, density: 0.5, scale: 0.5, brightness: 0.85, colors: c(&["#2cf5a0", "#3a7bff", "#b24bff"]), seed: 1 },
            Kind::Plasma => Params { kind: self, speed: 1.0, density: 0.5, scale: 0.5, brightness: 0.85, colors: c(&["#1b0f4a", "#c13a8f", "#ff9a3c", "#3a1d7a"]), seed: 1 },
            Kind::Drift => Params { kind: self, speed: 1.0, density: 0.5, scale: 0.5, brightness: 0.9, colors: c(&["#7fd4ff", "#ffffff", "#a58bff"]), seed: 1 },
            Kind::Clouds => Params { kind: self, speed: 0.5, density: 0.5, scale: 0.5, brightness: 0.9, colors: c(&["#0b1030", "#3b2f8f", "#d3689b", "#ffd9a8"]), seed: 1 },
        }
    }
}

/// The effect settings, resolved (every value present and in range).
#[derive(Clone, Debug, PartialEq)]
pub struct Params {
    pub kind: Kind,
    /// Loop speed 0.1..3 (1 = one loop in 10 s).
    pub speed: f32,
    /// How much there is: rays, particles, cloud cover (0..1).
    pub density: f32,
    /// Feature size (0..1).
    pub scale: f32,
    /// Overall brightness (0.1..1.5).
    pub brightness: f32,
    /// 2 to 4 colours.
    pub colors: Vec<[u8; 3]>,
    pub seed: u32,
}

impl Params {
    pub fn loop_seconds(&self) -> f32 {
        LOOP_BASE_S / self.speed
    }
}

/// theme.json `background`. Missing, or any kind other than "effect" = the theme's picture, as in 3.0.
/// Unset numbers and colours take the effect's defaults; the editor writes them as they are changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Background {
    /// "picture" | "effect"
    pub kind: String,
    /// "aurora" | "plasma" | "drift" | "clouds"
    pub effect: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub density: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scale: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub brightness: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colors: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u32>,
}

impl Background {
    /// The resolved effect, if this background is a known effect.
    pub fn params(&self) -> Option<Params> {
        if self.kind != "effect" {
            return None;
        }
        let k = Kind::parse(&self.effect)?;
        let mut p = k.defaults();
        let fin = |v: Option<f32>, lo: f32, hi: f32, d: f32| v.filter(|x| x.is_finite()).map(|x| x.clamp(lo, hi)).unwrap_or(d);
        p.speed = fin(self.speed, 0.1, 3.0, p.speed);
        p.density = fin(self.density, 0.0, 1.0, p.density);
        p.scale = fin(self.scale, 0.0, 1.0, p.scale);
        p.brightness = fin(self.brightness, 0.1, 1.5, p.brightness);
        if let Some(cs) = &self.colors {
            let cols: Vec<[u8; 3]> = cs.iter().take(4).map(|s| crate::live::theme::parse_color(s, [255, 255, 255])).collect();
            if cols.len() >= 2 {
                p.colors = cols;
            }
        }
        p.seed = self.seed.unwrap_or(p.seed);
        Some(p)
    }

    /// Bring stored values into range (what `params` would do anyway) so theme.json stays tidy.
    pub fn clamp(&mut self) {
        if self.kind == "effect" && Kind::parse(&self.effect).is_none() {
            self.effect = Kind::Aurora.name().into();
        }
        let fix = |v: &mut Option<f32>, lo: f32, hi: f32| {
            if let Some(x) = v {
                *v = if x.is_finite() { Some(x.clamp(lo, hi)) } else { None };
            }
        };
        fix(&mut self.speed, 0.1, 3.0);
        fix(&mut self.density, 0.0, 1.0);
        fix(&mut self.scale, 0.0, 1.0);
        fix(&mut self.brightness, 0.1, 1.5);
        if let Some(c) = &mut self.colors {
            c.truncate(4);
            if c.len() < 2 {
                self.colors = None;
            }
        }
    }
}

/// What every effect implements. `draw` fills `sp` on `data` (RGBA, opaque, canvas width `sp.cw`) for loop phase `phi`.
pub trait Effect: Send {
    fn draw(&mut self, phi: f64, sp: &Span, data: &mut [u8]);
}

pub fn make(p: &Params) -> Box<dyn Effect> {
    match p.kind {
        Kind::Aurora => Box::new(aurora::Aurora::new(p.clone())),
        Kind::Plasma => Box::new(plasma::Plasma::new(p.clone())),
        Kind::Drift => Box::new(drift::Drift::new(p.clone())),
        Kind::Clouds => Box::new(clouds::Clouds::new(p.clone())),
    }
}

/// A running effect: its settings and drawing state.
pub struct EffectBg {
    pub params: Params,
    fx: Box<dyn Effect>,
}

impl EffectBg {
    pub fn new(params: Params) -> EffectBg {
        let fx = make(&params);
        EffectBg { params, fx }
    }
    /// Draw at loop phase `phi` (see the module doc) into the rectangle `rect` of the canvas.
    pub fn draw_phase(&mut self, phi: f64, data: &mut [u8], cw: usize, ch: usize, rect: (i64, i64, i64, i64)) {
        if let Some(sp) = Span::new(cw, ch, rect) {
            self.fx.draw(phi, &sp, data);
        }
    }
    /// Draw at `secs` seconds after the start.
    pub fn draw_at(&mut self, secs: f64, data: &mut [u8], cw: usize, ch: usize, rect: (i64, i64, i64, i64)) {
        self.draw_phase(secs * self.params.speed as f64 / LOOP_BASE_S as f64, data, cw, ch, rect);
    }
}

/// The same table the editor shows, as JSON (names, defaults), for tests and documentation.
pub fn catalog() -> Value {
    Value::Array(
        Kind::ALL
            .iter()
            .map(|k| {
                let d = k.defaults();
                serde_json::json!({ "effect": k.name(), "speed": d.speed, "density": d.density, "scale": d.scale, "brightness": d.brightness,
                    "colors": d.colors.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect::<Vec<_>>() })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_parses_leniently_and_defaults_fill_in() {
        let b: Background = serde_json::from_str(r##"{"kind":"effect","effect":"plasma","speed":9,"colors":["#ff0000"],"bogus":1}"##).unwrap();
        let p = b.params().unwrap();
        assert_eq!(p.kind, Kind::Plasma);
        assert_eq!(p.speed, 3.0);
        assert_eq!(p.colors, Kind::Plasma.defaults().colors, "one colour is not a ramp: defaults stay");
        assert!(Background::default().params().is_none());
        let pic: Background = serde_json::from_str(r#"{"kind":"picture","effect":"aurora"}"#).unwrap();
        assert!(pic.params().is_none());
        let unknown: Background = serde_json::from_str(r#"{"kind":"effect","effect":"nope"}"#).unwrap();
        assert!(unknown.params().is_none());
    }

    #[test]
    fn unset_values_are_not_written() {
        let b = Background { kind: "effect".into(), effect: "drift".into(), ..Background::default() };
        let s = serde_json::to_string(&b).unwrap();
        assert!(!s.contains("speed") && !s.contains("colors"), "{s}");
    }
}

#[cfg(test)]
mod effect_tests {
    use super::*;

    const W: usize = 1680;
    const H: usize = 756;

    fn frame(k: Kind, phi: f64) -> Vec<u8> {
        let mut e = EffectBg::new(k.defaults());
        let mut d = vec![0u8; W * H * 4];
        e.draw_phase(phi, &mut d, W, H, (0, 0, W as i64, H as i64));
        d
    }

    #[test]
    fn every_effect_fills_the_frame_opaque_and_not_black() {
        for k in Kind::ALL {
            let d = frame(k, 0.3);
            assert!(d.chunks_exact(4).all(|p| p[3] == 255), "{} alpha", k.name());
            let mean = d.chunks_exact(4).map(|p| p[0] as u64 + p[1] as u64 + p[2] as u64).sum::<u64>() as f64 / (W * H) as f64 / 3.0;
            assert!(mean > 6.0 && mean < 200.0, "{} mean {mean}", k.name());
        }
    }

    #[test]
    fn every_effect_loops_exactly_after_one_loop() {
        for k in Kind::ALL {
            let (a, b) = (frame(k, 0.17), frame(k, 1.17));
            let worst = a.iter().zip(&b).map(|(x, y)| (*x as i32 - *y as i32).abs()).max().unwrap();
            assert!(worst <= 2, "{} differs by {worst} after a loop", k.name());
            let c = frame(k, 0.17 + 0.25);
            assert!(a.iter().zip(&c).filter(|(x, y)| x != y).count() > W * H / 50, "{} does not move", k.name());
        }
    }

    #[test]
    fn effects_stay_inside_their_rectangle() {
        for k in Kind::ALL {
            let mut e = EffectBg::new(k.defaults());
            let mut d = vec![7u8; W * H * 4];
            // a visible area shifted partly off the canvas, as with a screen fit shift
            e.draw_phase(0.4, &mut d, W, H, (-20, 10, 1600, 700));
            let px = |x: usize, y: usize| d[(y * W + x) * 4];
            assert_eq!(px(1650, 100), 7, "{} drew right of the rectangle", k.name());
            assert_eq!(px(100, 3), 7, "{} drew above the rectangle", k.name());
            assert_eq!(px(100, 720), 7, "{} drew below the rectangle", k.name());
            assert_ne!(d[(300 * W + 0) * 4 + 3], 7, "{} left the clipped edge empty", k.name());
        }
    }

    /// COOLERSCREEN_EFFECT_DUMP=<dir> writes a few frames of every effect as PNG, to look at.
    #[test]
    fn dump_frames_when_asked() {
        let Some(dir) = std::env::var_os("COOLERSCREEN_EFFECT_DUMP") else { return };
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for k in Kind::ALL {
            for (i, phi) in [0.0, 0.25, 0.5].into_iter().enumerate() {
                let d = frame(k, phi);
                let mut pm = tiny_skia::Pixmap::new(W as u32, H as u32).unwrap();
                pm.data_mut().copy_from_slice(&d);
                std::fs::write(dir.join(format!("{}-{i}.png", k.name())), pm.encode_png().unwrap()).unwrap();
            }
        }
    }

    /// Cost of one frame at 1680x756, per effect (run with --release --ignored --nocapture).
    #[test]
    #[ignore]
    fn time_each_effect() {
        for k in Kind::ALL {
            let mut e = EffectBg::new(k.defaults());
            let mut d = vec![0u8; W * H * 4];
            for i in 0..5 {
                e.draw_phase(i as f64 * 0.01, &mut d, W, H, (0, 0, W as i64, H as i64));
            }
            let t = std::time::Instant::now();
            let n = 120;
            for i in 0..n {
                e.draw_phase(i as f64 * 0.01, &mut d, W, H, (0, 0, W as i64, H as i64));
            }
            println!("{}: {:.2} ms per frame", k.name(), t.elapsed().as_secs_f64() * 1000.0 / n as f64);
        }
    }
}

/// Whole-engine CPU measurement (render + encode + send to a null sink + sampler), as a share of all logical cores.
/// `cargo test --release --lib measure_live_cpu -- --ignored --nocapture` (COOLERSCREEN_FAKE_DEVICE is not needed: the null sink).
#[cfg(test)]
mod measure {
    use crate::live::pipeline::{Pipeline, Sink};
    use crate::live::{Config, Theme, ThemeLive};
    use std::time::{Duration, Instant};

    fn cpu_100ns() -> u64 {
        use windows_sys::Win32::Foundation::FILETIME;
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        unsafe { GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) };
        let f = |t: FILETIME| ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64;
        f(k) + f(u)
    }

    #[test]
    #[ignore]
    fn measure_live_cpu() {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64;
        let secs: u64 = std::env::var("CS_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
        for fps in [60u32, 30] {
            for variant in ["none", "aurora", "plasma", "drift", "clouds", "aurora+widgets"] {
                let mut cfg = Config::default();
                cfg.fps = fps;
                cfg.music.enabled = false;
                cfg.notch = "off".into();
                let p = Pipeline::start(&cfg, Sink::Null);
                let mut th = if variant.ends_with("widgets") { Theme::standard_preset() } else { Theme::default() };
                let kind = variant.split('+').next().unwrap();
                if kind != "none" {
                    th.background = Some(super::Background { kind: "effect".into(), effect: kind.into(), ..Default::default() });
                }
                p.set_theme(ThemeLive::new(th, std::path::PathBuf::new()));
                std::thread::sleep(Duration::from_millis(1500));
                let (c0, t0, s0) = (cpu_100ns(), Instant::now(), p.shared.stats.snap());
                std::thread::sleep(Duration::from_secs(secs));
                let (c1, t1, s1) = (cpu_100ns(), Instant::now(), p.shared.stats.snap());
                let wall = (t1 - t0).as_secs_f64();
                let cpu = (c1 - c0) as f64 / 1e7;
                let n = (s1.sent - s0.sent).max(1) as f64;
                println!(
                    "{fps} fps {variant:15} cpu {:.1}% of one core = {:.2}% of {cores} cores | frames/s sent {:.1} rendered {:.1} | render {:.2} ms rotate {:.2} ms encode {:.2} ms | {:.0} KB/frame q{}",
                    100.0 * cpu / wall,
                    100.0 * cpu / wall / cores,
                    n / wall,
                    (s1.rendered - s0.rendered) as f64 / wall,
                    (s1.render_us - s0.render_us) as f64 / 1000.0 / (s1.rendered - s0.rendered).max(1) as f64,
                    (s1.rotate_us - s0.rotate_us) as f64 / 1000.0 / n,
                    (s1.encode_us - s0.encode_us) as f64 / 1000.0 / (s1.encoded - s0.encoded).max(1) as f64,
                    (s1.sent_bytes - s0.sent_bytes) as f64 / 1000.0 / n,
                    s1.quality
                );
                p.stop_fast(crate::live::session::Ending::LoopClip);
            }
        }
    }
}

#[cfg(test)]
mod pipeline_tests {
    use crate::live::pipeline::{Pipeline, Sink};
    use crate::live::{Config, Theme, ThemeLive};
    use std::time::{Duration, Instant};

    fn effect_theme(effect: &str) -> ThemeLive {
        let mut th = Theme::default();
        th.background = Some(super::Background { kind: "effect".into(), effect: effect.into(), ..Default::default() });
        ThemeLive::new(th, std::path::PathBuf::new())
    }

    /// An effect theme with no widgets still streams (a plain picture theme would idle), moves every frame, and stops at once.
    #[test]
    fn an_effect_theme_streams_every_frame_and_stops_at_once() {
        let mut cfg = Config::default();
        cfg.music.enabled = false;
        cfg.notch = "off".into();
        cfg.fps = 30;
        let p = Pipeline::start(&cfg, Sink::Null);
        let th = effect_theme("aurora");
        assert!(!th.nothing_to_stream(Default::default()) && th.has_live_items());
        p.set_theme(th);
        std::thread::sleep(Duration::from_millis(1500));
        let s = p.shared.stats.snap();
        assert!(s.sent >= 20, "about 30 frames a second are sent, got {}", s.sent);
        assert_eq!(s.gradient_frames, 0);
        assert_eq!(s.unchanged_skipped, 0, "every frame of an effect is new");
        let t = Instant::now();
        p.stop();
        assert!(t.elapsed() < Duration::from_millis(300), "no hand-over fade for an effect: {:?}", t.elapsed());
    }

    /// The editor preview and the My themes card render effect themes without a decoder or a clip.
    #[test]
    fn the_editor_preview_draws_the_effect() {
        let png = crate::live::render_preview(&effect_theme("plasma"), 640, 288, true).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let live = crate::live::render_preview(&effect_theme("plasma"), 640, 288, false).unwrap();
        assert!(live.len() > 20_000, "a plasma picture is not a flat colour: {} bytes", live.len());
    }

    /// An old theme (no background field) is unchanged by the effect code: still the 3.0 behaviour and the same JSON.
    #[test]
    fn a_theme_without_a_background_is_untouched() {
        let t = Theme::from_json(r#"{"widgets":[{"type":"clock"}]}"#).unwrap();
        assert!(t.background.is_none() && t.effect().is_none());
        assert!(!t.to_json().contains("background"));
        assert!(ThemeLive::new(Theme::default(), Default::default()).theme.effect().is_none());
        // a damaged background is ignored, not an error
        let bad = Theme::from_json(r#"{"background":"oops"}"#).unwrap();
        assert!(bad.background.is_none());
        let kept = Theme::from_json(r##"{"background":{"kind":"effect","effect":"clouds","speed":0.7,"colors":["#112233","#445566"]}}"##).unwrap();
        assert_eq!(kept.effect().unwrap().speed, 0.7);
        assert_eq!(Theme::from_json(&kept.to_json()).unwrap(), kept);
    }
}

#[cfg(test)]
mod dump {
    use crate::live::pipeline::{Pipeline, Sink};
    use crate::live::{Config, Theme, ThemeLive};
    use std::time::Duration;

    /// CS_DUMP_DIR=<dir>: run each effect with the Standard widgets through the real pipeline (null sink) and keep the JPEGs
    /// exactly as they would be sent (portrait 756x1680), to look at.
    #[test]
    #[ignore]
    fn dump_pipeline_frames() {
        let Some(dir) = std::env::var_os("CS_DUMP_DIR") else { return };
        for k in super::Kind::ALL {
            let d = std::path::Path::new(&dir).join(k.name());
            let _ = std::fs::remove_dir_all(&d);
            std::env::set_var("COOLERSCREEN_DUMP_FRAMES", &d);
            let mut cfg = Config::default();
            cfg.music.enabled = false;
            cfg.notch = "off".into();
            let p = Pipeline::start(&cfg, Sink::Null);
            std::env::remove_var("COOLERSCREEN_DUMP_FRAMES");
            let mut th = Theme::standard_preset();
            th.background = Some(super::Background { kind: "effect".into(), effect: k.name().into(), ..Default::default() });
            p.set_theme(ThemeLive::new(th, Default::default()));
            std::thread::sleep(Duration::from_millis(2500));
            p.stop_fast(crate::live::session::Ending::LoopClip);
        }
    }
}
