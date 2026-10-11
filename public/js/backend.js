// The one door between the UI and the backend.
//
//   backend.call(command, args)   -> Promise (rejects with Error{status, code, failedUsb, data})
//   backend.on(event, handler)    -> unsubscribe fn. Events: 'activity' ({rev, deviceBusy, jobs}), 'app' ({})
//   backend.job(jobId, onUpdate)  -> Promise of job.result (rejects with the job's error)
//   backend.mediaUrl(id, file, kind = 'o')  /  backend.mediaPath(serverUrl)
//   backend.refresh()             -> ask for a fresh activity snapshot now (HTTP: poll sooner; Tauri: re-read)
//   backend.importFile(file, localPath?)  -> { id }
//
// Command names, arguments and results follow docs/v2/api-contract.md (WS1). Two transports:
//   http  : the 1.2 Node server (and the Phase 0 sidecar). Commands map to endpoints in ROUTES below;
//           job/library/device events are synthesised by polling /api/activity (the only place that polls).
//   tauri : invoke() + listen() on cs:job / cs:library / cs:app / cs:device. No polling.
// Chosen at startup: Tauri when the page is served by the app itself (tauri.localhost), HTTP otherwise.
// Override with ?transport=http|tauri.

const q = new URLSearchParams(location.search).get('transport');
if (new URLSearchParams(location.search).get('mock') === 'tauri') await import('./dev/tauri-mock.js'); // dev/test only
const T = window.__TAURI_INTERNALS__;
const isTauriOrigin = location.hostname === 'tauri.localhost' || location.protocol === 'tauri:';
export const transport = q === 'http' ? 'http' : q === 'tauri' ? (T ? 'tauri' : 'http') : T && isTauriOrigin ? 'tauri' : 'http';

const omit = (a, ...keys) => { const o = { ...a }; for (const k of keys) delete o[k]; return o; };
const enc = encodeURIComponent;

// command -> [method, path(args), body(args)?]. The whole HTTP mapping lives here.
export const ROUTES = window.__CS_ROUTES__ = {
  app_state: ['GET', () => '/api/state'],
  settings_update: ['PUT', () => '/api/settings', a => a],
  activity: ['GET', () => '/api/activity'],
  job_get: ['GET', a => `/api/jobs/${a.jobId}`],
  originals_list: ['GET', () => '/api/originals'],
  missing_delete: ['POST', () => '/api/missing/delete', a => a],
  original_get: ['GET', a => `/api/originals/${a.originalId}`],
  original_patch: ['PATCH', a => `/api/originals/${a.originalId}`, a => omit(a, 'originalId')],
  original_delete: ['DELETE', a => `/api/originals/${a.originalId}`],
  original_reveal: ['POST', a => `/api/originals/${a.originalId}/reveal`],
  estimate: ['POST', a => `/api/originals/${a.originalId}/estimate`, a => a.settings],
  loop_search: ['POST', a => `/api/originals/${a.originalId}/loopsearch`, a => omit(a, 'originalId')],
  loop_point: ['POST', a => `/api/originals/${a.originalId}/looppoint`, a => omit(a, 'originalId')],
  version_create: ['POST', a => `/api/originals/${a.originalId}/versions`, a => a.settings],
  version_patch: ['PATCH', a => `/api/versions/${a.versionId}`, a => omit(a, 'versionId')],
  version_delete: ['DELETE', a => `/api/versions/${a.versionId}`],
  version_send: ['POST', a => `/api/versions/${a.versionId}/send`],
  version_remake: ['POST', a => `/api/versions/${a.versionId}/remake`],
  version_retry: ['POST', a => `/api/versions/${a.versionId}/retry`],
  sources_list: ['GET', () => '/api/sources'],
  browse_set: ['PUT', () => '/api/browse', a => a],
  link_add: ['POST', () => '/api/link', a => a],
  source_set: ['PUT', a => `/api/sources/${a.sourceId}`, a => omit(a, 'sourceId')],
  source_items: ['GET', a => `/api/sources/${a.sourceId}/items?page=${a.page || 1}${a.refresh ? '&refresh=1' : ''}`],
  source_add: ['POST', a => `/api/sources/${a.sourceId}/items/${enc(a.itemId)}/add`, a => omit(a, 'sourceId', 'itemId')],
  gallery_set_dir: ['PUT', () => '/api/gallery', a => a],
  gallery_open: ['POST', () => '/api/gallery/open'],
  gallery_sync: ['POST', () => '/api/gallery/sync'],
  device_state: ['GET', () => '/api/transport'],
  device_check: ['GET', () => '/api/device'],
  device_restart: ['POST', () => '/api/restart'],
  device_setup: ['POST', () => '/api/setup'],
  device_restore: ['POST', () => '/api/restore'],
  device_reconnect: ['POST', () => '/api/reconnect'],
  // No 1.2 endpoint: harmless no-ops on HTTP.
  set_focus: null,
  job_cancel: null,
};

const handlers = { activity: new Set(), app: new Set() };
const emit = (ev, payload) => { for (const h of handlers[ev] || []) { try { h(payload); } catch (e) { console.error(e); } } };

function httpError(r, j) {
  const e = new Error(j.error || r.statusText);
  return Object.assign(e, { status: r.status, code: j.code, failedUsb: !!j.failedUsb, data: j });
}
function tauriError(x) {
  if (x instanceof Error) return x;
  const o = typeof x === 'string' ? { error: x } : x || {};
  return Object.assign(new Error(o.error || o.message || 'Something went wrong.'), { status: o.status, code: o.code, failedUsb: !!o.failedUsb, data: o });
}

