//! Unit-level checks a part runs over its own plugin: no host, no frame, no
//! terminal; milliseconds per place (GUIDE 2.5, 3.2). They read the part's
//! view data ([`crate::view`]) and press keys and choose targets as
//! intents, as the host and Ink do.
//!
//! At a [`Place`] (a screen and pane, or a settings section, and the keys
//! that bring a fresh plugin there), [`Check::place`] checks:
//!
//! - **every row of the `x` menu** (the view's actions, else
//!   `Plugin::keys` / `Plugin::section_keys`): its key is not one the host
//!   keeps ([`host_keeps`]: the part would never see it), and pressed on a
//!   fresh plugin at the same place the part takes it (`Outcome::Handled`),
//!   unless it is a key the host acts on when the part does not
//!   ([`host_falls_back`]: `esc`, arrows, `PgUp`/`PgDn`, `⏎` on the nav
//!   pane, `+`/`-`).
//! - **every target** the view offers (a row, a tab): chosen on a fresh
//!   plugin, the part takes it.
//!
//! [`Check::actions`] checks what a part adds to another's `x` menu
//! (`Point::Actions`): each action, acted on a fresh plugin, returns effects.

use crate::compose::{Contribution, Parts, Point, Subject};
use crate::headless::{node_targets, offered};
use crate::plugin::{Binding, Outcome, Pane, Plugin};
use crate::view::{ActionSpec, Chord, Intent, Key, Node, PaneId, ScreenView, SectionView, Target, TreeRow, ViewCx};

/// Where a place is: a screen (by id) with a pane focused, or a settings
/// section (by id).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Screen(&'static str, Pane),
    Section(&'static str),
}

/// A place: a scope, and the keys pressed on the way from a fresh plugin,
/// each with the pane that had focus when it was pressed.
#[derive(Debug, Clone)]
pub struct Place {
    pub scope: Scope,
    pub keys: Vec<(Pane, Chord)>,
}

impl Place {
    /// A screen, its `pane` focused, no key pressed yet.
    pub fn screen(id: &'static str, pane: Pane) -> Self {
        Self { scope: Scope::Screen(id, pane), keys: vec![] }
    }

    /// A settings section, no key pressed yet.
    pub fn section(id: &'static str) -> Self {
        Self { scope: Scope::Section(id), keys: vec![] }
    }

    /// `key` pressed `n` times in the focused pane.
    pub fn press(mut self, key: impl Into<Chord>, n: usize) -> Self {
        let pane = self.pane();
        self.keys.extend(std::iter::repeat_n((pane, key.into()), n));
        self
    }

    /// `↓` `n` times.
    pub fn down(self, n: usize) -> Self {
        self.press(Chord::new(Key::Down), n)
    }

    /// `⏎` once.
    pub fn enter(self) -> Self {
        self.press(Chord::new(Key::Enter), 1)
    }

    /// `]` `n` times: a page's next tab.
    pub fn tab(self, n: usize) -> Self {
        self.press(Chord::char(']'), n)
    }

    /// Characters typed, one key each.
    pub fn typed(mut self, text: &str) -> Self {
        for c in text.chars() {
            self = self.press(Chord::char(c), 1);
        }
        self
    }

    /// Focus `pane` (the host's `1` `2` `3`); a section has one pane.
    pub fn focus(mut self, pane: Pane) -> Self {
        if let Scope::Screen(id, _) = self.scope {
            self.scope = Scope::Screen(id, pane);
        }
        self
    }

    fn pane(&self) -> Pane {
        match self.scope {
            Scope::Screen(_, pane) => pane,
            Scope::Section(_) => Pane::Middle,
        }
    }
}

impl std::fmt::Display for Place {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let keys: Vec<String> = self.keys.iter().map(|(_, k)| k.name()).collect();
        write!(f, "{:?} after [{}]", self.scope, keys.join(" "))
    }
}

/// Keys the host takes before a part sees them, when nothing is being
/// typed: `q` `x` `?` `b` `w`, the settings key, `tab`, `h` `l` `←` `→` and
/// digits (panes). A part's `x` row must not use one. (A capital is kept
/// only when it is a screen's key in the mix; `tests/mix.rs` checks those.)
pub fn host_keeps(key: Chord) -> bool {
    if key.ctrl {
        return false;
    }
    match key.key {
        Key::Tab | Key::BackTab | Key::Left | Key::Right => true,
        Key::Char(c) => {
            matches!(c, 'q' | 'x' | '?' | 'b' | 'w' | 'h' | 'l' | '1'..='9' | crate::host::SETTINGS_KEY | crate::host::HISTORY_KEY)
        }
        _ => false,
    }
}

