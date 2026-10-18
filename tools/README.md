# Developer and recovery tools

## Recovery: `cs2.exe`

`cs2.exe` is installed next to `CoolerScreen.exe` (and is in the portable zip). It is a small command-line program for a
screen that an interrupted or bad upload left stuck. **Close CoolerScreen first**: only one program can talk to the screen at a
time. In PowerShell, from the install folder (`%LOCALAPPDATA%\CoolerScreen` for the setup .exe):

| Command | What it does |
|---|---|
| `.\cs2.exe info` | Asks the screen for its status (restarts it once if it does not answer) and prints it. |
| `.\cs2.exe state` | What is reachable (WinUSB / COM port), without sending anything. |
| `.\cs2.exe restart` | Restarts the screen and waits for it to come back. |
| `.\cs2.exe unstick` | Feeds filler bytes when an interrupted upload left the screen waiting for the rest of the data. |
| `.\cs2.exe recover-race-delete` | **Recovery.** If an unplayable loop clip freezes the screen on every boot, this restarts it and deletes `/data/video/sleep_video.mp4` in the window right after boot (before the player opens it). Then send a clip with the app. |
| `.\cs2.exe delete-ota` | Deletes the leftover `/data/ota.cpio` firmware package (about 0.68 MB). Keep a copy of the vendor's `APP_OTA_*.bin` first. |
| `.\cs2.exe upload <file.mp4> [--intro]` | Test upload of an MJPEG clip (developer use). |
| `.\cs2.exe poll <count> <seconds>` | `info` every few seconds (watchdog / stability tests). |
| `.\cs2.exe core-check`, `core-state`, `core-restart`, `core-send <library>/<oid>/versions/<vid>` | The same through the app's own core (set `COOLERSCREEN_HOME` to a scratch folder for `core-send`). |

Only `unstick`, `recover-race-delete` and `delete-ota` send commands the app itself never sends (the screen's delete
command); each touches exactly the one path named above.

## Scripts in this folder (not part of the app)

Small scripts used while reverse-engineering the screen. The 1.x Node tools that talked to the screen (`winusb.js`,
`probe.js`, `send.js`, `unstick.js`, `recover-race-delete.js`, `delete-ota.js`) were replaced by `cs2` and removed with
the Node backend; they are in the 1.2.0 release commit (`3eb5b69`).

| Script | What it does |
|---|---|
| `cycle-port.ps1` | Power-cycles the screen's USB port (admin). The app's Reconnect button does the same. |
| `make-calibration.js`, `make-notch-ruler.js` | Build the calibration clips used to find the orientation and notch position. Run with Node after `npm install` (they use `ffmpeg-static`). |

Findings behind these (orientation, 18 fps playback, DTR watchdog, WinUSB fix, H.264 freeze) are summarised in the main
README under "For developers" and in `docs/roadmap/phase0-results.md`.

## Live-mode developer commands (3.0)

| Command | What it does |
|---|---|
| `.\cs2.exe wake` | Display on and play the loop clip (after a live session left the screen dark). |
| `.\cs2.exe live-test …` / `trial` | Raw live-mode tests and the guided trials used to find the live protocol. |
| `.\cs2.exe edge-tune` | Ruler picture to measure hidden edges (the app's Settings → Screen fit does this now). |
| `.\cs2.exe core-apply <originalId> [--real]` | Runs the app's Apply job from the command line (refuses while the app runs). |
| `.\cs2.exe core-repair-plan [--real]` | Read-only: what the startup library repair would do. |
| `cs-companion.exe --help` | Prototype A / measuring tool for the live engine (`--measure`, `--dry-run`, `--theme-of`, `--dump-frames`). |

The double-click launchers `test-live.cmd` and `tune-edges.cmd` were removed in 3.0 (the app covers both).

## UI testing: always set COOLERSCREEN_TEST_QUIET=1

Every test or development run of the app (fake device, WebView2 debug port, helpers) must set `COOLERSCREEN_TEST_QUIET=1`
(any build). The window is then created without focus, out of the taskbar, positioned off-screen (-32000,-32000) and never
activated or brought to the front (also not by the tray or a second launch); its position is not remembered. WebView2 still
renders, so driving it over the debug port (and `Page.captureScreenshot`) works. Without it a test window can steal focus
from the user's full-screen application. Also set `COOLERSCREEN_HOME`, `COOLERSCREEN_GALLERY`, `COOLERSCREEN_FAKE_DEVICE=winusb`,
`COOLERSCREEN_MULTI=1` (debug builds) and a separate `WEBVIEW2_USER_DATA_FOLDER`.

## Quiet restart of the installed app: `--background`

`CoolerScreen.exe --background` starts without showing or activating anything: Live on (Settings) = tray only; Live off =
the window is built minimised without activation (taskbar button, easy to quit). A second launch from the Start menu still
brings the window up as usual, except a second `--background` launch, which does nothing.
