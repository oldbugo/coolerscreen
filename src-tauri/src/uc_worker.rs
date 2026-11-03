//! Private inherited-handle protocol for UC. Only this binary includes this module.
use cs_core::core::api::{Backends, NoSources};
use cs_core::core::error::{CoreError, CoreResult};
use cs_core::core::fake_media::FakeMedia;
use cs_core::core::media_server::MediaServer;
use cs_core::core::scheduler::Job;
use cs_core::core::{App, Events};
use serde::Deserialize;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

const MAX_FRAME: usize = 32 * 1024 * 1024;
const MAX_REQUESTS: usize = 16;
const VERSION: u64 = 1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    v: u64,
    id: u64,
    command: String,
    args: Value,
}

#[derive(Clone)]
struct Output(Arc<Mutex<std::io::Stdout>>);
impl Output {
    fn send(&self, value: Value) {
        let Ok(mut bytes) = serde_json::to_vec(&value) else {
            return;
        };
        if bytes.len() > MAX_FRAME {
            let Some(id) = value.get("id").and_then(Value::as_u64) else {
                return;
            };
            bytes = serde_json::to_vec(&json!({"v":VERSION,"id":id,"ok":false,"error":{
                "error":"The worker response is too large.","status":413,"code":"tooLarge"}}))
            .unwrap();
        }
        bytes.push(b'\n');
        if let Ok(mut out) = self.0.lock() {
            let _ = out.write_all(&bytes).and_then(|_| out.flush());
        }
    }
    fn reply(&self, id: u64, result: CoreResult<Value>) {
        self.send(match result {
            Ok(result) => json!({"v":VERSION,"id":id,"ok":true,"result":result}),
            Err(error) => json!({"v":VERSION,"id":id,"ok":false,"error":error}),
        });
    }
    fn event(&self, event: &str, payload: Value) {
        self.send(json!({"v":VERSION,"event":event,"payload":payload}));
    }
}
impl Events for Output {
    fn job(&self, job: &Job) {
        self.event("cs:job", serde_json::to_value(job).unwrap_or(Value::Null));
    }
    fn library(&self, rev: u64) {
        self.event("cs:library", json!({"rev":rev}));
    }
    fn app_changed(&self) {
        self.event("cs:app", json!({}));
    }
    fn device_busy(&self, busy: bool) {
        self.event("cs:device", json!({"busy":busy}));
    }
}

/// Caps allocations before parsing. EOF with an unfinished frame is an error.
fn read_frame(reader: &mut impl BufRead) -> CoreResult<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    let read = reader
        .take((MAX_FRAME + 1) as u64)
        .read_until(b'\n', &mut frame)?;
    if read == 0 {
        return Ok(None);
    }
    if frame.len() > MAX_FRAME {
        return Err(CoreError::coded("tooLarge", "Worker request too large.").status(413));
    }
    if frame.last() != Some(&b'\n') {
        return Err(CoreError::coded("protocol", "Incomplete worker request."));
    }
    Ok(Some(frame))
}

fn parse(frame: &[u8]) -> CoreResult<Request> {
    let request: Request = serde_json::from_slice(frame)
        .map_err(|_| CoreError::coded("protocol", "Invalid worker request."))?;
    if request.v != VERSION || request.command.len() > 64 || !request.args.is_object() {
        return Err(CoreError::coded("protocol", "Unsupported worker request."));
    }
    validate(&request.command, &request.args)?;
    Ok(request)
}

fn bad(field: &str) -> CoreError {
    CoreError::coded("arguments", format!("Invalid {field}."))
}
fn string<'a>(args: &'a Value, field: &str) -> CoreResult<&'a str> {
    args[field]
        .as_str()
        .filter(|s| s.len() <= 32768 && !s.contains('\0'))
        .ok_or_else(|| bad(field))
}
fn optional_string<'a>(args: &'a Value, field: &str) -> CoreResult<Option<&'a str>> {
    if args.get(field).is_none_or(Value::is_null) {
        Ok(None)
    } else {
        string(args, field).map(Some)
    }
}
fn boolean(args: &Value, field: &str, default: bool) -> CoreResult<bool> {
    if args.get(field).is_none_or(Value::is_null) {
        Ok(default)
    } else {
        args[field].as_bool().ok_or_else(|| bad(field))
    }
}
fn number(args: &Value, field: &str, default: f64) -> CoreResult<f64> {
    if args.get(field).is_none_or(Value::is_null) {
        Ok(default)
    } else {
        args[field]
            .as_f64()
            .filter(|n| n.is_finite())
            .ok_or_else(|| bad(field))
    }
}
fn object<'a>(args: &'a Value, field: &str) -> CoreResult<&'a Value> {
    args.get(field)
        .filter(|v| v.is_object())
        .ok_or_else(|| bad(field))
}
fn strings(args: &Value, field: &str) -> CoreResult<Vec<String>> {
    let items = args[field]
        .as_array()
        .filter(|a| a.len() <= 10000)
        .ok_or_else(|| bad(field))?;
    items
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| s.len() <= 32768 && !s.contains('\0'))
                .map(String::from)
                .ok_or_else(|| bad(field))
        })
        .collect()
}

