//! Source: Wallpaper Engine (local). Lists the user's Steam Workshop subscriptions (431960) and Wallpaper Engine's own
//! projects (projects/myprojects) from `project.json` files, read-only.
//!  - video wallpapers: "Add to gallery" copies the video file into the Gallery folder.
//!  - scene and web wallpapers: "Record" opens the wallpaper in its own 2400x1080 window (Wallpaper Engine's documented
//!    command line: `-control openWallpaper ... -playInWindow`), records it with Windows Graphics Capture ([`super::recorder`]),
//!    then closes that window (`-control closeWallpaper`). The desktop wallpaper and Wallpaper Engine's settings (including
//!    mute) are not touched. Needs a running Wallpaper Engine.
use super::*;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;
use std::time::Duration;

pub struct WallpaperEngine;
const APP_ID: &str = "431960";
const PAGE: usize = 36;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// One recording at a time.
static RECORDING: Mutex<()> = Mutex::new(());

/// `"path"   "D:\\SteamLibrary"` lines of libraryfolders.vdf.
pub fn parse_vdf_paths(text: &str) -> Vec<String> {
    let mut out = vec![];
    for line in text.lines() {
        let l = line.trim();
        let Some(rest) = l.strip_prefix("\"path\"") else { continue };
        let rest = rest.trim();
        if let Some(r) = rest.strip_prefix('"') {
            if let Some(end) = r.find('"') {
                out.push(r[..end].replace("\\\\", "\\"));
            }
        }
    }
    out
}

pub fn steam_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![];
    if let Some(d) = std::env::var_os("COOLERSCREEN_STEAM_DIR") {
        roots.push(PathBuf::from(d)); // tests / unusual installs
    }
    let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".into());
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| "C:\\Program Files".into());
    for base in [Path::new(&pf86).join("Steam"), Path::new(&pf).join("Steam")] {
        let vdf = base.join("steamapps").join("libraryfolders.vdf");
        let Ok(txt) = std::fs::read_to_string(&vdf) else { continue };
        roots.push(base);
        roots.extend(parse_vdf_paths(&txt).into_iter().map(PathBuf::from));
    }
    let mut seen: Vec<PathBuf> = vec![];
    roots.retain(|r| {
        let fresh = !seen.contains(r);
        seen.push(r.clone());
        fresh
    });
    roots
}

pub fn folders(ctx: &SourceCtx) -> Vec<PathBuf> {
    let mut out = vec![];
    for r in steam_roots() {
        out.push(r.join("steamapps").join("workshop").join("content").join(APP_ID));
        out.push(r.join("steamapps").join("common").join("wallpaper_engine").join("projects").join("myprojects"));
    }
    out.extend(ctx.option_paths("folders"));
    out.retain(|d| d.is_dir());
    out
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub type_: String,
    pub action: Action,
    pub note: String,
    pub dir: PathBuf,
    pub file: Option<PathBuf>,
    pub project: PathBuf,
    pub preview: Option<PathBuf>,
}

/// Interpret one project.json found in `dir` (a folder inside a library folder `parent_is_workshop` or not).
pub fn parse_project(dir: &Path, folder_name: &str, workshop: bool, j: &serde_json::Value) -> Entry {
    let type_ = j.get("type").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
    // Paths from project.json must stay inside the item folder (no absolute paths, no "..", no links out).
    let file = j.get("file").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).and_then(|f| join_inside(dir, f));
    let preview = j.get("preview").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).and_then(|f| join_inside(dir, f)).filter(|p| p.is_file());
    let key = format!("we-{}", if workshop { folder_name.to_string() } else { sha1_hex10(&dir.to_string_lossy()) });
    let (mut action, mut note) = (Action::None, String::new());
    if type_ == "video" && file.as_ref().map_or(false, |f| f.exists()) {
        action = Action::Add;
    } else if (type_ == "scene" || type_ == "web") && file.is_some() {
        action = Action::Record;
    } else {
        note = if type_.is_empty() { "Not a wallpaper (preset or dependency)".into() } else { format!("\"{type_}\" wallpapers aren't supported") };
    }
    let title: String = j.get("title").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or(folder_name).chars().take(120).collect();
    Entry { id: key, title, type_: if type_.is_empty() { "other".into() } else { type_ }, action, note, dir: dir.to_path_buf(), file, project: dir.join("project.json"), preview }
}

