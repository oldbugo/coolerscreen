# Changelog

## 4.13.0 - 2026-10-19 (stream C: restart)

- **Restart CoolerScreen** button at the top of Settings: quits like tray Quit (the screen goes back to Embedded) and
  opens a fresh copy. It refuses while something is still running.
- **`CoolerScreen.exe --restart` and `--update <installer>`:** restart the app, or install an update and reopen it,
  started by the app itself. Updates no longer leave the app closed.

## 4.12.0 - 2026-10-19 (stream C: live editor preview, smooth dragging)

- **Digital twin:** the On screen editor shows the live picture as it is on the screen (music, processes, sensors and
  effects move), about 15 frames a second while the editor is visible. With Live off it is drawn for the editor only.
- **Smooth dragging:** items follow the mouse every frame; the screen catches up just after.

## 4.11.0 - 2026-10-19 (stream B: corner radius per item, notch-aware gaps)

- **Corner radius on every widget:** Properties has a Corner radius row. By default an item follows the theme's corner
  radius; set your own value per item, and the reset arrow returns it to the theme value.
- **Gaps start at the notch:** the gap, snap lines, default positions and the Standard layout are measured from the
  visible area (after the notch band and the screen fit), so nothing is placed under the black band.
- Theme contract v1.13.

## 4.10.0 - 2026-10-19 (stream D: editor layout)

- **Resizable panels:** drag the inner edge of the left or right panel; the width follows, then settles on the nearest
  preset (left 220/300/380 px, right 280/360/480 px) with a small spring at the limits. Sizes are remembered.
- **Layers on the left:** Layers moved from the bottom into the left panel under the palette, with a divider to share the
  height; the preview gets the space back.
- **Palette accordions:** each group folds open and closed (remembered); items line up the same in every group.
- **Deselect by clicking away:** a click outside the selected item and its Properties deselects it (moving the window
  keeps it; Esc deselects).
- **Properties:** "Position and size" is gone (place items on the preview; arrow keys still nudge); "Delete this item"
  is always at the bottom right.

## 4.9.2 - 2026-10-19 (stream C: Embedded size alignment)

- **One size limit everywhere:** the Picture page, "Preparing the picture" and the final check before sending now use
  the same number: the smaller of the measured capacity and the free space the screen itself reports. A picture that
  "fits" can no longer be refused at the last step. Each Store as Embedded reads the screen's free space first.
- A ready picture that no longer fits is made again at "best that fits" instead of being reused.
- The Picture page says when the screen's own free space is the limit.
- The "screen changed outside the app" check now gets a reading at every store (before, with Live on, it never did).
- `--quit` answers through system events only (no result file).

## 4.9.1 - 2026-10-19 (stream C: clean quit for updates)

- **`CoolerScreen.exe --quit`:** quits the running app exactly like tray Quit (live hands back to Embedded). It refuses
  while an upload, conversion or store is running, never opens a window and never starts the app. Updates now use it
  instead of force-closing the app.
- **Test copies can no longer reach your app:** a development or test copy of CoolerScreen (debug build, or its own data
  folder) can no longer hand its launch to your installed app and bring its window up. Opening CoolerScreen from the
  Start menu still brings up the running window.

## 4.9.0 - 2026-10-19 (stream A: faster Embedded stores)

- **No second send of the same clip:** if the exact clip is already on the screen (same content, and a fresh check of
  the screen agrees), storing finishes at once with "Already on the screen. Nothing was sent." When anything is
  uncertain it sends as before.
- **Why a store takes about 3.5 minutes:** 96% of the time is the screen writing its flash at about 50 KB/s (a 10.6 MB
  clip takes about 210 s). Converting takes a few seconds. Live cannot keep showing during the send: the screen takes
  one stream at a time and the file write occupies it. Details in `docs/v4/notes/A-store-speed.md`.

## 4.8.0 - 2026-10-19 (stream C: preview on the screen)

