//! `cs2 live-test`: 3.0 spike for the screen's live (PC-streamed) mode, firmware > 2.8 protocol as read from the vendor app:
//!   1. enter live: framed key 0x11, no payload (`55 AA 07 00 11 17 01`); repeated every 1.5 s as a keepalive, between frames
//!   2. each frame: `u32LE jpegLen | JPEG | u16LE sum`, sum = (4 length bytes + all JPEG bytes) & 0xFFFF, no 55 AA wrapper
//!   3. JPEG: landscape 1680x756 (2400x1080 x 0.7) baseline, quality 100 stepping down by 2 until <= 260 KB
//!   4. end: `FF D9 FF D9` as its own transfer, then 0x14 [1,0] (play the loop clip)
//! Opt-in extras that change stored device state or are not needed: --set-sleep-delay (0x15 [0,10]), --set-brightness (0x03 [0x5F]).
//! Never sends 0x05/0x0C/0x23/0x01 or any file upload.
use cs_core::device::{self, frame};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const MAX_FRAMES: usize = 7200;
const KEEPALIVE: Duration = Duration::from_millis(1500);

/// 5x7 glyphs, one row per byte (bit 4 = leftmost column).
fn glyph(c: char) -> [u8; 7] {
    match c {
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [31, 2, 4, 2, 1, 17, 14],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 30, 1, 1, 17, 14],
        '6' => [6, 8, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 2, 12],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        _ => [0; 7],
    }
}

/// Landscape RGB canvas, scaled from the 2400x1080 design by `k`.
struct Canvas {
    w: usize,
    h: usize,
    k: f64,
    rgb: Vec<u8>,
}

impl Canvas {
    fn u(&self, v: usize) -> usize {
        ((v as f64 * self.k).round() as usize).max(1)
    }

    fn test_pattern(k: f64) -> Canvas {
        let w = ((2400.0 * k / 2.0).round() as usize) * 2;
        let h = ((1080.0 * k / 2.0).round() as usize) * 2;
        let mut c = Canvas { w, h, k, rgb: vec![0; w * h * 3] };
        let bar = c.u(140);
        for y in 0..h {
            for x in 0..w {
                // diagonal gradient with 8 colour bars along the bottom, so orientation and colour are easy to judge
                let p = if y >= h - bar {
                    const BARS: [[u8; 3]; 8] = [
                        [255, 255, 255], [255, 255, 0], [0, 255, 255], [0, 255, 0],
                        [255, 0, 255], [255, 0, 0], [0, 0, 255], [0, 0, 0],
                    ];
                    BARS[x * 8 / w]
                } else {
                    [(x * 255 / w) as u8, (y * 255 / h) as u8, 140]
                };
                c.set(x, y, p);
            }
        }
        // white border and a white square in the top-left corner (the "up" marker)
        let t = c.u(6);
        c.rect(0, 0, w, t, [255; 3]);
        c.rect(0, h - t, w, t, [255; 3]);
        c.rect(0, 0, t, h, [255; 3]);
        c.rect(w - t, 0, t, h, [255; 3]);
        let (m, s) = (c.u(30), c.u(120));
        c.rect(m, m, s, s, [255; 3]);
        c
    }

    #[inline]
    fn set(&mut self, x: usize, y: usize, p: [u8; 3]) {
        let i = (y * self.w + x) * 3;
        self.rgb[i..i + 3].copy_from_slice(&p);
    }

    fn rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, p: [u8; 3]) {
        for y in y0..(y0 + h).min(self.h) {
            for x in x0..(x0 + w).min(self.w) {
                self.set(x, y, p);
            }
        }
    }

    /// Text at design coordinates, `s` design px per glyph pixel, black on a white box.
    fn text(&mut self, x0: usize, y0: usize, s: usize, text: &str) {
        let (x0, y0, s) = (self.u(x0), self.u(y0), self.u(s));
        let w = text.chars().count() * 6 * s + 2 * s;
        self.rect(x0, y0, w, 9 * s, [255; 3]);
        for (k, ch) in text.chars().enumerate() {
            for (ry, row) in glyph(ch).iter().enumerate() {
                for rx in 0..5 {
                    if row & (16 >> rx) != 0 {
                        self.rect(x0 + s + (k * 6 + rx) * s, y0 + s + ry * s, s, s, [0, 0, 0]);
                    }
                }
            }
        }
    }

    /// (width, height, RGB), landscape as is, or rotated 90 deg clockwise (ffmpeg transpose=1: x -> row, y -> column h-1-y).
    fn pixels(&self, portrait: bool) -> (usize, usize, Vec<u8>) {
        if !portrait {
            return (self.w, self.h, self.rgb.clone());
        }
        let mut out = vec![0u8; self.rgb.len()];
        for y in 0..self.h {
            for x in 0..self.w {
                let s = (y * self.w + x) * 3;
                let d = (x * self.h + (self.h - 1 - y)) * 3;
                out[d..d + 3].copy_from_slice(&self.rgb[s..s + 3]);
            }
        }
        (self.h, self.w, out)
    }
}

