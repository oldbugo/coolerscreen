// 4.0 stream B: the second batch of item types (docs/v3/theme-format.md, "4.0 widgets"): meters, cores, network, disks,
// worldclock, timer. widgets.js merges ENTRIES / PALETTE_ADD / TYPE_NAME and calls defaultSize / nameOf / paintProps for these
// types; themes.js owns the editor state. Everything saves and applies at once through api.set / api.edited (no Save buttons).

const clamp = (v, lo, hi) => Math.min(hi, Math.max(lo, v));

/** The new item types. */
export const MORE_TYPES = ['meters', 'cores', 'network', 'disks', 'worldclock', 'timer', 'livetext', 'volume'];
export const isMore = t => MORE_TYPES.includes(t);

const meterRows = () => ['cpu.load', 'ram.used', 'gpu.load', 'gpu.temp'].map(metric => ({ metric }));

// [key, def]; same shape as widgets.js ENTRIES
export const ENTRIES = {
  meters: { type: 'meters', name: 'Meters', icon: '☷', props: () => ({ rows: meterRows(), unit: 'C', bars: true, rate: 1 }) },
  cores: { type: 'cores', name: 'CPU cores', icon: '▦', props: () => ({ style: 'bars', average: true, color: null, rate: 2 }) },
  network: { type: 'network', name: 'Down + up', icon: '⇵', props: () => ({ graph: true, totals: false, history: 60, downColor: '#5ad2a0', upColor: '#aa8cff', rate: 2 }) },
  disks: { type: 'disks', name: 'All disks', icon: '▤', props: () => ({ drives: [], show: 'used', maxRows: 4 }) },
  worldclock: { type: 'worldclock', name: 'World clocks', icon: '◷', props: () => ({
    zones: [{ name: 'New York', tz: 'Eastern Standard Time' }, { name: 'London', tz: 'GMT Standard Time' }, { name: 'Tokyo', tz: 'Tokyo Standard Time' }], format: '24h', color: '#ffffff' }) },
  livetext: { type: 'livetext', name: 'Live text', icon: 'T', props: () => ({ text: '{time}  CPU {cpu}  GPU {gpu}', size: 0.08, color: '#ffffff', opacity: 1, align: 'left', weight: 'regular' }) },
  volume: { type: 'volume', name: 'Volume', icon: '♪', props: () => ({ style: 'bar', color: '#5ac8ff' }) },
  timer: { type: 'timer', name: 'Timer', icon: '⏲', props: () => ({ mode: 'stopwatch', running: false, startedAt: 0, elapsedMs: 0, duration: 300, workMin: 25, breakMin: 5, color: '#ffffff' }) },
};

/** [group name, entry keys] pairs added to the palette (widgets.js puts them into the existing groups). */
export const PALETTE_ADD = { Time: ['worldclock', 'timer'], Sensors: ['meters', 'cores', 'network', 'disks'], Media: ['volume'], Decoration: ['livetext'] };

export const TYPE_NAME = { meters: 'Meters', cores: 'CPU cores', network: 'Down + up', disks: 'All disks', worldclock: 'World clocks', timer: 'Timer', livetext: 'Live text', volume: 'Volume' };

/** Default box [w, h] as fractions of the 2400x1080 canvas. */
export function defaultSize(type, p = {}) {
  switch (type) {
    case 'meters': return [0.26, clamp(0.085 * Math.max(2, (p.rows || []).length || 4) + 0.04, 0.16, 0.8)];
    case 'cores': return p.style === 'heat' ? [0.28, 0.34] : [0.3, 0.42];
    case 'network': return p.graph === false ? [0.34, 0.16] : [0.36, 0.34];
    case 'disks': return [0.28, clamp(0.1 * (p.maxRows || 4) + 0.04, 0.22, 0.7)];
    case 'worldclock': return [0.26, clamp(0.12 * Math.max(1, (p.zones || []).length || 3) + 0.04, 0.2, 0.7)];
    case 'timer': return [0.28, 0.28];
    case 'livetext': return [0.5, 0.12];
    case 'volume': return p.style === 'ring' ? [0.14, 0.32] : [0.26, 0.16];
    default: return null;
  }
}

