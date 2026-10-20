//! The git part on its own, and what it adds to the repos and merge
//! requests other parts draw (as subjects, with the shared vocabulary).

use radar_core::host::{Event, View};
use radar_core::journal::Outcome as JournalOutcome;
use radar_core::plugin::{Effect, Pane};
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::ui::chips::words;
use radar_core::{Contribution, Host, Plugin, Point, Subject};

use crate::GitPlugin;
use crate::demo::sample;

fn hosted() -> Host {
    let mut host = Host::new(vec![Box::new(sample())], radar_core::plugin::DEMO_NOW).unwrap();
    host.load_config(Some("")).unwrap();
    // load_config re-configures the plugin from the (empty) file; put the
    // sample back the way a loaded config would.
    host.plugins[0] = Box::new(sample());
    host
}

/// What shows, as view text (`radar_core::headless::text`): the screen's
/// panes and rows, the open menu and the status line. No painted frame.
pub(crate) fn shown(host: &Host, width: u16, height: u16) -> String {
    radar_core::headless::text_of(host, width, height)
}

/// The part's words on the right of the top bar: what the text form leaves
/// out of the screen.
pub(crate) fn top_bar_words(host: &Host) -> Option<String> {
    host.chrome_view(160, 40).top_bar.words.map(|badge| badge.text)
}

/// The words of the notices showing: what the text form leaves out.
pub(crate) fn notices(host: &Host) -> String {
    let active = host.chrome_view(160, 40).notices.active;
    active.into_iter().map(|n| n.text).collect::<Vec<_>>().join("\n")
}

fn section(host: &mut Host, name: &str) {
    host.view = View::Settings;
    host.section = host
        .sections
        .iter()
        .position(|(_, s)| s.name == name)
        .unwrap();
}

fn repo(path: &str) -> Subject {
    Subject::new("repo", path, path.rsplit('/').next().unwrap()).with("project", path)
}

fn mr(path: &str, branch: &str) -> Subject {
    Subject::new("merge-request", format!("{path}!263"), "api !263")
        .with("project", path)
        .with("iid", "263")
        .with("branch", branch)
}

/// The words of the line git puts under a subject's title.
fn line(p: &GitPlugin, s: &Subject) -> String {
    let t = radar_core::Theme::new(radar_core::design::theme::Tier::Mono);
    match p.contribute(Point::Line, s) {
        Some(Contribution::Line(w)) => words(&t, &w).to_string(),
        other => panic!("no line: {other:?}"),
    }
}

/// `key: what` for each action git adds to a subject.
fn actions(p: &GitPlugin, s: &Subject) -> Vec<String> {
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
fn a_repo_and_a_merge_request_say_where_they_are_checked_out() {
    let mut p = sample();
    let api = "acme/team-a/api";
    // Where, then a terminal there and an editor on it, each a click.
    assert_eq!(
        line(&p, &repo(api)),
        "checkout  terminal  editor  /work/acme/api · java · on main"
    );
    let Some(Contribution::Line(w)) = p.contribute(Point::Line, &repo(api)) else { panic!() };
    let acts: Vec<(&str, Option<&str>)> = w.iter().filter(|c| c.action.is_some()).map(|c| (c.text.as_str(), c.action.as_deref())).collect();
    assert_eq!(acts, [("terminal", Some("terminal")), ("editor", Some("editor"))]);
    assert!(!p.act(&repo(api), "terminal").is_empty(), "the chip's action is one git does");
    assert_eq!(
        line(&p, &mr(api, "feature/ABC-263")),
        "checkout  terminal  editor  /work/acme/api · java · on main"
    );
    p.local.index.as_mut().unwrap().checkouts[0].branch = "feature/ABC-263".into();
    assert_eq!(
        line(&p, &mr(api, "feature/ABC-263")),
        "checkout  terminal  editor  /work/acme/api · java · on feature/ABC-263"
    );
    // The repo's row: cloned here, on its branch, in place of its own icon.
    let Some(Contribution::Mark { icon, words: w }) = p.contribute(Point::Mark, &repo(api)) else { panic!() };
    let t = radar_core::Theme::new(radar_core::design::theme::Tier::Mono);
    assert_eq!((icon, words(&t, &w).to_string()), (radar_core::ui::icon::Icon::Local, "  feature/ABC-263".into()));
    assert!(p.contribute(Point::Mark, &repo("acme/team-a/web")).is_none(), "no mark where it is not cloned");
    // Elsewhere: why not, and where to set the folders.
    assert_eq!(
        line(&p, &repo("acme/team-a/web")),
        "checkout  no checkout of this repo in /work"
    );
    p.cfg.roots.clear();
    p.local.index = None;
    assert!(
        line(&p, &repo(api)).contains("no local checkout known: add the folders you keep code in"),
        "{}",
        line(&p, &repo(api))
    );
    // Nothing on what it does not know.
    assert!(
        p.contribute(Point::Line, &Subject::new("ticket", "ABC-12", "ABC-12"))
            .is_none()
    );
    assert!(p.contribute(Point::Chips, &repo(api)).is_none());
}

#[test]
fn the_actions_are_this_machines_and_checking_out_is_for_a_merge_request() {
    let mut p = sample();
    let api = "acme/team-a/api";
    assert_eq!(
        actions(&p, &mr(api, "feature/ABC-263")),
        [
            "c: check out feature/ABC-263…",
            "e: open in an editor on feature/ABC-263…",
            "t: open a terminal on feature/ABC-263…",
            "s: look for checkouts again",
            "n: new branch…",
            ": update by rebasing onto its base…",
            ": push main…",
            ": park on a worktree…",
            ": link to another local folder…",
        ]
    );
    assert_eq!(
        actions(&p, &repo(api)),
        [
            "e: open in an editor…",
            "t: open a terminal there",
            "s: look for checkouts again",
            "n: new branch…",
            "u: update by rebasing onto its base…",
            ": push main…",
            ": park on a worktree…",
            ": link to another local folder…",
        ]
    );
    p.local.index.as_mut().unwrap().checkouts[0].branch = "feature/ABC-263".into();
    p.cfg.types[0].editor = "idea".into();
    assert_eq!(
        actions(&p, &mr(api, "feature/ABC-263"))[..2],
        ["c: update checkout feature/ABC-263 from origin…", "e: open in idea"]
    );
    assert_eq!(
        actions(&p, &repo("acme/team-a/web")),
        ["s: scan for a checkout again", ": link to a local folder…"]
    );
    p.cfg.roots.clear();
    assert_eq!(
        actions(&p, &repo("acme/team-a/web")),
        [
            "s: scan for a checkout · no folders set yet",
            ": link to a local folder…"
        ]
    );
}

#[test]
fn checking_out_asks_first_and_a_yes_switches_off_the_event_loop() {
    let mut p = sample();
    let s = mr("acme/team-a/api", "feature/ABC-263");
    let fx = p.act(&s, "checkout");
    let [Effect::Menu(m)] = &fx[..] else {
        panic!("{fx:?}")
    };
    assert_eq!(m.title, "check out feature/ABC-263 in /work/acme/api?");
    assert!(m.question);
    let id = m.id.clone();
    assert!(p.on_menu(&id, "no").is_empty());
    assert!(p.local.switching.is_empty(), "nothing done before yes");
    let fx = p.on_menu(&id, "yes");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))), "{fx:?}");
    assert!(p.local.switching.contains("/work/acme/api"));
    let t = radar_core::Theme::new(radar_core::design::theme::Tier::Mono);
    let Some(Contribution::Line(w)) = p.contribute(Point::Line, &s) else {
        panic!()
    };
    assert!(words(&t, &w).to_string().contains("· checking out…"));
    // Elsewhere it says why not, and asks nothing.
    let fx = p.act(&mr("acme/team-a/web", "x"), "checkout");
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m == "no checkout of this repo in /work"),
        "{fx:?}"
    );
}

