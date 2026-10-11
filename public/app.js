// CoolerScreen UI: Library (originals) | Detail (preview, Tune, Versions) | Activity strip | Settings drawer.
import { W, H, frameGeometry, fitZoom, bandPx, styleNotch, makeCornersOverlay, styleCorners } from './js/frame.js';
import { openDialog } from './js/dialog.js';

// ---------- small helpers ----------
const $ = id => document.getElementById(id);
function el(tag, props = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v == null || v === false) continue;
    if (k === 'class') e.className = v;
    else if (k === 'text') e.textContent = v;
    else if (k === 'style') e.style.cssText = v;
    else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
    else if (k in e && !k.startsWith('aria') && k !== 'role' && !k.startsWith('data')) e[k] = v;
    else e.setAttribute(k, v === true ? '' : v);
  }
  for (const c of kids.flat()) if (c != null && c !== false) e.append(c);
  return e;
}
const mb = b => (b / 1e6).toFixed(1) + ' MB';
const mb2 = b => (b / 1e6).toFixed(2) + ' MB';
const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));
const r1 = v => Math.round(v * 10) / 10;
function fmtTime(s, tenths = true) {
  if (!Number.isFinite(s)) return '–';
  s = Math.max(0, s);
  const m = Math.floor(s / 60), sec = s - m * 60;
  return `${m}:${(tenths ? sec.toFixed(1) : Math.floor(sec).toString()).padStart(tenths ? 4 : 2, '0')}`;
}
const fmtSecs = s => (Math.round(s * 10) / 10).toString().replace(/\.0$/, '') + ' s';
function fmtWhen(iso) {
  const d = new Date(iso);
  if (isNaN(d)) return '';
  const now = new Date();
  const t = d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  if (d.toDateString() === now.toDateString()) return 'today ' + t;
  const y = new Date(now); y.setDate(now.getDate() - 1);
  if (d.toDateString() === y.toDateString()) return 'yesterday ' + t;
  return d.toLocaleDateString([], { day: 'numeric', month: 'short', year: d.getFullYear() === now.getFullYear() ? undefined : 'numeric' }) + ' ' + t;
}
const fmtDate = iso => new Date(iso).toLocaleDateString([], { day: 'numeric', month: 'short', year: 'numeric' });
function fmtDuration(sec) {
  if (sec < 60) return 'less than a minute';
  const m = Math.ceil(sec / 60);
  return `about ${m} minute${m > 1 ? 's' : ''}`;
}
function fmtLeft(sec) {
  sec = Math.max(0, Math.round(sec));
  const m = Math.floor(sec / 60), s = sec % 60;
  return m ? `about ${m} min ${s} s left` : `about ${s} s left`;
}

async function api(path, method = 'GET', body) {
  const r = await fetch(path, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) { const e = new Error(j.error || r.statusText); Object.assign(e, { status: r.status, code: j.code, failedUsb: !!j.failedUsb, data: j }); throw e; }
  return j;
}

let toastT;
function toast(msg, bad = false, ms = 5000) {
  const t = $('toast');
  t.textContent = msg; t.className = 'toast show' + (bad ? ' bad' : '');
  clearTimeout(toastT); toastT = setTimeout(() => (t.className = 'toast'), ms);
}

// ---------- state ----------
const S = {
  app: null, originals: [], longMissing: [], rev: -1, sel: null, detail: null,
  jobs: [], deviceBusy: false,
  screen: { state: 'checking', info: null, error: null },
  showing: null, // null = the tuning (original), or a version id
  draft: null, est: { state: 'idle' }, adding: [],
  missingDismissed: false, outcomes: new Map(), dismissed: new Set(), loadedAt: Date.now(),
};
const DEFAULT_FRAME = { zoom: 1, x: 0.5, y: 0.5 };

// ---------- top bar ----------
const PILL = {
  checking: 'Checking screen…', ready: 'Screen ready', setup: 'Needs one-time setup', failedUsb: 'Not connected',
  notFound: 'Screen not found', busy: 'Screen in use', updating: 'Updating screen…', error: 'Screen problem',
};
function pillState() {
  const updating = S.jobs.some(j => (j.kind === 'send' || j.kind === 'setup') && j.status === 'running');
  return updating ? 'updating' : S.screen.state;
}
function renderTop() {
  const st = pillState();
  $('pill').dataset.state = st;
  $('pillText').textContent = PILL[st] || st;
  const os = S.app && S.app.onScreen;
  const b = $('onScreen');
  if (!os) { b.hidden = true; return; }
  b.hidden = false;
  const img = $('onScreenThumb');
  if (os.thumbUrl) { img.src = os.thumbUrl; img.hidden = false; } else { img.removeAttribute('src'); img.hidden = true; }
  let name;
  if (!os.known) name = 'unknown (sent with another app or an older version)';
  else name = `${os.originalName} · ${os.label || ''}` + (os.exists ? '' : ' (deleted from library)');
  if (os.incomplete) name += ' – last upload didn\'t finish';
  $('onScreenName').textContent = name;
  b.title = os.known ? `${name}\nSent ${fmtWhen(os.at)}` : name;
  b.disabled = !os.exists;
}
$('onScreen').onclick = () => {
  const os = S.app && S.app.onScreen;
  if (os && os.exists && os.originalId) { select(os.originalId, os.versionId); }
};
$('pill').onclick = () => openDrawer('setScreen');
$('gear').onclick = () => openDrawer();

// ---------- banners ----------
function renderBanners() {
  const box = $('banners');
  box.textContent = '';
  const st = S.screen.state;
  const add = (kind, text, ...buttons) => box.append(el('div', { class: 'banner ' + kind }, el('p', { text }), ...buttons));
  if (st === 'setup') add('warn', 'Your screen will switch off 30 seconds after this app closes until you run the one-time setup.',
    el('button', { class: 'btn primary', text: 'Set up screen', onclick: runSetup }));
  if (st === 'failedUsb') add('danger', 'Windows couldn\'t connect to the screen when the PC started. This is a known glitch.',
    el('button', { class: 'btn primary', text: 'Reconnect', title: 'Windows will ask for permission', onclick: runReconnect }));
  if (st === 'notFound') add('danger', 'Screen not found. Check the cooler\'s USB cable and power, then try again.',
    el('button', { class: 'btn secondary', text: 'Check again', onclick: checkScreen }));
  if (st === 'busy') add('warn', 'Another program is using the screen. Close LOVINGCOOL MONITOR and try again.',
    el('button', { class: 'btn secondary', text: 'Check again', onclick: checkScreen }));
  const os = S.app && S.app.onScreen;
  if (os && os.incomplete && !S.jobs.some(j => j.kind === 'send' && (j.status === 'running' || j.status === 'waiting'))) {
    add('danger', `The last upload of "${os.originalName} · ${os.label}" didn't finish, so the screen may show a broken picture.`,
      os.exists ? el('button', { class: 'btn primary', text: 'Send again', onclick: () => confirmSend(os.versionId) }) : null);
  }
  if (S.longMissing.length && !S.missingDismissed) {
    const n = S.longMissing.length;
    add('info', `${n} clip${n > 1 ? 's have' : ' has'} been missing from the Gallery folder for more than ${S.app ? S.app.missingNoticeDays : 30} days. Their versions are still kept.`,
      el('button', { class: 'btn secondary', text: 'Review…', onclick: reviewMissing }),
      el('button', { class: 'btn ghost', text: 'Not now', onclick: () => { S.missingDismissed = true; renderBanners(); } }));
  }
}

async function reviewMissing() {
  const list = el('ul', {}, S.longMissing.map(m => el('li', { text: `${m.name} – missing since ${fmtDate(m.since)}` })));
  const ok = await openDialog({
    title: 'Delete versions of missing clips?',
    body: el('div', {}, el('p', { text: 'These files are no longer in the Gallery folder. If you moved them on purpose, their saved versions can be deleted. Nothing is deleted unless you choose to.' }), list),
    actions: [{ label: 'Keep them', value: false, kind: 'secondary' }, { label: 'Delete their versions', value: true, kind: 'danger' }],
  });
  S.missingDismissed = true;
  if (ok) {
    try { await api('/api/missing/delete', 'POST', { ids: S.longMissing.map(m => m.id) }); toast('Deleted.'); } catch (e) { toast(e.message, true); }
    await loadOriginals();
  }
  renderBanners();
}

// ---------- screen check (once at launch, after setup/reconnect, or on request; never polled) ----------
let checking = false;
async function checkScreen() {
  if (checking) return;
  checking = true;
  S.screen = { ...S.screen, state: 'checking' };
  renderTop(); renderBanners(); renderScreenSettings();
  try {
    const info = await api('/api/device');
    S.screen = { state: info._needsSetup ? 'setup' : 'ready', info, error: null };
  } catch (e) {
    if (e.status === 409) S.screen = { state: 'updating', info: null, error: null, retry: true };
    else S.screen = { state: e.failedUsb ? 'failedUsb' : e.code === 'notFound' ? 'notFound' : e.code === 'busy' ? 'busy' : 'error', info: null, error: e.message };
  }
  checking = false;
  renderTop(); renderBanners(); renderScreenSettings();
}

// ---------- data loading ----------
async function loadApp() {
  S.app = await api('/api/state');
  renderTop(); renderBanners();
}
async function loadOriginals() {
  const r = await api('/api/originals');
  S.originals = r.originals; S.longMissing = r.longMissing; S.rev = r.rev;
  if (S.sel && !S.originals.find(o => o.id === S.sel)) { S.sel = null; S.detail = null; }
  if (!S.sel && S.originals.length) S.sel = S.originals[0].id;
  renderLibrary();
  renderBanners();
  await loadDetail();
}
let detailFor = null;
async function loadDetail() {
  if (!S.sel) { S.detail = null; detailFor = null; renderDetail(true); return; }
  let d;
  try { d = await api('/api/originals/' + S.sel); } catch { d = null; }
  const fresh = detailFor !== S.sel || !S.detail || (S.detail.previewUrl !== (d && d.previewUrl)) || (S.detail.canTune !== (d && d.canTune));
  S.detail = d;
  if (detailFor !== S.sel) {
    S.showing = null;
    S.draft = initialDraft(d);
    S.est = { state: 'idle' };
  }
  detailFor = S.sel;
  renderDetail(fresh);
  if (fresh && d && d.canTune) scheduleEstimate(50);
}

function initialDraft(d) {
  if (!d) return null;
  const max = S.app ? S.app.maxSeconds : 10;
  const dr = d.draft || {};
  const dur = d.duration || 0;
  const sf = d.fps || 25, of = S.app ? S.app.deviceFps : 18;
  // Start on a source frame, length a whole number of screen frames.
  const start = d.isImage ? 0 : Math.round(clamp(Number(dr.start) || 0, 0, Math.max(0, dur - 0.05)) * sf) / sf;
  const len0 = d.isImage ? (S.app ? S.app.imageSeconds : 3) : clamp(Number(dr.length) || Math.min(max, dur || max), 1 / of, Math.max(1 / of, Math.min(max, (dur || max) - start)));
  const length = d.isImage ? len0 : Math.max(1, Math.floor(len0 * of + 1e-6)) / of;
  const cd = (S.app && S.app.cornersDefault) || { on: false, r: 80 };
  return { start: Math.round(start * 1000) / 1000, length: Math.round(length * 1000) / 1000, quality: dr.quality ?? 'auto', frame: { ...DEFAULT_FRAME, ...(dr.frame || {}) },
    corners: { on: !!(dr.corners || cd).on, r: (dr.corners || cd).r ?? 80 }, detail: dr.detail || 0, blend: dr.blend || 0 };
}

async function select(id, showVersion = null) {
  if (S.sel !== id) {
    S.sel = id;
    detailFor = null;
    renderLibrary();
    await loadDetail();
  }
  if (showVersion) showPreview(showVersion);
}

