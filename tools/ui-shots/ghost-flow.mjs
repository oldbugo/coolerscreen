// Ghosting check against a quiet debug instance (fake screen) with a screen fit: drag a Meters item with the pointer and look at the
// twin; also compare the picture the engine sends (frame dump) after the move with a fresh start. node ghostcheck.mjs <port> <repo> <outdir>
import { pathToFileURL } from 'node:url';
const [port = '9366', repo, out] = process.argv.slice(2);
const { connect } = await import(pathToFileURL(repo + '/tools/ui-shots/cdp.mjs').href);
const c = await connect(port);
await c.send('Runtime.enable');
await c.size(1280, 860);
const wait = ms => new Promise(r => setTimeout(r, ms));
const js = code => c.eval(code);
const click = (sel, text) => js(`(() => { const e = [...document.querySelectorAll(${JSON.stringify(sel)})].find(e => !${JSON.stringify(text || '')} || e.textContent.includes(${JSON.stringify(text || '')})); if (!e) return false; e.click(); return true; })()`);
await js("Object.defineProperty(document, 'hidden', { get: () => false, configurable: true }); 1");
await click('#navMine'); await wait(800);
await click('.g-card', 'Test pattern'); await wait(400);
await click('.sel-bar button', 'Use this theme'); await wait(2500);
await js("window.__TAURI_INTERNALS__.invoke('settings_update', { screenFit: { top: 4, left: 9 } })"); await wait(800);
await click('#navMine'); await wait(500); await click('#navScreen'); await wait(2500);
await click('.pal[data-type="meters"]'); await wait(1500);
const info = await js(`(() => { const b = document.querySelector('.wbox.sel') || document.querySelector('.wbox'); const r = b.getBoundingClientRect(); const s = document.querySelector('.stage').getBoundingClientRect(); return JSON.stringify({ id: b.dataset.id, x: r.left, y: r.top, w: r.width, h: r.height, sx: s.left, sy: s.top, sw: s.width, sh: s.height }); })()`);
const I = JSON.parse(info);
console.log('box', info);
const sx = I.x + I.w / 2, sy = I.y + I.h / 2;
const ev = (type, x, y) => js(`(() => { const e = document.querySelector('.wbox[data-id="${I.id}"]'); e.dispatchEvent(new PointerEvent('${type}', { bubbles: true, cancelable: true, pointerId: 1, clientX: ${x}, clientY: ${y}, button: 0, buttons: 1, pointerType: 'mouse' })); return 1; })()`);
await ev('pointerdown', sx, sy);
const dirx = sx > I.sx + I.sw / 2 ? -1 : 1;
for (let n = 1; n <= 40; n++) { await ev('pointermove', sx + dirx * n * 7, sy + n * 2); await wait(25); if (n === 20) await c.shot(out + '/ghost-mid.png', 1280, 860); }
await wait(300);
await c.shot(out + '/ghost-held.png', 1280, 860);
await ev('pointerup', sx + dirx * 280, sy + 80);
await wait(1500);
await c.shot(out + '/ghost-after.png', 1280, 860);
console.log(c.errors.length ? 'PAGE ERRORS:\n' + c.errors.join('\n') : 'no page errors');
process.exit(0);
