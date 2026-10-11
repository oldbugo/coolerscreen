// Source: "Add from link": download one file from a direct http(s) link (a video, GIF, image, .ht or .zip). It does not read
// web pages or look for media inside them: the link must point at the file itself.
const http = require('http');
const https = require('https');
const fs = require('fs');
const path = require('path');
const { UA } = require('../themes');

const MAX_BYTES = 500e6;
const EXT = new Set(['.mp4', '.mov', '.mkv', '.webm', '.avi', '.m4v', '.gif', '.jpg', '.jpeg', '.png', '.bmp', '.webp', '.tif', '.tiff', '.ht', '.zip']);
const TYPE_EXT = { 'video/mp4': '.mp4', 'video/webm': '.webm', 'video/quicktime': '.mov', 'video/x-matroska': '.mkv', 'image/gif': '.gif',
  'image/jpeg': '.jpg', 'image/png': '.png', 'image/webp': '.webp', 'image/bmp': '.bmp', 'application/zip': '.zip' };

function nameFrom(u, res) {
  const cd = res.headers['content-disposition'] || '';
  const m = /filename\*=UTF-8''([^;]+)|filename="?([^";]+)"?/i.exec(cd);
  let n = m ? decodeURIComponent(m[1] || m[2]) : decodeURIComponent(path.basename(u.pathname) || '');
  n = n.replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').slice(0, 120);
  const type = String(res.headers['content-type'] || '').split(';')[0].trim().toLowerCase();
  if (!EXT.has(path.extname(n).toLowerCase())) {
    if (!TYPE_EXT[type]) return null;
    n = (n.replace(/\.[^.]*$/, '') || 'download') + TYPE_EXT[type];
  }
  return n;
}

function get(urlStr, dir, onProgress, hops = 0) {
  return new Promise((resolve, reject) => {
    let u;
    try { u = new URL(urlStr); } catch { return reject(new Error('That is not a valid link.')); }
    if (u.protocol !== 'https:' && u.protocol !== 'http:') return reject(new Error('Only http and https links are supported.'));
    const req = (u.protocol === 'https:' ? https : http).get(u, { headers: { 'User-Agent': UA }, timeout: 30000 }, res => {
      if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location) {
        res.resume();
        if (hops >= 5) return reject(new Error('Too many redirects.'));
        return resolve(get(new URL(res.headers.location, u).toString(), dir, onProgress, hops + 1));
      }
      if (res.statusCode !== 200) { res.resume(); return reject(new Error(`The server answered ${res.statusCode}.`)); }
      const type = String(res.headers['content-type'] || '').toLowerCase();
      const name = nameFrom(u, res);
      if (/^text\/html/.test(type) || !name) { res.resume(); return reject(new Error('That link opens a web page, not a video, GIF, image or theme file. Use a direct link to the file.')); }
      const total = Number(res.headers['content-length']) || 0;
      if (total > MAX_BYTES) { res.resume(); return reject(new Error('The file is larger than 500 MB.')); }
      fs.mkdirSync(dir, { recursive: true });
      const file = path.join(dir, `link-${Date.now()}${path.extname(name)}`);
      const out = fs.createWriteStream(file);
      let got = 0;
      res.on('data', d => { got += d.length; if (total) onProgress(got / total); if (got > MAX_BYTES) req.destroy(new Error('The file is larger than 500 MB.')); });
      res.pipe(out);
      out.on('finish', () => out.close(() => resolve({ file, name })));
      out.on('error', e => { fs.rmSync(file, { force: true }); reject(e); });
      req.on('error', () => fs.rmSync(file, { force: true }));
    });
    req.on('timeout', () => req.destroy(new Error('The download stalled (timeout).')));
    req.on('error', e => reject(new Error('Download failed: ' + e.message)));
  });
}

module.exports = {
  id: 'link',
  name: 'Add from link',
  description: 'Adds a button to Browse for pasting a direct link to a video, GIF, image or theme file (up to 500 MB). Web pages are not supported.',
  kind: 'action',
  async detect() { return { available: true, status: 'Ready' }; },
  async download(ctx, url, onProgress) { const r = await get(url, ctx.tmpDir, onProgress); return { ...r, temp: true }; },
};
