// Dev/test only (?mock=tauri): fakes window.__TAURI_INTERNALS__ on top of the 1.2 HTTP server so the Tauri
// transport in backend.js (invoke + cs:* events, no polling in the UI) can be exercised in a plain browser.
const cbs = new Map(); let cbId = 1;
const listeners = new Map(); // event -> [handler ids]
const emit = (event, payload) => { for (const id of listeners.get(event) || []) { const f = cbs.get(id); if (f) f({ event, payload }); } };

window.__TAURI_INTERNALS__ = {
  transformCallback(fn) { const id = cbId++; cbs.set(id, fn); return id; },
  async invoke(cmd, args = {}) {
    if (cmd === 'plugin:event|listen') { (listeners.get(args.event) || listeners.set(args.event, []).get(args.event)).push(args.handler); return 1; }
    if (cmd.startsWith('win_')) return false;
    const route = window.__CS_ROUTES__[cmd];
    if (route === undefined) throw { error: 'unknown command ' + cmd, status: 404 };
    if (!route) return null;
    const [method, path, body] = route;
    const payload = body ? body(args) : undefined;
    const r = await fetch(path(args), { method, headers: payload ? { 'Content-Type': 'application/json' } : {}, body: payload ? JSON.stringify(payload) : undefined });
    const j = await r.json().catch(() => ({}));
    if (!r.ok) throw { error: j.error || r.statusText, code: j.code || null, status: r.status, failedUsb: j.failedUsb };
    if (cmd === 'app_state') j.mediaBase = location.origin;
    return j;
  },
};

// event source: diff /api/activity into cs:job / cs:library / cs:device
const seen = new Map();
let rev = null, busy = null;
setInterval(async () => {
  try {
    const a = await (await fetch('/api/activity')).json();
    for (const j of a.jobs) { const h = JSON.stringify(j); if (seen.get(j.id) !== h) { seen.set(j.id, h); emit('cs:job', j); } }
    if (a.rev !== rev) { rev = a.rev; emit('cs:library', { rev }); }
    if (a.deviceBusy !== busy) { busy = a.deviceBusy; emit('cs:device', { busy }); }
  } catch {}
}, 400);
