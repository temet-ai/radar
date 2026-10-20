//! A paste message from the renderer reaches the host as one `Event::Paste`:
//! the capturing part gets the whole string, an open text field takes it,
//! and with nothing taking text nothing happens.

use super::*;
use crate::host::{Host, View};
use crate::plugin::{Effect, Menu, Outcome, Pane, Plugin, ScreenSpec};
use crate::view::{Chord, Intent};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    capturing: bool,
    pasted: Vec<String>,
}

/// A part with a text field: `n` opens a typed menu, and while `capturing`
/// it records each paste it is given.
struct Scribe(Arc<Mutex<State>>);

impl Plugin for Scribe {
    fn id(&self) -> &'static str {
        "scribe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec { id: "main", name: "notes", key: 'N' }]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        Some(v::ScreenView::new(
            scope,
            vec![v::PaneView::new(v::PaneId::Middle, "notes", v::Node::Document { lines: vec![], cut: false })],
        ))
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &Intent) -> Outcome {
        match intent {
            Intent::Key { chord } if *chord == Chord::char('n') => {
                Outcome::Handled(vec![Effect::Menu(Menu::typed("name", "name the note", ""))])
            }
            _ => Outcome::Ignored,
        }
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().capturing
    }
    fn paste(&mut self, text: &str) -> Vec<Effect> {
        self.0.lock().unwrap().pasted.push(text.to_string());
        vec![]
    }
}

fn ready() -> (Bridge, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut host = Host::new(vec![Box::new(Scribe(state.clone()))], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host.view = View::Screen(0);
    host.focus = Pane::Middle;
    let mut bridge = Bridge::new(host, 80, 24);
    bridge.receive(ToHost::Hello {
        protocol: PROTOCOL,
        view: crate::view::VERSION,
        capabilities: Capabilities {
            columns: 80,
            rows: 24,
            mouse: true,
            true_colour: true,
            image: ImageProtocol::None,
            plain_text: false,
        },
    });
    (bridge, state)
}

fn paste(text: &str) -> ToHost {
    ToHost::Paste { text: text.into() }
}

#[test]
fn a_paste_message_reaches_the_capturing_part_as_one_paste() {
    let (mut bridge, state) = ready();
    state.lock().unwrap().capturing = true;
    bridge.receive(paste("Sam Vimes\nof the Watch"));
    assert_eq!(state.lock().unwrap().pasted, ["Sam Vimes\nof the Watch"]);
}

#[test]
fn a_paste_message_fills_the_open_text_field() {
    let (mut bridge, state) = ready();
    bridge.receive(ToHost::Intent {
        seq: 1,
        cursor: 0,
        intent: Intent::Key { chord: Chord::char('n') },
    });
    assert!(bridge.host().menu.as_ref().is_some_and(|m| m.input.is_some()), "the field is open");
    bridge.receive(paste("my shelf"));
    let value = bridge.host().menu.as_ref().unwrap().input.as_ref().unwrap().value().to_string();
    assert_eq!(value, "my shelf");
    assert!(state.lock().unwrap().pasted.is_empty(), "the field took it, not the part");
}

#[test]
fn a_paste_message_with_nothing_capturing_does_nothing() {
    let (mut bridge, state) = ready();
    let out = bridge.receive(paste("Nobby Nobbs"));
    assert!(out.effects.is_empty() && !out.quit);
    assert!(state.lock().unwrap().pasted.is_empty());
    assert!(bridge.host().menu.is_none());
    // Before the welcome it is ignored too.
    let host = Host::new(vec![Box::new(Scribe(state.clone()))], 0).unwrap();
    let mut cold = Bridge::new(host, 80, 24);
    state.lock().unwrap().capturing = true;
    cold.receive(paste("Carrot"));
    assert!(state.lock().unwrap().pasted.is_empty());
}

#[test]
fn a_paste_longer_than_the_bound_is_cut() {
    let long = "a".repeat(MAX_INPUT + 10);
    match paste(&long).bounded() {
        ToHost::Paste { text } => assert_eq!(text.chars().count(), MAX_INPUT),
        other => panic!("{other:?}"),
    }
}
