// Serial protocol for Lovingcool-style USB screens (VID 33C3, PID 7788/7791).
// Frame: 55 AA | len16 LE (payload + 7) | key | payload | sum16 LE (sum of all previous bytes).
const fs = require('fs');
const { SerialPort } = require('serialport');

const VIDS = ['33C3'];
const PIDS = ['7788', '7791'];
const BAUD = 2000000;
const CHUNK = 40 * 1024;

const KEY = {
  restart: 0x01,
  brightness: 0x03,
  info: 0x06,
  live: 0x11,
  startup: 0x12,
  standby: 0x13,
  motionBeforeOff: 0x14,
  motionTimeout: 0x15,
  close: 0x21,
};

function frame(key, payload = Buffer.alloc(0)) {
  payload = Buffer.from(payload);
  const len = payload.length + 7;
  const head = Buffer.concat([Buffer.from([0x55, 0xaa, len & 0xff, len >> 8, key]), payload]);
  const sum = head.reduce((a, b) => a + b, 0) & 0xffff;
  return Buffer.concat([head, Buffer.from([sum & 0xff, sum >> 8])]);
}

async function findPort() {
  const ports = await SerialPort.list();
  return ports.find(p => VIDS.includes((p.vendorId || '').toUpperCase()) && PIDS.includes((p.productId || '').toUpperCase()));
}

class Screen {
  constructor(path) {
    this.path = path;
    this.rx = Buffer.alloc(0);
    this.waiters = [];
  }

  static async open(path, { reset = true } = {}) {
    if (!path) {
      const p = await findPort();
      if (!p) throw new Error('Screen not found (no USB serial device with VID 33C3)');
      path = p.path;
    }
    const s = new Screen(path);
    s.port = new SerialPort({ path, baudRate: BAUD, autoOpen: false });
    await new Promise((res, rej) => s.port.open(e => (e ? rej(e) : res())));
    s.port.on('data', d => s._onData(d));
    if (reset) {
      // Same reset the vendor app sends on connect: end any live JPEG stream.
      await s.raw(Buffer.from([0xff, 0xd9, 0xff, 0xd9]));
      await s.raw(Buffer.from([0, 0, 0, 0]));
    }
    await sleep(300);
    s.rx = Buffer.alloc(0);
    return s;
  }

  _onData(d) {
    this.rx = Buffer.concat([this.rx, d]);
    // Extract complete 55 AA frames.
    for (;;) {
      const i = this.rx.indexOf(Buffer.from([0x55, 0xaa]));
      if (i < 0) { this.rx = Buffer.alloc(0); return; }
      if (i > 0) this.rx = this.rx.subarray(i);
      if (this.rx.length < 4) return;
      const len = this.rx.readUInt16LE(2);
      if (this.rx.length < len) return;
      const f = this.rx.subarray(0, len);
      this.rx = this.rx.subarray(len);
      const body = f.subarray(5, len - 2);
      let msg;
      try { msg = JSON.parse(body.toString('utf8')); } catch { msg = { key: f[4], hex: body.toString('hex') }; }
      const w = this.waiters.shift();
      if (w) w(msg); else if (process.env.DEBUG) console.log('[unsolicited]', msg);
    }
  }

  reply(timeout = 3000) {
    return new Promise((res, rej) => {
      const t = setTimeout(() => {
        const i = this.waiters.indexOf(fn);
        if (i >= 0) this.waiters.splice(i, 1);
        rej(new Error('No reply from screen'));
      }, timeout);
      const fn = m => { clearTimeout(t); res(m); };
      this.waiters.push(fn);
    });
  }

  raw(buf) {
    return new Promise((res, rej) => {
      this.port.write(buf, e => e && rej(e));
      this.port.drain(e => (e ? rej(e) : res()));
    });
  }

  async cmd(key, payload, { wait = true, timeout } = {}) {
    const r = wait ? this.reply(timeout) : null;
    await this.raw(frame(key, payload));
    return r;
  }

  info() { return this.cmd(KEY.info); }

  setBrightness(v) { return this.cmd(KEY.brightness, [v]); }

  // [enable, value] — meaning partly inferred; see docs/PROTOCOL.md
  setMotionTimeout(a, b) { return this.cmd(KEY.motionTimeout, [a, b]); }
  setMotionBeforeOff(a, b) { return this.cmd(KEY.motionBeforeOff, [a, b], { wait: false }); }

  // Upload an MJPEG .mp4 as the onboard startup or standby animation.
  async uploadAnimation(file, type = 'startup', onProgress) {
    const data = fs.readFileSync(file);
    const size = Buffer.alloc(4);
    size.writeUInt32LE(data.length);
    const ack = await this.cmd(type === 'standby' ? KEY.standby : KEY.startup, size);
    const done = this.reply(30000).catch(e => ({ error: e.message }));
    for (let off = 0; off < data.length; off += CHUNK) {
      await this.raw(data.subarray(off, off + CHUNK));
      onProgress?.(Math.min(off + CHUNK, data.length), data.length);
    }
    return { ack, done: await done };
  }

  close() {
    return new Promise(res => (this.port?.isOpen ? this.port.close(() => res()) : res()));
  }
}

const sleep = ms => new Promise(r => setTimeout(r, ms));

module.exports = { Screen, frame, findPort, KEY };
