//! Declarative JSON source plug-ins (CoolerScreen 2.x). The 1.2 JavaScript plug-ins cannot be loaded from Rust; instead a
//! plug-in is a `.json` file in `Documents\CoolerScreen\Sources` that describes a JSON web API: where the list is, how its
//! fields map to items, and where files are downloaded from. There is no code, no HTML parsing and no scraping: the list URL
//! must answer JSON, and downloads go through the same checks as "Add from link" (direct file, no web pages, <= 500 MB,
//! <= 5 redirects) and only to the hosts the file names. See docs/sources.md ("Version 2").
use super::*;
use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct ListSpec {
    url: String,
    items_path: String,
    total_path: Option<String>,
    page_size: Option<u32>,
    fields: BTreeMap<String, String>,
}
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadSpec {
    /// Template, e.g. `https://example.com/files/{id}.mp4`.
    url: Option<String>,
    /// Path inside the raw list item holding the file URL (the item is found again by re-listing the first pages).
    url_path: Option<String>,
}
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
struct Spec {
    format: u32,
    name: String,
    #[serde(default)]
    description: String,
    key_prefix: Option<String>,
    #[serde(default)]
    allowed_hosts: Vec<String>,
    list: ListSpec,
    thumb_url: Option<String>,
    download: DownloadSpec,
}

pub struct Declarative {
    id: String,
    spec: Spec,
    hosts: Vec<String>,
}

/// Follow a dotted path (`a.b.0.c`) through objects and arrays.
pub fn json_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = v;
    for seg in path.split('.').filter(|s| !s.is_empty()) {
        cur = match cur {
            Value::Array(a) => a.get(seg.parse::<usize>().ok()?)?,
            Value::Object(o) => o.get(seg)?,
            _ => return None,
        };
    }
    Some(cur)
}
fn as_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}
fn encode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}
/// Replace `{name}` placeholders (values are percent-encoded).
pub fn fill(template: &str, vars: &[(&str, String)]) -> String {
    let mut s = template.to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), &encode(v));
    }
    s
}

impl Declarative {
    pub fn load(file: &Path) -> Result<Declarative> {
        let txt = std::fs::read_to_string(file).map_err(|e| SourceError(format!("Cannot read the file: {e}")))?;
        let spec: Spec = serde_json::from_str(txt.trim_start_matches('\u{feff}')).map_err(|e| SourceError(format!("Not a valid source file: {e}")))?;
        Self::from_spec(plugin_id(file), spec)
    }
    fn from_spec(id: String, spec: Spec) -> Result<Declarative> {
        if spec.format != 1 {
            return err("Unsupported source format (this version reads \"format\": 1).");
        }
        if spec.name.trim().is_empty() {
            return err("The source needs a \"name\".");
        }
        for k in ["id", "title"] {
            if !spec.list.fields.contains_key(k) {
                return err(format!("list.fields needs \"{k}\"."));
            }
        }
        if spec.download.url.is_none() == spec.download.url_path.is_none() {
            return err("download needs exactly one of \"url\" or \"urlPath\".");
        }
        let list_url = reqwest::Url::parse(&spec.list.url.replace("{page}", "1").replace("{query}", "")).map_err(|_| SourceError("list.url is not a valid address.".into()))?;
        if list_url.scheme() != "https" {
            return err("list.url must start with https://");
        }
        let mut hosts: Vec<String> = spec.allowed_hosts.iter().map(|h| h.to_lowercase()).collect();
        if hosts.is_empty() {
            hosts.extend(list_url.host_str().map(str::to_lowercase));
        }
        if !hosts.iter().any(|h| Some(h.as_str()) == list_url.host_str().map(str::to_lowercase).as_deref()) {
            return err("allowedHosts must include the host of list.url.");
        }
        if hosts.iter().any(|h| h.contains('/') || h.contains(':') || h.contains('*') || h.is_empty()) {
            return err("allowedHosts must be plain host names (no scheme, port or wildcard).");
        }
        Ok(Declarative { id, spec, hosts })
    }

    fn host_ok(&self, url: &str) -> bool {
        reqwest::Url::parse(url).map_or(false, |u| u.scheme() == "https" && u.host_str().map_or(false, |h| self.hosts.contains(&h.to_lowercase())))
    }
    fn key_for(&self, id: &str) -> String {
        let prefix = self.spec.key_prefix.clone().unwrap_or_else(|| self.id.clone());
        let clean = |s: &str| -> String { s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(40).collect() };
        format!("{}-{}", clean(&prefix), clean(id))
    }

