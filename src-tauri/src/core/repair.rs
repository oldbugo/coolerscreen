//! Library repair: two library items for the same gallery file (a returning file that was imported again as a new item while the
//! old one was marked missing). The duplicates are merged into one item: versions, theme.json and its pictures move over, the
//! loop slot and the current theme are pointed at the one that stays, and the emptied record goes to the Recycle Bin.
//! `repair_duplicates(false)` only describes what it would do.
use super::app::App;
use super::error::CoreResult;
use super::gallery::same_path;
use super::js::{self, truthy};
use super::library::is_id;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn modified(p: &Path) -> std::time::SystemTime {
    std::fs::metadata(p).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH)
}

/// Move a file or folder (rename, or copy then remove across drives).
fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            move_path(&e.path(), &to.join(e.file_name()))?;
        }
        std::fs::remove_dir(from)
    } else {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)
    }
}

impl App {
    /// The duplicates found and (with `apply`) merged; the emptied records go to the Recycle Bin.
    pub fn repair_duplicates(&self, apply: bool) -> Vec<Value> {
        self.repair_duplicates_with(apply, &super::recycle::recycle)
    }

    /// As `repair_duplicates`, with the way the emptied record is discarded given (tests do not use the real bin).
    pub fn repair_duplicates_with(&self, apply: bool, discard: &dyn Fn(&Path) -> CoreResult<()>) -> Vec<Value> {
        let originals = self.lib.list_originals();
        let mut groups: Vec<Vec<&Value>> = vec![];
        for o in &originals {
            let Some(f) = o["file"].as_str().filter(|f| !f.is_empty()) else { continue };
            match groups.iter_mut().find(|g| same_path(g[0]["file"].as_str().unwrap_or(""), Path::new(f))) {
                Some(g) => g.push(o),
                None => groups.push(vec![o]),
            }
        }
        let mut plans = vec![];
        for g in groups.into_iter().filter(|g| g.len() > 1 && g.iter().any(|o| truthy(o.get("missing"))) && g.iter().any(|o| !truthy(o.get("missing")))) {
            plans.push(self.merge_group(&g, apply, discard));
        }
        plans
    }

    fn merge_group(&self, g: &[&Value], apply: bool, discard: &dyn Fn(&Path) -> CoreResult<()>) -> Value {
        let id = |o: &Value| o["id"].as_str().unwrap_or("").to_string();
        let s = self.settings.load();
        let standby = s.slot("standby").cloned().unwrap_or(Value::Null);
        let slot_owner = standby["versionId"].as_str().and_then(|v| self.lib.find_version(v)).map(|(o, _)| o);
        let referenced = |o: &Value| {
            let i = id(o);
            standby["originalId"].as_str() == Some(i.as_str()) || standby["themeId"].as_str() == Some(i.as_str()) || slot_owner.as_deref() == Some(i.as_str())
        };
        let richness = |o: &Value| {
            let i = id(o);
            (self.lib.versions(&i).len(), self.lib.o_dir(&i).join("theme.json").is_file())
        };
        // keeper: what the screen refers to, else the one with more versions / a theme.json, else the live one, else the oldest
        let keep = g
            .iter()
            .max_by(|a, b| {
                referenced(a)
                    .cmp(&referenced(b))
                    .then(richness(a).cmp(&richness(b)))
                    .then((!truthy(a.get("missing"))).cmp(&!truthy(b.get("missing"))))
                    .then(b["created"].as_str().unwrap_or("").cmp(a["created"].as_str().unwrap_or("")))
            })
            .copied()
            .unwrap();
        let kid = id(keep);
        let others: Vec<&Value> = g.iter().copied().filter(|o| id(o) != kid).collect();
        let mut steps: Vec<String> = vec![];
        if truthy(keep.get("missing")) {
            steps.push(format!("{kid}: no longer marked missing (the file is there)"));
        }
        let before = json!({
            "keep": { "id": kid, "name": keep["name"], "missing": keep["missing"], "referencedByScreen": referenced(keep), "versions": self.lib.versions(&kid).len() },
            "duplicates": others.iter().map(|d| json!({ "id": id(d), "name": d["name"], "missing": d["missing"], "versions": self.lib.versions(&id(d)).len(), "hasTheme": self.lib.o_dir(&id(d)).join("theme.json").is_file() })).collect::<Vec<_>>(),
        });
        for d in &others {
            let did = id(d);
            let ver = self.lib.versions(&did);
            let (dt, kt) = (self.lib.o_dir(&did).join("theme.json"), self.lib.o_dir(&kid).join("theme.json"));
            let theme = match (dt.is_file(), kt.is_file()) {
                (false, _) => "no theme.json to move",
                (true, false) => "theme.json moves over",
                (true, true) if modified(&dt) > modified(&kt) => "the duplicate's theme.json is newer and is kept; the other becomes theme.prev.json",
                (true, true) => "the kept item's theme.json is newer and stays; the duplicate's becomes theme.prev.json",
            };
            let name = d["name"].as_str().unwrap_or("");
            let miss = if truthy(d.get("missing")) { ", missing" } else { "" };
            steps.push(format!("{did} ({name}{miss}): {} version(s) move to {kid}; {theme}; its record goes to the Recycle Bin", ver.len()));
            if apply {
                self.merge_into(&kid, &did, discard);
            }
        }
        if apply {
            let dups: Vec<String> = others.iter().map(|d| id(d)).collect();
            self.repoint(&kid, &dups);
            if truthy(keep.get("missing")) {
                let _ = self.lib.patch_original(&kid, &json!({ "missing": null }));
            }
            self.lib.bump();
            for st in &steps {
                crate::device::log(&format!("Library repair: {st}"));
                eprintln!("Library repair: {st}");
            }
        }
        let mut out = before;
        out["file"] = keep["file"].clone();
        out["steps"] = json!(steps);
        out["applied"] = json!(apply);
        out
    }

