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

## A2: the Embedded face of an effect theme

**What it does.** "Update Embedded" / "Store as Embedded" / "Use and store as Embedded" on an effect theme now records one loop of the
effect and stores it on the screen through the normal path (same job, same two steps; step 1 first says "Recording the effect").
Nothing new is ever sent to the device: the stored file is the ordinary MJPEG base, 4:2:0, portrait, from the usual conversion with the
screen fit, corners, notch, `onBase` text and pictures and the "best that fits" quality search against the flash budget. No
`0x05`, `0x0C` or `0x23` anywhere.

- **Recording** (`live/effects/record.rs`): frames at phase i/N for i in 0..N, N = loop seconds x 18 (the screen's rate), rendered at the
  2400x1080 design size and piped to ffmpeg as a high quality MJPEG intermediate in `<theme folder>/effect/loop-<key>-<N>.mkv`
  (cached; old recordings of the theme are deleted after a successful store).
- **Version**: `create_version` takes `effect: { key, file, length }` (the clip replaces the original's file as the source; start 0, length
  = the whole loop, blend 0). The version remembers `effect.key`; `run_version` converts from the recorded clip (and probes it). Library
  duplicate detection includes the effect key.
- **Fits the screen**: if the loop is longer than the auto maximum (`max_seconds`) or still too big at the lowest quality, it is recorded
  played faster (1.5x, 2x, 3x, 4x, 6x) until it fits; the live effect keeps its own speed.
- **Stale** (standing rule): `Params::key()` (settings + `RENDER_VERSION`) is compared with the stored version's `effect.key`; any effect
  change, or switching back to the picture, sets `baseStale` ("Embedded is out of date (effect/...)") exactly like a corner or fit
  change; live-only changes (widgets) do not. Bump `RENDER_VERSION` if an effect's drawing is improved.
- **Hand-over**: when the stored loop matches (`ThemeLive.effect_stored`), stopping live cross-fades to the effect at phase 0, which is the
  loop's first frame, drawn by the engine (no decoder); otherwise (no recording yet, or the effect changed) it stops at once as in A1.
- **Editor preview animates**: the editor redraws the preview about 2.5 times a second at the effect's own pace while an effect theme
  is showing and nothing is being edited (`theme_preview` takes an optional `phase`; the timer is in `themes.js`).

**Verified (A2)**
- `cargo test --release --lib -- --test-threads=4`: 242 passed. New: app-level test with the fake screen (record, store, version carries
  the key, not stale, live-only change keeps it, effect change makes it stale and `effect_stored` false, store again makes a second
  version, same effect reuses it, switching to the picture is stale, one recording kept); recording and stopping a recording; frame counts;
  preview at a phase; hand-over fade for a stored effect; `Params::key`-driven reuse.
- **Real conversion** (`cargo test --release --lib the_converted_loop_joins_up -- --ignored --nocapture`): every effect recorded at speed 2
  (90 frames), converted with the real engine and an 8 MB budget: 90 frames out, size at or under the budget (aurora 7.9 MB,
  plasma 7.5 MB, drift 2.5 MB, clouds 7.9 MB), and the step from the last frame back to the first is no bigger than the steps inside the clip
  (aurora seam 2.3 vs steps up to 2.8, plasma 3.7 vs 4.7, drift 0.4 vs 0.5, clouds 7.7 vs 7.9).
- JS checked with `node --check`; the editor was not run in the real app.

**Not verified (A2)**
- Storing an effect to the real screen (the PM does the device check); how the 18 fps stored loop looks next to the 60 fps live version on
  the panel, and the cross-fade at hand-over on hardware.
- The animated editor preview in the real app (timing, flicker); the Embedded banners in the real UI.
- Default speed 1 gives a 180-frame loop; whether the real budget makes the app speed those loops up depends on the user's screen
  measurements (the conversion loop handles it, tested only with the 8 MB case above).

## A3: five more effects (nine in all)

New (`src-tauri/src/live/effects/`, same rules: sideways or fade only, exactly periodic, settings speed/amount/size/brightness/colours,
`RENDER_VERSION` unchanged because the old effects draw as before; adding kinds changes no stored key):
- **Warp** (`warp.rs`): starfield streaming right to left; near stars thick and fast, far ones fine and slow; each crossing gets a new height.
- **Waves** (`waves.rs`): five layers of sea/hills in a sunset, each a pair of sine waves sliding sideways (far slow, near fast, alternating
  direction), haze into the sky. Colours: sky, horizon, far water, near water.
- **Lava** (`lava.rs`): metaballs that sway sideways (heights fixed) and merge and part; colours: background, rim, body, core.
- **Retro** (`retro.rs`): synthwave: striped sun, mountains, stars; the grid has fixed horizontal lines and vertical lines in perspective that
  slide sideways, one spacing per loop. Far lines fade rather than shimmer.
- **Pulse** (`pulse.rs`): the one **sound-reactive** effect. Columns of coloured light driven by the existing 64-band loopback levels
  (`audio::audio().levels()`, the visualizer's capture; it starts only while Pulse is the live background and stops itself 4 s after the last
  ask, so nothing runs in the background otherwise). Loudness changes brightness only. Without audio (editor preview, and the recording)
  it shows a periodic synthetic "music", so **the Embedded copy of Pulse is a demo loop, not reactive**; the editor text says so. Like every
  effect it is only on when the user picks it (nothing is the default). `Effect` got `wants_audio` and `audio` (default no-ops).
UI: `public/js/effects.js` only (table entries, colour swatches have names as tooltips: e.g. "Sky", "Horizon").

**CPU (A3, whole engine at 60 fps, same method as A1, busy PC):** aurora 4.1%, plasma 4.8%, drift 3.7%, clouds 4.7%, warp 3.6%,
waves 4.4%, lava 4.5%, retro 4.3%, pulse 3.4% (idle gradient 0.55%); the effect itself costs 0.3 to 2.8 ms a frame, the JPEG encode about 8 ms.
All are under 5%; none is flagged. Retro frames are larger (132 KB at q73, fine grid lines) but under the 200 KB cap.

**Real conversion of every effect** (speed 2, 90 frames, 8 MB budget, the real engine): all nine fit (retro 7.8 MB at quality 78, lava 6.6 MB,
warp 5.1 MB, waves 5.8 MB, pulse 7.6 MB) and every seam (last frame to first) is no bigger than the steps inside the clip.

**Tests:** 244 passed (every effect is opaque, not black, exactly periodic, stays in its rectangle, moves; pulse follows its levels and is the
only audio effect). Pictures of all nine looked at (`COOLERSCREEN_EFFECT_DUMP`).

**Web / scene import (not built, decision):** worth doing later only as a separate, opt-in feature. A web page needs a browser engine running
beside the app (WebView2 off-screen, a second renderer process kept alive, GPU use, no way to make the animation periodic for the Embedded
loop, and arbitrary page code to trust), which breaks "nothing hidden in the background" and the periodic rule; Wallpaper Engine scene files
need their own renderer (shaders, particle systems, assets under licence). The cheaper route to the same look is more generated effects, or
importing a Shadertoy-style shader written to our rules (sideways, periodic) once a GPU path exists. I would not start it before the user
asks for a specific wallpaper they want.

**Not verified (A3):** how any of the nine look on the real panel; the live audio path of Pulse with real sound (tested only by feeding levels
in a test; capture itself is the visualizer's existing code, which is also untested with real audio per the 3.0 notes).

