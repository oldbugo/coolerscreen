# CoolerScreen 2.0, Phase 0 spike: results

Measured 2026-10-11 on the user's PC (Windows 11 Home 26200, Smart App Control **On**,
`VerifiedAndReputablePolicyState=1`), Rust 1.99, Tauri 2.12.3, WebView2 154. Branch `v2`.

## Gate results

| Gate | Result |
| --- | --- |
| Smart App Control, unsigned Tauri exe, run as built | **Launches** (window shown, no Code Integrity block event) |
| Same exe copy with `Zone.Identifier` ZoneId=3 (internet mark) | **Launches** (hello world, and the full shell exe on its first ever run, so no cached verdict) |
| Unsigned NSIS installer (1.5 MB) with ZoneId=3 | **Launches** (setup wizard shown; closed without installing) |
| Device parity on hardware (WinUSB via `nusb`) | **Pass**: info, 8.8 MB loop upload, restart-and-wait, 10 polls over 4.5 min, Tauri-command restart |
| Size | **Pass**: shell exe 4.5 MB, installer 1.5 MB (no FFmpeg, no Node yet) |
| Memory | **No gain yet**: see below; WebView2 costs about what Electron's Chromium costs |

SAC caveat: this PC's SAC let unsigned, unknown binaries with an internet mark run. That is what
we saw here; it may differ on other PCs or once a real download (browser, SmartScreen) is involved.

## Measurements

Both apps run with the fake-device hook (`COOLERSCREEN_FAKE_DEVICE=1`) and a scratch
`COOLERSCREEN_HOME`, 3 runs each, process tree summed after 20 s idle on the Browse view.

| | Tauri spike (shell + WebView2 + Node sidecar) | Electron 1.2 (installed) |
| --- | --- | --- |
| App on disk | exe 4.5 MB (+ system `node.exe` 88.5 MB, not bundled yet) | 452 MB folder |
| Installer | 1.5 MB NSIS | about 190 MB zip (1.1.0 figure) |
| Window visible after launch | 119-187 ms (boot page) | 462-744 ms |
| App UI loaded | 811-1087 ms (sidecar start + page) | same as window (shown on ready) |
| Idle working set (tree) | 459-466 MB | 355-366 MB |
| Idle private bytes (tree) | 266-271 MB | 235-237 MB |

Breakdown (Tauri): shell 35 MB WS; WebView2 browser 117, GPU 79, renderer 103, 2 utilities 63,
crashpad 13; Node sidecar 54; conhost 8. Removing the sidecar in Phase 1 brings it to roughly
Electron's level (about 410 MB WS / 215 MB private). WebView2 working set partly counts runtime
DLL pages shared with Edge. **Conclusion: 2.0's win is disk size and start time, not RAM.**

## Hardware test (real SC70, WinUSB)

- `cs2 state` / `cs2 info`: opened over WinUSB with `nusb` 0.2.7 (interface 1, bulk OUT 0x02 /
  IN 0x81, found through the WinUSB associated-interface handle), info in 337 ms.
- `cs2 upload` of `粉蓝梦境` version `mv3kd5v3cbc1a5` (8,774,966 bytes, MJPEG) to the loop slot
  (0x13): ack `{"key":19,"hex":"00"}`, streamed in 169.9 s = **51.7 KB/s** (Node 1.2 from the same
  device.log: 52.1-53.9 KB/s over 8 uploads; the screen's flash is the limit). No "done" reply
  arrives for long uploads (same as Node). Then restart (0x01), back and answering on the first
  check: total 177.4 s. Free blocks afterwards 4480, matching the earlier state with this clip.
- 10 info polls 30 s apart: 10/10 ok, 323-383 ms each, display on.
- Inside the Tauri shell: `device_restart` command (restart + wait + info) ok; the close guard
  held the window while it ran.
- Not tested: "no reply" recovery (cannot be provoked safely), boot-time failed enumeration plus
  Reconnect, the COM-port fallback (screen is on WinUSB; code is ported and compiles).

## What was ported (`src-tauri/src/device/`)

- `frame.rs`: 55 AA framing and reply parser (resync, drop len < 7, skip 0x22 status frames,
  JSON or `{key, hex}`); unit tests.
- `mjpeg.rs`: `isMjpegMp4` exactly (top-level `moov` scan). Verdicts identical to Node on all 84
  media files in the library (12 MJPEG accepted).
- `usb.rs`: `nusb` WinUSB, 3 x 4 KB IN transfers kept pending in a reader thread, 40 KB OUT
  transfers with a 20 s timeout. `serial.rs`: `serialport` fallback (2 Mbaud, DTR on, reset bytes).
- `mod.rs` / `ops.rs`: info, upload with progress, restart-and-wait (25 s), open-retry, check with
  one auto-restart on "no reply", `sendFiles` flow (retry once on no-reply before streaming,
  restart at the end, warnings vs failures), failed-USB detection, friendly error codes.
  A key allow-list means 0x05, 0x0C and 0x23 can never be sent.
- `log.rs`: `%APPDATA%\CoolerScreen\device.log`, same line format and rotation (one extra line
  type: `upload of N bytes streamed in M ms, done -> ...`).
- CLI `cs2 info | state | restart | upload <file> [--intro] | poll <n> <s> | check <files>`.

## Shell (`src-tauri/`)

- Hosts `public/` unchanged by running the existing Node backend as a **sidecar**
  (`node -e ... start({port:0})`, system Node, repo path from the build or `COOLERSCREEN_REPO`).
  The sidecar exits when the shell exits, including a hard kill (stdin EOF). Device commands are
  also exposed natively: `device_state`, `device_info`, `device_restart`, `device_upload`.
- Seamless frameless window per the vault pattern: decorations off, background `#0f1115`, an
  injected 32 px transparent title bar with 46 x 32 buttons (close hover `#c42b1c`, pressed
  `#a82116`), the app's own top bar as drag region, maximise fills the work area exactly
  (5120 x 1392, taskbar visible), `window-state` event flips the icon, F11 toggles.
- Single instance (second launch exits, focuses the first), close guard (native device operation
  or a Node send/setup job running; asks before closing), HTML5 file drop left to the page.
- The Electron app is untouched and still works side by side.

## What's left (Phase 1+)

Replace the sidecar with Tauri commands (library, queue, conversion via an FFmpeg sidecar,
sources, setup scripts), move `api()` to `invoke()`, asset protocol for media, then packaging.
The spike reads `node_modules` through a local junction to the main checkout (not committed).

## Risks found

1. RAM is not lower than Electron (see above); only size and start time improve.
2. Custom title-bar buttons: no Snap Layouts hover menu (expected; decision still open).
3. The title bar overlays the top-right of the page; the shell pads `.topbar` by 138 px from
   the injected CSS. Any future full-bleed page header must leave that corner free.
4. Throughput about 3% below Node (one 40 KB transfer in flight, same as Node); not worth a
   riskier pipelined write on this device.
5. SAC behaviour was checked on this PC only.
