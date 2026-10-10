# Changelog

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
