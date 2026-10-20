#![cfg(all(
    feature = "gitlab",
    feature = "github",
    feature = "workspace",
    feature = "demo"
))]

use radar_core::Host;
use radar_core::host::{Event, View};
use radar_core::view::{Chord, Key};
use radar_core::view::Intent;
use radar_core::view::chrome::targets;

/// What shows, as the view data's text form: the top bar and source tabs,
/// the screen, the menu and the status line.
fn text(host: &Host) -> String {
    radar_core::headless::host_text_of(host, 100, 30)
}

/// The labels of the top bar's screen tabs (not history and settings).
fn screen_tabs(host: &Host) -> Vec<String> {
    host.chrome_view(100, 30)
        .top_bar
        .tabs
        .into_iter()
        .filter(|t| t.target.as_str().starts_with("chrome:tab:"))
        .map(|t| t.label)
        .collect()
}

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

    // One tab on the bar for the three screens, and their source tabs.
    let screen = text(&host);
    assert_eq!(screen_tabs(&host), ["repositories"], "{screen}");
    let sources: Vec<String> = host.chrome_view(100, 30).sources.into_iter().map(|s| s.label).collect();
    assert_eq!(sources, ["GitLab", "GitHub", "Local workspaces"], "{screen}");
    assert!(screen.contains("sources: <GitLab>  GitHub  Local workspaces"), "{screen}");

    // A click on GitHub's source tab.
    let target = host
        .chrome_view(100, 30)
        .sources
        .into_iter()
        .find(|s| s.target == targets::source_tab(1))
        .expect("GitHub's source tab")
        .target;
    host.update(Event::Intent(Intent::Activate { target }));
    assert_eq!(host.view, View::Screen(1));
    assert!(host.sources, "clicking a source focuses its row");
    let focused = text(&host);
    let status: Vec<&str> = focused.lines().skip_while(|l| !l.starts_with("── status · ")).collect();
    assert!(status.iter().any(|l| l.contains("sources")), "{focused}");
    host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(Key::Char('R')) }));
    assert_eq!(
        host.view,
        View::Screen(1),
        "Repositories keeps the chosen source"
    );
    host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(Key::Char('W')) }));
    assert_eq!(host.view, View::Screen(2));
    let local = text(&host);
    assert!(local.contains("local repositories"), "{local}");
    assert!(local.contains("acme"), "{local}");
    assert!(local.contains("api"), "{local}");
    host.focus = radar_core::Pane::Middle;
    host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(Key::Down) }));
    let selected = text(&host);
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
        host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(code) }));
    };

    press(&mut host, Key::Up);
    assert!(
        host.sources && !host.bar,
        "up from the first repository reaches the source row"
    );
    press(&mut host, Key::Up);
    assert!(
        host.bar && !host.sources,
        "up from sources reaches Repositories"
    );
    press(&mut host, Key::Down);
    assert!(
        host.sources && !host.bar,
        "down from Repositories reaches sources"
    );
    press(&mut host, Key::Right);
    assert_eq!(host.view, View::Screen(2));
    assert!(host.sources, "switching source keeps the row focused");
    press(&mut host, Key::Left);
    assert_eq!(host.view, View::Screen(1));
    press(&mut host, Key::Down);
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
    host.update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(Key::Char('R')) }));
    assert_eq!(host.view, View::Screen(0));
    let top = text(&host);
    assert_eq!(screen_tabs(&host), ["repositories"], "{top}");
    let on: Vec<String> =
        host.chrome_view(100, 30).top_bar.tabs.into_iter().filter(|t| t.active).map(|t| t.label).collect();
    assert_eq!(on, ["repositories"], "the tab is on:\n{top}");
}
