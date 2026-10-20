//! A part for the IPC tests: one screen whose one action asks a question
//! before it records a write, so a journey has an action, a confirmation
//! and a causal child.

use crate::host::{Host, JournalRecord, View};
use crate::journal::{ActionDescriptor, ActionId, Origin};
use crate::plugin::{Binding, Effect, Menu, Outcome, Pane, Plugin, ScreenSpec};
use crate::view::{self as v, ActionSpec, Chord, Intent};

pub(super) struct Probe {
    /// Not set up yet: the host shows its set-up page.
    pub(super) unready: bool,
    /// Pages opened on its screen.
    pub(super) depth: std::cell::Cell<usize>,
    /// The scroll it keeps for its pane: PgDn pages it.
    pub(super) scroll: crate::ui::Scroll,
}

pub(super) const ASK: &str = "probe.items.ask";
/// An action whose sync changed two rows.
pub(super) const SYNC: &str = "probe.items.sync";
/// An action that opens a page on the screen.
pub(super) const PAGE: &str = "probe.items.page";

impl Plugin for Probe {
    fn id(&self) -> &'static str {
        "probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> crate::compose::Setup {
        match self.unready {
            true => crate::compose::Setup::Needed("Add a tenancy.".into()),
            false => crate::compose::Setup::Ready,
        }
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "items",
            name: "items",
            key: 'I',
        }]
    }
    fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
        vec![Binding::new("p", "prompt first")]
    }
    fn view(&self, scope: &str, _: &v::ViewCx<'_>) -> Option<v::ScreenView> {
        let mut view = v::ScreenView::new(
            scope,
            vec![v::PaneView::new(
                v::PaneId::Middle,
                "items",
                v::Node::Document {
                    lines: vec![v::Text::from("one")],
                    cut: false,
                },
            )],
        );
        view.actions = vec![
            ActionSpec {
                mnemonic_at: None,
                mnemonic_shift: false,                id: ASK.into(),
                label: "prompt first".into(),
                key: Chord::parse("p"),
                heading: false,
                icon: None,
                change: None,
            },
            ActionSpec {
                mnemonic_at: None,
                mnemonic_shift: false,                id: PAGE.into(),
                label: "page".into(),
                key: None,
                heading: false,
                icon: None,
                change: None,
            },
            ActionSpec {
                mnemonic_at: None,
                mnemonic_shift: false,                id: SYNC.into(),
                label: "sync".into(),
                key: None,
                heading: false,
                icon: None,
                change: None,
            },
        ];
        Some(view)
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &Intent) -> Outcome {
        match intent {
            Intent::Context { .. } => Outcome::Handled(vec![Effect::Menu(Menu::confirm("pick", "which?", "this", "that"))]),
            Intent::Invoke { action } if action == ASK => Outcome::Handled(vec![
                Effect::Journal(ActionDescriptor::new(ASK, Origin::Plugin("probe".into()))),
                Effect::Menu(Menu::confirm("ask", "save it?", "save it", "keep it")),
            ]),
            Intent::Invoke { action } if action == PAGE => {
                self.depth.set(self.depth.get() + 1);
                Outcome::Handled(vec![])
            }
            Intent::Key { chord } if chord.key == v::Key::PageDown => {
                if self.scroll.page(true) { Outcome::done() } else { Outcome::Ignored }
            }
            Intent::Invoke { action } if action == SYNC => {
                let mut changes = crate::motion::Changes::default();
                changes.changed.insert("probe:1".into());
                changes.changed.insert("probe:2".into());
                changes.added.insert("probe:3".into());
                Outcome::Handled(vec![Effect::Changed(changes)])
            }
            _ => Outcome::Ignored,
        }
    }
    fn depth(&self, _: &str) -> usize {
        self.depth.get()
    }
    fn scroll(&self, _: &str, pane: Pane) -> Option<&crate::ui::Scroll> {
        (pane == Pane::Middle).then_some(&self.scroll)
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        if menu == "ask" && value == "yes" {
            vec![
                Effect::Journal(ActionDescriptor::new(
                    "probe.items.save",
                    Origin::Plugin("probe".into()),
                )),
                Effect::Notify("saved".into()),
            ]
        } else {
            vec![]
        }
    }
}

/// A host with the probe, on its screen.
pub(super) fn host() -> Host {
    let mut host = Host::new(vec![Box::new(Probe { unready: false, depth: Default::default(), scroll: Default::default() })], 0).unwrap();
    host.view = View::Screen(0);
    host
}

/// The journal's records with their ids numbered in order of appearance,
/// as tests/plugin_boundary.rs reads them: what was recorded, under what,
/// with what outcome.
pub(super) fn shape(records: &[JournalRecord]) -> Vec<String> {
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
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => {
                let parent = parent.as_ref().map(&mut n);
                format!("begin #{} under {parent:?}: {:?}", n(id), descriptor.kind)
            }
            JournalRecord::Outcome { id, outcome, .. } => format!("#{} {outcome:?}", n(id)),
            JournalRecord::Gap { dropped } => format!("gap {dropped}"),
        })
        .collect()
}

/// The same host with the probe not set up yet.
pub(super) fn unready_host() -> Host {
    let mut host = Host::new(vec![Box::new(Probe { unready: true, depth: Default::default(), scroll: Default::default() })], 0).unwrap();
    host.view = View::Screen(0);
    host
}
