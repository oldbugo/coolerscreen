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

// ---------------------------------------------------------------- the sampler (filled in by the orchestration step)

struct Shared {
    view: View,
    last_used: Option<Instant>,
    running: bool,
}

fn shared() -> &'static Mutex<Shared> {
    static S: OnceLock<Mutex<Shared>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(Shared { view: View::idle(State::NoGame), last_used: None, running: false }))
}

#[allow(dead_code)]
const KEEP: Duration = Duration::from_secs(30);
#[allow(dead_code)]
type SharedCounter = Arc<Mutex<counter::Counter>>;

/// The latest numbers (registers the demand; starts the sampler thread when needed). Never blocks the render thread.
pub fn read() -> View {
    let mut g = shared().lock().unwrap();
    g.last_used = Some(Instant::now());
    g.view.clone()
}

/// Why the number is (not) available, in words for the editor. Never starts anything.
pub fn status() -> Status {
    Status { ok: false, state: "unavailable", message: "Game FPS is not built yet.".into(), command: None }
}

/// Stop our event session and the sampler (called when CoolerScreen quits).
pub fn shutdown() {}
