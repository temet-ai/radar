use super::*;
use crate::{demo, join, screen};
use radar_core::Host;
use radar_core::Plugin as _;
use crate::tests::{shown, top_bar_words};
use radar_core::host::{Event, HostEffect};
use radar_core::view::Intent;
use radar_core::plugin::{DEMO_NOW, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn event(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn hosted(p: GithubPlugin) -> Host {
    let mut h = Host::new(vec![Box::new(GithubPlugin::default())], DEMO_NOW).unwrap();
    h.load_config(Some(&format!(
        "[github]\n{}",
        toml::to_string(&p.cfg).unwrap()
    )))
    .unwrap();
    h.plugins[0] = Box::new(p);
    h
}
fn repo_page() -> GithubPlugin {
    let mut p = demo::sample();
    let id = p.repos[0].id.clone();
    p.open_target(Target::Repository(id));
    screen::show_tab(&mut p, 2);
    p
}
fn run_page() -> GithubPlugin {
    let mut p = demo::sample();
    let id = p.repos[0].id.clone();
    p.open_target(Target::Run(id, 501, 2));
    p
}

#[test]
fn actions_pages_read_as_view_data() {
    let mut jobs = run_page();
    screen::show_tab(&mut jobs, 1);
    let mut pr = demo::page();
    screen::show_tab(&mut pr, 2);
    let mut top_bars = Vec::new();
    for (name, p) in [
        ("repository_actions", repo_page()),
        ("run_overview", run_page()),
        ("run_jobs", jobs),
        ("pull_actions", pr),
    ] {
        let h = hosted(p);
        let text = shown(&h, 160, 40);
        assert!(text.contains("Actions") || text.contains("Jobs"));
        insta::assert_snapshot!(name, text);
        top_bars.push((name, top_bar_words(&h)));
    }
    insta::assert_debug_snapshot!("page_top_bar_words", top_bars);
}

#[test]
fn actions_native_subject_is_attempt_qualified_and_rejects_conflicts() {
    let id = crate::model::RepositoryId::parse("acme/api").unwrap();
    let t = Target::Run(id, 501, 2);
    let s = t.subject();
    assert_eq!(join::target(&s), Some(t));
    assert_eq!(s.kind, "workflow-run");
    assert!(join::target(&s.clone().with("attempt", "1")).is_none());
    assert!(join::target(&s.clone().with("run_id", "502")).is_none());
    for url in [
        "https://github.com/acme/api/actions/runs/501/attempts/0",
        "https://github.com/acme/api/actions/runs/501/attempts/02",
        "https://github.com/acme/api/actions/runs/501/attempts/2?token=x",
        "https://github.com/acme/api/actions/runs/501/job/701",
    ] {
        assert!(join::from_url(url).is_none());
    }
    let saved = crate::history::subject(&s).unwrap();
    assert_eq!(saved.key.id, "acme/api/actions/runs/501/attempts/2");
}

#[test]
fn independent_step_rows_include_the_last_step() {
    let mut p = run_page();
    let target = scope(&p).unwrap().url();
    let state = p.actions.runs.get_mut(&target).unwrap();
    let job = &mut state.jobs.as_mut().unwrap().items[0];
    let sample = job.steps[0].clone();
    job.steps = (1..=40)
        .map(|n| WorkflowStep {
            number: n,
            name: format!("step-{n:02}"),
            ..sample.clone()
        })
        .collect();
    state.jobs.as_mut().unwrap().items.truncate(1);
    screen::show_tab(&mut p, 1);
    let mut h = hosted(p);
    h.update(Event::Key(event(KeyCode::End)));
    let text = shown(&h, 80, 24);
    assert!(text.contains("step-40"), "{text}");
    let fx = h.update(Event::Key(event(KeyCode::Char('o'))));
    assert!(fx.iter().any(
        |fx| matches!(fx,HostEffect::Open(url) if url.ends_with("/actions/runs/501/job/701"))
    ));
}

#[test]
fn pr_parent_snapshot_survives_run_back_and_overview_checks_tabs() {
    let mut p = demo::page();
    let original = p.pull_detail.clone().unwrap();
    let checks = p.checks.clone();
    screen::show_tab(&mut p, 2);
    p.open_target(Target::Run(original.repository.clone(), 501, 2));
    assert!(p.pull_detail.is_none());
    screen::key(&mut p, Pane::Middle, crate::chord(KeyCode::Esc));
    assert_eq!(p.pull_detail, Some(original));
    assert_eq!(p.checks, checks);
    screen::show_tab(&mut p, 0);
    assert!(p.pull_detail.is_some());
    screen::show_tab(&mut p, 1);
    assert!(p.checks.is_some());
}

#[test]
fn leaving_invalidates_loading_so_immediate_reentry_can_fetch() {
    let mut p = repo_page();
    let target = scope(&p).unwrap().clone();
    p.actions.lists.remove(&target.url());
    let fx = refresh(&mut p);
    assert_eq!(fx.len(), 1);
    let old = p.actions.lists[&target.url()].request;
    screen::key(&mut p, Pane::Middle, crate::chord(KeyCode::Esc));
    p.open_target(target.clone());
    let next = screen::show_tab(&mut p, 2);
    assert_eq!(next.len(), 1);
    assert!(p.actions.lists[&target.url()].loading);
    assert_ne!(p.actions.lists[&target.url()].request, old);
}

#[test]
fn attempt_picker_is_bounded_and_guards_target_and_generation() {
    let mut p = run_page();
    let target = scope(&p).unwrap().url();
    p.actions.runs.get_mut(&target).unwrap().latest = Some(u64::MAX);
    let Outcome::Handled(fx) = attempts_menu(&mut p) else {
        panic!()
    };
    let Effect::Menu(menu) = &fx[0] else { panic!() };
    assert!(menu.items.len() <= 5);
    assert!(choose_attempt(&mut p, "0").is_empty());
    attempts_menu(&mut p);
    p.generation += 1;
    let fx = choose_attempt(&mut p, "1");
    assert!(matches!(fx.as_slice(), [Effect::Notify(_)]));
}

#[test]
fn default_branch_summary_omits_fork_and_pr_events_and_uses_latest_observation() {
    let mut p = demo::sample();
    let key = Target::Repository(p.repos[0].id.clone()).url();
    let values = &mut p
        .actions
        .lists
        .get_mut(&key)
        .unwrap()
        .value
        .as_mut()
        .unwrap()
        .items;
    let mut latest = values.iter().find(|run| run.id == 501).unwrap().clone();
    latest.id = 800;
    latest.run_number = 100;
    latest.name = Some("latest-safe".into());
    let mut fork = latest.clone();
    fork.id = 801;
    fork.workflow_id = 800;
    fork.name = Some("fork-collision".into());
    fork.head_repository = Some(crate::model::RepositoryId::parse("other/api").unwrap());
    let mut pr = latest.clone();
    pr.id = 802;
    pr.workflow_id = 801;
    pr.name = Some("PR-collision".into());
    pr.event = "pull_request".into();
    values.extend([latest, fork, pr]);
    let words = repository_summary(&p, &p.repos[0])
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(words.contains("latest-safe"));
    assert!(!words.contains("Build and test"));
    assert!(!words.contains("fork-collision"));
    assert!(!words.contains("PR-collision"));
    assert!(words.contains("does not establish overall"));
}

#[test]
fn context_on_a_job_selects_it_without_opening_and_blank_has_no_row_actions() {
    let mut p = run_page();
    screen::show_tab(&mut p, 1);
    let mut h = hosted(p);
    assert!(shown(&h, 160, 40).contains("{actions-job:4}"));
    let fx = h.update(Event::Intent(Intent::Context {
        target: radar_core::view::Target::new("actions-job:4"),
    }));
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Open(_))));
    assert!(h.menu.is_some());
}

