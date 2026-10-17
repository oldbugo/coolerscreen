// 3.0 themes: the "On screen" theme editor and the Apply dialog (docs/v3/theme-format.md).
//
// The editor works on one theme: the one whose base is in the screen's loop slot ("current theme"). It edits a working copy
// of theme.json: boxes on the preview (move, resize), a palette to drop items from, an item list, and a properties panel.
// Every edit is saved after a short pause (theme_set, which also updates the live screen at once) and the rendered preview is
// refreshed (theme_preview). Edits to text/picture items that are baked into the stored base show "Base needs re-storing".
import { styleNotch } from './frame.js';
import { openDialog } from './dialog.js';

const MIN_BOX = 0.02;
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
const r4 = v => Math.round(v * 10000) / 10000;

const KINDS = {
  clock: { name: 'Clock', icon: '◔', w: 0.3, h: 0.34, props: () => ({ format: '24h', seconds: false, date: true, color: '#ffffff' }) },
  music: { name: 'Music', icon: '♪', w: 0.34, h: 0.22, props: () => ({ cover: true, progress: true }) },
  stats: { name: 'PC stats', icon: '▤', w: 0.26, h: 0.5, props: () => ({ items: ['cpu', 'ram', 'gpuLoad', 'gpuTemp', 'gpuClock'] }) },
  text: { name: 'Text', icon: 'T', w: 0.34, h: 0.16, props: () => ({ text: 'Your text', size: 0.08, color: '#ffffff', opacity: 1, align: 'left', weight: 'regular' }), base: true },
  image: { name: 'Picture', icon: '▣', w: 0.2, h: 0.3, props: () => ({ file: '', opacity: 1, fit: 'contain' }), base: true },
  weather: { name: 'Weather', icon: '☁', w: 0.26, h: 0.3, props: () => ({}), soon: true },
};
const STAT_ITEMS = [['cpu', 'PC load'], ['ram', 'Memory'], ['gpuLoad', 'Graphics card load'], ['gpuTemp', 'Graphics card temperature'], ['gpuClock', 'Graphics card speed'], ['cpuTemp', 'Processor temperature']];
const END_CHOICES = [
  ['loopStart', 'Fade out onto the stored loop', 'The live layer fades away and the screen carries on from the first frame of its stored loop. Nothing jumps.'],
  ['instant', 'Stop at once', 'The live layer disappears and the stored loop plays.'],
  ['screenOff', 'Fade to black and turn the screen off', 'The screen goes dark when CoolerScreen stops.'],
];