// ---------- library ----------
function renderLibrary() {
  const ul = $('libList');
  ul.textContent = '';
  for (const a of S.adding) {
    ul.append(el('li', { class: 'librow', role: 'option', 'aria-selected': 'false', 'aria-disabled': 'true' },
      el('div', { class: 'thumb skeleton' }), el('div', { class: 'txt' }, el('div', { class: 'name', text: a.name }),
        el('div', { class: 'sub' }, el('span', { class: 'spinner', 'aria-hidden': 'true' }), 'Adding…'))));
  }
  for (const o of S.originals) {
    const sub = el('div', { class: 'sub' });
    if (o.missing) sub.append(el('span', { class: 'chip warn', text: 'File missing' }));
    if (o.assets === 'error' && !o.versionCount) { sub.className = 'sub err'; sub.append('Can\'t read this file'); sub.title = o.assetsError || ''; }
    else if (o.busy) sub.append(el('span', { class: 'spinner', 'aria-hidden': 'true' }), 'Preparing version…');
    else sub.append(o.versionCount ? `${o.versionCount} version${o.versionCount > 1 ? 's' : ''}` : 'No versions yet');
    if (o.onScreen) sub.append(el('span', { class: 'chip ok', text: 'On screen' }));
    const thumb = o.assets === 'ready' ? el('img', { class: 'thumb', src: `/media/o/${o.id}/thumb.jpg?r=${encodeURIComponent(o.created)}`, alt: '', loading: 'lazy' })
      : el('div', { class: 'thumb' + (o.assets === 'pending' ? ' skeleton' : '') });
    const li = el('li', { class: 'librow', role: 'option', id: 'opt-' + o.id, 'aria-selected': String(o.id === S.sel), 'data-id': o.id, title: o.name },
      thumb, el('div', { class: 'txt' }, el('div', { class: 'name', text: o.name }), sub));
    li.onclick = () => select(o.id);
    ul.append(li);
  }
  if (S.sel) ul.setAttribute('aria-activedescendant', 'opt-' + S.sel); else ul.removeAttribute('aria-activedescendant');
}
$('libList').addEventListener('keydown', e => {
  const i = S.originals.findIndex(o => o.id === S.sel);
  let n = null;
  if (e.key === 'ArrowDown') n = Math.min(S.originals.length - 1, i + 1);
  else if (e.key === 'ArrowUp') n = Math.max(0, i - 1);
  else if (e.key === 'Home') n = 0;
  else if (e.key === 'End') n = S.originals.length - 1;
  else if (e.key === 'Enter') { e.preventDefault(); const t = document.querySelector('.detail-title'); if (t) t.focus(); return; }
  if (n == null || n < 0) return;
  e.preventDefault();
  const o = S.originals[n];
  select(o.id).then(() => { const r = $('opt-' + o.id); if (r) r.scrollIntoView({ block: 'nearest' }); });
});

// ---------- import ----------
$('addBtn').onclick = () => $('fileInput').click();
$('fileInput').onchange = e => { importFiles([...e.target.files]); e.target.value = ''; };
$('openFolder').onclick = () => api('/api/gallery/open', 'POST').catch(e => toast(e.message, true));

async function importFiles(files) {
  if (!files.length) { toast('Nothing to add: drop a file from your computer (not a link or text).', true); return; }
  const items = files.map(f => ({ name: f.name, file: f }));
  S.adding.push(...items);
  renderLibrary();
  let lastId = null;
  for (const it of items) {
    try {
      const localPath = window.cs && window.cs.pathFor ? window.cs.pathFor(it.file) : '';
      const res = localPath
        ? await fetch('/api/import-path', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ path: localPath }) })
        : await fetch('/api/import?' + new URLSearchParams({ name: it.file.name }), { method: 'POST', body: it.file });
      const j = await res.json().catch(() => ({}));
      if (!res.ok) throw new Error(j.error || 'Could not add the file.');
      lastId = j.id;
    } catch (e) {
      openDialog({ title: 'Couldn\'t add this file', body: /can't read/i.test(e.message) ? e.message : `CoolerScreen can't read "${it.name}". It may be damaged or in a format that isn't supported.`, actions: [{ label: 'OK', value: true, kind: 'primary' }] });
    }
    S.adding.splice(S.adding.indexOf(it), 1);
  }
  await loadOriginals();
  if (lastId) await select(lastId);
}
let dragDepth = 0;
window.addEventListener('dragenter', e => { if (e.dataTransfer && [...e.dataTransfer.types].includes('Files')) { dragDepth++; $('dropHint').hidden = false; } });
window.addEventListener('dragleave', () => { dragDepth = Math.max(0, dragDepth - 1); if (!dragDepth) $('dropHint').hidden = true; });
window.addEventListener('dragover', e => e.preventDefault());
window.addEventListener('drop', e => {
  e.preventDefault(); dragDepth = 0; $('dropHint').hidden = true;
  importFiles([...(e.dataTransfer?.files || [])]);
});

// ---------- detail ----------
let refs = {}; // elements of the current detail view
function renderDetail(full) {
  const main = $('detail');
  const d = S.detail;
  if (!d) {
    stopPreviewLoop();
    refs = {};
    main.textContent = '';
    if (!S.originals.length && !S.adding.length) main.append(emptyState());
    return;
  }
  if (full || !refs.root) buildDetail(d);
  updateDetail(d);
}

function emptyState() {
  return el('div', { class: 'empty' },
    el('div', { class: 'screenframe', 'aria-hidden': 'true' }),
    el('h2', { text: 'Put your own picture on the cooler screen' }),
    el('p', { class: 'muted', text: 'Drop a video, GIF, image or LOVINGCOOL theme (.ht) here,' }),
    el('div', { class: 'row' }, el('button', { class: 'btn primary', text: 'Add clip', onclick: () => $('fileInput').click() }),
      el('button', { class: 'btn ghost', text: 'Open Gallery folder', onclick: () => $('openFolder').click() })),
    el('p', { class: 'muted small', text: 'You\'ll adjust it and preview it before anything is sent.' }));
}

function buildDetail(d) {
  stopPreviewLoop();
  const main = $('detail');
  main.textContent = '';
  refs = { root: main }; // root marks a built view: later updates re-render parts, not the whole view
  // header
  refs.title = el('h1', { class: 'detail-title', tabindex: '-1' });
  refs.meta = el('div', { class: 'meta' });
  refs.titles = el('div', { class: 'titles' }, refs.title, refs.meta);
  refs.renameBtn = el('button', { class: 'btn secondary', text: 'Rename', onclick: startRename });
  refs.more = menuButton('More actions for this clip', () => {
    const o = S.detail;
    const items = [];
    if (o.file && !o.missing) items.push({ label: 'Show in Gallery folder', run: () => api(`/api/originals/${o.id}/reveal`, 'POST') });
    items.push({ label: o.missing ? 'Remove from library…' : 'Delete…', danger: true, run: confirmDeleteOriginal });
    return items;
  });
  main.append(el('div', { class: 'detail-head' }, refs.titles, refs.renameBtn, refs.more));
  refs.notices = el('div');
  main.append(refs.notices);

  // preview
  refs.stage = el('div', { class: 'screenframe', tabindex: '0', role: 'group', 'aria-label': 'Preview of the screen. Space plays or pauses.' });
  refs.notch = el('div', { class: 'notch', hidden: true, title: 'Your screen\'s camera hole is here. This area is kept black.' });
  refs.corners = makeCornersOverlay();
  refs.playBtn = el('button', { class: 'iconbtn', 'aria-label': 'Pause', onclick: togglePlay }, '❚❚');
  refs.time = el('span', { class: 'num' });
  refs.showing = el('span', { class: 'grow' });
  refs.notchToggle = el('input', { type: 'checkbox', id: 'notchOutline' });
  refs.notchToggle.onchange = () => refs.stage.classList.toggle('outline', refs.notchToggle.checked);
  refs.backToTune = el('button', { class: 'btn ghost', text: 'Back to your tuning', hidden: true, onclick: () => showPreview(null) });
  main.append(el('div', { class: 'preview-wrap' }, refs.stage,
    el('div', { class: 'preview-bar' }, refs.playBtn, refs.time, refs.showing, refs.backToTune,
      el('label', { class: 'check', title: 'Your screen\'s camera hole is here. This area is kept black.' }, refs.notchToggle, 'Show notch'))));
  refs.stage.addEventListener('keydown', e => { if (e.key === ' ') { e.preventDefault(); togglePlay(); } });
  setupPan();

  // tune + versions
  refs.tune = el('section', { class: 'panel', 'aria-labelledby': 'hTune' });
  refs.versions = el('section', { class: 'panel', 'aria-labelledby': 'hVersions' });
  main.append(el('div', { class: 'work' }, refs.tune, refs.versions));
  buildTune(d);
  showPreview(S.showing, true);
}

function updateDetail(d) {
  refs.title.textContent = d.name;
  const parts = [d.fileName || ''];
  parts.push(d.isImage ? 'Still image' : d.duration ? fmtTime(d.duration, false) : '');
  if (d.width) parts.push(`${d.width}x${d.height}`);
  refs.meta.textContent = parts.filter(Boolean).join(' · ');
  refs.renameBtn.disabled = false;
  // notices
  refs.notices.textContent = '';
  if (d.missing) refs.notices.append(el('div', { class: 'notice' },
    `The original file is no longer in the Gallery folder (missing since ${fmtDate(d.missing.since)}). Its versions are kept and can still be sent` +
    (d.canTune ? '.' : ', but new versions can\'t be made until the file is back.')));
  else if (d.sourceChanged) refs.notices.append(el('div', { class: 'notice info', text: 'The original file has changed since its versions were made. They can still be sent; new versions use the changed file.' }));
  if (d.assets === 'error') refs.notices.append(el('div', { class: 'notice danger' }, 'CoolerScreen can\'t read this file. It may be damaged or in a format that isn\'t supported.',
    el('details', {}, el('summary', { text: 'Show details' }), el('pre', { text: d.assetsError || '' }))));
  renderVersions(d);
  updateTuneState();
  updateShowingLabel();
}

// ----- rename (inline) -----
function startRename() {
  const d = S.detail;
  const input = el('input', { type: 'text', value: d.name, 'aria-label': 'Clip name' });
  const err = el('div', { class: 'field-err', role: 'alert' });
  refs.titles.replaceChildren(input, err);
  refs.renameBtn.disabled = true;
  input.focus(); input.select();
  let done = false;
  const finish = async save => {
    if (done) return;
    if (save && input.value.trim() && input.value.trim() !== d.name) {
      try { S.detail = await api('/api/originals/' + d.id, 'PATCH', { name: input.value.trim() }); }
      catch (e) { err.textContent = e.message; input.focus(); return; }
    }
    done = true;
    refs.titles.replaceChildren(refs.title, refs.meta);
    updateDetail(S.detail);
    refs.title.focus();
    loadOriginals();
  };
  input.addEventListener('keydown', e => { if (e.key === 'Enter') finish(true); if (e.key === 'Escape') { e.stopPropagation(); finish(false); } });
  input.addEventListener('blur', () => setTimeout(() => { if (!done && document.activeElement !== input) finish(true); }, 150));
}

async function confirmDeleteOriginal() {
  const d = S.detail;
  const n = d.versions.length;
  const onScreen = d.versions.some(v => v.onScreen);
  const lines = [];
  if (d.missing || !d.file) lines.push(`Its ${n} version${n === 1 ? '' : 's'} will be deleted.`);
  else lines.push(`The file "${d.fileName}" goes to the Recycle Bin and ${n ? `its ${n} version${n === 1 ? '' : 's'} ${n === 1 ? 'is' : 'are'}` : 'nothing else is'} deleted.`);
  if (onScreen) lines.push('The screen keeps playing it until you send something else.');
  const ok = await openDialog({
    title: `${d.missing ? 'Remove' : 'Delete'} "${d.name}"?`, body: el('div', {}, lines.map(t => el('p', { text: t }))),
    actions: [{ label: 'Cancel', value: false, kind: 'secondary' }, { label: d.missing ? 'Remove' : 'Delete', value: true, kind: 'danger' }],
  });
  if (!ok) return;
  try { await api('/api/originals/' + d.id, 'DELETE'); } catch (e) { toast(e.message, true, 10000); return; }
  S.sel = null; detailFor = null;
  await loadOriginals();
  await loadApp();
}

// ----- menus -----
function menuButton(label, itemsFn) {
  const btn = el('button', { class: 'iconbtn', 'aria-label': label, 'aria-haspopup': 'menu', 'aria-expanded': 'false', text: '⋯' });
  const wrap = el('div', { class: 'menu-wrap' }, btn);
  let menu = null;
  const close = (focus = true) => {
    if (!menu) return;
    menu.remove(); menu = null; btn.setAttribute('aria-expanded', 'false');
    document.removeEventListener('mousedown', outside, true);
    if (focus) btn.focus();
  };
  const outside = e => { if (menu && !wrap.contains(e.target)) close(false); };
  btn.onclick = e => {
    e.stopPropagation();
    if (menu) return close();
    menu = el('div', { class: 'menu', role: 'menu' });
    for (const it of itemsFn()) {
      menu.append(el('button', { role: 'menuitem', class: it.danger ? 'danger' : '', text: it.label, tabindex: '-1',
        onclick: ev => { ev.stopPropagation(); close(false); Promise.resolve(it.run()).catch(err => toast(err.message, true)); } }));
    }
    menu.addEventListener('keydown', ev => {
      const items = [...menu.querySelectorAll('button')];
      const i = items.indexOf(document.activeElement);
      if (ev.key === 'ArrowDown') { ev.preventDefault(); items[(i + 1) % items.length].focus(); }
      if (ev.key === 'ArrowUp') { ev.preventDefault(); items[(i - 1 + items.length) % items.length].focus(); }
      if (ev.key === 'Escape') { ev.preventDefault(); ev.stopPropagation(); close(); }
      if (ev.key === 'Tab') close(false);
    });
    menu.addEventListener('click', ev => ev.stopPropagation());
    wrap.append(menu);
    btn.setAttribute('aria-expanded', 'true');
    document.addEventListener('mousedown', outside, true);
    menu.querySelector('button').focus();
  };
  return wrap;
}

