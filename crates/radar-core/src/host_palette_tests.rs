//! A few real host contracts; matching itself stays in pure unit tests.
use super::*;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    targets: Vec<Subject>,
    shown: Vec<Subject>,
    actions: usize,
    answers: Vec<String>,
    typing: bool,
    page: bool,
    token: String,
    identity_scope: String,
}
struct Part {
    id: &'static str,
    key: char,
    state: Arc<Mutex<State>>,
}
impl Plugin for Part {
    fn id(&self) -> &'static str {
        self.id
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
            name: self.id,
            key: self.key,
        }]
    }
    fn panes(&self, _: &str) -> Vec<Pane> {
        if self.state.lock().unwrap().page {
            vec![Pane::Middle]
        } else {
            vec![Pane::Nav, Pane::Middle, Pane::Detail]
        }
    }
    fn owns(&self) -> Vec<crate::Owned> {
        vec![crate::Owned::page(self.id, self.id)]
    }
    fn palette_targets(&self) -> Vec<crate::palette::PaletteTarget> {
        let state = self.state.lock().unwrap();
        state
            .targets
            .iter()
            .cloned()
            .map(|subject| {
                let scope = format!(
                    "{}/{}",
                    subject.field("site").unwrap_or(""),
                    state.identity_scope
                );
                crate::palette::PaletteTarget::scoped(subject, scope)
            })
            .collect()
    }
    fn show(&mut self, subject: &Subject) -> Showing {
        self.state.lock().unwrap().shown.push(subject.clone());
        Showing::Page {
            screen: "items",
            effects: vec![],
        }
    }
    fn capturing(&self) -> bool {
        self.state.lock().unwrap().typing
    }
    fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
        vec![
            Binding::new("y", "copy selected item"),
            Binding::new("p", "ask before changing"),
        ]
    }
    fn menu_key_context(&self, _: &str, _: Pane, _: &str) -> Option<String> {
        Some(self.state.lock().unwrap().token.clone())
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        let mut view = v::ScreenView::new(
            scope,
            vec![v::PaneView::new(
                v::PaneId::Middle,
                "items",
                v::Node::Document {
                    lines: vec![],
                    cut: false,
                },
            )],
        );
        view.actions = self
            .keys(scope, Pane::Middle)
            .iter()
            .map(|binding| v::ActionSpec::named(format!("items.{}", binding.keys), binding))
            .collect();
        Some(view)
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
        if let crate::view::Intent::Key { chord } = intent {
            match chord.key {
                crate::view::Key::Char('y') => self.state.lock().unwrap().actions += 1,
                crate::view::Key::Char('p') if !chord.ctrl => {
                    return Outcome::Handled(vec![Effect::Menu(Menu::confirm(
                        "change",
                        "Change the selected item?",
                        "change",
                        "leave",
                    ))]);
                }
                _ => {}
            }
            Outcome::done()
        } else {
            Outcome::Ignored
        }
    }
    fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
        vec![Effect::Menu(*msg.downcast::<Menu>().expect("test menu"))]
    }
    fn on_menu(&mut self, _: &str, value: &str) -> Vec<Effect> {
        self.state.lock().unwrap().answers.push(value.into());
        vec![]
    }
}

fn fixture() -> (Host, Arc<Mutex<State>>, Arc<Mutex<State>>) {
    let watch = Arc::new(Mutex::new(State {
        targets: vec![Subject::new("watch", "1", "Watch report").with("site", "city")],
        token: "watch-1".into(),
        ..State::default()
    }));
    let guild = Arc::new(Mutex::new(State {
        targets: vec![Subject::new("guild", "2", "Guild ledger")],
        token: "guild-2".into(),
        ..State::default()
    }));
    let host = Host::new(
        vec![
            Box::new(Part {
                id: "watch",
                key: 'W',
                state: watch.clone(),
            }),
            Box::new(Part {
                id: "guild",
                key: 'G',
                state: guild.clone(),
            }),
        ],
        0,
    )
    .unwrap();
    (host, watch, guild)
}
fn key(host: &mut Host, code: Key) {
    host.update(crate::host::chord_event(Chord::new(code)));
}
fn open(host: &mut Host) {
    host.update(crate::host::chord_event(Chord::ctrl('p')));
}
fn query(host: &mut Host, text: &str) {
    host.update(crate::host::chord_event(Chord::ctrl('u')));
    host.update(Event::Paste(text.into()));
}

