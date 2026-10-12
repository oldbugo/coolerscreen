# CoolerScreen 2.0 release notes for the team, and the 3.0 brief

Written 2026-10-12, when 2.0.0 was tagged. Read this first when picking the project up again.

## Where 2.0 ended

- **Shape:** Tauri 2 + system WebView2 + Rust backend (`src-tauri/`), FFmpeg sidecar, UI in `public/`. Unsigned
  per-user NSIS installer (`npx --yes @tauri-apps/cli@2.0.0 build`, output under
  `src-tauri/target/release/bundle/nsis/`). The library format is still 1.2's (`%APPDATA%\CoolerScreen`), so 1.2 and
  2.0 can read each other's data.
- **Principle up to 2.0: nothing runs in the background.** Everything the screen shows lives on the screen itself (intro
  clip once, loop clip forever). The one-time WinUSB setup stops the firmware's 30 s DTR watchdog, so the app can be
  closed (or uninstalled) and the screen keeps playing. The user rejected a Windows service and a run-at-login entry.
- **Measured 2.0 results:** opening a new clip 11–32 s → ~2–7 s; clicking through 5 clips 65 s → ~4 s; Save as
  version 6–11 s → ~0.3 s; installer 24.6 MB, installed ~90 MB (83 MB is ffmpeg); RAM about the same as Electron
  (~340 MB is WebView2). Details: `docs/perf/2026-10-11-performance-investigation.md`, `docs/roadmap/phase0-results.md`.
- **Last round (2026-10-12):** My clips / Browse tabs; blend-aware loop finder with a ghost check; automatic
  Wallpaper Engine recording; background loop search for new clips.
- **Install note:** Smart App Control blocked a fresh unsigned installer once, then let the same file run about 10
  minutes later (likely after its cloud reputation check). Code signing would remove this; the user deferred it.
- **Not checked by the user before release:** Snap Layouts hover on the maximise button, and the full Restore driver →
  Set up screen round trip (two UAC prompts). Both worked in development tests.

## What we learned about loops (don't re-investigate)

- The screen plays a fixed ~18 fps, so a loop is a whole number of 1/18 s frames. A loop joins seamlessly only if the
  content really repeats within a length that fits the screen (~11 MB budget, so roughly 5–15 s at good quality).
- **Wallpaper Engine scenes with particles (fireflies, rain, ash, petals) and noise shaders never repeat.** For REM
  (`we-2105889348`), a 3-minute recording showed no repeat except a partial one every ~89 s: the music-synced effects
  come back, the particles don't. Even there it was 3× a normal frame step, and 89 s is far too long to fit. The best
  hard cut anywhere was 6.6× a normal frame step. Miku Symphony, Ocean Echo, Ayanami Rei, Pastel Girl and EVA Rain
  showed the same pattern.
- Comparing the two best-matching REM frames showed only particles, glints and steam differ. A 1–1.5 s cross-fade
  therefore hides most of the join (scored 1.25×, ~2% of the picture fades). Scenes where the camera drifts (Miku)
  fade with a brief double image (5–8% of the picture), labelled "Soft fade".
- **User's verdict (2026-10-12):** some recorded wallpapers still show a small jump or artefact at the join. That is
  the content, not the search, and is accepted as a limit. Don't spend more time on it unless the approach changes,
  e.g. a live companion that renders forever (see 3.0).
- The analysis tools are an ignored test, `media::loops::tests::real_files` (env `CS_LOOP_FILES`, `CS_LOOP_MAX`), and the
  live test `sources::live_tests::record_auto`.

## 3.0 brief: an optional always-on companion

**The change of principle:** up to 2.0, CoolerScreen avoided anything running in the background. For 3.0 the user wants
to explore a **companion that stays running**, which turns the screen from a static loop into a live display:

- what's playing (music title, artist, cover, progress),
- CPU / GPU temperatures, load, clocks, fan / pump speeds and other sensors,
- other widgets (clock, weather, notifications…), composed into layouts on the 2400x1080 landscape canvas.

**Known device facts that matter:**
- The vendor app already streams live content: it sends JPEG frames over the same serial/WinUSB link and ends a stream
  with `FF D9 FF D9` then `00 00 00 00`. The exact framing (key, chunking, frame rate) still needs capturing. Before
  writing anything, start with a read-only USB capture of LOVINGCOOL MONITOR's live mode, or a careful reading of
  its code.
