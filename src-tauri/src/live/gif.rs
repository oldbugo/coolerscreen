//! A small GIF decoder for the slideshow widget (animated GIFs): global/local palettes, transparency, interlacing, LZW,
//! disposal methods 0-3. Frames come out composited on the full logical screen, as premultiplied RGBA.
pub struct Frame {
    /// Premultiplied RGBA, `w * h * 4`.
    pub rgba: Vec<u8>,
    pub delay_ms: u32,
}

pub struct Gif {
    pub w: usize,
    pub h: usize,
    pub frames: Vec<Frame>,
}

/// Most frame data kept in memory for one GIF.
const MAX_BYTES: usize = 160 * 1024 * 1024;

pub fn is_gif(b: &[u8]) -> bool {
    b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a")
}

fn lzw(min: u8, data: &[u8], out_len: usize) -> Vec<u8> {
    let clear = 1u32 << min;
    let end = clear + 1;
    let mut size = min as u32 + 1;
    let mut next = end + 1;
    // prefix/suffix tables
    let mut prefix = vec![0u16; 4096];
    let mut suffix = vec![0u8; 4096];
    let mut stack: Vec<u8> = Vec::new();
    let mut out = Vec::with_capacity(out_len);
    let (mut bits, mut nbits) = (0u32, 0u32);
    let mut prev: Option<u32> = None;
    let mut first = 0u8;
    for &byte in data {
        bits |= (byte as u32) << nbits;
        nbits += 8;
        while nbits >= size {
            let code = bits & ((1 << size) - 1);
            bits >>= size;
            nbits -= size;
            if code == clear {
                size = min as u32 + 1;
                next = end + 1;
                prev = None;
                continue;
            }
            if code == end {
                return out;
            }
            let Some(p) = prev else {
                if code >= clear {
                    return out;
                }
                out.push(code as u8);
                first = code as u8;
                prev = Some(code);
                continue;
            };
            let mut c = code;
            if code >= next {
                // KwKwK case
                stack.push(first);
                c = p;
            }
            while c >= clear {
                stack.push(suffix[c as usize]);
                c = prefix[c as usize] as u32;
            }
            first = c as u8;
            stack.push(first);
            while let Some(b) = stack.pop() {
                out.push(b);
            }
            if next < 4096 {
                prefix[next as usize] = p as u16;
                suffix[next as usize] = first;
                next += 1;
                if next == (1 << size) && size < 12 {
                    size += 1;
                }
            }
            prev = Some(code);
            if out.len() >= out_len {
                return out;
            }
        }
    }
    out
}

