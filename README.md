# CoolerScreen

A free, unofficial replacement for the "LOVINGCOOL MONITOR" software for the **Lovingcool SC70 AIO cooler's 6.67" USB screen**.
Pick any video, GIF or image on your PC, and CoolerScreen converts it and stores it **inside the screen itself**, where it
loops forever. You only need the app while you are changing the picture.

**Optional since 3.0: a live screen.** Turn **Live** on and CoolerScreen also draws a clock, music, PC stats, weather and
more on top of your picture while it runs. It is **off by default**.

**Engine 4.24.0:** the keep-display-on setting (0x15 [00 0A]) is sent to the screen only once; later Live starts leave it out (record: `live-ending-stored.txt`). Every screen hang started right after it.

**Engine 4.23.0:** after a Live stop, the first new Live connect waits 30 s so the screen can settle (setting `settleAfterStopSeconds`, 0 to 120 s, 0 = off), and the enter-Live write may take 2 s. Quick reconnects after Unified Control updates had made the screen hang. USB commands and their order are unchanged.

**Engine 4.22.0:** crash recovery as in Unified Control 0.10.0. A crash in one job, request or live data source is logged in one short device.log line and recovered; locks survive it. In UC's cooler worker an unguarded crash still ends the worker so UC restarts it cleanly. USB commands and timings are unchanged.

**UC engine4.21.1:** buffered finite Live connection/stop step diagnostics are sent privately to UC after each transaction. UC owns the diagnostics opt-out and bounded local persistence. Commands, delays, timeouts and recovery policy remain unchanged; successful writes do not establish visible playback. The standalone installation is untouched.

**UC engine4.21.0:** named dock controls select/cycle Live themes, set/toggle Live and change screen brightness.
Theme actions never store Embedded content. Busy work is refused rather than queued for later execution. Brightness
uses a dedicated serialized device operation, fresh readback for relative steps, and distinct requested/reported values;
unknown or mismatched readback is explicitly unconfirmed. The existing Screen settings shows the reported brightness.
This milestone is bundled by UC and does not replace the standalone installation. Physical result/feel remains separate acceptance.

**UC engine4.20.0:** shared-services mode consumes UC-supervised music and weather observations as well as shared hardware.
A separate provider-only launch of the bundled worker opens no device, window or media server. It reads only requested
media sessions and explicitly selected weather places; idle source interests stop while the bounded weather cache stays
in memory until UC quits. Original playback/pause timing, artwork and weather observation times survive delivery.
Standalone mode retains its own readers; this engine milestone does not replace the standalone installation.

**UC engine4.19.0:** when launched by Unified Control in shared-hardware mode, visible sensor widgets consume UC's
single timestamped collector. Replaced local aggregate CPU/RAM/NVIDIA readers stay off; missing/expired data stays unknown.
Standalone mode retains its own readers. This milestone updates the UC engine, without replacing the standalone install.

**Connection reliability (4.18.1):** Live reports the current connection failure
and retries when the screen returns. It checks for a real firmware response before
starting and permits one safe firmware recovery attempt through the device queue.
Reconnect needs Windows approval and resets only the screen's identified USB port.
If Windows still reports a descriptor failure, a physical USB reconnect may be
needed. See [verification and limits](docs/v4/notes/connection-reliability.md).

**What it does NOT do (by default):** it installs no service, no startup entry and no background program, and never needs
administrator rights after the one-time setup. With Live off, closing it leaves the screen playing on its own, even after
you restart or shut down the PC. With Live on it stays visible in the tray and **Quit** in its menu stops it.

## Quick start (Windows 10/11, no programming needed)

1. **Close and uninstall LOVINGCOOL MONITOR.** (Only one program can talk to the screen at a time, and after step 4 the
   vendor app can no longer see the screen at all. You can undo this later, see "Undo".)
