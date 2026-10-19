//! 4.0 G2: the frame rate of the game in front (docs/v4/notes/B-game-fps.md). No injection, nothing installed or elevated by us.
//! Sources, in order: (1) our own ETW present counter (what Intel PresentMon does; needs the user in "Performance Log Users" once),
//! (2) RivaTuner Statistics Server shared memory if the user runs RTSS, (3) HWiNFO frame-rate sensors if HWiNFO shares them.
//! Parts: `counter` (pure frame-time statistics), `etw` (the Windows event session), `rtss` (shared-memory reader), `foreground`
//! (which program is in front, group check), this file (the sampler thread, source choice, reason lines, the widget props).
pub mod counter;
pub mod etw;
pub mod foreground;
pub mod rtss;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::theme::Widget;

// ---------------------------------------------------------------- widget props

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GameFpsProps {
    /// Show the 1 % low next to the big number.
    pub low: bool,
    /// Show the frame-time graph (history grows sideways).
    pub graph: bool,
    /// Seconds of frame-time history in the graph (10..120).
    pub history: f32,
    /// true = draw nothing at all while no game is in front (otherwise a dim "No game in front").
    pub hide_idle: bool,
    pub color: String,
}
impl Default for GameFpsProps {
    fn default() -> Self {
        GameFpsProps { low: true, graph: true, history: 30.0, hide_idle: false, color: "#5ac8ff".into() }
    }
}
impl Widget {
    pub fn gamefps(&self) -> GameFpsProps {
        serde_json::from_value(self.props.clone()).unwrap_or_default()
    }
}

/// Bound every prop of the item (called by `morewidgets::clean`).
pub fn clean(o: &mut Map<String, Value>) {
    use super::morewidgets::{boolean, colour, number};
    boolean(o, "low", true);
    boolean(o, "graph", true);
    number(o, "history", 30.0, 10.0, 120.0);
    boolean(o, "hideIdle", false);
    colour(o, "color", "#5ac8ff", false);
}

// ---------------------------------------------------------------- what the widget reads

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum State {
    /// A game is in front and a number is available.
    Ok,
    /// Nothing that presents frames is in front (or the desktop is).
    NoGame,
    /// Not allowed to read Windows' graphics events: the user is not in "Performance Log Users" yet.
    NeedsGroup,
    /// In the group but the session could not be started (an anti-cheat blocked it, or another program holds it).
    Blocked,
    /// No source at all (see the message).
    Unavailable,
}

#[derive(Clone, Debug)]
pub struct View {
    pub state: State,
    /// The program's name ("MonsterHunterWilds.exe" without the .exe).
    pub game: String,
    pub fps: Option<f32>,
    /// 1 % low in fps (the speed of the slowest 1 % of frames).
    pub low1: Option<f32>,
    pub frame_ms: Option<f32>,
    /// (age in seconds, frame time in ms), oldest first.
    pub hist: Vec<(f32, f32)>,
    /// "etw" | "rtss" | "hwinfo" | "".
    pub source: &'static str,
}
impl View {
    pub fn idle(state: State) -> View {
        View { state, game: String::new(), fps: None, low1: None, frame_ms: None, hist: Vec::new(), source: "" }
    }
}

#[derive(Clone, Debug)]
pub struct Status {
    pub ok: bool,
    /// "ok" | "noGame" | "needsGroup" | "blocked" | "unavailable"
    pub state: &'static str,
    pub message: String,
    /// The exact command the user runs once (elevated PowerShell), only for `needsGroup`.
    pub command: Option<String>,
}

// ---------------------------------------------------------------- the sampler

/// The sampler ends this long after the last widget draw (a hidden widget costs nothing; our event session is stopped with it).
const KEEP: Duration = Duration::from_secs(30);
/// How often the sampler looks (the widget redraws at most 10 times a second).
const TICK: Duration = Duration::from_millis(250);
/// A program in front counts as a game when it covers its monitor, or shows at least this many frames per second.
const GAME_MIN_FPS: f32 = 20.0;
/// Frame-time history kept for the graph (the widget draws up to its own `history`).
const HIST_SECS: f32 = 120.0;

type SharedCounter = Arc<Mutex<counter::Counter>>;

struct Shared {
    view: View,
    last_used: Option<Instant>,
    running: bool,
    /// Why the event session is not there (None = it is, or not tried yet).
    etw_err: Option<etw::EtwError>,
    stop: bool,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared { view: View::idle(State::NoGame), last_used: None, running: false, etw_err: None, stop: false }))
}