/// Every action git adds returns effects, and every row and target of its
/// settings sections is its own and acts (radar_core::check, on the plugin
/// alone).
#[test]
fn every_row_and_target_is_taken() {
    use radar_core::check::{Check, Place};
    let make = || Box::new(sample()) as Box<dyn Plugin>;
    let mut check = Check::new(&make);
    let mut places = Vec::new();
    for id in ["checkouts", "terminal", "editors"] {
        for row in 0..7 {
            places.push(Place::section(id).down(row));
        }
    }
    let gaps = check.places(&places);
    assert!(gaps.is_empty(), "{} gaps:\n{}", gaps.len(), gaps.join("\n"));
    let n = check.actions(&mr("acme/team-a/api", "feature/ABC-263"))
        + check.actions(&repo("acme/team-a/api"))
        + check.actions(&repo("acme/team-a/web"));
    assert_eq!(n, 19);
    assert!(check.gaps.is_empty(), "{:?}", check.gaps);
    assert!(check.checked > 40, "only {} checked", check.checked);
}

#[test]
fn settings_checkouts() {
    let mut host = hosted();
    section(&mut host, "checkouts");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

/// The terminal section with what was found on a desktop machine.
#[test]
fn settings_terminal() {
    let mut host = hosted();
    let mut p = sample();
    let k = |n: &str| *crate::terminal::by_name(n).unwrap();
    p.local.terminals = Some(crate::terminal::Terminals {
        chosen: Some(("ghostty".into(), "from $TERMINAL".into())),
        found: vec![k("ghostty"), k("alacritty")],
        current: Some("ghostty"),
    });
    host.plugins[0] = Box::new(p);
    section(&mut host, "terminal");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

#[test]
fn settings_editors() {
    let mut host = hosted();
    section(&mut host, "editors");
    insta::assert_snapshot!(shown(&host, 160, 40));
}

/// The input opened on a row starts where the row drew its value: what is
/// typed replaces it in place. Read from the view: the row's text and the
/// section's `Editing.at`.
fn input_sits_on_its_value(
    host: &mut Host,
    name: &str,
    downs: usize,
    open: &[KeyCode],
    row: &str,
    value: &str,
) {
    use radar_core::view::Node;
    let press = |host: &mut Host, code: KeyCode, m: KeyModifiers| {
        let _ = shown(host, 160, 40);
        host.update(Event::Key(KeyEvent::new(code, m)))
    };
    section(host, name);
    host.focus = Pane::Middle;
    for _ in 0..20 {
        press(host, KeyCode::Up, KeyModifiers::NONE);
    }
    for _ in 0..downs {
        press(host, KeyCode::Down, KeyModifiers::NONE);
    }
    let before = shown(host, 160, 40);
    let view = radar_core::headless::section_of(host, 160, 40).expect("the section");
    // The row's text as the view holds it: where the value starts in it.
    let Node::List { rows, .. } = &view.body else {
        panic!("{name}: not a list:\n{before}")
    };
    let text = rows
        .iter()
        .map(|r| radar_core::headless::text::text(&r.text))
        .find(|t| t.trim_start().starts_with(row))
        .unwrap_or_else(|| panic!("no row {row:?} in {name}:\n{before}"));
    let start = text.find(row).unwrap();
    let c = match text[start..].find(value) {
        Some(at) => radar_core::design::width::width(&text[..start + at]),
        None => panic!("no {value:?} on {text:?}"),
    };
    for code in open {
        press(host, *code, KeyModifiers::NONE);
    }
    assert!(
        host.plugins[0].capturing(),
        "{name}: {open:?} on {row:?} opened no input"
    );
    press(host, KeyCode::Char('e'), KeyModifiers::CONTROL);
    press(host, KeyCode::Char('u'), KeyModifiers::CONTROL);
    // w is text here, not the save key.
    let fx = press(host, KeyCode::Char('w'), KeyModifiers::NONE);
    assert!(
        !fx.iter()
            .any(|e| matches!(e, radar_core::host::HostEffect::Save(_))),
        "{name}: w saved instead of being typed"
    );
    let _ = shown(host, 160, 40);
    let view = radar_core::headless::section_of(host, 160, 40).expect("the section");
    let Node::List { editing: Some(e), .. } = &view.body else {
        panic!("{name}, {row:?}: the view has no input")
    };
    // A list row's mark takes one cell before its text.
    assert_eq!(
        (e.text.as_str(), usize::from(e.at)),
        ("w", c + 1),
        "{name}, {row:?}: the input does not start where {value:?} was"
    );
    press(host, KeyCode::Esc, KeyModifiers::NONE);
    assert!(!host.plugins[0].capturing());
}

#[test]
fn every_settings_input_starts_where_its_value_was_drawn() {
    use KeyCode::{Char, Enter};
    let mut host = hosted();
    // checkouts: a folder (look down takes digits only: its own test);
    // terminal: a command of your own; editors: a type.
    input_sits_on_its_value(&mut host, "checkouts", 1, &[Char('e')], "folder", "/work");
    input_sits_on_its_value(&mut host, "terminal", 0, &[Char('e')], "terminal   ", "radar");
    input_sits_on_its_value(&mut host, "checkouts", 0, &[Enter], "+ add a folder", "add a folder");
    input_sits_on_its_value(&mut host, "editors", 1, &[Char('n')], "java", "java");
    let markers = {
        let mut h = hosted();
        section(&mut h, "editors");
        let t = shown(&h, 160, 40);
        let l = t.lines().find(|l| l.contains("java ")).unwrap().to_string();
        let rest = &l[l.find("java").unwrap() + "java".len()..];
        rest.trim_start().chars().take(3).collect::<String>()
    };
    input_sits_on_its_value(&mut host, "editors", 1, &[Char('m')], "java", &markers);
    input_sits_on_its_value(&mut host, "editors", 0, &[Enter], "+ add a project type", "add a project type");
}

#[test]
fn the_cache_keeps_the_index_and_the_parts_section_says_what_it_adds() {
    let p = sample();
    let mut back = GitPlugin::default();
    back.restore(p.snapshot().unwrap());
    assert_eq!(back.local.index, p.local.index);
    let adds: Vec<&str> = p.adds().iter().map(|a| a.on).collect();
    assert_eq!(adds, ["repo", "merge-request", "repository", "pull-request"]);
}

// --- the actions: questions, answers, start work -----------------------------------

use crate::act::{self, ActMsg, Item, Op, StartResult, StartStatus};
use crate::ops::Plan;
use radar_core::plugin::Menu;

const API: &str = "acme/team-a/api";

fn mine() -> GitPlugin {
    let mut p = sample();
    p.cfg.me = "alice".into();
    p
}

/// The one menu among the effects.
fn menu_of(fx: Vec<Effect>) -> Menu {
    let mut menus: Vec<Menu> = fx
        .into_iter()
        .filter_map(|e| {
            if let Effect::Menu(m) = e {
                Some(m)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(menus.len(), 1, "one question");
    menus.remove(0)
}

/// `key hint` of each button, and the notes above them.
fn buttons(m: &Menu) -> Vec<String> {
    m.items
        .iter()
        .filter(|i| !i.note && !i.heading)
        .map(|i| format!("{} {}", i.key.unwrap_or(' '), i.hint))
        .collect()
}

fn notes(m: &Menu) -> Vec<&str> {
    m.items
        .iter()
        .filter(|i| i.note)
        .map(|i| i.label.as_str())
        .collect()
}

fn plan() -> Plan {
    Plan {
        base: "main".into(),
        current: "main".into(),
        ..Plan::default()
    }
}

fn planned(
    p: &mut GitPlugin,
    op: Op,
    branch: &str,
    plan: Plan,
    forced: bool,
    ahead: usize,
) -> Vec<Effect> {
    act::on_msg(
        p,
        ActMsg::Planned {
            op,
            dir: "/work/acme/api".into(),
            project: API.into(),
            branch: branch.into(),
            plan,
            forced,
            ahead,
            behind: 0,
            said: None,
        },
    )
}

#[test]
fn keys_find_the_ticket_and_the_stash_is_named_by_it_and_the_day() {
    assert_eq!(
        act::find_key("feature/ABC-263-add-x").as_deref(),
        Some("ABC-263")
    );
    assert_eq!(act::find_key("alice/ABC-12").as_deref(), Some("ABC-12"));
    assert_eq!(act::find_key("fix the login page"), None);
    assert_eq!(act::find_key("lowercase-12"), None);
    assert_eq!(
        act::find_key("Bump v2-1 deps ABC-9: x").as_deref(),
        Some("ABC-9")
    );
    assert_eq!(
        act::date(1_760_000_000 - radar_core::time::local_offset(1_760_000_000)),
        "20251009"
    );
    let name = act::stash_name("alice/ABC-12", radar_core::plugin::DEMO_NOW);
    assert!(name.starts_with("ABC-12-local-20"), "{name}");
}

#[test]
fn n_names_the_branch_by_the_convention_and_asks_off_the_loop() {
    let mut p = mine();
    p.cfg.conventions = vec![crate::branch::Convention {
        under: "acme/team-a".into(),
        branch: "{type}/{key}-{slug}".into(),
        base: "develop".into(),
    }];
    let s = mr(API, "feature/ABC-263")
        .with("title", "Add a stats endpoint")
        .with("type", "Story");
    let fx = p.act(&s, "new");
    assert!(
        fx.iter().any(|e| matches!(e, Effect::Spawn(_))),
        "reads the checkout off the loop"
    );
    assert!(p.act.busy.contains("/work/acme/api"));
    // Twice at once is refused.
    assert!(
        matches!(&p.act(&s, "new")[..], [Effect::Journal(_), Effect::Report(JournalOutcome::Blocked), Effect::Notify(m)] if m.contains("already"))
    );
    // No name set: say so, read nothing.
    let mut nameless = sample();
    let fx = nameless.act(&s, "new");
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m.contains("your name is not set")),
        "{fx:?}"
    );
    assert!(nameless.act.busy.is_empty());
    // A page with no ticket in it says so and reads nothing.
    let fx = p.act(&repo(API), "new");
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m.contains("no ticket key")),
        "{fx:?}"
    );
    assert_eq!(act::base_for(&p, API), "develop");
    let t = crate::branch::Ticket {
        key: "ABC-263",
        title: "Add a stats endpoint",
        kind: "Story",
    };
    assert_eq!(
        act::branch_for(&p, API, &t),
        "story/ABC-263-add-a-stats-endpoint"
    );
}

#[test]
fn a_dirty_checkout_is_offered_a_stash_a_worktree_or_nothing() {
    let mut p = mine();
    let plan = Plan { dirty: 3, ..plan() };
    let m = menu_of(planned(&mut p, Op::New, "alice/ABC-12", plan, false, 0));
    assert!(m.question);
    assert_eq!(m.title, "make alice/ABC-12 in /work/acme/api?");
    assert_eq!(
        notes(&m),
        ["3 files have uncommitted changes here: git will not switch with them"]
    );
    let b = buttons(&m);
    assert!(b[0].starts_with("s stash it as ABC-12-local-"), "{b:?}");
    assert_eq!(
        (b[1].as_str(), b[2].as_str()),
        ("w park on a worktree instead", "n no, leave it")
    );
    // n changes nothing and starts nothing.
    assert!(matches!(
        &p.on_menu(&m.id, "no")[..],
        [Effect::Report(JournalOutcome::Cancelled)]
    ));
    // s stashes off the loop.
    let fx = p.on_menu(&m.id, "stash");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))), "{fx:?}");
}

