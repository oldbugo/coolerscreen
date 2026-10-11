// Library = originals (files in the Gallery folder) and their versions (prepared copies for the screen).
// Cache layout under %APPDATA%\CoolerScreen\library:
//   <originalId>/original.json   thumb.jpg  frame.jpg  [proxy.mp4|proxy.jpg]  [original.<ext> for .ht/.zip]
//   <originalId>/versions/<versionId>/version.json  device.mp4  preview.mp4  thumb.jpg
// originalId is random and permanent (a file that is renamed or edited keeps its id and versions).
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const paths = require('./paths');
const settings = require('./settings');
const slots = require('./slots');
const { findLoops } = require('./loopsearch');
const { renderLoopPreview, sampleBytesPerSecond, findLoopPoint: findLoop, convert, estimate, qualityFromQ, normFrame, bandPx, DEVICE_FPS } = require('./convert');

const ID_RE = /^[a-z0-9]{8,32}$/;
const O_FILES = new Set(['thumb.jpg', 'frame.jpg', 'proxy.mp4', 'proxy.jpg']);
const V_FILES = new Set(['preview.mp4', 'thumb.jpg', 'device.mp4']);

const isId = id => typeof id === 'string' && ID_RE.test(id);
// The extracted video of a .ht/.zip original is stored as original.<ext> (original.json is the metadata).
const isSourceName = n => n.startsWith('original.') && n !== 'original.json' && !n.endsWith('.tmp');
const oDir = oid => path.join(paths.library, oid);
const vRoot = oid => path.join(oDir(oid), 'versions');
const vDir = (oid, vid) => path.join(vRoot(oid), vid);
const newId = () => Date.now().toString(36) + crypto.randomBytes(3).toString('hex');

// A counter the UI polls to know when to reload (any change to originals or versions bumps it).
let revision = 1;
const bump = () => ++revision;
const rev = () => revision;

function readJson(f) { try { return JSON.parse(fs.readFileSync(f, 'utf8')); } catch { return null; } }
function writeJson(f, obj) {
  const tmp = f + '.tmp';
  fs.writeFileSync(tmp, JSON.stringify(obj, null, 2));
  fs.renameSync(tmp, f);
}

// ---------- originals ----------
const readOriginal = oid => (isId(oid) ? readJson(path.join(oDir(oid), 'original.json')) : null);
function writeOriginal(o) {
  fs.mkdirSync(oDir(o.id), { recursive: true });
  writeJson(path.join(oDir(o.id), 'original.json'), o);
  bump();
  return o;
}
function patchOriginal(oid, patch) {
  const o = readOriginal(oid);
  if (!o) throw new Error('Unknown clip');
  return writeOriginal({ ...o, ...patch });
}
function listOriginals() {
  let ids = [];
  try { ids = fs.readdirSync(paths.library).filter(isId); } catch { return []; }
  return ids.map(readOriginal).filter(Boolean).sort((a, b) => (b.created || '').localeCompare(a.created || ''));
}

/** The media to convert from: the gallery file, or the video extracted from a .ht/.zip (kept in the cache). */
function sourcePath(o) {
  if (!o) return null;
  if (o.extracted) {
    try {
      const f = fs.readdirSync(oDir(o.id)).find(isSourceName);
      return f ? path.join(oDir(o.id), f) : null;
    } catch { return null; }
  }
  return o.file && !o.missing && fs.existsSync(o.file) ? o.file : null;
}

function removeOriginalCache(oid) {
  if (!isId(oid)) return;
  fs.rmSync(oDir(oid), { recursive: true, force: true });
  for (const [vid, o] of vIndex) if (o === oid) vIndex.delete(vid);
  bump();
}

