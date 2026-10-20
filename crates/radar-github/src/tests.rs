use super::*;
use radar_core::check::{Check as RowCheck, Place};
use radar_core::host::{Event, Hit, HostEffect, MouseKind, View};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::shell::{to_svg, to_text};
use model::Completion;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
pub(crate) fn hosted(p: GithubPlugin) -> radar_core::Host {
    let mut host = radar_core::Host::new(
        vec![Box::new(GithubPlugin::default())],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    let mut table = toml::Table::new();
    table.insert("github".into(), toml::Value::try_from(&p.cfg).unwrap());
    let config = toml::to_string(&table).unwrap();
    host.load_config(Some(&config)).unwrap();
    host.plugins[0] = Box::new(p);
    host
}
fn settings_host(section: &str) -> radar_core::Host {
    let mut host = hosted(demo::sample());
    host.update(Event::Key(key(KeyCode::Char(radar_core::host::SETTINGS_KEY))));
    host.section = host
        .sections
        .iter()
        .position(|(_, s)| s.group == "github" && s.id == section)
        .unwrap();
    host.on_heading = false;
    host.focus = Pane::Middle;
    host
}
/// The connection section with the cursor on its server row, past the
/// three sign-in choices.
fn connection_host() -> radar_core::Host {
    let mut host = settings_host("connection");
    for _ in 0..3 {
        host.update(Event::Key(key(KeyCode::Down)));
    }
    host
}
pub(crate) fn exported(name: &str, host: &radar_core::Host, w: u16, h: u16) {
    if let Ok(dir) = std::env::var("RADAR_GITHUB_RENDER_DIR") {
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(format!("{dir}/{name}_{w}x{h}.svg"), to_svg(host, w, h)).unwrap();
        std::fs::write(format!("{dir}/{name}_{w}x{h}.txt"), to_text(host, w, h)).unwrap();
    }
}

#[test]
fn rendered_screens_and_settings() {
    let mut places = vec![
        ("browse", hosted(demo::sample())),
        ("overview", hosted(demo::page())),
        ("connection", settings_host("connection")),
        ("repositories", settings_host("repositories")),
    ];
    let mut checks = demo::page();
    checks.pages.last_mut().unwrap().tab = 1;
    places.push(("checks", hosted(checks)));
    let mut unknown = demo::page();
    unknown.pages.last_mut().unwrap().tab = 1;
    unknown.checks = None;
    unknown.checks_error = Some("GitHub access denied; check repository permissions".into());
    places.push(("unknown_checks", hosted(unknown)));
    places.push(("deleted_source_checks", hosted(deleted_source_checks())));
    let mut menu = hosted(demo::sample());
    menu.update(Event::Key(key(KeyCode::Char('x'))));
    places.push(("actions", menu));
    let mut repo_page = demo::sample();
    repo_page.open_target(Target::Repository(repo_page.repos[0].id.clone()));
    places.push(("repository_overview", hosted(repo_page)));
    let mut repo_pulls = demo::sample();
    repo_pulls.open_target(Target::Repository(repo_pulls.repos[0].id.clone()));
    screen::show_tab(&mut repo_pulls, 1);
    places.push(("repository_pulls", hosted(repo_pulls)));
    let mut empty = demo::sample();
    empty.repo_at = 1;
    places.push(("empty", hosted(empty)));
    let mut loading = demo::sample();
    loading.repos[0].loading = true;
    loading.repos[0].repository = None;
    loading.repos[0].pulls = None;
    places.push(("loading", hosted(loading)));
    let mut cached = demo::sample();
    cached.repos[0].error = Some("GitHub resource is missing or inaccessible".into());
    cached.connection_check = Some(Check::Failed {
        said: "repository read failed".into(),
        ago: None,
    });
    places.push(("cached_error", hosted(cached)));
    let mut partial = demo::sample();
    partial.repos[0].pulls.as_mut().unwrap().completion =
        Completion::partial(1, "GitHub rate limit reached; retry after 60 seconds");
    partial.connection_check = Some(Check::Failed {
        said: "repository read incomplete".into(),
        ago: None,
    });
    places.push(("partial", hosted(partial)));
    for (name, host) in places {
        for (w, h) in [(80, 24), (160, 40)] {
            let text = to_text(&host, w, h);
            exported(name, &host, w, h);
            insta::assert_snapshot!(format!("{name}_{w}x{h}"), text);
        }
    }
}

#[test]
fn every_menu_row_and_mouse_target_acts() {
    let make = || Box::new(demo::sample()) as Box<dyn Plugin>;
    let mut check = RowCheck::new(&make);
    let gaps = check.places(&[
        Place::screen("github", Pane::Nav),
        Place::screen("github", Pane::Nav).down(1),
        Place::screen("github", Pane::Middle),
        Place::screen("github", Pane::Detail),
        Place::screen("github", Pane::Middle).down(2).enter(),
        Place::screen("github", Pane::Middle).down(2).enter().tab(1),
        Place::screen("github", Pane::Middle).enter(),
        Place::screen("github", Pane::Middle).enter().tab(1),
        Place::section("connection"),
        Place::section("connection").down(1),
        Place::section("connection").down(2),
        Place::section("connection").down(3),
        Place::section("connection").down(4),
        Place::section("connection").down(5),
        Place::section("connection").down(6),
        Place::section("connection").down(7),
        Place::section("connection").down(8),
        Place::section("repositories"),
        Place::section("repositories").down(1),
    ]);
    assert!(gaps.is_empty(), "{}", gaps.join("\n"));
    assert!(check.checked > 50);
}

#[test]
fn starts_once_and_repeated_refresh_is_coalesced() {
    let mut p = demo::sample();
    p.started = false;
    // With a token, one question for every repository: what moved.
    assert_eq!(
        p.tick(10)
            .iter()
            .filter(|e| matches!(e, Effect::SpawnAction { .. }))
            .count(),
        1
    );
    assert!(p.tick(11).is_empty());
    assert!(
        !p.refresh_all()
            .iter()
            .any(|e| matches!(e, Effect::SpawnAction { .. }))
    );
    let mut p = demo::page();
    assert_eq!(p.refresh_page().len(), 1);
    assert!(
        !p.refresh_page()
            .iter()
            .any(|e| matches!(e, Effect::SpawnAction { .. }))
    );
    assert!(
        p.snapshot()
            .is_some_and(|cache| cache["repos"].as_array().unwrap().len() == 2)
    );
}

fn repository_answer(p: &GithubPlugin, items: Vec<PullRequest>, complete: bool) -> Answer {
    let repo = &p.repos[0];
    Answer::Repository {
        generation: p.generation,
        request: repo.request,
        id: repo.id.clone(),
        action: history::action(
            "github.repository.read",
            &Target::Repository(repo.id.clone()),
        ),
        repository: Some(Ok(Fresh::New(repo.repository.clone().unwrap(), None))),
        pulls: Ok(Fresh::New(
            Collection {
                items,
                completion: if complete {
                    Completion::complete(1)
                } else {
                    Completion::partial(1, "page limit reached")
                },
            },
            vec![],
        )),
        reply: sync::Reply::default(),
    }
}

#[test]
fn refresh_preserves_selected_number_and_partial_does_not_pass() {
    let mut p = demo::sample();
    p.last_sync = None;
    let mut items = p.repos[0].pulls.as_ref().unwrap().items.clone();
    items.reverse();
    p.receive(repository_answer(&p, items, false));
    assert_eq!(p.selected_pull().unwrap().number, 42);
    assert!(p.last_sync.is_none());
    assert!(p.connection_check.is_none());
    assert!(p.top_right("github").unwrap().contains("incomplete"));
}

#[test]
fn removed_repository_and_changed_auth_discard_old_answers() {
    for token in ["", "env:OTHER_TOKEN"] {
        let mut p = demo::sample();
        let answer = repository_answer(&p, vec![], true);
        p.apply_config(Settings {
            token: token.into(),
            groups: vec![],
            repositories: vec!["acme/docs".into()],
            ..Settings::default()
        });
        let effects = p.receive(answer);
        assert!(p.repos[0].repository.is_none());
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, Effect::Report(ReadOutcome::Succeeded)))
        );
        assert!(effects.iter().any(|e| matches!(e, Effect::Observe { .. })));
    }
}

