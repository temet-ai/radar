//! The columns menus (#25): what the mouse does on a pane's headings, by
//! keys. A pane's menu offers its columns; each heading's menu moves it,
//! makes it wider or narrower, hides it, shows the hidden ones and puts the
//! part's columns back. Each choice is a [`LayoutIntent`] kept in
//! `[layout]`, and the menu opens again so a key can be pressed again.

use super::{Host, Owner, OpenMenu, View, free_letter};
use crate::layout::{self, LayoutIntent, MENU, MIN_COLUMN, MAX_COLUMN, STEP};
use crate::plugin::{Menu, MenuItem};
use crate::view::{ColumnSpec, PaneId};

/// Between the parts of a menu item's value: what it does, the place, the heading.
const SEP: char = '\u{1f}';

fn value(op: &str, key: &str, heading: &str) -> String {
    format!("{op}{SEP}{key}{SEP}{heading}")
}

impl Host {
    /// The places of the panes showing, by [`layout::key`], with the columns
    /// each draws (headed ones only; the names of a tree are not a column).
    fn columns_by_place(&self) -> Vec<(String, PaneId, Vec<ColumnSpec>)> {
        let View::Screen(_) = self.view else { return vec![] };
        let Some((p, spec)) = self.screen() else { return vec![] };
        let Some(view) = self.screen_view(p, spec.id) else { return vec![] };
        let (panes, tab) = match view.page.as_deref() {
            Some(page) => (&page.panes, Some(page.tabs.get(page.tab).map_or("", |t| t.label.as_str()))),
            None => (&view.panes, None),
        };
        panes
            .iter()
            .filter_map(|pane| {
                let columns: Vec<ColumnSpec> =
                    layout::drawn_columns(&pane.body)?.into_iter().filter(|c| !c.heading.is_empty()).collect();
                (!columns.is_empty()).then(|| (layout::key(&view.scope, pane.pane, tab), pane.pane, columns))
            })
            .collect()
    }

    /// The place of the focused pane, when it draws columns.
    pub(super) fn columns_key_here(&self) -> Option<String> {
        let focus = PaneId::from(self.focus);
        self.columns_by_place().into_iter().find(|(_, pane, _)| *pane == focus).map(|(key, ..)| key)
    }

    fn drawn_at(&self, key: &str) -> Option<Vec<ColumnSpec>> {
        self.columns_by_place().into_iter().find(|(k, ..)| k == key).map(|(.., c)| c)
    }

    fn hidden_at(&self, key: &str) -> Vec<String> {
        self.layout.columns.get(key).map(|c| c.hidden.clone()).unwrap_or_default()
    }

    fn open_layout_menu(&mut self, title: String, items: Vec<MenuItem>) {
        self.menu = Some(OpenMenu::open(Owner::Host, Menu::new(MENU, title, items)));
    }

    /// Every column of the place: choosing one opens its heading's menu; a
    /// hidden one is shown again.
    pub(super) fn open_columns_menu(&mut self, key: &str) {
        let Some(columns) = self.drawn_at(key) else { return };
        let hidden = self.hidden_at(key);
        let mut taken = vec!['d'];
        let mut items = Vec::new();
        let letter = |words: &str, taken: &mut Vec<char>| {
            let c = free_letter(words, taken);
            taken.extend(c);
            c
        };
        for c in &columns {
            let hint = if c.flex { "takes the rest".to_string() } else { format!("{} cells", c.width) };
            let v = value("open", key, &c.heading);
            items.push(match letter(&c.heading, &mut taken) {
                Some(k) => MenuItem::new(k, c.heading.clone(), hint, v),
                None => MenuItem::pick(c.heading.clone(), hint, v),
            });
        }
        for h in &hidden {
            let label = format!("show {h}");
            let v = value("show", key, h);
            items.push(match letter(&format!("{h} show"), &mut taken) {
                Some(k) => MenuItem::new(k, label, "hidden", v),
                None => MenuItem::pick(label, "hidden", v),
            });
        }
        if self.layout.columns.contains_key(key) {
            items.push(MenuItem::new('d', "reset to the default", "the part's order and widths, every column shown", value("reset", key, "")));
        }
        self.open_layout_menu("columns".into(), items);
    }

