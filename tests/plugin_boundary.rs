//! The renderer-neutral plugin boundary (M11 A0), end to end: the Git
//! Branches screen, with a ticket column the Jira part contributes, driven
//! once by terminal keys and once by intents (what a renderer sends for a
//! click or a chosen action), both read as view data. Both reach the same
//! part state, the same question, and the same journal actions.

#![cfg(all(feature = "git", feature = "jira", feature = "demo"))]

use radar_core::headless::Headless;
use radar_core::host::{Event, HostEffect, JournalRecord, View};
use radar_core::journal::ActionId;
use radar_core::view::{Chord, Key};
use radar_core::view::{Intent, Node, PaneId, Target, TreeRow};
use radar_core::{Host, Plugin};

const W: u16 = 220;
const H: u16 = 50;

/// Jira and git with their sample data, on the Branches screen.
fn hosted() -> Headless {
    let parts = || -> Vec<Box<dyn Plugin>> {
        let mut jira = radar_jira::JiraPlugin::new();
        jira.demo();
        let mut git = radar_git::GitPlugin::new();
        git.demo();
        vec![Box::new(jira), Box::new(git)]
    };
    let mut host = Host::new(parts(), radar_core::plugin::DEMO_NOW).unwrap();
    host.load_config(Some("")).unwrap();
    // Loading the (empty) config re-configured the parts: put the samples back.
    host.plugins = parts();
    let at = host.screens.iter().position(|(_, s)| s.id == "branch-screen").unwrap();
    host.view = View::Screen(at);
    Headless::new(host, W, H)
}

fn branch(name: &str) -> Target {
    TreeRow::target_of(&["c:/work/acme/api".into(), format!("b:{name}")])
}

#[test]
fn git_and_jira_have_no_renderer_specific_surface_gaps() {
    let gaps = hosted().view_gaps();
    assert!(gaps.is_empty(), "{gaps:#?}");
}

#[test]
fn every_compiled_in_part_has_neutral_screen_and_settings_views() {
    let mut host = Host::new(radar::parts(), radar_core::plugin::DEMO_NOW).unwrap();
    host.load_config(Some("")).unwrap();
    if let Err(gaps) = Headless::requiring_views(host, 80, 24) {
        panic!("registered surfaces without neutral views: {gaps:#?}");
    }
}

/// The journal's records with their ids numbered in order of appearance:
/// what was recorded, under what, with what outcome.
fn shape(records: &[JournalRecord]) -> Vec<String> {
    let mut ids: Vec<ActionId> = Vec::new();
    let mut n = |id: &ActionId| match ids.iter().position(|i| i == id) {
        Some(i) => i,
        None => {
            ids.push(id.clone());
            ids.len() - 1
        }
    };
    records
        .iter()
        .map(|r| match r {
            JournalRecord::Begin { id, descriptor, parent } => {
                let parent = parent.as_ref().map(&mut n);
                format!("begin #{} under {parent:?}: {descriptor:?}", n(id))
            }
            JournalRecord::Outcome { id, outcome, .. } => format!("#{} {outcome:?}", n(id)),
            JournalRecord::Gap { dropped } => format!("gap {dropped}"),
        })
        .collect()
}

fn journal(fx: &[HostEffect]) -> Vec<JournalRecord> {
    fx.iter()
        .filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

fn key(c: char) -> Event {
    Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(Key::Char(c)) })
}

