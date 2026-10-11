// Estimate / Best-that-fits experiments: decode+filter once to raw frames, then cheap mjpeg trial encodes (parallel),
// a q-model, and exact full-length search. usage: node exp_estimate.js <clip> <tag>   (needs out/baseline-<tag>.json)
const path = require('path');
const fs = require('fs');
const os = require('os');
const { spawn } = require('child_process');
const SCR = path.resolve(__dirname, '..');
process.env.COOLERSCREEN_HOME = path.join(SCR, 'home');
const { REPO, stage, startGpu, stopGpu } = require('./mon');
const conv = require(path.join(REPO, 'src', 'convert'));
const settings = require(path.join(REPO, 'src', 'settings'));
const ff = conv.ffmpegPath;

function run(args, opts = {}) {
  return new Promise((resolve, reject) => {
    const p = spawn(ff, ['-hide_banner', '-loglevel', 'error', '-nostdin', ...args], { windowsHide: true });
    let err = '';
    p.stderr.on('data', d => (err += d));
    p.on('close', c => (c === 0 ? resolve() : reject(new Error(err.slice(-400)))));
  });
}
const W = 1080, H = 2400; // stored portrait frame
async function pool(items, n, fn) { const out = []; let i = 0; await Promise.all(Array.from({ length: n }, async () => { while (i < items.length) { const k = i++; out[k] = await fn(items[k]); } })); return out; }

