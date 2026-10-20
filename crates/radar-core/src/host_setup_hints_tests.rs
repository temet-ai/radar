//! Footer hints follow the same readiness boundary as Host key delivery.

use std::sync::{Arc, Mutex};

use super::*;
use crate::compose::Setup;
use crate::plugin::{Effect, Outcome, Plugin, ScreenSpec, SectionSpec};
use crate::view::{Chord, Intent, Key};

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
        vec![ScreenSpec { id: "main", name: "editor", key: 'E' }]
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec { id: "connection", name: "connection", group: "editor" }]
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().capturing
    }
    fn hints(&self, _: &str, _: Pane) -> Vec<crate::design::help_bar::KeyHint<'static>> {
        vec![crate::design::help_bar::KeyHint::new("ctrl-r", "recover screen")]
    }
    fn section_hints(&self, _: &str) -> Vec<crate::design::help_bar::KeyHint<'static>> {
        vec![crate::design::help_bar::KeyHint::new("ctrl-r", "recover settings")]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        Some(v::ScreenView::new(
            scope,
            vec![v::PaneView::new(v::PaneId::Middle, "editor", v::Node::Document { lines: vec![], cut: false })],
        ))
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
    fn intent(&mut self, scope: &str, _: Pane, intent: &Intent) -> Outcome {
        match intent {
            Intent::Key { chord } if *chord == Chord::ctrl('r') => {
                Outcome::Handled(vec![Effect::Copy(if scope == "main" { "screen" } else { "settings" }.into())])
            }
            _ => Outcome::Ignored,
        }
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

fn ctrl_r(host: &mut Host) -> Vec<HostEffect> {
    host.update(Event::Intent(Intent::Key { chord: Chord::ctrl('r') }))
}

fn hints(host: &Host, width: u16, height: u16) -> Vec<String> {
    host.chrome_view(width, height).status.hints.into_iter().map(|h| format!("{} {}", h.keys, h.label)).collect()
}

fn press_hint(host: &mut Host, chord: Chord) {
    let target = host
        .chrome_view(120, 30)
        .status
        .hints
        .into_iter()
        .flat_map(|h| h.keys_of)
        .find(|k| k.chord == chord)
        .expect("the advertised key has a target")
        .target;
    host.update(Event::Intent(Intent::Activate { target }));
}

#[test]
fn setup_hints_ignore_hidden_capture_and_the_menu_still_opens_settings() {
    for (width, height) in [(80, 24), (160, 40)] {
        let (mut host, state) = fixture();
        let normal = hints(&host, width, height);
        state.lock().unwrap().capturing = true;
        assert!(!host.capturing(), "a part that is not set up does not capture");
        assert_eq!(hints(&host, width, height), normal, "a hidden editor must not change the set-up page");
        assert!(!normal.iter().any(|h| h.contains("recover screen")));
        assert!(ctrl_r(&mut host).is_empty());
        press_hint(&mut host, Chord::char('x'));
        let menu = &host.menu.as_ref().expect("the advertised x opens its menu").menu;
        assert!(menu.items.iter().any(|item| item.label == "set up editor in settings"));
        assert!(!menu.items.iter().any(|item| item.keys == "ctrl-r"));
        press_hint(&mut host, Chord::new(Key::Enter));
        assert_eq!(host.view, View::Settings);
        assert_eq!(host.current_section().unwrap().1.id, "connection");
        assert!(host.capturing());
        assert!(hints(&host, width, height).iter().any(|h| h == "ctrl-r recover settings"));
        assert!(
            ctrl_r(&mut host)
                .iter()
                .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "settings"))
        );
    }
}

#[test]
fn ready_screen_editor_hints_still_match_host_dispatch() {
    let (mut host, state) = fixture();
    *state.lock().unwrap() = State { ready: true, capturing: true };
    for (width, height) in [(80, 24), (160, 40)] {
        assert!(host.capturing());
        assert!(hints(&host, width, height).iter().any(|h| h == "ctrl-r recover screen"));
        assert!(
            ctrl_r(&mut host)
                .iter()
                .any(|effect| matches!(effect, HostEffect::Copy(text) if text == "screen"))
        );
    }
}
