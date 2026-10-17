//! Everything the 1.2 HTTP endpoints did (`src/server.js`), as `App` methods with the same JSON shapes.
//! `commands.rs` exposes them as Tauri commands (docs/v2/api-contract.md, section 2).
use super::api::{AddOpts, EncodeReq, LoopPointReq, LoopSearchReq, MediaTask, ProbeInfo, SourceCtx, SourceProvider, DEVICE_FPS, IMAGE_SECONDS};
use super::app::App;
use super::error::{CoreError, CoreResult};
use super::js::{self, num, to_num, truthy};
use super::library::{clean_settings, effective_settings};
use super::mask::notch_key;
use super::scheduler::{now_ms, Priority, Res, Resources, TaskSpec};
use super::settings::{budgets, capacity_from};
use crate::device::Slot;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

pub const Q_MIN: f64 = 2.0;
pub const Q_MAX: f64 = 69.0;

/// Friendly quality 1..100 from the mjpeg quantiser (1.2 `qualityFromQ`).
pub fn quality_from_q(q: f64) -> f64 {
    js::round(1.0 + ((Q_MAX - q) * 99.0) / (Q_MAX - Q_MIN)).clamp(1.0, 100.0)
}

fn gone_clip() -> CoreError {
    CoreError::not_found("This clip is no longer in the library.")
}
fn gone_version() -> CoreError {
    CoreError::not_found("This version no longer exists.")
}
fn updating() -> CoreError {
    CoreError::coded("updating", "The screen is busy with another operation.").status(409)
}

fn probe_of(o: &Value) -> ProbeInfo {
    ProbeInfo {
        duration: o["duration"].as_f64(),
        width: o["width"].as_u64().map(|x| x as u32),
        height: o["height"].as_u64().map(|x| x as u32),
        fps: o["fps"].as_f64(),
        codec: o["codec"].as_str().map(String::from),
        has_video: true,
        is_image: truthy(o.get("isImage")),
        rotated: truthy(o.get("rotated")),
    }
}

/// Blend length for a loop candidate: none when the join already looks like a normal frame step (ratio <= 1).
fn auto_blend(c: &Value) -> f64 {
    let ratio = c["ratio"].as_f64().unwrap_or(0.0);
    if !(ratio > 1.0) {
        return 0.0;
    }
    let want: f64 = if ratio <= 1.6 { 0.33 } else { 0.5 };
    let b = want.min(c["start"].as_f64().unwrap_or(0.0)).min(c["length"].as_f64().unwrap_or(0.0) / 2.0);
    js::round((b * DEVICE_FPS as f64 + 1e-6).floor() / DEVICE_FPS as f64 * 1000.0) / 1000.0
}

impl App {
    // ---------- state ----------

    fn on_screen(&self) -> Value {
        let s = self.settings.load();
        let Some(sb) = s.slot("standby").cloned() else { return Value::Null };
        let vid = sb["versionId"].as_str().map(String::from);
        let f = vid.as_deref().and_then(|v| self.lib.find_version(v));
        let o = f.as_ref().and_then(|(oid, _)| self.lib.read_original(oid));
        let mut out = sb.as_object().cloned().unwrap_or_default();
        out.insert("known".into(), json!(vid.is_some()));
        out.insert("exists".into(), json!(f.is_some()));
        out.insert("originalName".into(), o.as_ref().map(|o| o["name"].clone()).unwrap_or(sb["name"].clone()));
        out.insert(
            "label".into(),
            match &f {
                Some((_, v)) if truthy(v.get("label")) => v["label"].clone(),
                Some((_, v)) => json!(format!("v{}", js::fmt(v["n"].as_f64().unwrap_or(0.0)))),
                None => sb.get("label").cloned().unwrap_or(Value::Null),
            },
        );
        out.insert(
            "thumbUrl".into(),
            if f.is_some() { json!(self.lib.url(&format!("/media/v/{}/thumb.jpg", vid.unwrap()))) } else { Value::Null },
        );
        out.insert("originalId".into(), f.map(|(oid, _)| json!(oid)).unwrap_or(sb.get("originalId").cloned().unwrap_or(Value::Null)));
        Value::Object(out)
    }

    fn old_notch_count(&self, key: &str) -> usize {
        let mut n = 0;
        for o in self.lib.list_originals() {
            for v in self.lib.versions(o["id"].as_str().unwrap_or("")) {
                if v["status"] == "ready" && super::mask::base_key(v["notchKey"].as_str()) != key {
                    n += 1;
                }
            }
        }
        n
    }

    pub fn app_state(&self) -> Value {
        let s = self.settings.load();
        let b = budgets(&s, self.lib.splash_size());
        let key = notch_key(s.mask());
        json!({
            "appVersion": self.app_version, "desktopApp": self.sources.desktop_capture(), "deviceFps": DEVICE_FPS,
            "imageSeconds": IMAGE_SECONDS, "fake": self.device.fake_mode(),
            "mask": s.mask(), "cornersDefault": s.mask()["corners"], "maxSeconds": s.0["maxSeconds"],
            "budget": num(b.loop_bytes), "total": num(b.total), "budgetMeasured": b.measured, "capacityAt": b.capacity_at,
            "uploadRate": s.upload_rate().map(num).unwrap_or(json!(50000)),
            "galleryDir": self.gallery_dir().to_string_lossy(), "onScreen": self.on_screen(),
            "missingNoticeDays": super::gallery::MISSING_NOTICE_DAYS, "notchKey": key, "oldNotchCount": self.old_notch_count(&key),
            "mediaBase": self.media_base.lock().unwrap().clone(), "backend": "rust",
            "live": { "settings": s.live().to_full_value(), "status": self.live.status(), "autostart": super::autostart::is_on() },
        })
    }

