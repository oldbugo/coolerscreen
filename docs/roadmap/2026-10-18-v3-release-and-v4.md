# CoolerScreen 3.0 release notes for the team, and the 4.0 starting brief

Written 2026-10-18, when 3.0.0 was prepared. Read this first when picking the project up again. The 2.0 state and the
original 3.0 brief are in `2026-10-12-v2-release-and-v3.md`; the theme contract is `docs/v3/theme-format.md` (v1 to v1.7).

## Where 3.0 ended

**Final 3.0 = 3.0.3 (tag v3.0.3).** User, 2026-10-18: "this is the actual end of version 3". After 3.0.0 came: Use this
theme vs Embedded (3.0.1), quiet installs and tests (3.0.2), live-fit and theme-corner fixes (3.0.3). The user said the
result "looks and work[s] pretty good". User flows are kept in the vault page Projects/CoolerScreen user flows (standing
rule: record flows whenever they are discussed).

- **Shape:** the 2.0 app (Tauri 2, system WebView2, Rust core, ffmpeg) plus a live engine in `src-tauri/src/live/`.
  One version only, on `main`; v2.0.0 (tag) is the fallback. Everything on the screen is a **theme**: a stored **base**
  loop plus an optional **live layer** (widgets drawn by the engine over the base). Pages: On screen (editor), My themes,
  Browse. Companion form B: the app itself stays in the tray with its window destroyed (no second program, no service).
  Live, start with Windows, and sleep / lock / shutdown handling are opt-in; defaults behave like 2.0.
- **Features:** see `CHANGELOG.md` 3.0.0 (editor with layers, groups, snapping and guides; clock, date, sensor, weather,
  visualizer, slideshow, info and music widgets; Screen fit; rounded corners on the visible area; safety guards; no Save
  buttons).
