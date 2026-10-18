//! This build's mix of parts, as a user with a fresh config meets it: the
//! first screen, the parts section, and every `x` menu row on the way. Run
//! for no parts, each part alone and all parts by `scripts/check-mixes`.
//!
//! What is recorded (GUIDE 3, the testing pyramid): the first screen of no
//! parts, each part alone and all parts, each other screen once, with all
//! parts, and one screen per icon pack, with all parts.

use radar_core::Host;
use radar_core::host::{Event, View};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::shell::to_text;

/// A screen, drawn, and compared with `tests/snapshots/` where `compared`
/// says.
macro_rules! record {
    ($compared:expr, $name:expr, $text:expr) => {
        let text = $text;
        if $compared {
            insta::assert_snapshot!($name, text);
        }
    };
}

/// No parts, one part, or every part: the mixes whose first screen is
/// recorded per commit.
fn recorded_mix() -> bool {
    (radar::mix().len() <= 1 && !an_adapter()) || every_part()
}

/// A provider adapter feature (not a part): the screens differ by which
/// adapters are built in, so those builds only run the start-up smoke and
/// record nothing.
fn an_adapter() -> bool {
    cfg!(any(
        feature = "sessions-pty",
        feature = "sessions-copilot",
        feature = "sessions-claude",
        feature = "sessions-codex"
    ))
}

/// Every part: the representative mix, where each screen is recorded.
fn every_part() -> bool {
    cfg!(all(
        feature = "gitlab",
        feature = "github",
        feature = "jira",
        feature = "slack",
        feature = "git",
        feature = "workspace",
        feature = "workbench"
    ))
}

fn mix_name() -> String {
    let mix = radar::mix();
    if mix.is_empty() {
        "none".into()
    } else {
        mix.join("+")
    }
}

/// The same start on every machine: parts look around them for things to
/// offer (variables, files in the folders above), so the tests run with only
/// the variables a build needs and from an empty folder.
fn same_everywhere() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        const KEEP: [&str; 11] = [
            "PATH",
            "HOME",
            "TMPDIR",
            "TEMP",
            "TMP",
            "LANG",
            "TERM",
            "USER",
            "INSTA_UPDATE",
            "INSTA_FORCE_PASS",
            "RADAR_SANDBOX",
        ];
        for (name, _) in std::env::vars_os() {
            let keep = name.to_str().is_some_and(|n| {
                KEEP.contains(&n) || n.starts_with("CARGO") || n.starts_with("RUST")
            });
            if !keep {
                // SAFETY: done once, before any test of this binary reads the
                // environment (each test calls this first).
                unsafe { std::env::remove_var(&name) };
            }
        }
        let empty = std::env::temp_dir().join(format!("radar-mix-{}", std::process::id()));
        std::fs::create_dir_all(&empty).expect("a scratch folder");
        std::env::set_current_dir(&empty).expect("into the scratch folder");
    });
}

/// radar as it starts with a config file that has nothing for the parts yet.
fn started() -> Host {
    same_everywhere();
    let mut host = Host::new(radar::parts(), 1_791_271_000).expect("the parts fit together");
    host.load_config(Some("")).expect("an empty config loads");
    host
}

/// Explicit synthetic fixture, never production restore or credential auth.
#[cfg(feature = "slack")]
fn slack_fixture(host: &mut Host, at: usize, value: serde_json::Value) {
    let cfg: radar_slack::Settings = host.plugins[at].config().unwrap().try_into().unwrap();
    host.plugins[at] =
        Box::new(radar_slack::test_support::offline_plugin(cfg, value["data"].clone()).unwrap());
}

fn press(host: &mut Host, code: KeyCode) -> usize {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
        .len()
}

#[test]
fn the_first_screen_looks_finished_at_80x24() {
    record!(
        recorded_mix(),
        format!("{}_start_80x24", mix_name()),
        to_text(&started(), 80, 24)
    );
}

#[test]
fn the_first_screen_looks_finished_at_160x40() {
    record!(
        recorded_mix(),
        format!("{}_start_160x40", mix_name()),
        to_text(&started(), 160, 40)
    );
}

/// The same start through the Ink renderer's protocol, in process (no
/// node): the renderer says hello, is welcomed and sent the first view, the
/// one Headless reads, and its goodbye ends the run.
#[test]
fn the_first_screen_starts_through_the_ink_protocol() {
    use radar_core::ipc::{Bridge, Capabilities, ImageProtocol, PROTOCOL, ToHost, ToRenderer};
    let mut bridge = Bridge::new(started(), 80, 24);
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: radar_core::view::VERSION,
        capabilities: Capabilities {
            columns: 80,
            rows: 24,
            mouse: true,
            true_colour: true,
            image: ImageProtocol::None,
            plain_text: false,
        },
    });
    let told = bridge.outgoing();
    assert!(matches!(told.first(), Some(ToRenderer::Welcome { .. })), "{told:?}");
    let Some(ToRenderer::Snapshot(first)) = told.iter().find(|m| matches!(m, ToRenderer::Snapshot(_))) else {
        panic!("the first view is sent: {told:?}")
    };
    let headless = radar_core::headless::Headless::new(started(), 80, 24);
    // A section is sent only on the settings view (a screen keeps the host's
    // section index, which Headless::section() reads whatever shows).
    let section = (headless.host().view == View::Settings).then(|| headless.section()).flatten();
    assert_eq!((&first.screen, &first.section), (&headless.screen(), &section), "{}", mix_name());
    assert_eq!(first.chrome, headless.host().chrome_view(80, 24));
    assert!(bridge.receive(ToHost::Bye).quit, "goodbye ends the run");
}

/// With an icon pack, the first screen and the settings keep their shape:
/// icons on the tabs, the sections and the status line. Each pack's glyphs
/// are radar-core's unit tests; here one screen per pack.
fn the_first_screen_and_settings_with(pack: &str) {
    let mut host = started();
    host.colours.cfg.icons = pack.into();
    record!(
        every_part(),
        format!("{}_icons_{pack}_start_160x40", mix_name()),
        to_text(&host, 160, 40)
    );
    host.view = View::Settings;
    record!(
        false,
        format!("{}_icons_{pack}_settings_80x24", mix_name()),
        to_text(&host, 80, 24)
    );
}

#[test]
fn the_first_screen_and_settings_with_unicode_icons() {
    the_first_screen_and_settings_with("unicode");
}

#[test]
fn the_first_screen_and_settings_with_nerd_icons() {
    the_first_screen_and_settings_with("nerd");
}

#[test]
fn the_parts_section_lists_this_mix() {
    let mut host = started();
    host.view = View::Settings;
    host.section = host
        .sections
        .iter()
        .position(|(_, s)| s.id == "parts")
        .expect("the host's parts section");
    let text = to_text(&host, 120, 30);
    for p in radar::parts() {
        assert!(text.contains(p.name()), "{} missing:\n{text}", p.name());
    }
    record!(every_part(), format!("{}_parts_120x30", mix_name()), text);
}

