//! The workspace part on its own, and what it adds to the tickets other
//! parts draw (as subjects, with the shared vocabulary).

use radar_core::host::{Event, View};
use radar_core::plugin::{Effect, Pane};
use radar_core::view::{Chord, Intent, Key};
use radar_core::{Contribution, Host, Plugin, Point, Setup, Showing, Subject};

use crate::WorkspacePlugin;
use crate::demo::{KEY, sample};

fn hosted() -> Host {
    let mut host = Host::new(vec![Box::new(sample())], radar_core::plugin::DEMO_NOW).unwrap();
    host.load_config(Some("")).unwrap();
    // load_config re-configures the plugin from the (empty) file; put the
    // sample back the way a loaded config would.
    host.plugins[0] = Box::new(sample());
    host
}

/// An action of the local repositories screen invoked by its id, through the host,
/// does what its key does: `workspace.local_repositories.look_again` is `r`.
#[test]
fn an_action_invoked_by_its_id_runs_what_its_key_runs() {
    use radar_core::headless::Headless;
    use radar_core::view::{Chord, Intent, PaneId};
    let make = || {
        let mut headless = Headless::new(hosted(), 80, 24);
        headless.focus(PaneId::Middle);
        headless
    };
    let by_key = make().send(Intent::Key { chord: Chord::parse("r").unwrap() });
    let by_id = make().send(Intent::Invoke {
        action: "workspace.local_repositories.look_again".into(),
    });
    assert!(!by_key.is_empty(), "the key does something on the screen");
    assert_eq!(format!("{by_id:?}"), format!("{by_key:?}"));
}

fn section(host: &mut Host, name: &str) {
    host.view = View::Settings;
    host.section = host
        .sections
        .iter()
        .position(|(_, s)| s.name == name)
        .unwrap();
}

fn ticket(key: &str) -> Subject {
    Subject::new("ticket", key, key)
        .with("title", "Add stats")
        .with("project", key.split('-').next().unwrap())
}

/// What shows, as view text (`radar_core::headless::text`): the screen's
/// panes and rows, the open menu and the status line. No painted frame.
fn shown(host: &Host, width: u16, height: u16) -> String {
    radar_core::headless::text_of(host, width, height)
}

fn press(host: &mut Host, code: Key) {
    let _ = shown(host, 160, 40);
    host.update(Event::Intent(Intent::Key {
        chord: Chord::new(code),
    }));
}

/// `key: what` for each action the part adds to a subject.
fn actions(p: &WorkspacePlugin, s: &Subject) -> Vec<String> {
    match p.contribute(Point::Actions, s) {
        Some(Contribution::Actions(a)) => a
            .into_iter()
            .map(|a| {
                format!(
                    "{}: {}",
                    a.key.map(String::from).unwrap_or_default(),
                    a.what
                )
            })
            .collect(),
        other => panic!("no actions: {other:?}"),
    }
}

#[test]
fn a_ticket_gets_a_notes_tab_readme_first_each_row_a_note_to_open() {
    let p = sample();
    let Some(Contribution::Tab { title, rows }) = p.contribute(Point::Tab, &ticket(KEY)) else {
        panic!("no tab")
    };
    assert_eq!(title, "Notes");
    let lines: Vec<String> = rows.iter().map(|r| r.line.to_string()).collect();
    assert_eq!(
        lines,
        [
            "ticket · planning · README.md",
            "spec · approved · spec-ui.md",
            "spec · draft · spec-backend.md",
        ]
    );
    let first = rows[0].subject.as_ref().unwrap();
    assert!(first.is("note"));
    assert_eq!(
        first.url.as_deref(),
        Some("file:///work/acme/artifacts/2026-01-01/ABC-12-stats/README.md")
    );
    assert_eq!(first.field("ticket"), Some(KEY));
    assert_eq!(first.field("type"), Some("ticket"));
    assert_eq!(first.field("status"), Some("planning"));
    // A ticket with no notes gets no tab: absent means absent.
    assert!(p.contribute(Point::Tab, &ticket("ABC-99")).is_none());
    assert!(p.contribute(Point::Section, &ticket("ABC-99")).is_none());
    // And nothing on what is not a ticket.
    let repo = Subject::new("repo", "acme/a/api", "api");
    assert!(p.contribute(Point::Tab, &repo).is_none());
    assert!(p.contribute(Point::Actions, &repo).is_none());
}

#[test]
fn the_notes_section_is_the_readmes_first_lines() {
    let p = sample();
    let Some(Contribution::Section { title, lines, .. }) = p.contribute(Point::Section, &ticket(KEY))
    else {
        panic!("no section")
    };
    assert_eq!(title, "Notes");
    assert_eq!(lines[0].to_string(), "# ABC-12: Add stats");
    assert_eq!(lines.len(), 3);
}