- **Preview by selecting:** clicking a theme in My themes shows it on the real screen at once, with its live layer;
  clicking a Browse item shows its picture. Leaving the page, clearing the selection, closing the window or two
  minutes without activity puts back what was showing before. Nothing is saved by a preview.
- **Works with Live off too:** a temporary preview stream starts and the screen returns to Embedded afterwards; the
  Live setting is not changed.
- **Use this theme** makes it the live theme and takes you to On screen. "Store as Embedded" and "Use and store as
  Embedded" are no longer in My themes: storing happens from On screen.
- Browse previews show the item's picture only; videos and scenes are not played before they are added.

## 4.7.1 - 2026-10-19 (stream D: UI polish)

- The new widget panels and the effects Background section now use the same switches, buttons and fields as the rest
  of the app, and every widget kind has its own icon in the palette and the layers list.

## 4.7.0 - 2026-10-19 (stream A: five more effects)

- **New live effects (nine in all):** **Warp** (a starfield streaming sideways), **Waves** (layers of sea and hills at
  sunset), **Lava** (a lava lamp), **Retro** (a synthwave sun, mountains and sliding grid) and **Pulse**, which
  reacts to the sound your PC plays. Pulse listens only while it is the live background; its Embedded copy is a
  demo loop, not reactive.
- Each effect costs about 3.4 to 4.8% of the PC's CPU at 60 fps (measured on a busy PC), and all of them fit the
  screen's storage as Embedded loops.

## 4.6.0 - 2026-10-19 (stream D: UI revision)

The look is tidied up everywhere; every flow, button action and setting works as before.
- **One look per control:** one switch for every on/off setting (top-bar Live, Live layer, Snap, Guides, Properties,
  Settings), one button set, one segmented control, one set of line icons instead of mixed symbols and emoji.
- **Editor:** the preview fills the window instead of leaving an empty band; the top bar fits the smallest window.
- **Pages:** no empty strip at the bottom; Settings has a section index under its title; Properties uses one field
  layout; the Picture page now looks like the rest of the app.
- **Under the hood:** colours, sizes and corner radii come from one set of design tokens (`public/css/tokens.css`).

## 4.5.0 - 2026-10-19 (stream B: more widgets, second batch)

- **CPU temperature** (also CPU power, CPU fan and board temperature): read from HWiNFO (Shared Memory Support on) or
  LibreHardwareMonitor (Remote Web Server on) if you already run one. CoolerScreen never installs, starts or elevates
  them; without one the value stays a dash and the editor says how to turn a source on. Not yet tried with a real
  HWiNFO or LibreHardwareMonitor.
- **Disk read / write speed** as sensor and meter readings; **network totals** since start-up (a switch on Down + up).
- **Live text:** the Text widget can show live words like {time}, {cpu}, {cputemp} or {down}.
- **Volume:** the master volume level, with "Muted" when muted (read only).
- **Ping:** one ping a second to a host you type (default 1.1.1.1), as a number or graph; only while the widget is shown.
- **Agenda:** the next events from a calendar link (https/webcal) or an .ics file, with repeats and "Now".
- Theme contract v1.12.

## 4.4.0 - 2026-10-19 (stream A: live effects, Embedded face)

- **Effect themes can be stored as Embedded:** "Store as Embedded", "Update Embedded" and "Use and store as Embedded"
  now record one seamless loop of the effect and store it on the screen as a normal clip (MJPEG, with the screen fit,
  corners and notch). If the loop is too big for the screen it is recorded played faster until it fits.
- **Embedded goes out of date** when an effect setting changes, like a corner or fit change; widget changes do not.
- **Hand-over:** when live stops and the stored loop matches, the screen fades into the loop's first frame.
- **The editor preview animates** for effect themes.
- Theme contract v1.11.

## 4.3.0 - 2026-10-19 (stream C: small improvements, second batch)

