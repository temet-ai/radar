//! `[layout]`: what the user sized and arranged with the mouse, kept across
//! runs. The split between the panes is one for every screen of the same
//! shape (widening the detail pane in one part widens it in all), kept as
//! thousandths of the width so it survives a terminal resize. A table's or
//! tree's columns are kept per place ([`key`]): their order, the widths
//! dragged and the ones hidden, matched by heading, so no part knows of it.
//! A column a part adds later goes where the part puts it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::view::{ColumnSpec, Node, PaneId, PaneView, ScreenView};

/// Least and most cells a dragged column may take (with its cell of air).
pub const MIN_COLUMN: u16 = 3;
pub const MAX_COLUMN: u16 = 120;
/// Least cells a tree's names may be dragged to.
pub const MIN_NAMES: u16 = 8;
/// The id of the host's columns menus (a pane's columns, one heading's).
pub const MENU: &str = "layout";
/// Cells a column grows or shrinks by from its heading's menu.
pub const STEP: u16 = 2;

/// The `[layout]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LayoutConfig {
    /// The nav pane's share of the width, in thousandths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nav: Option<u16>,
    /// The detail pane's share of the width, in thousandths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<u16>,
    /// The agent dock's height in rows ([`crate::dock::DockState::rows`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dock: Option<u16>,
    /// Each place's columns, by [`key`].
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub columns: BTreeMap<String, Columns>,
}

/// One place's columns, by heading.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Columns {
    /// The order they were put in; a heading not named keeps its place
    /// after the one before it in the part's order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Widths dragged, in cells with the cell of air after the value.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub widths: BTreeMap<String, u16>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    /// A tree's names, dragged to this width ([`crate::view::Tree::names`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub names: Option<u16>,
}

/// Which border between the panes a drag moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum SplitEdge {
    /// Between the nav pane and the one right of it.
    Nav,
    /// Between the middle pane and the detail pane.
    Detail,
}

/// What a drag or a heading's menu changed. `key` names the place
/// ([`key`]); a renderer computes it the same way.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "layout", rename_all = "snake_case")]
pub enum LayoutIntent {
    /// A pane border dragged to `share` thousandths of the width; `None`
    /// puts the split back to its default (a double click).
    Split { edge: SplitEdge, share: Option<u16> },
    /// A column's edge dragged to `width` cells; `None` puts it back.
    ColumnWidth { key: String, heading: String, width: Option<u16> },
    /// A heading dropped somewhere else: the columns shown, in their new order.
    ColumnOrder { key: String, order: Vec<String> },
    ColumnHidden { key: String, heading: String, hidden: bool },
    /// A tree's names dragged to `width` cells; `None` lets them take what
    /// the columns leave again.
    NamesWidth { key: String, width: Option<u16> },
    /// The place's columns as the part has them.
    ColumnsReset { key: String },
    /// A right-click on a heading: the menu that moves, sizes and hides
    /// it, as the mouse does. Changes nothing kept.
    HeadingMenu { key: String, heading: String },
}

/// The split the renderer draws ([`ScreenView::layout`]): absent while
/// nothing was dragged.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct LayoutView {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nav: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<u16>,
}

/// The place a pane's columns are kept under: `<screen>/<pane>`, or on an
/// item's page `<screen>/page/<tab>/<pane>` (the tab's name in lower case,
/// `page` alone when it has no tabs). Ink's `layoutKey` is the same.
pub fn key(scope: &str, pane: PaneId, tab: Option<&str>) -> String {
    let pane = match pane {
        PaneId::Nav => "nav",
        PaneId::Middle => "middle",
        PaneId::Detail => "detail",
    };
    match tab {
        None => format!("{scope}/{pane}"),
        Some("") => format!("{scope}/page/{pane}"),
        Some(tab) => format!("{scope}/page/{}/{pane}", tab.to_lowercase()),
    }
}

