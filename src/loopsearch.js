// "Find loop points": searches the whole source, frame-accurate at the source frame rate, for the start/end pairs whose
// join looks most like ordinary motion.
//
// A loop plays source frames a .. b and then jumps back to a. On the screen the length is a whole number n of output
// frames (18 per second), so the frame that would naturally follow the end is j = round(a + n * srcFps / 18).
// The jump is invisible when the frames around a look like the frames around j: the seam cost is
//   (D(j-1, a-1) + 2 D(j, a) + D(j+1, a+1)) / 4,  D = mean absolute difference of small grey frames,
// which also checks motion continuity (the frames before and after the join), and it is reported relative to the usual
// difference between neighbouring frames at both points (1.0x = the jump looks like a normal frame step).
//
// Steps: (1) every frame of the source decoded once to 64x36 grey and cached in the clip's library folder;
// (2) all pairs within the length window scored on 32x18 versions (typed arrays; strided for very long sources);
// (3) the best few hundred re-scored at 64x36; (4) near-duplicates (start and end both within 0.5 s) suppressed;
// (5) the leaders re-scored at 192x108 from freshly decoded frames; the top 5 returned.
const fs = require('fs');
const path = require('path');
const { grayFrames } = require('./convert');

const FW = 64, FH = 36, CW = 32, CH = 18, RW = 192, RH = 108;
const MAX_FRAMES = 9000; // ~6 min at 25 fps; longer sources are searched in their first MAX_FRAMES frames
const OUT_FPS = 18;

function madArr(a, ao, b, bo, len) {
  let s = 0;
  for (let i = 0; i < len; i++) { const d = a[ao + i] - b[bo + i]; s += d < 0 ? -d : d; }
  return s / len;
}

/** All frames as one Uint8Array (n * 64 * 36), cached per source file (size + mtime). */
async function features(dir, file, onProgress, expectedFrames) {
  const st = fs.statSync(file);
  const key = `${st.size}-${Math.round(st.mtimeMs)}`;
  const bin = path.join(dir, `loopframes-${FW}x${FH}.bin`), meta = path.join(dir, `loopframes-${FW}x${FH}.json`);
  try {
    const m = JSON.parse(fs.readFileSync(meta, 'utf8'));
    if (m.key === key && fs.existsSync(bin)) { onProgress(1); return { data: new Uint8Array(fs.readFileSync(bin)), n: m.n }; }
  } catch {}
  const frames = await grayFrames(file, 0, null, MAX_FRAMES, FW, FH, k => onProgress(Math.min(1, k / (expectedFrames || 600))));
  const data = new Uint8Array(frames.length * FW * FH);
  frames.forEach((f, i) => data.set(f, i * FW * FH));
  fs.writeFileSync(bin, data);
  fs.writeFileSync(meta, JSON.stringify({ key, n: frames.length, w: FW, h: FH }));
  return { data, n: frames.length };
}

function downscale(data, n) {
  const out = new Uint8Array(n * CW * CH);
  for (let f = 0; f < n; f++) {
    const s = f * FW * FH, o = f * CW * CH;
    for (let y = 0; y < CH; y++) for (let x = 0; x < CW; x++) {
      const i = s + y * 2 * FW + x * 2;
      out[o + y * CW + x] = (data[i] + data[i + 1] + data[i + FW] + data[i + FW + 1] + 2) >> 2;
    }
  }
  return out;
}

/**
 * opts: { duration, fps (source), minLen, maxLen (seconds), onProgress(frac, msg) }
 * Returns { candidates: [{ start, length, end, cost, ratio, similarity }], frames, searched }.
 */
