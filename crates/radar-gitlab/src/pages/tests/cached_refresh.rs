//! Cached repository runs stay readable after a recorded failed refresh.
//! Tasks are created and dropped without polling them or resolving credentials.

use super::*;
use radar_core::host::{Event, Hit, HostEffect, MouseKind};
use radar_core::journal::Outcome as JournalOutcome;

const API: &str = "acme/team-a/api";
const WEB: &str = "acme/team-a/web";
const ERROR: &str = "GitLab refresh unavailable";

fn plugin(host: &mut radar_core::Host) -> &mut GitlabPlugin {
    assert_eq!(host.plugins[0].id(), "gitlab");
    // SAFETY: every host in this module contains exactly this plugin type.
    unsafe { &mut *(host.plugins[0].as_mut() as *mut dyn Plugin as *mut GitlabPlugin) }
}

fn receive(p: &mut GitlabPlugin, path: &str, result: Result<Vec<PipelineRun>, String>) {
    let outcome = if result.is_ok() {
        JournalOutcome::Succeeded
    } else {
        JournalOutcome::Failed
    };
    let action = crate::runs_history::action(&p.cfg.url, path, "main");
    let request = crate::runs_ownership::pending(p, path);
    let effects = p.on_msg(Box::new(GlMsg::Runs {
        request,
        action,
        result: result.map(|runs| crate::client::RunsRead {
            runs,
            evidence: Default::default(),
        }),
    }));
    assert!(matches!(effects.first(), Some(Effect::Report(got)) if *got == outcome));
}

fn fixture(path: &str, empty: bool) -> Vec<PipelineRun> {
    if empty {
        return vec![];
    }
    let mut runs = fixtures::runs();
    for run in &mut runs {
        run.url = run.url.as_ref().map(|url| url.replace(API, path));
    }
    runs
}

fn host(path: &str, empty: bool) -> radar_core::Host {
    let mut p = sample();
    push(&mut p, Kind::Repo(path.into()));
    p.pages[0].tab = repo::PIPELINES;
    receive(&mut p, path, Ok(fixture(path, empty)));
    let mut host = radar_core::Host::new(vec![Box::new(sample())], p.now).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host
}

