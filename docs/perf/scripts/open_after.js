// "Open a new clip" with the proposed pipeline, work running concurrently (estimate on CPU, loop features on NVDEC/CPU),
// as a 2.0 scheduler would. Prints when the meter, the loop cards and the meter for the picked loop are ready.
// usage: node open_after.js <clip> <tag>
const path = require('path');
const fs = require('fs');
const os = require('os');
const { spawn } = require('child_process');
const SCR = path.resolve(__dirname, '..');
process.env.COOLERSCREEN_HOME = path.join(SCR, 'home');
const { REPO, startGpu, stopGpu } = require('./mon');
const conv = require(path.join(REPO, 'src', 'convert'));
const settings = require(path.join(REPO, 'src', 'settings'));
const { findLoops } = require(path.join(SCR, 'proto', 'loopsearch2'));
const ff = conv.ffmpegPath;
const run = args => new Promise((res, rej) => { const p = spawn(ff, ['-hide_banner', '-loglevel', 'error', '-nostdin', ...args], { windowsHide: true }); let e = ''; p.stderr.on('data', d => (e += d)); p.on('close', c => (c ? rej(new Error(e)) : res())); });

(async () => {
  const file = path.resolve(process.argv[2]), tag = process.argv[3];
  const info = await conv.probe(file);
  const budget = 8940000, px = conv.bandPx(settings.load().mask.band);
  const gpu = info.codec === 'hevc' || info.width > 2560;
  const tmp = path.join(os.tmpdir(), 'csopen-' + tag); fs.mkdirSync(tmp, { recursive: true });
  const g = `fps=18,scale=2400:1080:force_original_aspect_ratio=increase:flags=lanczos:threads=0,crop=2400:1080,setsar=1,format=yuv420p,drawbox=x=0:y=0:w=${px}:h=1080:color=black:t=fill,transpose=1,format=yuvj420p`;
  async function estimate(start, length, id) {
    const raw = path.join(tmp, `sel${id}.nut`), sizes = {};
    await run(['-ss', String(start), '-t', String(length), '-i', file, '-an', '-vf', g, '-c:v', 'rawvideo', '-y', raw]);
    const enc = q => run(['-color_range', 'pc', '-i', raw, '-c:v', 'mjpeg', '-q:v', String(q), ...(q > 31 ? ['-qmax', '69', '-lmax', '30000'] : []), '-threads', '4', '-y', path.join(tmp, `e${id}-${q}.mp4`)]).then(() => (sizes[q] = fs.statSync(path.join(tmp, `e${id}-${q}.mp4`)).size));
    await Promise.all([4, 16].map(enc));
    const k = (Math.log(sizes[16]) - Math.log(sizes[4])) / Math.log(4);
    const qm = Math.max(2, Math.min(69, Math.round(Math.exp(Math.log(4) + (Math.log(budget) - Math.log(sizes[4])) / k))));
    await Promise.all([qm - 1, qm, qm + 1].filter(q => q >= 2 && q <= 69 && !(q in sizes)).map(enc));
    return Object.keys(sizes).map(Number).filter(q => sizes[q] <= budget).sort((a, b) => a - b)[0];
  }
  startGpu(); await new Promise(r => setTimeout(r, 800));
  const t0 = Date.now(), t = () => ((Date.now() - t0) / 1000).toFixed(2) + ' s';
  const out = {};
  await Promise.all([
    estimate(0, Math.floor(Math.min(10, info.duration) * 18) / 18, 'a').then(q => { out.meter1 = t(); console.log('meter (initial selection) ready', out.meter1, 'q', q); }),
    (async () => {
      const L = await findLoops(file, tmp, { duration: info.duration, fps: info.fps, minLen: 3, maxLen: Math.min(10, info.duration - 0.2), gpu });
      out.loopsList = t(); console.log('loop list ready', out.loopsList);
      const c0 = L.candidates[0];
      await Promise.all([
        Promise.all(L.candidates.map((c, i) => run([...(gpu ? ['-hwaccel', 'cuda', '-hwaccel_output_format', 'cuda'] : []), '-ss', String(c.start), '-t', String(c.length), '-i', file, '-an', '-vf',
          (gpu ? 'scale_cuda=640:360,hwdownload,format=nv12,' : '') + 'fps=18,scale=320:144:force_original_aspect_ratio=increase:flags=bilinear,crop=320:144,format=yuv420p',
          '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '26', '-threads', '2', '-y', path.join(tmp, `card${i}.mp4`)]))).then(() => { out.cards = t(); console.log('loop cards ready', out.cards); }),
        estimate(c0.start, c0.length, 'b').then(q => { out.meter2 = t(); console.log('meter (picked loop) ready', out.meter2, 'q', q); }),
      ]);
    })(),
  ]);
  stopGpu();
  fs.writeFileSync(path.join(SCR, 'out', `open_after-${tag}.json`), JSON.stringify(out));
  fs.rmSync(tmp, { recursive: true, force: true });
  process.exit(0);
})();
