# CoolerScreen: always-read guide

## Standing user instructions (read in full at every session start)

- 2026-10-10: Nothing hidden in the background. No Windows service; anything that runs must be visible and easy to quit.
- 2026-10-10: Never upload non-MJPEG files to the clip slots. Never send 0x05 delete, 0x0C OTA or 0x23 from the app.
- 2026-10-12: 3.0 defaults behave exactly like 2.0: "the first initial point would be to behave as per version 2". Every
  3.0 option is a setting the user can change: "it will be good if all of the things you're asking, all these options,
  are configurable". In particular: don't turn the screen off at shutdown by default; when the companion stops, the
  screen plays the stored loop clip; start-with-Windows is opt-in (off by default); no admin driver by default.
- 2026-10-12: Live frame rate is a setting with both 30 and 60 fps; default 60 ("if the screen supports it, that will
  be what I prefer ... maybe have both. So that I can pick and choose"). Goal includes live-rendered backgrounds
  "similar to Wallpaper Engine where it's not the actual video itself".
- 2026-10-12: 3.0 work happens on the `v3` branch; `main` stays the released 2.0 until the user approves.

Start-of-work reading: `docs/roadmap/2026-10-12-v2-release-and-v3.md` (2.0 state and the 3.0 brief).
