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
- 2026-10-17: One version only: "just that one version that we keep working on". Work on `main`; every build is
  installed as the user's CoolerScreen (no side-by-side test builds). The v2.0.0 tag/release is the fallback.
  (Supersedes 2026-10-12 "3.0 work on the v3 branch".)
- 2026-10-17: 3.0 is organised around THEMES with two faces: a live face (companion running) and a screen-only face
  (stored clip), both from the same theme, applied once from one place, with a seamless hand-over between them.
  The current MP4-centred UI must be redesigned for this; explore the design with the user before building.
- 2026-10-18: Claude may close the user's running CoolerScreen whenever needed (e.g. to install a build) without
  asking: "as long as you can make sure it's not in the process of doing anything" — check first that no job
  (upload, conversion, apply) is running (device.log / activity); prefer a clean quit so live hands back properly.
- 2026-10-18: No Save buttons. Every change is saved as it is made and applies to the live picture at once;
  closing the window or quitting to the tray changes nothing ("I don't wanna have another save"). Only changes that
  affect the stored clip (screen fit/shift, corner radius, notch, baked text/pictures, base clip/version) mark the
  base as needing to be stored again; live-only changes never do ("otherwise ... it shouldn't do anything").
- 2026-10-18: Record user flows every time they are discussed: "every time we talk about it, we should be noting it
  down and slowly building up a repository of the user flow ... within the vault". Vault page: Projects/CoolerScreen
  user flows (see the vault). Add/adjust the flow in the same turn the user describes it.
- 2026-10-18: Choosing a theme in My themes = "Use this theme": the live picture switches immediately; the screen's
  stored base is NOT changed by that — make that clear and offer storing it separately (user's hardware: top edge 6 px
  is the user's own value, not a product default; other users tune their own).

Start-of-work reading: `docs/roadmap/2026-10-12-v2-release-and-v3.md` (2.0 state and the 3.0 brief).
