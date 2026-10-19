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

## v1.4 changes (user, 2026-10-17, after trying v1.3) — supersedes the v1.2 `screenGuides` shape

User's description: the screen's curvature runs **vertically through the middle** of the screen, not along an edge.
Three guides only (guidance, never masks; never drawn on the screen except during calibration):

```json
"screenGuides": {
  "left":  { "size": 160 },
  "right": { "size": 180 },
  "curve": { "offset": 0, "left": 60, "right": 60 }
}
```
- `left.size` / `right.size`: band width in px from that edge (left = by the notch, right = hard to see e.g. RAM).
- `curve`: a vertical band; `offset` = px from the canvas centre (negative = left), `left`/`right` = px from the
  offset centre to the band's left/right edge. UI: a "Position" control for `offset`, plus ONE two-handle range
  slider for the band's edges (the part between the handles shows the width). Calibration draws all of it live.
- Old v1.2 values (curve edges, zones) are ignored/migrated harmlessly (a "right" zone → `right.size`).
- Per theme `guidesOffset`: same keys as deltas: `{ "left": {"size":0}, "right": {"size":0}, "curve": {"offset":0,"left":0,"right":0} }`.
- **Gap (per theme)**: theme.json `gap` px on the 2400x1080 canvas (default 24). Snap targets while moving/resizing
  include points at `gap` distance: inside the canvas edges, outside each guide band, and between boxes (gutter).
  Exact edge alignment stays a snap target too; Alt = free. It only adds snap points; nothing is enforced.
- UI wording rule from the user: as few words as possible. Corner radius = one number field labelled
  "Corner radius" showing the effective value, with a small reset icon button (↺) that clears the theme override;
  no radio buttons, no "px" subtext. Apply the same brevity to the guides and theme settings.

## v1.5 (user, 2026-10-17): screen fit (hidden edges) built into the app

Found with the temporary `cs2 edge-tune` tool on the user's screen: **top 4 px and left 9 px are hidden** by the
panel (right 0, bottom 0; screen px in 2400x1080 space; saved in `%APPDATA%\CoolerScreen\edge-offsets.json`). The
user: "the screen quality is not fantastic ... build this straight into the app itself so that other users could
also run it ... and fine tune".

- Settings `screenFit`: `{ "top": 0, "right": 0, "bottom": 0, "left": 0 }` (px hidden at each edge, panel coords,
  0–200). Default all 0 for new users; migrate the user's edge-offsets.json on first load if present.
- Meaning: the **visible rectangle** = panel inset by these. Every picture sent to the screen (bases made by
  conversion AND live frames) is the design canvas scaled to fit the visible rectangle (keep aspect, centre; the
  sub-pixel aspect difference is negligible), with the hidden strips black. Panel-fixed masks (notch band, corner
  radius) and the screen guides stay in PANEL coordinates and are applied after/independently (they describe the
  physical screen). The notch band is measured from the physical left edge, so it already includes the hidden 9 px.