#[test]
fn an_existing_branch_defaults_to_switching_and_recreating_says_what_is_lost() {
    let mut p = mine();
    let plan = Plan {
        exists_local: true,
        unpushed: 2,
        current: "main".into(),
        ..plan()
    };
    let m = menu_of(planned(
        &mut p,
        Op::New,
        "alice/ABC-12",
        plan.clone(),
        false,
        0,
    ));
    assert_eq!(m.title, "alice/ABC-12 exists in /work/acme/api?");
    assert_eq!(
        notes(&m),
        [
            "alice/ABC-12 is here already; it is on main",
            "2 commits on alice/ABC-12 are on no remote: recreating it loses them"
        ]
    );
    assert_eq!(
        buttons(&m),
        [
            "s switch to it",
            "r recreate it off fresh main",
            "n no, leave it"
        ]
    );
    let one = Plan {
        exists_local: true,
        unpushed: 0,
        ..plan.clone()
    };
    let m = menu_of(planned(&mut p, Op::New, "alice/ABC-12", one, false, 0));
    assert_eq!(notes(&m).len(), 1, "nothing is lost, so nothing is said");
    assert!(matches!(
        &p.on_menu(&m.id, "no")[..],
        [Effect::Report(JournalOutcome::Cancelled)]
    ));
    let fx = p.on_menu(&m.id, "switch");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))));
}