fn pull_answer(p: &GithubPlugin, pull: PullRequest) -> Answer {
    let target = Target::Pull(pull.repository.clone(), pull.number);
    Answer::Pull {
        generation: p.generation,
        request: p.page_request,
        action: history::action("github.pull-request.read", &target),
        target,
        checks: Ok(Fresh::New(demo::checks(&pull), CheckTags::new())),
        pull: Box::new(Ok(Fresh::New(pull, None))),
        reply: sync::Reply::default(),
    }
}

#[test]
fn navigation_and_reopen_fence_late_pull_answers() {
    let mut p = demo::page();
    let old = pull_answer(&p, p.pull_detail.clone().unwrap());
    screen::key(&mut p, Pane::Middle, chord(KeyCode::Esc));
    let same = p.selected_pull().unwrap().clone();
    p.open_target(Target::Pull(same.repository.clone(), same.number));
    p.receive(old);
    assert!(p.pull_detail.is_none());
    assert!(p.page_loading);
}

#[test]
fn a_new_head_never_uses_old_checks_when_the_check_read_fails() {
    let mut p = demo::page();
    let mut pull = p.pull_detail.clone().unwrap();
    pull.head_sha = "b".repeat(40);
    let mut answer = pull_answer(&p, pull);
    if let Answer::Pull { checks, .. } = &mut answer {
        *checks = Err("checks unavailable".into());
    }
    p.receive(answer);
    assert!(p.checks.is_none());
    assert!(p.checks_error.is_some());
}

fn deleted_source_checks() -> GithubPlugin {
    let mut p = demo::page();
    p.pages.last_mut().unwrap().tab = 1;
    let pull = p.pull_detail.as_mut().unwrap();
    pull.head_repository = None;
    let base = pull.repository.clone();
    let checks = p.checks.as_mut().unwrap();
    checks.head_repository = None;
    checks
        .check_runs
        .items
        .retain(|check| check.repository == base);
    checks
        .statuses
        .items
        .retain(|status| status.repository == base);
    checks.check_runs.completion =
        Completion::partial(1, "source repository unavailable; its checks are unknown");
    checks.statuses.completion =
        Completion::partial(1, "source repository unavailable; its statuses are unknown");
    checks.errors = vec!["source repository is unavailable; base results are shown".into()];
    p
}

#[test]
fn fork_checks_show_repository_provenance_and_deleted_source_keeps_base_results() {
    let mut p = demo::page();
    p.pages.last_mut().unwrap().tab = 1;
    let text = to_text(&hosted(p), 160, 40);
    assert!(text.contains("unit tests  (acme/api)"));
    assert!(text.contains("platform checks  (alice/api)"));
    assert!(text.contains("preview deployment  (acme/api)"));
    assert!(text.contains("Merge-ref workflows may differ"));
    let text = to_text(&hosted(deleted_source_checks()), 160, 40);
    assert!(text.contains("source repository  unavailable"));
    assert!(text.contains("unit tests  (acme/api)"));
    assert!(!text.contains("platform checks"));
    assert!(text.contains("Partial or unavailable check runs"));
    assert!(text.contains("read incomplete"));
}

#[test]
fn links_cannot_replay_after_same_target_refresh_or_replacement() {
    let mut p = demo::page();
    p.links.borrow_mut().push(Link {
        text: "old link".into(),
        url: "https://example.com/old".into(),
    });
    screen::key(&mut p, Pane::Middle, chord(KeyCode::Enter));
    p.refresh_page();
    assert!(
        !screen::follow_link(&mut p, "0")
            .iter()
            .any(|e| matches!(e, Effect::Show(_)))
    );
    p.links.borrow_mut().push(Link {
        text: "old link".into(),
        url: "https://example.com/old".into(),
    });
    screen::key(&mut p, Pane::Middle, chord(KeyCode::Enter));
    let mut pull = p.pull_detail.clone().unwrap();
    pull.body = Some("new description".into());
    p.receive(pull_answer(&p, pull));
    assert!(p.links.borrow().is_empty());
    assert!(
        !screen::follow_link(&mut p, "0")
            .iter()
            .any(|e| matches!(e, Effect::Show(_)))
    );
}

#[test]
fn host_typing_paste_validation_save_and_cancel() {
    let mut host = connection_host();
    host.update(Event::Key(key(KeyCode::Down)));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(host.plugins[0].capturing());
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    host.update(Event::Paste("raw secret value".into()));
    let text = to_text(&host, 160, 40);
    assert!(text.to_lowercase().contains("as you type"), "{text}");
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(host.plugins[0].capturing());
    assert_eq!(
        host.plugins[0].config().unwrap()["token"].as_str(),
        Some("env:GITHUB_TOKEN")
    );
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    host.update(Event::Paste("env:SECOND_TOKEN".into()));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(!host.plugins[0].capturing());
    assert_eq!(
        host.plugins[0].config().unwrap()["token"].as_str(),
        Some("env:SECOND_TOKEN")
    );
    assert!(host.menu.is_none());
    assert!(
        host.plugins[0].tick(30).is_empty(),
        "saving a token must not trigger repository reads on the next tick"
    );
    host.update(Event::Key(key(KeyCode::Enter)));
    host.update(Event::Paste("QH".into()));
    host.update(Event::Key(key(KeyCode::Esc)));
    assert_eq!(
        host.plugins[0].config().unwrap()["token"].as_str(),
        Some("env:SECOND_TOKEN")
    );
    assert!(!host.quit);
}

#[test]
fn the_server_row_edits_the_api_address_and_a_default_is_left_out_of_the_file() {
    let mut host = connection_host();
    assert!(host.plugins[0].config().unwrap().get("api").is_none());
    let text = to_text(&host, 160, 40);
    assert!(text.contains("https://api.github.com"), "{text}");
    assert!(text.contains("[github] api"), "{text}");
    assert!(text.to_lowercase().contains("accepts"), "{text}");
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(host.plugins[0].capturing(), "the server row is typed into");
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    host.update(Event::Paste("ftp://nowhere".into()));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(host.plugins[0].capturing(), "a bad address is not kept");
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    host.update(Event::Paste("http://127.0.0.1:8123/".into()));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(!host.plugins[0].capturing());
    assert_eq!(
        host.plugins[0].config().unwrap()["api"].as_str(),
        Some("http://127.0.0.1:8123")
    );
    assert!(to_text(&host, 160, 40).contains("http://127.0.0.1:8123"));
    host.update(Event::Key(key(KeyCode::Enter)));
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::CONTROL,
    )));
    host.update(Event::Paste("https://api.github.com".into()));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(host.plugins[0].config().unwrap().get("api").is_none());
}

#[test]
fn add_repository_accepts_exact_url_through_host() {
    let mut host = settings_host("repositories");
    host.update(Event::Key(key(KeyCode::Enter)));
    host.update(Event::Paste("github.com/acme/extra".into()));
    host.update(Event::Key(key(KeyCode::Enter)));
    let cfg = host.plugins[0].config().unwrap();
    assert_eq!(
        cfg["groups"][0]["repositories"].as_array().unwrap().len(),
        3
    );
    assert_eq!(
        cfg["groups"][0]["repositories"].as_array().unwrap()[2].as_str(),
        Some("acme/extra")
    );
}

mod interaction_motion {
    use super::*;