// ---------- preview ----------
let media = null, blendVid = null, rafId = 0, natural = { w: 0, h: 0 }, scrubbing = false;
const play = { mode: 'selection', phase: 'end' }; // mode: 'selection' | 'seam'
function stopPreviewLoop() { cancelAnimationFrame(rafId); rafId = 0; }
const OUT_FPS = () => (S.app ? S.app.deviceFps : 18);
const srcFps = () => (S.detail && S.detail.fps) || 25;
const r3 = v => Math.round(v * 1000) / 1000;
function fmtT2(s) {
  if (!Number.isFinite(s)) return '–';
  s = Math.max(0, s);
  const m = Math.floor(s / 60), sec = s - m * 60;
  return `${m}:${sec.toFixed(2).padStart(5, '0')}`;
}

function showPreview(vid, force = false) {
  const d = S.detail;
  if (!d || !refs.stage) return;
  if (!force && S.showing === vid && media) return;
  S.showing = vid;
  stopPreviewLoop();
  refs.stage.querySelectorAll('.media, .empty-msg').forEach(n => n.remove());
  media = null; blendVid = null;
  const v = vid ? d.versions.find(x => x.id === vid) : null;
  refs.stage.classList.toggle('version', !!v);
  if (v) {
    media = el('video', { class: 'media', src: `/media/v/${v.id}/preview.mp4`, muted: true, loop: true, autoplay: true, playsInline: true });
    media.muted = true;
    refs.stage.prepend(media);
    refs.notch.hidden = true;
    media.play().catch(() => {});
  } else if (d.previewUrl) {
    if (d.previewKind === 'image') {
      media = el('img', { class: 'media', src: d.previewUrl, alt: '' });
      media.onload = () => { natural = { w: media.naturalWidth, h: media.naturalHeight }; applyGeometry(); };
    } else {
      media = el('video', { class: 'media', src: d.previewUrl, muted: true, autoplay: true, playsInline: true, preload: 'auto' });
      media.muted = true;
      media.addEventListener('loadedmetadata', () => {
        natural = { w: media.videoWidth, h: media.videoHeight };
        applyGeometry();
        media.currentTime = S.draft.start;
        media.play().catch(() => {});
        drawFilmstrip();
      });
      // Second copy of the source for the loop blend: shows the frames from just before the start, fading in.
      blendVid = el('video', { class: 'media blend', src: d.previewUrl, muted: true, playsInline: true, preload: 'auto', 'aria-hidden': 'true' });
      blendVid.muted = true;
      blendVid.style.opacity = '0';
    }
    natural = { w: d.width || W, h: d.height || H };
    if (blendVid) refs.stage.prepend(blendVid);
    refs.stage.prepend(media);
    applyGeometry();
  } else {
    refs.stage.prepend(el('div', { class: 'empty-msg', text: d.assets === 'error' ? 'No preview: this file can\'t be read.' : d.missing ? 'The original file is missing.' : 'Preparing the preview…' }));
  }
  if (!v && S.app) styleNotch(refs.notch, liveMask());
  applyCorners();
  refs.stage.append(refs.notch, refs.corners);
  if (refs.stage.classList.contains('outline')) refs.notch.classList.add('outline');
  refs.stage.classList.toggle('pannable', !v && !!d.canTune);
  refs.backToTune.hidden = !v || !d.canTune;
  updateShowingLabel();
  renderVersionsHighlight();
  rafId = requestAnimationFrame(tickPreview);
}

// Corners are part of each version: the tuning shows the draft's corners; a version preview has them baked in.
function applyCorners() {
  if (!refs.corners) return;
  if (S.showing || !S.draft) { refs.corners.style.display = 'none'; return; }
  styleCorners(refs.corners, { corners: S.draft.corners });
}

function applyGeometry() {
  if (!media || S.showing || !S.draft || !natural.w) return;
  const g = frameGeometry(natural.w, natural.h, S.draft.frame);
  const st = { left: (g.left / W) * 100 + '%', top: (g.top / H) * 100 + '%', width: (g.width / W) * 100 + '%', height: (g.height / H) * 100 + '%' };
  // "Simplify fine detail" = Gaussian blur on the 2400x1080 canvas (src/convert.js detailSigma); same deviation here.
  const sigma = (Math.min(100, Math.max(0, S.draft.detail || 0)) / 100) * 3.5 * (refs.stage.clientWidth / W);
  st.filter = sigma > 0 ? `blur(${sigma.toFixed(2)}px)` : '';
  Object.assign(media.style, st);
  if (blendVid) Object.assign(blendVid.style, st);
  updateFrameControls();
}
window.addEventListener('resize', () => applyGeometry());

// Looping is driven by animation frames; a timer takes over when they are throttled (e.g. a hidden window).
let lastTick = 0;
setInterval(() => { if (rafId && performance.now() - lastTick > 120) tickBody(); }, 40);
function tickPreview() {
  rafId = requestAnimationFrame(tickPreview);
  tickBody();
}
function tickBody() {
  lastTick = performance.now();
  if (!refs.playBtn) return;
  refs.playBtn.hidden = !media || media.tagName !== 'VIDEO';
  if (!media || media.tagName !== 'VIDEO') { if (refs.time) refs.time.textContent = S.detail && S.detail.isImage && !S.showing ? `Still image · ${fmtSecs(S.draft.length)} on the screen` : ''; return; }
  const playing = !media.paused;
  refs.playBtn.textContent = playing ? '❚❚' : '▶';
  refs.playBtn.setAttribute('aria-label', playing ? 'Pause' : 'Play');
  if (S.showing) {
    refs.time.textContent = `${fmtTime(media.currentTime)} / ${fmtTime(media.duration)}`;
    return;
  }
  const { start, length } = S.draft;
  const end = start + length;
  const ct = media.currentTime;
  if (!scrubbing) {
    if (play.mode === 'seam') {
      // The seam: last second of the selection, then the first second, over and over.
      const a = Math.max(start, end - 1), b = Math.min(end, start + 1);
      if (play.phase === 'end' && (ct >= end - 0.01 || ct < a - 0.25)) { media.currentTime = start; play.phase = 'start'; }
      else if (play.phase === 'start' && (ct >= b - 0.01 || ct < start - 0.25)) { media.currentTime = a; play.phase = 'end'; }
    } else if (ct >= end - 0.01 || ct < start - 0.25) media.currentTime = start;
  }
  updateBlendOverlay(media.currentTime, playing);
  const rel = media.currentTime - start;
  refs.time.textContent = `${fmtT2(Math.max(0, rel))} / ${fmtT2(length)} · frame ${Math.min(Math.round(length * OUT_FPS()), Math.floor(Math.max(0, rel) * OUT_FPS() + 1e-6) + 1)}`;
  if (refs.playhead && S.detail.duration) { refs.playhead.style.left = trackPct(media.currentTime); }
}

/** Effective blend (as the converter will apply it): whole output frames, <= half the selection, <= time before start. */
function effectiveBlend() {
  const dr = S.draft;
  if (!dr || S.detail.isImage) return 0;
  const b = Math.max(0, Math.min(dr.blend || 0, 1.5, dr.length / 2, dr.start));
  return Math.floor(b * OUT_FPS() + 1e-6) / OUT_FPS();
}
function updateBlendOverlay(ct, playing) {
  if (!blendVid) return;
  const B = effectiveBlend();
  const end = S.draft.start + S.draft.length;
  if (B > 0 && ct >= end - B && ct < end) {
    const p = (ct - (end - B)) / B;
    const target = S.draft.start - B + (ct - (end - B));
    if (Math.abs(blendVid.currentTime - target) > 0.12 || (playing && blendVid.paused) || (!playing && Math.abs(blendVid.currentTime - target) > 0.02)) {
      blendVid.currentTime = target;
    }
    if (playing && blendVid.paused) blendVid.play().catch(() => {});
    if (!playing && !blendVid.paused) blendVid.pause();
    blendVid.style.opacity = String(p);
  } else {
    blendVid.style.opacity = '0';
    if (!blendVid.paused) blendVid.pause();
  }
}

function togglePlay() {
  if (!media || media.tagName !== 'VIDEO') return;
  if (media.paused) media.play().catch(() => {}); else media.pause();
}

function updateShowingLabel() {
  if (!refs.showing) return;
  const d = S.detail;
  const v = S.showing && d && d.versions.find(x => x.id === S.showing);
  if (v) refs.showing.textContent = `Showing: ${v.label || 'v' + v.n}${v.onScreen ? ' · on the screen now' : ''}`;
  else refs.showing.textContent = d && d.canTune ? 'Showing: your tuning (not saved yet)' : d && d.previewUrl ? 'Showing: the original' : '';
}

// drag the picture inside the frame to move it
function setupPan() {
  let drag = null;
  refs.stage.addEventListener('pointerdown', e => {
    if (S.showing || !S.detail.canTune || !media || e.button !== 0) return;
    const g = frameGeometry(natural.w, natural.h, S.draft.frame);
    drag = { x: e.clientX, y: e.clientY, fx: S.draft.frame.x, fy: S.draft.frame.y, g, scale: W / refs.stage.clientWidth };
    refs.stage.setPointerCapture(e.pointerId);
    refs.stage.classList.add('dragging');
  });
  refs.stage.addEventListener('pointermove', e => {
    if (!drag) return;
    const dx = (e.clientX - drag.x) * drag.scale, dy = (e.clientY - drag.y) * drag.scale;
    const f = S.draft.frame;
    if (drag.g.width !== W) f.x = clamp(drag.fx + dx / (W - drag.g.width), 0, 1);
    if (drag.g.height !== H) f.y = clamp(drag.fy + dy / (H - drag.g.height), 0, 1);
    f.x = Math.round(f.x * 1000) / 1000; f.y = Math.round(f.y * 1000) / 1000;
    applyGeometry();
  });
  const end = () => { if (!drag) return; drag = null; refs.stage.classList.remove('dragging'); draftChanged(); };
  refs.stage.addEventListener('pointerup', end);
  refs.stage.addEventListener('pointercancel', end);
}

// ---------- frame grabbing (for the filmstrip and the seam check; reads the same media the preview plays) ----------
let grabber = null, grabChain = Promise.resolve();
function grabFrame(time, w, h) {
  const url = S.detail && S.detail.previewUrl;
  const run = () => new Promise(resolve => {
    if (!url) return resolve(null);
    if (!grabber || grabber.dataset.src !== url) {
      grabber = el('video', { muted: true, preload: 'auto', playsInline: true });
      grabber.muted = true; grabber.dataset.src = url; grabber.src = url;
    }
    const v = grabber;
    const draw = () => {
      const c = document.createElement('canvas');
      c.width = w; c.height = h;
      try { c.getContext('2d').drawImage(v, 0, 0, w, h); } catch { return resolve(null); }
      resolve(c);
    };
    const seek = () => {
      // Like ffmpeg's fps filter (round to the nearest source frame); +1 ms lands inside that frame.
      const t = clamp(Math.round(time * srcFps()) / srcFps() + 0.001, 0, Math.max(0, (v.duration || time) - 0.001));
      if (Math.abs(v.currentTime - t) < 0.0005 && v.readyState >= 2) return draw();
      const done = () => { v.removeEventListener('seeked', done); clearTimeout(to); draw(); };
      const to = setTimeout(done, 3000);
      v.addEventListener('seeked', done);
      v.currentTime = t;
    };
    if (v.readyState >= 1) seek(); else v.addEventListener('loadedmetadata', seek, { once: true });
  });
  grabChain = grabChain.then(run, run);
  return grabChain;
}