#[test]
fn palette_search_is_inert_arrows_preserve_query_and_enter_routes_once() {
    let (mut host, watch, guild) = fixture();
    let before = (host.view, host.focus, host.bar, host.sources, host.tabs);
    open(&mut host);
    query(&mut host, "guild");
    assert!(guild.lock().unwrap().shown.is_empty());
    assert_eq!(
        (host.view, host.focus, host.bar, host.sources, host.tabs),
        before
    );
    let view = host.menu_view().unwrap();
    assert_eq!(view.typed.as_deref(), Some("guild"));
    assert_eq!(view.palette.as_ref().unwrap().matched, 2);
    assert_eq!(
        view.palette.as_ref().unwrap().highlights[0],
        vec![0, 1, 2, 3, 4]
    );
    key(&mut host, Key::Down);
    key(&mut host, Key::Up);
    assert_eq!(host.menu_view().unwrap().typed.as_deref(), Some("guild"));
    key(&mut host, Key::Enter);
    assert!(host.menu.is_none());
    assert_eq!(host.view, View::Screen(1));
    assert_eq!(guild.lock().unwrap().shown.len(), 1);
    assert!(watch.lock().unwrap().shown.is_empty());
    assert_eq!(host.back.len(), 1);

    // Provider-only matches still light the visible reason for the match.
    guild.lock().unwrap().targets[0].label = "Ledger".into();
    open(&mut host);
    query(&mut host, "guild");
    let view = host.menu_view().unwrap();
    let row = view
        .items
        .iter()
        .position(|item| item.label == "Ledger")
        .unwrap();
    let palette = view.palette.unwrap();
    assert!(palette.highlights[row].is_empty());
    assert_eq!(palette.hint_highlights[row], vec![0, 1, 2, 3, 4]);
}

#[test]
fn palette_cancel_empty_results_and_capture_preserve_underlying_state() {
    let (mut host, watch, _) = fixture();
    for (view, bar, sources, tabs) in [
        (View::Screen(0), true, false, false),
        (View::Screen(1), false, true, false),
        (View::Settings, false, false, true),
    ] {
        host.view = view;
        host.bar = bar;
        host.sources = sources;
        host.tabs = tabs;
        let before = (host.view, host.focus, host.bar, host.sources, host.tabs);
        open(&mut host);
        query(&mut host, "no such cached item");
        assert!(host.menu_view().unwrap().items.is_empty());
        key(&mut host, Key::Enter);
        assert!(host.menu.is_some());
        key(&mut host, Key::Esc);
        assert!(host.palette.is_none());
        assert_eq!(
            (host.view, host.focus, host.bar, host.sources, host.tabs),
            before
        );
    }
    host.view = View::Screen(0);
    host.bar = false;
    host.sources = false;
    host.tabs = false;
    watch.lock().unwrap().typing = true;
    open(&mut host);
    assert!(host.menu.is_none());
    assert!(watch.lock().unwrap().typing);
    watch.lock().unwrap().typing = false;
    host.menu = Some(OpenMenu::open(
        Owner::Host,
        Menu::confirm("keep", "Keep?", "keep", "leave"),
    ));
    open(&mut host);
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "keep");
}

#[test]
fn palette_actions_keep_freshness_and_confirmation_for_keyboard_and_intents() {
    let (mut host, watch, _) = fixture();
    open(&mut host);
    query(&mut host, "copy selected");
    let value = host.menu_view().unwrap().items[0].value.clone();
    host.update(Event::Intent(crate::view::Intent::Invoke { action: value }));
    assert_eq!(watch.lock().unwrap().actions, 1);
    open(&mut host);
    query(&mut host, "copy selected");
    watch.lock().unwrap().token = "watch-2".into();
    key(&mut host, Key::Enter);
    assert_eq!(watch.lock().unwrap().actions, 1);
    assert!(
        host.message
            .as_ref()
            .unwrap()
            .text
            .contains("target changed")
    );
    open(&mut host);
    query(&mut host, "ask before");
    let target = host.menu_view().unwrap().items[0].target.clone().unwrap();
    host.update(Event::Intent(crate::view::Intent::Activate { target }));
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "change");
    assert!(watch.lock().unwrap().answers.is_empty());
    key(&mut host, Key::Char('y'));
    assert_eq!(watch.lock().unwrap().answers, ["yes"]);
}

