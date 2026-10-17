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
// guides
const g1 = L.effectiveGuides({ curve: { right: 50 }, zones: [{ id: 'z1', name: 'RAM', edge: 'right', size: 180 }] }, { curve: { right: -20 }, zones: { z1: 30 } });
assert.equal(g1.curve.right, 30); assert.equal(g1.zones[0].size, 210);
assert.equal(L.zoneHit({ x: .9, y: .2, w: .05, h: .2 }, g1), 'RAM'); assert.equal(L.zoneHit({ x: .1, y: .2, w: .05, h: .2 }, g1), null);
// snapping
const lines = L.snapLines([{ x: .5, y: .2, w: .2, h: .2 }], g1);
let s = L.snapMove({ x: .498, y: .5, w: .1, h: .1 }, lines, .003, .006); assert.ok(Math.abs(s.x - .5) < 1e-9, 'left edge to the other box / centre');
s = L.snapMove({ x: .3, y: .31, w: .1, h: .1 }, lines, .003, .006); assert.equal(s.x, .3);
let r = L.snapResize({ x: .1, y: .1, w: .1999, h: .1 }, 'e', L.snapLines([], { curve: {}, zones: [] }), [{ w: .2, h: .3 }], .003, .006); assert.ok(Math.abs(r.w - .2) < 1e-9, 'width matches another box');
console.log('layers ok');
