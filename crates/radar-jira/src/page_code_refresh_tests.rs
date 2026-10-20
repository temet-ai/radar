//! Recorded page completions only; no returned task is polled.

use super::*;
use crate::{demo, tests};
use radar_core::host::{Event, HostEffect};
use radar_core::view::{Intent, Target};
use radar_core::{Contribution, Plugin, Point};

const SITE: &str = "acme";
const KEY: &str = "ABC-12";
const ERROR: &str = "Jira refresh unavailable";
const NOTICE: &str = "Jira page could not sync: Jira refresh unavailable";
const OTHER_URL: &str = "https://code.example/independent/42";

struct Contributor {
    id: &'static str,
    title: &'static str,
    url: String,
}

impl Plugin for Contributor {
    fn id(&self) -> &'static str {
        self.id
    }

    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }

    fn config(&self) -> Option<toml::Value> {
        None
    }

    fn contribute(&self, point: Point, subject: &Subject) -> Option<Contribution> {
        (point == Point::Tab && subject.kind == "ticket").then(|| Contribution::Tab {
            title: self.title.into(),
            rows: vec![Row {
                line: radar_core::view::Text::plain("Independent change"),
                subject: Some(Subject::link(self.url.clone())),
            }],
        })
    }
}

fn host(detail: Detail, contributors: Vec<Contributor>) -> radar_core::Host {
    let mut plugins: Vec<Box<dyn Plugin>> = vec![Box::new(demo::sample())];
    for contributor in contributors {
        plugins.push(Box::new(contributor));
    }
    let mut host = radar_core::Host::new(plugins, demo::NOW).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(demo::sample());
    let p = tests::jira_mut(&mut host);
    open(p, SITE, KEY);
    tests::complete_detail(p, SITE, KEY, Ok(detail));
    p.pages[0].tab = TABS.len();
    host
}

fn fail(host: &mut radar_core::Host) {
    let p = tests::jira_mut(host);
    let id = detail_key(SITE, KEY);
    let before = p.details[&id].value.clone();
    let at = p.details[&id].at;
    let revision = p.details[&id].revision;
    p.now += 5;
    tests::complete_detail(p, SITE, KEY, Err(ERROR.into()));
    let f = &p.details[&id];
    assert_eq!(f.value, before);
    assert_eq!((f.at, f.revision), (at, revision));
    assert!(!f.loading);
}

#[test]
fn retained_jira_code_shows_refresh_failure_until_retry_succeeds() {
    let fixture = tests::detail();
    let mut host = host(fixture.clone(), vec![]);
    let before = crate::tests::shown(&host, 160, 40);
    assert!(!fetch(tests::jira_mut(&mut host), SITE, KEY, true).is_empty());
    let shown = crate::tests::shown(&host, 160, 40);
    assert_eq!(
        shown, before,
        "healthy cached content stays unchanged beneath the host spinner"
    );
    assert!(!shown.contains("loading…"));
    fail(&mut host);
    let shown = crate::tests::shown(&host, 160, 40);
    assert!(shown.contains(NOTICE), "{shown}");
    assert!(shown.contains("api !263"), "{shown}");
    insta::assert_snapshot!("code_cached_refresh_failed", shown);
    assert!(!fetch(tests::jira_mut(&mut host), SITE, KEY, true).is_empty());
    let shown = crate::tests::shown(&host, 160, 40);
    assert!(shown.contains(NOTICE), "retry retains the failure");
    assert!(!shown.contains("loading…"));
    let p = tests::jira_mut(&mut host);
    p.now += 5;
    tests::complete_detail(p, SITE, KEY, Ok(fixture));
    assert!(p.details[&detail_key(SITE, KEY)].error.is_none());
    assert_eq!(p.details[&detail_key(SITE, KEY)].at, p.now);
    assert_eq!(crate::tests::shown(&host, 160, 40), before);
}

