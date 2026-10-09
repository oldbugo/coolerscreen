# CoolerScreen

An unofficial, open-source replacement for the vendor "LOVINGCOOL MONITOR" software for the
**Lovingcool SC70 AIO cooler's 6.67" USB screen**. Drop in any video, GIF or image; CoolerScreen converts it
to the format the screen stores on board as its startup animation, so it plays with no PC software running.

Features: import + automatic conversion (fits the screen's ~10 MB storage by adjusting fps/quality), a gallery of
everything you imported, one-click "Send to screen", and a configurable **notch mask** that hides the panel's
camera punch-hole. Runs as a small local web page (nothing is sent over the internet) plus a CLI.

## Supported device

- USB VID `33C3`, PID `7791` (also `7788` is recognised), model string `D215-NOR-JD9365-6.67inch-hor`, firmware 3.6 (tested).
- Windows is the tested platform. The screen appears as a USB serial (COM) port.

## Install and run (no programming needed)

1. Install **Node.js 18 or newer** (LTS) from https://nodejs.org.
2. Download this project (green "Code" button > Download ZIP on GitHub) and unzip it.
3. Open a terminal in the folder and run `npm install` (once).
4. **Close LOVINGCOOL MONITOR** (it keeps the screen's port locked).
5. Run `npm start`. Your browser opens at http://127.0.0.1:8765/.
6. Click **Set up screen (one-time)** (see "Why the setup step?" below). Windows asks for administrator approval once.

Use: drop a file in "Import". A dialog opens where you choose, per item:

- **Loop length** (seconds, slider + number) and **start** (trim),
- **FPS** (10-30, default 20),
- **Quality**: "Auto (fit to space)" (default; picks the best quality that fits) or "Manual" with a 1-100 slider (higher = better),
- **Send to** (only shown with "Advanced"): Loop (default), Intro or Both.

The estimated output size is shown before you convert, and the final size/fps/quality after. In Manual mode, if the
result is bigger than the space available, the file is kept but marked "too big for the screen" and Send is disabled.
Hover a gallery item to preview, then press **Send to screen**. After upload the screen may restart itself and
disappear from USB for a few seconds. **Check screen** reads the device info (not polled automatically).

### Loop and intro slots

The screen has two stored clips that **share about 10 MB**: the *intro* (`/data/video/start_video.mp4`, plays once at
power-on) and the *loop* (`/data/video/sleep_video.mp4`, plays forever afterwards). By default **Send to screen** puts
your clip in the **Loop** slot, makes the intro a tiny black clip (if it is not already), and tells the screen to start
playing the loop immediately. So the loop clip may use almost the whole budget (9.5 MB minus about 20 KB).
Tick "Advanced" to send a separate Intro clip, or the same clip to both (each then at most half the budget).
CoolerScreen remembers in `settings.json` what you last sent to each slot and its size; if the intro slot was never
written from this tool it assumes 1.8 MB for it when budgeting an intro. "Clear intro/loop slot" uploads a 1-frame black clip.

### Why the setup step? (the 30 second watchdog)

The screen's firmware turns the display off about 30 seconds after the host drops the serial "DTR" line, which
Windows' standard serial driver (usbser) does whenever the port is opened/closed or the device enumerates. The vendor app
avoids this by keeping the port open all the time. CoolerScreen does not run anything in the background, so instead the
one-time setup **binds Microsoft's built-in WinUSB driver** to the screen's serial function
(`USBVID_33C3&PID_7791&MI_00`). With WinUSB no DTR request is ever sent, so the screen keeps playing its loop forever,
with nothing running on your PC, even after restarts.

What the setup changes: it asks for administrator approval (UAC), runs `scripts/setup-winusb.ps1` (log:
`%TEMP%coolerscreen-setup.log`), which sets that one device's class to USBDevice, installs `winusb.inf` for it, adds an
interface GUID registry value for it, and restarts it. Nothing else on your system is touched, and it is safe to run again.
Side effect: the screen no longer shows up as a COM port, so **the vendor "LOVINGCOOL MONITOR" app will not see it** until you
undo this. **Undo:** Advanced > "Restore original driver" (or `node bin/coolerscreen.js restore`), which runs
`scripts/restore-serial.ps1` (`pnputil /remove-device` + `/scan-devices`) so Windows installs usbser again.
If you skip the setup, uploads still work over the COM port but the screen sleeps 30 s after the upload.

### Notch mask

The panel is a phone panel with a camera hole on the left edge in landscape view. The mask is applied to the
landscape 2400x1080 picture and is baked into the converted file (and the preview, so you see the result).
Modes: **band** (black strip of N px along an edge; default: left, 92 px), **circle** (centre x/y, radius,
feather), or off. Change it in "Notch mask", press Save, then **Re-convert** items (or "Re-convert all").
Items converted with an older mask are flagged "mask outdated".

### CLI

    node bin/coolerscreen.js upload clip.mp4 [--slot loop|startup|both] [--quality auto|1-100] [--fps 10-30]
                                             [--start sec] [--length sec] [--name text] [--no-send]
    node bin/coolerscreen.js setup        # one-time WinUSB driver setup (UAC prompt)
    node bin/coolerscreen.js restore      # undo the setup
    node bin/coolerscreen.js clear startup|standby
    node bin/coolerscreen.js slots
    node bin/coolerscreen.js info
    node bin/coolerscreen.js list

### Where data lives

`%APPDATA%\CoolerScreen\` : `settings.json` and `library\<id>\` (original, `preview.mp4`, `thumb.jpg`, `device.mp4`,
`meta.json`). Set `COOLERSCREEN_HOME` to use another folder.

## Conversion details

Input is scaled/cropped to cover 2400x1080, the mask is applied, then rotated 90 degrees clockwise (`transpose=1`) and
stored as 1080x2400 portrait MJPEG in an `.mp4` (no audio). The output must stay within the space left in its slot (the device has
~10 MB shared by both slots; oversize files display garbage). In Auto quality the converter estimates size on a sample, picks the lowest JPEG quantiser (`-q:v` 3..31) that fits the
available space, and steps fps down (30/25/20/15/12/10) if even q=31 is too big. The 1-100 quality scale maps linearly
to q 31..3. Default is the whole clip up to 10 s at 20 fps. Still images become a 3 s clip. ffmpeg comes from the `ffmpeg-static` package.

## Protocol summary

Serial (2,000,000 baud) or, after setup, the same frames over the WinUSB bulk endpoints (OUT `0x02`, IN `0x81` on the CDC data interface). Frame: `55 AA | len16LE (payload length + 7) | key | payload | sum16LE` (sum of all preceding bytes).

| Key | Meaning |
|-----|---------|
| `0x06` | info (JSON reply) |
| `0x12` | startup animation upload: payload = 4-byte LE file size, then the raw file bytes sent in 40 KB chunks |
| `0x13` | standby animation upload (same scheme) |
| `0x01` | restart |
| `0x03` | brightness |
| `0x15` | motion timeout (semantics unknown) |
| `0x14` `[1,0]` | play the standby (loop) clip now, forever |
| `0x0E` `[1/0]` | display on/off |
| `0x04` + ASCII path | play a file; `0x02` stop; `0x22` unsolicited status frames (~2 s) |

See `src/device.js`. On connect the vendor app sends `FF D9 FF D9` and `00 00 00 00` to end any live JPEG stream; we do the same.

## Disclaimer

Unofficial, not affiliated with or endorsed by Lovingcool or the maker of the vendor software. Based on observing the
device's USB serial traffic. Flashing custom content to hardware carries risk; use at your own risk, no warranty.

## License

MIT, (c) oldbugo.
