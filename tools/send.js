// node tools/send.js <key-hex> [byte ...]   e.g. node tools/send.js 15 1 10
// Sends one raw command and prints the reply (if any within 3 s).
const { Screen } = require('../src/device');

(async () => {
  const [key, ...bytes] = process.argv.slice(2);
  const s = await Screen.open();
  try {
    const r = await s.cmd(parseInt(key, 16), bytes.map(Number)).catch(e => ({ error: e.message }));
    console.log(JSON.stringify(r));
  } finally {
    await s.close();
  }
})().catch(e => { console.error(e.message); process.exit(1); });
