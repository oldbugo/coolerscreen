// Reproduce an upload inside Electron's main process (same runtime as CoolerScreen.exe).
// npx electron tools/electron-send.js <file.mp4> [standby|startup]
const { app } = require('electron');
const path = require('path');

app.whenReady().then(async () => {
  const [file, slot = 'standby'] = process.argv.slice(2).filter(a => !a.startsWith('-') && !a.endsWith('electron-send.js') && a !== '.');
  const deviceOps = require(path.join(__dirname, '..', 'src', 'deviceOps'));
  const t0 = Date.now();
  try {
    const r = await deviceOps.sendFiles([{ file, slot }], (f, m) => {}, { playLoop: slot === 'standby' });
    console.log('RESULT ok', JSON.stringify(r), ((Date.now() - t0) / 1000).toFixed(0) + 's');
  } catch (e) {
    console.log('RESULT error', e.message, ((Date.now() - t0) / 1000).toFixed(0) + 's');
  }
  app.quit();
});
