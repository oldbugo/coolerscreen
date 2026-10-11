// The Gallery folder (default Documents\CoolerScreen\Gallery): every video/GIF/image/.ht/.zip placed there is an
// original. The app keeps a permanent id per original (src/library.js) and matches files by path; a file renamed in
// Explorer is recognised by its unchanged size and modification time. A file that disappears is marked "missing"
// (its versions stay); the UI offers to delete originals missing for more than 30 days and never deletes them itself.
const fs = require('fs');
const os = require('os');
const path = require('path');
const { execFileSync, execFile } = require('child_process');
const settings = require('./settings');
const paths = require('./paths');
const lib = require('./library');
const gate = require('./gate');
const { probe, makeSourceAssets } = require('./convert');
const { isThemeFile, extractTheme } = require('./htTheme');

const MEDIA_EXT = new Set(['.mp4', '.mov', '.mkv', '.webm', '.avi', '.m4v', '.gif', '.jpg', '.jpeg', '.png', '.bmp', '.webp', '.tif', '.tiff', '.ht', '.zip']);
const STABLE_MS = 1500;
const MISSING_NOTICE_DAYS = 30;

let docsDir = null;
function documentsDir() {
  if (docsDir) return docsDir;
  if (process.platform === 'win32') {
    try {
      const out = execFileSync('powershell', ['-NoProfile', '-Command', '[Environment]::GetFolderPath("MyDocuments")'],
        { timeout: 8000, windowsHide: true }).toString().trim();
      if (out) return (docsDir = out);
    } catch {}
  }
  return (docsDir = path.join(os.homedir(), 'Documents'));
}

function galleryDir() {
  const env = process.env.COOLERSCREEN_GALLERY;
  if (env) return env;
  const s = settings.load().galleryDir;
  return s || path.join(documentsDir(), 'CoolerScreen', 'Gallery');
}

const isMedia = f => MEDIA_EXT.has(path.extname(f).toLowerCase());
const samePath = (a, b) => !!a && !!b && path.resolve(a).toLowerCase() === path.resolve(b).toLowerCase();
const fileKey = (f, st) => `${path.resolve(f).toLowerCase()}|${st.size}|${Math.round(st.mtimeMs)}`;

function sanitizeName(n) {
  n = String(n || '').replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/[. ]+$/, '').trim().slice(0, 100);
  return n || 'clip';
}

function uniquePath(dir, base, ext) {
  let p = path.join(dir, base + ext), i = 2;
  while (fs.existsSync(p)) p = path.join(dir, `${base} (${i++})${ext}`);
  return p;
}

const bad = new Map(); // fileKey -> reason, for files ffmpeg cannot read (retried when the file changes)
let chain = Promise.resolve();
let timer = null, watcher = null, watchedDir = null;
let assetsEnabled = false;
const assetsTried = new Set();

function isStable(file, st) {
  if (Date.now() - st.mtimeMs < STABLE_MS) return false;
  try { fs.closeSync(fs.openSync(file, 'r')); return true; } catch { return false; }
}

