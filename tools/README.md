# Developer and recovery tools

Small scripts used while reverse-engineering the screen. They are not part of the app. Run them
with Node from the project folder (PowerShell: one command per line, no `&&`). Close
CoolerScreen first, since only one program can talk to the screen at a time.

| Script | What it does |
|---|---|
| `winusb.js info\|restart\|play [path]\|desc` | Talk to the screen over WinUSB (after the one-time setup). |
| `probe.js`, `send.js <key-hex> [bytes]` | Same over the COM port, for a screen still on the standard serial driver. |
| `unstick.js` | Feeds filler bytes when an interrupted upload left the screen waiting for data. |
| `recover-race-delete.js` | **Recovery.** If an unplayable loop clip freezes the screen on every boot, this restarts it and deletes `/data/video/sleep_video.mp4` in the window right after boot. Then send a clip with the app. |
| `delete-ota.js` | Deletes the leftover `/data/ota.cpio` firmware package (about 0.68 MB) from the screen. Keep a copy of the vendor's `APP_OTA_*.bin` first. |
| `cycle-port.ps1` | Power-cycles the screen's USB port (admin). The app's Reconnect button does the same. |
| `make-calibration.js`, `make-notch-ruler.js` | Build the calibration clips used to find the orientation and notch position. |

Findings behind these (orientation, 18 fps playback, DTR watchdog, WinUSB fix, H.264 freeze) are
summarised in the main README under "For developers".
