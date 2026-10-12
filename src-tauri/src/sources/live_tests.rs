//! Live, read-only checks against the real network / machine. Run with:
//!   cargo test --lib sources::live_tests -- --ignored --nocapture --test-threads=1
use super::*;

fn ctx() -> SourceCtx {
    let base = std::env::temp_dir().join("cs-sources-live");
    SourceCtx {
        options: serde_json::Value::Null,
        tmp_dir: base.join("tmp"),
        data_dir: base.join("data"),
        sources_dir: base.join("sources"),
        ffmpeg: std::env::var("COOLERSCREEN_FFMPEG").map(Into::into).unwrap_or_else(|_| "C:\\Users\\pc\\Documents\\CoolerScreen\\node_modules\\ffmpeg-static\\ffmpeg.exe".into()),
    }
}
fn all_pages(p: &dyn SourceProvider, c: &SourceCtx) -> Vec<Item> {
    let mut v = vec![];
    for page in 1..=20 {
        let r = p.list(c, page, None, page == 1).unwrap();
        v.extend(r.items);
        if !r.has_more {
            break;
        }
    }
    v
}

#[test]
#[ignore]
fn lovingcool_list() {
    let c = ctx();
    let items = all_pages(&lovingcool::Lovingcool, &c);
    println!("lovingcool: {} items, first = {:?}", items.len(), items.first().map(|i| (&i.title, &i.thumb)));
    assert!(items.len() >= 40 && items.iter().all(|i| i.action == Action::Add));
}

#[test]
#[ignore]
fn otherbrands_list() {
    let c = ctx();
    let items = all_pages(&otherbrands::OtherBrands, &c);
    println!("otherbrands: {} items, badge {:?}", items.len(), items.first().map(|i| &i.badge));
    assert!(items.len() >= 25);
}

#[test]
#[ignore]
fn wallpaperengine_scan() {
    let c = ctx();
    println!("{}", wallpaperengine::WallpaperEngine.detect(&c).status);
    let all = wallpaperengine::scan(&c);
    let vid = all.iter().filter(|e| e.type_ == "video").count();
    println!(
        "WE: {} items, {} video, {} to record, {} unsupported",
        all.len(),
        vid,
        all.iter().filter(|e| e.action == Action::Record).count(),
        all.iter().filter(|e| e.action == Action::None).count()
    );
    assert!(!all.is_empty());
}

#[test]
#[ignore]
fn link_theme_cover() {
    let c = ctx();
    let page = lovingcool::Lovingcool.list(&c, 1, None, false).unwrap();
    let cover = page.items.iter().find_map(|i| i.thumb.clone()).expect("a cover url");
    println!("cover: {cover}");
    let r = link::Link.add(&c, &cover, &AddRequest::default(), &|_, _| {}, &Cancel::new()).unwrap();
    let len = std::fs::metadata(&r.file).unwrap().len();
    println!("downloaded {} ({} bytes) as {}", r.file.display(), len, r.name);
    assert!(len > 1000 && r.temp);
    std::fs::remove_file(&r.file).unwrap();
    // a web page must be refused
    assert!(link::Link.add(&c, "https://www.htssl.com/", &AddRequest::default(), &|_, _| {}, &Cancel::new()).is_err());
}

#[test]
#[ignore]
fn record_scene_5s() {
    let c = ctx();
    let Some(item) = wallpaperengine::scan(&c).into_iter().find(|e| e.action == Action::Record) else { panic!("no scene wallpaper") };
    println!("recording {} ({})", item.title, item.id);
    let t = std::time::Instant::now();
    let r = wallpaperengine::WallpaperEngine.add(&c, &item.id, &AddRequest { seconds: Some(5.0) }, &|f, m| println!("  {:.0}% {m}", f * 100.0), &Cancel::new()).unwrap();
    println!("recorded {} in {:?}", r.file.display(), t.elapsed());
    println!("size {} bytes", std::fs::metadata(&r.file).unwrap().len());
}

fn first_scene(c: &SourceCtx) -> wallpaperengine::Entry {
    wallpaperengine::scan(c).into_iter().find(|e| e.action == Action::Record).expect("a scene wallpaper")
}

