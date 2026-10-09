// Test: open, play start video, drop DTR ourselves, cancel the watchdog with a valid frame, close.
const { SerialPort } = require('serialport');
const { frame, findPort } = require('../src/device');
const sleep = ms => new Promise(r => setTimeout(r, ms));

(async () => {
  const p = await findPort();
  const port = new SerialPort({ path: p.path, baudRate: 2000000, autoOpen: false, hupcl: false });
  await new Promise((res, rej) => port.open(e => (e ? rej(e) : res())));
  port.on('data', d => { const h = d.toString('hex'); if (!h.startsWith('55aa0b0022')) console.log('rx', h); });
  await new Promise(res => port.set({ dtr: true, rts: true }, res));
  port.write(frame(0x0e, [1]));
  await sleep(200);
  port.write(frame(0x04, Buffer.from('/data/video/start_video.mp4')));
  await sleep(500);
  await new Promise(res => port.set({ dtr: false, rts: false }, res));
  await sleep(300);
  port.write(frame(0x0e, [1]));
  await sleep(500);
  await new Promise(res => port.close(() => res()));
  console.log('closed', new Date().toLocaleTimeString());
})().catch(e => { console.error(e.message); process.exit(1); });
