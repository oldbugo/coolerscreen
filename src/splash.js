// The app's built-in intro clip (startup slot): a ~1.5 s boot splash generated once with ffmpeg and cached.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const paths = require('./paths');
const { makeSplash, SPLASH } = require('./convert');

const EST = 560000; // size before the file has been made once (measured: about 0.54 MB)
const file = () => path.join(paths.base, `splash-v${SPLASH.version}.mp4`);
let made = null;

/** Make (once) and return { file, size, id } where id identifies this exact splash for "already on screen" checks. */
async function get() {
  const f = file();
  if (!fs.existsSync(f)) {
    fs.mkdirSync(paths.base, { recursive: true });
    const tmp = f + '.tmp.mp4';
    await makeSplash(tmp);
    fs.renameSync(tmp, f);
  }
  if (!made || made.file !== f) {
    const buf = fs.readFileSync(f);
    made = { file: f, size: buf.length, id: `v${SPLASH.version}-` + crypto.createHash('sha1').update(buf).digest('hex').slice(0, 12) };
  }
  return made;
}

function sizeEstimate() {
  try { return fs.statSync(file()).size; } catch { return EST; }
}

module.exports = { get, sizeEstimate, file };
