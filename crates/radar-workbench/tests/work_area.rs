//! The Work screen carries the M11 work-area sections as extension panes
//! (`work.map`, `work.timeline`, `work.tabs`, `work.agent`, `work.rail`),
//! built from real workbench state: a task created through the host, the
//! journal and the work API, read back through `Headless`.
//!
//! `RADAR_WORK_VIEW_FIXTURE=1` rewrites tests/fixtures/work_view.json, the
//! recorded Work view the Ink work-area tests render.

use radar_core::headless::Headless;
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionDescriptor, ActionId, Journal, Origin};
use radar_core::source_context::WorkReply;
use radar_core::view::{Chord, Intent, Key, Node, PaneId, ScreenView};
use radar_core::{Host, work_api};
use std::collections::VecDeque;

const NOW: i64 = 1000;

struct Store {
    journal: Journal,
    session: radar_core::journal::SessionId,
}

fn store() -> Store {
    let path = std::env::temp_dir()
        .join(format!("radar-work-area-{}", ActionId::new().as_str()))
        .join("journal.sqlite3");
    let mut journal = Journal::open(&path).unwrap();
    let session = journal.start_session().unwrap();
    let action = ActionId::new();
    journal
        .begin_with_id_at(
            &session,
            &action,
            ActionDescriptor::new("source.selected", Origin::Host),
            None,
            NOW,
        )
        .unwrap();
    Store { journal, session }
}

fn pump(host: &mut Host, s: &mut Store, effects: Vec<HostEffect>) {
    let mut queue: VecDeque<_> = effects.into();
    let mut steps = 0;
    while let Some(effect) = queue.pop_front() {
        steps += 1;
        assert!(steps < 500);
        match effect {
            HostEffect::Journal(JournalRecord::Begin { id, descriptor, parent }) => s
                .journal
                .begin_with_id_at(&s.session, &id, descriptor, parent.as_ref(), NOW)
                .unwrap(),
            HostEffect::Journal(JournalRecord::Outcome { id, outcome, metadata }) => {
                s.journal.record_at(&id, outcome, metadata, NOW).unwrap();
            }
            HostEffect::Journal(JournalRecord::Gap { .. }) => panic!("unexpected gap"),
            HostEffect::Work { plugin, id, request, mode } => {
                let result = work_api::test_support::execute(
                    &mut s.journal,
                    &s.session,
                    *request,
                    mode,
                    NOW,
                );
                queue.extend(host.update(Event::Msg {
                    plugin,
                    msg: Box::new(WorkReply { id, result }),
                }));
            }
            HostEffect::Tracked { effect, .. } => queue.push_front(*effect),
            HostEffect::WriteCache { .. } | HostEffect::Save(_) => {}
            _ => panic!("unexpected runner effect"),
        }
    }
}

fn send(h: &mut Headless, s: &mut Store, intent: Intent) {
    let effects = h.send(intent);
    pump(h.host_mut(), s, effects);
}

