// Records one window to a WebM file with Electron's desktopCapturer (Windows Graphics Capture: it works even when other
// windows cover the window). Only available when the app runs as the desktop app (Electron main process).
// Uses its own session partition, so the app's own pages are never offered a screen capture.
const fs = require('fs');
const crypto = require('crypto');

let electron = null;
try { if (process.versions.electron) electron = require('electron'); } catch {}
const available = () => !!(electron && electron.desktopCapturer && electron.app && electron.app.isReady());
const sleep = ms => new Promise(r => setTimeout(r, ms));
// A local file page: screen capture APIs need a secure context, which a data: URL is not.
function blankPage() {
  const f = require('path').join(require('../paths').base, 'capture.html');
  if (!fs.existsSync(f)) { fs.mkdirSync(require('path').dirname(f), { recursive: true }); fs.writeFileSync(f, '<!doctype html><title>capture</title>'); }
  return f;
}

async function findWindow(title, waitMs) {
  const end = Date.now() + waitMs;
  for (;;) {
    const sources = await electron.desktopCapturer.getSources({ types: ['window'], thumbnailSize: { width: 0, height: 0 } });
    const s = sources.find(x => x.name === title);
    if (s || Date.now() > end) return s || null;
    await sleep(500);
  }
}

// Runs in the hidden recording page: capture -> MediaRecorder (VP9) -> 1 s chunks sent to the main process.
function rendererScript({ chan, seconds, fps }) {
  return `(async () => {
    const { ipcRenderer } = require('electron');
    try {
      const stream = await navigator.mediaDevices.getDisplayMedia({ video: { frameRate: ${fps} }, audio: false });
      const rec = new MediaRecorder(stream, { mimeType: 'video/webm;codecs=vp9', videoBitsPerSecond: 30e6 });
      let pending = Promise.resolve();
      rec.ondataavailable = e => { if (e.data && e.data.size) pending = pending.then(async () => ipcRenderer.send('${chan}:data', await e.data.arrayBuffer())); };
      rec.onstop = () => pending.then(() => ipcRenderer.send('${chan}:end'));
      rec.start(1000);
      let t = 0;
      const tick = setInterval(() => { t++; ipcRenderer.send('${chan}:tick', t); }, 1000);
      setTimeout(() => { clearInterval(tick); rec.stop(); stream.getTracks().forEach(x => x.stop()); }, ${Math.round(seconds * 1000)});
    } catch (e) { ipcRenderer.send('${chan}:error', String(e && e.message || e)); }
  })()`;
}

/** Record the window titled `title` for `seconds` into `out` (WebM). onProgress(0..1). */
async function recordWindow(title, { seconds = 20, fps = 18, out, onProgress = () => {} }) {
  if (!available()) throw new Error('Recording needs the desktop app (CoolerScreen.exe).');
  const src = await findWindow(title, 15000);
  if (!src) throw new Error('The window to record did not appear.');
  const { BrowserWindow, session, ipcMain } = electron;
  const ses = session.fromPartition('coolerscreen-capture');
  ses.setDisplayMediaRequestHandler((req, cb) => cb({ video: src }));
  const chan = 'cs-rec-' + crypto.randomBytes(4).toString('hex');
  const ws = fs.createWriteStream(out);
  const win = new BrowserWindow({ show: false, webPreferences: { partition: 'coolerscreen-capture', nodeIntegration: true, contextIsolation: false, sandbox: false, backgroundThrottling: false } });
  const cleanup = () => {
    for (const s of ['data', 'end', 'tick', 'error']) ipcMain.removeAllListeners(`${chan}:${s}`);
    try { ses.setDisplayMediaRequestHandler(null); } catch {}
    try { win.destroy(); } catch {}
  };
  try {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('Recording did not finish.')), (seconds + 30) * 1000);
      ipcMain.on(`${chan}:data`, (e, buf) => ws.write(Buffer.from(buf)));
      ipcMain.on(`${chan}:tick`, (e, t) => onProgress(Math.min(1, t / seconds)));
      ipcMain.once(`${chan}:end`, () => { clearTimeout(timer); ws.end(resolve); });
      ipcMain.once(`${chan}:error`, (e, msg) => { clearTimeout(timer); reject(new Error('Recording failed: ' + msg)); });
      win.loadFile(blankPage()).then(() => win.webContents.executeJavaScript(rendererScript({ chan, seconds, fps }))).catch(reject);
    });
  } catch (e) {
    ws.destroy();
    fs.rmSync(out, { force: true });
    throw e;
  } finally { cleanup(); }
  return out;
}

module.exports = { available, recordWindow };
