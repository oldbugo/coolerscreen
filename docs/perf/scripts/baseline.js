// Baseline: what 1.2.0 does when a newly imported clip is opened, then saved, timed per stage (one at a time, like the gate).
// usage: node baseline.js <clip> <tag>
const path = require('path');
const fs = require('fs');
const SCR = path.resolve(__dirname, '..');
process.env.COOLERSCREEN_HOME = path.join(SCR, 'home');
const { REPO, procs, stage, startGpu, stopGpu } = require('./mon');
const conv = require(path.join(REPO, 'src', 'convert'));
const { findLoops } = require(path.join(REPO, 'src', 'loopsearch'));
const slots = require(path.join(REPO, 'src', 'slots'));
const settings = require(path.join(REPO, 'src', 'settings'));

(async () => {
  const file = path.resolve(process.argv[2]);
  const tag = process.argv[3] || path.basename(file);
  const dir = path.join(SCR, 'out', 'base-' + tag);
  fs.rmSync(dir, { recursive: true, force: true }); fs.mkdirSync(dir, { recursive: true });
  const s = settings.load();
  const budget = slots.budgets(s).loop;
  const mask = { mode: s.mask.mode, circle: s.mask.circle, band: s.mask.band };
  startGpu();
  await new Promise(r => setTimeout(r, 800));
  const res = [];
  const info = await stage('probe', () => conv.probe(file), res);
  await stage('sourceAssets(thumb,frame,proxy?)', () => conv.makeSourceAssets(file, dir, info), res);
  const len = Math.min(10, info.duration);
  const draft0 = { start: 0, length: Math.floor(len * 18) / 18, quality: 'auto', frame: conv.FRAME_DEFAULT, mask, corners: { on: false, r: 80 }, detail: 0, blend: 0, budgetBytes: budget, maxSeconds: 10 };
  const nE = procs.length;
  const e1 = await stage('estimate#1 (initial draft)', () => conv.estimate(file, dir, draft0), res);
  console.log('  estimate ->', JSON.stringify({ q: e1.q, bytes: e1.bytes, fits: e1.fits, encodes: procs.length - nE }));
  // loop search as library.searchLoops does
  const bps = await stage('loop: sampleBytesPerSecond', () => conv.sampleBytesPerSecond(file, dir, { start: 0, length: Math.min(2, info.duration), frame: conv.FRAME_DEFAULT, mask, corners: draft0.corners, budgetBytes: budget }, 25, 0), res);
  const maxLen = Math.max(3, Math.min(Math.max(1, Math.floor((budget * 0.94 / bps) * 2) / 2), info.duration - 0.2, 30));
  const marks = {};
  const lr = await stage('loop: findLoops (all)', () => findLoops(file, dir, { duration: info.duration, fps: info.fps, minLen: 3, maxLen,
    onProgress: (f, m) => { if (!marks[m]) marks[m] = Date.now(); } }), res);
  const ks = Object.keys(marks); ks.forEach((k, i) => console.log('   ', k, (i + 1 < ks.length ? marks[ks[i + 1]] : Date.now()) - marks[k], 'ms (approx)'));
  // cached second run (compute only)
  await stage('loop: findLoops (features cached)', () => findLoops(file, dir, { duration: info.duration, fps: info.fps, minLen: 3, maxLen }), res);
  const c0 = lr.candidates[0];
  await stage('loop: 5 card previews', async () => { for (let i = 0; i < lr.candidates.length; i++) { const c = lr.candidates[i]; await conv.renderLoopPreview(file, path.join(dir, `loop-${i}.mp4`), { start: c.start, length: c.length, blend: c.ratio > 1 ? Math.min(0.33, c.start, c.length / 2) : 0, srcFps: info.fps }); } }, res);
  const blend = c0.ratio > 1 ? Math.floor(Math.min(0.33, c0.start, c0.length / 2) * 18) / 18 : 0;
  const draft1 = { ...draft0, start: c0.start, length: c0.length, blend };
  const e2 = await stage('estimate#2 (picked loop)', () => conv.estimate(file, dir, draft1), res);
  console.log('  estimate ->', JSON.stringify({ q: e2.q, bytes: e2.bytes, fits: e2.fits }));
  const vdir = path.join(dir, 'v1');
  const n0 = procs.length;
  const cv = await stage('save version (convert)', () => conv.convert(file, vdir, { ...draft1, onProgress: () => {} }), res);
  console.log('  convert ->', JSON.stringify({ q: cv.q, size: cv.size, length: cv.length, budget }));
  for (const p of procs.slice(n0)) console.log('    ff', p.t1 - p.t0, 'ms', p.args.includes('libx264') ? 'x264-preview' : /-q:v (\d+)/.exec(p.args)?.[0], / -t (\S+)/.exec(p.args)?.[1]);
  stopGpu();
  fs.writeFileSync(path.join(SCR, 'out', `baseline-${tag}.json`), JSON.stringify({ tag, info, budget, res, e1, e2, cv, loops: lr.candidates }, null, 1));
  process.exit(0);
})();
