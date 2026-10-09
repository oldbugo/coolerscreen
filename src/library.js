const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const paths = require('./paths');
const settings = require('./settings');
const { convert, estimate, qualityFromQ } = require('./convert');
const slots = require('./slots');

const ID_RE = /^[a-z0-9]{8,32}$/;
const FILES = new Set(['preview.mp4', 'thumb.jpg', 'frame.jpg', 'device.mp4', 'meta.json']);

const dirOf = id => path.join(paths.library, id);
const isId = id => typeof id === 'string' && ID_RE.test(id);

function newId() {
  return Date.now().toString(36) + crypto.randomBytes(3).toString('hex');
}

function readMeta(id) {
  try { return JSON.parse(fs.readFileSync(path.join(dirOf(id), 'meta.json'), 'utf8')); } catch { return null; }
}

function writeMeta(id, meta) {
  fs.writeFileSync(path.join(dirOf(id), 'meta.json'), JSON.stringify(meta, null, 2));
}

function list() {
  let ids = [];
  try { ids = fs.readdirSync(paths.library).filter(isId); } catch { return []; }
  const cur = JSON.stringify(settings.load().mask);
  return ids.map(readMeta).filter(Boolean)
    .map(m => ({ ...m, maskOutdated: JSON.stringify(m.mask) !== cur }))
    .sort((a, b) => (b.created || '').localeCompare(a.created || ''));
}

function get(id) {
  return isId(id) ? readMeta(id) : null;
}

function originalPath(id) {
  const d = dirOf(id);
  const f = fs.readdirSync(d).find(n => n.startsWith('original.'));
  return f ? path.join(d, f) : null;
}

function devicePath(id) {
  const p = path.join(dirOf(id), 'device.mp4');
  return fs.existsSync(p) ? p : null;
}

function mediaPath(id, file) {
  if (!isId(id) || !FILES.has(file)) return null;
  const p = path.join(dirOf(id), file);
  return fs.existsSync(p) ? p : null;
}

/** Prepare a library entry from a source file (copied into the library). Returns id. */
function addSource(srcFile, name, sourceName) {
  const id = newId();
  const d = dirOf(id);
  fs.mkdirSync(d, { recursive: true });
  const ext = (path.extname(srcFile) || '.bin').toLowerCase().replace(/[^.a-z0-9]/g, '');
  fs.copyFileSync(srcFile, path.join(d, 'original' + ext));
  const source = sourceName || path.basename(srcFile);
  writeMeta(id, {
    id, name: name || path.basename(source, path.extname(source)), source,
    created: new Date().toISOString(), status: 'new',
  });
  return id;
}

function patchMeta(id, patch) {
  const m = readMeta(id);
  if (!m) throw new Error("Unknown item");
  const n = { ...m, ...patch };
  writeMeta(id, n);
  return n;
}

// Merge requested options: explicit opts > this item's last request > global defaults.
function resolveOpts(meta, opts, s) {
  const r = meta.requested || {};
  const pick = (k, d) => (opts[k] !== undefined && opts[k] !== null && opts[k] !== "" ? opts[k] : r[k] !== undefined ? r[k] : d);
  const requested = {
    start: Number(pick("start", 0)) || 0,
    length: Number(pick("length", 0)) || 0,
    fps: Number(pick("fps", s.fps)) || s.fps,
    quality: pick("quality", s.quality),
    slot: ["loop", "startup", "standby", "both"].includes(pick("slot", "loop")) ? pick("slot", "loop") : "loop",
  };
  if (requested.quality !== "auto") requested.quality = Math.min(100, Math.max(1, Math.round(Number(requested.quality)) || 90));
  return requested;
}

async function estimateEntry(id, opts = {}) {
  const meta = readMeta(id);
  if (!meta) throw new Error("Unknown item");
  const s = settings.load();
  const orig = originalPath(id);
  if (!orig) throw new Error("Original file missing");
  const req = resolveOpts(meta, opts, s);
  const r = await estimate(orig, dirOf(id), {
    ...req, mask: s.mask, maxSeconds: s.maxSeconds, budgetBytes: slots.budgetFor(req.slot, s),
  });
  return { ...r, slot: req.slot, quality: qualityFromQ(r.q) };
}

/** Convert (or re-convert) an entry. opts: { start, length, fps, quality, slot } override stored values. */
async function convertEntry(id, opts = {}, onProgress) {
  const meta = readMeta(id);
  if (!meta) throw new Error("Unknown item");
  const s = settings.load();
  const orig = originalPath(id);
  if (!orig) throw new Error("Original file missing");
  const req = resolveOpts(meta, opts, s);
  const budget = slots.budgetFor(req.slot, s);
  patchMeta(id, { status: meta.status === "ready" ? "ready" : "converting" });
  try {
    const r = await convert(orig, dirOf(id), {
      ...req, mask: s.mask, maxSeconds: s.maxSeconds, budgetBytes: budget, onProgress,
    });
    const next = {
      ...readMeta(id), status: "ready", error: undefined, requested: req,
      trim: { start: r.start, length: r.length },
      fps: r.fps, q: r.q, quality: r.quality, manual: req.quality !== "auto",
      size: r.size, tooBig: r.tooBig, targetSlot: req.slot, budgetBytes: budget,
      sourceDuration: r.duration, mask: s.mask, converted: new Date().toISOString(),
    };
    writeMeta(id, next);
    return next;
  } catch (e) {
    patchMeta(id, { status: meta.status === "ready" ? "ready" : "new", error: e.message });
    throw e;
  }
}

function rename(id, name) {
  const m = get(id);
  if (!m) throw new Error('Unknown item');
  m.name = String(name || '').trim().slice(0, 80) || m.name;
  writeMeta(id, m);
  return m;
}

function remove(id) {
  if (!isId(id)) return;
  fs.rmSync(dirOf(id), { recursive: true, force: true });
}

module.exports = { patchMeta, estimateEntry, originalPath, list, get, addSource, convertEntry, rename, remove, mediaPath, devicePath, isId, dirOf };