pub fn decode(b: &[u8]) -> Result<Gif, String> {
    if !is_gif(b) || b.len() < 13 {
        return Err("not a GIF".into());
    }
    let rd16 = |i: usize| -> usize { b.get(i).copied().unwrap_or(0) as usize | (b.get(i + 1).copied().unwrap_or(0) as usize) << 8 };
    let (w, h) = (rd16(6), rd16(8));
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return Err("bad GIF size".into());
    }
    let packed = b[10];
    let mut pos = 13;
    let mut global: Vec<[u8; 3]> = Vec::new();
    if packed & 0x80 != 0 {
        let n = 2usize << (packed & 7);
        for i in 0..n {
            let o = pos + i * 3;
            global.push([*b.get(o).ok_or("truncated")?, b[o + 1], b[o + 2]]);
        }
        pos += n * 3;
    }
    let mut canvas = vec![0u8; w * h * 4]; // straight RGBA
    let mut frames: Vec<Frame> = Vec::new();
    let (mut delay, mut transparent, mut disposal) = (10u32, None::<u8>, 0u8);
    while pos < b.len() {
        match b[pos] {
            0x21 => {
                let label = *b.get(pos + 1).ok_or("truncated")?;
                pos += 2;
                if label == 0xF9 && b.get(pos) == Some(&4) {
                    let p = b[pos + 1];
                    disposal = (p >> 2) & 7;
                    delay = (rd16(pos + 2) as u32 * 10).max(20); // browsers treat 0-1 cs as 100 ms; 20 ms floor is kinder
                    if rd16(pos + 2) <= 1 {
                        delay = 100;
                    }
                    transparent = if p & 1 != 0 { Some(b[pos + 4]) } else { None };
                }
                // skip sub-blocks
                loop {
                    let n = *b.get(pos).ok_or("truncated")? as usize;
                    pos += 1 + n;
                    if n == 0 {
                        break;
                    }
                }
            }
            0x2C => {
                let (fx, fy, fw, fh) = (rd16(pos + 1), rd16(pos + 3), rd16(pos + 5), rd16(pos + 7));
                let fp = *b.get(pos + 9).ok_or("truncated")?;
                pos += 10;
                let mut local: Option<Vec<[u8; 3]>> = None;
                if fp & 0x80 != 0 {
                    let n = 2usize << (fp & 7);
                    let mut t = Vec::new();
                    for i in 0..n {
                        let o = pos + i * 3;
                        t.push([*b.get(o).ok_or("truncated")?, b[o + 1], b[o + 2]]);
                    }
                    pos += n * 3;
                    local = Some(t);
                }
                let min = *b.get(pos).ok_or("truncated")?;
                pos += 1;
                let mut data = Vec::new();
                loop {
                    let n = *b.get(pos).ok_or("truncated")? as usize;
                    pos += 1;
                    if n == 0 {
                        break;
                    }
                    data.extend_from_slice(b.get(pos..pos + n).ok_or("truncated")?);
                    pos += n;
                }
                let idx = lzw(min.clamp(2, 11), &data, fw * fh);
                let pal = local.as_ref().unwrap_or(&global);
                let before = if disposal == 3 { Some(canvas.clone()) } else { None };
                // row order
                let rows: Vec<usize> = if fp & 0x40 != 0 {
                    let mut r = Vec::new();
                    for (start, step) in [(0, 8), (4, 8), (2, 4), (1, 2)] {
                        let mut y = start;
                        while y < fh {
                            r.push(y);
                            y += step;
                        }
                    }
                    r
                } else {
                    (0..fh).collect()
                };
                for (src_row, &dy) in rows.iter().enumerate() {
                    for x in 0..fw {
                        let i = src_row * fw + x;
                        let Some(&ci) = idx.get(i) else { continue };
                        if Some(ci) == transparent {
                            continue;
                        }
                        let (cx, cy) = (fx + x, fy + dy);
                        if cx >= w || cy >= h {
                            continue;
                        }
                        let c = pal.get(ci as usize).copied().unwrap_or([0, 0, 0]);
                        let o = (cy * w + cx) * 4;
                        canvas[o..o + 4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                    }
                }
                // emit the composited frame, premultiplied
                let mut out = canvas.clone();
                for p in out.chunks_exact_mut(4) {
                    let a = p[3] as u32;
                    if a != 255 {
                        for c in 0..3 {
                            p[c] = (p[c] as u32 * a / 255) as u8;
                        }
                    }
                }
                frames.push(Frame { rgba: out, delay_ms: delay });
                // disposal for the NEXT frame
                match disposal {
                    2 => {
                        for y in fy..(fy + fh).min(h) {
                            for x in fx..(fx + fw).min(w) {
                                canvas[(y * w + x) * 4..][..4].fill(0);
                            }
                        }
                    }
                    3 => {
                        if let Some(bf) = before {
                            canvas = bf;
                        }
                    }
                    _ => {}
                }
                disposal = 0;
                transparent = None;
                if frames.len() * w * h * 4 > MAX_BYTES || frames.len() >= 600 {
                    break;
                }
            }
            0x3B => break,
            _ => pos += 1,
        }
    }
    if frames.is_empty() {
        return Err("no frames".into());
    }
    Ok(Gif { w, h, frames })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// LZW codes (min code size 2) packed LSB-first with the code size growing the way a decoder expects.
    pub(crate) fn pack(codes: &[u32]) -> Vec<u8> {
        let (mut acc, mut n, mut bytes) = (0u32, 0u32, Vec::new());
        let (mut size, mut next) = (3u32, 6u32);
        for (i, c) in codes.iter().enumerate() {
            acc |= c << n;
            n += size;
            while n >= 8 {
                bytes.push((acc & 0xff) as u8);
                acc >>= 8;
                n -= 8;
            }
            if i > 1 && *c != 5 {
                next += 1;
                if next == (1 << size) {
                    size += 1;
                }
            }
        }
        if n > 0 {
            bytes.push(acc as u8);
        }
        bytes
    }

    /// A hand-made 2x2 GIF with two frames (red/green/blue/white, then all blue), delays 10 cs and 20 cs.
    pub(crate) fn tiny() -> Vec<u8> {
        let mut g = b"GIF89a".to_vec();
        g.extend([2, 0, 2, 0, 0xF1, 0, 0]);
        g.extend([255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]);
        for (delay, codes) in [(10u8, vec![4u32, 0, 1, 2, 3, 5]), (20, vec![4, 2, 2, 2, 2, 5])] {
            g.extend([0x21, 0xF9, 4, 0, delay, 0, 0, 0]);
            g.extend([0x2C, 0, 0, 0, 0, 2, 0, 2, 0, 0, 2]);
            let bytes = pack(&codes);
            g.push(bytes.len() as u8);
            g.extend(&bytes);
            g.push(0);
        }
        g.push(0x3B);
        g
    }

    #[test]
    fn decodes_frames_colours_and_delays() {
        let g = decode(&tiny()).unwrap();
        assert_eq!((g.w, g.h, g.frames.len()), (2, 2, 2));
        assert_eq!(&g.frames[0].rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&g.frames[0].rgba[4..8], &[0, 255, 0, 255]);
        assert_eq!(&g.frames[0].rgba[12..16], &[255, 255, 255, 255]);
        assert_eq!(&g.frames[1].rgba[8..12], &[0, 0, 255, 255]);
        assert_eq!((g.frames[0].delay_ms, g.frames[1].delay_ms), (100, 200));
        assert!(decode(b"nope").is_err());
        assert!(is_gif(&tiny()));
    }
}
