// CoolerScreen UI: Library (originals) | Detail (preview, Tune, Versions) | Activity strip | Settings drawer.
import { W, H, frameGeometry, fitZoom, bandPx, styleNotch } from './js/frame.js';
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
  const start = d.isImage ? 0 : clamp(Number(dr.start) || 0, 0, Math.max(0, dur - 0.1));
  const length = d.isImage ? (S.app ? S.app.imageSeconds : 3) : clamp(Number(dr.length) || Math.min(max, dur || max), 0.1, Math.max(0.1, Math.min(max, (dur || max) - start)));
  return { start: r1(start), length: r1(length), quality: dr.quality ?? 'auto', frame: { ...DEFAULT_FRAME, ...(dr.frame || {}) } };
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
  refs = {};
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
let media = null, rafId = 0, natural = { w: 0, h: 0 }, scrubbing = false;
function stopPreviewLoop() { cancelAnimationFrame(rafId); rafId = 0; }

function showPreview(vid, force = false) {
  const d = S.detail;
  if (!d || !refs.stage) return;
  if (!force && S.showing === vid && media) return;
  S.showing = vid;
  stopPreviewLoop();
  refs.stage.querySelectorAll('.media, .empty-msg').forEach(n => n.remove());
  media = null;
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
      });
    }
    natural = { w: d.width || W, h: d.height || H };
    refs.stage.prepend(media);
    applyGeometry();
  } else {
    refs.stage.prepend(el('div', { class: 'empty-msg', text: d.assets === 'error' ? 'No preview: this file can\'t be read.' : d.missing ? 'The original file is missing.' : 'Preparing the preview…' }));
  }
  if (!v && S.app) styleNotch(refs.notch, S.app.mask);
  refs.stage.append(refs.notch);
  if (refs.stage.classList.contains('outline')) refs.notch.classList.add('outline');
  refs.stage.classList.toggle('pannable', !v && !!d.canTune);
  refs.backToTune.hidden = !v || !d.canTune;
  updateShowingLabel();
  renderVersionsHighlight();
  rafId = requestAnimationFrame(tickPreview);
}

function applyGeometry() {
  if (!media || S.showing || !S.draft || !natural.w) return;
  const g = frameGeometry(natural.w, natural.h, S.draft.frame);
  Object.assign(media.style, { left: (g.left / W) * 100 + '%', top: (g.top / H) * 100 + '%', width: (g.width / W) * 100 + '%', height: (g.height / H) * 100 + '%' });
  updateFrameControls();
}