- **Screen changed outside the app:** after a check, if the screen's free space is clearly different from when
  CoolerScreen last stored something, a banner says another program may have written to it, with an "It's fine" button.
  Storing again clears it. Read-only: nothing extra is sent to the screen.

## 4.2.0 - 2026-10-19 (stream B: more widgets, first batch)

Six new widgets, added from the editor palette (World clocks and Timer under Time, the rest under Sensors):
- **Meters:** your own list of 1 to 8 readings (any sensor), each with a big value and a coloured bar.
- **CPU cores:** every core as bars or a heat grid, with the average load as a big number.
- **Down + up:** download and upload speeds together, with one two-colour graph.
- **All disks:** every drive's used space as bars ("used / size" or "free").
- **World clocks:** 1 to 4 cities, 24h or 12h, with a Tomorrow / Yesterday tag; daylight saving from Windows, no network.
- **Timer:** stopwatch, countdown or Pomodoro, started and paused from the editor; keeps time with the window closed.

Existing widgets and themes are unchanged. Theme contract v1.10.

## 4.1.0 - 2026-10-19 (stream A: live effects, first milestone)

- **Live effect backgrounds:** a theme's background can now be a live effect drawn by the app instead of a video:
  **Aurora**, **Plasma**, **Drift** (soft particles) and **Clouds**. Each has colours, speed, brightness and two
  effect settings, chosen in the editor under Theme settings > Background. Effects move sideways or fade (never up and
  down, which tears on this panel) and repeat seamlessly, ready for recording an Embedded loop later.
- **Cost:** about 3.3 to 4.2% of the PC's CPU at 60 fps with an effect (about 2% at 30 fps), measured on this PC.
- Nothing changes for existing themes; no theme gets an effect by itself. Theme contract v1.9.
- Not yet: effect themes do not record their Embedded loop (next effects milestone); when live stops the screen
  switches to whatever Embedded loop is stored.
- **Design documents (stream D):** UI audit with screenshots and a style guide (`docs/v4/`); no app change.

## 4.0.0 - 2026-10-19 (stream C: small improvements, first batch)

- **Frame rate counter:** Settings > Live screen shows the frames per second actually sent ("59.8 of 60"). A new switch
  (off by default) draws the frame rate on the screen. A still picture shows a low number on purpose: the app only
  re-sends a few frames per second when nothing changes.
- **GPU power and fan fixed:** they could never be read (a broken lookup name). NVML is now retried if it was missing at
  start, and device.log says why a GPU value is unavailable.
- **Countdown and daylight saving:** the countdown info widget counts real time across a clock change.
- **Clean build number:** generated build files are no longer tracked, so an installed clean build shows no "+".
- **Reliable tests:** the engine timing tests wait for the state instead of fixed sleeps; they pass at default threads.

## 3.0.3 - 2026-10-18 (final 3.0)

- **Embedded:** the picture stored on the screen is now called **Embedded** everywhere. A theme's background before it is
  stored is its **Picture**.
- **Use this theme:** choosing a theme in My themes switches the live picture at once and leaves Embedded alone;
  **Store as Embedded** is separate. With Live off the one button is **Use and store as Embedded**. Cards show **Live**
  and **Embedded**; the editor follows the live theme and says "Embedded: …" when they differ, or "Live is off" when it
  is off. A live theme that differs from Embedded hands over instantly when live stops.
- **Fixes from the user-flows audit:** a stray "null" in the My themes bar, leftover Save wording, About text, shorter
  texts, toasts at the top, wider editor preview, Browse empty state.
- **Quiet installs and tests:** `--background` start (tray if Live is on, otherwise no window, never takes focus); test
  copies run hidden (`COOLERSCREEN_TEST_QUIET=1`), so nothing interrupts a fullscreen app.
- **Fixed:** after a background start the live picture ignored the screen fit (corners looked cut off); changing a
  theme's corner radius changed the global default.

