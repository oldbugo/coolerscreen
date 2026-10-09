// Restart the screen, then poll info until display_on goes false. Prints elapsed seconds.
// node tools/time-off.js [maxSeconds] [preKey preBytes...]   e.g. node tools/time-off.js 120 15 1 60
const { Screen, findPort } = require('../src/device');
const sleep = ms => new Promise(r => setTimeout(r, ms));

(async () => {
  const [max = '120', preKey, ...preBytes] = process.argv.slice(2);
  let s = await Screen.open();
  if (preKey) console.log('pre', preKey, preBytes, JSON.stringify(await s.cmd(parseInt(preKey, 16), preBytes.map(Number)).catch(e => e.message)));
  await s.cmd(0x01).catch(() => {});
  await s.close();
  const t0 = Date.now();
  await sleep(1500);
  while (!(await findPort())) await sleep(200);
  for (let i = 0; ; i++) {
    await sleep(500);
    try { s = await Screen.open(undefined, { reset: !process.env.NORESET }); break; } catch (e) { if (i > 30) throw e; }
  }
  console.log(((Date.now() - t0) / 1000).toFixed(1) + 's reconnected');
  let last;
  while ((Date.now() - t0) / 1000 < +max) {
    const r = await s.info().catch(e => ({ error: e.message }));
    const on = r?.data?.diplay_on;
    if (on !== last) console.log(((Date.now() - t0) / 1000).toFixed(1) + 's display_on=' + on);
    last = on;
    if (on === false) break;
    await sleep(1000);
  }
  await s.close();
})().catch(e => { console.error(e.message); process.exit(1); });
