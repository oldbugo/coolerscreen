// The Gallery folder (default Documents\CoolerScreen\Gallery): every video/GIF/image/.ht/.zip placed there is a
// gallery item. Conversions are cached under %APPDATA%\CoolerScreen\library\<id>\ where id = hash(path|size|mtime).
const fs = require('fs');
const os = require('os');
const path = require('path');
const crypto = require('crypto');
const { execFileSync } = require('child_process');
const paths = require('./paths');
const settings = require('./settings');
const lib = require('./library');
const { probe } = require('./convert');
const { isThemeFile, extractTheme } = require('./htTheme');

const MEDIA_EXT = new Set(['.mp4', '.mov', '.mkv', '.webm', '.avi', '.m4v', '.gif', '.jpg', '.jpeg', '.png', '.bmp', '.webp', '.tif', '.tiff', '.ht', '.zip']);
const STABLE_MS = 1500;
const DELETED_DIR = '_Deleted';

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
const idFor = (file, st) =>
  crypto.createHash('sha1').update(`${path.resolve(file).toLowerCase()}|${st.size}|${Math.round(st.mtimeMs)}`).digest('hex').slice(0, 16);

function sanitizeName(n) {
  n = String(n || '').replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/[. ]+$/, '').trim().slice(0, 100);
  return n || 'clip';
}

function uniquePath(dir, base, ext) {
  let p = path.join(dir, base + ext), i = 2;
  while (fs.existsSync(p)) p = path.join(dir, `${base} (${i++})${ext}`);
  return p;
}

const bad = new Set(); // ids of files we could not read as media
let chain = Promise.resolve();
let timer = null, watcher = null, watchedDir = null;

function isStable(file, st) {
  if (Date.now() - st.mtimeMs < STABLE_MS) return false;
  try { fs.closeSync(fs.openSync(file, 'r')); return true; } catch { return false; }
}

/** Create the cache entry for a gallery file. Throws if ffmpeg can't read it. */
async function registerFile(file) {
  const st = fs.statSync(file);
  const id = idFor(file, st);
  if (lib.get(id)) return id;
  const d = lib.dirOf(id);
  fs.mkdirSync(d, { recursive: true });
  try {
    const ext = path.extname(file).toLowerCase();
    let name = path.basename(file, path.extname(file));
    let extracted = false;
    let orig = file;
    if (ext === '.ht' || ext === '.zip' || isThemeFile(file)) {
      const t = extractTheme(file, d);
      orig = path.join(d, 'original' + path.extname(t.path).toLowerCase());
      fs.renameSync(t.path, orig);
      extracted = true;
      if (ext === '.ht' && t.name) name = t.name;
    }
    const info = await probe(orig);
    if (!info.hasVideo) throw new Error('No video or image stream ffmpeg can read in ' + path.basename(file));
    lib.writeMeta(id, {
      id, name, source: path.basename(file), file, extracted, fileSize: st.size, fileMtime: Math.round(st.mtimeMs),
      created: new Date().toISOString(), status: 'new',
      sourceDuration: info.duration, isImage: info.isImage, srcWidth: info.width, srcHeight: info.height,
    });
    return id;
  } catch (e) {
    fs.rmSync(d, { recursive: true, force: true });
    throw e;
  }
}

function doSync() {
  const dir = galleryDir();
  fs.mkdirSync(dir, { recursive: true });
  return (async () => {
    let files = [];
    try { files = fs.readdirSync(dir, { withFileTypes: true }).filter(e => e.isFile() && isMedia(e.name) && !e.name.startsWith('~$')).map(e => path.join(dir, e.name)); } catch {}
    const present = new Set();
    let pending = false;
    for (const f of files) {
      let st;
      try { st = fs.statSync(f); } catch { continue; }
      const id = idFor(f, st);
      present.add(id);
      if (lib.get(id) || bad.has(id)) continue;
      if (!isStable(f, st)) { pending = true; continue; }
      try { await registerFile(f); } catch (e) { bad.add(id); console.log('Gallery: skipped ' + path.basename(f) + ': ' + e.message); }
    }
    // Items whose file is gone (or was replaced by a different version) leave the gallery.
    for (const m of lib.list()) if (m.file && !present.has(m.id)) lib.remove(m.id);
    if (pending) schedule(2000);
  })();
}