## 3.0.0 - 2026-10-18

CoolerScreen now draws on the screen while it runs, if you want it to. The 2.0 behaviour is still the default: nothing
runs in the background until you turn Live on, and the screen keeps playing its stored loop when the app is closed.
Your clips, versions and settings are used as they are.

- **Themes:** everything on the screen is a theme: a **base** (the loop stored on the screen) plus an optional **live
  layer** drawn on top by the app. One theme, two faces: with the app running you see base + live layer; without it the
  screen plays the same base. A plain clip or picture is just a theme with no live layer. Pages: **On screen** (the
  theme editor), **My themes** (apply, base settings) and **Browse** (adding makes an independent copy).
- **On screen editor:** drag items from a palette onto the preview or click them; move, resize, set details on the right.
  Layers list like a paint program: reorder by dragging, multi-select, group / ungroup (Ctrl+G, Ctrl+Shift+G), forward /
  back (Ctrl+] / Ctrl+[), hide with the eye, rename, delete. Boxes **snap** to each other, the middle and the guides,
  with an optional equal-gap snap (hold Alt to move freely). A Live layer switch turns all items on or off. Preview as
  **With live layer**, **Base only** or **Hand-over**. Any clip can be a theme's base.
- **Widgets:** clock (digital, analog, minimal), date (line, calendar, big), sensor in six styles (number, bar, ring,
  gauge, graph, sparkline) for CPU / GPU / RAM / network / disk readings, stats panel, weather (Open-Meteo, no key; you
  pick the city; cached; greyed when offline), audio visualizer (bars, ring, wave), picture slideshow, info widgets
  (uptime, CPU and GPU name, countdown, top processes), now-playing music (title, artist, cover, progress), text and
  pictures. A missing reading shows a grey dash, never 0. Text and pictures can also be drawn into the stored base.
- **Live engine:** 60 or 30 frames per second (60 by default), measured at about 0.7 to 1.2% of the PC's CPU with a plain
  background and about 2 to 4% with video and many widgets on a busy PC, at roughly 45 to 90 MB. **Hand-over:** it fades
  in over the screen and, when it stops, ends on the first frame of the stored loop so nothing jumps (fade in/out is a
  per-theme option). **Pause** and **Resume** in the tray. Sending a clip, restarting or checking the screen pauses live
  and brings it back.
- **Tray app:** with Live on, closing the window leaves CoolerScreen in the tray (menu: Open, Pause, 30 / 60 fps, Quit).
  "Free memory when the window is closed" removes the window completely (about 40 MB). `CoolerScreen.exe --tray` starts
  hidden in the tray with live on.
- **Opt-in system behaviour (all off or "normal clip" by default):** start with Windows (a visible start-up entry,
  removed by uninstalling), and what the screen does at sleep, lock and shutdown (show the stored clip, keep live
  running, or turn off).
- **Screen fit:** Settings > Screen fit for the edges the bezel hides (top / right / bottom / left) plus a whole-picture
  shift. The screen shows rulers while you tune, and your unsaved choice is applied to the real picture (live or base)
  at once. Existing `edge-offsets.json` values are picked up. A base stored with another fit is re-fitted by the engine
  and marked as needing to be stored again.
- **Rounded corners** follow the visible rectangle (after the screen fit and the notch band), in the live picture, the
  stored base and the editor preview; radius per theme with a default in Settings.
- **Screen guides:** hatched areas in the editor for the left and right bands and the curve band in the middle, with a
  warning on items inside one; calibrate them in Settings with lines drawn on the real screen; each theme can nudge them.