async function findLoops(file, dir, opts) {
  const report = opts.onProgress || (() => {});
  const sf = opts.fps || 25;
  report(0.01, 'Reading the frames');
  const { data, n } = await features(dir, file, f => report(0.01 + 0.45 * f, 'Reading the frames'), Math.round((opts.duration || 0) * sf));
  if (n < 4) throw new Error('The clip is too short to search for loop points.');
  const small = downscale(data, n);
  const P = CW * CH, Q = FW * FH;
  const Dc = (x, y) => madArr(small, x * P, small, y * P, P);
  const Df = (x, y) => madArr(data, x * Q, data, y * Q, Q);
  const step = new Float32Array(n); // usual difference between neighbouring frames
  for (let i = 1; i < n; i++) step[i] = Dc(i - 1, i);
  step[0] = step[1] || 0;

  const minN = Math.max(2, Math.round(opts.minLen * OUT_FPS)), maxN = Math.max(minN, Math.round(opts.maxLen * OUT_FPS));
  const ratioF = sf / OUT_FPS;
  // seam cost for start frame a and n output frames, with a frame-difference function D
  const cost = (D, a, nn) => {
    const j = Math.round(a + nn * ratioF);
    if (j >= n) return null;
    let s = 2 * D(j, a), w = 2;
    if (a >= 1) { s += D(j - 1, a - 1); w++; }
    if (j + 1 < n) { s += D(j + 1, a + 1); w++; }
    return { j, c: s / w };
  };
  const pairs = n * (maxN - minN + 1);
  const stride = Math.max(1, Math.ceil(pairs / 250000)); // keeps very long sources to about a second of work
  report(0.5, 'Comparing every start with every end');
  const cand = [];
  let done = 0;
  for (let a = 0; a < n - 2; a += stride) {
    for (let nn = minN; nn <= maxN; nn += stride) {
      const r = cost(Dc, a, nn);
      if (!r) break;
      cand.push([r.c, a, nn]);
    }
    if (++done % 50 === 0) { report(0.5 + 0.25 * (a / n), 'Comparing every start with every end'); await new Promise(setImmediate); }
  }
  if (!cand.length) throw new Error('No loop of that length fits in the clip. Lower the minimum length.');
  cand.sort((x, y) => x[0] - y[0]);
  // refine at 64x36 (and around strided positions)
  report(0.78, 'Refining the best matches');
  const refined = [];
  const seen = new Set();
  for (const [, a0, n0] of cand.slice(0, 300)) {
    for (let da = -(stride - 1); da <= stride - 1; da++) for (let dn = -(stride - 1); dn <= stride - 1; dn++) {
      const a = a0 + da, nn = n0 + dn;
      if (a < 0 || nn < minN || nn > maxN || seen.has(a * 100000 + nn)) continue;
      seen.add(a * 100000 + nn);
      const r = cost(Df, a, nn);
      if (r) refined.push({ a, nn, j: r.j, c: r.c });
    }
  }
  refined.sort((x, y) => x.c - y.c);
  // suppress near-duplicates: start and end both within 0.5 s of a better candidate
  const tol = 0.5 * sf, picked = [];
  for (const r of refined) {
    if (picked.some(p => Math.abs(p.a - r.a) <= tol && Math.abs(p.j - r.j) <= tol)) continue;
    picked.push(r);
    if (picked.length >= 8) break;
  }
  // final check at 192x108 from the source itself
  report(0.85, 'Checking the leaders at higher resolution');
  for (let i = 0; i < picked.length; i++) {
    const r = picked[i];
    // three frames around each point (f-1, f, f+1; from f when f is the first frame)
    const win = async f => { const s0 = Math.max(0, f - 1); return { s0, fr: await grayFrames(file, s0 / sf + 0.001, null, 3, RW, RH) }; };
    const A = await win(r.a), J = await win(r.j);
    const get = (w, f) => w.fr[f - w.s0];
    let s = 0, w = 0;
    for (const [wt, k] of [[2, 0], [1, -1], [1, 1]]) {
      const x = get(J, r.j + k), y = get(A, r.a + k);
      if (x && y) { s += wt * madArr(x, 0, y, 0, x.length); w += wt; }
    }
    r.hc = w ? s / w : r.c;
    // usual neighbouring-frame difference around both points, at the same resolution
    const nb = [];
    for (const [W2, f] of [[A, r.a], [J, r.j]]) for (const k of [0, 1]) { const x = get(W2, f - 1 + k), y = get(W2, f + k); if (x && y) nb.push(madArr(x, 0, y, 0, x.length)); }
    const loc = nb.length ? nb.reduce((p, q) => p + q, 0) / nb.length : ((step[r.a] || 0) + (step[r.j] || 0)) / 2;
    r.ratio = r.hc / Math.max(loc, 0.5); // 1.0 = the jump looks like an ordinary frame step (floor keeps still scenes sane)
    report(0.85 + 0.14 * ((i + 1) / picked.length), 'Checking the leaders at higher resolution');
  }
  picked.sort((x, y) => x.ratio - y.ratio || x.hc - y.hc);
  const r3 = v => Math.round(v * 1000) / 1000;
  return {
    frames: n,
    searched: { minLen: r3(minN / OUT_FPS), maxLen: r3(maxN / OUT_FPS), truncated: n >= MAX_FRAMES },
    candidates: picked.slice(0, 5).map(r => ({
      start: r3(r.a / sf), startFrame: r.a, length: r3(r.nn / OUT_FPS), frames: r.nn, end: r3(r.j / sf), endFrame: r.j - 1,
      cost: Math.round(r.hc * 100) / 100, ratio: Math.round(r.ratio * 100) / 100,
      similarity: Math.round((100 - (r.hc / 255) * 100) * 10) / 10,
    })),
  };
}

module.exports = { findLoops };