// ---------- tune ----------
function buildTune(d) {
  const t = refs.tune;
  t.textContent = '';
  t.append(el('h3', { id: 'hTune', text: 'Tune' }));
  if (!d.canTune) {
    t.append(el('p', { class: 'muted small', text: d.assets === 'pending' ? 'Preparing the preview… You can tune the clip in a moment.' :
      d.missing ? 'The original file is missing, so new versions can\'t be made.' : 'This file can\'t be read, so versions can\'t be made.' }));
    return;
  }
  const dr = S.draft;
  // 1. loop: trim, play modes, seam check, blend
  if (!d.isImage && d.duration) t.append(buildLoopGroup(d));
  else {
    t.append(el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Part of the clip' }),
      el('p', { class: 'muted small', text: `Still image: it is shown as a ${fmtSecs(dr.length)} clip that repeats.` })));
  }
  // 2. framing ("size") and rounded corners
  const fg = el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Size and position on the screen' }));
  refs.zoom = el('input', { type: 'range', id: 'fZoom', min: '0.2', max: '4', step: '0.01' });
  refs.zoomVal = el('span', { class: 'num small muted', style: 'min-width:48px' });
  refs.posX = el('input', { type: 'range', id: 'fX', min: '0', max: '1', step: '0.001' });
  refs.posY = el('input', { type: 'range', id: 'fY', min: '0', max: '1', step: '0.001' });
  refs.cornersOn = el('input', { type: 'checkbox', id: 'cOn' });
  refs.cornersR = el('input', { type: 'range', id: 'cR', min: '0', max: '300', step: '1', 'aria-label': 'Corner radius (pixels)' });
  refs.cornersRN = el('input', { type: 'number', id: 'cRN', min: '0', max: '300', step: '1', 'aria-label': 'Corner radius in pixels' });
  refs.cornersRow = el('div', { class: 'slider-row' }, el('label', { for: 'cR', class: 'small muted', text: 'Corner radius' }), refs.cornersR, refs.cornersRN);
  fg.append(
    el('div', { class: 'slider-row' }, el('label', { for: 'fZoom', class: 'small muted', text: 'Zoom' }), refs.zoom, refs.zoomVal),
    el('div', { class: 'slider-row' }, el('label', { for: 'fX', class: 'small muted', text: 'Left – right' }), refs.posX),
    el('div', { class: 'slider-row' }, el('label', { for: 'fY', class: 'small muted', text: 'Up – down' }), refs.posY),
    el('div', { class: 'row' },
      el('button', { class: 'btn secondary', text: 'Fill the screen', onclick: () => { dr.frame = { ...DEFAULT_FRAME }; applyGeometry(); draftChanged(); } }),
      el('button', { class: 'btn secondary', text: 'Show the whole picture', onclick: () => { dr.frame = { zoom: Math.round(fitZoom(natural.w, natural.h) * 1000) / 1000, x: 0.5, y: 0.5 }; applyGeometry(); draftChanged(); } }),
      el('span', { class: 'muted small', text: 'Tip: drag the preview to move the picture.' })),
    el('label', { class: 'check' }, refs.cornersOn, 'Rounded corners'), refs.cornersRow);
  refs.zoom.oninput = () => { dr.frame.zoom = Number(refs.zoom.value); applyGeometry(); };
  refs.posX.oninput = () => { dr.frame.x = Number(refs.posX.value); applyGeometry(); };
  refs.posY.oninput = () => { dr.frame.y = Number(refs.posY.value); applyGeometry(); };
  for (const r of [refs.zoom, refs.posX, refs.posY]) r.onchange = draftChanged;
  const onCorners = (fromNum) => {
    if (fromNum) refs.cornersR.value = refs.cornersRN.value; else refs.cornersRN.value = refs.cornersR.value;
    dr.corners = { on: refs.cornersOn.checked, r: clamp(Math.round(Number(refs.cornersR.value) || 0), 0, 300) };
    refs.cornersRow.hidden = !dr.corners.on;
    applyCorners();
  };
  refs.cornersOn.onchange = () => { onCorners(false); draftChanged(); };
  refs.cornersR.oninput = () => onCorners(false);
  refs.cornersRN.oninput = () => onCorners(true);
  refs.cornersR.onchange = refs.cornersRN.onchange = draftChanged;
  t.append(fg);
  // 3. quality + simplify fine detail
  const qg = el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Quality' }));
  refs.qAuto = el('input', { type: 'radio', name: 'q', value: 'auto' });
  refs.qCustom = el('input', { type: 'radio', name: 'q', value: 'custom' });
  refs.qSlider = el('input', { type: 'range', id: 'qVal', min: '1', max: '100', step: '1', 'aria-label': 'Custom quality' });
  refs.qNum = el('span', { class: 'num', style: 'min-width:28px' });
  refs.qRow = el('div', { class: 'slider-row' }, el('span', { class: 'small muted', text: 'Smaller file' }), refs.qSlider, el('span', { class: 'small muted', text: 'Sharper' }), refs.qNum);
  refs.detail = el('input', { type: 'range', id: 'dVal', min: '0', max: '100', step: '5' });
  refs.detailNum = el('span', { class: 'num', style: 'min-width:28px' });
  qg.append(el('label', { class: 'radio' }, refs.qAuto, 'Best that fits (recommended)'),
    el('label', { class: 'radio' }, refs.qCustom, 'Custom'), refs.qRow,
    el('div', { class: 'slider-row', style: 'margin-top:8px' }, el('label', { for: 'dVal', class: 'small muted', text: 'Simplify fine detail' }), refs.detail, refs.detailNum),
    el('p', { class: 'muted small', text: 'Softens tiny detail so more of the clip fits (0 = off). The preview shows the effect.' }));
  const onQ = () => {
    dr.quality = refs.qAuto.checked ? 'auto' : Number(refs.qSlider.value);
    refs.qRow.hidden = refs.qAuto.checked;
    refs.qNum.textContent = refs.qSlider.value;
    draftChanged();
  };
  refs.qAuto.onchange = refs.qCustom.onchange = onQ;
  refs.qSlider.oninput = () => { refs.qNum.textContent = refs.qSlider.value; };
  refs.qSlider.onchange = onQ;
  refs.detail.oninput = () => { dr.detail = Number(refs.detail.value); refs.detailNum.textContent = dr.detail || 'off'; applyGeometry(); };
  refs.detail.onchange = draftChanged;
  t.append(qg);
  // 4. fit meter
  refs.meterState = el('span', { class: 'meter-state' });
  refs.meter = el('div', { class: 'meter', role: 'img' }, el('div', { class: 'fill' }), el('div', { class: 'over', hidden: true }));
  refs.meterText = el('div', { class: 'small muted num' });
  refs.meterNote = el('div', { class: 'small muted' });
  refs.meterRetry = el('button', { class: 'btn ghost', text: 'Retry', hidden: true, onclick: () => scheduleEstimate(0) });
  t.append(el('div', { class: 'group' }, el('div', { class: 'meter-head' }, el('span', { class: 'group-label', text: 'Space on the screen' }), refs.meterState),
    el('div', { class: 'meter-wrap' }, refs.meter), refs.meterText, refs.meterNote, refs.meterRetry));
  // 5. actions
  refs.saveBtn = el('button', { class: 'btn primary big', text: 'Save as version', onclick: saveVersion });
  refs.saveReason = el('span', { class: 'reason', role: 'status' });
  refs.saveFix = el('button', { class: 'btn secondary', hidden: true });
  t.append(el('div', { class: 'actions' }, refs.saveBtn,
    el('button', { class: 'btn ghost', text: 'Reset', onclick: resetDraft }), refs.saveReason, refs.saveFix));
  syncTuneControls();
}

// ----- loop editing -----
const view = { zoomed: false, a: 0, b: 0 }; // visible part of the trim track (seconds)
function buildLoopGroup(d) {
  const g = el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Part of the clip (the loop)' }));
  refs.zoomTrack = el('button', { class: 'btn ghost', 'aria-pressed': 'false', text: 'Zoom to the selection', onclick: () => { view.zoomed = !view.zoomed; fitView(); positionTrim(); drawFilmstrip(); } });
  refs.trim = el('div', { class: 'trim' });
  refs.film = el('canvas', { class: 'filmstrip', 'aria-hidden': 'true' });
  refs.trimRange = el('div', { class: 'range', title: 'Drag to move the selected part' });
  refs.hStart = el('div', { class: 'handle', role: 'slider', tabindex: '0', 'aria-label': 'Start of the loop. Arrow keys move one frame, Shift+arrow ten frames.' });
  refs.hEnd = el('div', { class: 'handle', role: 'slider', tabindex: '0', 'aria-label': 'End of the loop. Arrow keys move one frame, Shift+arrow ten frames.' });
  refs.playhead = el('div', { class: 'playhead', 'aria-hidden': 'true' });
  refs.trim.append(refs.film, refs.trimRange, refs.playhead, refs.hStart, refs.hEnd);
  refs.viewLabel = el('span', { class: 'muted small num' });
  // start / end rows: frame buttons, numbers
  const nudgeBtn = (label, aria, fn) => el('button', { class: 'btn secondary nudge', text: label, 'aria-label': aria, onclick: e => fn(e.shiftKey ? 10 : 1) });
  refs.inStartF = el('input', { type: 'number', id: 'tStartF', min: '0', step: '1' });
  refs.inLenF = el('input', { type: 'number', id: 'tLenF', min: '1', step: '1' });
  refs.startInfo = el('span', { class: 'muted small num' });
  refs.endInfo = el('span', { class: 'muted small num' });
  const startRow = el('div', { class: 'frame-row' },
    el('span', { class: 'frame-row-label', text: 'Start' }),
    nudgeBtn('◀ frame', 'Start one frame earlier', n => nudgeStart(-n)), nudgeBtn('frame ▶', 'Start one frame later', n => nudgeStart(n)),
    el('label', { for: 'tStartF', class: 'small muted', text: 'source frame' }), refs.inStartF, refs.startInfo);
  const endRow = el('div', { class: 'frame-row' },
    el('span', { class: 'frame-row-label', text: 'End' }),
    nudgeBtn('◀ frame', 'End one frame earlier', n => nudgeEnd(-n)), nudgeBtn('frame ▶', 'End one frame later', n => nudgeEnd(n)),
    el('label', { for: 'tLenF', class: 'small muted', text: `length in frames (${OUT_FPS()} per second)` }), refs.inLenF, refs.endInfo);
  // play mode
  refs.pmSel = el('input', { type: 'radio', name: 'pm', value: 'selection', checked: play.mode === 'selection' });
  refs.pmSeam = el('input', { type: 'radio', name: 'pm', value: 'seam', checked: play.mode === 'seam' });
  const onMode = () => {
    play.mode = refs.pmSeam.checked ? 'seam' : 'selection';
    if (media && media.tagName === 'VIDEO') {
      const end = S.draft.start + S.draft.length;
      if (play.mode === 'seam') { play.phase = 'end'; media.currentTime = Math.max(S.draft.start, end - 1); } else media.currentTime = S.draft.start;
      media.play().catch(() => {});
    }
  };
  refs.pmSel.onchange = refs.pmSeam.onchange = onMode;
  const modes = el('div', { class: 'row', role: 'radiogroup', 'aria-label': 'Preview plays' },
    el('span', { class: 'small muted', text: 'Preview plays:' }),
    el('label', { class: 'radio' }, refs.pmSel, 'the selection'),
    el('label', { class: 'radio' }, refs.pmSeam, 'the seam (last second, then first second)'));
  // blend
  refs.blend = el('input', { type: 'range', id: 'bVal', min: '0', max: '1.5', step: String(1 / OUT_FPS()) });
  refs.blendNum = el('span', { class: 'num small', style: 'min-width:48px' });
  refs.blendNote = el('p', { class: 'muted small' });
  refs.blend.oninput = () => { S.draft.blend = r3(Number(refs.blend.value)); updateLoopInfo(); };
  refs.blend.onchange = draftChanged;
  const blendRow = el('div', { class: 'slider-row' }, el('label', { for: 'bVal', class: 'small muted', text: 'Blend the end into the start' }), refs.blend, refs.blendNum);
  // seam check
  refs.seamBtn = el('button', { class: 'btn secondary', text: 'Check the seam', 'aria-expanded': 'false', onclick: toggleSeamPanel });
  refs.findBtn = el('button', { class: 'btn secondary', text: 'Find best loop point', onclick: findLoopPoint });
  refs.findResult = el('div', { class: 'small', role: 'status' });
  refs.seamPanel = el('div', { class: 'seam-panel', hidden: true });
  g.append(el('div', { class: 'row trim-head' }, refs.viewLabel, el('span', { class: 'grow' }), refs.zoomTrack),
    refs.trim, startRow, endRow, modes, blendRow, refs.blendNote,
    el('div', { class: 'row' }, refs.seamBtn, refs.findBtn), refs.findResult, refs.seamPanel);
  setupTrim();
  fitView();
  return g;
}

