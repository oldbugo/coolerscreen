// Source: themes other brands publish on the same theme platform for the same 2400x1080 screen.
// Brand ids come from the vendor app's configuration (constant/appConstant.js, visibleBrandIds); a 2026-10-11 check found
// 2400x1080 themes only for brand 2 (31 themes; used by the HT DIGITAL, COUGAR Digital and majesty Display apps).
// Which brands have themes is re-checked at most once a day (one tiny request per brand).
const fs = require('fs');
const path = require('path');
const paths = require('../paths');
const themes = require('../themes');

const BRANDS = { 1: 'Brand 1', 2: 'HT DIGITAL / COUGAR / majesty', 6: 'NEXTGAMING / YICO', 9: 'COUGAR InfoHub', 10: 'Deep Colors Studio',
  11: 'SAMA / ARDOR GAMING / EVOLABS', 12: 'ASA', 13: 'YGT', 14: 'SAMA DIGITAL / SAMZHE', 15: 'EVOLVEO / TECNOMALL' };
const DAY = 24 * 3600 * 1000;
const countsFile = () => path.join(paths.base, 'themes-brands.json');

async function brandsWithThemes(refresh = false) {
  try {
    const c = JSON.parse(fs.readFileSync(countsFile(), 'utf8'));
    if (!refresh && Date.now() - c.at < DAY) return c.brands;
  } catch {}
  const brands = [];
  for (const id of Object.keys(BRANDS).map(Number)) {
    try { const r = await themes.list(1, { brand: id, pageSize: themes.PAGE_SIZE, refresh }); if (r.total > 0) brands.push({ id, total: r.total }); } catch {}
  }
  fs.mkdirSync(paths.base, { recursive: true });
  fs.writeFileSync(countsFile(), JSON.stringify({ at: Date.now(), brands }));
  return brands;
}

module.exports = {
  id: 'otherbrands',
  name: 'Other brands (same platform)',
  description: 'Themes other cooler brands publish on the same theme platform for this screen size. Made for their screens; most look fine on yours.',
  kind: 'online',
  async detect() { return { available: true, status: 'Online library (loaded when you browse it)' }; },
  // Pages run through the brands one after another.
  async list(ctx, { page = 1, refresh = false } = {}) {
    const brands = await brandsWithThemes(refresh);
    let p = page;
    for (const b of brands) {
      const pages = Math.ceil(b.total / themes.PAGE_SIZE);
      if (p <= pages) {
        const r = await themes.list(p, { brand: b.id, refresh });
        const total = brands.reduce((s, x) => s + x.total, 0);
        const last = brands[brands.length - 1] === b && p >= pages;
        return { items: r.themes.map(t => ({ id: `${b.id}:${t.themeId}`, title: t.name, thumb: t.cover, type: 'theme', action: 'add', key: t.themeId, badge: BRANDS[b.id] })), total, hasMore: !last };
      }
      p -= pages;
    }
    return { items: [], total: 0, hasMore: false };
  },
  async add(ctx, itemId, { onProgress }) {
    const [brand, themeId] = itemId.split(':');
    let t = themes.findCached(themeId, Number(brand));
    for (let pg = 1; !t && pg <= 10; pg++) { const r = await themes.list(pg, { brand: Number(brand) }); t = r.themes.find(x => x.themeId === themeId); if (!r.themes.length) break; }
    if (!t) throw new Error('Unknown theme. Refresh the list and try again.');
    const file = await themes.download(t, ctx.tmpDir, onProgress);
    return { file, name: t.file.name || `${t.themeId}.ht`, temp: true };
  },
};
