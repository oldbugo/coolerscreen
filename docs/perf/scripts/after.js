// Prototype of the proposed 2.0 pipeline (still FFmpeg as a sidecar), timed per user action.
//  - loops: one decode pass (NVDEC for HEVC / >2.5K sources) giving 64x36 + 192x108 grey, JS compute, no extra seeks
//  - estimate: decode+filter the selection once to raw frames (swscale threads), exact full-length sizes from raw:
//    round 1 q=4 & q=16 in parallel -> log-log model -> round 2 q-1..q+1 in parallel -> exact best q (+ keep the winner file)
//  - cards: 5 low-res previews in parallel
//  - save: winner file already encoded; only the H.264 preview + thumb from the raw frames
// usage: node after.js <clip> <tag>
const path = require('path');
const fs = require('fs');
const os = require('os');
const { spawn } = require('child_process');
const SCR = path.resolve(__dirname, '..');
process.env.COOLERSCREEN_HOME = path.join(SCR, 'home');
const { REPO, stage, startGpu, stopGpu } = require('./mon');
const conv = require(path.join(REPO, 'src', 'convert'));
const settings = require(path.join(REPO, 'src', 'settings'));
const { findLoops } = require(path.join(SCR, 'proto', 'loopsearch2'));
const ff = conv.ffmpegPath;
const run = args => new Promise((res, rej) => { const p = spawn(ff, ['-hide_banner', '-loglevel', 'error', '-nostdin', ...args], { windowsHide: true }); let e = ''; p.stderr.on('data', d => (e += d)); p.on('close', c => (c ? rej(new Error(e)) : res())); });