function fitView() {
  const d = S.detail, dr = S.draft;
  if (view.zoomed) {
    const pad = Math.max(0.5, dr.length * 0.15);
    view.a = Math.max(0, dr.start - pad); view.b = Math.min(d.duration, dr.start + dr.length + pad);
  } else { view.a = 0; view.b = d.duration; }
  if (refs.zoomTrack) {
    refs.zoomTrack.setAttribute('aria-pressed', String(view.zoomed));
    refs.zoomTrack.textContent = view.zoomed ? 'Show the whole clip' : 'Zoom to the selection';
  }
  if (refs.viewLabel) refs.viewLabel.textContent = `${fmtT2(view.a)} – ${fmtT2(view.b)}`;
}
const trackPct = t => (clamp((t - view.a) / Math.max(0.001, view.b - view.a), 0, 1) * 100) + '%';

// Filmstrip: thumbnails of the visible part of the track (cheap: grabbed from the preview media in the browser).
let filmSeq = 0, filmT = 0;
function drawFilmstrip() {
  clearTimeout(filmT);
  filmT = setTimeout(async () => {
    const c = refs.film, d = S.detail;
    if (!c || !d || !d.previewUrl || d.previewKind !== 'video') return;
    const seq = ++filmSeq;
    const w = c.clientWidth, h = c.clientHeight;
    if (!w || !h) return;
    c.width = w; c.height = h;
    const ctx = c.getContext('2d');
    const tw = Math.max(24, Math.round(h * (natural.w / natural.h || 16 / 9)));
    const n = Math.max(1, Math.ceil(w / tw));
    for (let i = 0; i < n; i++) {
      const t = view.a + ((i + 0.5) / n) * (view.b - view.a);
      const f = await grabFrame(t, tw, h);
      if (seq !== filmSeq) return;
      if (f) ctx.drawImage(f, i * tw, 0);
    }
  }, 250);
}

function syncTuneControls() {
  const dr = S.draft;
  if (!dr || !refs.saveBtn) return;
  if (refs.trim) { refs.blend.value = dr.blend || 0; updateLoopInfo(); positionTrim(); }
  refs.qAuto.checked = dr.quality === 'auto'; refs.qCustom.checked = dr.quality !== 'auto';
  refs.qSlider.value = dr.quality === 'auto' ? 80 : dr.quality;
  refs.qNum.textContent = refs.qSlider.value;
  refs.qRow.hidden = dr.quality === 'auto';
  refs.detail.value = dr.detail || 0; refs.detailNum.textContent = dr.detail || 'off';
  refs.cornersOn.checked = !!dr.corners.on; refs.cornersR.value = refs.cornersRN.value = dr.corners.r;
  refs.cornersRow.hidden = !dr.corners.on;
  applyCorners();
  updateFrameControls();
}

function updateLoopInfo() {
  const d = S.detail, dr = S.draft;
  if (!refs.trim) return;
  const sf = Math.round(dr.start * srcFps());
  const n = Math.round(dr.length * OUT_FPS());
  if (document.activeElement !== refs.inStartF) refs.inStartF.value = sf;
  if (document.activeElement !== refs.inLenF) refs.inLenF.value = n;
  refs.inStartF.max = Math.max(0, Math.floor((d.duration - 0.05) * srcFps()));
  refs.inLenF.max = Math.round(Math.min(S.app.maxSeconds, d.duration - dr.start) * OUT_FPS());
  refs.startInfo.textContent = `at ${fmtT2(dr.start)}${d.fps ? '' : ' (source frame rate unknown, assuming 25)'}`;
  refs.endInfo.textContent = `= ${fmtSecs(dr.length)}, ends at ${fmtT2(dr.start + dr.length)}`;
  const B = effectiveBlend();
  refs.blendNum.textContent = (dr.blend || 0) > 0 ? `${(dr.blend).toFixed(2)} s` : 'off';
  refs.blendNote.textContent = (dr.blend || 0) <= 0 ? 'Cross-fades the last moments of the loop into the frames just before its start, so the jump back is invisible. The length stays the same.'
    : B + 1e-6 < (dr.blend || 0) ? `Only ${B.toFixed(2)} s can be blended: it needs that much of the clip before the start (and at most half the loop). Move the start later for more.`
      : `The last ${B.toFixed(2)} s fade into the ${B.toFixed(2)} s just before the start.`;
}

function updateFrameControls() {
  if (!refs.zoom || !S.draft) return;
  const f = S.draft.frame;
  const fz = fitZoom(natural.w, natural.h);
  refs.zoom.min = Math.min(1, Math.floor(fz * 100) / 100);
  refs.zoom.value = f.zoom; refs.posX.value = f.x; refs.posY.value = f.y;
  refs.zoomVal.textContent = Math.round(f.zoom * 100) + '%';
  refs.zoom.setAttribute('aria-valuetext', `${Math.round(f.zoom * 100)} percent${f.zoom === 1 ? ', fills the screen' : ''}`);
  const g = frameGeometry(natural.w, natural.h, f);
  refs.posX.disabled = g.width === W; refs.posY.disabled = g.height === H;
}

function positionTrim() {
  const d = S.detail, dr = S.draft;
  if (!refs.trim || !d.duration) return;
  const end = dr.start + dr.length;
  refs.hStart.style.left = trackPct(dr.start);
  refs.hEnd.style.left = trackPct(end);
  refs.trimRange.style.left = trackPct(dr.start);
  refs.trimRange.style.width = `calc(${trackPct(end)} - ${trackPct(dr.start)})`;
  for (const [h, v, lo, hi] of [[refs.hStart, dr.start, 0, d.duration - 0.05], [refs.hEnd, end, dr.start + 1 / OUT_FPS(), Math.min(d.duration, dr.start + S.app.maxSeconds)]]) {
    h.setAttribute('aria-valuemin', r3(lo)); h.setAttribute('aria-valuemax', r3(hi)); h.setAttribute('aria-valuenow', r3(v));
    h.setAttribute('aria-valuetext', `${fmtT2(v)}, ${h === refs.hStart ? `source frame ${Math.round(v * srcFps())}` : `${Math.round(dr.length * OUT_FPS())} frames long`}`);
  }
  updateLoopInfo();
}

/** Start snaps to source frames; the length is a whole number of screen frames (1/18 s). */
function setTrim(start, length, seekTo) {
  const d = S.detail;
  const sf = srcFps(), of = OUT_FPS();
  start = clamp(Math.round(start * sf) / sf, 0, Math.max(0, d.duration - 0.05));
  const maxN = Math.max(1, Math.floor(Math.min(S.app.maxSeconds, d.duration - start) * of + 1e-6));
  const n = clamp(Math.round(length * of), 1, maxN);
  S.draft.start = r3(start); S.draft.length = r3(n / of);
  positionTrim();
  if (media && media.tagName === 'VIDEO' && !S.showing) media.currentTime = seekTo != null ? seekTo : S.draft.start;
}
// Nudge by whole frames and show the frame that moved (paused), so 1-frame changes can be seen.
function nudgeStart(frames) {
  const end = S.draft.start + S.draft.length;
  const s = S.draft.start + frames / srcFps();
  if (media && media.tagName === 'VIDEO') media.pause();
  setTrim(s, end - s, Math.max(0, s));
  draftChangedSoon();
}
function nudgeEnd(frames) {
  const l = S.draft.length + frames / OUT_FPS();
  if (media && media.tagName === 'VIDEO') media.pause();
  setTrim(S.draft.start, l, S.draft.start + Math.max(1, Math.round(l * OUT_FPS())) / OUT_FPS() - 1 / OUT_FPS());
  draftChangedSoon();
}

function setupTrim() {
  const d = S.detail;
  const timeAt = x => { const r = refs.trim.getBoundingClientRect(); return view.a + clamp((x - r.left) / r.width, 0, 1) * (view.b - view.a); };
  const drag = (elm, onMove) => {
    elm.addEventListener('pointerdown', e => {
      e.preventDefault(); e.stopPropagation();
      elm.setPointerCapture(e.pointerId);
      scrubbing = true;
      if (media && media.tagName === 'VIDEO') media.pause();
      const t0 = timeAt(e.clientX), s0 = S.draft.start, l0 = S.draft.length;
      const move = ev => onMove(timeAt(ev.clientX), t0, s0, l0);
      const up = () => {
        elm.removeEventListener('pointermove', move); elm.removeEventListener('pointerup', up); elm.removeEventListener('pointercancel', up);
        scrubbing = false;
        if (media && media.tagName === 'VIDEO') { media.currentTime = S.draft.start; media.play().catch(() => {}); }
        if (view.zoomed) { fitView(); positionTrim(); drawFilmstrip(); }
        draftChanged();
      };
      elm.addEventListener('pointermove', move); elm.addEventListener('pointerup', up); elm.addEventListener('pointercancel', up);
    });
  };
  drag(refs.hStart, t => { const end = S.draft.start + S.draft.length; const s = clamp(t, Math.max(0, end - S.app.maxSeconds), end - 1 / OUT_FPS()); setTrim(s, end - s, s); });
  drag(refs.hEnd, t => { const e = clamp(t, S.draft.start + 1 / OUT_FPS(), d.duration); setTrim(S.draft.start, e - S.draft.start, e); });
  drag(refs.trimRange, (t, t0, s0, l0) => { const s = clamp(s0 + t - t0, 0, d.duration - l0); setTrim(s, l0, s); });
  const keys = (h, isEnd) => h.addEventListener('keydown', e => {
    const dir = { ArrowLeft: -1, ArrowDown: -1, ArrowRight: 1, ArrowUp: 1 }[e.key];
    if (dir) { e.preventDefault(); (isEnd ? nudgeEnd : nudgeStart)(dir * (e.shiftKey ? 10 : 1)); return; }
    let s = S.draft.start, l = S.draft.length;
    if (e.key === 'PageUp' || e.key === 'PageDown') { const st = e.key === 'PageUp' ? 1 : -1; if (isEnd) l += st; else { s += st; l -= st; } }
    else if (e.key === 'Home') { if (isEnd) l = 1 / OUT_FPS(); else { l += s; s = 0; } }
    else if (e.key === 'End') { if (isEnd) l = d.duration - s; else { s = s + l - 1 / OUT_FPS(); l = 1 / OUT_FPS(); } }
    else return;
    e.preventDefault();
    if (!isEnd && l > S.app.maxSeconds) l = S.app.maxSeconds;
    setTrim(s, Math.max(1 / OUT_FPS(), l), s);
    draftChangedSoon();
  });
  keys(refs.hStart, false); keys(refs.hEnd, true);
  refs.inStartF.onchange = () => { const end = S.draft.start + S.draft.length; const s = (Number(refs.inStartF.value) || 0) / srcFps(); setTrim(s, Math.max(1 / OUT_FPS(), end - s)); draftChanged(); };
  refs.inLenF.onchange = () => { setTrim(S.draft.start, (Number(refs.inLenF.value) || 1) / OUT_FPS()); draftChanged(); };
}

