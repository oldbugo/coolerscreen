const { contextBridge, webUtils } = require('electron');
// Lets the page import dropped files by path (no upload copy).
contextBridge.exposeInMainWorld('cs', {
  pathFor: file => { try { return webUtils.getPathForFile(file); } catch { return ''; } },
});
