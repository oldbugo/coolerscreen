// Pure helpers for the theme editor (no DOM): layer order and groups, screen guides, snapping.
// `theme.widgets` is the drawing order (first = bottom, last = top); the Layers panel lists it top-first.
// Group members are contiguous in `widgets`; `theme.groups` = [{ id, name, hidden, collapsed }].

export const GW = 2400, GH = 1080;
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

/** Members contiguous (a group sits where its lowest member is), empty groups removed, unknown group ids dropped. */
export function normalizeGroups(t) {
  const known = new Set((t.groups || []).map(g => g.id));
  for (const w of t.widgets) if (w.group && !known.has(w.group)) w.group = null;
  const out = [], done = new Set();
  for (const w of t.widgets) {
    if (!w.group) out.push(w);
    else if (!done.has(w.group)) { done.add(w.group); out.push(...t.widgets.filter(x => x.group === w.group)); }
  }
  t.widgets = out;
  const used = new Set(out.map(w => w.group).filter(Boolean));
  t.groups = (t.groups || []).filter(g => used.has(g.id));
  return t;
}

export const membersOf = (t, gid) => t.widgets.filter(w => w.group === gid);

/** The rows of the Layers panel, top of the drawing order first: groups with their members (unless collapsed). */
export function layerRows(t) {
  const rows = [], seen = new Set();
  for (let i = t.widgets.length - 1; i >= 0; i--) {
    const w = t.widgets[i];
    if (!w.group) { rows.push({ kind: 'layer', w, depth: 0 }); continue; }
    if (seen.has(w.group)) continue;
    seen.add(w.group);
    const g = t.groups.find(x => x.id === w.group);
    if (!g) { rows.push({ kind: 'layer', w, depth: 0 }); continue; }
    const mem = membersOf(t, g.id).reverse();
    rows.push({ kind: 'group', g, members: mem });
    if (!g.collapsed) for (const m of mem) rows.push({ kind: 'layer', w: m, depth: 1 });
  }
  return rows;
}

/** Ids of the layers in panel order (top first); a collapsed group's members are still listed (they are selected with it). */
export const orderTopFirst = t => [...t.widgets].reverse().map(w => w.id);

/**
 * Move layers `ids` next to `target` ({ kind: 'layer', id } or { kind: 'group', id }); pos 'above' = drawn on top of it
 * (later in `widgets`), 'below' = under it. A layer dropped on a layer joins that layer's group; dropped next to a group
 * header it goes outside, or into the group's top when dropped on its lower half ('into'). A whole group moves as a block.
 */
export function moveLayers(t, ids, target, pos) {
  const idset = new Set(ids);
  const moving = t.widgets.filter(w => idset.has(w.id));
  if (!moving.length) return t;
  // a dragged group (every member of exactly one group is moving) keeps its membership
  const gids = [...new Set(moving.map(w => w.group))];
  const wholeGroup = gids.length === 1 && gids[0] && membersOf(t, gids[0]).every(w => idset.has(w.id));
  const rest = t.widgets.filter(w => !idset.has(w.id));
  let at, newGroup = null;
  const lastOf = g => Math.max(...rest.map((w, i) => (w.group === g ? i : -1)));
  const firstOf = g => rest.findIndex(w => w.group === g);
  if (target.kind === 'layer') {
    const tw = rest.find(w => w.id === target.id);
    if (!tw) return t;
    const idx = rest.indexOf(tw);
    if (wholeGroup && tw.group) { at = pos === 'above' ? lastOf(tw.group) + 1 : firstOf(tw.group); }
    else { at = pos === 'above' ? idx + 1 : idx; newGroup = tw.group || null; }
  } else {
    const g = target.id;
    if (!rest.some(w => w.group === g)) return t;
    if (pos === 'into' && !wholeGroup) { at = lastOf(g) + 1; newGroup = g; }
    else { at = pos === 'above' || pos === 'into' ? lastOf(g) + 1 : firstOf(g); }
  }
  if (!wholeGroup) for (const w of moving) w.group = newGroup;
  rest.splice(at, 0, ...moving);
  t.widgets = rest;
  return normalizeGroups(t);
}

