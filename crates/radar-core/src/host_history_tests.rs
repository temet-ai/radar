//! History is host-owned, read-only, and captures an exact subject at entry.
use super::*;
use crate::journal::{SubjectKey, SubjectRole, SubjectSnapshot};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    selected: String,
    typing: bool,
    keys: Vec<Chord>,
    messages: usize,
    intents: usize,
}
struct Probe(Arc<Mutex<State>>);
impl Plugin for Probe {
    fn id(&self) -> &'static str {
        "history-probe"
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
    fn owns(&self) -> Vec<crate::compose::Owned> {
        vec![crate::compose::Owned::page("card", "cards")]
    }
    fn subject(&self, _: &str, _: Pane) -> Option<Subject> {
        Some(Subject::new(
            "card",
            self.0.lock().unwrap().selected.clone(),
            "Selected card",
        ))
    }
    fn journal_subject(&self, subject: &Subject) -> Option<SubjectSnapshot> {
        Some(SubjectSnapshot::new(
            SubjectKey::new("probe", "https://example.test", "card", &subject.id).ok()?,
            SubjectRole::Primary,
        ))
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().typing
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
        use crate::view as v;
        Some(v::ScreenView::new(scope, vec![v::PaneView::new(
            v::PaneId::Middle, "items", v::Node::Document { lines: vec![], cut: false },
        )]))
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
        self.0.lock().unwrap().intents += 1;
        if let crate::view::Intent::Key { chord } = intent {
            self.0.lock().unwrap().keys.push(*chord);
            Outcome::done()
        } else {
            Outcome::Ignored
        }
    }
    fn on_msg(&mut self, _: Msg) -> Vec<Effect> {
        self.0.lock().unwrap().messages += 1;
        vec![]
    }
}
fn fixture() -> (Host, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        selected: "ONE-1".into(),
        ..Default::default()
    }));
    (
        Host::new(vec![Box::new(Probe(state.clone()))], 0).unwrap(),
        state,
    )
}
fn press(host: &mut Host, code: Key) -> Vec<HostEffect> {
    host.update(crate::host::chord_event(Chord::new(code)))
}
fn request(fx: &[HostEffect]) -> &crate::history::Request {
    fx.iter()
        .find_map(|fx| match fx {
            HostEffect::History(request) => Some(request.as_ref()),
            _ => None,
        })
        .unwrap()
}
#[test]
fn history_shortcut_keeps_editor_text_and_restores_underlying_navigation() {
    let (mut host, state) = fixture();
    state.lock().unwrap().typing = true;
    assert!(press(&mut host, Key::Char(HISTORY_KEY)).is_empty());
    assert!(host.history.is_none());
    assert_eq!(
        state.lock().unwrap().keys.last().unwrap().key,
        Key::Char(HISTORY_KEY)
    );
    state.lock().unwrap().typing = false;
    host.focus = Pane::Detail;
    host.bar = true;
    let before = (host.view, host.focus, host.bar, host.tabs);
    assert!(
        request(&press(&mut host, Key::Char(HISTORY_KEY)))
            .filter
            .subject
            .is_none()
    );
    assert!(host.history.is_some());
    press(&mut host, Key::Char('w'));
    press(&mut host, Key::Char('q'));
    assert!(!host.quit);
    press(&mut host, Key::Esc);
    assert!(host.history.is_none());
    assert_eq!(before, (host.view, host.focus, host.bar, host.tabs));
}
#[test]
fn history_menu_captures_selected_subject_before_background_selection_changes() {
    let (mut host, state) = fixture();
    press(&mut host, Key::Char('x'));
    let row = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .position(|row| row.label.starts_with("history of "))
        .unwrap();
    state.lock().unwrap().selected = "TWO-2".into();
    host.menu.as_mut().unwrap().row = row;
    let fx = press(&mut host, Key::Enter);
    assert_eq!(request(&fx).filter.subject.as_ref().unwrap().id, "ONE-1");
    assert!(host.menu.is_none());
}
#[test]
fn history_navigation_is_not_recorded_and_background_completions_are_delivered() {
    let (mut host, state) = fixture();
    host.enable_journal();
    let opened = press(&mut host, Key::Char(HISTORY_KEY));
    assert_eq!(
        opened.len(),
        1,
        "opening history must not record its own read"
    );
    let first = request(&opened).id;
    for code in [
        Key::Char('j'),
        Key::Char('x'),
        Key::Char('w'),
        Key::Tab,
    ] {
        let fx = press(&mut host, code);
        assert!(fx.iter().all(|fx| matches!(fx, HostEffect::History(_))));
    }
    host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(()),
    });
    assert_eq!(state.lock().unwrap().messages, 1);
    assert!(state.lock().unwrap().keys.is_empty());
    press(&mut host, Key::Esc);
    assert!(
        host.history.is_some(),
        "first escape returns from local help"
    );
    press(&mut host, Key::Esc);
    let second = request(&press(&mut host, Key::Char(HISTORY_KEY))).id;
    assert!(second > first);
}
#[test]
fn history_settings_has_all_activity_without_a_fake_subject() {
    let mut host = Host::new(vec![], 0).unwrap();
    // Nothing acts on a row here: `x` opens the pane's menu straight away.
    press(&mut host, Key::Char('x'));
    let menu = &host.menu.as_ref().unwrap().menu;
    assert!(menu.items.iter().any(|row| row.label == "activity history"));
    assert!(!menu.items.iter().any(|row| row.label.starts_with("history of ")));
    press(&mut host, Key::Esc);
    let fx = press(&mut host, Key::Char(HISTORY_KEY));
    assert!(request(&fx).filter.subject.is_none());
    host.update(Event::History {
        id: request(&fx).id,
        result: Err(crate::history::ReadError::Disabled),
    });
    assert!(host.history.is_some());
}