2. **Download** `CoolerScreen_<version>_x64-setup.exe` from the [Releases page](https://github.com/oldbugo/coolerscreen/releases).
   **Before running it:** right-click the file > **Properties** > tick **Unblock** > OK (Windows tags downloaded files).
   *Prefer no installer?* Download `CoolerScreen-<version>-win-x64.zip` instead, unblock it the same way, extract it
   somewhere you will find it again, and run `CoolerScreen.exe` from there.
3. **Run the setup.** It installs for your user only (no administrator prompt) into `%LOCALAPPDATA%\CoolerScreen`, adds a
   Start Menu shortcut and offers a desktop shortcut. The app is **not code-signed**, so Windows may say "Windows protected
   your PC": choose **More info > Run anyway**. (If Smart App Control blocks it with no way to continue, use the zip, or turn
   Smart App Control off in Windows Security if you are comfortable with that.) **Upgrading from CoolerScreen 1.x:** just run
   the setup. It removes the old program files (close the old app first), points your existing Start Menu / Desktop
   "CoolerScreen" shortcut at the new app, and leaves your clips, versions and settings alone.
4. **Open CoolerScreen and click "Set up screen"** in the yellow bar at the top (shown until the setup is done) and approve the
   Windows prompt. This is needed only once per PC.
   - *What it does:* switches the screen from Windows' standard serial (COM port) driver to **Microsoft's built-in WinUSB
     driver**. Only the screen's own USB entry is changed, nothing else on your PC.
   - *Why:* the screen's firmware turns its display off about 30 seconds after Windows closes the serial port. With
     WinUSB that never happens, so you do not need to keep any program running.
5. **Add** a video, GIF, image or vendor theme `.ht` file (drop it on the window or click **Add clip**) and open it. CoolerScreen
   finds the smoothest loops in it and picks the best one (click another card to change it); adjust size and position if you
   like, then press **Push to screen**. The upload takes **about 3 minutes** for a full-size clip (the screen's built-in
   memory writes slowly); the screen restarts and shows the clip when it ends.
6. **Close the app** (close the window). Done: the screen loops your clip forever.

To change the picture later, just start the app again. To uninstall: Windows Settings > Apps > CoolerScreen (your clips and
settings are kept).

## What's new in 3.0

- **Themes:** a theme is a stored **base** loop plus an optional **live layer**. Without the app the screen plays the base;
  with the app running you also see the live layer, and the hand-over between the two is seamless.
- **On screen editor:** drag clocks, sensors, weather and more onto the preview, with layers, groups and snapping.
- **Widgets:** clock, date, sensors (six styles), weather, audio visualizer, slideshow, info items, music, text, pictures.
- **Live screen** at 60 or 30 fps, in the tray, with opt-in start with Windows and sleep / lock / shutdown choices.
- **Screen fit, rounded corners and screen guides** so pictures fit what you can actually see.
- No Save buttons: changes are kept as you make them. Details in [CHANGELOG.md](CHANGELOG.md).

## What changed in 2.0

CoolerScreen 2.0 is a new engine under the same library; your clips, versions and settings carry over with no migration.

- **A native app instead of Electron + Node:** Rust and the system WebView2. There is no Node server, no browser mode and no
  bundled Chromium: the download is about a quarter of the size and it starts faster (memory use is about the same: most of it is WebView2).
- **Faster work:** thumbnails, loop search, estimates and conversion run in a Rust media pipeline around ffmpeg, scheduled
  by priority (the clip you are looking at first), several at a time, and cancelled when you move on.
- **Smoother previews:** previews, loop cards and thumbnails stream from a small local media server, so seeking and looping
  work on long clips.
- **A proper window:** seamless frameless look with Windows 11 Snap Layouts on the maximise button, remembers its size and
  position, and asks inside the app before closing during an upload.
- **Installer and portable zip**, a real app icon, and the recovery tools bundled as `cs2.exe`.
- **Smarter loops:** the loop finder scores every start and end as it will look on the screen, with a cross-fade where
  nothing repeats exactly (random particles, rain); loops are found in the background as soon as a clip arrives.
- **Automatic Wallpaper Engine recording:** stops once it holds a seamless loop, otherwise records 2 minutes.
- **Sources:** same built-in sources; plug-ins are now declarative `.json` files (no code), see [docs/sources.md](docs/sources.md).
- Gone: the browser fallback (`npm start`), the Electron app, the `coolerscreen` command line (use the app, or `cs2.exe` for
  recovery).

See [CHANGELOG.md](CHANGELOG.md) for the full list.

## Using it