    /// Everything of `did` into `kid`, then `did`'s record out of the library.
    fn merge_into(&self, kid: &str, did: &str, discard: &dyn Fn(&Path) -> CoreResult<()>) {
        let (kdir, ddir) = (self.lib.o_dir(kid), self.lib.o_dir(did));
        // versions (renumbered after the kept item's own)
        let mut n = self.lib.versions(kid).iter().filter_map(|v| v["n"].as_f64()).fold(0.0, f64::max);
        let mut moving = self.lib.versions(did);
        moving.sort_by(|a, b| a["n"].as_f64().partial_cmp(&b["n"].as_f64()).unwrap_or(std::cmp::Ordering::Equal));
        for v in moving {
            let Some(vid) = v["id"].as_str().filter(|i| is_id(i)).map(String::from) else { continue };
            let _ = std::fs::create_dir_all(self.lib.v_root(kid));
            let to = self.lib.v_dir(kid, &vid);
            if to.exists() || move_path(&self.lib.v_dir(did, &vid), &to).is_err() {
                continue;
            }
            n += 1.0;
            let _ = self.lib.patch_version(&vid, &json!({ "originalId": kid, "n": js::num(n) }));
        }
        // theme.json and its pictures
        let (dt, kt) = (ddir.join("theme.json"), kdir.join("theme.json"));
        let mut d_wins = false;
        if dt.is_file() {
            if kt.is_file() {
                d_wins = modified(&dt) > modified(&kt);
                let loser = if d_wins { &kt } else { &dt };
                let mut prev = kdir.join("theme.prev.json");
                let mut k = 2;
                while prev.exists() {
                    prev = kdir.join(format!("theme.prev-{k}.json"));
                    k += 1;
                }
                let _ = std::fs::write(&prev, std::fs::read(loser).unwrap_or_default());
                if d_wins {
                    let _ = std::fs::copy(&dt, &kt);
                }
            } else {
                let _ = move_path(&dt, &kt);
            }
        }
        let da = ddir.join("assets");
        if da.is_dir() {
            let ka = kdir.join("assets");
            let _ = std::fs::create_dir_all(&ka);
            for e in std::fs::read_dir(&da).into_iter().flatten().flatten() {
                let to = ka.join(e.file_name());
                if to.exists() {
                    if !d_wins {
                        continue;
                    }
                    // the winning theme refers to the duplicate's picture: keep the other one beside it
                    let mut keep = to.clone().into_os_string();
                    keep.push(".prev");
                    let _ = std::fs::rename(&to, PathBuf::from(keep));
                }
                let _ = move_path(&e.path(), &to);
            }
        }
        // tuning drafts and loop searches the kept item does not have
        if let (Some(d), Some(k)) = (self.lib.read_original(did), self.lib.read_original(kid)) {
            let mut patch = json!({});
            for key in ["draft", "loops"] {
                if k.get(key).is_none_or(Value::is_null) && d.get(key).is_some_and(|v| !v.is_null()) {
                    patch[key] = d[key].clone();
                }
            }
            if patch.as_object().is_some_and(|o| !o.is_empty()) {
                let _ = self.lib.patch_original(kid, &patch);
            }
        }
        // the record stops being an item at once; what is left of its folder goes to the Recycle Bin
        let _ = std::fs::rename(ddir.join("original.json"), ddir.join("original.json.merged"));
        self.lib.bump();
        if let Err(e) = discard(&ddir) {
            eprintln!("Library repair: could not recycle {}: {}", ddir.display(), e.message);
        }
    }

