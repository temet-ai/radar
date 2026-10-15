//! Renderer parity (M11 handover item 4, C4(a)): the same parts with their
//! sample data, driven three ways at once — through the Ratatui adapter
//! (painted frames, terminal keys and clicks), through `Headless` (view
//! data and intents) and through `ipc::Bridge`, the host side of the Ink
//! renderer's protocol (the messages an Ink renderer sends, no node). For
//! the repositories journey and the Work journey, every step shows the same
//! view with the same targets in all three, a write asks the same question,
//! and the journal records the same actions under the same causal parents.

#![cfg(all(feature = "gitlab", feature = "workbench", feature = "demo"))]

use radar_core::headless::{Headless, offered};
use radar_core::host::{Event, Hit, HostEffect, JournalRecord, MouseKind, View};
use radar_core::ipc::{Bridge, Capabilities, ImageProtocol, PROTOCOL, ToHost, ToRenderer};
use radar_core::journal::ActionId;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::shell::to_text;
use radar_core::view::{Chord, Intent, PaneId, ScreenView, Target};
use radar_core::Host;
use radar_core::writes::Mode;

const W: u16 = 160;
const H: u16 = 40;

/// Every compiled-in part with its sample data, as `radar shot` has them.
fn demo() -> Host {
    radar_core::shot::demo_host(radar::parts()).expect("the parts fit together")
}

/// The journal's records with their ids numbered in order of appearance:
/// what was recorded, under which parent, with what outcome.
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

/// The effects that send something: a request to the Work store or a task
/// started (inside their causal wrapper). A part's own cache written to radar's
/// state folder is not a send.
fn starts(fx: &[HostEffect]) -> usize {
    fn sends(e: &HostEffect) -> bool {
        match e {
            HostEffect::Tracked { effect, .. } => sends(effect),
            HostEffect::Work { .. } | HostEffect::Spawn { .. } => true,
            _ => false,
        }
    }
    fx.iter().filter(|e| sends(e)).count()
}

fn chord(s: &str) -> Chord {
    Chord::parse(s).unwrap_or_else(|| panic!("{s} is a chord"))
}

fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}

/// The three frontends over three hosts built the same way.
struct Three {
    /// The Ratatui adapter: frames drawn, terminal events in.
    tui: Headless,
    tui_journal: Vec<JournalRecord>,
    headless: Headless,
    bridge: Bridge,
    /// Everything the bridge told the renderer, in order.
    told: Vec<ToRenderer>,
    seq: u64,
    /// Clicks sent to the Ratatui adapter.
    clicks: usize,
    /// Work each frontend's host started (tasks, tracked work): Ratatui,
    /// Headless, the bridge.
    started: [usize; 3],
}

impl Three {
    fn new() -> Self {
        let mut tui_host = demo();
        tui_host.enable_journal();
        let mut bridge = Bridge::new(demo(), 80, 24);
        // The renderer says hello at the size of its terminal.
        bridge.receive(ToHost::Hello {
            protocol: PROTOCOL,
            view: radar_core::view::VERSION,
            capabilities: Capabilities {
                columns: W,
                rows: H,
                mouse: true,
                true_colour: true,
                image: ImageProtocol::None,
                plain_text: false,
            },
        });
        assert!(bridge.welcomed(), "the bridge welcomes a renderer of its protocol");
        let mut three = Self {
            tui: Headless::new(tui_host, W, H),
            tui_journal: Vec::new(),
            headless: Headless::new(demo(), W, H),
            bridge,
            told: Vec::new(),
            seq: 0,
            clicks: 0,
            started: [0; 3],
        };
        three.drain();
        three
    }

    /// Read what the bridge sends, acknowledging each snapshot as Ink does.
    fn drain(&mut self) {
        for _ in 0..4 {
            let out = self.bridge.outgoing();
            if out.is_empty() {
                break;
            }
            for m in &out {
                if let ToRenderer::Snapshot(s) = m {
                    self.bridge.receive(ToHost::Ack { cursor: s.cursor });
                }
            }
            self.told.extend(out);
        }
    }

