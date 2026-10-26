//! `[menus]`: how the row menus are customised (the gear in a row menu's
//! title edge, Settings › Menus). One table per kind of place, named by the
//! subject's kind (`[menus.book]`), holding action ids: the stable
//! `<part>.<surface>.<verb>` a part gives each row, or `<part>.<id>` for an
//! action another part adds. An id nobody offers any more is kept and
//! ignored; an action nobody has customised shows where its part puts it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The `[menus]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MenusConfig {
    /// The row's state picks the action after the default one.
    pub state_first: bool,
    /// How other parts' actions appear in a row's menu.
    pub other_parts: OtherParts,
    /// The gear shows in a row menu's title edge (`,` works either way).
    pub gear: bool,
    /// Each kind of place's changes, by the subject's kind.
    #[serde(flatten)]
    pub places: BTreeMap<String, Place>,
}

impl Default for MenusConfig {
    fn default() -> Self {
        Self {
            state_first: true,
            other_parts: OtherParts::InMenu,
            gear: true,
            places: BTreeMap::new(),
        }
    }
}

impl MenusConfig {
    pub fn place(&self, kind: &str) -> Place {
        self.places.get(kind).cloned().unwrap_or_default()
    }

    /// Keep `place` for `kind`; an unchanged one is not written.
    pub fn set_place(&mut self, kind: &str, place: Place) {
        if place == Place::default() {
            self.places.remove(kind);
        } else {
            self.places.insert(kind.to_string(), place);
        }
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Where other parts' actions go in a row's menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OtherParts {
    /// Under the owner's headings, or their part's name.
    #[default]
    InMenu,
    /// Behind one row at the end that opens them.
    Folded,
    /// Not in the row's menu (their keys still work).
    Off,
}

impl OtherParts {
    pub fn words(self) -> &'static str {
        match self {
            Self::InMenu => "in the menu",
            Self::Folded => "folded",
            Self::Off => "off",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::InMenu => Self::Folded,
            Self::Folded => Self::Off,
            Self::Off => Self::InMenu,
        }
    }
}

/// One kind of place's menu as the user arranged it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    /// First, in this order, whatever the row's state asks for.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pinned: Vec<String>,
    /// Not shown; still in the palette and `?`, and their keys work.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub hidden: Vec<String>,
    /// Behind one row at the end of the menu that opens them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub grouped: Vec<String>,
    /// The order of the actions the user moved, each among its heading's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// A key the user gave an action, in the menu and on the row.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub keys: BTreeMap<String, String>,
}

impl Place {
    /// Add `id` to `list` if it is not there, else take it out.
    pub fn toggle(list: &mut Vec<String>, id: &str) {
        match list.iter().position(|x| x == id) {
            Some(i) => {
                list.remove(i);
            }
            None => list.push(id.to_string()),
        }
    }

    /// The key the user gave `id`, if one.
    pub fn key(&self, id: &str) -> Option<char> {
        let mut chars = self.keys.get(id)?.chars();
        let c = chars.next()?;
        chars.next().is_none().then_some(c)
    }

    /// How many actions the user changed here, for Settings › Menus.
    pub fn changes(&self) -> String {
        let mut said = Vec::new();
        let n = |v: usize, one: &str, many: &str| match v {
            0 => None,
            1 => Some(format!("1 {one}")),
            v => Some(format!("{v} {many}")),
        };
        said.extend(n(self.hidden.len(), "hidden", "hidden"));
        said.extend(n(self.pinned.len(), "pinned", "pinned"));
        said.extend(n(self.grouped.len(), "grouped", "grouped"));
        said.extend(n(self.keys.len(), "key changed", "keys changed"));
        if !self.order.is_empty() {
            said.push("reordered".into());
        }
        if said.is_empty() {
            "as the parts put it".into()
        } else {
            said.join(" · ")
        }
    }
}

/// The host's own settings section: the row menus.
pub const MENUS_SECTION: crate::plugin::SectionSpec = crate::plugin::SectionSpec {
    id: "menus",
    name: "menus",
    group: "general",
};

/// A row of Settings › Menus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// A kind of place: its subject kind and its name in words.
    Place(String, String),
    StateFirst,
    OtherParts,
    Gear,
}

