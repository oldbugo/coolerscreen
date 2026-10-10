# CoolerScreen

A free, unofficial replacement for the "LOVINGCOOL MONITOR" software for the **Lovingcool SC70 AIO cooler's 6.67" USB screen**.
Pick any video, GIF or image on your PC, and CoolerScreen converts it and stores it **inside the screen itself**, where it
loops forever. You only need the app while you are changing the picture.

**What it does NOT do:** it installs no service, no startup entry and no background program. When you close it, the screen
keeps playing on its own, even after you restart or shut down the PC.

## Quick start (Windows, no programming needed)

1. **Close and uninstall LOVINGCOOL MONITOR.** (Only one program can talk to the screen at a time, and after step 4 the
   vendor app can no longer see the screen at all. You can undo this later, see "Undo".)
2. **Download** `CoolerScreen-<version>-win-x64.zip` from the [Releases page](https://github.com/oldbugo/coolerscreen/releases)
   **Before extracting:** right-click the zip > **Properties** > tick **Unblock** > OK. (Windows tags downloaded files;
   with *Smart App Control* switched on it refuses to run the launcher from a tagged zip.) Then **extract** it (right click >
   Extract All) somewhere you will find it again, for example your Documents folder.
3. **Double-click `Start CoolerScreen.bat`.** A black window opens (keep it open while you use the app) and your web
   browser opens the CoolerScreen page. Windows may warn that the file came from the internet ("Windows protected your
   PC" / "Open File - Security Warning"): choose **More info > Run anyway** / **Run**. The app is not code-signed.
4. **Click "Set up screen (one-time)"** and approve the Windows prompt. This is needed only once per PC.
   - *What it does:* switches the screen from Windows' standard serial (COM port) driver to **Microsoft's built-in WinUSB
     driver**. Only the screen's own USB entry is changed, nothing else on your PC.
   - *Why:* the screen's firmware turns its display off about 30 seconds after Windows closes the serial port. With
     WinUSB that never happens, so you do not need to keep any program running.
5. **Import** a video, GIF, image or vendor theme `.ht` file (drop it on the page). Choose the loop length, fps and quality (the defaults are good),
   check the notch mask preview, convert, then press **Send to screen**. The upload takes **about 3 minutes** for a full-size
   clip (the screen's built-in memory writes slowly); the picture appears when it ends.
6. **Close the app** (close the black window). Done: the screen loops your clip forever.

To change the picture later, just start the app again.

## Using it

- **Import formats:** videos, GIFs, images, and the vendor app's downloaded **theme files (`.ht`)**. Drop a `.ht` file
  (or a plain `.zip` containing a video or image) like any other file; the video inside is extracted and the theme's
  name is used as the title. Drops work anywhere on the page.
- **Gallery:** everything you import is kept, so you can switch between clips. Hover to preview, click to play larger.
- **Per-clip settings** ("Convert settings..."): start and loop length, fps 10-30, quality **Auto** (best that fits) or
  **Manual** 1-100. The estimated size is shown before converting. A clip that is too big for the screen is kept but
  cannot be sent.
- **Notch mask:** the panel has a camera punch-hole on the left edge (landscape view). The mask blacks that area out.
  Default: a black band, left edge, 92 px wide. You can switch to a circle or turn it off. After saving the mask, use
  "Re-convert" on a clip (or "Re-convert all") to apply it.
- **Loop and intro:** the screen stores two clips that share about 10 MB: the *intro* (plays once at power-on) and the
  *loop* (plays forever afterwards). "Send to screen" puts your clip in the **loop**, makes the intro black, and starts
  playing it. Tick "Advanced" to send a separate intro clip.
- **Check screen** shows the device info and whether the one-time setup is done.

Your library lives in `%APPDATA%\CoolerScreen\` (clips and `settings.json`).

## Undo / restore the original driver

In the app: **Advanced > Restore original driver** (approve the Windows prompt). Windows installs the normal COM-port
driver again and LOVINGCOOL MONITOR can see the screen again. (The screen will then switch its display off 30 s after any
program closes its port. That is how the vendor software works, which is why it must stay running.)
Command line equivalent: `node.exe bin\coolerscreen.js restore`.

## Troubleshooting

| Problem | What to do |
|---|---|
| "Windows couldn't connect to the screen at startup" (banner with a Reconnect button), or the screen is missing and Device Manager shows "Unknown USB Device (Device Descriptor Request Failed)" | A known quirk: after a PC boot Windows sometimes fails to enumerate the screen (the screen itself keeps playing its loop). Click **Reconnect** (or run `node.exe bin\coolerscreen.js reconnect`) and approve the Windows prompt. It power-cycles only that one USB port, which fixes it immediately. Nothing is installed or changed permanently. |
| "Screen not found" | Check the USB cable/header and that the cooler has power. Unplug and replug USB, then "Check screen". |
| Banner says the screen "needs setup" again | The driver was reset (moving the internal USB header to another port, or a big Windows update). Click "Set up screen (one-time)" again. |
| Screen goes dark about 30 s after sending | The one-time setup was not applied (or was undone). Run "Set up screen". |
| "Could not open the screen" / port busy | Close LOVINGCOOL MONITOR (and anything else using the screen). If you ran setup, the vendor app will not work anymore. |
| Clip is "too big for the screen" | Shorten the loop length, lower fps, or use Auto quality. The screen only has about 10 MB. Garbage on screen means an oversize file got through, so re-convert smaller. |
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
    npm start              # web app on http://127.0.0.1:8765/

npm 11 asks you to approve dependency install scripts; `package.json` already lists the needed ones under `allowScripts`
(`ffmpeg-static`, `@serialport/bindings-cpp`, `usb`). If ffmpeg.exe is missing afterwards, run `npm approve-scripts` and
reinstall. Build the release zip with `powershell -ExecutionPolicy Bypass -File scripts\make-release.ps1` (output in `dist\`).
Set `COOLERSCREEN_HOME` to keep library/settings in another folder.

### CLI

    node bin/coolerscreen.js upload clip.mp4 [--slot loop|startup|both] [--quality auto|1-100] [--fps 10-30]
                                             [--start sec] [--length sec] [--name text] [--no-send]
    node bin/coolerscreen.js setup         # one-time WinUSB driver setup (UAC prompt)
    node bin/coolerscreen.js restore       # undo the setup
    node bin/coolerscreen.js reconnect     # reset the USB port after a failed boot-time enumeration
    node bin/coolerscreen.js transport     # winusb | serial | none
    node bin/coolerscreen.js clear startup|standby
    node bin/coolerscreen.js slots
    node bin/coolerscreen.js info
    node bin/coolerscreen.js list

### How conversion works

The input is scaled/cropped to cover 2400x1080, the notch mask is applied there, then it is rotated 90 degrees clockwise
(`transpose=1`) and stored as 1080x2400 portrait MJPEG in an `.mp4` without audio. In Auto quality the converter estimates the size on
a sample, picks the lowest JPEG quantiser (`-q:v` 3..31) that fits the space left, and steps fps down (30/25/20/15/12/10) if
even q=31 does not fit. The 1-100 quality scale maps linearly to q 31..3. Defaults: whole clip up to 10 s, 20 fps. Still
images become a 3 s clip. Oversize files make the screen show garbage, so the size budget is enforced (total 9.5 MB shared by both slots).

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

Code map: `src/device.js` (serial + framing), `src/usbScreen.js` (WinUSB), `src/deviceOps.js` (transport choice, retries),
`src/convert.js` (ffmpeg), `src/library.js`, `src/send.js`, `src/slots.js`, `src/server.js`, `public/`.

## Tested hardware

Lovingcool SC70 AIO cooler screen, USB VID `33C3` PID `7791`, model string `D215-NOR-JD9365-6.67inch-hor`, firmware 3.6,
Windows 11. Other firmware versions and other Lovingcool-style screens are untested.

## Disclaimer

Unofficial, not affiliated with or endorsed by Lovingcool or the maker of the vendor software. Based on observing the
device's USB traffic. Replacing a USB driver and writing to the screen's storage carry some risk; use at your own risk,
no warranty. (Restoring the original driver is always possible, see "Undo".)

## License

MIT, (c) oldbugo. See `LICENSE` and `CHANGELOG.md`.