- A change of `screenFit` makes every stored base stale (it is part of the base's cache key / "needs re-storing").
- **Calibration page** in Settings ("Screen fit"): the edge-tune experience in the app — the screen shows the ruler
  picture live (engine calibration mode, 1680x756, re-sent ~5x/s; a single live frame is NOT shown by the device),
  four rows Top/Right/Bottom/Left with − / + buttons (one sent pixel ≈ 1.4 screen px) and the value, Save/Undo. Works
  whether or not live is on (opens a temporary live session for the page, then hands back). Terse wording.

## v1.6 (user, 2026-10-18): "the widgets and live features are very lacking … expand it by all the regular ones"

Basis: research of comparable apps (NZXT CAM, L-Connect, iCUE, TRCC, AIDA64 SensorPanel, Rainmeter, Turing smart
screen, Wallpaper Engine). Panel rules still apply: no vertical motion (tears) — animate with fades or horizontal
movement; numbers large (≥ 48 px on the 2400 canvas); a missing source shows "–" greyed, never 0.

New widget types (props; unknown ignored, missing = default):
- `sensor`: one reading in one style.
  `metric`: "cpu.load" | "cpu.core" (+`core` index) | "cpu.temp" | "cpu.clock" | "gpu.load" | "gpu.temp" | "gpu.clock" |
  "gpu.vram" | "gpu.power" | "gpu.fan" | "ram.used" | "net.down" | "net.up" | "disk.used" (+`drive` "C:") |
  "disk.activity". `style`: "number" | "bar" | "ring" | "gauge" (270° arc) | "graph" (area/line history) |
  "sparkline". Options: `label` (string or null = default name, "" = none), `unit` "C"|"F" for temps, `decimals`,
  `min`/`max` (null = auto per metric), `thresholds` [{ "at": 70, "color": "#f0b43c" }, { "at": 85, "color": "#ff6b6b" }]
  (default per metric), `color` (base), `history` seconds for graph/sparkline (default 60), `rate` Hz (1|2|4).
  Network auto-scales B/s→MB/s; `adapter` null = busiest.
- `clock`: + `style` "digital" (today) | "analog" | "minimal" (huge time only); analog hands move smoothly (rotation is
  fine: no straight horizontal edges moving vertically).
- `date`: `style` "line" ("Sunday, 18 October") | "calendar" (month grid with today marked) | "big" (day number + weekday).
- `weather`: Open-Meteo (no key, non-commercial). Location from settings `weather.place` {name, lat, lon, tz} chosen by
  typing a city (Geocoding API) — never IP-based; cache 15 min; offline → last value greyed. `style` "now" (icon,
  temp, condition) | "forecast" (now + next 3–5 days or 12 h). `unit` C/F. Off by default until a place is set.
- `visualizer`: WASAPI loopback spectrum, `bands` 16–64, `style` "bars-h" (horizontal bars from the left — no vertical
  motion) | "ring" | "wave-fade"; `color`, `sensitivity`. Silent → idle flat.
- `slideshow`: `files` [relative paths in the theme folder], `seconds` per image, `fade` s, `fit`; GIF animates.
- `info`: `kind` "uptime" | "cpuName" | "gpuName" | "countdown" (+`target` ISO time, `label`) | "topProcesses"
  (+`by` "cpu"|"ram", `count` 3–5).
- Existing `stats` panel stays (it's a preset group of rows).

Engine: a shared sampler (one thread) feeding all sensor widgets with ring-buffer history; poll only metrics in use;
NVML/sysinfo/PDH as researched (docs/roadmap/2026-10-12-v3-data-sources.md). Budget: still ~1–2 % CPU with a typical
layout. App: palette grouped (Time · Sensors · Media · Info · Decoration), property panels per type with live
preview, sensible default sizes per style, "Add the Standard layout" kept. Settings: Weather place picker; °C/°F
default.

## v1.7 (user, 2026-10-18): rounded corners follow the visible rectangle

Finding: the corner radius mask was drawn at the panel's full edges while the top N px are hidden by the bezel, so each top
corner's visible part was cut (a flat curve), worse the more top fit was set. Supersedes the v1.5 sentence that said masks stay
in panel coordinates, for the corners only:
- The rounded rectangle is the **visible rectangle** = the panel inset by `screenFit` top/right/bottom/left, NOT moved by
  `shiftX`/`shiftY` (the bezel is physical), and on a band edge also after the notch band: its left edge = max(fit.left, notch
  band px). Radius as before (theme override or default), at most half the short side. Everything outside is black.
- The notch band itself stays physical, measured from the panel's own edge. Screen guides are unchanged.
- Same geometry in the live engine (`live/mask.rs` `apply_fit`), the stored base (`media` `visible_rect` and the corner PNG,
  whose cache name holds the rectangle, so a changed fit makes a new file and the base stale) and the editor's preview overlay.

## Autosave rule (user, 2026-10-18): no Save buttons

Every edit (theme, notch, screen fit, screen guides, corner radius, Gallery folder, source folders) is stored as it is made
and applies to the live picture at once; closing the window or quitting to the tray changes nothing. Reset replaces Save/Undo.
Only changes that alter the stored clip (screen fit and shift, corner radius, notch, text/pictures drawn into the base, base
clip or version) mark the base as needing to be stored again, using the full notch+fit key; live-only changes never do.

## v1.8 (user, 2026-10-18): live theme vs Embedded

- **Vocabulary.** The picture stored on the screen is **Embedded** everywhere in the UI (not base / stored loop / normal clip /
  screen copy; "base" stays in code and in `theme.json`). A theme's own picture is "Picture" in the UI. Labels: "Use this theme",
  "Store as Embedded" / "Update Embedded", "Use and store as Embedded", chips "Live" and "Embedded", "Picture only" /
  "Picture + live: ...". Tray: "Pause live (screen shows Embedded)".
- **Two roles.** The *Embedded theme* is the original named by the loop slot (`slots.standby.themeId`, else the version's owner).
  The *live theme* is what Live shows: `liveTheme` in settings.json (written only when set), else the Embedded theme (2.0 behaviour).
  `live_theme_oid()` / `embedded_theme_oid()`; a `liveTheme` naming a vanished theme falls back to Embedded; the duplicate repair
  repoints it.
- **Use this theme** (`theme_use`): sets `liveTheme` (cleared when the theme is the Embedded one) and refreshes live at once.
  Embedded is not touched. **Store as Embedded** is the old Apply job minus its third step (`theme_apply`, job kind `apply`, two
  steps: make or reuse the picture, write it to the loop slot); it does not change `liveTheme`, and clears it when it names the
  stored theme (live then follows Embedded again). With Live **off** the primary My themes button is "Use and store as Embedded":
  `theme_use` then the store job.
- **`theme_get`** returns `live`, `embedded` (this theme owns Embedded), `embeddedDiffers` (live shows it, but the screen holds
  another picture) and `embeddedTheme { originalId, name }`. `baseUnstored` is about the Embedded theme's chosen picture;
  `baseStale` is computed for any theme whose picture is the stored version. `originals_list` returns `currentTheme` (live) and
  `embeddedTheme`; each card's `theme` has `live` and `embedded`.
- **Engine.** `ThemeLive.not_embedded` = the theme's base version is not the one in the slot. Then (a) `nothing_to_stream` is false:
  live streams the theme's own picture instead of idling on the loop; (b) when live stops (quit, pause, sleep, lock) the hand-over
  is **instant** (no fade, no first-frame cross-fade; "screen off" endings are unchanged). Same theme: the existing hand-over.
- **On screen** edits the live theme. When Embedded differs it shows a terse bar "Embedded: <name>" with "Store this as Embedded";
  stale: "Embedded is out of date (screen fit/corners/notch/text/pictures changed) · Update Embedded". With Live off the title
  says "Live is off · the screen shows Embedded".
- The "App responsibilities" Apply description above (3 steps) is superseded by this section.

## v1.9 (4.1.0, stream A1): live effect backgrounds

A theme's background can be a **live effect** the engine draws itself instead of the theme's picture (like Wallpaper Engine's
generated scenes, not a video). New optional theme field `background`; missing, `null` or any `kind` other than `"effect"` = the
picture exactly as in 3.0, so old themes load and look the same and are written back without the field.

```json
"background": { "kind": "effect", "effect": "aurora", "speed": 1, "density": 0.5, "scale": 0.5, "brightness": 0.85,
                "colors": ["#2cf5a0", "#3a7bff", "#b24bff"], "seed": 1 }
```
- `effect`: `"aurora"` (rays of light drifting sideways under stars), `"plasma"` (warped colour bands sliding past each other),
  `"drift"` (soft particles that drift sideways and fade), `"clouds"` (slow fractal clouds in a colour ramp). Unknown = the
  background is ignored (picture).
- Every setting is optional; a missing one takes the effect's default (listed in `src-tauri/src/live/effects/mod.rs`, mirrored in
  `public/js/effects.js`). `speed` 0.1 to 3 (1 = one loop in 10 s), `density` 0 to 1 (rays / bands / particles / cloud cover),
  `scale` 0 to 1 (feature size), `brightness` 0.1 to 1.5, `colors` 2 to 4 `#rrggbb` (a ramp; fewer than 2 = defaults), `seed` integer.
  Values outside the range are clamped on load.
- **Periodic by design:** an effect is a function of the settings and the loop phase `phi = seconds * speed / 10`, with period 1 in
  `phi`. A recording of `10 / speed` seconds loops seamlessly (used by the Embedded face, A2). Motion is sideways or a fade only (the
  panel tears on vertical motion).
- **Engine:** the effect fills the visible rectangle (screen fit) directly; no picture decoder runs. Widgets draw over it as over any
  background; `onBase` items are drawn live (there is no base picture under them). With an effect the live layer always streams
  (`nothing_to_stream` is false) and when live stops the hand-over is instant (until the effect is recorded as the Embedded loop).
- The editor's Theme settings has a "Background" section (Picture | Live effect, effect, speed, amount, size, brightness, colours,
  Reset). Edits save and apply at once. The stored picture is not touched; effect changes are live-only until A2.

## v1.10 (4.2.0, stream B1): more widgets

Additive: six new widget `type`s. Old themes load and look exactly as before; unknown props are ignored, missing = default; the
app cleans props on `theme_set` (`live/morewidgets.rs` `clean`). A missing source shows a grey dash, never 0. Bars grow sideways;
nothing moves up and down. Code: `src-tauri/src/live/morewidgets.rs` (props, cleaning, timer, time zones), `render/more.rs`
(drawing), `public/js/morewidgets.js` (palette and property panels).

- `meters`: your own list of readings, each a name, a big value and a bar. `rows` (1-8, default CPU load, RAM, GPU load, GPU
  temperature): `{ "metric": <any sensor metric>, "label": string|null, "core": 0, "drive": "C:" }`; `unit` "C"|"F"; `bars` bool (true);
  `rate` 1|2|4 Hz (1).
- `cores`: every logical core. `style` "bars"|"heat" (bars), `average` bool (true: the average load as a big number), `color`
  "#rrggbb"|null (null = green / amber / red by load), `rate` (2).
- `network`: download and upload together: two big numbers and one graph with both lines. `graph` bool (true), `history` 10-600 s
  (60), `adapter` string|null (null = the busiest), `downColor` ("#5ad2a0"), `upColor` ("#aa8cff"), `rate` (2).
- `disks`: fixed drives with a used bar. `drives` ["C:", ...] (empty = every fixed drive), `show` "used" ("412 GB / 931 GB") |
  "free", `maxRows` 2-6 (4). Bars turn amber at 85 % and red at 95 %.
- `worldclock`: 1-4 cities. `zones` `[{ "name": "Tokyo", "tz": "Tokyo Standard Time" }]` where `tz` is a Windows time-zone id
  (empty = this PC's own zone; an unknown id shows a dash); `format` "24h"|"12h"; `color`. Rules come from the Windows registry
  (no network). A row shows "Tomorrow" or "Yesterday" when its date differs from this PC's.
- `timer`: `mode` "stopwatch"|"timer"|"pomodoro"; `label`; `running` bool; `startedAt` (ms since 1970, the start of the current
  run); `elapsedMs` (time collected before this run); `duration` seconds (timer, 300); `workMin` 25 / `breakMin` 5 (pomodoro);
  `color`. The editor's Start / Pause / Reset buttons write these props (saved at once); the engine computes the display from the
  PC clock, so a running timer survives restarts. A finished timer pulses its opacity.

### 4.0 widgets, stream B2 (added 2026-10-19; contract version assigned by the PM)

More types and metrics, all additive (code: `live/extsensors.rs`, `live/ping.rs`, `live/agenda.rs`, `live/volume.rs`, `live/morewidgets.rs`,
`render/more.rs`, `public/js/morewidgets.js`).

- New sensor metrics (usable in `sensor` and in `meters` rows): `cpu.power` (W), `cpu.fan` (RPM), `board.temp`, `disk.read` and
  `disk.write` (bytes/s). `cpu.temp` (reserved since 1.6, always a dash until now) now reads a tool the user already runs:
  HWiNFO shared memory (`Global\HWiNFO_SENS_SM2`, "Shared Memory Support" on) or LibreHardwareMonitor's web server
  (`http://127.0.0.1:8085/data.json`). CoolerScreen never installs, starts or elevates either; with neither running the readings stay a
  grey dash, and the editor says so (`ext_sensor_status` command: `{ source, ok, message }`). The same four tool-backed metrics
  (`cpu.temp`, `cpu.power`, `cpu.fan`, `board.temp`) are the only ones affected.
- `network` gets `totals` bool (false): a footer with the data moved since start-up (adapter counters).
- `livetext`: the `text` widget's props (`text`, `size`, `color`, `opacity`, `align`, `weight`) with `{words}` replaced by live values:
  time, time12, seconds, date, weekday, cpu, cputemp, cpupower, cpufan, gpu, gputemp, vram, ram, down, up, diskread, diskwrite, uptime,
  volume. An unknown word stays as typed; a missing value shows a dash. Default text `{time}  CPU {cpu}  GPU {gpu}`.
- `volume`: `style` "bar"|"ring", `color`. The default playback device's master volume (read only; "Muted" when muted).
- `ping`: `host` (default "1.1.1.1"; letters, digits, dots, dashes), `style` "number"|"graph", `history` 10-600 s. One ICMP echo per
  second to that host while the item is shown (at most 3 hosts at once); green under 60 ms, amber to 120, red above or "Timeout".
- `agenda`: `source` (an https:// or webcal:// calendar link, or the path of an .ics file; empty = nothing chosen), `count` 2-6 (4),
  `allDay` bool (true), `color`. Link fetched every 15 minutes (file re-read when it changes) only while shown. Recurrence:
  DAILY / WEEKLY(BYDAY) / MONTHLY / YEARLY with INTERVAL, COUNT, UNTIL, EXDATE. The link is stored in theme.json (it may be a secret
  address: a future theme-sharing export should drop `agenda.source`).