/// The smoke: on a fresh start and (with `demo`) with the parts' sample
/// data, each screen and settings draws, a click on its top-bar tab and its
/// key go there, and the first row of its `x` menu, chosen, acts. What each
/// part's rows and targets do is its own crate's check (radar_core::check).
#[test]
fn each_screen_draws_and_a_click_and_a_key_act() {
    type Start = (&'static str, fn() -> Host);
    #[allow(unused_mut)]
    let mut hosts: Vec<Start> = vec![("fresh", started)];
    #[cfg(feature = "demo")]
    hosts.push(("with data", || {
        same_everywhere();
        radar_core::shot::demo_host(radar::parts()).expect("the parts fit together")
    }));
    for (name, make) in hosts {
        let source = make();
        let views: Vec<(String, char)> = source
            .screens
            .iter()
            .enumerate()
            .map(|(i, (_, s))| {
                if source.group_screens(i).len() == 1 {
                    source.screen_group(i).map_or_else(
                        || (s.name.to_string(), s.key),
                        |group| (group.name.to_string(), group.key),
                    )
                } else {
                    (s.name.to_string(), s.key)
                }
            })
            .chain([("Settings".to_string(), radar_core::host::SETTINGS_KEY)])
            .collect();
        for (i, (tab, key)) in views.iter().enumerate() {
            // A part not set up has no tab: nothing to click.
            if i < source.screens.len() && !source.screen_shown(i) {
                continue;
            }
            // From another view: a click on its tab in the top bar.
            let away = |host: &mut Host| {
                if i == 0 {
                    host.view = View::Settings;
                }
            };
            let mut host = make();
            away(&mut host);
            let top = to_text(&host, 120, 30);
            let bar = top.lines().next().unwrap_or_default();
            let target = if i < host.screens.len() && host.group_screens(i).len() > 1 {
                let representative = host.group_screens(i)[0];
                host.view = View::Screen(representative);
                let _ = to_text(&host, 120, 30);
                host.hits
                    .borrow()
                    .iter()
                    .find_map(|(rect, hit)| {
                        (*hit == radar_core::host::Hit::SourceTab(View::Screen(i))).then_some(*rect)
                    })
                    .expect("source tab")
            } else {
                // By where the bar put the tab, not by its name: on a narrow bar the names
                // give way to their keys.
                let view = if i + 1 == views.len() { View::Settings } else { View::Screen(i) };
                host.hits
                    .borrow()
                    .iter()
                    .find_map(|(rect, hit)| (*hit == radar_core::host::Hit::Tab(view)).then_some(*rect))
                    .unwrap_or_else(|| panic!("{name}: {tab} in the bar: {bar}"))
            };
            host.update(Event::Mouse {
                kind: radar_core::host::MouseKind::Press,
                column: target.x,
                row: target.y,
                at_ms: 1_000,
            });
            let shown = to_text(&host, 120, 30);
            assert!(
                shown.lines().nth(1).is_some_and(|l| !l.trim().is_empty()),
                "{name}: {tab} draws:\n{shown}"
            );
            // Its key goes to the same place.
            let mut keyed = make();
            away(&mut keyed);
            press(&mut keyed, KeyCode::Char(*key));
            let want = if i + 1 == views.len() {
                View::Settings
            } else {
                View::Screen(i)
            };
            assert_eq!(host.view, want, "{name}: a click on {tab} goes there");
            assert_eq!(keyed.view, want, "{name}: {key} goes to {tab}");
            // The first row of its x menu acts.
            press(&mut host, KeyCode::Char('x'));
            if host.menu.is_none() {
                continue; // nothing to do here, and the status line says so
            }
            let before = (
                to_text(&host, 120, 30),
                host.focus,
                host.view,
                host.message.clone(),
            );
            let fx = press(&mut host, KeyCode::Enter);
            let after = (
                to_text(&host, 120, 30),
                host.focus,
                host.view,
                host.message.clone(),
            );
            assert!(
                fx > 0 || after != before,
                "{name}: {tab}'s first x row did nothing"
            );
        }
    }
}

/// No part's `x` row is on a capital another screen of this mix takes:
/// the host would switch screens and the part would never see it.
#[test]
fn no_x_row_is_on_a_screens_key() {
    let screens: Vec<char> = started().screens.iter().map(|(_, s)| s.key).collect();
    #[allow(unused_mut)]
    let mut plugins = radar::parts();
    #[cfg(feature = "demo")]
    for p in &mut plugins {
        p.demo();
    }
    let mut found = Vec::new();
    for p in &plugins {
        let mut rows: Vec<(String, radar_core::Binding)> = Vec::new();
        for s in p.screens() {
            for pane in p.panes(s.id) {
                rows.extend(
                    p.keys(s.id, pane)
                        .into_iter()
                        .map(|b| (format!("{}/{pane:?}", s.id), b)),
                );
            }
        }
        for s in p.sections() {
            rows.extend(
                p.section_keys(s.id)
                    .into_iter()
                    .map(|b| (s.id.to_string(), b)),
            );
        }
        for (at, b) in rows {
            if let Some(KeyEvent { code: KeyCode::Char(c), .. }) = radar_core::plugin::parse_key(&b.keys)
                && screens.contains(&c)
            {
                found.push(format!("{} {at}: `{} {}`", p.name(), b.keys, b.what));
            }
        }
    }
    assert!(found.is_empty(), "{found:#?}");
}

/// Every screen of this mix, and settings: ↑ from the first row reaches the
/// top bar, ← → change the screen, ↓ lands back in the pane left.
#[test]
fn every_screen_reaches_the_top_bar_and_comes_back() {
    let top = started().top_screens();
    let n = top.len();
    for (i, screen) in top
        .iter()
        .copied()
        .map(Some)
        .chain(std::iter::once(None))
        .enumerate()
    {
        let mut host = started();
        host.view = screen.map_or(View::Settings, View::Screen);
        host.focus = radar_core::Pane::Middle;
        let _ = to_text(&host, 160, 40);
        // From the first pane, the nav pane where there is one.
        press(&mut host, KeyCode::Char('1'));
        let pane = host.focus;
        press(&mut host, KeyCode::Char('g'));
        let _ = to_text(&host, 160, 40);
        for _ in 0..20 {
            if host.bar {
                break;
            }
            press(&mut host, KeyCode::Up);
            let _ = to_text(&host, 160, 40);
        }
        assert!(
            host.bar,
            "screen {i} of {n}: ↑ never reached the bar:\n{}",
            to_text(&host, 160, 40)
        );
        let here = host.view;
        if n > 0 {
            press(&mut host, KeyCode::Right);
            assert_ne!(host.view, here, "→ changes the screen");
            press(&mut host, KeyCode::Left);
            let expected = match here {
                View::Screen(index) if host.group_screens(index).len() > 1 => {
                    View::Screen(host.group_screens(index)[0])
                }
                _ => here,
            };
            assert_eq!(host.view, expected, "← back to the same top-level place");
        }
        press(&mut host, KeyCode::Down);
        if host.sources {
            press(&mut host, KeyCode::Down);
        }
        assert_eq!(
            (host.bar, host.sources, host.focus),
            (false, false, pane),
            "↓ lands in the pane left"
        );
    }
}

/// GitLab and Jira set up side by side, from their caches as a user who
/// ran radar before has them: each adds to the other's screens and pages.
/// Nothing reaches a service: tasks are returned, never run, and the
/// tokens name variables that are not set.
#[cfg(all(feature = "gitlab", feature = "jira"))]
mod joined {
    use super::*;
    use serde_json::json;

    const CONFIG: &str = r#"
[gitlab]
url = "https://gitlab.example"
token = "env:RADAR_MIX_SURELY_UNSET"
base = "acme"

[[gitlab.source]]
group = "team-a"
code = "a"
color = "green"

[jira]

[[jira.site]]
name = "acme"
url = "https://acme.atlassian.net"
email = "alice@example.com"
token = "env:RADAR_MIX_SURELY_UNSET"
color = "blue"

[[jira.site.project]]
key = "ABC"

[[jira.view]]
name = "Our project"
jql = "project = ABC ORDER BY updated DESC"
"#;

    fn mr(iid: u64, title: &str, branch: &str, approved: bool) -> serde_json::Value {
        json!({
            "iid": iid, "title": title, "draft": false,
            "web_url": format!("https://gitlab.example/acme/team-a/api/-/merge_requests/{iid}"),
            "updated_at": "2026-10-06T07:00:00Z", "source_branch": branch, "target_branch": "main",
            "notes": 0, "conflicts": false, "merge_status": "NOT_APPROVED",
            "approved": approved, "approvals_required": 1, "approvals_left": if approved { 0 } else { 1 },
            "author": { "username": "alice", "name": "Alice Example" },
            "reviewers": [], "approved_by": if approved { json!(["bob"]) } else { json!([]) },
            "pipeline": "passed", "pipeline_url": null, "additions": 10, "deletions": 2, "files": 1,
            "project_path": "acme/team-a/api", "project_name": "api"
        })
    }

    fn ticket(key: &str, summary: &str, status: &str, category: &str) -> serde_json::Value {
        json!({
            "key": key, "summary": summary, "kind": "Story", "status": status, "category": category,
            "assignee": "Alice Example", "assignee_id": "acc-alice",
            "created": "2026-09-20T09:00:00.000+0000", "updated": "2026-10-06T06:00:00.000+0000",
            "project": "ABC", "project_name": "Acme ABC",
            "description": [{ "t": "para", "text": [{ "text": format!("What {key} is about.") }] }]
        })
    }

    pub(super) fn both() -> Host {
        let mut host = started();
        host.load_config(Some(CONFIG)).expect("the config loads");
        assert_eq!(host.plugins[0].name(), "GitLab");
        host.plugins[0].restore(json!({
            "me": "me",
            "data": { "acme/team-a": {
                "mrs": [
                    mr(263, "ABC-12: Add a stats endpoint for the dashboard", "feature/abc-12-stats", false),
                    mr(264, "Tidy the build", "abc-20-tidy", true),
                    mr(265, "Unrelated change", "tidy", false)
                ],
                "projects": [{ "name": "api", "full_path": "acme/team-a/api", "default_branch": "main" }]
            }}
        }));
        let jira = host.plugins.iter().position(|p| p.id() == "jira").unwrap();
        host.plugins[jira].restore(json!({
            "data": { "acme": {
                "me": { "account_id": "acc-alice", "name": "Alice Example" },
                "tickets": {
                    "ABC-12": ticket("ABC-12", "Add a stats endpoint for the dashboard", "In Review", "doing"),
                    "ABC-20": ticket("ABC-20", "Stats page shows yesterday twice", "To Do", "todo"),
                    "ABC-21": ticket("ABC-21", "Nothing written for it yet", "To Do", "todo")
                },
                "found": { "project = ABC ORDER BY updated DESC": ["ABC-12", "ABC-20", "ABC-21"] }
            }}
        }));
        host
    }

    #[cfg(feature = "slack")]
    #[test]
    fn review_asks_use_the_ticket_links_known_by_jira() {
        let mut host = both();
        // Explicit synthetic Slack authentication; no provider request is needed.
        let cache = host.plugins[0].snapshot().unwrap();
        host.load_config(Some(&format!(
            "{CONFIG}\n[slack]\ntoken = 'env:RADAR_MIX_SURELY_UNSET'\n"
        )))
        .unwrap();
        host.plugins[0].restore(cache);
        let slack = host.plugins.iter().position(|p| p.id() == "slack").unwrap();
        slack_fixture(&mut host, slack, json!({"data": {}}));
        key(&mut host, KeyCode::Char('R'));
        down_to(&mut host, "!263", "!263");
        host.focus = radar_core::Pane::Middle;
        key(&mut host, KeyCode::Char('x'));
        let menu = host.menu.as_mut().expect("MR actions");
        menu.row = menu
            .menu
            .items
            .iter()
            .position(|i| i.label.contains("ask for review in Slack"))
            .expect("review ask action");
        key(&mut host, KeyCode::Enter);
        let shown = to_text(&host, 220, 40);
        assert!(
            shown.contains("<https://acme.atlassian.net/browse/ABC-12|ABC-12>"),
            "{shown}"
        );
    }

    fn key(host: &mut Host, code: KeyCode) {
        let _ = to_text(host, 160, 40);
        press(host, code);
    }

    /// Down the tree on the current screen until a row with `text` is
    /// highlighted in the tree's own panel, then stop.
    fn down_to(host: &mut Host, text: &str, panel: &str) {
        for _ in 0..30 {
            let shown = to_text(host, 160, 40);
            let preview_names_it = shown.lines().skip(1).take(2).any(|l| l.contains(panel));
            if preview_names_it {
                return;
            }
            key(host, KeyCode::Down);
        }
        panic!("never reached {text}:\n{}", to_text(host, 160, 40));
    }

    /// Repositories → up through sources to the bar → Jira → down into Jira's nav pane → up to
    /// the bar → Slack → down: the same keys on every screen.
    #[cfg(feature = "slack")]
    #[test]
    fn the_top_bar_walks_from_repositories_to_jira_to_slack() {
        use radar_core::Pane;
        let mut host = both();
        let screen = |host: &Host| {
            let name = host
                .screen()
                .map_or("settings".to_string(), |(_, s)| s.name.to_lowercase());
            // `Repositories`, `Jira`, `Slack` as the tabs show them.
            let mut c = name.chars();
            c.next()
                .map(|f| f.to_uppercase().chain(c).collect::<String>())
                .unwrap_or_default()
        };
        key(&mut host, KeyCode::Char('R'));
        key(&mut host, KeyCode::Char('1'));
        assert_eq!(
            (screen(&host).as_str(), host.focus),
            ("Repositories", Pane::Nav)
        );
        key(&mut host, KeyCode::Char('g'));
        key(&mut host, KeyCode::Up);
        // The source row shows only where the repositories group has more
        // than one screen (GitLab with GitHub or workspace); where it does
        // not, ↑ goes straight to the bar.
        let repos = host.screens.iter().position(|(_, s)| s.id == "repos").expect("GitLab's screen");
        if host.group_screens(repos).len() > 1 {
            assert!(host.sources, "Repositories' nav pane: ↑ reaches the source row");
        }
        if host.sources {
            key(&mut host, KeyCode::Up);
        }
        assert!(host.bar, "↑ reaches the bar, through the source row when shown");
        key(&mut host, KeyCode::Right);
        assert_eq!((screen(&host).as_str(), host.bar), ("Jira", true));
        key(&mut host, KeyCode::Down);
        assert_eq!(
            (host.bar, host.focus),
            (false, Pane::Nav),
            "into Jira's nav pane"
        );
        key(&mut host, KeyCode::Up);
        assert!(host.bar, "from the nav pane's first row, the bar");
        key(&mut host, KeyCode::Right);
        // The tab after Jira: Slack where it is set up; a part that is not
        // has no tab, so the bar goes on to the next one.
        let after = host
            .top_screens()
            .into_iter()
            .skip_while(|&i| !host.screens[i].1.name.eq_ignore_ascii_case("jira"))
            .nth(1);
        // In a mix where Jira is the last screen's tab (no Slack set up, no
        // git or work), → stays on the bar and the walk ends here.
        let Some(want) = after.map(|i| host.screens[i].1.name.to_lowercase()) else {
            assert!(host.bar, "→ past the last tab stays on the bar");
            return;
        };
        assert_eq!((screen(&host).to_lowercase(), host.bar), (want, true));
        key(&mut host, KeyCode::Down);
        assert!(!host.bar);
        // Back to Jira through the bar: the pane it had, the nav pane.
        key(&mut host, KeyCode::Up);
        let _ = to_text(&host, 160, 40);
        for _ in 0..20 {
            if host.bar {
                break;
            }
            key(&mut host, KeyCode::Up);
        }
        key(&mut host, KeyCode::Left);
        key(&mut host, KeyCode::Down);
        assert_eq!((screen(&host).as_str(), host.focus), ("Jira", Pane::Nav));
    }

    #[test]
    pub(super) fn merge_requests_carry_their_tickets() {
        let mut host = both();
        // Repositories: the ticket column on the merge request naming it.
        key(&mut host, KeyCode::Char('R'));
        let shown = to_text(&host, 220, 40);
        assert!(shown.contains("ABC-12 · In Review"), "{shown}");
        assert!(shown.contains("ABC-20 · To Do"), "from the branch: {shown}");
        record!(
            super::every_part(),
            format!("{}_joined_repositories_220x40", super::mix_name()),
            shown
        );
    }

    #[test]
    pub(super) fn tickets_carry_their_code_at_160x40() {
        let mut host = both();
        // Jira: a code column from GitLab, only where a ticket has some.
        key(&mut host, KeyCode::Char('J'));
        let shown = to_text(&host, 160, 40);
        assert!(shown.contains("CODE") && shown.contains("!263 "), "{shown}");
        record!(
            super::every_part(),
            format!("{}_joined_tickets_160x40", super::mix_name()),
            shown
        );
    }

    #[test]
    pub(super) fn tickets_carry_their_code_at_80x24() {
        let mut host = both();
        key(&mut host, KeyCode::Char('J'));
        let _ = to_text(&host, 160, 40);
        record!(
            super::every_part(),
            format!("{}_joined_tickets_80x24", super::mix_name()),
            to_text(&host, 80, 24)
        );
    }

    /// Jira's own links, read in the background for the tickets in view,
    /// join a merge request to a ticket its title and branch never name,
    /// with no ticket page opened.
    #[test]
    pub(super) fn development_links_read_in_the_background_chip_a_merge_request_without_opening_the_ticket() {
        let spawns = |host: &mut Host, now: i64| {
            host.update(Event::Tick(now))
                .iter()
                .filter(|e| matches!(e, radar_core::host::HostEffect::Spawn { .. }))
                .count()
        };
        let mut host = both();
        key(&mut host, KeyCode::Char('R'));
        let before = to_text(&host, 220, 40);
        assert!(
            !before.contains("ABC-21"),
            "nothing links !265 yet: {before}"
        );
        // Searches give each ticket Jira's number; only that is asked by.
        fn with_ids(host: &mut Host, code: serde_json::Value) {
            let mut tickets = serde_json::Map::new();
            for (n, (k, s, st, c)) in [
                (
                    "ABC-12",
                    "Add a stats endpoint for the dashboard",
                    "In Review",
                    "doing",
                ),
                (
                    "ABC-20",
                    "Stats page shows yesterday twice",
                    "To Do",
                    "todo",
                ),
                ("ABC-21", "Nothing written for it yet", "To Do", "todo"),
            ]
            .into_iter()
            .enumerate()
            {
                let mut t = ticket(k, s, st, c);
                t["id"] = json!((10_001 + n).to_string());
                tickets.insert(k.into(), t);
            }
            let jira = host.plugins.iter().position(|p| p.id() == "jira").unwrap();
            host.plugins[jira].restore(json!({ "data": { "acme": {
                "me": { "account_id": "acc-alice", "name": "Alice Example" },
                "tickets": tickets,
                "found": { "project = ABC ORDER BY updated DESC": ["ABC-12", "ABC-20", "ABC-21"] },
                "code": code
            }}}));
        }
        with_ids(&mut host, json!({}));
        // Nothing held: a tick asks for the three panels beside the rest.
        let asked = spawns(&mut host, 1_791_271_100);
        let link = json!([{
            "id": "!265", "title": "Unrelated change", "status": "open", "repo": "api", "branch": "tidy",
            "url": "https://gitlab.example/acme/team-a/api/-/merge_requests/265"
        }]);
        let entry = |links: serde_json::Value| json!({ "at": 1_791_271_000, "updated": "2026-10-06T06:00:00.000+0000", "links": links });
        // Held and fresh, in a host just as started: the chip is there with
        // no page opened, and those three are not asked again.
        let mut again = both();
        with_ids(
            &mut again,
            json!({ "ABC-12": entry(json!([])), "ABC-20": entry(json!([])), "ABC-21": entry(link) }),
        );
        key(&mut again, KeyCode::Char('R'));
        let shown = to_text(&again, 220, 40);
        assert!(
            shown.contains("ABC-21 · To Do"),
            "the chip, no page opened: {shown}"
        );
        record!(
            super::every_part(),
            format!("{}_joined_background_chip_220x40", super::mix_name()),
            shown
        );
        assert_eq!(
            asked,
            spawns(&mut again, 1_791_271_100) + 3,
            "held and fresh: no panel is asked again"
        );
    }

    #[test]
    pub(super) fn a_tickets_code_tab_shows_the_merge_request_and_esc_comes_back() {
        let mut host = both();
        key(&mut host, KeyCode::Char('J'));
        down_to(&mut host, "ABC-12", "╭ 3 ABC-12 ");
        // The detail pane has GitLab's section.
        let shown = to_text(&host, 160, 40);
        assert!(
            shown.contains("MERGE REQUESTS") && shown.contains("api !263 · "),
            "{shown}"
        );
        key(&mut host, KeyCode::Enter);
        key(&mut host, KeyCode::Char('['));
        let shown = to_text(&host, 160, 40);
        assert!(
            shown.contains("[Code]") && shown.contains("api !263"),
            "{shown}"
        );
        record!(
            super::every_part(),
            format!("{}_joined_ticket_code_tab_160x40", super::mix_name()),
            shown
        );
        key(&mut host, KeyCode::Enter);
        let shown = to_text(&host, 160, 40);
        assert!(shown.contains("Repositories / a / api / !263"), "{shown}");
        // The merge request's page has Jira's ticket section.
        assert!(
            shown.contains("TICKET") && shown.contains("ABC-12 · ◐ In Review · Story"),
            "{shown}"
        );
        record!(
            super::every_part(),
            format!("{}_joined_mr_page_160x40", super::mix_name()),
            shown
        );
        key(&mut host, KeyCode::Esc);
        let shown = to_text(&host, 160, 40);
        assert!(
            shown.contains("Jira / acme / ABC / ABC-12") && shown.contains("[Code]"),
            "{shown}"
        );
    }

    #[test]
    fn selected_merge_request_history_uses_its_owners_scoped_identity_and_returns() {
        let mut host = both();
        key(&mut host, KeyCode::Char('R'));
        down_to(&mut host, "!263", "!263");
        let before = (host.view, host.focus);
        key(&mut host, KeyCode::Char('x'));
        let row = host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .position(|item| item.label == "history for selected item")
            .unwrap();
        host.menu.as_mut().unwrap().row = row;
        let effects = host.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        let [radar_core::host::HostEffect::History(request)] = effects.as_slice() else {
            panic!("selected history must only request a local page: {effects:?}");
        };
        let subject = request.filter.subject.as_ref().unwrap();
        assert_eq!(subject.provider, "gitlab");
        assert_eq!(subject.kind, "merge-request");
        assert!(subject.id.ends_with("!263"));
        assert!(subject.instance.starts_with("https://"));
        assert!(request.filter.since_ms.is_none());
        assert!(to_text(&host, 80, 24).contains("exact subject gitlab:merge-request"));
        key(&mut host, KeyCode::Esc);
        assert_eq!((host.view, host.focus), before);
        assert!(to_text(&host, 160, 40).contains("!263"));
    }

    #[test]
    pub(super) fn jiras_actions_on_a_merge_request_open_and_move_its_ticket() {
        let mut host = both();
        key(&mut host, KeyCode::Char('R'));
        down_to(&mut host, "!263", "!263");
        key(&mut host, KeyCode::Char('x'));
        let menu = &host.menu.as_ref().expect("the actions menu").menu;
        let rows: Vec<(String, String)> = menu
            .items
            .iter()
            .map(|i| (i.keys.clone(), i.label.clone()))
            .collect();
        assert!(rows.contains(&(String::new(), "Jira".into())), "{rows:?}");
        assert!(
            rows.contains(&("v".into(), "view ABC-12".into())),
            "{rows:?}"
        );
        assert!(
            rows.contains(&(">".into(), "move ABC-12 to …".into())),
            "{rows:?}"
        );
        #[cfg(feature = "png")]
        if let Some(directory) = std::env::var_os("CARGO_RADAR_HISTORY_SHOTS") {
            let directory = std::path::PathBuf::from(directory);
            std::fs::create_dir_all(&directory).unwrap();
            let svg = radar_core::shell::to_svg(&host, 100, 40);
            std::fs::write(
                directory.join("history_entry_menu_100x40.png"),
                radar_core::shot::to_png(&svg).unwrap(),
            )
            .unwrap();
        }
        record!(
            super::every_part(),
            format!("{}_joined_mr_keys_100x40", super::mix_name()),
            to_text(&host, 100, 40)
        );
        key(&mut host, KeyCode::Esc);
        // v: the ticket's page, on Jira's screen; esc comes back.
        key(&mut host, KeyCode::Char('v'));
        assert!(to_text(&host, 160, 40).contains("Jira / acme / ABC / ABC-12"));
        key(&mut host, KeyCode::Esc);
        assert!(to_text(&host, 160, 40).contains("Repositories"));
        // >: Jira is asked where it can go (a task, never run here).
        let _ = to_text(&host, 160, 40);
        let fx = host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('>'),
            KeyModifiers::NONE,
        )));
        assert!(
            fx.iter()
                .any(|e| matches!(e, radar_core::host::HostEffect::Spawn { .. })),
            "{fx:?}"
        );
    }
}

