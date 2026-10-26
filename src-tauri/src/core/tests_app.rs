//! End-to-end tests of the core through the same `App` methods the Tauri commands call, with the fake media
//! engine and the fake screen (no ffmpeg, no USB).
use super::api::{Backends, NoSources};
use super::app::{App, Events};
use super::device_service::DeviceService;
use super::fake_media::FakeMedia;
use super::scheduler::Job;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct Rec {
    jobs: Mutex<Vec<(String, String)>>,
    lib: Mutex<u32>,
    busy: Mutex<Vec<bool>>,
}
impl Events for Rec {
    fn job(&self, j: &Job) {
        self.jobs.lock().unwrap().push((j.kind.clone(), j.status.to_string()));
    }
    fn library(&self, _: u64) {
        *self.lib.lock().unwrap() += 1;
    }
    fn device_busy(&self, b: bool) {
        self.busy.lock().unwrap().push(b);
    }
}

fn setup() -> (tempfile::TempDir, Arc<App>, Arc<Rec>, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let gallery = d.path().join("Gallery");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&gallery).unwrap();
    std::fs::write(home.join("settings.json"), json!({ "galleryDir": gallery.to_string_lossy() }).to_string()).unwrap();
    let rec = Arc::new(Rec::default());
    let app = App::with_device(
        home,
        Backends { media: Arc::new(FakeMedia { delay: Duration::from_millis(20) }), sources: Arc::new(NoSources) },
        rec.clone(),
        "2.0.0-test",
        DeviceService::fake("winusb", 0.01, false),
    );
    (d, app, rec, gallery)
}

fn wait_job(app: &App, id: &str) -> Value {
    let end = Instant::now() + Duration::from_secs(20);
    loop {
        let j = app.job_get(id).unwrap();
        if ["done", "failed", "cancelled"].contains(&j["state"].as_str().unwrap()) {
            return j;
        }
        assert!(Instant::now() < end, "job {id} did not finish: {j}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until(mut f: impl FnMut() -> bool) {
    let end = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < end, "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Write a file with an old modification time (the gallery skips files still being written).
fn put(path: &std::path::Path, data: &[u8]) {
    std::fs::write(path, data).unwrap();
    let f = std::fs::File::options().write(true).open(path).unwrap();
    f.set_modified(std::time::SystemTime::now() - Duration::from_secs(60)).unwrap();
}

#[test]
fn whole_flow_import_tune_save_send() {
    let (_d, app, rec, gallery) = setup();
    put(&gallery.join("My clip.mp4"), b"fake video data");
    app.startup();
    let list = app.originals_list();
    assert_eq!(list["originals"].as_array().unwrap().len(), 1);
    let oid = list["originals"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(list["originals"][0]["name"], "My clip");
    wait_until(|| app.original_get(&oid).unwrap()["assets"] == "ready");
    app.set_media_base("http://127.0.0.1:1/tok");
    let det = app.original_get(&oid).unwrap();
    assert_eq!(det["frameUrl"], json!(format!("http://127.0.0.1:1/tok/media/o/{oid}/frame.jpg")));
    assert_eq!(det["canTune"], json!(true));

    // size meter
    let est = app.estimate(&oid, &json!({ "start": 1, "length": 5, "quality": "auto" })).unwrap();
    assert_eq!(est["fits"], json!(true));
    assert_eq!(est["settings"]["length"], json!(5));
    assert!(est["quality"].as_f64().unwrap() > 50.0);

    // loop search (visible job), stored with the original, card URLs absolute for the UI
    let j = app.loop_search(&oid, &json!({ "minLen": null, "maxLen": null })).unwrap();
    let j = wait_job(&app, j["jobId"].as_str().unwrap());
    assert_eq!(j["status"], "done", "{j}");
    let c0 = &j["result"]["candidates"][0];
    assert!(c0["preview"].as_str().unwrap().starts_with("http://127.0.0.1:1/tok/media/o/"));
    assert!(j["result"]["suggestedMax"].as_f64().unwrap() >= 1.0);
    assert_eq!(app.original_get(&oid).unwrap()["loops"]["candidates"].as_array().unwrap().len(), 3);
    assert_eq!(c0["blend"], json!(0), "ratio 0.9: no blend");
    // cards come from their own job; the media server waits for them
    let (p, _) = app.resolve_media(&format!("/media/o/{oid}/loop-2.mp4"), "").expect("card served once made");
    assert!(p.exists());

    // save a version
    let r = app.version_create(&oid, &json!({ "start": 1, "length": 5, "quality": "auto" })).unwrap();
    let vid = r["versionId"].as_str().unwrap().to_string();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["status"], "done", "{j}");
    let det = app.original_get(&oid).unwrap();
    assert_eq!(det["versions"][0]["status"], "ready");
    assert_eq!(det["versions"][0]["result"]["fps"], json!(18));
    let e = app.version_create(&oid, &json!({ "start": 1, "length": 5, "quality": "auto" })).unwrap_err();
    assert!(e.code_is("duplicate"));

    // put it on the (fake) screen: splash first, then the loop, then restart
    let r = app.version_send(&vid).unwrap();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["status"], "done", "{j}");
    assert_eq!(j["result"]["name"], "My clip");
    assert_eq!(j["bytesSent"], j["bytesTotal"], "progress counts real bytes (splash + loop)");
    let s = app.settings.load();
    assert_eq!(s.slot("standby").unwrap()["versionId"], json!(vid));
    assert!(s.slot("startup").unwrap()["splash"].as_str().unwrap().starts_with("v1-"));
    let st = app.app_state();
    assert_eq!(st["onScreen"]["versionId"], json!(vid));
    assert_eq!(st["onScreen"]["label"], "v1");
    assert_eq!(st["fake"], "winusb");
    assert!(st["budgetMeasured"].as_bool().unwrap(), "capacity measured from the final check");
    assert!(app.original_get(&oid).unwrap()["versions"][0]["onScreen"].as_bool().unwrap());

    // a second send does not upload the splash again
    let r = app.version_send(&vid).unwrap();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["status"], "done");

    // screen check measures the capacity
    let info = app.device_check().unwrap();
    assert!(info["_capacity"].as_f64().unwrap() > 1e6);

    // activity and events
    let act = app.activity();
    let kinds: Vec<&str> = act["jobs"].as_array().unwrap().iter().map(|j| j["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"version") && kinds.contains(&"send") && kinds.contains(&"loopsearch"));
    assert!(!kinds.contains(&"estimate") && !kinds.contains(&"assets"), "internal jobs hidden");
    assert!(rec.jobs.lock().unwrap().iter().any(|(k, s)| k == "send" && s == "done"));
    wait_until(|| *rec.lib.lock().unwrap() > 0);
    wait_until(|| rec.busy.lock().unwrap().last() == Some(&false));

    // rename moves the gallery file
    app.original_patch(&oid, &json!({ "name": "Renamed" })).unwrap();
    assert!(gallery.join("Renamed.mp4").exists());
    // draft
    let d = app.original_patch(&oid, &json!({ "draft": { "start": 2, "length": 3 } })).unwrap();
    assert_eq!(d["draft"]["length"], json!(3));
    let d = app.original_patch(&oid, &json!({ "draft": null })).unwrap();
    assert!(d["draft"].is_null());
}

#[test]
fn missing_files_and_renames() {
    let (_d, app, _rec, gallery) = setup();
    put(&gallery.join("a.mp4"), b"aaaa");
    put(&gallery.join("b.png"), b"bbbbbb");
    app.startup();
    let list = app.originals_list();
    assert_eq!(list["originals"].as_array().unwrap().len(), 2);
    let find = |name: &str| {
        app.originals_list()["originals"].as_array().unwrap().iter().find(|o| o["name"] == name).map(|o| o["id"].as_str().unwrap().to_string())
    };
    let a = find("a").unwrap();
    // renamed in Explorer: same size and time → same id, new name
    std::fs::rename(gallery.join("a.mp4"), gallery.join("a2.mp4")).unwrap();
    app.gallery_sync();
    assert_eq!(find("a2"), Some(a.clone()));
    // removed: marked missing, kept
    std::fs::remove_file(gallery.join("a2.mp4")).unwrap();
    app.gallery_sync();
    let o = app.original_get(&a).unwrap();
    assert!(o["missing"]["since"].is_string());
    assert!(app.long_missing(super::scheduler::now_ms()).is_empty());
    assert_eq!(app.long_missing(super::scheduler::now_ms() + 31 * 86_400_000).len(), 1);
    assert_eq!(app.missing_delete(&[a.clone()])["deleted"], json!(1));
    assert!(app.original_get(&a).is_err());
    // an unreadable (empty) file is skipped and not retried until it changes
    put(&gallery.join("empty.mp4"), b"");
    app.gallery_sync();
    assert!(find("empty").is_none());
}

#[test]
fn watcher_picks_up_new_files() {
    let (_d, app, _rec, gallery) = setup();
    app.startup();
    assert_eq!(app.originals_list()["originals"].as_array().unwrap().len(), 0);
    put(&gallery.join("new.mp4"), b"new file");
    wait_until(|| app.originals_list()["originals"].as_array().unwrap().len() == 1);
}

#[test]
fn deleting_a_converting_version_cancels_it() {
    let (_d, app, _rec, gallery) = setup();
    put(&gallery.join("c.mp4"), b"cccc");
    app.startup();
    let oid = app.originals_list()["originals"][0]["id"].as_str().unwrap().to_string();
    let r = app.version_create(&oid, &json!({ "start": 0, "length": 4 })).unwrap();
    let vid = r["versionId"].as_str().unwrap().to_string();
    let jid = r["jobId"].as_str().unwrap().to_string();
    wait_until(|| app.job_get(&jid).unwrap()["state"] == "running");
    app.version_delete(&vid).unwrap();
    let j = wait_job(&app, &jid);
    assert_eq!(j["state"], "cancelled");
    assert!(app.lib.find_version(&vid).is_none());
}

#[test]
fn estimate_is_paused_while_the_screen_is_busy_in_blocking_mode() {
    let (_d, app, _rec, gallery) = setup();
    put(&gallery.join("e.mp4"), b"eeee");
    app.startup();
    app.sched.set_policy(super::scheduler::Policy::blocking());
    let oid = app.originals_list()["originals"][0]["id"].as_str().unwrap().to_string();
    let r = app.device_restart();
    let jid = r["jobId"].as_str().unwrap().to_string();
    let e = app.estimate(&oid, &json!({})).unwrap_err();
    assert!(e.code_is("busy") && e.status == 409);
    assert!(app.device_check().unwrap_err().code_is("updating"));
    wait_job(&app, &jid);
    assert!(app.estimate(&oid, &json!({})).is_ok());
}

#[test]
fn cancelled_cards_job_clears_pending() {
    use super::scheduler::{Res, Resources, TaskSpec};
    let (_d, app, _rec, gallery) = setup();
    put(&gallery.join("k.mp4"), b"kkkk");
    app.startup();
    let oid = app.originals_list()["originals"][0]["id"].as_str().unwrap().to_string();
    // Hold every cpu unit so the cards job stays queued.
    let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let r2 = release.clone();
    let blocker = app.sched.submit(TaskSpec::new("t", "blocker", Resources::none().with(Res::Cpu, 1000)), move |ctx| {
        while !r2.load(std::sync::atomic::Ordering::SeqCst) {
            ctx.sleep(Duration::from_millis(5))?;
        }
        Ok(Value::Null)
    });
    let src = gallery.join("k.mp4");
    app.cards_job(&oid, &src, 4.0, Some(25.0), json!([{ "start": 0, "length": 1 }]));
    assert!(app.cards_pending.lock().unwrap().contains_key(&oid));
    // A newer search for the same clip replaces the queued cards job; the newer one owns the pending mark.
    app.cards_job(&oid, &src, 4.0, Some(25.0), json!([{ "start": 0, "length": 1 }]));
    app.sched.cancel_original(&oid);
    wait_until(|| !app.cards_pending.lock().unwrap().contains_key(&oid));
    let t = Instant::now();
    assert!(app.resolve_media(&format!("/media/o/{oid}/loop-0.mp4"), "").is_none());
    assert!(t.elapsed() < Duration::from_secs(1), "no 30 s hold once the job is gone");
    release.store(true, std::sync::atomic::Ordering::SeqCst);
    blocker.wait().unwrap();
}

#[test]
fn settings_update_keeps_corners() {
    let (_d, app, _rec, _g) = setup();
    app.settings.update(|s| s["mask"]["corners"] = json!({ "on": true, "r": 50 })).unwrap();
    let st = app.settings_update(&json!({ "mask": { "mode": "band", "band": { "edge": "right", "px": 40 }, "corners": { "on": false, "r": 1 } }, "maxSeconds": 20 })).unwrap();
    assert_eq!(st["mask"]["corners"], json!({ "on": true, "r": 50 }));
    assert_eq!(st["notchKey"], "band:right:48");
    assert_eq!(st["maxSeconds"], json!(20));
}

// ---------- 3.0 themes ----------

fn setup_slow(delay_ms: u64) -> (tempfile::TempDir, Arc<App>, Arc<Rec>, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    let gallery = d.path().join("Gallery");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&gallery).unwrap();
    std::fs::write(home.join("settings.json"), json!({ "galleryDir": gallery.to_string_lossy() }).to_string()).unwrap();
    let rec = Arc::new(Rec::default());
    let app = App::with_device(
        home,
        Backends { media: Arc::new(FakeMedia { delay: Duration::from_millis(delay_ms) }), sources: Arc::new(NoSources) },
        rec.clone(),
        "3.0.0-test",
        DeviceService::fake("winusb", 0.01, false),
    );
    (d, app, rec, gallery)
}

fn one_original(app: &App, gallery: &std::path::Path, name: &str) -> String {
    put(&gallery.join(name), b"fake video data");
    app.startup();
    wait_until(|| app.originals_list()["originals"].as_array().unwrap().len() == 1);
    let oid = app.originals_list()["originals"][0]["id"].as_str().unwrap().to_string();
    wait_until(|| app.original_get(&oid).unwrap()["assets"] == "ready");
    oid
}

#[test]
fn theme_get_gives_defaults_and_set_validates() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "t.mp4");
    let v = app.theme_get(&oid).unwrap();
    assert_eq!(v["theme"]["widgets"], json!([]));
    assert_eq!(v["theme"]["handover"]["end"], "loopStart");
    assert_eq!(v["theme"]["handover"]["fadeIn"], json!(0.8));
    assert_eq!(v["current"], json!(false));
    assert_eq!(v["baseStale"], json!(false));
    assert!(v["theme"]["baseVersionId"].is_null());
    assert!(!app.theme_dir(&oid).join("theme.json").exists(), "nothing is written until the user saves");

    // saved, clamped, and the base version cannot be set from the page
    let t = json!({ "baseVersionId": "zzzzzzzz", "widgets": [
        { "id": "a", "type": "clock", "x": 0.9, "y": 0.2, "w": 0.5, "h": 0.3, "props": { "seconds": true } },
        { "id": "b", "type": "image", "x": 0, "y": 0, "w": 0.2, "h": 0.2, "onBase": true, "props": { "file": "../../secret.png" } },
        { "id": "c", "type": "stats", "x": 0.6, "y": 0.1, "w": 0.3, "h": 0.3, "onBase": true } ],
        "handover": { "fadeIn": 9 } });
    let v = app.theme_set(&oid, &t).unwrap();
    assert!(v["theme"]["baseVersionId"].is_null());
    assert_eq!(v["theme"]["handover"]["fadeIn"], json!(3.0));
    let w = &v["theme"]["widgets"];
    assert!(w[0]["x"].as_f64().unwrap() + w[0]["w"].as_f64().unwrap() <= 1.0001, "box pulled inside the canvas");
    assert_eq!(w[1]["props"]["file"], "", "a picture path leaving the theme folder is dropped");
    assert_eq!(w[2]["onBase"], json!(false), "live-data items are never baked in");
    assert_eq!(v["hasBaseItems"], json!(true), "the picture is still baked in");
    assert!(app.theme_dir(&oid).join("theme.json").exists());
    assert_eq!(app.theme_get(&oid).unwrap()["theme"], v["theme"]);

    // invalid input is refused
    assert!(app.theme_set(&oid, &json!({ "widgets": [{ "type": "rocket" }] })).is_err());
    assert!(app.theme_set(&oid, &json!("nope")).is_err());
    let many: Vec<Value> = (0..41).map(|_| json!({ "type": "text" })).collect();
    assert!(app.theme_set(&oid, &json!({ "widgets": many })).is_err());
    assert!(app.theme_set("nosuchid1", &json!({})).is_err());
    // the list tells which kinds each theme has
    let l = app.originals_list();
    assert_eq!(l["originals"][0]["theme"]["widgets"], json!(["clock", "image", "stats"]));
}