#[test]
fn history_does_not_steal_h_from_typed_menus_or_colour_editors() {
    let (mut host, _) = fixture();
    host.menu = Some(OpenMenu::open(
        Owner::Host,
        Menu::typed("typed", "type here", ""),
    ));
    press(&mut host, Key::Char('H'));
    assert!(host.history.is_none());
    assert_eq!(
        host.menu.as_ref().unwrap().input.as_ref().unwrap().value(),
        "H"
    );
    host.menu = None;
    host.view = View::Settings;
    host.colours.edit = Some(crate::input::TextInput::new(""));
    press(&mut host, Key::Char('H'));
    assert!(host.history.is_none());
    assert_eq!(host.colours.edit.as_ref().unwrap().value(), "H");
}

#[test]
fn closed_overlay_result_cannot_replace_a_reopened_request() {
    let (mut host, _) = fixture();
    let old = request(&press(&mut host, Key::Char('H'))).id;
    press(&mut host, Key::Esc);
    let current = request(&press(&mut host, Key::Char('H'))).id;
    host.update(Event::History {
        id: old,
        result: Err(crate::history::ReadError::Disabled),
    });
    let screen = crate::headless::text_of(&host, 80, 24);
    assert!(screen.contains("Loading recorded observations"));
    assert!(!screen.contains("disabled"));
    host.update(Event::History {
        id: current,
        result: Err(crate::history::ReadError::Disabled),
    });
    assert!(crate::headless::text_of(&host, 80, 24).contains("disabled"));
}

#[test]
fn history_footer_keys_are_targets_that_close_it_without_reaching_the_part() {
    let (mut host, state) = fixture();
    press(&mut host, Key::Char('H'));
    let target = host.history.as_ref().unwrap().key_target("esc");
    let footer = host.chrome_view(80, 24).history.expect("the overlay's view").footer;
    assert!(footer.iter().any(|hint| hint.keys_of.iter().any(|key| key.target == target)), "{footer:?}");
    host.update(Event::Intent(crate::view::Intent::Activate { target }));
    assert!(host.history.is_none());
    assert!(state.lock().unwrap().keys.is_empty());
}

