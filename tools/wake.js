// Turn the display on and loop the onboard animation.
// node tools/wake.js [holdSeconds]   holdSeconds=0 closes immediately (keeping DTR high if possible)
const { SerialPort } = require('serialport');
const { frame, findPort } = require('../src/device');
const sleep = ms => new Promise(r => setTimeout(r, ms));

(async () => {
  const hold = Number(process.argv[2] || 0);
  const p = await findPort();
  const port = new SerialPort({ path: p.path, baudRate: 2000000, autoOpen: false, hupcl: false });
  await new Promise((res, rej) => port.open(e => (e ? rej(e) : res())));
  await new Promise(res => port.set({ dtr: true, rts: true }, res));
  port.on('data', d => console.log('rx', d.toString('hex')));
  port.write(frame(0x0e, [1]));
  await sleep(200);
  port.write(frame(0x14, [1, 0]));
  await sleep(500);
  if (hold) { console.log(`holding port open ${hold}s`); await sleep(hold * 1000); }
  await new Promise(res => port.close(() => res()));
  console.log('closed', new Date().toLocaleTimeString());
})().catch(e => { console.error(e.message); process.exit(1); });
