const http = require('http');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { exec } = require('child_process');
const paths = require('./paths');
const settings = require('./settings');
const lib = require('./library');
const deviceOps = require('./deviceOps');
const send = require('./send');
const setup = require('./setup');
const slots = require('./slots');
const { probe } = require('./convert');

const PUBLIC = path.join(__dirname, '..', 'public');
const TYPES = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.mp4': 'video/mp4', '.jpg': 'image/jpeg', '.json': 'application/json', '.svg': 'image/svg+xml',
};

const jobs = new Map();
let deviceBusy = false;

function newJob(kind) {
  const job = { id: crypto.randomBytes(6).toString('hex'), kind, status: 'running', progress: 0, message: '' };
  jobs.set(job.id, job);
  if (jobs.size > 100) jobs.delete(jobs.keys().next().value);
  return job;
}

function runJob(job, fn) {
  fn(job).then(
    r => { job.status = 'done'; job.progress = 1; job.result = r; },
    e => { job.status = 'error'; job.error = e.message || String(e); job.failedUsb = !!e.failedUsb; },
  );
  return job;
}

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
  const type = TYPES[path.extname(file)] || 'application/octet-stream';
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

async function handle(req, res) {
  const url = new URL(req.url, 'http://x');
  const p = decodeURIComponent(url.pathname);
  const m = req.method;
  let mm, mm0;

  // Basic DNS-rebinding / cross-site protection: only accept our own host and same-origin writes.
  const host = req.headers.host || '';
  if (!/^(127\.0\.0\.1|localhost)(:\d+)?$/.test(host)) return json(res, 403, { error: 'Forbidden host' });
  if (m !== 'GET' && req.headers.origin && new URL(req.headers.origin).host !== host) return json(res, 403, { error: 'Forbidden origin' });

  if (p.startsWith('/api/')) {
    if (p === '/api/settings' && m === 'GET') return json(res, 200, settings.load());
    if (p === '/api/settings' && m === 'PUT') return json(res, 200, settings.save({ ...(await readBody(req)), slots: settings.load().slots }));
    if (p === '/api/library' && m === 'GET') return json(res, 200, lib.list());

    if (p === '/api/import' && m === 'POST') {
      const name = url.searchParams.get('name') || 'upload';
      const tmp = path.join(paths.base, 'incoming-' + crypto.randomBytes(4).toString('hex') + path.extname(name).replace(/[^.a-zA-Z0-9]/g, ''));
      fs.mkdirSync(paths.base, { recursive: true });
      await new Promise((resolve, reject) => {
        const ws = fs.createWriteStream(tmp);
        req.pipe(ws); ws.on('finish', resolve); ws.on('error', reject); req.on('error', reject);
      });
      let id;
      try { id = lib.addSource(tmp, null, path.basename(name)); }
      catch (e) { return json(res, 400, { error: e.message }); }
      finally { fs.rmSync(tmp, { force: true }); }
      const info = await probe(lib.originalPath(id));
      if (!info.hasVideo) { lib.remove(id); return json(res, 400, { error: "That file has no video or image stream ffmpeg can read." }); }
      return json(res, 200, { id, item: lib.patchMeta(id, { sourceDuration: info.duration, isImage: info.isImage, srcWidth: info.width, srcHeight: info.height }) });
    }
    if (p === "/api/slots" && m === "GET") return json(res, 200, slots.budgets());
    if ((mm0 = /^\/api\/slots\/(startup|standby)\/clear$/.exec(p)) && m === "POST") {
      if (deviceBusy) return json(res, 409, { error: "The screen is busy with another operation." });
      deviceBusy = true;
      const job = runJob(newJob("clear"), async j => {
        try { return await send.clearSlot(mm0[1], (f, msg) => { j.progress = f; j.message = msg; }); } finally { deviceBusy = false; }
      });
      return json(res, 200, { jobId: job.id });
    }

    if ((mm = /^\/api\/jobs\/([a-f0-9]+)$/.exec(p)) && m === 'GET') {
      const j = jobs.get(mm[1]);
      return j ? json(res, 200, j) : json(res, 404, { error: 'No such job' });
    }
    if ((mm = /^\/api\/library\/([a-z0-9]+)$/.exec(p))) {
      if (!lib.get(mm[1])) return json(res, 404, { error: 'Unknown item' });
      if (m === 'PATCH') return json(res, 200, lib.rename(mm[1], (await readBody(req)).name));
      if (m === 'DELETE') { lib.remove(mm[1]); return json(res, 200, { ok: true }); }
    }
    if ((mm = /^\/api\/library\/([a-z0-9]+)\/reconvert$/.exec(p)) && m === 'POST') {
      if (!lib.get(mm[1])) return json(res, 404, { error: 'Unknown item' });
      const o = await readBody(req);
      const job = runJob(newJob('reconvert'), j => lib.convertEntry(mm[1], o, (f, msg) => { j.progress = f; j.message = msg; }));
      return json(res, 200, { jobId: job.id });
    }
    if ((mm = /^\/api\/library\/([a-z0-9]+)\/estimate$/.exec(p)) && m === 'POST') {
      if (!lib.get(mm[1])) return json(res, 404, { error: 'Unknown item' });
      try { return json(res, 200, await lib.estimateEntry(mm[1], await readBody(req))); }
      catch (e) { return json(res, 400, { error: e.message }); }
    }
    if ((mm = /^\/api\/library\/([a-z0-9]+)\/send$/.exec(p)) && m === 'POST') {
      if (!lib.get(mm[1])) return json(res, 404, { error: 'Unknown item' });
      if (deviceBusy) return json(res, 409, { error: 'The screen is busy with another operation.' });
      const slot = (await readBody(req)).slot;
      deviceBusy = true;
      const job = runJob(newJob('send'), async j => {
        try {
          const r = await send.sendItem(mm[1], slot, (f, msg) => { j.progress = f; j.message = msg; });
          return { ...r, note: 'Upload finished. The screen may restart by itself and re-appear on USB for a few seconds.' };
        } finally { deviceBusy = false; }
      });
      return json(res, 200, { jobId: job.id });
    }
    if (p === '/api/transport' && m === 'GET') return json(res, 200, await deviceOps.state());
    if ((p === '/api/setup' || p === '/api/restore' || p === '/api/reconnect') && m === 'POST') {
      if (deviceBusy) return json(res, 409, { error: 'The screen is busy with another operation.' });
      deviceBusy = true;
      const fn = p === '/api/setup' ? setup.setupWinUsb : p === '/api/reconnect' ? setup.reconnect : setup.restoreSerial;
      const job = runJob(newJob('setup'), async j => {
        try { return await fn((f, msg) => { j.progress = f; j.message = msg; }); } finally { deviceBusy = false; }
      });
      return json(res, 200, { jobId: job.id });
    }
    if (p === '/api/device' && m === 'GET') {
      if (deviceBusy) return json(res, 409, { error: 'The screen is busy with another operation.' });
      deviceBusy = true;
      try { return json(res, 200, await deviceOps.checkScreen()); }
      catch (e) { return json(res, 502, { error: e.message, failedUsb: !!e.failedUsb }); }
      finally { deviceBusy = false; }
    }
    return json(res, 404, { error: 'Not found' });
  }

  if (m !== 'GET') return json(res, 405, { error: 'Method not allowed' });

  const md = /^\/media\/([a-z0-9]+)\/([a-z0-9.]+)$/.exec(p);
  if (md) {
    const f = lib.mediaPath(md[1], md[2]);
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

function start({ port = Number(process.env.PORT) || 8765, open = true } = {}) {
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
module.exports = { start };
