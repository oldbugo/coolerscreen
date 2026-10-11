# Changelog

## Unreleased

- Simpler, "we pick for you" UI:
  - The app opens on a **Gallery** of all clips (large pictures, hover to play, status chips, Add clip tile); clicking a clip
    opens it, "‹ Gallery" goes back.
  - **Loop cards** are the only loop control: the whole-clip search runs automatically the first time a clip is opened
    (results remembered), the best loop is chosen, each card plays its own loop with "Seamless / Nearly seamless / Small
    jump"; a cross-fade is added automatically when the best join isn't seamless. "Shortest loop" / "Longest loop" with ⓘ
    tooltips. Manual frame editing only via ⋯ > "Edit loop manually…".
  - **Quality** is always "Best that fits" (no Custom/Simplify controls). **Versions** moved into a dropdown in the clip
    header; a bottom bar has "Save as version" and **"Push to screen"** (saves if needed, prepares, sends).
- Best that fits, re-benchmarked (SSIM vs the unblurred source at the same size, pink-blue 8.5 s / 4 s, Coast 8 s): one
  fixed JPEG quantiser with the range extended past 31 up to 69 and **no blur** beats blur + q<=31, 1- and 2-pass rate
  control and trellis. Auto now uses it and fills ~97-99% of the space (blur only as a last resort).

- "Find loop points": one button searches the whole clip, frame-accurate, for the smoothest start/end pairs within a
  loop-length range (motion across the join, cached small frames, leaders re-checked at higher resolution); the top 5
  with first/last frame thumbnails, length and seam score; "Use" applies one exactly.
- "Best that fits" chooses the JPEG quality and the fine-detail simplification together and shows what it chose; the
  Simplify slider is only in Custom.
- Rounded corners round the visible picture (inset by the notch band).
- Screen not responding: "Restart screen" (banner and Settings > Screen) sends the restart command, waits for the screen to
  come back and checks it; a check or upload start that gets no reply restarts the screen once automatically. A send now
  ends by restarting the screen (instead of the "play now" command, after which the screen once stopped answering) and a
  final check.

- Loop editing: frame-accurate start/end (source frame / screen frames), frame nudge buttons and arrow keys (Shift = 10),
  a filmstrip trim track with "Zoom to the selection", "play the seam" preview mode, a seam check (onion skin, side by
  side, difference, similarity score), "Find best loop point" (±2 s search), and "Blend the end into the start"
  (cross-fade, 0-1.5 s, mirrored in the preview).
- Rounded corners moved into Tune and are stored per version (the last choice is the default); the notch stays global.
  Versions made with a different notch are marked "Made with older screen settings – re-make".
- "Simplify fine detail" (Gaussian softening, mirrored in the preview) to fit longer parts; when even the lowest quality
  doesn't fit, the app suggests a level before suggesting a shorter part.
- Space on the screen is measured at each screen check (free storage + our two clips, minus the start-up logo and a 160 KB
  margin) instead of a fixed 9.5 MB.
- Fixed: the selected clip's view was rebuilt on every background refresh (lost open panels and focus).

- New window layout: Library on the left; preview, Tune and Versions on the right; screen status and "On screen now" in the
  top bar; one Activity bar for preparing and sending; Settings drawer (screen, notch, Gallery folder, advanced, about).
- Adding a clip no longer converts it. The preview plays immediately and matches the result (part, size and position,
  notch). New framing controls: zoom, left-right, up-down, drag to move, fill the screen or show the whole picture.
- Several versions per clip ("Save as version"), each sendable; clip ids are permanent (renames keep versions). Libraries
  from 1.1 are migrated automatically.
- Every version is made at 18 fps, the rate the screen actually plays at (it ignores file timing). Auto quality only lowers
  JPEG quality and says how long the part can be if it can't fit. The fps control is gone. Older versions made at other
  rates are marked "plays too fast/slow – re-make".
- The intro clip is no longer a user feature: the start-up slot holds a 1.5 s "CoolerScreen" logo (sent only when needed).
- Delete moves the file to the Recycle Bin and deletes its versions (in-app confirmation). Files that vanish from the
  Gallery folder are kept as "File missing" with their versions; after 30 days the app offers to delete those versions.
- The screen is checked once at launch (status pill + banners with one action each). Closing the window during an upload
  asks first. In-app dialogs replace all browser confirm/prompt boxes; keyboard and screen-reader basics.
- CLI: `list` shows clips and versions; new `add`, `make`, `send <version>`; `upload` does all three.

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