    #[test]
    fn repository_tree_fold_intents_verify_through_the_real_host() {
        use radar_core::motion::transition::MotionContext;
        use radar_core::view::{Intent, Node, PaneId, TreeRow};

        // A single host proves the owner/host dispatch boundary. Mapping, stale
        // identity, labels and raw-key rules are covered by pure owner tests.
        let mut p = screen::neutral::tests::tree_fixture();
        p.tree_at = Some(screen::TreeItem::Group(0));
        let mut host = hosted(p);
        host.focus = Pane::Middle;
        host.colours.cfg.motion = "full".into();
        host.set_motion_enabled(true);
        host.set_view_room(80, 24);
        let tree = |host: &radar_core::Host| {
            let view = host.screen_view(0, "github").unwrap();
            let Node::Tree(tree) = &view.pane(PaneId::Middle).unwrap().body else { panic!("real repository tree") };
            tree.clone()
        };
        let group = vec!["group:Watch".to_string()];
        let repo = vec!["group:Watch".to_string(), "repo:ankh-morpork/watch".to_string()];
        let group_target = TreeRow::target_of(&group);
        let initial = tree(&host);
        assert!(host.update(Event::Intent(Intent::Fold { target: group_target.clone(), open: false })).is_empty());
        let fold = host.take_motion_fold().expect("owner Fold must produce a verified host transition");
        assert_eq!(fold.path, group);
        assert_eq!(fold.pane, PaneId::Middle);
        assert_eq!(fold.slot, "body");
        assert!(fold.before_open && !fold.after_open);
        assert_eq!(fold.context, MotionContext::Screen {
            plugin: "github".into(), screen: "github".into(), page: None, depth: 0, tab: None,
        });
        assert_eq!(tree(&host).rows, initial.rows);

        host.update(Event::Intent(Intent::Activate { target: group_target }));
        let fold = host.take_motion_fold().expect("group activation keeps its normal fold behavior");
        assert_eq!(fold.path, group);
        assert!(!fold.before_open && fold.after_open);
        assert_eq!(host.plugins[0].depth("github"), 0);

        let repo_target = TreeRow::target_of(&repo);
        host.update(Event::Intent(Intent::Select { target: repo_target.clone() }));
        assert!(host.motion_fold().is_none());
        host.update(Event::Intent(Intent::Fold { target: repo_target.clone(), open: false }));
        assert_eq!(host.take_motion_fold().unwrap().path, repo);
        assert_eq!(tree(&host).rows, initial.rows);
        host.update(Event::Intent(Intent::Fold { target: repo_target.clone(), open: false }));
        assert!(host.motion_fold().is_none(), "accepted no-op must not animate");
        host.update(Event::Key(key(KeyCode::Char('-'))));
        let fold = host.take_motion_fold().expect("closed repository '-' closes its parent");
        assert_eq!(fold.path, group);
        assert!(!tree(&host).open.contains(&group));
        assert_eq!(tree(&host).selected, group);

        host.update(Event::Intent(Intent::Fold { target: TreeRow::target_of(&group), open: true }));
        assert!(host.take_motion_fold().unwrap().after_open);
        host.update(Event::Intent(Intent::Fold { target: repo_target, open: true }));
        assert!(host.take_motion_fold().unwrap().after_open);
        let mut pull = repo.clone();
        pull.push("pull:13".into());
        host.update(Event::Intent(Intent::Select { target: TreeRow::target_of(&pull) }));
        assert!(host.plugins[0].subject("github", Pane::Middle).unwrap().id.ends_with("/13"));
        host.update(Event::Key(key(KeyCode::Char('-'))));
        let fold = host.take_motion_fold().expect("PR '-' closes its repository");
        assert_eq!(fold.path, repo);
        assert_eq!(tree(&host).selected, repo);
        assert!(!tree(&host).open.contains(&repo));
        assert_eq!(tree(&host).rows, initial.rows);
    }
}

#[test]
fn mouse_rows_blank_space_double_click_and_context_selection() {
    let mut host = hosted(demo::sample());
    let _ = to_text(&host, 160, 40);
    let row = host
        .hits
        .borrow()
        .iter()
        .find_map(|(r, hit)| matches!(hit, Hit::Plugin { id, .. } if id == "group:github.com/acme\u{1f}repo:acme/api\u{1f}pull:43").then_some(*r))
        .unwrap();
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: row.x + 2,
        row: row.y,
        at_ms: 100,
    });
    assert!(
        host.plugins[0]
            .subject("github", Pane::Middle)
            .unwrap()
            .id
            .ends_with("/43")
    );
    let _ = to_text(&host, 160, 40);
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: row.x + 2,
        row: row.y + 4,
        at_ms: 600,
    });
    assert!(
        host.plugins[0]
            .subject("github", Pane::Middle)
            .unwrap()
            .id
            .ends_with("/43")
    );
    host.update(Event::Mouse {
        kind: MouseKind::Context,
        column: row.x + 2,
        row: row.y,
        at_ms: 1100,
    });
    assert_eq!(host.plugins[0].depth("github"), 0);
    assert!(host.menu.is_some());
    host.update(Event::Key(key(KeyCode::Esc)));
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: row.x + 2,
        row: row.y,
        at_ms: 1600,
    });
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: row.x + 2,
        row: row.y,
        at_ms: 1750,
    });
    assert_eq!(host.plugins[0].depth("github"), 1);
    let mut p = demo::sample();
    assert!(matches!(
        p.context_select("connection", "setting:6"),
        Outcome::Handled(_)
    ));
    assert!(!p.capturing());
    assert!(matches!(
        p.context_select("github", radar_core::view_tui::PANE_HIT),
        Outcome::Ignored
    ));
}

#[test]
fn stale_host_popup_never_copies_a_replacement_row() {
    for choice in ["key", "enter", "mouse", "mouse_after_redraw"] {
        let p = demo::sample();
        let new_pull = p.repos[0].pulls.as_ref().unwrap().items[1].clone();
        let answer = repository_answer(&p, vec![new_pull], true);
        let mut host = hosted(p);
        host.update(Event::Key(key(KeyCode::Char('x'))));
        let _ = to_text(&host, 160, 40);
        let menu_index = host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .position(|i| i.keys == "y")
            .unwrap();
        let rect = host
            .hits
            .borrow()
            .iter()
            .find_map(|(r, hit)| matches!(hit, Hit::MenuItem(i) if *i == menu_index).then_some(*r))
            .unwrap();
        host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(answer),
        });
        if choice == "mouse_after_redraw" {
            let _ = to_text(&host, 160, 40);
        }
        let effects = if choice.starts_with("mouse") {
            host.update(Event::Mouse {
                kind: MouseKind::Press,
                column: rect.x + 1,
                row: rect.y,
                at_ms: 100,
            })
        } else if choice == "enter" {
            host.menu.as_mut().unwrap().row = menu_index;
            host.update(Event::Key(key(KeyCode::Enter)))
        } else {
            host.update(Event::Key(key(KeyCode::Char('y'))))
        };
        assert!(!effects.iter().any(|e| matches!(e, HostEffect::Copy(_))));
        assert!(
            host.message
                .as_ref()
                .is_some_and(|m| m.contains("changed") || m.contains("reopen"))
        );
        host.update(Event::Key(key(KeyCode::Char('x'))));
        let effects = host.update(Event::Key(key(KeyCode::Char('y'))));
        assert!(
            effects
                .iter()
                .any(|e| matches!(e, HostEffect::Copy(url) if url.ends_with("/43")))
        );
    }
}

#[test]
fn unchanged_host_popup_survives_a_background_read() {
    let p = demo::sample();
    let items = p.repos[0].pulls.as_ref().unwrap().items.clone();
    let answer = repository_answer(&p, items, true);
    let mut host = hosted(p);
    host.update(Event::Key(key(KeyCode::Char('x'))));
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(answer),
    });
    let effects = host.update(Event::Key(key(KeyCode::Char('y'))));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, HostEffect::Copy(url) if url.ends_with("/42")))
    );
}

#[test]
fn modified_action_keys_and_foreign_subjects_are_refused() {
    let mut p = demo::sample();
    for code in ['o', 'y', 'r'] {
        assert!(matches!(
            p.key("github", Pane::Middle, Chord::ctrl(code)),
            Outcome::Ignored
        ));
    }
    assert!(matches!(
        p.show(&Subject::new(
            "pull-request",
            "https://example.com/acme/api/pull/42",
            "other"
        )),
        Showing::Cannot
    ));
    assert!(matches!(
        p.show(&join::pull_subject(
            &RepositoryId::parse("elsewhere/api").unwrap(),
            42
        )),
        Showing::Cannot
    ));
    assert!(p.pages.is_empty());
}

#[test]
fn page_back_and_tabs_keep_list_selection() {
    let mut host = hosted(demo::sample());
    host.update(Event::Key(key(KeyCode::Down)));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert_eq!(host.plugins[0].depth("github"), 1);
    host.update(Event::Key(key(KeyCode::Char(']'))));
    assert_eq!(host.plugins[0].tabs("github").unwrap().on, 1);
    host.update(Event::Key(key(KeyCode::Esc)));
    assert_eq!(host.plugins[0].depth("github"), 0);
    assert!(
        host.plugins[0]
            .subject("github", Pane::Middle)
            .unwrap()
            .id
            .ends_with("/43")
    );
    assert!(matches!(host.view, View::Screen(_)));
}