- `0x15 [enable, seconds]`: after a live stream ends, play the loop clip for N s and then turn the display off. This is
  stored on the device and is the hook for "what happens when the companion stops".
- WinUSB setup stays: a live stream needs the bulk endpoints anyway, and the watchdog must stay off.
- Upload speed is ~50 KB/s (flash). That limit is for file uploads; how fast live JPEG frames go is unknown. Live mode
  probably writes to RAM, not flash, so it is likely much faster. Measure it first: it decides frame rate and quality.
- Safety rules carry over: never send non-MJPEG files to the clip slots; never use 0x05 delete, 0x0C OTA or 0x23 from
  the app.

**Questions to settle at the start of 3.0:**
1. **Companion form:** tray app (same Tauri app with a hidden window and a Windows startup entry the user turns on), or
   a small separate Rust helper (no WebView, a few MB of RAM) that the main app configures. The user disliked services,
   so it must be visible, easy to quit and opt-in. Proposal: the helper, with a tray icon.
2. **Rendering:** draw frames in Rust (tiny-skia or similar) and encode JPEG (turbojpeg) at 2400x1080 rotated, to stay
   light; or render HTML widgets offscreen in WebView2 (flexible but heavier). Budget: under ~1% CPU and under ~50 MB RAM.
3. **Data sources:** Windows media session (GlobalSystemMediaTransportControlsSessionManager) for music; for sensors,
   LibreHardwareMonitor (needs a driver and admin) or NVML for the NVIDIA GPU plus a lighter CPU temperature source
   (Ryzen 9800X3D). Licensing and admin needs to be checked.
4. **Fallback:** when the companion stops, sleep, shutdown or crash should drop back cleanly to the stored loop clip
   (`0x15`, or let the device time out). The 2.0 "no software needed" mode must remain the default.
5. **Shutdown:** a running companion can finally do what the vendor did (send display off when the PC shuts down), which
   answers the parked "screen off when PC off" question without the BIOS ErP setting.

**Suggested first step for 3.0:** a read-only spike. Capture the vendor's live protocol, then push a single test JPEG
from a scratch tool and measure frames per second and latency on the real screen. Decide 1–2 above from those numbers.

## 3.0 decisions (user, 2026-10-12)

- **Defaults = 2.0 behaviour; every option configurable.** No screen-off at shutdown by default; the companion stopping
  falls back to the stored loop clip; start with Windows is an opt-in setting; no admin sensor driver by default.
- **Widgets wanted:** now-playing music, CPU/GPU temps and load, clock/date, and an animated gallery clip behind the
  widgets.
- **Companion form: B (decided by the user 2026-10-12 after the hardware comparison):** the CoolerScreen app keeps
  running in the tray when its window is closed, and closing frees the window (WebView destroyed). Was: prototype both and measure
  before choosing.
- **Branch:** `v3`; `main` stays 2.0.
- **Vendor app:** already uninstalled (its folder is empty), so the live protocol is tested directly. Working
  hypothesis: live mode is raw JPEG frames (FF D8 … FF D9) written straight to bulk OUT 0x02, ended by
  `FF D9 FF D9` + `00 00 00 00`.

## Live mode: verified on hardware (2026-10-12)

Source: the vendor's Electron app (LOVINGCOOL MONITOR 2.2.60, unpacked read-only from its updater cache), then tested
with `cs2 live-test` on the user's screen (fw 3.6). User saw the counter pictures.

- **Enter:** framed `0x11` (no payload) → reply `{"key":17,"hex":"00"}`. Re-send `0x11` every 1.5 s as keepalive.
- **Frame (fw > 2.8):** `u32LE len | JPEG | u16LE sum` (sum of the 4 length bytes + JPEG bytes, & 0xFFFF), no 55 AA
  wrapper, straight to bulk OUT 0x02. Landscape 1680x756 (2400x1080 × 0.7) like the vendor; ≤ 260 KB.
- **JPEG must be 4:2:0.** 4:4:4 frames are accepted on the wire but never shown (the first failed run). `jpeg-encoder`
  picks 4:4:4 at q ≥ 90, so force `SamplingFactor::F_2_2`.