fn fail(host: &mut radar_core::Host, path: &str) {
    let p = plugin(host);
    let before = format!("{:?}", p.runs[path].value);
    let at = p.runs[path].at;
    assert!(fetch(p, &Kind::Repo(path.into()), true).iter().any(|e|
        matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.runs")));
    p.now += 5;
    receive(p, path, Err(ERROR.into()));
    assert_eq!(p.runs[path].at, at);
    assert_eq!(format!("{:?}", p.runs[path].value), before);
    assert!(!p.runs[path].loading);
}

fn shot(host: &radar_core::Host, name: &str, w: u16, h: u16) {
    let Some(dir) = std::env::var_os("RADAR_CACHED_REFRESH_SHOTS") else {
        return;
    };
    let path = std::path::Path::new(&dir);
    std::fs::create_dir_all(path).unwrap();
    let svg = radar_core::shell::to_svg(host, w, h);
    std::fs::write(path.join(format!("gitlab-{name}-{w}x{h}.svg")), &svg).unwrap();
    std::fs::write(
        path.join(format!("gitlab-{name}-{w}x{h}.png")),
        radar_core::shot::to_png(&svg).expect("visual export needs radar-core/png"),
    )
    .unwrap();
}

#[test]
fn cached_runs_show_styled_refresh_error_until_success_with_red_main_and_empty_runs() {
    for (path, empty, label) in [
        (API, false, "rows"),
        (WEB, false, "red-main"),
        (API, true, "empty"),
    ] {
        let mut host = host(path, empty);
        let before: Vec<_> = [(80, 24), (160, 40)]
            .map(|(w, h)| radar_core::shell::to_text(&host, w, h))
            .into();
        fail(&mut host, path);
        for (w, h) in [(80, 24), (160, 40)] {
            let shown = radar_core::shell::to_text(&host, w, h);
            assert!(
                shown.contains(&format!("could not load: {ERROR}")),
                "{shown}"
            );
            assert!(shown.contains(if empty {
                "No pipelines on main lately."
            } else {
                "#1842"
            }));
            if path == WEB {
                assert!(shown.contains("why main is red") && shown.contains("unit-test"));
            }
            assert!(
                radar_core::shell::to_styles(&host, w, h)
                    .lines()
                    .any(
                        |line| line.contains("could not load:") && line.contains("fg:status.error")
                    )
            );
            insta::assert_snapshot!(format!("runs_cached_refresh_failed_{label}_{w}x{h}"), shown);
            shot(&host, &format!("failed-{label}"), w, h);
        }
        let p = plugin(&mut host);
        assert!(fetch(p, &Kind::Repo(path.into()), true).iter().any(|e|
            matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.runs")));
        for (w, h) in [(80, 24), (160, 40)] {
            let shown = radar_core::shell::to_text(&host, w, h);
            assert!(shown.contains(ERROR), "retry keeps the retained error");
            assert!(!shown.contains("loading…"));
        }
        let p = plugin(&mut host);
        p.now += 5;
        receive(p, path, Ok(fixture(path, empty)));
        assert_eq!(p.runs[path].at, p.now);
        assert!(p.runs[path].error.is_none());
        for ((w, h), before) in [(80, 24), (160, 40)].into_iter().zip(before) {
            let recovered = radar_core::shell::to_text(&host, w, h);
            // A forced red-main refresh also asks for diagnosis; that independent
            // request may still own the top-bar spinner after runs have recovered.
            assert_eq!(
                recovered.lines().skip(1).collect::<Vec<_>>(),
                before.lines().skip(1).collect::<Vec<_>>()
            );
            if path == API {
                assert_eq!(recovered, before);
            }
            shot(&host, &format!("recovered-{label}"), w, h);
        }
    }
}

#[test]
fn cached_runs_error_is_not_a_row_and_rendered_mouse_targets_keep_red_main_offsets() {
    for path in [API, WEB] {
        for (w, h) in [(80, 24), (160, 40)] {
            let mut host = host(path, false);
            fail(&mut host, path);
            let lead = repo::lead_rows(plugin(&mut host), path).len();
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
            assert_eq!(
                rows.iter()
                    .map(|(_, id)| id)
                    .collect::<std::collections::HashSet<_>>()
                    .len(),
                lead + 2
            );
            assert!(
                rows.iter()
                    .all(|(rect, _)| !rect.contains((2, error_y).into()))
            );
            let (rect, _) = rows
                .iter()
                .find(|(_, id)| id == &format!("row:{}", lead + 1))
                .unwrap();
            assert!(
                shown
                    .lines()
                    .nth(rect.y as usize)
                    .unwrap()
                    .contains("#1838")
            );
            host.update(Event::Mouse {
                column: rect.x + 1,
                row: rect.y,
                kind: MouseKind::Press,
                at_ms: 1000,
            });
            assert_eq!(plugin(&mut host).pages[0].row.get(), lead + 1);
            host.update(Event::Mouse {
                column: rect.x + 1,
                row: error_y,
                kind: MouseKind::Press,
                at_ms: 2000,
            });
            assert_eq!(plugin(&mut host).pages[0].row.get(), lead + 1);
            let fx = host.update(Event::Key(KeyEvent::new(
                KeyCode::Enter,
                KeyModifiers::NONE,
            )));
            assert!(fx.iter().any(|e| matches!(e, HostEffect::Open(url) if url == &format!("https://gitlab.com/{path}/-/pipelines/1"))));
            if lead > 0 {
                let (rect, _) = rows.iter().find(|(_, id)| id == "row:0").unwrap();
                assert!(
                    shown
                        .lines()
                        .nth(rect.y as usize)
                        .unwrap()
                        .contains("unit-test")
                );
                host.update(Event::Mouse {
                    column: rect.x + 1,
                    row: rect.y,
                    kind: MouseKind::Press,
                    at_ms: 3000,
                });
                host.update(Event::Key(KeyEvent::new(
                    KeyCode::Enter,
                    KeyModifiers::NONE,
                )));
                assert_eq!(
                    plugin(&mut host).pages.last().unwrap().kind,
                    Kind::Job {
                        project: WEB.into(),
                        id: 5001
                    }
                );
                host.update(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
                assert_eq!(plugin(&mut host).pages[0].row.get(), 0);
            }
        }
    }
}

#[test]
fn changed_default_ref_hides_old_rows_before_render_and_keyboard_or_mouse_activation() {
    for (w, h) in [(80, 24), (160, 40)] {
        let mut host = host(API, false);
        let shown = radar_core::shell::to_text(&host, w, h);
        assert!(shown.contains("#1842"));
        let rect = host.hits.borrow().iter().find_map(|(rect, hit)| match hit {
            Hit::Plugin { id, .. } if id == "row:0" => Some(*rect),
            _ => None,
        }).unwrap();
        let p = plugin(&mut host);
        for data in p.data.values_mut() {
            for project in &mut data.projects {
                if project.full_path == API { project.default_branch = "release/new".into(); }
            }
        }
        // The scope guard also protects input before a fresh terminal paint.
        let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert!(!fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
        let fx = host.update(Event::Mouse {
            column: rect.x + 1, row: rect.y, kind: MouseKind::Press, at_ms: 1000,
        });
        assert!(!fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
        // A second press inside the host's double-click window activates.
        let fx = host.update(Event::Mouse {
            column: rect.x + 1, row: rect.y, kind: MouseKind::Press, at_ms: 1200,
        });
        assert!(!fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
        let shown = radar_core::shell::to_text(&host, w, h);
        assert!(!shown.contains("#1842") && !shown.contains("#1838"), "{shown}");
        assert!(shown.contains("release/new"));
        let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)));
        assert!(fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. } | HostEffect::Tracked { .. })));
        receive(plugin(&mut host), API, Ok(fixture(API, false)));
        let shown = radar_core::shell::to_text(&host, w, h);
        assert!(shown.contains("#1842") && shown.contains("release/new"));
        let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
        assert!(fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
    }
}
