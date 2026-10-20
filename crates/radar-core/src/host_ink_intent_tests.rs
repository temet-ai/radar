//! What the host does with the intents a frontend with no terminal sends.
use super::*;
use crate::ScreenGroup;
use crate::view::{Ink, Intent, ListRow, Node, PaneId, PaneView, ScreenView, Seg, Target, Text};
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
                PaneView::new(
                    PaneId::Detail,
                    "detail",
                    Node::Document {
                        lines: vec![Text::plain("Branch ").push(Seg::new("copy", Ink::Accent).targeted("chip-1"))],
                        cut: false,
                    },
                ),
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
    host.update(crate::host::chord_event(Chord::new(Key::Char('P'))));
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

/// Ink sends every intent for a hit, so a right-press or a double click on a
/// pane's empty space arrives as `Context` or `Activate` on `pane:<id>`: the
/// host focuses the pane, opens no menu and the part never hears of it.
#[test]
fn a_context_or_activate_on_a_panes_own_target_only_focuses_that_pane() {
    for pane in ["pane:middle", "pane:detail"] {
        let (mut host, seen) = fixture();
        let want = if pane == "pane:middle" { Pane::Middle } else { Pane::Detail };
        let before = seen.lock().unwrap().len();
        for intent in [
            Intent::Context { target: Target::new(pane) },
            Intent::Activate { target: Target::new(pane) },
            Intent::Fold { target: Target::new(pane), open: true },
        ] {
            host.focus = Pane::Nav;
            host.bar = true;
            host.on_intent(intent.clone());
            assert_eq!(host.focus, want, "{intent:?} focuses {pane}");
            assert!(!host.bar, "{intent:?} takes the keys from the top bar");
            assert!(host.menu.is_none(), "{intent:?} on empty space opens no menu");
            assert_eq!(seen.lock().unwrap().len(), before, "no part hears of {intent:?}");
        }
    }
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

/// A chip drawn inside a line of the detail pane is a target of that pane,
/// not of the tree: a press on it from the tree focuses Detail and the part
/// hears it there (a Ratatui press focused the pane first).
#[test]
fn a_select_on_a_chip_in_the_detail_pane_focuses_it_while_the_tree_has_the_keys() {
    let (mut host, seen) = fixture();
    host.focus = Pane::Middle;
    host.bar = true;
    let select = Intent::Select {
        target: Target::new("chip-1"),
    };
    host.on_intent(select.clone());
    assert_eq!(host.focus, Pane::Detail);
    assert!(!host.bar);
    assert_eq!(seen.lock().unwrap().last(), Some(&(Pane::Detail, select)));
    host.focus = Pane::Nav;
    let activate = Intent::Activate {
        target: Target::new("chip-1"),
    };
    host.on_intent(activate.clone());
    assert_eq!(seen.lock().unwrap().last(), Some(&(Pane::Detail, activate)));
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
    host.on_key(Chord::new(Key::Down));
    assert!(!host.sources && !host.bar, "Down leaves the source row for the screen");
    assert_eq!(host.focus, Pane::Nav);
}

fn copy_records(effects: &[HostEffect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            HostEffect::Journal(crate::journal_runtime::JournalRecord::Begin { descriptor, .. }) => {
                Some(descriptor.kind.as_str().to_string())
            }
            _ => None,
        })
        .collect()
}