- **Browse copies:** adding from Browse makes an independent copy in My themes; adding again makes "Name (2)".
- **Safety:** nothing larger than the screen can hold is ever sent (the loop file is checked against the screen's free
  space before the first byte; impossible capacity readings are ignored); duplicate library items of the same file are
  merged at start-up (extras go to the Recycle Bin); a screen that is busy names who holds it (own live session, another
  CoolerScreen, LOVINGCOOL MONITOR, cs2 or something else) and is retried quietly every 4 seconds; Quit and shutdown can
  no longer hang (bounded waits and a 3-second exit watchdog); an upload is never cut off halfway, and Stop in the
  Apply window leaves the previous theme.
- **No Save buttons:** notch, screen fit, screen guides, corner radius and the Gallery folder are kept as you change them
  (Reset instead of Save / Undo). Live changes apply at once; only changes that alter the stored clip (fit, shift,
  corners, notch, text or pictures drawn into the base, base clip) mark the base as needing to be stored again.
- **Build id:** the build's hash, date and time are shown at the bottom of Settings, in the window title and on the first
  line of `device.log`. Every pause, resume and start of live is logged there with its reason.

Known gaps in 3.0.0:
- CPU temperature is not available yet (it needs a driver or a tool such as HWiNFO or LibreHardwareMonitor; see
  `docs/roadmap/2026-10-12-v3-data-sources.md`). The widget shows a grey dash.
- The audio visualizer has not been tried with real audio; GPU power showed a dash once; countdowns ignore daylight
  saving changes; there is no frame-rate counter.
- Start with Windows has not been tested over a real reboot, and the timing of the hand-over and of the screen-off endings
  has not been measured on the screen.
- Live backgrounds that are rendered rather than a recorded video (web / scene wallpapers) are not built yet.

See `docs/roadmap/2026-10-18-v3-release-and-v4.md`.


## 2.0.0 - 2026-10-12

A new engine under the same library: CoolerScreen is now a native Windows app (Rust + the system WebView2) instead of
Electron + Node. Your clips, versions and settings are used as they are, with no migration.

- **Faster and lighter:** the media pipeline (probe, thumbnails, loop search, estimates, conversion) is written in Rust
  around ffmpeg with a job scheduler that prioritises what you are looking at, runs independent work in parallel
  (CPU, GPU and network limits) and cancels work you scrolled away from. See `docs/roadmap/phase0-results.md` and
  `docs/v2/api-contract.md`.
- **Smooth previews:** a small local media server (token-protected, loopback only) streams thumbnails, loop cards and
  previews with seeking and looping.
- **One window, no browser mode:** the Node server, the Electron runtime and the browser fallback are gone. The window is
  seamless and frameless with Windows 11 **Snap Layouts** on the maximise button, remembers its size, position and
  maximised state, and asks inside the app before closing during an upload.
- **Installer:** a per-user setup (no administrator) that also replaces a 1.x install and keeps its shortcuts; plus a
  portable zip. The app icon is now a screen glyph.
- **Sources:** the built-in sources are ported to Rust; plug-ins are declarative `.json` files (no code), see
  `docs/sources.md`. Wallpaper Engine recording uses Windows Graphics Capture directly.
- **Recovery tools** are part of the app package as `cs2.exe` (`info`, `restart`, `unstick`, `recover-race-delete`,
  `delete-ota`); the 1.x Node tools and the command-line `bin/coolerscreen.js` were removed.
- Shutting the app down cancels all queued and running jobs and stops their ffmpeg processes.
- **My clips and Browse** are separate tabs: My clips (your clips, with name filter and sort) and Browse (enabled
  sources, clips you already have first, filter / show / refresh). Cards never reorder while something downloads.
- **Loop finder sees what the screen will show:** every start is compared with every end (no sampling) and each pair
  is scored as a hard cut or with the best cross-fade (1/3 to 1.5 s), counting the part of the picture that would
  show a double image during the fade. Labels: Seamless, Seamless fade, Soft fade, Nearly seamless, Small jump.
  Results from the old search are redone when a clip is opened.
- **Wallpaper Engine recording is automatic by default:** it stops once the recording holds a seamless loop (30 s to
  2 min), otherwise records 2 min; fixed lengths up to 3 min. Scenes with random effects (rain, fireflies, particles)
  never repeat exactly, so they get a cross-fade.
