const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { exec, execFile } = require('child_process');
const paths = require('./paths');
const settings = require('./settings');
const lib = require('./library');
const deviceOps = require('./deviceOps');
const send = require('./send');
const gate = require('./gate');
const gallery = require('./gallery');
const setup = require('./setup');
const slots = require('./slots');
const { DEVICE_FPS, IMAGE_SECONDS } = require('./convert');
const pkg = require('../package.json');

const PUBLIC = path.join(__dirname, '..', 'public');
const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.mp4': 'video/mp4', '.m4v': 'video/mp4', '.webm': 'video/webm', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg',
  '.png': 'image/png', '.webp': 'image/webp', '.gif': 'image/gif', '.json': 'application/json', '.svg': 'image/svg+xml',
};

// ---------- jobs (shown in the Activity strip) ----------
const jobs = new Map();
const estimateSeq = new Map();

function newJob(kind, info = {}) {
  const job = { id: crypto.randomBytes(6).toString('hex'), kind, status: 'running', progress: 0, message: '', startedAt: Date.now(), ...info };
  jobs.set(job.id, job);
  if (jobs.size > 100) jobs.delete(jobs.keys().next().value);
  return job;
}

// A job that waits its turn in the shared gate (see src/gate.js): 'waiting' + a message until it starts.
function gatedJob(gateKind, kind, info, fn) {
  const job = newJob(kind, info);
  job.status = 'waiting';
  gate.run(gateKind, () => { job.status = 'running'; job.message = ''; job.runningSince = Date.now(); return fn(job); }, {
    onWait: msg => { if (msg) { job.status = 'waiting'; job.message = msg; } },
  }).then(
    r => { job.status = 'done'; job.progress = 1; job.result = r; job.finishedAt = Date.now(); },
    e => {
      job.status = 'error'; job.error = e.message || String(e); job.code = e.code || null; job.failedUsb = !!e.failedUsb;
      job.finishedAt = Date.now();
    },
  );
  return job;
}

const publicJob = j => {
  const { result, ...rest } = j;
  return { ...rest, result: result && typeof result === 'object' ? { warning: result.warning, serialWarning: result.serialWarning, name: result.name, label: result.label } : undefined };
};

function activity() {
  const now = Date.now();
  const list = [...jobs.values()].filter(j => j.status === 'waiting' || j.status === 'running' || now - (j.finishedAt || 0) < 120000);
  return { rev: lib.rev(), deviceBusy: gate.deviceActive(), jobs: list.map(publicJob) };
}

// ---------- helpers ----------
const json = (res, code, obj) => {
  res.writeHead(code, { 'Content-Type': 'application/json', 'Cache-Control': 'no-store' });
  res.end(JSON.stringify(obj));
};

function readBody(req, limit = 1e6) {
  return new Promise((resolve, reject) => {
    let n = 0; const chunks = [];
    req.on('data', c => { n += c.length; if (n > limit) { reject(new Error('Body too large')); req.destroy(); } else chunks.push(c); });
    req.on('end', () => { try { resolve(chunks.length ? JSON.parse(Buffer.concat(chunks).toString()) : {}); } catch (e) { reject(e); } });
    req.on('error', reject);
  });
}

function serveFile(req, res, file, cache = false) {
  const stat = fs.statSync(file);
  const type = TYPES[path.extname(file).toLowerCase()] || 'application/octet-stream';
  const headers = { 'Content-Type': type, 'Accept-Ranges': 'bytes', 'Cache-Control': cache ? 'max-age=60' : 'no-cache' };
  const range = /bytes=(\d*)-(\d*)/.exec(req.headers.range || '');
  if (range) {
    let start = range[1] ? +range[1] : 0;
    let end = range[2] ? +range[2] : stat.size - 1;
    if (!range[1] && range[2]) { start = Math.max(0, stat.size - +range[2]); end = stat.size - 1; }
    end = Math.min(end, stat.size - 1);
    if (start > end) { res.writeHead(416, { 'Content-Range': `bytes */${stat.size}` }); return res.end(); }
    res.writeHead(206, { ...headers, 'Content-Range': `bytes ${start}-${end}/${stat.size}`, 'Content-Length': end - start + 1 });
    return fs.createReadStream(file, { start, end }).pipe(res);
  }
  res.writeHead(200, { ...headers, 'Content-Length': stat.size });
  fs.createReadStream(file).pipe(res);
}