Everything you put on the screen is a **theme**: a **base** (the loop stored on the screen, which keeps playing when
CoolerScreen is closed) plus an optional **live layer** (clock, music, PC stats, text, pictures that CoolerScreen draws on
top while it runs, if Live is on). The top bar has three pages:

- **On screen**: the theme that is on the screen now, as an editor. Drag items (Clock, Music, PC stats, Text, Picture) from
  the left onto the preview, or click them; move and resize them with the mouse; set their details on the right. Switch
  the preview between **With live layer**, **Base only** and **Hand-over** (how the live layer fades in, and what happens
  when CoolerScreen stops: by default it fades out onto the first frame of the stored loop, so nothing jumps). Edits are
  saved as you go and show on the live screen at once. Text and pictures can also be drawn into the stored base ("Also on
  the stored base"); changing those shows **Base needs re-storing**.
- **My themes**: every theme in your library as a picture (the base with its live layer), labelled "Base only" or
  "Base + live: clock · music · stats". Click one, then **Apply to screen**: a window that cannot be clicked past shows
  the steps (prepare the picture, store it on the screen, switch the live layer) with a progress bar; **Stop and keep the
  previous theme** ends it (the part that is writing to the screen finishes first, so the screen is never left half-written).
  **Base settings…** opens the loop finder, quality and saved versions of that theme's base picture (what "My clips" used
  to show). **Add file…** and dropping files work as before.
- **Browse**: sources you turned on (Wallpaper Engine, Lively, the LOVINGCOOL themes...). **Add to My themes** makes an
  independent copy; adding again makes "Name (2)".

