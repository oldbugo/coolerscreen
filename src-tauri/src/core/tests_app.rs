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
fn estimate_is_paused_while_the_screen_is_busy() {
    let (_d, app, _rec, gallery) = setup();
    put(&gallery.join("e.mp4"), b"eeee");
    app.startup();
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
fn settings_update_keeps_corners() {
    let (_d, app, _rec, _g) = setup();
    app.settings.update(|s| s["mask"]["corners"] = json!({ "on": true, "r": 50 })).unwrap();
    let st = app.settings_update(&json!({ "mask": { "mode": "band", "band": { "edge": "right", "px": 40 }, "corners": { "on": false, "r": 1 } }, "maxSeconds": 20 })).unwrap();
    assert_eq!(st["mask"]["corners"], json!({ "on": true, "r": 50 }));
    assert_eq!(st["notchKey"], "band:right:48");
    assert_eq!(st["maxSeconds"], json!(20));
}
