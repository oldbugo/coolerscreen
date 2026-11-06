//! The core application object: library, settings, scheduler, device, media/sources backends and events.
//! Tauri commands (core/commands.rs) are thin wrappers over `App` methods (core/service.rs) so everything can be
//! tested without a window.
use super::api::{Backends, MediaEngine, MediaTask, ProbeInfo, SourceCatalog};
use super::device_service::DeviceService;
use super::error::{CoreError, CoreResult};
use super::gallery::GalleryState;
use super::library::Library;
use super::scheduler::{Job, JobSink, Limits, Policy, Priority, Scheduler, TaskCtx, TaskSpec};
use super::settings::SettingsStore;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

/// What the UI is told (Tauri events in the app, a recorder in tests).
pub trait Events: Send + Sync + 'static {
    fn job(&self, _job: &Job) {}
    /// Originals/versions changed (debounced).
    fn library(&self, _rev: u64) {}
    /// Settings / slot records / capacity / gallery folder changed: the UI reloads `app_state` (debounced).
    fn app_changed(&self) {}
    fn device_busy(&self, _busy: bool) {}
}

pub struct NoEvents;
impl Events for NoEvents {}

struct SinkAdapter(Arc<dyn Events>, Arc<super::live_host::LiveHost>);
impl JobSink for SinkAdapter {
    fn job(&self, job: &Job) {
        self.0.job(job)
    }
    fn device_busy(&self, busy: bool) {
        // Live (prototype B) steps aside first, so the device task finds the screen free.
        self.1.device_busy(busy);
        self.0.device_busy(busy)
    }
}

/// Calls `f` at most once per `delay` after a burst of `poke`s.
pub struct Debounce {
    pending: AtomicBool,
    delay: Duration,
    f: Box<dyn Fn() + Send + Sync>,
}

impl Debounce {
    pub fn new(delay: Duration, f: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Debounce { pending: AtomicBool::new(false), delay, f: Box::new(f) })
    }
    pub fn poke(self: &Arc<Self>) {
        if self.pending.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(me.delay);
            me.pending.store(false, Ordering::SeqCst);
            (me.f)();
        });
    }
}

pub struct App {
    pub lib: Arc<Library>,
    pub settings: Arc<SettingsStore>,
    pub sched: Scheduler,
    pub media: Arc<dyn MediaEngine>,
    pub sources: Arc<dyn SourceCatalog>,
    pub events: Arc<dyn Events>,
    pub device: DeviceService,
    /// 3.0 prototype B: the live companion engine hosted in this process (idle unless started).
    pub live: Arc<super::live_host::LiveHost>,
    pub(crate) gallery: GalleryState,
    pub app_version: String,
    pub media_base: Mutex<String>,
    /// Originals whose loop cards job has not finished (the media server waits for those files).
    pub cards_pending: Mutex<std::collections::HashMap<String, String>>,
    /// COOLERSCREEN_GALLERY, read once by `App::new` (tests build the App with `with_device` and never see it).
    pub gallery_env: Mutex<Option<PathBuf>>,
    pub(crate) me: Mutex<Weak<App>>,
    /// The last screen info a check answered (shown while live holds the screen, instead of opening it a second time).
    pub(crate) last_info: Mutex<Option<serde_json::Value>>,
    pub(crate) brightness_requested:Mutex<Option<u8>>,
}

impl App {
    /// `base` = app folder (%APPDATA%\CoolerScreen or COOLERSCREEN_HOME).
    pub fn new(base: impl Into<PathBuf>, backends: Backends, events: Arc<dyn Events>, app_version: &str) -> Arc<App> {
        let base = base.into();
        let mut device = DeviceService::from_env(&base);
        if backends.media.is_fake() && device.fake_mode().is_none() {
            // Fake media makes placeholder files: they must never reach the real screen.
            eprintln!("Fake media engine: using the fake screen too.");
            device = DeviceService::fake("winusb", 1.0, false);
        }
        let app = Self::with_device(base, backends, events, app_version, device);
        *app.gallery_env.lock().unwrap() = std::env::var_os("COOLERSCREEN_GALLERY").map(PathBuf::from);
        app
    }