    /// Map a raw API answer to a page (separate from the network for tests).
    fn parse_list(&self, answer: &Value, page: u32, query: Option<&str>) -> Result<(ListPage, Vec<Value>)> {
        let Some(Value::Array(raw)) = json_path(answer, &self.spec.list.items_path) else { return err("The source answered in an unexpected shape (itemsPath).") };
        let f = &self.spec.list.fields;
        let get = |item: &Value, k: &str| f.get(k).and_then(|p| json_path(item, p)).and_then(as_text);
        let mut items = vec![];
        let mut kept = vec![];
        for r in raw {
            let (Some(id), Some(title)) = (get(r, "id"), get(r, "title")) else { continue };
            let thumb = get(r, "thumb")
                .or_else(|| self.spec.thumb_url.as_ref().map(|t| t.replace("{id}", &encode(&id))))
                .filter(|u| self.host_ok(u));
            let url_ok = self.spec.download.url.is_some() || self.spec.download.url_path.as_ref().and_then(|p| json_path(r, p)).and_then(as_text).map_or(false, |u| self.host_ok(&u));
            let (action, note) = if url_ok { (Action::Add, String::new()) } else { (Action::None, "No download address".to_string()) };
            items.push(Item { key: self.key_for(&id), id, title: title.chars().take(120).collect(), thumb, type_: get(r, "type").unwrap_or_else(|| "clip".into()), action, note, badge: None });
            kept.push(r.clone());
        }
        let total = self.spec.list.total_path.as_ref().and_then(|p| json_path(answer, p)).and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok())));
        let has_more = match (total, self.spec.list.page_size) {
            (Some(t), Some(ps)) if ps > 0 => (page as u64) * (ps as u64) < t,
            (Some(t), _) => (page as u64) * (raw.len() as u64) < t && !raw.is_empty(),
            (None, Some(ps)) => raw.len() as u32 >= ps,
            (None, None) => false,
        };
        let total = total.unwrap_or(items.len() as u64);
        // Only filter here when the address has no {query} (otherwise the server did).
        if !self.spec.list.url.contains("{query}") {
            let keep: Vec<bool> = items.iter().map(|i| title_matches(&i.title, query)).collect();
            let mut k = keep.iter();
            items.retain(|_| *k.next().unwrap());
            let mut k = keep.iter();
            kept.retain(|_| *k.next().unwrap());
        }
        Ok((ListPage { items, total, has_more }, kept))
    }

    fn fetch(&self, page: u32, query: Option<&str>) -> Result<(ListPage, Vec<Value>)> {
        let url = fill(&self.spec.list.url, &[("page", page.to_string()), ("query", query.unwrap_or("").to_string())]);
        let u = reqwest::Url::parse(&url).map_err(|_| SourceError("list.url is not a valid address.".into()))?;
        if !self.host_ok(u.as_str()) {
            return err("The list address is not on an allowed host.");
        }
        let answer = http::get_json(u, &format!("\"{}\"", self.spec.name))?;
        self.parse_list(&answer, page, query)
    }
}

