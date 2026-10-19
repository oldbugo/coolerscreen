//! Now playing: title, artist, cover, progress and play state from Windows' media sessions (GSMTC: Spotify, browsers,
//! Media Player, VLC, ...). A worker thread on the multithreaded apartment waits for session events (new track, play/pause,
//! session change) with a 1 s poll as a safety net. Properties are re-read about 400 ms after a track change (players
//! publish title and cover a moment apart); the cover is decoded once per track. The position is extrapolated while playing.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};

/// A decoded cover, straight RGBA.
#[derive(Debug)]
pub struct Cover {
    pub w: usize,
    pub h: usize,
    pub rgba: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct NowPlaying {
    pub app: String,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    /// Position (seconds) when `read_at` was taken; use [`NowPlaying::position`].
    pos_s: f64,
    pub duration_s: f64,
    read_at: Instant,
    /// When play/pause last changed (a paused player keeps its panel for a minute).
    pub state_since: Instant,
    pub cover: Option<Arc<Cover>>,
    /// Changes when the track changes (renderer re-scales the cover then).
    pub track_id: u64,
}

impl NowPlaying {
    pub fn position(&self) -> f64 {
        let p = if self.playing { self.pos_s + self.read_at.elapsed().as_secs_f64() } else { self.pos_s };
        if self.duration_s > 0.0 { p.min(self.duration_s) } else { p }
    }

    /// Should the panel be on screen: playing, or paused for less than a minute.
    pub fn visible(&self) -> bool {
        !self.title.is_empty() && (self.playing || self.state_since.elapsed() < Duration::from_secs(60))
    }
}

pub struct MusicWatcher {
    pub data: Arc<Mutex<Option<NowPlaying>>>,
    preferred: Arc<Mutex<Option<String>>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MusicWatcher {
    pub fn start(preferred_app: Option<String>) -> MusicWatcher {
        let data = Arc::new(Mutex::new(None));
        let preferred = Arc::new(Mutex::new(preferred_app));
        let stop = Arc::new(AtomicBool::new(false));
        let (d2, p2, s2) = (data.clone(), preferred.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            if let Err(e) = run(d2, p2, s2) {
                crate::device::log(&format!("companion: now playing unavailable: {e}"));
            }
        });
        MusicWatcher { data, preferred, stop, thread: Some(thread) }
    }

    pub fn set_preferred(&self, app: Option<String>) {
        *self.preferred.lock().unwrap() = app;
        if let Some(t) = &self.thread {
            t.thread().unpark();
        }
    }

    pub fn get(&self) -> Option<NowPlaying> {
        self.data.lock().unwrap().clone()
    }
}

impl Drop for MusicWatcher {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            t.thread().unpark();
            let _ = t.join();
        }
    }
}

