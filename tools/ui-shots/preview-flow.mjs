// Preview-on-the-screen flow check (C3) against a quiet debug instance with themes 'Test pattern' and 'Coast'. node tools/ui-shots/preview-flow.mjs <port> <repo root>
import { pathToFileURL } from 'node:url';
const [port = '9333', repo] = process.argv.slice(2);
const { connect } = await import(pathToFileURL(repo + '/tools/ui-shots/cdp.mjs').href);
const c = await connect(port);
await c.send('Runtime.enable');
await c.size(1280, 860);
const wait = ms => new Promise(r => setTimeout(r, ms));
const js = code => c.eval(code);
let fails = 0;
const check = async (name, code, ms = 0) => {
  if (ms) await wait(ms);
  let ok, v;
  try { v = await js(code); ok = !!v; } catch (e) { ok = false; v = e.message; }
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${ok ? '' : '  -> ' + JSON.stringify(v)}`);
  if (!ok) fails++;
};
const click = (sel, text) => js(`(() => { const e = [...document.querySelectorAll(${JSON.stringify(sel)})].find(e => !${JSON.stringify(text || '')} || e.textContent.includes(${JSON.stringify(text || '')})); if (!e) return false; e.click(); return true; })()`);
const st = () => js(`window.__TAURI_INTERNALS__.invoke('app_state').then(a => ({ live: a.live.status, inApp: a.live.settings.inApp }))`);

await click('#navMine'); await wait(800);
console.log('cards:', await js(`[...document.querySelectorAll('.g-card .g-name')].map(e => e.textContent).join('|')`));
// 1. Live off: selecting previews
await check('Live is off to start with', `window.__TAURI_INTERNALS__.invoke('app_state').then(a => !a.live.settings.inApp)`);
await click('.g-card', 'Test pattern'); await wait(1500);
let s = await st();
await check('Live off: selecting starts a temporary preview stream', `${JSON.stringify(s.live.previewing && s.live.running)}`);
await check('The Live setting stays off', `${!s.inApp}`);
await check('The bar says Previewing on the screen', `!!document.querySelector('.sel-bar .pv-chip') && /Previewing/.test(document.querySelector('.sel-bar .pv-chip').textContent)`);
await check('No Store as Embedded in My themes', `![...document.querySelectorAll('.sel-bar button')].some(b => /Store|Update Embedded|Use and store/.test(b.textContent))`);
await click('.g-card', 'Coast'); await wait(700);
s = await st();
await check('Selecting another theme keeps previewing (same stream)', `${JSON.stringify(s.live.previewing && s.live.running)}`);
// leaving ends it
await click('#navScreen'); await wait(1500);
s = await st();
await check('Leaving to On screen ends the preview and the temporary stream', `${JSON.stringify(!s.live.previewing && !s.live.running)}`);
// 2. rapid clicking: quick sequence ends on the last one without error
await click('#navMine'); await wait(500);
for (const n of ['Test pattern', 'Coast', 'Test pattern', 'Coast']) { await click('.g-card', n); await wait(40); }
await wait(1500);
s = await st();
await check('Rapid clicks: one stream, previewing', `${JSON.stringify(s.live.previewing && s.live.running)}`);
// 3. Use this theme: sets it, ends the preview, goes to On screen
await click('.sel-bar button', 'Use this theme'); await wait(2000);
s = await st();
await check('Use this theme goes to On screen', `!document.getElementById('screenView').hidden`);
await check('Use this theme ends the preview', `${JSON.stringify(!s.live.previewing)}`);
await check('Editor shows the used theme (Coast)', `document.getElementById('screenTitle').textContent.includes('Coast')`);
// 4. Live on: preview and revert
await click('#liveSwitch'); await wait(2500);
s = await st();
await check('Live is on and running', `${JSON.stringify(s.inApp && s.live.running)}`);
await click('#navMine'); await wait(600);
await click('.g-card', 'Test pattern'); await wait(1200);
s = await st();
await check('Live on: previewing without a second stream', `${JSON.stringify(s.live.previewing && s.live.running)}`);
await click('#navSettings') ; // may not exist
await click('#navScreen'); await wait(1500);
s = await st();
await check('Live on: leaving ends the preview, live keeps running', `${JSON.stringify(!s.live.previewing && s.live.running)}`);
// turn live off again so the instance is as it started
await click('#liveSwitch'); await wait(1500);
console.log(c.errors.length ? 'PAGE ERRORS:\n' + c.errors.join('\n') : 'no page errors');
console.log(fails ? `${fails} FAILED` : 'ALL PASSED');
process.exit(fails ? 1 : 0);