#[test]
fn theme_pictures_are_copied_into_the_theme_folder() {
    let (d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "t.mp4");
    let a = app.theme_asset_add_bytes(&oid, "My Logo.png", &super::fake_media::png(4, 4, [1, 2, 3])).unwrap();
    assert_eq!(a["file"], "assets/My Logo.png");
    let b = app.theme_asset_add_bytes(&oid, "My Logo.png", &super::fake_media::png(4, 4, [9, 9, 9])).unwrap();
    assert_eq!(b["file"], "assets/My Logo (2).png");
    assert!(app.theme_dir(&oid).join("assets/My Logo.png").is_file());
    assert!(app.theme_asset_add_bytes(&oid, "run.exe", b"MZ").is_err(), "only pictures");
    assert!(app.theme_asset_add_bytes(&oid, "empty.png", b"").is_err());
    let src = d.path().join("from disk.jpg");
    std::fs::write(&src, b"jpegbytes").unwrap();
    assert_eq!(app.theme_asset_add_path(&oid, &src.to_string_lossy()).unwrap()["file"], "assets/from disk.jpg");
    assert!(app.theme_asset_add_path(&oid, "relative.png").is_err());
}

#[test]
fn adding_from_browse_makes_independent_copies() {
    let (d, app, _rec, gallery) = setup();
    let tmp = d.path().join("dl");
    std::fs::create_dir_all(&tmp).unwrap();
    // What a source hands over twice for the same item: a downloaded file named with the source's key.
    let mut names = vec![];
    for _ in 0..3 {
        let f = tmp.join("dl.mp4");
        put(&f, b"same wallpaper bytes");
        let id = app.add_file(&f, Some("Ocean Echo [we-1635817562].mp4")).unwrap();
        names.push(app.lib.read_original(&id).unwrap()["name"].as_str().unwrap().to_string());
    }
    assert_eq!(names, ["Ocean Echo", "Ocean Echo (2)", "Ocean Echo (3)"]);
    assert!(gallery.join("Ocean Echo [we-1635817562].mp4").exists() && gallery.join("Ocean Echo [we-1635817562] (2).mp4").exists());
    assert_eq!(app.originals_list()["originals"].as_array().unwrap().len(), 3, "three independent themes");
}

#[test]
fn apply_stores_the_base_and_switches_the_theme() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Apply me.mp4");
    app.theme_set(&oid, &json!({ "widgets": [{ "id": "c", "type": "clock", "x": 0.1, "y": 0.1, "w": 0.4, "h": 0.3 }] })).unwrap();
    assert!(app.live_theme_oid().is_none());
    let r = app.theme_apply(&oid).unwrap();
    let jid = r["jobId"].as_str().unwrap().to_string();
    let mut seen = std::collections::BTreeSet::new();
    let end = Instant::now() + Duration::from_secs(20);
    let j = loop {
        let j = app.job_get(&jid).unwrap();
        assert!(Instant::now() < end, "apply job did not finish: {j}");
        seen.insert(j["message"].as_str().unwrap_or("").to_string());
        if ["done", "failed", "cancelled"].contains(&j["state"].as_str().unwrap()) {
            break j;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(j["state"], "done", "{j}");
    assert!(seen.iter().any(|m| m.starts_with("Step 1 of 2") || m.starts_with("Step 2 of 2")), "steps are reported: {seen:?}");
    let vid = j["result"]["versionId"].as_str().unwrap().to_string();
    assert_eq!(app.live_theme_oid().as_deref(), Some(oid.as_str()));
    assert_eq!(app.settings.load().slot("standby").unwrap()["versionId"], json!(vid));
    let v = app.theme_get(&oid).unwrap();
    assert_eq!(v["current"], json!(true));
    assert_eq!(v["baseVersionId"], json!(vid));
    assert_eq!(v["baseStale"], json!(false));
    assert!(app.lib.device_path(&vid).is_some());
    // applying again reuses the same base (no second version)
    let r = app.theme_apply(&oid).unwrap();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["versionId"], json!(vid));
    assert_eq!(app.lib.versions(&oid).len(), 1);
    // editing the live layer keeps the base; the list marks the current theme
    app.theme_set(&oid, &json!({ "widgets": [] })).unwrap();
    assert_eq!(app.theme_get(&oid).unwrap()["baseVersionId"], json!(vid));
    assert_eq!(app.originals_list()["currentTheme"], json!(oid));
    // a base made by a plain "Push to screen" counts too
    let other = app.lib.create_version(&oid, &json!({ "start": 0, "length": 2, "force": true })).unwrap();
    let ovid = other["id"].as_str().unwrap().to_string();
    app.version_handle(&oid, &other).wait().unwrap();
    app.send_handle(&ovid).unwrap().wait().unwrap();
    assert_eq!(app.theme_get(&oid).unwrap()["baseVersionId"], json!(ovid));
}

#[test]
fn stopping_an_apply_keeps_the_previous_theme() {
    let (_d, app, _rec, gallery) = setup_slow(400);
    let oid = one_original(&app, &gallery, "Stop me.mp4");
    let r = app.theme_apply(&oid).unwrap();
    let jid = r["jobId"].as_str().unwrap().to_string();
    wait_until(|| app.job_get(&jid).unwrap()["state"] == "running");
    std::thread::sleep(Duration::from_millis(100));
    assert!(app.sched.cancel(&jid));
    let j = wait_job(&app, &jid);
    assert_eq!(j["state"], "cancelled", "{j}");
    assert!(app.settings.load().slot("standby").is_none(), "nothing was stored on the screen");
    assert!(app.live_theme_oid().is_none());
    assert!(app.theme_get(&oid).unwrap()["theme"]["baseVersionId"].is_null());
    assert!(app.lib.versions(&oid).is_empty(), "no half-made base is left behind");
}

#[test]
fn deleting_the_current_theme_clears_it() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Gone.mp4");
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    assert_eq!(app.live_theme_oid().as_deref(), Some(oid.as_str()));
    app.original_delete(&oid).unwrap();
    assert!(app.live_theme_oid().is_none());
}