#[test]
fn code_rows_keep_their_intent_targets_and_status_is_not_a_row() {
    let fixture = tests::detail();
    let own_url = fixture.code[0].url.clone();
    let mut host = host(
        fixture,
        vec![Contributor {
            id: "code-fixture",
            title: "Code",
            url: OTHER_URL.into(),
        }],
    );
    fail(&mut host);
    let shown = crate::tests::shown(&host, 160, 40);
    let error_y = shown.lines().position(|line| line.contains(NOTICE)).unwrap();
    assert!(shown.contains("Independent change") && shown.contains("api !263"));
    let rows: Vec<_> = shown
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains("{row:"))
        .collect();
    assert_eq!(rows.len(), 2, "{shown}");
    assert!(rows.iter().all(|(y, _)| *y > error_y), "{shown}");
    for (index, label, url) in [
        (1, "api !263", own_url.as_str()),
        (0, "Independent change", OTHER_URL),
    ] {
        let id = format!("row:{index}");
        assert!(
            rows.iter().any(|(_, line)| line.contains(label) && line.contains(&format!("{{{id}}}"))),
            "{shown}"
        );
        host.update(Event::Intent(Intent::Select { target: Target::new(id) }));
        assert_eq!(tests::jira_mut(&mut host).pages[0].row.get(), index);
        let fx = host.update(crate::key_event(Chord::new(Key::Enter)));
        assert!(
            fx.iter().any(|e| matches!(e, HostEffect::Open(got) if got == url)),
            "{fx:?}"
        );
    }
}

#[test]
fn independent_tabs_and_fully_deduplicated_code_are_not_labelled_failed() {
    for case in ["independent-code", "deduplicated-code", "other-tab"] {
        let mut fixture = tests::detail();
        let contributor = match case {
            "deduplicated-code" => Contributor {
                id: "code-fixture",
                title: "cOdE",
                url: format!("{}/", fixture.code[0].url),
            },
            "other-tab" => Contributor {
                id: "code-fixture",
                title: "Notes",
                url: OTHER_URL.into(),
            },
            _ => {
                fixture.code.clear();
                Contributor {
                    id: "code-fixture",
                    title: "Code",
                    url: OTHER_URL.into(),
                }
            }
        };
        let mut host = host(fixture, vec![contributor]);
        if case == "other-tab" {
            tests::jira_mut(&mut host).pages[0].tab += 1;
        }
        let before = crate::tests::shown(&host, 160, 40);
        fail(&mut host);
        let shown = crate::tests::shown(&host, 160, 40);
        assert!(shown.contains("Independent change"), "{case}: {shown}");
        assert!(!shown.contains(ERROR), "{case}: {shown}");
        assert_eq!(shown, before, "{case}");
    }
}

#[test]
fn a_second_code_named_tab_never_inherits_the_first_tabs_jira_error() {
    let mut host = host(
        tests::detail(),
        vec![
            Contributor {
                id: "first-code",
                title: "Code",
                url: OTHER_URL.into(),
            },
            Contributor {
                id: "second-code",
                title: "cOdE",
                url: "https://code.example/second/43".into(),
            },
        ],
    );
    tests::jira_mut(&mut host).pages[0].tab = TABS.len() + 1;
    let before = crate::tests::shown(&host, 160, 40);
    fail(&mut host);
    let shown = crate::tests::shown(&host, 160, 40);
    assert!(shown.contains("Independent change"));
    assert!(!shown.contains(ERROR), "{shown}");
    assert_eq!(shown, before);
    tests::jira_mut(&mut host).pages[0].tab = TABS.len();
    let shown = crate::tests::shown(&host, 160, 40);
    assert!(
        shown.contains(NOTICE) && shown.contains("api !263"),
        "{shown}"
    );
}

#[test]
fn a_failed_refresh_keeps_the_code_rows_and_says_so_over_them() {
    let mut p = demo::sample();
    open(&mut p, SITE, KEY);
    tests::complete_detail(&mut p, SITE, KEY, Ok(tests::detail()));
    tests::complete_detail(&mut p, SITE, KEY, Err(ERROR.into()));
    let extra = added_tabs(&p, &p.pages[0], &radar_core::Parts::none());
    assert_eq!(extra.jira_code, Some(0));
}