    pub fn with_device(base: PathBuf, backends: Backends, events: Arc<dyn Events>, app_version: &str, device: DeviceService) -> Arc<App> {
        assert!(!backends.media.is_fake() || device.fake_mode().is_some(), "fake media requires the fake device");
        let settings = Arc::new(SettingsStore::new(base.join("settings.json")));
        let lib = Arc::new(Library::new(&base, settings.clone()));
        lib.set_splash_version(backends.media.splash_version());
        // COOLERSCREEN_BLOCK_MEDIA_DURING_UPLOAD=1: 1.2's full block (fallback if uploads stall on hardware).
        let policy = if std::env::var("COOLERSCREEN_BLOCK_MEDIA_DURING_UPLOAD").is_ok_and(|v| v == "1") { Policy::blocking() } else { Policy::default() };
        let live = super::live_host::LiveHost::new();
        let sched = Scheduler::new(Limits::default(), policy, Arc::new(SinkAdapter(events.clone(), live.clone())));
        let app = Arc::new(App {
            lib: lib.clone(),
            settings: settings.clone(),
            sched,
            media: backends.media,
            sources: backends.sources,
            events: events.clone(),
            device,
            live,
            gallery: GalleryState::default(),
            app_version: app_version.to_string(),
            media_base: Mutex::new(String::new()),
            cards_pending: Mutex::new(Default::default()),
            gallery_env: Mutex::new(None),
            me: Mutex::new(Weak::new()),
            last_info: Mutex::new(None),
            brightness_requested:Mutex::new(None),
        });
        *app.me.lock().unwrap() = Arc::downgrade(&app);
        let ev = events.clone();
        let l2 = Arc::downgrade(&lib);
        let lib_deb = Debounce::new(Duration::from_millis(100), move || {
            if let Some(l) = l2.upgrade() {
                ev.library(l.rev());
            }
        });
        lib.set_on_change(move |_| lib_deb.poke());
        let ev = events.clone();
        let app_deb = Debounce::new(Duration::from_millis(100), move || ev.app_changed());
        settings.set_on_change(move || app_deb.poke());
        // The live host tells the page (and the tray) when its idle state or the live items of the theme change.
        let ev = events.clone();
        app.live.set_on_change(move || ev.app_changed());
        let weak = Arc::downgrade(&app);
        app.live.set_on_recovery(move || {
            weak.upgrade().is_some_and(|app| app.recover_live_no_reply())
        });
        app
    }

    pub fn arc(&self) -> Arc<App> {
        self.me.lock().unwrap().upgrade().expect("App alive")
    }

    pub fn set_media_base(&self, base: &str) {
        *self.media_base.lock().unwrap() = base.to_string();
        self.lib.set_media_base(base);
    }

    /// Run a task through the scheduler and wait for it (on the calling thread).
    pub fn run_task(&self, spec: TaskSpec, f: impl Fn(&TaskCtx) -> CoreResult<Value> + Send + Sync + 'static) -> CoreResult<Value> {
        self.sched.submit(spec, f).wait()
    }

    /// Probe a file in the `io` class (used by gallery registration).
    pub fn probe(&self, file: &Path, priority: Priority) -> CoreResult<ProbeInfo> {
        let media = self.media.clone();
        let f = file.to_path_buf();
        let res = media.resources(MediaTask::Probe, None);
        let v = self.run_task(TaskSpec::new("probe", "Reading the file", res).priority(priority), move |ctx| {
            let i = media.probe(ctx, &f)?;
            serde_json::to_value(i).map_err(|e| CoreError::new(e.to_string()))
        })?;
        serde_json::from_value(v).map_err(|e| CoreError::new(e.to_string()))
    }

    pub fn settings_changed(&self) {
        self.events.app_changed();
    }
}
