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