/** Read a gallery file into an original (new id, or `existing` to refresh one whose file changed). Throws if unreadable. */
async function registerFile(file, existing = null) {
  const st = fs.statSync(file);
  const id = existing ? existing.id : lib.newId();
  const d = lib.oDir(id);
  fs.mkdirSync(d, { recursive: true });
  try {
    const ext = path.extname(file).toLowerCase();
    // files added from a source carry its key in brackets ("Title [we-123]"): keep the key in the file name, not in the clip's name
    let name = path.basename(file, path.extname(file)).replace(/ \[[a-z][a-z0-9]*-[a-z0-9-]+\]$/i, '');
    let extracted = false;
    let orig = file;
    if (ext === '.ht' || ext === '.zip' || isThemeFile(file)) {
      for (const n of fs.readdirSync(d)) if (lib.isSourceName(n)) fs.rmSync(path.join(d, n), { force: true });
      const t = extractTheme(file, d);
      orig = path.join(d, 'original' + path.extname(t.path).toLowerCase());
      fs.renameSync(t.path, orig);
      extracted = true;
      if (ext === '.ht' && t.name) name = t.name;
    }
    const info = await probe(orig);
    if (!info.hasVideo) throw new Error(`CoolerScreen can't read "${path.basename(file)}". It may be damaged or in a format that isn't supported.`);
    for (const n of ['thumb.jpg', 'frame.jpg', 'proxy.mp4', 'proxy.jpg']) fs.rmSync(path.join(d, n), { force: true });
    lib.writeOriginal({
      ...(existing || {}),
      id, name: existing ? existing.name : name, file, fileName: path.basename(file), extracted,
      fileSize: st.size, fileMtime: Math.round(st.mtimeMs), created: existing ? existing.created : new Date().toISOString(),
      duration: info.isImage ? null : info.duration, isImage: info.isImage, width: info.width, height: info.height, codec: info.codec, fps: info.fps,
      assets: 'pending', assetsError: null, proxy: null, missing: null,
      sourceChanged: !!existing && (lib.versions(id).length > 0),
    });
    return id;
  } catch (e) {
    if (!existing) fs.rmSync(d, { recursive: true, force: true });
    throw e;
  }
}

/** Thumbnail, notch frame and browser proxy for originals that don't have them yet (one at a time, in the queue). */
async function makeMissingAssets() {
  if (!assetsEnabled) return;
  for (const o of lib.listOriginals()) {
    // A failed attempt is retried once per app session (the next launch), never in a loop.
    if ((o.assets === 'ready' && o.fps !== undefined) || assetsTried.has(o.id + '|' + o.fileMtime)) continue;
    const src = lib.sourcePath(o);
    if (!src) continue;
    assetsTried.add(o.id + '|' + o.fileMtime);
    await gate.run('convert', async () => {
      const cur = lib.readOriginal(o.id);
      if (!cur) return;
      if (cur.assets === 'ready') { // older entry without a source frame rate: probe only
        const info = await probe(src);
        lib.patchOriginal(o.id, { fps: info.fps || null });
        return;
      }
      try {
        const info = await probe(src);
        const r = await makeSourceAssets(src, lib.oDir(o.id), info);
        lib.patchOriginal(o.id, { assets: 'ready', proxy: r.proxy, assetsError: null, fps: info.fps || null });
      } catch (e) {
        lib.patchOriginal(o.id, { assets: 'error', assetsError: e.message });
      }
    }).catch(() => {});
  }
}

