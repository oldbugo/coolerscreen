//! Reader for the vendor's ".ht" theme files (and plain .zip files), port of 1.2 `src/htTheme.js`.
//! A .ht is a ZIP with PKWARE "ZipCrypto" encryption (password "ht") containing temp.zip, which contains
//! <id>.jpg (preview), a .json (label, size, bgMedia) and the video mp4.
use super::error::{CoreError, CoreResult};
use serde_json::Value;
use std::path::{Path, PathBuf};

const PASSWORD: &[u8] = b"ht";
const VIDEO_EXT: [&str; 7] = [".mp4", ".mov", ".mkv", ".webm", ".avi", ".m4v", ".gif"];
const IMAGE_EXT: [&str; 5] = [".jpg", ".jpeg", ".png", ".bmp", ".webp"];

fn crc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (n, slot) in t.iter_mut().enumerate() {
            let mut c = n as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
            }
            *slot = c;
        }
        t
    })
}

fn crc_step(crc: u32, b: u8) -> u32 {
    crc_table()[((crc ^ b as u32) & 0xff) as usize] ^ (crc >> 8)
}

pub fn crc32(buf: &[u8]) -> u32 {
    !buf.iter().fold(0xffff_ffffu32, |c, &b| crc_step(c, b))
}

/// ZipCrypto decryption; the first 12 bytes of the result are the random header.
pub fn zip_crypto_decrypt(data: &[u8], password: &[u8]) -> Vec<u8> {
    let (mut k0, mut k1, mut k2) = (305_419_896u32, 591_751_049u32, 878_082_192u32);
    let update = |c: u8, k0: &mut u32, k1: &mut u32, k2: &mut u32| {
        *k0 = crc_step(*k0, c);
        *k1 = k1.wrapping_add(*k0 & 0xff);
        *k1 = k1.wrapping_mul(134_775_813).wrapping_add(1);
        *k2 = crc_step(*k2, (*k1 >> 24) as u8);
    };
    for &ch in password {
        update(ch, &mut k0, &mut k1, &mut k2);
    }
    let mut out = Vec::with_capacity(data.len());
    for &d in data {
        let t = (k2 | 2) & 0xffff;
        let c = d ^ (((t * (t ^ 1)) >> 8) & 0xff) as u8;
        out.push(c);
        update(c, &mut k0, &mut k1, &mut k2);
    }
    out
}

struct Entry {
    name: String,
    flags: u16,
    method: u16,
    time: u16,
    crc: u32,
    csize: usize,
    usize_: usize,
    offset: usize,
}

fn u16le(b: &[u8], i: usize) -> CoreResult<u16> {
    b.get(i..i + 2).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| CoreError::new("Corrupt zip"))
}
fn u32le(b: &[u8], i: usize) -> CoreResult<u32> {
    b.get(i..i + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| CoreError::new("Corrupt zip"))
}

fn read_entries(buf: &[u8]) -> CoreResult<Vec<Entry>> {
    if buf.len() < 22 {
        return Err(CoreError::new("Not a zip file"));
    }
    let lo = buf.len().saturating_sub(65557);
    let eocd = (lo..=buf.len() - 22).rev().find(|&i| u32le(buf, i).ok() == Some(0x0605_4b50)).ok_or_else(|| CoreError::new("Not a zip file"))?;
    let count = u16le(buf, eocd + 10)? as usize;
    let mut p = u32le(buf, eocd + 16)? as usize;
    let mut out = vec![];
    for _ in 0..count {
        if u32le(buf, p)? != 0x0201_4b50 {
            return Err(CoreError::new("Corrupt zip directory"));
        }
        let (nlen, elen, clen) = (u16le(buf, p + 28)? as usize, u16le(buf, p + 30)? as usize, u16le(buf, p + 32)? as usize);
        let name = String::from_utf8_lossy(buf.get(p + 46..p + 46 + nlen).ok_or_else(|| CoreError::new("Corrupt zip"))?).replace('\\', "/");
        out.push(Entry {
            name,
            flags: u16le(buf, p + 8)?,
            method: u16le(buf, p + 10)?,
            time: u16le(buf, p + 12)?,
            crc: u32le(buf, p + 16)?,
            csize: u32le(buf, p + 20)? as usize,
            usize_: u32le(buf, p + 24)? as usize,
            offset: u32le(buf, p + 42)? as usize,
        });
        p += 46 + nlen + elen + clen;
    }
    Ok(out)
}