(async () => {
  const file = path.resolve(process.argv[2]), tag = process.argv[3];
  const B = JSON.parse(fs.readFileSync(path.join(SCR, 'out', `baseline-${tag}.json`)));
  const info = B.info, budget = B.budget;
  const gpu = info.codec === 'hevc' || info.width > 2560;
  const s = settings.load();
  const px = conv.bandPx(s.mask.band);
  const tmp = path.join(os.tmpdir(), 'csafter'); fs.mkdirSync(tmp, { recursive: true });
  startGpu(); await new Promise(r => setTimeout(r, 800));
  const res = [];
  const R = { tag, gpu };

  // ---- loops
  const L = await stage(`loops: features 1 pass (${gpu ? 'NVDEC' : 'CPU'}) + JS compute, no seeks`, () => findLoops(file, tmp, { duration: info.duration, fps: info.fps, minLen: 3, maxLen: Math.max(3, Math.min(B.loops.reduce((m, c) => Math.max(m, c.length), 3) + 2, info.duration - 0.2, 30)), gpu }), res);
  R.loopsSameTop = L.candidates.slice(0, 3).map(c => [c.start, c.length]);
  R.loopsBaselineTop = B.loops.slice(0, 3).map(c => [c.start, c.length]);
  const C = B.loops; // cards for the same candidates as the baseline
  await stage('cards: 5 x 320x144 in parallel', () => Promise.all(C.map((c, i) => run([...(gpu ? ['-hwaccel', 'cuda', '-hwaccel_output_format', 'cuda'] : []), '-ss', String(c.start), '-t', String(c.length), '-i', file, '-an', '-vf',
    (gpu ? 'scale_cuda=640:360,hwdownload,format=nv12,' : '') + 'fps=18,scale=320:144:force_original_aspect_ratio=increase:flags=bilinear,crop=320:144,format=yuv420p',
    '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '26', '-threads', '2', '-movflags', '+faststart', '-y', path.join(tmp, `card${i}.mp4`)]))), res);

  // ---- estimate (picked loop)
  const c0 = B.loops[0];
  const start = c0.start, length = c0.length;
  const raw = path.join(tmp, `sel-${tag}.nut`);
  const g = `fps=18,scale=2400:1080:force_original_aspect_ratio=increase:flags=lanczos:threads=0,crop=2400:1080,setsar=1,format=yuv420p,drawbox=x=0:y=0:w=${px}:h=1080:color=black:t=fill,transpose=1,format=yuvj420p`;
  const enc = (q, out) => run(['-color_range', 'pc', '-i', raw, '-c:v', 'mjpeg', '-q:v', String(q), ...(q > 31 ? ['-qmax', '69', '-lmax', '30000'] : []), '-threads', '4', '-r', '18', '-movflags', '+faststart', '-y', out]).then(() => fs.statSync(out).size);
  const sizes = {};
  const est = await stage('estimate: prep raw + 2 rounds of parallel exact encodes', async () => {
    await run(['-ss', String(start), '-t', String(length), '-i', file, '-an', '-vf', g, '-c:v', 'rawvideo', '-y', raw]);
    const r1 = await Promise.all([4, 16].map(q => enc(q, path.join(tmp, `q${q}.mp4`)).then(b => (sizes[q] = b))));
    const k = (Math.log(r1[1]) - Math.log(r1[0])) / (Math.log(16) - Math.log(4));
    const qm = Math.max(2, Math.min(69, Math.round(Math.exp(Math.log(4) + (Math.log(budget) - Math.log(r1[0])) / k))));
    let lo = Math.max(2, qm - 1), hi = Math.min(69, qm + 1), rounds = 2;
    for (;;) {
      const qs = []; for (let q = lo; q <= hi; q++) if (!(q in sizes)) qs.push(q);
      await Promise.all(qs.map(q => enc(q, path.join(tmp, `q${q}.mp4`)).then(b => (sizes[q] = b))));
      const fit = Object.keys(sizes).map(Number).filter(q => sizes[q] <= budget).sort((a, b) => a - b)[0];
      if (fit != null && (fit === 2 || (fit - 1) in sizes)) return { q: fit, bytes: sizes[fit], model: qm, rounds };
      rounds++; lo = Math.max(2, (fit ?? hi + 1) - 2); hi = Math.min(69, fit != null ? fit - 1 : hi + 3);
    }
  }, res);
  R.estimate = est; R.baselineFinal = { q: B.cv.q, size: B.cv.size };
  // ---- tuning change with the same selection (e.g. corners / frame): prep again, i.e. same cost; a change of q only: ~0
  // ---- save as version
  await stage('save: winner already encoded -> preview x264 + thumb from raw', async () => {
    fs.copyFileSync(path.join(tmp, `q${est.q}.mp4`), path.join(tmp, `device-${tag}.mp4`));
    await run(['-color_range', 'pc', '-i', raw, '-an', '-vf', 'transpose=2,scale=1200:540:flags=bicubic,format=yuv420p', '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '22', '-movflags', '+faststart', '-y', path.join(tmp, `preview-${tag}.mp4`)]);
    await run(['-ss', '0.5', '-i', path.join(tmp, `preview-${tag}.mp4`), '-frames:v', '1', '-vf', 'scale=400:-2', '-q:v', '4', '-y', path.join(tmp, `thumb-${tag}.jpg`)]);
  }, res);
  await stage('save (cold: nothing cached) = estimate + save', async () => {
    fs.rmSync(raw); for (const q of Object.keys(sizes)) delete sizes[q];
    await run(['-ss', String(start), '-t', String(length), '-i', file, '-an', '-vf', g, '-c:v', 'rawvideo', '-y', raw]);
    await Promise.all([est.q - 1, est.q].filter(q => q >= 2).map(q => enc(q, path.join(tmp, `c${q}.mp4`))));
    await run(['-color_range', 'pc', '-i', raw, '-an', '-vf', 'transpose=2,scale=1200:540:flags=bicubic,format=yuv420p', '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '22', '-movflags', '+faststart', '-y', path.join(tmp, `preview-${tag}.mp4`)]);
  }, res);
  R.res = res;
  stopGpu();
  fs.writeFileSync(path.join(SCR, 'out', `after-${tag}.json`), JSON.stringify(R, null, 1));
  const { res: _, ...brief } = R; console.log(JSON.stringify(brief));
  for (const f of fs.readdirSync(tmp)) fs.rmSync(path.join(tmp, f), { force: true });
  process.exit(0);
})();
