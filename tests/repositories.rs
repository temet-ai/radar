#![cfg(all(
    feature = "gitlab",
    feature = "github",
    feature = "workspace",
    feature = "demo"
))]

use radar_core::Host;
use radar_core::host::{Event, Hit, MouseKind, View};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[test]
fn one_repositories_tab_switches_between_gitlab_github_and_local_workspaces() {
    let mut host = Host::new(
        vec![
            Box::new(radar_gitlab::GitlabPlugin::new()),
            Box::<radar_github::GithubPlugin>::default(),
            Box::new(radar_workspace::WorkspacePlugin::new()),
        ],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    for plugin in &mut host.plugins {
        plugin.demo();
    }
    let mut local_cache = host.plugins[2].snapshot().unwrap();
    local_cache["indexes"]["/work/acme"]["repositories"] = serde_json::json!([
        {"dir":"/work/acme/repositories/api", "project":"acme/api"}
    ]);
    host.plugins[2].restore(local_cache);
    assert_eq!(host.top_screens(), vec![0]);
    assert_eq!(host.group_screens(0), vec![0, 1, 2]);

    let screen = radar_core::shell::to_text(&host, 100, 30);
    assert!(
        screen.lines().next().unwrap().contains("Repositories"),
        "{screen}"
    );
    assert!(
        screen.lines().nth(1).unwrap().contains("GitLab"),
        "{screen}"
    );
    assert!(
        screen.lines().nth(1).unwrap().contains("GitHub"),
        "{screen}"
    );
    assert!(
        screen.lines().nth(1).unwrap().contains("Local workspaces"),
        "{screen}"
    );
    assert_eq!(
        host.hits
            .borrow()
            .iter()
            .filter(|(_, hit)| matches!(hit, Hit::Tab(View::Screen(_))))
            .count(),
        1
    );

    let target = host
        .hits
        .borrow()
        .iter()
        .find_map(|(rect, hit)| (*hit == Hit::SourceTab(View::Screen(1))).then_some(*rect))
        .unwrap();
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: target.x + 1,
        row: target.y,
        at_ms: 1,
    });
    assert_eq!(host.view, View::Screen(1));
    assert!(host.sources, "clicking a source focuses its row");
    let focused = radar_core::shell::to_text(&host, 100, 30);
    assert!(
        focused.lines().last().unwrap().contains("sources"),
        "{focused}"
    );
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('R'),
        KeyModifiers::NONE,
    )));
    assert_eq!(
        host.view,
        View::Screen(1),
        "Repositories keeps the chosen source"
    );
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('W'),
        KeyModifiers::NONE,
    )));
    assert_eq!(host.view, View::Screen(2));
    let local = radar_core::shell::to_text(&host, 100, 30);
    assert!(local.contains("local repositories"), "{local}");
    assert!(local.contains("acme"), "{local}");
    assert!(local.contains("api"), "{local}");
    host.focus = radar_core::Pane::Middle;
    host.update(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)));
    let selected = radar_core::shell::to_text(&host, 100, 30);
    assert!(
        selected.contains("/work/acme/repositories/api"),
        "{selected}"
    );
}

#[test]
fn vertical_arrows_visit_the_source_row_between_top_tab_and_panes() {
    let mut host = Host::new(
        vec![
            Box::new(radar_gitlab::GitlabPlugin::new()),
            Box::<radar_github::GithubPlugin>::default(),
            Box::new(radar_workspace::WorkspacePlugin::new()),
        ],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    for plugin in &mut host.plugins {
        plugin.demo();
    }
    host.view = View::Screen(1);
    host.focus = radar_core::Pane::Nav;
    let press = |host: &mut Host, code| {
        host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
    };

    press(&mut host, KeyCode::Up);
    assert!(
        host.sources && !host.bar,
        "up from the first repository reaches the source row"
    );
    press(&mut host, KeyCode::Up);
    assert!(
        host.bar && !host.sources,
        "up from sources reaches Repositories"
    );
    press(&mut host, KeyCode::Down);
    assert!(
        host.sources && !host.bar,
        "down from Repositories reaches sources"
    );
    press(&mut host, KeyCode::Right);
    assert_eq!(host.view, View::Screen(2));
    assert!(host.sources, "switching source keeps the row focused");
    press(&mut host, KeyCode::Left);
    assert_eq!(host.view, View::Screen(1));
    press(&mut host, KeyCode::Down);
    assert!(
        !host.sources && !host.bar,
        "down from sources enters the panes"
    );
    assert_eq!(host.focus, radar_core::Pane::Nav);
}

#[test]
fn repositories_key_works_when_gitlab_is_absent() {
    let mut host = Host::new(
        vec![
            Box::<radar_github::GithubPlugin>::default(),
            Box::new(radar_workspace::WorkspacePlugin::new()),
        ],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    // GitHub set up (a group), so its screen has a tab.
    host.load_config(Some("[[github.groups]]\nname = \"github.com/ankh-morpork\"\nowner = \"ankh-morpork\"\n"))
        .unwrap();
    assert_eq!(host.top_screens(), vec![0]);
    host.view = View::Settings;
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('R'),
        KeyModifiers::NONE,
    )));
    assert_eq!(host.view, View::Screen(0));
    let top = radar_core::shell::to_text(&host, 100, 30);
    assert!(
        top.lines().next().unwrap().contains("Repositories"),
        "{top}"
    );
}