- **Loops are found as soon as a clip arrives** (added, downloaded, recorded or imported), in the background; opening
  the clip joins that search.

## 1.2.0 - 2026-10-11

- **Browse** replaces the Gallery/Lovingcool tabs: "My clips" plus a checkbox for each enabled source, merged in one grid
  (paged per source, source badge, In your gallery / Not added / Needs recording, one action per card).
- **Settings > Sources** (all off by default): Lovingcool themes; Other brands on the same theme platform (brand 2, 31 themes
  for 2400x1080); Wallpaper Engine (copies video wallpapers, records scene/web wallpapers in a temporary window with Windows
  Graphics Capture, desktop app only); Lively Wallpaper (video/GIF/picture); Add from link (direct file links, 500 MB);
  plug-in sources from Documents\CoolerScreen\Sources (docs/sources.md, docs/example-source.js), loaded only when enabled.

- **Lovingcool themes** tab: the vendor's online theme catalogue for this screen (cover pictures, paging as you scroll,
  cached list with Refresh, "In your gallery" badge), one-click "Add to gallery" with progress (downloads the `.ht` into
  the Gallery folder; the existing import does the rest). CLI: `coolerscreen themes [--page n] [--download id]`.
- Layout stability: changing numbers use tabular figures, time read-outs and status lines have fixed widths/heights and
  never wrap, the activity bar is always one fixed-height line, the Versions button keeps its size while preparing, so
  nothing moves while a clip plays or a job runs.

- Simpler, "we pick for you" UI:
  - The app opens on a **Gallery** of all clips (large pictures, hover to play, status chips, Add clip tile); clicking a clip
    opens it, "‹ Gallery" goes back.
  - **Loop cards** are the only loop control: the whole-clip search runs automatically the first time a clip is opened
    (results remembered), the best loop is chosen, each card plays its own loop with "Seamless / Nearly seamless / Small
    jump"; a cross-fade is added automatically when the best join isn't seamless. "Shortest loop" / "Longest loop" with ⓘ
    tooltips. Manual frame editing only via ⋯ > "Edit loop manually…".
  - **Quality** is always "Best that fits" (no Custom/Simplify controls). **Versions** moved into a dropdown in the clip
    header; a bottom bar has "Save as version" and **"Push to screen"** (saves if needed, prepares, sends).
- Best that fits, re-benchmarked (SSIM vs the unblurred source at the same size, pink-blue 8.5 s / 4 s, Coast 8 s): one
  fixed JPEG quantiser with the range extended past 31 up to 69 and **no blur** beats blur + q<=31, 1- and 2-pass rate
  control and trellis. Auto now uses it and fills ~97-99% of the space (blur only as a last resort).

- "Find loop points": one button searches the whole clip, frame-accurate, for the smoothest start/end pairs within a
  loop-length range (motion across the join, cached small frames, leaders re-checked at higher resolution); the top 5
  with first/last frame thumbnails, length and seam score; "Use" applies one exactly.
- "Best that fits" chooses the JPEG quality and the fine-detail simplification together and shows what it chose; the
  Simplify slider is only in Custom.
- Rounded corners round the visible picture (inset by the notch band).
- Screen not responding: "Restart screen" (banner and Settings > Screen) sends the restart command, waits for the screen to
  come back and checks it; a check or upload start that gets no reply restarts the screen once automatically. A send now
  ends by restarting the screen (instead of the "play now" command, after which the screen once stopped answering) and a
  final check.

- Loop editing: frame-accurate start/end (source frame / screen frames), frame nudge buttons and arrow keys (Shift = 10),
  a filmstrip trim track with "Zoom to the selection", "play the seam" preview mode, a seam check (onion skin, side by
  side, difference, similarity score), "Find best loop point" (±2 s search), and "Blend the end into the start"
  (cross-fade, 0-1.5 s, mirrored in the preview).