#[test]
fn the_same_part_state_question_and_journal_through_either_frontend() {
    let (mut tui, mut headless) = (hosted(), hosted());

    // The headless frontend sees the screen as data, with the other part's
    // column on the branch whose name holds a ticket key.
    let view = headless.screen().expect("the Branches screen is a view");
    let Some(Node::Tree(tree)) = view.pane(PaneId::Middle).map(|p| &p.body) else {
        panic!("a tree")
    };
    assert!(tree.columns.iter().any(|c| c.heading == "ticket"), "{:?}", tree.columns);
    let shown = tree.visible();
    let (_, row) = shown
        .iter()
        .find(|(path, _)| TreeRow::target_of(path) == branch("alice/ABC-12"))
        .expect("the branch row");
    let cell = row.cells.iter().find(|c| c.heading == "ticket").expect("Jira's cell");
    assert_eq!(cell.text.words(), "ABC-12 · In Progress");
    assert_eq!(cell.show.as_ref().map(|s| s.id.as_str()), Some("ABC-12"));

    // The text form a test reads shows the same cell on that branch's row.
    let text = tui.text();
    assert!(
        text.lines().any(|l| l.contains("alice/ABC-12") && l.contains("ABC-12 · In Progress")),
        "{text}"
    );

    // Select the same branch: by keys on the terminal (↓ until its row is
    // selected), and by an intent on its target headless.
    let target = branch("spike");
    let selected_of = |h: &Headless| match h.screen().unwrap().pane(PaneId::Middle).map(|p| p.body.clone()) {
        Some(Node::Tree(t)) => TreeRow::target_of(&t.selected),
        _ => Target::new(""),
    };
    tui.focus(PaneId::Middle);
    for _ in 0..20 {
        if selected_of(&tui) == target {
            break;
        }
        tui.host_mut().update(Event::Intent(radar_core::view::Intent::Key { chord: Chord::new(Key::Down) }));
    }
    assert_eq!(selected_of(&tui), target, "↓ reaches the branch:\n{}", tui.text());
    headless.focus(PaneId::Middle);
    headless.send(Intent::Select { target: target.clone() });
    assert_eq!(tui.screen(), headless.screen(), "the same part state");
    let selected = match headless.screen().unwrap().pane(PaneId::Middle).map(|p| p.body.clone()) {
        Some(Node::Tree(t)) => t.selected,
        _ => vec![],
    };
    assert_eq!(TreeRow::target_of(&selected), target);

    // Delete it: `d` then `y` on the terminal; the action and the answer as
    // intents headless.
    let mut tui_fx = tui.host_mut().update(key('d'));
    let asked = headless.send(Intent::Invoke { action: "git.branch.delete".into() });
    let question = tui.menu().expect("a question");
    assert_eq!(Some(&question), headless.menu().as_ref(), "the same question");
    assert!(question.title.starts_with("delete the branch spike"), "{question:?}");
    tui_fx.extend(tui.host_mut().update(key('y')));
    let mut headless_fx = asked;
    headless_fx.extend(headless.send(Intent::Invoke { action: "yes".into() }));
    assert!(tui.menu().is_none() && headless.menu().is_none());

    let (a, b) = (shape(&journal(&tui_fx)), shape(&journal(&headless_fx)));
    assert!(
        a.iter().any(|l| l.contains("git.branch.delete")),
        "the part's own action is recorded: {a:#?}"
    );
    assert_eq!(a, b, "the same journal actions");
    let spawned = |fx: &[HostEffect]| fx.iter().filter(|e| matches!(e, HostEffect::Tracked { .. } | HostEffect::Spawn { .. })).count();
    assert_eq!(spawned(&tui_fx), spawned(&headless_fx), "the same work started");
    assert_eq!(tui.screen(), headless.screen(), "the same state after");
}

#[test]
fn a_contributed_cell_shows_its_subject() {
    let mut headless = hosted();
    // The owner of tickets opens it: the Jira screen.
    let jira_screen = |h: &Host| match h.view {
        View::Screen(i) => h.screens[i].0 == 0,
        _ => false,
    };
    assert!(!jira_screen(headless.host()), "not on Jira's screen yet");
    // The cell, by its row and the subject it shows.
    let view = headless.screen().unwrap();
    let Some(Node::Tree(tree)) = view.pane(PaneId::Middle).map(|p| &p.body) else { panic!() };
    assert!(tree.visible().iter().any(|(_, r)| r.cells.iter().any(|c| c.show.is_some())));
    let subject = tree
        .visible()
        .iter()
        .flat_map(|(_, r)| r.cells.iter())
        .find_map(|c| c.show.clone())
        .unwrap();
    headless.focus(PaneId::Middle);
    headless.send(Intent::Show {
        target: branch("alice/ABC-12"),
        subject,
    });
    assert!(jira_screen(headless.host()), "the ticket's owner shows it");
}

/// Every compiled-in part, with its sample data, on a screen of `w` x `h`.
fn all_parts(w: u16, h: u16) -> Headless {
    let mut host = radar_core::demo::demo_host(radar::parts()).unwrap();
    host.view = View::Screen(0);
    Headless::new(host, w, h)
}

