//! `cs2 trial`: guided hardware trials for the 3.0 live mode. A numbered menu; each trial says in plain English what
//! will appear, runs it, asks the user simple questions and logs everything as JSON lines to work/trials/<date>.jsonl.
//! Every trial ends by waking the screen (display on 0x0E [1], then loop clip 0x14 [1,0]); Ctrl+C also does that.
//! Only ever sends: 0x11 (live enter/keepalive), live JPEG frames, FF D9 FF D9, 0x0E, 0x14, 0x06 (info, read only) and,
//! in one end variant, 00 00 00 00. Never 0x15, 0x03, 0x01, 0x02, 0x05, 0x0C, 0x23 or uploads.
//! `cs2 trial --dry-run`: no device; pictures go to work/trials-dry/ and streaming is skipped.
use crate::live::{encode_fit, wrap_frame, Canvas};
use cs_core::device::{self, frame};
use serde_json::{json, Value};
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

static ABORT: AtomicBool = AtomicBool::new(false);
const KEEPALIVE: Duration = Duration::from_millis(1500);
type R<T> = Result<T, String>;

#[cfg(windows)]
unsafe extern "system" fn on_ctrl(_t: u32) -> i32 {
    ABORT.store(true, Ordering::SeqCst);
    1 // handled: the trial loop notices, restores the screen and exits
}

fn aborted() -> bool {
    ABORT.load(Ordering::SeqCst)
}

fn check_abort() -> R<()> {
    if aborted() { Err("aborted".into()) } else { Ok(()) }
}

// ---------------------------------------------------------------- logging and input

struct Log {
    file: std::fs::File,
    path: std::path::PathBuf,
}

impl Log {
    fn open(dry: bool) -> R<Log> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("work").join("trials");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stamp: String = device::log::iso_now().chars().take(19).filter(|c| *c != ':' && *c != '-').map(|c| if c == 'T' { '_' } else { c }).collect();
        let path = dir.join(format!("{stamp}{}.jsonl", if dry { "-dry" } else { "" }));
        let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        Ok(Log { file, path })
    }
    fn put(&mut self, trial: &str, event: &str, data: Value) {
        let line = json!({ "t": device::log::iso_now(), "trial": trial, "event": event, "data": data });
        let _ = writeln!(self.file, "{line}");
        let _ = self.file.flush();
    }
}

struct Input {
    rx: Receiver<String>,
}

