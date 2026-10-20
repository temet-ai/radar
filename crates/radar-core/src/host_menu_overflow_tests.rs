//! Reading a long menu never changes the action, drops text, or hides its field.

use super::*;
use crate::plugin::Typing;
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
