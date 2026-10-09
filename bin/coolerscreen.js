#!/usr/bin/env node
const path = require('path');
const lib = require('../src/library');
const send = require('../src/send');
const setup = require('../src/setup');
const slots = require('../src/slots');
const deviceOps = require('../src/deviceOps');

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
  coolerscreen upload <file> [options]
      Convert <file>, add it to the library, and upload it to the screen.
      --slot loop|startup|both      (default loop = the clip that plays forever; the intro is made black.
                                     startup = intro only, plays once at power-on; both = same file in each, <= half the space)
      --quality auto|1-100          (default auto = fit to the free space; manual 1-100, higher = better)
      --fps 10-30                   (default 20)
      --start <sec>  --length <sec> (trim; default whole clip up to 10 s)
      --name <text>  --no-send      (convert only)
  coolerscreen setup     One-time: switch the screen to the WinUSB driver (asks for administrator approval).
                         Without it the screen turns its display off ~30 s after any program closes its COM port.
  coolerscreen restore   Undo setup (back to the standard COM-port driver).
  coolerscreen transport Show how the screen is reachable (winusb / serial / none).
  coolerscreen clear startup|standby   Upload a 1-frame black clip to free that slot's space.
  coolerscreen slots     Show what was last sent to each slot and the space budget.
  coolerscreen info      Show screen info.
  coolerscreen list      List library items.
For the web app run "npm start".`;

const mb = b => (b / 1e6).toFixed(2) + ' MB';
const bar = label => f => process.stdout.write(`\r${label} ${Math.round(f * 100)}%      `);

(async () => {
  if (cmd === 'setup' || cmd === 'restore') {
    const r = await (cmd === 'setup' ? setup.setupWinUsb : setup.restoreSerial)((f, msg) => bar(msg)(f));
    console.log('\n' + (r.warning || (r.already ? 'Already set up.' : 'Done.')));
  } else if (cmd === 'transport') {
    console.log(JSON.stringify(await deviceOps.state(), null, 2));
  } else if (cmd === 'info') {
    console.log(JSON.stringify(await deviceOps.checkScreen(), null, 2));
  } else if (cmd === 'list') {
    for (const m of lib.list()) {
      console.log(`${m.id}  ${m.name}  ${m.status === 'ready' ? `${m.fps}fps q${m.q} ${mb(m.size)}${m.tooBig ? ' TOO BIG' : ''}` : m.status}${m.maskOutdated ? '  (mask outdated)' : ''}`);
    }
  } else if (cmd === 'slots') {
    const b = slots.budgets();
    console.log(`loop clip budget (intro made blank): ${mb(b.loop)}`);
    for (const s of ['startup', 'standby']) {
      const v = b.slots[s];
      console.log(`${s}: ${v ? `${v.name} (${mb(v.size)}, ${v.at})` : 'unknown'}   space available: ${mb(b[s])}${b.assumed[s] ? ' (other slot unknown, assuming ' + mb(b.assumedBytes) + ')' : ''}`);
    }
    console.log(`both: ${mb(b.both)} each`);
  } else if (cmd === 'clear' && ['startup', 'standby'].includes(pos[0])) {
    await send.clearSlot(pos[0], (f, msg) => bar(msg)(f));
    console.log('\nCleared ' + pos[0] + '.');
  } else if (cmd === 'upload' && pos[0]) {
    const slot = flags.slot || 'loop';
    if (!['loop', 'startup', 'standby', 'both'].includes(slot)) throw new Error('--slot must be loop, startup or both');
    let quality = flags.quality ?? 'auto';
    if (quality !== 'auto') {
      quality = Number(quality);
      if (!(quality >= 1 && quality <= 100)) throw new Error('--quality must be "auto" or 1-100');
    }
    const id = lib.addSource(path.resolve(pos[0]), flags.name);
    const meta = await lib.convertEntry(id, {
      start: flags.start != null ? Number(flags.start) : undefined,
      length: flags.length != null ? Number(flags.length) : undefined,
      fps: flags.fps != null ? Number(flags.fps) : undefined,
      quality, slot,
    }, (f, msg) => bar(msg)(f));
    console.log(`\nConverted: ${meta.fps} fps, q=${meta.q} (quality ${meta.quality}/100), ${mb(meta.size)}, ${meta.trim.length}s  (library id ${id})`);
    if (meta.tooBig) console.log(`WARNING: too big for "${slot}" (limit ${mb(meta.budgetBytes)}); it will not be sent.`);
    if (flags['no-send']) return;
    const sr = await send.sendItem(id, slot, (f, msg) => bar(msg)(f));
    if (sr.serialWarning) console.log('\nWARNING: ' + sr.serialWarning);
    console.log('\nDone. The screen may restart by itself for a few seconds.');
  } else {
    console.log(usage);
    process.exitCode = cmd && cmd !== '--help' ? 1 : 0;
  }
})().catch(e => { console.error('\nError: ' + e.message); process.exit(1); });
