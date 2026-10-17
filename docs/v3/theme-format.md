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

## v1.1 additions (user feedback 2026-10-17, after trying the first build)

- `liveLayer` (theme, bool, default true): the editor's single "Live layer" switch. Off = base only: while live is on
  and this theme is current, the app stops streaming (the screen plays its own base); preview shows the base only.
- Layers: the `widgets` array order IS the drawing order (first = bottom, last = top). New widget fields:
  `hidden` (bool, default false: not drawn live, not baked), `group` (string id or null).
  New theme field `groups`: `[{ "id": "g1", "name": "Group 1", "hidden": false, "collapsed": false }]`. Members of a
  group are contiguous in `widgets`; a hidden group hides its members. The engine only needs order + hidden (+ group
  hidden); grouping is otherwise an editor concept (move/select together).
- Base from any clip: `baseVersionId` may point to a version of ANY original in the library (not only the theme's
  own). The engine's fallback/background uses that version's original. Changing it does not send anything to the
  screen; the editor then shows "Base changed — store it on the screen" (the Apply flow).
- Hand-over is no longer a view of the editor; it lives in the theme's own settings (right panel when nothing is
  selected, or a "Theme settings" section). Same `handover` fields.
- Live start must never show a placeholder/gradient when the theme has a base or a clip: wait for the first real
  background frame before the first live frame (fixes the "flash" when turning live on).

## v1.2 additions (user, 2026-10-17): snapping and screen guides

Context from the user: the physical screen is **curved**, so content near the curve is seen at a different angle,
and in their case the **right edge is partly hidden behind the RAM** (colours still visible at an angle). These are
**guides, not masks**: nothing is darkened on the screen (unlike the notch black band).

- **Snapping (editor):** gentle snap of moving/resizing boxes to other boxes' edges and centres, canvas centre/edges,
  guide-zone edges, and matching width/height of other boxes. Weak threshold (≈6 px at preview scale), Alt held =
  no snap for that move. Toggle in the editor toolbar; default from settings `editor.snap` (default true).
- **Screen guides (global, settings.json `screenGuides`, coordinates in px on the landscape 2400x1080 canvas):**
  `{ "curve": { "left": 0, "right": 0, "top": 0, "bottom": 0 },
     "zones": [ { "id": "z1", "name": "RAM", "edge": "right"|"left"|"top"|"bottom", "size": 180 } ] }`
  `curve.<edge>` = how far in from that edge the curve starts (0 = flat). `zones` = "avoid" bands along an edge.
- **Per-theme offset** (theme.json `guidesOffset`, same shape, values added to the global ones, may be negative):
  `{ "curve": { "right": -20 }, "zones": { "z1": 30 } }`.
- **Display:** hatched, tinted overlays in the editor preview (live layer) and in Base settings; never drawn on the real
  screen except during calibration. Widgets overlapping a zone get a warning badge; zone edges are snap targets.
- **Calibration (settings "Screen guides"):** sliders per curve edge and per zone (add/rename/remove zones). While
  live is on, the engine draws the guide lines and labels on the REAL screen in real time
  (`Pipeline::set_calibration(Option<Guides>)` — draws over everything, temporary, cleared when leaving the page), so
  the user can line them up with the physical curve and the RAM. Save = global defaults.

## v1.3 additions (user, 2026-10-17): corner radius per theme, settings clean-up

- **Corner radius:** settings `mask.corners.r` becomes the DEFAULT radius in px on the 2400x1080 canvas, where
  **0 = no rounded corners** (replaces the separate on/off; migrate: `on:false` → 0, `on:true` → its r). Per theme:
  theme.json `corners`: null (= use default) or a number (0 = none for this theme). Applied to the base when it is made
  (as today's corner mask) and drawn by the engine on the live frames (with the notch mask). Remove the per-clip
  corners toggle in Tune; the theme setting replaces it. A changed radius marks the base as needing re-storing.
- **Remove "Longest clip (seconds)"** (`maxSeconds`) from Settings: the maximum loop/trim length is derived
  automatically from what fits on the screen (the loop finder's existing "auto"). Keep reading the old value harmlessly.
- Smoothness stays 30/60 fps (60 = default and max; the panel is very likely 60 Hz).