export function nameOf(w) {
  const p = w.props || {};
  if (w.type === 'timer') return p.label || ({ stopwatch: 'Stopwatch', timer: 'Timer', pomodoro: 'Pomodoro' })[p.mode] || 'Timer';
  if (w.type === 'livetext') { const t = (p.text || '').replace(/\s+/g, ' ').trim(); return t ? `Live text: “${t.slice(0, 20)}${t.length > 20 ? '…' : ''}”` : 'Live text'; }
  if (w.type === 'meters') return `Meters (${(p.rows || []).length})`;
  if (w.type === 'worldclock') return `World clocks (${(p.zones || []).length})`;
  return TYPE_NAME[w.type] || w.type;
}

/** Words a live text understands, (word, what it shows); the same list as live/morewidgets.rs WORDS. */
export const WORDS = [['time', 'Time (24-hour)'], ['time12', 'Time (12-hour)'], ['seconds', 'Time with seconds'], ['date', 'Date'], ['weekday', 'Weekday'],
  ['cpu', 'CPU load'], ['cputemp', 'CPU temperature'], ['cpupower', 'CPU power'], ['cpufan', 'CPU fan'], ['gpu', 'GPU load'], ['gputemp', 'GPU temperature'],
  ['vram', 'GPU memory'], ['ram', 'RAM used'], ['down', 'Download speed'], ['up', 'Upload speed'], ['diskread', 'Disk read speed'], ['diskwrite', 'Disk write speed'],
  ['uptime', 'Uptime'], ['volume', 'Volume']];

/** Windows time-zone ids with the usual city (id, city). */
export const ZONES = [
  ['', 'This PC'],
  ['Pacific Standard Time', 'Los Angeles'], ['Mountain Standard Time', 'Denver'], ['Central Standard Time', 'Chicago'], ['Eastern Standard Time', 'New York'],
  ['Atlantic Standard Time', 'Halifax'], ['Mexico Standard Time', 'Mexico City'], ['SA Pacific Standard Time', 'Bogotá'], ['E. South America Standard Time', 'São Paulo'],
  ['Argentina Standard Time', 'Buenos Aires'], ['Hawaiian Standard Time', 'Honolulu'], ['Alaskan Standard Time', 'Anchorage'],
  ['UTC', 'UTC'], ['GMT Standard Time', 'London'], ['W. Europe Standard Time', 'Berlin'], ['Romance Standard Time', 'Paris'], ['Central Europe Standard Time', 'Prague'],
  ['GTB Standard Time', 'Athens'], ['FLE Standard Time', 'Helsinki'], ['Turkey Standard Time', 'Istanbul'], ['Russian Standard Time', 'Moscow'],
  ['Egypt Standard Time', 'Cairo'], ['South Africa Standard Time', 'Johannesburg'], ['E. Africa Standard Time', 'Nairobi'], ['W. Central Africa Standard Time', 'Lagos'],
  ['Arab Standard Time', 'Riyadh'], ['Arabian Standard Time', 'Dubai'], ['Iran Standard Time', 'Tehran'], ['Pakistan Standard Time', 'Karachi'],
  ['India Standard Time', 'Mumbai'], ['Bangladesh Standard Time', 'Dhaka'], ['SE Asia Standard Time', 'Bangkok'], ['China Standard Time', 'Beijing'],
  ['Singapore Standard Time', 'Singapore'], ['W. Australia Standard Time', 'Perth'], ['Taipei Standard Time', 'Taipei'], ['Tokyo Standard Time', 'Tokyo'],
  ['Korea Standard Time', 'Seoul'], ['AUS Eastern Standard Time', 'Sydney'], ['E. Australia Standard Time', 'Brisbane'], ['Cen. Australia Standard Time', 'Adelaide'],
  ['New Zealand Standard Time', 'Auckland'], ['Fiji Standard Time', 'Fiji'],
];
const cityOf = tz => (ZONES.find(z => z[0] === tz) || [])[1] || '';

