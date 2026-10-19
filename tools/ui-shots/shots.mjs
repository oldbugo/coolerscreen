// The UI screenshot set used by the 4.0 UI audit and every restyle step (docs/v4/ui-audit.md).
//   node tools/ui-shots/shots.mjs <port> <outDir> [only-prefix]
// Needs a quiet test instance (tools/ui-shots/run.ps1) whose library holds at least three themes, Live on, and one theme
// stored as Embedded with some items (the scratch home used in D1). Drives the page only through clicks and keys,
// so it exercises the real flows. Nothing here talks to a real screen.
import { mkdirSync } from 'node:fs';
import { connect } from './cdp.mjs';

const [port = '9333', out = 'shots', only = ''] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
const c = await connect(port);
const wait = ms => new Promise(r => setTimeout(r, ms));
const js = async (code, ms = 500) => { const v = await c.eval(code); await wait(ms); return v; };
const click = (sel, text) => js(`(() => { const els = [...document.querySelectorAll(${JSON.stringify(sel)})]${text ? `.filter(e => ${JSON.stringify(text)}.split('|').some(t => e.textContent.includes(t)))` : ''}; const e = els[0]; if (!e) return 'missing ${sel} ${text || ''}'; e.click(); return 'ok'; })()`);
const shot = async (name, w = 1280, h = 860) => { if (only && !name.startsWith(only)) return; await c.shot(`${out}/${name}.png`, w, h); console.log('shot', name); };
const closeOverlays = () => js(`(() => { for (const b of document.querySelectorAll('.dialog-actions button')) if (/Cancel|Keep|Close/.test(b.textContent)) { b.click(); } const d = document.getElementById('drawerClose'); if (d && d.offsetParent) d.click(); const v = document.querySelector('.versions-pop'); if (v) v.hidden = true; return 1; })()`);

await c.size(1280, 860);
await closeOverlays();

// Start from the same state: "Test pattern" live (storing a theme can make it the live one).
await click('#navMine'); await js('document.querySelectorAll(".g-card.sel").forEach(e => e.click()); 1');
await click('.g-card', 'Test pattern'); await click('.sel-bar button', 'Use this theme'); await wait(1500);
await click('#navMine'); await js('document.querySelectorAll(".g-card.sel").forEach(e => e.click()); 1');
await shot('01-my-themes');
await click('.g-card', 'Coast'); await shot('02-my-themes-selected');
await shot('21-my-themes-900x640', 900, 640);
await shot('24-my-themes-1920x1080', 1920, 1080);
await c.size(1280, 860);
await click('.sel-bar button', 'Delete'); await wait(400); await shot('20-delete-dialog'); await closeOverlays();

// On screen editor
await click('#navScreen', null); await wait(1500);
await js('document.querySelector(".ed-center")?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true })); 1');
await shot('05-editor-theme-settings');
await js('document.querySelector(".ed-right").scrollTop = 99999; 1'); await shot('06-editor-theme-settings-lower');
await js('document.querySelector(".lrow")?.click(); 1', 800); await shot('07-editor-item-selected');
await js('(() => { const r = [...document.querySelectorAll(".lrow")].find(x => /CPU|Meters|Sensor/.test(x.textContent)); if (r) r.click(); return 1; })()', 800);
await shot('08-editor-sensor-properties');
await shot('22-editor-900x640', 900, 640);
await shot('23-editor-1920x1080', 1920, 1080);
await c.size(1280, 860);
// the effect background section (Theme settings), then back to the picture
await js('(() => { const s = document.querySelector(".stage"); s && s.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true })); return 1; })()', 600);
await js('document.getElementById("bg-effect")?.click(); 1', 1200);
await js('(() => { const b = document.getElementById("bgEffect"); b && b.scrollIntoView(); return 1; })()', 600);
await shot('09-editor-effect-background');
await js('document.getElementById("bg-picture")?.click(); 1', 1200);

// Settings drawer, section by section
await click('#gear'); await wait(400);
await shot('10-settings-top');
for (const [i, id] of [[11, 'setThemes'], [12, 'setFit'], [13, 'setSources'], [14, 'setLive'], [15, 'setAbout']].map(x => x)) {
  await js(`document.getElementById(${JSON.stringify(id)}).scrollIntoView(); 1`, 400);
  await shot(`${i}-settings-${id.slice(3).toLowerCase()}`);
}
await closeOverlays();

// Browse
await click('#navBrowse'); await wait(600); await shot('16-browse');

// Picture page (from the editor)
await click('#navScreen'); await wait(1200);
await click('.ed-center button', 'Picture'); await wait(2500);
await shot('17-picture-page');
await js('document.getElementById("detail").scrollTop = 99999; 1', 600); await shot('18-picture-page-lower');
await js('document.getElementById("detail").scrollTop = 0; 1', 300);
await click('#detail button', 'Versions'); await wait(500); await shot('19-versions-popover'); await closeOverlays();
await click('#detail .ctx-bar button', 'Back'); await wait(800);

// Store as Embedded on a theme that is not Embedded (the fake screen takes the upload)
await click('#navMine'); await wait(500);
await js('document.querySelectorAll(".g-card.sel").forEach(e => e.click()); 1');
await click('.g-card', 'Coast'); await click('.sel-bar button', 'Store as Embedded|Use and store'); await wait(1200);
await shot('04-store-as-embedded-dialog');
await wait(9000);
await click('#navMine'); await js('document.querySelectorAll(".g-card.sel").forEach(e => e.click()); 1');

// keyboard focus
await js('document.activeElement && document.activeElement.blur(); 1');
for (let i = 0; i < 3; i++) await c.key('Tab', 'Tab', 9);
await shot('25-keyboard-focus');

c.close();