#[test]
fn recent_entry_and_refresh_use_host_time_and_all_activity_remains_unbounded() {
    let (mut host, _) = fixture();
    host.now = 1_792_000_000;
    let fx = press(&mut host, Key::Char('H'));
    let id = request(&fx).id;
    assert_eq!(
        request(&fx).filter.since_ms,
        Some((host.now - 86_400) * 1000)
    );
    host.update(Event::History {
        id,
        result: Err(crate::history::ReadError::Unavailable),
    });
    host.now += 3600;
    let refreshed = press(&mut host, Key::Char('r'));
    assert_eq!(
        request(&refreshed).filter.since_ms,
        Some((host.now - 86_400) * 1000)
    );
    assert!(request(&refreshed).snapshot.is_none());
    // All of it, unbounded: `a` in the history the pane's menu opens.
    press(&mut host, Key::Esc);
    press(&mut host, Key::Char('x'));
    press(&mut host, Key::Char('x'));
    let row = host
        .menu
        .as_ref()
        .unwrap()
        .menu
        .items
        .iter()
        .position(|row| row.label == "activity history")
        .unwrap();
    host.menu.as_mut().unwrap().row = row;
    press(&mut host, Key::Enter);
    let all = press(&mut host, Key::Char('a'));
    assert_eq!(
        request(&all).filter,
        crate::journal::work_map::EventFilter::default()
    );
}

/// From #26: nothing sent while History is open reaches the screen under
/// it; a chrome target still acts; closing gives back the same place.
#[test]
fn history_masks_the_screen_under_it_from_every_intent() {
    use crate::view::{Chord, Intent, PaneId, Target};
    let (mut host, state) = fixture();
    host.enable_journal();
    host.focus = Pane::Nav;
    let before = (host.view, host.focus, host.bar, host.tabs);
    let first = request(&press(&mut host, Key::Char('H'))).id;
    let mut headless = crate::headless::Headless::new(host, 80, 24);
    headless.focus(PaneId::Detail);
    assert_eq!(headless.host().focus, Pane::Nav, "no focus moves behind the overlay");
    for intent in [
        Intent::Select { target: Target::new("underlying-row") },
        Intent::Activate { target: Target::new("underlying-row") },
        Intent::Invoke { action: "provider.write".into() },
        Intent::Key { chord: Chord::char('w') },
        Intent::Key { chord: Chord::char('q') },
    ] {
        assert!(headless.send(intent.clone()).is_empty(), "{intent:?}");
    }
    assert_eq!(state.lock().unwrap().intents, 0);
    assert!(headless.records().is_empty());
    assert!(!headless.host().quit);
    assert!(headless.host().history.is_some());
    let esc = headless.host().history.as_ref().unwrap().key_target("esc");
    headless.send(Intent::Activate { target: esc });
    assert!(headless.host().history.is_none(), "a chrome target of the overlay acts");
    let h = headless.host();
    assert_eq!(before, (h.view, h.focus, h.bar, h.tabs));
    let reopened = headless.send(Intent::Key { chord: Chord::char('H') });
    assert!(request(&reopened).id > first);
}

/// From #26: tracked input, however deeply wrapped, is the overlay's, once,
/// and never the part's; a task's completion still goes through.
#[test]
fn tracked_input_under_history_is_the_overlays_once() {
    use crate::view::{Chord, Intent};
    let (mut host, state) = fixture();
    host.enable_journal();
    let opened = press(&mut host, Key::Char('H'));
    host.update(Event::History {
        id: request(&opened).id,
        result: Err(crate::history::ReadError::Unavailable),
    });
    let before = host.history_request;
    let fx = host.update(Event::Tracked {
        action: ActionId::new(),
        semantic: None,
        event: Box::new(Event::Tracked {
            action: ActionId::new(),
            semantic: None,
            event: Box::new(Event::Intent(Intent::Key { chord: Chord::char('r') })),
        }),
    });
    assert_eq!(fx.len(), 1, "{fx:?}");
    assert_eq!(request(&fx).id, before + 1, "r reads again");
    assert!(!fx.iter().any(|e| matches!(e, HostEffect::Journal(_))), "not audited");
    assert_eq!(state.lock().unwrap().intents, 0);
    host.update(Event::Tracked {
        action: ActionId::new(),
        semantic: None,
        event: Box::new(Event::Msg {
            plugin: 0,
            msg: Box::new(()),
        }),
    });
    assert_eq!(state.lock().unwrap().messages, 1, "a completion is delivered");
}

/// From #26: a tab of the top bar, clicked under History, closes it and
/// goes there.
#[test]
fn a_tab_clicked_under_history_closes_it_and_goes_there() {
    let (mut host, _) = fixture();
    press(&mut host, Key::Char('H'));
    host.update(Event::Intent(crate::view::Intent::Activate { target: crate::view::chrome::targets::settings_tab() }));
    assert!(host.history.is_none());
    assert_eq!(host.view, View::Settings);
}

