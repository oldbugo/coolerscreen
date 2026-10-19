// E3 check against a quiet debug instance (fake screen): the twin canvas shows live frames; dragging is rAF-smooth. node e3check.mjs <port> <repo> <outdir>
import { pathToFileURL } from 'node:url';
const [port = '9366', repo, out] = process.argv.slice(2);
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

// the quiet test window reports itself as hidden (it is off screen): pretend it is visible, as for a user
await js("Object.defineProperty(document, 'hidden', { get: () => false, configurable: true }); 1");
await click('#navMine'); await wait(800);
await click('.g-card', 'Test pattern'); await wait(400);
await click('.sel-bar button', 'Use this theme'); await wait(2500);
const st = () => js(`window.__TAURI_INTERNALS__.invoke('app_state').then(a => a.live.status)`);
await check('On screen shows the editor', `!document.getElementById('screenView').hidden && !!document.querySelector('.stage')`);
await check('Twin canvas exists in the stage', `!!document.querySelector('.stage canvas.twin')`, 1500);
await check('Live is off and the engine runs the editor-only stream', `window.__TAURI_INTERNALS__.invoke('app_state').then(a => !a.live.settings.inApp && a.live.status.twinning && a.live.status.running)`);
await check('The twin is live: canvas visible and stage marked', `!document.querySelector('.stage canvas.twin').hidden && document.querySelector('.stage').classList.contains('twin-on')`);
// frames advance
const seq = () => js(`(async () => { const base = (await window.__TAURI_INTERNALS__.invoke('app_state')).mediaBase; const r = await fetch(base + '/twin/frame.jpg?since=0', { cache: 'no-store' }); return Number(r.headers.get('X-Twin-Seq')) || 0; })()`);
const s1 = await seq(); await wait(2000); const s2 = await seq();
console.log(`frames published in 2 s: ${s2 - s1}`);
await check('Frames keep coming (a clock ticks once a second; at most about 15 per second)', `${s2 - s1 >= 1 && s2 - s1 <= 45}`);
await check('The canvas has real pixels (not blank)', `(() => { const cv = document.querySelector('.stage canvas.twin'); const d = cv.getContext('2d').getImageData(0, 0, cv.width, cv.height).data; let n = 0; for (let i = 0; i < d.length; i += 4 * 97) n += d[i] + d[i+1] + d[i+2] > 30 ? 1 : 0; return n > 100; })()`);
await c.shot(out + '/twin-editor.png', 1280, 860);
// leaving the editor ends the editor-only stream
await click('#navMine'); await wait(2500);
let s = await st();
await check('Leaving On screen stops the editor-only stream', `${JSON.stringify(!s.twinning && !s.running)}`);
await click('#navScreen'); await wait(2500);
s = await st();
await check('Coming back starts it again', `${JSON.stringify(s.twinning && s.running)}`);

// smooth dragging: add a text item so there is a box, then drag with 240 Hz pointer events and count layout updates per frame
await click('.pal[data-type="text"]'); await wait(1200);
const res = await js(`(async () => {
  const box0 = document.querySelector('.wbox.sel') || document.querySelector('.wbox'); const id = box0.dataset.id;
  const cur = () => document.querySelector('.wbox[data-id="' + id + '"]');
  const r = box0.getBoundingClientRect();
  const sx = r.left + r.width / 2, sy = r.top + r.height / 2;
  const stageR = document.querySelector('.stage').getBoundingClientRect(); const dir = sx > stageR.left + stageR.width / 2 ? -1 : 1;
  let saves = 0; const sv = document.querySelector('.save-state'); let lastTxt = sv.textContent; const mo2 = new MutationObserver(() => { const t = sv.textContent; if (t !== lastTxt) { if (/Sav(ing|ed)|Saving/.test(t) && /ing/.test(t)) saves++; lastTxt = t; } }); mo2.observe(sv, { childList: true, characterData: true, subtree: true });
  let styleWrites = 0; const mo = new MutationObserver(m => { styleWrites += m.length; }); mo.observe(document.querySelector('.boxes'), { attributes: true, attributeFilter: ['style'], subtree: true });
  const ev = (type, x, y) => cur().dispatchEvent(new PointerEvent(type, { bubbles: true, cancelable: true, pointerId: 1, clientX: x, clientY: y, button: 0, buttons: 1, pointerType: 'mouse' }));
  ev('pointerdown', sx, sy);
  let frames = 0; let stop = false; const f = () => { frames++; if (!stop) requestAnimationFrame(f); }; requestAnimationFrame(f);
  const t0 = performance.now(); let n = 0;
  while (performance.now() - t0 < 1000) { n++; ev('pointermove', sx + dir * n * 0.4, sy + n * 0.1); await new Promise(r => setTimeout(r, 4)); }
  stop = true; styleWrites += mo.takeRecords().length; const dragWrites = styleWrites; mo.disconnect();
  ev('pointerup', sx + dir * n * 0.4, sy + n * 0.1);
  await new Promise(r => setTimeout(r, 700)); mo2.disconnect();
  return { events: n, frames, styleWrites: dragWrites, invokes: saves, moved: cur().getBoundingClientRect().left - r.left };
})()`);
console.log('drag:', JSON.stringify(res));
await check('Pointer events are folded into animation frames (placements per second at most the frame rate, fewer than the events)', `${res.styleWrites / 4 < res.events * 0.7 && res.styleWrites / 4 <= res.frames + 4}`);
await check('Theme updates while dragging are throttled (about 8 per second), plus the final one', `${res.invokes >= 3 && res.invokes <= 14}`);
await check('The box followed the pointer', `${Math.abs(res.moved) > 60}`);
console.log(c.errors.length ? 'PAGE ERRORS:\n' + c.errors.join('\n') : 'no page errors');
console.log(fails ? `${fails} FAILED` : 'ALL PASSED');
process.exit(fails ? 1 : 0);