/// Our event session lives here so `shutdown()` can stop it from the main thread.
fn session() -> &'static Mutex<Option<etw::EtwSession>> {
    static S: OnceLock<Mutex<Option<etw::EtwSession>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(None))
}

/// The latest numbers (registers the demand; starts the sampler thread when needed). Never blocks the render thread.
pub fn read() -> View {
    let mut g = shared().lock().unwrap();
    g.last_used = Some(Instant::now());
    if !g.running && !g.stop {
        g.running = true;
        std::thread::spawn(run);
    }
    g.view.clone()
}

/// Stop our event session and the sampler (called when CoolerScreen quits). Real-time sessions outlive their process, so this matters.
pub fn shutdown() {
    shared().lock().unwrap().stop = true;
    if let Some(mut s) = session().lock().unwrap().take() {
        s.stop();
    }
}

fn is_ignored_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "dwm" || n == "csrss" || n == "system" || n == "explorer" || n == "searchhost" || n == "startmenuexperiencehost"
}

fn run() {
    let counter: SharedCounter = Arc::new(Mutex::new(counter::Counter::new()));
    let own_pid = std::process::id();
    let mut last_try: Option<Instant> = None;
    let mut fg_since: Option<(u32, Instant)> = None;
    loop {
        {
            let g = shared().lock().unwrap();
            if g.stop || !g.last_used.is_some_and(|t| t.elapsed() < KEEP) {
                break;
            }
        }
        // 1. our own event session: created EMPTY first (so it exists before an anti-cheat starts), then the providers are enabled
        let have = session().lock().unwrap().is_some();
        if !have {
            let wait = match shared().lock().unwrap().etw_err {
                Some(etw::EtwError::AccessDenied) => Duration::from_secs(15),
                Some(_) => Duration::from_secs(5),
                None => Duration::ZERO,
            };
            if last_try.is_none_or(|t| t.elapsed() >= wait) {
                last_try = Some(Instant::now());
                match etw::EtwSession::start(counter.clone()) {
                    Ok(s) => {
                        let enabled = s.enable_providers();
                        *session().lock().unwrap() = Some(s);
                        shared().lock().unwrap().etw_err = enabled.err();
                    }
                    Err(e) => shared().lock().unwrap().etw_err = Some(e),
                }
            }
        }
        // 2. what is in front, and its numbers from the best source we have
        let fg = foreground::current().filter(|f| f.pid != own_pid && !is_ignored_name(&f.name));
        match &fg {
            Some(f) if fg_since.is_none_or(|(p, _)| p != f.pid) => fg_since = Some((f.pid, Instant::now())),
            None => fg_since = None,
            _ => {}
        }
        let now_ns = etw::EtwSession::now_ns();
        let have = session().lock().unwrap().is_some();
        let mut view: Option<View> = None;
        if have {
            if let Some(f) = &fg {
                let c = counter.lock().unwrap();
                if let Some(st) = c.stats(f.pid, now_ns) {
                    if f.fullscreen || st.fps >= GAME_MIN_FPS {
                        view = Some(View { state: State::Ok, game: f.name.clone(), fps: Some(st.fps), low1: st.low1, frame_ms: Some(st.frame_ms), hist: c.history(f.pid, now_ns, HIST_SECS), source: "etw" });
                    }
                }
            }
        }
        if view.is_none() {
            if let Some(f) = &fg {
                if let Some(r) = rtss::read(Some(f.pid)) {
                    if r.fps > 0.0 && !is_ignored_name(&r.name) {
                        view = Some(View { state: State::Ok, game: if r.name.is_empty() { f.name.clone() } else { r.name.clone() }, fps: Some(r.fps), low1: None, frame_ms: Some(r.frame_ms), hist: Vec::new(), source: "rtss" });
                    }
                }
                if view.is_none() && (f.fullscreen || have) {
                    if let Some((fps, ms)) = super::extsensors::game_fps() {
                        if fps > 0.0 {
                            view = Some(View { state: State::Ok, game: f.name.clone(), fps: Some(fps), low1: None, frame_ms: ms, hist: Vec::new(), source: "hwinfo" });
                        }
                    }
                }
            }
        }
        let view = view.unwrap_or_else(|| {
            let err = shared().lock().unwrap().etw_err.clone();
            match (&fg, have, err) {
                // no event session and nothing else answered: say why
                (_, false, Some(etw::EtwError::AccessDenied)) => View::idle(if foreground::in_performance_log_users() { State::Blocked } else { State::NeedsGroup }),
                (_, false, Some(_)) => View::idle(State::Unavailable),
                // a full-screen program has been in front for a while and the session saw no frames of it at all
                (Some(f), true, _) if f.fullscreen && fg_since.is_some_and(|(p, t)| p == f.pid && t.elapsed() > Duration::from_secs(6)) && counter.lock().unwrap().stats(f.pid, now_ns).is_none() => View::idle(State::Blocked),
                _ => View::idle(State::NoGame),
            }
        });
        shared().lock().unwrap().view = view;
        std::thread::sleep(TICK);
    }
    // nobody asked for a while: stop our session too (it is created again on the next draw)
    if let Some(mut s) = session().lock().unwrap().take() {
        s.stop();
    }
    let mut g = shared().lock().unwrap();
    g.running = false;
    g.view = View::idle(State::NoGame);
    g.etw_err = None;
}