/// Pointer held still at (600,400) while recording with cursor capture off, then on. Files are kept in
/// %TEMP%\cs-sources-live2 for inspection (frames at the pointer position).
#[test]
#[ignore]
fn record_cursor_off_vs_on() {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, SetCursorPos};
    let mut c = ctx();
    c.tmp_dir = std::env::temp_dir().join("cs-sources-live2");
    std::env::set_var("COOLERSCREEN_POPOUT_POS", "0,0");
    let item = first_scene(&c);
    let mut orig = POINT::default();
    unsafe { GetCursorPos(&mut orig).unwrap() };
    for (name, cursor) in [("off", false), ("on", true)] {
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let s2 = stop.clone();
        let mover = std::thread::spawn(move || {
            while !s2.load(std::sync::atomic::Ordering::SeqCst) {
                unsafe {
                    let _ = SetCursorPos(600, 400);
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
        let r = wallpaperengine::record(&c, &item, Some(5.0), cursor, &|_, _| {}, &Cancel::new());
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        mover.join().unwrap();
        let r = r.unwrap();
        let dest = c.tmp_dir.join(format!("cursor-{name}.mp4"));
        std::fs::rename(&r.file, &dest).unwrap();
        println!("{name}: {}", dest.display());
    }
    unsafe {
        let _ = SetCursorPos(orig.x, orig.y);
    }
}

/// Pop-out placed mostly off-screen (default position) and covered by another window: it must still be captured.
#[test]
#[ignore]
fn record_offscreen_and_covered() {
    let mut c = ctx();
    c.tmp_dir = std::env::temp_dir().join("cs-sources-live2");
    std::env::remove_var("COOLERSCREEN_POPOUT_POS");
    let item = first_scene(&c);
    println!("popout position: {:?}", wallpaperengine::popout_position());
    let r = wallpaperengine::record(&c, &item, Some(5.0), false, &|_, _| {}, &Cancel::new()).unwrap();
    let dest = c.tmp_dir.join("offscreen.mp4");
    std::fs::rename(&r.file, &dest).unwrap();
    println!("offscreen: {}", dest.display());
    // covered: pop-out at 0,0, then a maximised Notepad on top
    std::env::set_var("COOLERSCREEN_POPOUT_POS", "0,0");
    let before = std::process::Command::new("powershell").args(["-NoProfile", "-Command", "(Get-Process notepad -ErrorAction SilentlyContinue).Id -join ','"]).output().unwrap();
    let before = String::from_utf8_lossy(&before.stdout).trim().to_string();
    let cover = std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        let _ = std::process::Command::new("powershell").args(["-NoProfile", "-Command", "Start-Process notepad -WindowStyle Maximized"]).status();
    });
    let r = wallpaperengine::record(&c, &item, Some(5.0), false, &|_, _| {}, &Cancel::new());
    cover.join().unwrap();
    let _ = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!("Get-Process notepad -ErrorAction SilentlyContinue | Where-Object {{ '{before}' -notmatch ('(^|,)' + $_.Id + '(,|$)') }} | Stop-Process -Force")])
        .status();
    let r = r.unwrap();
    let dest = c.tmp_dir.join("covered.mp4");
    std::fs::rename(&r.file, &dest).unwrap();
    println!("covered: {}", dest.display());
}

/// Automatic recording (`CS_REC_ID`, e.g. we-1279609970): stops early when a seamless loop turns up, else 2 min.
#[test]
#[ignore]
fn record_auto() {
    let c = ctx();
    let id = std::env::var("CS_REC_ID").unwrap_or_else(|_| "we-1279609970".into());
    let item = wallpaperengine::scan(&c).into_iter().find(|e| e.id == id).expect("that wallpaper");
    let t = std::time::Instant::now();
    let r = wallpaperengine::record(&c, &item, None, false, &|_, _| {}, &Cancel::new()).unwrap();
    let ff = c.ffmpeg.clone();
    let info = crate::media::probe::probe(&ff, &r.file).unwrap();
    println!("{} recorded {:.1}s of video in {:?}", item.title, info.duration.unwrap_or(0.0), t.elapsed());
    std::fs::remove_file(&r.file).unwrap();
}
