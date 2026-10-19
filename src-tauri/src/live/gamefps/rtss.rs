//! RivaTuner Statistics Server shared memory (`RTSSSharedMemoryV2`), read-only. We only open the mapping the user's RTSS
//! already created (FILE_MAP_READ); we never start, install, inject or hook anything.
//!
//! Layout (little-endian). The official header is not public; CONFIRMED = seen in a second-hand source
//! (pozitronik/steelclock-go PR 52 reader: header offsets, 'RTSS' constant 0x52545353, min version 0x00020000,
//! staleness rule, fps = frames / (time1 - time0), skip time0 == 0), UNCONFIRMED = from the task brief / memory of RTSSSharedMemory.h:
//!   header: +0 dwSignature (CONFIRMED; both byte orders accepted here), +4 dwVersion (CONFIRMED), +8 dwAppEntrySize (CONFIRMED),
//!           +12 dwAppArrOffset (CONFIRMED), +16 dwAppArrSize (CONFIRMED; steelclock says up to 256 entries)
//!   app entry (the entry size from the header is always used as the stride):
//!           +0 dwProcessID, +4 szName[260] (char path), +264 dwFlags, +268 dwTime0 (ms tick), +272 dwTime1 (ms tick),
//!           +276 dwFrames, +280 dwFrameTime (microseconds: UNCONFIRMED, only used for the frame-time display).
//!   Offsets +0..+284 of the entry: the field order is UNCONFIRMED against the real header (the steelclock page names the fields
//!   but not their offsets). The later stat fields (dwStat*, frame-time buffer) are deliberately not read.

#[derive(Clone, Debug, PartialEq)]
pub struct RtssFrame {
    pub pid: u32,
    /// Program name without path and extension.
    pub name: String,
    pub fps: f32,
    pub frame_ms: f32,
}

const MAX_MAP: usize = 8 * 1024 * 1024;
const MIN_ENTRY: usize = 284;
const MAX_ENTRIES: usize = 4096;

/// The entry of the foreground program (or, when `foreground_pid` is None / has none, the freshest entry), None when RTSS is not running
/// or the entry is stale.
pub fn read(foreground_pid: Option<u32>) -> Option<RtssFrame> {
    let buf = map_copy()?;
    parse(&buf, foreground_pid, tick_ms())
}

#[cfg(windows)]
fn tick_ms() -> u32 {
    // GetTickCount (32-bit ms), the clock RTSS stamps its entries with
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount() }
}

#[cfg(not(windows))]
fn tick_ms() -> u32 {
    0
}

#[cfg(windows)]
fn map_copy() -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Memory::{MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ, MEMORY_BASIC_INFORMATION};
    let name: Vec<u16> = "RTSSSharedMemoryV2".encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let h = OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr());
        if h.is_null() {
            return None;
        }
        let view = MapViewOfFile(h, FILE_MAP_READ, 0, 0, 0);
        if view.Value.is_null() {
            CloseHandle(h);
            return None;
        }
        let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
        let n = VirtualQuery(view.Value, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
        let len = if n == 0 { 0 } else { mbi.RegionSize.min(MAX_MAP) };
        let copy = std::slice::from_raw_parts(view.Value as *const u8, len).to_vec();
        UnmapViewOfFile(view);
        CloseHandle(h);
        Some(copy)
    }
}

#[cfg(not(windows))]
fn map_copy() -> Option<Vec<u8>> {
    None
}