#[test]
fn nested_cross_repository_back_restores_exact_parent_data() {
    let mut p = demo::sample();
    let first = p.repos[0].id.clone();
    let second = p.repos[1].id.clone();
    p.open_target(Target::Repository(first.clone()));
    p.open_target(Target::Repository(second));
    screen::key(&mut p, Pane::Middle, chord(KeyCode::Esc));
    assert_eq!(
        p.pages.last().unwrap().target,
        Target::Repository(first.clone())
    );
    assert_eq!(p.repository().unwrap().id, first);
    assert_eq!(p.selected_pull().unwrap().number, 42);
    screen::show_tab(&mut p, 1);
    assert_eq!(p.target(Pane::Middle), Some(Target::Pull(first, 42)));
}

fn saved_text(effects: &[HostEffect]) -> Option<&str> {
    let saves: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Save(text) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(saves.len() <= 1, "a completed edit saves once");
    saves.into_iter().next()
}

fn replace_field(host: &mut radar_core::Host, value: &str) -> Vec<HostEffect> {
    assert!(host.plugins[0].capturing());
    assert!(
        saved_text(&host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('u'),
            KeyModifiers::CONTROL
        ))))
        .is_none()
    );
    assert!(saved_text(&host.update(Event::Paste(value.into()))).is_none());
    host.update(Event::Key(key(KeyCode::Enter)))
}

fn reloaded(text: &str) -> radar_core::Host {
    let mut host = radar_core::Host::new(vec![Box::new(GithubPlugin::default())], 0).unwrap();
    host.load_config(Some(text)).unwrap();
    host
}

#[test]
fn public_and_reference_settings_save_acknowledge_and_reload_without_resolving_credentials() {
    for value in ["", "env:SYNTHETIC_EXISTING_REFERENCE"] {
        let mut host = connection_host();
        assert!(!host.dirty());
        host.update(Event::Key(key(KeyCode::Down)));
        assert!(saved_text(&host.update(Event::Key(key(KeyCode::Enter)))).is_none());
        let effects = replace_field(&mut host, value);
        let text = saved_text(&effects)
            .expect("committed token must save")
            .to_owned();
        assert!(host.dirty());
        assert_eq!(
            effects
                .iter()
                .filter(|effect| matches!(effect, HostEffect::Spawn { .. }))
                .count(),
            1
        );
        host.update(Event::Saved(Ok(())));
        assert!(!host.dirty());
        assert!(
            host.update(Event::Tick(10)).is_empty(),
            "later must defer reads"
        );
        let fresh = reloaded(&text);
        assert_eq!(fresh.plugins[0].config(), host.plugins[0].config());
        assert_eq!(
            fresh.plugins[0].config().unwrap()["token"].as_str(),
            Some(value)
        );
        assert_eq!(fresh.plugins[0].set_up(), Setup::Ready);
        assert!(!fresh.dirty());
    }
}

#[test]
fn cancelled_invalid_and_fixed_settings_never_save_or_spawn_reads() {
    let mut host = connection_host();
    let before = host.plugins[0].config();
    // Home is the first sign-in choice; the server row is three below it.
    let down = |host: &mut radar_core::Host, n: usize| {
        host.update(Event::Key(key(KeyCode::Home)));
        for _ in 0..n {
            host.update(Event::Key(key(KeyCode::Down)));
        }
    };
    down(&mut host, 6);
    let effects = host.update(Event::Key(key(KeyCode::Enter)));
    assert!(effects.is_empty());
    assert!(!host.plugins[0].capturing());
    // The server row is typed into, but a bad address is never kept.
    down(&mut host, 3);
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(host.plugins[0].capturing());
    assert!(replace_field(&mut host, "ftp://elsewhere.test").is_empty());
    assert!(host.plugins[0].capturing());
    assert_eq!(host.plugins[0].config(), before);
    host.update(Event::Key(key(KeyCode::Esc)));
    down(&mut host, 4);
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(replace_field(&mut host, "bad token with spaces").is_empty());
    assert!(host.plugins[0].capturing());
    assert_eq!(host.plugins[0].config(), before);
    assert!(host.update(Event::Key(key(KeyCode::Esc))).is_empty());
    assert_eq!(host.plugins[0].config(), before);
    assert!(!host.dirty());
    assert!(host.update(Event::Tick(10)).is_empty());
}

#[test]
fn repository_add_edit_duplicate_cancel_and_remove_save_exact_config() {
    let mut host = settings_host("repositories");
    host.update(Event::Key(key(KeyCode::Enter)));
    let effects = replace_field(&mut host, "https://github.com/Acme/Extra");
    let text = saved_text(&effects).unwrap().to_owned();
    assert_eq!(
        reloaded(&text).plugins[0].config(),
        host.plugins[0].config()
    );
    host.update(Event::Saved(Ok(())));
    host.update(Event::Key(key(KeyCode::Char('n'))));
    host.update(Event::Key(key(KeyCode::Char('e'))));
    let effects = replace_field(&mut host, "acme/renamed");
    assert!(saved_text(&effects).is_some());
    host.update(Event::Saved(Ok(())));
    host.update(Event::Key(key(KeyCode::Char('n'))));
    let before = host.plugins[0].config();
    host.update(Event::Key(key(KeyCode::Home)));
    host.update(Event::Key(key(KeyCode::Enter)));
    assert!(replace_field(&mut host, "https://elsewhere.example/acme/api").is_empty());
    assert!(host.plugins[0].capturing());
    host.update(Event::Key(key(KeyCode::Esc)));
    assert_eq!(host.plugins[0].config(), before);
    host.update(Event::Key(key(KeyCode::Enter)));
    let effects = replace_field(&mut host, "acme/newdocs");
    assert_eq!(
        host.plugins[0].config().unwrap()["groups"][0]["repositories"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(saved_text(&effects).is_some());
    host.update(Event::Saved(Ok(())));
    host.update(Event::Key(key(KeyCode::Char('n'))));
    host.update(Event::Key(key(KeyCode::Char('d'))));
    assert!(host.update(Event::Key(key(KeyCode::Char('n')))).is_empty());
    assert!(!host.dirty());
    host.update(Event::Key(key(KeyCode::Char('d'))));
    let effects = host.update(Event::Key(key(KeyCode::Char('y'))));
    let text = saved_text(&effects).unwrap();
    assert_eq!(
        reloaded(text).plugins[0].config().unwrap()["groups"][0]["repositories"]
            .as_array()
            .unwrap(),
        &[
            toml::Value::from("acme/api"),
            toml::Value::from("acme/docs"),
            toml::Value::from("acme/renamed")
        ]
    );
    assert!(
        host.update(Event::Tick(10)).is_empty(),
        "removal must not silently refresh remaining repositories"
    );
}

#[test]
fn failed_settings_save_is_visible_and_write_retries_current_config() {
    let mut host = connection_host();
    host.update(Event::Key(key(KeyCode::Down)));
    host.update(Event::Key(key(KeyCode::Enter)));
    let effects = replace_field(&mut host, "");
    let first = saved_text(&effects).unwrap().to_owned();
    host.update(Event::Saved(Err("synthetic disk full".into())));
    assert!(host.unsaved());
    host.update(Event::Key(key(KeyCode::Char('n'))));
    let text = to_text(&host, 160, 40);
    assert!(text.contains("unsaved"));
    let effects = host.update(Event::Key(key(KeyCode::Char('w'))));
    assert_eq!(saved_text(&effects), Some(first.as_str()));
    host.update(Event::Saved(Ok(())));
    assert!(!host.unsaved());
    assert!(!host.dirty());
}

#[test]
fn explicit_access_check_uses_host_task_dispatch_and_empty_settings_explain_why() {
    let mut host = connection_host();
    host.update(Event::Key(key(KeyCode::Down)));
    host.update(Event::Key(key(KeyCode::Down)));
    let effects = host.update(Event::Key(key(KeyCode::Enter)));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, HostEffect::Spawn { .. }))
            .count(),
        1
    );
    assert!(saved_text(&effects).is_none());
    // Tasks are inspected but never polled: no credentials or network are used.
    let mut empty = settings_host("repositories");
    empty.plugins[0].configure(None).unwrap();
    let effects = empty.update(Event::Key(key(KeyCode::Char('c'))));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, HostEffect::Spawn { .. }))
            .count(),
        1
    );
}