// ---------------- HTTP transport ----------------
const http = {
  async call(command, args = {}) {
    if (!(command in ROUTES)) throw new Error('Unknown command ' + command);
    const route = ROUTES[command];
    if (!route) return command === 'job_cancel' ? { cancelled: false } : null;
    const [method, path, body] = route;
    const payload = body ? body(args) : undefined;
    const r = await fetch(path(args), { method, headers: payload ? { 'Content-Type': 'application/json' } : {}, body: payload ? JSON.stringify(payload) : undefined });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw httpError(r, j);
    return j;
  },
  mediaBase: '',
  async importFile(file, localPath) {
    const res = localPath
      ? await fetch('/api/import-path', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ path: localPath }) })
      : await fetch('/api/import?' + new URLSearchParams({ name: file.name }), { method: 'POST', body: file });
    const j = await res.json().catch(() => ({}));
    if (!res.ok) throw new Error(j.error || 'Could not add the file.');
    return j;
  },
  // Polling lives here only: fast while something runs, slow when idle or hidden (never touches the screen).
  start() {
    let timer = null, running = false;
    const tick = async () => {
      clearTimeout(timer);
      if (running) return;
      running = true;
      let busy = false;
      try {
        const a = await http.call('activity');
        busy = a.jobs.some(j => j.status === 'running' || j.status === 'waiting');
        emit('activity', a);
      } catch {}
      running = false;
      timer = setTimeout(tick, busy ? 1000 : document.hidden ? 6000 : 3000);
    };
    http.refresh = () => { clearTimeout(timer); timer = setTimeout(tick, 300); };
    tick();
  },
  refresh() {},
  async job(jobId, onUpdate) {
    for (;;) {
      const j = await http.call('job_get', { jobId });
      onUpdate && onUpdate(j);
      if (j.status === 'done') return j.result;
      if (j.status === 'error') throw Object.assign(new Error(j.error), { code: j.code, failedUsb: !!j.failedUsb, data: j });
      await new Promise(res => setTimeout(res, 400));
    }
  },
};

// ---------------- Tauri transport ----------------
const tauri = {
  mediaBase: '',
  jobs: new Map(), rev: -1, deviceBusy: false, listeners: new Set(),
  async call(command, args = {}) {
    try { return await T.invoke(command, args); } catch (x) { throw tauriError(x); }
  },
  listen(event, fn) {
    return T.invoke('plugin:event|listen', { event, target: { kind: 'Any' }, handler: T.transformCallback(e => fn(e.payload)) });
  },
  async importFile(file, localPath) {
    try {
      if (localPath) return await T.invoke('import_path', { path: localPath });
      return await T.invoke('import_bytes', await file.arrayBuffer(), { headers: { 'x-cs-name': encodeURIComponent(file.name) } });
    } catch (x) { throw tauriError(x); }
  },
  snapshot() {
    const now = Date.now();
    for (const [id, j] of this.jobs) if (j.finishedAt && now - j.finishedAt > 120000) this.jobs.delete(id);
    return { rev: this.rev, deviceBusy: this.deviceBusy, jobs: [...this.jobs.values()] };
  },
  push() {
    if (this.pushT) return;
    this.pushT = setTimeout(() => { this.pushT = null; emit('activity', this.snapshot()); }, 60);
  },
  async refresh() {
    try {
      const a = await this.call('activity');
      this.jobs = new Map(a.jobs.map(j => [j.id, j]));
      this.rev = a.rev; this.deviceBusy = a.deviceBusy;
      emit('activity', this.snapshot());
    } catch {}
  },
  start() {
    this.listen('cs:job', j => {
      this.jobs.set(j.id, j);
      for (const l of this.listeners) l(j);
      this.push();
    });
    this.listen('cs:library', p => { this.rev = p.rev; this.push(); });
    this.listen('cs:device', p => { this.deviceBusy = !!p.busy; this.push(); });
    this.listen('cs:app', () => emit('app', {}));
    this.refresh();
  },
  async job(jobId, onUpdate) {
    return new Promise((resolve, reject) => {
      const done = j => {
        onUpdate && onUpdate(j);
        if (j.status === 'done') { this.listeners.delete(on); resolve(j.result); return true; }
        if (j.status === 'error') { this.listeners.delete(on); reject(Object.assign(new Error(j.error), { code: j.code, failedUsb: !!j.failedUsb, data: j })); return true; }
        return false;
      };
      const on = j => { if (j.id === jobId) done(j); };
      this.listeners.add(on);
      this.call('job_get', { jobId }).then(done, e => { this.listeners.delete(on); reject(e); }); // the job may have finished already
    });
  },
};

const impl = transport === 'tauri' ? tauri : http;

export const backend = {
  transport,
  call: (command, args) => impl.call(command, args),
  on(event, handler) { handlers[event].add(handler); return () => handlers[event].delete(handler); },
  job: (jobId, onUpdate) => impl.job(jobId, onUpdate),
  refresh: () => impl.refresh(),
  importFile: (file, localPath) => impl.importFile(file, localPath),
  /** Called with AppState.mediaBase (Tauri core media server root). */
  setMediaBase(base) { if (base) impl.mediaBase = String(base).replace(/\/+$/, ''); },
  /** URL of a cached media file: kind 'o' (original: thumb.jpg, frame.jpg, proxy.mp4, ...) or 'v' (version: preview.mp4, thumb.jpg). */
  mediaUrl(id, file, kind = 'o') { return `${impl.mediaBase}/media/${kind}/${id}/${file}`; },
  /** Server-provided URLs are already absolute in Tauri; in HTTP mode they are same-origin paths. Pass-through. */
  mediaPath: u => u,
  start() { impl.start(); },
};
