// Launch the one-time driver scripts elevated (UAC prompt), then verify.
const { spawn } = require('child_process');
const fs = require('fs');
const os = require('os');
const path = require('path');
const deviceOps = require('./deviceOps');

const LOG = path.join(os.tmpdir(), 'coolerscreen-setup.log');
const SCRIPTS = path.join(__dirname, '..', 'scripts');
const sleep = ms => new Promise(r => setTimeout(r, ms));

function runElevated(script) {
  if (process.platform !== 'win32') return Promise.reject(new Error('Driver setup is only needed (and supported) on Windows.'));
  const file = path.join(SCRIPTS, script);
  try { fs.rmSync(LOG, { force: true }); } catch {}
  const ps = `Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '-NoProfile','-ExecutionPolicy','Bypass','-File','${file.replace(/'/g, "''")}'`;
  return new Promise((resolve, reject) => {
    const p = spawn('powershell', ['-NoProfile', '-Command', ps], { windowsHide: true });
    let err = '';
    p.stderr.on('data', d => (err += d));
    p.on('error', reject);
    p.on('close', () => {
      let log = '';
      try { log = fs.readFileSync(LOG, 'utf8'); } catch {}
      if (/RESULT: OK/.test(log)) return resolve(log);
      const m = /RESULT: ERROR (.*)/.exec(log);
      if (m) return reject(new Error('Setup failed: ' + m[1] + ` (log: ${LOG})`));
      reject(new Error(/canceled|cancelled/i.test(err) ? 'Setup was cancelled at the administrator prompt.' : 'Setup did not run or finished without a result. ' + err.trim() + ` (log: ${LOG})`));
    });
  });
}

async function waitFor(pred, seconds) {
  const end = Date.now() + seconds * 1000;
  while (Date.now() < end) { if (await pred()) return true; await sleep(1000); }
  return false;
}

/** Install WinUSB, verify by opening over WinUSB and restarting the screen. */
async function setupWinUsb(onProgress = () => {}) {
  const st = await deviceOps.state();
  if (st.transport === 'winusb') { onProgress(1, 'Already set up'); return { already: true }; }
  if (st.transport === 'none') throw new Error('Screen not found. Plug it in and make sure the cooler is powered.');
  onProgress(0.1, 'Waiting for administrator approval');
  await runElevated('setup-winusb.ps1');
  onProgress(0.6, 'Waiting for the screen to switch driver');
  if (!(await waitFor(async () => (await deviceOps.state()).transport === 'winusb', 25))) {
    throw new Error(`Driver was installed but the screen is not reachable over WinUSB yet. Unplug and replug the screen's USB cable and try "Check screen". (log: ${LOG})`);
  }
  onProgress(0.85, 'Verifying (restarting the screen)');
  const { screen } = await deviceOps.openAny();
  try { await screen.info(); await screen.cmd(0x01, [], { wait: false }); await sleep(500); } finally { await screen.close(); }
  onProgress(1, 'Done');
  return { already: false };
}

async function restoreSerial(onProgress = () => {}) {
  onProgress(0.1, 'Waiting for administrator approval');
  await runElevated('restore-serial.ps1');
  onProgress(0.7, 'Waiting for the COM port');
  const ok = await waitFor(async () => (await deviceOps.state()).transport === 'serial', 25);
  onProgress(1, 'Done');
  if (!ok) return { warning: 'Original driver restored, but the COM port has not appeared yet. Replug the USB cable if needed.' };
  return {};
}

/** Power-cycle the USB hub port of a screen that failed to enumerate at boot. */
async function reconnect(onProgress = () => {}) {
  const st = await deviceOps.state();
  if (st.transport !== 'none') { onProgress(1, 'Screen is already connected'); return { already: true }; }
  if (!st.failedUsb) throw new Error('Screen not found and no failed USB device was detected. Check the USB cable and that the cooler has power.');
  onProgress(0.1, 'Waiting for administrator approval');
  await runElevated('reconnect.ps1');
  onProgress(0.6, 'Waiting for the screen to appear');
  const ok = await waitFor(async () => (await deviceOps.state()).transport !== 'none', 25);
  onProgress(1, 'Done');
  if (!ok) throw new Error('The USB port was reset but the screen did not appear. Try unplugging and replugging the screen USB cable.');
  return { already: false };
}

module.exports = { setupWinUsb, restoreSerial, reconnect, LOG };
