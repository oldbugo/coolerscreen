//! Adapts WS3's providers (`crate::sources`, their own trait) to core's `SourceProvider` / `SourceCatalog`
//! (docs/v2/api-contract.md 5.2). WS3's code stays as it is; this file does the mapping:
//! context (adds data_dir / sources_dir / ffmpeg), items, progress and cancellation (TaskCtx → `sources::Cancel`),
//! and JSON plug-ins, which are listed by file name and only parsed when enabled.
use super::api::{AddOpts, AddedFile, Detect, ItemPage, SourceCatalog, SourceCtx, SourceItem, SourceProvider};
use super::error::{CoreError, CoreResult};
use super::scheduler::TaskCtx;
use crate::sources as ws3;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone)]
pub struct Env {
    /// App folder (%APPDATA%\CoolerScreen): list caches.
    pub data_dir: PathBuf,
    /// Documents\CoolerScreen\Sources: JSON plug-ins.
    pub sources_dir: PathBuf,
    pub ffmpeg: PathBuf,
}

fn ctx3(env: &Env, c: &SourceCtx) -> ws3::SourceCtx {
    ws3::SourceCtx {
        options: c.options.clone(),
        tmp_dir: c.tmp_dir.clone(),
        data_dir: env.data_dir.clone(),
        sources_dir: env.sources_dir.clone(),
        ffmpeg: env.ffmpeg.clone(),
    }
}

fn kind_str(k: ws3::Kind) -> &'static str {
    match k {
        ws3::Kind::Online => "online",
        ws3::Kind::Local => "local",
        ws3::Kind::Action => "action",
        ws3::Kind::Plugin => "plugin",
    }
}

fn src_err(e: ws3::SourceError) -> CoreError {
    if e.0 == "Cancelled." {
        CoreError::cancelled()
    } else {
        CoreError::new(e.0)
    }
}

