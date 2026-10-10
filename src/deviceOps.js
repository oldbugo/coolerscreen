// Transport selection + friendly errors. Ports/handles are opened only for the duration of a call.
// Prefers WinUSB (no watchdog); falls back to the COM port with a warning.
const sleep = ms => new Promise(r => setTimeout(r, ms));

// Test hook: COOLERSCREEN_FAKE_DEVICE=1 (or winusb | serial | none | failedusb | busy) never touches USB, the COM port or
// PowerShell; it simulates a screen in that state. COOLERSCREEN_FAKE_FAIL=1 makes fake uploads fail halfway.
const FAKE_ENV = process.env.COOLERSCREEN_FAKE_DEVICE;
const FAKE = FAKE_ENV ? (FAKE_ENV === '1' ? 'winusb' : FAKE_ENV.toLowerCase()) : null;
const fake = { transport: FAKE };
function fakeError() {
  const t = fake.transport;
  let e = null;
  if (t === 'none') { e = new Error('Screen not found. Check the USB cable and that the cooler is powered.'); e.code = 'notFound'; }
  if (t === 'failedusb') { e = new Error('Windows could not connect to the screen at startup (a known quirk). Click Reconnect (needs one administrator approval).'); e.failedUsb = true; e.code = 'failedUsb'; }
  if (t === 'busy') { e = new Error('Could not open the screen. Close "LOVINGCOOL MONITOR" (and any other program using the screen), then try again.'); e.code = 'busy'; }
  return e;
}

function friendly(e) {
  if (e && e.failedUsb) return e;
  const m = (e && e.message) || String(e);
  if (/not found/i.test(m)) {
    const n = new Error('Screen not found. Check the USB cable and that the cooler is powered.');
    n.code = 'notFound';
    return n;
  }
  if (/access denied|busy|in use|EBUSY|EACCES|cannot open|cannot claim|Opening/i.test(m)) {
    const n = new Error('Could not open the screen. Close "LOVINGCOOL MONITOR" (and any other program using the screen), then try again. (' + m + ')');
    n.code = 'busy';
    return n;
  }
  if (/timed? ?out|stalled/i.test(m)) {
    const n = new Error('The screen stopped responding. Press Reconnect (or unplug and replug its USB cable), then send again. (' + m + ')');
    n.code = 'stalled';
    return n;
  }
  return e instanceof Error ? e : new Error(m);
}

const SERIAL_WARNING = 'The screen is still on the standard serial (COM) driver, so it will turn its display off about 30 seconds after any program closes the port. Run the one-time setup ("Set up screen") to fix this.';

/** Returns { screen, transport: 'winusb'|'serial' }. */
async function openAny() {
  if (FAKE) throw fakeError() || new Error('Fake device: no real screen access.');
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
  if (FAKE) {
    if (fake.transport === 'winusb') return { transport: 'winusb', needsSetup: false, fake: true };
    if (fake.transport === 'serial') return { transport: 'serial', needsSetup: true, warning: SERIAL_WARNING, fake: true };
    return { transport: 'none', needsSetup: false, failedUsb: fake.transport === 'failedusb', fake: true };
  }
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
  if (FAKE) return Promise.resolve(fake.transport === 'failedusb');
  if (process.platform !== 'win32') return Promise.resolve(false);
  return new Promise(resolve => {
    const cmd = "@(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\\\VID_0000&PID_000' }).Count";
    require('child_process').execFile('powershell', ['-NoProfile', '-Command', cmd], { windowsHide: true, timeout: 15000 },
      (e, out) => resolve(!e && parseInt(out, 10) > 0));
  });
}

async function checkScreen() {
  if (FAKE) {
    await sleep(600);
    const err = fakeError();
    if (err) throw err;
    const transport = fake.transport === 'serial' ? 'serial' : 'winusb';
    return { model: 'D215-NOR-JD9365-6.67inch-hor (fake)', version: '3.6', _transport: transport, _needsSetup: transport === 'serial',
      _warning: transport === 'serial' ? SERIAL_WARNING : undefined, _fake: true };
  }
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
  if (FAKE) { // test hook: pretend to upload (about 10 s for the loop clip), never touch USB
    const err = fakeError();
    if (err) throw err;
    const total = items.length + (opts.playLoop ? 0.1 : 0);
    for (let i = 0; i < items.length; i++) {
      onProgress && onProgress(i / total, i === 0 ? 'Connecting to screen' : 'Waiting for screen to come back');
      await sleep(500);
      const steps = items[i].slot === 'standby' ? 20 : 4;
      for (let k = 1; k <= steps; k++) {
        if (process.env.COOLERSCREEN_FAKE_FAIL && items[i].slot === 'standby' && k > steps / 2) throw friendly(new Error('USB write timed out after 20 s (fake)'));
        onProgress && onProgress((i + k / steps) / total, 'Uploading ' + (items[i].slot === 'standby' ? 'loop' : 'intro') + ' clip');
        await sleep(500);
      }
    }
    if (opts.playLoop) { onProgress && onProgress(items.length / total, 'Starting playback'); await sleep(500); }
    return { ok: true, transport: 'fake', serialWarning: fake.transport === 'serial' ? SERIAL_WARNING : undefined };
  }
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

module.exports = { FAKE, fake, failedUsbPresent, checkScreen, sendFiles, state, openAny, openRetry, SERIAL_WARNING };