/// Real Slack discussions use Jira's URL ownership, even when another
/// configured site watches the same project. Reducer effects are inspected
/// or dropped; no task is polled and no provider or credential is needed.
#[cfg(all(feature = "jira", feature = "slack"))]
#[test]
fn slack_discussions_preserve_foreign_jira_links_and_open_the_configured_site() {
    use radar_core::Pane;
    use radar_core::host::HostEffect;
    use serde_json::json;

    const FOREIGN: &str = "https://foreign.atlassian.net/browse/ABC-12?from=slack#details";
    const BETA: &str = "https://beta.atlassian.net/browse/ABC-12";
    const CONFIG: &str = r#"
[[jira.site]]
name = "acme"
url = "https://acme.atlassian.net"
email = "alice@example.com"
token = "env:RADAR_MIX_SURELY_UNSET"

[[jira.site.project]]
key = "ABC"

[[jira.site]]
name = "beta"
url = "https://beta.atlassian.net"
email = "alice@example.com"
token = "env:RADAR_MIX_SURELY_UNSET"

[slack]
token = "env:RADAR_MIX_SURELY_UNSET"

[[slack.channel]]
alias = "team"
id = "C0000TEAM1"
name = "team"
kind = "team"
watched = true
"#;

    fn key(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
        let _ = to_text(host, 160, 40);
        host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    fn discussion(url: &str) -> Host {
        let mut host = started();
        host.load_config(Some(CONFIG))
            .expect("the Jira and Slack config loads");
        let slack = host
            .plugins
            .iter()
            .position(|p| p.id() == "slack")
            .expect("Slack");
        slack_fixture(
            &mut host,
            slack,
            json!({ "data": {
                "me": { "user_id": "U000ALICE", "user": "alice", "team_id": "T000ACME1",
                    "url": "https://acme.slack.com/", "tz_offset": 0 },
                "users": { "U000ALICE": "Alice Example" },
                "channels": { "C0000TEAM1": { "messages": [{
                    "ts": "1791270000.000100", "user": "U000ALICE", "text": url
                }] } }
            }}),
        );
        for code in [
            KeyCode::Char('S'),
            KeyCode::Char('1'),
            KeyCode::Down,
            KeyCode::Char('2'),
            KeyCode::Char('g'),
        ] {
            key(&mut host, code);
        }
        let _ = to_text(&host, 160, 40);
        host
    }

    // A raw URL still contains a visible ticket key. Its group and action,
    // rather than the presence of those letters, establish its ownership.
    let mut foreign = discussion(FOREIGN);
    let (slack, screen) = foreign.screen().expect("the discussions screen");
    assert_eq!(foreign.plugins[slack].id(), "slack");
    let rows = foreign.plugins[slack].keys(screen.id, Pane::Middle);
    assert_eq!(rows.first().map(|row| row.what.as_str()), Some("general"));
    assert!(rows.iter().any(|row| row.what == "hide what is under it"));
    key(&mut foreign, KeyCode::Down);
    key(&mut foreign, KeyCode::Enter);
    assert_eq!(
        foreign.plugins[slack].depth(screen.id),
        1,
        "the general discussion opens its thread"
    );
    key(&mut foreign, KeyCode::Char(']'));
    let shown = to_text(&foreign, 160, 40);
    let tabs = foreign.plugins[slack]
        .tabs(screen.id)
        .expect("the thread tabs");
    assert_eq!(tabs.names, ["Thread", "Links"]);
    assert_eq!(tabs.on, 1);
    assert!(
        shown.contains(FOREIGN),
        "the original URL is retained in Links:\n{shown}"
    );
    let effects = key(&mut foreign, KeyCode::Enter);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, HostEffect::Open(url) if url == FOREIGN)),
        "{effects:?}"
    );
    assert_eq!(foreign.screen().expect("still on the thread").0, slack);

    // Acme watches ABC; beta deliberately does not. The URL's site still
    // owns this group and the ticket page opened by its Enter action.
    let mut beta = discussion(BETA);
    let (slack, screen) = beta.screen().expect("the discussions screen");
    let rows = beta.plugins[slack].keys(screen.id, Pane::Middle);
    assert_eq!(rows.first().map(|row| row.what.as_str()), Some("ABC-12"));
    assert!(
        rows.iter()
            .any(|row| row.what == "open ABC-12 in radar, or its link")
    );
    key(&mut beta, KeyCode::Enter);
    let (jira, screen) = beta.screen().expect("the ticket screen");
    assert_eq!(beta.plugins[jira].id(), "jira");
    assert_eq!(beta.plugins[jira].depth(screen.id), 1);
    let ticket = beta.plugins[jira]
        .subject(screen.id, Pane::Middle)
        .expect("the opened ticket");
    assert_eq!(ticket.kind, "ticket");
    assert_eq!(ticket.id, "ABC-12");
    assert_eq!(ticket.field("site"), Some("beta"));
    assert_eq!(ticket.url.as_deref(), Some(BETA));
    key(&mut beta, KeyCode::Esc);
    assert_eq!(beta.screen().expect("back to discussions").0, slack);
}

