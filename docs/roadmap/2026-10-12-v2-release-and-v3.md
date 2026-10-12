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
- **Companion form:** undecided. Prototype both (small separate helper vs the main app hidden in the tray) and measure
  before choosing.
- **Branch:** `v3`; `main` stays 2.0.
- **Vendor app:** already uninstalled (its folder is empty), so the live protocol is tested directly. Working
  hypothesis: live mode is raw JPEG frames (FF D8 … FF D9) written straight to bulk OUT 0x02, ended by
  `FF D9 FF D9` + `00 00 00 00`.
