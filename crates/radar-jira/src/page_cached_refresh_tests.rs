//! Recorded detail completions only; returned tasks are never polled.

use super::*;
use crate::{demo, tests};
use radar_core::host::{Event, Hit, HostEffect, MouseKind};
use radar_core::ratatui::crossterm::event::KeyModifiers;

const SITE: &str = "acme";
const KEY: &str = "ABC-12";
const ERROR: &str = "Jira refresh unavailable";

fn fixture(empty: bool) -> Detail {
    let mut detail = tests::detail();
    if empty {
        detail.links.clear();
    }
    detail
}

fn host(empty: bool) -> radar_core::Host {
    let mut host = tests::hosted();
    let p = tests::jira_mut(&mut host);
    open(p, SITE, KEY);
    tests::complete_detail(p, SITE, KEY, Ok(fixture(empty)));
    p.pages[0].tab = LINKS;
    host
}

fn fail(host: &mut radar_core::Host) {
    let p = tests::jira_mut(host);
    let id = detail_key(SITE, KEY);
    let before = format!("{:?}", p.details[&id].value);
    let at = p.details[&id].at;
    let revision = p.details[&id].revision;
    p.now += 5;
    tests::complete_detail(p, SITE, KEY, Err(ERROR.into()));
    let f = &p.details[&id];
    assert_eq!((f.at, f.revision), (at, revision));
    assert_eq!(format!("{:?}", f.value), before);
    assert!(!f.loading);
}

fn shot(host: &radar_core::Host, name: &str, w: u16, h: u16) {
    let Some(dir) = std::env::var_os("RADAR_CACHED_REFRESH_SHOTS") else {
        return;
    };
    let path = std::path::Path::new(&dir);
    std::fs::create_dir_all(path).unwrap();
    let svg = radar_core::shell::to_svg(host, w, h);
    std::fs::write(path.join(format!("jira-{name}-{w}x{h}.svg")), &svg).unwrap();
    std::fs::write(
        path.join(format!("jira-{name}-{w}x{h}.png")),
        radar_core::shot::to_png(&svg).expect("visual export needs radar-core/png"),
    )
    .unwrap();
}

#[test]
fn cached_links_show_refresh_error_until_success_including_empty_links() {
    for empty in [false, true] {
        let label = if empty { "empty" } else { "rows" };
        let mut host = host(empty);
        let before: Vec<_> = [(80, 24), (160, 40)]
            .map(|(w, h)| radar_core::shell::to_text(&host, w, h))
            .into();
        // Healthy cached content keeps the existing loading policy.
        let p = tests::jira_mut(&mut host);
        assert!(!fetch(p, SITE, KEY, true).is_empty());
        for (w, h) in [(80, 24), (160, 40)] {
            assert!(!radar_core::shell::to_text(&host, w, h).contains("loading…"));
        }
        fail(&mut host);
        for (w, h) in [(80, 24), (160, 40)] {
            let shown = radar_core::shell::to_text(&host, w, h);
            assert!(
                shown.contains(&format!("could not load: {ERROR}")),
                "{shown}"
            );
            assert!(shown.contains(if empty {
                "No parent, children or links."
            } else {
                "ABC-13"
            }));
            assert!(
                radar_core::shell::to_styles(&host, w, h)
                    .lines()
                    .any(
                        |line| line.contains("could not load:") && line.contains("fg:status.error")
                    )
            );
            insta::assert_snapshot!(
                format!("links_cached_refresh_failed_{label}_{w}x{h}"),
                shown
            );
            shot(&host, &format!("failed-{label}"), w, h);
        }
        let p = tests::jira_mut(&mut host);
        assert!(!fetch(p, SITE, KEY, true).is_empty());
        for (w, h) in [(80, 24), (160, 40)] {
            let shown = radar_core::shell::to_text(&host, w, h);
            assert!(shown.contains(ERROR), "retry keeps the retained error");
            assert!(!shown.contains("loading…"));
        }
        let p = tests::jira_mut(&mut host);
        p.now += 5;
        tests::complete_detail(p, SITE, KEY, Ok(fixture(empty)));
        assert_eq!(p.details[&detail_key(SITE, KEY)].at, p.now);
        assert!(p.details[&detail_key(SITE, KEY)].error.is_none());
        for ((w, h), before) in [(80, 24), (160, 40)].into_iter().zip(before) {
            assert_eq!(radar_core::shell::to_text(&host, w, h), before);
            shot(&host, &format!("recovered-{label}"), w, h);
        }
    }
}

#[test]
fn cached_links_error_is_not_a_row_and_rendered_mouse_rows_keep_their_targets() {
    for (w, h) in [(80, 24), (160, 40)] {
        let mut host = host(false);
        fail(&mut host);
        let shown = radar_core::shell::to_text(&host, w, h);
        let error_y = shown.lines().position(|line| line.contains(ERROR)).unwrap() as u16;
        let rows: Vec<_> = host
            .hits
            .borrow()
            .iter()
            .filter_map(|(rect, hit)| match hit {
                Hit::Plugin { id, .. } if id.starts_with("row:") => Some((*rect, id.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 4);
        assert!(rows.iter().all(|(rect, _)| rect.y > error_y));
        let (rect, _) = rows.iter().find(|(_, id)| id == "row:1").unwrap();
        assert!(
            shown
                .lines()
                .nth(rect.y as usize)
                .unwrap()
                .contains("ABC-13")
        );
        host.update(Event::Mouse {
            column: rect.x + 1,
            row: rect.y,
            kind: MouseKind::Press,
            at_ms: 1000,
        });
        assert_eq!(tests::jira_mut(&mut host).pages[0].row.get(), 1);
        // Clicking the status itself only focuses its panel, retaining the row.
        host.update(Event::Mouse {
            column: rect.x + 1,
            row: error_y,
            kind: MouseKind::Press,
            at_ms: 2000,
        });
        assert_eq!(tests::jira_mut(&mut host).pages[0].row.get(), 1);
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert_eq!(
            tests::jira_mut(&mut host).pages.last().unwrap().key,
            "ABC-13"
        );
        host.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert_eq!(tests::jira_mut(&mut host).pages[0].row.get(), 1);
        radar_core::shell::to_text(&host, w, h);
        let (rect, _) = rows.iter().find(|(_, id)| id == "row:3").unwrap();
        host.update(Event::Mouse {
            column: rect.x + 1,
            row: rect.y,
            kind: MouseKind::Press,
            at_ms: 3000,
        });
        let fx = host.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(
            fx.iter()
                .any(|e| matches!(e, HostEffect::Open(url) if url == "https://example.com/design"))
        );
    }
}

#[test]
fn cached_links_status_never_draws_or_marks_rows_beyond_a_short_panel() {
    let mut p = demo::sample();
    open(&mut p, SITE, KEY);
    tests::complete_detail(&mut p, SITE, KEY, Ok(fixture(false)));
    tests::complete_detail(&mut p, SITE, KEY, Err(ERROR.into()));
    p.pages[0].tab = LINKS;
    for height in 0..=4 {
        // The page's frame takes four lines over the panel.
        let (buf, hits, area) = tests::draw_page(&p, 80, height + 4, 8 + 4);
        assert!(
            !hits.iter().any(|(_, id)| id.starts_with("row:")),
            "no room for a data row at {height}"
        );
        for y in area.bottom()..buf.area.height {
            assert!((0..80).all(|x| buf[(x, y)].symbol() == " "));
        }
    }
}