fn run(data: Arc<Mutex<Option<NowPlaying>>>, preferred: Arc<Mutex<Option<String>>>, stop: Arc<AtomicBool>) -> windows::core::Result<()> {
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    unsafe { RoInitialize(RO_INIT_MULTITHREADED)? };
    let mgr = Manager::RequestAsync()?.join()?;
    let me = std::thread::current();
    // Events only wake the loop; every wake re-reads everything (cheap), so handlers stay trivial.
    let wake = {
        let me = me.clone();
        move || me.unpark()
    };
    let w = wake.clone();
    let _ = mgr.CurrentSessionChanged(&windows::Foundation::TypedEventHandler::new(move |_, _| {
        w();
        Ok(())
    }));
    let mut hooked: Option<String> = None;
    let mut track_key = String::new();
    let mut track_id = 0u64;
    let mut cover: Option<Arc<Cover>> = None;
    let mut cover_tries = 0;
    let mut state_playing: Option<bool> = None;
    let mut state_since = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let pref = preferred.lock().unwrap().clone();
        let session = pick(&mgr, pref.as_deref());
        match session {
            None => {
                *data.lock().unwrap() = None;
                track_key.clear();
            }
            Some(s) => {
                let app = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
                if hooked.as_deref() != Some(app.as_str()) {
                    let (w1, w2, w3) = (wake.clone(), wake.clone(), wake.clone());
                    let _ = s.MediaPropertiesChanged(&windows::Foundation::TypedEventHandler::new(move |_, _| {
                        w1();
                        Ok(())
                    }));
                    let _ = s.PlaybackInfoChanged(&windows::Foundation::TypedEventHandler::new(move |_, _| {
                        w2();
                        Ok(())
                    }));
                    let _ = s.TimelinePropertiesChanged(&windows::Foundation::TypedEventHandler::new(move |_, _| {
                        w3();
                        Ok(())
                    }));
                    hooked = Some(app.clone());
                }
                if let Ok(np) = read(&s, &app) {
                    let key = format!("{}|{}|{}", np.app, np.title, np.artist);
                    if key != track_key {
                        // New track: let the player finish publishing (title and cover arrive a moment apart), then re-read.
                        std::thread::sleep(Duration::from_millis(400));
                        let np2 = read(&s, &app).unwrap_or_else(|_| np.clone());
                        track_key = format!("{}|{}|{}", np2.app, np2.title, np2.artist);
                        track_id += 1;
                        cover = decode_cover(&s);
                        cover_tries = 0;
                        state_playing = None;
                    } else if cover.is_none() && cover_tries < 6 {
                        cover_tries += 1;
                        cover = decode_cover(&s);
                    }
                    let mut np = read(&s, &app).unwrap_or_else(|_| np.clone());
                    if state_playing != Some(np.playing) {
                        state_playing = Some(np.playing);
                        state_since = Instant::now();
                    }
                    np.state_since = state_since;
                    np.cover = cover.clone();
                    np.track_id = track_id;
                    *data.lock().unwrap() = Some(np);
                }
            }
        }
        std::thread::park_timeout(Duration::from_secs(1));
    }
    Ok(())
}

/// Which candidate to follow, as `(AppUserModelId, playing, is Windows' current session)`: the preferred app (substring,
/// case-insensitive) if present; else Windows' current session if it is playing; else the first playing one; else the current one.
fn choose(cands: &[(String, bool, bool)], preferred: Option<&str>) -> Option<usize> {
    if let Some(p) = preferred.filter(|p| !p.is_empty()) {
        let p = p.to_lowercase();
        if let Some(i) = cands.iter().position(|c| c.0.to_lowercase().contains(&p)) {
            return Some(i);
        }
    }
    cands
        .iter()
        .position(|c| c.2 && c.1)
        .or_else(|| cands.iter().position(|c| c.1))
        .or_else(|| cands.iter().position(|c| c.2))
}

fn pick(mgr: &Manager, preferred: Option<&str>) -> Option<Session> {
    let list = mgr.GetSessions().ok()?;
    let current = mgr.GetCurrentSession().ok();
    let cur_id = current.as_ref().and_then(|s| s.SourceAppUserModelId().ok()).map(|h| h.to_string());
    let mut sessions = Vec::new();
    let mut cands = Vec::new();
    for i in 0..list.Size().ok()? {
        let Ok(s) = list.GetAt(i) else { continue };
        let id = s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default();
        let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).is_ok_and(|st| st == Status::Playing);
        cands.push((id.clone(), playing, cur_id.as_deref() == Some(id.as_str())));
        sessions.push(s);
    }
    match choose(&cands, preferred) {
        Some(i) => sessions.into_iter().nth(i),
        None => current,
    }
}

fn read(s: &Session, app: &str) -> windows::core::Result<NowPlaying> {
    let props = s.TryGetMediaPropertiesAsync()?.join()?;
    let status = s.GetPlaybackInfo()?.PlaybackStatus()?;
    let tl = s.GetTimelineProperties()?;
    let secs = |t: windows::Foundation::TimeSpan| t.Duration as f64 / 1e7;
    let (start, end, pos) = (secs(tl.StartTime()?), secs(tl.EndTime()?), secs(tl.Position()?));
    let mut pos_now = pos - start;
    if status == Status::Playing {
        // Position is as of LastUpdatedTime: bring it up to now.
        if let Ok(upd) = tl.LastUpdatedTime() {
            let now_ft = filetime_now();
            let age = (now_ft - upd.UniversalTime) as f64 / 1e7;
            if age > 0.0 && age < 3600.0 {
                pos_now += age;
            }
        }
    }
    Ok(NowPlaying {
        app: app.to_string(),
        title: props.Title()?.to_string(),
        artist: props.Artist()?.to_string(),
        playing: status == Status::Playing,
        pos_s: pos_now.max(0.0),
        duration_s: (end - start).max(0.0),
        read_at: Instant::now(),
        state_since: Instant::now(),
        cover: None,
        track_id: 0,
    })
}

