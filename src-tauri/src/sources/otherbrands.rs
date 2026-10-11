//! Source: themes other brands publish on the same theme platform for the same 2400x1080 screen.
//! Brand ids come from the vendor app's configuration (`visibleBrandIds`); a 2026-10-11 check found 2400x1080 themes only
//! for brand 2 (31 themes; used by the HT DIGITAL, COUGAR Digital and majesty Display apps). Which brands have themes is
//! re-checked at most once a day (one tiny request per brand).
use super::lovingcool::to_item;
use super::themes;
use super::*;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct OtherBrands;

const BRANDS: &[(u32, &str)] = &[
    (1, "Brand 1"),
    (2, "HT DIGITAL / COUGAR / majesty"),
    (6, "NEXTGAMING / YICO"),
    (9, "COUGAR InfoHub"),
    (10, "Deep Colors Studio"),
    (11, "SAMA / ARDOR GAMING / EVOLABS"),
    (12, "ASA"),
    (13, "YGT"),
    (14, "SAMA DIGITAL / SAMZHE"),
    (15, "EVOLVEO / TECNOMALL"),
];
const DAY_MS: u64 = 24 * 3600 * 1000;

#[derive(Serialize, Deserialize, Clone)]
struct BrandCount {
    id: u32,
    total: u64,
}
#[derive(Serialize, Deserialize)]
struct Counts {
    at: u64,
    brands: Vec<BrandCount>,
}
fn brand_name(id: u32) -> &'static str {
    BRANDS.iter().find(|b| b.0 == id).map(|b| b.1).unwrap_or("Other brand")
}
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn brands_with_themes(ctx: &SourceCtx, refresh: bool) -> Vec<BrandCount> {
    let file = ctx.data_dir.join("themes-brands.json");
    if let Some(c) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<Counts>(&b).ok()) {
        if !refresh && now_ms().saturating_sub(c.at) < DAY_MS {
            return c.brands;
        }
    }
    let mut brands = vec![];
    for (id, _) in BRANDS {
        if let Ok(r) = themes::list(ctx, 1, *id, themes::PAGE_SIZE, refresh) {
            if r.total > 0 {
                brands.push(BrandCount { id: *id, total: r.total });
            }
        }
    }
    let _ = std::fs::create_dir_all(&ctx.data_dir);
    if let Ok(b) = serde_json::to_vec(&Counts { at: now_ms(), brands: brands.clone() }) {
        let _ = std::fs::write(&file, b);
    }
    brands
}

impl SourceProvider for OtherBrands {
    fn id(&self) -> &str {
        "otherbrands"
    }
    fn name(&self) -> &str {
        "Other brands (same platform)"
    }
    fn description(&self) -> &str {
        "Themes other cooler brands publish on the same theme platform for this screen size. Made for their screens; most look fine on yours."
    }
    fn kind(&self) -> Kind {
        Kind::Online
    }
    fn detect(&self, _ctx: &SourceCtx) -> Detect {
        Detect::ok("Online library (loaded when you browse it)")
    }
    /// Pages run through the brands one after another. Item ids are `<brand>:<themeId>`.
    fn list(&self, ctx: &SourceCtx, page: u32, query: Option<&str>, refresh: bool) -> Result<ListPage> {
        let brands = brands_with_themes(ctx, refresh);
        let mut p = page.max(1) as u64;
        let ps = themes::PAGE_SIZE as u64;
        let total: u64 = brands.iter().map(|b| b.total).sum();
        for (i, b) in brands.iter().enumerate() {
            let pages = b.total.div_ceil(ps);
            if p <= pages {
                let r = themes::list(ctx, p as u32, b.id, themes::PAGE_SIZE, refresh)?;
                let last = i == brands.len() - 1 && p >= pages;
                let items = r.themes.iter().filter(|t| title_matches(&t.name, query))
                    .map(|t| to_item(t, format!("{}:{}", b.id, t.theme_id), Some(brand_name(b.id).into()))).collect();
                return Ok(ListPage { items, total, has_more: !last });
            }
            p -= pages;
        }
        Ok(ListPage::default())
    }
    fn add(&self, ctx: &SourceCtx, item_id: &str, _req: &AddRequest, progress: Progress, cancel: &Cancel) -> Result<AddResult> {
        let (brand, theme_id) = item_id.split_once(':').ok_or_else(|| SourceError("Unknown theme.".into()))?;
        let brand: u32 = brand.parse().map_err(|_| SourceError("Unknown theme.".into()))?;
        let t = themes::find(ctx, theme_id, brand, themes::PAGE_SIZE)?;
        let file = themes::download(ctx, &t, progress, cancel)?;
        let name = t.file.as_ref().and_then(|f| f.name.clone()).unwrap_or_else(|| format!("{}.ht", t.theme_id));
        Ok(AddResult { file, name: safe_name(&name), temp: true })
    }
}