function tickPreview() {
  rafId = requestAnimationFrame(tickPreview);
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
  if (!scrubbing && (media.currentTime >= end - 0.02 || media.currentTime < start - 0.25)) { media.currentTime = start; }
  refs.time.textContent = `${fmtTime(Math.max(0, media.currentTime - start))} / ${fmtTime(length)}`;
  if (refs.playhead && S.detail.duration) refs.playhead.style.left = (media.currentTime / S.detail.duration) * 100 + '%';
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
  // 1. trim
  if (!d.isImage && d.duration) {
    const g = el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Part of the clip' }));
    refs.trim = el('div', { class: 'trim' });
    const track = el('div', { class: 'track' });
    refs.trimRange = el('div', { class: 'range', title: 'Drag to move the selected part' });
    refs.hStart = el('div', { class: 'handle', role: 'slider', tabindex: '0', 'aria-label': 'Start' });
    refs.hEnd = el('div', { class: 'handle', role: 'slider', tabindex: '0', 'aria-label': 'End' });
    refs.playhead = el('div', { class: 'playhead', 'aria-hidden': 'true' });
    refs.trim.append(track, refs.trimRange, refs.playhead, refs.hStart, refs.hEnd);
    refs.inStart = el('input', { type: 'number', id: 'tStart', min: '0', step: '0.1' });
    refs.inLen = el('input', { type: 'number', id: 'tLen', min: '0.1', step: '0.1' });
    refs.lenHint = el('span', { class: 'muted small' });
    g.append(refs.trim, el('div', { class: 'trim-fields' },
      el('div', { class: 'field' }, el('label', { for: 'tStart', text: 'Start (s)' }), refs.inStart),
      el('div', { class: 'field' }, el('label', { for: 'tLen', text: 'Length (s)' }), refs.inLen), refs.lenHint));
    t.append(g);
    setupTrim();
  } else {
    t.append(el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Part of the clip' }),
      el('p', { class: 'muted small', text: `Still image: it is shown as a ${fmtSecs(dr.length)} clip that repeats.` })));
  }
  // 2. framing ("size")
  const fg = el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Size and position on the screen' }));
  refs.zoom = el('input', { type: 'range', id: 'fZoom', min: '0.2', max: '4', step: '0.01' });
  refs.zoomVal = el('span', { class: 'num small muted', style: 'min-width:48px' });
  refs.posX = el('input', { type: 'range', id: 'fX', min: '0', max: '1', step: '0.001' });
  refs.posY = el('input', { type: 'range', id: 'fY', min: '0', max: '1', step: '0.001' });
  fg.append(
    el('div', { class: 'slider-row' }, el('label', { for: 'fZoom', class: 'small muted', text: 'Zoom' }), refs.zoom, refs.zoomVal),
    el('div', { class: 'slider-row' }, el('label', { for: 'fX', class: 'small muted', text: 'Left – right' }), refs.posX),
    el('div', { class: 'slider-row' }, el('label', { for: 'fY', class: 'small muted', text: 'Up – down' }), refs.posY),
    el('div', { class: 'row' },
      el('button', { class: 'btn secondary', text: 'Fill the screen', onclick: () => { dr.frame = { ...DEFAULT_FRAME }; applyGeometry(); draftChanged(); } }),
      el('button', { class: 'btn secondary', text: 'Show the whole picture', onclick: () => { dr.frame = { zoom: Math.round(fitZoom(natural.w, natural.h) * 1000) / 1000, x: 0.5, y: 0.5 }; applyGeometry(); draftChanged(); } }),
      el('span', { class: 'muted small', text: 'Tip: drag the preview to move the picture.' })));
  refs.zoom.oninput = () => { dr.frame.zoom = Number(refs.zoom.value); applyGeometry(); };
  refs.posX.oninput = () => { dr.frame.x = Number(refs.posX.value); applyGeometry(); };
  refs.posY.oninput = () => { dr.frame.y = Number(refs.posY.value); applyGeometry(); };
  for (const r of [refs.zoom, refs.posX, refs.posY]) r.onchange = draftChanged;
  t.append(fg);
  // 3. quality
  const qg = el('fieldset', { class: 'group' }, el('legend', { class: 'group-label', text: 'Quality' }));
  refs.qAuto = el('input', { type: 'radio', name: 'q', value: 'auto' });
  refs.qCustom = el('input', { type: 'radio', name: 'q', value: 'custom' });
  refs.qSlider = el('input', { type: 'range', id: 'qVal', min: '1', max: '100', step: '1', 'aria-label': 'Custom quality' });
  refs.qNum = el('span', { class: 'num', style: 'min-width:28px' });
  refs.qRow = el('div', { class: 'slider-row' }, el('span', { class: 'small muted', text: 'Smaller file' }), refs.qSlider, el('span', { class: 'small muted', text: 'Sharper' }), refs.qNum);
  qg.append(el('label', { class: 'radio' }, refs.qAuto, 'Best that fits (recommended)'),
    el('label', { class: 'radio' }, refs.qCustom, 'Custom'), refs.qRow);
  const onQ = () => {
    dr.quality = refs.qAuto.checked ? 'auto' : Number(refs.qSlider.value);
    refs.qRow.hidden = refs.qAuto.checked;
    refs.qNum.textContent = refs.qSlider.value;
    draftChanged();
  };
  refs.qAuto.onchange = refs.qCustom.onchange = onQ;
  refs.qSlider.oninput = () => { refs.qNum.textContent = refs.qSlider.value; };
  refs.qSlider.onchange = onQ;
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
  t.append(el('div', { class: 'actions' }, refs.saveBtn,
    el('button', { class: 'btn ghost', text: 'Reset', onclick: resetDraft }), refs.saveReason));
  syncTuneControls();
}

