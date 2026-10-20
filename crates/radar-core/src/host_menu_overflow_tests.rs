//! Reading a long menu never changes the action, drops text, or hides its field.

use super::*;
use crate::plugin::{Typing, Verdict};
use std::sync::{Arc, Mutex};

type MenuLog = Arc<Mutex<Vec<(String, String)>>>;

struct MenuProbe {
    log: MenuLog,
    typing: Option<Typing>,
}

impl Plugin for MenuProbe {
    fn id(&self) -> &'static str {
        "menu-probe"
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

    fn typing(&self, _: &str, _: &str) -> Typing {
        self.typing.clone().unwrap_or_default()
    }

    fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
        self.log.lock().unwrap().push((menu.into(), value.into()));
        vec![]
    }
}

fn fixture(menu: Menu, typing: Option<Typing>) -> (Host, MenuLog) {
    let log = Arc::new(Mutex::new(vec![]));
    let mut host = Host::new(
        vec![Box::new(MenuProbe {
            log: log.clone(),
            typing,
        })],
        0,
    )
    .unwrap();
    host.load_config(Some("")).unwrap();
    host.menu = Some(OpenMenu::open(Owner::Plugin(0), menu));
    host.typed_changed(true);
    (host, log)
}

fn press(host: &mut Host, code: Key) {
    host.update(crate::host::chord_event(Chord::new(code)));
}

fn assert_choice(log: &MenuLog, menu: &str, value: &str) {
    assert_eq!(log.lock().unwrap().as_slice(), [(menu.to_string(), value.to_string())]);
}

/// What the renderer reports once it has laid the menu out: `total` lines
/// of which `height` show.
fn laid_out(host: &Host, height: usize, total: usize) {
    let viewport = &host.menu.as_ref().unwrap().viewport;
    viewport.height.set(height);
    viewport.total.set(total);
}

fn long_write_confirmation() -> Menu {
    let text = (0..36)
        .map(|n| format!("note {n:02}: keep this text while reading"))
        .collect::<Vec<_>>()
        .join("\n");
    Menu::write_confirm("review", "keep these notes?", &text, "keep", "leave").at("edit")
}

#[test]
fn paging_a_write_confirmation_emits_no_effects_and_keeps_edit_selected() {
    let menu = long_write_confirmation();
    let (mut host, log) = fixture(menu.clone(), None);
    laid_out(&host, 10, 40);
    let selected = host.menu.as_ref().unwrap().row;
    assert_eq!(host.menu.as_ref().unwrap().menu.items[selected].value, "edit");
    assert!(host.menu.as_ref().unwrap().viewport.more());
    assert!(
        host.update(crate::host::chord_event(Chord::new(Key::PageDown))).is_empty(),
        "paging only changes the reading position"
    );
    let open = host.menu.as_ref().unwrap();
    assert_eq!(open.viewport.at.get(), 9);
    assert_eq!(open.row, selected);
    assert_eq!(open.menu, menu);
    assert!(log.lock().unwrap().is_empty());
    host.update(crate::host::chord_event(Chord::new(Key::PageUp)));
    assert_eq!(host.menu.as_ref().unwrap().viewport.at.get(), 0);
    press(&mut host, Key::Enter);
    assert!(host.menu.is_none());
    assert_choice(&log, "review", "edit");
}

#[test]
fn paging_an_ordinary_menu_changes_the_reading_position_not_the_selection() {
    let menu = Menu::new(
        "pick",
        "choose an item",
        (0..30).map(|n| MenuItem::new('a', format!("item {n}"), "does it", format!("v{n}"))).collect(),
    );
    let (mut host, log) = fixture(menu, None);
    laid_out(&host, 8, 30);
    let row = host.menu.as_ref().unwrap().row;
    press(&mut host, Key::PageDown);
    let open = host.menu.as_ref().unwrap();
    assert_eq!((open.viewport.at.get(), open.row), (7, row));
    assert!(log.lock().unwrap().is_empty());
    press(&mut host, Key::Esc);
    assert!(host.menu.is_none());
}

// --- host state the painted tests used to prove, read from view data -------
//
// Dropped with the painter-only tests (Ink's hit mapping and input tests own
// them): a click on a wrapped continuation choosing its original item, and
// the mouse wheel (Ink maps it to Up/Down, which paging here covers).

