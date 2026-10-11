// Feasibility test: record a Wallpaper Engine pop-out window with Electron's desktopCapturer
// (Windows Graphics Capture: works even if other windows cover it). Writes work/we-wgc.webm.
// Run: npx electron tools/we-capture-electron.js
const { app, BrowserWindow, desktopCapturer, ipcMain, session } = require('electron');
const { execFileSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const NAME = 'CoolerScreen capture';
const WE = 'C:\\Program Files (x86)\\Steam\\steamapps\\common\\wallpaper_engine\\wallpaper64.exe';
const WS = 'C:\\Program Files (x86)\\Steam\\steamapps\\workshop\\content\\431960';
const OUT = path.join(__dirname, '..', 'work', 'we-wgc.webm');
const sleep = ms => new Promise(r => setTimeout(r, ms));

function firstScene() {
  for (const id of fs.readdirSync(WS)) {
    const p = path.join(WS, id, 'project.json');
    try { if (JSON.parse(fs.readFileSync(p, 'utf8')).type === 'scene') return p; } catch {}
  }
}

app.whenReady().then(async () => {
  const proj = firstScene();
  execFileSync(WE, ['-control', 'openWallpaper', '-file', proj, '-playInWindow', NAME, '-width', '2400', '-height', '1080', '-x', '0', '-y', '0']);
  await sleep(5000);
  const sources = await desktopCapturer.getSources({ types: ['window'], thumbnailSize: { width: 0, height: 0 } });
  const src = sources.find(s => s.name === NAME);
  console.log('window source found:', !!src);
  if (!src) { execFileSync(WE, ['-control', 'closeWallpaper', '-location', NAME]); return app.quit(); }
  session.defaultSession.setDisplayMediaRequestHandler((req, cb) => cb({ video: src }));
  const win = new BrowserWindow({ show: false, webPreferences: { nodeIntegration: true, contextIsolation: false } });
  ipcMain.once('done', (e, buf) => {
    fs.writeFileSync(OUT, Buffer.from(buf));
    console.log('recorded bytes:', buf.byteLength);
    execFileSync(WE, ['-control', 'closeWallpaper', '-location', NAME]);
    app.quit();
  });
  await win.loadFile(path.join(__dirname, '..', 'work', 'blank.html'));
  await win.webContents.executeJavaScript(`(async () => {
    const { ipcRenderer } = require('electron');
    const stream = await navigator.mediaDevices.getDisplayMedia({ video: { width: 2400, height: 1080, frameRate: 18 }, audio: false });
    const rec = new MediaRecorder(stream, { mimeType: 'video/webm;codecs=vp9', videoBitsPerSecond: 40e6 });
    const chunks = [];
    rec.ondataavailable = e => chunks.push(e.data);
    rec.onstop = async () => ipcRenderer.send('done', await new Blob(chunks).arrayBuffer());
    rec.start();
    setTimeout(() => { rec.stop(); stream.getTracks().forEach(t => t.stop()); }, 4000);
  })()`);
});

