//! Recorded page completions only; no returned task is polled.

use super::*;
use crate::{demo, tests};
use radar_core::host::{Event, Hit, HostEffect, MouseKind};
use radar_core::ratatui::crossterm::event::KeyModifiers;
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

fn shot(host: &radar_core::Host, name: &str, w: u16, h: u16) {
    let Some(dir) = std::env::var_os("RADAR_JIRA_CODE_REFRESH_SHOTS") else {
        return;
    };
    let path = std::path::Path::new(&dir);
    std::fs::create_dir_all(path).unwrap();
    let svg = radar_core::shell::to_svg(host, w, h);
    std::fs::write(path.join(format!("jira-code-{name}-{w}x{h}.svg")), &svg).unwrap();
    std::fs::write(
        path.join(format!("jira-code-{name}-{w}x{h}.png")),
        radar_core::shot::to_png(&svg).expect("visual export needs radar-core/png"),
    )
    .unwrap();
}

#[test]
fn retained_jira_code_shows_refresh_failure_until_retry_succeeds() {
    let fixture = tests::detail();
    let mut host = host(fixture.clone(), vec![]);
    let before: Vec<_> = [(80, 24), (160, 40)]
        .map(|(w, h)| radar_core::shell::to_text(&host, w, h))
        .into();
    assert!(!fetch(tests::jira_mut(&mut host), SITE, KEY, true).is_empty());
    for ((w, h), before) in [(80, 24), (160, 40)].into_iter().zip(&before) {
        let shown = radar_core::shell::to_text(&host, w, h);
        assert_eq!(
            shown.lines().skip(1).collect::<Vec<_>>(),
            before.lines().skip(1).collect::<Vec<_>>(),
            "healthy cached content stays unchanged beneath the host spinner"
        );
        assert!(!shown.contains("loading…"));
        shot(&host, "healthy", w, h);
    }
    fail(&mut host);
    for (w, h) in [(80, 24), (160, 40)] {
        let shown = radar_core::shell::to_text(&host, w, h);
        assert!(shown.contains(NOTICE), "{shown}");
        assert!(shown.contains("api !263"), "{shown}");
        assert!(
            radar_core::shell::to_styles(&host, w, h)
                .lines()
                .any(|line| line.contains("Jira page could not sync:")
                    && line.contains("fg:status.error"))
        );
        insta::assert_snapshot!(format!("code_cached_refresh_failed_{w}x{h}"), shown);
        shot(&host, "failed", w, h);
    }
    assert!(!fetch(tests::jira_mut(&mut host), SITE, KEY, true).is_empty());
    for (w, h) in [(80, 24), (160, 40)] {
        let shown = radar_core::shell::to_text(&host, w, h);
        assert!(shown.contains(NOTICE), "retry retains the failure");
        assert!(!shown.contains("loading…"));
    }
    let p = tests::jira_mut(&mut host);
    p.now += 5;
    tests::complete_detail(p, SITE, KEY, Ok(fixture));
    assert!(p.details[&detail_key(SITE, KEY)].error.is_none());
    assert_eq!(p.details[&detail_key(SITE, KEY)].at, p.now);
    for ((w, h), before) in [(80, 24), (160, 40)].into_iter().zip(before) {
        assert_eq!(radar_core::shell::to_text(&host, w, h), before);
        shot(&host, "recovered", w, h);
    }
}

