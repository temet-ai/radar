//! Opted-in menu keys keep their original target without freezing other actions.

use super::*;
use crate::plugin::parse_chord;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    target: Option<String>,
    keys: Vec<Chord>,
    answers: Vec<String>,
    typing: bool,
}

struct Probe(Arc<Mutex<State>>);

impl Plugin for Probe {
    fn id(&self) -> &'static str {
        "context-probe"
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
            Binding::new("y", "copy it"),
            Binding::new("enter", "open it"),
            Binding::new("ctrl-r", "recover it"),
            Binding::new("r", "refresh"),
            Binding::new("p", "ask first"),
            Binding::new("e", "edit"),
        ]
    }
    fn menu_key_context(&self, _: &str, _: Pane, action: &str) -> Option<String> {
        ["y", "enter", "ctrl-r"]
            .contains(&action)
            .then(|| self.0.lock().unwrap().target.clone())
            .flatten()
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        self.0.lock().unwrap().target = *msg.downcast::<Option<String>>().unwrap();
        vec![]
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
        let crate::view::Intent::Key { chord } = intent else {
            if let crate::view::Intent::Context { target } = intent {
                return self.select_context(&target.0);
            }
            return Outcome::Ignored;
        };
        let key = *chord;
        let mut state = self.0.lock().unwrap();
        state.keys.push(key);
        if state.typing {
            if key.key == Key::Esc {
                state.typing = false;
            }
            return Outcome::done();
        }
        match key.key {
            Key::Char('y') | Key::Enter => {
                Outcome::Handled(state.target.clone().into_iter().map(Effect::Copy).collect())
            }
            Key::Char('p') => Outcome::Handled(vec![Effect::Menu(Menu::confirm(
                "captured",
                "keep the original?",
                "keep",
                "leave",
            ))]),
            Key::Char('e') => {
                state.typing = true;
                Outcome::done()
            }
            _ => Outcome::done(),
        }
    }
    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.0
            .lock()
            .unwrap()
            .answers
            .push(format!("{menu}:{value}"));
        vec![]
    }
}

impl Probe {
    fn select_context(&mut self, id: &str) -> Outcome {
        if matches!(id, "row:a" | "row:b") {
            self.0.lock().unwrap().target = Some(format!("PRIVATE_TARGET_{}", id.trim_start_matches("row:").to_uppercase()));
            Outcome::done()
        } else {
            Outcome::Ignored
        }
    }
}

fn fixture() -> (Host, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        target: Some("PRIVATE_TARGET_A".into()),
        ..State::default()
    }));
    let mut host = Host::new(vec![Box::new(Probe(state.clone()))], 0).unwrap();
    host.load_config(Some("")).unwrap();
    (host, state)
}

fn press(host: &mut Host, key: &str) -> Vec<HostEffect> {
    host.update(crate::host::chord_event(parse_chord(key).unwrap()))
}

fn change(host: &mut Host, target: Option<&str>) {
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(target.map(str::to_owned)),
    });
}

fn context(host: &mut Host, id: &str) -> Vec<HostEffect> {
    host.update(Event::Intent(crate::view::Intent::Context { target: crate::view::Target::new(id) }))
}

#[test]
fn right_click_selects_its_row_without_activating_a_click_action() {
    let (mut host, state) = fixture();
    assert!(context(&mut host, "row:b").is_empty());
    assert_eq!(host.focus, Pane::Middle);
    assert_eq!(state.lock().unwrap().target.as_deref(), Some("PRIVATE_TARGET_B"));
    assert_eq!(host.menu.as_ref().unwrap().menu.id, KEYS_MENU);
    assert!(matches!(&press(&mut host, "y")[0], HostEffect::Copy(s) if s == "PRIVATE_TARGET_B"));
    assert_eq!(state.lock().unwrap().keys.len(), 1);
}

#[test]
fn second_right_click_replaces_context_menu_with_clicked_rows_actions() {
    let (mut host, state) = fixture();
    context(&mut host, "row:b");
    assert_eq!(state.lock().unwrap().target.as_deref(), Some("PRIVATE_TARGET_B"));
    context(&mut host, "row:a");
    assert_eq!(state.lock().unwrap().target.as_deref(), Some("PRIVATE_TARGET_A"));
    assert!(matches!(&press(&mut host, "y")[0], HostEffect::Copy(s) if s == "PRIVATE_TARGET_A"));
    assert_eq!(state.lock().unwrap().keys.len(), 1);
}

#[test]
fn right_clicking_a_non_context_cell_dismisses_the_previous_context_menu() {
    let (mut host, state) = fixture();
    context(&mut host, "row:b");
    context(&mut host, "elsewhere");
    assert!(host.menu.is_none());
    assert_eq!(state.lock().unwrap().target.as_deref(), Some("PRIVATE_TARGET_B"));
    assert!(state.lock().unwrap().keys.is_empty());
}

#[test]
fn right_click_on_an_action_cell_does_not_offer_another_rows_actions() {
    let (mut host, state) = fixture();
    assert!(context(&mut host, "open:b").is_empty());
    assert!(host.menu.is_none());
    assert_eq!(state.lock().unwrap().target.as_deref(), Some("PRIVATE_TARGET_A"));
}

