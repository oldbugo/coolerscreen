// Transport selection + friendly errors. Ports/handles are opened only for the duration of a call.
// Prefers WinUSB (no watchdog); falls back to the COM port with a warning.
const sleep = ms => new Promise(r => setTimeout(r, ms));

function friendly(e) {
  if (e && e.failedUsb) return e;
  const m = (e && e.message) || String(e);
  if (/not found/i.test(m)) {
    return new Error('Screen not found. Check the USB cable and that the cooler is powered.');
  }
  if (/access denied|busy|in use|EBUSY|EACCES|cannot open|cannot claim|Opening/i.test(m)) {
    return new Error('Could not open the screen. Close "LOVINGCOOL MONITOR" (and any other program using the screen), then try again. (' + m + ')');
  }
  return e instanceof Error ? e : new Error(m);
}

const SERIAL_WARNING = 'The screen is still on the standard serial (COM) driver, so it will turn its display off about 30 seconds after any program closes the port. Run the one-time setup ("Set up screen") to fix this.';

/** Returns { screen, transport: 'winusb'|'serial' }. */
async function openAny() {
  let usbErr;
  try {
    const { UsbScreen } = require('./usbScreen');
    return { screen: await UsbScreen.open(), transport: 'winusb' };
  } catch (e) { usbErr = e; }
  try {
    const { Screen, findPort } = require('./device');
    if (await findPort()) return { screen: await Screen.open(), transport: 'serial' };
  } catch (e) { throw friendly(e); }
  if (!(usbErr && /claim/.test(usbErr.message)) && (await failedUsbPresent())) {
    const e = new Error('Windows could not connect to the screen at startup (a known quirk). Click Reconnect (needs one administrator approval).');
    e.failedUsb = true;
    throw e;
  }
  throw friendly(usbErr && /claim/.test(usbErr.message) ? usbErr : new Error('Screen not found'));
}

/** What is reachable right now, without sending anything to the screen. */
async function state() {
  try {
    const { probe } = require('./usbScreen');
    if ((await probe()) === 'winusb') return { transport: 'winusb', needsSetup: false };
  } catch {}
  try {
    const { findPort } = require('./device');
    if (await findPort()) return { transport: 'serial', needsSetup: true, warning: SERIAL_WARNING };
  } catch {}
  return { transport: 'none', needsSetup: false, failedUsb: await failedUsbPresent() };
}

// Windows sometimes fails to enumerate the screen at boot: "Unknown USB Device (Device Descriptor Request Failed)".
function failedUsbPresent() {
  if (process.platform !== 'win32') return Promise.resolve(false);
  return new Promise(resolve => {
    const cmd = "@(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\\\VID_0000&PID_000' }).Count";
    require('child_process').execFile('powershell', ['-NoProfile', '-Command', cmd], { windowsHide: true, timeout: 15000 },
      (e, out) => resolve(!e && parseInt(out, 10) > 0));
  });
}

async function checkScreen() {
  const { screen, transport } = await openAny();
  try {
    const info = await screen.info();
    return { ...info, _transport: transport, _needsSetup: transport === 'serial', _warning: transport === 'serial' ? SERIAL_WARNING : undefined };
  } catch (e) { throw friendly(e); } finally { await screen.close(); }
}

// Open with retries (the screen can restart and re-enumerate after an upload).
async function openRetry(seconds) {
  const end = Date.now() + seconds * 1000;
  for (;;) {
    try { return await openAny(); } catch (e) {
      if (Date.now() > end) throw e;
      await sleep(1000);
    }
  }
}

/**
 * items: [{file, slot}] uploaded one after another (slot 'startup' or 'standby').
 * opts.playLoop: afterwards tell the screen to play the standby (loop) clip now (key 0x14 [1,0]).
 * onProgress(0..1 overall, text)
 */
async function sendFiles(items, onProgress, opts = {}) {
  let warning, transport;
  const total = items.length + (opts.playLoop ? 0.1 : 0);
  for (let i = 0; i < items.length; i++) {
    const { file, slot } = items[i];
    const label = slot === 'standby' ? 'standby' : 'startup';
    onProgress && onProgress(i / total, i === 0 ? 'Connecting to screen' : 'Waiting for screen to come back');
    const o = i === 0 ? await openAny() : await openRetry(25);
    const s = o.screen;
    transport = o.transport;
    let sent = false;
    try {
      await s.uploadAnimation(file, label, (a, b) => {
        if (a >= b) sent = true;
        onProgress && onProgress((i + a / b) / total, 'Uploading ' + (label === 'standby' ? 'loop' : 'intro') + ' clip (the screen writes slowly: ~3 min for 9 MB)');
      });
      sent = true;
    } catch (e) {
      // The screen may reset itself right after the last byte; that is not a failure.
      if (!sent) throw friendly(e);
      warning = String(e.message || e);
    } finally {
      await s.close().catch(() => {});
    }
    if (i < items.length - 1 || opts.playLoop) await sleep(2500);
  }
  if (opts.playLoop) {
    onProgress && onProgress(items.length / total, 'Starting playback');
    const o = await openRetry(25);
    try { await o.screen.cmd(0x14, [1, 0], { wait: false }); await sleep(300); } finally { await o.screen.close().catch(() => {}); }
    transport = o.transport;
  }
  return { ok: true, warning, transport, serialWarning: transport === 'serial' ? SERIAL_WARNING : undefined };
}

module.exports = { failedUsbPresent, checkScreen, sendFiles, state, openAny, openRetry, SERIAL_WARNING };
