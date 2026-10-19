import assert from 'node:assert/strict';
import * as L from '../public/js/layers.js';
const mk = (ids, groups = []) => ({ widgets: ids.map(s => { const [id, g] = s.split(':'); return { id, group: g || null, x: 0, y: 0, w: .1, h: .1 }; }), groups: groups.map(id => ({ id, name: id, hidden: false, collapsed: false })) });
const ord = t => t.widgets.map(w => w.id + (w.group ? ':' + w.group : '')).join(' ');
// Run: node tools/layers.test.mjs
// normalize: contiguity + unknown + empty
let t = mk(['a:g1', 'b', 'c:g1', 'd:nope'], ['g1', 'g2']);
L.normalizeGroups(t); assert.equal(ord(t), 'a:g1 c:g1 b d'); assert.deepEqual(t.groups.map(g => g.id), ['g1']);
// rows: top first, groups with members
t = mk(['a', 'b:g1', 'c:g1', 'd'], ['g1']);
assert.deepEqual(L.layerRows(t).map(r => r.kind === 'group' ? 'G' + r.g.id : r.w.id), ['d', 'Gg1', 'c', 'b', 'a']);
t.groups[0].collapsed = true;
assert.deepEqual(L.layerRows(t).map(r => r.kind === 'group' ? 'G' + r.g.id : r.w.id), ['d', 'Gg1', 'a']);
// move: a dropped above d (on top of everything)
t = mk(['a', 'b', 'c', 'd']); L.moveLayers(t, ['a'], { kind: 'layer', id: 'd' }, 'above'); assert.equal(ord(t), 'b c d a');
t = mk(['a', 'b', 'c', 'd']); L.moveLayers(t, ['d'], { kind: 'layer', id: 'a' }, 'below'); assert.equal(ord(t), 'd a b c');
// join a group by dropping on a member
t = mk(['a', 'b:g1', 'c:g1', 'd'], ['g1']); L.moveLayers(t, ['d'], { kind: 'layer', id: 'b' }, 'above'); assert.equal(ord(t), 'a b:g1 d:g1 c:g1'.replace('d:g1 c:g1', 'd:g1 c:g1'));
// leave a group: drop next to a group header (above) or an outside layer
t = mk(['a', 'b:g1', 'c:g1', 'd'], ['g1']); L.moveLayers(t, ['c'], { kind: 'layer', id: 'd' }, 'below'); assert.equal(ord(t), 'a b:g1 c d');
t = mk(['a', 'b:g1', 'c:g1'], ['g1']); L.moveLayers(t, ['a'], { kind: 'group', id: 'g1' }, 'into'); assert.equal(ord(t), 'b:g1 c:g1 a:g1');
// group dragged as a block above an outside layer; emptied group disappears
t = mk(['a', 'b:g1', 'c:g1', 'd'], ['g1']); L.moveLayers(t, ['b', 'c'], { kind: 'layer', id: 'd' }, 'above'); assert.equal(ord(t), 'a d b:g1 c:g1');
t = mk(['a:g1', 'b'], ['g1']); L.moveLayers(t, ['a'], { kind: 'layer', id: 'b' }, 'above'); assert.equal(ord(t), 'b a:g1');
// group / ungroup
t = mk(['a', 'b', 'c', 'd']); const g = L.groupLayers(t, ['a', 'c']); assert.equal(g, 'g1'); assert.equal(ord(t), 'b a:g1 c:g1 d'); assert.equal(t.groups[0].name, 'Group 1');
L.ungroupLayers(t, ['a']); assert.equal(ord(t), 'b a c d'); assert.equal(t.groups.length, 0);
// step forward/back
t = mk(['a', 'b', 'c']); L.stepLayers(t, ['a'], 1); assert.equal(ord(t), 'b a c'); L.stepLayers(t, ['a'], -1); assert.equal(ord(t), 'a b c');
t = mk(['a', 'b:g1', 'c:g1', 'd'], ['g1']); L.stepLayers(t, ['a'], 1); assert.equal(ord(t), 'b:g1 c:g1 a d');
t = mk(['a:g1', 'b:g1', 'c'], ['g1']); L.stepLayers(t, ['b'], 1); assert.equal(ord(t), 'a:g1 b:g1 c', 'a member stops at the edge of its group');
t = mk(['a', 'b', 'c', 'd']); L.stepLayers(t, ['b', 'c'], 1); assert.equal(ord(t), 'a d b c'.replace('a d b c', 'a d b c'));
// guides (v1.4)
const g1 = L.effectiveGuides({ left: { size: 100 }, right: { size: 180 }, curve: { offset: 0, left: 50, right: 50 } }, { right: { size: 30 }, curve: { offset: 20, right: -20 } });
assert.equal(g1.right.size, 210); assert.equal(g1.left.size, 100);
assert.deepEqual(g1.curve, { offset: 20, left: 50, right: 30 });
const big = L.effectiveGuides({ curve: { offset: 1100, left: 5000, right: 5000 } }, null);
assert.ok(L.curveEdges(big)[0] >= 0 && L.curveEdges(big)[1] <= 2400, 'curve band stays inside the canvas');
assert.equal(L.effectiveGuides(null, null).right.size, 0);
assert.equal(L.bandHit({ x: .9, y: .2, w: .05, h: .2 }, g1), 'Right'); assert.equal(L.bandHit({ x: .02, y: .2, w: .02, h: .2 }, g1), 'Left');
assert.equal(L.bandHit({ x: .5, y: .2, w: .02, h: .2 }, g1), 'Curve'); assert.equal(L.bandHit({ x: .25, y: .2, w: .1, h: .2 }, g1), null);
assert.equal(L.bandRects(L.effectiveGuides(null, null)).length, 0, 'empty bands are not drawn');
// snapping
const lines = L.snapLines([{ x: .5, y: .2, w: .2, h: .2 }], g1);
let s = L.snapMove({ x: .498, y: .5, w: .1, h: .1 }, lines, .003, .006); assert.ok(Math.abs(s.x - .5) < 1e-9, 'left edge to the other box / centre');
s = L.snapMove({ x: .3, y: .31, w: .1, h: .1 }, lines, .003, .006); assert.equal(s.x, .3);
const none = L.effectiveGuides(null, null);
let r = L.snapResize({ x: .1, y: .1, w: .1999, h: .1 }, 'e', L.snapLines([], none), [{ w: .2, h: .3 }], .003, .006); assert.ok(Math.abs(r.w - .2) < 1e-9, 'width matches another box');
// gap (24 px on 2400x1080): inside the canvas edges, outside the bands, between boxes
const near = (a, b) => Math.abs(a - b) < 1e-9;
const gl = L.snapLines([{ x: .5, y: .5, w: .1, h: .1 }], g1, 24);
s = L.snapMove({ x: .0105, y: .5, w: .1, h: .1 }, L.snapLines([], none, 24), .003, .006); assert.ok(near(s.x, 24 / 2400), 'inside the left canvas edge');
s = L.snapMove({ x: 1 - .1 - .0105, y: .5, w: .1, h: .1 }, L.snapLines([], none, 24), .003, .006); assert.ok(near(s.x, 1 - .1 - 24 / 2400), 'inside the right canvas edge');
s = L.snapMove({ x: .3, y: .0235, w: .1, h: .1 }, L.snapLines([], none, 24), .003, .006); assert.ok(near(s.y, 24 / 1080), 'inside the top canvas edge');
s = L.snapMove({ x: .0105 - 0.0001 + 100 / 2400, y: .5, w: .1, h: .1 }, L.snapLines([], g1, 24), .003, .006); assert.ok(near(s.x, 100 / 2400 + 24 / 2400), 'outside the left band');
s = L.snapMove({ x: 1 - 210 / 2400 - 24 / 2400 - .1 + .001, y: .5, w: .1, h: .1 }, L.snapLines([], g1, 24), .003, .006); assert.ok(near(s.x + .1, 1 - 210 / 2400 - 24 / 2400), 'outside the right band (its right edge snaps, not its left)');
s = L.snapMove({ x: .5 + .1 + .0105, y: .2, w: .1, h: .1 }, L.snapLines([{ x: .5, y: .5, w: .1, h: .1 }], none, 24), .003, .006); assert.ok(near(s.x, .6 + 24 / 2400), 'gutter right of another box');
s = L.snapMove({ x: .3, y: .5 - .1 - .0235, w: .1, h: .1 }, L.snapLines([{ x: .3, y: .5, w: .1, h: .1 }], none, 24), .003, .006); assert.ok(near(s.y + .1, .5 - 24 / 1080), 'gutter above another box');
assert.ok(gl.xs.length > lines.xs.length, 'gap adds lines');
s = L.snapMove({ x: .015, y: .5, w: .1, h: .1 }, L.snapLines([], none, 24), .003, .006); assert.equal(s.x, .015, 'outside the threshold: no snap');
s = L.snapMove({ x: .0105, y: .5, w: .1, h: .1 }, L.snapLines([], none, 0), .003, .006); assert.equal(s.x, .0105, 'gap 0: no gap lines');
// a box's right edge does not snap to an inside-left gap line
s = L.snapMove({ x: 24 / 2400 - .1 + .001, y: .5, w: .1, h: .1 }, L.snapLines([], none, 24), .003, .006); assert.notEqual(s.x + .1, 24 / 2400);
// resize to the gap
r = L.snapResize({ x: .1, y: .1, w: 1 - .1 - 24 / 2400 - .001, h: .1 }, 'e', L.snapLines([], none, 24), [], .003, .006); assert.ok(near(r.x + r.w, 1 - 24 / 2400), 'right edge to the inside gap');
console.log('layers ok');
// 4.0 E2: the visible area (notch band + screen fit) decides gaps, snap lines and where new items go
import * as S from '../public/js/safearea.js';
const band = { mode: 'band', band: { edge: 'left', px: 96 } }; // bandPx rounds to the 16 px grid: 96
assert.deepEqual(S.insets({ mask: band }), { left: 96, right: 0, top: 0, bottom: 0 });
assert.deepEqual(S.insets({ mask: { mode: 'off' } }), { left: 0, right: 0, top: 0, bottom: 0 });
assert.deepEqual(S.insets(null), { left: 0, right: 0, top: 0, bottom: 0 });
// the screen fit hides 9 px at the left: the band's visible part is 87 panel px, scaled into the design canvas
assert.equal(S.insets({ mask: band, screenFit: { left: 9 } }).left, Math.round(87 * 2400 / 2391));
assert.equal(S.insets({ mask: { mode: 'band', band: { edge: 'right', px: 96 } } }).right, 96);
// safe rectangle with a 24 px gap: band + gap from the left, gap elsewhere
const sr = S.safeRect({ mask: band }, 24);
assert.ok(Math.abs(sr.x0 - 120 / 2400) < 1e-9 && Math.abs(sr.y0 - 24 / 1080) < 1e-9 && Math.abs(sr.x1 - (1 - 24 / 2400)) < 1e-9);
// placing: a box at x = 0 moves out from under the band; a preset box is shrunk when it does not fit
let pb = S.placeInside({ x: 0, y: 0, w: .2, h: .2 }, { mask: band }, 24);
assert.ok(Math.abs(pb.x - 0.05) < 1e-9 && Math.abs(pb.y - 24 / 1080) < 1e-9);
pb = S.placeInside({ x: .0, y: .0, w: 1, h: 1 }, { mask: band }, 24, { shrink: true });
assert.ok(pb.x + pb.w <= 1 - 24 / 2400 + 1e-9 && pb.x >= .05 - 1e-9);
// snap lines start at the band's edge, not the panel edge; the gap lines start after it
const noG = { left: { size: 0 }, right: { size: 0 }, curve: { offset: 0, left: 0, right: 0 } };
const L0 = L.snapLines([], noG, 24, null), L1 = L.snapLines([], noG, 24, { left: 96 });
assert.ok(L0.xs.some(l => l.v === 0) && L0.xs.some(l => Math.abs(l.v - 24 / 2400) < 1e-9 && l.k === 's'));
assert.ok(!L1.xs.some(l => l.v === 0), 'no line at the panel edge under the band');
assert.ok(L1.xs.some(l => Math.abs(l.v - 96 / 2400) < 1e-9 && l.k === 'a') && L1.xs.some(l => Math.abs(l.v - 120 / 2400) < 1e-9 && l.k === 's'));
assert.ok(L1.xs.some(l => Math.abs(l.v - (1 - 24 / 2400)) < 1e-9 && l.k === 'e'), 'the right side still has its gap line');
console.log('layers + safearea tests ok');