/// GitLab's cache as a user who ran radar before has it: one repo, two merge
/// requests.
#[cfg(feature = "gitlab")]
mod repos {
    use super::*;
    use serde_json::json;

    pub(super) const NOW: i64 = 1_791_271_000;

    pub(super) const GITLAB: &str = r#"
[gitlab]
url = "https://gitlab.example"
token = "env:RADAR_MIX_SURELY_UNSET"
base = "acme"

[[gitlab.source]]
group = "team-a"
code = "a"
color = "green"
"#;

    pub(super) fn gitlab_cache(host: &mut Host) {
        gitlab_cache_as(host, "NOT_APPROVED", false);
    }

    /// The same, with !263 as GitLab says it: `merge_status`, and whether it
    /// is behind its target.
    pub(super) fn gitlab_cache_as(host: &mut Host, merge_status: &str, rebase: bool) {
        let at = host
            .plugins
            .iter()
            .position(|p| p.name() == "GitLab")
            .expect("GitLab");
        host.plugins[at].restore(json!({
            "me": "me",
            "data": { "acme/team-a": {
                "mrs": [{
                    "iid": 263, "title": "Add a stats endpoint", "draft": false,
                    "web_url": "https://gitlab.example/acme/team-a/api/-/merge_requests/263",
                    "updated_at": "2026-10-06T07:00:00Z", "source_branch": "stats", "target_branch": "main",
                    "notes": 0, "conflicts": false, "merge_status": merge_status, "rebase": rebase,
                    "approved": rebase, "approvals_required": 1, "approvals_left": if rebase { 0 } else { 1 },
                    "author": { "username": "alice", "name": "Alice Example" },
                    "reviewers": [], "approved_by": [], "pipeline": "passed", "pipeline_url": null,
                    "additions": 10, "deletions": 2, "files": 1,
                    "project_path": "acme/team-a/api", "project_name": "api"
                }],
                "projects": [
                    { "name": "api", "full_path": "acme/team-a/api", "default_branch": "main" },
                    { "name": "web", "full_path": "acme/team-a/web", "default_branch": "main" }
                ]
            }}
        }));
    }
}

