const $ = id => document.getElementById(id);
const el = (tag, props = {}, ...kids) => {
  const e = Object.assign(document.createElement(tag), props);
  for (const k of kids) e.append(k);
  return e;
};
let settings = null, items = [], selected = null, slotInfo = null;
const mb = b => (b / 1e6).toFixed(2) + ' MB';
const SLOT_NAMES = { loop: 'Loop (always shown)', startup: 'Intro (plays once at power-on)', standby: 'Loop (always shown)', both: 'Both (same file in intro and loop)' };
let advanced = false;

async function api(path, method = 'GET', body) {
  const r = await fetch(path, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined });
  const j = await r.json().catch(() => ({}));
  if (!r.ok) throw new Error(j.error || r.statusText);
  return j;
}

let toastT;
function toast(msg, bad = false, ms = 6000) {
  const t = $('toast');
  t.textContent = msg; t.className = 'show' + (bad ? ' bad' : '');
  clearTimeout(toastT); toastT = setTimeout(() => (t.className = ''), ms);
}

async function pollJob(jobId, onUpdate) {
  for (;;) {
    const j = await api('/api/jobs/' + jobId);
    onUpdate && onUpdate(j);
    if (j.status === 'done') return j.result;
    if (j.status === 'error') throw new Error(j.error);
    await new Promise(r => setTimeout(r, 400));
  }
}

function setBar(box, frac, text, err) {
  box.classList.remove('hidden');
  box.classList.toggle('err', !!err);
  box.querySelector('.bar').style.width = Math.round(frac * 100) + '%';
  box.querySelector('span').textContent = text;
}

// ---------- slots ----------
async function loadSlots() {
  slotInfo = await api('/api/slots');
  const box = $('slots');
  box.textContent = '';
  for (const s of ['startup', 'standby']) {
    const v = slotInfo.slots[s];
    const card = el('div', { className: 'slot' }, el('b', { textContent: (s === 'startup' ? 'Intro slot' : 'Loop slot') }),
      el('div', { textContent: v ? `Last sent: ${v.name} (${mb(v.size)})` : 'Last sent: unknown' }),
      el('div', { className: 'hint', textContent: (s === 'standby' ? `Space for a loop clip (intro made black): ${mb(slotInfo.loop)}` : `Space available for an intro clip: ${mb(slotInfo[s])}`) + (s === 'startup' && slotInfo.assumed[s] ? ` (other slot unknown; assuming it uses ${mb(slotInfo.assumedBytes)})` : '') }));
    const clr = el('button', { className: 'small danger', textContent: 'Clear ' + (s === 'startup' ? 'intro' : 'loop') + ' slot' });
    const prog = el('div', { className: 'progress hidden' }, el('div', { className: 'bar' }), el('span'));
    clr.onclick = async () => {
      if (!confirm(`Replace the ${s} animation with a 1-frame black clip to free space?\nMake sure LOVINGCOOL MONITOR is closed.`)) return;
      try {
        const { jobId } = await api(`/api/slots/${s}/clear`, 'POST');
        await pollJob(jobId, j => setBar(prog, j.progress, j.message));
        toast(s + ' slot cleared. The screen may restart for a few seconds.');
      } catch (e) { setBar(prog, 1, e.message, true); toast(e.message, true, 12000); }
      await loadSlots(); renderGallery();
    };
    card.append(clr, prog);
    box.append(card);
  }
}

// ---------- gallery ----------
async function loadLibrary() {
  items = await api('/api/library');
  if (!items.find(i => i.id === selected)) selected = items.find(i => i.status === 'ready')?.id || null;
  renderGallery();
  drawMask();
}