/** What the screen is showing, as far as this app knows, resolved against the library. */
function onScreen() {
  const s = settings.load();
  const sb = s.slots.standby;
  if (!sb) return null;
  const f = sb.versionId ? lib.findVersion(sb.versionId) : null;
  const o = f ? lib.readOriginal(f.oid) : null;
  return {
    ...sb, known: !!sb.versionId, exists: !!f,
    originalName: o ? o.name : sb.name, label: f ? (f.v.label || `v${f.v.n}`) : sb.label,
    thumbUrl: f ? `/media/v/${sb.versionId}/thumb.jpg` : null, originalId: f ? f.oid : sb.originalId,
  };
}

function appState() {
  const s = settings.load();
  const b = slots.budgets(s);
  return {
    appVersion: pkg.version, deviceFps: DEVICE_FPS, imageSeconds: IMAGE_SECONDS, fake: deviceOps.FAKE || null,
    mask: s.mask, maxSeconds: s.maxSeconds, budget: b.loop, total: b.total, uploadRate: s.uploadRate || 50000,
    galleryDir: gallery.galleryDir(), onScreen: onScreen(), missingNoticeDays: gallery.MISSING_NOTICE_DAYS,
    notchKey: lib.notchKey(s.mask), oldNotchCount: oldNotchCount(lib.notchKey(s.mask)),
  };
}

function oldNotchCount(key) {
  let n = 0;
  for (const o of lib.listOriginals()) for (const v of lib.versions(o.id)) if (v.status === 'ready' && v.notchKey !== key) n++;
  return n;
}

function originalsList() {
  const ctx = lib.context();
  const all = lib.listOriginals();
  return {
    rev: lib.rev(),
    originals: all.map(o => lib.summary(o, ctx)),
    longMissing: gallery.longMissing().map(o => ({ id: o.id, name: o.name, since: o.missing.since })),
  };
}

function versionJob(oid, v) {
  const o = lib.readOriginal(oid);
  return gatedJob('convert', 'version', { originalId: oid, versionId: v.id, label: `${o ? o.name : 'clip'} · v${v.n}` },
    j => lib.runVersion(v.id, (f, msg) => { j.progress = f; j.message = msg; }));
}

