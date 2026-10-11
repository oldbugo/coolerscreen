// Deletes the leftover firmware-update package /data/ota.cpio from the screen (frees ~0.68 MB).
// A copy of the same file is the vendor download APP_OTA_...v3.6.bin (backed up in Documents\CoolerScreen-backup-*\screen-files).
// Only this exact path is touched. Run: node tools/delete-ota.js
const { UsbScreen } = require('../src/usbScreen');

(async () => {
  const s = await UsbScreen.open();
  try {
    const before = (await s.info()).data.i_block_free * 512;
    const listing = await s.cmd(0x07, Buffer.from('/data'), { timeout: 5000 });
    if (!listing.data.dir_list.some(f => f.filename === 'ota.cpio')) { console.log('ota.cpio is not on the screen; nothing to do.'); return; }
    const r = await s.cmd(0x05, Buffer.from('/data/ota.cpio'), { timeout: 8000 }).catch(e => ({ error: e.message }));
    console.log('delete reply:', JSON.stringify(r));
    const after = (await s.info()).data.i_block_free * 512;
    const files = (await s.cmd(0x07, Buffer.from('/data'), { timeout: 5000 })).data.dir_list.map(f => f.filename);
    console.log('files now in /data:', files.join(', '));
    console.log(`free space: ${(before / 1048576).toFixed(2)} MB -> ${(after / 1048576).toFixed(2)} MB`);
  } finally {
    await s.close();
  }
})().catch(e => { console.error(e.message); process.exit(1); });