function renderGallery() {
  const g = $('gallery');
  g.textContent = '';
  $('empty').classList.toggle('hidden', items.length > 0);
  for (const it of items) {
    const card = el('div', { className: 'item' + (it.id === selected ? ' sel' : '') });
    const ready = it.status === 'ready';
    const thumb = el('div', { className: 'thumb' });
    if (ready) {
      thumb.append(el('img', { src: `/media/${it.id}/thumb.jpg?v=${encodeURIComponent(it.converted)}`, loading: 'lazy' }));
      let v = null;
      thumb.onmouseenter = () => {
        v = el('video', { src: `/media/${it.id}/preview.mp4?v=${encodeURIComponent(it.converted)}`, muted: true, loop: true, autoplay: true });
        thumb.append(v); v.play().catch(() => {});
      };
      thumb.onmouseleave = () => { v && v.remove(); v = null; };
      thumb.onclick = () => openPlayer(it);
    } else thumb.append(el('div', { className: 'hint', textContent: it.status === 'converting' ? 'Converting...' : 'Not converted yet' }));
    const meta = el('div', { className: 'meta' }, el('div', { className: 'name', textContent: it.name }));
    const info = el('div', { className: 'hint' });
    if (ready) {
      info.append(el('span', { className: 'badge', textContent: `${it.fps} fps` }),
        el('span', { className: 'badge', textContent: `quality ${it.quality}${it.manual ? '' : ' (auto)'}` }),
        el('span', { className: 'badge', textContent: mb(it.size) }), el('span', { className: 'badge', textContent: it.trim.length + ' s' }));
      if (it.maskOutdated) info.append(el('span', { className: 'badge warn', textContent: 'mask outdated', title: 'Settings mask differs from the one used for this item. Re-convert to apply.' }));
    }
    if (it.error) info.append(el('span', { className: 'badge bad', textContent: 'error: ' + it.error }));
    meta.append(info);
    const prog = el('div', { className: 'progress hidden' }, el('div', { className: 'bar' }), el('span'));
    const act = el('div', { className: 'actions' });
    if (ready) {
      const slotSel = el('select', { className: 'small' });
      for (const s of ['loop', 'startup', 'both']) slotSel.append(el('option', { value: s, textContent: SLOT_NAMES[s] }));
      slotSel.value = advanced ? (it.targetSlot === 'standby' ? 'loop' : it.targetSlot || 'loop') : 'loop';
      slotSel.classList.toggle('hidden', !advanced);
      const send = el('button', { className: 'primary small', textContent: 'Send to screen' });
      const flag = el('span', { className: 'badge bad hidden', textContent: 'too big for the screen' });
      const upd = () => {
        const budget = slotInfo ? slotInfo[slotSel.value] : it.budgetBytes;
        const tooBig = it.size > budget;
        send.disabled = tooBig;
        flag.classList.toggle('hidden', !tooBig);
        flag.title = `${mb(it.size)} > ${mb(budget)} available for ${slotSel.value}. Re-convert with lower quality/fps/length.`;
      };
      slotSel.onchange = upd; upd();
      send.onclick = () => sendItem(it, slotSel.value, prog);
      act.append(el('div', { className: 'sendrow' }, send, slotSel), flag);
    }
    const conv = el('button', { className: 'small', textContent: ready ? 'Convert settings...' : 'Set up & convert...' });
    conv.onclick = () => openDialog(it);
    act.append(conv);
    if (ready) {
      const ren = el('button', { className: 'small', textContent: 'Rename' });
      ren.onclick = async () => {
        const n = prompt('New name', it.name);
        if (n) { await api('/api/library/' + it.id, 'PATCH', { name: n }); loadLibrary(); }
      };
      const sel = el('button', { className: 'small', textContent: 'Use as mask background' });
      sel.onclick = () => { selected = it.id; renderGallery(); drawMask(); };
      act.append(ren, sel);
    }
    const del = el('button', { className: 'small danger', textContent: 'Delete' });
    del.onclick = async () => {
      if (!confirm(`Delete "${it.name}" from the library?`)) return;
      await api('/api/library/' + it.id, 'DELETE'); loadLibrary();
    };
    act.append(del);
    card.append(thumb, meta, act, prog);
    card.dataset.id = it.id;
    g.append(card);
  }
}

function openPlayer(it) {
  $('playerVideo').src = `/media/${it.id}/preview.mp4?v=${encodeURIComponent(it.converted)}`;
  $('playerTitle').textContent = it.name + ' (preview with mask; click outside to close)';
  $('player').classList.remove('hidden');
}
$('player').onclick = e => { if (e.target.tagName !== 'VIDEO') { $('player').classList.add('hidden'); $('playerVideo').pause(); } };

async function sendItem(it, slot, prog) {
  const what = slot === 'both' ? 'BOTH the intro and the loop clip' : slot === 'startup' ? 'the INTRO clip' : 'the loop clip (the intro is made black)';
  if (!confirm(`Replace ${what} with "${it.name}"?\nMake sure LOVINGCOOL MONITOR is closed.`)) return;
  try {
    setBar(prog, 0, 'Connecting...');
    const { jobId } = await api(`/api/library/${it.id}/send`, 'POST', { slot });
    const r = await pollJob(jobId, j => setBar(prog, j.progress, j.message + ' ' + Math.round(j.progress * 100) + '%'));
    setBar(prog, 1, 'Sent');
    toast((r.note || 'Sent') + (r.serialWarning ? '  WARNING: ' + r.serialWarning : ''), !!r.serialWarning, r.serialWarning ? 20000 : 6000);
  } catch (e) { setBar(prog, 1, e.message, true); toast(e.message, true, 12000); }
  loadSlots().then(renderGallery);
}