/// Parts with no item page in demo data, and why. A part listed here must
/// really reach none (checked), and every other part must reach one.
const NO_ITEM_PAGE: [(&str, &str); 4] = [
    ("sessions", "its sessions live in the agent dock and Settings → Agents; it owns no subject with a page (radar-sessions plugin.rs)"),
    ("git", "show() only selects the subject's row in the branch screen's tree (radar-git screen.rs)"),
    ("workspace", "show() opens the editor and never returns a page (radar-workspace lib.rs)"),
    ("workbench", "its item view is the Work screen itself (show() returns Showing::Page{screen: work}), with no PageView; see the work item test below"),
];

#[test]
fn every_page_a_part_opens_is_a_bounded_view() {
    let mut h = all_parts(160, 40);
    let mut kinds = Vec::new();
    for i in 0..h.screen_count() {
        for page in h.reach_pages(i, 12, 2) {
            assert!(page.gaps.is_empty(), "{page:#?}");
            kinds.push(page.kind);
        }
    }
    let parts: Vec<String> = radar::parts().iter().map(|p| p.id().to_string()).collect();
    for part in &parts {
        let reached = kinds.iter().any(|k| &k.part == part);
        match NO_ITEM_PAGE.iter().find(|(p, _)| p == part) {
            Some((_, why)) => assert!(!reached, "{part} now opens a page: drop it from NO_ITEM_PAGE ({why}): {kinds:#?}"),
            None => assert!(reached, "{part} reached no item page with demo data: {kinds:#?}"),
        }
    }
    let tabs = |part: &str, tab: &str| kinds.iter().any(|k| k.part == part && k.shape.iter().any(|t| t == tab));
    assert!(tabs("gitlab", "Changes") && tabs("gitlab", "Log"), "{kinds:#?}");
    assert!(tabs("github", "Checks") && tabs("github", "Pull requests"), "{kinds:#?}");
    assert!(tabs("jira", "Comments") && tabs("slack", "Thread"), "{kinds:#?}");
}

#[test]
fn a_filter_captures_keys_and_its_text_is_view_data() {
    let mut h = all_parts(160, 40);
    h.show_screen(0);
    assert!(!h.capturing() && h.capture().is_none());
    let chord = |s: &str| radar_core::view::Chord::parse(s).unwrap();
    h.send(Intent::Key { chord: chord("/") });
    assert!(h.capturing());
    assert_eq!(h.capture().map(|c| (c.kind, c.text, c.caret)), Some((radar_core::headless::CaptureKind::Field, String::new(), 0)));
    for c in ["a", "p", "i"] {
        h.send(Intent::Key { chord: chord(c) });
    }
    let typed = h.capture().expect("still typing");
    assert_eq!((typed.text.as_str(), typed.caret), ("api", 3));
    h.send(Intent::Key { chord: chord("esc") });
    assert!(!h.capturing());
}

/// Every row target the view offers is in the text form a test reads, and
/// `Intent::Select` on it (a click, from any renderer) selects that row.
#[test]
fn every_row_target_the_view_offers_is_shown_and_selects_its_row() {
    let probe = hosted();
    let text = probe.text();
    let offered = radar_core::headless::offered(&probe.screen().unwrap());
    let mut compared = 0;
    for o in &offered {
        let mut headless = hosted();
        let Some(body @ Node::Tree(_)) = headless.screen().unwrap().pane(o.pane).map(|p| p.body.clone()) else { continue };
        if !radar_core::headless::node_targets(&body).contains(&o.target) {
            continue; // a chip or a cell, not a row
        }
        let shown = format!("{{{}}}", o.target.0.replace(radar_core::view::PATH_SEP, "/"));
        assert!(text.contains(&shown), "{shown} is not in the text form:\n{text}");
        headless.focus(o.pane);
        headless.send(Intent::Select { target: o.target.clone() });
        let Some(Node::Tree(tree)) = headless.screen().unwrap().pane(o.pane).map(|p| p.body.clone()) else {
            panic!("{:?}: the tree went", o.target)
        };
        assert_eq!(TreeRow::target_of(&tree.selected), o.target, "a click selects its row");
        compared += 1;
    }
    assert!(compared >= 5, "{compared} of {} targets were rows", offered.len());
}