#[test]
fn changing_baked_items_marks_the_base_as_stale() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Baked.mp4");
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    assert_eq!(app.theme_get(&oid).unwrap()["baseStale"], json!(false));
    // a live-only item never touches the base
    let live = json!({ "widgets": [{ "id": "c", "type": "clock", "x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3 }] });
    assert_eq!(app.theme_set(&oid, &live).unwrap()["baseStale"], json!(false));
    // text that is also drawn into the stored base does
    let baked = json!({ "widgets": [{ "id": "t", "type": "text", "x": 0.1, "y": 0.1, "w": 0.3, "h": 0.2, "onBase": true, "props": { "text": "Hi" } }] });
    let v = app.theme_set(&oid, &baked).unwrap();
    assert_eq!(v["baseStale"], json!(true));
    assert_eq!(v["hasBaseItems"], json!(true));
    // taking it off again clears the notice
    assert_eq!(app.theme_set(&oid, &live).unwrap()["baseStale"], json!(false));
}

// ---------- theme format v1.1 ----------

fn two_originals(app: &App, gallery: &std::path::Path) -> (String, String) {
    put(&gallery.join("Alpha.mp4"), b"alpha video");
    put(&gallery.join("Beta.mp4"), b"beta video, a bit longer");
    app.startup();
    wait_until(|| app.originals_list()["originals"].as_array().unwrap().len() == 2);
    let find = |n: &str| app.originals_list()["originals"].as_array().unwrap().iter().find(|o| o["name"] == n).unwrap()["id"].as_str().unwrap().to_string();
    let (a, b) = (find("Alpha"), find("Beta"));
    for id in [&a, &b] {
        wait_until(|| app.original_get(id).unwrap()["assets"] == "ready");
    }
    (a, b)
}

#[test]
fn layers_groups_hidden_and_live_layer_are_kept_and_validated() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "t.mp4");
    let w = |id: &str, g: Option<&str>| json!({ "id": id, "type": "text", "x": 0.1, "y": 0.1, "w": 0.2, "h": 0.2, "group": g, "props": { "text": id, "name": format!("Layer {id}") } });
    let t = json!({
        "liveLayer": false,
        // g1's members are split by "b"; "e" names a group that does not exist; g3 has no members
        "widgets": [w("a", Some("g1")), w("b", None), w("c", Some("g1")), w("d", Some("g2")), w("e", Some("nope"))],
        "groups": [{ "id": "g1", "name": "  Title block  ", "hidden": true }, { "id": "g2", "name": "Two", "collapsed": true }, { "id": "g3", "name": "Empty" }]
    });
    let v = app.theme_set(&oid, &t).unwrap();
    let th = &v["theme"];
    assert_eq!(th["liveLayer"], json!(false), "the switch is saved");
    let order: Vec<&str> = th["widgets"].as_array().unwrap().iter().map(|w| w["id"].as_str().unwrap()).collect();
    assert_eq!(order, ["a", "c", "b", "d", "e"], "group members become contiguous, at the place of the first member");
    assert_eq!(th["widgets"][4]["group"], Value::Null, "an unknown group is dropped");
    let gids: Vec<&str> = th["groups"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap()).collect();
    assert_eq!(gids, ["g1", "g2"], "empty groups go");
    assert_eq!(th["groups"][0]["name"], "Title block");
    assert_eq!(th["groups"][0]["hidden"], json!(true));
    assert_eq!(th["groups"][1]["collapsed"], json!(true));
    assert_eq!(th["widgets"][0]["props"]["name"], "Layer a", "layer names survive");
    // new themes: the live layer is on
    assert_eq!(app.theme_get(&oid).unwrap()["theme"]["liveLayer"], json!(false));
    let v = app.theme_set(&oid, &json!({})).unwrap();
    assert_eq!(v["theme"]["liveLayer"], json!(true));
    // hidden items are not baked
    let baked = json!({ "widgets": [{ "id": "t", "type": "text", "onBase": true, "hidden": true }] });
    assert_eq!(app.theme_set(&oid, &baked).unwrap()["hasBaseItems"], json!(false));
}

/// Live can show one theme while the screen holds (Embedded) another one's picture.
#[test]
fn use_this_theme_switches_live_without_touching_embedded() {
    let (_d, app, _rec, gallery) = setup();
    let (a, b) = two_originals(&app, &gallery);
    let store = |oid: &str| {
        let r = app.theme_apply(oid).unwrap();
        let j = wait_job(&app, r["jobId"].as_str().unwrap());
        assert_eq!(j["state"], "done", "{j}");
        j["result"]["versionId"].as_str().unwrap().to_string()
    };
    let va = store(&a);
    assert_eq!(app.live_theme_oid().as_deref(), Some(a.as_str()));
    assert!(app.settings.load().live_theme().is_none(), "no explicit choice: live follows Embedded");
    // Beta has its own base (a ready version), not stored
    let v = app.lib.create_version(&b, &json!({ "start": 0, "length": 2, "force": true })).unwrap();
    let vb = v["id"].as_str().unwrap().to_string();
    app.version_handle(&b, &v).wait().unwrap();
    app.theme_set_base(&b, &vb).unwrap();
    // Use Beta: live switches, the slot is untouched
    let view = app.theme_use(&b).unwrap();
    assert_eq!(app.live_theme_oid().as_deref(), Some(b.as_str()));
    assert_eq!(app.embedded_theme_oid().as_deref(), Some(a.as_str()));
    assert_eq!(app.settings.load().slot("standby").unwrap()["versionId"], json!(va), "Embedded was not changed");
    assert_eq!((view["live"].clone(), view["embedded"].clone(), view["embeddedDiffers"].clone()), (json!(true), json!(false), json!(true)));
    assert_eq!(view["embeddedTheme"]["name"], json!(app.lib.read_original(&a).unwrap()["name"]));
    assert_eq!(view["baseUnstored"], json!(false), "the Embedded-theme banner is about Embedded's own theme");
    let l = app.originals_list();
    assert_eq!((l["currentTheme"].clone(), l["embeddedTheme"].clone()), (json!(b), json!(a)));
    let theme_flags = |id: &str| l["originals"].as_array().unwrap().iter().find(|o| o["id"] == json!(id)).unwrap()["theme"].clone();
    assert_eq!((theme_flags(&a)["live"].clone(), theme_flags(&a)["embedded"].clone()), (json!(false), json!(true)));
    assert_eq!((theme_flags(&b)["live"].clone(), theme_flags(&b)["embedded"].clone()), (json!(true), json!(false)));
    // the engine: Beta's base is not what the screen holds, so live must stream it and stop instantly; Alpha's is fine
    let tb = app.theme_live_of(&b, app.load_theme(&b));
    assert!(tb.not_embedded && !tb.nothing_to_stream(crate::core::fit::Fit::default()));
    assert!(!app.theme_live_of(&a, app.load_theme(&a)).not_embedded);
    // editing the live theme refreshes live; editing the Embedded one (not live) does not touch it
    app.theme_set(&b, &json!({ "widgets": [{ "id": "c", "type": "clock", "x": 0.1, "y": 0.1, "w": 0.4, "h": 0.3 }] })).unwrap();
    assert_eq!(app.live_theme_oid().as_deref(), Some(b.as_str()));
    // survives a restart: the choice is in settings.json
    assert_eq!(app.settings.load().live_theme().as_deref(), Some(b.as_str()));
    // storing Beta as Embedded: the slot changes, live stays on Beta and goes back to following Embedded
    let stored = store(&b);
    assert_eq!(stored, vb);
    assert_eq!(app.embedded_theme_oid().as_deref(), Some(b.as_str()));
    assert!(app.settings.load().live_theme().is_none());
    assert!(!app.theme_live_of(&b, app.load_theme(&b)).not_embedded);
    let view = app.theme_get(&b).unwrap();
    assert_eq!((view["embedded"].clone(), view["embeddedDiffers"].clone(), view["baseUnstored"].clone()), (json!(true), json!(false), json!(false)));
    // Use Alpha again, then store Beta again: Alpha stays live
    app.theme_use(&a).unwrap();
    assert_eq!(app.live_theme_oid().as_deref(), Some(a.as_str()));
    store(&b);
    assert_eq!(app.live_theme_oid().as_deref(), Some(a.as_str()), "storing does not change the live theme");
    // choosing the Embedded theme goes back to following it
    app.theme_use(&b).unwrap();
    assert!(app.settings.load().live_theme().is_none());
    // a theme that vanished: live falls back to Embedded
    app.settings.update(|s| s["liveTheme"] = json!("gone")).unwrap();
    assert_eq!(app.live_theme_oid().as_deref(), Some(b.as_str()));
}

/// A start without a window never ran `startup`: live must still get the saved screen fit (regression: live frames had no top inset).
#[test]
fn live_gets_the_saved_fit_before_it_starts_without_a_window() {
    let (_d, app, _rec, _gallery) = setup();
    app.settings.update(|s| s["screenFit"] = json!({ "top": 4, "left": 9 })).unwrap();
    assert!(app.live.effective_fit().is_zero(), "nothing gave live the fit yet (no startup)");
    app.prepare_live();
    assert_eq!(app.live.effective_fit(), crate::core::fit::Fit::new(4, 0, 0, 9));
}

/// Storing a theme with its own corner radius never changes the default in Settings (regression: the default became the theme's value).
#[test]
fn a_theme_corner_radius_does_not_change_the_default() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "corners.mp4");
    assert_eq!(app.settings.load().corner_radius(), 0.0);
    app.theme_set(&oid, &json!({ "corners": 24, "widgets": [] })).unwrap();
    assert_eq!(app.settings.load().corner_radius(), 0.0, "saving the theme leaves the default alone");
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    assert_eq!(app.settings.load().corner_radius(), 0.0, "storing it as Embedded too");
    assert_eq!(app.load_theme(&oid).corners, Some(24.0));
    let v = app.lib.versions(&oid);
    assert_eq!(v[0]["settings"]["corners"]["r"].as_f64(), Some(24.0), "the version itself was made with the theme's radius");
    // the default only changes in Settings, and a theme without an override follows it
    app.settings_update(&json!({ "cornerRadius": 40 })).unwrap();
    app.theme_set(&oid, &json!({ "corners": null, "widgets": [] })).unwrap();
    assert_eq!(app.corner_radius_of(&app.load_theme(&oid)), 40.0);
}

/// The stale flag follows the picture the screen holds, also while another theme is live.
#[test]
fn stale_embedded_is_reported_for_the_theme_that_owns_the_picture() {
    let (_d, app, _rec, gallery) = setup();
    let (a, b) = two_originals(&app, &gallery);
    let r = app.theme_apply(&a).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    app.theme_use(&b).unwrap();
    assert_eq!(app.theme_get(&a).unwrap()["baseStale"], json!(false));
    app.settings_update(&json!({ "cornerRadius": 40 })).unwrap();
    assert_eq!(app.theme_get(&a).unwrap()["baseStale"], json!(true), "corners are baked into Embedded");
    app.settings_update(&json!({ "cornerRadius": 0 })).unwrap();
    app.theme_set(&a, &json!({ "widgets": [{ "id": "c", "type": "clock", "x": 0.1, "y": 0.1, "w": 0.4, "h": 0.3 }] })).unwrap();
    assert_eq!(app.theme_get(&a).unwrap()["baseStale"], json!(false), "live-only items never mark Embedded");
}

#[test]
fn live_layer_off_of_the_current_theme_holds_live() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "t.mp4");
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    assert_eq!(app.live.status()["layerOff"], json!(false));
    app.theme_set(&oid, &json!({ "liveLayer": false })).unwrap();
    assert_eq!(app.live.status()["layerOff"], json!(true));
    app.theme_set(&oid, &json!({ "liveLayer": true })).unwrap();
    wait_until(|| app.live.status()["layerOff"] == json!(false));
}

