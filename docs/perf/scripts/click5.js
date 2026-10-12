// "Click through 5 new clips quickly" against the real 1.2.0 server (scratch library, fake device), replaying what
// public/app.js sends: select -> GET detail, estimate after 50 ms, auto loop search; when the search finishes and the clip is
// still selected -> GET detail + estimate for the picked loop. usage: node click5.js [gapMs]
const path = require('path');
const fs = require('fs');
const SCR = path.resolve(__dirname, '..');
process.env.COOLERSCREEN_HOME = path.join(SCR, 'home');
process.env.COOLERSCREEN_GALLERY = path.join(SCR, 'gallery');
process.env.COOLERSCREEN_FAKE_DEVICE = '1';
const { REPO, procs, startGpu, stopGpu } = require('./mon');
const GAP = Number(process.argv[2] || 1500);

// clips that look "newly imported": drop cached loop results so the auto search runs
const LIB = path.join(SCR, 'home', 'library');
const pick = ['mv3lymc594cbc0', 'mv3lzb5yfa5c1d', 'mv3lzw193ae73e', 'mv3m0g9e52cae4', 'mv3m1k1r0e1153'];
for (const id of pick) {
  const d = path.join(LIB, id);
  for (const n of fs.readdirSync(d)) if (/^loop(frames|-\d)/.test(n)) fs.rmSync(path.join(d, n));
  const f = path.join(d, 'original.json'); const o = JSON.parse(fs.readFileSync(f)); delete o.loops; delete o.draft; fs.writeFileSync(f, JSON.stringify(o, null, 2));
}

(async () => {
  const server = require(path.join(REPO, 'src', 'server'));
  const { url } = await server.start({ port: 8899, open: false });
  const base = url.replace(/\/$/, '');
  const api = async (p, method = 'GET', body) => { const r = await fetch(base + p, { method, headers: body ? { 'Content-Type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined }); return r.json(); };
  await new Promise(r => setTimeout(r, 4000)); // let the startup gallery sync finish
  startGpu();
  const T0 = Date.now(), t = () => ((Date.now() - T0) / 1000).toFixed(1) + 's';
  const log = []; const L = (...a) => { const s = [t(), ...a].join(' '); log.push(s); console.log(s); };
  let selected = null;
  const outstanding = new Set();
  const track = (name, p) => { outstanding.add(name); return p.finally(() => outstanding.delete(name)); };
  const qlog = setInterval(() => L('  in flight:', outstanding.size, [...outstanding].join(', ')), 2000);
  const done = [];
  for (let i = 0; i < pick.length; i++) {
    const id = pick[i]; selected = id;
    L(`click ${i + 1}: ${id}`);
    const d = await api('/api/originals/' + id);
    const draft = { start: 0, length: Math.floor(Math.min(10, d.duration) * 18) / 18, quality: 'auto', frame: { zoom: 1, x: 0.5, y: 0.5 }, corners: { on: false, r: 80 }, detail: 0, blend: 0 };
    const tc = Date.now();
    done.push(Promise.all([
      track(`est1-${i + 1}`, api(`/api/originals/${id}/estimate`, 'POST', draft).then(r => L(`  estimate#1 clip ${i + 1} back after ${((Date.now() - tc) / 1000).toFixed(1)} s`, r.stale ? '(stale)' : `q=${r.q}`))),
      track(`loops-${i + 1}`, (async () => {
        const { jobId } = await api(`/api/originals/${id}/loopsearch`, 'POST', { minLen: null, maxLen: null });
        for (;;) { const j = await api('/api/jobs/' + jobId); if (j.status === 'done' || j.status === 'error') { L(`  loops clip ${i + 1} ${j.status} after ${((Date.now() - tc) / 1000).toFixed(1)} s`); return j.result; } await new Promise(r => setTimeout(r, 400)); }
      })().then(async r => {
        if (selected !== id || !r || !r.candidates) return;
        const c = r.candidates[0];
        await new Promise(res => setTimeout(res, 600));
        const t2 = Date.now();
        const e = await track(`est2-${i + 1}`, api(`/api/originals/${id}/estimate`, 'POST', { ...draft, start: c.start, length: c.length, blend: c.blend || 0 }));
        L(`  estimate#2 (picked loop) clip ${i + 1} back after ${((Date.now() - tc) / 1000).toFixed(1)} s from click (${((Date.now() - t2) / 1000).toFixed(1)} s own)`, `q=${e.q}`);
      })),
    ]));
    if (i < pick.length - 1) await new Promise(r => setTimeout(r, GAP));
  }
  const tLast = Date.now();
  await Promise.all(done);
  L(`ALL settled; last click -> everything for last clip ready: ${((Date.now() - tLast) / 1000).toFixed(1)} s; ffmpeg processes: ${procs.length}`);
  clearInterval(qlog); stopGpu();
  fs.writeFileSync(path.join(SCR, 'out', `click5-gap${GAP}.log`), log.join('\n'));
  process.exit(0);
})();