fn extract_entry(buf: &[u8], e: &Entry) -> CoreResult<Vec<u8>> {
    let o = e.offset;
    if u32le(buf, o)? != 0x0403_4b50 {
        return Err(CoreError::new(format!("Corrupt zip entry: {}", e.name)));
    }
    let start = o + 30 + u16le(buf, o + 26)? as usize + u16le(buf, o + 28)? as usize;
    let mut data: Vec<u8> = buf.get(start..start + e.csize).ok_or_else(|| CoreError::new("Corrupt zip entry"))?.to_vec();
    if e.flags & 1 != 0 {
        if e.flags & 0x40 != 0 {
            return Err(CoreError::new("Unsupported zip encryption"));
        }
        let dec = zip_crypto_decrypt(&data, PASSWORD);
        let check = if e.flags & 8 != 0 { (e.time >> 8) as u8 } else { (e.crc >> 24) as u8 };
        if dec.len() < 12 || dec[11] != check {
            return Err(CoreError::new("Wrong password for theme file"));
        }
        data = dec[12..].to_vec();
    }
    match e.method {
        0 => Ok(data),
        8 => miniz_oxide::inflate::decompress_to_vec(&data).map_err(|_| CoreError::new("Corrupt compressed data in zip")),
        m => Err(CoreError::new(format!("Unsupported zip compression method {m}"))),
    }
}

struct Media {
    ext: String,
    size: usize,
    data: Vec<u8>,
}

struct Acc {
    json: Option<Value>,
    videos: Vec<Media>,
    images: Vec<Media>,
}

fn ext_of(name: &str) -> String {
    let base = name.rsplit('/').next().unwrap_or(name);
    match base.rfind('.') {
        Some(i) if i > 0 => base[i..].to_lowercase(),
        _ => String::new(),
    }
}

fn collect(buf: &[u8], acc: &mut Acc, depth: u32) -> CoreResult<()> {
    for e in read_entries(buf)? {
        if e.name.ends_with('/') {
            continue;
        }
        let ext = ext_of(&e.name);
        if ext == ".zip" && depth < 3 {
            collect(&extract_entry(buf, &e)?, acc, depth + 1)?;
        } else if ext == ".json" {
            if acc.json.is_none() {
                if let Ok(d) = extract_entry(buf, &e) {
                    let s = String::from_utf8_lossy(&d);
                    acc.json = serde_json::from_str(s.trim_start_matches('\u{feff}')).ok();
                }
            }
        } else if VIDEO_EXT.contains(&ext.as_str()) {
            acc.videos.push(Media { ext, size: e.usize_, data: extract_entry(buf, &e)? });
        } else if IMAGE_EXT.contains(&ext.as_str()) {
            acc.images.push(Media { ext, size: e.usize_, data: extract_entry(buf, &e)? });
        }
    }
    Ok(())
}