// ---------------------------------------------------------------- reasons in words (the editor shows them)

/// The one-time command for the user (elevated PowerShell), with their own account name.
pub fn group_command() -> String {
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "%USERNAME%".into());
    format!("net localgroup \"Performance Log Users\" \"{user}\" /add")
}

pub fn status_for(state: State, source: &str, game: &str) -> Status {
    match state {
        State::Ok => Status { ok: true, state: "ok", message: format!("Reading {game} from {}.", match source { "etw" => "Windows graphics events", "rtss" => "RivaTuner (RTSS)", "hwinfo" => "HWiNFO", _ => "the PC" }), command: None },
        State::NoGame => Status { ok: true, state: "noGame", message: "No game in front.".into(), command: None },
        State::NeedsGroup => Status {
            ok: false,
            state: "needsGroup",
            message: "Needs a one-time Windows setting: your account must join the \u{201C}Performance Log Users\u{201D} group before CoolerScreen can read game frame rates.".into(),
            command: Some(group_command()),
        },
        State::Blocked => Status { ok: false, state: "blocked", message: "Blocked while the game runs: restart CoolerScreen before the game.".into(), command: None },
        State::Unavailable => Status { ok: false, state: "unavailable", message: "Frame rate unavailable: Windows would not start the event session.".into(), command: None },
    }
}

/// Why the number is (not) available, in words for the editor. Looks at the sampler's last view; when nothing is running (Live off) it
/// only checks the group, it never starts a session.
pub fn status() -> Status {
    let (running, view, err) = {
        let g = shared().lock().unwrap();
        (g.running, g.view.clone(), g.etw_err.clone())
    };
    if running {
        return status_for(view.state, view.source, &view.game);
    }
    let _ = err;
    if foreground::may_start_event_sessions() {
        Status { ok: true, state: "ready", message: "Ready. The frame rate of the game in front shows while Live is on.".into(), command: None }
    } else {
        status_for(State::NeedsGroup, "", "")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_are_cleaned() {
        let mut o = serde_json::json!({ "history": 1000, "color": "red", "low": "x" }).as_object().unwrap().clone();
        clean(&mut o);
        assert_eq!(o["history"], 120.0);
        assert_eq!(o["color"], "#5ac8ff");
        assert_eq!(o["low"], true);
        let again = o.clone();
        clean(&mut o);
        assert_eq!(o, again);
    }

    #[test]
    fn reasons_are_in_plain_words() {
        let s = status_for(State::NeedsGroup, "", "");
        assert!(!s.ok && s.message.starts_with("Needs a one-time Windows setting"));
        let c = s.command.unwrap();
        assert!(c.starts_with("net localgroup \"Performance Log Users\" \"") && c.ends_with("\" /add"), "{c}");
        assert!(status_for(State::Blocked, "", "").message.contains("restart CoolerScreen before the game"));
        assert_eq!(status_for(State::NoGame, "", "").message, "No game in front.");
        assert!(status_for(State::Ok, "etw", "Game").message.contains("Game"));
    }

    #[test]
    fn ignored_programs() {
        assert!(is_ignored_name("dwm") && is_ignored_name("DWM") && is_ignored_name("explorer"));
        assert!(!is_ignored_name("MonsterHunterWilds"));
    }
}