#[test]
fn rendered_code_rows_keep_mouse_enter_targets_and_status_is_not_a_row() {
    for (w, h) in [(80, 24), (160, 40)] {
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
        let shown = radar_core::shell::to_text(&host, w, h);
        let error_y = shown
            .lines()
            .position(|line| line.contains(NOTICE))
            .unwrap() as u16;
        assert!(shown.contains("Independent change") && shown.contains("api !263"));
        let rows: Vec<_> = host
            .hits
            .borrow()
            .iter()
            .filter_map(|(rect, hit)| match hit {
                Hit::Plugin { id, .. } if id.starts_with("row:") => Some((*rect, id.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|(rect, _)| rect.y > error_y));
        let mut at_ms = 1000;
        for (index, label, url) in [
            (1, "api !263", own_url.as_str()),
            (0, "Independent change", OTHER_URL),
        ] {
            let (rect, _) = rows
                .iter()
                .find(|(_, id)| id == &format!("row:{index}"))
                .unwrap();
            assert!(shown.lines().nth(rect.y as usize).unwrap().contains(label));
            host.update(Event::Mouse {
                column: rect.x + 1,
                row: rect.y,
                kind: MouseKind::Press,
                at_ms,
            });
            at_ms += 1000;
            assert_eq!(
                tests::jira_mut(&mut host).pages[0].row.get(),
                index as usize
            );
            host.update(Event::Mouse {
                column: rect.x + 1,
                row: error_y,
                kind: MouseKind::Press,
                at_ms,
            });
            at_ms += 1000;
            assert_eq!(
                tests::jira_mut(&mut host).pages[0].row.get(),
                index as usize
            );
            let fx = host.update(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            assert!(
                fx.iter()
                    .any(|e| matches!(e, HostEffect::Open(got) if got == url)),
                "{fx:?}"
            );
        }
        shot(&host, "mixed-failed", w, h);
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
        let before: Vec<_> = [(80, 24), (160, 40)]
            .map(|(w, h)| radar_core::shell::to_text(&host, w, h))
            .into();
        fail(&mut host);
        for ((w, h), before) in [(80, 24), (160, 40)].into_iter().zip(before) {
            let shown = radar_core::shell::to_text(&host, w, h);
            assert!(shown.contains("Independent change"), "{case}: {shown}");
            assert!(!shown.contains(ERROR), "{case}: {shown}");
            assert_eq!(shown, before, "{case}");
        }
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
    let before: Vec<_> = [(80, 24), (160, 40)]
        .map(|(w, h)| radar_core::shell::to_text(&host, w, h))
        .into();
    fail(&mut host);
    for ((w, h), before) in [(80, 24), (160, 40)].into_iter().zip(before) {
        let shown = radar_core::shell::to_text(&host, w, h);
        assert!(shown.contains("Independent change"));
        assert!(!shown.contains(ERROR), "{shown}");
        assert_eq!(shown, before);
    }
    tests::jira_mut(&mut host).pages[0].tab = TABS.len();
    for (w, h) in [(80, 24), (160, 40)] {
        let shown = radar_core::shell::to_text(&host, w, h);
        assert!(
            shown.contains(NOTICE) && shown.contains("api !263"),
            "{shown}"
        );
    }
}

#[test]
fn code_status_and_rows_stay_inside_tiny_panels() {
    let mut p = demo::sample();
    open(&mut p, SITE, KEY);
    tests::complete_detail(&mut p, SITE, KEY, Ok(tests::detail()));
    // A failed refresh keeps the Code rows and says so over them.
    tests::complete_detail(&mut p, SITE, KEY, Err(ERROR.into()));
    let extra = added_tabs(&p, &p.pages[0], &radar_core::Parts::none());
    assert_eq!(extra.jira_code, Some(0));
    p.pages[0].tab = TABS.len();
    for width in [0, 1, 2, 3, 4, 80, 160] {
        for height in 0..=5 {
            // The page's frame takes four lines over the panel.
            let (buf, hits, area) = tests::draw_page(&p, width, height + 4, 9 + 4);
            let panel = radar_core::ratatui::layout::Rect {
                y: area.y + 4,
                height,
                ..area
            };
            let rows: Vec<_> = hits.iter().filter(|(_, id)| id.starts_with("row:")).collect();
            if height <= 3 || width <= 4 {
                assert!(rows.is_empty(), "no row room at {width}x{height}");
            }
            for (rect, _) in rows {
                assert!(rect.y > panel.y + 1 && rect.bottom() < panel.bottom());
                assert!(rect.x > panel.x && rect.right() < panel.right());
            }
            for y in 0..buf.area.height {
                for x in 0..buf.area.width {
                    if !area.contains((x, y).into()) {
                        assert_eq!(buf[(x, y)].symbol(), " ", "outside {area:?} at {x},{y}");
                    }
                }
            }
        }
    }
}