    fn show(&mut self, id: &str) {
        let at = self.headless.host().screens.iter().position(|(_, s)| s.id == id).unwrap_or_else(|| panic!("the {id} screen"));
        self.tui.host_mut().view = View::Screen(at);
        self.headless.show_screen(at);
        self.bridge.host_mut().view = View::Screen(at);
        self.drain();
    }

    /// A terminal event to the Ratatui adapter, on a freshly drawn frame.
    fn terminal(&mut self, event: Event) {
        let _ = to_text(self.tui.host(), W, H);
        let fx = self.tui.host_mut().update(event);
        self.started[0] += starts(&fx);
        self.tui_journal.extend(fx.into_iter().filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r),
            _ => None,
        }));
    }

    /// A click on the row the terminal drew for the `i`th target its view
    /// offers.
    fn click(&mut self, i: usize) {
        let target = nth(&self.tui.screen().expect("a screen"), i);
        self.click_on(&target);
    }

    fn click_on(&mut self, target: &Target) {
        let _ = to_text(self.tui.host(), W, H);
        let rect = self
            .tui
            .host()
            .hits
            .borrow()
            .iter()
            .find(|(_, h)| matches!(h, Hit::Plugin { id, .. } if *id == target.0))
            .map(|(r, _)| *r)
            .unwrap_or_else(|| panic!("{target:?} is drawn as a target"));
        self.clicks += 1;
        self.terminal(Event::Mouse {
            kind: MouseKind::Press,
            column: rect.x + 1,
            row: rect.y,
            at_ms: 0,
        });
    }

    /// An intent from the view-data frontends: Headless, and the Ink
    /// renderer's `Intent` message through the bridge.
    fn intent(&mut self, intent: Intent) {
        self.both(|_| intent.clone());
    }

    /// An intent about each frontend's own view (its targets name its own
    /// store), from Headless and through the bridge.
    fn both(&mut self, intent: impl Fn(&ScreenView) -> Intent) {
        let headless = intent(&self.headless.screen().expect("a screen"));
        self.started[1] += starts(&self.headless.send(headless));
        let bridge = intent(&self.bridge.snapshot_now(0).screen.expect("a screen"));
        self.seq += 1;
        let got = self.bridge.receive(ToHost::Intent { seq: self.seq, cursor: 0, intent: bridge });
        assert!(!got.quit, "an intent does not end the bridge");
        self.started[2] += starts(&got.effects);
        self.drain();
    }

    fn select(&mut self, i: usize) {
        self.both(|v| Intent::Select { target: nth(v, i) });
    }

    fn activate(&mut self, i: usize) {
        self.both(|v| Intent::Activate { target: nth(v, i) });
    }

    /// The mode for this run, as RADAR_MODE sets it (the parts read it on this
    /// thread, which the three hosts share).
    fn mode(&mut self, mode: Option<(Mode, String)>) {
        self.tui.host_mut().set_env_mode(mode.clone());
        self.headless.host_mut().set_env_mode(mode.clone());
        self.bridge.host_mut().set_env_mode(mode);
        self.drain();
    }

    /// `n`, the title typed and Enter; the question it asks, the same in all.
    fn new_task(&mut self, title: &str) -> radar_core::view::MenuView {
        self.terminal(key(KeyCode::Char('n')));
        let told = self.told.len();
        self.intent(Intent::Invoke { action: "work.new".into() });
        let typing = self.headless.menu().expect("the title is asked for");
        assert_eq!(self.tui.menu().map(|m| m.id), Some(typing.id.clone()));
        assert!(
            self.told[told..].iter().any(|m| matches!(m, ToRenderer::InputRequest(_))),
            "the renderer is asked for input: {:?}",
            &self.told[told..]
        );
        for c in title.chars() {
            self.terminal(key(KeyCode::Char(c)));
            self.intent(Intent::Key { chord: chord(&c.to_string()) });
        }
        self.terminal(key(KeyCode::Enter));
        self.intent(Intent::Key { chord: chord("enter") });
        let question = self.same_question();
        assert_eq!(question.id, "work.confirm", "{question:?}");
        assert!(question.title.starts_with("Save these local work annotations"), "{question:?}");
        question
    }

    /// Answer the question: a key on the terminal, its value as an intent.
    /// Returns the work the answer started, the same in all three (the
    /// Work store's write leaves as a task).
    fn answer(&mut self, key_char: char, value: &str) -> usize {
        let before = self.started;
        self.terminal(key(KeyCode::Char(key_char)));
        self.intent(Intent::Invoke { action: value.into() });
        assert!(self.headless.menu().is_none() && self.tui.menu().is_none() && self.bridge.host().menu_view().is_none(), "answered");
        let started = [0, 1, 2].map(|i| self.started[i] - before[i]);
        assert!(started.iter().all(|n| *n == started[0]), "the same work started: {started:?}");
        started[0]
    }

    /// Focus moves in the frontend, as each renderer does on its own.
    fn focus(&mut self, pane: PaneId) {
        self.headless.focus(pane);
        self.bridge.host_mut().focus = pane.into();
    }

    /// The screen as each frontend has it.
    fn views(&self) -> [Option<ScreenView>; 3] {
        [self.tui.screen(), self.headless.screen(), self.bridge.snapshot_now(0).screen]
    }

    /// The three show the same view, with the same targets, and the bridge
    /// sent the renderer exactly that view.
    fn same(&self, step: &str) -> ScreenView {
        let [tui, headless, bridge] = self.views();
        let sent = self.told.iter().rev().find_map(|m| match m {
            ToRenderer::Snapshot(s) => Some(s.screen.clone()),
            _ => None,
        });
        assert_eq!(sent, Some(bridge.clone()), "{step}: the last snapshot sent is the view");
        let section = self.told.iter().rev().find_map(|m| match m {
            ToRenderer::Snapshot(s) => Some(s.section.is_some()),
            _ => None,
        });
        assert_eq!(section, Some(false), "{step}: on a screen, no settings section is sent");
        let [tui, headless, bridge] = [tui, headless, bridge].map(|v| v.unwrap_or_else(|| panic!("{step}: a screen view")));
        assert_eq!(same_store(&tui), same_store(&headless), "{step}: Ratatui and Headless");
        assert_eq!(same_store(&headless), same_store(&bridge), "{step}: Headless and the bridge");
        let targets = same_targets(&headless);
        assert!(!offered(&headless).is_empty(), "{step}: the view offers targets");
        for v in [&tui, &bridge] {
            assert_eq!(same_targets(v), targets, "{step}: equal targets");
        }
        headless
    }

    /// The same journal: the same actions under the same parents.
    /// A click is recorded as a pointer dispatch (`ui.pointer.action`, and
    /// `plugin.pointer.action` for the part), the intent that does the same
    /// as `ui.action` and `plugin.input`: the same action under the same
    /// parent, by another hand. Each click is counted, then spelt as its
    /// intent; nothing else may differ.
    fn same_journal(&self) -> Vec<String> {
        let tui = shape(&self.tui_journal);
        let pointer = |kind: &str| tui.iter().filter(|l| l.starts_with("begin") && l.contains(&format!("StableCode(\"{kind}\")"))).count();
        assert_eq!((pointer("ui.pointer.action"), pointer("plugin.pointer.action")), (self.clicks, self.clicks), "{tui:#?}");
        let a: Vec<String> = tui
            .iter()
            .map(|l| l.replace("StableCode(\"ui.pointer.action\")", "StableCode(\"ui.action\")").replace("StableCode(\"plugin.pointer.action\")", "StableCode(\"plugin.input\")"))
            .collect();
        let (b, c) = (shape(self.headless.records()), shape(self.bridge.records()));
        assert_eq!(a, b, "Ratatui and Headless record the same actions");
        assert_eq!(b, c, "Headless and the bridge record the same actions");
        a
    }

    /// The question each frontend shows, and the one the bridge sent. The
    /// rows Ratatui measured when it drew the dialog (`lines`, `shown`) are
    /// its own; a renderer measures its own.
    fn same_question(&self) -> radar_core::view::MenuView {
        let unmeasured = |m: radar_core::view::MenuView| radar_core::view::MenuView { lines: 0, shown: 0, ..m };
        let asked = self.headless.host().menu_view().map(unmeasured).expect("a question is open");
        assert!(asked.question, "{asked:?}");
        assert_eq!(self.tui.host().menu_view().map(unmeasured).as_ref(), Some(&asked), "Ratatui asks the same");
        assert_eq!(self.bridge.host().menu_view().map(unmeasured).as_ref(), Some(&asked), "the bridge's host asks the same");
        let sent = self.told.iter().rev().find_map(|m| match m {
            ToRenderer::Question(q) => Some(unmeasured(q.menu.clone())),
            _ => None,
        });
        assert_eq!(sent.as_ref(), Some(&asked), "the renderer was sent the question");
        asked
    }
}

