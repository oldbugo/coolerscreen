// Example CoolerScreen plug-in source: lists the video files in one folder (no network, nothing written).
// Copy to Documents\CoolerScreen\Sources\, change FOLDER, then enable it in Settings > Sources. See docs/sources.md.
const fs = require('fs');
const path = require('path');

const FOLDER = 'C:\\Users\\Public\\Videos';
const VIDEO = new Set(['.mp4', '.webm', '.mov', '.mkv', '.gif']);
const key = f => 'folder-' + Buffer.from(f).toString('hex').slice(0, 16);

function files() {
  try { return fs.readdirSync(FOLDER).filter(f => VIDEO.has(path.extname(f).toLowerCase())); } catch { return []; }
}

module.exports = {
  name: 'Videos folder (example)',
  description: `Video files in ${FOLDER}.`,
  async detect() { return fs.existsSync(FOLDER) ? { available: true, status: `${files().length} videos` } : { available: false, status: 'Folder not found' }; },
  async list(ctx, { page = 1 }) {
    const all = files();
    const items = all.slice((page - 1) * 30, page * 30).map(f => ({ id: f, title: path.basename(f, path.extname(f)), action: 'add', key: key(f) }));
    return { items, total: all.length, hasMore: page * 30 < all.length };
  },
  async add(ctx, itemId) {
    const f = path.join(FOLDER, path.basename(itemId));
    if (!fs.existsSync(f)) throw new Error('The file is gone.');
    return { file: f, name: `${path.basename(f, path.extname(f))} [${key(itemId)}]${path.extname(f)}`, temp: false };
  },
};
