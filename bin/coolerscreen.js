#!/usr/bin/env node
const path = require('path');
const lib = require('../src/library');
const gallery = require('../src/gallery');
const send = require('../src/send');
const setup = require('../src/setup');
const slots = require('../src/slots');
const deviceOps = require('../src/deviceOps');
const { DEVICE_FPS } = require('../src/convert');

const args = process.argv.slice(2);
const cmd = args.shift();
const flags = {};
const pos = [];
for (let i = 0; i < args.length; i++) {
  if (args[i].startsWith('--')) {
    const k = args[i].slice(2);
    if (k === 'no-send' || k === 'help') flags[k] = true; else flags[k] = args[++i];
  } else pos.push(args[i]);
}

const usage = `CoolerScreen CLI
  coolerscreen list                     List clips (originals) in the Gallery folder and their versions.
  coolerscreen add <file>               Copy a file into the Gallery folder.
  coolerscreen make <clip> [options]    Make a new version of a clip (<clip> = id or name from "list").
      --start <sec> --length <sec>      Part of the clip (default: from the start, up to 10 s)
      --quality auto|1-100              (default auto = best quality that fits the screen)
      --zoom <0.2-4> --x <0-1> --y <0-1>  Framing: zoom 1 fills the screen; x/y 0.5 = centred
      --corners <0-300>                 Rounded corners radius in pixels (0 = square)
      --detail <0-100>                  Simplify fine detail (smaller file); 0 = off
      --blend <0-1.5>                   Blend the last seconds into the start for a seamless loop
  coolerscreen send <version id>        Put a version on the screen (about 3 minutes for a full-size clip).
  coolerscreen upload <file> [options] [--no-send]   add + make + send in one go.
  coolerscreen setup     One-time: switch the screen to the WinUSB driver (asks for administrator approval).
                         Without it the screen turns its display off ~30 s after any program closes its COM port.
  coolerscreen reconnect Fix "screen not found" after boot (Windows failed to enumerate it): resets its USB port (admin prompt).
  coolerscreen restore   Undo setup (back to the standard COM-port driver).
  coolerscreen transport Show how the screen is reachable (winusb / serial / none).
  coolerscreen clear startup|standby   Upload a 1-frame black clip to that slot (maintenance).
  coolerscreen slots     Show what this app last wrote to the screen and the space for a clip.
  coolerscreen info      Show screen info.
  coolerscreen themes [--page n] [--download <themeId>]
                         List Lovingcool's online themes for this screen, or add one to the Gallery folder.
Versions are always made at ${DEVICE_FPS} frames per second, the rate the screen plays at.
For the app run "npm run app" (window) or "npm start" (browser).`;

const mb = b => (b / 1e6).toFixed(2) + ' MB';
const bar = label => f => process.stdout.write(`\r${label} ${Math.round(f * 100)}%      `);

function findOriginal(key) {
  const all = lib.listOriginals();
  return all.find(o => o.id === key) || all.find(o => o.name === key) || all.find(o => o.name.toLowerCase() === String(key).toLowerCase());
}

function makeOpts() {
  const o = {};
  if (flags.start != null) o.start = Number(flags.start);
  if (flags.length != null) o.length = Number(flags.length);
  let quality = flags.quality ?? 'auto';
  if (quality !== 'auto') {
    quality = Number(quality);
    if (!(quality >= 1 && quality <= 100)) throw new Error('--quality must be "auto" or 1-100');
  }
  o.quality = quality;
  if (flags.zoom != null || flags.x != null || flags.y != null) o.frame = { zoom: flags.zoom ?? 1, x: flags.x ?? 0.5, y: flags.y ?? 0.5 };
  if (flags.corners != null) o.corners = { on: Number(flags.corners) > 0, r: Number(flags.corners) };
  if (flags.detail != null) o.detail = Number(flags.detail);
  if (flags.blend != null) o.blend = Number(flags.blend);
  return o;
}

async function make(oid) {
  let v;
  try { v = lib.createVersion(oid, makeOpts()); }
  catch (e) { if (e.code === 'duplicate') { console.log(e.message); return lib.findVersion(e.versionId).v; } throw e; }
  const r = await lib.runVersion(v.id, (f, msg) => bar(msg)(f));
  console.log(`\nMade v${r.n} (version id ${r.id}): ${mb(r.result.size)}, ${r.result.length} s, quality ${r.result.quality}/100${r.result.tooBig ? '  TOO BIG for the screen, it will not be sent' : ''}`);
  return r;
}

async function sendIt(vid) {
  const sr = await send.sendVersion(vid, (f, msg) => bar(msg)(f));
  if (sr.serialWarning) console.log('\nWARNING: ' + sr.serialWarning);
  console.log(`\nDone: ${sr.name} · ${sr.label} is on the screen (the screen restarted to show it).`);
}

