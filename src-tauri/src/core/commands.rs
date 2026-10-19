//! Tauri commands and events for the UI (docs/v2/api-contract.md). Thin async wrappers: every command runs its
//! `App` method on a blocking thread so the UI thread never waits on disk, ffmpeg or the screen.
use super::app::{App, Events};
use super::error::{CoreError, CoreResult};
use super::scheduler::Job;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::{Emitter, State};

/// Managed state: the core app.
pub struct Core(pub Arc<App>);

/// Events → `cs:job`, `cs:library`, `cs:app`, `cs:device`.
pub struct TauriEvents(pub tauri::AppHandle);

impl Events for TauriEvents {
    fn job(&self, job: &Job) {
        let _ = self.0.emit("cs:job", job);
    }
    fn library(&self, rev: u64) {
        let _ = self.0.emit("cs:library", json!({ "rev": rev }));
    }
    fn app_changed(&self) {
        let _ = self.0.emit("cs:app", json!({}));
    }
    fn device_busy(&self, busy: bool) {
        let _ = self.0.emit("cs:device", json!({ "busy": busy }));
    }
}

/// Late-bound events: the App is created before the Tauri handle exists.
#[derive(Default)]
pub struct EventRelay(pub std::sync::OnceLock<Arc<dyn Events>>);

impl Events for EventRelay {
    fn job(&self, j: &Job) {
        if let Some(e) = self.0.get() {
            e.job(j)
        }
    }
    fn library(&self, r: u64) {
        if let Some(e) = self.0.get() {
            e.library(r)
        }
    }
    fn app_changed(&self) {
        if let Some(e) = self.0.get() {
            e.app_changed()
        }
    }
    fn device_busy(&self, b: bool) {
        if let Some(e) = self.0.get() {
            e.device_busy(b)
        }
    }
}

async fn run<T: Send + 'static>(app: &State<'_, Core>, f: impl FnOnce(&App) -> CoreResult<T> + Send + 'static) -> Result<T, CoreError> {
    let a = app.0.clone();
    tauri::async_runtime::spawn_blocking(move || f(&a)).await.map_err(|e| CoreError::new(e.to_string()).status(500))?
}

// ---------- state, jobs ----------

#[tauri::command]
pub async fn app_state(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| Ok(a.app_state())).await
}

/// The JSON args of a call as one object (so a key set to null can be told from a missing one).
fn json_args(request: &tauri::ipc::Request<'_>) -> Value {
    match request.body() {
        tauri::ipc::InvokeBody::Json(v) => v.clone(),
        _ => json!({}),
    }
}

/// `{ mask?, maxSeconds? }`, or the same wrapped in `patch`.
#[tauri::command]
pub async fn settings_update(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    let a = json_args(&request);
    let patch = a.get("patch").filter(|p| p.is_object()).cloned().unwrap_or(a);
    run(&core, move |app| app.settings_update(&patch)).await
}

#[tauri::command]
pub async fn activity(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| Ok(a.activity())).await
}

#[tauri::command]
pub async fn job_get(core: State<'_, Core>, job_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.job_get(&job_id)).await
}

#[tauri::command]
pub async fn job_cancel(core: State<'_, Core>, job_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| Ok(json!({ "cancelled": a.sched.cancel(&job_id) }))).await
}

#[tauri::command]
pub async fn set_focus(core: State<'_, Core>, original_id: Option<String>, visible: Option<Vec<String>>) -> Result<Value, CoreError> {
    run(&core, move |a| {
        a.sched.set_focus(original_id.as_deref(), &visible.unwrap_or_default());
        Ok(Value::Null)
    })
    .await
}

// ---------- originals ----------

#[tauri::command]
pub async fn originals_list(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| Ok(a.originals_list())).await
}