/// The position, among the targets the view offers, of the first in `pane`.
fn first_in(view: &ScreenView, pane: PaneId) -> usize {
    offered(view).iter().position(|o| o.pane == pane).unwrap_or_else(|| panic!("a target in {pane:?}"))
}

fn nth(view: &ScreenView, i: usize) -> Target {
    offered(view).into_iter().nth(i).map(|o| o.target).expect("the target is offered")
}

/// As JSON, as a renderer reads it: with the Work store's database id (a
/// new one per store, so per host) written as zeros, and a tree's open rows
/// (a set; GitLab keeps them in a hash set, so its order is per instance)
/// in order. Everything else must be equal byte for byte.
fn same_store(view: &ScreenView) -> String {
    zero_store(serde_json::to_value(view).unwrap())
}

fn same_targets(view: &ScreenView) -> String {
    zero_store(serde_json::to_value(offered(view)).unwrap())
}

fn sort_open(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                match v {
                    serde_json::Value::Array(rows) if k == "open" => rows.sort_by_key(|r| r.to_string()),
                    _ => sort_open(v),
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(sort_open),
        _ => {}
    }
}

fn zero_store(mut value: serde_json::Value) -> String {
    sort_open(&mut value);
    let text = value.to_string();
    let mut out = String::with_capacity(text.len());
    let mut rest = text.as_str();
    while let Some(at) = rest.find("database_id") {
        let (head, tail) = rest.split_at(at + "database_id".len());
        out.push_str(head);
        // The id follows within a few quotes and colons.
        match tail.char_indices().take(12).find(|(i, _)| {
            tail.get(*i..*i + 36).is_some_and(|c| c.as_bytes()[8] == b'-' && c.as_bytes()[23] == b'-')
        }) {
            Some((i, _)) => {
                out.push_str(&tail[..i]);
                out.push_str("00000000-0000-0000-0000-000000000000");
                rest = &tail[i + 36..];
            }
            None => rest = tail,
        }
    }
    out.push_str(rest);
    out
}