// ----- seam check -----
const seam = { mode: 'onion', opacity: 0.5 };
function toggleSeamPanel() {
  const open = refs.seamPanel.hidden;
  refs.seamPanel.hidden = !open;
  refs.seamBtn.setAttribute('aria-expanded', String(open));
  if (open) buildSeamPanel();
}
function buildSeamPanel() {
  const p = refs.seamPanel;
  p.textContent = '';
  const modeSel = el('select', { id: 'seamMode', 'aria-label': 'Comparison' },
    el('option', { value: 'onion', text: 'Onion skin (first frame over the last)' }),
    el('option', { value: 'side', text: 'Side by side (last | first)' }),
    el('option', { value: 'diff', text: 'Difference (black = identical)' }));
  modeSel.value = seam.mode;
  const op = el('input', { type: 'range', id: 'seamOp', min: '0', max: '1', step: '0.05', value: String(seam.opacity) });
  const opRow = el('div', { class: 'slider-row' }, el('label', { for: 'seamOp', class: 'small muted', text: 'First frame opacity' }), op);
  refs.seamCanvas = el('canvas', { class: 'seam-canvas', role: 'img', 'aria-label': 'Comparison of the last and first frame of the loop' });
  refs.seamScore = el('p', { class: 'small', role: 'status' });
  modeSel.onchange = () => { seam.mode = modeSel.value; opRow.hidden = seam.mode !== 'onion'; drawSeam(); };
  op.oninput = () => { seam.opacity = Number(op.value); drawSeam(); };
  opRow.hidden = seam.mode !== 'onion';
  p.append(el('div', { class: 'row' }, el('label', { for: 'seamMode', class: 'small muted', text: 'Compare' }), modeSel,
    el('button', { class: 'btn ghost', text: 'Refresh', onclick: () => updateSeam() })), opRow, refs.seamCanvas, refs.seamScore);
  updateSeam();
}
let seamFrames = null, seamSeq = 0, seamT = 0;
function updateSeamSoon() { if (refs.seamPanel && !refs.seamPanel.hidden) { clearTimeout(seamT); seamT = setTimeout(updateSeam, 300); } }
async function updateSeam() {
  const dr = S.draft, seq = ++seamSeq;
  const w = 480, h = Math.round(w * (natural.h / natural.w || 9 / 16));
  const last = dr.start + dr.length - 1 / OUT_FPS();
  const B = effectiveBlend();
  const a = await grabFrame(last, w, h);
  const first = await grabFrame(dr.start, w, h);
  // With a blend, the last frame is mostly the frame just before the start: mix like the converter does.
  let lastC = a;
  if (B > 0 && a) {
    const pre = await grabFrame(dr.start - 1 / OUT_FPS(), w, h);
    const p = (B - 1 / OUT_FPS()) / B;
    lastC = document.createElement('canvas'); lastC.width = w; lastC.height = h;
    const x = lastC.getContext('2d');
    x.drawImage(a, 0, 0); x.globalAlpha = p; if (pre) x.drawImage(pre, 0, 0);
  }
  // The score compares the frame that would follow the end with the first frame (the same measure as
  // "Find best loop point"): identical means the jump back to the start is invisible.
  const next = B > 0 ? null : await grabFrame(dr.start + dr.length, w, h);
  if (seq !== seamSeq) return;
  seamFrames = { last: lastC, first, next, blended: B > 0, w, h };
  drawSeam();
}
function drawSeam() {
  const c = refs.seamCanvas, f = seamFrames;
  if (!c || !f || !f.last || !f.first) { if (refs.seamScore) refs.seamScore.textContent = 'Couldn\'t read the frames.'; return; }
  const side = seam.mode === 'side';
  c.width = side ? f.w * 2 + 8 : f.w; c.height = f.h;
  const x = c.getContext('2d');
  x.clearRect(0, 0, c.width, c.height);
  if (side) { x.drawImage(f.last, 0, 0); x.drawImage(f.first, f.w + 8, 0); }
  else if (seam.mode === 'diff') { x.drawImage(f.last, 0, 0); x.globalCompositeOperation = 'difference'; x.drawImage(f.first, 0, 0); x.globalCompositeOperation = 'source-over'; }
  else { x.drawImage(f.last, 0, 0); x.globalAlpha = seam.opacity; x.drawImage(f.first, 0, 0); x.globalAlpha = 1; }
  // similarity of small grey versions (same measure as "Find best loop point")
  const sm = cv => { const s = document.createElement('canvas'); s.width = 96; s.height = 54; const g = s.getContext('2d'); g.drawImage(cv, 0, 0, 96, 54); return g.getImageData(0, 0, 96, 54).data; };
  const hint = side ? ' Left: last frame of the loop. Right: first frame.' : seam.mode === 'diff' ? ' Bright areas change at the jump.' : '';
  if (f.blended || !f.next) { refs.seamScore.textContent = (f.blended ? 'The blend fades the end into the start, so the jump back is smooth.' : '') + hint; return; }
  const A = sm(f.next), Bd = sm(f.first);
  let sum = 0;
  for (let i = 0; i < A.length; i += 4) sum += Math.abs((A[i] * 0.299 + A[i + 1] * 0.587 + A[i + 2] * 0.114) - (Bd[i] * 0.299 + Bd[i + 1] * 0.587 + Bd[i + 2] * 0.114));
  const sim = 100 - (sum / (A.length / 4) / 255) * 100;
  refs.seamScore.textContent = `The frame after the end is ${sim.toFixed(1)}% like the first frame` + (sim >= 98.8 ? ' (looks seamless).' : sim >= 97.5 ? ' (a small jump).' : ' (a visible jump).') + hint;
}

async function findLoopPoint() {
  const d = S.detail, dr = S.draft;
  refs.findBtn.disabled = true;
  refs.findResult.textContent = 'Searching ±2 s around the end…';
  try {
    const r = await api(`/api/originals/${d.id}/looppoint`, 'POST', { start: dr.start, length: dr.length });
    const use = c => el('button', { class: 'btn ' + (c === r.best ? 'primary' : 'ghost'), text: `${c === r.best ? 'Use ' : ''}${fmtT2(c.end)} (${c.similarity}%)`,
      onclick: () => { setTrim(dr.start, c.length); draftChanged(); updateSeamSoon(); refs.findResult.textContent = `Loop end set to ${fmtT2(dr.start + dr.length)}.`; } });
    refs.findResult.textContent = '';
    refs.findResult.append(el('span', { text: `Best end: ${fmtT2(r.best.end)}, ${r.best.similarity}% like the start (now ${r.current.similarity}%). ` }), use(r.best),
      r.alternatives.length ? el('span', { class: 'muted', text: ' Others: ' }) : null, ...r.alternatives.map(use));
  } catch (e) { refs.findResult.textContent = e.message; }
  refs.findBtn.disabled = false;
}

let draftSaveT, draftKeyT;
function draftChangedSoon() { clearTimeout(draftKeyT); draftKeyT = setTimeout(draftChanged, 300); }
function draftChanged() {
  updateFrameControls();
  updateSeamSoon();
  S.est = { ...S.est, state: 'checking' };
  updateTuneState();
  scheduleEstimate();
  clearTimeout(draftSaveT);
  const id = S.detail.id, draft = JSON.parse(JSON.stringify(S.draft));
  draftSaveT = setTimeout(() => api('/api/originals/' + id, 'PATCH', { draft }).catch(() => {}), 800);
}
function resetDraft() {
  const d = S.detail;
  S.draft = initialDraft({ ...d, draft: null });
  syncTuneControls(); applyGeometry();
  if (view.zoomed) { fitView(); drawFilmstrip(); }
  if (media && media.tagName === 'VIDEO') media.currentTime = S.draft.start;
  draftChanged();
}

// ----- size estimate -----
let estT, estSeq = 0;
function scheduleEstimate(ms = 600) {
  clearTimeout(estT);
  if (!S.detail || !S.detail.canTune) return;
  S.est = { ...S.est, state: 'checking' };
  updateTuneState();
  estT = setTimeout(runEstimate, ms);
}
async function runEstimate() {
  const seq = ++estSeq, id = S.detail.id;
  const body = JSON.parse(JSON.stringify(S.draft));
  try {
    const r = await api(`/api/originals/${id}/estimate`, 'POST', body);
    if (r.stale || seq !== estSeq || !S.detail || S.detail.id !== id) return;
    S.est = { state: 'done', r };
  } catch (e) {
    if (seq !== estSeq) return;
    S.est = { ...S.est, state: e.code === 'busy' ? 'paused' : 'error', error: e.message };
  }
  updateTuneState();
}

function duplicateOf() {
  const d = S.detail, dr = S.draft;
  if (!d || !S.app) return null;
  return d.versions.find(v => {
    const s = v.settings;
    return v.status !== 'error' && String(v.notchKey || '').split('|')[0] === S.app.notchKey && (s.fps || S.app.deviceFps) === S.app.deviceFps &&
      s.start === (d.isImage ? 0 : dr.start) && s.length === dr.length && String(s.quality) === String(dr.quality) &&
      s.frame && s.frame.zoom === dr.frame.zoom && s.frame.x === dr.frame.x && s.frame.y === dr.frame.y &&
      !!s.corners.on === !!dr.corners.on && (!dr.corners.on || s.corners.r === dr.corners.r) &&
      (s.detail || 0) === (dr.detail || 0) && (s.blend || 0) === (d.isImage ? 0 : (dr.blend || 0));
  });
}

function updateTuneState() {
  if (!refs.meter || !S.app) return;
  const est = S.est, budget = S.app.budget;
  const r = est.r;
  const fill = refs.meter.querySelector('.fill'), over = refs.meter.querySelector('.over');
  refs.meter.className = 'meter';
  refs.meterRetry.hidden = true;
  refs.meterNote.textContent = '';
  let stateText = '', stateCls = '';
  if (r) {
    const frac = r.bytes / budget;
    fill.style.width = Math.min(100, frac * 100) + '%';
    over.hidden = frac <= 1;
    over.style.width = Math.min(50, (frac - 1) * 100) / 0.75 * 0.75 + '%';
    refs.meterText.textContent = `${mb(r.bytes)} of ${mb(budget)}` + (S.app.budgetMeasured ? ' (measured on your screen)' : ' (estimated; check the screen to measure)');
    refs.meter.setAttribute('aria-label', `Uses about ${mb(r.bytes)} of ${mb(budget)}`);
    if (!r.fits) { refs.meter.classList.add('danger'); stateText = `Too big by ${mb(r.bytes - budget)}`; stateCls = 'danger'; }
    else if (frac > 0.97) { refs.meter.classList.add('warn'); stateText = 'Close to the limit'; stateCls = 'warn'; }
    else { stateText = 'Fits'; stateCls = 'ok'; }
    refs.meterNote.textContent = (r.mode === 'auto' ? (r.fits ? (r.quality > 1 ? `Best that fits: quality ${r.quality}` : 'Best that fits: the lowest quality') : 'At the lowest quality') : `Quality ${S.draft.quality}`) + ' · estimate, the final size may differ slightly';
  } else { fill.style.width = '0'; over.hidden = true; refs.meterText.textContent = `– of ${mb(budget)}`; }
  if (est.state === 'checking') { refs.meter.classList.add('checking'); stateText = 'Checking size…'; stateCls = ''; }
  if (est.state === 'paused') { refs.meter.classList.add('checking'); stateText = 'Size check paused while the screen is updating'; stateCls = ''; }
  if (est.state === 'error') { stateText = 'Couldn\'t check the size'; stateCls = 'danger'; refs.meterRetry.hidden = false; refs.meterNote.textContent = est.error || ''; }
  refs.meterState.textContent = stateText;
  refs.meterState.className = 'meter-state ' + stateCls;
  // save button (+ one suggested fix when it doesn't fit)
  const dup = duplicateOf();
  let reason = '', disabled = false, danger = false;
  refs.saveFix.hidden = true;
  if (dup) { disabled = true; reason = `Already saved as ${dup.label || 'v' + dup.n}.`; }
  else if (est.state === 'done' && r && !r.fits) {
    disabled = true; danger = true;
    if (r.mode === 'auto') {
      reason = 'This part is too long to fit even at the lowest quality.' +
        (r.detailFits ? ` "Simplify fine detail" at ${r.detailFits} should make it fit` : '') +
        (r.maxFitLength ? `${r.detailFits ? ', or s' : ' S'}horten it to about ${fmtSecs(r.maxFitLength)}.` : '.');
      if (r.detailFits) {
        refs.saveFix.hidden = false;
        refs.saveFix.textContent = `Simplify detail: ${r.detailFits}`;
        refs.saveFix.onclick = () => { S.draft.detail = r.detailFits; syncTuneControls(); applyGeometry(); draftChanged(); };
      }
    } else reason = `Too big.${r.maxFitLength ? ` Shorten to about ${fmtSecs(r.maxFitLength)} or` : ''} choose Best that fits.`;
  }
  refs.saveBtn.disabled = disabled;
  refs.saveReason.textContent = reason;
  refs.saveReason.className = 'reason' + (danger ? ' danger' : '');
}

async function saveVersion() {
  const d = S.detail;
  refs.saveBtn.disabled = true;
  try {
    await api(`/api/originals/${d.id}/versions`, 'POST', JSON.parse(JSON.stringify(S.draft)));
    await loadDetail();
    loadOriginals();
    loadApp();
    pollSoon();
  } catch (e) {
    toast(e.message, true);
  }
  updateTuneState();
}

// ---------- versions ----------
function jobFor(vid, kind) { return S.jobs.find(j => j.versionId === vid && j.kind === kind && (j.status === 'running' || j.status === 'waiting')); }

function plainWait(msg) {
  if (!msg) return 'Waiting…';
  if (/screen operation/.test(msg)) return 'Waiting for the screen update to finish';
  if (/conversions/.test(msg)) return 'Waiting for the current preparation to finish';
  if (/previous conversion/.test(msg)) return 'Waiting for the previous version to finish';
  return msg.replace(/…$/, '');
}

function versionSummary(v) {
  const s = v.settings || {};
  const parts = [];
  if (!S.detail.isImage) parts.push(`${fmtTime(s.start)}–${fmtTime(s.start + s.length)}`);
  parts.push(fmtSecs((v.result && v.result.length) || s.length));
  parts.push(s.quality === 'auto' ? 'Best that fits' : `Quality ${s.quality}`);
  const f = s.frame;
  if (f && (f.zoom !== 1 || f.x !== 0.5 || f.y !== 0.5)) parts.push(f.zoom < 1 ? 'whole picture' : `zoom ${Math.round(f.zoom * 100)}%`);
  if (s.blend > 0) parts.push(`blend ${s.blend.toFixed(2)} s`);
  if (s.detail > 0) parts.push(`simplified ${s.detail}`);
  if (s.corners && s.corners.on) parts.push(`rounded ${s.corners.r}`);
  return parts.join(' · ');
}