static SCAN: ScanCache<Entry> = ScanCache::new();

/// `scan`, cached while the library folders are unchanged (30 s at most); `refresh` forces a new scan.
pub fn scan_cached(ctx: &SourceCtx, refresh: bool) -> std::sync::Arc<Vec<Entry>> {
    SCAN.get(&folders(ctx), std::time::Duration::from_secs(30), refresh, || scan(ctx))
}

pub fn scan(ctx: &SourceCtx) -> Vec<Entry> {
    let mut items = vec![];
    for f in folders(ctx) {
        let workshop = f.file_name().map_or(false, |n| n == APP_ID);
        let Ok(rd) = std::fs::read_dir(&f) else { continue };
        for e in rd.flatten() {
            let dir = e.path();
            let Ok(txt) = std::fs::read_to_string(dir.join("project.json")) else { continue };
            let Ok(j) = serde_json::from_str::<serde_json::Value>(txt.trim_start_matches('\u{feff}')) else { continue };
            items.push(parse_project(&dir, &e.file_name().to_string_lossy(), workshop, &j));
        }
    }
    items.sort_by(|a, b| (a.action == Action::None).cmp(&(b.action == Action::None)).then(a.title.to_lowercase().cmp(&b.title.to_lowercase())));
    items
}

fn cmd(exe: &Path) -> Command {
    let mut c = Command::new(exe);
    c.creation_flags(CREATE_NO_WINDOW);
    c
}

/// Path of the running Wallpaper Engine (wallpaper64/32.exe), if any.
pub fn running_exe() -> Option<PathBuf> {
    let out = Command::new("powershell")
        .creation_flags(CREATE_NO_WINDOW)
        .args(["-NoProfile", "-Command", "(Get-Process wallpaper64,wallpaper32 -ErrorAction SilentlyContinue | Select-Object -First 1).Path"])
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!s.is_empty()).then(|| PathBuf::from(s))
}

/// Run a Wallpaper Engine command-line control, giving up after 20 s.
fn run_control(exe: &Path, args: &[&str]) {
    let Ok(mut child) = cmd(exe).args(args).spawn() else { return };
    let end = std::time::Instant::now() + Duration::from_secs(20);
    while std::time::Instant::now() < end {
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
}

/// Closes the temporary window when dropped: runs on success, error, cancel and panic.
struct CloseOnDrop<'a> {
    exe: &'a Path,
    title: &'a str,
}
impl Drop for CloseOnDrop<'_> {
    fn drop(&mut self) {
        run_control(self.exe, &["-control", "closeWallpaper", "-location", self.title]);
    }
}