#[test]
fn late_run_result_reports_original_outcome_then_application_skip() {
    let mut p = run_page();
    let target = scope(&p).unwrap().clone();
    refresh(&mut p);
    let request = p.actions.runs[&target.url()].request;
    let page = p.page_request;
    let generation = p.generation;
    let value = p.actions.runs[&target.url()].value.clone().unwrap();
    let jobs = p.actions.runs[&target.url()].jobs.clone().unwrap();
    screen::key(&mut p, Pane::Middle, crate::chord(KeyCode::Esc));
    let effects = receive(
        &mut p,
        Answer::Run {
            generation,
            page,
            request,
            target: target.clone(),
            action: history::action("github.actions.run.read", &target),
            result: Ok(RunRead {
                run: value,
                latest: 2,
                jobs: Ok(jobs),
            }),
        },
    );
    assert!(matches!(
        effects.first(),
        Some(Effect::Report(ReadOutcome::Succeeded))
    ));
    assert!(matches!(
        effects.last(),
        Some(Effect::Observe {
            outcome: ReadOutcome::Blocked,
            ..
        })
    ));
    assert!(p.pages.is_empty());
}

#[test]
fn refreshed_order_preserves_run_and_job_step_identity() {
    let mut p = repo_page();
    let target = scope(&p).unwrap().clone();
    let state = p.actions.lists.get_mut(&target.url()).unwrap();
    state.row = 1;
    let selected = selected_run(&p).unwrap().id;
    refresh(&mut p);
    let state = &p.actions.lists[&target.url()];
    let request = state.request;
    let mut value = state.value.clone().unwrap();
    value.items.reverse();
    let answer = Answer::Runs {
        generation: p.generation,
        page: p.page_request,
        request,
        target: target.clone(),
        action: history::action("github.actions.runs.read", &target),
        result: Ok(ListRead::new(value, None)),
        reply: sync::Reply::default(),
    };
    receive(&mut p, answer);
    assert_eq!(selected_run(&p).unwrap().id, selected);
}