function renderVersions(d) {
  const box = refs.versions;
  box.textContent = '';
  box.append(el('h3', { id: 'hVersions', text: `Versions (${d.versions.length})` }));
  if (!d.versions.length) { box.append(el('p', { class: 'muted small', text: 'No versions yet. Adjust the clip and press Save as version.' })); return; }
  const ul = el('ul', { class: 'vlist' });
  const deviceRunning = S.jobs.some(j => j.kind === 'send' && (j.status === 'running' || j.status === 'waiting'));
  for (const v of d.versions) {
    const name = v.label || `v${v.n}`;
    const chips = [];
    if (v.onScreen) chips.push(el('span', { class: 'chip ok', text: v.onScreenIncomplete ? 'On screen (upload didn\'t finish)' : 'On screen' }));
    const sendJob = jobFor(v.id, 'send');
    const vjob = jobFor(v.id, 'version');
    if (v.status === 'ready') {
      if (v.tooBig) chips.push(el('span', { class: 'chip danger', text: 'Too big for the screen' }));
      if (v.speed) chips.push(el('span', { class: 'chip warn', text: `Plays too ${v.speed.plays} – re-make`, title: `Made at ${v.speed.fps} frames per second; the screen always plays ${S.app.deviceFps}.` }));
      if (v.oldNotch) chips.push(el('span', { class: 'chip warn', text: 'Made with older screen settings – re-make', title: 'The notch or rounded-corner setting has changed since this version was made.' }));
    }
    if (v.status === 'error') chips.push(el('span', { class: 'chip danger', text: 'Failed' }));
    const main = el('div', { class: 'vmain' }, el('div', { class: 'vtitle' }, el('b', { text: name }), el('span', { class: 'small muted', text: versionSummary(v) }), chips));
    if (v.status === 'queued' || v.status === 'converting') {
      const p = vjob ? (vjob.status === 'waiting' ? plainWait(vjob.message) : `Preparing… ${Math.round(vjob.progress * 100)}%`) : 'Waiting…';
      main.append(el('div', { class: 'progress', role: 'progressbar', 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': String(Math.round((vjob ? vjob.progress : 0) * 100)), 'aria-label': `Preparing ${name}` },
        el('div', { style: `width:${Math.round((vjob ? vjob.progress : 0) * 100)}%` })), el('div', { class: 'vsub', text: p }));
    } else if (v.status === 'error') {
      main.append(el('div', { class: 'vsub', text: 'Couldn\'t prepare this version.' + (v.errorCode === 'tooLong' && v.maxFitLength ? ` Try about ${fmtSecs(v.maxFitLength)} or less.` : '') }),
        el('details', {}, el('summary', { text: 'Show details' }), el('pre', { text: v.error || '' })));
    } else {
      main.append(el('div', { class: 'vsub num', text: `${mb(v.result.size)} · made ${fmtWhen(v.created)}` }));
    }
    const btns = el('div', { class: 'vbtns' });
    if (v.status === 'ready') {
      btns.append(el('button', { class: 'btn secondary', text: '▶ Preview', 'aria-label': `Preview ${name}`, onclick: e => { e.stopPropagation(); showPreview(v.id); refs.stage.scrollIntoView({ block: 'nearest' }); } }));
      if (sendJob) btns.append(el('span', { class: 'small muted', text: sendJob.status === 'waiting' ? 'Waiting to send…' : 'Sending…' }));
      else {
        const label = v.onScreen ? 'Send again' : deviceRunning ? 'Send (after current update)' : 'Send to screen';
        btns.append(el('button', { class: 'btn ' + (v.onScreen ? 'secondary' : 'primary'), text: label, disabled: v.tooBig, title: v.tooBig ? 'Too big for the screen. Make a shorter version or choose Best that fits.' : '',
          onclick: e => { e.stopPropagation(); confirmSend(v.id); } }));
      }
    }
    if (v.status === 'error') btns.append(el('button', { class: 'btn secondary', text: 'Try again', onclick: e => { e.stopPropagation(); retryVersion(v.id); } }));
    btns.append(el('span', { class: 'grow' }));
    btns.append(menuButton(`More actions for ${name}`, () => {
      const items = [{ label: 'Rename…', run: () => renameVersion(v) }];
      if (v.status === 'ready' && (v.oldNotch || v.speed)) items.push({ label: v.speed ? 'Re-make at the right speed' : 'Make updated copy', run: () => remakeVersion(v) });
      if (v.status !== 'queued' && v.status !== 'converting') items.push({ label: 'Delete version…', danger: true, run: () => deleteVersion(v) });
      return items;
    }));
    if (v.status === 'ready' && (v.oldNotch || v.speed)) {
      btns.insertBefore(el('button', { class: 'btn ghost', text: v.speed ? 'Re-make' : 'Make updated copy', onclick: e => { e.stopPropagation(); remakeVersion(v); } }), btns.querySelector('.grow'));
    }
    main.append(btns);
    const thumb = v.status === 'ready' ? el('img', { class: 'vthumb', src: `/media/v/${v.id}/thumb.jpg`, alt: '' }) : el('div', { class: 'vthumb' });
    const li = el('li', { class: 'vrow', 'data-vid': v.id }, thumb, main);
    li.onclick = () => { if (v.status === 'ready') showPreview(v.id); };
    ul.append(li);
  }
  box.append(ul);
  renderVersionsHighlight();
}
function renderVersionsHighlight() {
  if (!refs.versions) return;
  refs.versions.querySelectorAll('.vrow').forEach(r => r.classList.toggle('showing', r.dataset.vid === S.showing));
}

async function retryVersion(vid) {
  try { await api(`/api/versions/${vid}/retry`, 'POST'); await loadDetail(); pollSoon(); } catch (e) { toast(e.message, true); }
}
async function remakeVersion(v) {
  try { await api(`/api/versions/${v.id}/remake`, 'POST'); await loadDetail(); loadOriginals(); pollSoon(); } catch (e) { toast(e.message, true); }
}
async function renameVersion(v) {
  const input = el('input', { type: 'text', value: v.label || `v${v.n}`, id: 'vLabel', autofocus: true, maxLength: 40 });
  const val = await openDialog({ title: 'Rename version', body: el('div', { class: 'field' }, el('label', { for: 'vLabel', text: 'Name' }), input),
    actions: [{ label: 'Cancel', value: null, kind: 'secondary' }, { label: 'Save', value: 'save', kind: 'primary' }],
    onOpen: ({ close }) => input.addEventListener('keydown', e => { if (e.key === 'Enter') close('save'); }) });
  if (val !== 'save') return;
  try { await api(`/api/versions/${v.id}`, 'PATCH', { label: input.value }); await loadDetail(); loadApp(); } catch (e) { toast(e.message, true); }
}
async function deleteVersion(v) {
  const name = v.label || `v${v.n}`;
  const body = el('div', {}, el('p', { text: `"${S.detail.name} · ${name}" will be deleted. The original clip is kept.` }),
    v.onScreen ? el('p', { text: 'This version is on the screen now. Deleting it here does not remove it from the screen.' }) : null);
  const ok = await openDialog({ title: `Delete ${name}?`, body, actions: [{ label: 'Cancel', value: false, kind: 'secondary' }, { label: 'Delete', value: true, kind: 'danger' }] });
  if (!ok) return;
  try {
    await api(`/api/versions/${v.id}`, 'DELETE');
    if (S.showing === v.id) showPreview(null);
    await loadDetail(); loadOriginals(); loadApp();
  } catch (e) { toast(e.message, true); }
}

// ---------- send ----------
async function confirmSend(vid) {
  let d = S.detail;
  let v = d && d.versions.find(x => x.id === vid);
  if (!v) {
    const os = S.app.onScreen;
    if (os && os.versionId === vid && os.originalId) { await select(os.originalId); d = S.detail; v = d && d.versions.find(x => x.id === vid); }
    if (!v) return;
  }
  const name = `${d.name} · ${v.label || 'v' + v.n}`;
  const os = S.app.onScreen;
  const secs = v.result.size / (S.app.uploadRate || 50000) + 15;
  const curFig = el('figure', {}, os && os.thumbUrl ? el('img', { src: os.thumbUrl, alt: '' }) : el('div', { class: 'noimg' }),
    el('figcaption', { text: os ? (os.known ? `${os.originalName} · ${os.label}` : 'Unknown (sent with another app)') : 'Unknown' }));
  const body = el('div', {},
    el('div', { class: 'swap' },
      el('figure', {}, el('img', { src: `/media/v/${v.id}/thumb.jpg`, alt: '' }), el('figcaption', { text: `${name} (${mb(v.result.size)})` })),
      el('span', { 'aria-hidden': 'true', text: '→' }), el('span', { class: 'small muted', text: 'replaces' }), curFig),
    el('p', { text: `This takes ${fmtDuration(secs)}. Keep CoolerScreen open until it finishes.` }),
    el('p', { class: 'muted small', text: 'The screen may go dark or restart briefly during the upload. If LOVINGCOOL MONITOR is running, close it first.' }));
  const ok = await openDialog({ title: 'Send to the screen?', body, wide: true,
    actions: [{ label: 'Cancel', value: false, kind: 'secondary' }, { label: 'Send to screen', value: true, kind: 'primary' }] });
  if (!ok) return;
  try {
    const { jobId } = await api(`/api/versions/${vid}/send`, 'POST');
    sendJobs.add(jobId);
    await poll();
  } catch (e) { toast(e.message, true); }
}

// ---------- activity strip ----------
const sendJobs = new Set(); // send jobs started or seen running in this window
function renderActivity() {
  const box = $('activity');
  box.textContent = '';
  box.className = 'activity';
  const active = S.jobs.filter(j => j.status === 'running' || j.status === 'waiting');
  const now = Date.now();
  const finishedSends = S.jobs.filter(j => j.kind === 'send' && sendJobs.has(j.id) && (j.status === 'done' || j.status === 'error') && !S.dismissed.has(j.id));
  const outcome = finishedSends[finishedSends.length - 1];
  const rows = [];
  for (const j of active.slice(0, 3)) rows.push(activityRow(j));
  if (active.length > 3) rows.push(el('div', { class: 'act-row' }, el('span', { class: 'txt muted', text: `+${active.length - 3} more waiting` })));
  if (!active.length && outcome) {
    if (outcome.status === 'error') {
      box.classList.add('danger');
      const act = sendErrorAction(outcome);
      rows.push(el('div', { class: 'act-row' }, el('span', { class: 'txt wrap', text: `Couldn't send "${outcome.label}". ${plainSendError(outcome)}` }),
        act, el('button', { class: 'btn ghost', text: 'Dismiss', onclick: () => { S.dismissed.add(outcome.id); renderActivity(); } })));
    } else if (outcome.result && outcome.result.serialWarning) {
      box.classList.add('warn');
      rows.push(el('div', { class: 'act-row' }, el('span', { class: 'txt wrap', text: `Done. "${outcome.label}" is on the screen. The screen is not set up yet, so it will switch off about 30 seconds after this app closes.` }),
        el('button', { class: 'btn primary', text: 'Set up screen', onclick: runSetup }),
        el('button', { class: 'btn ghost', text: 'Dismiss', onclick: () => { S.dismissed.add(outcome.id); renderActivity(); } })));
    } else if (now - outcome.finishedAt < 8000) {
      box.classList.add('ok');
      rows.push(el('div', { class: 'act-row' }, el('span', { class: 'txt', text: `Done. "${outcome.label}" is on the screen.` })));
      setTimeout(renderActivity, 8000 - (now - outcome.finishedAt) + 50);
    }
  }
  box.hidden = !rows.length;
  box.append(...rows);
}

function activityRow(j) {
  let text, pct = Math.round((j.progress || 0) * 100);
  if (j.status === 'waiting') text = `${j.label}: ${plainWait(j.message)}`;
  else if (j.kind === 'send') {
    const m = j.message || '';
    if (/Connecting/.test(m)) text = `Sending "${j.label}": connecting to the screen`;
    else if (/intro/.test(m)) text = `Sending "${j.label}": writing the start-up logo · ${pct}%`;
    else if (/Starting playback/.test(m)) text = `Sending "${j.label}": starting playback`;
    else if (/Waiting for screen/.test(m)) text = `Sending "${j.label}": waiting for the screen to come back`;
    else {
      text = `Sending "${j.label}" to the screen · ${pct}%`;
      const elapsed = (Date.now() - (j.runningSince || j.startedAt)) / 1000;
      if (elapsed > 10 && j.progress > 0.02) text += ` · ${fmtLeft(elapsed / j.progress - elapsed)}`;
      else if (j.bytesTotal) text += ` · ${fmtLeft(j.bytesTotal * (1 - j.progress) / (S.app.uploadRate || 50000))}`;
    }
  } else if (j.kind === 'version') text = `Preparing "${j.label}" · ${pct}%`;
  else text = `${j.label}${j.message ? ': ' + j.message : ''}`;
  return el('div', { class: 'act-row' }, j.status === 'running' ? el('span', { class: 'spinner', 'aria-hidden': 'true' }) : null,
    el('span', { class: 'txt', text }),
    j.status === 'running' ? el('div', { class: 'bar', role: 'progressbar', 'aria-label': text, 'aria-valuemin': '0', 'aria-valuemax': '100', 'aria-valuenow': String(pct) }, el('div', { style: `width:${pct}%` })) : null);
}

function plainSendError(j) {
  if (j.failedUsb || j.code === 'failedUsb') return 'Windows lost the screen\'s USB connection.';
  if (j.code === 'busy') return 'Another program is using the screen. Close LOVINGCOOL MONITOR.';
  if (j.code === 'notFound') return 'Lost contact with the screen during the upload.';
  if (j.code === 'stalled') return 'The screen stopped responding. Unplugging isn\'t needed: press Reconnect, then send again.';
  return j.error || '';
}
function sendErrorAction(j) {
  if (j.failedUsb || j.code === 'failedUsb' || j.code === 'stalled') return el('button', { class: 'btn primary', text: 'Reconnect', onclick: runReconnect });
  if (j.code === 'notFound') return el('button', { class: 'btn primary', text: 'Check again', onclick: checkScreen });
  return el('button', { class: 'btn primary', text: 'Try again', onclick: () => { S.dismissed.add(j.id); confirmSend(j.versionId); } });
}

window.addEventListener('beforeunload', e => {
  if (S.jobs.some(j => j.kind === 'send' && j.status === 'running')) { e.preventDefault(); e.returnValue = ''; }
});

// ---------- polling (local server only; never touches the screen) ----------
let pollT = null, polling = false;
function pollSoon() { clearTimeout(pollT); pollT = setTimeout(poll, 300); }
async function poll() {
  clearTimeout(pollT);
  if (polling) return;
  polling = true;
  let busy = false;
  try {
    const a = await api('/api/activity');
    const before = new Map(S.jobs.map(j => [j.id, j.status]));
    S.jobs = a.jobs; S.deviceBusy = a.deviceBusy;
    busy = a.jobs.some(j => j.status === 'running' || j.status === 'waiting');
    let reloadApp = false;
    for (const j of a.jobs) {
      if (j.kind === 'send' && (j.status === 'running' || j.status === 'waiting')) sendJobs.add(j.id);
      const was = before.get(j.id);
      if (was && was !== j.status && (j.status === 'done' || j.status === 'error')) {
        if (j.kind === 'send') {
          reloadApp = true;
          if (j.status === 'done') S.screen = { ...S.screen, state: j.result && j.result.serialWarning ? 'setup' : 'ready' };
        }
        if (j.kind === 'setup') { reloadApp = true; }
      }
    }
    if (a.rev !== S.rev || reloadApp) await loadOriginals();
    else if (S.detail) {
      // progress only: refresh the versions list in place
      if (busy || before.size !== a.jobs.length) renderVersions(S.detail);
    }
    if (reloadApp) await loadApp();
    if (S.est.state === 'paused' && !a.deviceBusy) scheduleEstimate(0);
    if (S.screen.retry && !a.deviceBusy) { S.screen.retry = false; checkScreen(); }
    renderTop(); renderBanners(); renderActivity();
  } catch {}
  polling = false;
  pollT = setTimeout(poll, busy ? 1000 : document.hidden ? 6000 : 3000);
}

// ---------- device dialogs (setup / reconnect / restore) ----------
async function runDeviceJob(url, { title, text, confirmLabel, danger }) {
  const ok = await openDialog({ title, body: el('div', {}, text.map(t => el('p', { text: t }))),
    actions: [{ label: 'Cancel', value: false, kind: 'secondary' }, { label: confirmLabel, value: true, kind: danger ? 'danger' : 'primary' }] });
  if (!ok) return false;
  const status = el('p', { role: 'status', text: 'Starting…' });
  const bar = el('div', { class: 'progress' }, el('div'));
  let result = false;
  await openDialog({
    title, body: el('div', {}, status, bar), actions: [{ label: 'Close', value: true, kind: 'secondary' }],
    onOpen: async ({ close, setLocked }) => {
      setLocked(true);
      try {
        const { jobId } = await api(url, 'POST');
        for (;;) {
          const j = await api('/api/jobs/' + jobId);
          const msg = j.status === 'waiting' ? plainWait(j.message) : /administrator/.test(j.message) ? 'Waiting for your approval in the Windows prompt…' : j.message || 'Working…';
          status.textContent = msg;
          bar.firstChild.style.width = Math.round((j.progress || 0) * 100) + '%';
          if (j.status === 'done') { status.textContent = (j.result && j.result.warning) || 'Done.'; result = true; break; }
          if (j.status === 'error') { status.textContent = j.error; status.setAttribute('role', 'alert'); break; }
          await new Promise(r => setTimeout(r, 500));
        }
      } catch (e) { status.textContent = e.message; }
      setLocked(false);
      if (result) setTimeout(() => close(true), 2000);
    },
  });
  if (result) checkScreen();
  return result;
}
function runSetup() {
  return runDeviceJob('/api/setup', {
    title: 'Set up the screen (one time)', confirmLabel: 'Set up screen',
    text: ['This switches the screen to Microsoft\'s built-in WinUSB driver so it keeps its display on without any program running.',
      'Windows will ask for permission. Only the screen\'s own USB entry is changed.',
      'The vendor app (LOVINGCOOL MONITOR) won\'t see the screen until you restore the original driver in Settings.'],
  });
}
function runReconnect() {
  return runDeviceJob('/api/reconnect', {
    title: 'Reconnect the screen', confirmLabel: 'Reconnect',
    text: ['This resets the USB port the screen is connected to, which fixes the start-up glitch.', 'Windows will ask for permission. Nothing else is changed.'],
  });
}
function runRestore() {
  return runDeviceJob('/api/restore', {
    title: 'Restore the original driver?', confirmLabel: 'Restore original driver', danger: true,
    text: ['Windows installs the standard COM-port driver again, so LOVINGCOOL MONITOR can see the screen.',
      'Afterwards the screen switches its display off 30 seconds after any program closes its connection (this is how the vendor software works).',
      'Windows will ask for permission.'],
  });
}

// ---------- settings drawer ----------
let drawerReturn = null;
function openDrawer(section) {
  drawerReturn = document.activeElement;
  $('drawerBack').hidden = false;
  fillSettings();
  const d = $('drawer');
  d.focus();
  if (section) { const s = $(section); s.scrollIntoView({ block: 'start' }); const f = s.querySelector('button:not([hidden]), select, input'); if (f) f.focus(); }
}
function closeDrawer() {
  $('drawerBack').hidden = true;
  draftMask = null; // unsaved notch/corner changes are dropped
  applyLiveMask();
  if (drawerReturn && document.contains(drawerReturn)) drawerReturn.focus();
}
$('drawerClose').onclick = closeDrawer;
$('drawerBack').addEventListener('mousedown', e => { if (e.target === $('drawerBack')) closeDrawer(); });
$('drawerBack').addEventListener('keydown', e => {
  if (e.key === 'Escape' && !document.querySelector('.backdrop')) { e.preventDefault(); closeDrawer(); }
  if (e.key === 'Tab') {
    const f = [...$('drawer').querySelectorAll('button, input, select, summary, a[href]')].filter(x => !x.disabled && x.offsetParent !== null);
    const first = f[0], last = f[f.length - 1];
    if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last.focus(); }
    else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first.focus(); }
  }
});

