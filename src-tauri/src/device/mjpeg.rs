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
    [&b"mp4v"[..], b"jpeg", b"mjpa", b"mjpb"].iter().any(|g| contains(m, g))
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

    #[test]
    fn accepts_mjpeg_rejects_h264() {
        assert!(is_mjpeg_mp4(&mp4(b"....mp4v....", b"avc1 in jpeg data is fine")));
        assert!(!is_mjpeg_mp4(&mp4(b"....avc1....", b"")));
        assert!(!is_mjpeg_mp4(&mp4(b"....mp4v..hvc1", b"")));
        assert!(!is_mjpeg_mp4(b"not an mp4 file at all"));
        // mp4v only in mdat, not in moov: rejected
        assert!(!is_mjpeg_mp4(&mp4(b"........", b"mp4v")));
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
