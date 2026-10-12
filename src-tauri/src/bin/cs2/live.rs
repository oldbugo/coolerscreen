//! `cs2 live-test`: 3.0 spike. Hypothesis: the screen's live mode is raw JPEG frames (FF D8 ... FF D9) written straight
//! to WinUSB bulk OUT 0x02, ended by `FF D9 FF D9` then `00 00 00 00`. This sends N test frames as fast as the link
//! accepts them and measures it. Only raw mode is implemented (no evidence for a framed variant exists in the code or docs).
//! Afterwards it sends 0x14 [1,0] (play the loop clip) so the screen returns to normal. Never sends 0x05/0x0C/0x23 or uploads.
use cs_core::device::{self, frame};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

const LW: usize = 2400; // landscape canvas
const LH: usize = 1080;
const PW: usize = 1080; // portrait frame sent to the screen
const PH: usize = 2400;
const MAX_FRAMES: usize = 600;

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

/// Portrait RGB buffer. Landscape (x, y) maps to portrait row x, column 1079 - y (ffmpeg `transpose=1`, 90 deg clockwise).
struct Canvas {
    rgb: Vec<u8>,
}

impl Canvas {
    fn test_pattern() -> Canvas {
        let mut c = Canvas { rgb: vec![0; PW * PH * 3] };
        for y in 0..LH {
            for x in 0..LW {
                // diagonal gradient with 8 colour bars along the bottom, so rotation and colour are easy to judge
                let (r, g, b) = if y >= LH - 140 {
                    const BARS: [(u8, u8, u8); 8] = [
                        (255, 255, 255), (255, 255, 0), (0, 255, 255), (0, 255, 0),
                        (255, 0, 255), (255, 0, 0), (0, 0, 255), (0, 0, 0),
                    ];
                    BARS[x * 8 / LW]
                } else {
                    ((x * 255 / LW) as u8, (y * 255 / LH) as u8, 140)
                };
                c.set(x, y, [r, g, b]);
            }
        }
        // border and an "UP" marker corner (top-left of the landscape picture): white square
        for i in 0..LW {
            for t in 0..6 {
                c.set(i, t, [255, 255, 255]);
                c.set(i, LH - 1 - t, [255, 255, 255]);
            }
        }
        for i in 0..LH {
            for t in 0..6 {
                c.set(t, i, [255, 255, 255]);
                c.set(LW - 1 - t, i, [255, 255, 255]);
            }
        }
        c.rect(30, 30, 120, 120, [255, 255, 255]);
        c
    }

    #[inline]
    fn set(&mut self, x: usize, y: usize, p: [u8; 3]) {
        let i = (x * PW + (PW - 1 - y)) * 3;
        self.rgb[i..i + 3].copy_from_slice(&p);
    }

    fn rect(&mut self, x0: usize, y0: usize, w: usize, h: usize, p: [u8; 3]) {
        for y in y0..(y0 + h).min(LH) {
            for x in x0..(x0 + w).min(LW) {
                self.set(x, y, p);
            }
        }
    }

    /// Text in landscape coordinates, `s` px per glyph pixel, black on a white box.
    fn text(&mut self, x0: usize, y0: usize, s: usize, text: &str) {
        let w = text.chars().count() * 6 * s + 2 * s;
        self.rect(x0, y0, w, 9 * s, [255, 255, 255]);
        for (k, ch) in text.chars().enumerate() {
            let g = glyph(ch);
            for (ry, row) in g.iter().enumerate() {
                for rx in 0..5 {
                    if row & (16 >> rx) != 0 {
                        self.rect(x0 + s + (k * 6 + rx) * s, y0 + s + ry * s, s, s, [0, 0, 0]);
                    }
                }
            }
        }
    }

    fn encode(&self, quality: u8) -> Result<Vec<u8>, String> {
        let mut out = Vec::with_capacity(400_000);
        let enc = jpeg_encoder::Encoder::new(&mut out, quality);
        enc.encode(&self.rgb, PW as u16, PH as u16, jpeg_encoder::ColorType::Rgb).map_err(|e| format!("JPEG encode failed: {e}"))?;
        Ok(out)
    }
}

fn arg_val<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).map(String::as_str)
}