#[test]
fn accessible_repository_picker_groups_owners_and_keeps_selection_available() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        token: "github_pat_synthetic_picker_123".into(),
        groups: vec![],
        repositories: vec!["alice/radar".into()],
        ..Settings::default()
    });
    p.available = Some(Collection {
        items: [
            "alice/radar",
            "alice/notes",
            "acme-org/api",
            "acme-org/web",
        ]
        .into_iter()
        .map(|name| RepositoryId::parse(name).unwrap())
        .collect(),
        completion: Completion::complete(1),
    });
    let menu = settings::discovery_menu(&p).unwrap();
    assert!(
        menu.items
            .iter()
            .any(|item| item.heading && item.label == "github.com/alice")
    );
    assert!(
        menu.items
            .iter()
            .any(|item| item.heading && item.label == "github.com/acme-org")
    );
    assert!(
        !menu
            .items
            .iter()
            .any(|item| item.value == "repo:alice/radar")
    );
    let effects = settings::on_menu(&mut p, "discover", "owner:acme-org");
    assert!(p.cfg.repositories.contains(&"acme-org/api".into()));
    assert!(p.cfg.repositories.contains(&"acme-org/web".into()));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Menu(_)))
    );
    let effects = settings::on_menu(&mut p, "discover", "repo:alice/notes");
    assert!(p.cfg.repositories.contains(&"alice/notes".into()));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::SpawnAction { .. }))
    );
}

#[test]
fn track_all_forty_seven_repositories_appears_and_persists_in_its_group() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        token: "github_pat_synthetic_picker_123".into(),
        groups: vec![config::Group {
            name: "My projects".into(),
            owner: Some("alice".into()),
            repositories: vec![],
        }],
        ..Settings::default()
    });
    p.discovery_group = Some(0);
    p.discovery_owner = Some("alice".into());
    p.available = Some(Collection {
        items: (0..47)
            .map(|n| RepositoryId::parse(&format!("alice/repo{n}")).unwrap())
            .collect(),
        completion: Completion::complete(1),
    });
    assert!(
        settings::discovery_menu(&p)
            .unwrap()
            .items
            .iter()
            .any(|item| item.value == "owner:alice")
    );
    settings::on_menu(&mut p, "discover", "owner:alice");
    assert_eq!(p.cfg.groups[0].repositories.len(), 47);
    assert_eq!(p.repos.len(), 47);
    let saved = toml::to_string(&p.cfg).unwrap();
    let loaded = toml::from_str::<Settings>(&saved)
        .unwrap()
        .validated()
        .unwrap();
    assert_eq!(loaded.groups[0].repositories.len(), 47);
    assert_eq!(loaded.groups[0].name, "My projects");
    let host = hosted(p);
    let screen = to_text(&host, 160, 40);
    assert!(screen.contains("My projects"), "{screen}");
    assert!(screen.contains("repo0"), "{screen}");
    let compact = to_text(&host, 80, 24);
    assert!(compact.contains("My projects"), "{compact}");
}

#[test]
fn repositories_can_move_between_named_groups() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        groups: vec![
            config::Group {
                name: "Client work".into(),
                owner: None,
                repositories: vec!["acme/api".into()],
            },
            config::Group {
                name: "Personal".into(),
                owner: None,
                repositories: vec![],
            },
        ],
        ..Settings::default()
    });
    settings::select_repository_row(&mut p, 2);
    settings::key(&mut p, "repositories", chord(KeyCode::Char('m')));
    settings::on_menu(&mut p, "move:0:0", "1");
    assert!(p.cfg.groups[0].repositories.is_empty());
    assert_eq!(p.cfg.groups[1].repositories, ["acme/api"]);
    assert_eq!(p.cfg.repositories, ["acme/api"]);
}

#[test]
fn move_offers_to_create_a_destination_when_no_other_group_exists() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        groups: vec![config::Group {
            name: "github.com/acme".into(),
            owner: Some("acme".into()),
            repositories: vec!["acme/api".into()],
        }],
        ..Settings::default()
    });
    settings::select_repository_row(&mut p, 2);
    let Outcome::Handled(effects) =
        settings::key(&mut p, "repositories", chord(KeyCode::Char('m')))
    else {
        panic!("move should open a destination menu");
    };
    let Some(Effect::Menu(menu)) = effects.first() else {
        panic!("move should open a destination menu");
    };
    assert_eq!(menu.items.len(), 1);
    assert_eq!(menu.items[0].value, "new");

    settings::on_menu(&mut p, &menu.id, "new");
    let editor = p.editor.as_mut().expect("new group name input");
    editor.input.paste("Client work");
    settings::key(&mut p, "repositories", chord(KeyCode::Enter));
    assert_eq!(p.cfg.groups.len(), 2);
    assert!(p.cfg.groups[0].repositories.is_empty());
    assert_eq!(p.cfg.groups[1].name, "Client work");
    assert_eq!(p.cfg.groups[1].repositories, ["acme/api"]);
    assert_eq!(p.cfg.repositories, ["acme/api"]);
}

#[test]
fn bare_group_name_is_custom_and_can_hold_another_owners_repository() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        groups: vec![config::Group {
            name: "github.com/acme".into(),
            owner: Some("acme".into()),
            repositories: vec!["acme/api".into()],
        }],
        ..Settings::default()
    });
    settings::key(&mut p, "repositories", chord(KeyCode::Enter));
    p.editor.as_mut().unwrap().input.paste("favourites");
    settings::key(&mut p, "repositories", chord(KeyCode::Enter));
    assert_eq!(p.cfg.groups.len(), 2, "{:?}", p.cfg);
    assert_eq!(p.cfg.groups[1].name, "favourites");
    assert_eq!(p.cfg.groups[1].owner, None);
    assert!(!p.discovery_loading);

    settings::select_repository_row(&mut p, 2);
    let Outcome::Handled(effects) =
        settings::key(&mut p, "repositories", chord(KeyCode::Char('m')))
    else {
        panic!("move should open a destination menu");
    };
    let Some(Effect::Menu(menu)) = effects.first() else {
        panic!("move should open a destination menu");
    };
    assert!(
        menu.items
            .iter()
            .any(|item| item.label == "favourites" && item.value == "1")
    );
    settings::on_menu(&mut p, &menu.id, "1");
    assert_eq!(p.cfg.groups[1].repositories, ["acme/api"]);
}

#[test]
fn renaming_an_owner_group_to_a_bare_name_removes_the_owner_restriction() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        groups: vec![config::Group {
            name: "github.com/favourites".into(),
            owner: Some("favourites".into()),
            repositories: vec![],
        }],
        ..Settings::default()
    });
    settings::select_repository_row(&mut p, 1);
    settings::key(&mut p, "repositories", chord(KeyCode::Char('e')));
    p.editor.as_mut().unwrap().input = radar_core::input::TextInput::new("favourites");
    settings::key(&mut p, "repositories", chord(KeyCode::Enter));
    assert_eq!(p.cfg.groups[0].name, "favourites");
    assert_eq!(p.cfg.groups[0].owner, None);
}

#[test]
fn owner_input_and_browse_key_start_access_discovery_without_repositories() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        token: "github_pat_synthetic_picker_123".into(),
        groups: vec![],
        repositories: vec![],
        ..Settings::default()
    });
    assert!(matches!(
        settings::key(&mut p, "repositories", chord(KeyCode::Char('a'))),
        Outcome::Handled(_)
    ));
    assert!(p.discovery_loading);
    p.editor = None;
    p.discovery_loading = false;
    assert!(matches!(
        settings::key(&mut p, "repositories", chord(KeyCode::Enter)),
        Outcome::Handled(_)
    ));
    let editor = p.editor.as_mut().unwrap();
    editor.input.paste("github.com/acme-org");
    assert!(matches!(
        settings::key(&mut p, "repositories", chord(KeyCode::Enter)),
        Outcome::Handled(_)
    ));
    assert!(p.discovery_loading);
    assert!(p.cfg.repositories.is_empty());
}

#[test]
fn typed_token_is_masked_after_save_and_checks_without_repositories() {
    let mut host = connection_host();
    host.update(Event::Key(key(KeyCode::Down)));
    host.update(Event::Key(key(KeyCode::Enter)));
    let token = "github_pat_synthetic_screen_123";
    let effects = replace_field(&mut host, token);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, HostEffect::Spawn { .. }))
            .count(),
        1
    );
    assert!(!to_text(&host, 160, 40).contains(token));
    assert_eq!(
        host.plugins[0].config().unwrap()["token"].as_str(),
        Some(token)
    );
}

