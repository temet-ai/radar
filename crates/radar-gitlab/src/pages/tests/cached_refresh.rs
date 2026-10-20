//! Cached repository runs stay readable after a recorded failed refresh.
//! Tasks are created and dropped without polling them or resolving credentials.

use super::*;
use radar_core::host::{Event, HostEffect};
use radar_core::view::{Intent, Target};
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

/// The `row:<n>` targets the text form shows, with the line of each.
fn row_targets(shown: &str) -> Vec<(String, String)> {
    shown
        .lines()
        .filter_map(|line| {
            let (_, target) = line.rsplit_once('{')?;
            let target = target.strip_suffix('}')?;
            target.starts_with("row:").then(|| (target.to_string(), line.to_string()))
        })
        .collect()
}

fn select(host: &mut radar_core::Host, target: &str) {
    host.update(Event::Intent(Intent::Select { target: Target::new(target) }));
}

#[test]
fn cached_runs_show_styled_refresh_error_until_success_with_red_main_and_empty_runs() {
    let (w, h) = (160, 40);
    for (path, empty, label) in [
        (API, false, "rows"),
        (WEB, false, "red-main"),
        (API, true, "empty"),
    ] {
        let mut host = host(path, empty);
        let before = crate::tests::shown(&host, w, h);
        fail(&mut host, path);
        let shown = crate::tests::shown(&host, w, h);
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
        // The error is in the error tone, as the view names it.
        assert_eq!(crate::tests::ink_of(&host, "could not load:").as_deref(), Some("bad"));
        insta::assert_snapshot!(format!("runs_cached_refresh_failed_{label}"), shown);
        let p = plugin(&mut host);
        assert!(fetch(p, &Kind::Repo(path.into()), true).iter().any(|e|
            matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.runs")));
        let shown = crate::tests::shown(&host, w, h);
        assert!(shown.contains(ERROR), "retry keeps the retained error");
        assert!(!shown.contains("loading…"));
        let p = plugin(&mut host);
        p.now += 5;
        receive(p, path, Ok(fixture(path, empty)));
        assert_eq!(p.runs[path].at, p.now);
        assert!(p.runs[path].error.is_none());
        let recovered = crate::tests::shown(&host, w, h);
        assert_eq!(recovered, before);
    }
}

#[test]
fn cached_runs_error_is_not_a_row_and_row_targets_keep_red_main_offsets() {
    for path in [API, WEB] {
        let mut host = host(path, false);
        fail(&mut host, path);
        let lead = repo::lead_rows(plugin(&mut host), path).len();
        let shown = crate::tests::shown(&host, 160, 40);
        let rows = row_targets(&shown);
        assert_eq!(
            rows.iter()
                .map(|(id, _)| id)
                .collect::<std::collections::HashSet<_>>()
                .len(),
            lead + 2
        );
        assert!(
            rows.iter().all(|(_, line)| !line.contains(ERROR)),
            "the error line is not a row:\n{shown}"
        );
        let (id, line) = rows.iter().find(|(id, _)| id == &format!("row:{}", lead + 1)).unwrap();
        assert!(line.contains("#1838"), "{line}");
        select(&mut host, id);
        assert_eq!(plugin(&mut host).pages[0].row.get(), lead + 1);
        let fx = host.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(fx.iter().any(|e| matches!(e, HostEffect::Open(url) if url == &format!("https://gitlab.com/{path}/-/pipelines/1"))));
        if lead > 0 {
            let (id, line) = rows.iter().find(|(id, _)| id == "row:0").unwrap();
            assert!(line.contains("unit-test"), "{line}");
            select(&mut host, id);
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

#[test]
fn changed_default_ref_hides_old_rows_before_render_and_keyboard_or_row_activation() {
    let (w, h) = (160, 40);
    let mut host = host(API, false);
    let shown = crate::tests::shown(&host, w, h);
    assert!(shown.contains("#1842"));
    assert!(row_targets(&shown).iter().any(|(id, _)| id == "row:0"));
    let p = plugin(&mut host);
    for data in p.data.values_mut() {
        for project in &mut data.projects {
            if project.full_path == API { project.default_branch = "release/new".into(); }
        }
    }
    // The scope guard also protects input before a fresh paint.
    let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
    let fx = host.update(Event::Intent(Intent::Select { target: Target::new("row:0") }));
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
    // What a second press inside the double-click window sends.
    let fx = host.update(Event::Intent(Intent::Activate { target: Target::new("row:0") }));
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
    let shown = crate::tests::shown(&host, w, h);
    assert!(!shown.contains("#1842") && !shown.contains("#1838"), "{shown}");
    assert!(shown.contains("release/new"));
    let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE)));
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. } | HostEffect::Tracked { .. })));
    receive(plugin(&mut host), API, Ok(fixture(API, false)));
    let shown = crate::tests::shown(&host, w, h);
    assert!(shown.contains("#1842") && shown.contains("release/new"));
    let fx = host.update(Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)));
    assert!(fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
}