/// A merge request GitLab says is behind its target: its tree row offers the
/// rebase (a key, an `x` row, the detail footer, the status line, a click
/// on its STATE), asks before it and sends nothing on a no, next to any
/// other part.
#[cfg(feature = "gitlab")]
#[test]
fn a_merge_request_that_needs_a_rebase_offers_it_on_its_row_and_asks_first() {
    use radar_core::host::{HostEffect, MouseKind};
    use radar_core::plugin::Pane;
    use radar_core::view::{Intent, PaneId, ScreenView};
    let mut host = started();
    host.load_config(Some(repos::GITLAB))
        .expect("the config loads");
    repos::gitlab_cache_as(&mut host, "NEED_REBASE", true);
    let _ = to_text(&host, 220, 40);
    press(&mut host, KeyCode::Char('R'));
    for _ in 0..2 {
        let _ = to_text(&host, 160, 40);
        press(&mut host, KeyCode::Down);
    }
    let shown = to_text(&host, 160, 40);
    assert!(
        shown.contains("▷ needs rebase") && shown.contains("stats → main"),
        "{shown}"
    );
    let footer = shown.lines().rev().nth(2).expect("the pinned detail footer");
    assert!(
        footer.contains("│update ") && footer.contains("extra actions│"),
        "the compact rebase control and full menu stay at the detail foot:\n{shown}"
    );
    assert!(shown.lines().last().unwrap().contains("update by rebasing"), "on the status line:\n{shown}");
    record!(every_part(), format!("{}_row_actions_160x40", mix_name()), shown);
    // At 80 wide the detail pane is its own screen.
    let mut narrow = host.focus;
    std::mem::swap(&mut narrow, &mut host.focus);
    host.focus = radar_core::plugin::Pane::Detail;
    record!(
        every_part(),
        format!("{}_row_actions_80x24", mix_name()),
        to_text(&host, 80, 24)
    );
    host.focus = narrow;
    // The x menu lists it; choosing it presses u: a question, then a no.
    press(&mut host, KeyCode::Char('x'));
    let rows: Vec<(String, String)> = host
        .menu
        .as_ref()
        .expect("the actions menu")
        .menu
        .items
        .iter()
        .map(|i| (i.keys.clone(), i.label.clone()))
        .collect();
    assert!(
        rows.contains(&("u".into(), "update by rebasing onto main…".into())),
        "{rows:?}"
    );
    assert!(
        rows.contains(&("e".into(), "approve !263…".into())),
        "{rows:?}"
    );
    press(&mut host, KeyCode::Esc);
    let fx = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('u'),
        KeyModifiers::NONE,
    )));
    assert!(fx.is_empty(), "asked, not sent: {fx:?}");
    assert_eq!(
        host.menu.as_ref().expect("a question").menu.id,
        "act-rebase:acme/team-a/api!263"
    );
    let fx = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu.is_none() && !fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })),
        "{fx:?}"
    );
    // A click on the row's STATE asks the same.
    let text = to_text(&host, 160, 40);
    let (y, line) = text
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("▷ needs rebase") && l.contains("!263"))
        .expect("its row");
    let x = line[..line.find("▷ needs rebase").unwrap()].chars().count() as u16;
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: x + 2,
        row: y as u16,
        at_ms: 10_000,
    });
    assert_eq!(
        host.menu.as_ref().expect("a question").menu.id,
        "act-rebase:acme/team-a/api!263"
    );
    let original_question = host.menu.as_ref().unwrap().menu.clone();
    let fx = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu.is_none() && !fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })),
        "cancelling the native row question sends nothing: {fx:?}"
    );

    // The real owner's footer uses a displayed lease, then the same resolver
    // and confirmation as its key and STATE click. No task is ever polled.
    let rebase = |view: &ScreenView| {
        view.panes
            .iter()
            .find(|pane| pane.pane == PaneId::Detail)
            .and_then(|pane| pane.foot_actions.as_ref())
            .expect("the merge request's footer")
            .actions
            .iter()
            .find(|control| control.action.id == "gitlab.repos.rebase")
            .expect("the original rebase action")
            .clone()
    };
    host.focus = Pane::Middle;
    let (gitlab, screen) = host.screen().expect("the repositories screen");
    let raw = host.screen_view(gitlab, screen.id).expect("the raw owner view");
    assert!(rebase(&raw).target.is_none(), "owner data cannot dispatch");
    let live = rebase(&host.displayed_screen(160, 40).expect("the displayed view"));
    assert!(live.disabled.is_none());
    let target = live.target.expect("the displayed rebase control is live");
    let fx = host.update(Event::Intent(Intent::Activate {
        target: target.clone(),
    }));
    assert!(fx.is_empty(), "the footer asks without sending: {fx:?}");
    assert_eq!(host.focus, Pane::Detail, "admission focuses the owning pane");
    assert_eq!(host.menu.as_ref().expect("the original question").menu, original_question);
    assert_eq!(press(&mut host, KeyCode::Right), 0);
    let no_row = host.menu.as_ref().unwrap().row;
    assert_eq!(original_question.items[no_row].value, "no");
    for _ in 0..2 {
        let fx = host.update(Event::Intent(Intent::Activate {
            target: target.clone(),
        }));
        assert!(fx.is_empty(), "a repeated background target cannot dispatch: {fx:?}");
        assert_eq!(host.menu.as_ref().expect("the question stays open").menu, original_question);
        assert_eq!(
            host.menu.as_ref().unwrap().row, no_row,
            "a stale target cannot replace the modal with an identical new question"
        );
    }
    let fx = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu.is_none() && !fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })),
        "cancelling the footer question sends nothing: {fx:?}"
    );
    host.focus = Pane::Middle;
    let fx = host.update(Event::Intent(Intent::Activate {
        target: target.clone(),
    }));
    assert!(fx.is_empty() && host.menu.is_none(), "cancel cannot revive a lease: {fx:?}");
    assert_eq!(host.focus, Pane::Middle, "a retired target cannot steal focus");

    // A fresh lease still asks in read-only mode, and even yes cannot send.
    host.set_env_mode(Some((radar_core::writes::Mode::ReadOnly, "RADAR_MODE".into())));
    let fresh = rebase(&host.displayed_screen(160, 40).expect("the new displayed view"))
        .target
        .expect("the re-acquired rebase control");
    assert_ne!(fresh, target, "the old confirmation retired its lease");
    let fx = host.update(Event::Intent(Intent::Activate { target: fresh }));
    assert!(fx.is_empty(), "read-only activation still asks first: {fx:?}");
    assert_eq!(host.focus, Pane::Detail);
    assert_eq!(
        host.menu.as_ref().expect("the guarded original question").menu,
        original_question.noting(Some(
            "radar is read only (set by RADAR_MODE): nothing will be sent".into()
        ))
    );
    let fx = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu.is_none() && !fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })),
        "the original owner write guard refuses even yes: {fx:?}"
    );
    assert!(
        host.message.as_deref().is_some_and(|message|
            message.ends_with("GitLab did not rebase !263: not sent: radar is read only")
        ),
        "the provider's original read-only refusal: {:?}", host.message
    );
    host.set_env_mode(None);
}