#[test]
fn palette_revalidates_scoped_destinations_without_browser_fallback() {
    let (mut host, watch, _) = fixture();
    let mut second = watch.lock().unwrap().targets[0].clone();
    second.fields.insert("site".into(), "country".into());
    watch.lock().unwrap().targets.push(second);
    open(&mut host);
    query(&mut host, "Watch report");
    assert_eq!(
        host.menu_view().unwrap().items.len(),
        2,
        "same subject id at different sites stays distinct"
    );
    watch.lock().unwrap().targets.remove(0);
    let effects = host.update(crate::host::chord_event(Chord::new(Key::Enter)));
    assert!(watch.lock().unwrap().shown.is_empty());
    assert!(
        effects.is_empty(),
        "stale target cannot launch an external link"
    );
    assert!(
        host.message
            .as_ref()
            .unwrap()
            .text
            .contains("destination changed")
    );
}

#[test]
fn palette_discovery_prioritizes_actions_then_session_visits_and_bounds_results() {
    let (mut host, watch, _) = fixture();
    host.show(Subject::new("guild", "2", "Guild ledger"), 0);
    host.view = View::Screen(0);
    open(&mut host);
    let view = host.menu_view().unwrap();
    assert_eq!(view.items[0].label, "copy selected item");
    let places: Vec<_> = view
        .items
        .iter()
        .filter(|item| item.hint == "watch" || item.hint == "guild")
        .collect();
    assert_eq!(places[0].label, "Guild ledger");
    key(&mut host, Key::Esc);
    watch.lock().unwrap().targets = (0..300)
        .map(|n| Subject::new("watch", n.to_string(), format!("report {n}")))
        .collect();
    open(&mut host);
    query(&mut host, "report");
    let view = host.menu_view().unwrap();
    assert_eq!(view.items.len(), crate::view::chrome::MOST);
    assert_eq!(view.palette.unwrap().matched, 300);
    key(&mut host, Key::Up);
    assert_eq!(
        host.menu.as_ref().unwrap().row,
        crate::view::chrome::MOST - 1
    );
}

#[test]
fn palette_suspends_history_and_closing_it_restores_it() {
    let (mut host, _, _) = fixture();
    host.open_recent_history();
    let id = host.history_request;
    open(&mut host);
    assert!(host.history.is_none());
    assert!(host.palette.as_ref().unwrap().history.is_some());
    host.update(Event::History {
        id,
        result: Err(crate::history::ReadError::Unavailable),
    });
    key(&mut host, Key::Esc);
    assert!(host.history.is_some());
    assert!(host.history.as_ref().unwrap().view(80, 24).error.is_some());
    open(&mut host);
    key(&mut host, Key::Esc);
    assert!(host.history.is_some());
    assert!(host.palette.is_none());
    assert!(host.menu.is_none());
}

#[test]
fn background_question_replaces_palette_without_rewriting_its_choices() {
    let (mut host, watch, _) = fixture();
    open(&mut host);
    query(&mut host, "guild");
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Menu::confirm(
            "background",
            "Keep the change?",
            "keep",
            "leave",
        )),
    });
    assert!(host.palette.is_none());
    let menu = host.menu_view().unwrap();
    assert_eq!(menu.id, "background");
    assert!(menu.question);
    assert!(menu.palette.is_none());
    assert_eq!(
        menu.items
            .iter()
            .filter(|item| !item.heading && !item.note)
            .map(|item| item.value.as_str())
            .collect::<Vec<_>>(),
        ["yes", "no"]
    );
    key(&mut host, Key::Char('y'));
    assert_eq!(watch.lock().unwrap().answers, ["yes"]);
}

#[test]
fn palette_respects_setup_and_visit_identity_ignores_mutable_metadata() {
    let (mut host, watch, _) = fixture();
    host.open_setup(None).unwrap();
    open(&mut host);
    assert!(host.menu.is_none());
    assert!(host.setup.is_some());
    host.setup = None;
    let mut shown = watch.lock().unwrap().targets[0].clone();
    shown.url = Some("https://watch.example.test/report/1".into());
    host.show(shown, 0);
    let before = host.palette_visits.clone();
    watch.lock().unwrap().targets[0].label = "Updated Watch report".into();
    watch.lock().unwrap().targets[0]
        .fields
        .insert("status".into(), "ready".into());
    let shown = watch.lock().unwrap().targets[0].clone();
    host.show(shown, 0);
    let keys: Vec<_> = host
        .palette_visits
        .iter()
        .filter(|(key, _)| matches!(key, palette::Place::Subject(..)))
        .collect();
    assert_eq!(keys.len(), 1);
    assert_eq!(*keys[0].1, 2);
    assert_eq!(
        before
            .keys()
            .filter(|key| matches!(key, palette::Place::Subject(..)))
            .count(),
        1
    );
    for i in 0..1100 {
        host.visit_palette_place(palette::Place::Screen(i));
    }
    assert_eq!(host.palette_visits.len(), 1024);
}