// ---------- versions ----------
const vIndex = new Map(); // versionId -> originalId
function versions(oid) {
  let ids = [];
  try { ids = fs.readdirSync(vRoot(oid)).filter(isId); } catch { return []; }
  const out = [];
  for (const vid of ids) {
    const v = readJson(path.join(vDir(oid, vid), 'version.json'));
    if (v) { vIndex.set(vid, oid); out.push(v); }
  }
  return out.sort((a, b) => b.n - a.n);
}
function findVersion(vid) {
  if (!isId(vid)) return null;
  let oid = vIndex.get(vid);
  if (!oid || !fs.existsSync(vDir(oid, vid))) {
    oid = null;
    for (const o of listOriginals()) { if (fs.existsSync(vDir(o.id, vid))) { oid = o.id; break; } }
  }
  if (!oid) return null;
  const v = readJson(path.join(vDir(oid, vid), 'version.json'));
  if (!v) return null;
  vIndex.set(vid, oid);
  return { oid, v };
}
function writeVersion(oid, v) {
  fs.mkdirSync(vDir(oid, v.id), { recursive: true });
  writeJson(path.join(vDir(oid, v.id), 'version.json'), v);
  vIndex.set(v.id, oid);
  bump();
  return v;
}
function patchVersion(vid, patch) {
  const f = findVersion(vid);
  if (!f) throw new Error('Unknown version');
  return writeVersion(f.oid, { ...f.v, ...patch });
}
function deleteVersion(vid) {
  const f = findVersion(vid);
  if (!f) return false;
  fs.rmSync(vDir(f.oid, vid), { recursive: true, force: true });
  vIndex.delete(vid);
  bump();
  return true;
}
const devicePath = vid => { const f = findVersion(vid); const p = f && path.join(vDir(f.oid, vid), 'device.mp4'); return p && fs.existsSync(p) ? p : null; };

function mediaPathO(oid, file) {
  if (!isId(oid)) return null;
  if (file === 'source') { const o = readOriginal(oid); return sourcePath(o); }
  if (!O_FILES.has(file) && !/^loop-\d\.mp4$/.test(file)) return null;
  const p = path.join(oDir(oid), file);
  return fs.existsSync(p) ? p : null;
}
function mediaPathV(vid, file) {
  if (!V_FILES.has(file)) return null;
  const f = findVersion(vid);
  if (!f) return null;
  const p = path.join(vDir(f.oid, vid), file);
  return fs.existsSync(p) ? p : null;
}

// The screen mask (notch + rounded corners) as it is actually applied (band widths are rounded to the 16 px grid), so equal keys = equal pixels.
// Notch is a global screen setting; rounded corners belong to each version's own settings (since 2026-10-11).
// Versions made while corners were global have "|corners:R" in their key; that part is ignored when comparing.
const notchKey = mask => notchOnlyKey(mask);
const baseKey = k => String(k || '').split('|')[0];
function notchOnlyKey(mask) {
  if (!mask || mask.mode === 'off') return 'off';
  if (mask.mode === 'band') { const px = bandPx(mask.band); return px ? `band:${mask.band.edge}:${px}` : 'off'; }
  const c = mask.circle;
  return c.r > 0 ? `circle:${c.cx}:${c.cy}:${c.r}:${c.feather}` : 'off';
}