export function createThemes(ctx) {
  const { el, $, call, backend, S, toast } = ctx;
  const root = $('screenView');
  const E = {
    oid: null, info: null, theme: null, sel: null, view: 'live', rev: 0, saved: 0, saving: false, saveErr: null,
    liveUrl: null, baseUrl: null, baseSig: null, pvSeq: 0, previewOk: true, ui: null, saveT: 0, pvT: 0, loadSeq: 0,
  };
  const currentId = () => S.currentTheme || null;

  // ------------------------------------------------------------------ entering and leaving
  /** Show the editor for the current theme (called when the view opens and when the library changes). */
  async function enter() {
    if (S.rev < 0) return; // the library has not been read yet
    const oid = currentId();
    if (!oid) {
      E.oid = null; E.ui = null;
      renderEmpty(S.originals.length ? 'Nothing has been applied to the screen yet.' : 'You have no themes yet.');
      return;
    }
    if (E.oid !== oid || !E.ui) {
      await open(oid);
    } else {
      await syncFromServer();
    }
  }

  async function open(oid) {
    const seq = ++E.loadSeq;
    let info;
    try { info = await call('theme_get', { originalId: oid }); } catch (e) { if (seq === E.loadSeq) renderEmpty(e.message); return; }
    if (seq !== E.loadSeq) return;
    Object.assign(E, { oid, info, theme: info.theme, sel: null, view: E.oid === oid ? E.view : 'live', rev: 0, saved: 0, saveErr: null, liveUrl: null, baseUrl: null, baseSig: null, previewOk: true });
    buildEditor();
    paintAll();
    schedulePreview(0);
  }

  /** The library changed (a base was stored, a theme renamed...): refresh what the server knows, keep unsaved edits. */
  async function syncFromServer() {
    let info;
    try { info = await call('theme_get', { originalId: E.oid }); } catch { return; }
    const dirty = E.rev !== E.saved || E.saving || E.dragging;
    const baseChanged = info.baseVersionId !== E.info.baseVersionId;
    const themeChanged = JSON.stringify(info.theme) !== JSON.stringify(E.theme);
    E.info = info;
    if (!dirty && themeChanged) E.theme = info.theme;
    if (E.ui) {
      paintTitle(); paintBanner();
      if (!dirty && themeChanged) { paintItems(); paintStage(); paintProps(); } // (an open text field is never rebuilt without a reason)
    }
    if (baseChanged || (!dirty && themeChanged)) { if (baseChanged) E.baseSig = null; schedulePreview(0); }
  }

  // ------------------------------------------------------------------ empty state
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
    // left: palette + item list
    u.palette = el('ul', { class: 'palette', 'aria-label': 'Items you can add' });
    for (const [type, k] of Object.entries(KINDS)) {
      const b = el('button', { type: 'button', class: 'pal', draggable: !k.soon, disabled: !!k.soon, title: k.soon ? 'Coming later' : `Add ${k.name.toLowerCase()}: click, or drag it onto the preview`,
        'data-type': type, onclick: () => addWidget(type) },
        el('span', { class: 'pi', 'aria-hidden': 'true', text: k.icon }), el('span', { class: 'pn', text: k.name }), k.soon ? el('span', { class: 'soon', text: 'coming later' }) : null);
      b.addEventListener('dragstart', ev => { ev.dataTransfer.setData('text/x-cs-widget', type); ev.dataTransfer.effectAllowed = 'copy'; });
      u.palette.append(el('li', {}, b));
    }
    u.items = el('ul', { class: 'items', role: 'listbox', 'aria-label': 'Items in this theme' });
    u.itemsNone = el('p', { class: 'muted small', text: 'Nothing yet: this theme shows its base only.' });
    const left = el('aside', { class: 'ed-left', 'aria-label': 'Items' },
      el('h3', { text: 'Add' }), u.palette, el('h3', { text: 'In this theme' }), u.items, u.itemsNone);
    // centre
    u.title = el('h1', { id: 'screenTitle' });
    u.sub = el('span', { class: 'muted small' });
    u.saveState = el('span', { class: 'save-state', role: 'status', 'aria-live': 'polite' });
    u.views = el('div', { class: 'seg', role: 'group', 'aria-label': 'What the preview shows' });
    for (const [v, t] of [['live', 'With live layer'], ['base', 'Base only'], ['hand', 'Hand-over']]) {
      u.views.append(el('button', { type: 'button', 'data-v': v, text: t, 'aria-pressed': 'false', onclick: () => setView(v) }));
    }
    u.banner = el('div');
    u.stage = el('div', { class: 'stage', role: 'application', 'aria-label': 'Preview of the screen. Drag items to move them; use the handles to resize.' });
    u.imgBase = el('img', { class: 'layer', alt: '', draggable: false });
    u.imgLive = el('img', { class: 'layer', alt: '', draggable: false });
    u.notch = el('div', { class: 'notch', hidden: true, title: 'The camera hole. Keep items clear of it.' });
    u.note = el('div', { class: 'stage-note', hidden: true });
    u.boxes = el('div', { class: 'boxes' });
    u.stage.append(u.imgBase, u.imgLive, u.notch, u.boxes, u.note);
    u.stage.addEventListener('dragover', ev => { if ([...ev.dataTransfer.types].includes('text/x-cs-widget')) { ev.preventDefault(); ev.dataTransfer.dropEffect = 'copy'; u.stage.classList.add('drop-ok'); } });
    u.stage.addEventListener('dragleave', () => u.stage.classList.remove('drop-ok'));
    u.stage.addEventListener('drop', ev => {
      u.stage.classList.remove('drop-ok');
      const t = ev.dataTransfer.getData('text/x-cs-widget');
      if (!KINDS[t]) return;
      ev.preventDefault(); ev.stopPropagation();
      const r = u.stage.getBoundingClientRect();
      addWidget(t, (ev.clientX - r.left) / r.width, (ev.clientY - r.top) / r.height);
    });
    u.stage.addEventListener('pointerdown', ev => { if (ev.target === u.stage || ev.target === u.imgBase || ev.target === u.imgLive) select(null); });
    u.stageWrap = el('div', { class: 'stage-wrap' }, u.stage);
    u.hand = el('div', { class: 'hand-panel', hidden: true });
    u.baseBtn = el('button', { class: 'btn ghost', text: 'Base settings…', title: 'Loop, quality and saved versions of the base picture', onclick: () => ctx.openDetail(E.oid) });
    const center = el('section', { class: 'ed-center', 'aria-labelledby': 'screenTitle' },
      el('div', { class: 'ed-title' }, u.title, u.sub, el('span', { class: 'grow' }), u.saveState, u.baseBtn),
      u.banner,
      el('div', { class: 'ed-bar' }, u.views),
      u.stageWrap, u.hand);
    u.props = el('aside', { class: 'ed-right', 'aria-label': 'Properties' });
    root.append(el('div', { class: 'ed' }, left, center, u.props));
    root.onkeydown = onKeys;
  }

  function setView(v) {
    E.view = v;
    if (E.sel && v === 'base' && !byId(E.sel)?.onBase) E.sel = null;
    paintAll();
    if (v === 'base' || v === 'hand') schedulePreview(0);
  }

  function paintAll() {
    if (!E.ui) return;
    paintTitle(); paintBanner(); paintViews(); paintItems(); paintStage(); paintProps(); paintSave();
  }

  function paintTitle() {
    const u = E.ui;
    u.title.textContent = E.info.name;
    u.sub.textContent = 'is on the screen' + (S.app?.live?.settings?.inApp ? ' · live is on' : '');
  }
  function paintSave() {
    const u = E.ui;
    u.saveState.textContent = E.saveErr ? 'Not saved' : E.rev !== E.saved || E.saving ? 'Saving…' : E.saved ? 'Saved' : '';
    u.saveState.title = E.saveErr || '';
    u.saveState.style.color = E.saveErr ? 'var(--danger)' : '';
  }
  function paintViews() {
    for (const b of E.ui.views.children) b.setAttribute('aria-pressed', String(b.dataset.v === E.view));
    E.ui.stageWrap.hidden = E.view === 'hand';
    E.ui.hand.hidden = E.view !== 'hand';
    if (E.view === 'hand') paintHand();
  }
  function paintBanner() {
    const u = E.ui;
    u.banner.textContent = '';
    if (E.info.baseStale) {
      u.banner.append(el('div', { class: 'base-banner', role: 'status' },
        el('p', { text: 'Base needs re-storing: the text or pictures that are part of the base changed since it was stored on the screen. The live layer already shows the change.' }),
        el('button', { class: 'btn primary', text: 'Store the base again', onclick: () => ctx.applyTheme(E.oid, E.info.name) })));
    }
  }

  // ------------------------------------------------------------------ the working theme
  const widgets = () => E.theme.widgets;
  const byId = id => widgets().find(w => w.id === id);
  const label = w => {
    const k = KINDS[w.type];
    if (!k) return w.type;
    if (w.type === 'text') { const t = (w.props.text || '').replace(/\s+/g, ' ').trim(); return t ? `Text: “${t.slice(0, 22)}${t.length > 22 ? '…' : ''}”` : 'Text'; }
    if (w.type === 'image') return w.props.file ? `Picture: ${w.props.file.split('/').pop()}` : 'Picture (none chosen)';
    return k.name;
  };

  /** Something about the theme changed: save soon, refresh the rendered preview soon. */
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
      // nothing newer: take the server's cleaned copy (only when it differs, so a drag in progress is not rebuilt)
      if (E.rev === rev && !E.dragging && JSON.stringify(r.theme) !== JSON.stringify(E.theme)) { E.theme = r.theme; E.repaint = true; }
    } catch (e) {
      E.saveErr = e.message;
      toast(`The theme could not be saved: ${e.message}`, true);
    }
    E.saving = false;
    if (E.ui && E.oid === oid) {
      paintSave(); paintBanner();
      if (E.rev !== E.saved && !E.saveErr) E.saveT = setTimeout(save, 100);
      else if (E.repaint && !E.dragging) { E.repaint = false; paintItems(); paintStage(); paintProps(); }
    }
  }

  // ------------------------------------------------------------------ preview images
  const baseSigOf = () => JSON.stringify(widgets().filter(w => w.onBase));
  function schedulePreview(ms) {
    clearTimeout(E.pvT);
    E.pvT = setTimeout(refreshPreview, ms);
  }
  async function refreshPreview() {
    if (!E.ui || !E.oid) return;
    const seq = ++E.pvSeq, oid = E.oid;
    const theme = JSON.parse(JSON.stringify(E.theme));
    const live = call('theme_preview', { originalId: oid, theme, withLive: true, w: 1200, h: 540 });
    const sig = baseSigOf();
    const needBase = E.baseSig !== sig || !E.baseUrl;
    const base = needBase ? call('theme_preview', { originalId: oid, theme, withLive: false, w: 1200, h: 540 }) : null;
    const [l, b] = await Promise.allSettled([live, base || Promise.resolve(null)]);
    if (seq !== E.pvSeq || E.oid !== oid || !E.ui) return;
    E.previewOk = l.status === 'fulfilled' || (b && b.status === 'fulfilled');
    E.liveUrl = l.status === 'fulfilled' ? l.value.url : null;
    if (needBase) { E.baseUrl = b && b.status === 'fulfilled' ? b.value.url : null; E.baseSig = sig; }
    await Promise.all([setImg(E.ui.imgLive, E.liveUrl), setImg(E.ui.imgBase, E.baseUrl || fallbackFrame())]);
    paintLayers(); if (E.view === 'hand') paintHand();
  }
  const fallbackFrame = () => (S.originals.find(o => o.id === E.oid) || {}).frameUrl || null;
  function setImg(img, url) {
    return new Promise(res => {
      if (!url) { img.removeAttribute('src'); img.hidden = true; return res(); }
      if (img.dataset.url === url) { img.hidden = false; return res(); }
      const pre = new Image();
      pre.onload = pre.onerror = () => { img.src = url; img.dataset.url = url; img.hidden = false; res(); };
      pre.src = url;
    });
  }

  // ------------------------------------------------------------------ the stage: boxes
  function paintStage() {
    const u = E.ui;
    if (!u) return;
    styleNotch(u.notch, S.app && S.app.mask);
    u.boxes.textContent = '';
    const list = widgets().filter(w => (E.view === 'live' ? true : w.onBase));
    for (const w of list) u.boxes.append(makeBox(w));
    paintLayers();
  }
  /** The picture layers and the note on the stage (the boxes are left alone, so a drag in progress is not interrupted). */
  function paintLayers() {
    const u = E.ui;
    if (!u) return;
    u.imgLive.style.display = E.view === 'live' && E.liveUrl ? '' : 'none';
    u.imgBase.style.display = '';
    const list = widgets().filter(w => (E.view === 'live' ? true : w.onBase));
    if (!E.previewOk) { u.note.hidden = false; u.note.textContent = 'The rendered preview is not available yet; showing the base picture and the item boxes.'; }
    else if (E.view === 'base' && !list.length) { u.note.hidden = false; u.note.textContent = 'The base is the picture stored on the screen. Text and pictures marked "also on the stored base" appear here.'; }
    else u.note.hidden = true;
  }
  function placeBox(b, w) {
    b.style.left = w.x * 100 + '%'; b.style.top = w.y * 100 + '%'; b.style.width = w.w * 100 + '%'; b.style.height = w.h * 100 + '%';
    b.classList.toggle('top', w.y < 0.06);
  }
  function makeBox(w) {
    const b = el('div', { class: 'wbox' + (E.sel === w.id ? ' sel' : ''), tabindex: '0', role: 'button', 'data-id': w.id, 'aria-label': `${label(w)}. Arrow keys move it, Delete removes it.` },
      el('span', { class: 'wlabel', text: label(w) }), el('span', { class: 'wname', text: E.sel === w.id ? '' : label(w) }));
    placeBox(b, w);
    for (const h of ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w']) b.append(el('span', { class: 'wh', 'data-h': h }));
    b.addEventListener('pointerdown', ev => {
      if (ev.button !== 0) return;
      const h = ev.target.dataset && ev.target.dataset.h;
      select(w.id, { keepFocus: true });
      startDrag(ev, w, b, h || 'move');
    });
    b.addEventListener('focus', () => { if (E.sel !== w.id) select(w.id, { keepFocus: true }); });
    return b;
  }
  function startDrag(ev, w, box, mode) {
    ev.preventDefault();
    const sr = E.ui.stage.getBoundingClientRect();
    const o = { x: w.x, y: w.y, w: w.w, h: w.h }, sx = ev.clientX, sy = ev.clientY;
    const live = E.ui.boxes.querySelector(`.wbox[data-id="${CSS.escape(w.id)}"]`) || box;
    live.focus({ preventScroll: true });
    try { live.setPointerCapture(ev.pointerId); } catch { /* the pointer is gone */ }
    let moved = false;
    E.dragging = true;
    const move = e => {
      const dx = (e.clientX - sx) / sr.width, dy = (e.clientY - sy) / sr.height;
      let { x, y, w: ww, h: hh } = o;
      if (mode === 'move') { x = clamp(o.x + dx, 0, 1 - o.w); y = clamp(o.y + dy, 0, 1 - o.h); }
      else {
        if (mode.includes('e')) ww = clamp(o.w + dx, MIN_BOX, 1 - o.x);
        if (mode.includes('s')) hh = clamp(o.h + dy, MIN_BOX, 1 - o.y);
        if (mode.includes('w')) { const nx = clamp(o.x + dx, 0, o.x + o.w - MIN_BOX); ww = o.x + o.w - nx; x = nx; }
        if (mode.includes('n')) { const ny = clamp(o.y + dy, 0, o.y + o.h - MIN_BOX); hh = o.y + o.h - ny; y = ny; }
      }
      moved = true;
      Object.assign(w, { x: r4(x), y: r4(y), w: r4(ww), h: r4(hh) });
      placeBox(live, w);
      syncPosFields(w);
    };
    const up = () => {
      live.removeEventListener('pointermove', move);
      live.removeEventListener('pointerup', up);
      live.removeEventListener('pointercancel', up);
      E.dragging = false;
      if (moved) edited();
    };
    live.addEventListener('pointermove', move);
    live.addEventListener('pointerup', up);
    live.addEventListener('pointercancel', up);
  }
  function onKeys(ev) {
    if (!E.sel || ev.target.closest('input, textarea, select')) return;
    const w = byId(E.sel);
    if (!w) return;
    const step = ev.shiftKey ? 0.02 : 0.005;
    const d = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step] }[ev.key];
    if (d) {
      ev.preventDefault();
      w.x = r4(clamp(w.x + d[0], 0, 1 - w.w)); w.y = r4(clamp(w.y + d[1], 0, 1 - w.h));
      const b = E.ui.boxes.querySelector(`.wbox[data-id="${CSS.escape(w.id)}"]`);
      if (b) placeBox(b, w);
      syncPosFields(w); edited();
    } else if (ev.key === 'Delete' || (ev.key === 'Backspace' && !ev.target.closest('button'))) {
      ev.preventDefault(); removeWidget(w.id);
    }
  }

  // ------------------------------------------------------------------ adding, selecting, ordering
  function nextId() {
    const used = new Set(widgets().map(w => w.id));
    for (let i = 1; ; i++) if (!used.has('w' + i)) return 'w' + i;
  }
  function addWidget(type, cx, cy) {
    const k = KINDS[type];
    if (!k || k.soon || !E.ui) return;
    if (widgets().length >= 40) { toast('A theme can hold at most 40 items.', true); return; }
    const n = widgets().length;
    const w = k.w, h = k.h;
    let x, y;
    if (cx != null) { x = cx - w / 2; y = cy - h / 2; } else { x = 0.1 + 0.04 * (n % 8); y = 0.12 + 0.05 * (n % 8); }
    const wd = { id: nextId(), type, x: r4(clamp(x, 0, 1 - w)), y: r4(clamp(y, 0, 1 - h)), w, h, onBase: false, props: k.props() };
    widgets().push(wd);
    if (E.view === 'base' && !wd.onBase) E.view = 'live';
    E.sel = wd.id;
    paintViews(); paintItems(); paintStage(); paintProps();
    edited();
    const b = E.ui.boxes.querySelector(`.wbox[data-id="${CSS.escape(wd.id)}"]`);
    if (b) b.focus({ preventScroll: true });
  }
  function removeWidget(id) {
    const i = widgets().findIndex(w => w.id === id);
    if (i < 0) return;
    widgets().splice(i, 1);
    E.sel = null;
    paintItems(); paintStage(); paintProps();
    edited();
  }
  function moveWidget(id, dir) {
    const l = widgets(), i = l.findIndex(w => w.id === id), j = i + dir;
    if (i < 0 || j < 0 || j >= l.length) return;
    [l[i], l[j]] = [l[j], l[i]];
    paintItems(); paintStage();
    edited();
  }
  function select(id, { keepFocus = false } = {}) {
    if (E.sel === id) return;
    E.sel = id;
    paintItems(); paintProps();
    for (const b of E.ui.boxes.children) {
      const on = b.dataset.id === id;
      b.classList.toggle('sel', on);
      const nm = b.querySelector('.wname');
      if (nm) nm.textContent = on ? '' : label(byId(b.dataset.id) || {});
    }
    if (!keepFocus && id) { const b = E.ui.boxes.querySelector(`.wbox[data-id="${CSS.escape(id)}"]`); if (b) b.focus({ preventScroll: true }); }
  }

  // ------------------------------------------------------------------ the item list
  function paintItems() {
    const u = E.ui;
    if (!u) return;
    u.items.textContent = '';
    const l = widgets();
    u.itemsNone.hidden = l.length > 0;
    l.forEach((w, i) => {
      const li = el('li', { class: 'item', role: 'option', 'aria-selected': String(E.sel === w.id), 'data-id': w.id, onclick: () => select(w.id) },
        el('span', { class: 'it', text: label(w), title: label(w) }),
        el('button', { class: 'iconbtn', 'aria-label': `Move ${label(w)} earlier (drawn lower)`, title: 'Draw earlier', disabled: i === 0, text: '↑', onclick: e => { e.stopPropagation(); moveWidget(w.id, -1); } }),
        el('button', { class: 'iconbtn', 'aria-label': `Move ${label(w)} later (drawn on top)`, title: 'Draw later (on top)', disabled: i === l.length - 1, text: '↓', onclick: e => { e.stopPropagation(); moveWidget(w.id, 1); } }),
        el('button', { class: 'iconbtn', 'aria-label': `Delete ${label(w)}`, title: 'Delete', text: '✕', onclick: e => { e.stopPropagation(); removeWidget(w.id); } }));
      u.items.append(li);
    });
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

  function paintProps() {
    const u = E.ui;
    if (!u) return;
    posFields = null;
    u.props.textContent = '';
    const w = E.sel && byId(E.sel);
    if (!w) return paintThemeProps();
    const k = KINDS[w.type] || { name: w.type };
    const set = (key, v) => { w.props[key] = v; edited(); refreshLabels(w); };
    u.props.append(el('h3', { text: k.name }));
    const p = w.props;
    if (w.type === 'clock') {
      u.props.append(
        field('Style', select2('pFmt', p.format || '24h', [['24h', '24-hour'], ['12h', '12-hour']], v => set('format', v)), 'pFmt'),
        swRow('Show seconds', p.seconds, v => set('seconds', v)),
        swRow('Show the date', p.date !== false, v => set('date', v)),
        field('Colour', el('input', { type: 'color', id: 'pCol', value: p.color || '#ffffff', oninput: e => set('color', e.target.value) }), 'pCol'));
    } else if (w.type === 'music') {
      u.props.append(swRow('Show the cover', p.cover !== false, v => set('cover', v)), swRow('Show the progress bar', p.progress !== false, v => set('progress', v)),
        el('p', { class: 'muted small', text: 'Shows what is playing in Spotify, the browser or any other player. Hidden when nothing plays (change this under Hand-over).' }));
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
        try { const r = await backend.themeAsset(E.oid, f); w.props.file = r.file; edited(); refreshLabels(w); paintProps(); }
        catch (err) { toast(err.message, true); }
      } });
      u.props.append(el('div', { class: 'field' }, el('p', { class: 'small', text: p.file ? p.file.split('/').pop() : 'No picture chosen yet.' }),
          el('button', { class: 'btn secondary', text: p.file ? 'Choose another picture…' : 'Choose a picture…', onclick: () => file.click() }), file),
        field('Opacity', range('pOp', p.opacity ?? 1, 0, 1, 0.05, v => set('opacity', v), v => Math.round(v * 100) + '%'), 'pOp'),
        field('Fit', select2('pFit', p.fit || 'contain', [['contain', 'Whole picture'], ['cover', 'Fill the box']], v => set('fit', v)), 'pFit'));
    } else if (w.type === 'weather') {
      u.props.append(el('p', { class: 'muted small', text: 'Weather is coming in a later version.' }));
    }
    if (k.base) {
      u.props.append(el('hr', { class: 'sep' }), swRow('Also on the stored base', w.onBase, v => {
        w.onBase = v; paintStage(); refreshLabels(w); edited();
        E.baseSig = null;
      }, 'Draws it into the picture stored on the screen, so it stays there when CoolerScreen is closed. Changing it later means storing the base again.'));
    }
    u.props.append(el('hr', { class: 'sep' }), el('h3', { text: 'Position and size' }));
    const grid = el('div', { class: 'props-grid' });
    posFields = {};
    for (const [key, text] of [['x', 'Left (%)'], ['y', 'Top (%)'], ['w', 'Width (%)'], ['h', 'Height (%)']]) {
      const inp = el('input', { type: 'number', id: 'pos-' + key, min: 0, max: 100, step: 0.5, onchange: e => {
        const v = clamp(Number(e.target.value) / 100, 0, 1);
        if (key === 'x') w.x = clamp(v, 0, 1 - w.w); else if (key === 'y') w.y = clamp(v, 0, 1 - w.h);
        else if (key === 'w') w.w = clamp(v, MIN_BOX, 1 - w.x); else w.h = clamp(v, MIN_BOX, 1 - w.y);
        for (const kk of ['x', 'y', 'w', 'h']) w[kk] = r4(w[kk]);
        const b = u.boxes.querySelector(`.wbox[data-id="${CSS.escape(w.id)}"]`);
        if (b) placeBox(b, w);
        syncPosFields(w); edited();
      } });
      inp.value = (w[key] * 100).toFixed(1);
      posFields[key] = inp;
      grid.append(field(text, inp, 'pos-' + key));
    }
    u.props.append(grid, el('div', { class: 'row' }, el('button', { class: 'btn danger', text: 'Delete this item', onclick: () => removeWidget(w.id) })));
  }
  function refreshLabels(w) {
    const l = E.ui.items.querySelector(`.item[data-id="${CSS.escape(w.id)}"] .it`);
    if (l) { l.textContent = label(w); l.title = label(w); }
    const b = E.ui.boxes.querySelector(`.wbox[data-id="${CSS.escape(w.id)}"]`);
    if (b) { b.querySelector('.wlabel').textContent = label(w); b.setAttribute('aria-label', `${label(w)}. Arrow keys move it, Delete removes it.`); }
  }

  // theme-level: name, preset, hand-over summary, base
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
    const h = E.theme.handover || {};
    const endTxt = (END_CHOICES.find(c => c[0] === h.end) || END_CHOICES[0])[1];
    u.props.append(
      el('h3', { text: 'This theme' }),
      field('Name', nameIn, 'thName'), err,
      el('p', { class: 'muted small', text: widgets().length ? 'Click an item on the preview to change it.' : 'Add items from the left: drag them onto the preview, or click them.' }),
      el('div', { class: 'row' }, el('button', { class: 'btn secondary', text: 'Use the Standard layout', title: 'Clock, music and PC stats where they were before themes', onclick: usePreset })),
      el('hr', { class: 'sep' }),
      el('h3', { text: 'Hand-over' }),
      el('p', { class: 'small muted', text: `The live layer fades in over ${Number(h.fadeIn ?? 0.8).toFixed(1)} s. When CoolerScreen stops: ${endTxt.toLowerCase()}.` }),
      el('div', { class: 'row' }, el('button', { class: 'btn secondary', text: 'Hand-over settings…', onclick: () => setView('hand') })),
      el('hr', { class: 'sep' }),
      el('h3', { text: 'Base' }),
      el('p', { class: 'small muted', text: E.info.baseExists ? 'The base is the picture stored on the screen. It keeps playing when CoolerScreen is closed.' : 'The base picture has not been stored yet.' }),
      el('div', { class: 'row' },
        el('button', { class: 'btn secondary', text: 'Base settings…', onclick: () => ctx.openDetail(E.oid) }),
        el('button', { class: 'btn ghost', text: 'Store the base again', title: 'Make the base picture again and send it to the screen', onclick: () => ctx.applyTheme(E.oid, E.info.name) })));
  }
  async function usePreset() {
    if (widgets().length) {
      const ok = await openDialog({ title: 'Replace the items?', body: 'The Standard layout replaces the items in this theme with a clock, the music and the PC stats.',
        actions: [{ label: 'Cancel', value: false, kind: 'secondary' }, { label: 'Use Standard layout', value: true, kind: 'primary' }] });
      if (!ok) return;
    }
    try { const t = await call('theme_standard'); E.theme.widgets = t.widgets; E.sel = null; paintItems(); paintStage(); paintProps(); edited(); }
    catch (e) { toast(e.message, true); }
  }

  // ------------------------------------------------------------------ hand-over view
  function paintHand() {
    const u = E.ui, h = E.theme.handover || (E.theme.handover = {});
    u.hand.textContent = '';
    const img = url => (url ? el('img', { src: url, alt: '' }) : null);
    const baseUrl = E.baseUrl || fallbackFrame(), liveUrl = E.liveUrl || baseUrl;
    u.hand.append(
      el('h3', { text: 'When live starts' }),
      el('div', { class: 'hand-flow' },
        el('figure', {}, el('div', { class: 'frame' }, img(baseUrl)), el('figcaption', { text: 'The stored loop' })),
        el('span', { 'aria-hidden': 'true', text: '→' }),
        el('figure', {}, el('div', { class: 'frame' }, img(liveUrl)), el('figcaption', { text: 'The live layer fades in' }))),
      field('Fade-in time', range('hoFade', clamp(Number(h.fadeIn ?? 0.8), 0, 3), 0, 3, 0.1, v => { h.fadeIn = Math.round(v * 10) / 10; edited({ preview: false }); }, v => v.toFixed(1) + ' s'), 'hoFade'),
      el('h3', { text: 'When CoolerScreen stops' }));
    const cards = el('div', { class: 'hand-cards', role: 'radiogroup', 'aria-label': 'When CoolerScreen stops' });
    for (const [v, t, d] of END_CHOICES) {
      const r = el('input', { type: 'radio', name: 'hoEnd', value: v, checked: (h.end || 'loopStart') === v, onchange: () => { h.end = v; edited({ preview: false }); paintProps(); } });
      r.checked = (h.end || 'loopStart') === v;
      cards.append(el('label', {}, r, el('span', {}, el('b', { text: t }), el('br'), el('span', { class: 'muted small', text: d }))));
    }
    u.hand.append(cards,
      el('h3', { text: 'Music when nothing is playing' }),
      select2('hoMusic', h.musicIdle || 'hide', [['hide', 'Hide the music item'], ['keep', 'Keep it, showing an idle card']], v => { h.musicIdle = v; edited({ preview: false }); }));
  }

  return { enter, isOpen: () => !!E.ui, flush: () => (E.rev !== E.saved ? save() : Promise.resolve()) };
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