- **Measured live cost (user's PC, 16 logical cores):** about 0.7 to 1.2% of the PC's CPU with a plain background, about
  2 to 4% with a video background and many widgets on a busy PC; about 45 to 90 MB of memory with the window destroyed
  (about 240 MB with the WebView alive). Earlier prototype numbers are in the 2.0/3.0 brief file.
- **Verified on the real screen (SC70, fw 3.6):**
  - the live protocol works (frames, keepalive, end sequence), portrait 756x1680 frames, 4:2:0 JPEG, up to about 200 KB;
  - 60 fps is accepted without send errors (whether the panel shows every frame is unknown), with 30 as the alternative;
  - the user checked readability and the look of the live picture, tray Quit returning to the loop, and applied themes
    with the Apply window (uploads 2026-10-17 14:48, 16:07, 16:25 UTC);
  - hidden edges: top 4 px / left 9 px were measured with the ruler tool; the "top still cut" the user kept seeing turned
    out to be the rounded corners drawn at the panel edge. Fixed in 7a7a798 (corners follow the visible area) —
    **not yet confirmed by the user**; their saved top value was 16 at that point and may need lowering back to ~4.
    Screen guides exist but the user has not reported calibrating them.
- **Not verified by the user or on hardware:** start with Windows over a real reboot; screen-off endings (sleep, lock,
  shutdown) and the timing of the hand-over; the visualizer with real audio; CPU temperature (not built); weather on a long
  run; behaviour on another screen or firmware; installing 3.0.0 on a clean machine without the dev tools.

## What we learned

Device facts (do not re-investigate):
- Live frames must be **portrait**: landscape content rotated 90 degrees clockwise, like the clips. JPEG must be **4:2:0**
  (4:4:4 is accepted and never shown). Live frames are about 1680x756 landscape; full 2400x1080 shows black.
- A **single** live frame is not shown by the device: keep re-sending (about 5 per second even for a still picture).
- **Tearing** is a panel limit: motion along the landscape vertical axis tears at any frame rate, so widgets move sideways
  or fade, never up and down. Numbers large (48 px or more on the 2400 canvas).
- After a stream ends the screen can go black (the stored `0x15` value); sending `0x15 [0, 10]` once fixed it, and `0x0E [1]`
  plus `0x14 [1,0]` turn it back on. The stop sequence does this so the loop keeps playing.
- The firmware 30-second watchdog is avoided by the WinUSB setup; keepalive `0x11` every 1.5 s during a stream.
- Only one program can hold the screen. Anything else that wants it (upload, check, restart) must pause live first;
  a connect in progress counts as holding it.
- Hidden edges and curve: the bezel hides a few pixels (here top 4, left 9), the left and right bands and a middle curve
  distort or hide content; the notch band is measured from the physical edge. Rounded corners must follow the visible
  rectangle, not the panel edge.
- Flash capacity is about 11 MB in total; a conservative cap and a check before any byte are needed because oversize files
  make the screen show garbage.
- CPU temperature on this Ryzen has no reliable source without a kernel driver; GPU data comes from NVML.

Process lessons:
- Explore the design with the user before building (clickable mock-ups); the user redirected the whole product from "MP4
  with a live companion" to "themes with two faces" that way.
- Measure on the real screen before deciding (prototype A versus B decided the companion form).
- The user wants no hidden software, no Save buttons, and defaults equal to the previous version.
- Guards belong in the app, not in habits: size checks, a single holder of the screen, bounded waits on quit.
- Close the user's running app only when no job is running (standing instruction).

## Known gaps / open items

- CPU temperature through HWiNFO shared memory or LibreHardwareMonitor (opt-in, we never install or elevate anything) is
  not built; the widget shows a dash ("not available yet").
- No FPS counter to show the live frame rate.
- The visualizer has not been tested with real audio.
- GPU power showed a dash once; cause not found.
- The countdown info widget ignores daylight saving changes.
- Engine timing tests are flaky unless run with `--test-threads=4`.
- The build hash shows a "+" (dirty) because `tauri build` rewrites files under `gen/`.
- The 2.0 check "screen content changed outside the app" (compare what is on the screen with what the app last stored) was
  proposed and not built.
- Start with Windows is untested over a real reboot; hand-over timing and the screen-off endings are not measured on hardware.
- Wallpaper Engine-like live backgrounds (web, scene or generated, not a recorded video) are the big next step.

## 4.0 starting brief

Candidate directions. **User-stated** means the user said it; **proposed** is ours.

1. **Live-rendered backgrounds** (user-stated, 2026-10-12: "similar to Wallpaper Engine where it's not the actual video
   itself"). Backgrounds drawn live (shaders, particles, web pages or scenes) so they never need to loop. Likely route: a
   small renderer in the engine (generated effects first), then web or scene import. The base stored on the screen must
   still be made from the same theme (record a loop from it) to keep the two-faces rule. Needs a CPU budget decision.
2. **Richer widgets** (user-stated, 2026-10-18: widgets "very lacking ... all the regular ones"; 3.0 covered the common
   ones). More could be CPU temperature (needs the HWiNFO/LHM opt-in), network details, per-app audio, calendar,
   notifications, user text/web widgets. Which ones is **proposed** until the user picks.
3. **Marketplace / sharing of themes** (proposed): export and import a theme as one package (`theme.json`, assets, base
   clip), a Browse source for them, and previews. Browse already makes independent copies.
4. **Calibration for other people with the same screen** (user-stated in part, 2026-10-17: build screen fit in "so that
   other users could also run it"). Guided flows for hidden edges, guides and notch, and sensible defaults per model.
5. **Packaging and signing for sharing on GitHub** (proposed; code signing was deferred by the user in 2.0): a signed
   installer, an update path, a public README with screenshots.
6. **Quality of life** (proposed): FPS counter, the "content changed outside the app" check, hand-over and shutdown timing
   measurements, a real-reboot test of start with Windows.

Standing rules that carry over: nothing hidden in the background, never upload non-MJPEG to the clip slots, never send 0x05,
0x0C or 0x23, defaults behave like the previous version, every option is a setting, no Save buttons, one version on `main`.
