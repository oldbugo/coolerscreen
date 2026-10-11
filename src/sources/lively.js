// Source: Lively Wallpaper (local). Reads LivelyInfo.json in Lively's library, read-only. Video, GIF and picture wallpapers
// are copied into the Gallery folder; web/app wallpapers aren't supported (they can't be played in a window on request).
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const PAGE = 36;
// Lively's WallpaperType numbers (from its source): 7 video, 8 gif, 10 video stream, 11 picture; others are web/app types.
const TYPES = { 0: 'app', 1: 'web', 2: 'web audio', 3: 'url', 4: 'bizhawk', 5: 'unity', 6: 'godot', 7: 'video', 8: 'gif', 9: 'unity audio', 10: 'video stream', 11: 'picture' };
const IMPORTABLE = new Set([7, 8, 11]);

function libraries(opts = {}) {
  const local = process.env.LOCALAPPDATA || '';
  const dirs = [];
  if (process.env.COOLERSCREEN_LIVELY_DIR) dirs.push(process.env.COOLERSCREEN_LIVELY_DIR); // tests / unusual installs
  dirs.push(path.join(local, 'Lively Wallpaper', 'Library', 'wallpapers'));
  try {
    for (const p of fs.readdirSync(path.join(local, 'Packages'))) {
      if (/lively/i.test(p)) dirs.push(path.join(local, 'Packages', p, 'LocalCache', 'Local', 'Lively Wallpaper', 'Library', 'wallpapers'));
    }
  } catch {}
  for (const d of (opts.folders || [])) dirs.push(d);
  return [...new Set(dirs)].filter(d => fs.existsSync(d));
}

function scan(opts) {
  const items = [];
  for (const lib of libraries(opts)) {
    let subs = [];
    try { subs = fs.readdirSync(lib); } catch { continue; }
    for (const s of subs) {
      const dir = path.join(lib, s);
      let j;
      try { j = JSON.parse(fs.readFileSync(path.join(dir, 'LivelyInfo.json'), 'utf8').replace(/^﻿/, '')); } catch { continue; }
      const t = Number(j.Type);
      const fileName = j.FileName ? (path.isAbsolute(j.FileName) ? j.FileName : path.join(dir, j.FileName)) : null;
      const thumbName = j.Thumbnail || j.Preview;
      const thumb = thumbName ? (path.isAbsolute(thumbName) ? thumbName : path.join(dir, thumbName)) : null;
      const key = 'lively-' + crypto.createHash('sha1').update(dir).digest('hex').slice(0, 10);
      const ok = IMPORTABLE.has(t);
      const exists = fileName && fs.existsSync(fileName);
      items.push({ id: key, key, title: String(j.Title || s).slice(0, 120), type: TYPES[t] || 'other', file: exists ? fileName : null,
        action: ok && exists ? 'add' : 'none', note: ok ? (exists ? '' : 'The file is missing') : `"${TYPES[t] || 'this'}" wallpapers aren't supported`,
        thumb: thumb && fs.existsSync(thumb) ? thumb : null });
    }
  }
  return items.sort((a, b) => (a.action === 'none') - (b.action === 'none') || a.title.localeCompare(b.title));
}

const safeName = s => String(s).replace(/[<>:"/\\|?*\x00-\x1f]/g, '_').replace(/[. ]+$/, '').trim().slice(0, 80) || 'wallpaper';

module.exports = {
  id: 'lively',
  name: 'Lively Wallpaper',
  description: 'Video, GIF and picture wallpapers from your Lively Wallpaper library.',
  kind: 'local',
  options: [{ key: 'folders', label: 'Extra library folders (one per line)', type: 'folders' }],
  async detect(ctx) {
    if (!libraries(ctx.options).length) return { available: false, status: 'Not installed (no Lively Wallpaper library found)' };
    const items = scan(ctx.options);
    return { available: true, status: `Lively Wallpaper found: ${items.length} items (${items.filter(i => i.action === 'add').length} can be added)` };
  },
  async list(ctx, { page = 1 } = {}) {
    const all = scan(ctx.options);
    return { items: all.slice((page - 1) * PAGE, page * PAGE).map(i => ({ id: i.id, title: i.title, type: i.type, action: i.action, note: i.note, key: i.key, thumb: i.thumb ? `thumb:${i.id}` : null })),
      total: all.length, hasMore: page * PAGE < all.length };
  },
  thumbPath(ctx, itemId) { const i = scan(ctx.options).find(x => x.id === itemId); return i && i.thumb; },
  async add(ctx, itemId, { onProgress }) {
    const i = scan(ctx.options).find(x => x.id === itemId);
    if (!i || !i.file) throw new Error(i ? i.note || 'Not supported' : 'This wallpaper is no longer there.');
    onProgress(0.5);
    return { file: i.file, name: `${safeName(i.title)} [${i.key}]${path.extname(i.file).toLowerCase()}`, temp: false };
  },
};
