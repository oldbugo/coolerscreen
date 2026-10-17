// 3.0 themes: the "On screen" theme editor and the Apply dialog (docs/v3/theme-format.md, v1.1 to v1.3).
//
// The editor works on one theme: the one whose base is in the screen's loop slot ("current theme"). It edits a working copy
// of theme.json: boxes on the preview (move, resize, gentle snapping), a palette to drop items from, a Photoshop-style
// layer list (drag to reorder, groups, eyes, rename), and a properties panel. Every edit is saved after a short pause
// (theme_set, which also updates the live screen at once) and the rendered preview is refreshed (theme_preview).
import { styleNotch, styleCorners, makeCornersOverlay } from './frame.js';
import { openDialog } from './dialog.js';
import * as Ly from './layers.js';
import * as Wd from './widgets.js';

const MIN_BOX = 0.02;
const SNAP_PX = 6; // weak snapping: about 6 px at the size the preview is drawn
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
const r4 = v => Math.round(v * 10000) / 10000;
const lsGet = k => { try { return localStorage.getItem(k); } catch { return null; } };
const lsSet = (k, v) => { try { localStorage.setItem(k, v); } catch { /* not remembered */ } };

const STAT_ITEMS = [['cpu', 'PC load'], ['ram', 'Memory'], ['gpuLoad', 'Graphics card load'], ['gpuTemp', 'Graphics card temperature'], ['gpuClock', 'Graphics card speed'], ['cpuTemp', 'Processor temperature']];
const END_CHOICES = [
  ['loopStart', 'Fade out onto the stored loop', 'The live layer fades away and the screen carries on from the first frame of its stored loop. Nothing jumps.'],
  ['instant', 'Stop at once', 'The live layer disappears and the stored loop plays.'],
  ['screenOff', 'Fade to black and turn the screen off', 'The screen goes dark when CoolerScreen stops.'],
];

const SVG_EYE = on => `<svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" d="M2 12s3.6-6.5 10-6.5S22 12 22 12s-3.6 6.5-10 6.5S2 12 2 12z"/>${on ? '<circle cx="12" cy="12" r="3" fill="currentColor"/>' : '<path stroke="currentColor" stroke-width="2" d="M4 20 20 4"/>'}</svg>`;
const SVG_CARET = open => `<svg viewBox="0 0 24 24" width="14" height="14" aria-hidden="true"><path fill="currentColor" d="${open ? 'M5 8l7 8 7-8z' : 'M8 5l8 7-8 7z'}"/></svg>`;
const SVG_FOLDER = '<svg viewBox="0 0 24 24" width="15" height="15" aria-hidden="true"><path fill="currentColor" d="M3 6a2 2 0 0 1 2-2h5l2 2h7a2 2 0 0 1 2 2v9a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>';

/** Hatched, tinted overlays for the screen guides inside a `position: relative` box (never drawn on the real screen). */
export function fillGuides(layer, g, { labels = true } = {}) {
  layer.textContent = '';
  if (!g) return;
  const pct = v => v * 100 + '%';
  const add = (cls, r, text) => {
    const d = document.createElement('div');
    d.className = 'gz ' + cls;
    Object.assign(d.style, { left: pct(r.x0), top: pct(r.y0), width: pct(r.x1 - r.x0), height: pct(r.y1 - r.y0) });
    if (labels && text) { const s = document.createElement('span'); s.className = 'gl'; s.textContent = text; d.append(s); }
    layer.append(d);
    return d;
  };
  for (const r of Ly.bandRects(g)) {
    const d = add(r.key === 'curve' ? 'curve' : 'zone ' + r.key, r, r.name);
    if (r.key === 'curve') {
      const m = document.createElement('i');
      m.className = 'gmid'; m.style.left = ((0.5 + g.curve.offset / Ly.GW - r.x0) / (r.x1 - r.x0)) * 100 + '%';
      d.append(m);
    }
  }
}
export { Ly as layers };

