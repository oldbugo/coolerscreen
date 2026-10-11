//! MJPEG-only guard, an exact port of isMjpegMp4 in src/device.js.
//! The screen's player only decodes MJPEG; an H.264/HEVC mp4 freezes it on every boot.

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// True for an ISO mp4 whose video sample entry is MJPEG ('mp4v' with JPEG object type, or
/// 'jpeg'/'mjpa'/'mjpb'), and that carries no H.264/HEVC/AV1/VP9 sample entries. Only the
/// top-level 'moov' box is inspected (JPEG data could contain any bytes).
pub fn is_mjpeg_mp4(buf: &[u8]) -> bool {
    if buf.len() < 12 || &buf[4..8] != b"ftyp" {
        return false;
    }
    let len = buf.len() as u64;
    let mut moov: Option<&[u8]> = None;
    let mut off: u64 = 0;
    while off + 8 <= len {
        let o = off as usize;
        let mut size = u32::from_be_bytes(buf[o..o + 4].try_into().unwrap()) as u64;
        let typ = &buf[o + 4..o + 8];
        if size == 1 && off + 16 <= len {
            size = u64::from_be_bytes(buf[o + 8..o + 16].try_into().unwrap());
        } else if size == 0 {
            size = len - off;
        }
        if size < 8 {
            return false;
        }
        if typ == b"moov" {
            let end = len.min(off.saturating_add(size)) as usize;
            moov = Some(&buf[o..end]);
            break;
        }
        off = match off.checked_add(size) {
            Some(v) => v,
            None => break,
        };
    }
    let Some(m) = moov else { return false };
    for bad in [&b"avc1"[..], b"avc3", b"hvc1", b"hev1", b"av01", b"vp09"] {
        if contains(m, bad) {
            return false;
        }
    }
    if [&b"jpeg"[..], b"mjpa", b"mjpb"].iter().any(|g| contains(m, g)) {
        return true;
    }
    // 'mp4v' is also MPEG-4 Part 2 video (which would freeze the player): accept it only when every mp4v sample
    // entry declares JPEG (objectTypeIndication 0x6C) in its esds, as ffmpeg's mjpeg-in-mp4 does.
    let entries = mp4v_object_types(m);
    !entries.is_empty() && entries.iter().all(|t| *t == Some(0x6C))
}