/** User settings for a version, cleaned. */
const r3 = v => Math.round(v * 1000) / 1000;
const cleanCorners = c => ({ on: !!(c && c.on), r: Math.round(Math.min(300, Math.max(0, Number(c && c.r) || 0))) });
function cleanSettings(o, s, maxSeconds, cornersDefault) {
  s = s || {};
  const quality = s.quality === 'auto' || s.quality == null ? 'auto' : Math.min(100, Math.max(1, Math.round(Number(s.quality)) || 80));
  // Frame-accurate: times are kept to the millisecond (the UI snaps start to source frames, length to 1/18 s).
  let start = o.isImage ? 0 : r3(Math.max(0, Number(s.start) || 0));
  if (o.duration && start > o.duration - 0.05) start = r3(Math.max(0, o.duration - 0.05));
  let length = Number(s.length);
  const avail = o.isImage || !o.duration ? Infinity : o.duration - start;
  if (!(length > 0)) length = o.isImage ? 3 : Math.min(maxSeconds, avail);
  length = r3(Math.max(0.05, Math.min(length, avail, 120)));
  // Auto chooses the detail level itself (convert.chooseAuto); only Custom uses the slider.
  const detail = quality === 'auto' ? 0 : Math.round(Math.min(100, Math.max(0, Number(s.detail) || 0)));
  const blend = o.isImage ? 0 : r3(Math.min(1.5, Math.max(0, Number(s.blend) || 0)));
  return { start, length, quality, frame: normFrame(s.frame), corners: cleanCorners(s.corners || cornersDefault), detail, blend, manualLoop: !!s.manualLoop };
}
/** A version's settings with the fields older versions don't have filled in (corners came from the global mask then). */
function effectiveSettings(v) {
  const s = v.settings || {};
  return { ...s, corners: cleanCorners(s.corners || (v.notch && v.notch.corners)), detail: s.detail || 0, blend: s.blend || 0 };
}
const sameSettings = (a, b) => a.start === b.start && a.length === b.length && String(a.quality) === String(b.quality) &&
  JSON.stringify(normFrame(a.frame)) === JSON.stringify(normFrame(b.frame)) &&
  !!a.corners.on === !!b.corners.on && (!a.corners.on || a.corners.r === b.corners.r) && a.detail === b.detail && a.blend === b.blend;

/** Extra fields the UI needs, computed against the current notch, budget and screen record. */
function decorateVersion(v, ctx) {
  const r = v.result || {};
  const fps = r.fps || (v.settings && v.settings.fps);
  const speed = v.status === 'ready' && fps && fps !== DEVICE_FPS ? { fps, plays: fps < DEVICE_FPS ? 'fast' : 'slow' } : null;
  const onScreen = !!(ctx.standby && ctx.standby.versionId === v.id);
  return {
    ...v,
    settings: effectiveSettings(v),
    oldNotch: v.status === 'ready' && baseKey(v.notchKey) !== ctx.notchKey,
    speed,
    tooBig: v.status === 'ready' && r.size > ctx.budget,
    onScreen,
    onScreenIncomplete: onScreen && !!ctx.standby.incomplete,
  };
}
function context() {
  const s = settings.load();
  const { corners, ...notch } = s.mask;
  return { notchKey: notchKey(s.mask), budget: slots.budgets(s).loop, standby: s.slots.standby, maxSeconds: s.maxSeconds, mask: notch,
    cornersDefault: corners };
}

function summary(o, ctx = context()) {
  const vs = versions(o.id);
  return {
    id: o.id, name: o.name, fileName: o.file ? path.basename(o.file) : o.fileName, isImage: !!o.isImage,
    duration: o.duration, width: o.width, height: o.height, created: o.created,
    assets: o.assets || 'pending', assetsError: o.assetsError || null, missing: o.missing || null, sourceChanged: !!o.sourceChanged,
    versionCount: vs.length,
    busy: vs.some(v => v.status === 'queued' || v.status === 'converting'),
    failed: vs.some(v => v.status === 'error'),
    onScreen: vs.some(v => ctx.standby && ctx.standby.versionId === v.id),
    // for the Gallery: large still, hover preview, and whether any version should be re-made
    frameUrl: o.assets === 'ready' ? `/media/o/${o.id}/frame.jpg` : null,
    previewUrl: previewUrlOf(o),
    needsRemake: vs.some(v => { const d = decorateVersion(v, ctx); return d.oldNotch || !!d.speed; }),
  };
}
function previewUrlOf(o) {
  if (o.assets !== 'ready') return null;
  if (o.proxy) return `/media/o/${o.id}/${o.proxy}`;
  return sourcePath(o) ? `/media/o/${o.id}/source` : null;
}