const COMMANDS: &[&str] = &[
    "app_state",
    "settings_update",
    "activity",
    "job_get",
    "job_cancel",
    "set_focus",
    "originals_list",
    "import_bytes",
    "import_path",
    "missing_delete",
    "original_get",
    "original_patch",
    "original_delete",
    "original_reveal",
    "estimate",
    "loop_search",
    "loop_point",
    "version_create",
    "version_patch",
    "version_delete",
    "version_send",
    "version_remake",
    "version_retry",
    "sources_list",
    "browse_set",
    "link_add",
    "source_set",
    "source_items",
    "source_add",
    "gallery_set_dir",
    "gallery_open",
    "gallery_sync",
    "device_state",
    "device_check",
    "device_presence",
    "device_restart",
    "device_setup",
    "device_restore",
    "device_reconnect",
    "theme_get",
    "theme_set",
    "theme_standard",
    "theme_preview",
    "theme_asset_add",
    "theme_apply",
    "theme_use",
    "theme_set_base",
    "calibration_set",
    "fit_cal",
    "weather_search",
    "weather_set_place",
    "ext_sensor_status",
    "game_fps_status",
    "lyrics_status",
    "lyrics_offset_set",
    "screen_preview",
    "screen_preview_end",
    "twin_watch",
    "live_set",
    "activate",
    "health_status",
    "shutdown",
];

fn validate(command: &str, args: &Value) -> CoreResult<()> {
    let allowed: &[&str] = match command {
        "settings_update" => &[
            "patch",
            "mask",
            "maxSeconds",
            "cornerRadius",
            "screenGuides",
            "screenFit",
            "editor",
            "weather",
            "screenChanged",
        ],
        "job_get" | "job_cancel" => &["jobId"],
        "set_focus" => &["originalId", "visible"],
        "import_bytes" => &["name", "bytes", "base64"],
        "import_path" => &["path"],
        "missing_delete" => &["ids"],
        "original_get" | "original_delete" | "original_reveal" | "theme_get" | "theme_use"
        | "theme_apply" => &["originalId"],
        "original_patch" => &["originalId", "patch", "name", "draft"],
        "estimate" | "version_create" => &["originalId", "settings"],
        "loop_search" => &["originalId", "minLen", "maxLen"],
        "loop_point" => &["originalId", "start", "length"],
        "version_patch" => &["versionId", "label"],
        "version_delete" | "version_send" | "version_remake" | "version_retry" => &["versionId"],
        "browse_set" => &["ticked"],
        "link_add" => &["url"],
        "source_set" => &["sourceId", "enabled", "options"],
        "source_items" => &["sourceId", "page", "refresh"],
        "source_add" => &["sourceId", "itemId", "record", "seconds", "title"],
        "gallery_set_dir" => &["dir"],
        "theme_set" => &["originalId", "theme"],
        "theme_preview" => &["originalId", "theme", "withLive", "w", "h", "phase"],
        "theme_asset_add" => &["originalId", "path", "name", "bytes", "base64"],
        "theme_set_base" => &["originalId", "versionId"],
        "calibration_set" => &["guides"],
        "fit_cal" => &["action", "fit", "sel"],
        "weather_search" => &["query"],
        "weather_set_place" => &["place"],
        "lyrics_offset_set" => &["key", "seconds"],
        "screen_preview" => &["originalId", "sourceId", "item", "thumb"],
        "screen_preview_end" => &["keepalive"],
        "twin_watch" => &["on"],
        "live_set" => &["patch"],
        command if COMMANDS.contains(&command) => &[],
        _ => return Err(CoreError::coded("unsupported", "Unknown cooler command.").status(404)),
    };
    if args
        .as_object()
        .unwrap()
        .keys()
        .any(|key| !allowed.contains(&key.as_str()))
    {
        return Err(bad("arguments"));
    }
    // Reject incorrect optional primitive types instead of interpreting them as defaults.
    for key in [
        "enabled",
        "record",
        "refresh",
        "withLive",
        "keepalive",
        "on",
    ] {
        if args.get(key).is_some() {
            boolean(args, key, false)?;
        }
    }
    for key in [
        "seconds", "start", "length", "minLen", "maxLen", "phase", "w", "h", "page", "sel",
    ] {
        if args.get(key).is_some() {
            number(args, key, 0.0)?;
        }
    }
    Ok(())
}