impl LayoutConfig {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Lay what was kept over a view the part made.
    pub fn apply(&self, view: &mut ScreenView) {
        if self.nav.is_some() || self.detail.is_some() {
            view.layout = Some(LayoutView { nav: self.nav, detail: self.detail });
        }
        if self.columns.is_empty() {
            return;
        }
        let scope = view.scope.clone();
        for pane in &mut view.panes {
            self.arrange_pane(&key(&scope, pane.pane, None), pane);
        }
        if let Some(page) = view.page.as_deref_mut() {
            let tab = page.tabs.get(page.tab).map_or(String::new(), |t| t.label.clone());
            for pane in &mut page.panes {
                self.arrange_pane(&key(&scope, pane.pane, Some(&tab)), pane);
            }
        }
    }

    fn arrange_pane(&self, key: &str, pane: &mut PaneView) {
        let Some(kept) = self.columns.get(key) else {
            return;
        };
        match &mut pane.body {
            Node::Table { columns, .. } => {
                arrange(columns, kept);
            }
            Node::Tree(tree) => {
                if !tree.ordered_columns && !kept.order.is_empty() {
                    // The renderer puts other parts' columns before the
                    // part's last; an order the user chose is drawn as it is.
                    let last = tree.columns.iter().rposition(|c| !c.extra);
                    if let Some(last) = last {
                        let own_last = tree.columns.remove(last);
                        let at = tree.columns.iter().rposition(|c| !c.extra).map_or(0, |i| i + 1);
                        let extras: Vec<_> = tree.columns.drain(at..).collect();
                        tree.columns.extend(extras);
                        tree.columns.push(own_last);
                    }
                    tree.ordered_columns = true;
                }
                arrange(&mut tree.columns, kept);
                tree.names = kept.names;
            }
            _ => {}
        }
    }

    /// Take what a drag or a heading's menu changed.
    pub fn on_intent(&mut self, intent: &LayoutIntent) {
        match intent {
            LayoutIntent::Split { edge, share } => {
                let share = share.map(|s| s.clamp(50, 800));
                match edge {
                    SplitEdge::Nav => self.nav = share,
                    SplitEdge::Detail => self.detail = share,
                }
            }
            LayoutIntent::ColumnWidth { key, heading, width } => {
                let c = self.columns.entry(key.clone()).or_default();
                match width {
                    Some(w) => {
                        c.widths.insert(heading.clone(), (*w).clamp(MIN_COLUMN, MAX_COLUMN));
                    }
                    None => {
                        c.widths.remove(heading);
                    }
                }
            }
            LayoutIntent::ColumnOrder { key, order } => {
                self.columns.entry(key.clone()).or_default().order = order.clone();
            }
            LayoutIntent::ColumnHidden { key, heading, hidden } => {
                let c = self.columns.entry(key.clone()).or_default();
                c.hidden.retain(|h| h != heading);
                if *hidden {
                    c.hidden.push(heading.clone());
                }
            }
            LayoutIntent::NamesWidth { key, width } => {
                self.columns.entry(key.clone()).or_default().names = width.map(|w| w.clamp(MIN_NAMES, MAX_COLUMN));
            }
            LayoutIntent::ColumnsReset { key } => {
                self.columns.remove(key);
            }
            LayoutIntent::HeadingMenu { .. } => {}
        }
        self.columns.retain(|_, c| *c != Columns::default());
    }
}

/// The columns `body` draws, in the order it draws them: a table's as they
/// are; a tree's other parts' columns before the part's last, unless the
/// tree is drawn in its declared order. `None` for a body without columns.
pub fn drawn_columns(body: &Node) -> Option<Vec<ColumnSpec>> {
    let columns = match body {
        Node::Table { columns, .. } => columns.clone(),
        Node::Tree(tree) if tree.ordered_columns => tree.columns.clone(),
        Node::Tree(tree) => {
            let mut columns = tree.columns.clone();
            if let Some(last) = columns.iter().rposition(|c| !c.extra) {
                let own_last = columns.remove(last);
                columns.push(own_last);
            }
            columns
        }
        _ => return None,
    };
    (!columns.is_empty()).then_some(columns)
}