#[test]
fn the_actions_menu_opens_on_a_row_and_an_action_runs_by_its_id() {
    let mut h = hosted();
    let target = branch("spike");
    h.focus(PaneId::Middle);
    h.send(Intent::Select { target: target.clone() });
    // A context intent opens the actions menu, as a right-click does.
    h.send(Intent::Context { target });
    let menu = h.menu().expect("the actions menu");
    assert_eq!(menu.id, "keys");
    assert!(menu.items.iter().any(|i| !i.shown_only), "{menu:?}");
    h.send(Intent::Key { chord: radar_core::view::Chord::parse("esc").unwrap() });
    assert!(h.menu().is_none());

    let view = h.screen().unwrap();
    assert!(view.actions.iter().any(|a| a.id == "git.branch.delete"), "{:?}", view.actions);
    h.send(Intent::Invoke { action: "git.branch.delete".into() });
    let question = h.menu().expect("delete asks first");
    assert!(question.question && question.title.starts_with("delete the branch spike"), "{question:?}");
    h.send(Intent::Invoke { action: "yes".into() });
    let shown = shape(h.records());
    // The part's own action is recorded under the dispatch that ran it.
    let begin = shown.iter().find(|l| l.contains("git.branch.delete")).expect("recorded");
    assert!(begin.contains("under Some("), "{shown:#?}");
}

#[test]
fn a_multi_line_editor_is_driven_by_keys_and_its_text_is_view_data() {
    let mut h = all_parts(160, 40);
    let feed = (0..h.screen_count()).find(|i| h.show_screen(*i) == "feed").expect("the Slack feed");
    h.show_screen(feed);
    let chord = |s: &str| radar_core::view::Chord::parse(s).unwrap();
    h.send(Intent::Invoke { action: "slack.feed.reply".into() });
    let view = h.screen().unwrap();
    assert!(view.overlay.is_some(), "the reply editor is an overlay of the screen");
    let open = h.capture().expect("the reply editor captures");
    assert_eq!((open.kind, open.text.as_str(), open.caret), (radar_core::headless::CaptureKind::Editor, "", 0));
    assert!(h.capturing());
    for c in ["h", "i"] {
        h.send(Intent::Key { chord: chord(c) });
    }
    let typed = h.capture().unwrap();
    assert_eq!((typed.text.as_str(), typed.caret), ("hi", 2));
    // The editor's own view is what a renderer is sent: the extension node.
    let overlay = h.screen().unwrap().overlay.unwrap();
    let editor = radar_core::view::Editor::of(&overlay.body).expect("an editor node");
    assert_eq!(editor.line, "hi");
    h.send(Intent::Key { chord: chord("esc") });
    // Leaving a draft asks first; the answer is an intent too.
    let ask = h.menu().expect("dropping a draft asks");
    assert!(ask.question && ask.title.starts_with("drop this reply draft"), "{ask:?}");
    h.send(Intent::Invoke { action: "yes".into() });
    assert!(!h.capturing() && h.screen().unwrap().overlay.is_none() && h.capture().is_none());
}

#[test]
fn the_demo_work_item_opens_from_its_row_and_fills_the_work_view() {
    let mut h = all_parts(160, 40);
    let work = (0..h.screen_count()).find(|i| h.show_screen(*i) == "work").expect("the Work screen");
    h.show_screen(work);
    let view = h.screen().unwrap();
    let middle = view.pane(PaneId::Middle).expect("the items pane");
    let rows = radar_core::headless::node_targets(&middle.body);
    assert!(!rows.is_empty(), "the demo has a work item row: {:?}", middle.body);
    h.focus(PaneId::Middle);
    h.send(Intent::Select { target: rows[0].clone() });
    h.send(Intent::Activate { target: rows[0].clone() });
    let view = h.screen().unwrap();
    assert!(radar_core::headless::view_problems(&view, "work").is_empty());
    // Only an opened item has linked sources in its Detail pane, one entry
    // per source (the demo pre-loads the selected item's detail, so no
    // request is needed to answer).
    let detail = view.pane(PaneId::Detail).expect("the detail pane");
    let Node::Entries { rows, .. } = &detail.body else { panic!("linked sources: {:?}", detail.body) };
    let heads: Vec<String> = rows.iter().map(|r| r.head.words()).collect();
    for source in ["pull-request · acme/api/7", "ticket · ABC-12"] {
        assert!(heads.iter().any(|h| h.contains(source)), "{source} not in {heads:?}");
    }
    assert!(
        serde_json::to_string(&view).unwrap().contains("Ship explicit work-item navigation"),
        "the item's title is shown"
    );
    // The item's context arrives as the work-area sections of the same screen.
    let caps: Vec<String> = view
        .panes
        .iter()
        .filter_map(|p| match &p.body {
            Node::Extension { capability, .. } => Some(capability.clone()),
            _ => None,
        })
        .collect();
    for want in ["work.map", "work.timeline", "work.tabs", "work.rail"] {
        assert!(caps.iter().any(|c| c == want), "{want} missing: {caps:?}");
    }
}