- **Link speed:** ~10–11 MB/s; 250 KB frames reach ~40–43 fps. Shown frame rate not yet measured.
- **End:** `FF D9 FF D9` (own transfer), then `0x14 [1,0]`. Afterwards the stored sleep delay (`0x15`, set earlier by
  the vendor app) can switch the display off (`diplay_on:false`, screen black). `0x0E [1]` turns it back on; the 3.0
  stop sequence must do this (or, as an opt-in setting, change `0x15`) so the 2.0 default — the loop keeps playing — holds.
- `0x03 [n]` sets the stored brightness (info `brightness`); the vendor always sent 95. Not needed for live.
- No raw-JPEG mode: bare JPEG bytes are ignored.
- Sensors in the vendor app: bundled `HWiNFO64.dll` via a .NET helper writing JSON to `%TEMP%\LOVINGCOOL_MONITOR_INTERNAL.bin`.
  Music: Windows SMTC (vendor feature disabled for this brand).

### Live-mode trials, round 2 (2026-10-12)

- **Orientation solved:** frames must be portrait 756x1680 — landscape content rotated 90° clockwise (like the clips).
  User: "Correct: readable, right way up", fills the screen, all edge words readable.
- **Tearing is a panel limit:** user: "The line running left to right is always smooth, the line running top to bottom
  is always tearing" (at 15–60 fps). The panel has no vsync for live frames; motion along the landscape vertical axis
  tears. Design widgets so moving things move sideways (or fade), not up/down. Vendor default is 30 fps, ≤ 200 KB.
- **Black screen after live solved (measured with `cs2 info` polling):** with the device's previously stored `0x15`
  value, the display switched off ~57 s after a stream ended (once; a later `0x0E [1]` + `0x14 [1,0]` stayed on for
  3+ min). After sending the vendor's own `0x15 [00 0A]` once, the display stayed on (86 s and 97 s checks, the second
  without sending 0x15 again), so the value is stored. Reading: `0x15 [enable=0, 10]` = don't switch off after live.
  The companion sends `0x15 [00 0A]` on connect like the vendor (2.0 default: the loop keeps playing); an opt-in
  "turn the screen off N s after the companion stops" setting would send `0x15 [1, N]` (to be verified).

### Prototype A (separate `cs-companion` tray program) on hardware (2026-10-12)

User check: readable, "looks pretty good", no flickering; the notch hole shows (mask not yet applied); tray Quit
returns to the loop clip. Measured on the screen (20 s per run, 16 logical cores):

| Background | fps setting | Companion CPU (% of PC) | RAM | ffmpeg | Frames that change |
|---|---|---|---|---|---|
| gradient | 30 | 0.89 % | 43 MB | — | 11/s |
| gradient | 60 | 0.70 % | 43 MB | — | 11/s |
| video (Ocean Echo) | 30 | 1.62 % | 54 MB | ~2.7 % of PC, 226 MB | 19/s |
| video (Ocean Echo) | 60 | 2.55 % | 54 MB | ~2.4 % of PC, 236 MB | 28/s |

No send errors; encode ~8 ms, send ~4–10 ms per frame. Video is limited by software decode + JPEG encode; next:
hardware decode in ffmpeg, check source fps/pacing.

### A vs B on hardware (2026-10-12, gradient background, 20 s per row, % of whole PC / private RAM)

| Form | 30 fps | 60 fps | Idle, live off |
|---|---|---|---|
| A: separate `cs-companion` | 0.70 % / 35 MB | 0.78 % / 35 MB | — |
| B: app, window visible | 0.99 % / 240 MB | 0.85 % / 241 MB | 0.10 % / 223 MB |
| B: app hidden, WebView alive | 0.84 % / 237 MB | 0.87 % / 237 MB | 0.08 % / 223 MB |
| B: app, window destroyed | 0.96 % / 41 MB | 0.84 % / 41 MB | 0.08 % / 28 MB |

A with the GPU-decoded video background (Ocean Echo, 18 fps): 1.8–1.9 % of PC / 46 MB, plus ffmpeg 11–14 % of one
core and ~108 MB. No send errors anywhere; the display stayed on afterwards.
Conclusion: CPU is the same in every form; only the hidden-but-alive WebView costs memory (~200 MB). B with the
window destroyed matches A, and keeps one program owning the screen (no device clash between two programs).