function syncTuneControls() {
  const dr = S.draft, d = S.detail;
  if (!dr || !refs.saveBtn) return;
  if (refs.inStart) {
    refs.inStart.value = dr.start; refs.inLen.value = dr.length;
    refs.inStart.max = r1(Math.max(0, d.duration - 0.1));
    refs.inLen.max = r1(maxLen());
    refs.lenHint.textContent = `up to ${fmtSecs(Math.min(S.app.maxSeconds, d.duration))}`;
    positionTrim();
  }
  refs.qAuto.checked = dr.quality === 'auto'; refs.qCustom.checked = dr.quality !== 'auto';
  refs.qSlider.value = dr.quality === 'auto' ? 80 : dr.quality;
  refs.qNum.textContent = refs.qSlider.value;
  refs.qRow.hidden = dr.quality === 'auto';
  updateFrameControls();
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

const maxLen = () => Math.max(0.1, Math.min(S.app.maxSeconds, (S.detail.duration || S.app.maxSeconds) - S.draft.start));

function positionTrim() {
  const d = S.detail, dr = S.draft;
  if (!refs.trim || !d.duration) return;
  const pc = v => (v / d.duration) * 100 + '%';
  refs.hStart.style.left = pc(dr.start);
  refs.hEnd.style.left = pc(dr.start + dr.length);
  refs.trimRange.style.left = pc(dr.start);
  refs.trimRange.style.width = pc(dr.length);
  for (const [h, v, lo, hi] of [[refs.hStart, dr.start, 0, d.duration - 0.1], [refs.hEnd, dr.start + dr.length, dr.start + 0.1, Math.min(d.duration, dr.start + S.app.maxSeconds)]]) {
    h.setAttribute('aria-valuemin', r1(lo)); h.setAttribute('aria-valuemax', r1(hi)); h.setAttribute('aria-valuenow', r1(v));
    h.setAttribute('aria-valuetext', fmtTime(v));
  }
}

function setTrim(start, length, seekTo) {
  const d = S.detail;
  start = clamp(r1(start), 0, r1(Math.max(0, d.duration - 0.1)));
  length = clamp(r1(length), 0.1, r1(Math.max(0.1, Math.min(S.app.maxSeconds, d.duration - start))));
  S.draft.start = start; S.draft.length = length;
  refs.inStart.value = start; refs.inLen.value = length; refs.inLen.max = r1(maxLen());
  positionTrim();
  if (media && media.tagName === 'VIDEO' && !S.showing) media.currentTime = seekTo != null ? seekTo : start;
}

function setupTrim() {
  const d = S.detail;
  const timeAt = x => { const r = refs.trim.getBoundingClientRect(); return clamp((x - r.left) / r.width, 0, 1) * d.duration; };
  const drag = (elm, onMove) => {
    elm.addEventListener('pointerdown', e => {
      e.preventDefault(); e.stopPropagation();
      elm.setPointerCapture(e.pointerId);
      scrubbing = true;
      const wasPlaying = media && !media.paused;
      if (media && media.tagName === 'VIDEO') media.pause();
      const t0 = timeAt(e.clientX), s0 = S.draft.start, l0 = S.draft.length;
      const move = ev => onMove(timeAt(ev.clientX), t0, s0, l0);
      const up = () => {
        elm.removeEventListener('pointermove', move); elm.removeEventListener('pointerup', up); elm.removeEventListener('pointercancel', up);
        scrubbing = false;
        if (media && media.tagName === 'VIDEO') { media.currentTime = S.draft.start; if (wasPlaying || true) media.play().catch(() => {}); }
        draftChanged();
      };
      elm.addEventListener('pointermove', move); elm.addEventListener('pointerup', up); elm.addEventListener('pointercancel', up);
    });
  };
  drag(refs.hStart, t => { const end = S.draft.start + S.draft.length; const s = clamp(t, Math.max(0, end - S.app.maxSeconds), end - 0.1); setTrim(s, end - s, s); });
  drag(refs.hEnd, t => { const e = clamp(t, S.draft.start + 0.1, d.duration); setTrim(S.draft.start, e - S.draft.start, e); });
  drag(refs.trimRange, (t, t0, s0, l0) => { const s = clamp(s0 + t - t0, 0, d.duration - l0); setTrim(s, l0, s); });
  const keys = (h, isEnd) => h.addEventListener('keydown', e => {
    const step = { ArrowLeft: -0.1, ArrowDown: -0.1, ArrowRight: 0.1, ArrowUp: 0.1, PageDown: -1, PageUp: 1 }[e.key];
    let s = S.draft.start, l = S.draft.length;
    if (step != null) { if (isEnd) l += step; else { s += step; l -= step; } }
    else if (e.key === 'Home') { if (isEnd) l = 0.1; else { l += s; s = 0; } }
    else if (e.key === 'End') { if (isEnd) l = d.duration - s; else { s = s + l - 0.1; l = 0.1; } }
    else return;
    e.preventDefault();
    if (!isEnd && l > S.app.maxSeconds) l = S.app.maxSeconds;
    setTrim(s, Math.max(0.1, l), isEnd ? s + Math.max(0.1, l) - 0.3 : s);
    draftChangedSoon();
  });
  keys(refs.hStart, false); keys(refs.hEnd, true);
  refs.inStart.onchange = () => { setTrim(Number(refs.inStart.value) || 0, S.draft.length); draftChanged(); };
  refs.inLen.onchange = () => { setTrim(S.draft.start, Number(refs.inLen.value) || 0.1); draftChanged(); };
}

let draftSaveT, draftKeyT;
function draftChangedSoon() { clearTimeout(draftKeyT); draftKeyT = setTimeout(draftChanged, 300); }
function draftChanged() {
  updateFrameControls();
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
    if (e.code === 'busy') pausedEstimate = true;
  }
  updateTuneState();
}
let pausedEstimate = false;