#[test]
fn the_actions_open_the_workspace_and_a_terminal_and_look_again() {
    let mut p = sample();
    assert_eq!(
        actions(&p, &ticket(KEY)),
        [
            "s: start work on ABC-12…",
            "e: open acme in an editor…",
            ": open a terminal at the top of acme",
            "r: look for notes again",
        ]
    );
    p.cfg.spaces[0].editor = "nvim".into();
    assert_eq!(actions(&p, &ticket(KEY))[1], "e: open acme in nvim");
    // Each acts: a task off the event loop.
    for a in ["editor", "terminal", "reindex"] {
        let fx = p.act(&ticket(KEY), a);
        assert!(
            fx.iter()
                .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. })),
            "{a}: {fx:?}"
        );
    }
    assert!(p.state.indexing.contains("/work/acme"));
    // Looking again while it looks says so, and starts nothing more.
    let fx = p.act(&ticket(KEY), "reindex");
    assert!(
        matches!(&fx[..], [Effect::Observe { outcome: radar_core::journal::Outcome::Blocked, .. }, Effect::Notify(m)] if m == "already looking for notes"),
        "{fx:?}"
    );
}

#[test]
fn a_ticket_is_worked_in_the_workspace_that_lists_its_project() {
    let mut p = sample();
    let mut other = crate::config::Space::new("/work/other");
    other.tickets = vec!["XYZ".into()];
    p.cfg.spaces.push(other);
    assert!(p.contribute(Point::Tab, &ticket(KEY)).is_some());
    // XYZ-1 is the other workspace's: not indexed yet, so no notes.
    assert!(p.contribute(Point::Tab, &ticket("XYZ-1")).is_none());
    assert!(actions(&p, &ticket("XYZ-1"))[1].contains("open other"));
    // A project no one lists goes to the first.
    assert!(actions(&p, &ticket("QQ-1"))[1].contains("open acme"));
}

#[test]
fn opening_at_the_notes_folder_when_the_workspace_says_repo() {
    let mut p = sample();
    assert_eq!(p.open_dir(&p.cfg.spaces[0].clone(), KEY), "/work/acme");
    p.cfg.spaces[0].open_at = "repo".into();
    // The folder is not on this machine, so the top it is.
    assert_eq!(p.open_dir(&p.cfg.spaces[0].clone(), KEY), "/work/acme");
}

#[test]
fn it_owns_workspaces_and_notes_and_a_note_is_shown_by_opening_it() {
    let mut p = sample();
    let owned: Vec<&str> = p.owns().iter().map(|o| o.kind).collect();
    assert_eq!(owned, ["workspace", "note"]);
    assert!(p.owns().iter().all(|o| !o.page));
    let Some(Contribution::Tab { rows, .. }) = p.contribute(Point::Tab, &ticket(KEY)) else {
        panic!()
    };
    let note = rows[0].subject.clone().unwrap();
    let Showing::Done(fx) = p.show(&note) else {
        panic!("a note is shown by opening it")
    };
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. })),
        "{fx:?}"
    );
    assert!(matches!(
        p.show(&Subject::new("ticket", KEY, KEY)),
        Showing::Cannot
    ));
    let adds: Vec<&str> = p.adds().iter().map(|a| a.on).collect();
    assert_eq!(adds, ["ticket"]);
}

#[test]
fn without_a_workspace_the_part_is_not_set_up_and_says_how() {
    let p = WorkspacePlugin::new();
    let Setup::Needed(why) = p.set_up() else {
        panic!("ready with nothing")
    };
    assert!(why.contains("settings, workspace › workspaces"), "{why}");
    assert_eq!(sample().set_up(), Setup::Ready);
    // Nothing is written for the default.
    assert!(p.config().is_none());
}

#[test]
fn the_config_round_trips_through_the_plugin() {
    let mut p = WorkspacePlugin::new();
    let table: toml::Value = toml::from_str(
        "[[space]]\ndir = \"/work/acme\"\ntickets = [\"ABC\"]\nopen_at = \"sideways\"\n",
    )
    .unwrap();
    p.configure(Some(&table)).unwrap();
    assert_eq!(
        p.cfg.spaces[0].open_at, "top",
        "an unknown open_at is put right"
    );
    let back = p.config().unwrap();
    let mut again = WorkspacePlugin::new();
    again.configure(Some(&back)).unwrap();
    assert_eq!(again.cfg, p.cfg);
    assert!(toml::to_string(&back).unwrap().contains("[[space]]"));
}