- Rounded corners moved into Tune and are stored per version (the last choice is the default); the notch stays global.
  Versions made with a different notch are marked "Made with older screen settings – re-make".
- "Simplify fine detail" (Gaussian softening, mirrored in the preview) to fit longer parts; when even the lowest quality
  doesn't fit, the app suggests a level before suggesting a shorter part.
- Space on the screen is measured at each screen check (free storage + our two clips, minus the start-up logo and a 160 KB
  margin) instead of a fixed 9.5 MB.
- Fixed: the selected clip's view was rebuilt on every background refresh (lost open panels and focus).

- New window layout: Library on the left; preview, Tune and Versions on the right; screen status and "On screen now" in the
  top bar; one Activity bar for preparing and sending; Settings drawer (screen, notch, Gallery folder, advanced, about).
- Adding a clip no longer converts it. The preview plays immediately and matches the result (part, size and position,
  notch). New framing controls: zoom, left-right, up-down, drag to move, fill the screen or show the whole picture.
- Several versions per clip ("Save as version"), each sendable; clip ids are permanent (renames keep versions). Libraries
  from 1.1 are migrated automatically.
- Every version is made at 18 fps, the rate the screen actually plays at (it ignores file timing). Auto quality only lowers
  JPEG quality and says how long the part can be if it can't fit. The fps control is gone. Older versions made at other
  rates are marked "plays too fast/slow – re-make".
- The intro clip is no longer a user feature: the start-up slot holds a 1.5 s "CoolerScreen" logo (sent only when needed).
- Delete moves the file to the Recycle Bin and deletes its versions (in-app confirmation). Files that vanish from the
  Gallery folder are kept as "File missing" with their versions; after 30 days the app offers to delete those versions.
- The screen is checked once at launch (status pill + banners with one action each). Closing the window during an upload
  asks first. In-app dialogs replace all browser confirm/prompt boxes; keyboard and screen-reader basics.
- CLI: `list` shows clips and versions; new `add`, `make`, `send <version>`; `upload` does all three.

## 1.1.0 - 2026-10-10

- Desktop window (Electron) instead of a browser + .bat: `CoolerScreen.exe` is the unmodified official Electron runtime
  (renamed), app files in `resources/app`. Closing the window exits everything; nothing runs in the background.
- Gallery folder (`Documents\CoolerScreen\Gallery`): files placed there appear automatically, deleted files disappear,
  imports are copied there, rename renames the file, delete moves it to `_Deleted`; existing items are migrated.
- Import vendor `.ht` theme files (and plain `.zip` files containing a video or image); drops work anywhere on the page.
- Notch band width now rounds up to the 16 px MJPEG block grid.
- "Reconnect" for a screen that Windows failed to enumerate at boot (one admin approval, resets that USB port).
- `scripts/install-local.ps1` / `uninstall-local.ps1` (current user, no admin; shortcuts only).

## 1.0.0 - 2026-10-10

First release.

- Local web app (`npm start` / "Start CoolerScreen.bat"): import video, GIF or image, convert to the screen's
  1080x2400 MJPEG format, gallery with previews, rename, delete, re-convert.
- Per-item controls: start/loop length, fps 10-30, quality Auto (fit to space) or Manual 1-100, with a size estimate.
- Notch mask (band or circle) baked into the converted file and preview.
- Loop and intro slots with space budgeting; "Send to screen" puts the clip in the loop slot, blanks the intro and starts playback.
- WinUSB transport and one-time "Set up screen" (avoids the firmware's 30 s display-off watchdog; nothing runs in the background).
  "Restore original driver" undoes it.
- CLI: `upload`, `info`, `list`, `slots`, `clear`, `setup`, `restore`, `transport`.
- Portable Windows release zip with bundled Node.js (`scripts/make-release.ps1`).
