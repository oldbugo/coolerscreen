// Sources: places clips can come from besides your own files ("My clips" = the Gallery folder). Each source is a provider:
//   { id, name, description, kind: 'online'|'local'|'action'|'plugin', options?: [{ key, label, type }],
//     detect(ctx) -> { available, status },
//     list(ctx, { page, refresh }) -> { items: [{ id, title, thumb, type, action: 'add'|'record'|'none', note, key, badge }],
//                                       total, hasMore },
//     thumbPath?(ctx, itemId) -> local file for "thumb:<itemId>" thumbnails,
//     add(ctx, itemId, { onProgress(fraction, message), seconds }) -> { file, name, temp } }
// ctx = { options (this source's settings), tmpDir }. The file returned by add() is imported into the Gallery folder
// under `name`; items whose `key` appears in a Gallery file name show as "In your gallery".
// Every source except My clips is off until enabled in Settings > Sources. Plug-ins are .js files in
// Documents\CoolerScreen\Sources implementing the same interface (docs/sources.md); they are not even loaded until enabled.
const fs = require('fs');
const path = require('path');
const paths = require('../paths');
const settings = require('../settings');

const BUILTIN = [require('./lovingcool'), require('./otherbrands'), require('./wallpaperengine'), require('./lively'), require('./link')];
const loaded = new Map(); // plugin id -> { mtime, provider } | { error }

function pluginsDir() {
  if (process.env.COOLERSCREEN_SOURCES_DIR) return process.env.COOLERSCREEN_SOURCES_DIR;
  const { documentsDir } = require('../gallery');
  return path.join(documentsDir(), 'CoolerScreen', 'Sources');
}
function pluginFiles() {
  try { return fs.readdirSync(pluginsDir()).filter(f => /\.js$/i.test(f)).map(f => path.join(pluginsDir(), f)); } catch { return []; }
}
const pluginId = file => 'plugin-' + path.basename(file, '.js').toLowerCase().replace(/[^a-z0-9_-]/g, '-').slice(0, 40);

function loadPlugin(file) {
  const id = pluginId(file);
  const mtime = fs.statSync(file).mtimeMs;
  const hit = loaded.get(id);
  if (hit && hit.mtime === mtime) return hit;
  let entry;
  try {
    delete require.cache[require.resolve(file)];
    const p = require(file);
    if (!p || typeof p.list !== 'function' || typeof p.add !== 'function') throw new Error('It does not export list() and add().');
    entry = { mtime, provider: { kind: 'plugin', description: '', ...p, id, name: String(p.name || path.basename(file, '.js')).slice(0, 60), detect: p.detect || (async () => ({ available: true, status: 'Plug-in loaded' })) } };
  } catch (e) { entry = { mtime, error: e.message }; }
  loaded.set(id, entry);
  return entry;
}

const config = () => settings.load().sources;
const isEnabled = id => !!config().enabled[id];

/** All sources for Settings (plug-ins are listed by file name and only loaded when enabled). */
async function describeAll() {
  const c = config();
  const out = [];
  for (const p of BUILTIN) {
    const ctx = contextFor(p.id);
    let det;
    try { det = await p.detect(ctx); } catch (e) { det = { available: false, status: e.message }; }
    out.push({ id: p.id, name: p.name, description: p.description, kind: p.kind, enabled: !!c.enabled[p.id], options: p.options || [],
      values: c.options[p.id] || {}, ...det });
  }
  for (const f of pluginFiles()) {
    const id = pluginId(f);
    const row = { id, name: path.basename(f), description: `Plug-in file: ${f}`, kind: 'plugin', enabled: !!c.enabled[id], options: [], values: {},
      warning: 'Plug-ins are programs: they run with the same rights as CoolerScreen on your PC. Only enable plug-ins you trust.' };
    if (row.enabled) {
      const e = loadPlugin(f);
      if (e.error) Object.assign(row, { available: false, status: 'Could not load: ' + e.error });
      else {
        row.name = e.provider.name; row.description = e.provider.description || row.description;
        try { Object.assign(row, await e.provider.detect(contextFor(id))); } catch (err) { Object.assign(row, { available: false, status: err.message }); }
      }
    } else Object.assign(row, { available: true, status: 'Not loaded (disabled)' });
    out.push(row);
  }
  return out;
}

/** The provider for an enabled source (null if disabled or unknown). */
function provider(id) {
  if (!isEnabled(id)) return null;
  const b = BUILTIN.find(p => p.id === id);
  if (b) return b;
  const f = pluginFiles().find(x => pluginId(x) === id);
  if (!f) return null;
  const e = loadPlugin(f);
  return e.provider || null;
}

function contextFor(id) {
  return { options: config().options[id] || {}, tmpDir: path.join(paths.base, 'downloads') };
}

function setSource(id, { enabled, options }) {
  const s = settings.load();
  const known = BUILTIN.some(p => p.id === id) || pluginFiles().some(f => pluginId(f) === id);
  if (!known) throw new Error('Unknown source');
  if (enabled !== undefined) s.sources.enabled[id] = !!enabled;
  if (options !== undefined) s.sources.options[id] = options;
  if (!s.sources.enabled[id]) s.sources.ticked = s.sources.ticked.filter(x => x !== id);
  else if (enabled && !s.sources.ticked.includes(id)) s.sources.ticked.push(id); // a newly enabled source starts ticked
  return settings.save(s).sources;
}

function setTicked(ticked) {
  const s = settings.load();
  s.sources.ticked = (ticked || []).filter(id => id === 'mine' || s.sources.enabled[id]);
  return settings.save(s).sources;
}

module.exports = { describeAll, provider, contextFor, setSource, setTicked, isEnabled, pluginsDir, BUILTIN };
