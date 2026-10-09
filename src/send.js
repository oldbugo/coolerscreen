// High-level "put this item / blank on the screen" with slot budget checks and bookkeeping.
// Modes: loop (default; standby slot, intro made blank first, plays immediately), startup (intro only),
// standby (raw loop slot, no blank intro / no play), both (same file in both slots).
const path = require('path');
const fs = require('fs');
const lib = require('./library');
const settings = require('./settings');
const slots = require('./slots');
const paths = require('./paths');
const deviceOps = require('./deviceOps');
const { makeBlank } = require('./convert');

const MODES = ['loop', 'startup', 'standby', 'both'];

async function blankFile() {
  fs.mkdirSync(paths.base, { recursive: true });
  const blank = path.join(paths.base, 'blank.mp4');
  if (!fs.existsSync(blank)) await makeBlank(blank);
  return { blank, size: fs.statSync(blank).size };
}

async function sendItem(id, mode, onProgress) {
  mode = mode || 'loop';
  if (!MODES.includes(mode)) throw new Error('Slot must be loop, startup (intro), standby or both');
  const meta = lib.get(id);
  const file = meta && lib.devicePath(id);
  if (!file) throw new Error('This item has no converted file yet.');
  const size = fs.statSync(file).size;
  const budget = slots.budgetFor(mode);
  if (size > budget) {
    throw new Error(`Too big for the screen: ${(size / 1e6).toFixed(2)} MB, but only ${(budget / 1e6).toFixed(2)} MB is available for "${mode}". Re-convert with lower quality/fps/length.`);
  }
  const queue = [];
  const record = [];
  let playLoop = false;
  if (mode === 'loop') {
    const cur = settings.load().slots.startup;
    if (!slots.isBlank(cur)) {
      const b = await blankFile();
      queue.push({ file: b.blank, slot: 'startup' });
      record.push(['startup', { name: '(blank)', size: b.size }]);
    }
    queue.push({ file, slot: 'standby' });
    record.push(['standby', { name: meta.name, size }]);
    playLoop = true;
  } else if (mode === 'both') {
    queue.push({ file, slot: 'startup' }, { file, slot: 'standby' });
    record.push(['startup', { name: meta.name, size }], ['standby', { name: meta.name, size }]);
  } else {
    queue.push({ file, slot: mode });
    record.push([mode, { name: meta.name, size }]);
  }
  const r = await deviceOps.sendFiles(queue, onProgress, { playLoop });
  for (const [s, v] of record) settings.recordSlot(s, v);
  return { ...r, mode };
}

async function clearSlot(slot, onProgress) {
  if (slot !== 'startup' && slot !== 'standby') throw new Error('Slot must be startup (intro) or standby (loop)');
  const b = await blankFile();
  const r = await deviceOps.sendFiles([{ file: b.blank, slot }], onProgress);
  settings.recordSlot(slot, { name: '(blank)', size: b.size });
  return { ...r, slots: [slot] };
}

module.exports = { sendItem, clearSlot };
