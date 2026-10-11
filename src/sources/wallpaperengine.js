// Source: Wallpaper Engine (local). Lists the user's Steam Workshop subscriptions (431960) and Wallpaper Engine's own
// projects (projects/myprojects) from project.json files, read-only.
//  - video wallpapers: "Add to gallery" copies the video file into the Gallery folder.
//  - scene and web wallpapers: "Record" opens the wallpaper in its own 2400x1080 window (Wallpaper Engine's documented
//    command line: -control openWallpaper ... -playInWindow), records it with Windows Graphics Capture, then closes that
//    window (-control closeWallpaper). The desktop wallpaper and Wallpaper Engine's settings (including mute) are not
//    touched. Needs the desktop app and a running Wallpaper Engine.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { execFile } = require('child_process');
const recorder = require('./recorder');
const { runFfmpeg, probe } = require('../convert');

const APP_ID = '431960';
const PAGE = 36;
const sleep = ms => new Promise(r => setTimeout(r, ms));
let recording = Promise.resolve(); // one recording at a time

function steamRoots() {
  const roots = new Set();
  const pf86 = process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)';
  for (const base of [path.join(pf86, 'Steam'), path.join(process.env.ProgramFiles || 'C:\\Program Files', 'Steam')]) {
    const vdf = path.join(base, 'steamapps', 'libraryfolders.vdf');
    if (!fs.existsSync(vdf)) continue;
    roots.add(base);
    // "path"   "D:\\SteamLibrary"
    for (const m of fs.readFileSync(vdf, 'utf8').matchAll(/"path"\s+"([^"]+)"/g)) roots.add(m[1].replace(/\\\\/g, '\\'));
  }
  return [...roots];
}

function folders(opts = {}) {
  const out = [];
  for (const r of steamRoots()) {
    out.push({ dir: path.join(r, 'steamapps', 'workshop', 'content', APP_ID), kind: 'workshop' });
    const we = path.join(r, 'steamapps', 'common', 'wallpaper_engine', 'projects', 'myprojects');
    out.push({ dir: we, kind: 'myprojects' });
  }
  for (const d of (opts.folders || [])) out.push({ dir: d, kind: 'extra' });
  return out.filter(f => fs.existsSync(f.dir));
}

let cache = null; // { at, items }
function scan(opts, refresh) {
  if (cache && !refresh && Date.now() - cache.at < 60000) return cache.items;
  const items = [];
  for (const f of folders(opts)) {
    let ids = [];
    try { ids = fs.readdirSync(f.dir); } catch { continue; }
    for (const id of ids) {
      const dir = path.join(f.dir, id);
      let j;
      try { j = JSON.parse(fs.readFileSync(path.join(dir, 'project.json'), 'utf8')); } catch { continue; }
      const type = String(j.type || '').toLowerCase();
      const file = j.file ? path.join(dir, j.file) : null;
      const preview = j.preview ? path.join(dir, j.preview) : null;
      const key = `we-${f.kind === 'workshop' ? id : crypto.createHash('sha1').update(dir).digest('hex').slice(0, 10)}`;
      let action = 'none', note = '';
      if (type === 'video' && file && fs.existsSync(file)) action = 'add';
      else if ((type === 'scene' || type === 'web') && file) action = 'record';
      else note = type ? `"${type}" wallpapers aren't supported` : 'Not a wallpaper (preset or dependency)';
      items.push({ id: key, title: String(j.title || id).slice(0, 120), type: type || 'other', action, note, key, dir,
        file, project: path.join(dir, 'project.json'), preview: preview && fs.existsSync(preview) ? preview : null });
    }
  }
  items.sort((a, b) => (a.action === 'none') - (b.action === 'none') || a.title.localeCompare(b.title));
  cache = { at: Date.now(), items };
  return items;
}

function weExe() {
  return new Promise(resolve => {
    if (process.platform !== 'win32') return resolve(null);
    execFile('powershell', ['-NoProfile', '-Command', '(Get-Process wallpaper64,wallpaper32 -ErrorAction SilentlyContinue | Select-Object -First 1).Path'],
      { windowsHide: true, timeout: 10000 }, (e, out) => resolve(!e && out.trim() ? out.trim() : null));
  });
}
const run = (exe, args) => new Promise(resolve => execFile(exe, args, { windowsHide: true, timeout: 20000 }, () => resolve()));
const safeName = s => String(s).replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/[. ]+$/, '').trim().slice(0, 80) || 'wallpaper';

