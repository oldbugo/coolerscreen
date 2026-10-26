// Guarded wiring check: node tools/ui-shots/connection-flow.mjs <CDP port> [screenshot directory]
// Run only against tools/ui-shots/run.ps1's quiet DEBUG app with a scratch home and fake screen.
import { connect } from './cdp.mjs';
import { mkdir } from 'node:fs/promises';
import { join } from 'node:path';
const [port, out] = process.argv.slice(2);
if (!port || !/^\d+$/.test(port)) throw new Error('Supply the quiet fake debug app CDP port.');
const c = await connect(port), js = code => c.eval(code);
const wait = ms => new Promise(resolve => setTimeout(resolve, ms));
let failed = 0, installed = false;
async function check(name, expression, timeout = 7000) {
  const end = Date.now() + timeout;
  let result;
  do {
    result = await js(expression);
    if (result) break;
    await wait(250);
  } while (Date.now() < end);
  console.log(`${result ? 'PASS' : 'FAIL'} ${name}`);
  if (!result) failed++;
}
const set = body => js(`(() => { const t = window.__csConnectionFlow; ${body}; document.dispatchEvent(new Event('visibilitychange')); return true; })()`);
try {
  await c.send('Runtime.enable');
  const guard = await js(`(async () => {
    const a = await window.__TAURI_INTERNALS__.invoke('app_state');
    if (!/debug, fake screen:/.test(a.build?.screen || '') || !a.fake) throw new Error('Refusing: app must be a fake DEBUG build.');
    const d = await window.__TAURI_INTERNALS__.invoke('device_state');
    if (d.fake !== true) throw new Error('Refusing: real Tauri device_state must confirm fake=true.');
    return { version: a.appVersion, build: a.build.screen };
  })()`);
  console.log('Preflight:', JSON.stringify(guard));
  installed = true; // cleanup also covers a partially completed injection
  await js(`(async () => {
    if (window.__csConnectionFlow) throw new Error('Connection flow already installed.');
    const { backend } = await import('./js/backend.js');
    const original = backend.call, base = await original('app_state');
    const t = window.__csConnectionFlow = { backend, original, base, checks: 0, probes: 0, checkMode: 'missing', present: false,
      hidden: Object.getOwnPropertyDescriptor(document, 'hidden'), fps: document.getElementById('lvFps').value, mutations: [] };
    t.live = { ...base.live, settings: { ...base.live.settings, inApp: true }, status: {
      running: true, connected: false, fps: 60, measuredFps: 0, paused: false, deviceHold: false, systemHold: false,
      layerOff: false, idle: false, liveItems: true, noDevice: false, twinning: false, previewing: false,
      connection: { state: 'failedUsb', message: 'simulated descriptor failure', retryInSeconds: 15, attempts: 4 } } };
    backend.call = async (command, args) => {
      if (command === 'app_state') return { ...base, live: structuredClone(t.live) };
      if (command === 'device_check') {
        t.checks++;
        if (t.checkMode === 'missing') throw Object.assign(new Error('simulated missing screen'), { code: 'notFound' });
        return { model: 'Fake connection flow', _transport: 'winusb', _needsSetup: false };
      }
      if (command === 'device_presence') { t.probes++; return { present: t.present, transport: t.present ? 'winusb' : 'none', needsSetup: false }; }
      if (['live_set', 'settings_update', 'gallery_set_dir', 'device_restart', 'device_reconnect', 'device_setup', 'device_restore'].includes(command)) {
        t.mutations.push(command); throw new Error('Connection flow refuses a settings/device mutation: ' + command);
      }
      return original(command, args);
    };
    Object.defineProperty(document, 'hidden', { get: () => false, configurable: true });
    document.dispatchEvent(new Event('visibilitychange'));
    return true;
  })()`);
  await check('failed USB reaches pill, banner and retry text', `document.getElementById('pill').dataset.state === 'failedUsb' && /Retrying in 15 s/.test(document.getElementById('banners').textContent)`);
  await check('Reconnect action is offered without clicking it', `[...document.querySelectorAll('#banners button')].some(b => b.textContent === 'Reconnect' && /permission/.test(b.title))`);
  if (out) { await mkdir(out, { recursive: true }); await c.shot(join(out, 'connection-failed.png'), 1280, 860); }
  await js(`(() => { document.getElementById('pill').click(); document.getElementById('lvFps').focus(); document.getElementById('lvFps').value = '30'; return true; })()`);
  await wait(2500);
  await check('two-second refresh preserves focused input and draft value', `document.activeElement === document.getElementById('lvFps') && document.getElementById('lvFps').value === '30'`, 0);
  await set(`t.live.status.connected = true; t.live.status.connection = { state: 'connected', message: null, retryInSeconds: 0, attempts: 0 }`);
  await check('recovery clears stale failed banner', `document.getElementById('pill').dataset.state === 'ready' && !document.querySelector('#banners [data-connection-state]')`);
  if (out) await c.shot(join(out, 'connection-recovered.png'), 1280, 860);
  await set(`t.live.settings.inApp = false; t.live.status.running = false; t.live.status.connected = false; t.live.status.connection.state = 'off'; t.live.status.layerOff = true; t.live.status.idle = true`);
  await wait(2500);
  await check('Live off preserves recovered ready status', `document.getElementById('pill').dataset.state === 'ready'`, 0);
  await js(`(() => { document.getElementById('scrCheck').click(); return true; })()`);
  await check('manual missing state is shown with Live off', `document.getElementById('pill').dataset.state === 'notFound'`);
  await check('Live-off missing screen watches presence despite dormant theme flags', `window.__csConnectionFlow.probes > 0`, 14000);
  const before = await js('window.__csConnectionFlow.checks');
  await set('t.present = true');
  await check('arrival triggers exactly one scheduled check', `window.__csConnectionFlow.checks === ${before + 1}`, 14000);
  await wait(12000);
  await check('still-present missing result does not cause periodic checks', `window.__csConnectionFlow.checks === ${before + 1}`, 0);
  await check('no settings, reconnect or restart command was sent', 'window.__csConnectionFlow.mutations.length === 0', 0);
  console.log(c.errors.length ? 'PAGE ERRORS:\n' + c.errors.join('\n') : 'No page errors.');
  if (c.errors.length) failed++;
} finally {
  try {
    if (installed) await js(`(() => {
      const t = window.__csConnectionFlow;
      if (!t) return true;
      t.backend.call = t.original;
      if (t.hidden) Object.defineProperty(document, 'hidden', t.hidden); else delete document.hidden;
      document.getElementById('lvFps').value = t.fps;
      delete window.__csConnectionFlow;
      return true;
    })()`);
  } finally { c.close(); }
}
console.log(failed ? `${failed} FAILED` : 'ALL PASSED');
process.exitCode = failed ? 1 : 0;
