// Finish an interrupted upload: the screen keeps counting bytes until it has the size it was told,
// so feed zero filler until it acks (key 0x12/0x13 reply), then report. Usage: node tools/unstick.js
const { UsbScreen } = require('../src/usbScreen');

(async () => {
  const s = await UsbScreen.open();
  let acked = null;
  s.waiters.push(m => { acked = m; });
  const zero = Buffer.alloc(40 * 1024);
  let sent = 0;
  const t0 = Date.now();
  while (!acked && sent < 10 * 1024 * 1024) {
    await Promise.race([
      s._write(zero),
      new Promise((_, rej) => setTimeout(() => rej(new Error('write timed out after ' + sent + ' bytes')), 15000)),
    ]);
    sent += zero.length;
    if (sent % (1024 * 1024) < zero.length) console.log(`${(sent / 1048576).toFixed(1)} MB filler, ${((Date.now() - t0) / 1000).toFixed(0)} s`);
    await new Promise(r => setTimeout(r, 5));
  }
  await new Promise(r => setTimeout(r, 3000));
  console.log('ack:', JSON.stringify(acked), 'after', sent, 'bytes');
  await s.close();
})().catch(e => { console.error(e.message); process.exit(1); });