function doSync() {
  const dir = galleryDir();
  fs.mkdirSync(dir, { recursive: true });
  return (async () => {
    let files = [];
    try { files = fs.readdirSync(dir, { withFileTypes: true }).filter(e => e.isFile() && isMedia(e.name) && !e.name.startsWith('~$')).map(e => path.join(dir, e.name)); } catch {}
    let pending = false;
    const originals = lib.listOriginals();
    const claimed = new Set();
    // 1. Known paths: back from missing, or changed content.
    const unknown = [];
    for (const f of files) {
      let st;
      try { st = fs.statSync(f); } catch { continue; }
      const o = originals.find(x => samePath(x.file, f));
      if (!o) { unknown.push([f, st]); continue; }
      claimed.add(o.id);
      if (o.missing) lib.patchOriginal(o.id, { missing: null });
      if (o.fileSize == null || o.fileMtime == null) { // migrated entry without a recorded size/time: just record them
        lib.patchOriginal(o.id, { fileSize: st.size, fileMtime: Math.round(st.mtimeMs) });
        continue;
      }
      if (o.fileSize !== st.size || o.fileMtime !== Math.round(st.mtimeMs)) {
        if (bad.has(fileKey(f, st))) continue;
        if (!isStable(f, st)) { pending = true; continue; }
        try { await registerFile(f, lib.readOriginal(o.id)); } catch (e) { bad.set(fileKey(f, st), e.message); console.log('Gallery: ' + e.message); }
      }
    }
    // 2. Originals whose file is gone: missing (kept), unless the file was renamed (same size and time).
    // (A file outside the current Gallery folder counts as gone too: the folder defines the library.)
    const gone = originals.filter(o => !claimed.has(o.id) && o.file);
    for (const [f, st] of unknown) {
      const twin = gone.find(o => !claimed.has(o.id) && o.fileSize === st.size && o.fileMtime === Math.round(st.mtimeMs));
      if (twin) {
        claimed.add(twin.id);
        const ht = /\.(ht|zip)$/i.test(f);
        lib.patchOriginal(twin.id, { file: f, fileName: path.basename(f), missing: null, ...(ht ? {} : { name: path.basename(f, path.extname(f)) }) });
        continue;
      }
      if (bad.has(fileKey(f, st))) continue;
      if (!isStable(f, st)) { pending = true; continue; }
      try { await registerFile(f); } catch (e) { bad.set(fileKey(f, st), e.message); console.log('Gallery: skipped ' + path.basename(f) + ': ' + e.message); }
    }
    for (const o of gone) {
      if (!claimed.has(o.id) && !o.missing) lib.patchOriginal(o.id, { missing: { since: new Date().toISOString() } });
    }
    if (pending) schedule(2000);
  })();
}

/** Serialised scan of the gallery folder; afterwards makes any missing previews. */
function sync() {
  chain = chain.then(() => gate.run('convert', doSync), () => gate.run('convert', doSync))
    .catch(e => console.log('Gallery sync error: ' + e.message));
  chain.then(() => makeMissingAssets());
  return chain;
}

function schedule(ms = 1000) {
  clearTimeout(timer);
  timer = setTimeout(sync, ms);
}

function watch() {
  const dir = galleryDir();
  if (watcher && watchedDir === dir) return;
  if (watcher) { try { watcher.close(); } catch {} watcher = null; }
  try {
    watcher = fs.watch(dir, { persistent: false }, () => schedule(1000));
    watcher.on('error', () => {});
    watchedDir = dir;
  } catch (e) { console.log('Gallery: cannot watch folder: ' + e.message); }
}

/** Originals from before the Gallery folder existed (their only copy is in the cache): copy them into the folder. */
function adoptCacheOnly() {
  const dir = galleryDir();
  for (const o of lib.listOriginals()) {
    if (o.file) continue;
    try {
      const d = lib.oDir(o.id);
      const f = fs.readdirSync(d).find(n => lib.isSourceName(n));
      if (!f) continue;
      const target = uniquePath(dir, sanitizeName(o.name), path.extname(f).toLowerCase());
      fs.copyFileSync(path.join(d, f), target);
      const st = fs.statSync(target);
      lib.patchOriginal(o.id, { file: target, fileName: path.basename(target), extracted: false, fileSize: st.size, fileMtime: Math.round(st.mtimeMs) });
      fs.rmSync(path.join(d, f), { force: true });
    } catch (e) { console.log('Gallery: could not copy ' + o.name + ' into the gallery folder: ' + e.message); }
  }
}

/** opts.assets: also make thumbnails/previews (the app does; the CLI skips it). */
async function init(opts = {}) {
  if (opts.assets) assetsEnabled = true;
  fs.mkdirSync(paths.library, { recursive: true });
  fs.mkdirSync(galleryDir(), { recursive: true });
  const n = lib.migrate();
  if (n) console.log(`Library: migrated ${n} item(s) to originals + versions.`);
  adoptCacheOnly();
  await sync();
  watch();
}