/// From #26: History draws in a terminal too small for its footer.
#[test]
fn history_draws_in_a_tiny_terminal() {
    let (mut host, _) = fixture();
    press(&mut host, Key::Char('H'));
    for (width, height) in [(1, 1), (80, 1), (20, 5)] {
        let _ = crate::headless::text_of(&host, width, height);
        let _ = host.chrome_view(width, height);
    }
}

/// A target is pressed for the page it was drawn for: one sent back after
/// another page loaded, a refresh, or the overlay was closed and opened
/// again does nothing, never what now sits at its place.
#[test]
fn a_history_target_drawn_for_an_earlier_page_does_nothing() {
    use crate::history::HistoryFocus;
    use crate::history::tests::{entry, page};
    use crate::journal::Outcome as Observed;
    use crate::view::Intent;
    let (mut host, _) = fixture();
    let read = |host: &mut Host, request: &crate::history::Request, events: std::ops::RangeInclusive<i64>, more: bool| {
        let events = events.map(|n| entry(n, Observed::Requested)).collect();
        host.update(Event::History { id: request.id, result: Ok(Box::new(page(request.filter.clone(), events, more))) });
    };
    let first = request(&press(&mut host, Key::Char('H'))).clone();
    read(&mut host, &first, 1..=5, true);
    let one = host.chrome_view(160, 40).history.unwrap();
    assert_eq!((one.rows.len(), one.selected), (5, Some(0)));
    let row3 = one.rows[3].target.clone();
    let key = |view: &crate::history::HistoryView, label: &str| {
        view.footer.iter().flat_map(|h| &h.keys_of).chain(&view.detail_keys)
            .find(|k| k.label == label).unwrap_or_else(|| panic!("{label} shown")).target.clone()
    };
    let (enter, causal) = (key(&one, "enter"), key(&one, "c"));
    // n: page 2.
    let next = request(&press(&mut host, Key::Char('n'))).clone();
    read(&mut host, &next, 6..=10, false);
    let shown = |host: &Host| {
        let v = host.chrome_view(160, 40).history.unwrap();
        (v.events_title.clone(), v.scope.clone(), v.selected, v.focus)
    };
    let two = shown(&host);
    assert!(two.0.contains("page 2"), "{two:?}");
    assert_eq!((two.2, two.3), (Some(0), HistoryFocus::Events));
    for old in [&row3, &enter, &causal] {
        assert!(host.update(Event::Intent(Intent::Activate { target: old.clone() })).is_empty(), "{old:?}");
        assert!(host.update(Event::Intent(Intent::Select { target: old.clone() })).is_empty(), "{old:?}");
        assert_eq!(shown(&host), two, "{old:?} names page 1");
    }
    // The page shown takes its own targets.
    let now = host.chrome_view(160, 40).history.unwrap();
    host.update(Event::Intent(Intent::Activate { target: now.rows[3].target.clone() }));
    assert_eq!(shown(&host).2, Some(3));
    let row3_two = now.rows[3].target.clone();
    // Closed and opened again: page 1 at the same place, a new drawing.
    press(&mut host, Key::Esc);
    let again = request(&press(&mut host, Key::Char('H'))).clone();
    read(&mut host, &again, 1..=5, true);
    let reopened = shown(&host);
    for old in [&row3, &row3_two, &enter] {
        assert!(host.update(Event::Intent(Intent::Activate { target: old.clone() })).is_empty());
        assert_eq!(shown(&host), reopened, "{old:?} names a closed overlay");
    }
    // A refresh: the same rows read again, a new drawing.
    let fresh = host.chrome_view(160, 40).history.unwrap();
    let refresh = request(&press(&mut host, Key::Char('r'))).clone();
    read(&mut host, &refresh, 1..=5, true);
    host.update(Event::Intent(Intent::Activate { target: fresh.rows[3].target.clone() }));
    host.update(Event::Intent(Intent::Activate { target: key(&fresh, "enter") }));
    assert_eq!(shown(&host), reopened, "targets drawn before the refresh");
}
