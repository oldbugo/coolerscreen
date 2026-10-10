// Desktop window around the local server. Nothing keeps running after the window is closed.
const { app, BrowserWindow, shell, Menu } = require('electron');
const path = require('path');
const fs = require('fs');

const DEBUG_READY = process.env.COOLERSCREEN_READY_FILE; // test hook: write the server URL here when loaded

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  let win = null, server = null;
  app.on('second-instance', () => { if (win) { if (win.isMinimized()) win.restore(); win.focus(); } });

  app.whenReady().then(async () => {
    Menu.setApplicationMenu(null);
    const { start } = require('../src/server');
    const started = await start({ open: false, port: 0 });
    server = started.server;
    const url = started.url;

    win = new BrowserWindow({
      width: 1280, height: 860, minWidth: 900, minHeight: 640,
      title: 'CoolerScreen', backgroundColor: '#14161a', autoHideMenuBar: true, show: false,
      webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true, nodeIntegration: false, sandbox: true },
    });
    win.once('ready-to-show', () => win.show());
    // Links to other sites open in the normal browser; the app window only ever shows the local server.
    win.webContents.setWindowOpenHandler(({ url: u }) => { if (/^https?:/.test(u)) shell.openExternal(u); return { action: 'deny' }; });
    win.webContents.on('will-navigate', (e, u) => { if (!u.startsWith(url)) { e.preventDefault(); if (/^https?:/.test(u)) shell.openExternal(u); } });
    win.webContents.on('did-finish-load', () => {
      if (DEBUG_READY) {
        let native = '';
        try { const usb = require('usb'); native += 'usb(' + usb.getDeviceList().length + ' devices) '; } catch (e) { native += 'usb FAILED ' + e.message + ' '; }
        try { require('@serialport/bindings-cpp'); native += 'serialport ok '; } catch (e) { native += 'serialport FAILED ' + e.message + ' '; }
        try { native += 'ffmpeg=' + require('ffmpeg-static') + ' exists=' + fs.existsSync(require('ffmpeg-static')); } catch (e) { native += 'ffmpeg FAILED ' + e.message; }
        fs.writeFileSync(DEBUG_READY, url + '\n' + native + '\n');
      }
    });
    win.on('closed', () => { win = null; });
    await win.loadURL(url);
  }).catch(e => { console.error(e); app.exit(1); });

  // Closing the window ends everything: stop the server and exit.
  app.on('window-all-closed', () => { try { server && server.close(); } catch {} app.quit(); });
}
