//! Host-only palette state; ordinary menu action routing remains authoritative.
use super::*;
use crate::palette::{highlights, score};

pub(super) const ID: &str = "command-palette";
const DISCOVERY: usize = 12;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(super) enum Place {
    Subject(usize, String),
    Screen(usize),
    Settings,
}

#[derive(Clone)]
enum Destination {
    Action,
    Subject(usize, Subject),
    Screen(usize),
    Settings,
}

struct Entry {
    item: MenuItem,
    words: Vec<String>,
    destination: Destination,
    place: Option<Place>,
}

pub(super) struct State {
    entries: Vec<Entry>,
    matched: usize,
    generation: u64,
    pub(super) history: Option<crate::history::History>,
    route: (View, Pane, bool, bool, bool),
}

pub(super) fn shortcut(key: Chord) -> bool {
    key.key == Key::Char('p') && key.ctrl && !key.alt
}

impl Host {
    pub(super) fn open_palette(&mut self) {
        // Existing questions, typed menus and drafts retain their ownership. This
        // chord never drops or commits text merely to navigate somewhere else.
        if self.menu.is_some() || self.capturing() || self.setup.is_some() || self.dock.focused {
            return;
        }
        let history = self.history.take();
        self.open_keys_menu();
        let actions = self.menu.take().expect("keys menu was opened");
        let mut entries = Vec::new();
        if history.is_none() {
            for item in actions.menu.items {
                if item.heading || item.note {
                    continue;
                }
                entries.push(Entry {
                    words: vec![item.label.clone()],
                    item,
                    destination: Destination::Action,
                    place: None,
                });
            }
        }
        let action_count = entries.len();
        let mut seen = std::collections::HashSet::new();
        for p in 0..self.plugins.len() {
            if !self.ready(p) {
                continue;
            }
            for target in self.plugins[p].palette_targets() {
                let place = Place::Subject(p, target.identity.clone());
                if !seen.insert(place.clone()) {
                    continue;
                }
                let mut words = target.words;
                words.extend([
                    target.subject.label.clone(),
                    self.plugins[p].name().to_string(),
                ]);
                let mut item = MenuItem::pick(&target.subject.label, self.plugins[p].name(), "");
                item.icon = target.icon;
                entries.push(Entry {
                    item,
                    words,
                    destination: Destination::Subject(p, target.subject),
                    place: Some(place),
                });
            }
        }
        for (i, (p, screen)) in self.screens.iter().enumerate() {
            if !self.screen_shown(i) {
                continue;
            }
            let label = format!("{} / {}", self.plugins[*p].name(), screen.name);
            entries.push(Entry {
                item: MenuItem::pick(&label, "screen", "").shown_as(screen.key.to_string()),
                words: vec![label],
                destination: Destination::Screen(i),
                place: Some(Place::Screen(i)),
            });
        }
        entries.push(Entry {
            item: MenuItem::pick("settings", "screen", "").shown_as(","),
            words: vec!["settings".into()],
            destination: Destination::Settings,
            place: Some(Place::Settings),
        });
        // Stable cache order breaks equal-frequency ties, never hash-map order.
        entries[action_count..].sort_by_key(|entry| {
            std::cmp::Reverse(
                entry
                    .place
                    .as_ref()
                    .and_then(|place| self.palette_visits.get(place))
                    .copied()
                    .unwrap_or(0),
            )
        });
        // Action values still point into the captured keys-menu dispatch table;
        // palette indices are separate and never mistaken for plugin menu values.
        self.palette = Some(State {
            entries,
            matched: 0,
            generation: 0,
            history,
            route: (self.view, self.focus, self.bar, self.sources, self.tabs),
        });
        self.menu = Some(OpenMenu::open(
            Owner::Host,
            Menu::typed(ID, "command palette", ""),
        ));
        self.palette_changed();
    }