/// A long question: both choices stay pinned while the notes are read.
fn long_question() -> Menu {
    let mut menu = Menu::confirm("review", "keep these notes?", "keep", "leave").saying("opening-note");
    for n in 0..46 {
        menu = menu.saying(format!("detail {n:02}: read this line before choosing"));
    }
    menu.saying("closing-note")
}

fn offers() -> Vec<MenuItem> {
    (0..24)
        .map(|n| MenuItem::pick(format!("choice {n:02}"), "a possible value", format!("value-{n:02}")))
        .collect()
}

fn long_title() -> String {
    (0..24).map(|n| format!("title{n:02}")).collect::<Vec<_>>().join(" ")
}

/// What the host measures for a frame this big (the open menu's reading
/// position, rows shown and usable controls), as every frontend is sent it.
fn measured(host: &Host, size: (u16, u16)) {
    let _ = host.chrome_view(size.0, size.1);
}

fn at(host: &Host) -> usize {
    host.menu.as_ref().unwrap().viewport.at.get()
}

fn field(host: &Host) -> (String, usize) {
    let input = host.menu.as_ref().unwrap().input.as_ref().unwrap();
    (input.value().to_string(), input.cursor())
}

#[test]
fn a_scrolled_question_still_answers_with_y_and_with_enter_and_closes() {
    for (answer, value) in [(Key::Char('y'), "yes"), (Key::Enter, "yes"), (Key::Char('n'), "no")] {
        let (mut host, log) = fixture(long_question(), None);
        laid_out(&host, 10, 60);
        press(&mut host, Key::PageDown);
        assert!(at(&host) > 0);
        assert!(log.lock().unwrap().is_empty());
        press(&mut host, answer);
        assert!(host.menu.is_none(), "{answer:?} closes the question");
        assert_choice(&log, "review", value);
    }
}

#[test]
fn enter_after_paging_an_ordinary_menu_gives_the_selected_value() {
    let menu = Menu::new(
        "pick",
        "choose an item",
        (0..30).map(|n| MenuItem::new('a', format!("item {n}"), "does it", format!("v{n}"))).collect(),
    );
    let (mut host, log) = fixture(menu, None);
    laid_out(&host, 8, 30);
    press(&mut host, Key::PageDown);
    assert!(at(&host) > 0);
    press(&mut host, Key::Enter);
    assert!(host.menu.is_none());
    assert_choice(&log, "pick", "v0");
}