fn label_of(json: &Option<Value>) -> String {
    let Some(j) = json else { return String::new() };
    let l = ["label", "name", "title"].iter().find_map(|k| j.get(*k).filter(|v| super::js::truthy(Some(v))));
    match l {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Object(m)) => {
            let pick = ["zh", "cn", "en"].iter().find_map(|k| m.get(*k).filter(|v| super::js::truthy(Some(v)))).or_else(|| m.values().next());
            match pick {
                Some(Value::String(s)) => s.trim().to_string(),
                Some(v) if super::js::truthy(Some(v)) => v.to_string().trim().to_string(),
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
}

/// True if the file looks like a theme/zip container (by extension or PK header).
pub fn is_theme_file(file: &Path) -> bool {
    let ext = file.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
    if ext == "ht" || ext == "zip" {
        return true;
    }
    use std::io::Read;
    let mut b = [0u8; 4];
    std::fs::File::open(file).and_then(|mut f| f.read_exact(&mut b)).is_ok() && u32::from_le_bytes(b) == 0x0403_4b50
}

pub struct Extracted {
    pub path: PathBuf,
    /// "video" | "image"
    pub kind: &'static str,
    pub name: String,
    pub preview_jpg: Option<Vec<u8>>,
}

/// Extract the media (biggest video, else biggest image) from a .ht/.zip into `out_dir` as theme.<ext>.
pub fn extract_theme(file: &Path, out_dir: &Path) -> CoreResult<Extracted> {
    let buf = std::fs::read(file)?;
    if buf.len() <= 4 || u32le(&buf, 0)? != 0x0403_4b50 {
        return Err(CoreError::new("Not a theme or zip file"));
    }
    let mut acc = Acc { json: None, videos: vec![], images: vec![] };
    collect(&buf, &mut acc, 0)?;
    let stem = file.file_stem().and_then(|s| s.to_str()).unwrap_or("theme");
    let fallback = stem.split('_').next().unwrap_or(stem).to_string();
    let label = label_of(&acc.json);
    let name = if label.is_empty() { fallback } else { label };
    std::fs::create_dir_all(out_dir)?;
    let biggest = |l: &mut Vec<Media>| -> Option<Media> {
        let i = (0..l.len()).max_by(|&a, &b| l[a].size.cmp(&l[b].size).then(b.cmp(&a)))?;
        Some(l.swap_remove(i))
    };
    if let Some(v) = biggest(&mut acc.videos) {
        let dest = out_dir.join(format!("theme{}", v.ext));
        std::fs::write(&dest, &v.data)?;
        let jpg = biggest(&mut acc.images).map(|m| m.data);
        return Ok(Extracted { path: dest, kind: "video", name, preview_jpg: jpg });
    }
    if let Some(img) = biggest(&mut acc.images) {
        let dest = out_dir.join(format!("theme{}", img.ext));
        std::fs::write(&dest, &img.data)?;
        return Ok(Extracted { path: dest, kind: "image", name, preview_jpg: None });
    }
    Err(CoreError::new("This theme/zip file contains no video or image I can use."))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal stored (method 0) zip writer for the tests, optionally ZipCrypto-encrypted with "ht".
    fn zip(files: &[(&str, &[u8])], encrypt: bool) -> Vec<u8> {
        let mut out = vec![];
        let mut central = vec![];
        for (name, data) in files {
            let crc = crc32(data);
            let mut body = data.to_vec();
            let mut flags = 0u16;
            if encrypt {
                flags = 1;
                // Encrypt: header of 12 bytes whose last byte is the crc check byte.
                let mut header = [0u8; 12];
                header[11] = (crc >> 24) as u8;
                let plain: Vec<u8> = header.iter().chain(data.iter()).copied().collect();
                body = encrypt_zc(&plain, PASSWORD);
            }
            let off = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0]);
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(body.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&body);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0]);
            central.extend_from_slice(&flags.to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(body.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    fn encrypt_zc(plain: &[u8], password: &[u8]) -> Vec<u8> {
        let (mut k0, mut k1, mut k2) = (305_419_896u32, 591_751_049u32, 878_082_192u32);
        let up = |c: u8, k0: &mut u32, k1: &mut u32, k2: &mut u32| {
            *k0 = crc_step(*k0, c);
            *k1 = k1.wrapping_add(*k0 & 0xff).wrapping_mul(134_775_813).wrapping_add(1);
            *k2 = crc_step(*k2, (*k1 >> 24) as u8);
        };
        for &c in password {
            up(c, &mut k0, &mut k1, &mut k2);
        }
        plain
            .iter()
            .map(|&p| {
                let t = (k2 | 2) & 0xffff;
                let c = p ^ (((t * (t ^ 1)) >> 8) & 0xff) as u8;
                up(p, &mut k0, &mut k1, &mut k2);
                c
            })
            .collect()
    }

    #[test]
    fn extracts_nested_encrypted_theme() {
        let inner = zip(&[("123.jpg", b"jpgdata"), ("info.json", "{\"label\":{\"zh\":\"粉蓝梦境\"}}".as_bytes()), ("v.mp4", b"videodata!")], false);
        let ht = zip(&[("temp.zip", &inner)], true);
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("abc_1.ht");
        std::fs::write(&f, &ht).unwrap();
        assert!(is_theme_file(&f));
        let r = extract_theme(&f, &d.path().join("out")).unwrap();
        assert_eq!(r.kind, "video");
        assert_eq!(r.name, "粉蓝梦境");
        assert_eq!(std::fs::read(&r.path).unwrap(), b"videodata!");
        assert_eq!(r.preview_jpg.unwrap(), b"jpgdata");
    }

    #[test]
    fn zip_with_only_image_uses_file_stem() {
        let z = zip(&[("pic.png", b"png")], false);
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("Holiday_2.zip");
        std::fs::write(&f, &z).unwrap();
        let r = extract_theme(&f, d.path()).unwrap();
        assert_eq!((r.kind, r.name.as_str()), ("image", "Holiday"));
    }
}