/** Serialised scan of the gallery folder. */
function sync() {
  chain = chain.then(doSync, doSync).catch(e => console.log('Gallery sync error: ' + e.message));
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

/** One-time move of pre-gallery library items (originals inside the cache) into the Gallery folder. */
function migrate() {
  const dir = galleryDir();
  for (const m of lib.list()) {
    if (m.file) continue;
    try {
      const d = lib.dirOf(m.id);
      const f = fs.readdirSync(d).find(n => n.startsWith('original.'));
      if (!f) { lib.remove(m.id); continue; }
      const ext = path.extname(f).toLowerCase();
      const target = uniquePath(dir, sanitizeName(m.name), ext);
      fs.copyFileSync(path.join(d, f), target);
      rekey(m.id, target, { name: m.name, extracted: false });
      fs.rmSync(path.join(lib.dirOf(idFor(target, fs.statSync(target))), f), { force: true });
    } catch (e) { console.log('Gallery: could not migrate ' + m.name + ': ' + e.message); }
  }
}

/** Point an existing cache entry at a (new) gallery file, moving it to the id that file now has. */
function rekey(oldId, file, patch = {}) {
  const st = fs.statSync(file);
  const newId = idFor(file, st);
  const meta = lib.readMeta(oldId);
  const next = {
    ...meta, ...patch, id: newId, file, source: path.basename(file), fileSize: st.size, fileMtime: Math.round(st.mtimeMs),
  };
  if (newId !== oldId) {
    const nd = lib.dirOf(newId);
    if (fs.existsSync(nd)) fs.rmSync(nd, { recursive: true, force: true });
    fs.renameSync(lib.dirOf(oldId), nd);
  }
  lib.writeMeta(newId, next);
  return newId;
}

async function init() {
  fs.mkdirSync(paths.library, { recursive: true });
  fs.mkdirSync(galleryDir(), { recursive: true });
  migrate();
  await sync();
  watch();
}

/** Copy a file into the Gallery (keeping its name, "name (2)" on clashes) and register it. Returns the item id. */
async function addFile(src, displayFileName) {
  const dir = galleryDir();
  fs.mkdirSync(dir, { recursive: true });
  const orig = displayFileName || path.basename(src);
  const ext = path.extname(orig).toLowerCase();
  if (!isMedia(orig) && !isThemeFile(src)) {
    // Unknown extension: still accept if ffmpeg can read it, keep the given extension.
  }
  const base = sanitizeName(path.basename(orig, path.extname(orig)));
  const inside = path.resolve(path.dirname(src)).toLowerCase() === path.resolve(dir).toLowerCase();
  const target = inside ? src : uniquePath(dir, base, ext || '.bin');
  if (!inside) fs.copyFileSync(src, target);
  try {
    const id = await registerFile(target);
    watch();
    return id;
  } catch (e) {
    if (!inside) fs.rmSync(target, { force: true });
    throw e;
  }
}

function renameItem(id, newName) {
  const m = lib.get(id);
  if (!m || !m.file) throw new Error('Unknown item');
  const base = sanitizeName(newName);
  const ext = path.extname(m.file);
  const target = path.join(path.dirname(m.file), base + ext);
  if (target.toLowerCase() !== m.file.toLowerCase() && fs.existsSync(target)) throw new Error('A file named "' + base + ext + '" already exists in the gallery folder.');
  if (target !== m.file) fs.renameSync(m.file, target);
  const nid = rekey(id, target, { name: String(newName).trim().slice(0, 80) || base });
  return lib.get(nid);
}

/** Remove an item: the file moves to Gallery\_Deleted (never permanently deleted by the app). */
function removeItem(id) {
  const m = lib.get(id);
  if (!m) return;
  if (m.file && fs.existsSync(m.file)) {
    const trash = path.join(path.dirname(m.file), DELETED_DIR);
    fs.mkdirSync(trash, { recursive: true });
    const ext = path.extname(m.file);
    fs.renameSync(m.file, uniquePath(trash, path.basename(m.file, ext), ext));
  }
  lib.remove(id);
}

function setDir(dir) {
  dir = String(dir || '').trim();
  if (dir) fs.mkdirSync(dir, { recursive: true });
  const s = settings.load();
  settings.save({ ...s, galleryDir: dir });
  return init();
}

module.exports = { init, sync, addFile, renameItem, removeItem, setDir, galleryDir, sanitizeName, isMedia };