impl Input {
    fn new() -> Input {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut s = String::new();
            loop {
                s.clear();
                match std::io::stdin().read_line(&mut s) {
                    Ok(0) | Err(_) => break, // closed: the sender drops, prompts then abort
                    Ok(_) => {
                        if tx.send(s.trim().to_string()).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        Input { rx }
    }
    fn flush(&self) {
        while self.rx.try_recv().is_ok() {}
    }
    /// One line typed by the user (Enter alone = empty string).
    fn line(&self, wait: Duration) -> R<Option<String>> {
        let end = Instant::now() + wait;
        loop {
            check_abort()?;
            match self.rx.recv_timeout(Duration::from_millis(150).min(end.saturating_duration_since(Instant::now()).max(Duration::from_millis(1)))) {
                Ok(s) => return Ok(Some(s)),
                Err(RecvTimeoutError::Timeout) => {
                    if Instant::now() >= end {
                        return Ok(None);
                    }
                }
                Err(RecvTimeoutError::Disconnected) => return Err("input closed".into()),
            }
        }
    }
    fn enter(&self, prompt: &str) -> R<()> {
        println!("{prompt}");
        self.flush();
        self.line(Duration::from_secs(86400))?;
        Ok(())
    }
    fn yn(&self, q: &str) -> R<Option<bool>> {
        loop {
            println!("{q} (y = yes, n = no, Enter = skip)");
            self.flush();
            match self.line(Duration::from_secs(86400))?.unwrap_or_default().to_lowercase().as_str() {
                "y" | "yes" => return Ok(Some(true)),
                "n" | "no" => return Ok(Some(false)),
                "" => return Ok(None),
                _ => println!("Please type y, n, or just press Enter."),
            }
        }
    }
    fn choice(&self, q: &str, opts: &[&str]) -> R<Option<String>> {
        loop {
            println!("{q}");
            for (i, o) in opts.iter().enumerate() {
                println!("   {}) {o}", i + 1);
            }
            println!("   (type the number, or press Enter to skip)");
            self.flush();
            let s = self.line(Duration::from_secs(86400))?.unwrap_or_default();
            if s.is_empty() {
                return Ok(None);
            }
            if let Some(o) = s.parse::<usize>().ok().and_then(|n| opts.get(n.wrapping_sub(1))) {
                return Ok(Some(o.to_string()));
            }
            println!("Please type a number from the list.");
        }
    }
    fn note(&self) -> R<Option<String>> {
        println!("Anything else you noticed? Type it and press Enter, or just press Enter to skip.");
        self.flush();
        Ok(self.line(Duration::from_secs(86400))?.filter(|s| !s.is_empty()))
    }
}

// ---------------------------------------------------------------- device rig

struct Rig {
    scr: Option<device::Screen>,
    dry: bool,
    last_keep: Instant,
}

struct Stats {
    sent: usize,
    secs: f64,
    bytes: usize,
    worst_ms: f64,
    err: Option<String>,
}

impl Stats {
    fn json(&self, fps_target: f64, jpeg_kb: f64, q: u8) -> Value {
        json!({
            "target_fps": fps_target, "frames_sent": self.sent, "seconds": round(self.secs),
            "achieved_fps": round(self.sent as f64 / self.secs.max(0.001)),
            "kb_per_s": round(self.bytes as f64 / 1024.0 / self.secs.max(0.001)),
            "worst_write_ms": round(self.worst_ms), "jpeg_kb": round(jpeg_kb), "jpeg_quality": q, "error": self.err,
        })
    }
}

fn round(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

impl Rig {
    fn open(dry: bool) -> R<Rig> {
        let scr = if dry { None } else { Some(device::Screen::open_usb().map_err(|e| e.to_string())?) };
        Ok(Rig { scr, dry, last_keep: Instant::now() })
    }
    fn send(&mut self, b: &[u8]) -> R<()> {
        match &mut self.scr {
            Some(s) => s.recovery_raw(b).map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }
    fn sleep(&self, d: Duration) {
        if !self.dry {
            std::thread::sleep(d);
        }
    }
    fn drain(&mut self) {
        if let Some(s) = &mut self.scr {
            while s.recovery_poll(Duration::ZERO).is_some() {}
        }
    }
    fn enter_live(&mut self) -> R<()> {
        self.send(&frame::frame(0x11, &[]))?;
        self.last_keep = Instant::now();
        self.sleep(Duration::from_millis(100));
        Ok(())
    }
    fn keepalive(&mut self) -> R<()> {
        if self.last_keep.elapsed() >= KEEPALIVE {
            self.send(&frame::frame(0x11, &[]))?;
            self.last_keep = Instant::now();
        }
        Ok(())
    }
    /// Send `wrapped` frames (cycled) at up to `fps` for `secs`. Dry run: just 3 frames.
    fn stream(&mut self, wrapped: &[Vec<u8>], fps: f64, secs: f64) -> R<Stats> {
        let n = if self.dry { 3 } else { (fps * secs).round() as usize };
        let (mut bytes, mut sent, mut worst) = (0usize, 0usize, 0.0f64);
        let start = Instant::now();
        let mut err = None;
        for i in 0..n {
            check_abort()?;
            if !self.dry {
                if let Some(w) = Duration::from_secs_f64(i as f64 / fps).checked_sub(start.elapsed()) {
                    std::thread::sleep(w);
                }
            }
            if let Err(e) = self.keepalive() {
                err = Some(e);
                break;
            }
            let f = &wrapped[i % wrapped.len()];
            let t = Instant::now();
            if let Err(e) = self.send(f) {
                err = Some(e);
                break;
            }
            worst = worst.max(t.elapsed().as_secs_f64() * 1000.0);
            bytes += f.len();
            sent += 1;
            self.drain();
        }
        Ok(Stats { sent, secs: start.elapsed().as_secs_f64(), bytes, worst_ms: worst, err })
    }
    fn end_stream(&mut self) -> R<()> {
        self.send(&[0xff, 0xd9, 0xff, 0xd9])?;
        self.sleep(Duration::from_millis(200));
        Ok(())
    }
    fn display_on(&mut self) -> R<()> {
        self.send(&frame::frame(0x0e, &[1]))
    }
    fn play_loop(&mut self) -> R<()> {
        self.send(&frame::frame(0x14, &[1, 0]))
    }
    /// Restore: display on, then the loop clip. Always attempted, errors reported but not fatal.
    fn wake(&mut self) -> Option<String> {
        let a = self.display_on().err();
        self.sleep(Duration::from_millis(200));
        let b = self.play_loop().err();
        self.sleep(Duration::from_millis(500));
        a.or(b)
    }
    /// Read-only info (0x06): Some(true/false) = display on, None = no usable reply. Returns the raw reply too.
    fn info_display_on(&mut self) -> (Option<bool>, Value) {
        let Some(s) = &mut self.scr else { return (None, Value::Null) };
        match s.recovery_cmd(0x06, &[], Duration::from_millis(1500)) {
            Ok(v) => {
                let d = &v["data"]["diplay_on"];
                let on = d.as_bool().or_else(|| d.as_i64().map(|n| n != 0)).or_else(|| d.as_str().map(|t| t == "1" || t == "true"));
                (on, d.clone())
            }
            Err(e) => (None, json!(format!("no reply: {e}"))),
        }
    }
}

// ---------------------------------------------------------------- pictures

const Q_START: u8 = 100;
const MAX_BYTES: usize = 260_000;

struct Built {
    wrapped: Vec<Vec<u8>>,
    jpeg_kb: f64,
    quality: u8,
    first: Vec<u8>, // first plain JPEG, for the dry run
    dims: (usize, usize),
}

fn build(canvases: Vec<Canvas>, portrait: bool) -> R<Built> {
    let (mut q, mut tot, mut dims) = (Q_START, 0usize, (0, 0));
    let mut wrapped = Vec::new();
    let mut first = Vec::new();
    for c in &canvases {
        let (w, h, px) = c.pixels(portrait);
        dims = (w, h);
        let (j, used) = encode_fit(w, h, &px, q, MAX_BYTES)?;
        q = used;
        tot += j.len();
        if first.is_empty() {
            first = j.clone();
        }
        wrapped.push(wrap_frame(&j));
    }
    Ok(Built { wrapped, jpeg_kb: tot as f64 / 1024.0 / canvases.len() as f64, quality: q, first, dims })
}

fn orientation_picture() -> Canvas {
    let mut c = Canvas::blank(0.7, [20, 30, 80]);
    c.drect(0, 0, 2400, 10, [255; 3]);
    c.drect(0, 1070, 2400, 10, [255; 3]);
    c.drect(0, 0, 10, 1080, [255; 3]);
    c.drect(2390, 0, 10, 1080, [255; 3]);
    c.drect(10, 10, 100, 100, [230, 40, 40]); // corner colours: top-left red, top-right green, bottom-left blue, bottom-right yellow
    c.drect(2290, 10, 100, 100, [40, 200, 60]);
    c.drect(10, 970, 100, 100, [60, 100, 255]);
    c.drect(2290, 970, 100, 100, [240, 220, 40]);
    c.arrow_up(560, 150, 220, 160, 100, 420, [255, 220, 60]);
    c.text(900, 420, 16, "THIS WAY UP");
    c.text(1030, 30, 10, "TOP");
    c.text(960, 960, 10, "BOTTOM");
    c.text(130, 490, 10, "LEFT");
    c.text(2040, 490, 10, "RIGHT");
    c
}

fn speed_pictures(fps: u32, pool: usize) -> Vec<Canvas> {
    (0..pool)
        .map(|i| {
            let mut c = Canvas::blank(0.7, [10, 40, 30]);
            c.text(300, 80, 24, &format!("{fps} FPS"));
            c.text(300, 380, 24, &format!("F{i:04}"));
            c.drect(150, 760, 2100, 200, [30, 30, 30]);
            let x = 150 + (2100 - 120) * i / pool.max(1);
            c.drect(x, 760, 120, 200, [255, 230, 0]);
            c
        })
        .collect()
}

fn dry_save(name: &str, b: &Built) {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("work").join("trials-dry");
    let _ = std::fs::create_dir_all(&dir);
    let p = dir.join(name);
    let _ = std::fs::write(&p, &b.first);
    println!("   (dry run: picture {}x{} saved to {})", b.dims.0, b.dims.1, p.display());
}

// ---------------------------------------------------------------- trials

struct Ctx<'a> {
    rig: Rig,
    inp: &'a Input,
    log: &'a mut Log,
    poll: bool,
}

fn announce(inp: &Input, what: &str, look: &str) -> R<()> {
    println!("\n{what}\n   What to look for: {look}");
    inp.enter("Press Enter when you're looking at the screen.")
}

fn orient_trial(cx: &mut Ctx, id: &str, portrait: bool) -> R<String> {
    let (title, what) = if portrait {
        ("Orientation, sideways picture", "The screen will show a picture that is sent the other way round (a tall picture). It stays up for about 10 seconds.")
    } else {
        ("Orientation, normal picture", "The screen will show a wide picture with the words THIS WAY UP and a yellow arrow. It stays up for about 10 seconds.")
    };
    announce(cx.inp, &format!("Trial {id}: {title}\n{what}"), "Is the writing the right way up and the right way round? Is the arrow pointing up? Does the picture reach all four edges? Can you read TOP, BOTTOM, LEFT and RIGHT near the edges, or are some hidden (for example by the notch)?")?;
    let b = build(vec![orientation_picture()], portrait)?;
    if cx.rig.dry {
        dry_save(if portrait { "trial2-portrait.jpg" } else { "trial1-landscape.jpg" }, &b);
    }
    cx.log.put(id, "start", json!({ "portrait": portrait, "size": [b.dims.0, b.dims.1], "jpeg_kb": round(b.jpeg_kb), "quality": b.quality }));
    cx.rig.enter_live()?;
    println!("Showing the picture now...");
    let st = cx.rig.stream(&b.wrapped, 2.0, 10.0)?;
    cx.log.put(id, "measured", st.json(2.0, b.jpeg_kb, b.quality));
    let w = cx.rig.end_stream();
    cx.log.put(id, "ended", json!({ "error": w.err() }));
    let wake_err = cx.rig.wake();
    cx.log.put(id, "wake", json!({ "error": wake_err }));
    println!("The picture is finished and the screen is going back to normal.");
    let o = cx.inp.choice("How did the picture look?", &["Correct: readable, right way up", "Upside down", "Sideways (turned a quarter)", "Mirrored (writing backwards)", "Squashed or cropped", "Nothing showed"])?;
    let fill = cx.inp.yn("Did the picture fill the whole screen?")?;
    let edges = cx.inp.yn("Could you read all four words (TOP, BOTTOM, LEFT, RIGHT)?")?;
    let note = cx.inp.note()?;
    cx.log.put(id, "answers", json!({ "look": o, "fills_screen": fill, "all_edge_words_readable": edges, "note": note }));
    Ok(format!("{title}: {}; fills screen: {}; edge words readable: {}", o.as_deref().unwrap_or("skipped"), yn(fill), yn(edges)))
}

fn yn(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "yes",
        Some(false) => "no",
        None => "skipped",
    }
}

fn smooth_trial(cx: &mut Ctx, id: &str) -> R<String> {
    announce(
        cx.inp,
        &format!("Trial {id}: Smoothness\nThe screen will show a counter and a yellow bar sweeping left to right, first at 10 pictures per second, then 30, then 60, about 8 seconds each. The speed is written on the screen."),
        "Is the bar moving smoothly, a bit jerky, very jerky, or frozen? Remember how each speed looked, I will ask afterwards.",
    )?;
    cx.rig.enter_live()?;
    let mut results = Vec::new();
    for fps in [10u32, 30, 60] {
        let b = build(speed_pictures(fps, 30), false)?;
        if cx.rig.dry {
            dry_save(&format!("trial3-{fps}fps.jpg"), &b);
        }
        println!("Now showing {fps} FPS...");
        let st = cx.rig.stream(&b.wrapped, fps as f64, 8.0)?;
        println!("   (sent {:.1} pictures per second)", st.sent as f64 / st.secs.max(0.001));
        cx.log.put(id, "measured", st.json(fps as f64, b.jpeg_kb, b.quality));
        results.push((fps, st.sent as f64 / st.secs.max(0.001)));
    }
    let w = cx.rig.end_stream();
    cx.log.put(id, "ended", json!({ "error": w.err() }));
    let wake_err = cx.rig.wake();
    cx.log.put(id, "wake", json!({ "error": wake_err }));
    println!("Finished. The screen is going back to normal.");
    let mut parts = Vec::new();
    for (fps, got) in results {
        let a = cx.inp.choice(&format!("How did the {fps} FPS part look?"), &["Smooth", "Slightly jerky", "Very jerky", "Frozen / nothing moved"])?;
        cx.log.put(id, "answer", json!({ "speed": fps, "achieved_send_fps": round(got), "look": a }));
        parts.push(format!("{fps} fps (sent {got:.0}): {}", a.as_deref().unwrap_or("skipped")));
    }
    let note = cx.inp.note()?;
    cx.log.put(id, "answers", json!({ "note": note }));
    Ok(format!("Smoothness: {}", parts.join("; ")))
}

#[derive(Clone, Copy)]
enum End {
    Plain,        // FF D9 FF D9, then loop clip
    DisplayOn,    // FF D9 FF D9, display on, loop clip, display on again 5 s later
    Zeros,        // FF D9 FF D9, 00 00 00 00, loop clip
}

fn black_trial(cx: &mut Ctx, id: &str, end: End) -> R<String> {
    let (title, how) = match end {
        End::Plain => ("Black-screen timer, normal ending", "The moving counter stops and the screen is told to go back to the normal clip. Nothing else is sent afterwards."),
        End::DisplayOn => ("Black-screen timer, with an extra wake-up", "Same, but the screen is also told to stay on, once straight away and once more 5 seconds later."),
        End::Zeros => ("Black-screen timer, another ending", "Same as the normal ending, but a different closing signal is used first."),
    };
    announce(
        cx.inp,
        &format!("Trial {id}: {title}\nThe screen will show a moving counter for 10 seconds. {how}\nThen I will ask you to watch the screen for up to 2 minutes, to see whether it turns black by itself."),
        "After the counter stops: does the normal clip come back, and does the screen turn black later? Press Enter the moment it goes black. If it is still on after 2 minutes, type n and press Enter.",
    )?;
    let b = build(speed_pictures(10, 30), false)?;
    if cx.rig.dry {
        dry_save(&format!("trial{id}.jpg"), &b);
    }
    cx.rig.enter_live()?;
    println!("Showing the counter...");
    let st = cx.rig.stream(&b.wrapped, 10.0, 10.0)?;
    cx.log.put(id, "measured", st.json(10.0, b.jpeg_kb, b.quality));
    cx.rig.end_stream()?;
    let mut later: Vec<(f64, Vec<u8>, &str)> = Vec::new();
    match end {
        End::Plain => cx.rig.play_loop()?,
        End::DisplayOn => {
            cx.rig.display_on()?;
            cx.rig.sleep(Duration::from_millis(200));
            cx.rig.play_loop()?;
            later.push((5.0, frame::frame(0x0e, &[1]), "display on again"));
        }
        End::Zeros => {
            cx.rig.sleep(Duration::from_millis(50));
            cx.rig.send(&[0, 0, 0, 0])?;
            cx.rig.sleep(Duration::from_millis(200));
            cx.rig.play_loop()?;
        }
    }
    cx.log.put(id, "end_sequence_sent", json!({ "variant": match end { End::Plain => "FFD9, 0x14[1,0]", End::DisplayOn => "FFD9, 0x0E[1], 0x14[1,0], +5s 0x0E[1]", End::Zeros => "FFD9, 00000000, 0x14[1,0]" } }));
    println!("\nThe stream has ended. WATCH THE SCREEN NOW.\n   Press Enter the moment it goes black. If it is still on after 2 minutes, type n and press Enter.");
    cx.inp.flush();
    let t0 = Instant::now();
    let max = if cx.rig.dry { 0.0 } else { 150.0 };
    let (mut user_black, mut user_never, mut auto_black, mut next_poll) = (None::<f64>, false, None::<f64>, 5.0f64);
    let mut cap = max;
    while t0.elapsed().as_secs_f64() < cap {
        let el = t0.elapsed().as_secs_f64();
        for l in later.iter_mut().filter(|l| l.0 <= el && !l.2.is_empty()) {
            let r = cx.rig.send(&l.1).err();
            cx.log.put(id, "later_send", json!({ "what": l.2, "at_s": round(el), "error": r }));
            l.2 = "";
        }
        if cx.poll && el >= next_poll {
            next_poll += 5.0;
            let (on, raw) = cx.rig.info_display_on();
            cx.log.put(id, "poll_display_on", json!({ "at_s": round(el), "display_on": on, "raw": raw }));
            if on == Some(false) && auto_black.is_none() {
                auto_black = Some(el);
                println!("   (the tool also sees the display switch off, about {el:.0} s after the stream ended)");
                cap = cap.min(el + 10.0);
            }
        }
        if let Some(s) = cx.inp.line(Duration::from_millis(500))? {
            if s.eq_ignore_ascii_case("n") {
                user_never = true;
            } else {
                user_black = Some(t0.elapsed().as_secs_f64());
            }
            break;
        }
    }
    let wake_err = cx.rig.wake();
    cx.log.put(id, "wake", json!({ "error": wake_err }));
    println!("The screen is being woken up and set back to the normal clip.");
    let seen = cx.inp.choice("What did you see in the first few seconds after the counter stopped?", &["The normal clip came back", "The last counter picture stayed frozen", "Black straight away", "Something else"])?;
    let note = cx.inp.note()?;
    cx.log.put(id, "answers", json!({ "seconds_to_black_user": user_black.map(round), "still_on_after_waiting": user_never, "seconds_to_black_tool": auto_black.map(round), "after_stream": seen, "note": note }));
    let res = match (user_black, user_never) {
        (Some(s), _) => format!("went black after {s:.0} s"),
        (None, true) => "still on after waiting".to_string(),
        _ => "no answer".to_string(),
    };
    Ok(format!("{title}: {res}; tool saw black at {}", auto_black.map(|a| format!("{a:.0} s")).unwrap_or("never / no info".into())))
}

fn list() {
    println!("\nTrials:");
    println!("  1) Orientation: normal picture (THIS WAY UP and an arrow), about 20 seconds");
    println!("  2) Orientation: sideways picture (only needed if 1 was not correct)");
    println!("  3) Smoothness at 10, 30 and 60 pictures per second, about 1 minute");
    println!("  4) Does the screen turn black afterwards? Normal ending, up to 2.5 minutes");
    println!("  5) Same, with an extra stay-on signal, up to 2.5 minutes");
    println!("  6) Same, with a different closing signal, up to 2.5 minutes");
    println!("  a) All of them in order");
    println!("  q) Quit");
}

pub fn trial(args: &[String]) -> i32 {
    let dry = args.iter().any(|a| a == "--dry-run");
    let poll = !args.iter().any(|a| a == "--no-poll");
    #[cfg(windows)]
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(on_ctrl), 1);
    }
    let mut log = match Log::open(dry) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cannot create the log file: {e}");
            return 1;
        }
    };
    let inp = Input::new();
    println!("CoolerScreen live-mode trials{}", if dry { " (DRY RUN: no screen is used)" } else { "" });
    println!("I will show things on the screen and ask what you see. Answers are saved so they can be read later.");
    println!("Press Ctrl+C at any time: the screen is set back to normal first.");
    log.put("-", "session", json!({ "dry_run": dry, "poll_info": poll, "version": env!("CARGO_PKG_VERSION") }));
    let mut summaries: Vec<String> = Vec::new();
    let mut code = 0;
    'menu: loop {
        list();
        println!("Which trial? (a number, several like \"1 3\", a, or q)");
        let Ok(Some(line)) = inp.line(Duration::from_secs(86400)) else { break };
        let l = line.to_lowercase();
        let picks: Vec<u8> = if l == "q" {
            break;
        } else if l == "a" {
            vec![1, 2, 3, 4, 5, 6]
        } else {
            l.split(|c: char| c == ' ' || c == ',').filter_map(|t| t.parse().ok()).filter(|n| (1..=6).contains(n)).collect()
        };
        if picks.is_empty() {
            println!("Please pick a number from the list.");
            continue;
        }
        for n in picks {
            let id = n.to_string();
            let rig = match Rig::open(dry) {
                Ok(r) => r,
                Err(e) => {
                    println!("Could not open the screen: {e}\nClose any other program that uses it and try again.");
                    log.put(&id, "error", json!({ "open": e }));
                    code = 1;
                    continue 'menu;
                }
            };
            log.put(&id, "begin", json!({}));
            let mut cx = Ctx { rig, inp: &inp, log: &mut log, poll };
            let r = match n {
                1 => orient_trial(&mut cx, &id, false),
                2 => orient_trial(&mut cx, &id, true),
                3 => smooth_trial(&mut cx, &id),
                4 => black_trial(&mut cx, &id, End::Plain),
                5 => black_trial(&mut cx, &id, End::DisplayOn),
                _ => black_trial(&mut cx, &id, End::Zeros),
            };
            // Always restore, whatever happened (a second wake is harmless).
            let _ = cx.rig.end_stream();
            let w = cx.rig.wake();
            cx.log.put(&id, "final_wake", json!({ "error": w }));
            cx.rig.scr.take().map(|s| s.close());
            match r {
                Ok(s) => {
                    println!("\nSUMMARY trial {id}: {s}\n");
                    cx.log.put(&id, "summary", json!(s));
                    summaries.push(format!("{id}: {s}"));
                }
                Err(e) => {
                    println!("\nTrial {id} stopped: {e}. The screen was set back to normal.");
                    cx.log.put(&id, "stopped", json!(e));
                    summaries.push(format!("{id}: stopped ({e})"));
                    if aborted() || e == "input closed" {
                        break 'menu;
                    }
                    code = 1;
                }
            }
        }
    }
    println!("\nSummary:");
    for s in &summaries {
        println!("  {s}");
    }
    println!("\nResults saved to: {}", log.path.display());
    code
}