#[test]
fn a_theme_can_use_a_version_of_another_clip_as_its_base() {
    let (_d, app, _rec, gallery) = setup();
    let (a, b) = two_originals(&app, &gallery);
    // a ready version of Beta
    let v = app.lib.create_version(&b, &json!({ "start": 0, "length": 2, "force": true })).unwrap();
    let vb = v["id"].as_str().unwrap().to_string();
    app.version_handle(&b, &v).wait().unwrap();
    // not ready / unknown versions are refused
    assert!(app.theme_set_base(&a, "zzzzzzzz").is_err());
    let slow = app.lib.create_version(&b, &json!({ "start": 1, "length": 2, "force": true })).unwrap();
    assert!(app.theme_set_base(&a, slow["id"].as_str().unwrap()).is_err(), "a version still queued cannot be the base");
    // Alpha takes Beta's version: nothing is sent, the editor is told to store it
    app.settings.update(|_| {}).unwrap();
    let view = app.theme_set_base(&a, &vb).unwrap();
    assert_eq!(view["baseVersionId"], json!(vb));
    assert_eq!(view["baseOwner"]["originalId"], json!(b));
    assert_eq!(view["baseOwner"]["name"], "Beta");
    assert!(app.settings.load().slot("standby").is_none(), "nothing was sent to the screen");
    // live uses it at once: its file is the background and its clip the fallback
    let tl = app.theme_live_of(&a, app.load_theme(&a));
    assert_eq!(tl.base_video, app.lib.device_path(&vb));
    assert_eq!(tl.fallback_video, app.lib.source_path(&app.lib.read_original(&b).unwrap()));
    // apply Alpha: Beta's version is what gets stored, and Alpha (not Beta) is the current theme
    let r = app.theme_apply(&a).unwrap();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["state"], "done", "{j}");
    assert_eq!(j["result"]["versionId"], json!(vb), "no new version was made");
    assert_eq!(app.settings.load().slot("standby").unwrap()["versionId"], json!(vb));
    assert_eq!(app.live_theme_oid().as_deref(), Some(a.as_str()));
    let view = app.theme_get(&a).unwrap();
    assert_eq!(view["current"], json!(true));
    assert_eq!(view["baseUnstored"], json!(false));
    assert_eq!(app.theme_get(&b).unwrap()["current"], json!(false));
    assert_eq!(app.lib.versions(&a).len(), 0, "Alpha itself has no versions");
    // choosing yet another base later shows "base changed"
    let v2 = app.lib.create_version(&b, &json!({ "start": 0, "length": 3, "force": true })).unwrap();
    app.version_handle(&b, &v2).wait().unwrap();
    let view = app.theme_set_base(&a, v2["id"].as_str().unwrap()).unwrap();
    assert_eq!(view["baseUnstored"], json!(true));
    assert_eq!(app.settings.load().slot("standby").unwrap()["versionId"], json!(vb), "the screen still has the old base");
    // a restart keeps who is current (themeId lives in the slot record)
    assert_eq!(app.live_theme_oid().as_deref(), Some(a.as_str()));
}

// ---------- theme format v1.2 / v1.3 ----------

#[test]
fn default_corner_radius_guides_and_snap_settings_round_trip() {
    let (_d, app, _rec, _g) = setup();
    // old files: on:false means no radius, on:true keeps its r
    assert_eq!(app.settings.load().corner_radius(), 0.0);
    app.settings.update(|s| s["mask"]["corners"] = json!({ "on": true, "r": 50 })).unwrap();
    assert_eq!(app.settings.load().corner_radius(), 50.0);
    let st = app.settings_update(&json!({ "cornerRadius": 0 })).unwrap();
    assert_eq!(st["cornerRadius"], json!(0.0), "0 = none");
    assert_eq!(st["mask"]["corners"]["on"], json!(false));
    let st = app.settings_update(&json!({ "cornerRadius": 120.4, "editor": { "snap": false },
        "screenGuides": { "left": { "size": -5 }, "right": { "size": 180 }, "curve": { "offset": 1100, "left": 5000, "right": 60 } } })).unwrap();
    assert_eq!(st["cornerRadius"], json!(120.0));
    assert_eq!(st["editorSnap"], json!(false));
    assert_eq!(st["screenGuides"]["left"]["size"], json!(0.0), "negative size is clamped");
    assert_eq!(st["screenGuides"]["right"]["size"], json!(180.0));
    assert_eq!(st["screenGuides"]["curve"]["offset"], json!(1100.0));
    assert_eq!(st["screenGuides"]["curve"]["left"], json!(2300.0), "the band stays inside the canvas");
    assert_eq!(st["screenGuides"]["curve"]["right"], json!(60.0));
    // a v1.2 settings value is migrated: curve edges dropped, zones -> sides
    let st2 = app.settings_update(&json!({ "screenGuides": { "curve": { "right": 90, "left": 5, "top": 0, "bottom": 0 }, "zones": [{ "edge": "right", "size": 180 }, { "edge": "left", "size": 120 }] } })).unwrap();
    assert_eq!(st2["screenGuides"], json!({ "left": { "size": 120.0 }, "right": { "size": 180.0 }, "curve": { "offset": 0.0, "left": 0.0, "right": 0.0 } }));
    let st = app.settings_update(&json!({ "screenGuides": st["screenGuides"].clone() })).unwrap();
    // survives a reload from disk; an untouched file stays free of the new keys
    let again = app.app_state();
    assert_eq!(again["screenGuides"], st["screenGuides"]);
    let (_d2, app2, _r2, _g2) = setup();
    app2.settings.update(|_| {}).unwrap();
    let raw = std::fs::read_to_string(_d2.path().join("home/settings.json")).unwrap();
    assert!(!raw.contains("screenGuides") && !raw.contains("editor"), "{raw}");
}

#[test]
fn a_themes_own_corner_radius_is_applied_to_the_base_and_marks_it_stale_when_changed() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Round.mp4");
    app.settings_update(&json!({ "cornerRadius": 60 })).unwrap();
    let v = app.theme_get(&oid).unwrap();
    assert_eq!(v["cornersEffective"], json!(60.0));
    assert_eq!(v["cornersDefault"], json!(60.0));
    let r = app.theme_apply(&oid).unwrap();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["state"], "done", "{j}");
    let vid = j["result"]["versionId"].as_str().unwrap().to_string();
    let base = app.lib.find_version(&vid).unwrap().1;
    assert_eq!(base["settings"]["corners"], json!({ "on": true, "r": 60 }), "the default radius went into the base");
    assert_eq!(app.theme_get(&oid).unwrap()["baseStale"], json!(false));
    // this theme: no rounding at all -> the stored base is out of date
    let v = app.theme_set(&oid, &json!({ "corners": 0 })).unwrap();
    assert_eq!(v["cornersEffective"], json!(0.0));
    assert_eq!(v["baseStale"], json!(true));
    // changing the default does not matter for a theme with its own value, and does for one without
    assert_eq!(app.theme_get(&oid).unwrap()["cornersDefault"], json!(60.0));
    app.settings_update(&json!({ "cornerRadius": 20 })).unwrap();
    assert_eq!(app.theme_get(&oid).unwrap()["cornersEffective"], json!(0.0));
    let v = app.theme_set(&oid, &json!({ "corners": null })).unwrap();
    assert_eq!(v["cornersEffective"], json!(20.0));
    assert_eq!(v["baseStale"], json!(true), "60 on the screen, 20 wanted");
    // applying again makes a new base with the new radius
    let r = app.theme_apply(&oid).unwrap();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["state"], "done", "{j}");
    let vid2 = j["result"]["versionId"].as_str().unwrap();
    assert_ne!(vid2, vid);
    assert_eq!(app.lib.find_version(vid2).unwrap().1["settings"]["corners"]["r"], json!(20));
    assert_eq!(app.theme_get(&oid).unwrap()["baseStale"], json!(false));
    // the theme's guide offsets and effective guides
    app.settings_update(&json!({ "screenGuides": { "left": { "size": 100 }, "right": { "size": 180 }, "curve": { "offset": 0, "left": 50, "right": 50 } } })).unwrap();
    let v = app.theme_set(&oid, &json!({ "gap": 40, "guidesOffset": { "right": { "size": 30 }, "curve": { "offset": 20, "right": -20 } } })).unwrap();
    assert_eq!(v["guides"]["right"]["size"], json!(210.0));
    assert_eq!(v["guides"]["curve"]["offset"], json!(20.0));
    assert_eq!(v["guides"]["curve"]["right"], json!(30.0));
    assert_eq!(v["theme"]["gap"], json!(40.0));
    assert_eq!(v["theme"]["guidesOffset"]["right"]["size"], json!(30.0));
    let v = app.theme_set(&oid, &json!({ "gap": 99999 })).unwrap();
    assert_eq!(v["theme"]["gap"], json!(200.0));
    assert_eq!(app.theme_get(&oid).unwrap()["theme"]["gap"], json!(200.0));
}

// ---------- theme format v1.5: screen fit ----------

#[test]
fn screen_fit_is_validated_saved_and_reported() {
    let (_d, app, _rec, _gallery) = setup();
    assert_eq!(app.app_state()["screenFit"], json!({ "top": 0, "right": 0, "bottom": 0, "left": 0, "shiftX": 0, "shiftY": 0 }), "default: nothing hidden");
    let st = app.settings_update(&json!({ "screenFit": { "top": 4, "left": 9 } })).unwrap();
    assert_eq!(st["screenFit"], json!({ "top": 4, "right": 0, "bottom": 0, "left": 9, "shiftX": 0, "shiftY": 0 }));
    // out of range / not a number: clamped to 0..200, garbage counts as 0; keys left out keep their value
    let st = app.settings_update(&json!({ "screenFit": { "top": 999, "right": -5, "bottom": "x", "left": 9.4 } })).unwrap();
    assert_eq!(st["screenFit"], json!({ "top": 200, "right": 0, "bottom": 0, "left": 9, "shiftX": 0, "shiftY": 0 }));
    let st = app.settings_update(&json!({ "screenFit": { "right": 3 } })).unwrap();
    assert_eq!(st["screenFit"], json!({ "top": 200, "right": 3, "bottom": 0, "left": 9, "shiftX": 0, "shiftY": 0 }));
    // on disk it is written only when something is hidden
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(app.settings.file()).unwrap()).unwrap();
    assert_eq!(on_disk["screenFit"]["left"], json!(9));
    app.settings_update(&json!({ "screenFit": { "top": 0, "right": 0, "bottom": 0, "left": 0 } })).unwrap();
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(app.settings.file()).unwrap()).unwrap();
    assert!(on_disk.get("screenFit").is_none());
}

