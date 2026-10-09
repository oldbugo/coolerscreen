// node tools/upload.js <file.mp4> [startup|standby]
const fs = require('fs');
const { Screen } = require('../src/device');

(async () => {
  const [file, type = 'startup'] = process.argv.slice(2);
  console.log(`${file}: ${(fs.statSync(file).size / 1024).toFixed(0)} KB -> ${type}`);
  const s = await Screen.open();
  try {
    const r = await s.uploadAnimation(file, type, (n, t) => process.stdout.write(`\r${Math.round((n / t) * 100)}%`));
    console.log('\n', JSON.stringify(r));
    console.log(JSON.stringify((await s.info()).data));
  } finally {
    await s.close();
  }
})().catch(e => { console.error(e.message); process.exit(1); });
