//! Headless and the IPC Bridge give the same view data and journal (#39).
//!
//! One repositories journey with the demo parts: choose a repository, open
//! its page, switch its tab, go back. It runs through `Headless` and through
//! `ipc::Bridge`, the host side of the Ink renderer's protocol, with no
//! terminal and no Node: the renderer's messages are built here and the
//! bridge's replies are read here. At each step both show the same screen
//! view, the renderer was sent that view, and the journal records the same
//! actions under the same causal parents.

#![cfg(all(feature = "gitlab", feature = "demo"))]

use radar_core::Host;
use radar_core::headless::{Headless, offered};
use radar_core::host::{HostEffect, JournalRecord, View};
use radar_core::ipc::{Bridge, Capabilities, ImageProtocol, PROTOCOL, Received, ToHost, ToRenderer};
use radar_core::journal::ActionId;
use radar_core::view::{Chord, Intent, PaneId, ScreenView, TabView, VERSION};

const W: u16 = 160;
const H: u16 = 40;

/// Every compiled-in part with its sample data, as `radar shot` has them.
fn demo() -> Host {
    radar_core::demo::demo_host(radar::parts()).expect("the parts fit together")
}

/// The journal's records with ids numbered in order of appearance: what was
/// recorded, under which parent, with what outcome.
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

/// The effects that send something: a request to a store or a task started.
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

/// The view as JSON, with GitLab's open tree rows sorted: it keeps them in a
/// hash set, so their order differs per instance. Everything else is equal.
fn normal(view: &ScreenView) -> serde_json::Value {
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
    let mut value = serde_json::to_value(view).expect("a view is JSON");
    sort_open(&mut value);
    value
}

struct Pair {
    headless: Headless,
    bridge: Bridge,
    /// Everything the bridge told the renderer, in order.
    told: Vec<ToRenderer>,
    seq: u64,
    /// Work started so far by Headless and by the bridge.
    started: [usize; 2],
}

impl Pair {
    fn new() -> Self {
        let mut bridge = Bridge::new(demo(), 80, 24);
        // The renderer says hello at the size of its terminal.
        bridge.receive(ToHost::Hello {
            protocol: PROTOCOL,
            view: VERSION,
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
        let mut pair = Self {
            headless: Headless::new(demo(), W, H),
            bridge,
            told: Vec::new(),
            seq: 0,
            started: [0; 2],
        };
        pair.drain();
        pair
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

    /// Show the screen with this id, in both hosts.
    fn show(&mut self, id: &str) {
        let at = self
            .headless
            .host()
            .screens
            .iter()
            .position(|(_, s)| s.id == id)
            .unwrap_or_else(|| panic!("the {id} screen"));
        self.headless.show_screen(at);
        self.bridge.host_mut().view = View::Screen(at);
        self.drain();
        self.same(id);
    }

    /// The same intent in both frontends. `intent` maps the frontend's own
    /// view to the intent it sends, so a target names that view's own store.
    fn step(&mut self, step: &str, intent: impl Fn(&ScreenView) -> Intent) -> ScreenView {
        let headless = intent(&self.headless.screen().expect("a screen"));
        self.started[0] += starts(&self.headless.send(headless));
        let bridge = intent(&self.bridge.snapshot_now(0).screen.expect("a screen"));
        self.seq += 1;
        let Received { effects, quit } = self.bridge.receive(ToHost::Intent {
            seq: self.seq,
            cursor: 0,
            intent: bridge,
        });
        assert!(!quit, "{step}: an intent does not end the bridge");
        self.started[1] += starts(&effects);
        self.drain();
        self.same(step)
    }

    /// Both show the same view, the bridge sent the renderer that view, and
    /// both recorded the same journal and started the same work so far.
    fn same(&self, step: &str) -> ScreenView {
        let headless = self.headless.screen().unwrap_or_else(|| panic!("{step}: a screen"));
        let bridge = self
            .bridge
            .snapshot_now(0)
            .screen
            .unwrap_or_else(|| panic!("{step}: the bridge shows a screen"));
        assert_eq!(normal(&headless), normal(&bridge), "{step}: Headless and the bridge show the same view");
        let sent = self.told.iter().rev().find_map(|m| match m {
            ToRenderer::Snapshot(s) => Some(s.screen.clone()),
            _ => None,
        });
        assert_eq!(sent.flatten().as_ref().map(normal), Some(normal(&bridge)), "{step}: the renderer was sent that view");
        assert_eq!(
            shape(self.headless.records()),
            shape(self.bridge.records()),
            "{step}: Headless and the bridge record the same actions under the same parents"
        );
        assert_eq!(self.started[0], self.started[1], "{step}: the same work started");
        headless
    }
}

/// Repositories: choose a repository, open its page, switch its tab, come back.
mod repositories {
use super::*;

#[test]
fn the_journey_is_the_same_through_headless_and_the_bridge() {
    let mut t = Pair::new();
    t.show("repos");

    // A repository row in the middle pane (its target names the repository,
    // not the group above it), as renderer_parity chose it.
    let view = t.same("repositories");
    let repo = offered(&view)
        .into_iter()
        .find(|o| o.pane == PaneId::Middle && o.target.0.contains("r:acme/"))
        .unwrap_or_else(|| panic!("a repository row: {:?}", offered(&view)))
        .target;

    t.step("a repository selected", |_| Intent::Select { target: repo.clone() });
    let page = t.step("its page opens", |_| Intent::Activate { target: repo.clone() });
    assert!(page.page.is_some(), "the repository's page opens: {page:?}");

    // The page's second tab, by its target.
    let tab = TabView::target(1);
    assert!(offered(&page).iter().any(|o| o.target == tab), "the page has tabs: {page:?}");
    let tabbed = t.step("the page's second tab", |_| Intent::Select { target: tab.clone() });
    assert_eq!(tabbed.page.as_ref().map(|p| p.tab), Some(1));

    let back = t.step("back on the list", |_| Intent::Key { chord: chord("esc") });
    assert!(back.page.is_none(), "esc leaves the page");
    assert!(!shape(t.headless.records()).is_empty(), "the journey records its actions");
}
}