impl SourceProvider for Declarative {
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.spec.name
    }
    fn description(&self) -> &str {
        &self.spec.description
    }
    fn kind(&self) -> Kind {
        Kind::Plugin
    }
    fn detect(&self, _ctx: &SourceCtx) -> Detect {
        Detect::ok(format!("Plug-in loaded ({})", self.hosts.join(", ")))
    }
    fn list(&self, _ctx: &SourceCtx, page: u32, query: Option<&str>, _refresh: bool) -> Result<ListPage> {
        Ok(self.fetch(page.max(1), query)?.0)
    }
    fn add(&self, ctx: &SourceCtx, item_id: &str, _req: &AddRequest, progress: Progress, cancel: &Cancel) -> Result<AddResult> {
        let url = if let Some(t) = &self.spec.download.url {
            fill(t, &[("id", item_id.to_string())])
        } else {
            let path = self.spec.download.url_path.as_ref().unwrap();
            let mut found = None;
            'pages: for p in 1..=10 {
                cancel.check()?;
                let (page, raw) = self.fetch(p, None)?;
                for (it, r) in page.items.iter().zip(raw.iter()) {
                    if it.id == item_id {
                        found = json_path(r, path).and_then(as_text);
                        break 'pages;
                    }
                }
                if !page.has_more {
                    break;
                }
            }
            found.ok_or_else(|| SourceError("Unknown item. Refresh the list and try again.".into()))?
        };
        if !self.host_ok(&url) {
            return err("This item's download address is not on an allowed host (https only).");
        }
        let (file, name) = link::download_file(&url, &ctx.tmp_dir, Some(&self.hosts), progress, cancel)?;
        let ext = file.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
        let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| item_id.to_string());
        Ok(AddResult { file, name: format!("{} [{}]{ext}", safe_name(&stem), self.key_for(item_id)), temp: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn spec(extra: &str) -> Declarative {
        let j = format!(
            r#"{{"format":1,"name":"T","list":{{"url":"https://example.com/api?page={{page}}&q={{query}}","itemsPath":"data.list","totalPath":"data.total","pageSize":2,
            "fields":{{"id":"id","title":"name","thumb":"img.url","type":"kind"}}}},"thumbUrl":"https://example.com/t/{{id}}.jpg",
            "download":{extra},"keyPrefix":"ex"}}"#
        );
        Declarative::from_spec("plugin-t".into(), serde_json::from_str(&j).unwrap()).unwrap()
    }
    #[test]
    fn maps_items() {
        let d = spec(r#"{"url":"https://example.com/f/{id}.mp4"}"#);
        let a = serde_json::json!({"data":{"total":3,"list":[
            {"id":7,"name":"Sea","img":{"url":"https://example.com/s.jpg"},"kind":"video"},
            {"id":"x y","name":"Hill","img":{"url":"https://evil.net/s.jpg"}},
            {"name":"no id"}]}});
        let (p, raw) = d.parse_list(&a, 1, None).unwrap();
        assert_eq!(p.items.len(), 2);
        assert_eq!(raw.len(), 2);
        assert_eq!(p.items[0].id, "7");
        assert_eq!(p.items[0].key, "ex-7");
        assert_eq!(p.items[0].thumb.as_deref(), Some("https://example.com/s.jpg"));
        assert_eq!(p.items[0].type_, "video");
        // a mapped thumbnail on a foreign host is dropped
        assert_eq!(p.items[1].thumb, None);
        assert_eq!(p.total, 3);
        assert!(p.has_more);
        assert_eq!(fill("https://e.com/?q={query}", &[("query", "a b&c".into())]), "https://e.com/?q=a%20b%26c");
    }
    #[test]
    fn url_path_and_hosts() {
        let d = spec(r#"{"urlPath":"file.url"}"#);
        let a = serde_json::json!({"data":{"list":[{"id":1,"name":"A","file":{"url":"https://example.com/a.mp4"}},{"id":2,"name":"B","file":{"url":"https://evil.net/b.mp4"}}]}});
        let (p, _) = d.parse_list(&a, 1, None).unwrap();
        assert_eq!(p.items[0].action, Action::Add);
        assert_eq!(p.items[1].action, Action::None);
        assert!(d.host_ok("https://example.com/x") && !d.host_ok("http://example.com/x") && !d.host_ok("https://sub.example.com/x"));
    }
    #[test]
    fn rejects_bad_files() {
        let bad = |s: &str| Declarative::from_spec("p".into(), serde_json::from_str(s).unwrap()).is_err();
        let base = |list_url: &str, hosts: &str, dl: &str| format!(r#"{{"format":1,"name":"x","allowedHosts":{hosts},"list":{{"url":"{list_url}","itemsPath":"a","fields":{{"id":"i","title":"t"}}}},"download":{dl}}}"#);
        assert!(bad(&base("http://example.com/a", "[]", r#"{"url":"https://example.com/{id}"}"#)));
        assert!(bad(&base("https://example.com/a", r#"["other.com"]"#, r#"{"url":"https://example.com/{id}"}"#)));
        assert!(bad(&base("https://example.com/a", r#"["*.com"]"#, r#"{"url":"https://example.com/{id}"}"#)));
        assert!(bad(&base("https://example.com/a", "[]", r#"{}"#)));
        assert!(!bad(&base("https://example.com/a", "[]", r#"{"url":"https://example.com/{id}"}"#)));
    }
    #[test]
    fn paths() {
        let v = serde_json::json!({"a":[{"b":5}]});
        assert_eq!(json_path(&v, "a.0.b").and_then(as_text).as_deref(), Some("5"));
        assert!(json_path(&v, "a.1.b").is_none());
    }
}
