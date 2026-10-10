const fs = require('fs');
const paths = require('./paths');

// Mask coordinates are in the LANDSCAPE 2400x1080 canvas (before the 90 degree rotation).
const DEFAULTS = {
  mask: {
    mode: 'band', // 'circle' | 'band' | 'off'
    circle: { cx: 45, cy: 540, r: 45, feather: 0 },
    band: { edge: 'left', px: 92 }, // left | right | top | bottom
    corners: { on: false, r: 80 }, // rounded corners: black outside a rounded rectangle of radius r (landscape px)
  },
  maxSeconds: 10,
  galleryDir: "", // empty = Documents\CoolerScreen\Gallery
  totalBudgetMB: 9.5, // shared by startup (intro) + standby (loop)
  uploadRate: null, // bytes per second measured on the last upload (for time estimates)
  // What this app last wrote to each slot: { name, size, at, originalId, versionId, label, splash, incomplete }
  slots: { startup: null, standby: null },
};

const num = (v, d, lo, hi) => {
  v = Number(v);
  if (!Number.isFinite(v)) return d;
  return Math.min(hi, Math.max(lo, v));
};
const str = (v, n = 80) => (typeof v === 'string' && v ? v.slice(0, n) : undefined);

function slot(v) {
  if (!v || !Number.isFinite(Number(v.size)) || v.size < 0) return null;
  const r = { name: String(v.name || "").slice(0, 80), size: Number(v.size), at: String(v.at || "") };
  for (const k of ['originalId', 'versionId', 'label', 'splash']) if (str(v[k])) r[k] = str(v[k]);
  if (v.incomplete) r.incomplete = true;
  return r;
}

function sanitize(s = {}) {
  const m = s.mask || {};
  const c = m.circle || {};
  const b = m.band || {};
  const k = m.corners || {};
  const D = DEFAULTS;
  return {
    mask: {
      mode: ['circle', 'band', 'off'].includes(m.mode) ? m.mode : D.mask.mode,
      circle: {
        cx: num(c.cx, D.mask.circle.cx, 0, 2400),
        cy: num(c.cy, D.mask.circle.cy, 0, 1080),
        r: num(c.r, D.mask.circle.r, 0, 600),
        feather: num(c.feather, D.mask.circle.feather, 0, 200),
      },
      band: {
        edge: ['left', 'right', 'top', 'bottom'].includes(b.edge) ? b.edge : D.mask.band.edge,
        px: num(b.px, D.mask.band.px, 0, 1000),
      },
      corners: { on: k.on === true, r: Math.round(num(k.r, D.mask.corners.r, 0, 300)) },
    },
    maxSeconds: num(s.maxSeconds, D.maxSeconds, 1, 120),
    galleryDir: typeof s.galleryDir === "string" ? s.galleryDir.trim().slice(0, 240) : "",
    totalBudgetMB: num(s.totalBudgetMB, D.totalBudgetMB, 1, 10),
    uploadRate: s.uploadRate == null ? null : num(s.uploadRate, null, 5000, 5e6),
    slots: { startup: slot(s.slots && s.slots.startup), standby: slot(s.slots && s.slots.standby) },
  };
}

function load() {
  try {
    return sanitize(JSON.parse(fs.readFileSync(paths.settings, 'utf8')));
  } catch {
    return sanitize({});
  }
}

function save(s) {
  const clean = sanitize(s);
  fs.mkdirSync(paths.base, { recursive: true });
  fs.writeFileSync(paths.settings, JSON.stringify(clean, null, 2));
  return clean;
}

function recordSlot(slotName, v) {
  const s = load();
  s.slots[slotName] = v ? { ...v, at: new Date().toISOString() } : null;
  return save(s);
}

/** Change fields of the slot record without touching the others (used by migration and deletes). */
function patchSlot(slotName, patch) {
  const s = load();
  if (!s.slots[slotName]) return s;
  s.slots[slotName] = { ...s.slots[slotName], ...patch };
  return save(s);
}

module.exports = { recordSlot, patchSlot, load, save, sanitize, DEFAULTS };
