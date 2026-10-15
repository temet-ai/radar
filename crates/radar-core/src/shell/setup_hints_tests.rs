//! Footer hints follow the same readiness boundary as Host key delivery.

use std::sync::{Arc, Mutex};

use super::*;
use crate::compose::Setup;
use crate::host::{Event, HostEffect, MouseKind};
use crate::plugin::{Effect, Outcome, Plugin, ScreenSpec, SectionSpec};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Default)]
struct State {
    ready: bool,
    capturing: bool,
}

struct Editor(Arc<Mutex<State>>);

impl Plugin for Editor {
    fn id(&self) -> &'static str {
        "editor"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn set_up(&self) -> Setup {
        if self.0.lock().unwrap().ready {
            Setup::Ready
        } else {
            Setup::Needed("Connect the editor.".into())
        }
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "main",
            name: "editor",
            key: 'E',
        }]
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec {
            id: "connection",
            name: "connection",
            group: "editor",
        }]
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().capturing
    }
    fn hints(&self, _: &str, _: Pane) -> Vec<KeyHint<'static>> {
        vec![KeyHint::new("ctrl-r", "recover screen")]
    }
    fn section_hints(&self, _: &str) -> Vec<KeyHint<'static>> {
        vec![KeyHint::new("ctrl-r", "recover settings")]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        Some(v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "editor", v::Node::Document { lines: vec![], cut: false },
        )]))
    }
    fn section_view(&self, section: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::SectionView> {
        Some(crate::view::SectionView {
            version: crate::view::VERSION,
            scope: section.into(),
            body: crate::view::Node::Document { lines: vec![], cut: false },
            detail_title: "about".into(),
            detail: vec![],
            detail_blocks: vec![],
            actions: vec![],
        })
    }
    fn intent(&mut self, scope: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
        if let crate::view::Intent::Key { chord } = intent {
            recover(crate::view_tui::key_of(*chord), if scope == "main" { "screen" } else { "settings" })
        } else {
            Outcome::Ignored
        }
    }
}

fn recover(key: KeyEvent, scope: &str) -> Outcome {
    if key == KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL) {
        Outcome::Handled(vec![Effect::Copy(scope.into())])
    } else {
        Outcome::Ignored
    }
}

fn fixture() -> (Host, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State::default()));
    let mut host = Host::new(vec![Box::new(Editor(state.clone()))], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host.view = View::Screen(0);
    host.focus = Pane::Middle;
    (host, state)
}

fn press(host: &mut Host, code: KeyCode, modifiers: KeyModifiers) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::new(code, modifiers)))
}

fn click_hint(host: &mut Host, code: KeyCode, at_ms: u64) {
    let area = host
        .hits
        .borrow()
        .iter()
        .find_map(|(area, hit)| {
            matches!(hit, Hit::Key(key, _) if key.code == code).then_some(*area)
        })
        .expect("the advertised key has a click target");
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: area.x,
        row: area.y,
        at_ms,
    });
}

#[test]
fn setup_hints_ignore_hidden_capture_and_the_menu_still_opens_settings() {
    for (width, height) in [(80, 24), (160, 40)] {
        let (mut host, state) = fixture();
        let normal = to_text(&host, width, height);
        state.lock().unwrap().capturing = true;
        assert!(!host.capturing());
        assert_eq!(
            to_text(&host, width, height),
            normal,
            "a hidden editor must not change the healthy set-up page"
        );
        assert!(!normal.contains("recover screen"));
        assert!(press(&mut host, KeyCode::Char('r'), KeyModifiers::CONTROL).is_empty());
        to_text(&host, width, height);
        click_hint(&mut host, KeyCode::Char('x'), 0);
        let menu = &host
            .menu
            .as_ref()
            .expect("the advertised x opens its menu")
            .menu;
        assert!(
            menu.items
                .iter()
                .any(|item| item.label == "set up editor in settings")
        );
        assert!(!menu.items.iter().any(|item| item.keys == "ctrl-r"));
        to_text(&host, width, height);
        click_hint(&mut host, KeyCode::Enter, 1000);
        assert_eq!(host.view, View::Settings);
        assert_eq!(host.current_section().unwrap().1.id, "connection");
        assert!(host.capturing());
        let screen = to_text(&host, width, height);
        assert!(
            screen
                .lines()
                .last()
                .unwrap()
                .contains("ctrl-r recover settings")
        );
        assert!(
            press(&mut host, KeyCode::Char('r'), KeyModifiers::CONTROL)
                .iter()
                .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "settings"))
        );
    }
}

#[test]
fn ready_screen_editor_hints_still_match_host_dispatch() {
    let (mut host, state) = fixture();
    *state.lock().unwrap() = State {
        ready: true,
        capturing: true,
    };
    for (width, height) in [(80, 24), (160, 40)] {
        assert!(host.capturing());
        let screen = to_text(&host, width, height);
        assert!(
            screen
                .lines()
                .last()
                .unwrap()
                .contains("ctrl-r recover screen")
        );
        assert!(
            press(&mut host, KeyCode::Char('r'), KeyModifiers::CONTROL)
                .iter()
                .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "screen"))
        );
    }
}