#[test]
fn old_connection_check_cannot_replace_a_changed_token() {
    let mut p = GithubPlugin::default();
    p.apply_config(Settings {
        token: "github_pat_synthetic_old_123".into(),
        groups: vec![],
        repositories: vec![],
        ..Settings::default()
    });
    let generation = p.generation;
    p.check_connection();
    let request = p.connection_request;
    p.apply_config(Settings {
        token: "github_pat_synthetic_new_123".into(),
        groups: vec![],
        repositories: vec![],
        ..Settings::default()
    });
    let effects = p.receive(Answer::Connection {
        generation,
        request,
        action: history::connection_action(),
        result: Ok((Some("old-user".into()), None)),
    });
    assert!(p.connection_check.is_none());
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Observe { .. }))
    );
}

#[derive(Default)]
struct CompositionState {
    actions: Vec<(String, Subject)>,
    link: String,
    depth: usize,
}
struct CompositionProbe {
    ready: bool,
    state: std::sync::Arc<std::sync::Mutex<CompositionState>>,
}
impl Plugin for CompositionProbe {
    fn id(&self) -> &'static str {
        "composition-probe"
    }
    fn name(&self) -> &'static str {
        "Linked context"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> Setup {
        if self.ready {
            Setup::Ready
        } else {
            Setup::Needed("configure the probe".into())
        }
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "probe",
            name: "Probe",
            key: 'P',
        }]
    }
    fn view(
        &self,
        screen: &str,
        _: &radar_core::view::ViewCx<'_>,
    ) -> Option<radar_core::view::ScreenView> {
        (screen == "probe").then(|| {
            radar_core::view::ScreenView::new(
                "probe",
                vec![radar_core::view::PaneView::new(
                    radar_core::view::PaneId::Middle,
                    "Probe",
                    radar_core::view::Node::Document {
                        lines: vec![radar_core::view::Text::plain("Probe")],
                        cut: false,
                    },
                )],
            )
        })
    }
    fn owns(&self) -> Vec<Owned> {
        vec![Owned::page("guide", "guides")]
    }
    fn recognise(&self, text: &str) -> Vec<(std::ops::Range<usize>, Subject)> {
        let url = "https://guide.example/entry";
        text.match_indices(url)
            .map(|(at, _)| {
                (
                    at..at + url.len(),
                    Subject::new("guide", url, "guide").with_url(url),
                )
            })
            .collect()
    }
    fn show(&mut self, subject: &Subject) -> Showing {
        if subject.kind != "guide" {
            return Showing::Cannot;
        }
        self.state.lock().unwrap().depth += 1;
        Showing::Page {
            screen: "probe",
            effects: vec![],
        }
    }
    fn depth(&self, _: &str) -> usize {
        self.state.lock().unwrap().depth
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &radar_core::view::Intent) -> Outcome {
        let Some(chord) = intent.chord() else {
            return Outcome::Ignored;
        };
        match chord.key {
            radar_core::view::Key::Char('n') => Outcome::Handled(vec![Effect::Show(Subject::link(
                self.state.lock().unwrap().link.clone(),
            ))]),
            radar_core::view::Key::Esc if self.state.lock().unwrap().depth > 0 => {
                self.state.lock().unwrap().depth -= 1;
                Outcome::done()
            }
            _ => Outcome::Ignored,
        }
    }
    fn contribute(
        &self,
        point: radar_core::Point,
        subject: &Subject,
    ) -> Option<radar_core::Contribution> {
        if !matches!(subject.kind.as_str(), "repository" | "pull-request") {
            return None;
        }
        Some(match point {
            radar_core::Point::Actions => radar_core::Contribution::Actions(vec![
                radar_core::Action::new('v', "view linked context", "inspect"),
                radar_core::Action::new('y', "conflicting copy", "conflict"),
            ]),
            radar_core::Point::Line => radar_core::Contribution::Line(vec![
                radar_core::Chip::new("local folder", radar_core::plugin::Tone::Plain).acts("folder"),
            ]),
            radar_core::Point::Section => radar_core::Contribution::Section {
                icon: None,
                title: "Linked evidence".into(),
                lines: vec![radar_core::view::Text::plain("Related cached information")],
            },
            _ => return None,
        })
    }
    fn act(&mut self, subject: &Subject, action: &str) -> Vec<Effect> {
        self.state
            .lock()
            .unwrap()
            .actions
            .push((action.into(), subject.clone()));
        vec![Effect::Copy(format!("probe:{action}"))]
    }
}

fn composed_host(
    p: GithubPlugin,
    ready: bool,
) -> (
    radar_core::Host,
    std::sync::Arc<std::sync::Mutex<CompositionState>>,
) {
    let state = std::sync::Arc::new(std::sync::Mutex::new(CompositionState::default()));
    let config = format!("[github]\n{}", toml::to_string(&p.cfg).unwrap());
    let mut host = radar_core::Host::new(
        vec![
            Box::new(GithubPlugin::default()),
            Box::new(CompositionProbe {
                ready,
                state: state.clone(),
            }),
        ],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    host.load_config(Some(&config)).unwrap();
    host.plugins[0] = Box::new(p);
    (host, state)
}

fn choose_probe_action(host: &mut radar_core::Host, choice: &str) -> Vec<HostEffect> {
    if choice == "direct" {
        return host.update(Event::Key(key(KeyCode::Char('v'))));
    }
    host.update(Event::Key(key(KeyCode::Char('x'))));
    if choice == "key" {
        return host.update(Event::Key(key(KeyCode::Char('v'))));
    }
    let index = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .position(|item| item.label == "view linked context")
        .unwrap();
    if choice == "enter" {
        host.menu.as_mut().unwrap().row = index;
        host.update(Event::Key(key(KeyCode::Enter)))
    } else {
        to_text(host, 160, 40);
        let area = host
            .hits
            .borrow()
            .iter()
            .find_map(|(area, hit)| matches!(hit, Hit::MenuItem(i) if *i == index).then_some(*area))
            .unwrap();
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: area.x + 1,
            row: area.y,
            at_ms: 100,
        })
    }
}

#[test]
fn foreign_contribution_actions_use_host_keys_enter_and_mouse_with_owned_identity() {
    for choice in ["direct", "key", "enter", "mouse"] {
        let (mut host, state) = composed_host(demo::sample(), true);
        let effects = choose_probe_action(&mut host, choice);
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "probe:inspect"))
        );
        let state = state.lock().unwrap();
        assert_eq!(state.actions.len(), 1);
        let subject = &state.actions[0].1;
        assert_eq!(subject.id, "https://github.com/acme/api/pull/42");
        assert_eq!(
            subject.field("repository_url"),
            Some("https://github.com/acme/api")
        );
        assert_eq!(
            subject.field("source_repository_url"),
            Some("https://github.com/alice/api")
        );
        assert_eq!(subject.field("branch"), Some("refresh-ownership"));
        assert_eq!(subject.field("head_sha"), Some("a".repeat(40).as_str()));
    }
    let (mut host, state) = composed_host(demo::sample(), true);
    host.update(Event::Key(key(KeyCode::Char('x'))));
    let conflicting = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .find(|item| item.label == "conflicting copy")
        .unwrap();
    assert!(conflicting.keys.is_empty());
    let effects = host.update(Event::Key(key(KeyCode::Char('y'))));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, HostEffect::Copy(url) if url.ends_with("/42")))
    );
    assert!(state.lock().unwrap().actions.is_empty());
}

#[test]
fn contributed_sections_and_title_line_actions_are_absent_when_unready_and_safe_to_click() {
    for ready in [false, true] {
        let (mut host, state) = composed_host(demo::page(), ready);
        let text = to_text(&host, 160, 40);
        if ready {
            exported("linked_overview", &host, 160, 40);
            exported("linked_overview", &host, 80, 24);
        }
        assert_eq!(text.contains("local folder"), ready);
        assert_eq!(text.contains("Related cached information"), ready);
        if !ready {
            assert!(choose_probe_action(&mut host, "direct").is_empty());
            assert!(state.lock().unwrap().actions.is_empty());
            continue;
        }
        let area = host
            .hits
            .borrow()
            .iter()
            .find_map(|(area, hit)| {
                matches!(hit, Hit::Plugin { id, .. } if id.starts_with("under:")).then_some(*area)
            })
            .unwrap();
        assert!(
            host.update(Event::Mouse {
                kind: MouseKind::Context,
                column: area.x,
                row: area.y,
                at_ms: 100
            })
            .is_empty()
        );
        assert!(host.menu.is_none());
        assert!(state.lock().unwrap().actions.is_empty());
        let effects = host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: area.x,
            row: area.y,
            at_ms: 500,
        });
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "probe:folder"))
        );
        assert_eq!(state.lock().unwrap().actions.len(), 1);
    }
    let mut p = demo::sample();
    p.open_target(Target::Repository(p.repos[0].id.clone()));
    let (host, _) = composed_host(p, true);
    let text = to_text(&host, 80, 24);
    exported("linked_repository", &host, 80, 24);
    exported("linked_repository", &host, 160, 40);
    assert!(text.contains("local folder"));
    assert!(text.contains("LINKED EVIDENCE"));
}