fn sleep_cancellable(d: Duration, cancel: &Cancel) -> Result<()> {
    let end = std::time::Instant::now() + d;
    while std::time::Instant::now() < end {
        cancel.check()?;
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

/// Automatic recording: at most this long...
pub const AUTO_MAX_S: f32 = 120.0;
/// ...and at least this long (more starts and ends to choose from)...
const AUTO_MIN_S: f32 = 30.0;
/// ...stopping as soon as the frames hold a seamless loop of 3 s to this length (longer loops rarely fit the screen).
const AUTO_LOOP_MAX_S: f64 = 15.0;
/// Longest fixed-length recording.
pub const MAX_S: f32 = 180.0;

/// The automatic recording's stop test: a hard cut that already looks like a normal frame step.
pub fn seamless_loop_in(f: &crate::media::loops::LoopFrames) -> bool {
    use crate::media::loops;
    let none = |_s0: usize| -> crate::media::Result<Vec<Vec<u8>>> { Err(crate::media::MediaError::Invalid("no seeking while recording".into())) };
    let opts = loops::LoopOptions { fps: 18.0, min_len: 3.0, max_len: AUTO_LOOP_MAX_S };
    match loops::find_loops(f, opts, &none, &crate::media::CancelToken::new(), &|_, _| {}) {
        Ok(r) => r.candidates.first().map_or(false, |c| c.blend.unwrap_or(0.0) == 0.0 && c.ratio <= 1.05),
        Err(_) => false,
    }
}

/// Record a scene/web wallpaper to an MP4 in `ctx.tmp_dir`: `seconds` long, or automatic (None: until a seamless loop
/// turns up, between 30 s and 2 min; scenes with random particles never repeat and use the whole 2 min).
pub(crate) fn record(ctx: &SourceCtx, i: &Entry, seconds: Option<f32>, cursor: bool, progress: Progress, cancel: &Cancel) -> Result<AddResult> {
    let _one_at_a_time = RECORDING.lock().unwrap_or_else(|p| p.into_inner());
    let Some(exe) = running_exe() else { return err("Start Wallpaper Engine first (it is needed to play the wallpaper while recording).") };
    let (seconds, early_stop) = match seconds {
        Some(s) => (s.clamp(3.0, MAX_S), None),
        None => (
            AUTO_MAX_S,
            Some(recorder::EarlyStop { min_s: AUTO_MIN_S, every_s: 5.0, check: std::sync::Arc::new(seamless_loop_in) }),
        ),
    };
    let title = format!("CoolerScreen capture {}", sha1_hex10(&format!("{:?}{}", std::time::SystemTime::now(), std::process::id()))[..6].to_string());
    let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    std::fs::create_dir_all(&ctx.tmp_dir)?;
    let out = ctx.tmp_dir.join(format!("{}-{ms}.mp4", i.id));
    progress(0.02, "Opening the wallpaper in its own window");
    let project = i.project.to_string_lossy().to_string();
    let (px, py) = popout_position();
    let (px, py) = (px.to_string(), py.to_string());
    {
        let _close = CloseOnDrop { exe: &exe, title: &title };
        run_control(&exe, &["-control", "openWallpaper", "-file", &project, "-playInWindow", &title, "-width", "2400", "-height", "1080", "-x", &px, "-y", &py, "-borderless"]);
        sleep_cancellable(Duration::from_secs(4), cancel)?; // let it load and start animating
        recorder::record_window(&title, &recorder::Params { seconds, early_stop, fps: 18, cursor, out: out.clone(), ffmpeg: ctx.ffmpeg.clone() }, &|f, m| progress(0.05 + 0.9 * f, m), cancel)?;
    }
    progress(1.0, "");
    Ok(AddResult { file: out, name: format!("{} [{}].mp4", safe_name(&i.title), i.id), temp: true })
}

impl SourceProvider for WallpaperEngine {
    fn id(&self) -> &str {
        "wallpaperengine"
    }
    fn name(&self) -> &str {
        "Wallpaper Engine"
    }
    fn description(&self) -> &str {
        "Your Wallpaper Engine wallpapers (Steam Workshop and your own projects). Video wallpapers are copied; scene and web wallpapers are recorded from a temporary window."
    }
    fn kind(&self) -> Kind {
        Kind::Local
    }
    fn options(&self) -> Vec<OptionDef> {
        vec![OptionDef { key: "folders".into(), label: "Extra wallpaper folders (one per line)".into(), type_: "folders".into() }]
    }
    fn detect(&self, ctx: &SourceCtx) -> Detect {
        if folders(ctx).is_empty() {
            return Detect::no("Not found (no Steam library with Wallpaper Engine content)");
        }
        let items = scan(ctx);
        let rec = items.iter().filter(|i| i.action == Action::Record).count();
        let vid = items.iter().filter(|i| i.action == Action::Add).count();
        Detect::ok(format!("Wallpaper Engine found: {} items ({vid} video, {rec} to record)", items.len()))
    }
    fn list(&self, ctx: &SourceCtx, page: u32, query: Option<&str>, refresh: bool) -> Result<ListPage> {
        let all: Vec<Entry> = scan_cached(ctx, refresh).iter().filter(|e| title_matches(&e.title, query)).cloned().collect();
        let p = page.max(1) as usize;
        let items = all.iter().skip((p - 1) * PAGE).take(PAGE).map(|i| Item {
            id: i.id.clone(), title: i.title.clone(), thumb: i.preview.as_ref().map(|_| format!("thumb:{}", i.id)), type_: i.type_.clone(),
            action: i.action, note: i.note.clone(), key: i.id.clone(), badge: None }).collect();
        Ok(ListPage { items, total: all.len() as u64, has_more: p * PAGE < all.len() })
    }
    fn thumb_path(&self, ctx: &SourceCtx, item_id: &str) -> Option<PathBuf> {
        scan_cached(ctx, false).iter().find(|e| e.id == item_id).and_then(|e| e.preview.clone())
    }
    fn add(&self, ctx: &SourceCtx, item_id: &str, req: &AddRequest, progress: Progress, cancel: &Cancel) -> Result<AddResult> {
        let Some(i) = scan_cached(ctx, false).iter().find(|e| e.id == item_id).cloned().or_else(|| scan_cached(ctx, true).iter().find(|e| e.id == item_id).cloned()) else { return err("This wallpaper is no longer there.") };
        match i.action {
            Action::Add => {
                progress(0.5, "");
                let file = i.file.clone().unwrap();
                let ext = file.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
                Ok(AddResult { name: format!("{} [{}]{ext}", safe_name(&i.title), i.id), file, temp: false })
            }
            Action::Record => record(ctx, &i, req.seconds, false, progress, cancel),
            Action::None => err(if i.note.is_empty() { "This wallpaper type is not supported.".to_string() } else { i.note }),
        }
    }
}

/// Where to open the pop-out so the pointer is unlikely to hover it (scenes with mouse parallax react to it): the
/// bottom-right corner of the whole desktop, so only ~100 px of the window is on a visible monitor.
/// `COOLERSCREEN_POPOUT_POS="x,y"` overrides (tests).
pub fn popout_position() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::*;
    if let Ok(s) = std::env::var("COOLERSCREEN_POPOUT_POS") {
        if let Some((a, b)) = s.split_once(',') {
            if let (Ok(x), Ok(y)) = (a.trim().parse(), b.trim().parse()) {
                return (x, y);
            }
        }
    }
    unsafe {
        let (l, t) = (GetSystemMetrics(SM_XVIRTUALSCREEN), GetSystemMetrics(SM_YVIRTUALSCREEN));
        let (w, h) = (GetSystemMetrics(SM_CXVIRTUALSCREEN), GetSystemMetrics(SM_CYVIRTUALSCREEN));
        (l + w - 100, t + h - 100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vdf() {
        let t = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        assert_eq!(parse_vdf_paths(t), vec!["C:\\Program Files (x86)\\Steam".to_string(), "D:\\SteamLibrary".to_string()]);
    }
    #[test]
    fn projects() {
        let root = std::env::temp_dir().join(format!("cs-we-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let v = root.join("111");
        let s = root.join("222");
        let p = root.join("333");
        for d in [&v, &s, &p] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(v.join("a.mp4"), b"x").unwrap();
        std::fs::write(v.join("project.json"), "{\"title\":\"Vid\",\"type\":\"Video\",\"file\":\"a.mp4\",\"preview\":\"nope.jpg\"}").unwrap();
        std::fs::write(s.join("project.json"), "{\"title\":\"Sce\",\"type\":\"scene\",\"file\":\"scene.json\"}").unwrap();
        std::fs::write(p.join("project.json"), "{\"title\":\"App\",\"type\":\"application\",\"file\":\"x.exe\"}").unwrap();
        let ctx = SourceCtx { options: serde_json::json!({ "folders": [root.to_string_lossy()] }), tmp_dir: root.clone(), data_dir: root.clone(), sources_dir: root.clone(), ffmpeg: "ffmpeg".into() };
        let items: Vec<Entry> = scan(&ctx).into_iter().filter(|e| e.dir.starts_with(&root)).collect();
        assert_eq!(items.len(), 3);
        assert_eq!(items.iter().find(|e| e.title == "Vid").unwrap().action, Action::Add);
        assert_eq!(items.iter().find(|e| e.title == "Sce").unwrap().action, Action::Record);
        let app = items.iter().find(|e| e.title == "App").unwrap();
        assert_eq!(app.action, Action::None);
        assert!(app.note.contains("application"));
        assert!(items.iter().all(|e| e.preview.is_none()));
        let r = WallpaperEngine.add(&ctx, &items.iter().find(|e| e.title == "Vid").unwrap().id, &AddRequest::default(), &|_, _| {}, &Cancel::new()).unwrap();
        assert!(r.name.starts_with("Vid [we-") && r.name.ends_with(".mp4") && !r.temp);
        let _ = std::fs::remove_dir_all(&root);
    }
}