/// Run a WS3 add on its own thread, with progress and cancellation bridged from the task. Cancelling returns at once
/// even if the provider is stuck in a network read (the thread finishes on its own later); a file the provider
/// produces after that, or for a cancelled task, is deleted when it is a temporary download.
fn run_add(task: &TaskCtx, p: Arc<dyn ws3::SourceProvider>, c: ws3::SourceCtx, item: String, req: ws3::AddRequest) -> CoreResult<AddedFile> {
    enum Msg {
        P(f32, String),
        Done(ws3::Result<ws3::AddResult>),
    }
    let drop_temp = |r: &ws3::Result<ws3::AddResult>| {
        if let Ok(a) = r {
            if a.temp {
                let _ = std::fs::remove_file(&a.file);
            }
        }
    };
    task.check()?;
    let cancel = ws3::Cancel::new();
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();
    {
        let cancel = cancel.clone();
        std::thread::Builder::new()
            .name("source-add".into())
            .spawn(move || {
                let tp = Mutex::new(tx.clone());
                let prog = move |f: f32, m: &str| {
                    let _ = tp.lock().unwrap().send(Msg::P(f, m.to_string()));
                };
                let r = p.add(&c, &item, &req, &prog, &cancel);
                if let Err(std::sync::mpsc::SendError(Msg::Done(r))) = tx.send(Msg::Done(r)) {
                    drop_temp(&r); // nobody is waiting any more (cancelled)
                }
            })
            .map_err(|e| CoreError::new(e.to_string()))?;
    }
    let r = loop {
        match rx.recv_timeout(Duration::from_millis(50)) {
            // WS3 reports 0..1 for its own work; the import into the Gallery is the last 12 %.
            Ok(Msg::P(f, m)) => task.progress((f as f64).clamp(0.0, 1.0) * 0.88, &m),
            Ok(Msg::Done(r)) => break r,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if task.is_cancelled() => {
                cancel.cancel();
                // Give a cooperative provider a moment to stop and clean up; otherwise leave it behind.
                let end = std::time::Instant::now() + Duration::from_secs(2);
                while let Some(left) = end.checked_duration_since(std::time::Instant::now()) {
                    match rx.recv_timeout(left) {
                        Ok(Msg::Done(r)) => {
                            drop_temp(&r);
                            break;
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                return Err(task.check().err().unwrap_or_else(CoreError::cancelled));
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Err(CoreError::new("The source stopped unexpectedly.")),
        }
    };
    if task.is_cancelled() {
        drop_temp(&r);
        return Err(task.check().err().unwrap_or_else(CoreError::cancelled));
    }
    let a = r.map_err(src_err)?;
    Ok(AddedFile { file: a.file, name: a.name, temp: a.temp })
}

/// A built-in WS3 provider.
struct Builtin {
    p: Arc<dyn ws3::SourceProvider>,
    env: Env,
    kind: &'static str,
}

impl SourceProvider for Builtin {
    fn id(&self) -> &str {
        self.p.id()
    }
    fn name(&self) -> &str {
        self.p.name()
    }
    fn description(&self) -> &str {
        self.p.description()
    }
    fn kind(&self) -> &str {
        self.kind
    }
    fn options(&self) -> Vec<Value> {
        self.p.options().into_iter().map(|o| json!({ "key": o.key, "label": o.label, "type": o.type_ })).collect()
    }
    fn detect(&self, ctx: &SourceCtx) -> Detect {
        let d = self.p.detect(&ctx3(&self.env, ctx));
        Detect { available: d.available, status: d.status }
    }
    fn list(&self, ctx: &SourceCtx, page: u32, refresh: bool) -> CoreResult<ItemPage> {
        let r = self.p.list(&ctx3(&self.env, ctx), page, None, refresh).map_err(src_err)?;
        Ok(ItemPage {
            items: r
                .items
                .into_iter()
                .map(|i| SourceItem {
                    id: i.id,
                    title: i.title,
                    thumb: i.thumb,
                    kind: i.type_,
                    action: match i.action {
                        ws3::Action::Add => "add",
                        ws3::Action::Record => "record",
                        ws3::Action::None => "none",
                    }
                    .into(),
                    note: i.note,
                    key: Some(i.key).filter(|k| !k.is_empty()),
                    badge: i.badge.unwrap_or_default(),
                })
                .collect(),
            total: Some(r.total),
            has_more: r.has_more,
        })
    }
    fn thumb_path(&self, ctx: &SourceCtx, item: &str) -> Option<PathBuf> {
        self.p.thumb_path(&ctx3(&self.env, ctx), item)
    }
    fn add(&self, ctx: &SourceCtx, task: &TaskCtx, item: &str, opts: &AddOpts) -> CoreResult<AddedFile> {
        let c = ctx3(&self.env, ctx);
        let req = ws3::AddRequest { seconds: opts.seconds.map(|s| s as f32) };
        run_add(task, self.p.clone(), c, item.to_string(), req)
    }
    /// "Add from link": WS3's link source takes the URL as the item id.
    fn download(&self, ctx: &SourceCtx, task: &TaskCtx, url: &str) -> CoreResult<AddedFile> {
        self.add(ctx, task, url, &AddOpts::default())
    }
}

/// A JSON plug-in: listed by file name; the file is parsed only when a call needs it (core only calls detect/list/add
/// for enabled sources).
struct Plugin {
    id: String,
    file: PathBuf,
    file_name: String,
    env: Env,
    loaded: Mutex<Option<Arc<ws3::declarative::Declarative>>>,
}

impl Plugin {
    fn load(&self) -> CoreResult<Arc<ws3::declarative::Declarative>> {
        let mut g = self.loaded.lock().unwrap();
        if let Some(p) = g.as_ref() {
            return Ok(p.clone());
        }
        let p = Arc::new(ws3::declarative::Declarative::load(&self.file).map_err(|e| CoreError::new(format!("Could not load: {e}")))?);
        *g = Some(p.clone());
        Ok(p)
    }
}

impl SourceProvider for Plugin {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.file_name
    }
    fn description(&self) -> &str {
        "Plug-in file (declarative JSON)."
    }
    fn kind(&self) -> &str {
        "plugin"
    }
    fn extra(&self) -> Value {
        let mut v = json!({ "warning": ws3::PLUGIN_WARNING, "file": self.file.to_string_lossy() });
        if let Some(p) = self.loaded.lock().unwrap().as_ref() {
            use ws3::SourceProvider as _;
            v["name"] = json!(p.name());
            v["description"] = json!(p.description());
        }
        v
    }
    fn detect(&self, ctx: &SourceCtx) -> Detect {
        use ws3::SourceProvider as _;
        match self.load() {
            Ok(p) => {
                let d = p.detect(&ctx3(&self.env, ctx));
                Detect { available: d.available, status: d.status }
            }
            Err(e) => Detect { available: false, status: e.message },
        }
    }
    fn list(&self, ctx: &SourceCtx, page: u32, refresh: bool) -> CoreResult<ItemPage> {
        let p = self.load()?;
        Builtin { p: Arc::new(PluginRef(p)), env: self.env.clone(), kind: "plugin" }.list(ctx, page, refresh)
    }
    fn thumb_path(&self, ctx: &SourceCtx, item: &str) -> Option<PathBuf> {
        use ws3::SourceProvider as _;
        self.load().ok()?.thumb_path(&ctx3(&self.env, ctx), item)
    }
    fn add(&self, ctx: &SourceCtx, task: &TaskCtx, item: &str, opts: &AddOpts) -> CoreResult<AddedFile> {
        let p = self.load()?;
        Builtin { p: Arc::new(PluginRef(p)), env: self.env.clone(), kind: "plugin" }.add(ctx, task, item, opts)
    }
}

/// Shares one loaded plug-in with the `Builtin` mapping code.
struct PluginRef(Arc<ws3::declarative::Declarative>);

impl ws3::SourceProvider for PluginRef {
    fn id(&self) -> &str {
        self.0.id()
    }
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn kind(&self) -> ws3::Kind {
        self.0.kind()
    }
    fn options(&self) -> Vec<ws3::OptionDef> {
        self.0.options()
    }
    fn detect(&self, ctx: &ws3::SourceCtx) -> ws3::Detect {
        self.0.detect(ctx)
    }
    fn list(&self, ctx: &ws3::SourceCtx, page: u32, q: Option<&str>, refresh: bool) -> ws3::Result<ws3::ListPage> {
        self.0.list(ctx, page, q, refresh)
    }
    fn thumb_path(&self, ctx: &ws3::SourceCtx, id: &str) -> Option<PathBuf> {
        self.0.thumb_path(ctx, id)
    }
    fn add(&self, ctx: &ws3::SourceCtx, id: &str, req: &ws3::AddRequest, p: ws3::Progress, c: &ws3::Cancel) -> ws3::Result<ws3::AddResult> {
        self.0.add(ctx, id, req, p, c)
    }
}

/// Built-ins + JSON plug-ins in `sources_dir`.
pub struct Catalog {
    env: Env,
    builtins: Vec<Arc<dyn SourceProvider>>,
    plugins: Mutex<Vec<Arc<Plugin>>>,
}

impl Catalog {
    pub fn new(env: Env) -> Self {
        let builtins = ws3::builtin()
            .into_iter()
            .map(|p| {
                let kind = kind_str(p.kind());
                Arc::new(Builtin { p: Arc::from(p), env: env.clone(), kind }) as Arc<dyn SourceProvider>
            })
            .collect();
        Catalog { env, builtins, plugins: Mutex::new(vec![]) }
    }
}

impl SourceCatalog for Catalog {
    fn providers(&self) -> Vec<Arc<dyn SourceProvider>> {
        let files = ws3::plugin_files(&self.env.sources_dir);
        let mut cache = self.plugins.lock().unwrap();
        // Keep loaded plug-ins whose file is unchanged; a changed file is re-read on next use.
        cache.retain(|p| files.contains(&p.file));
        for f in files {
            if !cache.iter().any(|p| p.file == f) {
                cache.push(Arc::new(Plugin {
                    id: ws3::plugin_id(&f),
                    file_name: f.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    file: f,
                    env: self.env.clone(),
                    loaded: Mutex::new(None),
                }));
            }
        }
        let mut out = self.builtins.clone();
        out.extend(cache.iter().map(|p| p.clone() as Arc<dyn SourceProvider>));
        out
    }
    fn plugins_dir(&self) -> PathBuf {
        self.env.sources_dir.clone()
    }
    fn desktop_capture(&self) -> bool {
        cfg!(windows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::scheduler::{Limits, NoSink, Policy, Res, Resources, Scheduler, TaskSpec};
    use std::time::Instant;

    /// A provider stuck in a "network read" that ignores cancellation, then returns a temporary file.
    struct Stuck(PathBuf);
    impl ws3::SourceProvider for Stuck {
        fn id(&self) -> &str {
            "stuck"
        }
        fn name(&self) -> &str {
            "Stuck"
        }
        fn description(&self) -> &str {
            ""
        }
        fn kind(&self) -> ws3::Kind {
            ws3::Kind::Online
        }
        fn detect(&self, _: &ws3::SourceCtx) -> ws3::Detect {
            ws3::Detect::ok("")
        }
        fn list(&self, _: &ws3::SourceCtx, _: u32, _: Option<&str>, _: bool) -> ws3::Result<ws3::ListPage> {
            Ok(Default::default())
        }
        fn add(&self, _: &ws3::SourceCtx, _: &str, _: &ws3::AddRequest, p: ws3::Progress, _: &ws3::Cancel) -> ws3::Result<ws3::AddResult> {
            p(0.5, "Downloading");
            std::thread::sleep(Duration::from_millis(3000));
            std::fs::write(&self.0, b"downloaded").unwrap();
            Ok(ws3::AddResult { file: self.0.clone(), name: "x.mp4".into(), temp: true })
        }
    }

    #[test]
    fn cancel_returns_at_once_and_temp_download_is_deleted() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("dl.mp4");
        let env = Env { data_dir: d.path().into(), sources_dir: d.path().into(), ffmpeg: "ffmpeg".into() };
        let b = Arc::new(Builtin { p: Arc::new(Stuck(file.clone())), env, kind: "online" });
        let s = Scheduler::new(Limits::default(), Policy::default(), Arc::new(NoSink));
        let ctx = SourceCtx { options: json!({}), tmp_dir: d.path().into() };
        let b2 = b.clone();
        let h = s.submit(TaskSpec::new("source", "add", Resources::one(Res::Net)), move |t| {
            b2.add(&ctx, t, "item", &AddOpts::default()).map(|a| json!(a.file))
        });
        std::thread::sleep(Duration::from_millis(150));
        let t0 = Instant::now();
        assert!(s.cancel(&h.id));
        assert!(h.wait().unwrap_err().is_cancelled());
        assert!(t0.elapsed() < Duration::from_millis(2600), "returned without waiting for the provider");
        assert!(!file.exists());
        std::thread::sleep(Duration::from_millis(1500)); // the provider finishes on its own
        assert!(!file.exists(), "temporary download deleted after the cancel");
    }
}