/// Baseline JPEG at `start_q`, stepping down by 2 until it is at most `max_bytes` (the vendor's rule). Returns (jpeg, quality).
fn encode_fit(w: usize, h: usize, rgb: &[u8], start_q: u8, max_bytes: usize) -> Result<(Vec<u8>, u8), String> {
    let mut q = start_q;
    loop {
        let mut out = Vec::with_capacity(max_bytes);
        // 4:2:0 like the vendor's Chromium encoder and our ffmpeg clips; jpeg-encoder picks 4:4:4 at q >= 90.
        let mut enc = jpeg_encoder::Encoder::new(&mut out, q);
        enc.set_sampling_factor(jpeg_encoder::SamplingFactor::F_2_2);
        enc.encode(rgb, w as u16, h as u16, jpeg_encoder::ColorType::Rgb)
            .map_err(|e| format!("JPEG encode failed: {e}"))?;
        if out.len() <= max_bytes || q <= 2 {
            return Ok((out, q));
        }
        q = q.saturating_sub(2).max(1);
    }
}

/// Live-mode frame: u32LE length | JPEG | u16LE sum of (length bytes + JPEG bytes).
fn wrap_frame(jpeg: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(jpeg.len() + 6);
    v.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
    v.extend_from_slice(jpeg);
    let sum = v.iter().fold(0u32, |a, &b| a + b as u32) & 0xffff;
    v.extend_from_slice(&(sum as u16).to_le_bytes());
    v
}

fn arg_val<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn num<T: std::str::FromStr>(args: &[String], name: &str) -> Result<Option<T>, String> {
    arg_val(args, name).map(|v| v.parse().map_err(|_| format!("{name}: bad value '{v}'"))).transpose()
}

fn valid_jpeg(b: &[u8]) -> bool {
    b.len() > 4 && b[0] == 0xff && b[1] == 0xd8 && b[b.len() - 2] == 0xff && b[b.len() - 1] == 0xd9
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02X}")).collect::<Vec<_>>().join(" ")
}

