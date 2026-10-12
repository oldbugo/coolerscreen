# CoolerScreen roadmap: finish 1.2, then 2.0 on a native shell

Written 2026-10-11. Inputs: the user's direction in the CoolerScreen session, and the
"Rhine Music Windows" session's conclusions saved in the knowledge vault
(`Reference/Desktop app shells 2026`, `Interface patterns/Seamless frameless window`,
vault 5aa0ec8). Estimates are estimates; nothing in 2.0 is built or measured yet.

## Now: finish 1.2 (current iteration)

1. Land the **Sources** round (unified Browse view; sources enabled in Settings: Lovingcool
   themes, other brands on the same platform, Wallpaper Engine incl. scene recording, Lively,
   Add from link, plug-in folder).
2. Hardware check on the real screen, install locally, user check.
3. **Clean-up:** remove stale experiment files that no longer apply (e.g. `work/`, old
   `scripts/start-browser.bat` if unused), keep `tools/` as documented dev/recovery tools
   (README section), drop test-only env hooks from user-facing docs, update README/CHANGELOG,
   bump to **1.2.0**.
4. Commit, push, tag `v1.2.0`, GitHub release with the zip (repo still private until the user
   decides).

## Next: 2.0 "native shell" (next iteration)

**Goal (user direction):** drop Electron; Tauri 2 + system WebView2 + Rust backend + seamless
frameless window. Keep the UI the user has approved; change what's underneath.

**Expected gain (estimate):** app folder from 454 MB to roughly 90 MB, almost all of it FFmpeg;
lower with a trimmed FFmpeg build. Faster start, less memory.

### Phase 0: spike and go/no-go (first, small)
- Tauri 2 skeleton that hosts today's `public/` UI unchanged, with the frameless window
  (decorations off, background colour matched, 32 px transparent HTML title bar, drag region,
  Windows 11-style buttons).
- **Port the device layer to Rust first**, because it is the riskiest part: WinUSB via `nusb`
  (claim interface 1, bulk OUT 0x02 / IN 0x81, 55 AA framing, ignore 0x22 status frames),
  info, upload with 20 s write timeouts, restart-and-wait, MJPEG-only guard, device log.
  Serial fallback via the `serialport` crate.
- **Test on the real cooler:** info, a full 10 MB upload, restart ending, recovery from
  "no reply", boot-time failed enumeration plus Reconnect.
- **Gate:** device parity on hardware, plus measured size and memory. If it fails, stay on
  Electron 1.x and revisit Electrobun or Deno desktop when they mature.

### Performance (user priority for 2.0; measured 2026-10-11)
Findings: `docs/perf/2026-10-11-performance-investigation.md`. The slowness is pipeline design,
not language: a single FIFO queue with no priority or cancellation (clicking through 5 clips
took 65 s and 214 ffmpeg runs for abandoned clips), plus redundant decode/scale work in the
estimate, loop check and Save as version. A Rust rewrite alone is about neutral. Phase 1 must
build in, in this order:
1. Scheduler with priority (selected clip first) and cancellation (kill work for clips the user
   left); separate limits for CPU encodes, GPU decodes and the exclusive screen upload.
2. Decode the selection once into raw frames (tag `-color_range pc`), then a two-point size
   model plus one parallel round for the exact quality; Save as version reuses the winning encode.
3. New-clip order: loop search first, estimate only the picked loop.
4. One-pass loop frames (192x108 and 64x36 together); loop cards rendered in parallel at 320x144.
5. NVDEC/NVENC for HEVC, 4K and long sources and for proxies (detect, fall back to CPU).
6. Scaler threads (identical output, 1.2–1.8x faster).
7. Rust/SIMD loop compute and Tauri events last (worth under 1 s).
Expected: opening a new clip 19–32 s → about 4–10 s; Save as version 7–12 s → about 0.4–2.7 s.

### Phase 1: port the backend
- Replace the local HTTP server with Tauri commands and events; the UI's `api()` layer moves
  to `invoke()`, with no visual change. Serve media with Tauri's asset protocol.
- Library, versions, migration (same `%APPDATA%\CoolerScreen` layout, so no user migration),
  queue, gallery folder watcher (`notify` crate), settings.
- Conversion: keep FFmpeg as a **sidecar** (same filter graphs). Investigate a trimmed FFmpeg
  build (decoders the sources need, plus mjpeg/libvpx/png encoders) to cut 79 MB.
- Loop finder in Rust (same algorithm; faster), Best that fits search, splash, masks.
- Sources: HTTP providers via `reqwest`; Wallpaper Engine scan; **scene recording via Windows
  Graphics Capture in Rust** (e.g. the `windows-capture` crate) instead of Electron's
  `desktopCapturer`; plug-in folder (needs a design decision: JS plug-ins would need a JS
  runtime; options are WASM plug-ins, a declarative JSON provider format, or dropping
  plug-ins).
- One-time setup, restore and reconnect keep the existing elevated PowerShell scripts, called
  from Rust.

### Phase 2: window polish
- Frameless window per the vault pattern. **Decision for the user:** custom HTML buttons
  (exact look, but no Snap Layouts hover menu) or native overlay buttons / `tauri-plugin-frame`
  (keeps Snap Layouts; claimed, not yet verified).
- Single instance, a "don't close during upload" guard, remember window size and position.

### Phase 3: packaging and Smart App Control

**User decision (2026-10-11): ship 2.0 unsigned for now; signing can be added at any time
later.** Evidence: Rhine Music's own `Rhine Music.exe` is unsigned and runs on this PC (extracted
without the internet mark). Later options: SignPath Foundation (free for public open-source
repos, automated GitHub build), Microsoft Artifact Signing (roughly US$10/month at launch;
eligibility for individuals in Australia unverified), or a traditional certificate.

Original risk notes:
- 1.x passes Smart App Control only because `CoolerScreen.exe` is the unmodified, reputable
  Electron runtime. **A new Rust exe (and its installer) is unsigned and unknown, so Smart App
  Control on this PC will very likely block it.**
- Options: (a) code signing, for example Microsoft Trusted Signing (Artifact Signing; cost and
  availability for an individual in Australia to be checked) or an OV certificate;
  (b) ship 2.0 to people without Smart App Control and keep 1.x as the fallback;
  (c) build locally on the user's PC (locally built binaries carry no internet mark, but Smart
  App Control may still block unknown binaries; must be tested in Phase 0).
- **Test this in Phase 0 with a hello-world Tauri exe on this PC before investing further.**

### Phase 4: parity and release
- Feature parity checklist against 1.2, a hardware regression pass (uploads, recovery, setup and
  restore, sources, Wallpaper Engine recording), size and memory measured, `v2.0.0` release.

## Open decisions for the user
1. Code signing (cost) versus accepting Smart App Control friction: decide after the Phase 0 test.
2. Custom title-bar buttons versus keeping Snap Layouts.
3. Plug-in sources in 2.0: WASM, declarative JSON, or drop.
4. Repo visibility (public) and when.
