// WinUSB transport for the screen: same interface as Screen in device.js, but over libusb bulk endpoints.
// With the WinUSB driver bound (see scripts/setup-winusb.ps1) no CDC DTR request is ever sent,
// so the firmware's 30 s "DTR low -> display off" watchdog never triggers.
const { Screen, frame } = require('./device');

const VID = 0x33c3;
const PIDS = [0x7791, 0x7788];
const CHUNK = 40 * 1024;
const STATUS_KEY = 0x22; // unsolicited status frames every ~2 s; never a reply

let usbLib = null;
function usb() {
  if (!usbLib) usbLib = require('usb');
  return usbLib;
}

function findDevice() {
  const u = usb();
  for (const pid of PIDS) {
    const d = u.findByIds(VID, pid);
    if (d) return d;
  }
  return null;
}

class UsbScreen extends Screen {
  constructor(dev, iface, epIn, epOut) {
    super('winusb');
    this.dev = dev;
    this.iface = iface;
    this.epIn = epIn;
    this.epOut = epOut;
    this.transport = 'winusb';
  }

  /** Throws if the screen is absent or not bound to WinUSB (claim fails). */
  static async open() {
    const u = usb();
    const dev = findDevice();
    if (!dev) throw new Error('Screen not found (WinUSB)');
    try { dev.open(); } catch (e) { throw new Error('Cannot open the screen over WinUSB: ' + e.message); }
    let found = null;
    for (const i of dev.interfaces || []) {
      const out = i.endpoints.find(e => e.direction === 'out' && e.transferType === u.usb.LIBUSB_TRANSFER_TYPE_BULK);
      const inn = i.endpoints.find(e => e.direction === 'in' && e.transferType === u.usb.LIBUSB_TRANSFER_TYPE_BULK);
      if (out && inn) { found = { i, out, inn }; break; }
    }
    if (!found) { try { dev.close(); } catch {} throw new Error('Screen has no bulk interface (WinUSB driver not bound?)'); }
    try { found.i.claim(); } catch (e) {
      try { dev.close(); } catch {}
      throw new Error('Cannot claim the screen interface (is another program using it?): ' + e.message);
    }
    const s = new UsbScreen(dev, found.i, found.inn, found.out);
    s.epIn.on('data', d => s._onData(d));
    s.epIn.on('error', () => {});
    s.epIn.startPoll(3, 4096);
    await new Promise(r => setTimeout(r, 200));
    s.rx = Buffer.alloc(0);
    return s;
  }

  // Same framing as Screen._onData, but ignores unsolicited status frames.
  _onData(d) {
    this.rx = Buffer.concat([this.rx, d]);
    for (;;) {
      const i = this.rx.indexOf(Buffer.from([0x55, 0xaa]));
      if (i < 0) { this.rx = Buffer.alloc(0); return; }
      if (i > 0) this.rx = this.rx.subarray(i);
      if (this.rx.length < 4) return;
      const len = this.rx.readUInt16LE(2);
      if (len < 7) { this.rx = this.rx.subarray(2); continue; }
      if (this.rx.length < len) return;
      const f = this.rx.subarray(0, len);
      this.rx = this.rx.subarray(len);
      if (f[4] === STATUS_KEY) continue;
      const body = f.subarray(5, len - 2);
      let msg;
      try { msg = JSON.parse(body.toString('utf8')); } catch { msg = { key: f[4], hex: body.toString('hex') }; }
      const w = this.waiters.shift();
      if (w) w(msg);
    }
  }

  _write(buf) {
    return new Promise((res, rej) => this.epOut.transfer(buf, e => (e ? rej(e) : res())));
  }

  async raw(buf) {
    for (let off = 0; off < buf.length; off += CHUNK) await this._write(buf.subarray(off, off + CHUNK));
  }

  async close() {
    try { await new Promise(r => this.epIn.stopPoll(() => r())); } catch {}
    try { await new Promise(r => this.iface.release(true, () => r())); } catch {}
    try { this.dev.close(); } catch {}
  }
}

/** 'winusb' (usable now), 'none' (not visible to libusb). Does not leave the device open. */
async function probe() {
  try {
    const s = await UsbScreen.open();
    await s.close();
    return 'winusb';
  } catch { return 'none'; }
}

module.exports = { UsbScreen, probe, findDevice, frame };
