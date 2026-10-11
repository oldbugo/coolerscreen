// "Lovingcool themes": the vendor's public online theme catalogue (the same list the LOVINGCOOL MONITOR app shows).
// Found in the vendor app (main/routes/theme.js, util/common.js buildUrl, constant/appConstant.js), confirmed 2026-10-11:
//   GET https://www.htssl.com/api/ht/theme/getAllThemes
//       ?current=<page, 1-based>&pageSize=<n>&fullScreen=true&deviceWidth=2400&deviceHeight=1080&rotate=0
//       &visibleBrandIds[]=7            (7 = the LOVINGCOOL brand; no key, signature or login)
//   -> { code: 200, data: { total, list: [{ themeId, name, deviceWidth, deviceHeight, themeType, createdAt,
//        packageFile: { url, fileName }, coverPictureFile: { url } }] } }
//   Theme packages (.ht) and covers are plain HTTPS files on theme.htssl.com.
// Nothing is downloaded unless the user asks; the list is cached for a day.
const https = require('https');
const fs = require('fs');
const path = require('path');
const paths = require('./paths');
const pkg = require('../package.json');

const API = 'https://www.htssl.com/api/ht/theme/getAllThemes';
const FILE_HOST = 'theme.htssl.com';
const BRAND = 7;
const SCREEN = { width: 2400, height: 1080 };
const PAGE_SIZE = 24;
const CACHE_MS = 24 * 3600 * 1000;
const UA = `CoolerScreen/${pkg.version} (+github.com/oldbugo/coolerscreen)`;
const cacheFile = () => path.join(paths.base, 'themes-cache.json');

function getJson(url) {
  return new Promise((resolve, reject) => {
    const req = https.get(url, { headers: { 'User-Agent': UA, Accept: 'application/json' }, timeout: 20000 }, res => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', d => { body += d; if (body.length > 5e6) req.destroy(new Error('Response too large')); });
      res.on('end', () => {
        if (res.statusCode !== 200) return reject(new Error(`The theme library answered ${res.statusCode}.`));
        try { resolve(JSON.parse(body)); } catch { reject(new Error('The theme library sent something unexpected.')); }
      });
    });
    req.on('timeout', () => req.destroy(new Error('The theme library did not answer (timeout).')));
    req.on('error', e => reject(new Error('Could not reach the theme library: ' + e.message)));
  });
}

function readCache() { try { return JSON.parse(fs.readFileSync(cacheFile(), 'utf8')); } catch { return { pages: {} }; } }
function writeCache(c) { fs.mkdirSync(paths.base, { recursive: true }); fs.writeFileSync(cacheFile(), JSON.stringify(c)); }

const clean = t => ({
  themeId: String(t.themeId), name: String(t.name || t.themeId), width: t.deviceWidth, height: t.deviceHeight,
  created: t.createdAt || null, cover: t.coverPictureFile && t.coverPictureFile.url,
  file: t.packageFile && { url: t.packageFile.url, name: t.packageFile.fileName },
});

/** One page of themes for this screen ({ total, page, pageSize, themes }), from the cache unless refresh. */
async function list(page = 1, { refresh = false } = {}) {
  page = Math.max(1, Math.floor(Number(page) || 1));
  let cache = readCache();
  if (refresh) cache = { pages: {} };
  const hit = cache.pages[page];
  if (hit && Date.now() - hit.at < CACHE_MS) return { ...hit.data, cached: true };
  const u = new URL(API);
  const q = { current: page, pageSize: PAGE_SIZE, fullScreen: 'true', deviceWidth: SCREEN.width, deviceHeight: SCREEN.height, rotate: 0 };
  for (const [k, v] of Object.entries(q)) u.searchParams.set(k, String(v));
  u.searchParams.append('visibleBrandIds[]', String(BRAND));
  const r = await getJson(u.toString());
  if (!r || r.code !== 200 || !r.data || !Array.isArray(r.data.list)) throw new Error('The theme library sent something unexpected.');
  const data = {
    total: Number(r.data.total) || 0, page, pageSize: PAGE_SIZE,
    themes: r.data.list.map(clean).filter(t => t.file && t.file.url && (!t.width || (t.width === SCREEN.width && t.height === SCREEN.height))),
  };
  cache.pages[page] = { at: Date.now(), data };
  writeCache(cache);
  return data;
}

function findCached(themeId) {
  for (const p of Object.values(readCache().pages)) {
    const t = p.data.themes.find(x => x.themeId === themeId);
    if (t) return t;
  }
  return null;
}

/** Download a theme's .ht file into `dir` (only from the vendor's theme file host). onProgress(fraction). Returns the path. */
function download(theme, dir, onProgress = () => {}) {
  const u = new URL(theme.file.url);
  if (u.protocol !== 'https:' || u.hostname !== FILE_HOST) return Promise.reject(new Error('Unexpected download address for this theme.'));
  fs.mkdirSync(dir, { recursive: true });
  const target = path.join(dir, `${theme.themeId.replace(/[^A-Za-z0-9_-]/g, '_')}.ht`);
  const tmp = target + '.part';
  return new Promise((resolve, reject) => {
    const req = https.get(u, { headers: { 'User-Agent': UA }, timeout: 30000 }, res => {
      if (res.statusCode !== 200) { res.resume(); return reject(new Error(`Download failed (${res.statusCode}).`)); }
      const total = Number(res.headers['content-length']) || 0;
      let got = 0;
      const out = fs.createWriteStream(tmp);
      res.on('data', d => { got += d.length; if (total) onProgress(got / total); if (got > 300e6) req.destroy(new Error('Theme file too large.')); });
      res.pipe(out);
      out.on('finish', () => out.close(() => {
        try { fs.renameSync(tmp, target); resolve(target); } catch (e) { reject(e); }
      }));
      out.on('error', e => { fs.rmSync(tmp, { force: true }); reject(e); });
    });
    req.on('timeout', () => req.destroy(new Error('The download stalled (timeout).')));
    req.on('error', e => { fs.rmSync(tmp, { force: true }); reject(new Error('Download failed: ' + e.message)); });
  });
}

module.exports = { list, findCached, download, SCREEN, BRAND, API };