#[test]
fn pr_refresh_invalidates_an_abandoned_actions_read() {
    let mut p = demo::page();
    screen::show_tab(&mut p, 2);
    let target = scope(&p).unwrap().clone();
    p.actions.lists.remove(&target.url());
    refresh(&mut p);
    let old = p.actions.lists[&target.url()].request;
    screen::show_tab(&mut p, 0);
    p.refresh_page();
    assert!(!p.actions.lists[&target.url()].loading);
    let fx = screen::show_tab(&mut p, 2);
    assert_eq!(fx.len(), 1);
    assert_ne!(p.actions.lists[&target.url()].request, old);
}

#[test]
fn actions_loading_empty_partial_denied_and_cached_errors_are_explicit() {
    let mut places = vec![];
    for mode in ["loading", "empty", "partial", "denied", "cached_error"] {
        let mut p = repo_page();
        let key = scope(&p).unwrap().url();
        let state = p.actions.lists.get_mut(&key).unwrap();
        match mode {
            "loading" => {
                state.value = None;
                state.loading = true;
            }
            "empty" => state.value.as_mut().unwrap().items.clear(),
            "partial" => {
                state.value.as_mut().unwrap().completion = crate::model::Completion::partial(
                    1,
                    "GitHub page limit reached; more results may exist",
                )
            }
            "denied" => {
                state.value = None;
                state.error = Some("GitHub access denied; check repository permissions".into());
            }
            "cached_error" => state.error = Some("GitHub request failed or timed out".into()),
            _ => unreachable!(),
        }
        places.push((format!("actions_{mode}"), p));
    }
    let mut jobs = run_page();
    screen::show_tab(&mut jobs, 1);
    let key = scope(&jobs).unwrap().url();
    let state = jobs.actions.runs.get_mut(&key).unwrap();
    state.jobs.as_mut().unwrap().completion =
        crate::model::Completion::partial(1, "GitHub rate limit reached; retry after 60s");
    places.push(("jobs_partial".into(), jobs));
    let mut denied = run_page();
    screen::show_tab(&mut denied, 1);
    let key = scope(&denied).unwrap().url();
    let state = denied.actions.runs.get_mut(&key).unwrap();
    state.jobs = None;
    state.jobs_error = Some("GitHub access denied; check repository permissions".into());
    places.push(("jobs_denied".into(), denied));
    let mut top_bars = Vec::new();
    for (name, p) in places {
        let host = hosted(p);
        insta::assert_snapshot!(name.clone(), shown(&host, 160, 40));
        top_bars.push((name, top_bar_words(&host)));
    }
    insta::assert_debug_snapshot!("state_top_bar_words", top_bars);
}

#[test]
fn actions_menu_rows_and_mouse_targets_all_act() {
    use radar_core::check::{Check, Place};
    let make = || Box::new(demo::sample()) as Box<dyn radar_core::Plugin>;
    let mut check = Check::new(&make);
    let repo = Place::screen("github", Pane::Nav).enter().tab(2);
    let gaps = check.places(&[
        repo.clone(),
        repo.clone().enter(),
        repo.enter().tab(1),
        Place::screen("github", Pane::Middle).enter().tab(2),
    ]);
    assert!(gaps.is_empty(), "{}", gaps.join("\n"));
}

