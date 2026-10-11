// Media -> device format. Landscape 2400x1080 (framing, cover by default) -> notch mask -> rotate 90 deg CW ->
// 1080x2400 MJPEG mp4 at the screen's fixed playback rate.
const { spawn } = require('child_process');
const fs = require('fs');
const path = require('path');
const paths = require('./paths');

let ffmpeg = require('ffmpeg-static');
if (ffmpeg && ffmpeg.includes('app.asar')) ffmpeg = ffmpeg.replace('app.asar', 'app.asar.unpacked');

const W = 2400, H = 1080;
const IMAGE_EXT = new Set(['.png', '.jpg', '.jpeg', '.bmp', '.webp', '.tif', '.tiff']);
// Measured on the SC70 (2026-10-10, two runs: 17.6 and 18.0 fps): the screen ignores the file's timing and always
// plays at about 18 frames per second, so every version is encoded at exactly this rate to play at real speed.
const DEVICE_FPS = 18;
const IMAGE_SECONDS = 3; // a still image becomes a short clip of this length

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
      const c = /Video:\s*([a-z0-9_]+)/i.exec(err);
      const fr = /Video:.*?(\d+(?:\.\d+)?) fps/.exec(err);
      const rot = /rotat(?:e|ion)[^\d-]*(-?\d+(?:\.\d+)?)/i.exec(err);
      let width = v ? +v[1] : null, height = v ? +v[2] : null;
      // Phone videos store a rotation; ffmpeg (and the browser) apply it, so report the displayed size.
      if (rot && Math.abs(Math.round(+rot[1])) % 180 === 90) [width, height] = [height, width];
      resolve({
        duration: d ? +d[1] * 3600 + +d[2] * 60 + +d[3] : null,
        width, height,
        fps: fr ? +fr[1] : null,
        codec: c ? c[1].toLowerCase() : null,
        hasVideo: /Video:/.test(err),
        isImage: IMAGE_EXT.has(path.extname(file).toLowerCase()),
      });
    });
  });
}

const even = n => n - (n % 2);

// MJPEG 4:2:0 compresses in 16x16 blocks of the stored (portrait, transposed) frame. A band edge that
// falls inside a block leaves faint compression specks in the black area, which are visible on AMOLED.
// Round the band up so its edge lands on a block boundary. Landscape left/right/bottom map to portrait
// rows/columns starting at a block boundary; the top edge maps to the last columns of the 1080 px width.
function bandPx(band) {
  const px = Math.round(band.px);
  if (px <= 0) return 0;
  const offset = band.edge === 'top' ? (1080 % 16) : 0;
  return Math.ceil((px - offset) / 16) * 16 + offset;
}