/// 100 ns ticks since 1601, the epoch GSMTC timestamps use.
fn filetime_now() -> i64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::SystemInformation::GetSystemTimeAsFileTime;
    let mut f = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    unsafe { GetSystemTimeAsFileTime(&mut f) };
    (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as i64
}

/// Decode the session's thumbnail (any size) to RGBA, scaled so its longer side is at most 320 px.
fn decode_cover(s: &Session) -> Option<Arc<Cover>> {
    use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode, ExifOrientationMode};
    let props = s.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
    let thumb = props.Thumbnail().ok()?;
    let stream = thumb.OpenReadAsync().ok()?.join().ok()?;
    let dec = BitmapDecoder::CreateAsync(&stream).ok()?.join().ok()?;
    let (w, h) = (dec.PixelWidth().ok()? as f64, dec.PixelHeight().ok()? as f64);
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let k = (320.0 / w.max(h)).min(1.0);
    let (tw, th) = (((w * k).round() as u32).max(1), ((h * k).round() as u32).max(1));
    let tr = BitmapTransform::new().ok()?;
    tr.SetScaledWidth(tw).ok()?;
    tr.SetScaledHeight(th).ok()?;
    let px = dec
        .GetPixelDataTransformedAsync(BitmapPixelFormat::Rgba8, BitmapAlphaMode::Straight, &tr, ExifOrientationMode::IgnoreExifOrientation, ColorManagementMode::DoNotColorManage)
        .ok()?
        .join()
        .ok()?;
    let data = px.DetachPixelData().ok()?;
    if data.len() != (tw * th * 4) as usize {
        return None;
    }
    Some(Arc::new(Cover { w: tw as usize, h: th as usize, rgba: data.to_vec() }))
}

impl NowPlaying {
    /// A made-up track for layout checks (dry run) and tests: a gradient cover, playing, at `pos_s` of `duration_s`.
    pub fn demo(title: &str, artist: &str, playing: bool, pos_s: f64, duration_s: f64) -> NowPlaying {
        let (w, h) = (320usize, 320usize);
        let mut rgba = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 4;
                rgba[i] = (40 + x * 200 / w) as u8;
                rgba[i + 1] = (90 + y * 120 / h) as u8;
                rgba[i + 2] = (220 - (x + y) * 150 / (w + h)) as u8;
            }
        }
        NowPlaying {
            app: "demo".into(),
            title: title.into(),
            artist: artist.into(),
            playing,
            pos_s,
            duration_s,
            read_at: Instant::now(),
            state_since: Instant::now(),
            cover: Some(Arc::new(Cover { w, h, rgba })),
            track_id: 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::choose;

    fn c(id: &str, playing: bool, current: bool) -> (String, bool, bool) {
        (id.to_string(), playing, current)
    }

    #[test]
    fn preferred_wins_even_when_paused() {
        let l = [c("Spotify.exe", true, true), c("chrome.exe", false, false)];
        assert_eq!(choose(&l, Some("CHROME")), Some(1));
        assert_eq!(choose(&l, Some("")), Some(0));
        assert_eq!(choose(&l, Some("vlc")), Some(0));
    }

    #[test]
    fn two_playing_the_current_one_wins() {
        let l = [c("a", true, false), c("b", true, true)];
        assert_eq!(choose(&l, None), Some(1));
    }

    #[test]
    fn current_paused_other_playing_the_playing_one() {
        let l = [c("a", false, true), c("b", true, false)];
        assert_eq!(choose(&l, None), Some(1));
    }

    #[test]
    fn none_playing_the_current_one() {
        let l = [c("a", false, false), c("b", false, true)];
        assert_eq!(choose(&l, None), Some(1));
        assert_eq!(choose(&[c("a", false, false)], None), None);
    }

    #[test]
    fn empty_is_none() {
        assert_eq!(choose(&[], None), None);
        assert_eq!(choose(&[], Some("x")), None);
    }
}