/// Keys the host acts on itself when the part does not take them: `esc`
/// goes back, arrows reach the bar or say there is nowhere to go,
/// `PgUp`/`PgDn` say there is nothing to scroll, `+`/`-` say there is
/// nothing to fold, and `⏎` on the nav pane goes into the next pane.
pub fn host_falls_back(key: Chord, pane: Pane) -> bool {
    match key.key {
        Key::Esc | Key::Up | Key::Down | Key::PageUp | Key::PageDown => true,
        Key::Char('+' | '=' | '-') => true,
        Key::Enter => pane == Pane::Nav,
        _ => false,
    }
}

/// What a place shows, read from the part's view data.
struct Shown {
    /// The `x` menu's rows.
    rows: Vec<Binding>,
    /// The targets the view offers, with the pane that shows each.
    targets: Vec<(PaneId, Target)>,
}

/// The checks over plugins made by `make`, each read at `size`.
pub struct Check<'a> {
    make: &'a dyn Fn() -> Box<dyn Plugin>,
    pub size: (u16, u16),
    pub now: i64,
    /// What did nothing, one line each.
    pub gaps: Vec<String>,
    /// Rows, targets and actions tried.
    pub checked: usize,
    /// Rows and targets already tried, by scope, page depth and words: the
    /// same row on the next tree row of its kind is tried once.
    seen: std::collections::HashSet<String>,
}

fn bindings(actions: &[ActionSpec]) -> Vec<Binding> {
    actions
        .iter()
        .map(|a| {
            if a.heading {
                Binding::heading(a.label.clone())
            } else {
                Binding::new(a.key.map_or_else(|| a.id.clone(), |c| c.name()), a.label.clone())
            }
        })
        .collect()
}

/// Where a key goes in a screen's view: movement along the focused pane's
/// rows selects through the part, `⏎` on the selected row chooses it, and
/// anything else is the part's own.
fn screen_key(view: &ScreenView, pane: PaneId, chord: Chord) -> Intent {
    let panes = view.page.as_ref().map_or(view.panes.as_slice(), |p| p.panes.as_slice());
    let Some(pane) = panes.iter().find(|p| p.pane == pane) else {
        return Intent::Key { chord };
    };
    let key = Intent::Key { chord };
    let enter = chord == Chord::new(Key::Enter);
    match &pane.body {
        Node::List { rows, selected: Some(at), editing: None } => {
            if let Some(i) = crate::view::rows_step(rows, *at, chord) {
                return Intent::Select { target: rows[i].target.clone() };
            }
            match rows.get(*at) {
                Some(r) if enter => Intent::Activate { target: r.target.clone() },
                _ => key,
            }
        }
        Node::Table { rows, selected: Some(at), .. } => {
            if let Some(i) = crate::view::list_step(rows.len(), *at, chord) {
                return Intent::Select { target: rows[i].target.clone() };
            }
            match rows.get(*at) {
                Some(r) if enter => Intent::Activate { target: r.target.clone() },
                _ => key,
            }
        }
        Node::Entries { rows, selected: Some(at), .. } => {
            if let Some(i) = crate::view::list_step(rows.len(), *at, chord) {
                return Intent::Select { target: rows[i].target.clone() };
            }
            match rows.get(*at) {
                Some(r) if enter => Intent::Activate { target: r.target.clone() },
                _ => key,
            }
        }
        Node::Tree(tree) if tree.editing.is_none() => {
            if let Some(i) = crate::view::tree_step(tree, chord) {
                return i;
            }
            if enter && !tree.selected.is_empty() {
                return Intent::Activate { target: TreeRow::target_of(&tree.selected) };
            }
            key
        }
        _ => key,
    }
}

/// A key in a settings section's view: movement along its rows selects
/// through the part; anything else is passed on.
fn section_key(view: &SectionView, chord: Chord) -> Intent {
    if let Node::List { rows, selected, editing: None } = &view.body
        && !rows.is_empty()
    {
        let at = selected.unwrap_or(0);
        let up = matches!(chord.key, Key::Up | Key::Char('k')) && !chord.ctrl && !chord.alt;
        if let Some(i) = crate::view::rows_step(rows, at, chord).or_else(|| up.then_some(at)) {
            return Intent::Select { target: rows[i].target.clone() };
        }
    }
    if let Node::Tree(tree) = &view.body
        && tree.editing.is_none()
    {
        if let Some(i) = crate::view::tree_step(tree, chord) {
            return i;
        }
        if matches!(chord.key, Key::Up | Key::Char('k')) && !chord.ctrl && !chord.alt && !chord.shift && tree.selectable(&tree.selected) {
            return Intent::Select { target: TreeRow::target_of(&tree.selected) };
        }
    }
    Intent::Key { chord }
}