#[test]
fn changed_or_missing_context_refuses_accelerators_enter_and_modified_keys() {
    for target in [Some("PRIVATE_TARGET_B"), None] {
        for key in ["y", "enter", "ctrl-r"] {
            let (mut host, state) = fixture();
            press(&mut host, "x");
            change(&mut host, target);
            let effects = press(&mut host, key);
            assert!(effects.is_empty());
            assert!(state.lock().unwrap().keys.is_empty());
            assert!(host.menu.is_none());
            assert!(host.message.as_deref().unwrap().contains("press x"));
            assert!(!host.message.as_deref().unwrap().contains("PRIVATE"));
        }
    }
}

#[test]
fn unchanged_context_and_reopening_work_but_dismissal_never_dispatches() {
    let (mut host, state) = fixture();
    press(&mut host, "x");
    change(&mut host, Some("PRIVATE_TARGET_A"));
    assert!(matches!(&press(&mut host, "y")[0], HostEffect::Copy(s) if s == "PRIVATE_TARGET_A"));
    press(&mut host, "x");
    press(&mut host, "esc");
    change(&mut host, Some("PRIVATE_TARGET_B"));
    press(&mut host, "x");
    assert!(matches!(&press(&mut host, "y")[0], HostEffect::Copy(s) if s == "PRIVATE_TARGET_B"));
    assert_eq!(state.lock().unwrap().keys.len(), 2);
}

#[test]
fn opted_in_key_cannot_replay_into_a_changed_host_route_or_editor() {
    for changed in ["view", "focus", "bar", "tabs", "typing"] {
        let (mut host, state) = fixture();
        press(&mut host, "x");
        match changed {
            "view" => host.view = View::Settings,
            "focus" => host.focus = Pane::Detail,
            "bar" => host.bar = true,
            "tabs" => host.tabs = true,
            "typing" => state.lock().unwrap().typing = true,
            _ => unreachable!(),
        }
        assert!(press(&mut host, "y").is_empty(), "{changed}");
        assert!(state.lock().unwrap().keys.is_empty(), "{changed}");
        assert!(host.menu.is_none());
    }
}

#[test]
fn unguarded_refresh_direct_keys_and_plugin_prompts_keep_their_own_semantics() {
    let (mut host, state) = fixture();
    press(&mut host, "x");
    change(&mut host, Some("PRIVATE_TARGET_B"));
    press(&mut host, "r");
    assert_eq!(state.lock().unwrap().keys, [parse_chord("r").unwrap()]);
    assert!(matches!(&press(&mut host, "y")[0], HostEffect::Copy(s) if s == "PRIVATE_TARGET_B"));
    press(&mut host, "p");
    change(&mut host, None);
    press(&mut host, "y");
    assert_eq!(state.lock().unwrap().answers, ["captured:yes"]);
    press(&mut host, "e");
    press(&mut host, "x");
    press(&mut host, "ctrl-r");
    assert!(host.menu.is_none());
    assert!(state.lock().unwrap().typing);
}

#[test]
fn rejected_menu_target_is_blocked_without_confirmed_history_or_private_tokens() {
    let (mut host, _) = fixture();
    host.enable_journal();
    let opened = press(&mut host, "x");
    let menu = opened
        .iter()
        .find_map(|e| match e {
            HostEffect::Journal(JournalRecord::Begin { id, descriptor, .. })
                if descriptor.kind.as_str() == "ui.menu" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert!(!format!("{:?}", host.menu_keys).contains("PRIVATE"));
    change(&mut host, Some("PRIVATE_TARGET_B"));
    let effects = press(&mut host, "y");
    let outcomes: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            HostEffect::Journal(JournalRecord::Outcome { id, outcome, .. }) if id == &menu => {
                Some(*outcome)
            }
            _ => None,
        })
        .collect();
    assert_eq!(outcomes, [JournalOutcome::Blocked]);
    assert!(!format!("{effects:?}").contains("PRIVATE"));
    assert!(!effects.iter().any(|e| matches!(
        e,
        HostEffect::Journal(JournalRecord::Outcome {
            outcome: JournalOutcome::Confirmed,
            ..
        })
    )));
}

#[test]
fn a_row_intent_while_a_field_has_the_keys_waits_for_it_as_a_click_does() {
    use crate::view::{Chord, Intent, Key, Target};
    let (mut host, state) = fixture();
    state.lock().unwrap().typing = true;
    let row = Target::new("row:b");
    for intent in [
        Intent::Select { target: row.clone() },
        Intent::Context { target: row.clone() },
        Intent::Activate { target: row.clone() },
        Intent::Fold { target: row.clone(), open: true },
    ] {
        assert!(host.update(Event::Intent(intent.clone())).is_empty(), "{intent:?}");
        assert_eq!(state.lock().unwrap().target.as_deref(), Some("PRIVATE_TARGET_A"), "{intent:?}");
        assert!(host.menu.is_none(), "{intent:?}");
        assert!(state.lock().unwrap().typing, "{intent:?} leaves the field open");
    }
    assert!(state.lock().unwrap().keys.is_empty());
    // The field's own keys still reach it, and once it is done the same
    // press lands on its row.
    host.update(Event::Intent(Intent::Key { chord: Chord::new(Key::Esc) }));
    assert!(!state.lock().unwrap().typing);
    host.update(Event::Intent(Intent::Context { target: row }));
    assert_eq!(state.lock().unwrap().target.as_deref(), Some("PRIVATE_TARGET_B"));
    assert_eq!(host.menu.as_ref().unwrap().menu.id, KEYS_MENU);
}