/** Group the layers `ids` (contiguous at the place of the topmost one). Returns the new group's id. */
export function groupLayers(t, ids) {
  const idset = new Set(ids);
  const moving = t.widgets.filter(w => idset.has(w.id));
  if (!moving.length) return null;
  const used = new Set(t.groups.map(g => g.id));
  let n = 1; while (used.has('g' + n)) n++;
  const id = 'g' + n;
  let k = 1; const names = new Set(t.groups.map(g => g.name));
  while (names.has('Group ' + k)) k++;
  t.groups.push({ id, name: 'Group ' + k, hidden: false, collapsed: false });
  const topIdx = Math.max(...moving.map(w => t.widgets.indexOf(w)));
  const rest = t.widgets.filter(w => !idset.has(w.id));
  const before = t.widgets.slice(0, topIdx + 1).filter(w => !idset.has(w.id)).length;
  for (const w of moving) w.group = id;
  rest.splice(before, 0, ...moving);
  t.widgets = rest;
  normalizeGroups(t);
  return id;
}

/** Dissolve the groups of the given layers/groups. */
export function ungroupLayers(t, ids, groupIds = []) {
  const gs = new Set(groupIds);
  for (const w of t.widgets) if (ids.includes(w.id) && w.group) gs.add(w.group);
  for (const w of t.widgets) if (gs.has(w.group)) w.group = null;
  t.groups = t.groups.filter(g => !gs.has(g.id));
  return normalizeGroups(t);
}

/** One step towards the top (dir +1, "bring forward") or the bottom (-1). */
export function stepLayers(t, ids, dir) {
  const idset = new Set(ids);
  const ws = t.widgets;
  const order = dir > 0 ? [...ws.keys()].reverse() : [...ws.keys()];
  for (const i of order) {
    const w = ws[i];
    if (!idset.has(w.id)) continue;
    let j = i + dir;
    if (j < 0 || j >= ws.length) continue;
    const nb = ws[j];
    if (idset.has(nb.id)) continue; // a neighbour that moves too: the block moves as one
    if (nb.group !== w.group) {
      if (w.group) continue; // at the edge of its group
      // an ungrouped layer hops over a whole neighbouring group
      while (j + dir >= 0 && j + dir < ws.length && ws[j + dir].group === nb.group) j += dir;
    }
    ws.splice(i, 1); ws.splice(j, 0, w);
  }
  return normalizeGroups(t);
}

// ---------------------------------------------------------------------------- screen guides

/** Global guides + a theme's offsets, clamped like the engine does. */
export function effectiveGuides(g, off) {
  g = g || {}; off = off || {};
  const c = (g.curve || {}), o = (off.curve || {});
  const cv = (k, max) => clamp((Number(c[k]) || 0) + (Number(o[k]) || 0), 0, max);
  return {
    curve: { left: cv('left', GW / 2), right: cv('right', GW / 2), top: cv('top', GH / 2), bottom: cv('bottom', GH / 2) },
    zones: (g.zones || []).map(z => ({ ...z, size: clamp((Number(z.size) || 0) + (Number((off.zones || {})[z.id]) || 0), 0, z.edge === 'left' || z.edge === 'right' ? GW : GH) })),
  };
}

/** Normalised (0..1) rectangles of the avoid zones, and of the curve bands. */
export function bandRect(edge, px) {
  const fx = px / GW, fy = px / GH;
  if (edge === 'left') return { x0: 0, y0: 0, x1: fx, y1: 1 };
  if (edge === 'right') return { x0: 1 - fx, y0: 0, x1: 1, y1: 1 };
  if (edge === 'top') return { x0: 0, y0: 0, x1: 1, y1: fy };
  return { x0: 0, y0: 1 - fy, x1: 1, y1: 1 };
}
export const zoneRects = g => (g.zones || []).filter(z => z.size > 0).map(z => ({ id: z.id, name: z.name, ...bandRect(z.edge, z.size) }));
export const curveRects = g => ['left', 'right', 'top', 'bottom'].filter(e => g.curve[e] > 0).map(e => ({ edge: e, ...bandRect(e, g.curve[e]) }));