module.exports = {
  id: 'wallpaperengine',
  name: 'Wallpaper Engine',
  description: 'Your Wallpaper Engine wallpapers (Steam Workshop and your own projects). Video wallpapers are copied; scene and web wallpapers are recorded from a temporary window (desktop app only).',
  kind: 'local',
  options: [{ key: 'folders', label: 'Extra wallpaper folders (one per line)', type: 'folders' }],
  async detect(ctx) {
    const items = scan(ctx.options, true);
    if (!folders(ctx.options).length) return { available: false, status: 'Not found (no Steam library with Wallpaper Engine content)' };
    const rec = items.filter(i => i.action === 'record').length, vid = items.filter(i => i.action === 'add').length;
    return { available: true, status: `Wallpaper Engine found: ${items.length} items (${vid} video, ${rec} to record)` };
  },
  async list(ctx, { page = 1, refresh = false } = {}) {
    const all = scan(ctx.options, refresh);
    const items = all.slice((page - 1) * PAGE, page * PAGE).map(i => ({ id: i.id, title: i.title, type: i.type, action: i.action, note: i.note,
      key: i.key, thumb: i.preview ? `thumb:${i.id}` : null }));
    return { items, total: all.length, hasMore: page * PAGE < all.length };
  },
  thumbPath(ctx, itemId) { const i = scan(ctx.options).find(x => x.id === itemId); return i && i.preview; },
  async add(ctx, itemId, { onProgress, seconds = 20 }) {
    const i = scan(ctx.options).find(x => x.id === itemId);
    if (!i) throw new Error('This wallpaper is no longer there.');
    const name = `${safeName(i.title)} [${i.key}]`;
    if (i.action === 'add') {
      onProgress(0.5);
      return { file: i.file, name: name + path.extname(i.file).toLowerCase(), temp: false };
    }
    if (i.action !== 'record') throw new Error(i.note || 'This wallpaper type is not supported.');
    if (!recorder.available()) throw new Error('Recording needs the desktop app (CoolerScreen.exe).');
    const turn = recording.then(() => this.record(ctx, i, { onProgress, seconds, name }));
    recording = turn.catch(() => {});
    return turn;
  },
  async record(ctx, i, { onProgress, seconds, name }) {
    const exe = await weExe();
    if (!exe) throw new Error('Start Wallpaper Engine first (it is needed to play the wallpaper while recording).');
    seconds = Math.min(60, Math.max(3, Number(seconds) || 20));
    const title = 'CoolerScreen capture ' + crypto.randomBytes(3).toString('hex');
    const raw = path.join(ctx.tmpDir, `${i.key}-${Date.now()}.webm`);
    const out = path.join(ctx.tmpDir, `${i.key}-${Date.now()}.mp4`);
    fs.mkdirSync(ctx.tmpDir, { recursive: true });
    try {
      onProgress(0.02, 'Opening the wallpaper in its own window');
      await run(exe, ['-control', 'openWallpaper', '-file', i.project, '-playInWindow', title, '-width', '2400', '-height', '1080', '-x', '0', '-y', '0', '-borderless']);
      await sleep(4000); // let it load and start animating
      await recorder.recordWindow(title, { seconds, fps: 18, out: raw, onProgress: f => onProgress(0.05 + 0.8 * f, 'Recording') });
    } finally {
      await run(exe, ['-control', 'closeWallpaper', '-location', title]); // always close the temporary window
    }
    try {
      // The capture can include a title bar on top: keep the bottom part with the screen's 20:9 shape.
      onProgress(0.9, 'Finishing the recording');
      const info = await probe(raw);
      let vf = 'fps=18';
      if (info.width && info.height && info.height > Math.round(info.width * 9 / 20) + 2) {
        const h = 2 * Math.floor(info.width * 9 / 20 / 2);
        vf = `crop=${2 * Math.floor(info.width / 2)}:${h}:0:${info.height - h},fps=18`;
      }
      await runFfmpeg(['-i', raw, '-vf', vf + ',format=yuv420p', '-an', '-c:v', 'libx264', '-preset', 'veryfast', '-crf', '14', '-movflags', '+faststart', '-y', out]);
    } finally { fs.rmSync(raw, { force: true }); }
    return { file: out, name: name + '.mp4', temp: true };
  },
};
