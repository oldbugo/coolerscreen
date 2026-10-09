// Media -> device format. Landscape 2400x1080 (cover crop) -> mask -> rotate 90 deg CW -> 1080x2400 MJPEG mp4.
const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');

let ffmpeg = require('ffmpeg-static');
if (ffmpeg && ffmpeg.includes('app.asar')) ffmpeg = ffmpeg.replace('app.asar', 'app.asar.unpacked');

const W = 2400, H = 1080;
const IMAGE_EXT = new Set(['.png', '.jpg', '.jpeg', '.bmp', '.webp', '.tif', '.tiff']);
const FPS_LADDER = [30, 25, 20, 15, 12, 10];

function runFfmpeg(args, { onTime } = {}) {
  return new Promise((resolve, reject) => {
    const p = spawn(ffmpeg, ['-hide_banner', '-loglevel', 'error', '-nostdin', '-progress', 'pipe:1', ...args],
      { windowsHide: true });
    let err = '';
    p.stderr.on('data', d => { err += d; if (err.length > 20000) err = err.slice(-20000); });
    let buf = '';
    p.stdout.on('data', d => {
      buf += d;
      let i;
      while ((i = buf.indexOf('\n')) >= 0) {
        const line = buf.slice(0, i).trim();
        buf = buf.slice(i + 1);
        const m = /^out_time=(\d+):(\d+):(\d+(?:\.\d+)?)/.exec(line);
        if (m && onTime) onTime(+m[1] * 3600 + +m[2] * 60 + +m[3]);
      }
    });
    p.on('error', reject);
    p.on('close', code => {
      if (code === 0) resolve();
      else reject(new Error('ffmpeg failed: ' + (err.trim().split('\n').slice(-4).join(' | ') || 'exit ' + code)));
    });
  });
}

// ffprobe is not bundled; parse `ffmpeg -i`.
function probe(file) {
  return new Promise(resolve => {
    const p = spawn(ffmpeg, ['-hide_banner', '-nostdin', '-i', file], { windowsHide: true });
    let err = '';
    p.stderr.on('data', d => (err += d));
    p.on('error', () => resolve({}));
    p.on('close', () => {
      const d = /Duration:\s*(\d+):(\d+):(\d+(?:\.\d+)?)/.exec(err);
      const v = /Video:.*?(\d{2,5})x(\d{2,5})/.exec(err);
      resolve({
        duration: d ? +d[1] * 3600 + +d[2] * 60 + +d[3] : null,
        width: v ? +v[1] : null,
        height: v ? +v[2] : null,
        hasVideo: /Video:/.test(err),
        isImage: IMAGE_EXT.has(path.extname(file).toLowerCase()),
      });
    });
  });
}

const even = n => n - (n % 2);

// Returns filtergraph pieces that take [base] and produce [m] (masked landscape).
function maskGraph(mask) {
  if (!mask || mask.mode === 'off') return '[base]null[m]';
  if (mask.mode === 'band') {
    const px = Math.round(mask.band.px);
    if (px <= 0) return '[base]null[m]';
    const box = {
      left: [0, 0, px, H], right: [W - px, 0, px, H], top: [0, 0, W, px], bottom: [0, H - px, W, px],
    }[mask.band.edge];
    return `[base]drawbox=x=${box[0]}:y=${box[1]}:w=${box[2]}:h=${box[3]}:color=black:t=fill[m]`;
  }
  const { cx, cy, r, feather } = mask.circle;
  if (r <= 0) return '[base]null[m]';
  const reach = r + feather + 2;
  const x0 = even(Math.max(0, Math.floor(cx - reach)));
  const y0 = even(Math.max(0, Math.floor(cy - reach)));
  const x1 = Math.min(W, Math.ceil(cx + reach));
  const y1 = Math.min(H, Math.ceil(cy + reach));
  const w = even(x1 - x0) || 2, h = even(y1 - y0) || 2;
  const dist = `hypot(X+${x0}-${cx},Y+${y0}-${cy})`;
  const k = feather > 0 ? `clip((${dist}-${r})/${feather},0,1)` : `gte(${dist},${r})`;
  const ge = c => `${c}='${c}(X,Y)*${k}'`;
  return `[base]split[a][b];[b]crop=${w}:${h}:${x0}:${y0},format=gbrp,` +
    `geq=${ge('r')}:${ge('g')}:${ge('b')},format=yuv420p[c];[a][c]overlay=${x0}:${y0},format=yuv420p[m]`;
}