#[test]
fn a_new_branch_or_one_only_on_origin_is_a_plain_question() {
    let mut p = mine();
    let m = menu_of(planned(&mut p, Op::New, "alice/ABC-12", plan(), false, 0));
    assert_eq!(m.title, "make alice/ABC-12 in /work/acme/api?");
    assert_eq!(
        notes(&m),
        ["fetches main and starts alice/ABC-12 from origin/main"]
    );
    let remote = Plan {
        exists_remote: true,
        ..plan()
    };
    let m = menu_of(planned(&mut p, Op::New, "alice/ABC-12", remote, false, 0));
    assert_eq!(m.title, "check out alice/ABC-12 in /work/acme/api?");
    // A name git refuses is said, not asked.
    let fx = planned(&mut p, Op::New, "alice/ABC 12", plan(), false, 0);
    assert!(
        matches!(&fx[..], [Effect::Report(JournalOutcome::Blocked), Effect::Notice(m)] if m.tone == radar_core::view::chrome::NoticeTone::Error && m.contains("not a branch name git takes")),
        "{fx:?}"
    );
}

#[test]
fn rebase_asks_and_refuses_what_it_cannot_do() {
    let mut p = mine();
    let on = Plan {
        current: "alice/ABC-12".into(),
        ..plan()
    };
    let m = menu_of(planned(
        &mut p,
        Op::Rebase,
        "alice/ABC-12",
        on.clone(),
        false,
        0,
    ));
    assert_eq!(m.title, "rebase alice/ABC-12 on origin/main?");
    assert_eq!(buttons(&m), ["y yes, rebase it", "n no, leave it"]);
    let said = notes(&m).join(" ");
    assert!(
        said.contains("a conflict stops it") && said.contains("git rebase --abort"),
        "{said}"
    );
    for (plan, why) in [
        (
            Plan {
                dirty: 1,
                ..on.clone()
            },
            "1 file has uncommitted changes",
        ),
        (plan(), "main is the base"),
        (
            Plan {
                current: String::new(),
                ..plan()
            },
            "no branch is checked out",
        ),
    ] {
        let fx = planned(&mut p, Op::Rebase, &plan.current.clone(), plan, false, 0);
        assert!(
            matches!(&fx[..], [Effect::Report(JournalOutcome::Blocked), Effect::Notice(m)] if m.tone == radar_core::view::chrome::NoticeTone::Error && m.contains(why)),
            "{why}: {fx:?}"
        );
    }
}

#[test]
fn a_push_is_a_write_that_adds_when_new_and_edits_when_forced() {
    let mut p = mine();
    let on = Plan {
        current: "alice/ABC-12".into(),
        ..plan()
    };
    let new = menu_of(planned(
        &mut p,
        Op::Push,
        "alice/ABC-12",
        on.clone(),
        false,
        2,
    ));
    assert_eq!(new.change, Some(radar_core::writes::Change::Add));
    assert_eq!(new.title, "push alice/ABC-12 to origin?");
    assert_eq!(
        notes(&new),
        [
            "origin does not have alice/ABC-12: it is created there",
            "2 commits to send"
        ]
    );
    let there = Plan {
        exists_remote: true,
        ..on
    };
    let forced = menu_of(planned(
        &mut p,
        Op::Push,
        "alice/ABC-12",
        there.clone(),
        true,
        4,
    ));
    assert_eq!(forced.change, Some(radar_core::writes::Change::Edit));
    assert!(
        notes(&forced)[0].contains("this rewrites it"),
        "{:?}",
        notes(&forced)
    );
    assert!(notes(&forced)[1].contains("--force-with-lease"));
    let plain = menu_of(planned(&mut p, Op::Push, "alice/ABC-12", there, false, 1));
    assert_eq!(plain.change, Some(radar_core::writes::Change::Edit));
    assert_eq!(notes(&plain), ["1 commit to send"]);
    assert!(matches!(
        &p.on_menu(&plain.id, "no")[..],
        [Effect::Report(JournalOutcome::Cancelled)]
    ));
}

#[test]
fn a_forced_push_counts_the_commits_origin_loses() {
    let mut p = mine();
    let there = Plan {
        current: "main".into(),
        exists_remote: true,
        ..plan()
    };
    let m = menu_of(act::on_msg(
        &mut p,
        ActMsg::Planned {
            op: Op::Push,
            dir: "/work/acme/api".into(),
            project: API.into(),
            branch: "main".into(),
            plan: there,
            forced: true,
            ahead: 0,
            behind: 17,
            said: None,
        },
    ));
    assert_eq!(
        notes(&m)[0],
        "17 commits on origin/main are not here: pushing rewrites origin and drops them"
    );
}

#[test]
fn link_to_a_local_folder_offers_this_repos_clones_then_where_radar_looks() {
    let p = mine();
    let m = menu_of(act::ask_link(&p, API));
    let offered: Vec<String> = p.typing(&m.id, "").offers.into_iter().map(|o| o.value).collect();
    // Then the usual folders under home (which this machine has varies).
    assert_eq!(offered[..2], ["/work/acme/api", "/work/"]);
    assert_eq!(m.typed.as_deref(), Some("/work/acme/api"), "⏎ links the clone found");
}