/// Pure parser for tests: `buf` = the mapped bytes, `tick_ms` = GetTickCount() now.
pub fn parse(buf: &[u8], foreground_pid: Option<u32>, tick_ms: u32) -> Option<RtssFrame> {
    let u32_at = |o: usize| -> Option<u32> { buf.get(o..o.checked_add(4)?).map(|s| u32::from_le_bytes(s.try_into().unwrap())) };
    let sig = u32_at(0)?;
    if sig != 0x5254_5353 && sig != 0x5353_5452 {
        return None;
    }
    if u32_at(4)? < 0x0002_0000 {
        return None;
    }
    let (esize, off, count) = (u32_at(8)? as usize, u32_at(12)? as usize, u32_at(16)? as usize);
    if esize < MIN_ENTRY || esize > 64 * 1024 || count == 0 {
        return None;
    }
    let count = count.min(MAX_ENTRIES);

    let mut best_fg: Option<(RtssFrame, u32)> = None;
    let mut best_any: Option<(RtssFrame, u32)> = None; // (frame, age in ms)
    for i in 0..count {
        let Some(o) = i.checked_mul(esize).and_then(|x| x.checked_add(off)) else { break };
        let Some(end) = o.checked_add(esize) else { break };
        if end > buf.len() {
            break;
        }
        let (Some(pid), Some(t0), Some(t1), Some(frames), Some(ft)) = (u32_at(o), u32_at(o + 268), u32_at(o + 272), u32_at(o + 276), u32_at(o + 280)) else { break };
        if pid == 0 || t0 == 0 {
            continue;
        }
        let dt = t1.wrapping_sub(t0);
        if dt == 0 || dt > 0x8000_0000 {
            continue;
        }
        let age = tick_ms.wrapping_sub(t1);
        if age > 2000u32.max(dt.saturating_mul(2)) {
            continue; // stale (also catches an entry stamped in the future: age wraps huge)
        }
        let fps = 1000.0 * frames as f32 / dt as f32;
        if !fps.is_finite() || fps <= 0.0 {
            continue;
        }
        let name_bytes = &buf[o + 4..o + 264];
        let n = name_bytes.iter().position(|c| *c == 0).unwrap_or(name_bytes.len());
        let path = String::from_utf8_lossy(&name_bytes[..n]).to_string();
        let file = path.rsplit(['\\', '/']).next().unwrap_or("");
        let name = std::path::Path::new(file).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let frame_ms = if ft > 0 { ft as f32 / 1000.0 } else { 1000.0 / fps };
        let fr = RtssFrame { pid, name, fps, frame_ms };
        if Some(pid) == foreground_pid && best_fg.as_ref().is_none_or(|(_, a)| age < *a) {
            best_fg = Some((fr.clone(), age));
        }
        if best_any.as_ref().is_none_or(|(_, a)| age < *a) {
            best_any = Some((fr, age));
        }
    }
    best_fg.or(best_any).map(|(f, _)| f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ESIZE: usize = 1352;

    fn entry(b: &mut [u8], o: usize, pid: u32, path: &str, t0: u32, t1: u32, frames: u32, ft: u32) {
        b[o..o + 4].copy_from_slice(&pid.to_le_bytes());
        b[o + 4..o + 4 + path.len()].copy_from_slice(path.as_bytes());
        b[o + 268..o + 272].copy_from_slice(&t0.to_le_bytes());
        b[o + 272..o + 276].copy_from_slice(&t1.to_le_bytes());
        b[o + 276..o + 280].copy_from_slice(&frames.to_le_bytes());
        b[o + 280..o + 284].copy_from_slice(&ft.to_le_bytes());
    }

    fn header(n: usize, esize: usize) -> Vec<u8> {
        let mut b = vec![0u8; 256 + n * esize];
        b[0..4].copy_from_slice(&0x5254_5353u32.to_le_bytes());
        b[4..8].copy_from_slice(&0x0002_0010u32.to_le_bytes());
        b[8..12].copy_from_slice(&(esize as u32).to_le_bytes());
        b[12..16].copy_from_slice(&256u32.to_le_bytes());
        b[16..20].copy_from_slice(&(n as u32).to_le_bytes());
        b
    }

    fn two() -> Vec<u8> {
        let mut b = header(3, ESIZE);
        entry(&mut b, 256, 100, r"C:\Games\Old\old.exe", 5000, 6000, 30, 33_000);
        entry(&mut b, 256 + ESIZE, 200, r"C:\Games\Wilds\MonsterHunterWilds.exe", 9000, 10_000, 59, 16_900);
        b
    }

    #[test]
    fn valid_entry_is_read() {
        let f = parse(&two(), Some(200), 10_300).unwrap();
        assert_eq!((f.pid, f.name.as_str()), (200, "MonsterHunterWilds"));
        assert!((f.fps - 59.0).abs() < 0.01);
        assert!((f.frame_ms - 16.9).abs() < 0.01);
    }

    #[test]
    fn freshest_when_no_foreground_match() {
        assert_eq!(parse(&two(), None, 10_300).unwrap().pid, 200);
        assert_eq!(parse(&two(), Some(999), 10_300).unwrap().pid, 200);
    }

    #[test]
    fn foreground_wins_over_fresher() {
        // pid 100 is still current (age 1500 <= 2000) but older than pid 200
        let f = parse(&two(), Some(100), 7_500).unwrap_or_else(|| panic!("none"));
        assert_eq!(f.pid, 100);
        assert!((f.fps - 30.0).abs() < 0.01);
        let g = parse(&two(), Some(100), 10_300).unwrap();
        assert_eq!(g.pid, 200, "stale foreground entry falls back to a current one");
    }

    #[test]
    fn stale_is_none() {
        assert_eq!(parse(&two(), Some(200), 20_000), None);
        // tick before the stamp (future entry) is not current
        assert_eq!(parse(&two(), Some(200), 1_000), None);
    }

    #[test]
    fn tick_wraparound() {
        let mut b = header(1, ESIZE);
        entry(&mut b, 256, 7, "x.exe", u32::MAX - 500, 500, 60, 0);
        let f = parse(&b, Some(7), 800).unwrap();
        assert!((f.fps - 60.0).abs() < 0.1, "dt = 1001 ms across the wrap: {}", f.fps);
    }

    #[test]
    fn bad_input_never_panics() {
        let ok = two();
        let mut sig = ok.clone();
        sig[0] = 0;
        assert_eq!(parse(&sig, None, 10_300), None);
        let mut ver = ok.clone();
        ver[4..8].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        assert_eq!(parse(&ver, None, 10_300), None);
        for n in [0, 3, 19, 20, 100, 256, 300, 256 + ESIZE - 1, 256 + ESIZE + 100] {
            let _ = parse(&ok[..n.min(ok.len())], Some(200), 10_300);
        }
        assert_eq!(parse(&ok[..256 + ESIZE - 1], Some(200), 10_300), None, "truncated first entry");
        // entry size too big / too small / absurd offset / zero entries
        for (esz, off, cnt) in [(10_000_000u32, 256u32, 3u32), (100, 256, 3), (ESIZE as u32, u32::MAX, 3), (ESIZE as u32, 256, 0), (ESIZE as u32, 256, u32::MAX), (u32::MAX, 256, 3)] {
            let mut b = ok.clone();
            b[8..12].copy_from_slice(&esz.to_le_bytes());
            b[12..16].copy_from_slice(&off.to_le_bytes());
            b[16..20].copy_from_slice(&cnt.to_le_bytes());
            let _ = parse(&b, Some(200), 10_300);
        }
        assert_eq!(parse(&header(0, ESIZE), None, 10_300), None);
        // unused entry (time0 == 0) and pid 0 are skipped
        let mut u = header(2, ESIZE);
        entry(&mut u, 256, 5, "a.exe", 0, 1000, 10, 0);
        entry(&mut u, 256 + ESIZE, 0, "b.exe", 1, 1000, 10, 0);
        assert_eq!(parse(&u, None, 1000), None);
    }

    #[test]
    fn unterminated_name_and_no_rtss() {
        let mut b = header(1, ESIZE);
        entry(&mut b, 256, 9, &"a".repeat(260), 100, 1100, 60, 0);
        assert_eq!(parse(&b, None, 1200).unwrap().name.len(), 260);
        // RTSS is not running on a test machine (or if it is, a frame/None, never a panic)
        let _ = read(Some(std::process::id()));
    }
}
