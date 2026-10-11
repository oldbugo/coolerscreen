// The one door between the UI and the Rust core (docs/v2/api-contract.md).
//
//   backend.call(command, args)   -> Promise (rejects with Error{status, code, failedUsb, data})
//   backend.on(event, handler)    -> unsubscribe fn. Events: 'activity' ({rev, deviceBusy, jobs}), 'app' ({})
//   backend.job(jobId, onUpdate)  -> Promise of job.result (rejects with the job's error)
//   backend.mediaUrl(id, file, kind = 'o')   URL on the core's local media server (Range requests, seeking)
//   backend.refresh()             -> re-read the activity snapshot now
//   backend.importFile(file)      -> { id }
//
// Commands are Tauri commands; progress arrives as events (cs:job, cs:library, cs:device, cs:app), never by polling.
const T = window.__TAURI_INTERNALS__;
if (!T) document.body.textContent = 'CoolerScreen must be started from its own window (CoolerScreen.exe).';

const handlers = { activity: new Set(), app: new Set() };
const emit = (ev, payload) => { for (const h of handlers[ev] || []) { try { h(payload); } catch (e) { console.error(e); } } };

function tauriError(x) {
  if (x instanceof Error) return x;
  const o = typeof x === 'string' ? { error: x } : x || {};
  return Object.assign(new Error(o.error || o.message || 'Something went wrong.'), { status: o.status, code: o.code, failedUsb: !!o.failedUsb, data: o });
}

let mediaBase = '';
const jobs = new Map();
const jobWaiters = new Set();
let rev = -1, deviceBusy = false, pushT = null;

const call = async (command, args = {}) => {
  try { return await T.invoke(command, args); } catch (x) { throw tauriError(x); }
};
const listen = (event, fn) =>
  T.invoke('plugin:event|listen', { event, target: { kind: 'Any' }, handler: T.transformCallback(e => fn(e.payload)) });

function snapshot() {
  const now = Date.now();
  for (const [id, j] of jobs) if (j.finishedAt && now - j.finishedAt > 120000) jobs.delete(id);
  return { rev, deviceBusy, jobs: [...jobs.values()] };
}
function push() {
  if (pushT) return;
  pushT = setTimeout(() => { pushT = null; emit('activity', snapshot()); }, 60);
}
async function refresh() {
  try {
    const a = await call('activity');
    jobs.clear();
    for (const j of a.jobs) jobs.set(j.id, j);
    rev = a.rev; deviceBusy = a.deviceBusy;
    emit('activity', snapshot());
  } catch {}
}

export const backend = {
  call,
  on(event, handler) { handlers[event].add(handler); return () => handlers[event].delete(handler); },
  refresh,
  start() {
    listen('cs:job', j => { jobs.set(j.id, j); for (const w of jobWaiters) w(j); push(); });
    listen('cs:library', p => { rev = p.rev; push(); });
    listen('cs:device', p => { deviceBusy = !!p.busy; push(); });
    listen('cs:app', () => emit('app', {}));
    refresh();
  },
  job(jobId, onUpdate) {
    return new Promise((resolve, reject) => {
      const done = j => {
        onUpdate && onUpdate(j);
        if (j.status === 'done') { jobWaiters.delete(on); resolve(j.result); return true; }
        if (j.status === 'error') { jobWaiters.delete(on); reject(Object.assign(new Error(j.error), { code: j.code, failedUsb: !!j.failedUsb, data: j })); return true; }
        return false;
      };
      const on = j => { if (j.id === jobId) done(j); };
      jobWaiters.add(on);
      call('job_get', { jobId }).then(done, e => { jobWaiters.delete(on); reject(e); }); // it may have finished already
    });
  },
  async importFile(file) {
    try { return await T.invoke('import_bytes', await file.arrayBuffer(), { headers: { 'x-cs-name': encodeURIComponent(file.name) } }); }
    catch (x) { throw tauriError(x); }
  },
  /** Called with AppState.mediaBase (the core media server root). */
  setMediaBase(base) { if (base) mediaBase = String(base).replace(/\/+$/, ''); },
  /** Cached media: kind 'o' (original: thumb.jpg, frame.jpg, proxy.mp4, ...) or 'v' (version: preview.mp4, thumb.jpg). */
  mediaUrl(id, file, kind = 'o') { return `${mediaBase}/media/${kind}/${id}/${file}`; },
};