(async () => {
  if (cmd === 'setup' || cmd === 'restore' || cmd === 'reconnect') {
    const r = await (cmd === 'setup' ? setup.setupWinUsb : cmd === 'reconnect' ? setup.reconnect : setup.restoreSerial)((f, msg) => bar(msg)(f));
    console.log('\n' + (r.warning || (r.already ? 'Already set up.' : 'Done.')));
  } else if (cmd === 'transport') {
    console.log(JSON.stringify(await deviceOps.state(), null, 2));
  } else if (cmd === 'info') {
    console.log(JSON.stringify(await deviceOps.checkScreen(), null, 2));
  } else if (cmd === 'list') {
    await gallery.init();
    const ctx = lib.context();
    for (const o of lib.listOriginals()) {
      console.log(`${o.id}  ${o.name}${o.missing ? `  (file missing since ${o.missing.since.slice(0, 10)})` : ''}`);
      for (const v of lib.versions(o.id)) {
        const r = v.result || {};
        const flagsTxt = [];
        if (ctx.standby && ctx.standby.versionId === v.id) flagsTxt.push('ON SCREEN');
        if (r.fps && r.fps !== DEVICE_FPS) flagsTxt.push(`made at ${r.fps} fps: plays too ${r.fps < DEVICE_FPS ? 'fast' : 'slow'}, re-make`);
        if (v.status === 'ready' && v.notchKey !== ctx.notchKey) flagsTxt.push('older notch');
        if (v.status === 'ready' && r.size > ctx.budget) flagsTxt.push('TOO BIG');
        console.log(`    ${v.id}  ${v.label || 'v' + v.n}  ${v.status === 'ready' ? `${mb(r.size)} ${r.length} s quality ${r.quality}` : v.status}${flagsTxt.length ? '  [' + flagsTxt.join(', ') + ']' : ''}`);
      }
    }
  } else if (cmd === 'themes') {
    const themes = require('../src/themes');
    const os = require('os');
    if (flags.download) {
      let t = themes.findCached(flags.download);
      for (let pg = 1; !t && pg <= 20; pg++) { const r = await themes.list(pg); t = r.themes.find(x => x.themeId === flags.download); if (!r.themes.length) break; }
      if (!t) throw new Error('No theme with that id. See "coolerscreen themes".');
      await gallery.init();
      const file = await themes.download(t, os.tmpdir(), f => bar('Downloading')(f));
      try { const id = await gallery.addFile(file, t.file.name); console.log(`\nAdded "${t.name}" to the gallery (clip id ${id}).`); }
      finally { require('fs').rmSync(file, { force: true }); }
    } else {
      const r = await themes.list(flags.page || 1);
      console.log(`Lovingcool themes for 2400x1080: ${r.total} (page ${r.page}, ${r.pageSize} per page)`);
      for (const t of r.themes) console.log(`  ${t.themeId}  ${t.name}`);
    }
  } else if (cmd === 'slots') {
    const b = slots.budgets();
    console.log(`space for a clip: ${mb(b.loop)} (of ${mb(b.total)}; the rest holds the boot splash)`);
    for (const s of ['startup', 'standby']) {
      const v = b.slots[s];
      console.log(`${s === 'startup' ? 'intro (startup)' : 'clip (standby)'}: ${v ? `${v.name}${v.label ? ' · ' + v.label : ''} (${mb(v.size)}, ${v.at})${v.incomplete ? ' UPLOAD DID NOT FINISH' : ''}` : 'unknown'}`);
    }
  } else if (cmd === 'clear' && ['startup', 'standby'].includes(pos[0])) {
    await send.clearSlot(pos[0], (f, msg) => bar(msg)(f));
    console.log('\nCleared ' + pos[0] + '.');
  } else if (cmd === 'add' && pos[0]) {
    await gallery.init();
    const id = await gallery.addFile(path.resolve(pos[0]));
    console.log(`Added: ${lib.readOriginal(id).name} (clip id ${id})`);
  } else if (cmd === 'make' && pos[0]) {
    await gallery.init();
    const o = findOriginal(pos.join(' '));
    if (!o) throw new Error('No clip with that id or name. See "coolerscreen list".');
    await make(o.id);
  } else if (cmd === 'send' && pos[0]) {
    if (!lib.findVersion(pos[0])) throw new Error('No version with that id. See "coolerscreen list".');
    await sendIt(pos[0]);
  } else if (cmd === 'upload' && pos[0]) {
    await gallery.init();
    const id = await gallery.addFile(path.resolve(pos[0]));
    const v = await make(id);
    if (flags['no-send'] || !v || v.status !== 'ready' || v.result.tooBig) return;
    await sendIt(v.id);
  } else {
    console.log(usage);
    process.exitCode = cmd && cmd !== '--help' ? 1 : 0;
  }
})().catch(e => { console.error('\nError: ' + e.message); process.exit(1); });