/// `order` with `heading` moved `by` places (-1 left, 1 right); `None` at
/// either end or when it is not there.
pub fn moved(order: &[String], heading: &str, by: isize) -> Option<Vec<String>> {
    let at = order.iter().position(|h| h == heading)?;
    let to = at.checked_add_signed(by).filter(|to| *to < order.len())?;
    let mut out = order.to_vec();
    out.swap(at, to);
    Some(out)
}

/// Put `columns` in the kept order, at the kept widths, without the hidden
/// ones. The column that takes what is left (a title) is never hidden, and
/// one column always stays.
fn arrange(columns: &mut Vec<ColumnSpec>, kept: &Columns) {
    if !kept.order.is_empty() {
        let mut out: Vec<ColumnSpec> = kept
            .order
            .iter()
            .filter_map(|h| columns.iter().find(|c| &c.heading == h).cloned())
            .collect();
        for (i, c) in columns.iter().enumerate() {
            if out.iter().any(|o| o.heading == c.heading) {
                continue;
            }
            let at = match i.checked_sub(1) {
                Some(prev) => out
                    .iter()
                    .position(|o| o.heading == columns[prev].heading)
                    .map_or(0, |p| p + 1),
                None => 0,
            };
            out.insert(at, c.clone());
        }
        *columns = out;
    }
    for c in columns.iter_mut() {
        if let Some(w) = kept.widths.get(&c.heading) {
            let w = (*w).clamp(MIN_COLUMN, MAX_COLUMN);
            if w != c.width {
                c.natural = Some(c.width);
                c.width = w;
            }
        }
    }
    if !kept.hidden.is_empty() {
        let shown = |c: &ColumnSpec| c.flex || !kept.hidden.contains(&c.heading);
        if columns.iter().any(shown) {
            columns.retain(shown);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Tree;

    fn cols(names: &[&str]) -> Vec<ColumnSpec> {
        names.iter().map(|n| ColumnSpec::new(*n, 8, 0)).collect()
    }

    fn names(columns: &[ColumnSpec]) -> Vec<&str> {
        columns.iter().map(|c| c.heading.as_str()).collect()
    }

    #[test]
    fn a_kept_order_wins_and_a_new_column_follows_its_part_neighbour() {
        let mut c = cols(&["state", "approvals", "threads", "updated", "for you"]);
        let kept = Columns {
            order: vec!["updated".into(), "state".into(), "threads".into()],
            ..Columns::default()
        };
        arrange(&mut c, &kept);
        // approvals came after state, for you after updated
        assert_eq!(names(&c), ["updated", "for you", "state", "approvals", "threads"]);
    }

    #[test]
    fn widths_are_kept_within_bounds_and_hidden_columns_go_but_never_the_last() {
        let mut c = cols(&["state", "updated"]);
        c.push(ColumnSpec::flex("title", 8));
        let kept = Columns {
            widths: BTreeMap::from([("state".into(), 500), ("updated".into(), 1)]),
            hidden: vec!["updated".into(), "title".into()],
            ..Columns::default()
        };
        arrange(&mut c, &kept);
        assert_eq!(names(&c), ["state", "title"]);
        assert_eq!((c[0].width, c[0].natural), (MAX_COLUMN, Some(8)), "the part's width goes with it");
        let mut one = cols(&["state"]);
        arrange(&mut one, &Columns { hidden: vec!["state".into()], ..Columns::default() });
        assert_eq!(names(&one), ["state"]);
    }

    #[test]
    fn intents_change_the_table_and_an_undone_place_is_not_kept() {
        let mut l = LayoutConfig::default();
        let key = key("watch", PaneId::Middle, None);
        l.on_intent(&LayoutIntent::ColumnWidth { key: key.clone(), heading: "state".into(), width: Some(30) });
        l.on_intent(&LayoutIntent::ColumnHidden { key: key.clone(), heading: "for you".into(), hidden: true });
        assert_eq!(l.columns[&key].widths["state"], 30);
        assert_eq!(l.columns[&key].hidden, ["for you"]);
        l.on_intent(&LayoutIntent::ColumnWidth { key: key.clone(), heading: "state".into(), width: None });
        l.on_intent(&LayoutIntent::ColumnHidden { key: key.clone(), heading: "for you".into(), hidden: false });
        assert!(l.is_default());
        l.on_intent(&LayoutIntent::Split { edge: SplitEdge::Detail, share: Some(2000) });
        assert_eq!(l.detail, Some(800));
    }

    #[test]
    fn the_table_round_trips_through_toml() {
        let mut l = LayoutConfig { nav: Some(150), dock: Some(12), ..LayoutConfig::default() };
        l.columns.insert(
            "watch/middle".into(),
            Columns { order: vec!["".into(), "state".into()], widths: BTreeMap::from([("state".into(), 24)]), hidden: vec![], names: Some(40) },
        );
        let text = toml::to_string(&l).unwrap();
        assert_eq!(toml::from_str::<LayoutConfig>(&text).unwrap(), l);
    }

    #[test]
    fn a_tree_with_an_order_draws_it_as_kept() {
        let mut columns = cols(&["state", "updated"]);
        columns.insert(1, ColumnSpec::extra("patrol", 8));
        let mut view = ScreenView::new(
            "watch",
            vec![PaneView::new(
                PaneId::Middle,
                "repositories",
                Node::Tree(Tree { columns, ..Tree::default() }),
            )],
        );
        let mut l = LayoutConfig::default();
        l.on_intent(&LayoutIntent::ColumnOrder { key: "watch/middle".into(), order: vec!["updated".into(), "patrol".into(), "state".into()] });
        l.on_intent(&LayoutIntent::NamesWidth { key: "watch/middle".into(), width: Some(2) });
        l.apply(&mut view);
        let Node::Tree(tree) = &view.panes[0].body else { panic!() };
        assert!(tree.ordered_columns);
        assert_eq!(tree.names, Some(MIN_NAMES), "the names keep a least width");
        assert_eq!(names(&tree.columns), ["updated", "patrol", "state"]);
        assert!(view.layout.is_none());
    }

    #[test]
    fn a_heading_moves_one_place_and_stops_at_either_end() {
        let order: Vec<String> = ["state", "updated", "for you"].map(String::from).into();
        assert_eq!(moved(&order, "updated", -1).unwrap(), ["updated", "state", "for you"]);
        assert_eq!(moved(&order, "updated", 1).unwrap(), ["state", "for you", "updated"]);
        assert_eq!(moved(&order, "state", -1), None);
        assert_eq!(moved(&order, "for you", 1), None);
        assert_eq!(moved(&order, "patrol", 1), None);
    }

    #[test]
    fn a_tree_draws_other_parts_columns_before_its_last() {
        let mut columns = cols(&["state", "updated"]);
        columns.push(ColumnSpec::extra("patrol", 8));
        let tree = Tree { columns, ..Tree::default() };
        assert_eq!(names(&drawn_columns(&Node::Tree(tree.clone())).unwrap()), ["state", "patrol", "updated"]);
        let ordered = Tree { ordered_columns: true, ..tree };
        assert_eq!(names(&drawn_columns(&Node::Tree(ordered)).unwrap()), ["state", "updated", "patrol"]);
        assert!(drawn_columns(&Node::Tree(Tree::default())).is_none());
    }

    #[test]
    fn page_keys_name_the_tab() {
        assert_eq!(key("watch", PaneId::Middle, Some("Night Shift")), "watch/page/night shift/middle");
        assert_eq!(key("watch", PaneId::Detail, Some("")), "watch/page/detail");
    }
}
