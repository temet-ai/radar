//! The columns menus (#25): a heading's menu and the pane's columns menu do
//! by keys what the mouse does on the headings, and what they change is kept.
use super::*;
use crate::layout::LayoutIntent;
use crate::view::{ColumnSpec, Intent, Node, PaneId, PaneView, ScreenView};

/// A part with one table: `state`, `updated` and `title`, which takes the rest.
struct Watch;

impl Plugin for Watch {
    fn id(&self) -> &'static str {
        "watch"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec { id: "patrols", name: "Patrols", key: 'P' }]
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
        let columns = vec![ColumnSpec::new("state", 8, 0), ColumnSpec::new("updated", 6, 0), ColumnSpec::flex("title", 10)];
        let table = Node::Table { columns, rows: vec![], selected: None, empty: None };
        Some(ScreenView::new(scope, vec![PaneView::new(PaneId::Middle, "patrols", table)]))
    }
}

fn host() -> Host {
    let mut host = Host::new(vec![Box::new(Watch)], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host.focus = Pane::Middle;
    host
}

fn items(host: &Host) -> Vec<(Option<char>, String)> {
    let open = host.menu.as_ref().expect("a menu is open");
    open.menu.items.iter().filter(|i| !i.heading).map(|i| (i.key, i.label.clone())).collect()
}

fn press(host: &mut Host, c: char) -> Vec<HostEffect> {
    host.update(Event::Intent(Intent::Key { chord: Chord::new(Key::Char(c)) }))
}

fn drawn(host: &Host) -> Vec<(String, u16)> {
    let view = crate::view::screen_showing(host, 120, 30).unwrap();
    let Node::Table { columns, .. } = &view.panes[0].body else { panic!("a table") };
    columns.iter().map(|c| (c.heading.clone(), c.width)).collect()
}

#[test]
fn a_heading_menu_moves_sizes_and_hides_its_column_and_each_change_is_saved() {
    let mut host = host();
    let place = crate::layout::key("patrols", PaneId::Middle, None);
    host.update(Event::Intent(Intent::Layout(LayoutIntent::HeadingMenu { key: place, heading: "state".into() })));
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "state column");
    let keys: Vec<Option<char>> = items(&host).into_iter().map(|(k, _)| k).collect();
    assert_eq!(keys, [Some('r'), Some('w'), Some('n'), Some('h')], "the first column has no move left");

    press(&mut host, 'r');
    assert_eq!(drawn(&host).iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["updated", "state", "title"]);
    // the menu opens again: the key works again, and now it can move left too
    assert!(items(&host).iter().any(|(k, _)| *k == Some('l')));
    press(&mut host, 'w');
    let out = press(&mut host, 'w');
    assert_eq!(drawn(&host)[1], ("state".into(), 12));
    let saved = out.iter().find_map(|fx| match fx { HostEffect::Save(t) => Some(t.clone()), _ => None }).expect("saved");
    assert!(saved.contains("state = 12") && saved.contains("order = ["), "{saved}");

    press(&mut host, 'h');
    assert_eq!(drawn(&host).iter().map(|c| c.0.as_str()).collect::<Vec<_>>(), ["updated", "title"]);
    // a hidden column comes back from the pane's columns menu, which opens after a hide
    assert_eq!(host.menu.as_ref().unwrap().menu.title, "columns");
    let show = items(&host).into_iter().find(|(_, l)| l == "show state").and_then(|(k, _)| k).expect("show state");
    press(&mut host, show);
    assert_eq!(drawn(&host).len(), 3);
    press(&mut host, 'd');
    assert_eq!(drawn(&host), [("state".into(), 8), ("updated".into(), 6), ("title".into(), 10)]);
    assert!(host.layout.is_default());
}

#[test]
fn a_right_click_on_a_tree_heading_opens_its_menu() {
    struct Repos;
    impl Plugin for Repos {
        fn id(&self) -> &'static str {
            "repos"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn screens(&self) -> Vec<ScreenSpec> {
            vec![ScreenSpec { id: "repos", name: "Repos", key: 'R' }]
        }
        fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
            let tree = crate::view::Tree { columns: vec![ColumnSpec::new("state", 8, 0), ColumnSpec::new("updated", 6, 0)], ..Default::default() };
            Some(ScreenView::new(scope, vec![PaneView::new(PaneId::Middle, "repos", Node::Tree(tree))]))
        }
    }
    let mut host = Host::new(vec![Box::new(Repos)], 0).unwrap();
    host.load_config(Some("")).unwrap();
    let place = crate::layout::key("repos", PaneId::Middle, None);
    host.update(Event::Intent(Intent::Layout(LayoutIntent::HeadingMenu { key: place, heading: "updated".into() })));
    assert_eq!(items(&host).into_iter().map(|(_, l)| l).collect::<Vec<_>>(), ["move left", "wider", "narrower", "hide"]);
}

#[test]
fn the_panes_menu_offers_its_columns_and_the_title_cannot_be_hidden_or_sized() {
    let mut host = host();
    host.open_actions_menu(MenuOf::Pane);
    let columns = items(&host).into_iter().find(|(_, l)| l == "columns…").and_then(|(k, _)| k).expect("columns… in the pane's menu");
    press(&mut host, columns);
    let labels: Vec<String> = items(&host).into_iter().map(|(_, l)| l).collect();
    assert_eq!(labels, ["state", "updated", "title"]);
    let title = items(&host).into_iter().find(|(_, l)| l == "title").and_then(|(k, _)| k).unwrap();
    press(&mut host, title);
    assert_eq!(items(&host).into_iter().map(|(_, l)| l).collect::<Vec<_>>(), ["move left"]);
}