#[test]
fn run_popup_rejects_replaced_attempt_on_every_dispatch_path() {
    for choice in ["key", "enter", "mouse", "mouse_after_redraw"] {
        let p = repo_page();
        let target = scope(&p).unwrap().clone();
        let state = &p.actions.lists[&target.url()];
        let mut value = state.value.clone().unwrap();
        value.items[0].run_attempt = 3;
        let answer = Answer::Runs {
            generation: p.generation,
            page: p.page_request,
            request: state.request,
            target: target.clone(),
            action: history::action("github.actions.runs.read", &target),
            result: Ok(ListRead::new(value, None)),
            reply: sync::Reply::default(),
        };
        let mut h = hosted(p);
        h.update(Event::Key(event(KeyCode::Char('x'))));
        shown(&h, 160, 40);
        let menu_index = h
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .position(|i| i.keys == "y")
            .unwrap();
        // A press on the row, by the value the menu view gave it before the read.
        let action = h.menu.as_ref().unwrap().menu.items[menu_index].value.clone();
        h.update(Event::Msg {
            plugin: 0,
            msg: Box::new(crate::Answer::Actions(Box::new(answer))),
        });
        if choice == "mouse_after_redraw" {
            shown(&h, 160, 40);
        }
        let fx = if choice.starts_with("mouse") {
            h.update(Event::Intent(Intent::Invoke { action }))
        } else if choice == "enter" {
            h.menu.as_mut().unwrap().row = menu_index;
            h.update(Event::Key(event(KeyCode::Enter)))
        } else {
            h.update(Event::Key(event(KeyCode::Char('y'))))
        };
        assert!(
            !fx.iter().any(|e| matches!(e, HostEffect::Copy(_))),
            "{choice}"
        );
        assert!(
            h.message
                .as_ref()
                .is_some_and(|m| m.contains("changed") || m.contains("reopen"))
        );
    }
}

#[test]
fn run_popup_survives_unchanged_target_refresh_reordering() {
    let p = repo_page();
    let target = scope(&p).unwrap().clone();
    let state = &p.actions.lists[&target.url()];
    let mut value = state.value.clone().unwrap();
    value.items.reverse();
    let answer = Answer::Runs {
        generation: p.generation,
        page: p.page_request,
        request: state.request,
        target: target.clone(),
        action: history::action("github.actions.runs.read", &target),
        result: Ok(ListRead::new(value, None)),
        reply: sync::Reply::default(),
    };
    let mut h = hosted(p);
    h.update(Event::Key(event(KeyCode::Char('x'))));
    h.update(Event::Msg {
        plugin: 0,
        msg: Box::new(crate::Answer::Actions(Box::new(answer))),
    });
    let fx = h.update(Event::Key(event(KeyCode::Char('y'))));
    assert!(fx.iter().any(
        |e| matches!(e,HostEffect::Copy(url)if url.ends_with("/actions/runs/501/attempts/2"))
    ));
}

#[test]
fn selected_job_identity_never_opens_a_different_run_attempt() {
    let mut p = run_page();
    screen::show_tab(&mut p, 1);
    let target = scope(&p).unwrap().url();
    p.actions
        .runs
        .get_mut(&target)
        .unwrap()
        .jobs
        .as_mut()
        .unwrap()
        .items[0]
        .run_attempt = 1;
    assert!(selected_job(&p).is_none());
    assert!(link(&p).is_some_and(|url| url.ends_with("/actions/runs/501/attempts/2")));
}

/// Every string of a view's JSON that holds a control character.
fn strings_with_controls(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::String(s) if s.chars().any(char::is_control) => out.push(s.clone()),
        serde_json::Value::Array(items) => items.iter().for_each(|v| strings_with_controls(v, out)),
        serde_json::Value::Object(map) => map.values().for_each(|v| strings_with_controls(v, out)),
        _ => {}
    }
}