/// A folder under the system's temp folder, removed when dropped.
struct Scratch(std::path::PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("radar-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A clone as radar reads one (its `.git/config` origin and `HEAD`, on main),
/// without running git.
fn repo_at(s: &Scratch, rel: &str, origin: &str) -> std::path::PathBuf {
    let dir = s.0.join(rel);
    std::fs::create_dir_all(dir.join(".git")).unwrap();
    std::fs::write(dir.join(".git/config"), format!("[remote \"origin\"]\n\turl = {origin}\n")).unwrap();
    std::fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    dir
}

#[test]
fn linking_a_repo_with_no_clone_known_looks_for_it_first_then_offers_what_it_found() {
    let s = Scratch::new("look");
    // Found by its origin, and a folder with its name but another origin.
    let api = repo_at(&s, "code/team-a/api", "git@example.com:acme/team-a/web.git");
    repo_at(&s, "other/web", "git@example.com:someone/fork.git");
    let places = [s.0.display().to_string()];
    let found = crate::local::look_for("acme/team-a/web", &places, 4);
    let dirs: Vec<&str> = found.iter().map(|c| c.dir.as_str()).collect();
    assert_eq!(dirs, [api.display().to_string().as_str(), s.0.join("other/web").display().to_string().as_str()]);
    // Through the plugin: a look off the event loop, then the field.
    let mut p = mine();
    let fx = p.act(&repo("acme/team-a/web"), "link");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))), "{fx:?}");
    assert!(fx.iter().any(|e| matches!(e, Effect::Notify(m) if m == "looking for web on this machine…")));
    let fx = p.on_msg(Box::new(crate::local::LocalMsg::LookedFor { project: "acme/team-a/web".into(), found }));
    let m = menu_of(fx);
    assert_eq!(m.typed.as_deref(), Some(api.display().to_string().as_str()), "starts on the clone found");
    let t = p.typing(&m.id, &api.display().to_string());
    assert!(matches!(t.verdict, Some(radar_core::Verdict::Good(ref w)) if w == "a clone of it, on main · ⏎ links it"), "{t:?}");
    assert_eq!(t.offers[0].hint, "a clone of it, on main");
    // Found nothing: says so, and the field starts empty.
    let mut p = mine();
    let fx = p.on_msg(Box::new(crate::local::LocalMsg::LookedFor { project: "acme/team-a/web".into(), found: vec![] }));
    assert!(fx.iter().any(|e| matches!(e, Effect::Notify(m) if m.starts_with("no clone of it found"))));
    assert_eq!(menu_of(fx).typed.as_deref(), Some(""));
}

#[test]
fn parking_asks_where_and_refuses_a_folder_that_is_there() {
    let mut p = mine();
    let m = menu_of(planned(&mut p, Op::Park, "alice/ABC-12", plan(), false, 0));
    assert_eq!(m.title, "park alice/ABC-12 on a worktree?");
    assert!(
        notes(&m)[0] == "at /work/acme/api-ABC-12" && notes(&m)[1] == "beside the clone",
        "{:?}",
        notes(&m)
    );
    assert_eq!(
        act::worktree_path(&p, "/work/acme/api", "feature/ABC-12-x"),
        "/work/acme/api-ABC-12-x"
    );
    assert!(matches!(
        &p.on_menu(&m.id, "no")[..],
        [Effect::Report(JournalOutcome::Cancelled)]
    ));
    let fx = p.on_menu(&m.id, "yes");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))));
}

#[test]
fn repository_worktree_locations_override_the_global_template_and_round_trip() {
    let mut p = crate::GitPlugin::default();
    p.cfg.worktree = "/work/global/{topic}".into();
    p.cfg.links.push(crate::local::Link {
        repo: "ankh-morpork/watch".into(), dir: "/work/watch".into(),
        worktree: "/work/trees/watch/{topic}".into(), ..Default::default()
    });
    let mut fresh = crate::GitPlugin::default();
    fresh.configure(p.config().as_ref()).unwrap();
    assert_eq!(act::worktree_path(&fresh, "/work/watch", "vimes/WATCH-12"), "/work/trees/watch/WATCH-12");
    assert_eq!(act::worktree_path(&fresh, "/work/watch", "vimes/WATCH-12-abcdef1"), "/work/trees/watch/WATCH-12-abcdef1");
    assert_eq!(act::worktree_path(&fresh, "/work/library", "vimes/WATCH-12"), "/work/global/WATCH-12");
    fresh.cfg.links[0].worktree.clear();
    assert_eq!(act::worktree_path(&fresh, "/work/watch", "vimes/WATCH-12"), "/work/global/WATCH-12");
}

/// A terminal or editor on a merge request opens on its branch: the clone is
/// on another, so radar asks where, and each answer does what it says.
#[test]
fn a_terminal_for_a_merge_request_opens_on_its_branch_asking_where() {
    let mut p = mine();
    let s = mr(API, "feature/ABC-263");
    let m = menu_of(p.act(&s, "terminal"));
    assert_eq!(m.title, "open feature/ABC-263: where?");
    assert!(m.question);
    assert_eq!(notes(&m), ["/work/acme/api is on main", "a worktree would be at /work/acme/api-ABC-263"]);
    assert_eq!(
        buttons(&m),
        [
            "w in a worktree beside it",
            "c check it out in api",
            "o on main as it is",
        ]
    );
    // Here: the checkout is switched, the terminal waits for it.
    let fx = p.on_menu(&m.id, "here");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))));
    assert_eq!(p.local.then_open.get("/work/acme/api").map(String::as_str), Some("terminal"));
    let fx = p.on_msg(Box::new(crate::local::LocalMsg::SwitchedBranch {
        dir: "/work/acme/api".into(),
        branch: "feature/ABC-263".into(),
        result: Err("1 uncommitted change in /work/acme/api: commit or stash first; nothing was changed".into()),
    }));
    assert!(fx.iter().any(|e| matches!(e, Effect::Notice(m) if m.tone == radar_core::view::chrome::NoticeTone::Error && m.contains("w in its question opens it in a worktree"))), "{fx:?}");
    assert!(p.local.then_open.is_empty(), "nothing opens after a refusal");
    // A worktree: made off the loop, the terminal waits for it.
    let fx = p.on_menu(&m.id, "worktree");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))));
    assert_eq!(p.local.then_open.get("/work/acme/api-ABC-263").map(String::as_str), Some("terminal"));
    let fx = p.on_msg(Box::new(crate::local::LocalMsg::Worktree {
        dir: "/work/acme/api-ABC-263".into(),
        result: Err("/work/acme/api-ABC-263 already exists".into()),
        parked: None,
    }));
    assert!(fx.iter().any(|e| matches!(e, Effect::Notice(m) if m.tone == radar_core::view::chrome::NoticeTone::Error && m.contains("no worktree made"))));
    assert!(matches!(fx[0], Effect::Report(JournalOutcome::Unknown)));
    assert!(p.local.then_open.is_empty(), "failed worktrees leave no pending launch");
    // As it is: opened on the clone's own branch now.
    assert!(p.on_menu(&m.id, "as").is_empty(), "the clone is not on disk in the sample, so nothing opens");
    // A checkout on the branch already: no question.
    p.local.index.as_mut().unwrap().checkouts[0].branch = "feature/ABC-263".into();
    let fx = p.act(&s, "terminal");
    assert!(fx.iter().all(|e| !matches!(e, Effect::Menu(_))) && !fx.is_empty(), "{fx:?}");
}