/// Slack's agent rows reach the part from the host: a key on a row (not one
/// the host keeps for a screen), the row's id, and an unlisted id.
#[test]
fn the_agent_keys_and_ids_of_a_slack_message_reach_the_part_through_the_host() {
    let requests = |h: &Headless, kind: &str| shape(h.records()).iter().filter(|l| l.contains(kind)).count();
    let mut h = all_parts(160, 40);
    let feed = (0..h.screen_count()).find(|i| h.show_screen(*i) == "feed").expect("the Slack feed");
    h.show_screen(feed);
    h.focus(PaneId::Middle);
    let rows = radar_core::headless::node_targets(&h.screen().unwrap().pane(PaneId::Middle).unwrap().body);
    let linked = rows
        .iter()
        .find(|t| {
            h.send(Intent::Select { target: (*t).clone() });
            h.screen().unwrap().actions.iter().any(|a| a.id == "agent.review_locally")
        })
        .expect("a message that links a merge request offers the review")
        .clone();
    h.send(Intent::Select { target: linked });
    let chord = |s: &str| radar_core::view::Chord::parse(s).unwrap();
    h.send(Intent::Invoke { action: "agent.unlisted".into() });
    assert_eq!(requests(&h, "agent."), 0, "an unlisted id asks for nothing: {:#?}", shape(h.records()));
    h.send(Intent::Key { chord: chord("A") });
    assert_eq!(requests(&h, "agent.send.request"), 1, "A: {:#?}", shape(h.records()));
    h.send(Intent::Key { chord: chord("v") });
    assert_eq!(requests(&h, "agent.review.request"), 1, "v: {:#?}", shape(h.records()));
    h.send(Intent::Invoke { action: "agent.review_locally".into() });
    assert_eq!(requests(&h, "agent.review.request"), 2, "the id: {:#?}", shape(h.records()));
}

/// GitLab's merge request row offers the review through the host: a key on
/// the row, the shared id, and an unlisted id that asks for nothing.
#[test]
fn the_review_key_and_id_of_a_gitlab_merge_request_reach_the_part_through_the_host() {
    let requests = |h: &Headless, kind: &str| shape(h.records()).iter().filter(|l| l.contains(kind)).count();
    let mut h = all_parts(160, 40);
    let repos = (0..h.screen_count()).find(|i| h.show_screen(*i) == "repos").expect("the GitLab screen");
    h.show_screen(repos);
    h.focus(PaneId::Middle);
    let rows = radar_core::headless::node_targets(&h.screen().unwrap().pane(PaneId::Middle).unwrap().body);
    let mr = rows
        .iter()
        .find(|t| {
            h.send(Intent::Select { target: (*t).clone() });
            h.screen().unwrap().actions.iter().any(|a| a.id == "agent.review_locally")
        })
        .expect("an open merge request offers the review")
        .clone();
    h.send(Intent::Select { target: mr });
    let chord = |s: &str| radar_core::view::Chord::parse(s).unwrap();
    h.send(Intent::Invoke { action: "agent.unlisted".into() });
    assert_eq!(requests(&h, "agent."), 0, "an unlisted id asks for nothing: {:#?}", shape(h.records()));
    let key = h.screen().unwrap().actions.iter().find(|a| a.id == "agent.review_locally").and_then(|a| a.key);
    assert_eq!(key, Some(chord("i")), "the row lists its key");
    h.send(Intent::Key { chord: chord("i") });
    assert_eq!(requests(&h, "agent.review.request"), 1, "i: {:#?}", shape(h.records()));
    h.send(Intent::Invoke { action: "agent.review_locally".into() });
    assert_eq!(requests(&h, "agent.review.request"), 2, "the id: {:#?}", shape(h.records()));
}