// ---------- convert dialog ----------
let dlgItem = null, estSeq = 0, estTimer = null, dlgBusy = false;
const D = id => $(id);
function link(range, num, onchange) {
  range.oninput = () => { num.value = range.value; onchange(); };
  num.oninput = () => { range.value = num.value; onchange(); };
}
function dlgValues() {
  const manual = D('dQMode').value === 'manual';
  return {
    start: +D('dStart').value || 0,
    length: +D('dLen').value || 0,
    fps: Math.min(30, Math.max(10, +D('dFps').value || 20)),
    quality: manual ? Math.min(100, Math.max(1, +D('dQ').value || 90)) : 'auto',
    slot: D('dSlot').value,
  };
}
function dlgRanges() {
  const it = dlgItem, dur = it.sourceDuration;
  if (it.isImage || !dur) { D('dStartR').disabled = D('dStart').disabled = !!it.isImage; D('dStartR').max = 0; D('dLenR').max = 30; return; }
  const start = Math.min(+D('dStart').value || 0, Math.max(0, dur - 0.5));
  D('dStartR').max = Math.max(0.5, +(dur - 0.5).toFixed(1));
  D('dLenR').max = Math.max(0.5, Math.min(60, +(dur - start).toFixed(1)));
}
function dlgChanged() {
  dlgRanges();
  D('dQRow').classList.toggle('hidden', D('dQMode').value !== 'manual');
  const slot = D('dSlot').value;
  const b = slotInfo ? slotInfo[slot] : null;
  D('dBudget').textContent = b ? `Space for ${slot}: ${mb(b)}` +
    (slot === 'both' ? ' per file (half of total)' : slotInfo.assumed[slot] ? ` (other slot unknown, assuming ${mb(slotInfo.assumedBytes)})` : '') : '';
  D('dEst').className = 'est'; D('dEst').textContent = 'Estimating size...';
  clearTimeout(estTimer);
  estTimer = setTimeout(runEstimate, 700);
}
async function runEstimate() {
  const seq = ++estSeq;
  try {
    const e = await api(`/api/library/${dlgItem.id}/estimate`, 'POST', dlgValues());
    if (seq !== estSeq) return;
    const box = D('dEst');
    box.className = 'est ' + (e.fits ? 'ok' : 'bad');
    if (e.mode === 'auto') {
      box.textContent = e.fits ? `Estimated ${mb(e.bytes)} at ${e.fps} fps, quality ${e.quality}/100 (fits in ${mb(e.budget)})`
        : `Will not fit even at ${e.fps} fps and lowest quality (~${mb(e.bytes)} vs ${mb(e.budget)}). Shorten the loop.`;
    } else {
      box.textContent = `Estimated ${mb(e.bytes)} at ${e.fps} fps, quality ${e.quality}/100` +
        (e.fits ? ` (fits in ${mb(e.budget)})` : ` - TOO BIG for the screen (limit ${mb(e.budget)}); it will be kept but cannot be sent`);
    }
  } catch (err) { if (seq === estSeq) { D('dEst').className = 'est bad'; D('dEst').textContent = err.message; } }
}
function openDialog(it) {
  dlgItem = it;
  const r = it.requested || {};
  const s = settings;
  D('dlgTitle').textContent = 'Convert: ' + it.name;
  D('dlgSrc').textContent = it.isImage ? 'Still image (becomes a short clip)' : it.sourceDuration ? `Source length: ${it.sourceDuration.toFixed(1)} s` : '';
  const len = r.length || it.trim?.length || (it.isImage ? 3 : Math.min(s.maxSeconds, it.sourceDuration || s.maxSeconds));
  D('dStart').value = D('dStartR').value = r.start || 0;
  D('dLen').value = D('dLenR').value = +(+len).toFixed(2);
  D('dFps').value = D('dFpsR').value = r.fps || s.fps;
  const q = r.quality ?? s.quality;
  D('dQMode').value = q === 'auto' ? 'auto' : 'manual';
  D('dQ').value = D('dQR').value = q === 'auto' ? 90 : q;
  D('dSlot').value = advanced ? (r.slot === 'standby' ? 'loop' : r.slot || 'loop') : 'loop';
  D('dSlotRow').classList.toggle('hidden', !advanced);
  D('dProg').classList.add('hidden');
  D('dGo').disabled = false;
  D('dlg').classList.remove('hidden');
  loadSlots().then(dlgChanged);
}
for (const [r, n] of [['dStartR', 'dStart'], ['dLenR', 'dLen'], ['dFpsR', 'dFps'], ['dQR', 'dQ']]) link(D(r), D(n), dlgChanged);
D('dQMode').onchange = D('dSlot').onchange = dlgChanged;
D('dCancel').onclick = () => { if (!dlgBusy) { D('dlg').classList.add('hidden'); estSeq++; clearTimeout(estTimer); loadLibrary(); } };
D('dGo').onclick = async () => {
  dlgBusy = true; D('dGo').disabled = true; clearTimeout(estTimer); estSeq++;
  try {
    const { jobId } = await api(`/api/library/${dlgItem.id}/reconvert`, 'POST', dlgValues());
    const r = await pollJob(jobId, j => setBar(D('dProg'), j.progress, j.message + ' ' + Math.round(j.progress * 100) + '%'));
    setBar(D('dProg'), 1, 'Done');
    D('dEst').className = 'est ' + (r.tooBig ? 'bad' : 'ok');
    D('dEst').textContent = `Result: ${mb(r.size)}, ${r.fps} fps, quality ${r.quality}/100 (q=${r.q}), ${r.trim.length} s` +
      (r.tooBig ? ` - TOO BIG for the screen (limit ${mb(r.budgetBytes)}); kept but cannot be sent` : '');
    dlgItem = r;
  } catch (e) { setBar(D('dProg'), 1, e.message, true); toast(e.message, true, 12000); }
  dlgBusy = false; D('dGo').disabled = false;
  loadLibrary();
};