/// Git next to GitLab: a merge request's page says where its repo is
/// checked out here, and its menu, keys and row detail pane carry git's
/// check out, editor, terminal and look again; checking out asks first.
#[cfg(all(feature = "gitlab", feature = "git"))]
#[test]
fn git_says_where_a_repo_is_checked_out_and_checks_out_a_branch_after_asking() {
    use radar_core::host::HostEffect;
    let mut host = started();
    let config = format!("{}\n[git]\nroots = [\"/work\"]\n", repos::GITLAB);
    host.load_config(Some(&config)).expect("the config loads");
    repos::gitlab_cache(&mut host);
    let git = host
        .plugins
        .iter()
        .position(|p| p.name() == "Git")
        .expect("Git");
    host.plugins[git].restore(serde_json::json!({ "index": {
        "checkouts": [{
            "dir": "/work/acme/api", "remote": "git@gitlab.example:acme/team-a/api.git",
            "project": "acme/team-a/api", "branch": "main", "files": ["build.gradle"]
        }],
        "at": repos::NOW - 60, "folders": 3, "cut": false, "missing": []
    }}));
    press(&mut host, KeyCode::Char('R'));
    for _ in 0..2 {
        let _ = to_text(&host, 160, 40);
        press(&mut host, KeyCode::Down);
    }
    let row = to_text(&host, 160, 40);
    let footer = row.lines().rev().nth(2).expect("the pinned detail footer");
    assert!(
        footer.contains("extra actions│"),
        "the detail footer exposes the full contextual menu:\n{row}"
    );
    // Contributed actions live in the Detail menu, reached through its footer.
    let focus = host.focus;
    host.focus = radar_core::Pane::Detail;
    press(&mut host, KeyCode::Char('x'));
    let menu = &host.menu.as_ref().expect("the detail actions menu").menu;
    assert!(
        menu.items.iter().any(|item| item.keys == "c" && item.label == "check out stats…"),
        "the detail menu retains git's checkout action: {:?}",
        menu.items
    );
    press(&mut host, KeyCode::Esc);
    host.focus = focus;
    assert!(
        row.contains("api  main"),
        "the repo row says it is cloned here, on its branch:\n{row}"
    );
    let _ = to_text(&host, 160, 40);
    press(&mut host, KeyCode::Enter);
    let page = to_text(&host, 160, 40);
    assert!(
        page.contains("checkout  terminal  editor  /work/acme/api · java · on main"),
        "{page}"
    );
    // A click on `terminal` under the title: the clone is on main, not the
    // merge request's branch, so it asks where to open it.
    let (y, l) = page
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("checkout  terminal  editor"))
        .expect("the line");
    let x = l[..l.find("terminal").unwrap()].chars().count() as u16;
    host.update(Event::Mouse {
        kind: radar_core::host::MouseKind::Press,
        column: x + 2,
        row: y as u16,
        at_ms: 0,
    });
    assert_eq!(
        host.menu.as_ref().expect("asked where").menu.title,
        "open stats: where?"
    );
    press(&mut host, KeyCode::Esc);
    press(&mut host, KeyCode::Char('x'));
    let rows: Vec<(String, String)> = host
        .menu
        .as_ref()
        .expect("the actions menu")
        .menu
        .items
        .iter()
        .map(|i| (i.keys.clone(), i.label.clone()))
        .collect();
    for want in [
        ("", "Git"),
        ("c", "check out stats…"),
        ("e", "open in an editor on stats…"),
        ("t", "open a terminal on stats…"),
        ("s", "look for checkouts again"),
    ] {
        assert!(
            rows.contains(&(want.0.into(), want.1.into())),
            "{want:?} in {rows:?}"
        );
    }
    press(&mut host, KeyCode::Esc);
    let fx = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('c'),
        KeyModifiers::NONE,
    )));
    assert!(fx.is_empty(), "asked, not done: {fx:?}");
    let menu = &host.menu.as_ref().expect("a question").menu;
    assert_eq!(menu.title, "check out stats in /work/acme/api?");
    let fx = host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('n'),
        KeyModifiers::NONE,
    )));
    assert!(
        host.menu.is_none() && !fx.iter().any(|e| matches!(e, HostEffect::Spawn { .. })),
        "{fx:?}"
    );
    // A terminal on its branch: asked where; as it is, nothing is checked out.
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Char('t'),
        KeyModifiers::NONE,
    )));
    assert_eq!(
        host.menu.as_ref().expect("asked where").menu.title,
        "open stats: where?"
    );
}