/// Setup's part steps, each opened as `radar setup <step>` opens it: the
/// card is the part's own settings section, so its actions are listed and
/// run by their ids through the host, and an unlisted id does nothing.
#[cfg(all(feature = "gitlab", feature = "github", feature = "slack"))]
mod setup_steps {
    use super::*;
    use radar_core::view::{Target, TreeRow};

    /// A row of the step's section: its words and where selecting it goes.
    struct Row {
        words: String,
        target: Target,
    }

    /// Every compiled-in part with its sample, setup open at `step`.
    fn at_step(step: &str) -> Headless {
        let mut h = all_parts(160, 40);
        h.host_mut().open_setup(Some(step)).unwrap();
        h
    }

    fn rows(h: &Headless) -> Vec<Row> {
        match h.section().expect("the step's section").body {
            Node::List { rows, .. } => rows.into_iter().map(|r| Row { words: r.text.words(), target: r.target }).collect(),
            Node::Tree(tree) => tree
                .visible()
                .into_iter()
                .map(|(path, row)| Row { words: row.name.words(), target: TreeRow::target_of(&path) })
                .collect(),
            other => panic!("a list or a tree: {other:?}"),
        }
    }

    /// Select the first row whose words hold `words`.
    fn select(h: &mut Headless, words: &str) {
        let row = rows(h).into_iter().find(|r| r.words.contains(words)).unwrap_or_else(|| panic!("no row {words:?}"));
        h.send(Intent::Select { target: row.target });
    }

    fn listed(h: &Headless, id: &str) -> bool {
        h.section().unwrap().actions.iter().any(|a| a.id == id)
    }

    /// An effect, out of the journal's tracking.
    fn bare(e: &HostEffect) -> &HostEffect {
        match e {
            HostEffect::Tracked { effect, .. } => bare(effect),
            e => e,
        }
    }

    fn opened(fx: &[HostEffect]) -> Option<&str> {
        fx.iter().find_map(|e| if let HostEffect::Open(u) = bare(e) { Some(u.as_str()) } else { None })
    }

    fn copied(fx: &[HostEffect]) -> Option<&str> {
        fx.iter().find_map(|e| if let HostEffect::Copy(t) = bare(e) { Some(t.as_str()) } else { None })
    }

    fn part(h: &Headless, id: &str) -> usize {
        h.host().plugins.iter().position(|p| p.id() == id).unwrap()
    }

    /// An unlisted id opens, copies, asks and types nothing.
    fn unlisted_does_nothing(h: &mut Headless, id: &str) {
        let p = part(h, id.split('.').next().unwrap());
        let fx = h.send(Intent::Invoke { action: id.into() });
        assert!(opened(&fx).is_none() && copied(&fx).is_none(), "{id}: {fx:?}");
        assert!(h.menu().is_none() && !h.host().plugins[p].capturing(), "{id}");
    }

    #[test]
    fn gitlabs_step_opens_the_token_page_and_follows_groups_by_id() {
        let mut h = at_step("gitlab");
        unlisted_does_nothing(&mut h, "gitlab.connection.unlisted");
        assert!(listed(&h, "gitlab.connection.token_page"));
        let fx = h.send(Intent::Invoke { action: "gitlab.connection.token_page".into() });
        assert!(opened(&fx).is_some_and(|u| u.contains("/-/user_settings/personal_access_tokens?name=radar")), "{fx:?}");
        let fx = h.send(Intent::Invoke { action: "gitlab.connection.copy_token_link".into() });
        assert!(copied(&fx).is_some_and(|u| u.contains("personal_access_tokens")), "{fx:?}");
        select(&mut h, "add a group or repository");
        assert!(listed(&h, "gitlab.connection.follow"));
        h.send(Intent::Invoke { action: "gitlab.connection.follow".into() });
        let p = part(&h, "gitlab");
        assert!(h.host().plugins[p].capturing(), "typing the group to follow");
        h.send(Intent::Key { chord: radar_core::view::Chord::parse("esc").unwrap() });
        select(&mut h, "team-a");
        h.send(Intent::Invoke { action: "gitlab.connection.unfollow".into() });
        assert!(h.menu().is_some_and(|m| m.question && m.title.starts_with("stop tracking")), "{:?}", h.menu());
        h.send(Intent::Key { chord: radar_core::view::Chord::parse("esc").unwrap() });
        // The token's access: read and write asks the new-token page for api.
        select(&mut h, "read and write");
        assert!(listed(&h, "gitlab.connection.choose"));
        h.send(Intent::Invoke { action: "gitlab.connection.choose".into() });
        assert_eq!(h.host().plugins[p].config().unwrap()["access"].as_str(), Some("write"));
        let fx = h.send(Intent::Invoke { action: "gitlab.connection.token_page".into() });
        assert!(opened(&fx).is_some_and(|u| u.ends_with("scopes=api")), "{fx:?}");
    }