/** Readings that need a tool the user runs (HWiNFO / LibreHardwareMonitor). */
export const EXT_METRICS = ['cpu.temp', 'cpu.power', 'cpu.fan', 'board.temp'];

/** A muted line saying where the metric is read from, or why it is unavailable (empty for ordinary metrics). */
export function extNote(api, metric) {
  const p = api.el('p', { class: 'muted small' });
  if (!EXT_METRICS.includes(metric)) { p.hidden = true; return p; }
  p.textContent = 'Checking…';
  api.backend.call('ext_sensor_status').then(s => { p.textContent = s.message; p.classList.toggle('warn-text', !s.ok); }, () => { p.hidden = true; });
  return p;
}

export function hasPanel(type) { return isMore(type); }

/**
 * Append the property controls of `w` to `host`. `api` is the same object widgets.js gets (el, h{field, select2, range,
 * numInput, swRow}, set, edited, repaint, relabel, moved); `ctx` = { METRICS, metricName, setStyle }.
 */
export function paintProps(w, host, api, ctx) {
  const { el, h } = api;
  const { field, select2, range, numInput, swRow } = h;
  const p = w.props;
  const colour = (key, text, dflt, resettable = false) => {
    const input = el('input', { type: 'color', id: 'pCol-' + key, value: p[key] || dflt, oninput: e => { api.set(key, e.target.value); if (reset) reset.disabled = false; } });
    input.value = p[key] || dflt;
    let reset = null;
    if (resettable) reset = el('button', { type: 'button', class: 'icon-btn', title: 'Automatic', 'aria-label': 'Automatic colour', disabled: !p[key], text: '↺', onclick: () => { api.set(key, null); input.value = dflt; reset.disabled = true; } });
    return field(text, resettable ? el('div', { class: 'inline-ctl' }, input, reset) : input, 'pCol-' + key);
  };
  const rateField = () => field('Rate', select2('pRate', String(p.rate || 1), [['1', '1 / s'], ['2', '2 / s'], ['4', '4 / s']], v => api.set('rate', Number(v))), 'pRate');
  /** A change that alters the item's best size (rows, cities, styles): resize unless the user sized it by hand. */
  const restructure = fn => {
    const before = defaultSize(w.type, p);
    const mine = before && Math.abs(w.w - before[0]) < 0.003 && Math.abs(w.h - before[1]) < 0.003;
    fn();
    if (mine) {
      const [nw, nh] = defaultSize(w.type, p);
      w.w = Math.round(clamp(nw, 0.02, 1) * 10000) / 10000; w.h = Math.round(clamp(nh, 0.02, 1) * 10000) / 10000;
      w.x = Math.round(clamp(w.x, 0, 1 - w.w) * 10000) / 10000; w.y = Math.round(clamp(w.y, 0, 1 - w.h) * 10000) / 10000;
      api.moved(w);
    }
    api.edited(); api.relabel(); api.repaint();
  };

  switch (w.type) {
    case 'meters': return metersPanel();
    case 'cores':
      host.append(
        field('Style', select2('pStyle', p.style || 'bars', [['bars', 'Bars'], ['heat', 'Heat map']], v => restructure(() => { p.style = v; })), 'pStyle'),
        swRow('Average', p.average !== false, v => api.set('average', v)),
        colour('color', 'Colour', '#5ac8ff', true), rateField());
      return true;
    case 'network': {
      host.append(swRow('Graph', p.graph !== false, v => restructure(() => { p.graph = v; })), swRow('Totals', !!p.totals, v => api.set('totals', v), 'Data moved since start-up.'));
      if (p.graph !== false) host.append(field('History (s)', range('pHist', clamp(p.history || 60, 10, 600), 10, 600, 10, v => api.set('history', Math.round(v)), v => String(v)), 'pHist'));
      host.append(colour('downColor', 'Download', '#5ad2a0'), colour('upColor', 'Upload', '#aa8cff'), rateField());
      return true;
    }
    case 'disks': {
      const drives = el('input', { type: 'text', id: 'pDrives', maxLength: 30, spellcheck: false, placeholder: 'All fixed drives',
        onchange: e => {
          const list = [...new Set((e.target.value.match(/[A-Za-z]/g) || []).map(c => c.toUpperCase() + ':'))].slice(0, 8);
          p.drives = list; e.target.value = list.join(', '); api.edited();
        } });
      drives.value = (p.drives || []).join(', ');
      host.append(
        field('Show', select2('pShow', p.show || 'used', [['used', 'Used / size'], ['free', 'Free']], v => api.set('show', v)), 'pShow'),
        field('Drives', drives, 'pDrives'),
        field('Rows', select2('pRows', String(clamp(p.maxRows || 4, 2, 6)), [2, 3, 4, 5, 6].map(n => [String(n), String(n)]), v => restructure(() => { p.maxRows = Number(v); })), 'pRows'));
      return true;
    }
    case 'worldclock': return worldPanel();
    case 'timer': return timerPanel();
    case 'livetext': return liveTextPanel();
    case 'volume':
      host.append(
        field('Style', select2('pStyle', p.style || 'bar', [['bar', 'Bar'], ['ring', 'Ring']], v => restructure(() => { p.style = v; })), 'pStyle'),
        colour('color', 'Colour', '#5ac8ff'));
      return true;
    default: return false;
  }

  function metersPanel() {
    const rows = Array.isArray(p.rows) ? p.rows : (p.rows = []);
    const list = el('div', { class: 'meter-rows' });
    const draw = () => {
      list.textContent = '';
      rows.forEach((r, i) => {
        const sel = el('select', { 'aria-label': 'Reading', onchange: e => { r.metric = e.target.value; api.edited(); draw(); } });
        let og = null, last = '';
        for (const [id, group, name] of ctx.METRICS) {
          if (group !== last) { og = el('optgroup', { label: group }); sel.append(og); last = group; }
          og.append(el('option', { value: id, text: name }));
        }
        sel.value = r.metric || 'cpu.load';
        const extra = [];
        if (r.metric === 'cpu.core') extra.push(numInput('', r.core ?? 0, { min: 0, max: 255, onchange: v => { r.core = clamp(v ?? 0, 0, 255); api.edited(); } }));
        if (r.metric === 'disk.used') {
          const d = el('input', { type: 'text', maxLength: 2, spellcheck: false, placeholder: 'C:', 'aria-label': 'Drive', onchange: e => { const v = e.target.value.trim().toUpperCase(); r.drive = /^[A-Z]:?$/.test(v) ? v.replace(/:?$/, ':') : 'C:'; e.target.value = r.drive; api.edited(); } });
          d.value = r.drive || 'C:';
          extra.push(d);
        }
        const lab = el('input', { type: 'text', maxLength: 24, spellcheck: false, placeholder: ctx.metricName(r.metric || 'cpu.load'), 'aria-label': 'Name', oninput: e => { r.label = e.target.value.trim() ? e.target.value : null; api.edited(); } });
        lab.value = typeof r.label === 'string' ? r.label : '';
        const move = (d) => { const j = i + d; if (j < 0 || j >= rows.length) return; [rows[i], rows[j]] = [rows[j], rows[i]]; api.edited(); draw(); };
        list.append(el('div', { class: 'meter-row' },
          sel, ...extra, lab,
          el('div', { class: 'inline-ctl' },
            el('button', { type: 'button', class: 'iconbtn', title: 'Move up', 'aria-label': 'Move up', text: '↑', disabled: i === 0, onclick: () => move(-1) }),
            el('button', { type: 'button', class: 'iconbtn', title: 'Move down', 'aria-label': 'Move down', text: '↓', disabled: i === rows.length - 1, onclick: () => move(1) }),
            el('button', { type: 'button', class: 'iconbtn', title: 'Remove', 'aria-label': 'Remove this row', text: '✕', onclick: () => restructure(() => { rows.splice(i, 1); }) }))));
      });
      if (!rows.length) list.append(el('p', { class: 'muted small', text: 'No rows yet.' }));
    };
    draw();
    const add = el('button', { type: 'button', class: 'btn secondary', text: 'Add a row', disabled: rows.length >= 8, onclick: () => restructure(() => { rows.push({ metric: 'cpu.load' }); }) });
    const ext = rows.map(r => r.metric).find(m => EXT_METRICS.includes(m));
    host.append(el('p', { class: 'small muted', text: 'Rows' }), list, ext ? extNote(api, ext) : el('span'), el('div', { class: 'row' }, add),
      swRow('Bars', p.bars !== false, v => api.set('bars', v)));
    if (rows.some(r => r.metric === 'cpu.temp' || r.metric === 'gpu.temp')) {
      host.append(field('Unit', select2('pUnit', p.unit || ctx.tempUnit(), [['C', '°C'], ['F', '°F']], v => api.set('unit', v)), 'pUnit'));
    }
    host.append(rateField());
    return true;
  }

  function liveTextPanel() {
    const t = el('input', { type: 'text', id: 'pText', maxLength: 200, spellcheck: false, oninput: e => { p.text = e.target.value; api.edited(); api.relabel(); } });
    t.value = typeof p.text === 'string' ? p.text : '';
    // insert a word where the cursor is
    const words = el('select', { id: 'pWord', 'aria-label': 'Insert a word', onchange: e => {
      const word = e.target.value; e.target.value = '';
      if (!word) return;
      const a = t.selectionStart ?? t.value.length, b = t.selectionEnd ?? a;
      t.value = t.value.slice(0, a) + '{' + word + '}' + t.value.slice(b);
      t.focus(); t.selectionStart = t.selectionEnd = a + word.length + 2;
      p.text = t.value; api.edited(); api.relabel();
    } }, [el('option', { value: '', text: 'Insert a word…' }), ...WORDS.map(([id, text]) => el('option', { value: id, text }))]);
    words.value = '';
    host.append(
      field('Text', t, 'pText'), field('Words', words, 'pWord'),
      field('Size', range('pSize', Math.round(clamp(p.size ?? 0.08, 0.02, 0.3) * 100), 2, 30, 1, v => api.set('size', Math.round(v) / 100), v => v + ' %'), 'pSize'),
      field('Weight', select2('pWeight', p.weight || 'regular', [['light', 'Light'], ['regular', 'Regular'], ['bold', 'Bold']], v => api.set('weight', v)), 'pWeight'),
      field('Align', select2('pAlign', p.align || 'left', [['left', 'Left'], ['center', 'Centre'], ['right', 'Right']], v => api.set('align', v)), 'pAlign'),
      colour('color', 'Colour', '#ffffff'),
      field('Opacity', range('pOp', Math.round(clamp(p.opacity ?? 1, 0, 1) * 100), 0, 100, 5, v => api.set('opacity', v / 100), v => v + ' %'), 'pOp'));
    return true;
  }

  function worldPanel() {
    const zones = Array.isArray(p.zones) ? p.zones : (p.zones = []);
    const list = el('div', { class: 'meter-rows' });
    const draw = () => {
      list.textContent = '';
      zones.forEach((z, i) => {
        const sel = el('select', { 'aria-label': 'Time zone', onchange: e => {
          // the name follows the zone's city until the user typed their own
          const old = cityOf(z.tz);
          z.tz = e.target.value;
          if (!z.name || z.name === old) z.name = cityOf(z.tz) === 'This PC' ? 'Local' : cityOf(z.tz);
          api.edited(); api.relabel(); draw();
        } },
        ZONES.map(([id, city]) => el('option', { value: id, text: city })));
        sel.value = ZONES.some(zz => zz[0] === z.tz) ? z.tz : '';
        const nm = el('input', { type: 'text', maxLength: 24, spellcheck: false, 'aria-label': 'Name', placeholder: 'Name', oninput: e => { z.name = e.target.value; api.edited(); } });
        nm.value = z.name || '';
        list.append(el('div', { class: 'meter-row' }, sel, nm,
          el('div', { class: 'inline-ctl' }, el('button', { type: 'button', class: 'iconbtn', title: 'Remove', 'aria-label': 'Remove this city', text: '✕', onclick: () => restructure(() => { zones.splice(i, 1); }) }))));
      });
      if (!zones.length) list.append(el('p', { class: 'muted small', text: 'No cities yet.' }));
    };
    draw();
    host.append(el('p', { class: 'small muted', text: 'Cities' }), list,
      el('div', { class: 'row' }, el('button', { type: 'button', class: 'btn secondary', text: 'Add a city', disabled: zones.length >= 4, onclick: () => restructure(() => { zones.push({ name: 'Paris', tz: 'Romance Standard Time' }); }) })),
      field('Format', select2('pFmt', p.format || '24h', [['24h', '24-hour'], ['12h', '12-hour']], v => api.set('format', v)), 'pFmt'),
      colour('color', 'Colour', '#ffffff'));
    return true;
  }

  function timerPanel() {
    const mode = p.mode || 'stopwatch';
    const running = !!p.running;
    const startStop = el('button', { type: 'button', class: 'btn', text: running ? 'Pause' : (p.elapsedMs > 0 ? 'Resume' : 'Start'), onclick: () => {
      if (p.running) { p.elapsedMs = (p.elapsedMs || 0) + Math.max(0, Date.now() - (p.startedAt || Date.now())); p.running = false; }
      else { p.running = true; p.startedAt = Date.now(); }
      api.edited(); api.repaint();
    } });
    const reset = el('button', { type: 'button', class: 'btn secondary', text: 'Reset', onclick: () => { p.running = false; p.elapsedMs = 0; p.startedAt = 0; api.edited(); api.repaint(); } });
    const lab = el('input', { type: 'text', id: 'pLab', maxLength: 30, spellcheck: false, placeholder: ({ stopwatch: 'Stopwatch', timer: 'Timer', pomodoro: 'Pomodoro' })[mode], oninput: e => api.set('label', e.target.value.trim() ? e.target.value : null) });
    lab.value = p.label || '';
    host.append(
      field('Mode', select2('pMode', mode, [['stopwatch', 'Stopwatch'], ['timer', 'Timer'], ['pomodoro', 'Pomodoro']], v => { p.mode = v; p.running = false; p.elapsedMs = 0; p.startedAt = 0; api.edited(); api.relabel(); api.repaint(); }), 'pMode'),
      el('div', { class: 'row' }, startStop, reset));
    if (mode === 'timer') {
      host.append(el('div', { class: 'props-grid' },
        field('Minutes', numInput('pMin', Math.floor((p.duration || 300) / 60), { min: 0, max: 5999, onchange: v => { p.duration = clamp((v ?? 0) * 60 + ((p.duration || 300) % 60), 1, 359999); api.edited(); } }), 'pMin'),
        field('Seconds', numInput('pSec', Math.round((p.duration || 300) % 60), { min: 0, max: 59, onchange: v => { p.duration = clamp(Math.floor((p.duration || 300) / 60) * 60 + (v ?? 0), 1, 359999); api.edited(); } }), 'pSec')));
    } else if (mode === 'pomodoro') {
      host.append(el('div', { class: 'props-grid' },
        field('Focus (min)', numInput('pWork', p.workMin ?? 25, { min: 1, max: 180, onchange: v => api.set('workMin', clamp(v ?? 25, 1, 180)) }), 'pWork'),
        field('Break (min)', numInput('pBreak', p.breakMin ?? 5, { min: 1, max: 60, onchange: v => api.set('breakMin', clamp(v ?? 5, 1, 60)) }), 'pBreak')));
    }
    host.append(field('Name', lab, 'pLab'), colour('color', 'Colour', '#ffffff'));
    return true;
  }
}
