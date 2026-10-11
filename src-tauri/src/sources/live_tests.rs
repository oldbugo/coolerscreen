//! Live, read-only checks against the real network / machine. Run with:
//!   cargo test --lib sources::live_tests -- --ignored --nocapture --test-threads=1
use super::*;

fn ctx() -> SourceCtx {
    let base = std::env::temp_dir().join("cs-sources-live");
    SourceCtx {
        options: serde_json::Value::Null,
        tmp_dir: base.join("tmp"),
        data_dir: base.join("data"),
        sources_dir: base.join("sources"),
        ffmpeg: std::env::var("COOLERSCREEN_FFMPEG").map(Into::into).unwrap_or_else(|_| "C:\\Users\\pc\\Documents\\CoolerScreen\\node_modules\\ffmpeg-static\\ffmpeg.exe".into()),
    }
}
fn all_pages(p: &dyn SourceProvider, c: &SourceCtx) -> Vec<Item> {
    let mut v = vec![];
    for page in 1..=20 {
        let r = p.list(c, page, None, page == 1).unwrap();
        v.extend(r.items);
        if !r.has_more {
            break;
        }
    }
    v
}

#[test]
#[ignore]
fn lovingcool_list() {
    let c = ctx();
    let items = all_pages(&lovingcool::Lovingcool, &c);
    println!("lovingcool: {} items, first = {:?}", items.len(), items.first().map(|i| (&i.title, &i.thumb)));
    assert!(items.len() >= 40 && items.iter().all(|i| i.action == Action::Add));
}

#[test]
#[ignore]
fn otherbrands_list() {
    let c = ctx();
    let items = all_pages(&otherbrands::OtherBrands, &c);
    println!("otherbrands: {} items, badge {:?}", items.len(), items.first().map(|i| &i.badge));
    assert!(items.len() >= 25);
}

#[test]
#[ignore]
fn wallpaperengine_scan() {
    let c = ctx();
    println!("{}", wallpaperengine::WallpaperEngine.detect(&c).status);
    let all = wallpaperengine::scan(&c);
    let vid = all.iter().filter(|e| e.type_ == "video").count();
    println!(
        "WE: {} items, {} video, {} to record, {} unsupported",
        all.len(),
        vid,
        all.iter().filter(|e| e.action == Action::Record).count(),
        all.iter().filter(|e| e.action == Action::None).count()
    );
    assert!(!all.is_empty());
}

#[test]
#[ignore]
fn link_theme_cover() {
    let c = ctx();
    let page = lovingcool::Lovingcool.list(&c, 1, None, false).unwrap();
    let cover = page.items.iter().find_map(|i| i.thumb.clone()).expect("a cover url");
    println!("cover: {cover}");
    let r = link::Link.add(&c, &cover, &AddRequest::default(), &|_, _| {}, &Cancel::new()).unwrap();
    let len = std::fs::metadata(&r.file).unwrap().len();
    println!("downloaded {} ({} bytes) as {}", r.file.display(), len, r.name);
    assert!(len > 1000 && r.temp);
    std::fs::remove_file(&r.file).unwrap();
    // a web page must be refused
    assert!(link::Link.add(&c, "https://www.htssl.com/", &AddRequest::default(), &|_, _| {}, &Cancel::new()).is_err());
}

#[test]
#[ignore]
fn record_scene_5s() {
    let c = ctx();
    let Some(item) = wallpaperengine::scan(&c).into_iter().find(|e| e.action == Action::Record) else { panic!("no scene wallpaper") };
    println!("recording {} ({})", item.title, item.id);
    let t = std::time::Instant::now();
    let r = wallpaperengine::WallpaperEngine.add(&c, &item.id, &AddRequest { seconds: Some(5.0) }, &|f, m| println!("  {:.0}% {m}", f * 100.0), &Cancel::new()).unwrap();
    println!("recorded {} in {:?}", r.file.display(), t.elapsed());
    println!("size {} bytes", std::fs::metadata(&r.file).unwrap().len());
}