    pub(super) fn palette_changed(&mut self) {
        let (Some(state), Some(open)) = (&mut self.palette, &mut self.menu) else {
            return;
        };
        if open.owner != Owner::Host || open.menu.id != ID {
            return;
        }
        self.palette_generation = self
            .palette_generation
            .checked_add(1)
            .expect("palette generation exhausted");
        state.generation = self.palette_generation;
        self.hits.borrow_mut().clear();
        *self.hovered.borrow_mut() = None;
        let query = open
            .input
            .as_ref()
            .map(|input| input.value())
            .unwrap_or_default();
        let mut results: Vec<_> = state
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let rank = score(&query, &entry.words);
                (query.trim().is_empty() || rank > 0).then_some((i, rank))
            })
            .collect();
        if !query.trim().is_empty() {
            results.sort_by_key(|(_, rank)| std::cmp::Reverse(*rank));
        } else {
            let actions: Vec<_> = results
                .iter()
                .copied()
                .filter(|(i, _)| state.entries[*i].place.is_none())
                .take(DISCOVERY / 2)
                .collect();
            let places = results
                .iter()
                .copied()
                .filter(|(i, _)| state.entries[*i].place.is_some());
            results = actions.into_iter().chain(places).collect();
        }
        state.matched = if query.trim().is_empty() {
            state.entries.len()
        } else {
            results.len()
        };
        let limit = if query.trim().is_empty() {
            DISCOVERY
        } else {
            crate::view::chrome::MOST
        };
        open.menu.items = results
            .into_iter()
            .take(limit)
            .map(|(i, _)| {
                let mut item = state.entries[i].item.clone();
                item.value = format!("palette:{}:{i}", state.generation);
                item.key = None;
                item
            })
            .collect();
        open.row = if open.menu.items.is_empty() {
            NO_ROW
        } else {
            0
        };
        open.viewport.at.set(0);
        open.viewport.selected.set(None);
    }

    pub(super) fn clear_palette_hits(&mut self) {
        self.hits.borrow_mut().clear();
        *self.hovered.borrow_mut() = None;
        self.last_press = None;
    }

    pub(super) fn cancel_palette(&mut self) {
        self.clear_palette_hits();
        if let Some(state) = self.palette.take() {
            self.history = state.history;
        }
        self.menu = None;
        self.menu_keys.clear();
        self.cancel_menu_trace(true);
    }

    pub(super) fn palette_key(&mut self, key: Chord) -> Vec<HostEffect> {
        if key.key == Key::Esc || shortcut(key) {
            self.cancel_palette();
            return vec![];
        }
        let Some(open) = self.menu.as_mut() else {
            return vec![];
        };
        match key.key {
            Key::Up | Key::Down | Key::Tab | Key::BackTab => {
                if !open.menu.items.is_empty() {
                    let forward = matches!(key.key, Key::Down | Key::Tab);
                    let n = open.menu.items.len();
                    open.row = if open.row >= n {
                        0
                    } else if forward {
                        (open.row + 1) % n
                    } else {
                        (open.row + n - 1) % n
                    };
                }
                return vec![];
            }
            Key::Enter => return self.choose_palette(),
            _ => {}
        }
        if let Some(input) = open.input.as_mut() {
            input.key(key);
        }
        self.palette_changed();
        vec![]
    }

    fn choose_palette(&mut self) -> Vec<HostEffect> {
        let (Some(open), Some(state)) = (&self.menu, &self.palette) else {
            return vec![];
        };
        if !open.viewport.usable.get() {
            return vec![];
        }
        let Some(entry) = open
            .menu
            .items
            .get(open.row)
            .and_then(|item| item.value.rsplit(':').next()?.parse::<usize>().ok())
            .and_then(|index| state.entries.get(index))
        else {
            return vec![];
        };
        let destination = entry.destination.clone();
        let place = entry.place.clone();
        let original_value = entry.item.value.clone();
        let route = state.route;
        match destination {
            Destination::Action => {
                if route != (self.view, self.focus, self.bar, self.sources, self.tabs) {
                    self.cancel_palette();
                    self.notify_error("that target changed; open the palette again");
                    return vec![];
                }
                self.clear_palette_hits();
                self.palette = None;
                let open = self.menu.as_mut().expect("palette is open");
                open.menu.id = KEYS_MENU.into();
                open.menu.typed = None;
                open.input = None;
                open.menu.items[open.row].value = original_value;
                self.menu_key(Chord::new(Key::Enter))
            }
            Destination::Subject(p, subject) => {
                let current = self.ready(p)
                    && self.plugins[p].palette_targets().iter().any(|target| {
                        Some(Place::Subject(p, target.identity.clone())) == place
                            && target.subject == subject
                            && target.subject.url == subject.url
                            && target.subject.fields == subject.fields
                    });
                if !current {
                    self.cancel_palette();
                    self.notify_error("that destination changed; open the palette again");
                    return vec![];
                }
                self.finish_palette_navigation();
                self.show_owned(p, subject, 0, false)
            }
            Destination::Screen(i) => {
                if !self.screen_shown(i) {
                    self.cancel_palette();
                    self.notify_error("that screen changed; open the palette again");
                    return vec![];
                }
                self.finish_palette_navigation();
                self.view = View::Screen(i);
                self.back.clear();
                self.leave_sources();
                vec![]
            }
            Destination::Settings => {
                self.finish_palette_navigation();
                self.view = View::Settings;
                self.back.clear();
                vec![]
            }
        }
    }

    fn finish_palette_navigation(&mut self) {
        self.clear_palette_hits();
        self.palette = None;
        self.menu = None;
        self.menu_keys.clear();
        self.complete_menu_trace(false);
        self.bar = false;
        self.sources = false;
        self.tabs = false;
        self.dock.focused = false;
    }

    pub(super) fn palette_row(&self, value: &str) -> Option<usize> {
        self.palette.as_ref()?;
        let open = self.menu.as_ref()?;
        if open.owner != Owner::Host || open.menu.id != ID {
            return None;
        }
        open.menu.items.iter().position(|item| item.value == value)
    }

    pub(super) fn palette_view(&self) -> Option<crate::view::chrome::PaletteView> {
        let state = self.palette.as_ref()?;
        let open = self.menu.as_ref()?;
        let query = open
            .input
            .as_ref()
            .map(|input| input.value())
            .unwrap_or_default();
        Some(crate::view::chrome::PaletteView {
            matched: state.matched,
            total: state.entries.len(),
            highlights: open
                .menu
                .items
                .iter()
                .map(|item| highlights(&item.label, &query))
                .collect(),
            hint_highlights: open
                .menu
                .items
                .iter()
                .map(|item| highlights(&item.hint, &query))
                .collect(),
        })
    }

    pub(super) fn visit_palette_subject(&mut self, owner: usize, subject: &Subject) {
        let targets = self.plugins[owner].palette_targets();
        let mut matches = targets.iter().filter(|target| {
            target.subject == *subject
                && target
                    .subject
                    .fields
                    .iter()
                    .all(|(key, value)| subject.fields.get(key) == Some(value))
        });
        if let Some(target) = matches.next()
            && matches.next().is_none()
        {
            self.visit_palette_place(Place::Subject(owner, target.identity.clone()));
        }
    }

    pub(super) fn visit_palette_place(&mut self, place: Place) {
        const MAX_VISITS: usize = 1024;
        if self.palette_visits.len() >= MAX_VISITS && !self.palette_visits.contains_key(&place) {
            let least = self
                .palette_visits
                .iter()
                .min_by_key(|(key, count)| (**count, (*key).clone()))
                .map(|(key, _)| key.clone());
            if let Some(key) = least {
                self.palette_visits.remove(&key);
            }
        }
        let count = self.palette_visits.entry(place).or_default();
        *count = count.saturating_add(1);
    }
}