/// Repositories: choose a repository, open its page, come back.
#[test]
fn the_repositories_journey_is_the_same_through_ratatui_headless_and_the_bridge() {
    let mut t = Three::new();
    t.show("repos");
    let view = t.same("repositories");
    let repo = offered(&view)
        .iter()
        .position(|o| o.pane == PaneId::Middle && o.target.0.contains("r:acme/"))
        .unwrap_or_else(|| panic!("a repository row: {:?}", offered(&view)));

    t.click(repo);
    t.focus(PaneId::Middle);
    t.select(repo);
    t.same("a repository selected");

    t.terminal(key(KeyCode::Enter));
    t.activate(repo);
    let page = t.same("its page");
    assert!(page.page.is_some(), "the repository's page opens: {page:?}");

    // The page's second tab, by its target.
    let tab = radar_core::view::TabView::target(1);
    assert!(offered(&page).iter().any(|o| o.target == tab), "the page has tabs: {page:?}");
    t.click_on(&tab);
    t.intent(Intent::Select { target: tab.clone() });
    let tabbed = t.same("the page's second tab");
    assert_eq!(tabbed.page.as_ref().map(|p| p.tab), Some(1));

    t.terminal(key(KeyCode::Esc));
    t.intent(Intent::Key { chord: chord("esc") });
    let back = t.same("back on the list");
    assert!(back.page.is_none(), "esc leaves the page");
    t.same_journal();

    // GitLab's settings: the section is sent there, and only there.
    let gitlab = t.headless.host().plugins.iter().position(|p| p.id() == "gitlab").unwrap();
    let at = t
        .headless
        .host()
        .sections
        .iter()
        .position(|(o, _)| *o == radar_core::host::Owner::Plugin(gitlab))
        .expect("a GitLab section");
    for host in [t.tui.host_mut(), t.headless.host_mut(), t.bridge.host_mut()] {
        host.view = View::Settings;
        host.section = at;
    }
    t.drain();
    let section = t.headless.section().expect("GitLab's section");
    assert_eq!(t.tui.section().as_ref(), Some(&section));
    let sent = t.told.iter().rev().find_map(|m| match m {
        ToRenderer::Snapshot(s) => Some(s.section.clone()),
        _ => None,
    });
    assert_eq!(sent, Some(Some(section)), "the settings view sends its section");
    t.show("repos");
    t.same("repositories again, no section sent");
}