#[test]
fn palette_normalizes_stale_panes_and_background_typed_menu_keeps_its_input() {
    let (mut host, _, guild) = fixture();
    guild.lock().unwrap().page = true;
    host.last_pane.insert(View::Screen(1), Pane::Nav);
    open(&mut host);
    query(&mut host, "guild / guild");
    key(&mut host, Key::Enter);
    assert_eq!(host.view, View::Screen(1));
    assert_eq!(host.focus, Pane::Middle);
    open(&mut host);
    query(&mut host, "report");
    host.update(Event::Msg {
        plugin: 1,
        msg: Box::new(Menu::typed(
            "background-input",
            "Name it",
            "keep these words",
        )),
    });
    assert!(host.palette.is_none());
    let menu = host.menu_view().unwrap();
    assert_eq!(menu.id, "background-input");
    assert_eq!(menu.typed.as_deref(), Some("keep these words"));
    assert!(menu.items.is_empty());
}

#[test]
fn palette_stale_clicks_and_invocations_cannot_answer_a_new_question() {
    let (mut host, watch, _) = fixture();
    open(&mut host);
    query(&mut host, "ask before");
    host.update(Event::Intent(crate::view::Intent::Activate {
        target: crate::view::chrome::targets::menu_item(0),
    }));
    assert_eq!(
        host.menu.as_ref().unwrap().menu.id,
        palette::ID,
        "old unscoped menu row is inert"
    );
    let item = host.menu_view().unwrap().items[0].clone();
    let click = crate::view::Intent::Activate {
        target: item.target.clone().unwrap(),
    };
    host.update(Event::Intent(click.clone()));
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "change");
    host.update(Event::Intent(click.clone()));
    host.update(Event::Intent(crate::view::Intent::Invoke {
        action: item.value.clone(),
    }));
    assert!(
        watch.lock().unwrap().answers.is_empty(),
        "queued palette clicks cannot confirm"
    );
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "change");
    key(&mut host, Key::Esc);
    open(&mut host);
    query(&mut host, "ask before");
    host.update(Event::Intent(click));
    host.update(Event::Intent(crate::view::Intent::Invoke {
        action: item.value,
    }));
    assert_eq!(
        host.menu.as_ref().unwrap().menu.id,
        palette::ID,
        "a reopened palette has a new generation"
    );
    let item = host.menu_view().unwrap().items[0].clone();
    query(&mut host, "copy selected");
    host.update(Event::Intent(crate::view::Intent::Activate {
        target: item.target.unwrap(),
    }));
    assert_eq!(
        watch.lock().unwrap().actions,
        0,
        "a changed result list cannot reinterpret an old row"
    );
    key(&mut host, Key::Esc);
    host.dock.focused = true;
    open(&mut host);
    assert!(
        host.menu.is_none(),
        "composer draft ownership stays with the dock"
    );
}

#[test]
fn palette_rejects_account_scope_change_even_when_the_subject_is_unchanged() {
    let (mut host, watch, _) = fixture();
    open(&mut host);
    query(&mut host, "Watch report");
    watch.lock().unwrap().identity_scope = "another account".into();
    key(&mut host, Key::Enter);
    assert!(watch.lock().unwrap().shown.is_empty());
    assert!(
        host.message
            .as_ref()
            .unwrap()
            .text
            .contains("destination changed")
    );
}

#[test]
fn palette_footer_click_cannot_confirm_the_next_question() {
    let (mut host, watch, _) = fixture();
    open(&mut host);
    let row = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .position(|item| item.label.contains("ask before"))
        .unwrap();
    host.menu.as_mut().unwrap().row = row;
    let target = host
        .chrome_view(80, 24)
        .status
        .hints
        .into_iter()
        .flat_map(|h| h.keys_of)
        .find(|k| k.chord == Chord::new(Key::Enter))
        .unwrap()
        .target;
    assert!(target.as_str().starts_with("chrome:palette-key:"));
    let click = crate::view::Intent::Activate { target };
    host.update(Event::Intent(click.clone()));
    assert_eq!(host.menu.as_ref().unwrap().menu.id, "change");
    host.update(Event::Intent(click));
    assert!(watch.lock().unwrap().answers.is_empty());
}
