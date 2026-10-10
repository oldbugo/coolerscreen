// Reader for the vendor's ".ht" theme files (and plain .zip files): no dependencies.
// A .ht is a ZIP with PKWARE "ZipCrypto" encryption (password "ht") containing temp.zip, which contains
// <id>.jpg (preview), a .json (label, size, bgMedia) and the video mp4.
const fs = require('fs');
const path = require('path');
const zlib = require('zlib');

const PASSWORD = 'ht';
const VIDEO_EXT = ['.mp4', '.mov', '.mkv', '.webm', '.avi', '.m4v', '.gif'];
const IMAGE_EXT = ['.jpg', '.jpeg', '.png', '.bmp', '.webp'];

const CRC = (() => {
  const t = new Uint32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c >>> 0;
  }
  return t;
})();
const crcStep = (crc, b) => (CRC[(crc ^ b) & 0xff] ^ (crc >>> 8)) >>> 0;

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = crcStep(c, buf[i]);
  return (c ^ 0xffffffff) >>> 0;
}

function zipCryptoDecrypt(data, password) {
  let k0 = 305419896, k1 = 591751049, k2 = 878082192;
  const update = c => {
    k0 = crcStep(k0, c);
    k1 = (k1 + (k0 & 0xff)) >>> 0;
    k1 = (Math.imul(k1, 134775813) + 1) >>> 0;
    k2 = crcStep(k2, k1 >>> 24);
  };
  for (const ch of Buffer.from(password, 'utf8')) update(ch);
  const out = Buffer.allocUnsafe(data.length);
  for (let i = 0; i < data.length; i++) {
    const t = (k2 | 2) & 0xffff;
    const c = data[i] ^ (((t * (t ^ 1)) >>> 8) & 0xff);
    out[i] = c;
    update(c);
  }
  return out; // first 12 bytes are the random header
}

function readEntries(buf) {
  let eocd = -1;
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 65557); i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) { eocd = i; break; }
  }
  if (eocd < 0) throw new Error('Not a zip file');
  const count = buf.readUInt16LE(eocd + 10);
  let p = buf.readUInt32LE(eocd + 16);
  const entries = [];
  for (let n = 0; n < count; n++) {
    if (buf.readUInt32LE(p) !== 0x02014b50) throw new Error('Corrupt zip directory');
    const flags = buf.readUInt16LE(p + 8);
    const nlen = buf.readUInt16LE(p + 28), elen = buf.readUInt16LE(p + 30), clen = buf.readUInt16LE(p + 32);
    const nameBuf = buf.subarray(p + 46, p + 46 + nlen);
    entries.push({
      name: nameBuf.toString(flags & 0x800 ? 'utf8' : 'utf8').replace(/\\/g, '/'),
      flags, method: buf.readUInt16LE(p + 10), time: buf.readUInt16LE(p + 12), crc: buf.readUInt32LE(p + 16),
      csize: buf.readUInt32LE(p + 20), usize: buf.readUInt32LE(p + 24), offset: buf.readUInt32LE(p + 42),
    });
    p += 46 + nlen + elen + clen;
  }
  return entries;
}

function extractEntry(buf, e, password = PASSWORD) {
  const o = e.offset;
  if (buf.readUInt32LE(o) !== 0x04034b50) throw new Error('Corrupt zip entry: ' + e.name);
  const start = o + 30 + buf.readUInt16LE(o + 26) + buf.readUInt16LE(o + 28);
  let data = buf.subarray(start, start + e.csize);
  if (e.flags & 1) {
    if (e.flags & 0x40) throw new Error('Unsupported zip encryption');
    const dec = zipCryptoDecrypt(data, password);
    const check = e.flags & 8 ? e.time >>> 8 : e.crc >>> 24;
    if (dec[11] !== check) throw new Error('Wrong password for theme file');
    data = dec.subarray(12);
  }
  if (e.method === 0) return Buffer.from(data);
  if (e.method === 8) return zlib.inflateRawSync(data);
  throw new Error('Unsupported zip compression method ' + e.method);
}

const looksLikeZip = buf => buf.length > 4 && buf.readUInt32LE(0) === 0x04034b50;

function collect(buf, acc, depth) {
  for (const e of readEntries(buf)) {
    if (e.name.endsWith('/')) continue;
    const ext = path.extname(e.name).toLowerCase();
    const base = path.basename(e.name);
    if (ext === '.zip' && depth < 3) {
      collect(extractEntry(buf, e), acc, depth + 1);
    } else if (ext === '.json') {
      try { acc.json = acc.json || JSON.parse(extractEntry(buf, e).toString('utf8').replace(/^﻿/, '')); } catch {}
    } else if (VIDEO_EXT.includes(ext)) {
      acc.videos.push({ base, ext, size: e.usize, get: () => extractEntry(buf, e) });
    } else if (IMAGE_EXT.includes(ext)) {
      acc.images.push({ base, ext, size: e.usize, get: () => extractEntry(buf, e) });
    }
  }
}

function labelOf(json) {
  const l = json && (json.label || json.name || json.title);
  if (typeof l === 'string') return l.trim();
  if (l && typeof l === 'object') return String(l.zh || l.cn || l.en || Object.values(l)[0] || '').trim();
  return '';
}

/** True if the file looks like a theme/zip container (by extension or PK header). */
function isThemeFile(file) {
  const ext = path.extname(file).toLowerCase();
  if (ext === '.ht' || ext === '.zip') return true;
  try {
    const fd = fs.openSync(file, 'r');
    const b = Buffer.alloc(4);
    fs.readSync(fd, b, 0, 4, 0);
    fs.closeSync(fd);
    return b.readUInt32LE(0) === 0x04034b50;
  } catch { return false; }
}

/**
 * Extract the media from a .ht/.zip into outDir.
 * Returns { path, kind: 'video'|'image', name, previewJpg } (previewJpg is a Buffer or null).
 * Files are written with plain ASCII names (theme.mp4 etc.).
 */
function extractTheme(file, outDir) {
  const buf = fs.readFileSync(file);
  if (!looksLikeZip(buf)) throw new Error('Not a theme or zip file');
  const acc = { json: null, videos: [], images: [] };
  collect(buf, acc, 0);
  const fallbackName = path.basename(file, path.extname(file)).split('_')[0];
  const name = labelOf(acc.json) || fallbackName;
  fs.mkdirSync(outDir, { recursive: true });
  const biggest = list => list.slice().sort((a, b) => b.size - a.size)[0];
  const v = biggest(acc.videos);
  if (v) {
    const dest = path.join(outDir, 'theme' + v.ext);
    fs.writeFileSync(dest, v.get());
    const jpg = biggest(acc.images);
    return { path: dest, kind: 'video', name, previewJpg: jpg ? jpg.get() : null };
  }
  const img = biggest(acc.images);
  if (img) {
    const dest = path.join(outDir, 'theme' + img.ext);
    fs.writeFileSync(dest, img.get());
    return { path: dest, kind: 'image', name, previewJpg: null };
  }
  throw new Error('This theme/zip file contains no video or image I can use.');
}

module.exports = { extractTheme, isThemeFile, zipCryptoDecrypt, crc32, readEntries };
