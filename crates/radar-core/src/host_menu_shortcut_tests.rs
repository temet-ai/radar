//! Modifier bindings must remain discoverable without borrowing a plain key.

use super::*;
use crate::plugin::{Outcome, parse_chord};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    keys: Vec<Chord>,
    typing: bool,
}

struct Recovery(Arc<Mutex<State>>);

impl Plugin for Recovery {
    fn id(&self) -> &'static str {
        "recovery"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "items",
            name: "items",
            key: 'I',
        }]
    }
    fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
        vec![
            Binding::heading("selected item"),
            Binding::new("ctrl-r", "kept text…"),
            Binding::new("r", "refresh now"),
            Binding::new("e", "edit text…"),
            Binding::new("Y", "copy item key"),
        ]
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().typing
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        let mut view = v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "items", v::Node::Document { lines: vec![], cut: false },
        )]);
        view.actions = self.keys(scope, Pane::Middle).iter()
            .map(|b| v::ActionSpec::named(format!("items.{scope}.{}", b.keys), b)).collect();
        Some(view)
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
        let crate::view::Intent::Key { chord } = intent else { return Outcome::Ignored };
        let key = *chord;
        let mut state = self.0.lock().unwrap();
        state.keys.push(key);
        if key.key == Key::Esc {
            state.typing = false;
        }
        if key.key == Key::Char('e') {
            state.typing = true;
        }
        Outcome::done()
    }
}

fn fixture() -> (Host, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut host = Host::new(vec![Box::new(Recovery(state.clone()))], 0).unwrap();
    host.load_config(Some("")).unwrap();
    (host, state)
}

fn press(host: &mut Host, keys: &str) {
    host.update(crate::host::chord_event(parse_chord(keys).unwrap()));
}

#[test]
fn control_bindings_parse_as_one_complete_key() {
    assert_eq!(
        parse_chord("ctrl-r"),
        Some(Chord::ctrl('r'))
    );
    assert_eq!(
        parse_chord("r"),
        Some(Chord::new(Key::Char('r')))
    );
    for invalid in ["ctrl-", "ctrl-rr", "ctrl-ctrl-r", "is:review"] {
        assert_eq!(parse_chord(invalid), None, "{invalid}");
    }
}

#[test]
fn recovery_menu_row_is_visible_and_enter_keeps_its_modifier() {
    let (mut host, state) = fixture();
    press(&mut host, "x");
    for (w, h) in [(80, 24), (160, 40)] {
        let screen = crate::headless::text_of(&host, w, h);
        assert!(
            screen.contains("ctrl-r") && screen.contains("kept text"),
            "{screen}"
        );
    }
    assert_eq!(host.menu.as_ref().unwrap().menu.items[0].key, None);
    press(&mut host, "enter");
    assert!(host.menu.is_none());
    assert_eq!(
        state.lock().unwrap().keys,
        vec![parse_chord("ctrl-r").unwrap()]
    );
}

#[test]
fn menu_accelerators_distinguish_control_from_plain_refresh() {
    let (mut host, state) = fixture();
    for action in ["r", "ctrl-r", "r"] {
        press(&mut host, "x");
        press(&mut host, action);
        assert!(host.menu.is_none());
    }
    assert_eq!(
        state.lock().unwrap().keys,
        ["r", "ctrl-r", "r"].map(|key| parse_chord(key).unwrap())
    );
}

#[test]
fn combined_modifiers_do_not_borrow_bindings_and_shifted_letters_still_work() {
    let (mut host, state) = fixture();
    press(&mut host, "x");
    host.update(crate::host::chord_event(Chord { alt: true, ..Chord::ctrl('r') }));
    assert!(host.menu.is_some());
    assert!(state.lock().unwrap().keys.is_empty());
    host.update(crate::host::chord_event(Chord::char('Y')));
    assert!(host.menu.is_none());
    assert_eq!(state.lock().unwrap().keys, vec![parse_chord("Y").unwrap()]);
}

#[test]
fn dismissing_reopening_and_activating_recovery_dispatches_only_once() {
    let (mut host, state) = fixture();
    press(&mut host, "x");
    press(&mut host, "esc");
    assert!(state.lock().unwrap().keys.is_empty());
    press(&mut host, "x");
    let activate = |host: &mut Host| {
        host.update(Event::Intent(crate::view::Intent::Activate {
            target: crate::view::chrome::targets::menu_item(0),
        }));
    };
    activate(&mut host);
    assert!(host.menu.is_none());
    // The closed popup's row is not a second activation.
    activate(&mut host);
    assert_eq!(
        state.lock().unwrap().keys,
        vec![parse_chord("ctrl-r").unwrap()]
    );
}

#[test]
fn control_shortcuts_do_not_alias_confirmation_choices_or_escape_the_editor() {
    let (mut host, state) = fixture();
    host.menu = Some(OpenMenu::open(
        Owner::Plugin(0),
        Menu::confirm("ask", "change it?", "change it", "leave it"),
    ));
    press(&mut host, "ctrl-y");
    assert!(host.menu.is_some(), "Ctrl-Y is not confirmation by Y");
    host.update(crate::host::chord_event(Chord { alt: true, ..Chord::char('y') }));
    assert!(host.menu.is_some(), "Alt-Y is not plain Y confirmation");
    press(&mut host, "ctrl-q");
    assert!(host.menu.is_some(), "Ctrl-Q is not plain Q dismissal");
    press(&mut host, "esc");
    press(&mut host, "e");
    press(&mut host, "x");
    press(&mut host, "ctrl-r");
    assert!(host.menu.is_none(), "editor keeps its shortcuts");
    assert!(state.lock().unwrap().typing);
    press(&mut host, "esc");
    press(&mut host, "x");
    assert!(host.menu.is_some(), "menu works again after editing ends");
}

#[test]
fn repeated_page_tab_clicks_never_synthesize_enter() {
    let (mut host, state) = fixture();
    for _ in 0..3 {
        host.update(Event::Intent(crate::view::Intent::Activate {
            target: crate::view::Target::new("tab:1"),
        }));
    }
    assert!(state.lock().unwrap().keys.is_empty());
}