#[test]
fn untrusted_actions_text_cannot_emit_terminal_controls_in_cells_or_menus() {
    let unsafe_text = "name\u{1b}]52;c;clipboard\u{7}\u{1b}[31m\u{85}\u{9b}31m";
    for page in ["runs", "run", "jobs"] {
        let mut p = if page == "runs" {
            repo_page()
        } else {
            run_page()
        };
        for state in p.actions.lists.values_mut() {
            if let Some(value) = state.value.as_mut() {
                for run in &mut value.items {
                    run.name = Some(unsafe_text.into());
                    run.display_title = unsafe_text.into();
                    run.head_branch = Some(unsafe_text.into());
                    run.event = unsafe_text.into();
                    run.created_at = unsafe_text.into();
                    run.updated_at = unsafe_text.into();
                }
            }
        }
        for state in p.actions.runs.values_mut() {
            if let Some(run) = state.value.as_mut() {
                run.name = Some(unsafe_text.into());
                run.display_title = unsafe_text.into();
                run.head_branch = Some(unsafe_text.into());
                run.event = unsafe_text.into();
                run.created_at = unsafe_text.into();
                run.updated_at = unsafe_text.into();
            }
            if let Some(jobs) = state.jobs.as_mut() {
                for job in &mut jobs.items {
                    job.name = unsafe_text.into();
                    job.started_at = Some(unsafe_text.into());
                    job.completed_at = Some(unsafe_text.into());
                    for step in &mut job.steps {
                        step.name = unsafe_text.into();
                    }
                }
            }
        }
        if page == "jobs" {
            screen::show_tab(&mut p, 1);
        }
        let mut h = hosted(p);
        for menu in [false, true] {
            if menu {
                h.update(Event::Key(event(KeyCode::Char('x'))));
            }
            let text = shown(&h, 160, 40);
            // The host strips the controls from every part's view data
            // (`radar_core::view::clean`), so the cells and the text form
            // hold the words GitHub sent without them.
            assert!(text.contains("name"), "{page} menu={menu}");
            assert!(
                !text.chars().any(|c| c.is_control() && c != '\n'),
                "{page} menu={menu}: the text form holds a control"
            );
            let view = radar_core::view::screen_showing(&h, 160, 40).expect("the screen");
            let mut controls = Vec::new();
            strings_with_controls(&serde_json::to_value(&view).unwrap(), &mut controls);
            assert!(controls.is_empty(), "{page} menu={menu}: cells hold controls: {controls:?}");
            if let Some(open) = &h.menu {
                for item in &open.menu.items {
                    assert!(
                        !format!("{}{}{}", item.label, item.hint, item.value)
                            .chars()
                            .any(|c| c.is_control()),
                        "{page} menu item {:?}",
                        item.label
                    );
                }
            }
        }
    }
}

#[test]
fn every_listed_action_has_its_own_named_id() {
    use crate::chord_for;
    use radar_core::view::Intent;
    let mut seen: Vec<(&str, Vec<GithubPlugin>, bool)> = Vec::new();
    // (surface, states, settings?) — every state a surface lists rows in.
    let mut browse = Vec::new();
    for at in 0..3 {
        let mut p = demo::sample();
        p.group_at = at.min(p.cfg.groups.len());
        browse.push(p);
    }
    seen.push(("browse", browse, false));
    let mut pages = vec![demo::page()];
    for tab in 0..3 {
        let mut p = demo::page();
        screen::show_tab(&mut p, tab);
        pages.push(p);
    }
    let mut repo = demo::sample();
    let id = repo.repos[0].id.clone();
    repo.open_target(Target::Repository(id));
    pages.push(repo);
    seen.push(("page", pages, false));
    seen.push(("actions", vec![repo_page(), run_page()], false));
    let mut connection = Vec::new();
    let mut repositories = Vec::new();
    for row in 0..4 {
        let mut p = demo::sample();
        p.connection_row = row;
        crate::settings::select_repository_row(&mut p, row);
        connection.push(p);
        let mut p = demo::sample();
        crate::settings::select_repository_row(&mut p, row);
        repositories.push(p);
    }
    seen.push(("connection", connection, true));
    seen.push(("repositories", repositories, true));
    for (surface, states, settings) in seen {
        // The rows a surface lists over all its states, by id: one id is one
        // key, and none is a fallback.
        let mut ids: std::collections::BTreeMap<String, String> = Default::default();
        for p in &states {
            let rows = if settings {
                crate::settings::keys(p, surface)
            } else {
                crate::screen::keys(p, Pane::Middle)
            };
            for (id, b) in rows.iter().filter(|(_, b)| !b.is_heading()) {
                let id = id.clone();
                assert!(
                    id.starts_with("github."),
                    "{surface}: row `{}` ({}) has no named id: {id}",
                    b.what,
                    b.keys
                );
                let key = ids.entry(id.clone()).or_insert_with(|| b.keys.clone());
                // Rows that share an id are one action and share its key.
                assert_eq!(*key, b.keys, "{surface}: {id} names two keys");
                let invoke = Intent::Invoke { action: id.clone() };
                assert_eq!(
                    chord_for(&rows, &invoke),
                    radar_core::view::Chord::parse(&b.keys),
                    "{surface}: {id} does not resolve to the row's key"
                );
            }
        }
        // Two different keys never share an id (checked above) and two
        // different ids never share a key on one screen of rows.
        let mut by_key: std::collections::BTreeMap<(&str, &String), &String> = Default::default();
        for (id, key) in &ids {
            let on = id.rsplit_once('.').map_or("", |(on, _)| on);
            if let Some(other) = by_key.insert((on, key), id) {
                assert!(
                    key == "↑ ↓" || key == "⏎",
                    "{surface}: {other} and {id} both use {key}"
                );
            }
        }
    }
}