export function createThemes(ctx) {
  const { el, $, call, backend, S, toast } = ctx;
  const root = $('screenView');
  const E = {
    oid: null, info: null, theme: null, selIds: [], selGroup: null, anchor: null, rev: 0, saved: 0, saving: false, saveErr: null,
    liveUrl: null, baseUrl: null, baseSig: null, pvSeq: 0, previewOk: true, ui: null, saveT: 0, pvT: 0, loadSeq: 0, dragging: false,
    snap: true, guides: lsGet('cs.guides') !== '0', dragIds: [],
  };
  const currentId = () => S.currentTheme || null;
  const widgets = () => E.theme.widgets;
  const byId = id => widgets().find(w => w.id === id);
  const groupById = id => (E.theme.groups || []).find(g => g.id === id);
  const sel = () => E.selIds.map(byId).filter(Boolean);
  const radius = () => (E.theme.corners ?? (S.app && S.app.cornerRadius) ?? 0);
  const guidesNow = () => Ly.effectiveGuides(S.app && S.app.screenGuides, E.theme.guidesOffset);
  const hiddenNow = w => !!w.hidden || !!(w.group && (groupById(w.group) || {}).hidden);
  const shownOnStage = w => !hiddenNow(w) && (E.theme.liveLayer !== false || w.onBase);

  // ------------------------------------------------------------------ entering and leaving
  async function enter() {
    if (S.rev < 0) return; // the library has not been read yet
    const oid = currentId();
    if (!oid) {
      E.oid = null; E.ui = null;
      renderEmpty(S.originals.length ? 'Nothing has been applied to the screen yet.' : 'You have no themes yet.');
      return;
    }
    if (E.oid !== oid || !E.ui) await open(oid);
    else await syncFromServer();
  }

  async function open(oid) {
    const seq = ++E.loadSeq;
    let info;
    try { info = await call('theme_get', { originalId: oid }); } catch (e) { if (seq === E.loadSeq) renderEmpty(e.message); return; }
    if (seq !== E.loadSeq) return;
    Object.assign(E, { oid, info, theme: fixTheme(info.theme), selIds: [], selGroup: null, anchor: null, rev: 0, saved: 0, saveErr: null, liveUrl: null, baseUrl: null, baseSig: null, previewOk: true });
    E.snap = !S.app || S.app.editorSnap !== false;
    E.fitKey = JSON.stringify((S.app && S.app.screenFit) || {});
    buildEditor();
    paintAll();
    schedulePreview(0);
  }
  function fixTheme(t) {
    t.groups = t.groups || [];
    t.guidesOffset = t.guidesOffset || {};
    t.guidesOffset.left = t.guidesOffset.left || {}; t.guidesOffset.right = t.guidesOffset.right || {}; t.guidesOffset.curve = t.guidesOffset.curve || {};
    if (!Number.isFinite(t.gap)) t.gap = Ly.DEFAULT_GAP;
    if (t.liveLayer === undefined) t.liveLayer = true;
    return t;
  }

  async function syncFromServer() {
    let info;
    try { info = await call('theme_get', { originalId: E.oid }); } catch { return; }
    const dirty = E.rev !== E.saved || E.saving || E.dragging;
    const baseChanged = info.baseVersionId !== E.info.baseVersionId;
    const theirs = fixTheme(info.theme);
    const themeChanged = JSON.stringify(theirs) !== JSON.stringify(E.theme);
    E.info = info;
    if (!dirty && themeChanged) E.theme = theirs;
    if (E.ui) {
      paintTitle(); paintBanner();
      if (!dirty && themeChanged) { paintLayerList(); paintStage(); paintProps(); }
    }
    if (baseChanged || (!dirty && themeChanged)) { if (baseChanged) E.baseSig = null; schedulePreview(0); }
  }

  /** The app settings changed (guides, default radius): redraw what depends on them. */
  function appChanged() {
    if (!E.ui) return;
    // a new screen fit makes the stored base stale: read the editor's view again so the banner shows
    const fk = JSON.stringify((S.app && S.app.screenFit) || {});
    if (E.fitKey !== undefined && E.fitKey !== fk) syncFromServer();
    E.fitKey = fk;
    paintStage(); paintProps();
  }

  function renderEmpty(text) {
    root.textContent = '';
    root.append(el('div', { class: 'ed-empty' },
      el('h1', { id: 'screenTitle', text: 'Nothing on the screen yet' }),
      el('p', { class: 'muted', text: `${text} Pick a theme in My themes and apply it: it is stored on the screen, and this page then lets you design what CoolerScreen draws on top of it (clock, music, PC stats, text, pictures).` }),
      el('div', { class: 'row' },
        el('button', { class: 'btn primary big', text: 'Go to My themes', onclick: () => ctx.goHome('mine') }),
        el('button', { class: 'btn secondary big', text: 'Browse', onclick: () => ctx.goHome('browse') }))));
  }

  // ------------------------------------------------------------------ editor skeleton
  function buildEditor() {
    root.textContent = '';
    const u = (E.ui = {});
    u.palette = el('div', { class: 'palette', 'aria-label': 'Items you can add' });
    u.picker = el('div', { class: 'metric-pick', hidden: true });
    for (const [group, keys] of Wd.PALETTE) {
      const list = el('ul', { class: 'pal-grid' });
      for (const key of keys) {
        const k = Wd.ENTRIES[key];
        const b = el('button', { type: 'button', class: 'pal', draggable: true, title: k.pick ? 'Click to choose what it shows, or drag it onto the preview' : `Add ${k.name.toLowerCase()}: click, or drag it onto the preview`,
          'data-type': key, onclick: () => (k.pick ? togglePicker() : addWidget(key)) },
          el('span', { class: 'pi', 'aria-hidden': 'true', text: k.icon }), el('span', { class: 'pn', text: k.name }));
        b.addEventListener('dragstart', ev => { ev.dataTransfer.setData('text/x-cs-widget', key); ev.dataTransfer.effectAllowed = 'copy'; });
        list.append(el('li', {}, b));
        if (k.pick) list.append(el('li', { class: 'pal-wide' }, u.picker));
      }
      u.palette.append(el('h4', { class: 'pal-head', text: group }), list);
    }
    buildPicker();
    u.layers = el('div', { class: 'layers', role: 'listbox', 'aria-multiselectable': 'true', 'aria-label': 'Layers, top of the picture first' });
    u.layersNone = el('p', { class: 'muted small', text: 'Nothing yet: this theme shows its base only.' });
    u.btnGroup = el('button', { class: 'iconbtn', title: 'Group the selected layers (Ctrl+G)', 'aria-label': 'Group the selected layers', text: '▣', onclick: () => doGroup() });
    u.btnUngroup = el('button', { class: 'iconbtn', title: 'Ungroup (Ctrl+Shift+G)', 'aria-label': 'Ungroup', text: '▢', onclick: () => doUngroup() });
    u.btnDelete = el('button', { class: 'iconbtn', title: 'Delete the selected layers (Delete)', 'aria-label': 'Delete the selected layers', text: '✕', onclick: () => doDelete() });
    const left = el('aside', { class: 'ed-left', 'aria-label': 'Items' },
      el('h3', { text: 'Add' }), u.palette,
      el('div', { class: 'layers-head' }, el('h3', { text: 'Layers' }), el('span', { class: 'grow' }), u.btnGroup, u.btnUngroup, u.btnDelete),
      u.layers, u.layersNone);
    // centre
    u.title = el('h1', { id: 'screenTitle' });
    u.sub = el('span', { class: 'muted small' });
    u.saveState = el('span', { class: 'save-state', role: 'status', 'aria-live': 'polite' });
    u.banner = el('div');
    // the one "Live layer" switch, snapping and guides
    u.layerSw = el('input', { type: 'checkbox', id: 'liveLayerSw', role: 'switch', onchange: e => { E.theme.liveLayer = e.target.checked; E.baseSig = null; paintBar(); paintStage(); edited(); } });
    u.snapSw = el('input', { type: 'checkbox', id: 'snapSw', onchange: e => { E.snap = e.target.checked; } });
    u.guidesSw = el('input', { type: 'checkbox', id: 'guidesSw', onchange: e => { E.guides = e.target.checked; lsSet('cs.guides', E.guides ? '1' : '0'); paintStage(); } });
    u.bar = el('div', { class: 'ed-bar' },
      el('label', { class: 'sw-inline', for: 'liveLayerSw', title: 'On: the clock, music and other items are drawn on the screen. Off: the screen shows only its base.' }, u.layerSw, el('span', { class: 'switch-track', 'aria-hidden': 'true' }, el('span', { class: 'switch-knob' })), el('span', { text: 'Live layer' })),
      u.layerHint = el('span', { class: 'muted small', hidden: true, text: 'No live items yet. Add one from the left.' }),
      el('label', { class: 'check', title: 'Items snap gently to each other, the middle and the guides. Hold Alt to move freely.' }, u.snapSw, 'Snap'),
      el('label', { class: 'check', title: 'Show where the screen curves and the hard-to-see areas (calibrate them in Settings > Screen guides)' }, u.guidesSw, 'Guides'));
    u.stage = el('div', { class: 'stage', role: 'application', 'aria-label': 'Preview of the screen. Drag items to move them; use the handles to resize.' });
    u.imgBase = el('img', { class: 'layer', alt: '', draggable: false });
    u.imgLive = el('img', { class: 'layer', alt: '', draggable: false });
    u.corners = makeCornersOverlay();
    u.gz = el('div', { class: 'guides-layer' });
    u.notch = el('div', { class: 'notch', hidden: true, title: 'The camera hole. Keep items clear of it.' });
    u.note = el('div', { class: 'stage-note', hidden: true });
    u.boxes = el('div', { class: 'boxes' });
    u.snaps = el('div', { class: 'snaps' });
    u.stage.append(u.imgBase, u.imgLive, u.corners, u.gz, u.notch, u.boxes, u.snaps, u.note);
    u.stage.addEventListener('dragover', ev => { if ([...ev.dataTransfer.types].includes('text/x-cs-widget')) { ev.preventDefault(); ev.dataTransfer.dropEffect = 'copy'; u.stage.classList.add('drop-ok'); } });
    u.stage.addEventListener('dragleave', () => u.stage.classList.remove('drop-ok'));
    u.stage.addEventListener('drop', ev => {
      u.stage.classList.remove('drop-ok');
      const t = ev.dataTransfer.getData('text/x-cs-widget');
      if (!Wd.ENTRIES[t]) return;
      ev.preventDefault(); ev.stopPropagation();
      const r = u.stage.getBoundingClientRect();
      addWidget(t, (ev.clientX - r.left) / r.width, (ev.clientY - r.top) / r.height);
    });
    u.stage.addEventListener('pointerdown', ev => { if (ev.target === u.stage || ev.target.closest('.guides-layer') || ev.target === u.imgBase || ev.target === u.imgLive) setSel([]); });
    u.stageWrap = el('div', { class: 'stage-wrap' }, u.stage);
    u.baseBtn = el('button', { class: 'btn ghost', text: 'Base settings…', title: 'Choose the base picture: any clip, its loop, quality and versions', onclick: () => ctx.openBase(E.oid, E.info) });
    const center = el('section', { class: 'ed-center', 'aria-labelledby': 'screenTitle' },
      el('div', { class: 'ed-title' }, u.title, u.sub, el('span', { class: 'grow' }), u.saveState, u.baseBtn),
      u.banner, u.bar, u.stageWrap);
    u.props = el('aside', { class: 'ed-right', 'aria-label': 'Properties' });
    root.append(el('div', { class: 'ed' }, left, center, u.props));
  }

  function paintAll() {
    if (!E.ui) return;
    paintTitle(); paintBanner(); paintBar(); paintLayerList(); paintStage(); paintProps(); paintSave();
  }
  function paintTitle() {
    const u = E.ui;
    u.title.textContent = E.info.name;
    u.sub.textContent = 'is on the screen' + (S.app?.live?.settings?.inApp ? (E.theme.liveLayer === false ? ' · live layer is off' : widgets().some(w => !hiddenNow(w) && !w.onBase) ? ' · live is on' : ' · no live items') : '');
  }
  function paintSave() {
    const u = E.ui;
    u.saveState.textContent = E.saveErr ? 'Not saved' : E.rev !== E.saved || E.saving ? 'Saving…' : E.saved ? 'Saved' : '';
    u.saveState.title = E.saveErr || '';
    u.saveState.style.color = E.saveErr ? 'var(--danger)' : '';
  }
  function paintBar() {
    const u = E.ui;
    u.layerSw.checked = E.theme.liveLayer !== false;
    u.layerHint.hidden = !(E.theme.liveLayer !== false && !widgets().some(w => !hiddenNow(w) && !w.onBase));
    u.snapSw.checked = E.snap;
    u.guidesSw.checked = E.guides;
  }
  function paintBanner() {
    const u = E.ui;
    u.banner.textContent = '';
    const run = () => ctx.applyTheme(E.oid, E.info.name);
    if (E.info.baseUnstored) {
      const o = E.info.baseOwner;
      u.banner.append(el('div', { class: 'base-banner', role: 'status' },
        el('p', { text: `Base changed — store it on the screen. The new base${o ? ` (from “${o.name}”)` : ''} is already used by the live preview; the screen still holds the old one.` }),
        el('button', { class: 'btn primary', text: 'Store it on the screen', onclick: run })));
    } else if (E.info.baseStale) {
      u.banner.append(el('div', { class: 'base-banner', role: 'status' },
        el('p', { text: 'Base needs re-storing: the screen fit, corners, text or pictures changed since it was stored. Live already shows the change.' }),
        el('button', { class: 'btn primary', text: 'Store the base again', onclick: run })));
    }
  }

  // ------------------------------------------------------------------ the working theme
  const nameOf = Wd.nameOf;

  function edited({ preview = true } = {}) {
    E.rev++;
    clearTimeout(E.saveT);
    E.saveT = setTimeout(save, 450);
    paintSave();
    if (preview) schedulePreview(350);
  }

  async function save() {
    clearTimeout(E.saveT);
    if (E.saving) { E.saveT = setTimeout(save, 200); return; }
    if (E.rev === E.saved) return;
    const rev = E.rev, oid = E.oid;
    E.saving = true; E.saveErr = null; paintSave();
    try {
      const r = await call('theme_set', { originalId: oid, theme: E.theme });
      if (E.oid !== oid) return;
      E.info = { ...E.info, ...r, name: E.info.name };
      E.saved = rev;
      const theirs = fixTheme(r.theme);
      if (E.rev === rev && !E.dragging && JSON.stringify(theirs) !== JSON.stringify(E.theme)) { E.theme = theirs; E.repaint = true; }
    } catch (e) {
      E.saveErr = e.message;
      toast(`The theme could not be saved: ${e.message}`, true);
    }
    E.saving = false;
    if (E.ui && E.oid === oid) {
      paintSave(); paintBanner();
      if (E.rev !== E.saved && !E.saveErr) E.saveT = setTimeout(save, 100);
      else if (E.repaint && !E.dragging) { E.repaint = false; paintLayerList(); paintStage(); paintProps(); }
    }
  }

  // ------------------------------------------------------------------ preview images
  const baseSigOf = () => JSON.stringify([widgets().filter(w => w.onBase && !hiddenNow(w)), radius()]);
  function schedulePreview(ms) { clearTimeout(E.pvT); E.pvT = setTimeout(refreshPreview, ms); }
  async function refreshPreview() {
    if (!E.ui || !E.oid) return;
    const seq = ++E.pvSeq, oid = E.oid;
    const theme = JSON.parse(JSON.stringify(E.theme));
    const wantLive = E.theme.liveLayer !== false;
    const live = wantLive ? call('theme_preview', { originalId: oid, theme, withLive: true, w: 1200, h: 540 }) : Promise.resolve(null);
    const sig = baseSigOf();
    const needBase = E.baseSig !== sig || !E.baseUrl;
    const base = needBase ? call('theme_preview', { originalId: oid, theme, withLive: false, w: 1200, h: 540 }) : Promise.resolve(null);
    const [l, b] = await Promise.allSettled([live, base]);
    if (seq !== E.pvSeq || E.oid !== oid || !E.ui) return;
    E.previewOk = (l.status === 'fulfilled' && (!wantLive || l.value)) || (b.status === 'fulfilled' && !!b.value) || (!needBase && b.status === 'fulfilled');
    E.liveUrl = l.status === 'fulfilled' && l.value ? l.value.url : null;
    if (needBase) { E.baseUrl = b.status === 'fulfilled' && b.value ? b.value.url : null; E.baseSig = sig; }
    await Promise.all([setImg(E.ui.imgLive, E.liveUrl), setImg(E.ui.imgBase, E.baseUrl || fallbackFrame())]);
    paintImages();
  }
  const fallbackFrame = () => (S.originals.find(o => o.id === E.oid) || {}).frameUrl || null;
  function setImg(img, url) {
    return new Promise(res => {
      if (!url) { img.removeAttribute('src'); delete img.dataset.url; img.hidden = true; return res(); }
      if (img.dataset.url === url) { img.hidden = false; return res(); }
      const pre = new Image();
      pre.onload = pre.onerror = () => { img.src = url; img.dataset.url = url; img.hidden = false; res(); };
      pre.src = url;
    });
  }

  // ------------------------------------------------------------------ the stage: boxes, guides, corners
  function paintStage() {
    const u = E.ui;
    if (!u) return;
    styleNotch(u.notch, S.app && S.app.mask);
    styleCorners(u.corners, { on: radius() > 0, r: radius() }, S.app && S.app.mask);
    fillGuides(u.gz, E.guides ? guidesNow() : null);
    u.boxes.textContent = '';
    const g = guidesNow();
    for (const w of widgets()) if (shownOnStage(w)) u.boxes.append(makeBox(w, g));
    paintImages();
  }
  function paintImages() {
    const u = E.ui;
    if (!u) return;
    const on = E.theme.liveLayer !== false;
    u.imgLive.style.display = on && E.liveUrl ? '' : 'none';
    u.imgBase.style.display = '';
    const any = widgets().some(shownOnStage);
    if (!E.previewOk) { u.note.hidden = false; u.note.textContent = 'The rendered preview is not available yet; showing the base picture and the item boxes.'; }
    else if (!on) { u.note.hidden = false; u.note.textContent = 'Live layer is off: the screen shows only its base' + (any ? ' (and the text and pictures marked as part of it).' : '.'); }
    else u.note.hidden = true;
  }
  function placeBox(b, w) {
    b.style.left = w.x * 100 + '%'; b.style.top = w.y * 100 + '%'; b.style.width = w.w * 100 + '%'; b.style.height = w.h * 100 + '%';
    b.classList.toggle('top', w.y < 0.06);
  }
  function makeBox(w, g) {
    const on = E.selIds.includes(w.id), single = on && E.selIds.length === 1;
    const hit = Ly.bandHit(w, g);
    const b = el('div', { class: 'wbox' + (on ? ' sel' : '') + (single ? ' one' : '') + (hit && E.guides ? ' warn' : ''), tabindex: '0', role: 'button', 'data-id': w.id, 'aria-label': `${nameOf(w)}${hit ? `. Overlaps ${hit} guide` : ''}. Arrow keys move it, Delete removes it.` },
      el('span', { class: 'wlabel', text: nameOf(w) }), el('span', { class: 'wname', text: on ? '' : nameOf(w) }),
      hit && E.guides ? el('span', { class: 'wwarn', title: `Overlaps ${hit}`, text: '⚠' }) : null);
    placeBox(b, w);
    for (const h of ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']) b.append(el('span', { class: 'wh', 'data-h': h }));
    b.addEventListener('pointerdown', ev => {
      if (ev.button !== 0) return;
      const h = ev.target.dataset && ev.target.dataset.h;
      if (ev.ctrlKey || ev.metaKey || ev.shiftKey) setSel(E.selIds.includes(w.id) ? E.selIds.filter(i => i !== w.id) : [...E.selIds, w.id], { keepFocus: true });
      else if (!E.selIds.includes(w.id)) setSel([w.id], { keepFocus: true });
      startDrag(ev, w, h || 'move');
    });
    b.addEventListener('focus', () => { if (!E.selIds.includes(w.id)) setSel([w.id], { keepFocus: true }); });
    return b;
  }
  const boxEl = id => E.ui.boxes.querySelector(`.wbox[data-id="${CSS.escape(id)}"]`);

  function drawSnapLines(vx, hy) {
    const s = E.ui.snaps;
    s.textContent = '';
    for (const x of vx) s.append(el('div', { class: 'snapline v', style: `left:${x * 100}%` }));
    for (const y of hy) s.append(el('div', { class: 'snapline h', style: `top:${y * 100}%` }));
  }

  function startDrag(ev, w, mode) {
    ev.preventDefault();
    const sr = E.ui.stage.getBoundingClientRect();
    const moving = mode === 'move' ? (E.selIds.includes(w.id) ? sel() : [w]) : [w];
    const orig = moving.map(m => ({ m, x: m.x, y: m.y, w: m.w, h: m.h }));
    const ids = new Set(moving.map(m => m.id));
    const others = widgets().filter(o => !ids.has(o.id) && shownOnStage(o));
    const lines = Ly.snapLines(others, guidesNow(), E.theme.gap ?? Ly.DEFAULT_GAP);
    const thrX = SNAP_PX / sr.width, thrY = SNAP_PX / sr.height;
    const ux0 = Math.min(...orig.map(o => o.x)), uy0 = Math.min(...orig.map(o => o.y));
    const ux1 = Math.max(...orig.map(o => o.x + o.w)), uy1 = Math.max(...orig.map(o => o.y + o.h));
    const sx = ev.clientX, sy = ev.clientY;
    const live = boxEl(w.id);
    if (live) { live.focus({ preventScroll: true }); try { live.setPointerCapture(ev.pointerId); } catch { /* gone */ } }
    const target = live || E.ui.stage;
    let moved = false;
    E.dragging = true;
    const move = e => {
      const dx = (e.clientX - sx) / sr.width, dy = (e.clientY - sy) / sr.height;
      const snapOn = E.snap && !e.altKey;
      let vx = [], hy = [];
      if (mode === 'move') {
        let ddx = clamp(dx, -ux0, 1 - ux1), ddy = clamp(dy, -uy0, 1 - uy1);
        if (snapOn) {
          const s = Ly.snapMove({ x: ux0 + ddx, y: uy0 + ddy, w: ux1 - ux0, h: uy1 - uy0 }, lines, thrX, thrY);
          ddx = clamp(s.x - ux0, -ux0, 1 - ux1); ddy = clamp(s.y - uy0, -uy0, 1 - uy1); vx = s.vx; hy = s.hy;
        }
        for (const o of orig) { o.m.x = r4(o.x + ddx); o.m.y = r4(o.y + ddy); const b = boxEl(o.m.id); if (b) placeBox(b, o.m); }
      } else {
        const o = orig[0];
        let x = o.x, y = o.y, ww = o.w, hh = o.h;
        if (mode.includes('e')) ww = clamp(o.w + dx, MIN_BOX, 1 - o.x);
        if (mode.includes('s')) hh = clamp(o.h + dy, MIN_BOX, 1 - o.y);
        if (mode.includes('w')) { const nx = clamp(o.x + dx, 0, o.x + o.w - MIN_BOX); ww = o.x + o.w - nx; x = nx; }
        if (mode.includes('n')) { const ny = clamp(o.y + dy, 0, o.y + o.h - MIN_BOX); hh = o.y + o.h - ny; y = ny; }
        if (snapOn) {
          const s = Ly.snapResize({ x, y, w: ww, h: hh }, mode, lines, others, thrX, thrY);
          x = clamp(s.x, 0, 1); y = clamp(s.y, 0, 1); ww = clamp(s.w, MIN_BOX, 1 - x); hh = clamp(s.h, MIN_BOX, 1 - y); vx = s.vx; hy = s.hy;
        }
        Object.assign(o.m, { x: r4(x), y: r4(y), w: r4(ww), h: r4(hh) });
        const b = boxEl(o.m.id); if (b) placeBox(b, o.m);
      }
      moved = true;
      drawSnapLines(vx, hy);
      if (moving.length === 1) syncPosFields(moving[0]);
    };
    const up = () => {
      target.removeEventListener('pointermove', move);
      target.removeEventListener('pointerup', up);
      target.removeEventListener('pointercancel', up);
      E.dragging = false;
      drawSnapLines([], []);
      if (moved) { refreshWarnings(); edited(); }
    };
    target.addEventListener('pointermove', move);
    target.addEventListener('pointerup', up);
    target.addEventListener('pointercancel', up);
  }
  /** Badges for boxes that overlap a hard-to-see area (kept current while items move). */
  function refreshWarnings() {
    const g = guidesNow();
    for (const w of widgets()) {
      const b = E.ui.boxes.querySelector(`.wbox[data-id="${CSS.escape(w.id)}"]`);
      if (!b) continue;
      const hit = E.guides ? Ly.bandHit(w, g) : null;
      b.classList.toggle('warn', !!hit);
      let badge = b.querySelector('.wwarn');
      if (hit && !badge) { badge = el('span', { class: 'wwarn', text: '⚠' }); b.append(badge); }
      if (badge) { if (hit) badge.title = `Overlaps ${hit}`; else badge.remove(); }
    }
    paintLayerList();
    // the properties note about an overlapped area follows the selected item
    const one = E.selIds.length === 1 ? byId(E.selIds[0]) : null;
    const now = one && E.guides ? !!Ly.bandHit(one, g) : false;
    if (one && now !== E.hitShown) { E.hitShown = now; paintProps(); }
  }

  // Shortcuts work whenever the On screen page is showing and nothing modal is open (focus may be on the page or a row).
  document.addEventListener('keydown', ev => {
    if (root.hidden || !E.ui || ev.defaultPrevented) return;
    if (document.querySelector('.backdrop') || !$('drawerBack').hidden || !$('applyBack').hidden) return;
    onKeys(ev);
  });
  function onKeys(ev) {
    if (ev.target.closest('input, textarea, select')) return;
    const k = ev.key.toLowerCase();
    const mod = ev.ctrlKey || ev.metaKey;
    if (mod && k === 'g') { ev.preventDefault(); if (ev.shiftKey) doUngroup(); else doGroup(); return; }
    if (mod && (ev.key === ']' || ev.key === '[')) { ev.preventDefault(); stepSel(ev.key === ']' ? 1 : -1); return; }
    if (mod && k === 'a') { ev.preventDefault(); setSel(widgets().map(w => w.id)); return; }
    if (ev.key === 'Escape' && E.selIds.length) { setSel([]); return; }
    if (!E.selIds.length) return;
    const step = ev.shiftKey ? 0.02 : 0.005;
    const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[ev.key];
    if (d) {
      ev.preventDefault();
      const ms = sel();
      const x0 = Math.min(...ms.map(m => m.x)), y0 = Math.min(...ms.map(m => m.y)), x1 = Math.max(...ms.map(m => m.x + m.w)), y1 = Math.max(...ms.map(m => m.y + m.h));
      const dx = clamp(d[0], -x0, 1 - x1), dy = clamp(d[1], -y0, 1 - y1);
      for (const m of ms) { m.x = r4(m.x + dx); m.y = r4(m.y + dy); const b = boxEl(m.id); if (b) placeBox(b, m); }
      if (ms.length === 1) syncPosFields(ms[0]);
      refreshWarnings(); edited();
    } else if (ev.key === 'Delete' || (ev.key === 'Backspace' && !ev.target.closest('button'))) {
      ev.preventDefault(); doDelete();
    }
  }

  // ------------------------------------------------------------------ adding, selecting, ordering
  function nextId() {
    const used = new Set(widgets().map(w => w.id));
    for (let i = 1; ; i++) if (!used.has('w' + i)) return 'w' + i;
  }
  /** Sensor: the metric list opens under the palette item. */
  function buildPicker() {
    const u = E.ui;
    u.picker.textContent = '';
    let last = '';
    for (const [id, group, name] of Wd.METRICS) {
      if (group !== last) { u.picker.append(el('h5', { class: 'pal-head', text: group })); last = group; }
      u.picker.append(el('button', { type: 'button', class: 'pick', text: name, onclick: () => { u.picker.hidden = true; addWidget('sensor', null, null, { metric: id, style: id === 'cpu.core' || id === 'disk.used' ? 'bar' : id.startsWith('net.') ? 'graph' : 'number' }); } }));
    }
  }
  function togglePicker() { E.ui.picker.hidden = !E.ui.picker.hidden; }
  function addWidget(type, cx, cy, extra) {
    const k = Wd.ENTRIES[type];
    if (!k || !E.ui) return;
    Wd.setApp(S.app);
    if (widgets().length >= 40) { toast('A theme can hold at most 40 items.', true); return; }
    const n = widgets().length;
    const props = { ...k.props(), ...(extra || {}) };
    const [w, h] = Wd.defaultSize(k.type, props);
    let x, y;
    if (cx != null) { x = cx - w / 2; y = cy - h / 2; } else { x = 0.1 + 0.04 * (n % 8); y = 0.12 + 0.05 * (n % 8); }
    const wd = { id: nextId(), type: k.type, x: r4(clamp(x, 0, 1 - w)), y: r4(clamp(y, 0, 1 - h)), w, h, onBase: false, hidden: false, group: null, props };
    widgets().push(wd);
    E.theme.liveLayer = true; // adding an item shows the live layer again
    E.selIds = [wd.id]; E.selGroup = null; E.anchor = wd.id;
    paintBar(); paintLayerList(); paintStage(); paintProps();
    edited();
    const b = boxEl(wd.id);
    if (b) b.focus({ preventScroll: true });
  }
  function doDelete() {
    const ids = new Set(E.selIds);
    if (!ids.size) return;
    E.theme.widgets = widgets().filter(w => !ids.has(w.id));
    Ly.normalizeGroups(E.theme);
    E.selIds = []; E.selGroup = null;
    paintLayerList(); paintStage(); paintProps();
    edited();
  }
  function doGroup() {
    if (!E.selIds.length) return;
    const gid = Ly.groupLayers(E.theme, E.selIds);
    if (!gid) return;
    E.selGroup = gid; E.selIds = Ly.membersOf(E.theme, gid).map(w => w.id);
    paintLayerList(); paintStage(); paintProps();
    edited();
  }
  function doUngroup() {
    const gids = E.selGroup ? [E.selGroup] : [];
    if (!gids.length && !sel().some(w => w.group)) return;
    Ly.ungroupLayers(E.theme, E.selIds, gids);
    E.selGroup = null;
    paintLayerList(); paintStage(); paintProps();
    edited();
  }
  function stepSel(dir) {
    if (!E.selIds.length) return;
    Ly.stepLayers(E.theme, E.selIds, dir);
    paintLayerList(); paintStage();
    edited();
  }
  function setSel(ids, { keepFocus = false, group = null } = {}) {
    ids = ids.filter(id => byId(id));
    const same = ids.length === E.selIds.length && ids.every((id, i) => id === E.selIds[i]) && group === E.selGroup;
    if (same) return;
    E.selIds = ids; E.selGroup = group;
    if (ids.length) E.anchor = ids[ids.length - 1];
    paintLayerList(); paintProps();
    for (const b of E.ui.boxes.children) {
      const on = ids.includes(b.dataset.id);
      b.classList.toggle('sel', on);
      b.classList.toggle('one', on && ids.length === 1);
      const nm = b.querySelector('.wname');
      if (nm) nm.textContent = on ? '' : nameOf(byId(b.dataset.id) || {});
    }
    if (!keepFocus && ids.length === 1) { const b = boxEl(ids[0]); if (b) b.focus({ preventScroll: true }); }
  }

  // ------------------------------------------------------------------ the layer list
  function rename(row, current, commit) {
    const nameEl = row.querySelector('.lname');
    const input = el('input', { type: 'text', class: 'lrename', value: current, maxLength: 60, 'aria-label': 'Name' });
    input.value = current;
    let done = false;
    const finish = save => { if (done) return; done = true; if (save) commit(input.value.trim()); else paintLayerList(); };
    input.onkeydown = e => { e.stopPropagation(); if (e.key === 'Enter') finish(true); else if (e.key === 'Escape') finish(false); };
    input.onblur = () => finish(true);
    input.onclick = e => e.stopPropagation();
    nameEl.replaceWith(input);
    input.focus(); input.select();
  }

  function dropPos(row, ev, isGroup) {
    const r = row.getBoundingClientRect(), f = (ev.clientY - r.top) / r.height;
    if (isGroup) return f < 0.4 ? 'above' : 'into';
    return f < 0.5 ? 'above' : 'below';
  }
  function clearDropMarks() { for (const r of E.ui.layers.querySelectorAll('.drop-above,.drop-below,.drop-into')) r.classList.remove('drop-above', 'drop-below', 'drop-into'); }

  function paintLayerList() {
    const u = E.ui;
    if (!u) return;
    const keep = u.layers.scrollTop;
    u.layers.textContent = '';
    const rows = Ly.layerRows(E.theme);
    u.layersNone.hidden = rows.length > 0;
    const g = guidesNow();
    const orderIds = Ly.orderTopFirst(E.theme);
    for (const r of rows) {
      const isGroup = r.kind === 'group';
      const ids = isGroup ? r.members.map(m => m.id) : [r.w.id];
      const selected = isGroup ? (E.selGroup === r.g.id) : E.selIds.includes(r.w.id);
      const hidden = isGroup ? !!r.g.hidden : hiddenNow(r.w);
      const eye = el('button', { class: 'leye', type: 'button', 'aria-label': hidden ? 'Show' : 'Hide', title: hidden ? 'Show' : 'Hide', 'aria-pressed': String(!hidden) });
      eye.innerHTML = SVG_EYE(!hidden);
      const name = isGroup ? (r.g.name || 'Group') : nameOf(r.w);
      const hit = !isGroup && Ly.bandHit(r.w, g);
      const row = el('div', { class: 'lrow' + (isGroup ? ' group' : '') + (selected ? ' sel' : '') + (hidden ? ' off' : '') + (r.depth ? ' in' : ''), role: 'option', draggable: 'true', 'aria-selected': String(selected), 'data-id': isGroup ? r.g.id : r.w.id, 'data-kind': r.kind });
      if (isGroup) {
        const caret = el('button', { class: 'lcaret', type: 'button', 'aria-label': r.g.collapsed ? 'Expand group' : 'Collapse group', 'aria-expanded': String(!r.g.collapsed) });
        caret.innerHTML = SVG_CARET(!r.g.collapsed);
        caret.onclick = e => { e.stopPropagation(); r.g.collapsed = !r.g.collapsed; paintLayerList(); edited({ preview: false }); };
        const fold = el('span', { class: 'licon' }); fold.innerHTML = SVG_FOLDER;
        row.append(caret, eye, fold, el('span', { class: 'lname', text: name, title: name }), el('span', { class: 'lcount small muted', text: String(r.members.length) }));
      } else {
        row.append(...[el('span', { class: 'lindent' }), eye, el('span', { class: 'licon', 'aria-hidden': 'true', text: Wd.typeIcon(r.w.type) }), el('span', { class: 'lname', text: name, title: name }),
          hit && E.guides ? el('span', { class: 'lwarn', title: `Overlaps ${hit}`, text: '⚠' }) : null].filter(Boolean));
      }
      eye.onclick = e => {
        e.stopPropagation();
        if (isGroup) r.g.hidden = !r.g.hidden; else r.w.hidden = !r.w.hidden;
        E.baseSig = null; paintLayerList(); paintStage(); edited();
      };
      row.addEventListener('click', e => {
        if (e.target.closest('.leye, .lcaret, .lrename')) return;
        if (isGroup) { setSel(ids, { group: r.g.id }); return; }
        if (e.shiftKey && E.anchor) {
          const a = orderIds.indexOf(E.anchor), b = orderIds.indexOf(r.w.id);
          if (a >= 0 && b >= 0) { const [lo, hi] = a < b ? [a, b] : [b, a]; setSel(orderIds.slice(lo, hi + 1), { keepFocus: true }); return; }
        }
        if (e.ctrlKey || e.metaKey) { setSel(E.selIds.includes(r.w.id) ? E.selIds.filter(i => i !== r.w.id) : [...E.selIds, r.w.id], { keepFocus: true }); return; }
        setSel([r.w.id], { keepFocus: true });
      });
      row.addEventListener('dblclick', e => {
        if (e.target.closest('.leye, .lcaret')) return;
        e.preventDefault();
        if (isGroup) rename(row, r.g.name, v => { r.g.name = v || 'Group'; paintLayerList(); paintProps(); edited({ preview: false }); });
        else rename(row, r.w.props.name || '', v => { if (v) r.w.props.name = v; else delete r.w.props.name; paintLayerList(); refreshBoxLabel(r.w); paintProps(); edited({ preview: false }); });
      });
      row.addEventListener('dragstart', e => {
        // (the list is not redrawn here: replacing the dragged row would cancel the drag)
        const mine = isGroup ? ids : (E.selIds.includes(r.w.id) ? [...E.selIds] : [r.w.id]);
        E.dragIds = mine;
        e.dataTransfer.setData('text/x-cs-layer', mine.join(','));
        e.dataTransfer.effectAllowed = 'move';
      });
      row.addEventListener('dragend', () => { E.dragIds = []; clearDropMarks(); });
      row.addEventListener('dragover', e => {
        if (!E.dragIds.length || E.dragIds.includes(isGroup ? '' : r.w.id)) return;
        e.preventDefault(); e.dataTransfer.dropEffect = 'move';
        clearDropMarks();
        row.classList.add('drop-' + dropPos(row, e, isGroup));
      });
      row.addEventListener('dragleave', () => row.classList.remove('drop-above', 'drop-below', 'drop-into'));
      row.addEventListener('drop', e => {
        if (!E.dragIds.length) return;
        e.preventDefault();
        const pos = dropPos(row, e, isGroup);
        clearDropMarks();
        const dragged = E.dragIds; E.dragIds = [];
        if (!isGroup && dragged.includes(r.w.id)) return;
        Ly.moveLayers(E.theme, dragged, isGroup ? { kind: 'group', id: r.g.id } : { kind: 'layer', id: r.w.id }, pos);
        E.selIds = dragged.filter(id => byId(id)); E.selGroup = null;
        paintLayerList(); paintStage(); paintProps();
        edited();
      });
      u.layers.append(row);
    }
    u.layers.scrollTop = keep;
    const hasSel = E.selIds.length > 0;
    u.btnGroup.disabled = !hasSel;
    u.btnDelete.disabled = !hasSel;
    u.btnUngroup.disabled = !(E.selGroup || sel().some(w => w.group));
  }
  function refreshBoxLabel(w) {
    const b = boxEl(w.id);
    if (b) { b.querySelector('.wlabel').textContent = nameOf(w); const nm = b.querySelector('.wname'); if (nm && nm.textContent) nm.textContent = nameOf(w); }
  }

  // ------------------------------------------------------------------ properties
  let posFields = null;
  function syncPosFields(w) {
    if (!posFields) return;
    for (const k of ['x', 'y', 'w', 'h']) if (posFields[k] && document.activeElement !== posFields[k]) posFields[k].value = (w[k] * 100).toFixed(1);
  }
  const swRow = (text, checked, onchange, note) => {
    const id = 'sw' + Math.random().toString(36).slice(2, 8);
    const input = el('input', { type: 'checkbox', id, checked: !!checked, role: 'switch', onchange: e => onchange(e.target.checked) });
    input.checked = !!checked;
    return el('div', {}, el('div', { class: 'sw-row' }, el('label', { for: id, text }), input), note ? el('p', { class: 'muted small', text: note }) : null);
  };
  const field = (text, control, id) => el('div', { class: 'field' }, el('label', { for: id, text }), control);
  function select2(id, value, opts, onchange) {
    const s = el('select', { id, onchange: e => onchange(e.target.value) }, opts.map(([v, t]) => el('option', { value: v, text: t })));
    s.value = value;
    return s;
  }
  function range(id, value, min, max, step, onchange, fmt = v => v) {
    const out = el('span', { class: 'num small muted', text: fmt(value) });
    const r = el('input', { type: 'range', id, min, max, step, value, oninput: e => { out.textContent = fmt(Number(e.target.value)); onchange(Number(e.target.value)); } });
    r.value = value;
    return el('div', { class: 'slider-row' }, r, out);
  }
  const numInput = (id, value, { min = 0, max = 9999, step = 1, onchange }) => {
    const i = el('input', { type: 'number', id, min, max, step, onchange: e => onchange(e.target.value === '' ? null : Number(e.target.value)) });
    i.value = value ?? '';
    return i;
  };

  function paintProps() {
    const u = E.ui;
    if (!u) return;
    posFields = null;
    u.props.textContent = '';
    const ms = sel();
    if (E.selGroup && groupById(E.selGroup)) return paintGroupProps(groupById(E.selGroup));
    if (ms.length > 1) return paintMultiProps(ms);
    if (ms.length === 1) return paintWidgetProps(ms[0]);
    paintThemeProps();
  }

  function paintMultiProps(ms) {
    const u = E.ui;
    u.props.append(el('h3', { text: `${ms.length} layers selected` }),
      el('p', { class: 'muted small', text: 'Drag any of them on the preview to move them together. Ctrl+G groups them.' }),
      el('div', { class: 'row' }, el('button', { class: 'btn secondary', text: 'Group', onclick: doGroup }), el('button', { class: 'btn danger', text: 'Delete', onclick: doDelete })));
  }
  function paintGroupProps(g) {
    const u = E.ui;
    const nm = el('input', { type: 'text', id: 'gName', maxLength: 60, spellcheck: false, oninput: e => { g.name = e.target.value; paintLayerList(); edited({ preview: false }); } });
    nm.value = g.name;
    u.props.append(el('h3', { text: 'Group' }), field('Name', nm, 'gName'),
      swRow('Visible', !g.hidden, v => { g.hidden = !v; E.baseSig = null; paintLayerList(); paintStage(); edited(); }),
      el('p', { class: 'muted small', text: `${Ly.membersOf(E.theme, g.id).length} layers. Click the group on the preview to move them all together.` }),
      el('div', { class: 'row' }, el('button', { class: 'btn secondary', text: 'Ungroup', onclick: doUngroup }), el('button', { class: 'btn danger', text: 'Delete group', onclick: doDelete })));
  }

  function paintWidgetProps(w) {
    const u = E.ui;
    Wd.setApp(S.app);
    const set = (key, v) => { w.props[key] = v; edited(); refreshBoxLabel(w); const l = u.layers.querySelector(`.lrow[data-id="${CSS.escape(w.id)}"] .lname`); if (l) l.textContent = nameOf(w); };
    const nameIn = el('input', { type: 'text', id: 'pName', maxLength: 60, spellcheck: false, placeholder: nameOf({ ...w, props: { ...w.props, name: '' } }), oninput: e => {
      if (e.target.value.trim()) w.props.name = e.target.value; else delete w.props.name;
      refreshBoxLabel(w); const l = u.layers.querySelector(`.lrow[data-id="${CSS.escape(w.id)}"] .lname`); if (l) l.textContent = nameOf(w); edited({ preview: false });
    } });
    nameIn.value = w.props.name || '';
    u.props.append(el('h3', { text: Wd.typeTitle(w) }), field('Layer name', nameIn, 'pName'));
    const p = w.props;
    if (['clock', 'sensor', 'date', 'weather', 'visualizer', 'slideshow', 'info'].includes(w.type)) {
      Wd.paintTypeProps(w, u.props, {
        el, h: { field, select2, range, numInput, swRow }, set, edited, S, backend, toast, oid: E.oid,
        repaint: () => paintProps(), moved: m => { const b = boxEl(m.id); if (b) placeBox(b, m); syncPosFields(m); refreshWarnings(); },
        relabel: () => { refreshBoxLabel(w); const l = u.layers.querySelector(`.lrow[data-id="${CSS.escape(w.id)}"] .lname`); if (l) l.textContent = nameOf(w); },
        openSettings: sec => ctx.openSettings && ctx.openSettings(sec),
      });
    } else if (w.type === 'music') {
      u.props.append(swRow('Show the cover', p.cover !== false, v => set('cover', v)), swRow('Show the progress bar', p.progress !== false, v => set('progress', v)),
        el('p', { class: 'muted small', text: 'Shows what is playing in Spotify, the browser or any other player. Hidden when nothing plays (change this in the theme settings).' }));
    } else if (w.type === 'stats') {
      const items = Array.isArray(p.items) ? p.items : [];
      const box = el('div', { class: 'checks', role: 'group', 'aria-label': 'What to show' });
      for (const [id, text] of STAT_ITEMS) {
        const c = el('input', { type: 'checkbox', checked: items.includes(id), onchange: e => {
          const cur = (w.props.items || []).filter(x => x !== id);
          if (e.target.checked) cur.push(id);
          w.props.items = STAT_ITEMS.map(([i]) => i).filter(i => cur.includes(i));
          edited();
        } });
        c.checked = items.includes(id);
        box.append(el('label', {}, c, text));
      }
      u.props.append(el('p', { class: 'small muted', text: 'Show' }), box);
    } else if (w.type === 'text') {
      const ta = el('textarea', { id: 'pText', rows: 3, maxLength: 300, spellcheck: false, oninput: e => set('text', e.target.value) });
      ta.value = p.text || '';
      u.props.append(field('Text', ta, 'pText'),
        field('Size', range('pSize', clamp(Number(p.size) || 0.08, 0.02, 0.3), 0.02, 0.3, 0.005, v => set('size', r4(v)), v => Math.round(v * 100) + '%'), 'pSize'),
        field('Colour', el('input', { type: 'color', id: 'pCol', value: p.color || '#ffffff', oninput: e => set('color', e.target.value) }), 'pCol'),
        field('Opacity', range('pOp', p.opacity ?? 1, 0, 1, 0.05, v => set('opacity', v), v => Math.round(v * 100) + '%'), 'pOp'),
        field('Align', select2('pAl', p.align || 'left', [['left', 'Left'], ['center', 'Centre'], ['right', 'Right']], v => set('align', v)), 'pAl'),
        field('Weight', select2('pWt', p.weight || 'regular', [['light', 'Light'], ['regular', 'Regular'], ['bold', 'Bold']], v => set('weight', v)), 'pWt'));
    } else if (w.type === 'image') {
      const file = el('input', { type: 'file', accept: 'image/png,image/jpeg,image/webp,image/gif,image/bmp', hidden: true, onchange: async e => {
        const f = e.target.files[0]; e.target.value = '';
        if (!f) return;
        try { const r = await backend.themeAsset(E.oid, f); w.props.file = r.file; edited(); refreshBoxLabel(w); paintLayerList(); paintProps(); }
        catch (err) { toast(err.message, true); }
      } });
      u.props.append(el('div', { class: 'field' }, el('p', { class: 'small', text: p.file ? p.file.split('/').pop() : 'No picture chosen yet.' }),
          el('button', { class: 'btn secondary', text: p.file ? 'Choose another picture…' : 'Choose a picture…', onclick: () => file.click() }), file),
        field('Opacity', range('pOp', p.opacity ?? 1, 0, 1, 0.05, v => set('opacity', v), v => Math.round(v * 100) + '%'), 'pOp'),
        field('Fit', select2('pFit', p.fit || 'contain', [['contain', 'Whole picture'], ['cover', 'Fill the box']], v => set('fit', v)), 'pFit'));
    }
    if (Wd.canBeOnBase(w.type)) {
      u.props.append(el('hr', { class: 'sep' }), swRow('Also on the stored base', w.onBase, v => {
        w.onBase = v; paintStage(); E.baseSig = null; edited();
      }, 'Draws it into the picture stored on the screen, so it stays there when CoolerScreen is closed. Changing it later means storing the base again.'));
    }
    const hit = Ly.bandHit(w, guidesNow());
    if (hit && E.guides) u.props.append(el('p', { class: 'warn-text small', text: `⚠ Overlaps ${hit}` }));
    u.props.append(el('hr', { class: 'sep' }), el('h3', { text: 'Position and size' }));
    const grid = el('div', { class: 'props-grid' });
    posFields = {};
    for (const [key, text] of [['x', 'Left (%)'], ['y', 'Top (%)'], ['w', 'Width (%)'], ['h', 'Height (%)']]) {
      const inp = el('input', { type: 'number', id: 'pos-' + key, min: 0, max: 100, step: 0.5, onchange: e => {
        const v = clamp(Number(e.target.value) / 100, 0, 1);
        if (key === 'x') w.x = clamp(v, 0, 1 - w.w); else if (key === 'y') w.y = clamp(v, 0, 1 - w.h);
        else if (key === 'w') w.w = clamp(v, MIN_BOX, 1 - w.x); else w.h = clamp(v, MIN_BOX, 1 - w.y);
        for (const kk of ['x', 'y', 'w', 'h']) w[kk] = r4(w[kk]);
        const b = boxEl(w.id);
        if (b) placeBox(b, w);
        syncPosFields(w); refreshWarnings(); edited();
      } });
      inp.value = (w[key] * 100).toFixed(1);
      posFields[key] = inp;
      grid.append(field(text, inp, 'pos-' + key));
    }
    u.props.append(grid, el('div', { class: 'row' }, el('button', { class: 'btn danger', text: 'Delete this item', onclick: doDelete })));
  }

  // The theme's own settings: name, layout preset, corners, guides, hand-over, base.
  function paintThemeProps() {
    const u = E.ui;
    const nameIn = el('input', { type: 'text', id: 'thName', maxLength: 80, spellcheck: false });
    nameIn.value = E.info.name;
    const err = el('div', { class: 'field-err', role: 'alert' });
    nameIn.onchange = async () => {
      const v = nameIn.value.trim();
      if (!v || v === E.info.name) { nameIn.value = E.info.name; return; }
      try { await call('original_patch', { originalId: E.oid, name: v }); E.info.name = v; err.textContent = ''; paintTitle(); ctx.loadOriginals(); }
      catch (e) { err.textContent = e.message; }
    };
    const t = E.theme, h = t.handover || (t.handover = {});
    const dflt = Math.round((S.app && S.app.cornerRadius) || 0);
    const onCorners = () => { E.baseSig = null; paintStage(); paintBanner(); edited(); };
    const cornerNum = numInput('thCorner', t.corners ?? dflt, { min: 0, max: 300, onchange: v => { if (v == null) { cornerNum.value = t.corners ?? dflt; return; } t.corners = clamp(Math.round(v), 0, 300); resetBtn.disabled = false; onCorners(); } });
    const resetBtn = el('button', { type: 'button', class: 'icon-btn', 'aria-label': 'Reset to default', title: 'Reset to default', id: 'thCornerReset', disabled: t.corners == null,
      onclick: () => { t.corners = null; cornerNum.value = dflt; resetBtn.disabled = true; onCorners(); } });
    resetBtn.innerHTML = '<svg viewBox="0 0 24 24" width="16" height="16" aria-hidden="true"><path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M20 11a8 8 0 0 0-14.3-4.5M4 4v4h4M4 13a8 8 0 0 0 14.3 4.5M20 20v-4h-4"/></svg>';
    const radio = (name, value, checked, text, on) => { const r = el('input', { type: 'radio', name, value, checked, onchange: on }); r.checked = checked; return el('label', { class: 'rad' }, r, text); };
    const gapNum = numInput('thGap', t.gap ?? Ly.DEFAULT_GAP, { min: 0, max: 200, onchange: v => { t.gap = v == null ? Ly.DEFAULT_GAP : clamp(Math.round(v), 0, 200); gapNum.value = t.gap; edited({ preview: false }); } });
    const G = Ly.effectiveGuides(S.app && S.app.screenGuides, null);
    const gBox = el('div', { class: 'offs' });
    const offRow = (label, key, field, base) => {
      const i = numInput('off-' + key + '-' + field, t.guidesOffset[key][field] || 0, { min: -600, max: 600, step: 5, onchange: v => { t.guidesOffset[key][field] = v || 0; paintStage(); edited({ preview: false }); paintLayerList(); } });
      i.setAttribute('aria-label', label); i.title = 'Global ' + base;
      return el('div', { class: 'off-row' }, el('label', { class: 'small', for: i.id, text: label }), i);
    };
    gBox.append(offRow('Left', 'left', 'size', G.left.size), offRow('Right', 'right', 'size', G.right.size),
      offRow('Curve position', 'curve', 'offset', G.curve.offset), offRow('Curve left', 'curve', 'left', G.curve.left), offRow('Curve right', 'curve', 'right', G.curve.right));
    u.props.append(
      el('h3', { text: 'Theme settings' }),
      field('Name', nameIn, 'thName'), err,
      el('p', { class: 'muted small', text: widgets().length ? 'Click a layer on the preview or in the list to change it.' : 'Add items from the left: drag them onto the preview, or click them.' }),
      el('div', { class: 'row' }, el('button', { class: 'btn secondary', text: 'Use the Standard layout', title: 'Clock, music and PC stats where they were before themes', onclick: usePreset })),
      el('hr', { class: 'sep' }),
      el('div', { class: 'field' }, el('label', { for: 'thCorner', text: 'Corner radius' }), el('div', { class: 'inline-ctl' }, cornerNum, resetBtn)),
      field('Gap', gapNum, 'thGap'),
      el('hr', { class: 'sep' }),
      el('div', { class: 'row between' }, el('h3', { text: 'Guides' }),
        el('button', { class: 'btn ghost small-btn', text: 'Reset', title: 'Back to the global guides', onclick: () => { t.guidesOffset = { left: {}, right: {}, curve: {} }; paintProps(); paintStage(); edited({ preview: false }); } })),
      gBox,
      el('hr', { class: 'sep' }),
      el('h3', { text: 'Hand-over' }),
      field('Fade-in when live starts', range('hoFade', clamp(Number(h.fadeIn ?? 0.8), 0, 3), 0, 3, 0.1, v => { h.fadeIn = Math.round(v * 10) / 10; edited({ preview: false }); }, v => v.toFixed(1) + ' s'), 'hoFade'),
      el('p', { class: 'small muted', text: 'When CoolerScreen stops:' }));
    for (const [v, tt, d] of END_CHOICES) {
      u.props.append(radio('hoEnd', v, (h.end || 'loopStart') === v, tt, () => { h.end = v; edited({ preview: false }); }));
      u.props.append(el('p', { class: 'muted small hint', text: d }));
    }
    u.props.append(field('Music when nothing is playing', select2('hoMusic', h.musicIdle || 'hide', [['hide', 'Hide the music item'], ['keep', 'Keep it, showing an idle card']], v => { h.musicIdle = v; edited({ preview: false }); }), 'hoMusic'),
      el('hr', { class: 'sep' }),
      el('h3', { text: 'Base' }),
      el('p', { class: 'small muted', text: E.info.baseOwner ? `The base is “${E.info.baseOwner.name}”, the picture stored on the screen. It keeps playing when CoolerScreen is closed.` : 'The base picture has not been stored yet.' }),
      el('div', { class: 'row' },
        el('button', { class: 'btn secondary', text: 'Base settings…', title: 'Choose any clip as the base, with its loop, quality and versions', onclick: () => ctx.openBase(E.oid, E.info) }),
        el('button', { class: 'btn ghost', text: 'Store the base again', title: 'Make the base picture again and send it to the screen', onclick: () => ctx.applyTheme(E.oid, E.info.name) })));
  }
  async function usePreset() {
    if (widgets().length) {
      const ok = await openDialog({ title: 'Replace the items?', body: 'The Standard layout replaces the items in this theme with a clock, the music and the PC stats.',
        actions: [{ label: 'Cancel', value: false, kind: 'secondary' }, { label: 'Use Standard layout', value: true, kind: 'primary' }] });
      if (!ok) return;
    }
    try { const t = await call('theme_standard'); E.theme.widgets = t.widgets; E.theme.groups = []; E.selIds = []; E.selGroup = null; E.theme.liveLayer = true; paintBar(); paintLayerList(); paintStage(); paintProps(); edited(); }
    catch (e) { toast(e.message, true); }
  }

  return { enter, appChanged, isOpen: () => !!E.ui, flush: () => (E.rev !== E.saved ? save() : Promise.resolve()) };
}

// ---------------------------------------------------------------------- the Apply dialog
let applying = false;
const STEPS = ['Preparing the picture', 'Storing it on the screen', 'Switching the live layer'];

/**
 * Apply a theme: a modal that locks the whole window while the base is made, stored on the screen and the live layer is
 * switched. Resolves 'done' | 'stopped' | 'failed'. The only button stops it and keeps the previous theme.
 */
async function runApply(oid, name) {
  const { call, backend } = runApply.ctx;
  if (applying) return 'failed';
  applying = true;
  const $ = id => document.getElementById(id);
  const back = $('applyBack'), stop = $('applyStop'), note = $('applyNote'), bar = $('applyBar');
  const prevFocus = document.activeElement;
  const inerted = [];
  for (const c of document.body.children) if (c !== back && c.tagName !== 'SCRIPT' && !c.inert) { c.inert = true; inerted.push(c); }
  $('applyTitle').textContent = `Applying “${name}”`;
  const steps = $('applySteps');
  steps.textContent = '';
  const rows = STEPS.map((t, i) => {
    const mark = document.createElement('span'); mark.className = 'mark'; mark.setAttribute('aria-hidden', 'true'); mark.textContent = String(i + 1);
    const li = document.createElement('li'); li.append(mark, document.createTextNode(t));
    steps.append(li);
    return { li, mark };
  });
  const setStep = (n, failed = false) => {
    rows.forEach((r, i) => {
      const st = failed && i === n - 1 ? 'fail' : i < n - 1 ? 'done' : i === n - 1 ? 'run' : '';
      r.li.className = st;
      r.mark.textContent = st === 'done' ? '✓' : st === 'fail' ? '!' : String(i + 1);
      r.mark.replaceChildren(...(st === 'run' ? [Object.assign(document.createElement('span'), { className: 'spinner' })] : [document.createTextNode(r.mark.textContent)]));
    });
  };
  const setBar = f => { const pct = Math.round(clampN(f) * 100); bar.firstChild.style.width = pct + '%'; bar.setAttribute('aria-valuenow', String(pct)); };
  const clampN = v => Math.min(1, Math.max(0, v || 0));
  setStep(1); setBar(0);
  note.textContent = 'Starting…';
  stop.disabled = false; stop.hidden = false; stop.textContent = 'Stop and keep the previous theme';
  back.hidden = false;
  stop.focus();
  const onKey = e => {
    if (e.key === 'Tab') { e.preventDefault(); stop.focus(); }
    else if (e.key === 'Escape') e.preventDefault(); // the lock is deliberate: only the button ends it
  };
  document.addEventListener('keydown', onKey, true);
  const release = () => {
    document.removeEventListener('keydown', onKey, true);
    back.hidden = true;
    for (const c of inerted) c.inert = false;
    applying = false;
    if (prevFocus && document.contains(prevFocus) && prevFocus.focus) prevFocus.focus();
  };

  let jobId = null, result = 'failed', stopping = false, shown = 0;
  stop.onclick = async () => {
    stopping = true;
    stop.disabled = true;
    note.textContent = 'Stopping… the previous theme stays on the screen.';
    if (jobId) { try { await call('job_cancel', { jobId }); } catch { /* it ends on its own */ } }
  };
  try {
    ({ jobId } = await call('theme_apply', { originalId: oid }));
    await backend.job(jobId, j => {
      const m = /^Step (\d) of 3: (.*)$/.exec(j.message || '');
      if (m) setStep(Number(m[1]));
      shown = Math.max(shown, j.progress || 0); // the bar never goes back
      setBar(shown);
      // Writing to the screen cannot be stopped halfway (it would leave a broken picture): that part finishes first.
      const writing = !!m && Number(m[1]) === 2 && !/Waiting/.test(m[2]);
      if (!stopping) stop.disabled = writing;
      note.textContent = stopping ? note.textContent : writing ? 'Writing to the screen. This part is not stopped halfway, because that could leave the screen with a broken picture.'
        : j.status === 'waiting' ? runApply.ctx.plainWait(j.message) : m ? m[2] : j.message || '';
    });
    rows.forEach(r => { r.li.className = 'done'; r.mark.textContent = '✓'; });
    setBar(1);
    note.textContent = 'Done. The theme is on the screen.';
    stop.hidden = true;
    result = 'done';
    await new Promise(r => setTimeout(r, 900));
  } catch (e) {
    if (e.code === 'cancelled' || (e.data && e.data.state === 'cancelled')) {
      result = 'stopped';
    } else {
      const m = /^Step (\d)/.exec((e.data && e.data.message) || '');
      setStep(m ? Number(m[1]) : 1, true);
      note.textContent = `${e.message} The previous theme is still on the screen.`;
      stop.disabled = false; stop.textContent = 'Close';
      await new Promise(r => { stop.onclick = r; });
      result = 'failed';
    }
  }
  release();
  return result;
}

/** Wire the dialog to the app's helpers (called once by app.js). */
export function initApply(ctx) { runApply.ctx = ctx; }
export { runApply };
