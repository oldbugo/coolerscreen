// Flow check for UI changes: drives the real page of a quiet test instance (tools/ui-shots/run.ps1) through the main flows
// and checks what happened. Prints PASS/FAIL per check and any page errors. Fake screen only.
//   node tools/ui-shots/flows.mjs <port>
import { connect } from './cdp.mjs';

const [port = '9333'] = process.argv.slice(2);
const c = await connect(port);
await c.send('Runtime.enable');
await c.size(1280, 860);
const wait = ms => new Promise(r => setTimeout(r, ms));
const js = (code) => c.eval(code);
let fails = 0;
const check = async (name, code, ms = 0) => {
  if (ms) await wait(ms);
  let ok, v;
  try { v = await js(code); ok = !!v; } catch (e) { ok = false; v = e.message; }
  console.log(`${ok ? 'PASS' : 'FAIL'}  ${name}${ok ? '' : '  -> ' + JSON.stringify(v)}`);
  if (!ok) fails++;
};
const click = (sel, text) => js(`(() => { const e = [...document.querySelectorAll(${JSON.stringify(sel)})].find(e => !${JSON.stringify(text || '')} || e.textContent.includes(${JSON.stringify(text || '')})); if (!e) return false; e.click(); return true; })()`);

// close anything left open
await js(`(() => { document.querySelectorAll('.dialog-actions button').forEach(b => /Cancel|Keep/.test(b.textContent) && b.click()); const d = document.getElementById('drawerClose'); if (d && d.offsetParent) d.click(); return 1; })()`);

// B1: My themes, select, Use this theme
await click('#navMine'); await wait(400);
await js(`document.querySelectorAll('.g-card.sel').forEach(e => e.click()); 1`);
await click('.g-card', 'Test pattern'); await wait(300);
await check('My themes: selecting a theme shows its bar with Use this theme', `!!document.querySelector('.sel-bar:not([hidden])') && [...document.querySelectorAll('.sel-bar button')].some(b => /Use this theme|Using|Use and store/.test(b.textContent))`);
await check('My themes: no Save button anywhere', `![...document.querySelectorAll('button')].some(b => /^\\s*Save\\b/.test(b.textContent))`);
// a theme that is already live shows a disabled "Using"; then the next flow starts from Coast instead
if (await js(`[...document.querySelectorAll('.sel-bar button')].some(b => b.textContent === 'Using')`)) {
  await click('.g-card', 'Coast'); await wait(300); await click('.sel-bar button', 'Use this theme'); await wait(1500);
  await check('Use this theme opens On screen with that theme', `!document.getElementById('screenView').hidden && document.getElementById('screenTitle').textContent.includes('Coast')`);
  await click('#navMine'); await wait(400); await click('.g-card', 'Test pattern'); await wait(300);
}
await click('.sel-bar button', 'Use this theme'); await wait(1500);
await check('Use this theme opens On screen with that theme', `!document.getElementById('screenView').hidden && document.getElementById('screenTitle').textContent.includes('Test pattern')`);

// top-bar Live switch: off, then on again (same setting as Settings > Live screen)
await check('Live switch is on', `document.getElementById('liveSwitch').checked`);
await click('#liveSwitch'); await wait(1500);
await check('Live switch turns off (and Settings agrees)', `!document.getElementById('liveSwitch').checked && document.querySelector('.live-switch').dataset.state === 'off'`);
await check('Editor says Live is off', `/Live is off/.test(document.querySelector('.ed-title').textContent)`);
await click('#liveSwitch'); await wait(2000);
await check('Live switch turns on again', `document.getElementById('liveSwitch').checked && document.querySelector('.live-switch').dataset.state !== 'off'`);