    #[test]
    fn githubs_step_signs_in_with_gh_by_id() {
        let mut h = at_step("github");
        unlisted_does_nothing(&mut h, "github.connection.unlisted");
        select(&mut h, "gh's sign-in");
        assert!(listed(&h, "github.connection.choose"));
        h.send(Intent::Invoke { action: "github.connection.choose".into() });
        let p = part(&h, "github");
        assert_eq!(h.host().plugins[p].config().unwrap()["token"].as_str(), Some("gh"));
        let fx = h.send(Intent::Invoke { action: "github.connection.token_page".into() });
        assert!(opened(&fx).is_some_and(|u| u.contains("/settings/personal-access-tokens/new?name=radar")), "{fx:?}");
        let fx = h.send(Intent::Invoke { action: "github.connection.copy_token_link".into() });
        assert!(copied(&fx).is_some(), "{fx:?}");
        select(&mut h, "add a repository or group");
        h.send(Intent::Invoke { action: "github.connection.follow".into() });
        assert!(h.host().plugins[p].capturing(), "typing the repository to follow");
    }

    #[test]
    fn jiras_step_opens_the_token_page_and_watches_projects_by_id() {
        let mut h = at_step("jira");
        unlisted_does_nothing(&mut h, "jira.connection.unlisted");
        select(&mut h, "account");
        let fx = h.send(Intent::Invoke { action: "jira.connection.token_page".into() });
        assert!(opened(&fx).is_some_and(|u| u.contains("id.atlassian.com/manage-profile/security/api-tokens")), "{fx:?}");
        let fx = h.send(Intent::Invoke { action: "jira.connection.copy_token_link".into() });
        assert!(copied(&fx).is_some(), "{fx:?}");
        select(&mut h, "projects ");
        assert!(listed(&h, "jira.connection.watch"));
        h.send(Intent::Invoke { action: "jira.connection.watch".into() });
        let p = part(&h, "jira");
        assert!(h.host().plugins[p].capturing(), "typing the project to watch");
    }

    #[test]
    fn slacks_step_copies_the_manifest_and_adds_channels_by_id() {
        let mut h = at_step("slack");
        unlisted_does_nothing(&mut h, "slack.connection.unlisted");
        let fx = h.send(Intent::Invoke { action: "slack.connection.copy_manifest".into() });
        assert!(copied(&fx).is_some_and(|m| m.contains("\"oauth_config\"") && m.contains("channels:history")), "{fx:?}");
        let fx = h.send(Intent::Invoke { action: "slack.connection.new_app".into() });
        assert!(opened(&fx).is_some_and(|u| u.contains("manifest_json=")), "{fx:?}");
        select(&mut h, "add a channel");
        assert!(listed(&h, "slack.connection.add_channel"));
        h.send(Intent::Invoke { action: "slack.connection.add_channel".into() });
        let p = part(&h, "slack");
        assert!(h.host().plugins[p].capturing(), "typing the channel to add");
    }

    #[test]
    fn local_codes_step_looks_in_a_found_place_by_id() {
        let mut h = all_parts(160, 40);
        let at = part(&h, "git");
        let mut git = radar_git::GitPlugin::new();
        git.demo();
        git.offer_places(vec![("~/ankh-morpork".into(), 3)]);
        h.host_mut().plugins[at] = Box::new(git);
        h.host_mut().open_setup(Some("local")).unwrap();
        unlisted_does_nothing(&mut h, "git.checkouts.unlisted");
        select(&mut h, "~/ankh-morpork");
        assert!(listed(&h, "git.checkouts.follow"));
        h.send(Intent::Invoke { action: "git.checkouts.follow".into() });
        let roots = h.host().plugins[at].config().unwrap()["roots"].clone();
        assert!(roots.as_array().unwrap().iter().any(|r| r.as_str() == Some("~/ankh-morpork")), "{roots:?}");
    }

