// Recovery after a loop clip the screen cannot play (it freezes the player on every boot):
// delete ONLY /data/video/sleep_video.mp4, then restart the screen so it boots without it.
// Run: node tools/recover-delete-loop.js   (afterwards upload a good clip with the app or CLI)
const { UsbScreen } = require('../src/usbScreen');
const sleep = ms => new Promise(r => setTimeout(r, ms));

(async () => {
  const s = await UsbScreen.open();
  try {
    const r = await s.cmd(0x05, Buffer.from('/data/video/sleep_video.mp4'), { timeout: 6000 }).catch(e => ({ error: e.message }));
    console.log('delete reply:', JSON.stringify(r));
    await sleep(500);
    const rr = await s.cmd(0x01, [], { timeout: 4000 }).catch(e => ({ error: e.message }));
    console.log('restart reply:', JSON.stringify(rr));
  } finally {
    await s.close();
  }
  console.log('Done. Wait ~15 seconds, then tell Claude.');
})().catch(e => { console.error(e.message); process.exit(1); });