function detail(oid) {
  const o = readOriginal(oid);
  if (!o) return null;
  const ctx = context();
  const src = sourcePath(o);
  return {
    ...summary(o, ctx), file: o.file, extracted: !!o.extracted, draft: o.draft || null, canTune: !!src && o.assets === 'ready',
    previewUrl: o.assets !== 'ready' ? null : o.proxy ? `/media/o/${o.id}/${o.proxy}` : src ? `/media/o/${o.id}/source` : null,
    previewKind: o.isImage ? 'image' : 'video', fps: o.fps || null,
    loops: o.loops && o.loops.key === String(o.fileMtime) ? o.loops : null,
    versions: versions(oid).map(v => decorateVersion(v, ctx)),
  };
}

// ---------- making versions ----------
function createVersion(oid, body) {
  const o = readOriginal(oid);
  if (!o) throw new Error('Unknown clip');
  if (!sourcePath(o)) throw new Error('The original file is missing, so a new version can\'t be made.');
  const ctx = context();
  const st = cleanSettings(o, body, ctx.maxSeconds, ctx.cornersDefault);
  const vs = versions(oid);
  const dup = vs.find(v => v.status !== 'error' && baseKey(v.notchKey) === ctx.notchKey && v.settings && sameSettings(effectiveSettings(v), st) &&
    (v.settings.fps || DEVICE_FPS) === DEVICE_FPS);
  if (dup && !body.force) { const e = new Error(`Already saved as v${dup.n}.`); e.code = 'duplicate'; e.versionId = dup.id; throw e; }
  // Remember the corner choice as the default for the next clip's tuning.
  const s = settings.load();
  if (JSON.stringify(s.mask.corners) !== JSON.stringify(st.corners)) settings.save({ ...s, mask: { ...s.mask, corners: st.corners } });
  const v = {
    id: newId(), originalId: oid, n: vs.reduce((m, x) => Math.max(m, x.n), 0) + 1, label: null, created: new Date().toISOString(),
    settings: { ...st, fps: DEVICE_FPS }, notch: ctx.mask, notchKey: ctx.notchKey, result: null, status: 'queued', error: null,
  };
  return writeVersion(oid, v);
}

async function runVersion(vid, onProgress) {
  const f = findVersion(vid);
  if (!f) throw new Error('This version was deleted.');
  const o = readOriginal(f.oid);
  const src = sourcePath(o);
  if (!src) { patchVersion(vid, { status: 'error', error: 'The original file is missing.' }); throw new Error('The original file is missing.'); }
  const s = settings.load();
  const budget = slots.budgets(s).loop;
  patchVersion(vid, { status: 'converting', error: null });
  try {
    const st = effectiveSettings(f.v);
    const r = await convert(src, vDir(f.oid, vid), {
      start: st.start, length: st.length, quality: st.quality, frame: st.frame, mask: f.v.notch, maxSeconds: s.maxSeconds,
      corners: st.corners, detail: st.detail, blend: st.blend,
      budgetBytes: budget, onProgress,
    });
    if (!findVersion(vid)) { throw new Error('This version was deleted.'); }
    return patchVersion(vid, {
      status: 'ready', error: null,
      result: { size: r.size, q: r.q, quality: r.quality, detail: r.detail || 0, fps: r.fps, start: r.start, length: r.length, tooBig: r.tooBig, budgetBytes: budget },
    });
  } catch (e) {
    if (findVersion(vid)) patchVersion(vid, { status: 'error', error: e.message, errorCode: e.code || null, maxFitLength: e.maxFitLength || null });
    throw e;
  }
}

/** Make a copy of an old version with the same trim/quality/framing, the current notch and the screen's frame rate. */
function remakeVersion(vid) {
  const f = findVersion(vid);
  if (!f) throw new Error('Unknown version');
  const { start, length, quality, frame, corners, detail, blend } = effectiveSettings(f.v);
  return createVersion(f.oid, { start, length, quality, frame, corners, detail, blend, force: true });
}

async function estimateFor(oid, body) {
  const o = readOriginal(oid);
  const src = sourcePath(o);
  if (!src) throw new Error('The original file is missing.');
  const ctx = context();
  const st = cleanSettings(o, body, ctx.maxSeconds, ctx.cornersDefault);
  const r = await estimate(src, oDir(oid), { ...st, mask: ctx.mask, maxSeconds: ctx.maxSeconds, budgetBytes: ctx.budget });
  return { ...r, quality: qualityFromQ(r.q), settings: st };
}