function inputArgs(file, { isImage, fps, start, length }) {
  if (isImage) return ['-loop', '1', '-framerate', String(fps), '-t', String(length), '-i', file];
  return ['-ss', String(start), '-t', String(length), '-i', file];
}

function baseGraph(fps) {
  return `[0:v]fps=${fps},scale=${W}:${H}:force_original_aspect_ratio=increase:flags=lanczos,` +
    `crop=${W}:${H},setsar=1,format=yuv420p[base]`;
}

function deviceArgs(file, src, o, q, out) {
  const g = `${baseGraph(o.fps)};${maskGraph(o.mask)};[m]transpose=1,format=yuvj420p[out]`;
  return [...inputArgs(file, { ...src, ...o }), '-filter_complex', g, '-map', '[out]', '-an',
    '-c:v', 'mjpeg', '-q:v', String(q), '-movflags', '+faststart', '-y', out];
}

async function encodeDevice(file, src, o, q, out, onTime) {
  await runFfmpeg(deviceArgs(file, src, o, q, out), { onTime });
  return fs.statSync(out).size;
}

// Landscape H.264 preview (mask applied), plus thumbnail and an unmasked frame for the settings canvas.
async function makePreview(file, src, o, dir, onTime) {
  const g = `${baseGraph(o.fps)};${maskGraph(o.mask)};[m]scale=1200:540:flags=bicubic,format=yuv420p[out]`;
  await runFfmpeg([...inputArgs(file, { ...src, ...o }), '-filter_complex', g, '-map', '[out]', '-an',
    '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '22', '-movflags', '+faststart', '-y',
    path.join(dir, 'preview.mp4')], { onTime });
  const seek = String(Math.min(0.5, Math.max(0, o.length / 2)));
  await runFfmpeg(['-ss', seek, '-i', path.join(dir, 'preview.mp4'), '-frames:v', '1',
    '-vf', 'scale=400:-2', '-q:v', '4', '-y', path.join(dir, 'thumb.jpg')]);
  const g2 = `${baseGraph(o.fps)};[base]scale=1200:540,format=yuv420p[out]`;
  await runFfmpeg([...inputArgs(file, { ...src, ...o, length: Math.min(o.length, 1) }), '-filter_complex', g2,
    '-map', '[out]', '-frames:v', '1', '-q:v', '4', '-y', path.join(dir, 'frame.jpg')]);
}

// Friendly quality 1-100 (higher = better) <-> ffmpeg mjpeg -q:v 31..3
const qFromQuality = Q => Math.round(31 - ((Math.min(100, Math.max(1, Number(Q))) - 1) * 28) / 99);
const qualityFromQ = q => Math.round(1 + ((31 - q) * 99) / 28);

const rnd = () => Math.random().toString(36).slice(2, 8);

async function prepare(file, dir, opts) {
  const info = await probe(file);
  if (!info.hasVideo) throw new Error('No video/image stream found in this file.');
  const isImage = info.isImage;
  const maxFps = Math.min(30, Math.max(10, Math.round(opts.fps || 20)));
  const startFps = isImage ? Math.min(maxFps, 10) : maxFps;
  const start = isImage ? 0 : Math.max(0, Number(opts.start) || 0);
  const avail = info.duration != null ? Math.max(0.1, info.duration - start) : Infinity;
  let length = Number(opts.length);
  if (!(length > 0)) length = isImage ? 3 : Math.min(opts.maxSeconds || 10, avail);
  length = Math.max(0.1, Math.min(length, avail));
  length = Math.round(length * 100) / 100;
  if (!Number.isFinite(length)) length = 10;
  const manual = opts.quality != null && opts.quality !== 'auto';
  fs.mkdirSync(dir, { recursive: true });
  return {
    file, dir, info, isImage, src: { isImage }, startFps, start, length, manual,
    q: manual ? qFromQuality(opts.quality) : null,
    budget: opts.budgetBytes, mask: opts.mask,
    base: { start, length, mask: opts.mask },
  };
}

// Estimated full-length size from a short sample encode.
async function sampleEstimate(c, fps, q) {
  const sLen = Math.min(2, c.length);
  const sStart = c.start + Math.max(0, c.length / 2 - sLen / 2);
  const out = path.join(c.dir, `sample.${rnd()}.tmp.mp4`);
  try {
    const bytes = await encodeDevice(c.file, c.src, { ...c.base, fps, start: c.isImage ? 0 : sStart, length: sLen }, q, out);
    return (bytes * c.length) / sLen;
  } finally { try { fs.unlinkSync(out); } catch {} }
}

function fpsLadder(startFps) {
  const l = FPS_LADDER.filter(f => f <= startFps);
  if (!l.includes(startFps)) l.push(startFps);
  return l.sort((a, b) => b - a);
}

