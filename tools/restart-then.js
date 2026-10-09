// Restart the screen, wait for it to come back, then send commands.
// node tools/restart-then.js "15 1 60" "14 1 60"
const { Screen, findPort } = require('../src/device');
const sleep = ms => new Promise(r => setTimeout(r, ms));

(async () => {
  let s = await Screen.open(undefined, { reset: false });
  await s.cmd(0x01).catch(() => {});
  await s.close();
  const t0 = Date.now();
  await sleep(1500);
  while (!(await findPort())) await sleep(200);
  for (let i = 0; ; i++) {
    await sleep(300);
    try { s = await Screen.open(undefined, { reset: false }); break; } catch (e) { if (i > 40) throw e; }
  }
  for (const c of process.argv.slice(2)) {
    const [k, ...b] = c.split(/\s+/);
    const r = await s.cmd(parseInt(k, 16), b.map(Number)).catch(e => ({ error: e.message }));
    console.log(((Date.now() - t0) / 1000).toFixed(1) + 's', c, JSON.stringify(r));
  }
  await s.close();
})().catch(e => { console.error(e.message); process.exit(1); });