#[test]
fn edge_offsets_from_the_tuning_tool_are_taken_over_once() {
    let (d, app, _rec, _gallery) = setup();
    let old = d.path().join("home").join("edge-offsets.json");
    std::fs::write(&old, r#"{ "top": 4, "right": 0, "bottom": 0, "left": 9, "note": "from cs2 edge-tune" }"#).unwrap();
    app.startup();
    assert_eq!(app.app_state()["screenFit"], json!({ "top": 4, "right": 0, "bottom": 0, "left": 9, "shiftX": 0, "shiftY": 0 }));
    assert!(!old.exists(), "the file is set aside after the migration");
    assert!(d.path().join("home").join("edge-offsets.migrated.json").exists());
    // a later change is never overwritten by the old file
    std::fs::write(&old, r#"{ "top": 50, "right": 0, "bottom": 0, "left": 50 }"#).unwrap();
    app.settings_update(&json!({ "screenFit": { "top": 5 } })).unwrap();
    app.startup();
    assert_eq!(app.app_state()["screenFit"]["top"], json!(5));
    assert_eq!(app.app_state()["screenFit"]["left"], json!(9));
}

#[test]
fn changing_the_screen_fit_makes_the_stored_base_stale() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Fit.mp4");
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    let first = app.theme_get(&oid).unwrap()["baseVersionId"].as_str().unwrap().to_string();
    assert_eq!(app.theme_get(&oid).unwrap()["baseStale"], json!(false));
    let key0 = app.app_state()["notchKey"].as_str().unwrap().to_string();
    app.settings_update(&json!({ "screenFit": { "top": 4, "left": 9 } })).unwrap();
    assert_ne!(app.app_state()["notchKey"].as_str().unwrap(), key0, "the fit is part of the cache key");
    assert!(app.app_state()["notchKey"].as_str().unwrap().starts_with(&key0));
    assert_eq!(app.app_state()["oldNotchCount"], json!(1), "the stored version is counted as made with another setting");
    let v = app.theme_get(&oid).unwrap();
    assert_eq!(v["baseStale"], json!(true), "Base needs re-storing");
    // applying again makes a new version with the fit (the old one is not reused), and clears the notice
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    let v = app.theme_get(&oid).unwrap();
    assert_ne!(v["baseVersionId"].as_str().unwrap(), first);
    assert_eq!(v["baseStale"], json!(false));
    let made = app.lib.find_version(v["baseVersionId"].as_str().unwrap()).unwrap().1;
    assert_eq!(made["notch"]["fit"], json!({ "top": 4, "right": 0, "bottom": 0, "left": 9, "shiftX": 0, "shiftY": 0 }), "the version records the fit it was made with");
    let tl = app.theme_live_of(&oid, app.load_theme(&oid));
    assert_eq!(tl.base_fit, Some(super::fit::Fit::new(4, 0, 0, 9)), "the engine is told which fit the base has");
    // back to no fit: stale again, and the first version (made without a fit) is the one that matches
    app.settings_update(&json!({ "screenFit": { "top": 0, "left": 0 } })).unwrap();
    assert_eq!(app.theme_get(&oid).unwrap()["baseStale"], json!(true));
}

// ---------- a returning file is the same item; duplicates from before are merged ----------

#[test]
fn a_file_that_comes_back_is_its_missing_item_again() {
    let (_d, app, _rec, gallery) = setup();
    put(&gallery.join("Clip [we-1].mp4"), b"first version of the file");
    app.startup();
    let ids = |app: &App| app.originals_list()["originals"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let a = ids(&app)[0].clone();
    // it was recorded under another path, then went missing
    std::fs::remove_file(gallery.join("Clip [we-1].mp4")).unwrap();
    app.lib.patch_original(&a, &json!({ "file": gallery.join("old").join("CLIP [we-1].mp4").to_string_lossy() })).unwrap();
    app.gallery_sync();
    assert!(app.original_get(&a).unwrap()["missing"]["since"].is_string());
    // it returns (another size and time, as a re-download would give): the same item, not a second one
    put(&gallery.join("Clip [we-1].mp4"), b"the file again, but a different size");
    app.gallery_sync();
    assert_eq!(ids(&app), vec![a.clone()], "no second item for the returning file");
    let o = app.original_get(&a).unwrap();
    assert!(o["missing"].is_null(), "no longer missing");
    assert!(o["file"].as_str().unwrap().ends_with("Clip [we-1].mp4"));
    assert_eq!(o["name"], json!("Clip"), "the name of the item is kept");
}

fn dup_record(app: &App, name: &str, file: &std::path::Path, created: &str, missing: bool) -> String {
    let id = super::library::new_id();
    app.lib
        .write_original(&json!({ "id": id, "name": name, "file": file.to_string_lossy(), "fileName": file.file_name().unwrap().to_string_lossy(),
            "created": created, "fileSize": 10, "fileMtime": 1, "assets": "ready", "missing": if missing { json!({ "since": "2026-10-12T11:09:07Z" }) } else { Value::Null } }))
        .unwrap();
    id
}

fn dup_version(app: &App, oid: &str, n: u32) -> String {
    let vid = super::library::new_id();
    app.lib.write_version(oid, &json!({ "id": vid, "originalId": oid, "n": n, "status": "ready", "notchKey": "off", "settings": { "start": 0, "length": 5 } })).unwrap();
    vid
}

fn trash(d: &std::path::Path) -> impl Fn(&std::path::Path) -> super::error::CoreResult<()> {
    let to = d.join("trash");
    move |p: &std::path::Path| {
        std::fs::create_dir_all(&to)?;
        std::fs::rename(p, to.join(p.file_name().unwrap()))?;
        Ok(())
    }
}

fn set_mtime(p: &std::path::Path, secs_ago: u64) {
    let f = std::fs::File::options().write(true).open(p).unwrap();
    f.set_modified(std::time::SystemTime::now() - Duration::from_secs(secs_ago)).unwrap();
}

/// The shape found in the library: the missing item is the one the loop slot and the current theme refer to, the newer item of
/// the same file has no theme.json (and a version of its own).
#[test]
fn duplicates_of_one_file_are_merged_into_the_one_the_screen_uses() {
    let (d, app, _rec, gallery) = setup();
    let file = gallery.join("Asuka [we-2782447433].mp4");
    put(&file, b"the clip");
    let a = dup_record(&app, "Asuka", &file, "2026-10-01T00:00:00Z", true); // missing, referenced
    let b = dup_record(&app, "Asuka", &file, "2026-10-12T12:00:00Z", false); // re-imported later
    let va = dup_version(&app, &a, 1);
    let vb = dup_version(&app, &b, 1);
    std::fs::write(app.lib.o_dir(&a).join("theme.json"), r#"{"v":1,"widgets":[{"id":"c","type":"clock","x":0.1,"y":0.1,"w":0.3,"h":0.3}],"baseVersionId":null}"#).unwrap();
    std::fs::create_dir_all(app.lib.o_dir(&a).join("assets")).unwrap();
    std::fs::write(app.lib.o_dir(&a).join("assets").join("logo.png"), b"png").unwrap();
    app.settings.update(|s| s["slots"]["standby"] = json!({ "name": "x", "size": 1000000, "at": "t", "originalId": a, "versionId": va, "themeId": a })).unwrap();
    // read-only plan first
    let plan = app.repair_duplicates_with(false, &trash(d.path()));
    assert_eq!(plan.len(), 1);
    assert_eq!(plan[0]["keep"]["id"], json!(a), "the item the screen refers to stays");
    assert_eq!(plan[0]["duplicates"][0]["id"], json!(b));
    assert_eq!(plan[0]["applied"], json!(false));
    assert_eq!(app.originals_list()["originals"].as_array().unwrap().len(), 2, "planning changes nothing");
    assert!(app.lib.find_version(&vb).is_some_and(|(o, _)| o == b));
    // apply (the user had chosen the duplicate with "Use this theme": live then points at the one that stays)
    app.settings.update(|s| s["liveTheme"] = json!(b)).unwrap();
    let done = app.repair_duplicates_with(true, &trash(d.path()));
    assert_eq!(done.len(), 1);
    assert_eq!(app.settings.load().live_theme().as_deref(), Some(a.as_str()), "the live theme was repointed");
    let list = app.originals_list();
    assert_eq!(list["originals"].as_array().unwrap().len(), 1);
    assert_eq!(list["originals"][0]["id"], json!(a));
    let o = app.original_get(&a).unwrap();
    assert!(o["missing"].is_null(), "the file is there: not missing any more");
    let (owner, v) = app.lib.find_version(&vb).unwrap();
    assert_eq!(owner, a, "the version of the duplicate moved over");
    assert_eq!(v["originalId"], json!(a));
    assert_eq!(app.lib.versions(&a).len(), 2);
    let ns: Vec<f64> = app.lib.versions(&a).iter().map(|v| v["n"].as_f64().unwrap()).collect();
    assert!(ns.contains(&1.0) && ns.contains(&2.0), "version numbers stay unique: {ns:?}");
    assert!(app.lib.find_version(&va).is_some());
    assert!(app.lib.o_dir(&a).join("theme.json").is_file() && app.lib.o_dir(&a).join("assets").join("logo.png").is_file(), "theme and pictures kept");
    assert!(!app.lib.o_dir(&b).join("original.json").exists());
    assert!(d.path().join("trash").join(&b).exists(), "the emptied record went to the bin, not away for good");
    let sb = app.settings.load().slot("standby").unwrap().clone();
    assert_eq!((sb["originalId"].as_str(), sb["versionId"].as_str(), sb["themeId"].as_str()), (Some(a.as_str()), Some(va.as_str()), Some(a.as_str())));
    // nothing left to do
    assert!(app.repair_duplicates_with(true, &trash(d.path())).is_empty());
}

/// The screen refers to the newer item, which has its own newer theme.json: it stays, the older theme is kept aside.
#[test]
fn merging_keeps_the_newer_theme_and_the_older_one_beside_it_and_repoints_the_slot() {
    let (d, app, _rec, gallery) = setup();
    let file = gallery.join("Clip.mp4");
    put(&file, b"the clip");
    let a = dup_record(&app, "Clip", &file, "2026-10-01T00:00:00Z", true);
    let b = dup_record(&app, "Clip", &file, "2026-10-12T12:00:00Z", false);
    let vb = dup_version(&app, &b, 1);
    let (ta, tb) = (app.lib.o_dir(&a).join("theme.json"), app.lib.o_dir(&b).join("theme.json"));
    std::fs::write(&ta, r#"{"v":1,"widgets":[]}"#).unwrap();
    std::fs::write(&tb, r#"{"v":1,"widgets":[{"id":"t","type":"text","x":0,"y":0,"w":0.2,"h":0.2}]}"#).unwrap();
    set_mtime(&ta, 3600);
    set_mtime(&tb, 10);
    std::fs::create_dir_all(app.lib.o_dir(&b).join("assets")).unwrap();
    std::fs::write(app.lib.o_dir(&b).join("assets").join("p.png"), b"png").unwrap();
    app.settings.update(|s| s["slots"]["standby"] = json!({ "name": "x", "size": 1000000, "at": "t", "originalId": b, "versionId": vb, "themeId": b })).unwrap();
    let plan = app.repair_duplicates_with(true, &trash(d.path()));
    assert_eq!(plan[0]["keep"]["id"], json!(b), "referenced by the screen: it stays");
    assert_eq!(app.originals_list()["originals"].as_array().unwrap().len(), 1);
    let th: Value = serde_json::from_str(&std::fs::read_to_string(app.lib.o_dir(&b).join("theme.json")).unwrap()).unwrap();
    assert_eq!(th["widgets"].as_array().unwrap().len(), 1, "its own (newer) theme is kept");
    let prev: Value = serde_json::from_str(&std::fs::read_to_string(app.lib.o_dir(&b).join("theme.prev.json")).unwrap()).unwrap();
    assert_eq!(prev["widgets"].as_array().unwrap().len(), 0, "the older theme is kept as theme.prev.json");
    assert!(app.lib.o_dir(&b).join("assets").join("p.png").is_file());
    assert!(app.original_get(&b).unwrap()["missing"].is_null());
    assert_eq!(app.settings.load().slot("standby").unwrap()["themeId"], json!(b));
    // the other way round: the kept item is referenced by the slot, the duplicate holds the newer theme.json, which wins
    let (d2, app2, _r, g2) = setup();
    let f2 = g2.join("Clip.mp4");
    put(&f2, b"the clip");
    let a2 = dup_record(&app2, "Clip", &f2, "2026-10-12T12:00:00Z", false);
    let b2 = dup_record(&app2, "Clip", &f2, "2026-10-01T00:00:00Z", true);
    let (x, y) = (app2.lib.o_dir(&a2).join("theme.json"), app2.lib.o_dir(&b2).join("theme.json"));
    std::fs::write(&x, r#"{"v":1,"widgets":[]}"#).unwrap();
    std::fs::write(&y, r#"{"v":1,"widgets":[{"id":"t","type":"text","x":0,"y":0,"w":0.2,"h":0.2}]}"#).unwrap();
    set_mtime(&x, 3600);
    set_mtime(&y, 10);
    let v2 = dup_version(&app2, &b2, 1);
    let va2 = dup_version(&app2, &a2, 1);
    app2.settings.update(|s| s["slots"]["standby"] = json!({ "name": "x", "size": 1000000, "at": "t", "originalId": a2, "versionId": va2, "themeId": a2 })).unwrap();
    let plan = app2.repair_duplicates_with(true, &trash(d2.path()));
    assert_eq!(plan[0]["keep"]["id"], json!(a2));
    let th: Value = serde_json::from_str(&std::fs::read_to_string(app2.lib.o_dir(&a2).join("theme.json")).unwrap()).unwrap();
    assert_eq!(th["widgets"].as_array().unwrap().len(), 1);
    assert!(app2.lib.o_dir(&a2).join("theme.prev.json").is_file());
    assert_eq!(app2.lib.find_version(&v2).unwrap().0, a2);
}

// ---------- a base that does not fit the screen is never used ----------

fn sized_version(app: &App, oid: &str, n: u32, bytes: u64) -> String {
    let vid = super::library::new_id();
    app.lib
        .write_version(oid, &json!({ "id": vid, "originalId": oid, "n": n, "status": "ready", "notchKey": "off", "settings": { "start": 0, "length": 10, "quality": "auto" },
            "result": { "size": bytes, "fps": 18, "length": 10, "start": 0 } }))
        .unwrap();
    vid
}

#[test]
fn an_oversize_base_is_replaced_by_the_slot_version_of_the_same_file_and_never_used() {
    let (d, app, _rec, gallery) = setup();
    let file = gallery.join("Asuka.mp4");
    put(&file, b"the clip");
    let a = dup_record(&app, "Asuka", &file, "2026-10-01T00:00:00Z", true); // the old item, its version is on the screen
    let b = dup_record(&app, "Asuka", &file, "2026-10-12T12:00:00Z", false); // the item the theme was applied for
    let good = sized_version(&app, &a, 1, 7_000_000);
    let big = sized_version(&app, &b, 1, 12_855_458);
    std::fs::write(app.lib.o_dir(&b).join("theme.json"), format!(r#"{{"v":1,"widgets":[],"baseVersionId":"{big}"}}"#)).unwrap();
    app.settings.update(|s| s["slots"]["standby"] = json!({ "name": "x", "size": 7000000, "at": "t", "originalId": a, "versionId": good, "themeId": b })).unwrap();
    // never used as a base, even before any repair
    let tl = app.theme_live_of(&b, app.load_theme(&b));
    assert!(tl.base_video.is_none(), "an oversize version is not the live background or base");
    assert!(app.over_budget(&app.lib.find_version(&big).unwrap().1));
    // the plan (read only) says what would happen
    let plan = app.repair_library_with(false, &trash(d.path()));
    assert_eq!(plan["themeBases"][0]["from"], json!(big));
    assert_eq!(plan["themeBases"][0]["to"], json!(good));
    assert!(std::fs::read_to_string(app.lib.o_dir(&b).join("theme.json")).unwrap().contains(&big), "planning changes nothing");
    // repair: duplicates merge first, then the theme points to the version that is on the screen
    let done = app.repair_library_with(true, &trash(d.path()));
    assert_eq!(done["themeBases"].as_array().unwrap().len(), 1);
    let kept = if app.lib.read_original(&a).is_some() { a.clone() } else { b.clone() };
    let th: Value = serde_json::from_str(&std::fs::read_to_string(app.lib.o_dir(&kept).join("theme.json")).unwrap()).unwrap();
    assert_eq!(th["baseVersionId"], json!(good));
    assert!(app.repair_library_with(true, &trash(d.path()))["themeBases"].as_array().unwrap().is_empty(), "nothing left to repair");
    // with no usable version of the same file the base is cleared
    let (d2, app2, _r, g2) = setup();
    let f2 = g2.join("Other.mp4");
    put(&f2, b"x");
    let o2 = dup_record(&app2, "Other", &f2, "2026-10-01T00:00:00Z", false);
    let big2 = sized_version(&app2, &o2, 1, 13_000_000);
    std::fs::write(app2.lib.o_dir(&o2).join("theme.json"), format!(r#"{{"v":1,"baseVersionId":"{big2}"}}"#)).unwrap();
    app2.repair_library_with(true, &trash(d2.path()));
    let th: Value = serde_json::from_str(&std::fs::read_to_string(app2.lib.o_dir(&o2).join("theme.json")).unwrap()).unwrap();
    assert!(th["baseVersionId"].is_null());
}

#[test]
fn apply_always_asks_for_the_best_that_fits() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Quality.mp4");
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    let first = app.theme_get(&oid).unwrap()["baseVersionId"].as_str().unwrap().to_string();
    // an older base made with a fixed quality
    app.lib.patch_version(&first, &json!({ "settings": { "quality": 95 } })).unwrap();
    app.settings_update(&json!({ "screenFit": { "top": 4, "left": 9 } })).unwrap();
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    let second = app.theme_get(&oid).unwrap()["baseVersionId"].as_str().unwrap().to_string();
    assert_ne!(first, second);
    assert_eq!(app.lib.find_version(&second).unwrap().1["settings"]["quality"], json!("auto"));
}

#[test]
fn an_inflated_capacity_is_not_trusted() {
    use super::settings::{budgets, capacity_from, MAX_CAPACITY};
    let (_d, app, _rec, _g) = setup();
    app.settings.update(|s| {
        s["slots"]["startup"] = json!({ "name": "(boot splash)", "size": 537054, "at": "t", "splash": "v1-x" });
        s["slots"]["standby"] = json!({ "name": "x", "size": 12855458, "at": "t" });
    })
    .unwrap();
    let st = app.settings.load();
    // what the screen said right after the oversize upload: nothing free
    let full = json!({ "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 0 } });
    assert!(capacity_from(&full, &st).is_none(), "a full flash is not a measurement");
    // more than the flash holds
    let lie = json!({ "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 200 } });
    assert!(capacity_from(&lie, &st).is_none());
    // a record from before (13.39 MB) is capped when the budget is worked out
    app.settings.update(|s| s["capacity"] = json!({ "bytes": 13_392_512, "at": "x" })).unwrap();
    let b = budgets(&app.settings.load(), Some(537_054));
    assert!(b.total <= MAX_CAPACITY && b.loop_bytes < 10_900_000.0 && b.loop_bytes > 10_800_000.0, "budget {:?}", b);
    // the startup repair replaces the record, and says so
    app.repair_library_with(true, &|_| Ok(()));
    assert_eq!(app.settings.load().capacity().unwrap().0, MAX_CAPACITY, "the inflated record is replaced by the conservative value");
    // a true measurement still works (hardware: 792 free blocks, 10.66 MB loop + 0.54 MB intro = 11.6 MB)
    app.settings.update(|s| s["slots"]["standby"] = json!({ "name": "x", "size": 10_662_507, "at": "t" })).unwrap();
    let ok = json!({ "data": { "i_blocks": 24520, "i_block_size": 512, "i_block_free": 792 } });
    assert_eq!(capacity_from(&ok, &app.settings.load()), Some(MAX_CAPACITY), "the true 11,605,065 is counted as the conservative value");
}

#[test]
fn screen_fit_shift_is_saved_validated_and_part_of_the_base_key() {
    let (_d, app, _rec, _g) = setup();
    let st = app.settings_update(&json!({ "screenFit": { "top": 4, "left": 9, "shiftX": 3, "shiftY": -50 } })).unwrap();
    assert_eq!(st["screenFit"]["shiftX"], json!(3));
    assert_eq!(st["screenFit"]["shiftY"], json!(-20), "-20..20");
    let k1 = app.app_state()["notchKey"].as_str().unwrap().to_string();
    assert!(k1.ends_with("+fit:4.0.0.9.3.-20"), "{k1}");
    app.settings_update(&json!({ "screenFit": { "shiftX": 0, "shiftY": 0 } })).unwrap();
    let k2 = app.app_state()["notchKey"].as_str().unwrap().to_string();
    assert!(k2.ends_with("+fit:4.0.0.9") && k1 != k2);
    // fit_cal in "theme" mode tunes the live picture and logs; ending gives it back
    let r = app.fit_cal(&json!({ "action": "start", "mode": "theme", "fit": { "top": 6, "shiftX": 2 } })).unwrap();
    assert_eq!(r["tuning"], json!(true));
    assert_eq!(app.live.effective_fit().top, 6);
    app.fit_cal(&json!({ "action": "set", "mode": "theme", "fit": { "top": 7, "shiftX": 2 } })).unwrap();
    assert_eq!(app.live.effective_fit().top, 7);
    app.fit_cal(&json!({ "action": "end" })).unwrap();
    assert_eq!(app.live.effective_fit().top, 4, "the saved fit again");
}

#[test]
fn v1_6_widgets_are_cleaned_and_saved() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "t.mp4");
    let t = json!({ "widgets": [
        { "id": "s", "type": "sensor", "x": 0.1, "y": 0.1, "w": 0.2, "h": 0.2, "props": { "metric": "gpu.temp", "style": "ring", "history": 1, "thresholds": [{ "at": 90, "color": "#ff0000" }, { "at": 70, "color": "#ffaa00" }], "min": 100, "max": 10 } },
        { "id": "p", "type": "slideshow", "x": 0.4, "y": 0.1, "w": 0.3, "h": 0.3, "props": { "files": ["assets/a.png", "../evil.png"], "seconds": 0 } },
        { "id": "i", "type": "info", "x": 0.1, "y": 0.5, "w": 0.3, "h": 0.2, "props": { "kind": "countdown", "target": "garbage" } } ] });
    let v = app.theme_set(&oid, &t).unwrap();
    let w = &v["theme"]["widgets"];
    assert_eq!(w[0]["props"]["style"], "ring");
    assert_eq!(w[0]["props"]["history"], json!(5));
    assert_eq!(w[0]["props"]["thresholds"][0]["at"], json!(70.0), "sorted");
    assert!(w[0]["props"]["min"].is_null() && w[0]["props"]["max"].is_null());
    assert_eq!(w[1]["props"]["files"], json!(["assets/a.png"]));
    assert_eq!(w[1]["props"]["seconds"], json!(1));
    assert!(w[2]["props"]["target"].is_null());
    assert!(app.theme_set(&oid, &json!({ "widgets": [{ "type": "sensor" }, { "type": "hologram" }] })).is_err());
}

#[test]
fn v4_more_widgets_are_cleaned_and_saved() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "t.mp4");
    let t = json!({ "widgets": [
        { "id": "m", "type": "meters", "x": 0.1, "y": 0.1, "w": 0.2, "h": 0.3, "props": { "rows": [{ "metric": "gpu.temp" }, { "metric": "bogus" }], "rate": 9 } },
        { "id": "z", "type": "worldclock", "x": 0.4, "y": 0.1, "w": 0.2, "h": 0.3, "props": { "zones": [{ "name": "Tokyo", "tz": "Tokyo Standard Time" }, { "name": "x", "tz": "../etc" }] } },
        { "id": "t", "type": "timer", "x": 0.7, "y": 0.1, "w": 0.2, "h": 0.3, "props": { "mode": "timer", "duration": 0, "running": true, "startedAt": 1700000000000.0 } } ] });
    let v = app.theme_set(&oid, &t).unwrap();
    let w = &v["theme"]["widgets"];
    assert_eq!(w[0]["props"]["rows"][1]["metric"], "cpu.load");
    assert_eq!(w[0]["props"]["rate"], json!(4));
    assert_eq!(w[1]["props"]["zones"][0]["tz"], "Tokyo Standard Time");
    assert_eq!(w[1]["props"]["zones"][1]["tz"], "");
    assert_eq!(w[2]["props"]["duration"], json!(1.0));
    assert_eq!(w[2]["props"]["running"], json!(true));
    // the saved file reads back with the same types
    assert_eq!(app.theme_get(&oid).unwrap()["theme"]["widgets"][2]["type"], "timer");
}

#[test]
fn weather_place_and_unit_are_stored_in_settings() {
    let (_d, app, _rec, _gallery) = setup();
    let st = app.app_state();
    assert_eq!(st["weather"], json!({ "place": null, "unit": "C" }), "nothing is set by itself");
    assert!(!std::fs::read_to_string(app.settings.file()).unwrap_or_default().contains("weather"));
    let st = app.weather_set_place(&json!({ "name": "Sydney", "lat": -33.8678, "lon": 151.2073, "tz": "Australia/Sydney", "country": "Australia" })).unwrap();
    assert_eq!(st["weather"]["place"], json!({ "name": "Sydney", "lat": -33.8678, "lon": 151.2073, "tz": "Australia/Sydney" }));
    let st = app.settings_update(&json!({ "weather": { "unit": "F" } })).unwrap();
    assert_eq!(st["weather"]["unit"], "F");
    assert_eq!(st["weather"]["place"]["name"], "Sydney", "the unit change keeps the place");
    assert_eq!(app.settings_update(&json!({ "weather": { "unit": "K" } })).unwrap()["weather"]["unit"], "F", "an unknown unit is ignored");
    assert!(app.weather_set_place(&json!({ "name": "Nowhere", "lat": 200, "lon": 0 })).is_err());
    assert!(app.weather_search("a").is_err(), "too short, and no network call");
    let st = app.weather_set_place(&Value::Null).unwrap();
    assert!(st["weather"]["place"].is_null() && st["weather"]["unit"] == "F");
}

#[test]
fn theme_pictures_are_served_only_from_the_assets_folder() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "t.mp4");
    let a = app.theme_asset_add_bytes(&oid, "pic.png", &super::fake_media::png(4, 4, [1, 2, 3])).unwrap();
    assert_eq!(a["file"], "assets/pic.png");
    assert!(app.resolve_media(&format!("/media/o/{oid}/assets/pic.png"), "").is_some());
    assert!(app.resolve_media(&format!("/media/o/{oid}/assets/missing.png"), "").is_none());
    assert!(app.resolve_media(&format!("/media/o/{oid}/assets/..%5Cx"), "").is_none());
    assert!(app.resolve_media(&format!("/media/o/{oid}/assets/.."), "").is_none());
    assert!(app.resolve_media("/media/o/nosuchid1/assets/pic.png", "").is_none());
}

#[test]
fn a_busy_screen_names_who_holds_it_and_the_check_reports_it() {
    let d = tempfile::tempdir().unwrap();
    let home = d.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let app = App::with_device(
        home,
        Backends { media: Arc::new(FakeMedia { delay: Duration::from_millis(20) }), sources: Arc::new(NoSources) },
        Arc::new(Rec::default()),
        "2.0.0-test",
        DeviceService::fake("busy", 0.01, false),
    );
    let e = app.device_check().unwrap_err();
    assert!(e.code_is("busy"), "{e}");
    assert!(e.message.contains("using the screen") || e.message.contains("using it"), "{}", e.message);
    assert!(!e.message.contains("LOVINGCOOL") || e.message.starts_with("LOVINGCOOL MONITOR is using"), "only named when it is the holder: {}", e.message);
    // the build is shown: hash, commit date, build time
    let st = app.app_state();
    assert!(st["build"]["hash"].as_str().is_some_and(|h| !h.is_empty()) && st["build"]["builtAt"].as_u64().unwrap_or(0) > 1_700_000_000, "{}", st["build"]);
    assert!(super::buildinfo::startup_line("2.9.0").contains(&format!("pid {}", std::process::id())));
}

#[test]
fn live_status_edits_and_conversions_do_not_pause_but_upload_and_manual_check_do() {
    use crate::live::Config;
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Live all along.mp4");
    app.live.set_no_device(true);
    app.live.set_config_override(Some(Config::default()));
    app.live.start();
    std::thread::sleep(Duration::from_millis(400));
    assert!(app.live.streaming(), "live runs");
    let (p0, r0) = (app.live.pause_count(), app.live.resume_count());

    // Routine snapshots, presence, theme edits and media work never touch the live stream.
    for i in 0..3 {
        assert!(app.device_presence()["present"] == true);
        assert!(app.app_state()["live"]["status"]["connected"] == true);
        assert_eq!(app.device_state().unwrap()["live"], json!(true));
        app.theme_set(&oid, &json!({ "widgets": [{ "id": "c", "type": "clock", "x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3, "props": { "seconds": i % 2 == 0 } }] })).unwrap();
        app.refresh_live_theme();
    }
    let r = app.version_create(&oid, &json!({ "start": 1, "length": 5, "quality": "auto" })).unwrap();
    let j = wait_job(&app, r["jobId"].as_str().unwrap());
    assert_eq!(j["status"], "done", "{j}");
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!((app.live.pause_count(), app.live.resume_count()), (p0, r0), "no pause for status, presence, edits or a conversion");
    assert!(app.live.streaming());

    // an upload is a real device write: exactly one pause and one resume
    let vid = r["versionId"].as_str().unwrap().to_string();
    let s = app.version_send(&vid).unwrap();
    let j = wait_job(&app, s["jobId"].as_str().unwrap());
    assert_eq!(j["status"], "done", "{j}");
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!((app.live.pause_count(), app.live.resume_count()), (p0 + 1, r0 + 1), "one pause and one resume for the upload");
    assert!(app.live.streaming(), "live is back");
    // An explicit Check again verifies firmware rather than recycling cached Live health.
    let info = app.device_check().unwrap();
    assert!(info.get("_live").is_none());
    assert!(info.get("_capacity").is_some());
    wait_until(|| app.live.streaming());
    assert_eq!((app.live.pause_count(), app.live.resume_count()), (p0 + 2, r0 + 2));
    let log = std::fs::read_to_string(crate::device::log::base_dir().join("device.log")).unwrap_or_default();
    assert!(log.contains("device task: send"), "the reason is logged");
    app.live.stop();
}

#[test]
fn connection_recovery_rechecks_firmware_and_respects_active_device_jobs() {
    use super::scheduler::{Res, Resources, TaskSpec};
    let d = tempfile::tempdir().unwrap();
    let rec = Arc::new(Rec::default());
    let app = App::with_device(
        d.path().join("home"),
        Backends { media: Arc::new(FakeMedia { delay: Duration::from_millis(20) }), sources: Arc::new(NoSources) },
        rec.clone(), "test", DeviceService::fake("noreply", 0.01, false),
    );
    app.live.set_no_device(true);
    app.live.set_config_override(Some(crate::live::Config::default()));
    app.live.start();
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    let rx = Arc::new(Mutex::new(rx));
    let job = app.sched.submit(TaskSpec::new("send", "Simulated upload ownership", Resources::one(Res::Device)), move |_| {
        rx.lock().unwrap().recv_timeout(Duration::from_secs(10)).unwrap(); Ok(json!({}))
    });
    wait_until(|| app.sched.device_active());
    app.recover_live_no_reply();
    assert!(!rec.jobs.lock().unwrap().iter().any(|(kind, _)| kind == "recovery"), "no recovery can overlap an existing device job");
    tx.send(()).unwrap(); job.wait().unwrap();
    assert!(app.device.read_info().unwrap_err().code_is("noReply"), "skipped recovery did not touch firmware");
    wait_until(|| !app.sched.device_active());
    app.recover_live_no_reply();
    wait_until(|| rec.jobs.lock().unwrap().iter().any(|(kind, state)| kind == "recovery" && state == "done"));
    assert!(app.device.read_info().is_ok(), "confirmed noReply was recovered by the exclusive device task");
    assert!(rec.busy.lock().unwrap().contains(&true));
    app.live.stop();
}

#[test]
fn only_picture_inputs_make_the_stored_base_stale() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Stale rules.mp4");
    app.theme_set(&oid, &json!({ "widgets": [{ "id": "c", "type": "clock", "x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3 }] })).unwrap();
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    let stale = |app: &App| {
        let v = app.theme_get(&oid).unwrap();
        assert_eq!(v["current"], json!(true));
        v["baseStale"] == json!(true) || v["baseUnstored"] == json!(true)
    };
    assert!(!stale(&app), "just applied");

    // live-only changes never make it stale
    app.theme_set(&oid, &json!({ "widgets": [
        { "id": "c", "type": "clock", "x": 0.2, "y": 0.2, "w": 0.4, "h": 0.4, "props": { "seconds": true, "style": "analog" } },
        { "id": "s", "type": "sensor", "x": 0.5, "y": 0.5, "w": 0.2, "h": 0.2, "props": { "metric": "gpu.temp", "style": "gauge" } },
        { "id": "m", "type": "music", "x": 0.1, "y": 0.6, "w": 0.3, "h": 0.2 } ],
        "liveLayer": false, "handover": { "fadeIn": 2, "end": "instant", "musicIdle": "keep" }, "gap": 40, "guidesOffset": { "left": { "size": 30 } } })).unwrap();
    assert!(!stale(&app), "widgets, live layer, hand-over, gap, guide offsets");
    app.settings_update(&json!({ "editor": { "snap": false }, "weather": { "unit": "F" }, "screenGuides": { "left": { "size": 100 }, "right": { "size": 180 }, "curve": { "offset": 0, "left": 50, "right": 50 } } })).unwrap();
    app.weather_set_place(&json!({ "name": "Sydney", "lat": -33.8, "lon": 151.2, "tz": "Australia/Sydney" })).unwrap();
    assert!(!stale(&app), "snap, weather, global guides");
    app.theme_set(&oid, &json!({ "widgets": [] })).unwrap();
    assert!(!stale(&app), "no widgets at all");

    // picture inputs do
    app.settings_update(&json!({ "screenFit": { "top": 4, "left": 9 } })).unwrap();
    assert!(stale(&app), "screen fit");
    app.settings_update(&json!({ "screenFit": { "top": 0, "left": 0 } })).unwrap();
    assert!(!stale(&app), "back to what the base was made with");
    app.settings_update(&json!({ "screenFit": { "shiftX": 6 } })).unwrap();
    assert!(stale(&app), "the shift");
    app.settings_update(&json!({ "screenFit": { "shiftX": 0 } })).unwrap();
    app.settings_update(&json!({ "cornerRadius": 60 })).unwrap();
    assert!(stale(&app), "corner radius");
    app.settings_update(&json!({ "cornerRadius": 0 })).unwrap();
    assert!(!stale(&app));
    app.theme_set(&oid, &json!({ "corners": 40 })).unwrap();
    assert!(stale(&app), "the theme's own corner radius");
    app.theme_set(&oid, &json!({ "corners": null })).unwrap();
    assert!(!stale(&app));
    let mask = app.app_state()["mask"].clone();
    let mut m2 = mask.clone();
    m2["band"]["px"] = json!(200);
    app.settings_update(&json!({ "mask": m2 })).unwrap();
    assert!(stale(&app), "the notch");
    app.settings_update(&json!({ "mask": mask })).unwrap();
    assert!(!stale(&app));
    app.theme_set(&oid, &json!({ "widgets": [{ "id": "t", "type": "text", "x": 0.1, "y": 0.1, "w": 0.3, "h": 0.2, "onBase": true, "props": { "text": "Baked" } }] })).unwrap();
    assert!(stale(&app), "an item baked into the base");
    app.theme_set(&oid, &json!({ "widgets": [{ "id": "t", "type": "text", "x": 0.1, "y": 0.1, "w": 0.3, "h": 0.2, "onBase": false, "props": { "text": "Baked" } }] })).unwrap();
    assert!(!stale(&app), "the same item, live only");
}

/// C2: the screen's free space is remembered when the app stored something; a clearly different value at a later check means
/// another program wrote to the screen. Only the read-only info answer is used.
#[test]
fn a_check_flags_screen_content_changed_outside_the_app() {
    let (_d, app, _rec, _g) = setup();
    let flagged = |app: &App| app.app_state()["screenChanged"].clone();
    // first check: nothing to compare with yet, the answer becomes the baseline
    app.device_check().unwrap();
    assert!(flagged(&app).is_null());
    // the same answer again, and a wobble inside the tolerance: nothing
    app.device_check().unwrap();
    app.device.fake_set_free(4880 - 40);
    app.device_check().unwrap();
    assert!(flagged(&app).is_null(), "{}", flagged(&app));
    // another program stored a big file: free space dropped a lot
    app.device.fake_set_free(300);
    app.device_check().unwrap();
    let c = flagged(&app);
    assert_eq!((c["wasFree"].as_f64().unwrap(), c["nowFree"].as_f64().unwrap()), (4880.0 * 512.0, 300.0 * 512.0));
    // checking again keeps the same open difference (the "was" stays what we stored)
    app.device.fake_set_free(200);
    app.device_check().unwrap();
    let c2 = flagged(&app);
    assert_eq!((c2["wasFree"].as_f64().unwrap(), c2["nowFree"].as_f64().unwrap()), (4880.0 * 512.0, 200.0 * 512.0));
    assert_eq!(c2["at"], c["at"]);
    // "It's fine": the screen as it is now becomes the baseline
    app.settings_update(&json!({ "screenChanged": "accept" })).unwrap();
    assert!(flagged(&app).is_null());
    app.device_check().unwrap();
    assert!(flagged(&app).is_null());
    // flagged again, then the app stores something itself: the final check of the store is the new baseline
    app.device.fake_set_free(4880);
    let info = app.device_check().unwrap();
    assert!(!flagged(&app).is_null());
    app.rebase_screen(Some(&info));
    assert!(flagged(&app).is_null());
    app.device_check().unwrap();
    assert!(flagged(&app).is_null());
    // a store whose final check failed drops the baseline; the next check starts over without a false alarm
    app.device.fake_set_free(4700);
    app.rebase_screen(None);
    app.device_check().unwrap();
    assert!(flagged(&app).is_null());
    // the settings file keeps both keys only while they mean something
    let raw = std::fs::read_to_string(app.settings.file()).unwrap();
    assert!(raw.contains("screenBaseline") && !raw.contains("screenChanged"));
}

#[test]
fn an_effect_theme_stores_a_recorded_loop_and_goes_stale_when_the_effect_changes() {
    if !crate::media::ffmpeg::locate().exists() {
        return; // the loop is recorded through ffmpeg
    }
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Effect theme.mp4");
    let fx = |color: &str| json!({ "background": { "kind": "effect", "effect": "drift", "speed": 3.0, "colors": [color, "#ffffff"] } });
    let run = |app: &App| {
        let r = app.theme_apply(&oid).unwrap();
        let j = wait_job(app, r["jobId"].as_str().unwrap());
        assert_eq!(j["state"], "done", "{j}");
        j["result"]["versionId"].as_str().unwrap().to_string()
    };
    app.theme_set(&oid, &fx("#7fd4ff")).unwrap();
    let v1 = run(&app);
    let (_, ver) = app.lib.find_version(&v1).unwrap();
    assert!(ver["effect"]["key"].is_string(), "the version remembers which effect it records: {ver}");
    // the recording is 3.33 s at the screen's 18 fps = 60 frames, the whole loop
    assert_eq!(ver["settings"]["length"].as_f64().unwrap().round(), 3.0);
    let v = app.theme_get(&oid).unwrap();
    assert_eq!(v["baseVersionId"], json!(v1));
    assert_eq!(v["baseStale"], json!(false), "just stored");
    assert!(app.theme_live_of(&oid, app.load_theme(&oid)).effect_stored, "live may hand over to it");
    // a live-only change keeps it; a changed effect makes Embedded stale and the live engine stops handing over to the old loop
    assert_eq!(app.theme_set(&oid, &json!({ "widgets": [{ "id": "c", "type": "clock", "x": 0.1, "y": 0.1, "w": 0.3, "h": 0.3 }], "background": fx("#7fd4ff")["background"] })).unwrap()["baseStale"], json!(false));
    let stale = app.theme_set(&oid, &fx("#ff6b6b")).unwrap();
    assert_eq!(stale["baseStale"], json!(true));
    assert!(!app.theme_live_of(&oid, app.load_theme(&oid)).effect_stored);
    // storing again makes a new loop; storing the same effect again reuses it
    let v2 = run(&app);
    assert_ne!(v1, v2);
    assert_eq!(app.theme_get(&oid).unwrap()["baseStale"], json!(false));
    assert_eq!(run(&app), v2);
    assert_eq!(app.lib.versions(&oid).len(), 2);
    // back to the picture: the stored loop is not its picture any more
    let back = app.theme_set(&oid, &json!({})).unwrap();
    assert_eq!(back["baseStale"], json!(true));
    // an old loop recording is cleaned up once a new one exists
    let n = std::fs::read_dir(app.theme_dir(&oid).join("effect")).map(|d| d.count()).unwrap_or(0);
    assert_eq!(n, 1, "only the current recording is kept");
}

/// Preview on the real screen: selecting a theme shows it, saves nothing, never marks Embedded stale; ending puts the screen back.
#[test]
fn previewing_a_theme_saves_nothing_and_ends_cleanly() {
    let (_d, app, _rec, gallery) = setup();
    app.live.set_no_device(true); // never open the real screen from a test
    put(&gallery.join("My clip.mp4"), b"fake video data");
    app.startup();
    let oid = app.originals_list()["originals"][0]["id"].as_str().unwrap().to_string();
    wait_until(|| app.original_get(&oid).unwrap()["assets"] == "ready");
    let settings_before = std::fs::read(app.settings.file()).unwrap();
    let theme_json = app.theme_dir(&oid).join("theme.json");
    let theme_before = std::fs::read(&theme_json).ok();
    let state_before = app.app_state()["live"]["settings"].clone();
    // preview, twice (another click), and the status says so
    app.preview_theme_on_screen(&oid).unwrap();
    app.preview_theme_on_screen(&oid).unwrap();
    assert!(app.live.previewing() && app.live.status()["previewing"] == json!(true));
    assert!(app.live.running());
    assert!(app.theme_view(&oid).unwrap()["baseStale"] == json!(false), "a preview never marks Embedded stale");
    // nothing written
    assert_eq!(std::fs::read(app.settings.file()).unwrap(), settings_before, "settings.json untouched");
    assert_eq!(std::fs::read(&theme_json).ok(), theme_before, "theme.json untouched");
    assert_eq!(app.app_state()["live"]["settings"], state_before, "the Live setting stays as it was");
    assert_eq!(app.preview_end_on_screen()["ended"], json!(true));
    assert!(!app.live.previewing() && !app.live.running(), "Live was off: the temporary stream is gone");
    assert_eq!(app.preview_end_on_screen()["ended"], json!(false));
    assert_eq!(std::fs::read(app.settings.file()).unwrap(), settings_before);
}

#[test]
fn an_identical_clip_is_not_sent_again_but_anything_doubtful_is() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Same twice.mp4");
    let apply = |app: &App| {
        let r = app.theme_apply(&oid).unwrap();
        let j = wait_job(app, r["jobId"].as_str().unwrap());
        assert_eq!(j["state"], "done", "{j}");
        j
    };
    let uploads = |app: &App| app.sched.activity(0).into_iter().filter(|j| j.kind == "send").count();
    let j1 = apply(&app);
    assert_eq!(j1["result"]["alreadyOnScreen"], json!(false), "the first store sends");
    let vid = j1["result"]["versionId"].as_str().unwrap().to_string();
    let slot = app.settings.load().slot("standby").cloned().unwrap();
    assert!(slot["hash"].is_string(), "the slot remembers the content hash: {slot}");
    // the same clip again, screen unchanged: finished at once, nothing sent
    let before = std::time::Instant::now();
    let j2 = apply(&app);
    assert_eq!(j2["result"]["alreadyOnScreen"], json!(true));
    assert_eq!(j2["result"]["versionId"], json!(vid));
    assert!(before.elapsed() < Duration::from_secs(3), "no upload happened: {:?}", before.elapsed());
    assert_eq!(j2["message"], json!("Already on the screen"));
    assert_eq!(app.live_theme_oid().as_deref(), Some(oid.as_str()));
    let _ = uploads(&app);
    // the screen's free space moved (something else wrote to it): send
    app.device.fake_set_free(4700);
    assert_eq!(apply(&app)["result"]["alreadyOnScreen"], json!(false), "a different free space means we cannot know: send");
    app.device.fake_set_free(4880);
    // after that store the baseline is the new free space; same again skips
    assert_eq!(apply(&app)["result"]["alreadyOnScreen"], json!(false), "free space differs from the new baseline (4700 vs 4880): send");
    assert_eq!(apply(&app)["result"]["alreadyOnScreen"], json!(true));
    // an open "changed outside" note, an incomplete record, or a missing baseline: send
    app.settings.update(|s| s["screenChanged"] = json!({ "was": 1, "now": 2, "blockSize": 512, "at": "x" })).unwrap();
    // (storing clears the open note, so check the guard itself)
    let h = slot["hash"].as_str().unwrap();
    assert!(app.already_on_screen(&vid, slot["size"].as_u64().unwrap(), h).is_none());
    app.settings.update(|s| { s.as_object_mut().unwrap().remove("screenChanged"); }).unwrap();
    assert!(app.already_on_screen(&vid, slot["size"].as_u64().unwrap(), h).is_some());
    assert!(app.already_on_screen(&vid, slot["size"].as_u64().unwrap() + 1, h).is_none(), "another size");
    assert!(app.already_on_screen(&vid, slot["size"].as_u64().unwrap(), "0000").is_none(), "another content");
    app.settings.patch_slot("standby", &json!({ "incomplete": true })).unwrap();
    assert!(app.already_on_screen(&vid, slot["size"].as_u64().unwrap(), h).is_none(), "an interrupted store is never trusted");
    app.settings.patch_slot("standby", &json!({ "incomplete": false })).unwrap();
    app.settings.update(|s| { s.as_object_mut().unwrap().remove("screenBaseline"); }).unwrap();
    assert!(app.already_on_screen(&vid, slot["size"].as_u64().unwrap(), h).is_none(), "no baseline: send");
}

/// C5: one loop budget. The screen's own free space (read at a check or before a store) lowers it, so a version is never made
/// bigger than the screen can take now, and the number the pages show is the number the device guard uses.
#[test]
fn the_loop_budget_follows_the_screens_real_room() {
    use crate::device::ops::loop_room;
    let (_d, app, _rec, _g) = setup();
    let budget = |a: &App| a.app_state()["budget"].as_f64().unwrap();
    let plain = budget(&app); // nothing measured yet: the configured total less the splash
    // the screen has a lot of room: the capacity estimate stays the limit
    app.device.fake_set_free(23_000);
    app.device_check().unwrap();
    assert_eq!(budget(&app), plain, "plenty of room: unchanged");
    assert_eq!(app.app_state()["budgetRoomLimited"], json!(false));
    // the screen reports much less free space than expected (it changed behind our back)
    app.device.fake_set_free(15_000);
    app.device_check().unwrap();
    let info = app.device.read_info().unwrap();
    let want = loop_room(&info, None, 0).unwrap() as f64;
    assert!(want < plain, "{want} < {plain}");
    assert_eq!(budget(&app), want, "the budget is exactly what the device guard will allow");
    assert_eq!(app.app_state()["budgetRoomLimited"], json!(true));
    // and the outside change is flagged by the same check
    assert!(!app.app_state()["screenChanged"].is_null());
    // the page's other numbers follow: a version bigger than the room is marked too big
    // (the store pre-check says so in words too)
    let st = app.settings.load();
    assert_eq!(crate::core::settings::budgets(&st, None).loop_bytes, want);
    // refreshed before a store: the screen got space back (the user freed it), the budget goes back up
    app.device.fake_set_free(21_000);
    app.refresh_loop_room();
    assert_eq!(budget(&app), plain.min(loop_room(&app.device.read_info().unwrap(), None, 0).unwrap() as f64));
    // a store without a final answer drops the record, the capacity estimate stands
    app.rebase_screen(None);
    assert_eq!(budget(&app), plain);
}

/// C5: storing converts against what the screen can take NOW; a version made earlier that no longer fits is re-made instead of
/// failing at the send step.
#[test]
fn storing_makes_the_picture_fit_the_screens_real_room() {
    use crate::device::ops::loop_room;
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Tight.mp4");
    app.device.fake_set_free(23_000); // plenty first
    app.device_check().unwrap();
    let store = |app: &App| {
        let r = app.theme_apply(&oid).unwrap();
        let j = wait_job(app, r["jobId"].as_str().unwrap());
        assert_eq!(j["state"], "done", "{j}");
        app.theme_view(&oid).unwrap()["baseVersionId"].as_str().unwrap().to_string()
    };
    let size_of = |app: &App, vid: &str| app.lib.find_version(vid).unwrap().1["result"]["size"].as_f64().unwrap();
    let v1 = store(&app);
    let s1 = size_of(&app, &v1);
    // the screen now has much less room (something else is on it): the next store, with no check in between, reads the room first
    app.device.fake_set_free(100);
    let room = loop_room(&app.device.read_info().unwrap(), app.settings.load().slot("standby").and_then(|s| s["size"].as_u64()), 0).unwrap() as f64;
    let v2 = store(&app);
    assert_ne!(v1, v2, "the earlier picture no longer fits, so it was made again");
    assert!(size_of(&app, &v2) <= room, "{} <= {room}", size_of(&app, &v2));
    assert!(size_of(&app, &v2) < s1);
}