    /// PUT /api/settings: the notch and maxSeconds (corners default comes from the last version made).
    pub fn settings_update(&self, body: &Value) -> CoreResult<Value> {
        let mask = body.get("mask").filter(|m| truthy(Some(m))).cloned();
        let max = body.get("maxSeconds").filter(|m| !m.is_null()).cloned();
        self.settings.update(|s| {
            if let Some(mut m) = mask {
                m["corners"] = s["mask"]["corners"].clone();
                s["mask"] = m;
            }
            if let Some(x) = max {
                s["maxSeconds"] = x;
            }
        })?;
        self.lib.bump();
        Ok(self.app_state())
    }

    pub fn activity(&self) -> Value {
        json!({ "rev": self.lib.rev(), "deviceBusy": self.sched.device_active(), "jobs": self.sched.activity(120_000) })
    }

    pub fn job_get(&self, id: &str) -> CoreResult<Value> {
        let j = self.sched.job(id).ok_or_else(|| CoreError::not_found("No such job"))?;
        Ok(serde_json::to_value(j).unwrap_or(Value::Null))
    }

    // ---------- originals ----------

    pub fn originals_list(&self) -> Value {
        let ctx = self.lib.context();
        let all = self.lib.list_originals();
        let long: Vec<Value> = self
            .long_missing(now_ms())
            .iter()
            .map(|o| json!({ "id": o["id"], "name": o["name"], "since": o["missing"]["since"] }))
            .collect();
        let cur = self.current_theme_oid();
        let originals: Vec<Value> = all
            .iter()
            .map(|o| {
                let mut s = self.lib.summary(o, &ctx);
                s["theme"] = self.theme_summary(o["id"].as_str().unwrap_or(""), cur.as_deref());
                s
            })
            .collect();
        json!({ "rev": self.lib.rev(), "originals": originals, "longMissing": long, "currentTheme": cur })
    }

