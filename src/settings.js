const fs = require('fs');
const paths = require('./paths');

// Mask coordinates are in the LANDSCAPE 2400x1080 canvas (before the 90 degree rotation).
const DEFAULTS = {
  mask: {
    mode: 'band', // 'circle' | 'band' | 'off'
    circle: { cx: 45, cy: 540, r: 45, feather: 0 },
    band: { edge: 'left', px: 92 }, // left | right | top | bottom
  },
  fps: 20,
  maxSeconds: 10,
  quality: "auto", // "auto" or 1-100
  totalBudgetMB: 9.5, // shared by startup + standby
  slots: { startup: null, standby: null }, // last sent: { name, size, at }
};

const num = (v, d, lo, hi) => {
  v = Number(v);
  if (!Number.isFinite(v)) return d;
  return Math.min(hi, Math.max(lo, v));
};

function slot(v) {
  if (!v || !Number.isFinite(Number(v.size)) || v.size < 0) return null;
  return { name: String(v.name || "").slice(0, 80), size: Number(v.size), at: String(v.at || "") };
}

function sanitize(s = {}) {
  const m = s.mask || {};
  const c = m.circle || {};
  const b = m.band || {};
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
    },
    fps: Math.round(num(s.fps, D.fps, 5, 30)),
    maxSeconds: num(s.maxSeconds, D.maxSeconds, 1, 120),
    quality: s.quality === "auto" || s.quality == null ? "auto" : Math.round(num(s.quality, 90, 1, 100)),
    totalBudgetMB: num(s.totalBudgetMB, D.totalBudgetMB, 1, 10),
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
  s.slots[slotName] = v ? { name: v.name, size: v.size, at: new Date().toISOString() } : null;
  return save(s);
}

module.exports = { recordSlot, load, save, sanitize, DEFAULTS };
