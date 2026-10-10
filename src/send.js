// "Put this version on the screen": the version goes to the loop (standby) slot and plays immediately. The intro
// (startup) slot always holds the app's boot splash; it is uploaded first only when the recorded intro is not already
// this exact splash. Space is checked before anything is sent.
const path = require('path');
const fs = require('fs');
const lib = require('./library');
const settings = require('./settings');
const slots = require('./slots');
const paths = require('./paths');
const splash = require('./splash');
const deviceOps = require('./deviceOps');
const { makeBlank } = require('./convert');

const mb = b => (b / 1e6).toFixed(2) + ' MB';

/** onProgress(frac, message, info) — info.bytesTotal / info.bytesSent for time-left estimates. */
async function sendVersion(vid, onProgress = () => {}) {
  const f = lib.findVersion(vid);
  const file = f && lib.devicePath(vid);
  if (!file || f.v.status !== 'ready') throw new Error('This version is not ready yet.');
  const o = lib.readOriginal(f.oid);
  const size = fs.statSync(file).size;
  const budget = slots.budgets().loop;
  if (size > budget) throw new Error(`Too big for the screen: ${mb(size)}, but there is room for ${mb(budget)}. Make a shorter version or choose "Best that fits".`);

  const items = [];
  const cur = settings.load().slots.startup;
  let sp = null;
  try { sp = await splash.get(); } catch (e) { console.log('Splash could not be made: ' + e.message); }
  if (sp && !(cur && cur.splash === sp.id && !cur.incomplete)) items.push({ file: sp.file, slot: 'startup', size: sp.size });
  items.push({ file, slot: 'standby', size });
  const bytesTotal = items.reduce((a, x) => a + x.size, 0);

  const record = {
    name: o ? o.name : '(deleted clip)', size, originalId: f.oid, versionId: vid, label: f.v.label || `v${f.v.n}`,
  };
  const started = Date.now();
  let loopStarted = false;
  let r;
  try {
    r = await deviceOps.sendFiles(items, (frac, msg) => {
      // sendFiles reports per file: frac in [i/total, (i+1)/total). The loop file is the last one.
      if (frac > (items.length - 1) / (items.length + 0.1)) loopStarted = true;
      onProgress(frac, msg, { bytesTotal, bytesSent: Math.round(Math.min(1, frac) * bytesTotal) });
    }, { playLoop: true });
  } catch (e) {
    if (loopStarted) {
      if (items[0].slot === 'startup') settings.recordSlot('startup', { name: '(boot splash)', size: sp.size, splash: sp.id });
      settings.recordSlot('standby', { ...record, incomplete: true });
    }
    throw e;
  }
  if (items[0].slot === 'startup') settings.recordSlot('startup', { name: '(boot splash)', size: sp.size, splash: sp.id });
  settings.recordSlot('standby', record);
  const secs = (Date.now() - started) / 1000;
  if (secs > 20 && r.transport !== 'fake') settings.save({ ...settings.load(), uploadRate: Math.round(bytesTotal / secs) });
  return { ...r, versionId: vid, name: record.name, label: record.label };
}

// CLI maintenance: upload a 1-frame black clip to a slot.
async function clearSlot(slot, onProgress) {
  if (slot !== 'startup' && slot !== 'standby') throw new Error('Slot must be startup (intro) or standby (loop)');
  fs.mkdirSync(paths.base, { recursive: true });
  const blank = path.join(paths.base, 'blank.mp4');
  if (!fs.existsSync(blank)) await makeBlank(blank);
  const r = await deviceOps.sendFiles([{ file: blank, slot }], onProgress);
  settings.recordSlot(slot, { name: '(blank)', size: fs.statSync(blank).size });
  return { ...r, slots: [slot] };
}

module.exports = { sendVersion, clearSlot };