// ---------- routes ----------
async function handle(req, res) {
  const url = new URL(req.url, 'http://x');
  const p = decodeURIComponent(url.pathname);
  const m = req.method;
  let mm;

  // Basic DNS-rebinding / cross-site protection: only accept our own host and same-origin writes.
  const host = req.headers.host || '';
  if (!/^(127\.0\.0\.1|localhost)(:\d+)?$/.test(host)) return json(res, 403, { error: 'Forbidden host' });
  if (m !== 'GET' && req.headers.origin && new URL(req.headers.origin).host !== host) return json(res, 403, { error: 'Forbidden origin' });

  if (p.startsWith('/api/')) {
    if (p === '/api/state' && m === 'GET') return json(res, 200, appState());
    if (p === '/api/settings' && m === 'PUT') {
      const body = await readBody(req);
      const cur = settings.load();
      settings.save({ ...cur, mask: body.mask || cur.mask, maxSeconds: body.maxSeconds ?? cur.maxSeconds });
      lib.bump();
      return json(res, 200, appState());
    }
    if (p === '/api/activity' && m === 'GET') return json(res, 200, activity());
    if ((mm = /^\/api\/jobs\/([a-f0-9]+)$/.exec(p)) && m === 'GET') {
      const j = jobs.get(mm[1]);
      return j ? json(res, 200, publicJob(j)) : json(res, 404, { error: 'No such job' });
    }

    // ----- originals -----
    if (p === '/api/originals' && m === 'GET') return json(res, 200, originalsList());
    if (p === '/api/import' && m === 'POST') {
      const name = url.searchParams.get('name') || 'upload';
      const tmp = path.join(paths.base, 'incoming-' + crypto.randomBytes(4).toString('hex') + path.extname(name).replace(/[^.a-zA-Z0-9]/g, ''));
      fs.mkdirSync(paths.base, { recursive: true });
      await new Promise((resolve, reject) => {
        const ws = fs.createWriteStream(tmp);
        req.pipe(ws); ws.on('finish', resolve); ws.on('error', reject); req.on('error', reject);
      });
      try { const id = await gallery.addFile(tmp, path.basename(name)); return json(res, 200, { id }); }
      catch (e) { return json(res, 400, { error: e.message }); }
      finally { fs.rmSync(tmp, { force: true }); }
    }
    // Import by local path (Electron drag and drop): no upload, the file is copied into the Gallery folder.
    if (p === '/api/import-path' && m === 'POST') {
      const src = String((await readBody(req)).path || '');
      if (!path.isAbsolute(src) || !fs.existsSync(src) || !fs.statSync(src).isFile()) return json(res, 400, { error: 'File not found: ' + src });
      try { return json(res, 200, { id: await gallery.addFile(src, path.basename(src)) }); }
      catch (e) { return json(res, 400, { error: e.message }); }
    }
    if (p === '/api/missing/delete' && m === 'POST') {
      return json(res, 200, { deleted: gallery.deleteMissing((await readBody(req)).ids) });
    }
    if ((mm = /^\/api\/originals\/([a-z0-9]+)(\/[a-z]+)?$/.exec(p))) {
      const oid = mm[1], sub = mm[2] || '';
      if (!lib.readOriginal(oid)) return json(res, 404, { error: 'This clip is no longer in the library.' });
      if (!sub && m === 'GET') return json(res, 200, lib.detail(oid));
      if (!sub && m === 'PATCH') {
        const body = await readBody(req);
        try {
          if (body.name != null) gallery.renameOriginal(oid, body.name);
          if (body.draft !== undefined) {
            const o = lib.readOriginal(oid);
            lib.patchOriginal(oid, { draft: body.draft ? lib.cleanSettings(o, body.draft, settings.load().maxSeconds) : null });
          }
          return json(res, 200, lib.detail(oid));
        } catch (e) { return json(res, 400, { error: e.message }); }
      }
      if (!sub && m === 'DELETE') {
        try { await gallery.deleteOriginal(oid); return json(res, 200, { ok: true }); }
        catch (e) { return json(res, 400, { error: e.message }); }
      }
      if (sub === '/reveal' && m === 'POST') {
        const o = lib.readOriginal(oid);
        if (process.platform === 'win32' && o.file && fs.existsSync(o.file)) execFile('explorer.exe', ['/select,', o.file], () => {});
        else exec(process.platform === 'win32' ? `explorer.exe "${gallery.galleryDir()}"` : `xdg-open "${gallery.galleryDir()}"`, () => {});
        return json(res, 200, { ok: true });
      }
      if (sub === '/estimate' && m === 'POST') {
        if (gate.deviceActive()) return json(res, 409, { error: 'Size check paused while the screen is updating.', code: 'busy' });
        const body = await readBody(req);
        const seq = (estimateSeq.get(oid) || 0) + 1;
        estimateSeq.set(oid, seq);
        try {
          // Newer estimate requests for the same original supersede queued older ones.
          return json(res, 200, await gate.run('convert', () => (estimateSeq.get(oid) === seq ? lib.estimateFor(oid, body) : { stale: true })));
        } catch (e) { return json(res, 400, { error: e.message }); }
      }
      if (sub === '/versions' && m === 'POST') {
        const body = await readBody(req);
        try {
          const v = lib.createVersion(oid, body);
          const job = versionJob(oid, v);
          return json(res, 200, { versionId: v.id, jobId: job.id });
        } catch (e) { return json(res, 400, { error: e.message, code: e.code || null, versionId: e.versionId || null }); }
      }
      return json(res, 404, { error: 'Not found' });
    }

    // ----- versions -----
    if ((mm = /^\/api\/versions\/([a-z0-9]+)(\/[a-z]+)?$/.exec(p))) {
      const vid = mm[1], sub = mm[2] || '';
      const f = lib.findVersion(vid);
      if (!f) return json(res, 404, { error: 'This version no longer exists.' });
      if (!sub && m === 'PATCH') {
        const label = String((await readBody(req)).label || '').trim().slice(0, 40);
        return json(res, 200, lib.patchVersion(vid, { label: label || null }));
      }
      if (!sub && m === 'DELETE') {
        if (f.v.status === 'queued' || f.v.status === 'converting') return json(res, 409, { error: 'This version is still being prepared. Delete it when it has finished.' });
        lib.deleteVersion(vid);
        return json(res, 200, { ok: true });
      }
      if (sub === '/send' && m === 'POST') {
        const o = lib.readOriginal(f.oid);
        const job = gatedJob('device', 'send', { originalId: f.oid, versionId: vid, label: `${o ? o.name : 'clip'} · ${f.v.label || 'v' + f.v.n}` },
          j => send.sendVersion(vid, (fr, msg, info) => { j.progress = fr; j.message = msg; if (info) { j.bytesTotal = info.bytesTotal; j.bytesSent = info.bytesSent; } }));
        return json(res, 200, { jobId: job.id });
      }
      if ((sub === '/remake' || sub === '/retry') && m === 'POST') {
        try {
          let v;
          if (sub === '/retry' && f.v.status === 'error') { v = lib.patchVersion(vid, { status: 'queued', error: null }); }
          else v = lib.remakeVersion(vid);
          const job = versionJob(f.oid, v);
          return json(res, 200, { versionId: v.id, jobId: job.id });
        } catch (e) { return json(res, 400, { error: e.message }); }
      }
      return json(res, 404, { error: 'Not found' });
    }

    // ----- gallery folder -----
    if (p === '/api/gallery' && m === 'PUT') {
      try { await gallery.setDir((await readBody(req)).dir); return json(res, 200, appState()); }
      catch (e) { return json(res, 400, { error: e.message }); }
    }
    if (p === '/api/gallery/open' && m === 'POST') {
      const dir = gallery.galleryDir();
      fs.mkdirSync(dir, { recursive: true });
      exec(process.platform === 'win32' ? `explorer.exe "${dir}"` : process.platform === 'darwin' ? `open "${dir}"` : `xdg-open "${dir}"`, () => {});
      return json(res, 200, { dir });
    }
    if (p === '/api/gallery/sync' && m === 'POST') { await gallery.sync(); return json(res, 200, { ok: true }); }

    // ----- screen -----
    if (p === '/api/transport' && m === 'GET') {
      if (gate.deviceActive()) return json(res, 409, { error: 'The screen is busy with another operation.', code: 'updating' });
      return json(res, 200, await gate.run('device', () => deviceOps.state()));
    }
    if ((p === '/api/setup' || p === '/api/restore' || p === '/api/reconnect') && m === 'POST') {
      const fn = p === '/api/setup' ? setup.setupWinUsb : p === '/api/reconnect' ? setup.reconnect : setup.restoreSerial;
      const label = p === '/api/setup' ? 'Setting up the screen' : p === '/api/reconnect' ? 'Reconnecting the screen' : 'Restoring the original driver';
      const job = gatedJob('device', 'setup', { label }, j => fn((f, msg) => { j.progress = f; j.message = msg; }));
      return json(res, 200, { jobId: job.id });
    }
    // One short check (opens the screen briefly). The UI calls it once at launch, after setup/reconnect and on
    // "Check again"; it never polls.
    if (p === '/api/device' && m === 'GET') {
      if (gate.deviceActive()) return json(res, 409, { error: 'The screen is busy with another operation.', code: 'updating' });
      try { return json(res, 200, await gate.run('device', () => deviceOps.checkScreen())); }
      catch (e) { return json(res, 502, { error: e.message, failedUsb: !!e.failedUsb, code: e.code || (e.failedUsb ? 'failedUsb' : null) }); }
    }
    return json(res, 404, { error: 'Not found' });
  }

  if (m !== 'GET') return json(res, 405, { error: 'Method not allowed' });

  if ((mm = /^\/media\/o\/([a-z0-9]+)\/([a-z0-9.]+)$/.exec(p))) {
    const f = lib.mediaPathO(mm[1], mm[2]);
    if (!f) { res.writeHead(404); return res.end(); }
    return serveFile(req, res, f);
  }
  if ((mm = /^\/media\/v\/([a-z0-9]+)\/([a-z0-9.]+)$/.exec(p))) {
    const f = lib.mediaPathV(mm[1], mm[2]);
    if (!f) { res.writeHead(404); return res.end(); }
    return serveFile(req, res, f);
  }

  const rel = p === '/' ? 'index.html' : p.slice(1);
  const f = path.join(PUBLIC, rel);
  if (!f.startsWith(PUBLIC + path.sep) || !fs.existsSync(f) || !fs.statSync(f).isFile()) { res.writeHead(404); return res.end('Not found'); }
  serveFile(req, res, f);
}