(async () => {
  const file = path.resolve(process.argv[2]), tag = process.argv[3];
  const B = JSON.parse(fs.readFileSync(path.join(SCR, 'out', `baseline-${tag}.json`)));
  const s = settings.load();
  const mask = { mode: s.mask.mode, circle: s.mask.circle, band: s.mask.band };
  const c0 = B.loops[0];
  const blend = c0.ratio > 1 ? Math.floor(Math.min(0.33, c0.start, c0.length / 2) * 18) / 18 : 0;
  const draft = { start: c0.start, length: c0.length, quality: 'auto', frame: conv.FRAME_DEFAULT, mask, corners: { on: false, r: 80 }, detail: 0, blend, budgetBytes: B.budget, maxSeconds: 10 };
  const budget = B.budget;
  const dir = path.join(SCR, 'out', 'est-' + tag); fs.mkdirSync(dir, { recursive: true });
  const tmp = path.join(os.tmpdir(), 'csbench'); fs.mkdirSync(tmp, { recursive: true });
  startGpu(); await new Promise(r => setTimeout(r, 800));
  const res = [];
  const report = { tag, budget, draft };

  // 0. 1.2.0 estimate end to end (for reference) + one sample encode
  const e = await stage('1.2.0 estimate (auto, picked loop)', () => conv.estimate(file, dir, draft), res);
  report.est120 = { q: e.q, bytes: e.bytes };

  // the filter graph of a version, but writing raw frames instead of mjpeg
  const info = await conv.probe(file);
  const graphFor = (start, length) => {
    // rebuild via convert's own exported pieces: use deviceArgs-equivalent by encoding with -f rawvideo
    const o = { start, length, mask, frame: conv.FRAME_DEFAULT, fps: 18, srcFps: info.fps, isImage: false, cornerFile: null, detail: 0, blend: 0 };
    return o;
  };
  // We cannot import baseGraph (not exported), so reproduce the default-frame graph (no blend, no corners) inline.
  const px = conv.bandPx(mask.band);
  const g = `fps=18,scale=2400:1080:force_original_aspect_ratio=increase:flags=lanczos,crop=2400:1080,setsar=1,format=yuv420p,drawbox=x=0:y=0:w=${px}:h=1080:color=black:t=fill,transpose=1,format=yuvj420p`;
  const sLen = Math.min(2, draft.length), sStart = draft.start + Math.max(0, draft.length / 2 - sLen / 2);
  const rawS = path.join(tmp, `sample-${tag}.nut`);
  await stage('A: decode+filter 2 s sample -> raw (cpu)', () => run(['-ss', String(sStart), '-t', String(sLen), '-i', file, '-an', '-vf', g, '-c:v', 'rawvideo', '-y', rawS]), res);
  await stage('A2: same with NVDEC (hwaccel cuda, download)', () => run(['-hwaccel', 'cuda', '-ss', String(sStart), '-t', String(sLen), '-i', file, '-an', '-vf', g, '-c:v', 'rawvideo', '-y', rawS + '.nv.nut']), res);
  const enc = (src, q, out, threads) => run(['-color_range', 'pc', '-i', src, '-c:v', 'mjpeg', '-q:v', String(q), ...(q > 31 ? ['-qmax', '69', '-lmax', '30000'] : []), ...(threads ? ['-threads', String(threads)] : []), '-f', 'mp4', '-y', out]).then(() => fs.statSync(out).size);
  await stage('B: one mjpeg trial from raw (default threads)', () => enc(rawS, 20, path.join(tmp, `t-${tag}.mp4`)), res);
  await stage('B1: one mjpeg trial from raw (-threads 1)', () => enc(rawS, 20, path.join(tmp, `t-${tag}.mp4`), 1), res);
  // full sweep q=2..69 in parallel (8 at a time, 2 threads each)
  const qs = Array.from({ length: 68 }, (_, i) => i + 2);
  const sizes = await stage('C: sweep q 2..69 from raw, 8 parallel', () => pool(qs, 8, q => enc(rawS, q, path.join(tmp, `t-${tag}-${q}.mp4`), 2)), res);
  const est = q => (sizes[q - 2] * draft.length) / sLen;
  const exactSampleQ = qs.find(q => est(q) <= budget * 0.97) ?? null;
  report.sampleCurve = Object.fromEntries(qs.map(q => [q, Math.round(est(q))]));
  report.exactSampleQ = exactSampleQ;
  // model: two probes (q=6 and q=30), interpolate log(size) linearly in log(q); then one check encode
  const model = (qa, qb) => {
    const la = Math.log(est(qa)), lb = Math.log(est(qb));
    const k = (lb - la) / (Math.log(qb) - Math.log(qa));
    const target = Math.log(budget * 0.97);
    return Math.max(2, Math.min(69, Math.ceil(Math.exp(Math.log(qa) + (target - la) / k))));
  };
  report.model_6_30 = model(6, 30); report.model_4_16 = model(4, 16); report.model_10_40 = model(10, 40);
  // C2: parallel 4-ary search from raw: 3 rounds of 4 parallel trials
  await stage('C2: exact q on sample via 4-way parallel search (raw)', async () => {
    let lo = 2, hi = 69, rounds = 0;
    while (lo < hi) {
      rounds++;
      const pts = [...new Set([1, 2, 3, 4].map(i => lo + Math.floor(((hi - lo) * i) / 5)))].filter(q => q >= lo && q < hi);
      const r = await Promise.all(pts.map(q => enc(rawS, q, path.join(tmp, `p-${tag}-${q}.mp4`), 4).then(sz => [q, (sz * draft.length) / sLen])));
      for (const [q, b] of r) { if (b <= budget * 0.97) hi = Math.min(hi, q); else lo = Math.max(lo, q + 1); }
    }
    report.parallelRounds = rounds;
    report.parallelSearchQ = hi; return hi;
  }, res);

  // D: full-length exact: decode+filter whole selection -> raw, then exact search on real full size
  const rawF = path.join(tmp, `full-${tag}.nut`);
  await stage('D: decode+filter whole selection -> raw (cpu)', () => run(['-ss', String(draft.start), '-t', String(draft.length), '-i', file, '-an', '-vf', g, '-c:v', 'rawvideo', '-y', rawF]), res);
  report.rawFullMB = Math.round(fs.statSync(rawF).size / 1e6);
  await stage('D1: one full-length mjpeg from raw (default threads)', () => enc(rawF, 20, path.join(tmp, `f-${tag}.mp4`)), res);
  const fullAt = {};
  const near = [exactSampleQ - 2, exactSampleQ - 1, exactSampleQ, exactSampleQ + 1, exactSampleQ + 2].filter(q => q >= 2 && q <= 69);
  await stage(`D2: ${near.length} full-length encodes around sample q, parallel`, () => pool(near, near.length, q => enc(rawF, q, path.join(tmp, `f-${tag}-${q}.mp4`), 4).then(sz => (fullAt[q] = sz))), res);
  report.fullSizesNearSampleQ = fullAt;
  report.exactFullQ = Object.keys(fullAt).map(Number).sort((a, b) => a - b).find(q => fullAt[q] <= budget) ?? null;
  report.sampleErrPct = Object.fromEntries(Object.keys(fullAt).map(q => [q, +(100 * (est(+q) - fullAt[q]) / fullAt[q]).toFixed(1)]));
  report.baselineConvert = { q: B.cv.q, size: B.cv.size };
  report.res = res;
  stopGpu();
  fs.writeFileSync(path.join(SCR, 'out', `exp_estimate-${tag}.json`), JSON.stringify(report, null, 1));
  const { sampleCurve, res: _r, ...brief } = report; console.log(JSON.stringify(brief, null, 1));
  for (const f of fs.readdirSync(tmp)) if (f.includes(tag)) fs.rmSync(path.join(tmp, f), { force: true });
  process.exit(0);
})();