fn mark(open: u64) -> Marker {
    Marker {
        open,
        latest: Some("2026-10-13T09:00:00Z".into()),
    }
}
fn tag(value: &str) -> Validator {
    Validator {
        etag: Some(value.into()),
        last_modified: None,
    }
}
fn spawned(effects: &[Effect]) -> usize {
    effects
        .iter()
        .filter(|e| matches!(e, Effect::SpawnAction { .. }))
        .count()
}

/// The repository's Actions tab, its runs read `ago` seconds ago while the
/// repository was at marker 3, and nothing running.
fn read_ago(ago: i64) -> (GithubPlugin, String) {
    let mut p = repo_page();
    let key = scope(&p).unwrap().url();
    p.repos[0].sync.marker = Some(mark(3));
    let at = p.now - ago;
    let state = p.actions.lists.get_mut(&key).unwrap();
    for run in &mut state.value.as_mut().unwrap().items {
        run.status = "completed".into();
    }
    state.marker = Some(mark(3));
    state.at = Some(at);
    state.ask = Some(RunsAsk {
        branch: None,
        per_page: 50,
    });
    state.tags = vec![tag("\"runs\"")];
    (p, key)
}

#[test]
fn reopening_the_runs_of_a_repository_that_has_not_moved_asks_nothing() {
    let (mut p, _) = read_ago(60);
    assert_eq!(
        spawned(&ensure(&mut p)),
        0,
        "held, current: no request at all"
    );
    let (mut p, _) = read_ago(sync::RUNS_EVERY - 1);
    assert_eq!(spawned(&ensure(&mut p)), 0);
    // Past that a push may have run something: asked, and a 304 is free.
    let (mut p, _) = read_ago(sync::RUNS_EVERY);
    assert_eq!(spawned(&ensure(&mut p)), 1);
}

#[test]
fn a_repository_that_moved_is_asked_when_its_tab_opens() {
    let (mut p, _) = read_ago(60);
    p.repos[0].sync.marker = Some(mark(4));
    assert_eq!(spawned(&ensure(&mut p)), 1);
}

#[test]
fn runs_still_going_are_asked_again_after_a_minute_not_a_quarter_hour() {
    let (mut p, key) = read_ago(sync::EVERY - 1);
    p.actions
        .lists
        .get_mut(&key)
        .unwrap()
        .value
        .as_mut()
        .unwrap()
        .items[0]
        .status = "in_progress".into();
    assert_eq!(spawned(&ensure(&mut p)), 0);
    let (mut p, key) = read_ago(sync::EVERY);
    p.actions
        .lists
        .get_mut(&key)
        .unwrap()
        .value
        .as_mut()
        .unwrap()
        .items[0]
        .status = "in_progress".into();
    assert_eq!(spawned(&ensure(&mut p)), 1);
}

#[test]
fn with_no_moved_state_to_compare_runs_are_asked_again_after_a_minute() {
    let (mut p, key) = read_ago(sync::EVERY - 1);
    p.repos[0].sync.marker = None;
    p.actions.lists.get_mut(&key).unwrap().marker = None;
    assert_eq!(spawned(&ensure(&mut p)), 0);
    let (mut p, key) = read_ago(sync::EVERY);
    p.repos[0].sync.marker = None;
    p.actions.lists.get_mut(&key).unwrap().marker = None;
    assert_eq!(spawned(&ensure(&mut p)), 1);
}

fn answer_of(p: &GithubPlugin, key: &str, read: ListRead, reply: sync::Reply) -> Answer {
    let target = scope(p).unwrap().clone();
    assert_eq!(target.url(), key);
    Answer::Runs {
        generation: p.generation,
        page: p.page_request,
        request: p.actions.lists[key].request,
        target: target.clone(),
        action: history::action("github.actions.runs.read", &target),
        result: Ok(read),
        reply,
    }
}

