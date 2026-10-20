//! Recorded detail completions only; returned tasks are never polled.

use super::*;
use crate::tests;
use radar_core::host::{Event, HostEffect};
use radar_core::view::{Intent, Target};
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

#[test]
fn cached_links_show_refresh_error_until_success_including_empty_links() {
    for empty in [false, true] {
        let label = if empty { "empty" } else { "rows" };
        let mut host = host(empty);
        let before = crate::tests::shown(&host, 160, 40);
        // Healthy cached content keeps the existing loading policy.
        let p = tests::jira_mut(&mut host);
        assert!(!fetch(p, SITE, KEY, true).is_empty());
        assert!(!crate::tests::shown(&host, 160, 40).contains("loading…"));
        fail(&mut host);
        let shown = crate::tests::shown(&host, 160, 40);
        assert!(
            shown.contains(&format!("could not load: {ERROR}")),
            "{shown}"
        );
        assert!(shown.contains(if empty {
            "No parent, children or links."
        } else {
            "ABC-13"
        }));
        insta::assert_snapshot!(format!("links_cached_refresh_failed_{label}"), shown);
        let p = tests::jira_mut(&mut host);
        assert!(!fetch(p, SITE, KEY, true).is_empty());
        let shown = crate::tests::shown(&host, 160, 40);
        assert!(shown.contains(ERROR), "retry keeps the retained error");
        assert!(!shown.contains("loading…"));
        let p = tests::jira_mut(&mut host);
        p.now += 5;
        tests::complete_detail(p, SITE, KEY, Ok(fixture(empty)));
        assert_eq!(p.details[&detail_key(SITE, KEY)].at, p.now);
        assert!(p.details[&detail_key(SITE, KEY)].error.is_none());
        assert_eq!(crate::tests::shown(&host, 160, 40), before);
    }
}

#[test]
fn cached_links_error_is_not_a_row_and_row_intents_keep_their_targets() {
    let mut host = host(false);
    fail(&mut host);
    let shown = crate::tests::shown(&host, 160, 40);
    let error_y = shown.lines().position(|line| line.contains(ERROR)).unwrap();
    let rows: Vec<_> = shown
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("{row:"))
        .collect();
    assert_eq!(rows.len(), 4, "{shown}");
    assert!(rows.iter().all(|(y, _)| *y > error_y), "{shown}");
    assert!(rows.iter().any(|(_, line)| line.contains("ABC-13") && line.contains("{row:1}")), "{shown}");
    host.update(Event::Intent(Intent::Select { target: Target::new("row:1") }));
    assert_eq!(tests::jira_mut(&mut host).pages[0].row.get(), 1);
    host.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    assert_eq!(tests::jira_mut(&mut host).pages.last().unwrap().key, "ABC-13");
    host.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
    assert_eq!(tests::jira_mut(&mut host).pages[0].row.get(), 1);
    crate::tests::shown(&host, 160, 40);
    host.update(Event::Intent(Intent::Select { target: Target::new("row:3") }));
    let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    assert!(
        fx.iter()
            .any(|e| matches!(e, HostEffect::Open(url) if url == "https://example.com/design"))
    );
}