// Returns filtergraph pieces that take [base] and produce [m] (masked landscape).
function maskGraph(mask) {
  if (!mask || mask.mode === 'off') return '[base]null[m]';
  if (mask.mode === 'band') {
    const px = bandPx(mask.band);
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

// Framing of the source inside the 2400x1080 screen. zoom is relative to "cover" (1 = fills the screen exactly,
// <1 leaves black bars, >1 crops more); x/y (0..1) align the picture: 0 = its left/top edge on the screen's
// left/top edge, 0.5 = centred, 1 = right/bottom edges aligned. public/js/frame.js mirrors this arithmetic.
const FRAME_DEFAULT = { zoom: 1, x: 0.5, y: 0.5 };
function normFrame(f) {
  f = f || {};
  const n = (v, d, lo, hi) => (v !== null && v !== '' && Number.isFinite(Number(v)) ? Math.min(hi, Math.max(lo, Number(v))) : d);
  return {
    zoom: Math.round(n(f.zoom, 1, 0.2, 4) * 1000) / 1000,
    x: Math.round(n(f.x, 0.5, 0, 1) * 1000) / 1000,
    y: Math.round(n(f.y, 0.5, 0, 1) * 1000) / 1000,
  };
}
const isDefaultFrame = f => f.zoom === 1 && f.x === 0.5 && f.y === 0.5;

function frameGraph(frame) {
  const f = normFrame(frame);
  if (isDefaultFrame(f)) return `scale=${W}:${H}:force_original_aspect_ratio=increase:flags=lanczos,crop=${W}:${H}`;
  const S = `${f.zoom}*max(${W}/iw\\,${H}/ih)`;
  return `scale=w='2*trunc(iw*${S}/2)':h='2*trunc(ih*${S}/2)':flags=lanczos,` +
    `pad=w='max(iw\\,${W})':h='max(ih\\,${H})':x='2*trunc((ow-iw)*${f.x}/2)':y='2*trunc((oh-ih)*${f.y}/2)':color=black,` +
    `crop=w=${W}:h=${H}:x='2*trunc((iw-${W})*${f.x}/2)':y='2*trunc((ih-${H})*${f.y}/2)'`;
}

// Rounded corners: black outside a rounded rectangle covering the whole 2400x1080 canvas. Made once per radius as an
// RGBA PNG (1 px anti-aliased edge) and overlaid on every frame, instead of evaluating a per-pixel expression per frame.
// public/js/frame.js styleCorners() draws the same shape for the preview.
const cornersOn = c => !!(c && c.on && Math.round(c.r) > 0);
// The visible picture: the canvas minus the notch band (a band on the left edge hides the left corners, so the corners
// are rounded at the band's inner edge). public/js/frame.js visibleRect() is the same.
function visibleRect(mask) {
  const r = { x0: 0, y0: 0, x1: W, y1: H };
  if (mask && mask.mode === 'band') {
    const px = bandPx(mask.band);
    if (px > 0) ({ left: () => (r.x0 = px), right: () => (r.x1 = W - px), top: () => (r.y0 = px), bottom: () => (r.y1 = H - px) })[mask.band.edge]();
  }
  return r;
}

async function cornerMaskFile(corners, mask) {
  if (!cornersOn(corners)) return null;
  const { x0, y0, x1, y1 } = visibleRect(mask);
  const R = Math.min(Math.round(corners.r), Math.floor(Math.min(x1 - x0, y1 - y0) / 2));
  const dir = path.join(paths.base, 'masks');
  const f = path.join(dir, `corners-${W}x${H}-r${R}-${x0}-${y0}-${x1}-${y1}.png`);
  if (fs.existsSync(f)) return f;
  fs.mkdirSync(dir, { recursive: true });
  // Distance from the pixel centre to the rounded rectangle, beyond the corner arcs (0 inside), anti-aliased over 1 px.
  const dx = `max(max(${x0 + R}-(X+0.5),(X+0.5)-${x1 - R}),0)`, dy = `max(max(${y0 + R}-(Y+0.5),(Y+0.5)-${y1 - R}),0)`;
  const a = `255*clip(hypot(${dx},${dy})-${R}+0.5,0,1)`;
  const tmp = f + '.tmp.png';
  await runFfmpeg(['-f', 'lavfi', '-i', `color=black:s=${W}x${H}:d=1`, '-frames:v', '1',
    '-vf', `format=rgba,geq=r='0':g='0':b='0':a='${a}'`, '-y', tmp]);
  fs.renameSync(tmp, f);
  return f;
}

// With a loop blend of B seconds the input starts B seconds before the selection (those frames fade in at the end).
function inputArgs(file, { isImage, fps, start, length, cornerFile, blend = 0 }) {
  const extra = cornerFile ? ['-i', cornerFile] : [];
  if (isImage) return ['-loop', '1', '-framerate', String(fps), '-t', String(length), '-i', file, ...extra];
  return ['-ss', String(r3(start - blend)), '-t', String(r3(length + blend)), '-i', file, ...extra];
}
const r3 = v => Math.round(v * 1000) / 1000;

// "Simplify fine detail" 0-100 -> Gaussian blur sigma 0-3.5 px on the 2400x1080 canvas. Measured on an anime-style theme
// (2026-10-11): gblur shrinks MJPEG frames far more per unit of visible change than hqdn3d or smartblur.
// The preview mirrors it with a CSS blur of the same standard deviation (public/app.js).
const detailSigma = level => Math.round((Math.min(100, Math.max(0, Number(level) || 0)) / 100) * 3.5 * 100) / 100;

// [m] (notch applied) -> [mc] (corners applied). The corner PNG is input 1; overlay repeats its single frame.
const cornersGraph = cornerFile => (cornerFile ? '[m][1:v]overlay=0:0:format=auto,format=yuv420p[mc]' : '[m]null[mc]');

// Source -> [base]: frame rate, framing, optional detail simplification, optional loop blend.
// Loop blend (the standard seamless-loop trick): the last B seconds of the selection cross-fade into the B seconds just
// before its start, so the final frame leads straight into the first one. The output length stays the selection length.
function baseGraph(o) {
  const sigma = detailSigma(o.detail);
  // After "-ss X" the first decoded frame (the first source frame at or after X) gets timestamp 0. Shift timestamps so
  // that 0 is X itself; then output frame n is the source frame nearest X + n/18 and frame Bn is exactly the start.
  const X = (o.start || 0) - (o.blend || 0), sf = o.srcFps;
  const lead = !o.isImage && sf ? Math.max(0, Math.ceil(X * sf - 1e-6) / sf - X) : 0;
  const shift = lead > 1e-4 ? `settb=AVTB,setpts=PTS+${lead.toFixed(6)}/TB,` : ''; // fine timebase: the shift is sub-frame
  const pre = `[0:v]${shift}fps=${o.fps},${frameGraph(o.frame)},setsar=1,format=yuv420p${sigma > 0 ? `,gblur=sigma=${sigma}` : ''}`;
  const B = o.blend || 0;
  if (!(B > 0)) return `${pre}[base]`;
  // Trim by frame index (exact), not by timestamp: frame Bn of the resampled stream is the selection's first frame.
  const Bn = Math.round(B * o.fps), Ln = Math.round(o.length * o.fps);
  return `${pre},split[ba][bb];[ba]trim=start_frame=${Bn}:end_frame=${Bn + Ln},setpts=PTS-STARTPTS[bm];` +
    `[bb]trim=start_frame=0:end_frame=${Bn},setpts=PTS-STARTPTS[bp];` +
    `[bm][bp]xfade=transition=fade:duration=${B}:offset=${r3(o.length - B)},format=yuv420p[base]`;
}

function deviceArgs(file, src, o, q, out) {
  const g = `${baseGraph(o)};${maskGraph(o.mask)};${cornersGraph(o.cornerFile)};[mc]transpose=1,format=yuvj420p[out]`;
  return [...inputArgs(file, { ...src, ...o }), '-filter_complex', g, '-map', '[out]', '-an',
    '-c:v', 'mjpeg', '-q:v', String(q), '-r', String(o.fps), '-movflags', '+faststart', '-y', out];
}

async function encodeDevice(file, src, o, q, out, onTime) {
  await runFfmpeg(deviceArgs(file, src, o, q, out), { onTime });
  return fs.statSync(out).size;
}

// Landscape H.264 preview of a version (framing + mask applied) and its thumbnail.
async function makePreview(file, src, o, dir, onTime) {
  const g = `${baseGraph(o)};${maskGraph(o.mask)};${cornersGraph(o.cornerFile)};[mc]scale=1200:540:flags=bicubic,format=yuv420p[out]`;
  await runFfmpeg([...inputArgs(file, { ...src, ...o }), '-filter_complex', g, '-map', '[out]', '-an',
    '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '22', '-movflags', '+faststart', '-y',
    path.join(dir, 'preview.mp4')], { onTime });
  const seek = String(Math.min(0.5, Math.max(0, o.length / 2)));
  await runFfmpeg(['-ss', seek, '-i', path.join(dir, 'preview.mp4'), '-frames:v', '1',
    '-vf', 'scale=400:-2', '-q:v', '4', '-y', path.join(dir, 'thumb.jpg')]);
}

// Browser-playable formats shown without a proxy.
const DIRECT_EXT = new Set(['.mp4', '.m4v', '.webm']);
const DIRECT_CODECS = new Set(['h264', 'vp8', 'vp9', 'av1']);
const DIRECT_IMAGE = new Set(['.jpg', '.jpeg', '.png', '.webp']);
const canPlayDirect = (file, info) => info.isImage
  ? DIRECT_IMAGE.has(path.extname(file).toLowerCase())
  : DIRECT_EXT.has(path.extname(file).toLowerCase()) && DIRECT_CODECS.has(info.codec);
const PROXY_MAX_SECONDS = 600;

/**
 * App-only assets of an original (never sent to the screen): thumb.jpg (20:9 cover), frame.jpg (unmasked 1200x540
 * cover, for the notch editor) and, when the browser can't show the source itself, a proxy (proxy.mp4 / proxy.jpg)
 * that keeps the source's aspect ratio and timing so trim and framing in the browser match the converter.
 */
async function makeSourceAssets(file, dir, info, onTime) {
  fs.mkdirSync(dir, { recursive: true });
  const seekArgs = info.isImage || !info.duration ? [] : ['-ss', String(Math.min(info.duration / 2, 1))];
  const cover = (w, h) => `scale=${w}:${h}:force_original_aspect_ratio=increase,crop=${w}:${h},setsar=1`;
  await runFfmpeg([...seekArgs, '-i', file, '-frames:v', '1', '-vf', cover(240, 108), '-q:v', '4', '-y', path.join(dir, 'thumb.jpg')]);
  await runFfmpeg([...seekArgs, '-i', file, '-frames:v', '1', '-vf', cover(1200, 540), '-q:v', '4', '-y', path.join(dir, 'frame.jpg')]);
  if (canPlayDirect(file, info)) return { proxy: null };
  if (info.isImage) {
    await runFfmpeg(['-i', file, '-frames:v', '1', '-vf', "scale=w='min(2400,iw)':h='min(2400,ih)':force_original_aspect_ratio=decrease",
      '-q:v', '3', '-y', path.join(dir, 'proxy.jpg')]);
    return { proxy: 'proxy.jpg' };
  }
  const tmp = path.join(dir, 'proxy.tmp.mp4');
  await runFfmpeg(['-i', file, '-t', String(PROXY_MAX_SECONDS), '-an', '-vf',
    "scale=w='min(1600,iw)':h='min(1600,ih)':force_original_aspect_ratio=decrease,scale=trunc(iw/2)*2:trunc(ih/2)*2,setsar=1,format=yuv420p",
    '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '23', '-movflags', '+faststart', '-y', tmp], { onTime });
  fs.renameSync(tmp, path.join(dir, 'proxy.mp4'));
  return { proxy: 'proxy.mp4' };
}

// Friendly quality 1-100 (higher = better) <-> ffmpeg mjpeg -q:v 31..3
const qFromQuality = Q => Math.round(31 - ((Math.min(100, Math.max(1, Number(Q))) - 1) * 28) / 99);
const qualityFromQ = q => Math.round(1 + ((31 - q) * 99) / 28);

const rnd = () => Math.random().toString(36).slice(2, 8);

async function prepare(file, dir, opts) {
  const info = await probe(file);
  if (!info.hasVideo) throw new Error('No video/image stream found in this file.');
  const isImage = info.isImage;
  const start = isImage ? 0 : r3(Math.max(0, Number(opts.start) || 0));
  const avail = info.duration != null && !isImage ? Math.max(0.1, info.duration - start) : Infinity;
  let length = Number(opts.length);
  if (!(length > 0)) length = isImage ? IMAGE_SECONDS : Math.min(opts.maxSeconds || 10, avail);
  length = Math.max(0.1, Math.min(length, avail));
  length = r3(length);
  if (!Number.isFinite(length)) length = 10;
  const manual = opts.quality != null && opts.quality !== 'auto';
  fs.mkdirSync(dir, { recursive: true });
  const frame = normFrame(opts.frame);
  // Blend: whole output frames, at most half the selection, and only as much as exists before the start.
  let blend = isImage ? 0 : Math.max(0, Math.min(Number(opts.blend) || 0, 1.5, length / 2, start));
  blend = Math.floor(blend * DEVICE_FPS + 1e-6) / DEVICE_FPS;
  return {
    file, dir, info, isImage, src: { isImage }, fps: DEVICE_FPS, start, length, manual,
    q: manual ? qFromQuality(opts.quality) : null,
    budget: opts.budgetBytes,
    blend,
    base: { start, length, mask: opts.mask, frame, fps: DEVICE_FPS, srcFps: info.fps, isImage, cornerFile: await cornerMaskFile(opts.corners, opts.mask), detail: opts.detail || 0, blend: r3(blend) },
  };
}

// Estimated full-length size from a short sample encode.
async function sampleEstimate(c, q) {
  const sLen = Math.min(2, c.length);
  const sStart = c.start + Math.max(0, c.length / 2 - sLen / 2);
  const out = path.join(c.dir, `sample.${rnd()}.tmp.mp4`);
  try {
    const bytes = await encodeDevice(c.file, c.src, { ...c.base, start: c.isImage ? 0 : sStart, length: sLen, blend: 0 }, q, out);
    return (bytes * c.length) / sLen;
  } finally { try { fs.unlinkSync(out); } catch {} }
}

// Longest length (seconds, rounded down to 0.5 s) that should fit at the lowest quality, from the q=31 estimate.
function maxFitLength(c, bytesAtWorst) {
  const perSec = bytesAtWorst / c.length;
  if (!(perSec > 0)) return null;
  return Math.max(0.5, Math.floor(((c.budget * 0.94) / perSec) * 2) / 2);
}

// Smallest q (best quality) in [lo, hi] whose estimate fits (assumes hi fits).
async function findQRange(c, lo, hi) {
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((await sampleEstimate(c, mid)) <= c.budget * 0.96) hi = mid; else lo = mid + 1;
  }
  return lo;
}

// Smallest q (best quality) whose estimate fits, or null if even q=31 does not; also the q=31 estimate.
async function findQ(c) {
  const worst = await sampleEstimate(c, 31);
  if (worst > c.budget * 1.15) return { q: null, worst };
  return { q: await findQRange(c, 3, 31), worst };
}

/**
 * "Best that fits", fully automatic: chooses the "simplify fine detail" level and the JPEG quality together.
 * Rule (from measurements on anime-style and photographic clips, 2026-10-11): sharpness from a finer JPEG quantiser is
 * worth more than a little blur, until the quality would drop below AUTO_MIN_QUALITY; from there a mild blur buys back
 * far more quality than it costs. So: the least blur that still allows quality >= AUTO_MIN_QUALITY; if no level up to
 * 60 gets there, the level with the best (quality - 0.5 x detail) among those that fit; levels 80/100 only when
 * nothing else fits. Returns { detail, q } or { q: null, worst } when even detail 100 at q=31 is too big.
 */
const AUTO_LEVELS = [0, 15, 30, 45, 60], AUTO_LAST_RESORT = [80, 100], AUTO_MIN_QUALITY = 45;
async function chooseAuto(c, report = () => {}) {
  const at = d => ({ ...c, base: { ...c.base, detail: d } });
  const qT = qFromQuality(AUTO_MIN_QUALITY);
  for (const d of AUTO_LEVELS) {
    report(d);
    const cd = at(d);
    if ((await sampleEstimate(cd, qT)) <= c.budget * 0.96) return { detail: d, q: await findQRange(cd, 3, qT) };
  }
  let best = null, worst = null;
  for (const d of [...AUTO_LEVELS, ...AUTO_LAST_RESORT]) {
    if (best && AUTO_LAST_RESORT.includes(d)) break;
    report(d);
    const cd = at(d);
    worst = await sampleEstimate(cd, 31);
    if (worst > c.budget * 0.96) continue;
    const q = await findQRange(cd, qT, 31);
    const score = qualityFromQ(q) - 0.5 * d;
    if (best && score <= best.score) break; // more blur stopped paying off
    best = { detail: d, q, score };
  }
  return best ? { detail: best.detail, q: best.q } : { q: null, worst };
}

/** Bytes per second of output at a "sensible" quality (AUTO_MIN_QUALITY, no blur): sizes loop-length suggestions. */
async function sampleBytesPerSecond(file, dir, opts) {
  const c = await prepare(file, dir, { ...opts, quality: AUTO_MIN_QUALITY });
  return (await sampleEstimate({ ...c, base: { ...c.base, detail: 0 } }, qFromQuality(AUTO_MIN_QUALITY))) / c.length;
}

/** Predict the result without writing output. opts as convert(). */
async function estimate(file, dir, opts) {
  const c = await prepare(file, dir, opts);
  const common = { length: c.length, start: c.start, budget: c.budget, duration: c.info.duration, fps: c.fps };
  if (c.manual) {
    const bytes = await sampleEstimate(c, c.q);
    const r = { ...common, mode: 'manual', q: c.q, detail: c.base.detail || 0, bytes: Math.round(bytes), fits: bytes <= c.budget };
    if (!r.fits) r.maxFitLength = maxFitLength(c, bytes); // at the chosen quality
    return r;
  }
  const a = await chooseAuto(c);
  if (a.q != null) {
    const bytes = await sampleEstimate({ ...c, base: { ...c.base, detail: a.detail } }, a.q);
    return { ...common, mode: 'auto', q: a.q, detail: a.detail, bytes: Math.round(bytes), fits: true };
  }
  return { ...common, mode: 'auto', q: 31, detail: 100, bytes: Math.round(a.worst), fits: false, maxFitLength: maxFitLength(c, a.worst) };
}

/**
 * Convert `file` into dir/device.mp4 (+ preview.mp4, thumb.jpg), always at DEVICE_FPS.
 * opts: { start, length, quality: 'auto'|1..100, detail, frame, mask, corners, blend, budgetBytes, maxSeconds, onProgress }
 * Auto chooses detail and quality together (chooseAuto), then encodes, raising q (then detail) if the real file is
 * bigger than the estimate; error code 'tooLong' with maxFitLength if nothing fits. Manual uses the given quality and
 * detail and keeps the file even if over budget (tooBig: true).
 */
async function convert(file, dir, opts) {
  const report = opts.onProgress || (() => {});
  const c = await prepare(file, dir, opts);
  const out = path.join(dir, 'device.mp4');
  const tmp = path.join(dir, 'device.tmp.mp4');

  let result = null, worst = null, base = c.base;
  try {
    if (c.manual) {
      report(0.02, 'Making the preview');
      await makePreview(file, c.src, base, dir, t => report(0.02 + 0.2 * Math.min(1, t / c.length), 'Making the preview'));
      const size = await encodeDevice(file, c.src, base, c.q, tmp, t => report(0.3 + 0.68 * Math.min(1, t / c.length), 'Preparing'));
      fs.renameSync(tmp, out);
      result = { size, q: c.q, detail: base.detail || 0, tooBig: size > c.budget };
    } else {
      report(0.03, 'Choosing the best quality that fits');
      const a = await chooseAuto(c, d => report(0.03 + 0.2 * (d / 100), 'Choosing the best quality that fits'));
      worst = a.worst;
      if (a.q != null) {
        const levels = [...AUTO_LEVELS, ...AUTO_LAST_RESORT].filter(d => d >= a.detail);
        let li = 0, q = a.q;
        base = { ...c.base, detail: levels[0] };
        report(0.25, 'Making the preview');
        await makePreview(file, c.src, base, dir, t => report(0.25 + 0.15 * Math.min(1, t / c.length), 'Making the preview'));
        for (;;) {
          const size = await encodeDevice(file, c.src, base, q, tmp, t => report(0.4 + 0.58 * Math.min(1, t / c.length), 'Preparing'));
          if (size <= c.budget) { fs.renameSync(tmp, out); result = { size, q, detail: base.detail, tooBig: false }; break; }
          if (q < 31) { q++; continue; }
          if (++li >= levels.length) break;
          base = { ...c.base, detail: levels[li] };
          q = (await findQ({ ...c, base })).q ?? 31;
          await makePreview(file, c.src, base, dir);
        }
      }
    }
  } finally {
    try { fs.unlinkSync(tmp); } catch {}
  }
  if (!result) {
    const n = worst != null ? maxFitLength(c, worst) : null;
    const e = new Error('This part is too long to fit even at the lowest quality.' + (n ? ` Try about ${n} s or less.` : ' Try a shorter length.'));
    e.code = 'tooLong'; e.maxFitLength = n;
    throw e;
  }
  report(1, 'Done');
  return { ...result, fps: c.fps, quality: qualityFromQ(result.q), start: c.start, length: c.length, frame: c.base.frame, blend: c.blend,
    duration: c.info.duration, isImage: c.isImage };
}

// Small grey frames (source frame rate, no resampling) decoded straight to memory, for loop-point matching.
const LW = 96, LH = 54;
function grayFrames(file, start, dur, maxFrames, fw = LW, fh = LH, onFrames) {
  return new Promise((resolve, reject) => {
    const args = ['-hide_banner', '-loglevel', 'error', '-nostdin', '-ss', String(Math.max(0, start)), '-i', file,
      ...(dur ? ['-t', String(dur)] : []), ...(maxFrames ? ['-frames:v', String(maxFrames)] : []),
      '-an', '-fps_mode', 'passthrough', '-vf', `scale=${fw}:${fh}:flags=area,format=gray`, '-f', 'rawvideo', 'pipe:1'];
    const p = spawn(ffmpeg, args, { windowsHide: true });
    const chunks = []; let err = '', got = 0;
    p.stdout.on('data', d => { chunks.push(d); got += d.length; if (onFrames) onFrames(Math.floor(got / (fw * fh))); });
    p.stderr.on('data', d => (err += d));
    p.on('error', reject);
    p.on('close', code => {
      if (code !== 0) return reject(new Error('ffmpeg failed: ' + err.trim().split('\n').slice(-2).join(' | ')));
      const buf = Buffer.concat(chunks), n = Math.floor(buf.length / (fw * fh)), out = [];
      for (let i = 0; i < n; i++) out.push(buf.subarray(i * fw * fh, (i + 1) * fw * fh));
      resolve(out);
    });
  });
}
const mad = (a, b) => { let s = 0; for (let i = 0; i < a.length; i++) s += Math.abs(a[i] - b[i]); return s / a.length; };

/**
 * "Find best loop point": the loop is seamless when the frame that would follow its last frame looks like its first
 * frame. Searches source frames within +-2 s of the current end for the one most similar to the start frame
 * (mean absolute difference of 96x54 grey thumbnails). Returns the best end and a few alternatives; similarity is
 * 100 - difference in percent.
 */
async function findLoopPoint(file, { start, end, duration, fps }) {
  const srcFps = fps || 25;
  const [first] = await grayFrames(file, start, null, 1);
  if (!first) throw new Error('Could not read the start frame.');
  const lo = Math.max(start + 0.5, end - 2), hi = Math.min(duration || end + 2, end + 2);
  if (!(hi > lo)) throw new Error('The clip is too short to search for a loop point.');
  const frames = await grayFrames(file, lo, hi - lo);
  const t0 = Math.ceil(lo * srcFps - 1e-6) / srcFps;
  const cands = frames.map((f, i) => ({ end: Math.round((t0 + i / srcFps) * 1000) / 1000, diff: mad(first, f) }));
  const cur = cands.reduce((b, c) => (Math.abs(c.end - end) < Math.abs(b.end - end) ? c : b), cands[0]);
  const sorted = [...cands].sort((a, b) => a.diff - b.diff);
  const sim = d => Math.round((100 - (d / 255) * 100) * 10) / 10;
  const pick = c => ({ end: c.end, length: r3(Math.round((c.end - start) * DEVICE_FPS) / DEVICE_FPS), similarity: sim(c.diff) });
  const alts = [];
  for (const c of sorted) { if (alts.every(a => Math.abs(a.end - c.end) > 0.3)) alts.push(c); if (alts.length >= 4) break; }
  return { best: pick(sorted[0]), current: pick(cur), alternatives: alts.slice(1).map(pick), searched: [lo, hi] };
}

// Tiny 1-frame black 1080x2400 MJPEG mp4 (CLI "clear").
async function makeBlank(out) {
  await runFfmpeg(['-f', 'lavfi', '-i', 'color=black:s=1080x2400:r=1', '-frames:v', '1', '-c:v', 'mjpeg',
    '-pix_fmt', 'yuvj420p', '-q:v', '31', '-movflags', '+faststart', '-y', out]);
  return fs.statSync(out).size;
}

// The built-in intro (startup slot): a short boot splash, black with a small centred "CoolerScreen" wordmark that
// fades in and out. Centred on the 2400x1080 canvas, far from the notch band on the left edge.
const SPLASH = { version: 1, seconds: 1.5, text: 'CoolerScreen' };
function splashFont() {
  const dir = path.join(process.env.WINDIR || 'C:\\Windows', 'Fonts');
  for (const f of ['segoeuisl.ttf', 'segoeui.ttf', 'arial.ttf']) {
    const p = path.join(dir, f);
    if (fs.existsSync(p)) return p;
  }
  return null; // let ffmpeg/fontconfig pick a default
}

async function makeSplash(out) {
  const font = splashFont();
  // drawtext option escaping: the font path's drive colon and backslashes must be escaped.
  const fontOpt = font ? `fontfile='${font.replace(/\\/g, '/').replace(/:/g, '\\:')}':` : '';
  const T = SPLASH.seconds, fade = 0.4;
  const alpha = `if(lt(t\\,${fade})\\,t/${fade}\\,if(gt(t\\,${T - fade})\\,max(0\\,(${T}-t)/${fade})\\,1))`;
  const g = `color=black:s=${W}x${H}:r=${DEVICE_FPS}:d=${T},` +
    `drawtext=${fontOpt}text='${SPLASH.text}':fontsize=84:fontcolor=0xE6E8EC:alpha='${alpha}':x=(w-text_w)/2:y=(h-text_h)/2,` +
    'transpose=1,format=yuvj420p';
  await runFfmpeg(['-f', 'lavfi', '-i', g, '-an', '-c:v', 'mjpeg', '-q:v', '8', '-r', String(DEVICE_FPS),
    '-movflags', '+faststart', '-y', out]);
  return fs.statSync(out).size;
}

module.exports = {
  convert, estimate, probe, cornerMaskFile, visibleRect, grayFrames, sampleBytesPerSecond, AUTO_MIN_QUALITY, detailSigma, findLoopPoint, makeBlank, makeSplash, makeSourceAssets, maskGraph, frameGraph, normFrame, isDefaultFrame,
  bandPx, qFromQuality, qualityFromQ, runFfmpeg, ffmpegPath: ffmpeg, W, H, DEVICE_FPS, IMAGE_SECONDS, FRAME_DEFAULT, SPLASH,
};