#[test]
fn native_links_route_through_host_and_back_preserves_origin_focus() {
    for (url, native) in [
        ("https://github.com/acme/api/pull/42", true),
        ("https://github.com/acme/docs", true),
        ("https://github.com/unconfigured/api/pull/42", false),
        ("https://elsewhere.example/acme/api/pull/42", false),
        ("https://github.com/acme/api/pull/42/files", false),
    ] {
        let (mut host, state) = composed_host(demo::sample(), true);
        state.lock().unwrap().link = url.into();
        host.update(Event::Key(key(KeyCode::Char('P'))));
        host.focus = Pane::Detail;
        let origin = host.view;
        let effects = host.update(Event::Key(key(KeyCode::Char('n'))));
        if native {
            assert_ne!(host.view, origin);
            assert!(
                effects
                    .iter()
                    .all(|effect| !matches!(effect, HostEffect::Open(_)))
            );
            assert_eq!(host.plugins[0].depth("github"), 1);
            host.update(Event::Key(key(KeyCode::Esc)));
            assert_eq!((host.view, host.focus), (origin, Pane::Detail));
        } else {
            assert_eq!(host.view, origin);
            assert!(
                effects
                    .iter()
                    .any(|effect| matches!(effect, HostEffect::Open(open) if open == url))
            );
            assert_eq!(host.plugins[0].depth("github"), 0);
        }
    }
}

#[test]
fn description_links_open_foreign_native_pages_and_return_to_current_page() {
    let mut p = demo::page();
    p.pull_detail.as_mut().unwrap().body =
        Some("Read [the guide](https://guide.example/entry).".into());
    let (mut host, state) = composed_host(p, true);
    let original = host.view;
    to_text(&host, 160, 40);
    host.update(Event::Key(key(KeyCode::Enter)));
    let index = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .position(|item| item.label == "the guide")
        .unwrap();
    host.menu.as_mut().unwrap().row = index;
    let effects = host.update(Event::Key(key(KeyCode::Enter)));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, HostEffect::Open(_)))
    );
    assert_ne!(host.view, original);
    assert_eq!(state.lock().unwrap().depth, 1);
    host.update(Event::Key(key(KeyCode::Esc)));
    assert_eq!(host.view, original);
    assert_eq!(host.plugins[0].depth("github"), 1);
}

#[test]
fn emitted_pull_subject_uses_only_exact_held_data_and_never_caller_metadata() {
    let mut p = demo::sample();
    let original = p.subject("github", Pane::Middle).unwrap();
    let supplied = original
        .clone()
        .with(
            "source_repository_url",
            "https://elsewhere.example/other/repo",
        )
        .with("branch", "injected");
    assert!(matches!(p.show(&supplied), Showing::Page { .. }));
    let loading = p.subject("github", Pane::Middle).unwrap();
    assert!(loading.field("source_repository_url").is_none());
    assert!(loading.field("branch").is_none());
    let mut held = demo::pull(
        &RepositoryId::parse("acme/api").unwrap(),
        42,
        "current title",
    );
    held.head_branch = "current-branch".into();
    p.receive(pull_answer(&p, held.clone()));
    let subject = p.subject("github", Pane::Middle).unwrap();
    assert_eq!(
        subject.field("source_repository_url"),
        Some("https://github.com/alice/api")
    );
    assert_eq!(subject.field("branch"), Some("current-branch"));
    assert_eq!(subject.field("title"), Some("current title"));
    p.pull_detail.as_mut().unwrap().head_repository = None;
    assert!(
        p.subject("github", Pane::Middle)
            .unwrap()
            .field("source_repository_url")
            .is_none()
    );
    p.pull_detail.as_mut().unwrap().number = 99;
    let wrong = p.subject("github", Pane::Middle).unwrap();
    assert!(wrong.field("title").is_none());
    assert!(wrong.field("branch").is_none());
}

#[test]
fn contributed_popup_rejects_changed_source_same_pull_on_every_dispatch_path() {
    for choice in ["key", "enter", "mouse", "mouse_after_redraw"] {
        let p = demo::sample();
        let mut items = p.repos[0].pulls.as_ref().unwrap().items.clone();
        items[0].head_repository = Some(RepositoryId::parse("other/api").unwrap());
        items[0].head_sha = "b".repeat(40);
        let answer = repository_answer(&p, items, true);
        let (mut host, state) = composed_host(p, true);
        host.update(Event::Key(key(KeyCode::Char('x'))));
        to_text(&host, 160, 40);
        let index = host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .position(|item| item.label == "view linked context")
            .unwrap();
        let area = host
            .hits
            .borrow()
            .iter()
            .find_map(|(area, hit)| matches!(hit, Hit::MenuItem(i) if *i == index).then_some(*area))
            .unwrap();
        host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(answer),
        });
        if choice == "mouse_after_redraw" {
            to_text(&host, 160, 40);
        }
        let effects = if choice.starts_with("mouse") {
            host.update(Event::Mouse {
                kind: MouseKind::Press,
                column: area.x + 1,
                row: area.y,
                at_ms: 100,
            })
        } else if choice == "enter" {
            host.menu.as_mut().unwrap().row = index;
            host.update(Event::Key(key(KeyCode::Enter)))
        } else {
            host.update(Event::Key(key(KeyCode::Char('v'))))
        };
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, HostEffect::Copy(_)))
        );
        assert!(state.lock().unwrap().actions.is_empty());
        assert!(
            host.message
                .as_deref()
                .is_some_and(|message| message.contains("changed") || message.contains("reopen"))
        );
        choose_probe_action(&mut host, "direct");
        let state = state.lock().unwrap();
        assert_eq!(state.actions.len(), 1);
        assert_eq!(
            state.actions[0].1.field("source_repository_url"),
            Some("https://github.com/other/api")
        );
    }
}

#[test]
fn unchanged_contributed_popup_survives_background_read_but_not_reconfiguration() {
    let p = demo::sample();
    let answer = repository_answer(&p, p.repos[0].pulls.as_ref().unwrap().items.clone(), true);
    let (mut host, state) = composed_host(p, true);
    host.update(Event::Key(key(KeyCode::Char('x'))));
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(answer),
    });
    let effects = host.update(Event::Key(key(KeyCode::Char('v'))));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "probe:inspect"))
    );
    assert_eq!(state.lock().unwrap().actions.len(), 1);
    host.update(Event::Key(key(KeyCode::Char('x'))));
    let cfg = host.plugins[0].config().unwrap();
    host.plugins[0].configure(Some(&cfg)).unwrap();
    let effects = host.update(Event::Key(key(KeyCode::Char('v'))));
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, HostEffect::Copy(_)))
    );
    assert_eq!(state.lock().unwrap().actions.len(), 1);
}

#[test]
fn title_action_hit_cannot_replay_after_a_page_refresh_response() {
    let p = demo::page();
    let mut pull = p.pull_detail.clone().unwrap();
    pull.head_repository = Some(RepositoryId::parse("other/api").unwrap());
    let answer = pull_answer(&p, pull);
    let (mut host, state) = composed_host(p, true);
    to_text(&host, 160, 40);
    let area = host
        .hits
        .borrow()
        .iter()
        .find_map(|(area, hit)| {
            matches!(hit, Hit::Plugin { id, .. } if id.starts_with("under:")).then_some(*area)
        })
        .unwrap();
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(answer),
    });
    let effects = host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: area.x,
        row: area.y,
        at_ms: 100,
    });
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, HostEffect::Copy(_)))
    );
    assert!(state.lock().unwrap().actions.is_empty());
    to_text(&host, 160, 40);
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: area.x,
        row: area.y,
        at_ms: 700,
    });
    assert_eq!(state.lock().unwrap().actions.len(), 1);
    assert_eq!(
        state.lock().unwrap().actions[0]
            .1
            .field("source_repository_url"),
        Some("https://github.com/other/api")
    );
}

