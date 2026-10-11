# CoolerScreen

A free, unofficial replacement for the "LOVINGCOOL MONITOR" software for the **Lovingcool SC70 AIO cooler's 6.67" USB screen**.
Pick any video, GIF or image on your PC, and CoolerScreen converts it and stores it **inside the screen itself**, where it
loops forever. You only need the app while you are changing the picture.

**What it does NOT do:** it installs no service, no startup entry and no background program. When you close it, the screen
keeps playing on its own, even after you restart or shut down the PC.

## Quick start (Windows, no programming needed)

1. **Close and uninstall LOVINGCOOL MONITOR.** (Only one program can talk to the screen at a time, and after step 4 the
   vendor app can no longer see the screen at all. You can undo this later, see "Undo".)
2. **Download** `CoolerScreen-<version>-win-x64.zip` from the [Releases page](https://github.com/oldbugo/coolerscreen/releases).
   **Before extracting:** right-click the zip > **Properties** > tick **Unblock** > OK (Windows tags downloaded files).
   Then **extract** it (right click > Extract All) somewhere you will find it again, for example your Documents folder.
3. **Double-click `CoolerScreen.exe`.** The CoolerScreen window opens. The app is **not code-signed**: Smart App Control
   normally allows it because it is the unmodified official Electron runtime. If Windows still blocks it, choose
   **More info > Run anyway**; if that is not offered, use the browser fallback below.
4. **Click "Set up screen"** in the yellow bar at the top (shown until the setup is done) and approve the Windows prompt. This is needed only once per PC.
   - *What it does:* switches the screen from Windows' standard serial (COM port) driver to **Microsoft's built-in WinUSB
     driver**. Only the screen's own USB entry is changed, nothing else on your PC.
   - *Why:* the screen's firmware turns its display off about 30 seconds after Windows closes the serial port. With
     WinUSB that never happens, so you do not need to keep any program running.
5. **Add** a video, GIF, image or vendor theme `.ht` file (drop it on the window or click **+ Add clip**). It plays right away in a
   screen-shaped preview. Pick the part you want, adjust size/position and quality in **Tune** (the defaults are good), press
   **Save as version**, then **Send to screen** on that version. The upload takes **about 3 minutes** for a full-size clip
   (the screen's built-in memory writes slowly); the picture appears when it ends.
6. **Close the app** (close the window). Done: the screen loops your clip forever.

To change the picture later, just start the app again.

*Browser fallback (needs Node.js 18+ from nodejs.org):* run `scripts\start-browser.bat` from the extracted `resources\app` folder (or `npm start` in the source tree); the app opens in your browser instead of its own window.

## Using it

The window has the **Library** (your clips) on the left and the selected clip on the right: a preview shaped like the
screen, **Tune**, and its **Versions**. The bar at the top shows the screen's status and what is **on screen now**; the
gear opens **Settings**.

- **Adding clips:** videos, GIFs, images, the vendor app's downloaded **theme files (`.ht`)** and plain `.zip` files
  containing a video or image. Drop files anywhere on the window (several at once is fine) or use **+ Add clip**. Nothing
  is converted when you add a clip.
- **Gallery folder:** your clips are the files in `Documents\CoolerScreen\Gallery` ("Open Gallery folder"). Any file you
  put there appears in the app automatically; adding through the app copies the file there (same name, "name (2)" if
  taken). Renaming a clip in the app renames its file. The folder can be changed in Settings.
- **Tune: the loop.** Choose the part of the clip that loops. Everything is frame-accurate: the start is a source frame,
  the length a number of screen frames (18 per second). Drag the handles on the filmstrip, use "◀ frame / frame ▶"
  (Shift = 10 frames) or the arrow keys on a handle, or type the numbers; "Zoom to the selection" makes single frames
  easy to hit on long clips. The preview can play **the selection** or **the seam** (the last second, then the first second,
  over and over) so you can judge the join.
  - **Check the seam** compares the last frame with the first: onion skin (first frame over the last, adjustable), side by
    side, or difference (black = identical), with a similarity score.
  - **Find best loop point** searches ±2 s around the end for the frame most like the first frame and offers it.
  - **Blend the end into the start** (0-1.5 s) cross-fades the last moments into the frames just before the start (the
    classic seamless-loop trick; the length stays the same). It needs that much clip before the start.
- **Tune: size, corners and quality.** Size and position on the screen (zoom, left-right, up-down, or drag the preview;
  "Fill the screen" is the default, "Show the whole picture" adds black bars). **Rounded corners** with an adjustable radius
  (per version; the last choice is the default for the next clip). Quality **Best that fits** or Custom, and **Simplify
  fine detail** (0 = off), which softens tiny detail so longer parts fit. The preview shows all of it, including the notch.
- **Space on the screen** shows the estimated size against the screen's real free space, measured at each screen check
  (free storage + what the app's two clips occupy, minus the start-up logo and a 160 KB margin; about 10.9 MB on the
  tested screen; 9.5 MB minus the logo until the first check). If even the lowest quality doesn't fit, it suggests a
  "Simplify fine detail" level that fits, then how long the part can be.
- **Versions:** **Save as version** prepares a copy for the screen (a few seconds). A clip can have several versions, so you
  can compare them (click one to preview it) and send any of them. Versions are kept in `%APPDATA%\CoolerScreen`.
- **Send to screen** replaces what is on the screen. Keep the app open until the bar at the bottom says it is done.
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
- **Screen status** is checked once when the app opens (the app talks to the screen for a moment); use Settings >
  Screen > **Check again** after plugging it in. The app never polls the screen.

Settings and versions live in `%APPDATA%\CoolerScreen\`; your original clips are in the Gallery folder.

## Undo / restore the original driver

In the app: **Settings (gear) > Advanced > Restore original driver** (approve the Windows prompt). Windows installs the normal COM-port
driver again and LOVINGCOOL MONITOR can see the screen again. (The screen will then switch its display off 30 s after any
program closes its port. That is how the vendor software works, which is why it must stay running.)
Command line equivalent: `node.exe bin\coolerscreen.js restore`.

## Troubleshooting

| Problem | What to do |
|---|---|
| "Windows couldn't connect to the screen at startup" (banner with a Reconnect button), or the screen is missing and Device Manager shows "Unknown USB Device (Device Descriptor Request Failed)" | A known quirk: after a PC boot Windows sometimes fails to enumerate the screen (the screen itself keeps playing its loop). Click **Reconnect** (or run `node.exe bin\coolerscreen.js reconnect`) and approve the Windows prompt. It power-cycles only that one USB port, which fixes it immediately. Nothing is installed or changed permanently. |
| "Screen not found" | Check the USB cable/header and that the cooler has power. Unplug and replug USB, then **Check again** (click the status pill at the top). |
| Banner says the screen "needs setup" again | The driver was reset (moving the internal USB header to another port, or a big Windows update). Click "Set up screen" again. |
| Screen goes dark about 30 s after sending | The one-time setup was not applied (or was undone). Run "Set up screen". |
| "Could not open the screen" / port busy | Close LOVINGCOOL MONITOR (and anything else using the screen). If you ran setup, the vendor app will not work anymore. |
| Clip is "too big for the screen" | Choose a shorter part or "Best that fits". The screen only has about 9 MB for your clip. Garbage on screen means an oversize file got through, so make a smaller version. |
| Clip plays too fast or too slow | It was made by an older CoolerScreen at a different frame rate. Use **Re-make** on that version. |
| The setup says it failed | Read `%TEMP%\coolerscreen-setup.log` and open an issue with its contents. |
| Browser page does not open | Open http://127.0.0.1:8765/ yourself (the black window prints the exact address). |

### Screen stays on when the PC is off

Many motherboards keep the USB ports powered while the PC is shut down, so the screen keeps showing its loop. The vendor
app used to switch the screen off at shutdown, which needs a running program; CoolerScreen runs nothing in the
background, so it cannot do that. The no-software fix is in the BIOS/UEFI: enable **ErP Ready** / **ErP** (Gigabyte "ErP",
ASUS "ErP Ready", MSI "EuP 2013") or disable **USB power in S4/S5** (names vary by board). Then the USB ports lose power
when the PC is off and the screen turns off with it. Note: this also stops USB charging (phones etc.) while the PC is off.

## For developers

Needs Node.js 18+ (Windows is the tested platform).

    git clone https://github.com/oldbugo/coolerscreen
    cd coolerscreen
    npm install            # ffmpeg-static downloads ffmpeg.exe in its install script
    npm start              # web app on http://127.0.0.1:8765/ (browser)
    npm run app            # same app in a desktop window (Electron)

npm 11 asks you to approve dependency install scripts; `package.json` already lists the needed ones under `allowScripts`
(`ffmpeg-static`, `@serialport/bindings-cpp`, `usb`). If ffmpeg.exe is missing afterwards, run `npm approve-scripts` and
reinstall. Build the release zip with `powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1` (output in `dist\`).
Set `COOLERSCREEN_HOME` to keep library/settings in another folder.

### CLI

    node bin/coolerscreen.js list                     # clips (originals) and their versions
    node bin/coolerscreen.js add <file>               # copy into the Gallery folder
    node bin/coolerscreen.js make <clip> [--start sec] [--length sec] [--quality auto|1-100] [--zoom 0.2-4] [--x 0-1] [--y 0-1]
                                         [--corners 0-300] [--detail 0-100] [--blend 0-1.5]
    node bin/coolerscreen.js send <version id>        # put a version on the screen
    node bin/coolerscreen.js upload <file> [make options] [--no-send]   # add + make + send
    node bin/coolerscreen.js setup         # one-time WinUSB driver setup (UAC prompt)
    node bin/coolerscreen.js restore       # undo the setup
    node bin/coolerscreen.js reconnect     # reset the USB port after a failed boot-time enumeration
    node bin/coolerscreen.js transport     # winusb | serial | none
    node bin/coolerscreen.js clear startup|standby
    node bin/coolerscreen.js slots
    node bin/coolerscreen.js info

Testing without a screen: set `COOLERSCREEN_FAKE_DEVICE=1` (or `serial`, `none`, `failedusb`, `busy` to simulate those
states; `COOLERSCREEN_FAKE_FAIL=1` makes uploads fail halfway). Nothing then touches USB, the COM port or the driver
scripts. `COOLERSCREEN_HOME` and `COOLERSCREEN_GALLERY` point the library and the Gallery folder elsewhere.

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

Code map: `src/device.js` (serial + framing), `src/usbScreen.js` (WinUSB), `src/deviceOps.js` (transport choice, retries, fake
device), `src/convert.js` (ffmpeg), `src/library.js` (clips and versions), `src/gallery.js` (Gallery folder), `src/send.js`,
`src/splash.js` (start-up logo), `src/slots.js`, `src/gate.js` (one-at-a-time queue), `src/server.js`, `public/`.

## Tested hardware

Lovingcool SC70 AIO cooler screen, USB VID `33C3` PID `7791`, model string `D215-NOR-JD9365-6.67inch-hor`, firmware 3.6,
Windows 11. Other firmware versions and other Lovingcool-style screens are untested.

## Disclaimer

Unofficial, not affiliated with or endorsed by Lovingcool or the maker of the vendor software. Based on observing the
device's USB traffic. Replacing a USB driver and writing to the screen's storage carry some risk; use at your own risk,
no warranty. (Restoring the original driver is always possible, see "Undo".)

## License

MIT, (c) oldbugo. See `LICENSE` and `CHANGELOG.md`.
