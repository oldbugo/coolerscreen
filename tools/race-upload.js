// Recovery: restart the screen and send a known-good loop clip in the short window after boot,
// before the player reaches a clip it cannot decode. node tools/race-upload.js <good.mp4>
const fs = require('fs');
const { UsbScreen, findDevice } = require('../src/usbScreen');
const sleep = ms => new Promise(r => setTimeout(r, ms));

(async () => {
  const file = process.argv[2];
  const size = Buffer.alloc(4);
  size.writeUInt32LE(fs.statSync(file).size);
  let s = await UsbScreen.open();
  await s.cmd(0x01, [], { timeout: 3000 }).catch(() => {});
  await s.close();
  const t0 = Date.now();
  await sleep(300);
  for (;;) {
    if (Date.now() - t0 > 30000) throw new Error('screen did not come back within 30 s');
    if (!findDevice()) { await sleep(50); continue; }
    try { s = await UsbScreen.open(); } catch { await sleep(50); continue; }
    const ack = await s.cmd(0x13, size, { timeout: 2500 }).catch(e => ({ error: e.message }));
    console.log(`${Date.now() - t0} ms after restart: upload command reply ${JSON.stringify(ack)}`);
    if (ack && ack.key === 0x13) break;
    await s.close();
    await sleep(100);
  }
  const data = fs.readFileSync(file);
  for (let off = 0; off < data.length; off += 40 * 1024) await s.raw(data.subarray(off, off + 40 * 1024));
  console.log(`sent ${data.length} bytes in ${((Date.now() - t0) / 1000).toFixed(0)} s`);
  await s.close();
})().catch(e => { console.error(e.message); process.exit(1); });
