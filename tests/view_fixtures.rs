//! Every surface the parts show with demo data, as the view data a renderer
//! is sent: each screen, settings section, item page and the host chrome, at
//! 80x24 and 160x40. With `RADAR_VIEW_FIXTURES_DIR` set, one JSON file per
//! surface is written there (`<part>_<surface>_<w>x<h>.json`, the bare
//! ScreenView, SectionView or ChromeView) and `_surfaces.json`,
//! `{"surfaces": [sorted `<part>_<surface>` names]}`; without it the surfaces are only
//! checked: unique names, bounded, and read back from their JSON.
//!
//! RADAR_VIEW_FIXTURES_DIR=$PWD/ink/test/fixtures \
//!   CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar --test view_fixtures

use std::collections::BTreeMap;

use radar_core::headless::Headless;
use radar_core::host::View;
use serde_json::Value;

const SIZES: [(u16, u16); 2] = [(80, 24), (160, 40)];

fn demo(w: u16, h: u16) -> Headless {
    let mut host = radar_core::shot::demo_host(radar::parts()).unwrap();
    host.view = View::Screen(0);
    Headless::new(host, w, h)
}

fn slug(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c.to_ascii_lowercase() {
            c if c.is_ascii_alphanumeric() => out.push(c),
            _ if !out.ends_with('-') => out.push('-'),
            _ => {}
        }
    }
    out.trim_matches('-').to_string()
}

/// `<part>_<surface>` -> the JSON at each size.
type Surfaces = BTreeMap<String, Vec<((u16, u16), Value)>>;

fn surfaces() -> Surfaces {
    let mut all: Surfaces = BTreeMap::new();
    let mut put = |name: String, size: (u16, u16), value: Value| {
        all.entry(name).or_default().push((size, value));
    };
    for (w, h) in SIZES {
        let mut head = demo(w, h);
        for (part, view) in head.screen_views() {
            put(format!("{part}_{}", slug(&view.scope)), (w, h), serde_json::to_value(&view).unwrap());
        }
        for (part, view) in head.section_views() {
            put(format!("{part}_settings-{}", slug(&view.scope)), (w, h), serde_json::to_value(&view).unwrap());
        }
        for i in 0..head.screen_count() {
            for page in head.reach_pages(i, 12, 2) {
                assert!(page.gaps.is_empty(), "{page:#?}");
                let k = &page.kind;
                let name = format!("{}_{}-page{}-{}", k.part, slug(&k.screen), k.depth, slug(&k.shape.join(" ")));
                put(name, (w, h), serde_json::to_value(&page.view).unwrap());
            }
        }
        put("host_chrome".into(), (w, h), serde_json::to_value(head.chrome()).unwrap());
    }
    all
}

#[test]
fn every_surface_is_a_named_bounded_view_that_reads_back() {
    let all = surfaces();
    assert!(all.len() > 20, "{:?}", all.keys().collect::<Vec<_>>());
    for (name, sizes) in &all {
        assert_eq!(sizes.len(), SIZES.len(), "{name} is not at every size");
        for (_, value) in sizes {
            let text = serde_json::to_string(value).unwrap();
            assert!(text.len() < 1 << 20, "{name}: {} bytes", text.len());
            let back: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(&back, value, "{name}");
        }
    }
    let has = |part: &str| all.keys().any(|n| n.starts_with(&format!("{part}_")) && n.contains("-page"));
    assert!(has("gitlab") && has("github") && has("jira"), "{:?}", all.keys().collect::<Vec<_>>());
}

#[test]
fn the_fixtures_are_written_when_asked() {
    let Ok(dir) = std::env::var("RADAR_VIEW_FIXTURES_DIR") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let all = surfaces();
    for (name, sizes) in &all {
        for ((w, h), value) in sizes {
            let text = serde_json::to_string_pretty(value).unwrap();
            std::fs::write(dir.join(format!("{name}_{w}x{h}.json")), text + "\n").unwrap();
        }
    }
    let names: Vec<&String> = all.keys().collect();
    let manifest = serde_json::json!({ "surfaces": names });
    std::fs::write(dir.join("_surfaces.json"), serde_json::to_string_pretty(&manifest).unwrap() + "\n").unwrap();
}