    /// References to merged duplicates: the loop slot's item and theme, and the Live background clip.
    fn repoint(&self, kid: &str, dups: &[String]) {
        let is_dup = |v: &Value| v.as_str().is_some_and(|x| dups.iter().any(|d| d == x));
        let _ = self.settings.update(|s| {
            for slot in ["standby", "startup"] {
                for key in ["originalId", "themeId"] {
                    if is_dup(&s["slots"][slot][key]) {
                        s["slots"][slot][key] = json!(kid);
                    }
                }
            }
            if is_dup(&s["liveTheme"]) {
                s["liveTheme"] = json!(kid);
            }
            if is_dup(&s["live"]["background"]) {
                s["live"]["background"] = json!(kid);
            }
        });
    }
}

static REPAIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

impl App {
    /// Themes whose chosen base is a version bigger than the screen can hold (a failed upload once left one): the base is cleared,
    /// or becomes the version in the loop slot when that one is within budget and belongs to the same item (or the same file).
    pub fn repair_theme_bases(&self, apply: bool) -> Vec<Value> {
        let budget = self.lib.context().budget;
        let standby = self.settings.load().slot("standby").cloned().unwrap_or(Value::Null);
        let slot_version = standby["versionId"].as_str().and_then(|v| self.lib.find_version(v));
        let mut plans = vec![];
        for o in self.lib.list_originals() {
            let oid = o["id"].as_str().unwrap_or("").to_string();
            let tpath = self.lib.o_dir(&oid).join("theme.json");
            let Some(mut theme) = js::read_json(&tpath).filter(Value::is_object) else { continue };
            let Some(bid) = theme["baseVersionId"].as_str().map(String::from) else { continue };
            let Some((_, bv)) = self.lib.find_version(&bid) else { continue };
            let size = bv["result"]["size"].as_f64().unwrap_or(0.0);
            if size <= budget {
                continue;
            }
            // the loop slot's version, when it is this item's own or another item for the same file
            let fallback = slot_version.as_ref().filter(|(so, sv)| {
                sv["status"] == "ready"
                    && sv["result"]["size"].as_f64().is_some_and(|x| x <= budget)
                    && (so == &oid
                        || self.lib.read_original(so).is_some_and(|s| {
                            matches!((s["file"].as_str(), o["file"].as_str()), (Some(a), Some(b)) if same_path(a, Path::new(b)))
                        }))
            });
            let to = fallback.map(|(_, sv)| sv["id"].clone()).unwrap_or(Value::Null);
            let step = format!(
                "{oid} ({}): base {bid} is {:.2} MB, over the {:.2} MB the screen can hold; base becomes {}",
                o["name"].as_str().unwrap_or(""),
                size / 1e6,
                budget / 1e6,
                to.as_str().unwrap_or("none (made again when the theme is stored as Embedded)")
            );
            if apply {
                theme["baseVersionId"] = to.clone();
                if js::write_json(&tpath, &theme).is_ok() {
                    crate::device::log(&format!("Library repair: {step}"));
                    eprintln!("Library repair: {step}");
                    self.lib.bump();
                }
            }
            plans.push(json!({ "originalId": oid, "name": o["name"], "from": bid, "sizeBytes": size, "budgetBytes": budget, "to": to, "step": step, "applied": apply }));
        }
        plans
    }

    /// Everything the startup repair does, in order: duplicate items first, then bad theme bases. Safe to call more than once.
    pub fn repair_library(&self) -> Value {
        self.repair_library_with(true, &super::recycle::recycle)
    }

    pub fn repair_library_with(&self, apply: bool, discard: &dyn Fn(&Path) -> CoreResult<()>) -> Value {
        let _g = REPAIR_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let duplicates = self.repair_duplicates_with(apply, discard);
        let bases = self.repair_theme_bases(apply);
        // a capacity record above what the screen can really hold (a failed upload recorded 13.39 MB) becomes the conservative value
        let cap = self.settings.load().capacity();
        if let Some((bytes, _)) = cap.filter(|(b, _)| *b > super::settings::MAX_CAPACITY) {
            if apply {
                let _ = self.settings.update(|s| s["capacity"] = json!({ "bytes": js::num(super::settings::MAX_CAPACITY), "at": js::iso_now() }));
                crate::device::log(&format!("Library repair: capacity record {bytes} bytes was more than the screen can hold; now {}", super::settings::MAX_CAPACITY));
            }
        }
        if apply {
            // always one line, so device.log shows that the check ran
            crate::device::log(&format!("Library repair: checked the library ({} duplicate group(s), {} theme base(s) fixed)", duplicates.len(), bases.len()));
        }
        json!({ "duplicates": duplicates, "themeBases": bases })
    }
}