/// GitLab alone: nothing about alerts anywhere on its repos.
#[cfg(all(feature = "gitlab", not(feature = "slack")))]
#[test]
fn without_slack_gitlab_shows_nothing_about_alerts() {
    let mut host = started();
    host.load_config(Some(repos::GITLAB))
        .expect("the config loads");
    repos::gitlab_cache(&mut host);
    let _ = to_text(&host, 220, 40);
    press(&mut host, KeyCode::Char('R'));
    for (w, h) in [(220, 40), (80, 24)] {
        let shown = to_text(&host, w, h).to_lowercase();
        assert!(
            shown.contains("api") && !shown.contains("firing") && !shown.contains("alert"),
            "{shown}"
        );
    }
    // Its repo page, every tab: no marker, no Alerts tab.
    let _ = to_text(&host, 160, 40);
    press(&mut host, KeyCode::Down);
    let _ = to_text(&host, 160, 40);
    press(&mut host, KeyCode::Enter);
    let shown = to_text(&host, 160, 40);
    assert!(shown.contains("Repositories /"), "{shown}");
    assert!(
        !shown.to_lowercase().contains("alert") && !shown.contains("firing"),
        "{shown}"
    );
}

/// GitLab and Slack set up side by side: an alert the rules place on a repo
/// marks its row and its page, and its Alerts tab opens the alert's thread.
/// Nothing reaches a service: tasks are returned, never run.
#[cfg(all(feature = "gitlab", feature = "slack"))]
mod alerts {
    use super::repos::{GITLAB, NOW, gitlab_cache};
    use super::*;
    use serde_json::json;

