// Talk to the screen over WinUSB (no serial driver => no DTR => no watchdog).
// node tools/winusb.js info | restart | play [path] | desc
const usb = require('usb');
const { frame } = require('../src/device');

const dev = usb.findByIds(0x33c3, 0x7791) || usb.findByIds(0x33c3, 0x7788);
if (!dev) { console.error('screen not found'); process.exit(1); }
const cmd = process.argv[2] || 'info';

function describe() {
  for (const i of dev.configDescriptor.interfaces)
    for (const a of i)
      console.log('if', a.bInterfaceNumber, 'class', a.bInterfaceClass, 'eps',
        a.endpoints.map(e => '0x' + e.bEndpointAddress.toString(16) + (e.bmAttributes === 2 ? ' bulk' : e.bmAttributes === 3 ? ' int' : '')).join(', '));
}

(async () => {
  dev.open();
  describe();
  if (cmd === 'desc') return dev.close();
  let epIn, epOut, iface;
  for (const i of dev.interfaces) {
    const out = i.endpoints.find(e => e.direction === 'out' && e.transferType === usb.usb.LIBUSB_TRANSFER_TYPE_BULK);
    const inn = i.endpoints.find(e => e.direction === 'in' && e.transferType === usb.usb.LIBUSB_TRANSFER_TYPE_BULK);
    if (out && inn) { iface = i; epOut = out; epIn = inn; break; }
  }
  if (!iface) throw new Error('no bulk interface');
  iface.claim();
  console.log('claimed interface', iface.interfaceNumber);
  const transfer = buf => new Promise((res, rej) => epOut.transfer(buf, e => (e ? rej(e) : res())));
  const read = ms => new Promise(res => {
    epIn.timeout = ms;
    epIn.transfer(4096, (e, d) => res(e ? null : d));
  });
  let payload = [];
  let key = 0x06;
  if (cmd === 'restart') key = 0x01;
  if (cmd === 'play') { key = 0x04; payload = Buffer.from(process.argv[3] || '/data/video/start_video.mp4'); }
  if (cmd === 'play') await transfer(frame(0x0e, [1]));
  await transfer(frame(key, payload));
  for (let i = 0; i < 4; i++) {
    const d = await read(1000);
    if (!d) continue;
    const s = d.toString('utf8', 5, d.length - 2);
    console.log(s.startsWith('{') ? s : 'rx ' + d.toString('hex'));
    if (d[4] === key || s.startsWith('{')) break;
  }
  await new Promise(r => iface.release(true, () => r()));
  dev.close();
})().catch(e => { console.error(e.message); try { dev.close(); } catch {} process.exit(1); });
