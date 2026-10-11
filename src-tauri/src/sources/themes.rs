//! The vendor platform's public online theme catalogue (the list LOVINGCOOL MONITOR shows), used by the
//! "Lovingcool themes" and "Other brands" sources.
//!
//!   GET https://www.htssl.com/api/ht/theme/getAllThemes
//!       ?current=<page, 1-based>&pageSize=<n>&fullScreen=true&deviceWidth=2400&deviceHeight=1080&rotate=0
//!       &visibleBrandIds[]=<brand>        (7 = LOVINGCOOL; no key, signature or login)
//!   -> { code: 200, data: { total, list: [{ themeId, name, deviceWidth, deviceHeight, createdAt,
//!        packageFile: { url, fileName }, coverPictureFile: { url } }] } }
//!
//! Theme packages (.ht) and covers are plain HTTPS files on theme.htssl.com. Lists are cached for a day; nothing is
//! downloaded unless the user asks.
use super::{err, http, Cancel, Progress, Result, SourceCtx, SourceError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub const API: &str = "https://www.htssl.com/api/ht/theme/getAllThemes";
pub const FILE_HOST: &str = "theme.htssl.com";
pub const LOVINGCOOL: u32 = 7;
pub const SCREEN: (u32, u32) = (2400, 1080);
pub const PAGE_SIZE: u32 = 24;
const CACHE_MS: u64 = 24 * 3600 * 1000;
const MAX_THEME_BYTES: u64 = 300_000_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeFile {
    pub url: String,
    pub name: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Theme {
    pub theme_id: String,
    pub name: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub brand: u32,
    pub created: Option<String>,
    pub cover: Option<String>,
    pub file: Option<ThemeFile>,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ThemePage {
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub themes: Vec<Theme>,
}
#[derive(Serialize, Deserialize, Default)]
struct CachedPage {
    at: u64,
    data: ThemePage,
}
#[derive(Serialize, Deserialize, Default)]
struct Cache {
    pages: BTreeMap<String, CachedPage>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
fn cache_file(ctx: &SourceCtx, brand: u32) -> PathBuf {
    ctx.data_dir.join(if brand == LOVINGCOOL { "themes-cache.json".to_string() } else { format!("themes-cache-{brand}.json") })
}
fn read_cache(ctx: &SourceCtx, brand: u32) -> Cache {
    std::fs::read(cache_file(ctx, brand)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}
fn write_cache(ctx: &SourceCtx, brand: u32, c: &Cache) {
    let _ = std::fs::create_dir_all(&ctx.data_dir);
    if let Ok(b) = serde_json::to_vec(c) {
        let _ = std::fs::write(cache_file(ctx, brand), b);
    }
}

fn s(v: &Value) -> Option<String> {
    v.as_str().map(str::to_string)
}

/// Parse one API answer; keeps only themes with a package and (when stated) the 2400x1080 size.
pub fn parse_list(r: &Value, page: u32, page_size: u32, brand: u32) -> Result<ThemePage> {
    let ok = r.get("code").and_then(Value::as_i64) == Some(200);
    let list = r.pointer("/data/list").and_then(Value::as_array);
    let (true, Some(list)) = (ok, list) else { return err("The theme library sent something unexpected.") };
    let total = r.pointer("/data/total").and_then(|v| v.as_u64().or_else(|| v.as_str().and_then(|x| x.parse().ok()))).unwrap_or(0);
    let themes = list
        .iter()
        .filter_map(|t| {
            let id = t.get("themeId").map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))?;
            let file_url = t.pointer("/packageFile/url").and_then(Value::as_str).filter(|u| !u.is_empty())?.to_string();
            let w = t.get("deviceWidth").and_then(Value::as_u64).map(|x| x as u32);
            let h = t.get("deviceHeight").and_then(Value::as_u64).map(|x| x as u32);
            if let (Some(w), Some(h)) = (w, h) {
                if (w, h) != SCREEN {
                    return None;
                }
            }
            Some(Theme {
                name: t.get("name").and_then(s).unwrap_or_else(|| id.clone()),
                theme_id: id,
                width: w,
                height: h,
                brand,
                created: t.get("createdAt").and_then(s),
                cover: t.pointer("/coverPictureFile/url").and_then(s),
                file: Some(ThemeFile { url: file_url, name: t.pointer("/packageFile/fileName").and_then(s) }),
            })
        })
        .collect();
    Ok(ThemePage { total, page, page_size, themes })
}

/// One page of a brand's themes for this screen, cached for a day unless `refresh`.
pub fn list(ctx: &SourceCtx, page: u32, brand: u32, page_size: u32, refresh: bool) -> Result<ThemePage> {
    let page = page.max(1);
    let key = format!("{page}x{page_size}");
    let mut cache = if refresh { Cache::default() } else { read_cache(ctx, brand) };
    if let Some(hit) = cache.pages.get(&key) {
        if now_ms().saturating_sub(hit.at) < CACHE_MS {
            return Ok(hit.data.clone());
        }
    }
    let mut u = reqwest::Url::parse(API).unwrap();
    u.query_pairs_mut()
        .append_pair("current", &page.to_string())
        .append_pair("pageSize", &page_size.to_string())
        .append_pair("fullScreen", "true")
        .append_pair("deviceWidth", &SCREEN.0.to_string())
        .append_pair("deviceHeight", &SCREEN.1.to_string())
        .append_pair("rotate", "0")
        .append_pair("visibleBrandIds[]", &brand.to_string());
    let r = http::get_json(u, "the theme library")?;
    let data = parse_list(&r, page, page_size, brand)?;
    cache.pages.insert(key, CachedPage { at: now_ms(), data: data.clone() });
    write_cache(ctx, brand, &cache);
    Ok(data)
}

pub fn find_cached(ctx: &SourceCtx, theme_id: &str, brand: u32) -> Option<Theme> {
    read_cache(ctx, brand).pages.values().flat_map(|p| p.data.themes.iter()).find(|t| t.theme_id == theme_id).cloned()
}

/// Cached copy, else walk the first pages of the list looking for it.
pub fn find(ctx: &SourceCtx, theme_id: &str, brand: u32, page_size: u32) -> Result<Theme> {
    if let Some(t) = find_cached(ctx, theme_id, brand) {
        return Ok(t);
    }
    for p in 1..=10 {
        let r = list(ctx, p, brand, page_size, false)?;
        if let Some(t) = r.themes.into_iter().find(|t| t.theme_id == theme_id) {
            return Ok(t);
        }
        if (p as u64) * (page_size as u64) >= r.total {
            break;
        }
    }
    err("Unknown theme. Refresh the list and try again.")
}

/// Download a theme's .ht into `ctx.tmp_dir` (only from the vendor's theme file host, https, no redirects).
pub fn download(ctx: &SourceCtx, theme: &Theme, progress: Progress, cancel: &Cancel) -> Result<PathBuf> {
    let url = theme.file.as_ref().map(|f| f.url.as_str()).unwrap_or("");
    let u = reqwest::Url::parse(url).map_err(|_| SourceError("Unexpected download address for this theme.".into()))?;
    if u.scheme() != "https" || u.host_str() != Some(FILE_HOST) {
        return err("Unexpected download address for this theme.");
    }
    let safe: String = theme.theme_id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect();
    let target = ctx.tmp_dir.join(format!("{safe}.ht"));
    let resp = http::client(0)?.get(u).send().map_err(|e| SourceError(format!("Download failed: {e}")))?;
    if resp.status().as_u16() != 200 {
        return err(format!("Download failed ({}).", resp.status().as_u16()));
    }
    http::stream_to_file(resp, &target, MAX_THEME_BYTES, progress, cancel)?;
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_and_filters() {
        let v = serde_json::json!({"code":200,"data":{"total":3,"list":[
          {"themeId":"a1","name":"One","deviceWidth":2400,"deviceHeight":1080,"packageFile":{"url":"https://theme.htssl.com/x.ht","fileName":"x.ht"},"coverPictureFile":{"url":"https://theme.htssl.com/c.jpg"}},
          {"themeId":"a2","name":"Wrong size","deviceWidth":1920,"deviceHeight":480,"packageFile":{"url":"https://theme.htssl.com/y.ht"}},
          {"themeId":"a3","name":"No package"}]}});
        let p = parse_list(&v, 1, 24, 7).unwrap();
        assert_eq!(p.total, 3);
        assert_eq!(p.themes.len(), 1);
        assert_eq!(p.themes[0].theme_id, "a1");
        assert_eq!(p.themes[0].cover.as_deref(), Some("https://theme.htssl.com/c.jpg"));
        assert!(parse_list(&serde_json::json!({"code":500}), 1, 24, 7).is_err());
    }
    #[test]
    fn refuses_foreign_hosts() {
        let ctx = SourceCtx { options: Value::Null, tmp_dir: std::env::temp_dir(), data_dir: std::env::temp_dir(), sources_dir: std::env::temp_dir(), ffmpeg: "ffmpeg".into() };
        let t = Theme { theme_id: "x".into(), name: "x".into(), width: None, height: None, brand: 7, created: None, cover: None,
            file: Some(ThemeFile { url: "https://evil.example.com/x.ht".into(), name: None }) };
        assert!(download(&ctx, &t, &|_, _| {}, &Cancel::new()).is_err());
        let t2 = Theme { file: Some(ThemeFile { url: "http://theme.htssl.com/x.ht".into(), name: None }), ..t };
        assert!(download(&ctx, &t2, &|_, _| {}, &Cancel::new()).is_err());
    }
}