impl crate::host::Host {
    /// The kinds of place there are menus for: every kind a set-up part
    /// owns, then any the config names that no part owns now.
    pub fn menu_places(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (i, p) in self.plugins.iter().enumerate() {
            if !self.ready(i) {
                continue;
            }
            for o in p.owns() {
                if !out.iter().any(|(k, _)| k == o.kind) {
                    out.push((o.kind.to_string(), o.name.to_string()));
                }
            }
        }
        for kind in self.menus.places.keys() {
            if !out.iter().any(|(k, _)| k == kind) {
                out.push((kind.clone(), kind.replace('_', " ")));
            }
        }
        out
    }

    /// The section's rows, in order.
    pub fn menus_rows(&self) -> Vec<Row> {
        let mut rows: Vec<Row> = self.menu_places().into_iter().map(|(k, n)| Row::Place(k, n)).collect();
        rows.extend([Row::StateFirst, Row::OtherParts, Row::Gear]);
        rows
    }

    /// A key in Settings › Menus: `j`/`k` move, `⏎`/`space` flips a switch,
    /// `d` puts a kind of place back as the parts have it, `⏎` on a place
    /// says how to customise it.
    pub(crate) fn menus_key(&mut self, key: crate::view::Chord) -> Vec<crate::host::HostEffect> {
        use crate::view::Key;
        let rows = self.menus_rows();
        let at = self.menus_row.min(rows.len().saturating_sub(1));
        match key.key {
            Key::Down | Key::Char('j') => self.menus_row = (at + 1).min(rows.len().saturating_sub(1)),
            Key::Up | Key::Char('k') => self.menus_row = at.saturating_sub(1),
            Key::Esc => self.focus = crate::plugin::Pane::Nav,
            Key::Char('d') => {
                if let Some(Row::Place(kind, name)) = rows.get(at) {
                    self.menus.set_place(kind, Place::default());
                    self.notify(format!("{name} rows: as the parts put them").into());
                }
            }
            Key::Enter | Key::Char(' ') => match rows.get(at) {
                Some(Row::StateFirst) => self.menus.state_first = !self.menus.state_first,
                Some(Row::OtherParts) => self.menus.other_parts = self.menus.other_parts.next(),
                Some(Row::Gear) => self.menus.gear = !self.menus.gear,
                Some(Row::Place(_, name)) => {
                    self.notify(format!("right-click a {name} row, or select one and press x, then ,").into());
                }
                None => {}
            },
            _ => {}
        }
        vec![]
    }

    /// Settings › Menus's keys for its `x` menu.
    pub(crate) fn menus_bindings(&self) -> Vec<crate::plugin::Binding> {
        use crate::plugin::Binding;
        let rows = self.menus_rows();
        match rows.get(self.menus_row.min(rows.len().saturating_sub(1))) {
            Some(Row::Place(_, name)) => vec![
                Binding::heading(format!("{name} rows")),
                Binding::new("⏎", "how to customise it"),
                Binding::new("d", "put it back as the parts have it"),
            ],
            Some(_) => vec![Binding::heading("menus"), Binding::new("⏎", "change it")],
            None => vec![],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unchanged_menus_table_is_not_written_and_a_place_reads_back() {
        let mut cfg = MenusConfig::default();
        assert!(cfg.is_default());
        let mut place = Place::default();
        Place::toggle(&mut place.hidden, "shelf.books.copy_link");
        place.keys.insert("desk.check".into(), "g".into());
        cfg.set_place("book", place.clone());
        let text = toml::to_string(&cfg).unwrap();
        assert!(text.contains("[book]"), "{text}");
        let back: MenusConfig = toml::from_str(&text).unwrap();
        assert_eq!(back.place("book"), place);
        assert_eq!(back.place("book").key("desk.check"), Some('g'));
        assert_eq!(back.place("book").changes(), "1 hidden · 1 key changed");
        cfg.set_place("book", Place::default());
        assert!(cfg.is_default());
    }

    #[test]
    fn the_switches_read_with_their_defaults() {
        let cfg: MenusConfig = toml::from_str("other_parts = \"folded\"").unwrap();
        assert!(cfg.state_first && cfg.gear);
        assert_eq!(cfg.other_parts, OtherParts::Folded);
    }
}