/// What the renderer selected is copied by the host, as a drag is in the
/// Ratatui runner: one effect the runner sends through `copy_ways`, journalled
/// as `system.copy.selection` (and not as an action on the screen under it).
#[test]
fn a_copy_intent_becomes_the_copy_effect_and_is_journalled_as_a_selection() {
    let (mut host, seen) = fixture();
    host.enable_journal();
    let effects = host.update(Event::Intent(Intent::Copy { text: "ABC-12".into() }));
    assert_eq!(copy_records(&effects), ["system.copy.selection"]);
    let copies: Vec<_> = effects
        .iter()
        .filter_map(|e| match e {
            HostEffect::Tracked { effect, .. } => match effect.as_ref() {
                HostEffect::CopySelection(text) => Some(text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(copies, ["ABC-12"]);
    assert!(seen.lock().unwrap().is_empty(), "no part sees a copy");
}

#[test]
fn a_copy_intent_copies_with_no_journal_too() {
    let (mut host, _) = fixture();
    let effects = host.update(Event::Intent(Intent::Copy { text: "two\nrows".into() }));
    assert!(matches!(&effects[..], [HostEffect::CopySelection(t)] if t == "two\nrows"));
}

#[test]
fn a_copy_intent_over_the_clipboard_cap_is_refused_and_blank_text_is_nothing() {
    let (mut host, _) = fixture();
    host.enable_journal();
    let big = "x".repeat(crate::ipc::MAX_CLIPBOARD + 1);
    let effects = host.update(Event::Intent(Intent::Copy { text: big }));
    assert!(effects.iter().all(|e| !matches!(
        e,
        HostEffect::Copy(_) | HostEffect::CopySelection(_) | HostEffect::Tracked { .. }
    )));
    assert!(copy_records(&effects).is_empty(), "a refused copy is not recorded as a copy");
    assert!(host.message.as_ref().is_some_and(|m| m.contains("too much text")));
    let exactly = "x".repeat(crate::ipc::MAX_CLIPBOARD);
    assert!(!host.update(Event::Intent(Intent::Copy { text: exactly })).is_empty());
    host.message = None;
    assert!(host.update(Event::Intent(Intent::Copy { text: "  \n ".into() })).is_empty());
}

/// A drag selects inside the pane it started in: the frame marks the inside
/// of each pane (border, title and scrollbar column left out), and nothing
/// of the top bar or the status line.
#[test]
fn a_frame_marks_each_panes_inside_as_selectable_and_nothing_of_the_chrome() {
    let (host, _) = fixture();
    let _ = crate::shell::to_text(&host, 100, 24);
    let areas = host.frontend.selectable();
    assert_eq!(areas.len(), 2, "the nav and the middle pane: {areas:?}");
    for a in &areas {
        assert!(a.rect.x >= 1 && a.rect.y >= 2, "inside the border and under the top bar: {a:?}");
        assert!(a.rect.bottom() <= 22, "above the bottom border and the status line: {a:?}");
        assert!(!a.text, "lists are rows to choose from, not text");
    }
    assert!(areas[0].rect.right() < areas[1].rect.x, "the panes' borders lie between them");
    // A new frame starts afresh.
    let _ = crate::shell::to_text(&host, 100, 24);
    assert_eq!(host.frontend.selectable().len(), 2);
}

#[test]
fn an_open_menu_is_the_only_place_a_drag_selects() {
    let (mut host, _) = fixture();
    host.on_key(Chord::new(Key::Char('x')));
    assert!(host.menu.is_some());
    let _ = crate::shell::to_text(&host, 100, 24);
    let areas = host.frontend.selectable();
    assert_eq!(areas.len(), 1, "{areas:?}");
    let menu = host
        .hits
        .borrow()
        .iter()
        .find_map(|(rect, hit)| matches!(hit, Hit::MenuBox).then_some(*rect))
        .unwrap();
    assert!(areas[0].rect.x > menu.x && areas[0].rect.right() < menu.right(), "inside the menu's border");
}

#[test]
fn the_status_line_names_the_terminals_own_selection_key_while_a_drag_selects() {
    let (mut host, _) = fixture();
    let words = |host: &Host| {
        host.chrome_status().hints.iter().map(|h| (h.keys.clone(), h.label.clone())).collect::<Vec<_>>()
    };
    assert!(!words(&host).iter().any(|(k, _)| k == "shift"));
    host.selecting = Some("shift");
    assert_eq!(words(&host), [("shift".to_string(), "terminal's own selection".to_string())]);
    host.selecting = Some("option");
    assert_eq!(words(&host)[0].0, "option");
    host.selecting = None;
    assert!(words(&host).len() > 1, "the usual keys come back");
}

#[test]
fn only_text_nodes_are_text_panes() {
    use crate::view::Text;
    let doc = Node::Document { lines: vec![Text::default()], cut: false };
    assert!(doc.is_text());
    assert!(Node::Rich { blocks: vec![], pictures: None }.is_text());
    assert!(Node::State { state: crate::view::State::Empty, text: String::new() }.is_text());
    let list = Node::List { rows: vec![], selected: None, editing: None };
    assert!(!list.is_text());
}