// ---------- import ----------
async function importFileRaw(file) {
  const box = $('importProgress');
  try {
    setBar(box, 0.1, 'Uploading ' + file.name + '...');
    const res = await fetch('/api/import?' + new URLSearchParams({ name: file.name }), { method: 'POST', body: file });
    const j = await res.json();
    if (!res.ok) throw new Error(j.error);
    setBar(box, 1, 'Added "' + file.name + '"');
    await loadLibrary();
    openDialog(j.item);
  } catch (e) { setBar(box, 1, e.message, true); toast(e.message, true, 12000); }
}
const drop = $('drop');
drop.onclick = () => $('file').click();
$('file').onchange = e => { if (e.target.files[0]) importFileRaw(e.target.files[0]); e.target.value = ''; };
drop.ondragover = e => { e.preventDefault(); drop.classList.add('over'); };
drop.ondragleave = () => drop.classList.remove('over');
drop.ondrop = e => { e.preventDefault(); drop.classList.remove('over'); if (e.dataTransfer.files[0]) importFileRaw(e.dataTransfer.files[0]); };

$('reconvertAll').onclick = async () => {
  for (const it of items.filter(i => i.status === 'ready')) {
    const card = document.querySelector(`.item[data-id="${it.id}"]`);
    const prog = card.querySelector('.progress');
    try {
      const { jobId } = await api(`/api/library/${it.id}/reconvert`, 'POST', {});
      await pollJob(jobId, j => setBar(prog, j.progress, j.message));
    } catch (e) { toast(e.message, true); }
  }
  loadLibrary();
};

// ---------- device ----------
$('checkBtn').onclick = async () => {
  const s = $('deviceStatus'), b = $('checkBtn');
  b.disabled = true; s.className = 'status'; s.textContent = 'Screen: checking...';
  try {
    const info = await api('/api/device');
    const parts = Object.entries(info).filter(([k, v]) => !k.startsWith('_')).filter(([, v]) => typeof v !== 'object').map(([k, v]) => `${k}: ${v}`);
    s.className = 'status ok'; s.textContent = 'Screen: connected (' + (info._transport === 'winusb' ? 'WinUSB' : 'serial') + ')';
    $('setupBanner').classList.toggle('hidden', !info._needsSetup);
    s.title = JSON.stringify(info, null, 2);
    toast(parts.join('  |  ') || JSON.stringify(info), false, 15000);
  } catch (e) { s.className = 'status bad'; s.textContent = 'Screen: ' + e.message; }
  b.disabled = false;
};