function duplicateOf() {
  const d = S.detail, dr = S.draft;
  if (!d || !S.app) return null;
  return d.versions.find(v => v.status !== 'error' && v.notchKey === S.app.notchKey && (v.settings.fps || S.app.deviceFps) === S.app.deviceFps &&
    v.settings.start === (d.isImage ? 0 : dr.start) && v.settings.length === dr.length && String(v.settings.quality) === String(dr.quality) &&
    v.settings.frame && v.settings.frame.zoom === dr.frame.zoom && v.settings.frame.x === dr.frame.x && v.settings.frame.y === dr.frame.y);
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
    refs.meterText.textContent = `${mb(r.bytes)} of ${mb(budget)}`;
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
  // save button
  const dup = duplicateOf();
  let reason = '', disabled = false, danger = false;
  if (dup) { disabled = true; reason = `Already saved as ${dup.label || 'v' + dup.n}.`; }
  else if (est.state === 'done' && r && !r.fits) {
    disabled = true; danger = true;
    reason = r.mode === 'auto'
      ? `This part is too long to fit even at the lowest quality.${r.maxFitLength ? ` Try about ${fmtSecs(r.maxFitLength)} or less.` : ''}`
      : `Too big.${r.maxFitLength ? ` Shorten to about ${fmtSecs(r.maxFitLength)} or` : ''} choose Best that fits.`;
  }
  refs.saveBtn.disabled = disabled;
  refs.saveReason.textContent = reason;
  refs.saveReason.className = 'reason' + (danger ? ' danger' : '');
}

async function saveVersion() {
  const d = S.detail;
  refs.saveBtn.disabled = true;
  try {
    const r = await api(`/api/originals/${d.id}/versions`, 'POST', JSON.parse(JSON.stringify(S.draft)));
    await loadDetail();
    loadOriginals();
    pollSoon();
    void r;
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
      if (v.oldNotch) chips.push(el('span', { class: 'chip warn', text: 'Made with an older notch setting' }));
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
function drawNotch() {
  const m = formMask();
  $('nBand').hidden = m.mode !== 'band';
  $('nCircle').hidden = m.mode !== 'circle';
  styleNotch($('notchShape'), m);
  $('notchShape').classList.add('edit');
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
    $('nMsg').textContent = 'Saved. New versions use this notch.' + (n ? ` ${n} version${n > 1 ? 's were' : ' was'} made with the old notch. You can make updated copies from each clip's Versions list.` : '');
    if (refs.notch && !S.showing) styleNotch(refs.notch, S.app.mask);
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