pub fn live_test(args: &[String]) -> i32 {
    match run(args) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ERROR {e}");
            1
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    let has = |f: &str| args.iter().any(|a| a == f);
    let fps_cap: Option<f64> = num(args, "--fps")?;
    let seconds: Option<f64> = num(args, "--seconds")?;
    let frames: usize = match (num::<usize>(args, "--frames")?, seconds, fps_cap) {
        (Some(n), _, _) => n,
        (None, Some(s), Some(f)) => (s * f).round() as usize,
        (None, Some(_), None) => return Err("--seconds needs --fps (or give --frames)".into()),
        _ => 30,
    };
    let quality: u8 = num(args, "--quality")?.unwrap_or(100);
    let max_kb: usize = num(args, "--max-kb")?.unwrap_or(260);
    let scale: f64 = num(args, "--scale")?.unwrap_or(0.7);
    let portrait = match arg_val(args, "--orient").unwrap_or("landscape") {
        "landscape" => false,
        "portrait" => true,
        o => return Err(format!("--orient {o}: use landscape or portrait")),
    };
    if !(1..=MAX_FRAMES).contains(&frames) || !(1..=100).contains(&quality) || !(0.1..=1.0).contains(&scale) {
        return Err(format!("--frames 1..{MAX_FRAMES}, --quality 1..100, --scale 0.1..1.0"));
    }

    // 1. Build frames up front (so encoding time does not pollute write timing). A pool of distinct frames is cycled for long runs;
    //    the counter on screen then shows (frame index mod pool size).
    let t0 = Instant::now();
    let mut jpegs: Vec<Vec<u8>> = Vec::new();
    if let Some(path) = arg_val(args, "--image") {
        let b = std::fs::read(path).map_err(|e| format!("cannot read {path}: {e}"))?;
        if !valid_jpeg(&b) {
            return Err(format!("{path} is not a JPEG (needs FF D8 ... FF D9)"));
        }
        eprintln!("using {path}: {} bytes, sent {frames} times (no frame counter)", b.len());
        jpegs.push(b);
    } else {
        let base = Canvas::test_pattern(scale);
        let pool = if has("--dry-run") { 1 } else { frames.min(120) };
        let mut q = quality;
        let mut dims = (0, 0);
        for i in 0..pool {
            let mut c = Canvas { w: base.w, h: base.h, k: base.k, rgb: base.rgb.clone() };
            c.text(300, 250, 28, &format!("F{i:04}"));
            c.text(300, 580, 14, &format!("T{:07.3}S", i as f64 / 30.0));
            let (w, h, px) = c.pixels(portrait);
            dims = (w, h);
            let (j, used) = encode_fit(w, h, &px, q, max_kb * 1000)?;
            q = used; // later frames start from the quality that fitted
            jpegs.push(j);
        }
        eprintln!(
            "{pool} frame(s) {}x{} ({}), quality {q} (started {quality}, limit {max_kb} KB), built in {} ms{}",
            dims.0,
            dims.1,
            if portrait { "portrait" } else { "landscape" },
            t0.elapsed().as_millis(),
            if pool < frames { format!("; cycled, on-screen counter = index mod {pool}") } else { String::new() }
        );
    }
    if jpegs.iter().any(|j| !valid_jpeg(j)) {
        return Err("internal error: a frame is not a valid JPEG".into());
    }
    let wrapped: Vec<Vec<u8>> = jpegs.iter().map(|j| wrap_frame(j)).collect();

    if has("--dry-run") {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("work");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let p = dir.join(format!("live-test-{}.jpg", if portrait { "portrait" } else { "landscape" }));
        std::fs::write(&p, &jpegs[0]).map_err(|e| e.to_string())?;
        let w = &wrapped[0];
        println!("dry run: wrote {} ({} bytes); device not opened", p.display(), jpegs[0].len());
        println!("enter/keepalive frame (0x11): {}", hex(&frame::frame(0x11, &[])));
        println!("wire frame {} bytes: header {} | JPEG starts {} | JPEG ends {} | sum {}", w.len(), hex(&w[..4]), hex(&w[4..8]), hex(&w[w.len() - 8..w.len() - 2]), hex(&w[w.len() - 2..]));
        println!("end of stream: FF D9 FF D9 (own transfer), then 0x14 [1,0]: {}", hex(&frame::frame(0x14, &[1, 0])));
        return Ok(());
    }

    // 2. Open over WinUSB (never the serial fallback: a serial close would arm the 30 s watchdog).
    let mut scr = device::Screen::open_usb().map_err(|e| e.to_string())?;
    eprintln!("opened over {}", scr.transport);
    let (st0, b0, t0n) = (frame::STATUS_FRAMES.load(Ordering::Relaxed), frame::IN_BYTES.load(Ordering::Relaxed), frame::IN_TRANSFERS.load(Ordering::Relaxed));
    let mut replies = 0usize;
    let mut drain = |scr: &mut device::Screen, label: &str, wait: Duration| {
        while let Some(m) = scr.recovery_poll(wait) {
            replies += 1;
            println!("  IN 0x81 ({label}): {m}");
        }
    };
    // Baseline: is anything coming back before we send a thing? (status frames every ~2 s in normal mode)
    eprintln!("listening 2.5 s on IN 0x81 before sending anything");
    drain(&mut scr, "baseline", Duration::from_millis(2500));
    let report_in = |label: &str| {
        println!(
            "[{label}] IN so far: {} transfers, {} bytes, {} status (0x22) frames",
            frame::IN_TRANSFERS.load(Ordering::Relaxed) - t0n,
            frame::IN_BYTES.load(Ordering::Relaxed) - b0,
            frame::STATUS_FRAMES.load(Ordering::Relaxed) - st0
        )
    };
    report_in("baseline");

    let mut failed: Option<String> = None;
    let send = |scr: &mut device::Screen, b: &[u8]| -> Result<(), String> { scr.recovery_raw(b).map_err(|e| e.to_string()) };
    let mut setup = || -> Result<(), String> {
        if has("--set-sleep-delay") {
            eprintln!("0x15 [00 0A] (stored on the device)");
            send(&mut scr, &frame::frame(0x15, &[0x00, 0x0a]))?;
            drain(&mut scr, "0x15", Duration::from_millis(100));
        }
        if let Some(b) = num::<u8>(args, "--brightness")?.or(has("--set-brightness").then_some(0x5f)) {
            eprintln!("0x03 [{b}]");
            send(&mut scr, &frame::frame(0x03, &[b.min(100)]))?;
            drain(&mut scr, "0x03", Duration::from_millis(100));
        }
        eprintln!("enter live: 0x11");
        send(&mut scr, &frame::frame(0x11, &[]))?;
        drain(&mut scr, "reply to 0x11", Duration::from_millis(100));
        Ok(())
    };
    if let Err(e) = setup() {
        failed = Some(e);
    }

    let (mut sum_ms, mut worst_ms, mut bytes, mut sent) = (0.0f64, 0.0f64, 0usize, 0usize);
    let start = Instant::now();
    let mut last_keep = Instant::now();
    if failed.is_none() {
        eprintln!("sending {frames} frames{}", fps_cap.map(|f| format!(" at up to {f} fps")).unwrap_or_default());
        for i in 0..frames {
            if let Some(cap) = fps_cap {
                if let Some(w) = Duration::from_secs_f64(i as f64 / cap).checked_sub(start.elapsed()) {
                    std::thread::sleep(w);
                }
            }
            if last_keep.elapsed() >= KEEPALIVE {
                if let Err(e) = send(&mut scr, &frame::frame(0x11, &[])) {
                    failed = Some(e);
                    break;
                }
                last_keep = Instant::now();
            }
            let f = &wrapped[i % wrapped.len()];
            let t = Instant::now();
            if let Err(e) = send(&mut scr, f) {
                eprintln!("frame {i}: WRITE FAILED after {} ms: {e}", t.elapsed().as_millis());
                failed = Some(e);
                break;
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            sum_ms += ms;
            worst_ms = worst_ms.max(ms);
            bytes += f.len();
            sent += 1;
            if frames <= 60 || i % 10 == 0 || i + 1 == frames {
                let el = start.elapsed().as_secs_f64().max(0.001);
                println!("frame {i:5}: {:6.1} KB  write {ms:7.1} ms  | avg {:5.1} fps  {:7.0} KB/s", f.len() as f64 / 1024.0, sent as f64 / el, bytes as f64 / 1024.0 / el);
            }
            drain(&mut scr, "during stream", Duration::ZERO);
        }
    }
    let el = start.elapsed().as_secs_f64().max(0.001);

    // 3. End of stream, then back to the loop clip. Always attempted, even after a failure.
    eprintln!("end of stream: FF D9 FF D9, then 0x14 [1,0]");
    let e1 = scr.recovery_raw(&[0xff, 0xd9, 0xff, 0xd9]).err().map(|e| e.to_string());
    if has("--end-zeros") {
        std::thread::sleep(Duration::from_millis(50));
        let _ = scr.recovery_raw(&[0, 0, 0, 0]);
    }
    std::thread::sleep(Duration::from_millis(200));
    let e2 = scr.recovery_raw(&frame::frame(0x14, &[1, 0])).err().map(|e| e.to_string());
    println!("end-of-stream write errors: {e1:?}, 0x14 write error: {e2:?}");
    drain(&mut scr, "after end", Duration::from_millis(1500));
    report_in("end");

    // 4. The stored 0x15 sleep delay can switch the display off a few seconds after a stream ends (info replies
    //    are swallowed by the IN reader here, so it can't be polled). For the watch period keep sending display
    //    on (0x0E [1]); harmless when it is already on.
    let watch: f64 = num(args, "--watch-after").ok().flatten().unwrap_or(30.0);
    let w0 = Instant::now();
    while w0.elapsed().as_secs_f64() < watch {
        let r = scr.recovery_raw(&frame::frame(0x0e, &[1])).err().map(|e| e.to_string());
        drain(&mut scr, "0x0E", Duration::from_millis(3000));
        println!("[after +{:.0} s] 0x0E [1] sent (error: {r:?})", w0.elapsed().as_secs_f64());
    }
    scr.close();

    println!("---");
    println!(
        "{sent}/{frames} frames, {:.1} KB in {el:.2} s: {:.1} fps, {:.0} KB/s; write avg {:.1} ms, worst {worst_ms:.1} ms; {replies} non-status IN message(s)",
        bytes as f64 / 1024.0,
        sent as f64 / el,
        bytes as f64 / 1024.0 / el,
        sum_ms / sent.max(1) as f64
    );
    match failed {
        Some(e) => Err(format!("stopped early: {e}")),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_frame_layout_and_checksum() {
        // len 4 -> 04 00 00 00, JPEG FF D8 FF D9; sum = 4 + 0xFF + 0xD8 + 0xFF + 0xD9 = 0x03B3 -> B3 03
        let w = wrap_frame(&[0xff, 0xd8, 0xff, 0xd9]);
        assert_eq!(w, vec![0x04, 0, 0, 0, 0xff, 0xd8, 0xff, 0xd9, 0xb3, 0x03]);
    }

    #[test]
    fn sum_wraps_at_16_bits_and_covers_length_bytes() {
        let j: Vec<u8> = std::iter::repeat(0xffu8).take(1000).collect();
        let w = wrap_frame(&j);
        let expect = ((1000u32 * 255) + 0xe8 + 0x03) & 0xffff; // length 1000 = E8 03 00 00
        assert_eq!(&w[..4], &[0xe8, 0x03, 0, 0]);
        assert_eq!(u16::from_le_bytes([w[w.len() - 2], w[w.len() - 1]]) as u32, expect);
    }

    #[test]
    fn enter_live_frame_matches_vendor_bytes() {
        assert_eq!(frame::frame(0x11, &[]), vec![0x55, 0xaa, 0x07, 0x00, 0x11, 0x17, 0x01]);
    }
}