    /// Look down is a number typed in place: ⏎ on its row opens the field,
    /// a digit replaces it, ⏎ keeps it; no menu between.
    #[test]
    fn local_codes_look_down_takes_a_digit_in_place() {
        let mut h = at_step("local");
        let at = part(&h, "git");
        select(&mut h, "look down");
        let row = rows(&h).into_iter().find(|r| r.words.contains("look down")).unwrap();
        h.send(Intent::Activate { target: row.target });
        assert!(h.menu().is_none() && h.host().plugins[at].capturing());
        h.send(Intent::Key { chord: radar_core::view::Chord::char('2') });
        h.send(Intent::Key { chord: radar_core::view::Chord::new(radar_core::view::Key::Enter) });
        assert!(!h.host().plugins[at].capturing());
        assert_eq!(h.host().plugins[at].config().unwrap()["depth"].as_integer(), Some(2));
    }

    /// The terminal is its own settings section: its actions run by id.
    #[test]
    fn the_terminal_section_types_a_command_by_id() {
        let mut h = all_parts(160, 40);
        let at = part(&h, "git");
        let section = h.host().sections.iter().position(|(p, s)| *p == radar_core::host::Owner::Plugin(at) && s.id == "terminal").expect("git's terminal section");
        h.host_mut().view = View::Settings;
        h.host_mut().section = section;
        unlisted_does_nothing(&mut h, "git.terminal.unlisted");
        assert!(listed(&h, "git.terminal.edit"));
        h.send(Intent::Invoke { action: "git.terminal.edit".into() });
        assert!(h.host().plugins[at].capturing(), "typing a command");
    }
}

/// Git's chores on a merge request are one row of its `x` menu, "push,
/// worktree, new branch, link…": choosing it by its value opens git's own small menu,
/// whose rows run by their values, and a value nobody lists does nothing.
#[test]
fn the_chores_row_of_a_merge_request_opens_gits_menu_and_an_unlisted_value_does_nothing() {
    let mut h = all_parts(160, 40);
    let repos = (0..h.screen_count()).find(|i| h.show_screen(*i) == "repos").expect("the GitLab screen");
    h.show_screen(repos);
    h.focus(PaneId::Middle);
    let rows = radar_core::headless::node_targets(&h.screen().unwrap().pane(PaneId::Middle).unwrap().body);
    let chores = |h: &Headless| h.menu().and_then(|m| m.items.iter().find(|i| i.label.starts_with("push, worktree")).map(|i| i.value.clone()));
    let mut found = None;
    for t in &rows {
        h.send(Intent::Select { target: t.clone() });
        h.send(Intent::Context { target: t.clone() });
        if let Some(value) = chores(&h) {
            found = Some(value);
            break;
        }
        h.send(Intent::Key { chord: radar_core::view::Chord::parse("esc").unwrap() });
    }
    let value = found.expect("a merge request whose repo is checked out here offers the chores");
    let keys = h.menu().unwrap().id.clone();
    let ran = |h: &Headless| shape(h.records()).iter().filter(|l| l.contains("plugin.action")).count();
    // A value nobody lists: the menu stays as it was and nothing is asked.
    h.send(Intent::Invoke { action: "git.unlisted".into() });
    assert_eq!(h.menu().unwrap().id, keys);
    let before = ran(&h);
    h.send(Intent::Invoke { action: value });
    assert!(ran(&h) > before, "the row ran git's action: {:#?}", shape(h.records()));
    let menu = h.menu().expect("the chores menu");
    assert!(menu.id.starts_with("git-chores:"), "{}", menu.id);
    let labels: Vec<&str> = menu.items.iter().map(|i| i.label.as_str()).collect();
    assert!(labels.iter().any(|l| l.starts_with("push ")) && labels.contains(&"new branch…"), "{labels:?}");
    let before = ran(&h);
    h.send(Intent::Invoke { action: "git.unlisted".into() });
    assert_eq!(h.menu().unwrap().id, menu.id, "an unlisted value changes nothing");
    assert_eq!(ran(&h), before, "and runs no action");
    h.send(Intent::Invoke { action: "link".into() });
    assert!(h.menu().is_none_or(|m| !m.id.starts_with("git-chores:")), "a row of the chores menu was chosen by its value");
}