/** Search near the selection's end for the source frame most like its first frame (see convert.findLoopPoint). */
async function findLoopPoint(oid, body) {
  const o = readOriginal(oid);
  const src = sourcePath(o);
  if (!src) throw new Error('The original file is missing.');
  if (o.isImage) throw new Error('A still image always loops seamlessly.');
  const st = cleanSettings(o, body, 120);
  return findLoop(src, { start: st.start, end: st.start + st.length, duration: o.duration, fps: o.fps });
}

/**
 * Whole-clip loop search (src/loopsearch.js). maxLen defaults to the longest loop that fits the space on the screen at a
 * sensible quality (measured on a sample), within the clip.
 */
async function searchLoops(oid, body, onProgress) {
  const o = readOriginal(oid);
  const src = sourcePath(o);
  if (!src) throw new Error('The original file is missing.');
  if (o.isImage) throw new Error('A still image always loops seamlessly.');
  const ctx = context();
  const dur = o.duration || 0;
  const minLen = Math.max(0.5, Math.min(Number(body.minLen) || 3, dur - 0.2));
  let maxLen = Number(body.maxLen);
  let suggestedMax = null;
  if (!(maxLen > 0)) {
    // "Auto": the longest loop Best that fits can still make look good (quality 25 on the extended quantiser scale).
    onProgress && onProgress(0.005, 'Working out the longest loop that fits');
    const draft = o.draft || {};
    const bps = await sampleBytesPerSecond(src, oDir(oid), { start: draft.start || 0, length: Math.min(2, dur), frame: draft.frame,
      mask: ctx.mask, corners: draft.corners, budgetBytes: ctx.budget }, 25, 0);
    suggestedMax = Math.max(1, Math.floor((ctx.budget * 0.94 / bps) * 2) / 2);
    maxLen = suggestedMax;
  }
  maxLen = Math.max(minLen, Math.min(maxLen, dur - 0.2, 30));
  const r = await findLoops(src, oDir(oid), { duration: dur, fps: o.fps, minLen, maxLen, onProgress: (f, m) => onProgress && onProgress(f * 0.9, m) });
  // Blend automatically when the join is not seamless on its own; then a small preview of each loop, made the same way.
  onProgress && onProgress(0.92, 'Making the loop previews');
  for (let i = 0; i < r.candidates.length; i++) {
    const c = r.candidates[i];
    c.blend = autoBlend(c, o.fps);
    await renderLoopPreview(src, path.join(oDir(oid), `loop-${i}.mp4`), { start: c.start, length: c.length, blend: c.blend, srcFps: o.fps });
    c.preview = `/media/o/${oid}/loop-${i}.mp4?t=${Date.now()}`;
  }
  const result = { ...r, suggestedMax, minLen: body.minLen || null, maxLen: body.maxLen || null };
  patchOriginal(oid, { loops: { key: String(o.fileMtime), at: new Date().toISOString(), ...result } });
  return result;
}

/** Blend length for a loop candidate: none when the join already looks like a normal frame step (ratio <= 1). */
function autoBlend(c, srcFps) {
  if (!(c.ratio > 1.0)) return 0;
  const want = c.ratio <= 1.6 ? 0.33 : 0.5;
  const b = Math.min(want, c.start, c.length / 2);
  return Math.round(Math.floor(b * DEVICE_FPS + 1e-6) / DEVICE_FPS * 1000) / 1000;
}

/** At startup: versions that were queued or converting when the app closed did not finish. */
function resetInterrupted() {
  for (const o of listOriginals()) {
    for (const v of versions(o.id)) {
      if (v.status === 'queued' || v.status === 'converting') {
        writeVersion(o.id, { ...v, status: 'error', error: 'The app was closed before this version was finished.' });
      }
    }
    for (const n of fs.readdirSync(oDir(o.id))) if (/\.tmp(\.mp4)?$/.test(n)) fs.rmSync(path.join(oDir(o.id), n), { force: true });
  }
}