/// objectTypeIndication of every 'mp4v' sample entry in `moov` (None when its esds is missing or unreadable).
fn mp4v_object_types(moov: &[u8]) -> Vec<Option<u8>> {
    let mut out = vec![];
    let mut i = 4;
    while i + 4 <= moov.len() {
        if &moov[i..i + 4] == b"mp4v" {
            let start = i - 4;
            let size = u32::from_be_bytes(moov[start..i].try_into().unwrap()) as usize;
            // A sample entry is at least 86 bytes (8 header + 78 visual fields); anything else is not a box header.
            if size >= 86 && start + size <= moov.len() {
                out.push(esds_object_type(&moov[start..start + size]));
                i = start + size;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn esds_object_type(entry: &[u8]) -> Option<u8> {
    let at = entry.windows(4).position(|w| w == b"esds")?;
    let mut p = at + 4 + 4; // type, version + flags
    let desc_len = |p: &mut usize| -> Option<usize> {
        let mut n = 0usize;
        for _ in 0..4 {
            let b = *entry.get(*p)?;
            *p += 1;
            n = (n << 7) | (b & 0x7f) as usize;
            if b & 0x80 == 0 {
                return Some(n);
            }
        }
        Some(n)
    };
    if *entry.get(p)? != 0x03 {
        return None;
    }
    p += 1;
    desc_len(&mut p)?;
    p += 2; // ES_ID
    let flags = *entry.get(p)?;
    p += 1;
    if flags & 0x80 != 0 {
        p += 2;
    }
    if flags & 0x40 != 0 {
        p += 1 + *entry.get(p)? as usize;
    }
    if flags & 0x20 != 0 {
        p += 2;
    }
    if *entry.get(p)? != 0x04 {
        return None;
    }
    p += 1;
    desc_len(&mut p)?;
    entry.get(p).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fbox(t: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(body);
        v
    }

    fn mp4(entry: &[u8], mdat: &[u8]) -> Vec<u8> {
        let mut v = fbox(b"ftyp", b"isom\0\0\x02\0");
        v.extend(fbox(b"mdat", mdat));
        v.extend(fbox(b"moov", &fbox(b"trak", entry)));
        v
    }

    /// An 'mp4v' sample entry (78 bytes of visual fields) holding an esds with this objectTypeIndication.
    fn mp4v_entry(oti: u8) -> Vec<u8> {
        // esds: version/flags, ES_Descriptor(0x03, len, ES_ID, flags=0), DecoderConfig(0x04, len, OTI, ...)
        let esds = fbox(b"esds", &[0, 0, 0, 0, 0x03, 0x80, 0x80, 0x80, 0x19, 0, 1, 0, 0x04, 0x80, 0x80, 0x80, 0x11, oti, 0x11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        let mut body = vec![0u8; 78];
        body.extend(esds);
        fbox(b"mp4v", &body)
    }

    #[test]
    fn accepts_mjpeg_rejects_h264() {
        assert!(is_mjpeg_mp4(&mp4(&mp4v_entry(0x6C), b"avc1 in jpeg data is fine")));
        assert!(is_mjpeg_mp4(&mp4(b"....jpeg....", b"")));
        assert!(!is_mjpeg_mp4(&mp4(b"....avc1....", b"")));
        assert!(!is_mjpeg_mp4(&mp4(&[mp4v_entry(0x6C), b"hvc1".to_vec()].concat(), b"")));
        assert!(!is_mjpeg_mp4(b"not an mp4 file at all"));
        // mp4v only in mdat, not in moov: rejected
        assert!(!is_mjpeg_mp4(&mp4(b"........", b"mp4v")));
    }

    #[test]
    fn rejects_mpeg4_part2_in_mp4v() {
        assert!(!is_mjpeg_mp4(&mp4(&mp4v_entry(0x20), b"")), "MPEG-4 Part 2 visual");
        assert!(!is_mjpeg_mp4(&mp4(b"....mp4v....", b"")), "mp4v without an esds");
        assert!(!is_mjpeg_mp4(&mp4(&[mp4v_entry(0x6C), mp4v_entry(0x20)].concat(), b"")), "one bad track is enough");
    }

    /// Real files: CS_MJPEG_OK = folder whose */versions/*/device.mp4 must all pass (the library);
    /// CS_FFMPEG = ffmpeg used to make an MPEG-4 Part 2 .mp4 that must be refused.
    #[test]
    #[ignore = "needs CS_MJPEG_OK (library folder) and CS_FFMPEG"]
    fn real_files() {
        let lib = std::path::PathBuf::from(std::env::var("CS_MJPEG_OK").unwrap());
        let mut n = 0;
        for o in std::fs::read_dir(&lib).unwrap().flatten() {
            let Ok(vs) = std::fs::read_dir(o.path().join("versions")) else { continue };
            for v in vs.flatten() {
                let f = v.path().join("device.mp4");
                if let Ok(d) = std::fs::read(&f) {
                    assert!(is_mjpeg_mp4(&d), "{}", f.display());
                    n += 1;
                }
            }
        }
        assert!(n > 0);
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("mpeg4.mp4");
        let st = std::process::Command::new(std::env::var("CS_FFMPEG").unwrap())
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=18", "-t", "1", "-c:v", "mpeg4", "-y"])
            .arg(&out)
            .status()
            .unwrap();
        assert!(st.success());
        assert!(!is_mjpeg_mp4(&std::fs::read(&out).unwrap()), "MPEG-4 Part 2 must be refused");
        let mj = dir.path().join("mjpeg.mp4");
        std::process::Command::new(std::env::var("CS_FFMPEG").unwrap())
            .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc=size=320x240:rate=18", "-t", "1", "-c:v", "mjpeg", "-y"])
            .arg(&mj)
            .status()
            .unwrap();
        assert!(is_mjpeg_mp4(&std::fs::read(&mj).unwrap()), "ffmpeg mjpeg must pass");
        eprintln!("{n} library versions passed");
    }
}

/// Fixtures for other modules' tests.
#[cfg(test)]
pub mod tests_support {
    fn fbox(t: &[u8], body: &[u8]) -> Vec<u8> {
        let mut v = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(t);
        v.extend_from_slice(body);
        v
    }

    /// A tiny file the MJPEG guard accepts ('jpeg' sample entry in a top-level moov).
    pub fn tiny_mjpeg_mp4() -> Vec<u8> {
        let mut v = fbox(b"ftyp", b"isom\0\0\x02\0");
        v.extend(fbox(b"mdat", &[0u8; 64]));
        v.extend(fbox(b"moov", &fbox(b"trak", b"....jpeg....")));
        v
    }
}
