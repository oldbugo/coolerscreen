// Hold the port open (DTR high) and play a named onboard file via key 0x04.
// node tools/play-hold.js [seconds] [path]
const { SerialPort } = require('serialport');
const { frame, findPort } = require('../src/device');
const sleep = ms => new Promise(r => setTimeout(r, ms));

(async () => {
  const hold = Number(process.argv[2] || 60);
  const file = process.argv[3] || '/data/video/start_video.mp4';
  const p = await findPort();
  const port = new SerialPort({ path: p.path, baudRate: 2000000, autoOpen: false });
  await new Promise((res, rej) => port.open(e => (e ? rej(e) : res())));
  await new Promise(res => port.set({ dtr: true, rts: true }, res));
  port.on('data', d => { const h = d.toString('hex'); if (!h.startsWith('55aa0b0022')) console.log('rx', h); });
  port.write(frame(0x0e, [1]));
  await sleep(200);
  port.write(frame(0x04, Buffer.from(file)));
  console.log('playing', file, 'holding', hold, 's');
  await sleep(hold * 1000);
  await new Promise(res => port.close(() => res()));
  console.log('closed', new Date().toLocaleTimeString());
})().catch(e => { console.error(e.message); process.exit(1); });