function openBrowser(url) {
  const cmd = process.platform === 'win32' ? `start "" "${url}"` : process.platform === 'darwin' ? `open "${url}"` : `xdg-open "${url}"`;
  exec(cmd, () => {});
}

async function start({ port = Number(process.env.PORT) || 8765, open = true } = {}) {
  try { lib.resetInterrupted(); } catch {}
  try { await gallery.init({ assets: true }); } catch (e) { console.log('Gallery init failed: ' + e.message); }
  const server = http.createServer((req, res) => {
    handle(req, res).catch(e => { if (!res.headersSent) json(res, 500, { error: e.message }); else res.end(); });
  });
  return new Promise((resolve, reject) => {
    let tries = 0;
    const listen = p => server.listen(p, '127.0.0.1');
    server.on('error', e => {
      if (e.code === 'EADDRINUSE' && tries++ < 10) listen(port + tries); else reject(e);
    });
    server.on('listening', () => {
      const url = `http://127.0.0.1:${server.address().port}/`;
      console.log(`CoolerScreen running at ${url}  (Ctrl+C to quit)`);
      if (open) openBrowser(url);
      resolve({ server, url });
    });
    listen(port);
  });
}

/** True while something is being written to the screen (Electron asks before closing the window). */
const uploading = () => [...jobs.values()].some(j => (j.kind === 'send' || j.kind === 'setup') && j.status === 'running');

module.exports = { start, uploading };