    const SLACK: &str = r#"
[slack]
token = "env:RADAR_MIX_SURELY_UNSET"

[[slack.channel]]
alias = "team-alerts"
id = "C0000ALRT1"
name = "team-alerts"
kind = "alerts"
reader = "alertmanager"
color = "orange"
watched = true

[[slack.alerts.map]]
prefix = "api-"
repo = "acme/team-a/api"
service = "api"
"#;

    fn alert(ts: &str, title: &str, color: &str, env: &str) -> serde_json::Value {
        json!({
            "ts": ts, "bot_name": "alertbot", "subtype": "bot_message",
            "cards": [{ "color": color, "title": title,
                "text": format!("*Environment:* {env}\n*Priority:* P1\n*Console:* <https://console.acme.example/alerts/42|console>") }]
        })
    }

    fn both() -> Host {
        let mut host = started();
        host.load_config(Some(&format!("{GITLAB}{SLACK}")))
            .expect("the config loads");
        gitlab_cache(&mut host);
        let at = host
            .plugins
            .iter()
            .position(|p| p.name() == "Slack")
            .expect("Slack");
        // Slack's offline fixture counts from the demo clock (`DEMO_NOW`, the
        // same as `NOW`), so the alerts are dated from it and their age does
        // not move while the test runs: 3h 07m 30s ago.
        let now = NOW;
        let (fired, other) = (
            format!("{}.000100", now - 3 * 3600 - 450),
            format!("{}.000200", now - 7200),
        );
        slack_fixture(
            &mut host,
            at,
            json!({ "data": {
                "me": { "user_id": "U0000ALICE", "user": "alice", "team": "Acme", "team_id": "T000ACME1",
                    "url": "https://acme.slack.com/", "tz_offset": 0 },
                "channels": { "C0000ALRT1": { "messages": [
                    alert(&fired, "[FIRING:1] api-HighErrorRate", "danger", "prod"),
                    alert(&other, "[FIRING:1] web-Slow", "danger", "prod"),
                ] } }
            }}),
        );
        host
    }

    fn key(host: &mut Host, code: KeyCode) {
        let _ = to_text(host, 160, 40);
        press(host, code);
    }

    /// The screen with each alert's age (`firing 3h 07m`) as `#h ##m`, so
    /// the snapshots do not hold a clock reading. Its width stays, so the
    /// layout is still checked.
    fn aged(shown: &str) -> String {
        let mut out = String::with_capacity(shown.len());
        let mut rest = shown;
        while let Some(at) = rest.find("firing ") {
            let (head, tail) = rest.split_at(at + "firing ".len());
            out.push_str(head);
            let age = tail
                .find(|c: char| !(c.is_ascii_digit() || matches!(c, 'd' | 'h' | 'm' | 's' | ' ')))
                .unwrap_or(tail.len());
            let age = if tail.starts_with(|c: char| c.is_ascii_digit()) {
                age
            } else {
                0
            };
            out.extend(
                tail[..age]
                    .chars()
                    .map(|c| if c.is_ascii_digit() { '#' } else { c }),
            );
            rest = &tail[age..];
        }
        out.push_str(rest);
        out
    }

    #[test]
    pub(super) fn a_firing_alert_marks_its_repo_row_and_not_a_repo_no_rule_names() {
        let mut host = both();
        let _ = to_text(&host, 220, 40);
        press(&mut host, KeyCode::Char('R'));
        let shown = to_text(&host, 220, 40);
        let api = shown
            .lines()
            .find(|l| l.contains(" api "))
            .expect("the api row");
        assert!(api.contains("firing · 1"), "{shown}");
        let web = shown
            .lines()
            .find(|l| l.contains(" web "))
            .expect("the web row");
        assert!(!web.contains("firing"), "web-Slow is on no repo: {shown}");
        record!(
            super::every_part(),
            format!("{}_alerts_repositories_220x40", super::mix_name()),
            aged(&shown)
        );
    }

    #[test]
    pub(super) fn the_repo_page_carries_the_marker_and_an_alerts_tab_whose_row_opens_the_thread() {
        let mut host = both();
        key(&mut host, KeyCode::Char('R'));
        key(&mut host, KeyCode::Down);
        key(&mut host, KeyCode::Enter);
        let shown = to_text(&host, 160, 40);
        assert!(
            shown.contains("Repositories / a / api") && shown.contains("firing · 1"),
            "{shown}"
        );
        assert!(shown.contains("Alerts"), "{shown}");
        key(&mut host, KeyCode::Char('['));
        let shown = aged(&to_text(&host, 160, 40));
        assert!(
            shown.contains("▲ api-HighErrorRate · prod · P1 · firing #h ##m · #team-alerts"),
            "{shown}"
        );
        record!(
            super::every_part(),
            format!("{}_alerts_repo_page_160x40", super::mix_name()),
            shown
        );
        record!(
            super::every_part(),
            format!("{}_alerts_repo_page_80x24", super::mix_name()),
            aged(&to_text(&host, 80, 24))
        );
        // ⏎: the alert's thread, on Slack's screen; esc comes back to the repo.
        key(&mut host, KeyCode::Enter);
        let shown = to_text(&host, 160, 40);
        assert!(
            shown.contains("Slack / #team-alerts / alertbot:"),
            "{shown}"
        );
        key(&mut host, KeyCode::Esc);
        assert!(to_text(&host, 160, 40).contains("Repositories / a / api"));
    }
}
