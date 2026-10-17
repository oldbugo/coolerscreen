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
    assert!(app.current_theme_oid().is_none());
    let r = app.theme_apply(&oid).unwrap();
    let jid = r["jobId"].as_str().unwrap().to_string();
    let mut seen = std::collections::BTreeSet::new();
    let j = loop {
        let j = app.job_get(&jid).unwrap();
        seen.insert(j["message"].as_str().unwrap_or("").to_string());
        if ["done", "failed", "cancelled"].contains(&j["state"].as_str().unwrap()) {
            break j;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(j["state"], "done", "{j}");
    assert!(seen.iter().any(|m| m.starts_with("Step 1 of 3") || m.starts_with("Step 2 of 3")), "steps are reported: {seen:?}");
    let vid = j["result"]["versionId"].as_str().unwrap().to_string();
    assert_eq!(app.current_theme_oid().as_deref(), Some(oid.as_str()));
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
    assert!(app.current_theme_oid().is_none());
    assert!(app.theme_get(&oid).unwrap()["theme"]["baseVersionId"].is_null());
    assert!(app.lib.versions(&oid).is_empty(), "no half-made base is left behind");
}

#[test]
fn deleting_the_current_theme_clears_it() {
    let (_d, app, _rec, gallery) = setup();
    let oid = one_original(&app, &gallery, "Gone.mp4");
    let r = app.theme_apply(&oid).unwrap();
    assert_eq!(wait_job(&app, r["jobId"].as_str().unwrap())["state"], "done");
    assert_eq!(app.current_theme_oid().as_deref(), Some(oid.as_str()));
    app.original_delete(&oid).unwrap();
    assert!(app.current_theme_oid().is_none());
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
    assert_eq!(app.current_theme_oid().as_deref(), Some(a.as_str()));
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
    assert_eq!(app.current_theme_oid().as_deref(), Some(a.as_str()));
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