#[test]
fn worktree_completion_keeps_step_results_separate_from_the_pending_launch() {
    let scratch = Scratch::new("worktree-launch-journal");
    let dir = repo_at(&scratch, "api-ABC-263", "git@example.com:acme/api.git");
    let branch = "feature/ABC-263";
    std::fs::write(dir.join(".git/HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
    let dir = dir.display().to_string();
    for what in ["terminal", "editor"] {
        let mut p = GitPlugin::new();
        p.cfg.roots = vec![scratch.0.display().to_string()];
        p.local.then_open.insert(dir.clone(), what.into());
        let fx = p.on_msg(Box::new(crate::local::LocalMsg::Worktree {
            dir: dir.clone(),
            result: Ok("created tree; after command failed".into()),
            parked: Some(act::WorktreeResult {
                clone: scratch.0.join("api").display().to_string(),
                path: dir.clone(),
                branch: branch.into(),
                creation: act::WorktreeStatus::Succeeded,
                after: Some(act::WorktreeStatus::Unknown),
            }),
        }));
        assert!(p.local.then_open.is_empty());
        assert!(matches!(fx[0], Effect::Report(JournalOutcome::Partial)));
        let launch = fx.iter().position(|e| matches!(e,
            Effect::Journal(a) if a.kind.as_str() == format!("local.{what}.launch")
        )).expect("the requested launch follows the worktree");
        assert!(matches!(&fx[launch..], [Effect::Journal(_), Effect::Spawn(_)]));
        for (kind, outcome) in [
            ("git.worktree.add", JournalOutcome::Succeeded),
            ("git.worktree.after", JournalOutcome::Unknown),
        ] {
            let step = fx.iter().position(|e| matches!(e,
                Effect::Journal(a) if a.kind.as_str() == kind
            )).expect("typed worktree step");
            assert!(step < launch);
            assert!(matches!(fx[step + 1], Effect::Report(o) if o == outcome));
        }
        let scan = fx.iter().position(|e| matches!(e,
            Effect::Journal(a) if a.kind.as_str() == "git.checkouts.scan"
        )).expect("the new worktree is rescanned");
        assert!(scan < launch && matches!(fx[scan + 1], Effect::Spawn(_)));
        let Effect::Journal(action) = &fx[launch] else { unreachable!() };
        assert!(action.subjects.iter().any(|s| s.key.kind == "branch" && s.key.id == branch));
        assert!(action.subjects.iter().any(|s| s.key.kind == "repo" && s.key.id == "acme/api"));
        assert!(p.local.index.is_none(), "identity does not depend on the pending scan");
    }
}

#[test]
fn checkout_completion_reports_success_then_opens_the_requested_branch() {
    let scratch = Scratch::new("checkout-launch-journal");
    let dir = repo_at(&scratch, "api", "git@example.com:acme/api.git");
    let branch = "feature/ABC-263";
    std::fs::write(dir.join(".git/HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
    let dir = dir.display().to_string();
    let mut p = GitPlugin::new();
    p.local.then_open.insert(dir.clone(), "terminal".into());
    p.local.switching.insert(dir.clone());
    let fx = p.on_msg(Box::new(crate::local::LocalMsg::SwitchedBranch {
        dir: dir.clone(),
        branch: branch.into(),
        result: Ok("checked out".into()),
    }));
    assert!(matches!(fx[0], Effect::Report(JournalOutcome::Succeeded)));
    assert!(p.local.then_open.is_empty() && p.local.switching.is_empty());
    let Effect::Journal(action) = &fx[fx.len() - 2] else { panic!("launch descriptor") };
    assert_eq!(action.kind.as_str(), "local.terminal.launch");
    assert!(action.subjects.iter().any(|s| s.key.kind == "branch" && s.key.id == branch));
    assert!(matches!(fx.last(), Some(Effect::Spawn(_))));
}

#[test]
fn link_and_use_change_what_is_acted_on() {
    let mut p = mine();
    // A look found nothing for web (as if it ran): the field opens empty.
    p.local.looked.insert("acme/team-a/web".into(), vec![]);
    let m = menu_of(p.act(&repo("acme/team-a/web"), "link"));
    assert_eq!(m.title, "link acme/team-a/web to a local folder");
    assert_eq!(m.typed.as_deref(), Some(""), "a field to type the folder in");
    // Nothing typed: no web clone was found, so the folders radar looks in
    // and home are offered; nothing is said yet.
    let t = p.typing(&m.id, "");
    let offered: Vec<&str> = t.offers.iter().map(|o| o.value.as_str()).collect();
    assert_eq!(offered[0], "/work/", "the folders radar looks in, then the usual ones");
    assert!(t.verdict.is_none());
    // Typed: what it is, as you type; a folder that is not here is refused.
    let t = p.typing(&m.id, "/radar/no/such/folder");
    assert!(matches!(t.verdict, Some(radar_core::Verdict::Bad(ref w)) if w.contains("not a folder")), "{t:?}");
    let tmp = std::env::temp_dir().display().to_string();
    let t = p.typing(&m.id, &format!("{tmp}/"));
    assert!(matches!(t.verdict, Some(radar_core::Verdict::Bad(ref w)) if w.contains("not a git clone")), "{t:?}");
    // On a repo with a clone found, that clone is offered first, said as one.
    let m_api = menu_of(p.act(&repo(API), "link"));
    assert_eq!(m_api.typed.as_deref(), Some("/work/acme/api"), "starts on the clone in use");
    assert_eq!(p.typing(&m_api.id, "").offers[0].value, "/work/acme/api");
    // What ⏎ gives is linked, a trailing / dropped.
    p.on_menu(&m.id, "/work/acme/api/");
    assert_eq!(p.cfg.links.len(), 1);
    assert_eq!(
        (p.cfg.links[0].repo.as_str(), p.cfg.links[0].dir.as_str()),
        ("acme/team-a/web", "/work/acme/api")
    );
    // Linking again replaces, never doubles.
    p.on_menu(&m.id, "/work/acme/api");
    assert_eq!(p.cfg.links.len(), 1);
    // One checkout: nothing to pick. Several: the choice is kept.
    let fx = p.act(&repo(API), "use");
    assert!(
        matches!(&fx[..], [Effect::Notify(m)] if m.contains("only one")),
        "{fx:?}"
    );
    p.local
        .index
        .as_mut()
        .unwrap()
        .checkouts
        .push(crate::local::Checkout {
            dir: "/work/acme/api-ABC-12".into(),
            project: API.into(),
            branch: "alice/ABC-12".into(),
            main: "/work/acme/api".into(),
            ..Default::default()
        });
    assert!(actions(&p, &repo(API)).contains(&": use another worktree…".to_string()));
    let m = menu_of(p.act(&repo(API), "use"));
    assert_eq!(m.items.len(), 2);
    assert!(m.items[0].hint.contains("in use"));
    let fx = p.on_menu(&m.id, "/work/acme/api-ABC-12");
    assert!(fx.iter().any(|e| matches!(e, Effect::PersistCache)));
    assert_eq!(
        crate::local::chosen(&p, API).unwrap().dir,
        "/work/acme/api-ABC-12"
    );
    assert!(
        line(&p, &repo(API)).contains("api-ABC-12"),
        "{}",
        line(&p, &repo(API))
    );
}

fn start_payload() -> serde_json::Value {
    serde_json::json!({
        "ticket": {"key": "ABC-12", "title": "Add a stats endpoint", "type": "Story"},
        "workspace": "/work",
        "repos": [
            {"dir": "/work/acme/api", "repo": API, "how": "checkout"},
            {"dir": "/work/acme/web", "repo": "acme/team-a/web", "how": "worktree"},
        ]
    })
}

fn item(dir: &str, repo: &str, branch: &str) -> Item {
    Item {
        dir: dir.into(),
        repo: repo.into(),
        branch: branch.into(),
        base: "main".into(),
        worktree: String::new(),
        stash: String::new(),
        exists: false,
        refused: String::new(),
    }
}

#[test]
fn workspace_start_reads_every_repo_then_asks_one_question_and_run_spawns() {
    let mut p = mine();
    let fx = p.on_bus("workspace.start", &start_payload());
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))), "{fx:?}");
    assert!(p.on_bus("something.else", &start_payload()).is_empty());
    let items = vec![
        Item {
            stash: "ABC-12-local-20261009".into(),
            ..item("/work/acme/api", API, "alice/ABC-12")
        },
        Item {
            worktree: "/work/acme/web-ABC-12".into(),
            ..item("/work/acme/web", "acme/team-a/web", "alice/ABC-12")
        },
    ];
    let m = menu_of(act::on_msg(
        &mut p,
        ActMsg::StartPlanned {
            key: "ABC-12".into(),
            items,
        },
    ));
    assert_eq!(m.title, "start ABC-12 in 2 repos?");
    assert_eq!(
        notes(&m),
        [
            "acme/team-a/api: alice/ABC-12 from origin/main, stash ABC-12-local-20261009",
            "acme/team-a/web: alice/ABC-12 from origin/main, worktree at /work/acme/web-ABC-12",
        ]
    );
    assert_eq!(buttons(&m), ["y yes, run it", "n no, leave it"]);
    // n: nothing runs.
    let mut q = mine();
    q.act.start = p.act.start.clone();
    let fx = q.on_menu(&m.id, "no");
    assert!(
        matches!(&fx[..], [Effect::Report(JournalOutcome::Cancelled), Effect::Notify(m)] if m.contains("nothing done")),
        "{fx:?}"
    );
    assert!(q.act.start.is_none());
    // y runs them off the loop.
    let fx = p.on_menu(&m.id, "yes");
    assert!(fx.iter().any(|e| matches!(e, Effect::Spawn(_))), "{fx:?}");
}

