# CoolerScreen 2.0: steps that need you (administrator approval)

These three screen operations change Windows drivers or reset a USB port, so Windows asks for administrator
approval (a UAC prompt). The app runs the same scripts as 1.2 (`scripts/*.ps1`, bundled with 2.0). Everything else
in 2.0 was tested on your cooler without admin rights.

Before you start: close CoolerScreen 1.2 and LOVINGCOOL MONITOR. Open CoolerScreen 2.0. Allow about 10 minutes.
Each step can be checked in `%TEMP%\coolerscreen-setup.log` (the last line is `RESULT: OK` or `RESULT: ERROR ...`) and
in `%APPDATA%\CoolerScreen\device.log`.

## 1. Restore the original driver, then set up again (round trip)

1. Settings (gear) > Advanced > **Restore original driver…** > confirm > approve the Windows prompt.
   - Expect: "Done", or a note that the COM port has not appeared yet (then unplug and replug the screen's USB cable).
   - The top bar then shows the screen needs setup (serial driver). The display turns off about 30 s after any
     program closes the port: this is the known firmware watchdog, expected on the serial driver.
2. Settings > Screen > **Set up screen** > approve the prompt.
   - Expect: "Done" within about 30 s; the screen restarts once (verification). Top bar: "Screen ready".
3. Press **Check again**. Expect: connected, set up (WinUSB).

If step 2 fails: replug the screen's USB cable and press Check again; the log says which part failed.

## 2. Reconnect (only when Windows failed to connect the screen at startup)

This happens after some PC boots ("Unknown USB Device (Device Descriptor Request Failed)"). It cannot be caused on
purpose safely, so test it the next time it happens naturally:

1. The top bar shows that Windows could not connect to the screen, with **Reconnect**.
2. Press Reconnect > approve the prompt. Expect the screen to appear within about 25 s.
3. If it does not: unplug and replug the screen's USB cable.

## 3. Optional checks

- After 2.0's first screen check, Settings > Screen > Details shows the measured capacity (`_capacity`).
- 1.2 does not know what 2.0 last put on the screen (2.0 test runs used a separate copy of your library). 1.2 shows
  "unknown" until you push something from 1.2 again; 2.0 shows it once you push from 2.0.
