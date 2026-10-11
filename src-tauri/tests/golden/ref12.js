// 1.2.0 reference for the media golden tests and the before/after benchmark.
// Runs the 1.2 Node media functions exactly as the app does when a new clip is opened and saved, timing each stage.
// usage: node ref12.js <clip> <outdir> [budgetBytes]     (needs ffmpeg-static: NODE_PATH=<repo>/node_modules)
// Writes <outdir>/ref12.json and <outdir>/v12-*/device.mp4 (1.2 converts for the golden comparisons).
const path = require('path');
const fs = require('fs');
const clip = path.resolve(process.argv[2]);
const out = path.resolve(process.argv[3]);
const budget = Number(process.argv[4]) || 8940000;
fs.mkdirSync(out, { recursive: true });
process.env.COOLERSCREEN_HOME = path.join(out, 'home12');
const REPO = path.resolve(__dirname, '..', '..', '..');
const conv = require(path.join(REPO, 'src', 'convert'));
const { findLoops } = require(path.join(REPO, 'src', 'loopsearch'));

const MASK = { mode: 'band', band: { edge: 'left', px: 92 }, circle: { cx: 45, cy: 540, r: 45, feather: 0 } };
const now = () => Number(process.hrtime.bigint() / 1000000n);
const autoBlend = (c, srcFps) => {
  if (!(c.ratio > 1.0)) return 0;
  const want = c.ratio <= 1.6 ? 0.33 : 0.5;
  const b = Math.min(want, c.start, c.length / 2);
  return Math.round(Math.floor(b * 18 + 1e-6) / 18 * 1000) / 1000;
};

(async () => {
  const R = { clip, budget, t: {} };
  const t = async (k, f) => { const s = now(); const r = await f(); R.t[k] = now() - s; return r; };
  const info = await conv.probe(clip);
  R.info = info;
  const dir = path.join(out, 'o12');
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  const dur = info.duration;
  // ---- open a clip: loop-length suggestion, loop search, cards, estimate of the picked loop (library.searchLoops)
  const t0 = now();
  const bps = await t('loopBps', () => conv.sampleBytesPerSecond(clip, dir, { start: 0, length: Math.min(2, dur), frame: null, mask: MASK, corners: { on: false, r: 0 }, budgetBytes: budget }, 25, 0));
  const maxLen = Math.max(3, Math.min(Math.max(1, Math.floor((budget * 0.94 / bps) * 2) / 2), dur - 0.2, 30));
  R.maxLen = maxLen;
  const L = await t('findLoops', () => findLoops(clip, dir, { duration: dur, fps: info.fps, minLen: 3, maxLen }));
  R.loops = L;
  await t('cards', async () => {
    for (let i = 0; i < L.candidates.length; i++) {
      const c = L.candidates[i];
      c.blend = autoBlend(c, info.fps);
      await conv.renderLoopPreview(clip, path.join(dir, `loop-${i}.mp4`), { start: c.start, length: c.length, blend: c.blend, srcFps: info.fps });
    }
  });
  const c0 = L.candidates[0];
  const sel = { start: c0.start, length: c0.length, quality: 'auto', frame: conv.FRAME_DEFAULT, mask: MASK, corners: { on: false, r: 0 }, detail: 0, blend: c0.blend, budgetBytes: budget, maxSeconds: 10 };
  R.sel = sel;
  const e1 = await t('estimate', () => conv.estimate(clip, dir, sel));
  R.t.openReady = now() - t0;
  R.estimate = e1;
  // ---- a tuning change: corners on + reframed -> new estimate
  const sel2 = { ...sel, corners: { on: true, r: 80 }, frame: { zoom: 1.2, x: 0.3, y: 0.6 } };
  R.sel2 = sel2;
  await conv.cornerMaskFile(sel2.corners, sel2.mask); // mask PNG cached for both implementations
  R.estimate2 = await t('estimateAfterChange', () => conv.estimate(clip, dir, sel2));
  // ---- save as version (1.2 always repeats the whole search)
  R.convert = await t('save', () => conv.convert(clip, path.join(out, 'v12-a'), sel));
  R.convert2 = await t('save2', () => conv.convert(clip, path.join(out, 'v12-b'), sel2));
  fs.writeFileSync(path.join(out, 'ref12.json'), JSON.stringify(R, null, 1));
  console.log(JSON.stringify({ t: R.t, q: [R.estimate.q, R.convert.q, R.estimate2.q, R.convert2.q], size: [R.convert.size, R.convert2.size], top: L.candidates.map(c => [c.startFrame, c.frames, c.ratio]) }));
})().catch(e => { console.error(e); process.exit(1); });