/// Raw body = the file's bytes; header `x-cs-name` = its (URI-encoded) name.
#[tauri::command]
pub async fn import_bytes(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(CoreError::new("Send the file as raw bytes."));
    };
    let bytes = bytes.clone();
    let name = request
        .headers()
        .get("x-cs-name")
        .and_then(|v| v.to_str().ok())
        .map(|s| super::media_server::query_param(&format!("n={s}"), "n").unwrap_or_default())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "upload".into());
    run(&core, move |a| a.import_bytes(&name, &bytes)).await
}

#[tauri::command]
pub async fn import_path(core: State<'_, Core>, path: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.import_path(&path)).await
}

#[tauri::command]
pub async fn missing_delete(core: State<'_, Core>, ids: Vec<String>) -> Result<Value, CoreError> {
    run(&core, move |a| Ok(a.missing_delete(&ids))).await
}

#[tauri::command]
pub async fn original_get(core: State<'_, Core>, original_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.original_get(&original_id)).await
}

/// `{ originalId, name?, draft? }` (draft null clears), or name/draft wrapped in `patch`.
#[tauri::command]
pub async fn original_patch(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    let a = json_args(&request);
    let oid = a["originalId"].as_str().unwrap_or("").to_string();
    let patch = a.get("patch").filter(|p| p.is_object()).cloned().unwrap_or(a);
    run(&core, move |app| app.original_patch(&oid, &patch)).await
}

#[tauri::command]
pub async fn original_delete(core: State<'_, Core>, original_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.original_delete(&original_id)).await
}

#[tauri::command]
pub async fn original_reveal(core: State<'_, Core>, original_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.original_reveal(&original_id)).await
}

#[tauri::command]
pub async fn estimate(core: State<'_, Core>, original_id: String, settings: Value) -> Result<Value, CoreError> {
    run(&core, move |a| a.estimate(&original_id, &settings)).await
}

#[tauri::command]
pub async fn loop_search(core: State<'_, Core>, original_id: String, min_len: Option<f64>, max_len: Option<f64>) -> Result<Value, CoreError> {
    run(&core, move |a| a.loop_search(&original_id, &json!({ "minLen": min_len, "maxLen": max_len }))).await
}

#[tauri::command]
pub async fn loop_point(core: State<'_, Core>, original_id: String, start: f64, length: f64) -> Result<Value, CoreError> {
    run(&core, move |a| a.loop_point(&original_id, &json!({ "start": start, "length": length }))).await
}

// ---------- versions ----------

#[tauri::command]
pub async fn version_create(core: State<'_, Core>, original_id: String, settings: Value) -> Result<Value, CoreError> {
    run(&core, move |a| a.version_create(&original_id, &settings)).await
}

#[tauri::command]
pub async fn version_patch(core: State<'_, Core>, version_id: String, label: Option<String>) -> Result<Value, CoreError> {
    run(&core, move |a| a.version_patch(&version_id, label.as_deref().unwrap_or(""))).await
}

#[tauri::command]
pub async fn version_delete(core: State<'_, Core>, version_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.version_delete(&version_id)).await
}

#[tauri::command]
pub async fn version_send(core: State<'_, Core>, version_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.version_send(&version_id)).await
}

#[tauri::command]
pub async fn version_remake(core: State<'_, Core>, version_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.version_remake(&version_id, false)).await
}

#[tauri::command]
pub async fn version_retry(core: State<'_, Core>, version_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.version_remake(&version_id, true)).await
}

// ---------- sources ----------

#[tauri::command]
pub async fn sources_list(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| Ok(a.sources_list())).await
}

#[tauri::command]
pub async fn browse_set(core: State<'_, Core>, ticked: Vec<String>) -> Result<Value, CoreError> {
    run(&core, move |a| a.browse_set(&ticked)).await
}

#[tauri::command]
pub async fn link_add(core: State<'_, Core>, url: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.link_add(&url)).await
}

#[tauri::command]
pub async fn source_set(core: State<'_, Core>, source_id: String, enabled: Option<bool>, options: Option<Value>) -> Result<Value, CoreError> {
    run(&core, move |a| a.source_set(&source_id, enabled, options)).await
}