    /// One heading's menu: what dragging it, its edge, and the rest of the
    /// pane's columns menu do.
    pub(super) fn open_heading_menu(&mut self, key: &str, heading: &str) {
        let Some(columns) = self.drawn_at(key) else { return };
        let Some(at) = columns.iter().position(|c| c.heading == heading) else { return };
        let column = &columns[at];
        let mut items = Vec::new();
        if let Some(before) = at.checked_sub(1).map(|i| &columns[i]) {
            items.push(MenuItem::new('l', "move left", format!("before {}", before.heading), value("left", key, heading)));
        }
        if let Some(after) = columns.get(at + 1) {
            items.push(MenuItem::new('r', "move right", format!("after {}", after.heading), value("right", key, heading)));
        }
        if !column.flex {
            let cells = format!("{} cells", column.width);
            if column.width < MAX_COLUMN {
                items.push(MenuItem::new('w', "wider", cells.clone(), value("wider", key, heading)));
            }
            if column.width > MIN_COLUMN {
                items.push(MenuItem::new('n', "narrower", cells, value("narrower", key, heading)));
            }
            if columns.len() > 1 {
                items.push(MenuItem::new('h', "hide", "show it again from the pane's columns", value("hide", key, heading)));
            }
        }
        let mut taken = vec!['l', 'r', 'w', 'n', 'h', 'd'];
        for h in self.hidden_at(key) {
            let label = format!("show {h}");
            let v = value("show", key, &h);
            let c = free_letter(&format!("show {h}"), &taken);
            taken.extend(c);
            items.push(match c {
                Some(k) => MenuItem::new(k, label, "hidden", v),
                None => MenuItem::pick(label, "hidden", v),
            });
        }
        if self.layout.columns.contains_key(key) {
            items.push(MenuItem::new('d', "reset to the default", "the part's order and widths, every column shown", value("reset", key, "")));
        }
        self.open_layout_menu(format!("{heading} column"), items);
    }

    /// A choice in a columns menu: kept in `[layout]`, then the menu again.
    pub(super) fn on_columns_menu(&mut self, choice: &str) {
        let mut parts = choice.splitn(3, SEP);
        let (Some(op), Some(key), Some(heading)) = (parts.next(), parts.next(), parts.next()) else { return };
        let (key, heading) = (key.to_string(), heading.to_string());
        let columns = self.drawn_at(&key).unwrap_or_default();
        let order: Vec<String> = columns.iter().map(|c| c.heading.clone()).collect();
        let width = columns.iter().find(|c| c.heading == heading).map(|c| c.width);
        let intent = match op {
            "open" => return self.open_heading_menu(&key, &heading),
            "left" | "right" => layout::moved(&order, &heading, if op == "left" { -1 } else { 1 })
                .map(|order| LayoutIntent::ColumnOrder { key: key.clone(), order }),
            "wider" => width.map(|w| LayoutIntent::ColumnWidth { key: key.clone(), heading: heading.clone(), width: Some(w.saturating_add(STEP)) }),
            "narrower" => width.map(|w| LayoutIntent::ColumnWidth { key: key.clone(), heading: heading.clone(), width: Some(w.saturating_sub(STEP)) }),
            "hide" | "show" => Some(LayoutIntent::ColumnHidden { key: key.clone(), heading: heading.clone(), hidden: op == "hide" }),
            "reset" => Some(LayoutIntent::ColumnsReset { key: key.clone() }),
            _ => None,
        };
        let Some(intent) = intent else { return };
        self.layout.on_intent(&intent);
        match op {
            "hide" | "reset" => self.open_columns_menu(&key),
            "show" if !columns.iter().any(|c| c.heading == heading) => self.open_columns_menu(&key),
            _ => self.open_heading_menu(&key, &heading),
        }
    }
}
