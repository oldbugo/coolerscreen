// Loop card previews (5 x 320x144 H.264) and the leader re-check: sequential (1.2.0) vs parallel vs low-res graph vs NVENC.
// usage: node exp_loopcards.js <clip> <tag>
const path = require('path');
const fs = require('fs');
const { spawn } = require('child_process');
const SCR = path.resolve(__dirname, '..');
process.env.COOLERSCREEN_HOME = path.join(SCR, 'home');
const { REPO, stage, startGpu, stopGpu } = require('./mon');
const conv = require(path.join(REPO, 'src', 'convert'));
const ff = conv.ffmpegPath;
const run = args => new Promise((res, rej) => { const p = spawn(ff, ['-hide_banner', '-loglevel', 'error', '-nostdin', ...args], { windowsHide: true }); let e = ''; p.stderr.on('data', d => (e += d)); p.on('close', c => (c ? rej(new Error(e)) : res())); });

(async () => {
  const file = path.resolve(process.argv[2]), tag = process.argv[3];
  const B = JSON.parse(fs.readFileSync(path.join(SCR, 'out', `baseline-${tag}.json`)));
  const fps = B.info.fps;
  const dir = path.join(SCR, 'out', 'cards-' + tag); fs.mkdirSync(dir, { recursive: true });
  startGpu(); await new Promise(r => setTimeout(r, 800));
  const res = [];
  const C = B.loops;
  const lowres = (c, i, enc) => {
    const blend = 0; // blend handled the same way as 1.2.0 in a real port; here just the trim
    return ['-ss', String(c.start), '-t', String(c.length), '-i', file, '-an', '-vf', `fps=18,scale=320:144:force_original_aspect_ratio=increase:flags=bilinear,crop=320:144,format=yuv420p`,
      ...(enc === 'nvenc' ? ['-c:v', 'h264_nvenc', '-preset', 'p1', '-cq', '28'] : ['-c:v', 'libx264', '-preset', 'veryfast', '-crf', '26', '-threads', '2']), '-movflags', '+faststart', '-y', path.join(dir, `l${i}.mp4`)];
  };
  await stage('1.2.0: 5 cards sequential (full-res graph)', async () => { for (let i = 0; i < C.length; i++) await conv.renderLoopPreview(file, path.join(dir, `a${i}.mp4`), { start: C[i].start, length: C[i].length, blend: 0, srcFps: fps }); }, res);
  await stage('5 cards parallel (full-res graph)', () => Promise.all(C.map((c, i) => conv.renderLoopPreview(file, path.join(dir, `b${i}.mp4`), { start: c.start, length: c.length, blend: 0, srcFps: fps }))), res);
  await stage('5 cards parallel, direct 320x144 bilinear, x264', () => Promise.all(C.map((c, i) => run(lowres(c, i)))), res);
  await stage('5 cards parallel, direct 320x144, NVDEC+NVENC', () => Promise.all(C.map((c, i) => run(['-hwaccel', 'cuda', ...lowres(c, i, 'nvenc')]))), res);
  // leader re-check: 16 x 3-frame seeks at 192x108 (1.2.0) vs parallel
  const seeks = C.flatMap(c => [c.startFrame, c.endFrame + 1]).concat([1, 2, 3, 4, 5, 6]).slice(0, 16);
  await stage('1.2.0: 16 leader seeks sequential', async () => { for (const f of seeks) await conv.grayFrames(file, Math.max(0, f - 1) / fps + 0.001, null, 3, 192, 108); }, res);
  await stage('16 leader seeks, 8 parallel', async () => { let k = 0; await Promise.all(Array.from({ length: 8 }, async () => { while (k < seeks.length) { const f = seeks[k++]; await conv.grayFrames(file, Math.max(0, f - 1) / fps + 0.001, null, 3, 192, 108); } })); }, res);
  stopGpu();
  fs.writeFileSync(path.join(SCR, 'out', `exp_loopcards-${tag}.json`), JSON.stringify(res, null, 1));
  process.exit(0);
})();