fn work(width: u16, height: u16) -> (Headless, Store) {
    let mut host = Host::new(
        vec![Box::new(radar_workbench::WorkbenchPlugin::default())],
        radar_core::plugin::DEMO_NOW,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    (Headless::new(host, width, height), store())
}

/// Create one task through the host: the real path, journal included.
fn with_task(h: &mut Headless, s: &mut Store, title: &str) {
    send(h, s, Intent::Invoke { action: "work.new".into() });
    let effects = h.host_mut().update(Event::Paste(title.into()));
    pump(h.host_mut(), s, effects);
    send(h, s, Intent::Key { chord: Chord::new(Key::Enter) });
}

fn extensions(view: &ScreenView) -> Vec<(String, serde_json::Value)> {
    view.panes
        .iter()
        .filter_map(|p| match &p.body {
            Node::Extension { capability, data, fallback, .. } => {
                assert!(matches!(**fallback, Node::Document { .. }), "{capability} needs a fallback");
                Some((capability.clone(), data.clone()))
            }
            _ => None,
        })
        .collect()
}

fn names(view: &ScreenView) -> Vec<String> {
    extensions(view).into_iter().map(|(c, _)| c).collect()
}

#[test]
fn empty_work_carries_map_timeline_tabs_but_no_agent_area() {
    let (h, _s) = work(160, 40);
    let view = h.screen().unwrap();
    assert_eq!(names(&view), ["work.map", "work.timeline", "work.tabs", "work.rail"]);
    // The screen's own three panes come first and are unchanged in kind.
    assert!(view.pane(PaneId::Nav).is_some() && view.pane(PaneId::Middle).is_some());
    assert!(matches!(view.panes[2].body, Node::Document { .. } | Node::Entries { .. }));
}

#[test]
fn a_created_task_reaches_the_map_and_a_pending_confirmation_stays_in_the_view() {
    let (mut h, mut s) = work(160, 40);
    send(&mut h, &mut s, Intent::Invoke { action: "work.refresh".into() });
    send(&mut h, &mut s, Intent::Invoke { action: "work.new".into() });
    let effects = h.host_mut().update(Event::Paste("Ship the work area".into()));
    pump(h.host_mut(), &mut s, effects);
    send(&mut h, &mut s, Intent::Key { chord: Chord::new(Key::Enter) });
    // The confirmation question is open: the view must still carry the
    // pending section, so a collapsed layout can keep it first.
    assert!(h.menu().unwrap().question);
    let view = h.screen().unwrap();
    let ext = extensions(&view);
    let agent = ext.iter().find(|(c, _)| c == "work.agent").expect("pending question in view");
    assert_eq!(agent.1["pending"], true);
    send(&mut h, &mut s, Intent::Invoke { action: "yes".into() });
    let view = h.screen().unwrap();
    assert!(!names(&view).contains(&"work.agent".to_string()), "answered question leaves");
    let ext = extensions(&view);
    let map = &ext.iter().find(|(c, _)| c == "work.map").unwrap().1;
    let items = map["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["title"], "Ship the work area");
    assert_eq!(items[0]["selected"], true);
    let rail = &ext.iter().find(|(c, _)| c == "work.rail").unwrap().1;
    assert!(rail["actions"].as_array().unwrap().iter().any(|a| a["id"] == "work.new"));
    for (_, data) in &ext {
        assert!(data["target"].as_str().is_some_and(|t| t.starts_with("work-area:")));
        assert!(data["pending"].is_boolean());
    }
}

/// Journal ids and wall-clock times differ per run; the text must not.
fn steady(text: &str) -> String {
    let is_uuid = |c: &str| {
        c.len() == 36
            && c.char_indices().all(|(i, ch)| {
                if matches!(i, 8 | 13 | 18 | 23) { ch == '-' } else { ch.is_ascii_hexdigit() }
            })
    };
    let mut out = text.to_string();
    for i in text.char_indices().map(|(i, _)| i) {
        if let Some(c) = text.get(i..i + 36).filter(|c| is_uuid(c)) {
            out = out.replace(c, "00000000-0000-4000-8000-000000000000");
        }
    }
    for t in text.split(|c: char| !c.is_ascii_digit()).filter(|t| t.len() == 13) {
        out = out.replace(t, "1700000000000");
    }
    out
}

/// The Ratatui shell's text of the same host state at both sizes, recorded as
/// tests/fixtures/<name>_ratatui_<w>x<h>.txt for the Ink parity tests; in a
/// normal run the committed text must still be what Ratatui paints.
fn ratatui_text(name: &str, h: &Headless) {
    for (w, ht) in [(80u16, 24u16), (160, 40)] {
        let text = steady(&radar_core::shell::to_text(h.host(), w, ht));
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(format!("{name}_ratatui_{w}x{ht}.txt"));
        if std::env::var_os("RADAR_WORK_VIEW_FIXTURE").is_some() {
            std::fs::write(&path, &text).unwrap();
        }
        let recorded = std::fs::read_to_string(&path).expect("run with RADAR_WORK_VIEW_FIXTURE=1 once");
        assert_eq!(recorded, text, "{name} at {w}x{ht} drifted; RADAR_WORK_VIEW_FIXTURE=1 re-records");
    }
}

/// Record `view` as tests/fixtures/<name>.json when RADAR_WORK_VIEW_FIXTURE is
/// set, then check the committed file still has the work-area shape.
fn record(name: &str, h: &Headless) -> serde_json::Value {
    let view = &h.screen().unwrap();
    ratatui_text(name, h);
    let json = serde_json::to_string_pretty(view).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.json"));
    if std::env::var_os("RADAR_WORK_VIEW_FIXTURE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, format!("{json}\n")).unwrap();
    }
    let recorded: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(&path).expect("run with RADAR_WORK_VIEW_FIXTURE=1 once"),
    )
    .unwrap();
    // Journal ids differ per run; the work-area shape must not.
    let shape = |v: &serde_json::Value| {
        v["panes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|p| p["body"]["capability"].as_str().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&recorded), shape(&serde_json::from_str(&json).unwrap()));
    assert_eq!(recorded["scope"], "work");
    recorded
}

#[test]
fn records_the_idle_work_view_fixture() {
    let (mut h, mut s) = work(160, 40);
    send(&mut h, &mut s, Intent::Invoke { action: "work.refresh".into() });
    with_task(&mut h, &mut s, "Ship the work area");
    send(&mut h, &mut s, Intent::Invoke { action: "yes".into() });
    record("work_view", &h);
}

#[test]
fn records_the_work_view_with_a_pending_question_fixture() {
    let (mut h, mut s) = work(160, 40);
    send(&mut h, &mut s, Intent::Invoke { action: "work.refresh".into() });
    with_task(&mut h, &mut s, "Ship the work area");
    assert!(h.menu().unwrap().question);
    let recorded = record("work_view_pending", &h);
    let agent = recorded["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["body"]["capability"] == "work.agent")
        .expect("the pending question is in the recorded view");
    assert_eq!(agent["body"]["data"]["pending"], true);
    assert_eq!(agent["body"]["data"]["status"], "awaiting confirmation");
}

/// GUIDE 2.12: no chevrons and no `[+]`/`[-]`; a selected item is shown by a
/// style on its line, not by a glyph in front of it.
#[test]
fn no_work_view_line_marks_a_row_with_a_chevron() {
    let (mut h, mut s) = work(160, 40);
    send(&mut h, &mut s, Intent::Invoke { action: "work.refresh".into() });
    with_task(&mut h, &mut s, "Ship the work area");
    send(&mut h, &mut s, Intent::Invoke { action: "yes".into() });
    let view = h.screen().unwrap();
    let mut styled = 0;
    for pane in &view.panes {
        let Node::Extension { fallback, .. } = &pane.body else { continue };
        let Node::Document { lines, .. } = &**fallback else { continue };
        for line in lines {
            let text = line.words();
            for bad in ['▸', '▶', '›', '▾', '▼'] {
                assert!(!text.contains(bad), "{bad} marks a line: {text:?}");
            }
            assert!(!text.contains("[+]") && !text.contains("[-]"), "{text:?}");
            styled += line.0.iter().filter(|seg| seg.strong).count();
        }
    }
    assert!(styled > 0, "the selected item has a style on its line");
}
