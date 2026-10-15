//! What the host does with the intents a frontend with no terminal sends.
use super::*;
use crate::ScreenGroup;
use crate::view::{Intent, ListRow, Node, PaneId, PaneView, ScreenView, Target};
use std::sync::{Arc, Mutex};

/// The panes the probe's intents arrived in, with the intent.
type Seen = Arc<Mutex<Vec<(Pane, Intent)>>>;

struct Probe(Seen);

impl Plugin for Probe {
    fn id(&self) -> &'static str {
        "ink-probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "probe",
            name: "Probe",
            key: 'P',
        }]
    }
    fn view(&self, _: &str, _: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
        let list = |pane, rows: &[&str]| {
            PaneView::new(
                pane,
                "pane",
                Node::List {
                    rows: rows.iter().map(|r| ListRow::new(*r, *r)).collect(),
                    selected: Some(0),
                    editing: None,
                },
            )
        };
        Some(ScreenView::new(
            "probe",
            vec![
                list(PaneId::Nav, &["nav-a", "nav-b"]),
                list(PaneId::Middle, &["ABC-11", "ABC-12"]),
            ],
        ))
    }
    fn intent(&mut self, _: &str, pane: Pane, intent: &Intent) -> Outcome {
        self.0.lock().unwrap().push((pane, intent.clone()));
        Outcome::done()
    }
}

fn fixture() -> (Host, Seen) {
    let seen: Seen = Arc::default();
    let mut host = Host::new(vec![Box::new(Probe(seen.clone()))], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host.update(Event::Key(KeyEvent::new(KeyCode::Char('P'), KeyModifiers::NONE)));
    (host, seen)
}

#[test]
fn a_select_on_a_row_of_another_pane_focuses_that_pane_before_the_part_gets_it() {
    let (mut host, seen) = fixture();
    host.focus = Pane::Nav;
    host.bar = true;
    let select = Intent::Select {
        target: Target::new("ABC-12"),
    };
    host.on_intent(select.clone());
    assert_eq!(host.focus, Pane::Middle);
    assert!(!host.bar, "a click takes the keys from the top bar");
    assert_eq!(seen.lock().unwrap().last(), Some(&(Pane::Middle, select)));
}

#[test]
fn a_select_on_a_row_of_the_focused_pane_or_an_unknown_target_keeps_the_focus() {
    let (mut host, _) = fixture();
    host.focus = Pane::Nav;
    host.on_intent(Intent::Select {
        target: Target::new("nav-b"),
    });
    assert_eq!(host.focus, Pane::Nav);
    host.on_intent(Intent::Select {
        target: Target::new("nowhere"),
    });
    assert_eq!(host.focus, Pane::Nav);
}

/// Ink's press on a pane's empty space, and its wheel over a pane, send
/// `pane:<id>`: the pane is focused, the keys leave the top bar, and no part
/// sees it (`Hit::Pane` in `on_mouse` does the same).
#[test]
fn a_select_on_a_panes_own_target_only_focuses_that_pane() {
    let (mut host, seen) = fixture();
    host.focus = Pane::Middle;
    host.bar = true;
    let before = seen.lock().unwrap().len();
    host.on_intent(Intent::Select {
        target: Target::new("pane:nav"),
    });
    assert_eq!(host.focus, Pane::Nav);
    assert!(!host.bar, "the wheel or a press on a pane takes the keys from the top bar");
    assert_eq!(seen.lock().unwrap().len(), before, "no part hears of it");
    // A work area section's own target (`pane:middle#1`) and an unknown pane go on as before.
    host.on_intent(Intent::Select {
        target: Target::new("pane:middle#1"),
    });
    assert_eq!(host.focus, Pane::Nav);
    host.on_intent(Intent::Select {
        target: Target::new("pane:sideways"),
    });
    assert_eq!(host.focus, Pane::Nav);
}

#[test]
fn a_panes_own_target_moves_nothing_while_a_menu_is_open() {
    let (mut host, _) = fixture();
    host.focus = Pane::Middle;
    host.on_intent(Intent::Context {
        target: Target::new("ABC-12"),
    });
    assert!(host.menu.is_some(), "the context menu is open");
    host.on_intent(Intent::Select {
        target: Target::new("pane:nav"),
    });
    assert_eq!(host.focus, Pane::Middle);
}

#[test]
fn a_context_on_a_row_of_another_pane_focuses_it_too() {
    let (mut host, _) = fixture();
    host.focus = Pane::Nav;
    host.on_intent(Intent::Context {
        target: Target::new("ABC-11"),
    });
    assert_eq!(host.focus, Pane::Middle);
}

#[test]
fn the_chrome_names_the_capitals_the_host_takes() {
    let (host, _) = fixture();
    let chrome = host.chrome_view(80, 24);
    assert!(chrome.screen_keys.contains(&'P'), "{:?}", chrome.screen_keys);
    assert!(!chrome.screen_keys.contains(&'G'));
}

#[test]
fn a_context_intent_opens_the_keys_menu_as_a_right_click_does() {
    let (mut host, _) = fixture();
    assert!(host.menu.is_none());
    host.on_intent(Intent::Context {
        target: Target::new("ABC-11"),
    });
    assert!(host.menu.is_some(), "no menu after a context intent");
    // A second one while the menu is open leaves it as it is.
    host.on_intent(Intent::Context {
        target: Target::new("ABC-12"),
    });
    assert!(host.menu.is_some());
}

/// Two screens of one group: a source row with two tabs.
struct Grouped(&'static str, char);

impl Plugin for Grouped {
    fn id(&self) -> &'static str {
        self.0
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: self.0,
            name: self.0,
            key: self.1,
        }]
    }
    fn screen_group(&self, _: &str) -> Option<ScreenGroup> {
        Some(ScreenGroup {
            id: "grp",
            name: "Group",
            key: 'Z',
            source: self.0,
        })
    }
    fn view(&self, _: &str, _: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
        Some(ScreenView::new(
            self.0,
            vec![PaneView::new(
                PaneId::Nav,
                "pane",
                Node::List {
                    rows: vec![ListRow::new("one", "one"), ListRow::new("two", "two")],
                    selected: Some(0),
                    editing: None,
                },
            )],
        ))
    }
}

fn grouped() -> Host {
    let mut host = Host::new(vec![Box::new(Grouped("alpha", 'A')), Box::new(Grouped("beta", 'B'))], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host
}

/// A click on a source tab is an Activate (Ink's controller sends one for every
/// chrome target) and ends where Ratatui's Press ends: the screen switched and
/// the keys on the source row, from which Down enters the screen.
#[test]
fn an_activate_on_a_source_tab_ends_where_a_press_on_it_ends() {
    let mut host = grouped();
    host.view = View::Screen(0);
    host.enter_bar();
    let tab = crate::view::chrome::targets::source_tab(1);
    host.on_intent(Intent::Select { target: tab.clone() });
    assert_eq!(host.view, View::Screen(0), "a select is not a click");
    host.on_intent(Intent::Activate { target: tab });
    assert_eq!(host.view, View::Screen(1));
    assert!(host.sources && !host.bar && !host.tabs, "a click focuses the source row");
    host.on_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert!(!host.sources && !host.bar, "Down leaves the source row for the screen");
    assert_eq!(host.focus, Pane::Nav);
}
