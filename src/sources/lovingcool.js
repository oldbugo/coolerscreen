// Source: Lovingcool's online theme library (see src/themes.js for the API).
const themes = require('../themes');

const toItem = t => ({
  id: t.themeId, title: t.name, thumb: t.cover || null, type: 'theme', action: 'add', key: t.themeId,
});

module.exports = {
  id: 'lovingcool',
  name: 'Lovingcool themes',
  description: 'Lovingcool\'s free online theme library for this screen (the themes the vendor app offers).',
  kind: 'online',
  async detect() { return { available: true, status: 'Online library (loaded when you browse it)' }; },
  async list(ctx, { page = 1, refresh = false } = {}) {
    const r = await themes.list(page, { refresh });
    const pages = Math.ceil(r.total / r.pageSize);
    return { items: r.themes.map(toItem), total: r.total, hasMore: page < pages };
  },
  async add(ctx, itemId, { onProgress }) {
    let t = themes.findCached(itemId);
    for (let p = 1; !t && p <= 10; p++) { const r = await themes.list(p); t = r.themes.find(x => x.themeId === itemId); if (!r.themes.length) break; }
    if (!t) throw new Error('Unknown theme. Refresh the list and try again.');
    const file = await themes.download(t, ctx.tmpDir, onProgress);
    return { file, name: t.file.name || `${t.themeId}.ht`, temp: true };
  },
};