In On screen, the **Layers** list works like a paint program: drag to reorder (top of the list = on top), Ctrl-click or
Shift-click to select several, **Ctrl+G** groups and **Ctrl+Shift+G** ungroups, **Ctrl+]** / **Ctrl+[** move forward or back,
the eye hides a layer or group, double-click renames, Delete removes. The **Live layer** switch turns every item on or
off (off = the screen shows only its base). Boxes **snap** gently to each other, the middle and the guides (hold Alt to
move freely; the Snap box in the bar and Settings > Themes set the default). **Guides** (hatched) show where the screen
curves and the hard-to-see areas, and a ⚠ marks items inside one. Calibrate them in Settings > **Screen guides**: with
Live on, the lines are drawn on the real screen while you move the sliders. Each theme can nudge them (Theme settings),
and has its own rounded-corner radius (Settings > Themes holds the default; 0 = square corners). **Base settings…**
lets you pick ANY clip as the theme's base (nothing is sent until you press "Store it on the screen").

The right of the top bar shows the screen's status together with the **Live** switch (the same as Settings > Live screen);
the gear opens **Settings**.

- **Sources** (Settings > Sources; all off until you turn them on, then they get a checkbox in Browse; see
  [docs/sources.md](docs/sources.md)):
  - **Lovingcool themes** and **Other brands (same platform)**: the vendor platform's free online theme libraries for this
    screen (about 50 + 31 themes). **Add to gallery** downloads the theme. Lists are cached for a day; themes remain theirs.
  - **Wallpaper Engine**: your Workshop wallpapers and own projects (read-only). Video wallpapers are copied into the Gallery
    folder; scene and web wallpapers are **recorded** (desktop app, Wallpaper Engine running): they open in a separate
    window for the length you choose (default 20 s), are recorded, and the window closes again. Your desktop wallpaper is
    not changed. (The wallpaper's own sound, if any, may play while it records.)
  - **Lively Wallpaper**: video, GIF and picture wallpapers from your Lively library are copied; web/app ones aren't supported.
  - **Add from link**: adds an "Add from link…" button for a direct link to a video, GIF, image or theme file (up to 500 MB).
  - **Plug-ins**: declarative `.json` files in `Documents\CoolerScreen\Sources` that describe a web API (no code; they stay off until you enable them, see the docs link above).
- **Adding clips:** videos, GIFs, images, the vendor app's downloaded **theme files (`.ht`)** and plain `.zip` files
  containing a video or image. Drop files anywhere on the window (several at once is fine) or use **+ Add clip**. Nothing
  is converted when you add a clip.
- **Gallery folder:** your clips are the files in `Documents\CoolerScreen\Gallery` ("Open Gallery folder"). Any file you
  put there appears in the app automatically; adding through the app copies the file there (same name, "name (2)" if
  taken). Renaming a clip in the app renames its file. The folder can be changed in Settings.
- **Loop:** when you open a clip, CoolerScreen searches the whole clip, frame by frame, for the places where it can
  loop most smoothly and shows the best ones as cards, each playing its own loop, with its length and "Seamless", "Nearly
  seamless" or "Small jump". The best one is chosen for you; click another card to choose it (the preview plays it and the
  line under the preview shows where it sits in the clip). "Shortest loop" / "Longest loop" (the ⓘ explains them; empty =
  the longest that looks good in the space on your screen) and "Search again" change the search. When the best join is not
  quite seamless, the end is cross-faded into the start automatically. The results are remembered. (Exact frame editing is
  still there for special cases: ⋯ > "Edit loop manually…".)
- **Size and position** on the screen (zoom, left-right, up-down, or drag the preview; "Fill the screen" is the default,
  "Show the whole picture" adds black bars) and **Rounded corners** with an adjustable radius (per version; they round the
  visible picture, so with the notch band on the left the left corners are rounded at the band's edge).
- **Quality** is always "Best that fits": CoolerScreen picks the finest compression that fills about 98% of the space on the
  screen (it says what it chose). The space is measured at each screen check (free storage + what the app's two clips
  occupy, minus the start-up logo and a 160 KB margin; about 10.9 MB on the tested screen; 9.5 MB minus the logo before
  the first check). If even the coarsest compression doesn't fit, it offers to shorten the loop.
- **Push to screen** (bottom right) puts the current loop on the screen: it saves it as a version (if it isn't one yet),
  prepares it and sends it, after one confirmation. Keep the app open until the bar at the bottom says it is done; the
  screen then restarts to show the new clip and the app checks that it answers. **Save as version** keeps a copy without
  sending it.
- **Versions** (the "Versions" button at the top right of a clip): every saved version with its picture, date and "On
  screen" badge, and Preview, Push to screen, Rename, Delete.
- **Frame rate:** the screen always plays about 18 frames per second, whatever the file says, so every version is made at
  18 fps and plays at real speed. Versions made by CoolerScreen 1.1 or earlier (other rates) are marked "plays too
  fast/slow – re-make"; **Re-make** creates a corrected copy with the same settings.
- **Deleting:** "Delete…" on a clip moves its file to the **Recycle Bin** and deletes its versions. If a file disappears
  from the Gallery folder some other way (deleted or moved in Explorer), the clip stays in the library marked "File
  missing" and its versions are kept and can still be sent; after 30 days the app offers to delete those versions (it never
  deletes them by itself). Renaming a file in Explorer keeps its versions.
- **Notch:** the panel has a camera punch-hole on the left edge. Settings > Notch blacks that area out (default: a black band,
  92 px, applied as 96 px to stay on the screen's 16 px compression grid). New versions use the new setting; older versions
  are marked "Made with an older notch setting" with a **Make updated copy** action.
- Versions made with a different notch setting are marked "Made with older screen settings – re-make".
- **Start-up logo:** the screen plays a short clip once at power-on before your clip loops. CoolerScreen puts a 1.5 s
  "CoolerScreen" logo (black, small centred wordmark) there the first time you send, and only re-sends it when it is not
  already there. It uses about 0.5 MB, so about 9 MB remain for your clip. There is nothing to set.
- **Autosave:** there are no Save buttons. Edits in On screen and in Settings (notch, screen fit, guides, corners, folders)
  are kept as you make them. Only changes that alter the stored clip (screen fit, corners, notch, text or pictures drawn
  into the base) show **Base needs re-storing**; live-only changes never do.
- **Screen status** follows the current Live connection. With Live off, the app
  checks once when it opens; while a missing-screen warning is visible it watches
  for the screen to return and checks it once. This does not repeatedly open a
  healthy screen. Settings > Screen > **Check again** performs a real check.

Settings and versions live in `%APPDATA%\CoolerScreen\`; your original clips are in the Gallery folder.

### Live screen (3.0, optional, off by default)

The **Live** switch in the top bar (or Settings > **Live screen**) makes the screen show the live layer of your On screen
theme (clock, music, PC stats, text, pictures) drawn by CoolerScreen over the theme's own base. Nothing changes until you
turn it on. What is shown, and how it fades in and out, is set per theme in On screen; Settings keeps the global choices.

- With it on, closing the window leaves CoolerScreen running in the tray (the icon near the clock). Its menu has Open,
  Pause, 30 / 60 frames per second (60 by default) and **Quit**. Quit stops live and the screen plays its normal clip again.
  "Free memory when the window is closed" (on by default once live is on) removes the window completely while it is in the
  tray, so the app uses about 40 MB instead of about 250 MB.
- Sending a clip, restarting or checking the screen pauses live and brings it back afterwards (the screen can only talk to
  one program at a time).
- Extra choices, all off or "normal clip" by default: start CoolerScreen hidden in the tray when Windows starts (a
  start-up entry that you can turn off here, in Task Manager > Startup, and that uninstalling removes); turn the screen off
  when the PC shuts down; and what the screen does when the PC sleeps or is locked (show its normal clip, keep live
  running, or turn off).
- Startup from a shortcut or script: `CoolerScreen.exe --tray` starts hidden in the tray with live on.
- Live uses about 0.7 to 1.2% of the PC's CPU with a plain background and about 2 to 4% with video and many widgets, and
  about 45 to 90 MB of memory (more while the window is open).

### Screen fit (3.0)

The panel hides a few pixels at its edges. Settings > **Screen fit** has Top / Right / Bottom / Left (and a shift of the
whole picture): the screen shows rulers while you adjust, and the real picture follows at once. Pictures and live frames
are then fitted into what you can see, and rounded corners follow the same visible area. Changing it makes stored
pictures need re-storing (a banner and **Store now** offer it). Nothing to do if you are happy with the edges.

### Weather (3.0)

Settings > **Weather**: type a city and pick it; choose °C or °F. Data comes from Open-Meteo (free, no key, only asked
for the city you pick, never from your IP address). Without a city the Weather widget stays empty. Offline, the last
value is shown greyed.

### Data sources and no administrator (3.0)

Everything below works without administrator rights: CPU load, RAM, disks, network (Windows counters), NVIDIA GPU
readings (from the driver), the music that is playing (Windows media controls: Spotify, browsers, VLC, ...), and the
sound spectrum (Windows audio loopback, no microphone). **CPU temperature is not available yet:** Windows gives no
reliable reading without a special driver. A later version may read it from HWiNFO or LibreHardwareMonitor if you run
one yourself; CoolerScreen will never install a driver or ask for administrator rights for it. A missing reading shows a
grey dash.


## Undo / restore the original driver

In the app: **Settings (gear) > Advanced > Restore original driver** (approve the Windows prompt). Windows installs the normal COM-port
driver again and LOVINGCOOL MONITOR can see the screen again. (The screen will then switch its display off 30 s after any
program closes its port. That is how the vendor software works, which is why it must stay running.)

## Troubleshooting

| Problem | What to do |
|---|---|
| Windows cannot recognise the USB connection, or Device Manager shows "Unknown USB Device (Device Descriptor Request Failed)" | Embedded can keep playing while the USB connection is unavailable. Click **Reconnect** and approve the Windows prompt. It resets only one port matching the screen's remembered location. If the screen still does not return, physically reconnect the screen's USB cable when safe; a port reset may not remove its power. |
| "Screen not responding" (it keeps playing but doesn't answer) | Seen after the screen has played for a while. Click **Restart screen** (top banner, or Settings > Screen): it sends the screen's restart command, waits for it to come back and checks it. The app also does this by itself once when a check or the start of an upload gets no reply (never in the middle of an upload). If the screen doesn't come back, use **Reconnect**. |
| "Screen not found" | Check the USB cable/header and that the cooler has power. Unplug and replug USB, then **Check again** (click the status pill at the top). |
| Banner says the screen "needs setup" again | The driver was reset (moving the internal USB header to another port, or a big Windows update). Click "Set up screen" again. |
| Screen goes dark about 30 s after sending | The one-time setup was not applied (or was undone). Run "Set up screen". |
| "Screen busy" / "in use by ..." | Only one program can use the screen at a time. The message names who has it (another CoolerScreen, LOVINGCOOL MONITOR, a tool, or something else); close that program and CoolerScreen retries by itself every few seconds. If it says CoolerScreen itself, live is running: use the tray menu or the Live switch to pause it. `device.log` (in `%APPDATA%CoolerScreen`) records the reason. |
| Live is on and the window is closed: how do I quit? | Right-click the CoolerScreen icon in the tray (near the clock, maybe under the ^ arrow) > **Quit**. The screen then plays its stored clip. |
| A widget shows a grey dash | That reading is not available (CPU temperature, or no NVIDIA GPU). Pick another reading. |
| "Could not open the screen" / port busy | Close LOVINGCOOL MONITOR (and anything else using the screen). If you ran setup, the vendor app will not work anymore. |
| Clip is "too big for the screen" | Choose a shorter part or "Best that fits". The screen only has about 9 MB for your clip. Garbage on screen means an oversize file got through, so make a smaller version. |
| Clip plays too fast or too slow | It was made by an older CoolerScreen at a different frame rate. Use **Re-make** on that version. |
| The setup says it failed | Read `%TEMP%\coolerscreen-setup.log` and open an issue with its contents. |
| The window is blank or "must be started from its own window" | CoolerScreen needs Microsoft's WebView2 runtime (part of Windows 11 and of current Windows 10). Install the "Evergreen Bootstrapper" from Microsoft's WebView2 page, then start the app again. |
| The window opened off-screen after unplugging a monitor | Delete `%APPDATA%\CoolerScreen\window.json`; the window then opens at its default place. (Positions outside every connected monitor are ignored automatically.) |

### Recovery tools (stuck screen)

If an upload was interrupted and the screen now freezes, shows a broken picture, or never answers, close CoolerScreen and
use `cs2.exe` from its folder (`%LOCALAPPDATA%\CoolerScreen` for the setup, or the extracted zip). In PowerShell, one command
per line:

    cd $env:LOCALAPPDATA\CoolerScreen
    .\cs2.exe info                  # does the screen answer? (restarts it once if not)
    .\cs2.exe restart               # restart it and wait for it to come back
    .\cs2.exe unstick               # an upload stopped halfway and the screen waits for the rest of the data
    .\cs2.exe recover-race-delete   # a clip that cannot be played freezes the screen at every boot: deletes it right after boot

Then send a clip with the app. More detail in [tools/README.md](tools/README.md).

### Screen stays on when the PC is off

Many motherboards keep the USB ports powered while the PC is shut down, so the screen keeps showing its loop. The vendor
app used to switch the screen off at shutdown, which needs a running program. With Live off CoolerScreen runs nothing,
so it cannot do that; with Live on you can choose "turn the screen off at shutdown" in Settings > Live screen (off by
default). The no-software fix is in the BIOS/UEFI: enable **ErP Ready** / **ErP** (Gigabyte "ErP",
ASUS "ErP Ready", MSI "EuP 2013") or disable **USB power in S4/S5** (names vary by board). Then the USB ports lose power
when the PC is off and the screen turns off with it. Note: this also stops USB charging (phones etc.) while the PC is off.

## For developers

### Build

Needs Windows 10/11, [Rust](https://rustup.rs) (stable, MSVC toolchain), Node.js 18+ (only to fetch the Tauri CLI and
`ffmpeg.exe`) and the WebView2 runtime.

    git clone https://github.com/oldbugo/coolerscreen
    cd coolerscreen
    npm install                     # Tauri CLI + ffmpeg-static (ffmpeg.exe, bundled into the app)
    npx tauri dev                   # run the app with the frontend from public/
    cargo test --manifest-path src-tauri/Cargo.toml     # unit tests (no screen needed)
    powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1   # installer + portable zip in dist\

npm 11 asks you to approve dependency install scripts; `package.json` already lists `ffmpeg-static` under `allowScripts`.
If `node_modules\ffmpeg-static\ffmpeg.exe` is missing afterwards, run `npm approve-scripts` and reinstall.
`scripts\install-local.ps1` installs your own build for the current user (the same place and shortcuts the old 1.x local
install used); `scripts\uninstall-local.ps1` undoes it.

Testing without a screen: set `COOLERSCREEN_FAKE_DEVICE=1` (or `winusb`, `serial`, `none`, `failedusb`, `busy`, `noreply`,
`noreply-stuck` to simulate those states; `COOLERSCREEN_FAKE_FAIL=1` makes uploads fail halfway). Nothing then touches
USB, the COM port or the driver scripts. `COOLERSCREEN_HOME` and `COOLERSCREEN_GALLERY` point the library and the Gallery
folder at scratch folders (always do this when experimenting: the default is your real library).

### Architecture

    public/            the UI (plain JS modules, no build step); js/backend.js is its only door to the core
    src-tauri/
      src/main.rs      Tauri shell: registers the commands, starts the core and the media server
      src/window/      frameless window: title bar, Snap Layouts overlay (snap.rs), remembered rectangle (rect.rs)
      src/core/        the application core: library on disk, settings, Gallery folder watcher, scheduler, jobs,
                       device service, media server, commands (commands.rs) and events
      src/media/       the media engine around ffmpeg: probe, assets, loop search, estimate, convert (MJPEG for the screen)
      src/sources/     Browse sources: Lovingcool themes, other brands, Wallpaper Engine, Lively, link, JSON plug-ins
      src/device/      the screen: framing, WinUSB (nusb) and COM-port links, upload, restart
      src/bin/cs2.rs   command-line hardware tests and recovery tools
      installer-hooks.nsh   NSIS hooks (replaces a 1.x install)
    scripts/           one-time driver scripts (bundled), make-release.ps1, install-local.ps1
    docs/              v2/api-contract.md, sources.md, roadmap/, design/

- **Commands and events.** The UI calls Tauri commands (`original_get`, `version_send`, ... full table in
  [docs/v2/api-contract.md](docs/v2/api-contract.md)) and receives `cs:job`, `cs:library`, `cs:device` and `cs:app` events;
  nothing polls. Long work returns a job id immediately.
- **Scheduler.** Every job asks for units of resource classes (cpu, gpu, device, net, capture, io) and has a priority
  (user > selected > visible > background). `set_focus` re-ranks queued work to the clip on screen; preemptible background
  work yields to selected work; leaving a clip cancels its loop search; the screen is only ever touched by the one task that
  holds `device`, and an upload is never cancelled. On exit all jobs are cancelled and their ffmpeg children end.
- **Media server.** A loopback HTTP server on a random port with a random token in every path serves thumbnails, frames,
  loop cards and previews with `Range` support (so `<video>` seeks and loops); the UI gets absolute URLs from the core.
- **Window.** Frameless with a drawn title bar. Windows 11 shows Snap Layouts only when `WM_NCHITTEST` answers
  `HTMAXBUTTON`, but WebView2 covers the window, so a tiny invisible native child window over the maximise button answers
  that and forwards its mouse state to the page.
- **Data** is unchanged from 1.x: `%APPDATA%\CoolerScreen\library\<clip id>\original.json` + `versions\<version id>\`, settings in
  `settings.json`, the Gallery folder in `Documents\CoolerScreen\Gallery`.

Related reading: [docs/roadmap/phase0-results.md](docs/roadmap/phase0-results.md) (measurements behind the move to Rust),
[docs/roadmap/2026-10-11-next-iterations.md](docs/roadmap/2026-10-11-next-iterations.md), [docs/design/](docs/design/),
[docs/sources.md](docs/sources.md).


## Protocol and conversion notes

### How conversion works

The input is resampled to **18 fps** (the screen's fixed playback rate, measured at 17.6 and 18.0 fps; it ignores the file's
timing; timestamps are aligned so frame n is the source frame nearest start + n/18), framed on the 2400x1080 canvas (zoom
relative to "cover", then pad/crop with the x/y alignment; the window's preview uses the same arithmetic,
`public/js/frame.js`), optionally softened ("Simplify fine detail" = Gaussian blur, sigma up to 3.5 px; measured on an
anime-style theme it shrinks MJPEG frames far more per unit of visible change than hqdn3d or smartblur), optionally
loop-blended (`xfade` of the last B seconds with the B seconds before the start), the notch mask is applied, then (if on)
the rounded-corner mask (a cached 2400x1080 RGBA PNG per radius, 1 px anti-aliased edge, overlaid on every frame), then it
is rotated 90 degrees clockwise
(`transpose=1`) and stored as 1080x2400 portrait MJPEG in an `.mp4` without audio. In Auto quality the converter estimates the
size on a sample and picks the lowest JPEG quantiser (`-q:v` 3..31) that fits; if even q=31 does not fit it reports the
longest length that would. The 1-100 quality scale maps linearly to q 31..3. Defaults: from the start, up to 10 s. Still
images become a 3 s clip. Oversize files make the screen show garbage, so the size budget is enforced (the measured
capacity, or 9.5 MB before the first screen check, shared by the start-up logo and your clip).

Library layout: `%APPDATA%\CoolerScreen\library\<clip id>\original.json` (+ thumbnail, notch frame, and a browser proxy
for formats the window can't play) and `versions\<version id>\` (`device.mp4`, `preview.mp4`, `thumb.jpg`,
`version.json`). Clip ids are permanent. Libraries from 1.1 are migrated automatically on first start (each converted item
becomes a clip with one version; the old `meta.json` is kept as `meta.v1.json`).

### Protocol and firmware findings

Frame: `55 AA | len16LE (payload length + 7) | key | payload | sum16LE` (sum of all preceding bytes). Transport is a USB CDC
serial port (VID `33C3`, PID `7791` or `7788`, 2,000,000 baud), or, after setup, the same frames over the WinUSB bulk
endpoints of the CDC data interface (OUT `0x02`, IN `0x81`). On a serial connect the vendor app sends `FF D9 FF D9` and
`00 00 00 00` to end any live JPEG stream.

| Key | Meaning |
|-----|---------|
| `0x06` | info (JSON reply) |
| `0x12` | upload startup/intro clip (`/data/video/start_video.mp4`): 4-byte LE size, then the raw file in 40 KB chunks |
| `0x13` | upload standby/loop clip (`/data/video/sleep_video.mp4`), same scheme |
| `0x01` | restart |
| `0x03` | brightness |
| `0x0E` `[1/0]` | display on/off |
| `0x14` `[1,0]` | play the standby (loop) clip now, forever |
| `0x15` `[enable, seconds]` | after a *live PC stream* ends: play the loop clip for N s then turn off (saved in `/data/sleep_delay.txt`); does not affect the 30 s watchdog |
| `0x04` + ASCII path | play a file |
| `0x02` | stop playback |
| `0x07` | directory list |
| `0x22` | unsolicited status frame sent by the screen about every 2 s (ignore) |
| `0x05` | **DANGEROUS: deletes a file on the screen. Not used or exposed by this tool.** |

Boot sequence: the intro clip plays once, then the loop clip plays forever. **Watchdog:** when the host's USB serial
driver sets DTR low (it does on enumeration and on port close), the firmware turns the display off after 30 s. WinUSB never
sends that request, which is why the one-time setup fixes it. Setup is implemented in `scripts/setup-winusb.ps1` (SetupAPI:
device class -> USBDevice, install `C:\Windows\INF\winusb.inf` for `USB\VID_33C3&PID_7791&MI_00`, add a
`DeviceInterfaceGUIDs` value, restart the device); undo is `scripts/restore-serial.ps1` (`pnputil /remove-device`, `/scan-devices`).



## Tested hardware

Lovingcool SC70 AIO cooler screen, USB VID `33C3` PID `7791`, model string `D215-NOR-JD9365-6.67inch-hor`, firmware 3.6,
Windows 11. Start with Windows has not been tested over a real reboot. Other firmware versions and other Lovingcool-style screens are untested.

## Disclaimer

Unofficial, not affiliated with or endorsed by Lovingcool or the maker of the vendor software. Based on observing the
device's USB traffic. Replacing a USB driver and writing to the screen's storage carry some risk; use at your own risk,
no warranty. (Restoring the original driver is always possible, see "Undo".)

## License

MIT, (c) oldbugo. See `LICENSE` and `CHANGELOG.md`.