function renderScreenSettings() {
  const st = S.screen;
  const text = {
    checking: 'Checking…', ready: 'Connected · set up (no background program needed)', setup: 'Connected, but the one-time setup is needed: the screen switches off 30 seconds after this app closes.',
    failedUsb: 'Not connected: Windows couldn\'t connect to the screen when the PC started.', notFound: 'Screen not found. Check the cooler\'s USB cable and power.',
    busy: 'Another program is using the screen. Close LOVINGCOOL MONITOR.', updating: 'Busy updating the screen.', error: st.error || 'Problem talking to the screen.',
  }[pillState()];
  $('scrStatus').textContent = text;
  $('scrSetup').hidden = st.state !== 'setup';
  $('scrSetupAgain').hidden = st.state !== 'ready';
  $('scrCheck').disabled = st.state === 'checking';
  $('scrInfo').textContent = st.info ? Object.entries(st.info).filter(([k, v]) => typeof v !== 'object').map(([k, v]) => `${k}: ${v}`).join('\n') : (st.error || '–');
}
$('scrCheck').onclick = checkScreen;
$('scrSetup').onclick = runSetup;
$('scrSetupAgain').onclick = runSetup;
$('aRestore').onclick = runRestore;

function fillSettings() {
  const a = S.app;
  if (!a) return;
  $('appVer').textContent = a.appVersion;
  maskToForm(a.mask);
  $('gDir').value = a.galleryDir; $('gMsg').textContent = '';
  $('aMax').value = a.maxSeconds;
  $('nMsg').textContent = '';
  const bg = $('notchBg');
  if (S.detail && S.detail.assets === 'ready') { bg.src = `/media/o/${S.detail.id}/frame.jpg`; bg.hidden = false; } else { bg.removeAttribute('src'); bg.hidden = true; }
  renderScreenSettings();
  drawNotch();
}
function formMask() {
  return {
    mode: $('nMode').value,
    band: { edge: $('nEdge').value, px: Number($('nPx').value) || 0 },
    circle: { cx: Number($('nCx').value) || 0, cy: Number($('nCy').value) || 0, r: Number($('nR').value) || 0, feather: Number($('nF').value) || 0 },
  };
}
function maskToForm(m) {
  $('nMode').value = m.mode; $('nEdge').value = m.band.edge; $('nPx').value = $('nPxR').value = m.band.px;
  $('nCx').value = m.circle.cx; $('nCy').value = m.circle.cy; $('nR').value = m.circle.r; $('nF').value = m.circle.feather;
}
// While the Settings drawer is open, the main preview shows the unsaved notch setting too.
let draftMask = null;
const liveMask = () => draftMask || (S.app && S.app.mask);
function applyLiveMask() {
  if (!refs.notch || S.showing) return;
  styleNotch(refs.notch, liveMask());
}
function drawNotch() {
  const m = formMask();
  $('nBand').hidden = m.mode !== 'band';
  $('nCircle').hidden = m.mode !== 'circle';
  styleNotch($('notchShape'), m);
  $('notchShape').classList.add('edit');
  draftMask = m;
  applyLiveMask();
  const eff = bandPx(m.band);
  $('nPxNote').textContent = m.mode === 'band' && eff && eff !== m.band.px ? `Applied as ${eff} px (rounded up to the screen's 16 px compression grid so the black area stays clean).` : '';
}
$('nPxR').oninput = () => { $('nPx').value = $('nPxR').value; drawNotch(); };
$('nPx').oninput = () => { $('nPxR').value = $('nPx').value; drawNotch(); };
for (const id of ['nMode', 'nEdge', 'nCx', 'nCy', 'nR', 'nF']) $(id).addEventListener('input', drawNotch);
$('nSave').onclick = async () => {
  try {
    S.app = await api('/api/settings', 'PUT', { mask: formMask() });
    const n = S.app.oldNotchCount || 0;
    $('nMsg').textContent = 'Saved. New versions use this notch.' + (n ? ` ${n} version${n > 1 ? 's were' : ' was'} made with an older notch. You can make updated copies from each clip's Versions list.` : '');
    draftMask = formMask();
    applyLiveMask();
    await loadDetail();
  } catch (e) { $('nMsg').textContent = e.message; }
};
$('gSave').onclick = async () => {
  try { S.app = await api('/api/gallery', 'PUT', { dir: $('gDir').value }); $('gDir').value = S.app.galleryDir; $('gMsg').textContent = 'Saved.'; await loadOriginals(); }
  catch (e) { $('gMsg').textContent = e.message; }
};
$('gOpen').onclick = () => $('openFolder').click();
$('aSave').onclick = async () => {
  try { S.app = await api('/api/settings', 'PUT', { maxSeconds: Number($('aMax').value) }); toast('Saved.'); if (S.detail) { detailFor = null; await loadDetail(); } }
  catch (e) { toast(e.message, true); }
};

// ---------- start ----------
(async function init() {
  await loadApp();
  await loadOriginals();
  renderActivity();
  poll();
  checkScreen(); // once at launch (opens the screen briefly); never polled
})().catch(e => toast(e.message, true, 10000));