#[test]
fn the_cache_keeps_the_index_and_what_was_found() {
    let p = sample();
    let mut back = WorkspacePlugin::default();
    back.restore(p.snapshot().unwrap());
    assert_eq!(back.state.indexes, p.state.indexes);
    assert_eq!(back.state.offers, p.state.offers);
}

#[test]
fn the_index_is_built_at_start_for_each_workspace_and_a_finished_task_keeps_it() {
    let mut p = WorkspacePlugin::default();
    p.cfg.spaces = vec![crate::config::Space::new("/definitely/not/here")];
    let fx = p.tick(5);
    // One task for the notes, one looking for workspaces.
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. }))
            .count(),
        2,
        "{fx:?}"
    );
    assert!(p.state.indexing.contains("/definitely/not/here") && p.state.finding);
    // Ticking again starts nothing while they run.
    assert!(p.tick(6).is_empty());
    let fx = p.on_work(crate::WorkMsg::Indexed {
        dir: "/definitely/not/here".into(),
        index: crate::notes::build(&p.cfg.spaces[0].clone(), 5),
        observed: crate::find::Observation::default(),
        action: crate::read_action("workspace.notes.index", Some("/definitely/not/here")),
    });
    assert!(matches!(
        fx.first(),
        Some(Effect::Report(radar_core::journal::Outcome::Blocked))
    ));
    assert!(matches!(fx.last(), Some(Effect::PersistCache)));
    assert!(p.state.indexes["/definitely/not/here"].missing);
    assert!(p.tick(7).is_empty(), "indexed once, not again");
}

/// Every row and target of the settings section is its own and acts, and
/// what the part adds to a ticket's `x` menu returns effects
/// (radar_core::check, on the plugin alone).
#[test]
fn every_row_and_target_is_taken() {
    use radar_core::check::{Check, Place};
    let make = || Box::new(sample()) as Box<dyn Plugin>;
    let mut check = Check::new(&make);
    let places: Vec<Place> = (0..4)
        .map(|row| Place::section("workspaces").down(row))
        .collect();
    let gaps = check.places(&places);
    assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
    let n = check.actions(&ticket(KEY));
    assert_eq!(n, 4);
    assert!(check.gaps.is_empty(), "{:?}", check.gaps);
    assert!(check.checked > 12, "only {} checked", check.checked);
}

#[test]
fn settings_workspaces() {
    let mut host = hosted();
    section(&mut host, "workspaces");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn settings_workspaces_a_workspace_row() {
    let mut host = hosted();
    section(&mut host, "workspaces");
    host.focus = Pane::Middle;
    for _ in 0..2 {
        press(&mut host, Key::Down);
    }
    insta::assert_snapshot!(shown(&host, 160, 40));
}

/// Typing through the host: add a workspace by its folder (refused until it
/// is a folder), and the new row is looked into.
#[test]
fn adding_a_workspace_is_typed_validated_and_looked_into() {
    let mut host = hosted();
    section(&mut host, "workspaces");
    host.focus = Pane::Middle;
    // The found workspace is the first row; the add row is next.
    press(&mut host, Key::Down);
    press(&mut host, Key::Enter);
    let field = |host: &radar_core::Host| host.menu.as_ref().is_some_and(|m| m.input.is_some());
    assert!(field(&host), "a field to type the folder in");
    for c in "/definitely/not/here".chars() {
        press(&mut host, Key::Char(c));
    }
    press(&mut host, Key::Enter);
    assert!(field(&host), "a folder that is not there is refused");
    assert_eq!(host.message.as_deref(), Some("not yet: not a folder on this machine"));
    // A workspace with clones under repositories/ and notes by ticket.
    let scratch = crate::find::fixture::Scratch::new("adding");
    scratch.clone("top", "git@example.com:acme/top.git");
    scratch.clone("top/repositories/team-a/api", "git@example.com:acme/team-a/api.git");
    scratch.clone("top/repositories/team-a/web", "git@example.com:acme/team-a/web.git");
    scratch.file("top/.okf.yaml", "artifacts: {}\n");
    scratch.file("top/artifacts/2026-01-01/ABC-12-stats/README.md", "# ABC-12\n");
    scratch.file("top/artifacts/ABC-14/README.md", "# ABC-14\n");
    scratch.file("top/artifacts/done/DEF-3/README.md", "# DEF-3\n");
    press(&mut host, Key::Esc);
    press(&mut host, Key::Enter);
    for c in scratch.path("top").display().to_string().chars() {
        press(&mut host, Key::Char(c));
    }
    let said = host.menu.as_ref().unwrap().menu.items[0].label.clone();
    assert_eq!(said, "✓ 2 clones under repositories/ · notes by .okf.yaml · tickets ABC, DEF");
    press(&mut host, Key::Enter);
    assert!(!field(&host));
    let cfg = host.plugins[0].config().unwrap();
    let text = toml::to_string(&cfg).unwrap();
    assert!(text.contains(&scratch.path("top").display().to_string()), "{text}");
    assert!(text.contains("tickets = [\"ABC\", \"DEF\"]"), "the projects its notes are about:\n{text}");
}

#[test]
fn a_found_workspace_is_a_question_used_with_y_or_passed_with_n() {
    let mut p = sample();
    assert_eq!(crate::settings::SECTIONS[0].group, "workspace");
    let fx = p.on_menu("ws-found:0", "use");
    assert_eq!(p.cfg.spaces.len(), 2);
    assert_eq!(p.cfg.spaces[1].dir, "/work/other");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. })),
        "{fx:?}"
    );
    let mut p = sample();
    assert!(p.on_menu("ws-found:0", "pass").is_empty());
    assert_eq!(p.cfg.spaces.len(), 1);
    let rows_before = p.state.passed.len();
    assert_eq!(rows_before, 1, "not asked again this time");
}

