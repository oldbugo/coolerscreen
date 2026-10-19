# Stream B: more widgets (notes)

Branch `claude/recursing-matsumoto-31e4c5`. Written by the stream B worker session, 2026-10-18.

## What 3.0.3 already has (so the list below is only what is missing)

clock (digital / analogue / minimal), date (line / calendar / big), sensor (CPU load, one core, CPU clock, GPU load /
temp / clock / VRAM / power / fan, RAM, net down, net up, disk used, disk activity; as number / bar / ring / gauge /
graph / line), stats panel, music (cover + progress), weather (now / forecast), visualizer, slideshow (GIF), picture,
text, info (uptime, CPU/GPU name, countdown, top processes).

The gap: every sensor widget shows **one** reading. PC-monitor users expect panels that show several at once (all cores,
all drives, up + down together, a list of their own choosing), plus the time tools (world clocks, timers).

## Proposed widget list (ordered by what PC-monitor screen users expect)

| # | Widget | Why | Source | Opt-in source? | Milestone |
|---|---|---|---|---|---|
| 1 | **Meters** (own list of 2-8 rows, any metric, each a bar with big value) | The AIDA64 / NZXT "system monitor" panel; the fixed Stats panel cannot be customised | existing metrics hub | no | B1 |
| 2 | **CPU cores** (every logical core as bars or a heat grid + average) | Per-core view is in every monitor app; 16-32 threads need a compact form | NtQuerySystemInformation (as `cpu.core`) | no | B1 |
| 3 | **Network** (download + upload together: two big numbers + one two-colour graph) | Up and down next to each other is the standard network tile | GetIfTable2 (existing hub) | no | B1 |
| 4 | **Disks** (all fixed drives: used bar, free space) | "Disk usage" overview; today one drive per widget | GetLogicalDrives + GetDiskFreeSpaceEx | no | B1 |
| 5 | **World clocks** (2-4 cities, time + day offset) | Very common, cheap, no data source | Windows time-zone registry (no network) | no | B1 |
| 6 | **Timer** (stopwatch, countdown timer, Pomodoro) | Desk-screen staple; started/paused from the editor | system clock | no | B1 |
| 7 | CPU temperature (+ fans, other board sensors) | The most asked-for reading; no safe built-in source | HWiNFO shared memory / LibreHardwareMonitor web server the user already runs | **yes** (user runs HWiNFO or LHM; we never install or elevate) | B2 |
| 8 | Agenda / calendar events | Day overview next to the clock | a local `.ics` file or the user's published ICS link | **yes** (file or link chosen by the user) | B2 |
| 9 | Disk read / write speed, network totals today | "Disk activity" and "data used" | PDH counters; daily counter kept in the app folder | no | B2 |
| 10 | Volume (master level + mute) | Visible feedback of the PC's volume | WASAPI endpoint volume | no | B2 |
| 11 | Ping / latency to a host (and a connection light) | Gamers' favourite | ICMP to a host the user types | **yes** (the user names the host) | B2 |
| 12 | Text with live words (`{cpu}`, `{time}`) | Custom readouts | existing sources | no | B2 |
| - | Not now: battery (desktop PC), public IP (needs a web call), per-app volume (large API surface), notifications | | | | later |

Rules for all of them: nothing appears in existing themes (new types only; no existing widget changes behaviour),
defaults are sensible, every option is a setting that saves and applies at once, a missing source shows a grey dash (never
0), big numbers (>= 48 px on the 2400 canvas), no motion along the vertical axis (bars grow sideways, fades only).

## B1 (built 2026-10-19): six widgets

Branch `claude/recursing-matsumoto-31e4c5`. New types: **Meters, CPU cores, Down + up (network), All disks, World clocks,
Timer**. Contract: `docs/v3/theme-format.md`, section "4.0 widgets, stream B". Palette: World clocks and Timer under Time;
Meters, CPU cores, Down + up and All disks under Sensors. Existing widgets are untouched and no existing theme changes.

What each does (all options are editor settings, saved at once, no Save buttons):
- **Meters**: your own list of 1-8 readings (any sensor metric, optional own name, move up/down), each a name, a big value and a bar
  coloured by the metric's usual green/amber/red steps; bars on/off, C/F, rate.
- **CPU cores**: every logical core as horizontal bars (columns chosen so rows stay readable) or a heat grid; average load as a big
  number; colour automatic (by load) or one chosen colour.