/** Does the box overlap an avoid zone? -> the zone's name (or null). */
export function zoneHit(w, g) {
  for (const z of zoneRects(g)) {
    if (w.x < z.x1 - 1e-6 && w.x + w.w > z.x0 + 1e-6 && w.y < z.y1 - 1e-6 && w.y + w.h > z.y0 + 1e-6) return z.name || 'an avoid area';
  }
  return null;
}

// ---------------------------------------------------------------------------- snapping

/** Vertical (x) and horizontal (y) snap lines in 0..1: canvas edges and centre, other boxes, guide-zone and curve edges. */
export function snapLines(others, guides) {
  const xs = [0, 0.5, 1], ys = [0, 0.5, 1];
  for (const b of others) { xs.push(b.x, b.x + b.w / 2, b.x + b.w); ys.push(b.y, b.y + b.h / 2, b.y + b.h); }
  for (const r of [...zoneRects(guides), ...curveRects(guides)]) { xs.push(r.x0, r.x1); ys.push(r.y0, r.y1); }
  return { xs, ys };
}

function nearest(cands, lines, thr) {
  let best = null;
  for (const c of cands) for (const l of lines) {
    const d = l - c.v;
    if (Math.abs(d) <= thr && (!best || Math.abs(d) < Math.abs(best.d))) best = { d, line: l, ...c };
  }
  return best;
}

/**
 * Snap a moving box (x, y, w, h): its left/centre/right and top/middle/bottom to the lines. Returns the corrected
 * { x, y } and the lines that were hit ({ vx: [...], hy: [...] }) for drawing.
 */
export function snapMove(b, L, thrX, thrY) {
  const sx = nearest([{ v: b.x }, { v: b.x + b.w / 2 }, { v: b.x + b.w }], L.xs, thrX);
  const sy = nearest([{ v: b.y }, { v: b.y + b.h / 2 }, { v: b.y + b.h }], L.ys, thrY);
  return { x: b.x + (sx ? sx.d : 0), y: b.y + (sy ? sy.d : 0), vx: sx ? [sx.line] : [], hy: sy ? [sy.line] : [] };
}

/**
 * Snap a resize: only the edges being dragged (`mode` letters n/s/e/w) go to lines; the new width/height also snaps to the
 * width/height of other boxes. `r` = { x, y, w, h } as computed by the pointer; returns the corrected rect + lines hit.
 */
export function snapResize(r, mode, L, others, thrX, thrY) {
  let { x, y, w, h } = r; const vx = [], hy = [];
  if (mode.includes('e')) { const s = nearest([{ v: x + w }], L.xs, thrX); if (s) { w += s.d; vx.push(s.line); } }
  if (mode.includes('w')) { const s = nearest([{ v: x }], L.xs, thrX); if (s) { x += s.d; w -= s.d; vx.push(s.line); } }
  if (mode.includes('s')) { const s = nearest([{ v: y + h }], L.ys, thrY); if (s) { h += s.d; hy.push(s.line); } }
  if (mode.includes('n')) { const s = nearest([{ v: y }], L.ys, thrY); if (s) { y += s.d; h -= s.d; hy.push(s.line); } }
  // matching sizes (only when no edge snapped on that axis)
  if ((mode.includes('e') || mode.includes('w')) && !vx.length) {
    const s = nearest([{ v: w }], others.map(o => o.w), thrX);
    if (s) { if (mode.includes('w')) x -= s.d; w += s.d; }
  }
  if ((mode.includes('n') || mode.includes('s')) && !hy.length) {
    const s = nearest([{ v: h }], others.map(o => o.h), thrY);
    if (s) { if (mode.includes('n')) y -= s.d; h += s.d; }
  }
  return { x, y, w, h, vx, hy };
}
