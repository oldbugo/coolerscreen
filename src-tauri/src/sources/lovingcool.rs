//! Source: Lovingcool's online theme library (see `themes` for the API).
use super::themes::{self, Theme};
use super::*;

pub struct Lovingcool;

pub(crate) fn to_item(t: &Theme, id: String, badge: Option<String>) -> Item {
    Item { id, title: t.name.clone(), thumb: t.cover.clone(), type_: "theme".into(), action: Action::Add, note: String::new(), key: t.theme_id.clone(), badge }
}

impl SourceProvider for Lovingcool {
    fn id(&self) -> &str {
        "lovingcool"
    }
    fn name(&self) -> &str {
        "Lovingcool themes"
    }
    fn description(&self) -> &str {
        "Lovingcool's free online theme library for this screen (the themes the vendor app offers)."
    }
    fn kind(&self) -> Kind {
        Kind::Online
    }
    fn detect(&self, _ctx: &SourceCtx) -> Detect {
        Detect::ok("Online library (loaded when you browse it)")
    }
    fn list(&self, ctx: &SourceCtx, page: u32, query: Option<&str>, refresh: bool) -> Result<ListPage> {
        let r = themes::list(ctx, page, themes::LOVINGCOOL, themes::PAGE_SIZE, refresh)?;
        let pages = r.total.div_ceil(themes::PAGE_SIZE as u64);
        let items = r.themes.iter().filter(|t| title_matches(&t.name, query)).map(|t| to_item(t, t.theme_id.clone(), None)).collect();
        Ok(ListPage { items, total: r.total, has_more: (page as u64) < pages })
    }
    fn add(&self, ctx: &SourceCtx, item_id: &str, _req: &AddRequest, progress: Progress, cancel: &Cancel) -> Result<AddResult> {
        let t = themes::find(ctx, item_id, themes::LOVINGCOOL, themes::PAGE_SIZE)?;
        let file = themes::download(ctx, &t, progress, cancel)?;
        let name = t.file.as_ref().and_then(|f| f.name.clone()).unwrap_or_else(|| format!("{}.ht", t.theme_id));
        Ok(AddResult { file, name: safe_name(&name), temp: true })
    }
}