- **Down + up**: two big numbers (download, upload; B/s to GB/s scaling) and one two-line graph (10-600 s); graph can be turned off;
  both colours editable.
- **All disks**: every fixed drive (or the letters you type), bar + "used / size" or "free"; 2-6 rows; amber 85 %, red 95 %.
- **World clocks**: 1-4 cities from a list of 43 zones (names editable), 24h/12h, "Tomorrow/Yesterday" tag; rules come from the Windows
  registry, so no network and daylight saving is right.
- **Timer**: stopwatch, countdown timer (minutes + seconds) or Pomodoro (focus/break minutes); Start / Pause / Reset buttons in the
  editor panel; display is computed from the PC clock so it keeps running across restarts and with the window closed; a finished
  timer turns red and pulses its opacity (no movement).

Choices I made (the user is asleep; change freely):
- The timer is controlled from the editor only (no on-screen or tray control; the screen has no touch). A tray start/stop could be B2.
- Sizes: big numbers fill their rows; the box defines size. Defaults are 26-36 % wide so numbers are above 48 px on 2400. Defaults
  per type are chosen in `morewidgets.js` `defaultSize` and follow rows/cities/style.
- Core and drive data reuse the shared sampler (`metrics.rs`); I added no thread, no new data source and no admin need. The sampler
  polls only what widgets show.
- Zone list is a fixed set of common cities; the 4-city limit keeps numbers large.

Code map: `src-tauri/src/live/morewidgets.rs` (props, cleaning, timer maths, time zones, drive list), `src-tauri/src/live/render/more.rs`
(drawing), `public/js/morewidgets.js` (palette + panels). Small hooks in shared files: `render.rs` (`mod more;` and the `_ =>` arm of
`draw_one`), `render/sensors.rs` (`pub(super)` on a few helpers), `theme.rs` validate, `core/themes.rs` (type check + clean),
`live/mod.rs`, `Cargo.toml` (windows-sys feature `Win32_System_Time`), `public/js/widgets.js` and `themes.js` (merge entries, one list).

### Verification (B1)
- `cargo test --release --lib -- --test-threads=4`: 223 passed, 0 failed, 13 ignored (the ignored ones were already ignored).
- New tests: morewidgets (cleaning bounds, timer maths incl. pomodoro cycle, civil days, Tokyo = UTC+9 via the registry, drive list);
  render/more (all six in every option and in 2 %-tiny boxes, empty lists, garbage props; live data cost); core (`theme_set` cleans and
  saves the new types, unknown types still refused).
- Frame pictures (look at them): `docs/v4/notes/B-pictures/`
  - `b1-preview-all.png`, `b1-preview-variants.png`: engine preview renders of all six and the other options (sample data).
  - `b1-live-frame-stacked.png`: a real live frame from a quiet fake-device instance (`COOLERSCREEN_DUMP_FRAMES`, rotated back to
    landscape), real CPU / RAM / network / disk readings, widgets deliberately piled on each other (the editor drops new items at one
    spot, as before).
  - `b1-editor.png`: the On screen editor with the new palette entries and a property panel.
- CPU: one full redraw of all six with live data costs 5.9 ms (0.6 % of one core at the 10 redraws/s the engine uses); the whole quiet
  instance with live on and all six (plus two older sensors) used 1.45 % of the PC's CPU (16 logical cores) and 82 MB over 20 s, while
  other sessions were compiling. Sampler thread polls only shown metrics as before.
- Editor driven over the debug port: palette click adds each type, panels build, Timer Start flips to Pause, Saved appears.

### Not verified
- Not checked on the real screen (stream rules: only the PM touches the device); 4:2:0 JPEG look of thin lines is unseen on the panel.
- Clicked through over the debug port: Disks and Timer panels (Start flips to Pause), Meters (add a row, change its reading: layer name
  follows "Meters (5)"), World clocks (add a city, change a zone: the name follows). Not clicked: every other control (colours, ranges,
  Cores / Down + up panels), though they use the same helpers.
- The row editors of Meters and World clocks use plain stacked controls (classes `meter-rows` / `meter-row` / `inline-ctl`, no CSS of
  their own yet): the UI revision (stream D) should style them; see `b1-editor-worldclock-panel.png`.
- World-clock zone rules use the registry's current-year rules; a zone whose government changed DST after this Windows build is as
  right as Windows is.
- Heavy pile-ups of overlapping widgets are drawn in order as for every other widget; nothing special for overlap.