#[test]
fn a_repo_git_refuses_stops_the_start_and_the_rest_are_not_run() {
    let mut bad = item("/work/acme/api", API, "alice/ABC 12");
    bad.refused = "\"alice/ABC 12\" is not a branch name git takes: it has a space".into();
    let line = act::item_line(&bad);
    assert!(line.ends_with("(refused)"), "{line}");
    let results = act::run_items(
        &[
            bad,
            item("/work/acme/web", "acme/team-a/web", "alice/ABC-12"),
        ],
        "",
    );
    assert_eq!(results.len(), 2);
    assert!(!results[0].ok && results[0].why.contains("not a branch name"));
    assert!(!results[1].ok);
    assert_eq!(results[1].why, "not run: stopped at acme/team-a/api");
}

fn start_menu(p: &mut GitPlugin, items: Vec<Item>) -> Menu {
    menu_of(act::on_msg(
        p,
        ActMsg::StartPlanned {
            key: "ABC-12".into(),
            items,
        },
    ))
}

#[test]
fn the_start_question_names_the_command_a_parked_worktree_runs() {
    let parked = Item {
        worktree: "/work/acme/web-ABC-12".into(),
        ..item("/work/acme/web", "acme/team-a/web", "alice/ABC-12")
    };
    let mut p = mine();
    p.cfg.after_worktree = "just setup".into();
    let m = start_menu(&mut p, vec![parked.clone()]);
    assert!(
        notes(&m).contains(&"then runs just setup in each new worktree"),
        "{:?}",
        notes(&m)
    );
    // Nothing parked, or no command set: nothing to say.
    let m = start_menu(&mut p, vec![item("/work/acme/api", API, "alice/ABC-12")]);
    assert!(!notes(&m).iter().any(|n| n.contains("then runs")));
    p.cfg.after_worktree.clear();
    let m = start_menu(&mut p, vec![parked]);
    assert!(!notes(&m).iter().any(|n| n.contains("then runs")));
}