    /// POST /api/import: the bytes of a dropped file.
    pub fn import_bytes(&self, name: &str, bytes: &[u8]) -> CoreResult<Value> {
        let ext: String = Path::new(name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '.')
            .collect();
        std::fs::create_dir_all(self.lib.base())?;
        let tmp = self.lib.base().join(format!("incoming-{}{ext}", &super::library::new_id()[..8]));
        std::fs::write(&tmp, bytes)?;
        let base = Path::new(name).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "upload".into());
        let r = self.add_file(&tmp, Some(&base));
        let _ = std::fs::remove_file(&tmp);
        let id = r?;
        self.prefind_loops(&id);
        Ok(json!({ "id": id }))
    }

    /// POST /api/import-path: a local file (drag and drop with a path); copied into the Gallery folder.
    pub fn import_path(&self, src: &str) -> CoreResult<Value> {
        let p = PathBuf::from(src);
        if !p.is_absolute() || !p.is_file() {
            return Err(CoreError::new(format!("File not found: {src}")));
        }
        let id = self.add_file(&p, None)?;
        self.prefind_loops(&id);
        Ok(json!({ "id": id }))
    }

    pub fn missing_delete(&self, ids: &[String]) -> Value {
        let n = self.delete_missing(ids);
        self.refresh_live_theme();
        json!({ "deleted": n })
    }

    pub fn original_get(&self, oid: &str) -> CoreResult<Value> {
        self.lib.detail(oid).ok_or_else(gone_clip)
    }

    /// PATCH: `{ name?, draft? }` (draft null clears it).
    pub fn original_patch(&self, oid: &str, patch: &Value) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(gone_clip)?;
        if let Some(n) = patch.get("name").filter(|n| !n.is_null()) {
            self.rename_original(oid, n.as_str().unwrap_or(&n.to_string()))?;
        }
        if let Some(d) = patch.get("draft") {
            let o = self.lib.read_original(oid).ok_or_else(gone_clip)?;
            let draft = if truthy(Some(d)) { clean_settings(&o, d, self.settings.load().max_seconds(), &Value::Null) } else { Value::Null };
            self.lib.patch_original(oid, &json!({ "draft": draft }))?;
        }
        self.original_get(oid)
    }

    pub fn original_delete(&self, oid: &str) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(gone_clip)?;
        self.delete_original(oid)?;
        self.refresh_live_theme();
        Ok(json!({ "ok": true }))
    }

    pub fn original_reveal(&self, oid: &str) -> CoreResult<Value> {
        let o = self.lib.read_original(oid).ok_or_else(gone_clip)?;
        let f = o["file"].as_str().map(PathBuf::from).filter(|f| f.exists());
        #[cfg(windows)]
        {
            let mut c = std::process::Command::new("explorer.exe");
            match &f {
                Some(f) => c.arg(format!("/select,{}", f.display())),
                None => c.arg(self.gallery_dir()),
            };
            let _ = c.spawn();
        }
        let _ = f;
        Ok(json!({ "ok": true }))
    }

    // ---------- tuning: estimate, loops ----------

    fn encode_req(&self, oid: &str, o: &Value, st: &Value, ctx: &super::library::Ctx) -> CoreResult<EncodeReq> {
        let src = self.lib.source_path(o).ok_or_else(|| CoreError::new("The original file is missing."))?;
        Ok(EncodeReq {
            file: src,
            dir: self.lib.o_dir(oid),
            start: st["start"].as_f64().unwrap_or(0.0),
            length: st["length"].as_f64().unwrap_or(0.0),
            quality: st["quality"].clone(),
            detail: st["detail"].as_f64().unwrap_or(0.0) as u32,
            frame: st["frame"].clone(),
            mask: ctx.mask.clone(),
            corners: st["corners"].clone(),
            blend: st["blend"].as_f64().unwrap_or(0.0),
            budget_bytes: ctx.budget.max(0.0) as u64,
            max_seconds: ctx.max_seconds,
            info: probe_of(o),
            overlay: None,
        })
    }

    /// The size meter: superseded requests for the same clip return `{ stale: true }`.
    pub fn estimate(&self, oid: &str, body: &Value) -> CoreResult<Value> {
        if self.sched.device_active() && self.sched.policy().device_blocks_media {
            return Err(CoreError::coded("busy", "Size check paused while the screen is updating.").status(409));
        }
        let o = self.lib.read_original(oid).ok_or_else(gone_clip)?;
        let ctx = self.lib.context();
        let st = clean_settings(&o, body, ctx.max_seconds, &ctx.corners_default);
        let req = self.encode_req(oid, &o, &st, &ctx)?;
        let media = self.media.clone();
        let res = media.resources(MediaTask::Estimate, Some(&req.info));
        let spec = TaskSpec::new("estimate", "Checking the size", res)
            .original(oid)
            .priority(Priority::Selected)
            .coalesce(format!("estimate:{oid}"))
            .cancel_on_blur();
        match self.run_task(spec, move |ctx| media.estimate(ctx, &req)) {
            Ok(mut r) => {
                if r.get("quality").is_none() {
                    if let Some(q) = r["q"].as_f64() {
                        r["quality"] = num(quality_from_q(q));
                    }
                }
                r["settings"] = st;
                Ok(r)
            }
            Err(e) if e.is_cancelled() => Ok(json!({ "stale": true })),
            Err(e) => Err(e),
        }
    }

    pub fn loop_point(&self, oid: &str, body: &Value) -> CoreResult<Value> {
        let o = self.lib.read_original(oid).ok_or_else(gone_clip)?;
        let src = self.lib.source_path(&o).ok_or_else(|| CoreError::new("The original file is missing."))?;
        if truthy(o.get("isImage")) {
            return Err(CoreError::new("A still image always loops seamlessly."));
        }
        let st = clean_settings(&o, body, 120.0, &Value::Null);
        let start = st["start"].as_f64().unwrap_or(0.0);
        let req = LoopPointReq { file: src, start, end: start + st["length"].as_f64().unwrap_or(0.0), duration: o["duration"].as_f64(), fps: o["fps"].as_f64() };
        let media = self.media.clone();
        let res = media.resources(MediaTask::LoopPoint, None);
        self.run_task(TaskSpec::new("looppoint", "Finding the loop point", res).original(oid).priority(Priority::Selected), move |ctx| {
            media.loop_point(ctx, &req)
        })
    }

    /// Whole-clip loop search as a visible job; the result is stored with the original (1.2 `searchLoops`).
    pub fn loop_search(&self, oid: &str, body: &Value) -> CoreResult<Value> {
        let auto = !truthy(body.get("minLen")) && !truthy(body.get("maxLen"));
        // Opening a clip whose automatic search already runs (started when it was added) joins that search.
        if auto {
            if let Some(id) = self.sched.live_with_key(&format!("loopsearch:{oid}")) {
                return Ok(json!({ "jobId": id }));
            }
        }
        self.submit_loop_search(oid, body, false)
    }

    /// A clip just added: find its loops now, in the background, so its loop cards are ready when it is opened.
    pub fn prefind_loops(&self, oid: &str) {
        let Some(o) = self.lib.read_original(oid) else { return };
        if truthy(o.get("isImage")) || super::library::loops_current(&o) {
            return;
        }
        let _ = self.submit_loop_search(oid, &json!({}), true);
    }

    fn submit_loop_search(&self, oid: &str, body: &Value, background: bool) -> CoreResult<Value> {
        let o = self.lib.read_original(oid).ok_or_else(gone_clip)?;
        let name = o["name"].as_str().unwrap_or("clip").to_string();
        let res = self.media.resources(MediaTask::LoopSearch, Some(&probe_of(&o)));
        let mut spec = TaskSpec::new("loopsearch", format!("Finding loop points in \"{name}\""), res)
            .original(oid)
            .priority(if background { Priority::Background } else { Priority::Selected })
            .auto_priority()
            .coalesce(format!("loopsearch:{oid}"))
            .visible();
        if !background {
            spec = spec.cancel_on_blur(); // leaving a clip stops its search (the UI searches again when the clip is reopened)
        }
        let (app, oid2, body) = (self.arc(), oid.to_string(), body.clone());
        let h = self.sched.submit(spec, move |ctx| app.search_loops(ctx, &oid2, &body));
        Ok(json!({ "jobId": h.id }))
    }

    fn search_loops(&self, tctx: &super::scheduler::TaskCtx, oid: &str, body: &Value) -> CoreResult<Value> {
        let o = self.lib.read_original(oid).ok_or_else(|| CoreError::new("The original file is missing."))?;
        let src = self.lib.source_path(&o).ok_or_else(|| CoreError::new("The original file is missing."))?;
        if truthy(o.get("isImage")) {
            return Err(CoreError::new("A still image always loops seamlessly."));
        }
        let ctx = self.lib.context();
        let dur = o["duration"].as_f64().unwrap_or(0.0);
        let ml = to_num(body.get("minLen"));
        let min_len = (if ml.is_nan() || ml == 0.0 { 3.0 } else { ml }).min(dur - 0.2).max(0.5);
        let mut max_len = to_num(body.get("maxLen"));
        let mut suggested = Value::Null;
        if !(max_len > 0.0) {
            // "Auto": the longest loop Best that fits can still make look good (quality 25).
            tctx.progress(0.005, "Working out the longest loop that fits");
            let draft = o.get("draft").filter(|d| d.is_object()).cloned().unwrap_or(json!({}));
            let start = to_num(draft.get("start"));
            let st = json!({
                "start": num(if start.is_nan() { 0.0 } else { start }), "length": num(dur.min(2.0)), "quality": 25,
                "frame": super::mask::norm_frame(draft.get("frame").unwrap_or(&Value::Null)),
                "corners": draft.get("corners").cloned().unwrap_or(Value::Null), "detail": 0, "blend": 0,
            });
            let req = self.encode_req(oid, &o, &st, &ctx)?;
            let bps = self.media.bytes_per_second(tctx, &req, 25)?;
            let s = ((ctx.budget * 0.94 / bps) * 2.0).floor() / 2.0;
            let s = s.max(1.0);
            suggested = num(s);
            max_len = s;
        }
        let max_len = max_len.min(dur - 0.2).min(30.0).max(min_len);
        let req = LoopSearchReq {
            file: src.clone(),
            dir: self.lib.o_dir(oid),
            original_id: oid.into(),
            duration: dur,
            fps: o["fps"].as_f64(),
            min_len,
            max_len,
            budget_bytes: ctx.budget.max(0.0) as u64,
        };
        let mut r = self.media.loop_search(tctx, &req)?;
        let t = now_ms();
        if let Some(cs) = r.get_mut("candidates").and_then(Value::as_array_mut) {
            for (i, c) in cs.iter_mut().enumerate() {
                if c.get("blend").is_none() {
                    c["blend"] = num(auto_blend(c));
                }
                c["preview"] = json!(format!("/media/o/{oid}/loop-{i}.mp4?t={t}"));
            }
        }
        let mut result = r.as_object().cloned().unwrap_or_default();
        result.insert("suggestedMax".into(), suggested);
        result.insert("minLen".into(), body.get("minLen").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
        result.insert("maxLen".into(), body.get("maxLen").filter(|v| truthy(Some(v))).cloned().unwrap_or(Value::Null));
        let mut stored = Map::new();
        stored.insert("key".into(), json!(match o.get("fileMtime") {
            Some(Value::Number(n)) => js::fmt(n.as_f64().unwrap_or(0.0)),
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) => "null".into(),
            _ => "undefined".into(),
        }));
        stored.insert("at".into(), json!(js::iso_now()));
        stored.insert("search".into(), json!(super::library::LOOP_SEARCH_VERSION));
        for (k, v) in &result {
            stored.insert(k.clone(), v.clone());
        }
        self.lib.patch_original(oid, &json!({ "loops": stored }))?;
        self.cards_job(oid, &src, dur, o["fps"].as_f64(), result.get("candidates").cloned().unwrap_or(json!([])));
        // The UI gets absolute card URLs (the media server holds a card request until the cards job made it).
        let mut out = Value::Object(result);
        if let Some(cs) = out.get_mut("candidates").and_then(Value::as_array_mut) {
            for c in cs {
                let p = c["preview"].as_str().unwrap_or("").to_string();
                c["preview"] = json!(self.lib.url(&p));
            }
        }
        Ok(out)
    }

    /// Loop card previews as their own job, so the size meter for the picked loop runs alongside. A newer search's
    /// cards replace older ones for the same clip (coalesced); the "pending" mark (the media server waits for cards
    /// while it is set) is cleared when this job ends in any way, unless a newer cards job took over.
    pub(crate) fn cards_job(&self, oid: &str, src: &Path, duration: f64, fps: Option<f64>, candidates: Value) {
        let dir = self.lib.o_dir(oid);
        let req = super::api::LoopCardsReq { file: src.to_path_buf(), dir: dir.clone(), candidates, duration, fps };
        let o = self.lib.read_original(oid);
        let res = self.media.resources(MediaTask::Cards, o.as_ref().map(probe_of).as_ref());
        let spec = TaskSpec::new("cards", "Making the loop previews", res)
            .original(oid)
            .priority(Priority::Selected)
            .auto_priority()
            .preemptible()
            .coalesce(format!("cards:{oid}"));
        let app = self.arc();
        let mut pending = self.cards_pending.lock().unwrap();
        let h = self.sched.submit(spec, move |ctx| {
            // Old cards belong to the previous search (an older cards job was cancelled by coalescing).
            if let Ok(rd) = std::fs::read_dir(&dir) {
                for e in rd.filter_map(Result::ok) {
                    let n = e.file_name().to_string_lossy().to_string();
                    if n.starts_with("loop-") && n.ends_with(".mp4") {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
            }
            app.media.loop_cards(ctx, &req).map(|_| Value::Null)
        });
        pending.insert(oid.to_string(), h.id.clone());
        drop(pending);
        let (app, oid) = (self.arc(), oid.to_string());
        std::thread::spawn(move || {
            let _ = h.wait(); // done, failed or cancelled (a preempted job is re-queued and does not end here)
            let mut p = app.cards_pending.lock().unwrap();
            if p.get(&oid) == Some(&h.id) {
                p.remove(&oid);
            }
        });
    }

    // ---------- versions ----------

    fn version_job(&self, oid: &str, v: &Value) -> String {
        self.version_handle(oid, v).id
    }

    pub(crate) fn version_handle(&self, oid: &str, v: &Value) -> super::scheduler::JobHandle {
        let o = self.lib.read_original(oid);
        let label = format!("{} · v{}", o.as_ref().and_then(|o| o["name"].as_str()).unwrap_or("clip"), js::fmt(v["n"].as_f64().unwrap_or(0.0)));
        let info = o.as_ref().map(probe_of);
        let res = self.media.resources(MediaTask::Convert, info.as_ref());
        let vid = v["id"].as_str().unwrap_or("").to_string();
        let spec = TaskSpec::new("version", label, res).original(oid).version(&vid).priority(Priority::User).visible();
        let app = self.arc();
        self.sched.submit(spec, move |ctx| app.run_version(ctx, &vid))
    }

    fn run_version(&self, tctx: &super::scheduler::TaskCtx, vid: &str) -> CoreResult<Value> {
        let (oid, v) = self.lib.find_version(vid).ok_or_else(|| CoreError::new("This version was deleted."))?;
        let o = self.lib.read_original(&oid).ok_or_else(|| CoreError::new("This version was deleted."))?;
        if self.lib.source_path(&o).is_none() {
            self.lib.patch_version(vid, &json!({ "status": "error", "error": "The original file is missing." }))?;
            return Err(CoreError::new("The original file is missing."));
        }
        let ctx = self.lib.context();
        self.lib.patch_version(vid, &json!({ "status": "converting", "error": null }))?;
        let st = effective_settings(&v);
        let mut c2 = ctx.clone();
        c2.mask = v.get("notch").cloned().unwrap_or(ctx.mask.clone());
        let mut req = self.encode_req(&oid, &o, &st, &c2)?;
        if let Some(f) = v["overlay"]["file"].as_str() {
            if !std::path::Path::new(f).is_file() {
                let msg = "The text and pictures for this base are missing. Apply the theme again to make them.";
                self.lib.patch_version(vid, &json!({ "status": "error", "error": msg }))?;
                return Err(CoreError::new(msg));
            }
            req.overlay = Some(PathBuf::from(f));
        }
        let out = self.lib.v_dir(&oid, vid);
        match self.media.convert(tctx, &req, &out) {
            Ok(r) => {
                if self.lib.find_version(vid).is_none() {
                    return Err(CoreError::new("This version was deleted."));
                }
                let g = |k: &str| r.get(k).cloned().unwrap_or(Value::Null);
                let quality = r.get("quality").cloned().unwrap_or_else(|| r["q"].as_f64().map(|q| num(quality_from_q(q))).unwrap_or(Value::Null));
                self.lib.patch_version(
                    vid,
                    &json!({ "status": "ready", "error": null, "result": {
                        "size": g("size"), "q": g("q"), "quality": quality, "detail": r.get("detail").cloned().unwrap_or(json!(0)),
                        "fps": g("fps"), "start": g("start"), "length": g("length"), "tooBig": g("tooBig"), "budgetBytes": num(ctx.budget) } }),
                )
            }
            Err(e) => {
                if self.lib.find_version(vid).is_some() && !e.is_cancelled() {
                    self.lib.patch_version(
                        vid,
                        &json!({ "status": "error", "error": e.message, "errorCode": e.code, "maxFitLength": e.extra.get("maxFitLength").cloned().unwrap_or(Value::Null) }),
                    )?;
                }
                Err(e)
            }
        }
    }

    pub fn version_create(&self, oid: &str, body: &Value) -> CoreResult<Value> {
        self.lib.read_original(oid).ok_or_else(gone_clip)?;
        let v = self.lib.create_version(oid, body)?;
        let job = self.version_job(oid, &v);
        Ok(json!({ "versionId": v["id"], "jobId": job }))
    }

    pub fn version_patch(&self, vid: &str, label: &str) -> CoreResult<Value> {
        self.lib.find_version(vid).ok_or_else(gone_version)?;
        let l: String = label.trim().chars().take(40).collect();
        self.lib.patch_version(vid, &json!({ "label": if l.is_empty() { Value::Null } else { json!(l) } }))
    }

    /// A queued/converting version is cancelled first, then deleted (1.2 refused with 409).
    pub fn version_delete(&self, vid: &str) -> CoreResult<Value> {
        let (_, v) = self.lib.find_version(vid).ok_or_else(gone_version)?;
        if v["status"] == "queued" || v["status"] == "converting" {
            self.sched.cancel_version(vid);
            let end = std::time::Instant::now() + Duration::from_secs(10);
            while self.sched.activity(0).iter().any(|j| j.version_id.as_deref() == Some(vid) && !j.state.finished()) {
                if std::time::Instant::now() > end {
                    return Err(CoreError::new("This version is still being prepared. Delete it when it has finished.").status(409));
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        self.lib.delete_version(vid);
        Ok(json!({ "ok": true }))
    }

    pub fn version_remake(&self, vid: &str, retry: bool) -> CoreResult<Value> {
        let (oid, v) = self.lib.find_version(vid).ok_or_else(gone_version)?;
        let nv = if retry && v["status"] == "error" {
            self.lib.patch_version(vid, &json!({ "status": "queued", "error": null }))?
        } else {
            self.lib.remake_version(vid)?
        };
        let job = self.version_job(&oid, &nv);
        Ok(json!({ "versionId": nv["id"], "jobId": job }))
    }

    // ---------- screen ----------

    /// The boot splash, made once (by the media engine) and cached as splash-v<N>.mp4. Returns (file, size, id).
    fn splash(&self, tctx: &super::scheduler::TaskCtx) -> CoreResult<(PathBuf, u64, String)> {
        let f = self.lib.splash_file();
        if !f.exists() {
            std::fs::create_dir_all(self.lib.base())?;
            let tmp = PathBuf::from(format!("{}.tmp.mp4", f.display()));
            self.media.splash(tctx, &tmp)?;
            std::fs::rename(&tmp, &f)?;
        }
        let buf = std::fs::read(&f)?;
        let mut h = sha1_smol::Sha1::new();
        h.update(&buf);
        let id = format!("v{}-{}", self.media.splash_version(), &h.digest().to_string()[..12]);
        Ok((f, buf.len() as u64, id))
    }

    /// "Put this version on the screen" (1.2 `send.sendVersion`), run inside a device task.
    fn send_version(&self, tctx: &super::scheduler::TaskCtx, vid: &str) -> CoreResult<Value> {
        let (oid, v) = self.lib.find_version(vid).ok_or_else(|| CoreError::new("This version is not ready yet."))?;
        let file = self.lib.device_path(vid).filter(|_| v["status"] == "ready").ok_or_else(|| CoreError::new("This version is not ready yet."))?;
        let o = self.lib.read_original(&oid);
        let size = std::fs::metadata(&file)?.len();
        let budget = budgets(&self.settings.load(), self.lib.splash_size()).loop_bytes;
        if size as f64 > budget {
            let mb = |b: f64| format!("{:.2} MB", b / 1e6);
            return Err(CoreError::new(format!(
                "Too big for the screen: {}, but there is room for {}. Make a shorter version or choose \"Best that fits\".",
                mb(size as f64),
                mb(budget)
            )));
        }
        let mut items: Vec<(PathBuf, Slot)> = vec![];
        let cur = self.settings.load().slot("startup").cloned();
        let sp = match self.splash(tctx) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("Splash could not be made: {}", e.message);
                None
            }
        };
        if let Some((f, _, id)) = &sp {
            let already = cur.as_ref().is_some_and(|c| c["splash"].as_str() == Some(id) && !truthy(c.get("incomplete")));
            if !already {
                items.push((f.clone(), Slot::Startup));
            }
        }
        items.push((file, Slot::Standby));
        let sizes: Vec<u64> = items.iter().map(|(f, _)| std::fs::metadata(f).map(|m| m.len()).unwrap_or(0)).collect();
        let bytes_total: u64 = sizes.iter().sum();
        let name = o.as_ref().and_then(|o| o["name"].as_str()).unwrap_or("(deleted clip)").to_string();
        let label = if truthy(v.get("label")) { v["label"].as_str().unwrap_or("").to_string() } else { format!("v{}", js::fmt(v["n"].as_f64().unwrap_or(0.0))) };
        let record = json!({ "name": name, "size": size, "originalId": oid, "versionId": vid, "label": label });
        let started = std::time::Instant::now();
        let n = items.len();
        let mut loop_started = false;
        let intro_first = items[0].1 == Slot::Startup;
        let mut sent = 0u64;
        let r = self.device.send_files(&items, true, &mut |frac, msg, a, b| {
            if frac > (n as f64 - 1.0) / (n as f64 + 0.1) {
                loop_started = true;
            }
            // frac runs over the uploads (n parts) plus 0.1 for the final restart; (a, b) are the current file's bytes.
            let idx = ((frac * (n as f64 + 0.1)).floor() as usize).min(n);
            if b > 0 {
                sent = sizes[..idx.min(n - 1)].iter().sum::<u64>() + a;
            } else if idx >= n {
                sent = bytes_total;
            }
            // Progress by bytes (the intro splash is small), the restart is the last 3 %.
            let p = if idx >= n { 0.97 + 0.03 * (frac * (n as f64 + 0.1) - n as f64).clamp(0.0, 1.0) } else { 0.97 * sent as f64 / bytes_total.max(1) as f64 };
            tctx.progress_bytes(p, msg, sent.min(bytes_total), bytes_total);
        });
        let record_splash = |s: &Option<(PathBuf, u64, String)>| {
            if let Some((_, size, id)) = s {
                let _ = self.settings.record_slot("startup", Some(json!({ "name": "(boot splash)", "size": size, "splash": id })));
            }
        };
        let r = match r {
            Ok(r) => r,
            Err(e) => {
                if loop_started {
                    if intro_first {
                        record_splash(&sp);
                    }
                    let mut rec = record.clone();
                    rec["incomplete"] = json!(true);
                    let _ = self.settings.record_slot("standby", Some(rec));
                }
                return Err(e);
            }
        };
        if intro_first {
            record_splash(&sp);
        }
        self.settings.record_slot("standby", Some(record))?;
        self.after_base_sent(vid); // 3.0: this version is the base of its theme and the live layer follows
        let secs = started.elapsed().as_secs_f64();
        if secs > 20.0 && r["transport"] != "fake" {
            let rate = (bytes_total as f64 / secs).round();
            self.settings.update(|s| s["uploadRate"] = num(rate))?;
        }
        // Measured capacity from the final check (free blocks + what our two clips now occupy).
        if let Some(info) = r.get("finalInfo").filter(|i| i.is_object()) {
            self.save_capacity(info);
        }
        self.lib.bump();
        let mut out = r.as_object().cloned().unwrap_or_default();
        out.insert("versionId".into(), json!(vid));
        out.insert("name".into(), json!(name));
        out.insert("label".into(), json!(label));
        Ok(Value::Object(out))
    }

    fn save_capacity(&self, info: &Value) -> Option<f64> {
        let cap = capacity_from(info, &self.settings.load())?;
        let _ = self.settings.update(|s| s["capacity"] = json!({ "bytes": num(cap), "at": js::iso_now() }));
        Some(cap)
    }

    fn device_job(&self, kind: &str, label: &str, spec_mod: impl FnOnce(TaskSpec) -> TaskSpec, f: impl Fn(&App, &super::scheduler::TaskCtx) -> CoreResult<Value> + Send + Sync + 'static) -> String {
        self.device_handle(kind, label, spec_mod, f).id
    }

    fn device_handle(&self, kind: &str, label: &str, spec_mod: impl FnOnce(TaskSpec) -> TaskSpec, f: impl Fn(&App, &super::scheduler::TaskCtx) -> CoreResult<Value> + Send + Sync + 'static) -> super::scheduler::JobHandle {
        let spec = spec_mod(TaskSpec::new(kind, label, Resources::one(Res::Device)).priority(Priority::User).visible());
        let app = self.arc();
        self.sched.submit(spec, move |ctx| f(&app, ctx))
    }

    pub fn version_send(&self, vid: &str) -> CoreResult<Value> {
        Ok(json!({ "jobId": self.send_handle(vid)?.id }))
    }

    /// The send job of a version (the same one "Push to screen" runs).
    pub(crate) fn send_handle(&self, vid: &str) -> CoreResult<super::scheduler::JobHandle> {
        let (oid, v) = self.lib.find_version(vid).ok_or_else(gone_version)?;
        let o = self.lib.read_original(&oid);
        let label = format!(
            "{} · {}",
            o.as_ref().and_then(|o| o["name"].as_str()).unwrap_or("clip"),
            if truthy(v.get("label")) { v["label"].as_str().unwrap_or("").to_string() } else { format!("v{}", js::fmt(v["n"].as_f64().unwrap_or(0.0))) }
        );
        let vid2 = vid.to_string();
        Ok(self.device_handle("send", &label, |s| s.original(&oid).version(vid), move |app, ctx| app.send_version(ctx, &vid2)))
    }

    pub fn device_state(&self) -> CoreResult<Value> {
        if self.sched.device_active() {
            return Err(updating());
        }
        let app = self.arc();
        self.run_task(TaskSpec::new("state", "Screen state", Resources::one(Res::Device)), move |_| Ok(app.device.state()))
    }

    /// One short check (opens the screen briefly); restarts once on "no reply"; measures the capacity.
    pub fn device_check(&self) -> CoreResult<Value> {
        if self.sched.device_active() {
            return Err(updating());
        }
        let app = self.arc();
        let mut info = self.run_task(TaskSpec::new("check", "Checking the screen", Resources::one(Res::Device)), move |ctx| {
            app.device.check(&mut |p, m| ctx.progress(p, m))
        })?;
        let cap = self.save_capacity(&info);
        info["_capacity"] = cap.map(num).unwrap_or(Value::Null);
        Ok(info)
    }

    pub fn device_restart(&self) -> Value {
        let id = self.device_job("setup", "Restarting the screen", |s| s, |app, ctx| app.device.restart(&mut |p, m| ctx.progress(p, m)));
        json!({ "jobId": id })
    }

    /// "setup" | "restore" | "reconnect"
    pub fn device_script(&self, which: &str) -> CoreResult<Value> {
        let label = match which {
            "setup" => "Setting up the screen",
            "reconnect" => "Reconnecting the screen",
            "restore" => "Restoring the original driver",
            _ => return Err(CoreError::new("Unknown screen operation")),
        };
        let w = which.to_string();
        let id = self.device_job("setup", label, |s| s, move |app, ctx| {
            let mut p = |f: f64, m: &str| ctx.progress(f, m);
            match w.as_str() {
                "setup" => app.device.setup_winusb(&mut p),
                "reconnect" => app.device.reconnect(&mut p),
                _ => app.device.restore_serial(&mut p),
            }
        });
        Ok(json!({ "jobId": id }))
    }

    /// App exit: cancel every queued and running task (kills their ffmpeg children; a running device upload cannot be cut off).
    pub fn shutdown(&self) {
        self.sched.cancel_where(|_| true);
        // Give cancelled tasks a moment to stop (their ffmpeg children are already killed) so media clean-up after
        // this does not race them. Bounded: a running screen upload cannot be cancelled and is never waited for
        // longer than this.
        self.sched.wait_idle(Duration::from_secs(1), true);
    }

    /// True while something is being written to the screen (close guard).
    pub fn uploading(&self) -> bool {
        self.sched.running_kind(&["send", "setup"])
    }

    // ---------- gallery folder ----------

    pub fn gallery_set_dir_cmd(&self, dir: &str) -> CoreResult<Value> {
        self.set_gallery_dir(dir)?;
        self.events.app_changed();
        Ok(self.app_state())
    }

    pub fn gallery_open(&self) -> CoreResult<Value> {
        let dir = self.gallery_dir();
        std::fs::create_dir_all(&dir)?;
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("explorer.exe").arg(&dir).spawn();
        }
        Ok(json!({ "dir": dir.to_string_lossy() }))
    }

    // ---------- sources ----------

    pub fn source_ctx(&self, id: &str) -> SourceCtx {
        let s = self.settings.load();
        SourceCtx { options: s.sources()["options"].get(id).cloned().unwrap_or(json!({})), tmp_dir: self.lib.base().join("downloads") }
    }

    /// An enabled provider (None if disabled or unknown).
    pub fn provider(&self, id: &str) -> Option<Arc<dyn SourceProvider>> {
        if self.settings.load().sources()["enabled"][id] != true {
            return None;
        }
        self.sources.providers().into_iter().find(|p| p.id() == id)
    }

    pub fn sources_list(&self) -> Value {
        let s = self.settings.load();
        let c = s.sources();
        let rows: Vec<Value> = self
            .sources
            .providers()
            .iter()
            .map(|p| {
                let id = p.id().to_string();
                let on = c["enabled"][&id] == true;
                // A disabled plug-in is never read (it decides which addresses the app contacts).
                let det = if p.kind() == "plugin" && !on {
                    super::api::Detect { available: true, status: "Not loaded (disabled)".into() }
                } else {
                    p.detect(&self.source_ctx(&id))
                };
                let mut row = json!({
                    "id": id, "name": p.name(), "description": p.description(), "kind": p.kind(), "enabled": c["enabled"][&id] == true,
                    "options": p.options(), "values": c["options"].get(&id).cloned().unwrap_or(json!({})),
                    "available": det.available, "status": det.status,
                });
                if let Some(extra) = p.extra().as_object() {
                    for (k, v) in extra {
                        row[k] = v.clone();
                    }
                }
                row
            })
            .collect();
        json!({ "sources": rows, "ticked": c["ticked"], "desktopApp": self.sources.desktop_capture(), "pluginsDir": self.sources.plugins_dir().to_string_lossy() })
    }

    pub fn source_set(&self, id: &str, enabled: Option<bool>, options: Option<Value>) -> CoreResult<Value> {
        if !self.sources.providers().iter().any(|p| p.id() == id) {
            return Err(CoreError::new("Unknown source"));
        }
        let s = self.settings.update(|s| {
            let src = &mut s["sources"];
            if let Some(e) = enabled {
                src["enabled"][id] = json!(e);
            }
            if let Some(o) = options {
                src["options"][id] = o;
            }
            let on = src["enabled"][id] == true;
            let mut ticked: Vec<Value> = src["ticked"].as_array().cloned().unwrap_or_default();
            if !on {
                ticked.retain(|t| t != id);
            } else if enabled == Some(true) && !ticked.iter().any(|t| t == id) {
                ticked.push(json!(id));
            }
            src["ticked"] = json!(ticked);
        })?;
        Ok(s.sources().clone())
    }

    pub fn browse_set(&self, ticked: &[String]) -> CoreResult<Value> {
        let s = self.settings.update(|s| {
            let en = s["sources"]["enabled"].clone();
            let t: Vec<&String> = ticked.iter().filter(|id| *id == "mine" || en[id.as_str()] == true).collect();
            s["sources"]["ticked"] = json!(t);
        })?;
        Ok(s.sources().clone())
    }

    pub fn source_items(&self, sid: &str, page: u32, refresh: bool) -> CoreResult<Value> {
        let prov = self.provider(sid).ok_or_else(|| CoreError::new("This source is turned off. Turn it on in Settings > Sources.").status(403))?;
        let page = page.max(1);
        let r = prov.list(&self.source_ctx(sid), page, refresh).map_err(|e| e.status(502))?;
        let items: Vec<Value> = r
            .items
            .iter()
            .take(200)
            .map(|it| {
                let thumb = match it.thumb.as_deref() {
                    Some(t) if t.starts_with("thumb:") => json!(self.lib.url(&format!("/sources/{sid}/thumb?item={}", encode_uri_component(&t[6..])))),
                    Some(t) if t.starts_with("https://") => json!(t),
                    _ => Value::Null,
                };
                let title = if it.title.is_empty() { it.id.clone() } else { it.title.clone() };
                json!({
                    "id": it.id, "title": title.chars().take(160).collect::<String>(), "type": it.kind,
                    "action": if it.action == "add" || it.action == "record" { it.action.as_str() } else { "none" },
                    "note": it.note, "badge": it.badge, "thumb": thumb,
                })
            })
            .collect();
        Ok(json!({ "page": page, "total": r.total, "hasMore": r.has_more, "items": items }))
    }

    /// Media server: a source's local thumbnail.
    pub fn source_thumb(&self, sid: &str, item: &str) -> Option<PathBuf> {
        let p = self.provider(sid)?;
        p.thumb_path(&self.source_ctx(sid), item).filter(|f| f.exists())
    }

    pub fn source_add(&self, sid: &str, item: &str, body: &Value) -> CoreResult<Value> {
        let prov = self.provider(sid).ok_or_else(|| CoreError::new("This source is turned off. Turn it on in Settings > Sources.").status(403))?;
        let recording = body.get("record") == Some(&json!(true));
        if recording && !self.sources.desktop_capture() {
            return Err(CoreError::new("Recording is not available on this PC."));
        }
        let opts = AddOpts { seconds: body.get("seconds").and_then(Value::as_f64), record: recording };
        let title: String = body.get("title").and_then(Value::as_str).unwrap_or(item).chars().take(60).collect();
        let mut spec = TaskSpec::new("source", format!("{} \"{title}\"", if recording { "Recording" } else { "Adding" }), prov.resources(item, &opts))
            .priority(Priority::User)
            .visible();
        spec.source_id = Some(sid.into());
        spec.item_id = Some(item.into());
        let (app, sid2, item2) = (self.arc(), sid.to_string(), item.to_string());
        let h = self.sched.submit(spec, move |ctx| {
            let sctx = app.source_ctx(&sid2);
            let r = prov.add(&sctx, ctx, &item2, &opts)?;
            app.import_added(ctx, r)
        });
        Ok(json!({ "jobId": h.id }))
    }

    fn import_added(&self, ctx: &super::scheduler::TaskCtx, r: super::api::AddedFile) -> CoreResult<Value> {
        // A temporary download is always deleted: after the import, and also when cancelled or failed.
        let res = ctx.check().and_then(|_| {
            ctx.progress(0.9, "Adding to the gallery");
            self.add_file(&r.file, Some(&r.name))
        });
        if r.temp {
            let _ = std::fs::remove_file(&r.file);
        }
        let oid = res?;
        self.prefind_loops(&oid);
        Ok(json!({ "originalId": oid, "name": r.name }))
    }

    pub fn link_add(&self, url: &str) -> CoreResult<Value> {
        let prov = self.provider("link").ok_or_else(|| CoreError::new("Turn on \"Add from link\" in Settings > Sources first.").status(403))?;
        let spec = TaskSpec::new("source", "Adding from link", Resources::one(Res::Net)).priority(Priority::User).visible();
        let (app, u) = (self.arc(), url.trim().to_string());
        let h = self.sched.submit(spec, move |ctx| {
            ctx.progress(0.0, "Downloading");
            let r = prov.download(&app.source_ctx("link"), ctx, &u)?;
            app.import_added(ctx, r)
        });
        Ok(json!({ "jobId": h.id }))
    }

    // ---------- startup ----------

    /// What 1.2's server did at start: mark interrupted versions, scan the gallery (+ assets), watch it.
    pub fn startup(&self) {
        self.lib.reset_interrupted();
        self.gallery_init(true);
        self.refresh_live_theme();
    }

    /// Resolve a media-server path (after the token).
    pub fn resolve_media(&self, path: &str, query: &str) -> Option<(PathBuf, bool)> {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match parts.as_slice() {
            ["media", "o", oid, file] => {
                // A loop card still being rendered: hold the request (up to 30 s) instead of a 404 the <video> keeps.
                if file.starts_with("loop-") {
                    let end = std::time::Instant::now() + Duration::from_secs(30);
                    // (cards are written in place, so wait for the whole job, not just the file)
                    while self.cards_pending.lock().unwrap().contains_key(*oid)
                        && std::time::Instant::now() < end
                    {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                }
                self.lib.media_path_o(oid, file).map(|p| (p, false))
            }
            ["media", "v", vid, file] => self.lib.media_path_v(vid, file).map(|p| (p, false)),
            ["sources", sid, "thumb"] => {
                let item = super::media_server::query_param(query, "item")?;
                self.source_thumb(sid, &item).map(|p| (p, true))
            }
            _ => None,
        }
    }
}

/// `encodeURIComponent`
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl App {
    /// The live engine's config for this app: companion.json (fps, ...) with the app's live settings on top. The background
    /// and the layout come from the current theme (`refresh_live_theme`), and music is a widget, so the old per-live
    /// background picker and music switch no longer apply.
    pub fn live_config(&self) -> crate::live::Config {
        let l = self.settings.load().live();
        let mut c = crate::live::Config::load();
        c.show_seconds = l.show_seconds;
        c.background = None;
        c.music.enabled = true;
        c
    }
}
