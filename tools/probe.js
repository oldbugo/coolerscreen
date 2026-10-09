// Read-only check: connect and print device info.
const { Screen } = require('../src/device');

(async () => {
  const s = await Screen.open(process.argv[2]);
  try {
    console.log(JSON.stringify(await s.info(), null, 2));
  } finally {
    await s.close();
  }
})().catch(e => { console.error(e.message); process.exit(1); });
