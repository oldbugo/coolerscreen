// Recovery, second form: restart the screen, then send "delete /data/video/sleep_video.mp4" the moment it is
// reachable again - before the player opens the unplayable clip (an open file cannot be deleted).
// Run: node tools/recover-race-delete.js
const { UsbScreen, findDevice } = require('../src/usbScreen');
const sleep = ms => new Promise(r => setTimeout(r, ms));
const PATH = Buffer.from('/data/video/sleep_video.mp4');

(async () => {
  let s = await UsbScreen.open();
  await s.cmd(0x01, [], { timeout: 3000 }).catch(() => {});
  await s.close();
  const t0 = Date.now();
  while (findDevice()) { if (Date.now() - t0 > 5000) break; await sleep(20); } // wait for it to drop off USB
  for (let attempt = 1; Date.now() - t0 < 30000; attempt++) {
    if (!findDevice()) { await sleep(20); continue; }
    try { s = await UsbScreen.open(); } catch { await sleep(30); continue; }
    const r = await s.cmd(0x05, PATH, { timeout: 1500 }).catch(e => ({ error: e.message }));
    await s.close();
    console.log(`${Date.now() - t0} ms: delete reply ${JSON.stringify(r)}`);
    if (r && r.key === 5 && r.hex === '00') { console.log('Deleted. The screen should now be usable. Tell Claude.'); return; }
    await sleep(100);
  }
  console.log('Did not get a successful delete within 30 s. Tell Claude what was printed.');
})().catch(e => { console.error(e.message); process.exit(1); });
