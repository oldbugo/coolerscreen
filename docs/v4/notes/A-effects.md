# Stream A: live effects (notes)

Branch `claude/determined-greider-88a800`. Written by the stream A session; the PM turns this into the changelog.

## A1: effect renderer and four generated backgrounds

**What it is.** A theme can have `background: { kind: "effect", effect, ... }` (contract section "v4 effects" in
`docs/v3/theme-format.md`). The engine draws the effect itself into the frame (no ffmpeg, no video); widgets draw over it as usual.
Without the field a theme is exactly as in 3.0 (the field is not written back).

Code: `src-tauri/src/live/effects/` (new module: `mod.rs` settings + trait + tests + CPU measurement, `util.rs`, `aurora.rs`,
`plasma.rs`, `drift.rs`, `clouds.rs`). Small hooks: `theme.rs` (field, `effect()`, `instant_handover()`), `render.rs`
(`sync_effect`, `draw_effect`, `set_effect_phase`), `pipeline.rs` (no decoder for effect themes, instant hand-over),
`preview.rs` (editor preview draws the effect at phase 0.3). UI: `public/js/effects.js` + 3 lines in `public/js/themes.js`
(Background section in Theme settings; the base-only preview signature includes the background).

**Effects** (all: sideways motion or fades only, periodic in the loop phase, 2 to 4 colours, speed, brightness):
- Aurora: vertical rays under stars, three layers drifting left and right. Settings: Rays, Glow height.
- Plasma: warped colour bands sliding past each other. Settings: Bands, Size.
- Drift: soft particles/bokeh that fade up and down while drifting right; every particle lives a whole number of times per loop.
  Settings: Amount, Size.
- Clouds: fractal clouds, two layers sliding in opposite directions plus a fading third. Settings: Cover, Size.

**Choices made (the user is asleep):**
- CPU on one thread, written into the frame directly (no GPU): measured, see below. Effects fill the visible rectangle of the
  screen fit directly, so the fit costs nothing extra.
- Loop = 10 s / speed. Clouds default to speed 0.5 (20 s loop). Everything periodic so A2 can record a seamless loop.
- With an effect: the live layer always streams, hand-over at stop is instant (the Embedded loop is whatever was stored; A2 will make
  the hand-over a cross-fade onto the recorded loop's first frame).
- `onBase` widget items are drawn live over an effect (there is no base picture under them).
- Effect settings are live-only: they do not mark Embedded as stale yet (A2).
- Default: no theme has an effect; nothing turns on by itself.

## A1 measurements (this PC, 16 logical cores, Windows, other build sessions running at the same time, so treat as upper bounds)

Whole live engine (render + rotate + JPEG encode + null sink + sampler thread), 8 s per row, as a share of all cores
(`cargo test --release --lib measure_live_cpu -- --ignored --nocapture`, process CPU time from GetProcessTimes):

| | 60 fps | 30 fps | render per frame | encode per frame | JPEG size |
|---|---|---|---|---|---|
| plain gradient (3.0 default, frames mostly skipped) | 0.5% | 0.6% | 0.02 ms | 7.2 ms | 26 KB |
| aurora | 3.5% | 1.8% | 1.0 ms | 7.9 ms | 90 KB |
| plasma | 4.1% | 2.3% | 2.5 ms | 8.0 ms | 127 KB |
| drift | 3.3% | 1.8% | 0.5 ms | 7.3 ms | 39 KB |
| clouds | 4.2% | 2.7% | 2.4 ms | 8.0 ms | 109 KB |
| aurora + Standard widgets | 4.4% | 2.6% | 3.4 ms | 8.0 ms | 122 KB |

All within the 5% budget at 60 fps; no effect needs to be flagged. The JPEG encode (about 8 ms a frame, existing code) is now two
thirds of the cost: the effect itself is 1 to 2.5 ms. Frames stay under the 200 KB cap at quality 90 to 92.

How it got there: the first versions took about 5 ms a frame per field effect (float loops). The crate builds with `opt-level = "s"`,
where LLVM does not vectorise (even a trivial byte loop ran at 0.7 ns a byte; `opt-level=3` or `target-cpu=native` barely helped the
float loops), so the hot loops (aurora row mix, bilinear upscale) are written with SSE2 intrinsics (always present on x86_64; other
targets use plain loops). That made them about 5 times faster without touching the build profile.

## A1 verification

- `cargo test --release --lib -- --test-threads=4`: 227 passed (new: fast sine, hash, palette, background parsing and defaults,
  every effect opaque and not black, **every effect repeats exactly after one loop (phi and phi+1 differ by at most 2/255)**, effects
  stay inside a shifted rectangle, effect themes stream every frame with no gradient frames and stop in under 300 ms, old themes
  unchanged and round-trip without the field, the editor preview draws the effect).
- Pictures: `COOLERSCREEN_EFFECT_DUMP=<dir> cargo test --release --lib dump_frames_when_asked` writes landscape PNGs of every effect;
  `CS_DUMP_DIR=<dir> cargo test --release --lib dump_pipeline_frames -- --ignored` runs each effect with the Standard widgets
  through the real pipeline (null sink) and keeps the JPEGs exactly as sent (portrait 756x1680). Looked at: aurora, plasma, drift, clouds.
- Editor controls: the Background section was exercised in a stand-in page (the real editor needs Tauri): Picture/Live effect switch,
  effect picker, sliders, colours, loop length, edits reported. **Not tried inside the real app window.**

## A1 not verified

- The editor section inside the running app (theme_set / preview round trip) and My themes cards for effect themes.
- Anything on the real screen: how the effects look on the panel, whether 60 fps is smooth, banding on the real LCD.
- Behaviour with a screen fit and shift on live frames beyond the unit test (effects fill the visible rectangle; tested with a
  shifted rectangle).
- The preview in the editor shows one fixed moment (phase 0.3), it does not animate.
