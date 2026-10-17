# CoolerScreen 3.0: theme format and responsibilities (contract)

Owner: the orchestrator. Status: v1, 2026-10-17. Engine (`src-tauri/src/live/`) and app (`core/`, `public/`, …)
both build to this; propose changes in your report instead of diverging.

Product decisions behind it: `docs/roadmap/2026-10-12-v2-release-and-v3.md`, sections "Direction change: themes"
and "Design decisions from the first mock-up review" (and the user's 2026-10-17 follow-up below). Mock-ups: private
design canvas "CoolerScreen 3.0 themes".

## Concept

Every library item (an "original": clip, Wallpaper Engine import, .ht, still image) **is a theme**. A theme is one
thing: a **base** (the loop stored on the screen — the existing MJPEG "version" in the loop slot) plus an optional
**live layer** (widgets drawn by the engine on top of the same base while CoolerScreen runs), plus per-theme
**hand-over** options. A theme without widgets is "base only".

## File: `<library>/originals/<originalId>/theme.json` (missing = defaults below)

```json
{
  "v": 1,
  "baseVersionId": "<versionId or null>",
  "widgets": [
    { "id": "w1", "type": "clock", "x": 0.08, "y": 0.18, "w": 0.48, "h": 0.42, "onBase": false,
      "props": { "format": "24h", "seconds": true, "date": true } }
  ],
  "handover": { "fadeIn": 0.8, "end": "loopStart", "musicIdle": "hide" }
}
```

- `baseVersionId`: the version that is (or will be) stored on the screen as this theme's base; null until first
  applied. Apply creates/reuses a version with the app's current quality settings.
- Coordinates: fractions (0–1) of the **landscape 2400x1080 canvas**, box top-left `x,y`, size `w,h`. The notch band is
  drawn on top by the engine (global setting), editors should keep widgets clear of it but nothing enforces it.
- `onBase` (text and image only): also baked into the base recording. Live-data widgets are always live-only.
- Widget `type` and `props` (unknown props ignored, missing = default):
  - `clock`: `format` "24h"|"12h", `seconds` bool (false), `date` bool (true), `color` "#rrggbb" (white).
  - `music`: `cover` bool (true), `progress` bool (true). Hidden when nothing plays (see `handover.musicIdle`).
  - `stats`: `items` array of "cpu","ram","gpuLoad","gpuTemp","gpuClock","cpuTemp" (all but cpuTemp by default).
  - `text`: `text` string, `size` 0.02–0.3 (fraction of canvas height, default 0.08), `color` "#rrggbb", `opacity` 0–1,
    `align` "left"|"center"|"right", `weight` "light"|"regular"|"bold".
  - `image`: `file` (path relative to the theme folder, e.g. "assets/logo.png"), `opacity` 0–1, `fit` "contain"|"cover".
  - `weather`: reserved, not built yet (engine draws a placeholder label "Weather").
  Each widget draws itself inside its box, scaled to the box (the box defines size).
- `handover`: `fadeIn` seconds (0–3) for the live layer when live starts; `end` "loopStart" (fade the live layer out,
  then bring the background to the stored loop's first frame and stop, so the screen continues seamlessly) |
  "instant" | "screenOff" (fade to black, then screen-off ending); `musicIdle` "hide"|"keep".
- **Default theme** (no theme.json): `widgets: []`, base only, `handover` as above.
- **Preset "Standard"** (button in the editor): clock top-left, music under it, stats on the right — today's layout.

## Engine responsibilities (`src-tauri/src/live/`)

- `Pipeline::set_theme(ThemeLive)` at runtime (no restart): widgets + handover + background source. Renderer draws
  widgets generically from the list (replacing the fixed layout; keep today's visual style for clock/music/stats).
- Background while live = **the theme's base**: decode the base version file (portrait MJPEG, `transpose=1` of
  landscape) back to landscape, at its own frame rate, looping, so the live background is the same loop the screen
  plays on its own. Fall back to the original clip, then a plain gradient, if no version exists.
- Hand-over: fade the live layer in over `fadeIn` on session start. On stop with `end=loopStart`: fade widgets out
  (~0.5 s) while cross-fading the background to the loop's first frame (≤0.5 s), send that frame, then the LoopClip
  ending (0x14 [1,0] restarts the loop from its first frame). Must finish within ~1.2 s. `stop_fast` (shutdown) skips
  fades. `end=screenOff` uses the ScreenOff ending.
- `render_preview(theme, size, with_live: bool) -> RGBA/PNG` for the app's editor preview (one frame, no device).
- `render_base_overlay(theme) -> Option<PNG 2400x1080 RGBA>`: the `onBase` items only, for baking into the base.

## App responsibilities (`core/`, `public/`, `live_app.rs`, …)

- Load/save theme.json per original (commands `theme_get {originalId}`, `theme_set {originalId, theme}`), validate.
- "Current theme" = the original whose base is in the loop slot (already tracked as "on screen now"); live uses it.
- **Apply** (from My themes): locked modal with steps and progress — (1) make/reuse the base version (bake
  `render_base_overlay` into it if any `onBase` items, via ffmpeg overlay), (2) send it to the loop slot (existing send
  job; live pauses and resumes), (3) switch the live layer (`set_theme`). Only button: "Stop and keep the previous
  theme" (cancel; previous base stays). Everything else in the window is blocked.
- Browse: adding creates an independent copy in My themes (no link back; adding again creates "Name (2)"). Remove the
  "already added" linkage in the UI.
- UI (see mock-ups; keep the current app's top bar style):
  - Top bar: CoolerScreen · nav **On screen | My themes | Browse** · right side: ONE combined indicator = screen
    status + Live on/off switch · settings gear.
  - **On screen** = the theme editor for the current theme, preview taking most of the space: widget palette on the
    left (drag onto the preview, or click to add), properties of the selected widget on the right, widgets movable and
    resizable on the preview. View options inside the editor: "With live layer" / "Base only" / "Hand-over" settings.
    Edits apply to the live layer immediately (`set_theme`) and save to theme.json; edits that change the base
    (`onBase` items) show "Base needs re-storing" with a button that runs the Apply flow.
  - **My themes** = the library grid (cards show base, or base + live layer; label "Base only" / "Base + live: …").
    Select → Apply. Add from file stays.
  - **Browse** = today's sources view, separated, add = independent copy.
  - Settings keeps global things (fps 30/60, start with Windows, sleep/lock/shutdown, notch, screen tools).
- Defaults stay 2.0-like: live off until the user turns it on.