// ---------- mask ----------
const F = ['mMode', 'bEdge', 'bPx', 'cX', 'cY', 'cR', 'cF'];
function formToMask() {
  return {
    mode: $('mMode').value,
    band: { edge: $('bEdge').value, px: +$('bPx').value },
    circle: { cx: +$('cX').value, cy: +$('cY').value, r: +$('cR').value, feather: +$('cF').value },
  };
}
function maskToForm(m) {
  $('mMode').value = m.mode; $('bEdge').value = m.band.edge; $('bPx').value = m.band.px;
  $('cX').value = m.circle.cx; $('cY').value = m.circle.cy; $('cR').value = m.circle.r; $('cF').value = m.circle.feather;
  syncFs();
}
function syncFs() {
  const mode = $('mMode').value;
  $('fsBand').style.display = mode === 'band' ? '' : 'none';
  $('fsCircle').style.display = mode === 'circle' ? '' : 'none';
}
let bgImg = null, bgFor = null;
function drawMask() {
  const c = $('maskCanvas'), x = c.getContext('2d'), m = formToMask();
  const sel = items.find(i => i.id === selected);
  const draw = () => {
    x.clearRect(0, 0, 1200, 540);
    if (bgImg && bgImg.complete && sel) x.drawImage(bgImg, 0, 0, 1200, 540);
    else { x.fillStyle = '#2a2e36'; x.fillRect(0, 0, 1200, 540); }
    x.strokeStyle = 'rgba(255,255,255,.12)'; x.lineWidth = 1;
    for (let gx = 0; gx <= 2400; gx += 200) { x.beginPath(); x.moveTo(gx / 2, 0); x.lineTo(gx / 2, 540); x.stroke(); }
    for (let gy = 0; gy <= 1080; gy += 200) { x.beginPath(); x.moveTo(0, gy / 2); x.lineTo(1200, gy / 2); x.stroke(); }
    x.fillStyle = 'rgba(255,40,40,.65)';
    if (m.mode === 'band') {
      const p = m.band.px / 2;
      ({ left: () => x.fillRect(0, 0, p, 540), right: () => x.fillRect(1200 - p, 0, p, 540), top: () => x.fillRect(0, 0, 1200, p), bottom: () => x.fillRect(0, 540 - p, 1200, p) })[m.band.edge]();
    } else if (m.mode === 'circle') {
      const { cx, cy, r, feather } = m.circle;
      if (feather > 0) {
        const g = x.createRadialGradient(cx / 2, cy / 2, r / 2, cx / 2, cy / 2, (r + feather) / 2);
        g.addColorStop(0, 'rgba(255,40,40,.65)'); g.addColorStop(1, 'rgba(255,40,40,0)');
        x.fillStyle = g; x.fillRect(0, 0, 1200, 540);
      } else { x.beginPath(); x.arc(cx / 2, cy / 2, r / 2, 0, 7); x.fill(); }
    }
  };
  if (sel && bgFor !== sel.id) {
    bgFor = sel.id; bgImg = new Image(); bgImg.onload = draw; bgImg.src = `/media/${sel.id}/frame.jpg?v=${encodeURIComponent(sel.converted)}`;
  }
  draw();
}
for (const id of F) $(id).addEventListener('input', () => { syncFs(); drawMask(); });
$('saveMask').onclick = async () => {
  try {
    settings = await api('/api/settings', 'PUT', { ...settings, mask: formToMask() });
    maskToForm(settings.mask); drawMask();
    $('maskMsg').textContent = 'Saved.';
    loadLibrary();
  } catch (e) { toast(e.message, true); }
};

(async function init() {
  settings = await api('/api/settings');
  maskToForm(settings.mask);
  await loadSlots();
  await loadLibrary();
})().catch(e => toast(e.message, true));

// ---------- driver setup ----------
async function runSetupJob(url, label, box) {
  try {
    const { jobId } = await api(url, 'POST');
    const r = await pollJob(jobId, j => setBar(box, j.progress, j.message));
    setBar(box, 1, 'Done');
    toast(r.warning || label + ' finished.', !!r.warning, 12000);
    $('setupBanner').classList.add('hidden');
  } catch (e) { setBar(box, 1, e.message, true); toast(e.message, true, 15000); }
}
$('setupBtn').onclick = () => {
  if (!confirm("One-time setup: this replaces the screen's standard COM-port driver with Microsoft's WinUSB driver so the screen keeps its display on. Windows will ask for administrator approval. The vendor app will no longer see the screen until you restore the original driver (Advanced section).\n\nContinue?")) return;
  runSetupJob('/api/setup', 'Setup', $('setupProg'));
};
$('restoreBtn').onclick = () => {
  if (!confirm('Restore the original COM-port driver? Afterwards the screen will turn its display off 30 s after any program closes its port (this is how the vendor software works).')) return;
  runSetupJob('/api/restore', 'Restore', $('setupProg'));
};
$('advToggle').onchange = e => { advanced = e.target.checked; renderGallery(); };