// ---------- migration from 1.1 (one conversion per item, meta.json) ----------
/**
 * Each old library/<id>/meta.json becomes an original (same id, now permanent). Its converted file becomes version 1
 * (settings from what was requested, result from the meta). Items never converted become originals without versions.
 * meta.json is kept as meta.v1.json. The "last sent" record is linked to the migrated version when name and size match.
 * Returns the number of items migrated.
 */
function migrate() {
  let ids = [];
  try { ids = fs.readdirSync(paths.library).filter(isId); } catch { return 0; }
  let n = 0;
  const s = settings.load();
  for (const id of ids) {
    const d = oDir(id);
    const metaF = path.join(d, 'meta.json');
    if (fs.existsSync(path.join(d, 'original.json')) || !fs.existsSync(metaF)) continue;
    const m = readJson(metaF);
    if (!m) continue;
    const o = {
      id, name: m.name || 'clip', file: m.file || null, fileName: m.source || null, extracted: !!m.extracted || !m.file,
      fileSize: m.fileSize || null, fileMtime: m.fileMtime || null, created: m.created || new Date().toISOString(),
      duration: m.sourceDuration ?? null, isImage: !!m.isImage, width: m.srcWidth || null, height: m.srcHeight || null,
      assets: 'pending', proxy: null, missing: null, sourceChanged: false,
      draft: m.requested ? { start: m.trim ? m.trim.start : m.requested.start || 0, length: m.trim ? m.trim.length : m.requested.length || 0,
        quality: m.requested.quality ?? 'auto', frame: { zoom: 1, x: 0.5, y: 0.5 } } : null,
    };
    const dev = path.join(d, 'device.mp4');
    if (fs.existsSync(dev) && m.size) {
      const vid = newId();
      const vd = vDir(id, vid);
      fs.mkdirSync(vd, { recursive: true });
      for (const f of ['device.mp4', 'preview.mp4', 'thumb.jpg']) if (fs.existsSync(path.join(d, f))) fs.renameSync(path.join(d, f), path.join(vd, f));
      const req = m.requested || {};
      const trim = m.trim || { start: req.start || 0, length: req.length || 0 };
      const v = {
        id: vid, originalId: id, n: 1, label: null, created: m.converted || o.created,
        settings: { start: trim.start, length: trim.length, quality: req.quality === 'auto' || req.quality == null ? 'auto' : req.quality,
          frame: { zoom: 1, x: 0.5, y: 0.5 }, fps: m.fps || req.fps || null },
        notch: m.mask || s.mask, notchKey: notchKey(m.mask || s.mask),
        result: { size: m.size, q: m.q, quality: m.quality, fps: m.fps, start: trim.start, length: trim.length, tooBig: !!m.tooBig, budgetBytes: m.budgetBytes },
        status: 'ready', error: null, migrated: true,
      };
      writeJson(path.join(vd, 'version.json'), v);
      const sb = s.slots.standby;
      if (sb && !sb.versionId && sb.name === o.name && sb.size === m.size) {
        settings.patchSlot('standby', { originalId: id, versionId: vid, label: 'v1' });
      }
    }
    writeJson(path.join(d, 'original.json'), o);
    fs.renameSync(metaF, path.join(d, 'meta.v1.json'));
    n++;
  }
  if (n) bump();
  return n;
}

module.exports = {
  isId, isSourceName, newId, oDir, vDir, rev, bump,
  readOriginal, writeOriginal, patchOriginal, listOriginals, sourcePath, removeOriginalCache, summary, detail, context,
  versions, findVersion, writeVersion, patchVersion, deleteVersion, devicePath, createVersion, runVersion, remakeVersion,
  estimateFor, findLoopPoint, searchLoops, resetInterrupted, migrate, mediaPathO, mediaPathV, notchKey, cleanSettings,
};