#[tauri::command]
pub async fn source_items(core: State<'_, Core>, source_id: String, page: Option<u32>, refresh: Option<bool>) -> Result<Value, CoreError> {
    run(&core, move |a| a.source_items(&source_id, page.unwrap_or(1), refresh.unwrap_or(false))).await
}

#[tauri::command]
pub async fn source_add(
    core: State<'_, Core>,
    source_id: String,
    item_id: String,
    record: Option<bool>,
    seconds: Option<f64>,
    title: Option<String>,
) -> Result<Value, CoreError> {
    run(&core, move |a| a.source_add(&source_id, &item_id, &json!({ "record": record.unwrap_or(false), "seconds": seconds, "title": title }))).await
}

// ---------- themes (3.0, docs/v3/theme-format.md) ----------

#[tauri::command]
pub async fn theme_get(core: State<'_, Core>, original_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.theme_get(&original_id)).await
}

#[tauri::command]
pub async fn theme_set(core: State<'_, Core>, original_id: String, theme: Value) -> Result<Value, CoreError> {
    run(&core, move |a| a.theme_set(&original_id, &theme)).await
}

#[tauri::command]
pub async fn theme_standard(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| Ok(a.theme_standard())).await
}

/// `{ originalId, theme?, withLive, w?, h? }` -> `{ url }` (a PNG on the media server).
#[tauri::command]
pub async fn theme_preview(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    let a = json_args(&request);
    let oid = a["originalId"].as_str().unwrap_or("").to_string();
    let theme = a.get("theme").filter(|t| t.is_object()).cloned();
    let with_live = a["withLive"].as_bool().unwrap_or(true);
    let (w, h) = (a["w"].as_u64().unwrap_or(1200) as u32, a["h"].as_u64().unwrap_or(540) as u32);
    run(&core, move |app| app.theme_preview(&oid, theme.as_ref(), with_live, w, h)).await
}

/// JSON `{ originalId, path }`, or the raw bytes of a picture with headers `x-cs-oid` and `x-cs-name` -> `{ file }`.
#[tauri::command]
pub async fn theme_asset_add(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    if let tauri::ipc::InvokeBody::Raw(bytes) = request.body() {
        let hdr = |k: &str| {
            request
                .headers()
                .get(k)
                .and_then(|v| v.to_str().ok())
                .map(|s| super::media_server::query_param(&format!("n={s}"), "n").unwrap_or_default())
                .unwrap_or_default()
        };
        let (oid, name, bytes) = (hdr("x-cs-oid"), hdr("x-cs-name"), bytes.clone());
        return run(&core, move |a| a.theme_asset_add_bytes(&oid, &name, &bytes)).await;
    }
    let a = json_args(&request);
    let oid = a["originalId"].as_str().unwrap_or("").to_string();
    let path = a["path"].as_str().unwrap_or("").to_string();
    run(&core, move |app| app.theme_asset_add_path(&oid, &path)).await
}

/// Use a version of any clip as the theme's base (nothing is sent to the screen) -> the theme view.
#[tauri::command]
pub async fn theme_set_base(core: State<'_, Core>, original_id: String, version_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.theme_set_base(&original_id, &version_id)).await
}

/// Screen guides calibration: `{ guides }` draws the lines on the live screen, `{ guides: null }` clears them.
#[tauri::command]
pub async fn calibration_set(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    let a = json_args(&request);
    let g = a.get("guides").filter(|g| g.is_object()).map(crate::live::theme::Guides::from_value);
    run(&core, move |app| {
        app.live.set_calibration(g);
        Ok(json!({ "live": app.live.running() }))
    })
    .await
}

/// Screen fit calibration: `{ action: "start" | "set" | "end", fit: { top, right, bottom, left }, sel: 0..3 }`. `start` puts the ruler
/// picture on the screen (live steps aside), `set` changes it, `end` gives the screen back. -> `{ calibrating, connected }`.
#[tauri::command]
pub async fn fit_cal(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    let a = json_args(&request);
    run(&core, move |app| app.fit_cal(&a)).await
}