#[test]
fn removing_a_workspace_asks_first() {
    let mut p = sample();
    let fx = p.on_menu("ws-space:0", "remove");
    let [Effect::Menu(m)] = &fx[..] else {
        panic!("{fx:?}")
    };
    assert!(m.question && m.title.contains("acme"));
    assert_eq!(p.cfg.spaces.len(), 1, "nothing is done before yes");
    p.on_menu("ws-remove:0", "no");
    assert_eq!(p.cfg.spaces.len(), 1);
    p.on_menu("ws-remove:0", "yes");
    assert!(p.cfg.spaces.is_empty());
    assert!(p.state.indexes.is_empty());
}

#[test]
fn an_editor_picked_is_kept_for_the_workspace_and_opens() {
    let mut p = sample();
    let fx = p.on_menu("workspace-editor:/work/acme\tfolder\t/work/acme", "nvim");
    assert_eq!(p.cfg.spaces[0].editor, "nvim");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Spawn(_) | Effect::SpawnAction { .. })),
        "{fx:?}"
    );
}

/// A tracker that knows only the vocabulary: it owns tickets and nothing
/// else (GUIDE 1.13's stand-in).
struct Tracker;

impl Plugin for Tracker {
    fn id(&self) -> &'static str {
        "tracker"
    }

    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }

    fn config(&self) -> Option<toml::Value> {
        None
    }

    fn owns(&self) -> Vec<radar_core::Owned> {
        vec![radar_core::Owned::page("ticket", "tickets")]
    }
}

/// Next to a part that owns tickets, through radar-core's composition: the
/// notes arrive as a tab and a section, the actions under the part's name
/// with the keys the menu keeps.
#[test]
fn next_to_a_tracker_a_ticket_gets_the_notes_a_section_and_actions() {
    let host = Host::new(
        vec![Box::new(Tracker), Box::new(sample())],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    let parts = radar_core::Parts::new(&host.plugins, Some(0));
    let t = ticket(KEY);
    let tabs = parts.tabs(&t);
    assert_eq!(tabs.len(), 1);
    assert_eq!((tabs[0].0.as_str(), tabs[0].1.len()), ("Notes", 3));
    let sections = parts.sections(&t);
    assert_eq!(sections[0].0, "Notes");
    let acts = parts.actions(&t, &[]);
    assert_eq!(acts[0].0, "Workspace");
    let keys: Vec<Option<char>> = acts[0].1.iter().map(|a| a.key).collect();
    assert_eq!(keys, [Some('s'), Some('e'), None, Some('r')]);
    // Where the tracker's own rows use `e`, the row stays and is chosen from the menu.
    let keys: Vec<Option<char>> = parts.actions(&t, &['e'])[0]
        .1
        .iter()
        .map(|a| a.key)
        .collect();
    assert_eq!(keys, [Some('s'), None, None, Some('r')]);
    // The part owns notes and workspaces next to the tracker's tickets.
    assert!(parts.has("ticket"));
    assert!(radar_core::compose::owner_of(&host.plugins, "note").is_some());
    // Alone, with no workspace configured, it adds nothing anywhere.
    let host = Host::new(vec![Box::new(Tracker), Box::new(WorkspacePlugin::new())], 0).unwrap();
    let parts = radar_core::Parts::new(&host.plugins, Some(0));
    assert!(parts.tabs(&ticket(KEY)).is_empty() && parts.actions(&ticket(KEY), &[]).is_empty());
    assert!(radar_core::compose::owner_of(&host.plugins, "note").is_none());
}