/** Copy a file into the Gallery (keeping its name, "name (2)" on clashes) and register it. Returns the original id. */
async function addFile(src, displayFileName) {
  const dir = galleryDir();
  fs.mkdirSync(dir, { recursive: true });
  const orig = displayFileName || path.basename(src);
  const ext = path.extname(orig).toLowerCase();
  const base = sanitizeName(path.basename(orig, path.extname(orig)));
  const inside = samePath(path.dirname(src), dir);
  if (inside) {
    const known = lib.listOriginals().find(o => samePath(o.file, src));
    if (known) return known.id;
  }
  const target = inside ? src : uniquePath(dir, base, ext || '.bin');
  if (!inside) fs.copyFileSync(src, target);
  try {
    const id = await gate.run('convert', () => registerFile(target));
    watch();
    makeMissingAssets();
    return id;
  } catch (e) {
    if (!inside) fs.rmSync(target, { force: true });
    throw e;
  }
}

function renameOriginal(id, newName) {
  const o = lib.readOriginal(id);
  if (!o) throw new Error('Unknown clip');
  const label = String(newName || '').trim().slice(0, 80);
  if (!label) throw new Error('Enter a name.');
  if (!o.file || o.missing || !fs.existsSync(o.file)) return lib.patchOriginal(id, { name: label });
  const base = sanitizeName(label);
  const ext = path.extname(o.file);
  const target = path.join(path.dirname(o.file), base + ext);
  if (!samePath(target, o.file) && fs.existsSync(target)) throw new Error('A file with this name already exists in the Gallery folder.');
  if (target !== o.file) fs.renameSync(o.file, target);
  return lib.patchOriginal(id, { name: label, file: target, fileName: path.basename(target) });
}

// Windows: send to the Recycle Bin (recoverable). Elsewhere: delete.
function recycle(file) {
  if (process.platform !== 'win32') { fs.rmSync(file, { force: true }); return Promise.resolve(); }
  return new Promise((resolve, reject) => {
    const cmd = "Add-Type -AssemblyName Microsoft.VisualBasic; [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($env:CS_DEL_FILE, 'OnlyErrorDialogs', 'SendToRecycleBin')";
    execFile('powershell', ['-NoProfile', '-NonInteractive', '-Command', cmd], { windowsHide: true, timeout: 30000, env: { ...process.env, CS_DEL_FILE: file } },
      e => (e || fs.existsSync(file) ? reject(new Error('Could not move the file to the Recycle Bin' + (e ? ': ' + e.message.split('\n')[0] : '.'))) : resolve()));
  });
}

/** Delete an original from the UI: its file goes to the Recycle Bin and all its versions are deleted. */
async function deleteOriginal(id) {
  const o = lib.readOriginal(id);
  if (!o) return;
  // A "missing" original's file is not in the Gallery folder (maybe moved elsewhere on purpose): never touch it.
  if (o.file && !o.missing && fs.existsSync(o.file)) await recycle(o.file);
  lib.removeOriginalCache(id);
}

/** Originals missing from the folder for longer than the notice period. */
function longMissing(now = Date.now()) {
  return lib.listOriginals().filter(o => o.missing && now - Date.parse(o.missing.since) > MISSING_NOTICE_DAYS * 864e5);
}

/** Delete the versions of originals whose file is missing (user-confirmed in the UI). */
function deleteMissing(ids) {
  let n = 0;
  for (const id of ids || []) {
    const o = lib.readOriginal(id);
    if (o && o.missing) { lib.removeOriginalCache(id); n++; }
  }
  return n;
}

function setDir(dir) {
  dir = String(dir || '').trim();
  if (dir) fs.mkdirSync(dir, { recursive: true });
  const s = settings.load();
  settings.save({ ...s, galleryDir: dir });
  return init();
}

module.exports = {
  documentsDir, init, sync, addFile, renameOriginal, deleteOriginal, deleteMissing, longMissing, setDir, galleryDir, sanitizeName, isMedia,
  MISSING_NOTICE_DAYS,
};