/// "Use this theme": live switches to it at once, Embedded is not touched -> the theme's view.
#[tauri::command]
pub async fn theme_use(core: State<'_, Core>, original_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.theme_use(&original_id)).await
}

/// Store a theme as Embedded (base picture, written to the screen) -> `{ jobId }`.
#[tauri::command]
pub async fn theme_apply(core: State<'_, Core>, original_id: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.theme_apply(&original_id)).await
}

// ---------- gallery folder ----------

#[tauri::command]
pub async fn gallery_set_dir(core: State<'_, Core>, dir: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.gallery_set_dir_cmd(&dir)).await
}

#[tauri::command]
pub async fn gallery_open(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| a.gallery_open()).await
}

#[tauri::command]
pub async fn gallery_sync(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| {
        a.gallery_sync();
        Ok(json!({ "ok": true }))
    })
    .await
}

// ---------- screen ----------

#[tauri::command]
pub async fn device_state(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| a.device_state()).await
}

#[tauri::command]
pub async fn device_check(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| a.device_check()).await
}

#[tauri::command]
pub async fn device_restart(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| Ok(a.device_restart())).await
}

#[tauri::command]
pub async fn device_setup(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| a.device_script("setup")).await
}

#[tauri::command]
pub async fn device_restore(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| a.device_script("restore")).await
}

#[tauri::command]
pub async fn device_reconnect(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, |a| a.device_script("reconnect")).await
}

/// Every core command name (build.rs app manifest and capabilities use the same list).
pub const COMMANDS: &[&str] = &[
    "app_state", "settings_update", "activity", "job_get", "job_cancel", "set_focus", "originals_list", "import_bytes", "import_path",
    "missing_delete", "original_get", "original_patch", "original_delete", "original_reveal", "estimate", "loop_search", "loop_point",
    "version_create", "version_patch", "version_delete", "version_send", "version_remake", "version_retry", "sources_list", "browse_set",
    "link_add", "source_set", "source_items", "source_add", "gallery_set_dir", "gallery_open", "gallery_sync", "device_state",
    "device_check", "device_restart", "device_setup", "device_restore", "device_reconnect", "theme_get", "theme_set", "theme_standard",
    "theme_preview", "theme_asset_add", "theme_apply", "theme_use", "theme_set_base", "calibration_set", "fit_cal", "weather_search", "weather_set_place", "ext_sensor_status",
];

// ---------- 4.0 external sensors (CPU temperature through HWiNFO / LibreHardwareMonitor the user runs) ----------

/// `{}` -> `{ source: "hwinfo" | "lhm" | "none", ok, message }`: where CPU temperature etc. are read from, or why they cannot be.
#[tauri::command]
pub async fn ext_sensor_status(core: State<'_, Core>) -> Result<Value, CoreError> {
    run(&core, move |_a| {
        let s = crate::live::extsensors::status();
        Ok(json!({ "source": s.source, "ok": s.ok, "message": s.message }))
    })
    .await
}

// ---------- weather place ----------

/// `{ query }` -> `{ results: [{ name, country, admin1, lat, lon, tz }] }` (Open-Meteo geocoding; only when the user searches).
#[tauri::command]
pub async fn weather_search(core: State<'_, Core>, query: String) -> Result<Value, CoreError> {
    run(&core, move |a| a.weather_search(&query)).await
}

/// `{ place: { name, lat, lon, tz } | null }` -> the app state.
#[tauri::command]
pub async fn weather_set_place(core: State<'_, Core>, request: tauri::ipc::Request<'_>) -> Result<Value, CoreError> {
    let place = json_args(&request).get("place").cloned().unwrap_or(Value::Null);
    run(&core, move |a| a.weather_set_place(&place)).await
}