// Smallest q (best quality) whose estimate fits, or null if even q=31 does not.
async function findQ(c, fps) {
  if ((await sampleEstimate(c, fps, 31)) > c.budget * 1.15) return null;
  let lo = 3, hi = 31;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((await sampleEstimate(c, fps, mid)) <= c.budget * 0.96) hi = mid; else lo = mid + 1;
  }
  return lo;
}

/** Predict the result without writing output. opts as convert(). */
async function estimate(file, dir, opts) {
  const c = await prepare(file, dir, opts);
  const common = { length: c.length, start: c.start, budget: c.budget, duration: c.info.duration };
  if (c.manual) {
    const bytes = await sampleEstimate(c, c.startFps, c.q);
    return { ...common, mode: 'manual', fps: c.startFps, q: c.q, bytes: Math.round(bytes), fits: bytes <= c.budget };
  }
  for (const fps of fpsLadder(c.startFps)) {
    const q = await findQ(c, fps);
    if (q != null) {
      const bytes = await sampleEstimate(c, fps, q);
      return { ...common, mode: 'auto', fps, q, bytes: Math.round(bytes), fits: true };
    }
  }
  const fps = Math.min(10, c.startFps);
  const bytes = await sampleEstimate(c, fps, 31);
  return { ...common, mode: 'auto', fps, q: 31, bytes: Math.round(bytes), fits: false };
}

/**
 * Convert `file` into dir/device.mp4 (+ preview/thumb).
 * opts: { start, length, fps, quality: 'auto'|1..100, mask, budgetBytes, maxSeconds, onProgress(frac, msg) }
 * Auto: fits budget (throws if impossible). Manual: keeps the file even if over budget (tooBig: true).
 */
async function convert(file, dir, opts) {
  const report = opts.onProgress || (() => {});
  const c = await prepare(file, dir, opts);
  const out = path.join(dir, 'device.mp4');
  const tmp = path.join(dir, 'device.tmp.mp4');

  report(0.02, 'Rendering preview');
  await makePreview(file, c.src, { ...c.base, fps: c.startFps }, dir,
    t => report(0.02 + 0.2 * Math.min(1, t / c.length), 'Rendering preview'));

  let result = null;
  try {
    if (c.manual) {
      report(0.3, `Encoding ${c.startFps} fps, q=${c.q}`);
      const size = await encodeDevice(file, c.src, { ...c.base, fps: c.startFps }, c.q, tmp,
        t => report(0.3 + 0.68 * Math.min(1, t / c.length), `Encoding ${c.startFps} fps, q=${c.q}`));
      fs.renameSync(tmp, out);
      result = { size, fps: c.startFps, q: c.q, tooBig: size > c.budget };
    } else {
      const ladder = fpsLadder(c.startFps);
      for (let li = 0; li < ladder.length && !result; li++) {
        const fps = ladder[li];
        report(0.25 + 0.03 * li, `Estimating size at ${fps} fps`);
        let q = await findQ(c, fps);
        if (q == null) continue;
        for (;;) {
          report(0.4, `Encoding ${fps} fps, q=${q}`);
          const size = await encodeDevice(file, c.src, { ...c.base, fps }, q, tmp,
            t => report(0.4 + 0.58 * Math.min(1, t / c.length), `Encoding ${fps} fps, q=${q}`));
          if (size <= c.budget) { fs.renameSync(tmp, out); result = { size, fps, q, tooBig: false }; break; }
          if (q >= 31) break;
          q++;
        }
      }
    }
  } finally {
    try { fs.unlinkSync(tmp); } catch {}
  }
  if (!result) throw new Error('Could not fit this clip into the available space even at 10 fps and lowest quality. Try a shorter length, or free the other slot.');
  report(1, 'Done');
  return { ...result, quality: qualityFromQ(result.q), start: c.start, length: c.length, duration: c.info.duration, isImage: c.isImage };
}

// Tiny 1-frame black 1080x2400 MJPEG mp4, used to "clear" a slot.
async function makeBlank(out) {
  await runFfmpeg(['-f', 'lavfi', '-i', 'color=black:s=1080x2400:r=1', '-frames:v', '1', '-c:v', 'mjpeg',
    '-pix_fmt', 'yuvj420p', '-q:v', '31', '-movflags', '+faststart', '-y', out]);
  return fs.statSync(out).size;
}

module.exports = { convert, estimate, probe, makeBlank, maskGraph, qFromQuality, qualityFromQ, runFfmpeg, ffmpegPath: ffmpeg, W, H };
