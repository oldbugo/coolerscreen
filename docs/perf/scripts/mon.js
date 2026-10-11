// Shared helpers: spawn instrumentation (count/time ffmpeg processes) + CPU/GPU sampling per stage.
const cp = require('child_process');
const os = require('os');
const REPO = 'C:\\Users\\pc\\Documents\\CoolerScreen';

const procs = []; // {t0,t1,args}
const origSpawn = cp.spawn;
cp.spawn = function (cmd, args, opts) {
  const p = origSpawn.call(this, cmd, args, opts);
  if (/ffmpeg/i.test(String(cmd))) {
    const rec = { t0: Date.now(), args: (args || []).join(' ') };
    procs.push(rec);
    p.on('close', () => (rec.t1 = Date.now()));
  }
  return p;
};

// CPU sampler (os.cpus deltas) every 250 ms
const cpuSamples = [];
let last = os.cpus();
const cpuT = setInterval(() => {
  const now = os.cpus();
  let busy = 0, total = 0;
  now.forEach((c, i) => {
    const a = last[i].times, b = c.times;
    const t = (b.user - a.user) + (b.nice - a.nice) + (b.sys - a.sys) + (b.irq - a.irq) + (b.idle - a.idle);
    total += t; busy += t - (b.idle - a.idle);
  });
  last = now;
  cpuSamples.push({ t: Date.now(), cpu: total ? (100 * busy) / total : 0 });
}, 250);
cpuT.unref();

// GPU sampler
const gpuSamples = [];
let smi;
function startGpu() {
  smi = origSpawn('nvidia-smi', ['--query-gpu=utilization.gpu,utilization.decoder,utilization.encoder', '--format=csv,noheader,nounits', '-lms', '250'], { windowsHide: true });
  let buf = '';
  smi.stdout.on('data', d => {
    buf += d; let i;
    while ((i = buf.indexOf('\n')) >= 0) {
      const l = buf.slice(0, i).trim(); buf = buf.slice(i + 1);
      const [g, dec, enc] = l.split(',').map(Number);
      if (Number.isFinite(g)) gpuSamples.push({ t: Date.now(), g, dec, enc });
    }
  });
}
function stopGpu() { try { smi.kill(); } catch {} }

const avg = a => (a.length ? a.reduce((x, y) => x + y, 0) / a.length : 0);
const max = a => (a.length ? Math.max(...a) : 0);
async function stage(name, fn, results) {
  const n0 = procs.length, t0 = Date.now();
  let r, err;
  try { r = await fn(); } catch (e) { err = e.message; }
  const t1 = Date.now();
  await new Promise(res => setTimeout(res, 300));
  const cs = cpuSamples.filter(s => s.t > t0 && s.t <= t1 + 250).map(s => s.cpu);
  const gs = gpuSamples.filter(s => s.t > t0 && s.t <= t1 + 250);
  const row = {
    stage: name, ms: t1 - t0, ffmpeg: procs.length - n0,
    cpuAvg: +avg(cs).toFixed(0), cpuMax: +max(cs).toFixed(0),
    gpuAvg: +avg(gs.map(s => s.g)).toFixed(0), decAvg: +avg(gs.map(s => s.dec)).toFixed(0), encAvg: +avg(gs.map(s => s.enc)).toFixed(0),
    ...(err ? { err } : {}),
  };
  console.log(JSON.stringify(row));
  if (results) results.push(row);
  return r;
}
module.exports = { REPO, procs, stage, startGpu, stopGpu };