// C1: add an item by click, it appears in the layer list under the preview, then delete it
const n0 = await js(`document.querySelectorAll('.ed-layers .lrow').length`);
await click('.pal[data-type="text"]'); await wait(1200);
await check('Clicking Text in the palette adds a layer', `document.querySelectorAll('.ed-layers .lrow').length === ${n0} + 1`);
await check('The new item is selected and its Properties show', `/Text/.test(document.querySelector('.ed-right h3').textContent)`);
await check('Saved without a Save button', `/Sav/.test(document.querySelector('.save-state').textContent)`, 1200);
await click('.ed-layers .layers-head .iconbtn[aria-label="Delete the selected layers"]'); await wait(1000);
await check('Delete removes it again', `document.querySelectorAll('.ed-layers .lrow').length === ${n0}`);
// a 4.x item: its Style choice is a segmented control and still changes the item
if (await js(`!!document.querySelector('.pal[data-type="cores"]')`)) {
  await click('.pal[data-type="cores"]'); await wait(1200);
  await click('#pStyle button', 'Heat map'); await wait(1000);
  await check('CPU cores: Style "Heat map" is chosen and stays chosen', `document.querySelector('#pStyle button[aria-pressed="true"]')?.textContent === 'Heat map'`);
  await click('.ed-layers .layers-head .iconbtn[aria-label="Delete the selected layers"]'); await wait(800);
  await check('…and the item is deleted again', `document.querySelectorAll('.ed-layers .lrow').length === ${n0}`);
}
// Snap and Guides are switches and still toggle
await click('#snapSw'); await check('Snap toggles', `!document.getElementById('snapSw').checked`, 200); await click('#snapSw');
await click('#guidesSw'); await check('Guides toggles (guide bands hide)', `!document.getElementById('guidesSw').checked`, 300); await click('#guidesSw');
// Theme settings: Background section (stream A) still works
await js(`(() => { const s = document.querySelector('.stage'); s.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true })); return 1; })()`); await wait(500);
await check('Theme settings show with no item selected', `/Theme settings/.test(document.querySelector('.ed-right').textContent)`);
await check('Background: Picture / Live effect switch is there', `!!document.getElementById('bg-effect') && !!document.getElementById('bg-picture')`);

// Settings drawer: index links scroll to their section; a switch there toggles and saves
await click('#gear'); await wait(500);
await check('Settings index lists the sections', `document.querySelectorAll('.drawer-index .chip-link').length >= 10`);
await click('.drawer-index .chip-link', 'Live screen'); await wait(900);
await check('Index link scrolls Live screen into view', `(() => { const b = document.querySelector('#drawer .drawer-body').getBoundingClientRect(); const s = document.getElementById('setLive').getBoundingClientRect(); return Math.abs(s.top - b.top) < 40; })()`);
await check('Index marks the section in view', `document.querySelector('.drawer-index .chip-link.on')?.textContent === 'Live screen'`);
const fpsOv = await js(`document.getElementById('lvFpsOv').checked`);
await click('#lvFpsOv'); await wait(1200);
await check('Settings switch applies at once (frame-rate overlay)', `document.getElementById('lvFpsOv').checked === ${!fpsOv}`);
await click('#lvFpsOv'); await wait(1200);
await check('…and back', `document.getElementById('lvFpsOv').checked === ${fpsOv}`);
await js(`document.getElementById('drawerClose').click(); 1`); await wait(300);
await check('Settings closes', `document.getElementById('drawerBack').hidden`);

// Picture page: opens from the editor, one way back
await click('.ed-center button', 'Picture'); await wait(2000);
await check('Picture page opens', `!document.querySelector('.panes').hidden && !!document.querySelector('#detail .detail-title')`);
await check('Picture page: breadcrumb hidden while the context header shows', `getComputedStyle(document.querySelector('#detail .crumbs')).display === 'none' && !document.querySelector('#detail .ctx-bar').hidden`);
await click('#detail .ctx-bar button', 'Back'); await wait(1000);
await check('Back returns to On screen', `!document.getElementById('screenView').hidden`);

// dialogs: rename and delete open and cancel without changing anything
await click('#navMine'); await wait(400);
await js(`document.querySelectorAll('.g-card.sel').forEach(e => e.click()); 1`);
await click('.g-card', 'Coast'); await wait(300);
await click('.sel-bar button', 'Rename'); await wait(400);
await check('Rename dialog opens', `!!document.querySelector('.dialog #thRename')`);
await click('.dialog-actions button', 'Cancel'); await wait(300);
await click('.sel-bar button', 'Delete'); await wait(400);
await check('Delete dialog opens with Cancel focused (safe default)', `document.activeElement && document.activeElement.textContent === 'Cancel'`);
await click('.dialog-actions button', 'Cancel'); await wait(300);
await check('Nothing was deleted', `[...document.querySelectorAll('.g-card .g-name')].some(e => e.textContent === 'Coast')`);

// narrow window: nothing overlaps the window buttons
await c.size(900, 640); await wait(400);
await check('900 px: top bar content ends before the window buttons', `(() => { const g = document.getElementById('gear').getBoundingClientRect(); return g.right <= innerWidth - 138; })()`);
await c.size(1280, 860);

console.log(fails ? `${fails} check(s) failed` : 'all checks passed');
console.log(c.errors.length ? 'page errors:\n  ' + c.errors.join('\n  ') : 'no page errors');
c.close();
process.exit(fails ? 1 : 0);