#[test]
fn settings_context_selects_without_editing_or_checking_and_blank_dismisses() {
    let mut host = connection_host();
    to_text(&host, 160, 40);
    let row = host
        .hits
        .borrow()
        .iter()
        .find_map(|(area, hit)| {
            matches!(hit, Hit::Plugin { id, .. } if id == "setting:6").then_some(*area)
        })
        .unwrap();
    assert!(
        host.update(Event::Mouse {
            kind: MouseKind::Context,
            column: row.x + 1,
            row: row.y,
            at_ms: 100
        })
        .is_empty()
    );
    assert!(!host.plugins[0].capturing());
    assert_eq!(
        host.section_view_in(0, "connection", 160, 40)
            .unwrap()
            .detail_title,
        "token"
    );
    host.update(Event::Key(key(KeyCode::Char('e'))));
    assert!(host.plugins[0].capturing());
    host.update(Event::Key(key(KeyCode::Esc)));
    to_text(&host, 160, 40);
    host.update(Event::Mouse {
        kind: MouseKind::Context,
        column: row.x + 1,
        row: row.y,
        at_ms: 600,
    });
    assert!(host.menu.is_some());
    host.update(Event::Mouse {
        kind: MouseKind::Context,
        column: row.x + 1,
        row: row.y + 7,
        at_ms: 1000,
    });
    assert!(host.menu.is_none());
    assert!(!host.plugins[0].capturing());
    assert!(!host.dirty());
}

#[test]
fn list_preview_renders_cached_line_and_fences_action_hits_on_selection_and_refresh() {
    for repository in [false, true] {
        let mut p = demo::sample();
        if repository {
            p.repo_at = 1;
        }
        let expected = p.target(Pane::Detail).unwrap().url();
        let (mut host, state) = composed_host(p, true);
        host.focus = Pane::Detail;
        for (width, height) in [(80, 24), (160, 40)] {
            let text = to_text(&host, width, height);
            assert!(text.contains("local folder"));
            if !repository {
                exported("linked_preview", &host, width, height);
            }
        }
        let area = host
            .hits
            .borrow()
            .iter()
            .find_map(|(area, hit)| {
                matches!(hit, Hit::Plugin { id, pane: Pane::Detail, .. } if id.starts_with("under:"))
                    .then_some(*area)
            })
            .unwrap();
        assert!(
            host.update(Event::Mouse {
                kind: MouseKind::Context,
                column: area.x,
                row: area.y,
                at_ms: 100
            })
            .is_empty()
        );
        assert!(host.menu.is_none());
        assert!(state.lock().unwrap().actions.is_empty());
        let effects = host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: area.x,
            row: area.y,
            at_ms: 500,
        });
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "probe:folder"))
        );
        assert_eq!(state.lock().unwrap().actions[0].1.id, expected);
        host.focus = Pane::Middle;
        host.update(Event::Key(key(if repository {
            KeyCode::Up
        } else {
            KeyCode::Down
        })));
        let effects = host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: area.x,
            row: area.y,
            at_ms: 1100,
        });
        assert!(
            effects
                .iter()
                .all(|effect| !matches!(effect, HostEffect::Copy(_)))
        );
        assert_eq!(state.lock().unwrap().actions.len(), 1);
    }
    let p = demo::sample();
    let mut pulls = p.repos[0].pulls.as_ref().unwrap().items.clone();
    pulls[0].head_repository = Some(RepositoryId::parse("other/api").unwrap());
    let answer = repository_answer(&p, pulls, true);
    let (mut host, state) = composed_host(p, true);
    to_text(&host, 160, 40);
    let area = host
        .hits
        .borrow()
        .iter()
        .find_map(|(area, hit)| {
            matches!(hit, Hit::Plugin { id, pane: Pane::Detail, .. } if id.starts_with("under:"))
                .then_some(*area)
        })
        .unwrap();
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(answer),
    });
    let effects = host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: area.x,
        row: area.y,
        at_ms: 100,
    });
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, HostEffect::Copy(_)))
    );
    assert!(state.lock().unwrap().actions.is_empty());
    to_text(&host, 160, 40);
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: area.x,
        row: area.y,
        at_ms: 700,
    });
    assert_eq!(state.lock().unwrap().actions.len(), 1);
    assert_eq!(
        state.lock().unwrap().actions[0]
            .1
            .field("source_repository_url"),
        Some("https://github.com/other/api")
    );
}

#[test]
fn actions_are_invoked_by_their_stable_ids() {
    use radar_core::view::Intent;
    let invoke = |action: &str| Intent::Invoke {
        action: action.into(),
    };
    let mut p = demo::page();
    let rows = crate::screen::keys(&p, Pane::Middle);
    let ids: Vec<String> = rows.iter().map(|(id, _)| id.clone()).collect();
    for id in [
        "github.page.open_on_github",
        "github.page.copy_link",
        "github.page.refresh",
        "github.page.back",
    ] {
        assert!(ids.iter().any(|i| i == id), "{id} missing from {ids:?}");
    }
    let Outcome::Handled(effects) = Plugin::intent(
        &mut p,
        "github",
        Pane::Middle,
        &invoke("github.page.open_on_github"),
    ) else {
        panic!("open by id was not handled");
    };
    assert!(matches!(effects.as_slice(), [Effect::Open(_)]));
    assert!(matches!(
        Plugin::intent(
            &mut p,
            "github",
            Pane::Middle,
            &invoke("github.page.refresh")
        ),
        Outcome::Handled(_)
    ));
    let mut p = demo::sample();
    assert!(matches!(
        Plugin::intent(
            &mut p,
            "connection",
            Pane::Middle,
            &invoke("github.connection.check")
        ),
        Outcome::Handled(_)
    ));
    assert!(matches!(
        Plugin::intent(
            &mut p,
            "github",
            Pane::Middle,
            &invoke("github.page.nonsense")
        ),
        Outcome::Ignored
    ));
}

#[test]
fn actions_are_invoked_by_their_stable_ids_through_headless() {
    use radar_core::headless::Headless;
    use radar_core::view::{Intent, PaneId};
    let mut headless = Headless::new(hosted(demo::page()), 160, 40);
    headless.focus(PaneId::Middle);
    let ids: Vec<String> = headless
        .screen()
        .unwrap()
        .actions
        .iter()
        .map(|a| a.id.clone())
        .filter(|id| !id.is_empty())
        .collect();
    assert!(
        ids.iter().any(|id| id == "github.page.open_on_github"),
        "{ids:?}"
    );
    assert!(ids.iter().all(|id| id.starts_with("github.")), "{ids:?}");
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "no id repeats: {ids:?}");
    let effects = headless.send(Intent::Invoke {
        action: "github.page.open_on_github".into(),
    });
    assert!(
        effects.iter().any(|e| matches!(
            e,
            HostEffect::Tracked { effect, .. } if matches!(**effect, HostEffect::Open(_))
        )),
        "the id ran the open row: {effects:?}"
    );
    // An id nobody listed does nothing.
    let effects = headless.send(Intent::Invoke {
        action: "github.page.nothing".into(),
    });
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, HostEffect::Tracked { .. })),
        "{effects:?}"
    );
}

mod notice_severity_tests {
    use super::*;

    #[test]
    fn connection_result_selects_notice_severity_before_icon_rendering() {
        for passed in [false, true] {
            let mut p = GithubPlugin {
                connection_request: 1,
                connection_check: Some(Check::Running),
                ..GithubPlugin::default()
            };
            let effects = p.receive(Answer::Connection {
                generation: p.generation,
                request: p.connection_request,
                action: history::connection_action(),
                result: if passed { Ok((None, None)) } else { Err("request refused".into()) },
            });
            if passed {
                assert!(matches!(effects.last(), Some(Effect::NotifyWith(_, words))
                    if words == "GitHub public API access works"));
            } else {
                assert!(matches!(effects.last(), Some(Effect::Notice(message))
                    if message.tone == radar_core::view::chrome::NoticeTone::Error
                        && message.icon == Some(radar_core::ui::Mark::Failed.icon())
                        && message.text == "request refused"));
            }
        }
    }
}