impl<'a> Check<'a> {
    pub fn new(make: &'a dyn Fn() -> Box<dyn Plugin>) -> Self {
        Check {
            make,
            size: (120, 32),
            now: crate::plugin::DEMO_NOW,
            gaps: vec![],
            checked: 0,
            seen: std::collections::HashSet::new(),
        }
    }

    fn id_of(scope: Scope) -> &'static str {
        match scope {
            Scope::Screen(id, _) | Scope::Section(id) => id,
        }
    }

    fn cx(&self, pane: Pane) -> ViewCx<'static> {
        ViewCx {
            now: self.now,
            focus: pane.into(),
            width: self.size.0,
            height: self.size.1,
            parts: Parts::none(),
            icon_lead: 2,
            setup: false,
        }
    }

    /// What the part shows at `scope` with `pane` focused: its `x` menu and
    /// the targets its view offers.
    fn shown(&self, p: &dyn Plugin, scope: Scope, pane: Pane) -> Shown {
        let cx = self.cx(pane);
        match scope {
            Scope::Screen(id, _) => match p.view(id, &cx) {
                Some(v) => Shown { rows: bindings(&v.actions), targets: offered(&v).into_iter().map(|o| (o.pane, o.target)).collect() },
                None => Shown { rows: p.keys(id, pane), targets: vec![] },
            },
            Scope::Section(id) => match p.section_view(id, &cx) {
                Some(v) => Shown {
                    rows: bindings(&v.actions),
                    targets: node_targets(&v.body).into_iter().map(|t| (PaneId::Middle, t)).collect(),
                },
                None => Shown { rows: p.section_keys(id), targets: vec![] },
            },
        }
    }

    /// The targets the part's view offers at `scope`, with the pane that
    /// shows each.
    pub fn targets(&self, p: &dyn Plugin, scope: Scope) -> Vec<(PaneId, Target)> {
        let pane = match scope {
            Scope::Screen(_, pane) => pane,
            Scope::Section(_) => Pane::Middle,
        };
        self.shown(p, scope, pane).targets
    }

    /// `key` pressed in `pane`, routed on the part's view as the host does.
    fn press(&self, p: &mut dyn Plugin, scope: Scope, pane: Pane, key: Chord) -> Outcome {
        let cx = self.cx(pane);
        let intent = if p.capturing() {
            Intent::Key { chord: key }
        } else {
            match scope {
                Scope::Screen(id, _) => match p.view(id, &cx) {
                    Some(v) if v.overlay.is_some() => Intent::Key { chord: key },
                    Some(v) => screen_key(&v, pane.into(), key),
                    None => return Outcome::Ignored,
                },
                Scope::Section(id) => match p.section_view(id, &cx) {
                    Some(v) => section_key(&v, key),
                    None => return Outcome::Ignored,
                },
            }
        };
        p.intent(Self::id_of(scope), pane, &intent)
    }

    /// A fresh plugin brought to `place`.
    pub fn at(&self, place: &Place) -> Box<dyn Plugin> {
        let mut p = (self.make)();
        for (pane, key) in &place.keys {
            let _ = self.press(p.as_mut(), place.scope, *pane, *key);
        }
        p
    }

    /// Every `x` row and every target at `place`; see the module.
    pub fn place(&mut self, place: &Place) {
        let p = self.at(place);
        let pane = place.pane();
        if p.capturing() {
            self.gaps.push(format!("{place}: still typing; the x menu is not open here"));
            return;
        }
        let Shown { rows, targets } = self.shown(p.as_ref(), place.scope, pane);
        // What the menu is about (its first heading) and how deep in pages:
        // a row seen with both the same is not tried again.
        let depth = match place.scope {
            Scope::Screen(id, _) => p.depth(id),
            Scope::Section(_) => 0,
        };
        let heading = rows.iter().find(|b| b.is_heading()).map(|b| b.what.clone()).unwrap_or_default();
        drop(p);
        for b in rows.iter().filter(|b| !b.is_heading()) {
            let Some(key) = crate::plugin::parse_chord(&b.keys) else { continue };
            self.checked += 1;
            if host_keeps(key) {
                self.gaps.push(format!("{place}: `{} {}` is a key the host keeps; the part never sees it", b.keys, b.what));
                continue;
            }
            if host_falls_back(key, pane) {
                continue;
            }
            if !self.seen.insert(format!("{:?}/{depth}/{heading}/{} {}", place.scope, b.keys, b.what)) {
                continue;
            }
            let mut p = self.at(place);
            if matches!(self.press(p.as_mut(), place.scope, pane, key), Outcome::Ignored) {
                self.gaps.push(format!("{place}: `{} {}` did nothing", b.keys, b.what));
            }
        }
        for (on, target) in targets {
            let id = target.as_str().to_string();
            if !self.seen.insert(format!("{:?}/{depth}/{heading}/click {id}", place.scope)) {
                continue;
            }
            self.checked += 1;
            let mut p = self.at(place);
            let chosen = p.intent(Self::id_of(place.scope), pane, &Intent::Select { target });
            if matches!(chosen, Outcome::Ignored) {
                self.gaps.push(format!("{place}: choosing `{id}` in the {on:?} pane did nothing"));
            }
        }
    }

    /// Every place; the gaps found.
    pub fn places(&mut self, places: &[Place]) -> Vec<String> {
        for p in places {
            self.place(p);
        }
        self.gaps.clone()
    }

    /// What the part adds to `subject`'s `x` menu: each action, on a fresh
    /// plugin with no other parts' relations, returns effects. The number
    /// of actions.
    pub fn actions(&mut self, subject: &Subject) -> usize {
        let p = (self.make)();
        let Some(Contribution::Actions(actions)) = p.contribute(Point::Actions, subject) else {
            return 0;
        };
        for a in &actions {
            // (A key the host or the owner already uses is dropped by the
            // host; the row is then chosen from the menu.)
            self.checked += 1;
            let mut p = (self.make)();
            if p.act_with_related(subject, &a.id, &[]).is_empty() {
                self.gaps.push(format!("{}: `{}` did nothing", subject.label, a.what));
            }
        }
        actions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::{Effect, ScreenSpec};

    /// One row it takes, one it ignores, one on a key the host keeps, and
    /// a target it ignores when chosen.
    struct Part;
    impl Plugin for Part {
        fn id(&self) -> &'static str {
            "part"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn screens(&self) -> Vec<ScreenSpec> {
            vec![ScreenSpec { id: "s", name: "part", key: 'P' }]
        }
        fn view(&self, _: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
            use crate::view::{ActionSpec, ListRow, Node, PaneId, PaneView, ScreenView};
            let actions = [
                Binding::heading("it"),
                Binding::new("o", "open it"),
                Binding::new("z", "does nothing"),
                Binding::new("x", "clear it"),
                Binding::new("esc", "back"),
            ].iter().map(|b| ActionSpec::named(format!("part.s.{}", b.keys), b)).collect();
            let mut view = ScreenView::new("s", vec![PaneView::new(PaneId::Middle, "part", Node::List {
                rows: vec![ListRow::new("row", "row"), ListRow::new("dead", "dead")],
                selected: None,
                editing: None,
            })]);
            view.actions = actions;
            Some(view)
        }
        fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
            use crate::view::{Intent, Key};
            match intent {
                Intent::Key { chord } if chord.key == Key::Char('o') => Outcome::Handled(vec![Effect::Notify("o".into())]),
                Intent::Select { target } if target.as_str() == "row" => Outcome::done(),
                _ => Outcome::Ignored,
            }
        }
    }

    #[test]
    fn a_row_or_target_the_part_ignores_and_a_kept_key_are_gaps() {
        let make = || Box::new(Part) as Box<dyn Plugin>;
        let mut check = Check::new(&make);
        let gaps = check.places(&[Place::screen("s", Pane::Middle)]);
        assert_eq!(gaps.len(), 3, "{gaps:#?}");
        assert!(gaps[0].contains("`z does nothing` did nothing"), "{gaps:#?}");
        assert!(gaps[1].contains("`x clear it` is a key the host keeps"), "{gaps:#?}");
        assert!(gaps[2].contains("choosing `dead`"), "{gaps:#?}");
        assert_eq!(check.checked, 6, "four rows (esc falls back) and two targets");
    }

    #[test]
    fn the_keys_the_host_keeps_and_falls_back_on() {
        let k = Chord::char;
        for c in ['q', 'x', '?', 'b', 'w', 'h', 'l', '1', '9', crate::host::SETTINGS_KEY, crate::host::HISTORY_KEY] {
            assert!(host_keeps(k(c)), "{c}");
        }
        for c in ['a', 'o', 'u', 'm', '/', '[', ']'] {
            assert!(!host_keeps(k(c)), "{c}");
        }
        assert!(!host_keeps(Chord::ctrl('x')));
        assert!(host_falls_back(Chord::new(Key::Enter), Pane::Nav));
        assert!(!host_falls_back(Chord::new(Key::Enter), Pane::Middle));
        assert!(host_falls_back(Chord::new(Key::Esc), Pane::Detail));
    }

    #[test]
    fn the_check_names_no_terminal_library() {
        let source = include_str!("check.rs");
        for word in ["ratatui", "crossterm", "Buffer", "Rect", "KeyEvent", "view_tui"] {
            let used = source
                .lines()
                .take_while(|l| !l.starts_with("#[cfg(test)]"))
                .filter(|l| !l.trim_start().starts_with("//"))
                .any(|l| l.contains(word));
            assert!(!used, "check.rs uses {word}");
        }
    }
}