#[test]
fn a_304_keeps_the_runs_and_only_restamps_them() {
    let (mut p, key) = read_ago(sync::RUNS_EVERY);
    assert_eq!(spawned(&ensure(&mut p)), 1);
    let held = p.actions.lists[&key].value.clone();
    let ask = p.actions.lists[&key].ask.clone().unwrap();
    let read = ListRead {
        runs: Fresh::Same,
        pull: None,
        ask,
        marker: Some(mark(3)),
    };
    let answer = answer_of(&p, &key, read, sync::Reply::default());
    let effects = receive(&mut p, answer);
    let state = &p.actions.lists[&key];
    assert_eq!(state.value, held, "nothing replaced, nothing parsed");
    assert_eq!(
        state.tags,
        [tag("\"runs\"")],
        "the validator still describes them"
    );
    assert_eq!(state.at, Some(p.now), "current again");
    assert!(!state.loading && state.error.is_none());
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::PersistCache)),
        "an unchanged answer writes nothing"
    );
    assert_eq!(spawned(&ensure(&mut p)), 0, "and the tab reopens free");
}

#[test]
fn new_runs_replace_the_rows_and_their_validator_and_are_written_to_the_cache() {
    let (mut p, key) = read_ago(sync::RUNS_EVERY);
    ensure(&mut p);
    let mut runs = p.actions.lists[&key].value.clone().unwrap();
    runs.items.truncate(1);
    let read = ListRead {
        runs: Fresh::New(runs, vec![tag("\"two\"")]),
        pull: None,
        ask: RunsAsk {
            branch: None,
            per_page: 60,
        },
        marker: Some(mark(4)),
    };
    let answer = answer_of(&p, &key, read, sync::Reply::default());
    let effects = receive(&mut p, answer);
    let state = &p.actions.lists[&key];
    assert_eq!(state.value.as_ref().unwrap().items.len(), 1);
    assert_eq!(
        (state.tags.clone(), state.marker.clone()),
        (vec![tag("\"two\"")], Some(mark(4)))
    );
    assert_eq!(state.ask.as_ref().unwrap().per_page, 60);
    assert!(effects.iter().any(|e| matches!(e, Effect::PersistCache)));
}

#[test]
fn what_a_runs_read_spent_and_was_refused_counts_against_the_round() {
    let (mut p, key) = read_ago(sync::RUNS_EVERY);
    ensure(&mut p);
    let ask = p.actions.lists[&key].ask.clone().unwrap();
    let read = ListRead {
        runs: Fresh::Same,
        pull: None,
        ask,
        marker: Some(mark(3)),
    };
    let reply = sync::Reply {
        stop: Some(crate::model::ApiError {
            kind: crate::model::ApiErrorKind::RateLimit,
            status: Some(403),
            retry_after_secs: Some(90),
            rate_limit_reset: None,
        }),
        traffic: crate::model::Traffic {
            requests: 1,
            ..Default::default()
        },
        ..Default::default()
    };
    let answer = answer_of(&p, &key, read, reply);
    receive(&mut p, answer);
    assert_eq!(p.sync.traffic.requests, 1);
    assert_eq!(
        p.sync.wait_until(p.now),
        Some(p.now + 90),
        "the Actions tab waits too"
    );
}

#[test]
fn the_runs_ask_is_sized_to_the_view_and_names_a_pull_requests_branch() {
    for (rows, per_page) in [(0, 50), (10, 30), (24, 50), (40, 80), (60, 100), (500, 100)] {
        assert_eq!(runs_page(rows), per_page, "{rows} rows");
    }
    let pull = demo::pull(&RepositoryId::parse("acme/api").unwrap(), 42, "x");
    assert_eq!(
        ask_for(Some(&pull), 40),
        RunsAsk {
            branch: Some("refresh-ownership".into()),
            per_page: 40
        }
    );
    assert_eq!(
        ask_for(None, 40).branch,
        None,
        "the repository tab shows every branch"
    );
    let mut unnamed = pull;
    unnamed.head_branch.clear();
    assert_eq!(ask_for(Some(&unnamed), 40).branch, None);
}

#[test]
fn a_validator_is_sent_only_for_the_ask_it_came_with() {
    let ask = RunsAsk {
        branch: None,
        per_page: 50,
    };
    let held = Some((ask.clone(), vec![tag("\"a\"")]));
    assert_eq!(tags_for(held.clone(), &ask), [tag("\"a\"")]);
    let other_size = RunsAsk {
        per_page: 80,
        ..ask.clone()
    };
    assert!(
        tags_for(held.clone(), &other_size).is_empty(),
        "another page size is another URL"
    );
    let other_branch = RunsAsk {
        branch: Some("topic".into()),
        ..ask
    };
    assert!(tags_for(held, &other_branch).is_empty());
    assert!(tags_for(None, &other_branch).is_empty());
}

