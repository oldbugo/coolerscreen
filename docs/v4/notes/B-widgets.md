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

## B2 (built 2026-10-19): CPU temperature and six more

Branch `claude/recursing-matsumoto-31e4c5`, on top of main 28bd568 (4.2.0). Contract: `docs/v3/theme-format.md`, section "4.0 widgets,
stream B2". New: external sensors, disk read/write, network totals, **Live text, Volume, Ping, Agenda**.

- **CPU temperature (and CPU power, CPU fan, board temperature)**: read from a tool the user already runs: HWiNFO shared memory first
  (`Global\HWiNFO_SENS_SM2`, read-only mapping, about every 2 s), else LibreHardwareMonitor's web server (`127.0.0.1:8085/data.json`, every
  15 s when it is not answering). We never install, start or elevate either. Nothing is looked at unless a widget shows one of these four
  metrics (the sampler is demand driven). They work in `sensor` (all styles) and `meters`. With neither tool running the value stays the
  grey dash (as in 3.0) and the editor shows a line under the metric: "No CPU temperature source. Start HWiNFO with Shared Memory Support
  on, or LibreHardwareMonitor with its Remote Web Server on (port 8085). CoolerScreen never installs or starts them." (new command
  `ext_sensor_status`, capability `allow-ext-sensor-status`). With a source it says "Reading from HWiNFO." and if the tool has no CPU
  sensor, that. No Settings switch: it is passive and only active while such a widget exists (a Settings off-switch can come with the UI
  revision if wanted).
- **Disk read / write speed**: new metrics `disk.read`, `disk.write` (PDH "Disk Read/Write Bytes/sec", all disks), in `sensor` and `meters`.
- **Network totals**: `network` has a Totals switch (off): data moved since start-up (adapter counters).
- **Live text**: the Text widget with `{words}` (time, date, weekday, cpu, cputemp, cpupower, cpufan, gpu, gputemp, vram, ram, down, up,
  diskread, diskwrite, uptime, volume); an "Insert a word" dropdown in the panel.
- **Volume**: master volume of the default playback device, bar or ring, "Muted" greyed when muted (read only).
- **Ping**: one ICMP echo a second to a host the user types (default 1.1.1.1), number or graph; says so in the panel. At most 3 hosts at
  once. It only runs while a Ping item is shown.
- **Agenda**: next 2-6 events from an https/webcal calendar link or an `.ics` file path; day names ("Today", "Tomorrow", "Thu"), times,
  all-day, "Now" for the current one. Own ICS parser with recurrence (daily / weekly + BYDAY / monthly / yearly, INTERVAL, COUNT, UNTIL,
  EXDATE), cancelled events dropped, UTC converted with the PC's zone.

Choices (user asleep; change freely): ping default host 1.1.1.1 because adding the widget is the user's own action and the panel says what
it sends; the calendar link is typed/pasted, not picked from a dialog (a picker would need a new app command; a non-programmer can paste a
Google "secret address in iCal format" or a path); Ping/Agenda/Volume/Live text are in the palette next to their neighbours (Sensors, Time,
Media, Decoration). Not built: a writable volume control, calendar accounts/sign-in, per-app volume, CalDAV.

### Verification (B2)
- `cargo test --release --lib -- --test-threads=4`: 256 passed, 0 failed, 16 ignored.
- New tests: HWiNFO layout parsed from a synthetic shared memory (and truncated / foreign data refused), LibreHardwareMonitor tree incl.
  decimal commas, nonsense temperatures dropped, status message; live words expansion; ping host rules + a real echo to 127.0.0.1;
  agenda: recurrence, EXDATE, cancelled, monthly skipping 31sts, durations, UTC, a real file read through the loader, source
  classification; volume read from the real device (it reported "level 0.22, muted"); real PDH disk read (30 MB/s) and adapter
  counters; every sensor metric as a meter row; all new widgets drawn in every option and in tiny boxes.
- Pictures (`docs/v4/notes/B-pictures/`): `b2-preview.png` (sample data, every new widget), `b2-live-frame.png` (a real live frame from a
  quiet debug instance: agenda from a real .ics file, a real 30 ms ping, real volume "Muted", live text with real load, meters with
  a dash for CPU temp because no tool runs here), `b2-editor-meters-note.png` (the editor showing the reason line).
- Editor driven over the debug port: palette entries for the new items, Live text panel, Agenda source field (set to a file path and the
  events appeared live), Meters row switched to CPU temperature shows the reason line.

### Not verified
- **CPU temperature against a real HWiNFO or LibreHardwareMonitor**: neither runs on this PC. The parsers are tested on synthetic data built
  from the documented layouts (HWiNFO `HWiNFO_SENS_SM2` header and reading element, LHM `data.json` tree); the real thing may differ in
  details (e.g. label names). Please check once with the user's HWiNFO (Settings > Shared Memory Support) before telling them it works.
- Agenda with a real calendar link (https) and with time zones other than the PC's (TZID events are treated as local times).
- Not on the real screen. Not run: the Ping graph and the Agenda panel edit flow beyond the source field.
- Stale-looking facts: the ping/agenda threads end 5 s / 30 s after the last widget draw; not measured for CPU separately (they sleep 1 s / 5 s).

### Incident (told the PM)
At about 12:30 I started my RELEASE test build with `COOLERSCREEN_MULTI=1` while the user's installed CoolerScreen was running. MULTI only
works in debug builds, so the single-instance plugin forwarded my launch to the user's app (it may have raised its window once). My copy
exited at once, nothing was sent to the screen. Afterwards I used a debug build (`cargo build`, pid checked by path) for every test instance.
`tools/README.md` says "(debug builds)" for MULTI; a release test build needs the installed app to be closed first.