#[test]
fn a_long_title_opens_with_the_action_in_sight_but_a_long_question_opens_at_its_start() {
    let menu = Menu::new("pick", long_title(), vec![MenuItem::new('a', "keep", "", "kept")]);
    let (host, log) = fixture(menu, None);
    measured(&host, (34, 10));
    assert!(at(&host) > 0, "the title is too long to show before the action");
    assert_eq!(host.menu.as_ref().unwrap().row, 0);
    assert!(log.lock().unwrap().is_empty());

    let question = Menu::confirm("review", long_title(), "keep", "leave").saying("draft note");
    let (host, log) = fixture(question, None);
    let row = host.menu.as_ref().unwrap().row;
    measured(&host, (34, 10));
    assert_eq!(at(&host), 0, "a question starts with what it asks");
    assert_eq!(host.menu.as_ref().unwrap().row, row, "its choice is kept");
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn a_narrow_question_keeps_an_explicit_no_default_and_its_accelerators() {
    for (answer, value) in [(Key::Enter, "no"), (Key::Char('y'), "yes")] {
        let (mut host, log) = fixture(long_question().at("no"), None);
        measured(&host, (34, 10));
        let row = host.menu.as_ref().unwrap().row;
        assert_eq!(host.menu.as_ref().unwrap().menu.items[row].value, "no");
        press(&mut host, Key::PageDown);
        measured(&host, (34, 10));
        assert!(at(&host) > 0);
        assert_eq!(host.menu.as_ref().unwrap().row, row);
        assert!(log.lock().unwrap().is_empty());
        press(&mut host, answer);
        assert!(host.menu.is_none());
        assert_choice(&log, "review", value);
    }
}

#[test]
fn typed_menu_paging_keeps_the_value_cursor_and_offers_and_editing_still_works() {
    let (mut host, log) = fixture(
        Menu::typed("name", "name the item", "kept text"),
        Some(Typing { verdict: None, offers: offers() }),
    );
    laid_out(&host, 4, 30);
    let items = host.menu.as_ref().unwrap().menu.items.clone();
    let before = field(&host);
    press(&mut host, Key::PageDown);
    assert!(at(&host) > 0);
    assert_eq!(host.menu.as_ref().unwrap().row, NO_ROW);
    assert_eq!(field(&host), before);
    assert_eq!(host.menu.as_ref().unwrap().menu.items, items);
    assert!(log.lock().unwrap().is_empty());
    press(&mut host, Key::PageUp);
    assert_eq!(at(&host), 0);
    press(&mut host, Key::Left);
    press(&mut host, Key::Char('!'));
    assert_eq!(field(&host).0, "kept tex!t");
    host.update(crate::host::chord_event(Chord::ctrl('u')));
    assert_eq!(field(&host).0, "t");
    press(&mut host, Key::Enter);
    assert_choice(&log, "name", "t");
}

#[test]
fn a_refused_value_keeps_the_field_text_and_enter_sends_nothing() {
    let reason = (0..28).map(|n| format!("reason{n:02}")).collect::<Vec<_>>().join(" ");
    let (mut host, log) = fixture(
        Menu::typed("name", "name the item", "kept text"),
        Some(Typing { verdict: Some(Verdict::Bad(reason.clone())), offers: vec![] }),
    );
    measured(&host, (34, 10));
    assert_eq!(host.menu.as_ref().unwrap().refused.as_deref(), Some(reason.as_str()));
    press(&mut host, Key::Enter);
    assert!(host.menu.is_some(), "the field stays open");
    assert_eq!(field(&host).0, "kept text");
    assert_eq!(host.menu.as_ref().unwrap().refused.as_deref(), Some(reason.as_str()));
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn a_resize_clamps_the_reading_position_and_changes_nothing_else() {
    let (mut host, log) = fixture(long_question(), None);
    let original = host.menu.as_ref().unwrap().menu.clone();
    let row = host.menu.as_ref().unwrap().row;
    measured(&host, (80, 24));
    for _ in 0..80 {
        press(&mut host, Key::PageDown);
        measured(&host, (80, 24));
    }
    let max = host.menu.as_ref().unwrap().viewport.max();
    assert!(max > 0 && at(&host) == max, "read to the end");
    measured(&host, (160, 40));
    assert!(at(&host) < max, "a taller screen clamps the old bottom");
    for size in [(34, 10), (160, 40), (80, 24)] {
        measured(&host, size);
        assert!(at(&host) <= host.menu.as_ref().unwrap().viewport.max());
        assert_eq!(host.menu.as_ref().unwrap().row, row);
        assert_eq!(host.menu.as_ref().unwrap().menu, original);
    }
    assert!(log.lock().unwrap().is_empty());
}

#[test]
fn on_a_tiny_terminal_nothing_hidden_is_submitted_and_escape_closes() {
    for size in [(1, 1), (2, 3), (8, 4), (20, 4)] {
        for menu in [
            Menu::confirm("ask", "keep the item?", "keep", "leave"),
            Menu::new("pick", "choose an item", vec![MenuItem::new('a', "keep", "", "kept")]),
            Menu::typed("name", "name the item", "kept text"),
        ] {
            let (mut host, log) = fixture(menu, None);
            measured(&host, size);
            assert!(!host.menu.as_ref().unwrap().viewport.usable.get(), "{size:?} cannot show this menu safely");
            press(&mut host, Key::Enter);
            if host.menu.as_ref().unwrap().input.is_none() {
                press(&mut host, Key::Char('y'));
                press(&mut host, Key::Char('a'));
            }
            assert!(host.menu.is_some(), "a hidden choice must not be submitted at {size:?}");
            assert!(log.lock().unwrap().is_empty());
            press(&mut host, Key::Esc);
            assert!(host.menu.is_none());
        }
    }
}