#[test]
fn the_runs_survive_a_restart_and_the_tab_opens_from_them_for_free() {
    let (mut p, key) = read_ago(120);
    let id = p.repos[0].id.clone();
    p.sync.dirty = false;
    let cache = p.snapshot().unwrap();
    assert_eq!(cache["runs"].as_array().unwrap().len(), 1);
    let mut fresh = GithubPlugin::default();
    fresh.apply_config(p.cfg.clone());
    fresh.restore(cache.clone());
    fresh.now = p.now;
    assert_eq!(
        fresh.actions.lists[&key].value, p.actions.lists[&key].value,
        "drawn before any read"
    );
    fresh.repos[0].sync.marker = Some(mark(3));
    let opened = fresh.open_target(Target::Repository(id.clone()));
    assert_eq!(spawned(&opened), 0);
    assert_eq!(
        spawned(&screen::show_tab(&mut fresh, 2)),
        0,
        "not moved: no request"
    );
    // The repository moved before the first round said so: asked, with its tag.
    let mut moved = GithubPlugin::default();
    moved.apply_config(p.cfg.clone());
    moved.restore(cache.clone());
    moved.now = p.now;
    moved.repos[0].sync.marker = Some(mark(4));
    moved.open_target(Target::Repository(id.clone()));
    assert_eq!(spawned(&screen::show_tab(&mut moved, 2)), 1);
    assert_eq!(
        moved.actions.lists[&key].tags,
        [tag("\"runs\"")],
        "kept to ask with"
    );
    // Rows of another repository never fill this one's tab.
    let mut forged = cache;
    forged["runs"][0]["value"]["items"][0]["repository"] =
        serde_json::json!({"owner": "acme", "name": "docs"});
    let mut other = GithubPlugin::default();
    other.apply_config(p.cfg.clone());
    other.restore(forged);
    assert!(other.actions.lists.is_empty());
}

#[test]
fn the_cache_keeps_the_runs_of_the_most_recently_read_repositories_only() {
    let mut p = demo::sample();
    for i in 0..30 {
        let id = RepositoryId::parse(&format!("acme/r{i}")).unwrap();
        p.repos.push(crate::RepoState::new(id.clone()));
        p.actions.lists.insert(
            Target::Repository(id).url(),
            Runs {
                value: Some(Collection {
                    items: vec![],
                    completion: crate::model::Completion::complete(1),
                }),
                ask: Some(RunsAsk {
                    branch: None,
                    per_page: 50,
                }),
                at: Some(1000 + i),
                ..Runs::default()
            },
        );
    }
    let kept = saved(&p);
    assert_eq!(kept.len(), SAVED_MAX);
    assert_eq!(kept[0].id.name, "r29", "newest first");
}

#[test]
fn a_settings_save_keeps_the_runs_of_what_is_still_configured_with_the_same_token() {
    let (mut p, key) = read_ago(60);
    let pull_key = Target::Pull(p.repos[0].id.clone(), 42).url();
    assert!(
        p.actions.lists.contains_key(&pull_key),
        "the demo seeded a pull request's runs"
    );
    p.apply_config(p.cfg.clone());
    assert!(
        p.actions.lists[&key].value.is_some(),
        "the repository's runs are kept"
    );
    assert!(
        !p.actions.lists.contains_key(&pull_key),
        "a page's rows are not"
    );
    assert!(!p.actions.lists[&key].loading);
    let mut other = p.cfg.clone();
    other.token = "env:ANOTHER".into();
    p.apply_config(other);
    assert!(p.actions.lists.is_empty(), "another credential reads again");
}

#[test]
fn a_bounded_list_says_so_and_is_not_an_incomplete_read() {
    let mut p = repo_page();
    let key = scope(&p).unwrap().url();
    p.actions
        .lists
        .get_mut(&key)
        .unwrap()
        .value
        .as_mut()
        .unwrap()
        .completion = crate::model::Completion::bounded(1, 2);
    assert!(!incomplete(&p));
    let text = shown(&hosted(p), 160, 40);
    assert!(
        text.contains("newest 2 shown; older ones are not read"),
        "{text}"
    );
    assert!(!text.contains("Partial result"), "{text}");
}