fn valid_jpeg(b: &[u8]) -> bool {
    b.len() > 4 && b[0] == 0xff && b[1] == 0xd8 && b[b.len() - 2] == 0xff && b[b.len() - 1] == 0xd9
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
    let frames: usize = arg_val(args, "--frames").map(|v| v.parse().map_err(|_| "--frames needs a number")).transpose()?.unwrap_or(30);
    let quality: u8 = arg_val(args, "--quality").map(|v| v.parse().map_err(|_| "--quality needs 1-100")).transpose()?.unwrap_or(75);
    let fps_cap: Option<f64> = arg_val(args, "--fps").map(|v| v.parse().map_err(|_| "--fps needs a number")).transpose()?;
    let dry = args.iter().any(|a| a == "--dry-run");
    match arg_val(args, "--mode").unwrap_or("raw") {
        "raw" => {}
        m => return Err(format!("--mode {m}: only raw is implemented (no evidence of any framed variant in the code or docs)")),
    }
    if !(1..=MAX_FRAMES).contains(&frames) || !(1..=100).contains(&quality) {
        return Err(format!("--frames must be 1..{MAX_FRAMES} and --quality 1..100"));
    }

    // 1. Build all frames up front, so encoding time does not pollute the write measurements.
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
        let base = Canvas::test_pattern();
        eprintln!("test pattern built in {} ms", t0.elapsed().as_millis());
        let n = if dry { 1 } else { frames };
        for i in 0..n {
            let mut c = Canvas { rgb: base.rgb.clone() };
            c.text(200, 200, 28, &format!("F{i:04}"));
            c.text(200, 520, 14, &format!("T{:07.3}S", i as f64 / 30.0));
            jpegs.push(c.encode(quality)?);
        }
        eprintln!("encoded {n} frame(s) at quality {quality} in {} ms", t0.elapsed().as_millis());
    }
    for j in &jpegs {
        if !valid_jpeg(j) {
            return Err("internal error: encoded frame is not a valid JPEG".into());
        }
    }

    if dry {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("work");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let p = dir.join("live-test.jpg");
        std::fs::write(&p, &jpegs[0]).map_err(|e| e.to_string())?;
        println!("dry run: wrote {} ({} bytes, {PW}x{PH}); device not opened", p.display(), jpegs[0].len());
        return Ok(());
    }

    // 2. Open the screen over WinUSB (never the serial fallback: a serial close would arm the 30 s watchdog).
    let mut scr = device::Screen::open_usb().map_err(|e| e.to_string())?;
    eprintln!("opened over {}; sending {frames} raw JPEG frames to OUT 0x02", scr.transport);
    let status0 = frame::STATUS_FRAMES.load(Ordering::Relaxed);
    let mut unparsed = 0usize;
    let mut sum_ms = 0.0f64;
    let mut worst_ms = 0.0f64;
    let mut bytes = 0usize;
    let mut failed: Option<String> = None;
    let start = Instant::now();
    for i in 0..frames {
        if let Some(cap) = fps_cap {
            let due = Duration::from_secs_f64(i as f64 / cap);
            if let Some(w) = due.checked_sub(start.elapsed()) {
                std::thread::sleep(w);
            }
        }
        let j = &jpegs[i % jpegs.len()];
        let t = Instant::now();
        if let Err(e) = scr.recovery_raw(j) {
            failed = Some(e.to_string());
            eprintln!("frame {i}: WRITE FAILED after {} ms: {e}", t.elapsed().as_millis());
            break;
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        sum_ms += ms;
        worst_ms = worst_ms.max(ms);
        bytes += j.len();
        let el = start.elapsed().as_secs_f64().max(0.001);
        println!(
            "frame {i:4}: {:6.1} KB  write {ms:7.1} ms  | avg {:5.1} fps  {:7.0} KB/s",
            j.len() as f64 / 1024.0,
            (i + 1) as f64 / el,
            bytes as f64 / 1024.0 / el
        );
        // Anything the screen answered that is not a status frame (parsed 55 AA frame, JSON or key+hex).
        while let Some(m) = scr.recovery_poll(Duration::ZERO) {
            unparsed += 1;
            println!("  IN 0x81 after frame {i}: {m}");
        }
    }
    let el = start.elapsed().as_secs_f64().max(0.001);

    // 3. End of stream, then back to the loop clip. Attempted even after a write failure.
    eprintln!("sending end-of-stream (FF D9 FF D9, 00 00 00 00)");
    let e1 = scr.recovery_raw(&[0xff, 0xd9, 0xff, 0xd9]);
    std::thread::sleep(Duration::from_millis(50));
    let e2 = scr.recovery_raw(&[0, 0, 0, 0]);
    eprintln!("end-of-stream write results: {:?} / {:?}", e1.as_ref().err().map(|e| e.to_string()), e2.as_ref().err().map(|e| e.to_string()));
    std::thread::sleep(Duration::from_millis(200));
    let r = scr.recovery_cmd(0x14, &[1, 0], Duration::from_secs(3));
    match &r {
        Ok(v) => println!("0x14 [1,0] reply: {v}"),
        Err(e) => println!("0x14 [1,0] sent, no parsed reply ({e}); check the screen is back on the loop clip"),
    }
    while let Some(m) = scr.recovery_poll(Duration::from_millis(300)) {
        unparsed += 1;
        println!("  IN 0x81 late: {m}");
    }
    let status = frame::STATUS_FRAMES.load(Ordering::Relaxed) - status0;
    scr.close();

    println!("---");
    println!(
        "{} frames, {:.1} KB in {el:.2} s: {:.1} fps, {:.0} KB/s; write avg {:.1} ms, worst {worst_ms:.1} ms",
        if failed.is_some() { "partial" } else { "all" },
        bytes as f64 / 1024.0,
        frames as f64 / el,
        bytes as f64 / 1024.0 / el,
        sum_ms / frames as f64
    );
    println!("IN 0x81: {unparsed} non-status frame(s), {status} status (0x22) frame(s) ignored");
    match failed {
        Some(e) => Err(format!("stopped early: {e}")),
        None => Ok(()),
    }
}