/// Work: open the demo work item, then start a new local task three times.
/// Read only: the question says nothing will be saved; declined, then
/// answered yes, and nothing is saved either way. Edit mode: answered yes,
/// and it is saved. The three frontends show the same question each time
/// and record the same actions with the same causality.
#[test]
fn the_work_journey_asks_the_same_question_and_records_the_same_actions() {
    let mut t = Three::new();
    t.show("work");
    let view = t.same("the Work screen");
    let item = first_in(&view, PaneId::Middle);

    t.click(item);
    t.focus(PaneId::Middle);
    t.select(item);
    t.same("the work item selected");
    t.terminal(key(KeyCode::Enter));
    t.activate(item);
    let opened = t.same("the work item open");
    let rows = |v: &ScreenView| v.pane(PaneId::Middle).map(|p| radar_core::headless::node_targets(&p.body).len());
    let before = rows(&opened);

    // Read only, as RADAR_MODE=read-only sets it for a run.
    t.mode(Some((Mode::ReadOnly, "RADAR_MODE".into())));
    let note = "radar is read only (set by RADAR_MODE): nothing will be sent";
    let asked = t.new_task("declined");
    assert!(serde_json::to_string(&asked).unwrap().contains(note), "the question says so: {asked:?}");
    assert_eq!(t.answer('n', "no"), 0, "declined: nothing started");
    let declined = t.same("read only, declined");
    t.same_journal();
    t.new_task("refused");
    assert_eq!(t.answer('y', "yes"), 0, "read only: the save is not sent");
    let refused = t.same("read only, answered yes");
    t.same_journal();
    assert_eq!((rows(&declined), rows(&refused)), (before, before), "no item was added");

    // Edit mode: the same question without the note, and it is saved.
    t.mode(None);
    let asked = t.new_task("parity");
    assert!(!serde_json::to_string(&asked).unwrap().contains(note), "{asked:?}");
    assert!(t.answer('y', "yes") > 0, "edit mode: the save is sent to the Work store");
    t.same("edit mode, answered yes");

    let journal = t.same_journal();
    // Each title typed is one commit, not a record per key; the question and
    // the save it led to are recorded under the part's dispatch that asked.
    let saved = journal
        .iter()
        .position(|l| l.starts_with("begin") && l.contains("StableCode(\"cache.save\")"))
        .unwrap_or_else(|| panic!("the save is recorded: {journal:#?}"));
    let parent_of = |l: &str| l.split(" under ").nth(1).and_then(|r| r.split(':').next()).map(str::to_string);
    let last_menu = journal[..saved]
        .iter()
        .rev()
        .find(|l| l.starts_with("begin") && l.contains("StableCode(\"ui.menu\")"))
        .expect("the question");
    assert!(parent_of(last_menu).is_some_and(|p| p.starts_with("Some(")), "{journal:#?}");
    assert_eq!(parent_of(&journal[saved]), parent_of(last_menu), "{journal:#?}");
    let commits = journal.iter().filter(|l| l.contains("StableCode(\"input.commit\")")).count();
    assert_eq!(commits, 3, "one commit per title typed: {journal:#?}");
    let answers = journal.iter().filter(|l| l.contains("StableCode(\"ui.menu.action\")")).count();
    assert_eq!(answers, 3, "one record per answer: {journal:#?}");
}