fn decode_bytes(args: &Value) -> CoreResult<Vec<u8>> {
    match (args.get("bytes"), args.get("base64")) {
        (Some(Value::Array(items)), None) if items.len() <= MAX_FRAME => items
            .iter()
            .map(|v| {
                v.as_u64()
                    .filter(|n| *n <= 255)
                    .map(|n| n as u8)
                    .ok_or_else(|| bad("bytes"))
            })
            .collect(),
        (None, Some(Value::String(encoded))) => decode_base64(encoded),
        _ => Err(bad("file bytes")),
    }
}
fn decode_base64(encoded: &str) -> CoreResult<Vec<u8>> {
    if encoded.len() > MAX_FRAME || encoded.len() % 4 != 0 {
        return Err(bad("file bytes"));
    }
    let mut output = Vec::with_capacity(encoded.len() / 4 * 3);
    let val = |b: u8| match b {
        b'A'..=b'Z' => Some(b - b'A'),
        b'a'..=b'z' => Some(b - b'a' + 26),
        b'0'..=b'9' => Some(b - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let blocks = encoded.as_bytes().chunks_exact(4);
    let last = blocks.len().saturating_sub(1);
    for (index, block) in blocks.enumerate() {
        let a = val(block[0]).ok_or_else(|| bad("file bytes"))?;
        let b = val(block[1]).ok_or_else(|| bad("file bytes"))?;
        let pad = if block[2] == b'=' {
            2
        } else if block[3] == b'=' {
            1
        } else {
            0
        };
        if pad > 0 && (index != last || block[3] != b'=') {
            return Err(bad("file bytes"));
        }
        let c = if pad == 2 {
            0
        } else {
            val(block[2]).ok_or_else(|| bad("file bytes"))?
        };
        let d = if pad > 0 {
            0
        } else {
            val(block[3]).ok_or_else(|| bad("file bytes"))?
        };
        if (pad == 2 && b & 15 != 0) || (pad == 1 && c & 3 != 0) {
            return Err(bad("file bytes"));
        }
        output.push(a << 2 | b >> 4);
        if pad < 2 {
            output.push(b << 4 | c >> 2);
        }
        if pad == 0 {
            output.push(c << 6 | d);
        }
    }
    Ok(output)
}

fn native_conflict(fake: bool) -> CoreResult<()> {
    if fake {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let result = std::process::Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq CoolerScreen.exe", "/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .output()
            .map_err(|_| {
                CoreError::coded("ownership", "Could not check CoolerScreen ownership.")
            })?;
        if !result.status.success() {
            return Err(CoreError::coded(
                "ownership",
                "Could not check CoolerScreen ownership.",
            ));
        }
        if String::from_utf8_lossy(&result.stdout).lines().any(|line| {
            line.split(',').next().is_some_and(|name| {
                name.trim_matches('"')
                    .eq_ignore_ascii_case("CoolerScreen.exe")
            })
        }) {
            return Err(CoreError::coded(
                "cooler_owned",
                "Close CoolerScreen from its tray before enabling it in Unified Control.",
            )
            .status(409));
        }
    }
    Ok(())
}

fn live_set(app: &App, args: &Value) -> CoreResult<Value> {
    let patch = object(args, "patch")?;
    let allowed = [
        "inApp",
        "destroyWindow",
        "showSeconds",
        "fpsOverlay",
        "music",
        "screenOffOnShutdown",
        "background",
        "onSleep",
        "onLock",
        "fps",
        "paused",
        "autostart",
    ];
    if patch
        .as_object()
        .unwrap()
        .keys()
        .any(|key| !allowed.contains(&key.as_str()))
    {
        return Err(bad("live setting"));
    }
    if patch.get("autostart").is_some() {
        return Err(CoreError::coded(
            "unsupported",
            "Use Unified Control's Start with Windows setting.",
        ));
    }
    let before = app.settings.load().live();
    let mut now = before.clone();
    for (field, destination) in [
        ("inApp", &mut now.in_app),
        ("destroyWindow", &mut now.destroy_window),
        ("showSeconds", &mut now.show_seconds),
        ("fpsOverlay", &mut now.fps_overlay),
        ("music", &mut now.music),
        ("screenOffOnShutdown", &mut now.screen_off_on_shutdown),
    ] {
        if patch.get(field).is_some() {
            *destination = boolean(patch, field, false)?;
        }
    }
    if patch.get("background").is_some() {
        now.background = string(patch, "background")?.chars().take(80).collect();
    }
    for (field, destination) in [("onSleep", &mut now.on_sleep), ("onLock", &mut now.on_lock)] {
        if let Some(value) = optional_string(patch, field)? {
            if !["loop", "keep", "off"].contains(&value) {
                return Err(bad(field));
            }
            *destination = value.into();
        }
    }
    let fps = if patch.get("fps").is_some() {
        let fps = patch["fps"]
            .as_u64()
            .filter(|v| [30, 60].contains(v))
            .ok_or_else(|| bad("fps"))?;
        Some(fps as u32)
    } else {
        None
    };
    let paused = if patch.get("paused").is_some() {
        Some(boolean(patch, "paused", false)?)
    } else {
        None
    };
    let full = now.to_full_value();
    app.settings.update(|settings| settings["live"] = full)?;
    if let Some(fps) = fps {
        app.live.set_fps(fps);
    }
    if let Some(paused) = paused {
        app.live.set_user_paused(paused);
    }
    if now.in_app {
        app.prepare_live();
        // A running pipeline may belong to an editor-only twin or preview.
        // Claim it for saved Live before reloading, so view cleanup cannot stop it.
        let config = app.live_config();
        app.live.start_with(config.clone());
        app.live.reload(&config);
    } else if before.in_app || app.live.running() {
        app.live.stop();
    }
    app.settings_changed();
    Ok(app.app_state())
}

fn dispatch(app: &App, command: &str, a: &Value) -> CoreResult<Value> {
    validate(command, a)?;
    let s = |field| string(a, field);
    if ![
        "app_state",
        "activity",
        "job_get",
        "originals_list",
        "sources_list",
        "theme_standard",
        "device_presence",
        "lyrics_status",
        "ext_sensor_status",
        "game_fps_status",
        "health_status",
    ]
    .contains(&command)
    {
        native_conflict(app.device.fake_mode().is_some())?;
    }
    match command {
        "health_status"=>Ok(health_value(&app.live.status())),
        "app_state" => Ok(app.app_state()),
        "activity" => Ok(app.activity()),
        "settings_update" => app.settings_update(a.get("patch").map(|_| object(a,"patch")).transpose()?.unwrap_or(a)),
        "job_get" => app.job_get(s("jobId")?),
        "job_cancel" => Ok(json!({"cancelled":app.sched.cancel(s("jobId")?)})),
        "set_focus" => { let visible = if a.get("visible").is_some_and(|v|!v.is_null()) { strings(a,"visible")? } else { vec![] }; app.sched.set_focus(optional_string(a,"originalId")?, &visible); Ok(Value::Null) }
        "originals_list" => Ok(app.originals_list()),
        "import_bytes" => app.import_bytes(s("name")?, &decode_bytes(a)?),
        "import_path" => app.import_path(s("path")?),
        "missing_delete" => Ok(app.missing_delete(&strings(a,"ids")?)),
        "original_get" => app.original_get(s("originalId")?),
        "original_patch" => app.original_patch(s("originalId")?, a.get("patch").map(|_| object(a,"patch")).transpose()?.unwrap_or(a)),
        "original_delete" => app.original_delete(s("originalId")?),
        "estimate" => app.estimate(s("originalId")?,object(a,"settings")?),
        "loop_search" => app.loop_search(s("originalId")?, &json!({"minLen":a.get("minLen"),"maxLen":a.get("maxLen")})),
        "loop_point" => app.loop_point(s("originalId")?, &json!({"start":number(a,"start",0.0)?,"length":number(a,"length",0.0)?})),
        "version_create" => app.version_create(s("originalId")?,object(a,"settings")?),
        "version_patch" => app.version_patch(s("versionId")?,optional_string(a,"label")?.unwrap_or("")),
        "version_delete" => app.version_delete(s("versionId")?),
        "version_send" => app.version_send(s("versionId")?),
        "version_remake" | "version_retry" => app.version_remake(s("versionId")?, command=="version_retry"),
        "sources_list" => Ok(app.sources_list()),
        "browse_set" => app.browse_set(&strings(a,"ticked")?),
        "link_add" => app.link_add(s("url")?),
        "source_set" => app.source_set(s("sourceId")?,a.get("enabled").filter(|v|!v.is_null()).map(|_|boolean(a,"enabled",false)).transpose()?,a.get("options").cloned()),
        "source_items" => { let page=number(a,"page",1.0)?; if page<1.0 || page>100000.0 || page.fract()!=0.0 { return Err(bad("page")); } app.source_items(s("sourceId")?,page as u32,boolean(a,"refresh",false)?) }
        "source_add" => app.source_add(s("sourceId")?,s("itemId")?,&json!({"record":boolean(a,"record",false)?,"seconds":a.get("seconds"),"title":optional_string(a,"title")?})),
        "gallery_set_dir" => app.gallery_set_dir_cmd(s("dir")?),
        "gallery_sync" => {app.gallery_sync();Ok(json!({"ok":true}))},
        "device_state" => app.device_state(),
        "device_presence" => Ok(app.device_presence()),
        "device_check" => app.device_check(),
        "device_restart" => Ok(app.device_restart()),
        "theme_get" => app.theme_get(s("originalId")?),
        "theme_set" => app.theme_set(s("originalId")?,object(a,"theme")?),
        "theme_standard" => Ok(app.theme_standard()),
        "theme_use" => app.theme_use(s("originalId")?),
        "theme_apply" => app.theme_apply(s("originalId")?),
        "theme_set_base" => app.theme_set_base(s("originalId")?,s("versionId")?),
        "theme_preview" => {
            let (w,h)=(number(a,"w",1200.0)?,number(a,"h",540.0)?);
            if w<1.0 || w>2400.0 || h<1.0 || h>2400.0 || w.fract()!=0.0 || h.fract()!=0.0 {return Err(bad("preview size"));}
            app.theme_preview(s("originalId")?, a.get("theme").filter(|v|!v.is_null()).map(|_|object(a,"theme")).transpose()?,boolean(a,"withLive",true)?,w as u32,h as u32,a.get("phase").filter(|v|!v.is_null()).map(|_|number(a,"phase",0.0)).transpose()?)
        }
        "theme_asset_add" => { if a.get("path").is_some() {app.theme_asset_add_path(s("originalId")?,s("path")?)} else {app.theme_asset_add_bytes(s("originalId")?,s("name")?,&decode_bytes(a)?)} }
        "calibration_set" => {let guides=a.get("guides").filter(|v|!v.is_null()).map(|_|object(a,"guides").map(cs_core::live::theme::Guides::from_value)).transpose()?;app.live.set_calibration(guides);Ok(json!({"live":app.live.running()}))},
        "fit_cal" => { if !["start","set","end"].contains(&s("action")?) {return Err(bad("fit action"));} app.fit_cal(a) },
        "weather_search" => app.weather_search(s("query")?),
        "weather_set_place" => app.weather_set_place(a.get("place").ok_or_else(||bad("place"))?),
        "ext_sensor_status" => {let s=cs_core::live::extsensors::status();Ok(json!({"source":s.source,"ok":s.ok,"message":s.message}))},
        "game_fps_status" => {let s=cs_core::live::gamefps::status();Ok(json!({"ok":s.ok,"state":s.state,"message":s.message,"command":s.command}))},
        "lyrics_status" => {let s=cs_core::live::lyrics::status();Ok(json!({"track":s.track,"key":s.key,"state":s.state,"source":s.source,"offset":s.offset}))},
        "lyrics_offset_set" => {let key=s("key")?;if key.is_empty()||key.len()>64||!key.chars().all(|c|c.is_ascii_alphanumeric()){return Err(bad("lyrics key"));}Ok(json!({"offset":cs_core::live::lyrics::set_offset(&cs_core::live::lyrics::default_dir(),key,number(a,"seconds",0.0)?)}))},
        "screen_preview" => match (optional_string(a,"originalId")?,optional_string(a,"sourceId")?,optional_string(a,"item")?) {
            (Some(oid),_,_)=>app.preview_theme_on_screen(oid),
            (None,Some(source),Some(item))=>app.preview_picture_on_screen(source,item,optional_string(a,"thumb")?),
            _=>Err(bad("preview target")),
        },
        "screen_preview_end" => Ok(if boolean(a,"keepalive",false)? {app.preview_keepalive_on_screen()} else {app.preview_end_on_screen()}),
        "twin_watch" => {if boolean(a,"on",true)?{app.live.twin_start()}else{app.live.twin_stop()};Ok(json!({"running":app.live.running(),"twinning":app.live.twinning()}))},
        "live_set" => live_set(app,a),
        "activate" => {
            // The host invokes this only after readiness for its saved enabled engine.
            // Merely spawning the worker still never starts Live.
            if app.settings.load().live_in_app() {
                app.prepare_live();
                let config = app.live_config();
                app.live.start_with(config.clone());
                app.live.reload(&config);
            }
            Ok(app.app_state())
        },
        "original_reveal" | "gallery_open" | "device_setup" | "device_restore" | "device_reconnect" => {
            if app.device.fake_mode().is_some() {return Err(CoreError::coded("test_mode","This operation is disabled in simulated mode.").status(403));}
            match command {
                "original_reveal"=>app.original_reveal(s("originalId")?),
                "gallery_open"=>app.gallery_open(),
                "device_setup"|"device_restore"|"device_reconnect"=>{
                    let (which,file)=match command {"device_setup"=>("setup","setup-winusb.ps1"),"device_restore"=>("restore","restore-serial.ps1"),_=>("reconnect","reconnect.ps1")};
                    if !app.device.scripts_dir().join(file).is_file(){return Err(CoreError::coded("resources","The cooler setup resource is missing. Reinstall Unified Control.").status(503));}
                    app.device_script(which)
                },
                _=>unreachable!()
            }
        },
        _ => Err(CoreError::coded("unsupported","Unknown cooler command.").status(404)),
    }
}

/// The health reply deliberately omits raw messages, settings, theme IDs/paths
/// and firmware replies. Connection stage is coarse; this does not claim to
/// identify which individual handshake write failed.
fn health_value(live: &Value) -> Value {
    let connection = &live["connection"];
    let state = connection["state"].as_str().unwrap_or("error");
    let message = connection["message"]
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    let held = live["paused"] == true
        || live["deviceHold"] == true
        || live["systemHold"] == true
        || live["idle"] == true;
    let (state, code, stage) = if live["running"] != true {
        ("off", "off", "off")
    } else if held {
        ("paused", "paused", "hold")
    } else if live["connected"] == true {
        ("connected", "connected", "streaming")
    } else {
        let code = match state {
            "connecting" => "connecting",
            "notFound" => "not_found",
            "failedUsb" => "descriptor_failure",
            "busy" => "busy",
            "noReply" => "no_reply",
            _ if message.contains("timed out") || message.contains("timeout") => "write_timeout",
            _ if message.contains("short write") => "short_write",
            _ => "transport_error",
        };
        (
            match state {
                "connecting" | "notFound" | "failedUsb" | "busy" | "noReply" | "error" => state,
                _ => "error",
            },
            code,
            "connecting",
        )
    };
    json!({"running":live["running"]==true,"connected":live["connected"]==true,
        "paused":live["paused"]==true,"deviceHold":live["deviceHold"]==true,"systemHold":live["systemHold"]==true,"idle":live["idle"]==true,
        "connectionState":state,"code":code,"stage":stage,"attempts":connection["attempts"].as_u64().unwrap_or(0),"retryInSeconds":connection["retryInSeconds"].as_u64().unwrap_or(0).min(60),
        "measuredFps":live["measuredFps"].as_f64().filter(|v|v.is_finite()&&*v>=0.0&&*v<=1000.0).unwrap_or(0.0),
        "rxBytes":cs_core::device::frame::IN_BYTES.load(std::sync::atomic::Ordering::Relaxed),
        "rxTransfers":cs_core::device::frame::IN_TRANSFERS.load(std::sync::atomic::Ordering::Relaxed),
        "statusFrames":cs_core::device::frame::STATUS_FRAMES.load(std::sync::atomic::Ordering::Relaxed)})
}

fn shutdown(app: &App) {
    app.live.stop();
    app.shutdown();
    cs_core::live::gamefps::shutdown();
    cs_core::media::shutdown();
}

fn jobs_busy(app: &App) -> bool {
    // Preserve CoolerScreen's user-work contract. Thumbnail/preview jobs are
    // cancellable background work; shutdown cancels them through App::shutdown.
    app.busy_for_quit() || app.uploading()
}

#[cfg(windows)]
struct DeviceLease(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl DeviceLease {
    fn acquire(fake: bool) -> CoreResult<Option<Self>> {
        if fake {
            return Ok(None);
        }
        use windows_sys::Win32::{
            Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS},
            System::Threading::CreateMutexW,
        };
        let name: Vec<u16> = "Local\\UnifiedControlCoolerEngine\0"
            .encode_utf16()
            .collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(CoreError::coded(
                "ownership",
                "Could not acquire the cooler device lock.",
            ));
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            return Err(CoreError::coded(
                "cooler_owned",
                "Another Unified Control cooler engine is running.",
            )
            .status(409));
        }
        Ok(Some(Self(handle)))
    }
}
#[cfg(windows)]
impl Drop for DeviceLease {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.0);
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
mod session {
    use super::*;
    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        UI::WindowsAndMessaging::*,
    };
    static APP: std::sync::OnceLock<std::sync::Weak<App>> = std::sync::OnceLock::new();
    unsafe extern "system" fn procedure(
        hwnd: HWND,
        message: u32,
        wp: WPARAM,
        lp: LPARAM,
    ) -> LRESULT {
        let Some(app) = APP.get().and_then(std::sync::Weak::upgrade) else {
            return DefWindowProcW(hwnd, message, wp, lp);
        };
        let cfg = app.settings.load().live();
        let live = &app.live;
        match message {
            WM_QUERYENDSESSION => 1,
            WM_ENDSESSION => {
                if wp != 0 {
                    live.stop_fast(cfg.screen_off_on_shutdown);
                }
                0
            }
            WM_POWERBROADCAST => {
                match wp as u32 {
                    PBT_APMSUSPEND => match cfg.on_sleep.as_str() {
                        "loop" => {
                            live.hold(cs_core::core::live_host::SLEEP, Duration::from_millis(2500))
                        }
                        "off" => live.hold_screen_off(cs_core::core::live_host::SLEEP),
                        _ => {}
                    },
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                        live.release(cs_core::core::live_host::SLEEP, Duration::from_secs(3))
                    }
                    _ => {}
                }
                1
            }
            0x02B1 => {
                match wp {
                    7 => {
                        let live = live.clone();
                        std::thread::spawn(move || match cfg.on_lock.as_str() {
                            "loop" => {
                                live.hold(cs_core::core::live_host::LOCK, Duration::from_secs(3))
                            }
                            "off" => live.hold_screen_off(cs_core::core::live_host::LOCK),
                            _ => {}
                        });
                    }
                    8 => live.release(cs_core::core::live_host::LOCK, Duration::from_secs(1)),
                    _ => {}
                }
                0
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
    pub struct Hook {
        thread: u32,
        join: Option<std::thread::JoinHandle<()>>,
    }
    impl Hook {
        pub fn start(app: &Arc<App>, fake: bool) -> CoreResult<Option<Self>> {
            if fake {
                return Ok(None);
            }
            APP.set(Arc::downgrade(app)).map_err(|_| {
                CoreError::coded("session", "The cooler session listener already exists.")
            })?;
            let (tx, rx) = std::sync::mpsc::channel();
            let join = std::thread::spawn(move || unsafe {
                let name: Vec<u16> = "UnifiedControlCoolerSession\0".encode_utf16().collect();
                let class = WNDCLASSW {
                    lpfnWndProc: Some(procedure),
                    lpszClassName: name.as_ptr(),
                    ..std::mem::zeroed()
                };
                RegisterClassW(&class);
                let hwnd = CreateWindowExW(
                    0,
                    name.as_ptr(),
                    name.as_ptr(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                );
                if hwnd.is_null() {
                    let _ = tx.send(None);
                    return;
                }
                // Never shown. A top-level window receives power and end-session notifications.
                windows_sys::Win32::System::RemoteDesktop::WTSRegisterSessionNotification(hwnd, 0);
                let _ = tx.send(Some(
                    windows_sys::Win32::System::Threading::GetCurrentThreadId(),
                ));
                let mut message: MSG = std::mem::zeroed();
                while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                windows_sys::Win32::System::RemoteDesktop::WTSUnRegisterSessionNotification(hwnd);
                DestroyWindow(hwnd);
                UnregisterClassW(name.as_ptr(), std::ptr::null_mut());
            });
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(Some(thread)) => Ok(Some(Self {
                    thread,
                    join: Some(join),
                })),
                _ => Err(CoreError::coded(
                    "session",
                    "Could not start cooler session handling.",
                )),
            }
        }
    }
    impl Drop for Hook {
        fn drop(&mut self) {
            unsafe { PostThreadMessageW(self.thread, WM_QUIT, 0, 0) };
            if let Some(join) = self.join.take() {
                let _ = join.join();
            }
        }
    }
}

fn home_from_env() -> CoreResult<PathBuf> {
    if std::env::var("UC_COOLER_WORKER").as_deref() != Ok("1") {
        return Err(CoreError::coded(
            "launch",
            "Start the cooler from Unified Control.",
        ));
    }
    let home = std::env::var_os("COOLERSCREEN_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .ok_or_else(|| CoreError::coded("launch", "An explicit cooler data folder is required."))?;
    Ok(home)
}

pub fn run() -> CoreResult<()> {
    let home = home_from_env()?;
    let fake = cs_core::core::device_service::fake_mode_from_env().is_some()
        || std::env::var("COOLERSCREEN_FAKE_MEDIA").is_ok_and(|v| v == "1");
    let blocked = native_conflict(fake).err();
    #[cfg(windows)]
    let _lease = if blocked.is_none() {
        DeviceLease::acquire(fake)?
    } else {
        None
    };
    std::fs::create_dir_all(&home)?;
    // A new worker must not default to a shared Documents gallery. Migrated explicit paths survive.
    if !home.join("settings.json").exists() {
        let gallery = home.join("Gallery");
        std::fs::create_dir_all(&gallery)?;
        std::fs::write(
            home.join("settings.json"),
            serde_json::to_vec(&json!({"galleryDir":gallery})).unwrap(),
        )?;
    }
    let out = Output(Arc::new(Mutex::new(std::io::stdout())));
    let backends = if fake {
        Backends {
            media: Arc::new(FakeMedia::default()),
            sources: Arc::new(NoSources),
        }
    } else {
        cs_core::backends_with_work_dir(home.join("work"))
    };
    let app = App::new(
        &home,
        backends,
        Arc::new(out.clone()),
        env!("CARGO_PKG_VERSION"),
    );
    app.live.set_no_device(fake);
    #[cfg(windows)]
    let _session = if blocked.is_none() {
        session::Hook::start(&app, fake)?
    } else {
        None
    };
    let resolver = app.clone();
    let server = MediaServer::start(Arc::new(move |path, query| {
        resolver.resolve_media(path, query)
    }))?;
    app.set_media_base(&server.base_url());
    if blocked.is_none() {
        app.startup();
    }
    // Starting the worker never starts saved live playback. Only an explicit UI operation can do that.
    out.event(
        "ready",
        json!({"engineVersion":env!("CARGO_PKG_VERSION"),"capabilities":COMMANDS,"fake":fake,
        "blocked":blocked.is_some(),"message":blocked.as_ref().map(|e|e.message.as_str())}),
    );
    let active = Arc::new((Mutex::new(0usize), Condvar::new()));
    let mutations = Arc::new(Mutex::new(()));
    let mut input = BufReader::new(std::io::stdin());
    let mut protocol_error = None;
    loop {
        let frame = match read_frame(&mut input) {
            Ok(Some(frame)) => frame,
            Ok(None) => break,
            Err(error) => {
                protocol_error = Some(error);
                break;
            }
        };
        let request = match parse(&frame) {
            Ok(request) => request,
            Err(error) => {
                let id = serde_json::from_slice::<Value>(&frame)
                    .ok()
                    .and_then(|v| v["id"].as_u64());
                if let Some(id) = id {
                    out.reply(id, Err(error));
                    continue;
                } else {
                    protocol_error = Some(error);
                    break;
                }
            }
        };
        let mut count = active.0.lock().unwrap();
        if request.command == "shutdown" {
            if *count > 0 || jobs_busy(&app) {
                out.reply(
                    request.id,
                    Err(CoreError::coded(
                        "busy",
                        "CoolerScreen is busy. Try again when its jobs finish.",
                    )
                    .status(409)),
                );
                continue;
            }
            shutdown(&app);
            out.reply(request.id, Ok(json!({"stopped":true})));
            break;
        }
        if let Some(error) = &blocked {
            if ![
                "app_state",
                "activity",
                "job_get",
                "originals_list",
                "theme_standard",
                "sources_list",
                "health_status",
            ]
            .contains(&request.command.as_str())
            {
                out.reply(request.id, Err(error.clone()));
                continue;
            }
        }
        if *count >= MAX_REQUESTS {
            out.reply(
                request.id,
                Err(CoreError::coded("busy", "Too many cooler requests.").status(429)),
            );
            continue;
        }
        *count += 1;
        drop(count);
        let (app, out, active, mutations) =
            (app.clone(), out.clone(), active.clone(), mutations.clone());
        std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                // State/activity are responsive while a slow import or preview runs.
                let _guard = if [
                    "app_state",
                    "activity",
                    "job_get",
                    "device_presence",
                    "health_status",
                ]
                .contains(&request.command.as_str())
                {
                    None
                } else {
                    Some(mutations.lock().unwrap())
                };
                dispatch(&app, &request.command, &request.args)
            }))
            .unwrap_or_else(|_| {
                Err(CoreError::coded("worker", "Cooler request failed.").status(500))
            });
            let mut count = active.0.lock().unwrap();
            *count -= 1;
            active.1.notify_all();
            drop(count);
            // A completed response is an admission boundary: the host may send
            // shutdown immediately after receiving it.
            out.reply(request.id, result);
        });
    }
    // Parent loss closes stdin. Let admitted operations and noncancellable uploads settle before cleanup.
    let mut count = active.0.lock().unwrap();
    while *count > 0 {
        count = active.1.wait(count).unwrap();
    }
    drop(count);
    while jobs_busy(&app) {
        std::thread::sleep(Duration::from_millis(100));
    }
    shutdown(&app);
    drop(server);
    protocol_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn health_is_read_only_whitelist_and_keeps_timeout_distinct_from_absence() {
        let live = json!({"running":true,"connected":false,"connection":{"state":"error","message":"USB write timed out seed-secret C:\\private\\file","attempts":9,"retryInSeconds":15},"theme":"secret","account":"secret"});
        let health = health_value(&live);
        assert_eq!(health["code"], "write_timeout");
        assert_eq!(health["stage"], "connecting");
        let serialized = health.to_string();
        for forbidden in ["seed-secret", "private", "theme", "account"] {
            assert!(!serialized.contains(forbidden));
        }
        let mut absent = live.clone();
        absent["connection"]["state"] = json!("notFound");
        assert_eq!(health_value(&absent)["code"], "not_found");
        absent["systemHold"] = json!(true);
        assert_eq!(health_value(&absent)["stage"], "hold");
        assert!(parse(br#"{"v":1,"id":1,"command":"health_status","args":{}}"#).is_ok());
    }
    #[test]
    fn bounded_protocol_rejects_unknown_and_wrong_types() {
        assert!(parse(br#"{"v":1,"id":2,"command":"activity","args":{}}"#).is_ok());
        for frame in [
            br#"{"v":2,"id":2,"command":"activity","args":{}}"#.as_slice(),
            br#"{"v":1,"id":2,"command":"shell","args":{}}"#,
            br#"{"v":1,"id":2,"command":"activity","args":{"path":"x"}}"#,
            br#"{"v":1,"id":2,"command":"twin_watch","args":{"on":"yes"}}"#,
            br#"{"v":1,"id":2,"command":"activity","args":{},"extra":true}"#,
        ] {
            assert!(parse(frame).is_err());
        }
        assert!(read_frame(&mut BufReader::new(&b"partial"[..])).is_err());
        let too_big = vec![b'x'; MAX_FRAME + 1];
        assert!(read_frame(&mut BufReader::new(too_big.as_slice())).is_err());
    }
    #[test]
    fn file_payloads_are_strict() {
        assert_eq!(decode_base64("AAECA/8=").unwrap(), vec![0, 1, 2, 3, 255]);
        for invalid in ["AA=", "A===", "AA=A", "AB==", "AAB=", "AA==AAAA"] {
            assert!(decode_base64(invalid).is_err());
        }
        assert!(decode_bytes(&json!({"bytes":[256]})).is_err());
        assert!(decode_bytes(&json!({"bytes":[],"base64":""})).is_err());
    }
    #[test]
    fn fake_core_dispatch_preserves_theme_and_saved_live_settings() {
        let dir = tempfile::tempdir().unwrap();
        struct RestoreHome(Option<std::ffi::OsString>);
        impl Drop for RestoreHome {
            fn drop(&mut self) {
                match self.0.take() {
                    Some(value) => std::env::set_var("COOLERSCREEN_HOME", value),
                    None => std::env::remove_var("COOLERSCREEN_HOME"),
                }
            }
        }
        let _restore = RestoreHome(std::env::var_os("COOLERSCREEN_HOME"));
        std::env::set_var("COOLERSCREEN_HOME", dir.path());
        let app = App::with_device(
            dir.path().into(),
            Backends {
                media: Arc::new(FakeMedia::default()),
                sources: Arc::new(NoSources),
            },
            Arc::new(cs_core::core::app::NoEvents),
            "worker-test",
            cs_core::core::device_service::DeviceService::fake("winusb", 0.01, false),
        );
        app.live.set_no_device(true);
        let state = dispatch(
            &app,
            "live_set",
            &json!({"patch":{"inApp":false,"showSeconds":true,"fps":30}}),
        )
        .unwrap();
        assert_eq!(state["live"]["settings"]["showSeconds"], true);
        assert_eq!(app.live.fps(), 30);
        assert!(!app.live.running());
        assert!(dispatch(&app, "live_set", &json!({"patch":{"fps":45}})).is_err());
        assert!(dispatch(&app, "live_set", &json!({"patch":{"autostart":true}})).is_err());
        assert!(dispatch(&app, "device_setup", &json!({}))
            .unwrap_err()
            .code_is("test_mode"));
        assert!(dispatch(&app, "activate", &json!({})).is_ok());
        assert!(!app.live.running());
        // The editor can open its own temporary twin before Live is enabled.
        // Enabling Live must claim that pipeline: closing the editor must not
        // switch the saved Live setting back to an effectively stopped stream.
        dispatch(&app, "twin_watch", &json!({"on":true})).unwrap();
        assert!(app.live.running() && app.live.twinning());
        dispatch(&app, "live_set", &json!({"patch":{"inApp":true,"music":false}})).unwrap();
        assert!(!app.live.twinning(), "enabled Live owns the stream instead of the editor");
        dispatch(&app, "twin_watch", &json!({"on":false})).unwrap();
        assert!(app.live.running(), "leaving the editor keeps enabled Live running");
        dispatch(&app, "live_set", &json!({"patch":{"inApp":false}})).unwrap();
        assert!(!app.live.running(), "explicitly disabling Live still stops it");
        app.settings
            .update(|settings| settings["live"] = json!({"inApp":true,"music":false}))
            .unwrap();
        assert!(!app.live.running());
        // Host activation can race the first editor twin request after ready.
        dispatch(&app, "twin_watch", &json!({"on":true})).unwrap();
        assert!(app.live.twinning());
        dispatch(&app, "activate", &json!({})).unwrap();
        assert!(app.live.running() && !app.live.twinning(), "saved Live activation claims an early editor stream");
        dispatch(&app, "twin_watch", &json!({"on":false})).unwrap();
        assert!(app.live.running(), "activation survives editor cleanup");
        dispatch(&app, "live_set", &json!({"patch":{"inApp":false}})).unwrap();
        app.live.tune_start(cs_core::core::fit::Fit::default());
        assert!(app.live.tuning() && app.live.running());
        dispatch(&app, "live_set", &json!({"patch":{"inApp":true}})).unwrap();
        app.live.tune_end();
        assert!(app.live.running(), "enabled Live also takes ownership from temporary fit tuning");
        dispatch(&app, "live_set", &json!({"patch":{"inApp":false}})).unwrap();
        app.live.tune_start(cs_core::core::fit::Fit::default());
        dispatch(&app, "live_set", &json!({"patch":{"inApp":false}})).unwrap();
        dispatch(&app, "live_set", &json!({"patch":{"inApp":true}})).unwrap();
        app.live.tune_end();
        assert!(app.live.running(), "explicit Live off clears stale temporary fit ownership before the next enable");
        assert!(dispatch(&app, "theme_standard", &json!({}))
            .unwrap()
            .is_object());
        shutdown(&app);
    }
}