#[test]
fn the_start_question_says_a_refused_repo_stops_the_ones_after_it() {
    let mut bad = item("/work/acme/api", API, "alice/ABC 12");
    bad.refused = "it has a space".into();
    let mut p = mine();
    let m = start_menu(
        &mut p,
        vec![
            item("/work/acme/web", "acme/team-a/web", "alice/ABC-12"),
            bad.clone(),
            item("/work/acme/docs", "acme/team-a/docs", "alice/ABC-12"),
            item("/work/acme/ops", "acme/team-a/ops", "alice/ABC-12"),
        ],
    );
    assert!(
        notes(&m)
            .contains(&"the run stops at the first refused repo: the 2 repos after it are not run"),
        "{:?}",
        notes(&m)
    );
    let m = start_menu(&mut p, vec![bad]);
    assert!(notes(&m).contains(&"a refused repo stops the run"));
    let m = start_menu(&mut p, vec![item("/work/acme/api", API, "alice/ABC-12")]);
    assert!(!notes(&m).iter().any(|n| n.contains("refused")));
}

#[test]
fn the_results_become_git_started_with_exactly_these_fields() {
    let results = vec![
        StartResult {
            dir: "/work/acme/api".into(),
            repo: API.into(),
            branch: "alice/ABC-12".into(),
            base: "main".into(),
            worktree: String::new(),
            stash: "ABC-12-local-20261009".into(),
            ok: true,
            why: String::new(),
            status: StartStatus::Succeeded,
            parked: None,
        },
        StartResult {
            dir: "/work/acme/web".into(),
            repo: "acme/team-a/web".into(),
            branch: "alice/ABC-12".into(),
            base: "main".into(),
            worktree: "/work/acme/web-ABC-12".into(),
            stash: String::new(),
            ok: false,
            why: "already exists".into(),
            status: StartStatus::Unknown,
            parked: None,
        },
    ];
    let mut p = mine();
    let fx = act::on_msg(
        &mut p,
        ActMsg::Finished {
            key: "ABC-12".into(),
            results,
        },
    );
    let emitted: Vec<_> = fx
        .iter()
        .filter_map(|e| {
            if let Effect::Emit { topic, payload } = e {
                Some((*topic, payload.clone()))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        emitted,
        [(
            "git.started",
            serde_json::json!({
                "ticket": "ABC-12",
                "results": [
                    {"dir": "/work/acme/api", "repo": API, "branch": "alice/ABC-12", "base": "main",
                     "worktree": "", "stash": "ABC-12-local-20261009", "ok": true, "why": ""},
                    {"dir": "/work/acme/web", "repo": "acme/team-a/web", "branch": "alice/ABC-12", "base": "main",
                     "worktree": "/work/acme/web-ABC-12", "stash": "", "ok": false, "why": "already exists"},
                ]
            })
        )]
    );
    assert!(fx.iter().any(|e| matches!(e, Effect::Notice(m) if m.tone == radar_core::view::chrome::NoticeTone::Error && m.contains("1 of 2 done") && m.contains("already exists"))));
}

fn sh(dir: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=alice",
            "-c",
            "user.email=alice@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Three throwaway clones with their own origins: a start runs in order, one
/// dirty clone is stashed, one is parked on a worktree, and the results are
/// the `git.started` payload.
#[test]
fn a_start_over_three_clones_runs_each_and_reports_git_started() {
    let root = std::env::temp_dir().join(format!("radar-git-start-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut items = Vec::new();
    for n in 0..3 {
        let origin = root.join(format!("o{n}.git"));
        let clone = root.join(format!("c{n}"));
        std::fs::create_dir_all(&origin).unwrap();
        sh(&origin, &["init", "--bare", "-b", "main"]);
        std::fs::create_dir_all(&clone).unwrap();
        sh(&clone, &["init", "-b", "main"]);
        sh(
            &clone,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        std::fs::write(clone.join("a.txt"), "one").unwrap();
        sh(&clone, &["add", "a.txt"]);
        sh(&clone, &["commit", "-m", "first"]);
        sh(&clone, &["push", "-u", "origin", "main"]);
        let dir = clone.display().to_string();
        let mut it = item(&dir, &format!("acme/team-a/r{n}"), "alice/ABC-12");
        match n {
            1 => {
                std::fs::write(clone.join("a.txt"), "two").unwrap();
                it.stash = "ABC-12-local-20261009".into();
            }
            2 => it.worktree = root.join("c2-ABC-12").display().to_string(),
            _ => {}
        }
        items.push(it);
    }
    let results = act::run_items(&items, "");
    let _ = std::fs::remove_dir_all(root.join("c2-ABC-12/.git"));
    assert!(results.iter().all(|r| r.ok), "{results:?}");
    assert_eq!(results[1].stash, "ABC-12-local-20261009");
    assert_eq!(results[2].worktree, items[2].worktree);
    let head = |d: &str| {
        let o = std::process::Command::new("git")
            .args(["-C", d, "branch", "--show-current"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&o.stdout).trim().to_string()
    };
    assert_eq!(head(&items[0].dir), "alice/ABC-12");
    assert_eq!(head(&items[1].dir), "alice/ABC-12");
    assert_eq!(
        head(&items[2].dir),
        "main",
        "the parked clone stays where it was"
    );
    let payload = act::started_payload("ABC-12", &results);
    assert_eq!(payload["results"].as_array().unwrap().len(), 3);
    assert_eq!(payload["results"][1]["stash"], "ABC-12-local-20261009");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn journal_checkout_identity_keeps_its_local_scope() {
    let p = sample();
    let subject = Subject::new("checkout", "/work/acme/api", "private display text");
    let snapshot = p.journal_subject(&subject).unwrap();
    assert_eq!(snapshot.key.provider, "local");
    assert_eq!(snapshot.key.instance, "/work/acme/api");
    assert_eq!(snapshot.key.id, "/work/acme/api");
    let home = p
        .journal_subject(&Subject::new("checkout", "~/work", "x"))
        .unwrap();
    assert_eq!(
        home.key.instance,
        crate::local::journal_path("~/work")
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(home.key.instance, home.key.id);

    assert!(!format!("{snapshot:?}").contains("private display text"));
    assert!(
        p.journal_subject(&Subject::new("ticket", "ABC-12", "x"))
            .is_none()
    );
}
