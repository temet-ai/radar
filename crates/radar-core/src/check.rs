//! Unit-level checks a part runs over its own plugin: no host, no full
//! frame, milliseconds per place (GUIDE 2.5, 3.2).
//!
//! At a [`Place`] (a screen and pane, or a settings section, and the keys
//! that bring a fresh plugin there), [`Check::place`] checks:
//!
//! - **every row of the `x` menu** (`Plugin::keys` / `Plugin::section_keys`):
//!   its key is not one the host keeps ([`host_keeps`]: the part would never
//!   see it), and pressed on a fresh plugin at the same place the part takes
//!   it (`Outcome::Handled`), unless it is a key the host acts on when the
//!   part does not ([`host_falls_back`]: `esc`, arrows, `PgUp`/`PgDn`, `⏎`
//!   on the nav pane, `+`/`-`).
//! - **every click target** the part exposes while drawing the place (except
//!   a pane's background): clicked on a fresh plugin, the part takes it.
//!
//! [`Check::actions`] checks what a part adds to another's `x` menu
//! (`Point::Actions`): each action, acted on a fresh plugin, returns effects.
//!
//! Only the part's own area is drawn, once per place, at [`Check::size`].

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};

use crate::compose::{Contribution, Parts, Point, Subject};
use crate::plugin::{Outcome, Pane, Plugin};
use crate::view_tui::{Cx, PANE_HIT};

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
    pub keys: Vec<(Pane, KeyEvent)>,
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

    /// `code` pressed `n` times in the focused pane.
    pub fn press(mut self, code: KeyCode, n: usize) -> Self {
        let pane = self.pane();
        self.keys.extend(std::iter::repeat_n((pane, KeyEvent::new(code, KeyModifiers::NONE)), n));
        self
    }

    /// `↓` `n` times.
    pub fn down(self, n: usize) -> Self {
        self.press(KeyCode::Down, n)
    }

    /// `⏎` once.
    pub fn enter(self) -> Self {
        self.press(KeyCode::Enter, 1)
    }

    /// `]` `n` times: a page's next tab.
    pub fn tab(self, n: usize) -> Self {
        self.press(KeyCode::Char(']'), n)
    }

    /// Characters typed, one key each.
    pub fn typed(mut self, text: &str) -> Self {
        for c in text.chars() {
            self = self.press(KeyCode::Char(c), 1);
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
        let keys: Vec<String> = self.keys.iter().map(|(_, k)| format!("{:?}", k.code)).collect();
        write!(f, "{:?} after [{}]", self.scope, keys.join(" "))
    }
}

/// Keys the host takes before a part sees them, when nothing is being
/// typed: `q` `x` `?` `b` `w`, the settings key, `tab`, `h` `l` `←` `→` and
/// digits (panes). A part's `x` row must not use one. (A capital is kept
/// only when it is a screen's key in the mix; `tests/mix.rs` checks those.)
pub fn host_keeps(key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return false;
    }
    match key.code {
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => true,
        KeyCode::Char(c) => {
            matches!(c, 'q' | 'x' | '?' | 'b' | 'w' | 'h' | 'l' | '1'..='9' | crate::host::SETTINGS_KEY | crate::host::HISTORY_KEY)
        }
        _ => false,
    }
}

/// Keys the host acts on itself when the part does not take them: `esc`
/// goes back, arrows reach the bar or say there is nowhere to go,
/// `PgUp`/`PgDn` say there is nothing to scroll, `+`/`-` say there is
/// nothing to fold, and `⏎` on the nav pane goes into the next pane.
pub fn host_falls_back(key: KeyEvent, pane: Pane) -> bool {
    match key.code {
        KeyCode::Esc | KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => true,
        KeyCode::Char('+' | '=' | '-') => true,
        KeyCode::Enter => pane == Pane::Nav,
        _ => false,
    }
}

/// The checks over plugins made by `make`, each drawn at `size`.
pub struct Check<'a> {
    make: &'a dyn Fn() -> Box<dyn Plugin>,
    pub size: (u16, u16),
    pub now: i64,
    /// What did nothing, one line each.
    pub gaps: Vec<String>,
    /// Rows, targets and actions tried.
    pub checked: usize,
    theme: crate::theme::Theme,
    /// The Ratatui frontend's state, for parts drawn from views.
    fe: crate::view_tui::Frontend,
    /// Rows and targets already tried, by scope, page depth and words: the
    /// same row on the next tree row of its kind is tried once.
    seen: std::collections::HashSet<String>,
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
            theme: crate::theme::Theme::new(crate::design::theme::Tier::Mono),
            fe: crate::view_tui::Frontend::default(),
        }
    }

    fn scope(scope: Scope) -> crate::view_tui::Scope<'static> {
        match scope {
            Scope::Screen(id, _) => crate::view_tui::Scope::Screen(id),
            Scope::Section(id) => crate::view_tui::Scope::Section(id),
        }
    }

    /// Draw the place's area of `p` once; the click targets it marked.
    pub fn draw(&self, p: &dyn Plugin, scope: Scope) -> Vec<(Rect, Pane, String)> {
        let area = Rect::new(0, 0, self.size.0, self.size.1);
        let mut buf = Buffer::empty(area);
        let (cursor, hits) = (std::cell::Cell::new(None), std::cell::RefCell::new(vec![]));
        let (focus, focused) = match scope {
            Scope::Screen(_, pane) => (pane, false),
            Scope::Section(_) => (Pane::Middle, true),
        };
        let cx = Cx {
            theme: &self.theme,
            now: self.now,
            focused,
            focus,
            bar: false,
            tabs: false,
            cursor: &cursor,
            hits: &hits,
            parts: Parts::none(),
        };
        crate::view_tui::draw_part(&self.fe, p, Self::scope(scope), area, &mut buf, &cx);
        hits.into_inner()
    }

    fn press(&self, p: &mut dyn Plugin, scope: Scope, pane: Pane, key: KeyEvent) -> Outcome {
        crate::view_tui::press_part(&self.fe, p, Self::scope(scope), pane, key, self.now)
    }

    /// A fresh plugin brought to `place`, drawn before each `⏎` (a tree
    /// knows its rows once drawn) and once at the end.
    pub fn at(&self, place: &Place) -> Box<dyn Plugin> {
        let p = self.reach(place);
        let _ = self.draw(p.as_ref(), place.scope);
        p
    }

    fn reach(&self, place: &Place) -> Box<dyn Plugin> {
        let mut p = (self.make)();
        for (pane, key) in &place.keys {
            if key.code == KeyCode::Enter {
                let _ = self.draw(p.as_ref(), place.scope);
            }
            let _ = self.press(p.as_mut(), place.scope, *pane, *key);
        }
        p
    }

    /// Every `x` row and every click target at `place`; see the module.
    pub fn place(&mut self, place: &Place) {
        let p = self.at(place);
        let pane = place.pane();
        if p.capturing() {
            self.gaps.push(format!("{place}: still typing; the x menu is not open here"));
            return;
        }
        let rows = crate::view_tui::keys_part(&self.fe, p.as_ref(), Self::scope(place.scope), pane, self.now);
        let hits = self.draw(p.as_ref(), place.scope);
        // What the menu is about (its first heading) and how deep in pages:
        // a row seen with both the same is not tried again.
        let depth = match place.scope {
            Scope::Screen(id, _) => p.depth(id),
            Scope::Section(_) => 0,
        };
        let heading = rows.iter().find(|b| b.is_heading()).map(|b| b.what.clone()).unwrap_or_default();
        drop(p);
        for b in rows.iter().filter(|b| !b.is_heading()) {
            let Some(key) = crate::plugin::parse_key(&b.keys) else { continue };
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
        for (rect, _, id) in hits {
            if id == PANE_HIT || rect.is_empty() || !self.seen.insert(format!("{:?}/{depth}/{heading}/click {id}", place.scope)) {
                continue;
            }
            self.checked += 1;
            let mut p = self.at(place);
            let at = Position::new(rect.x + rect.width / 2, rect.y);
            let clicked = crate::view_tui::click_part(&self.fe, p.as_mut(), Self::scope(place.scope), pane, &id, at, false, self.now);
            if matches!(clicked, Outcome::Ignored) {
                self.gaps.push(format!("{place}: a click on `{id}` at {},{} did nothing", at.x, at.y));
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
    use crate::plugin::{Binding, Effect, ScreenSpec};

    /// One row it takes, one it ignores, one on a key the host keeps, and
    /// a mark it ignores when clicked.
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
        assert!(gaps[2].contains("a click on `dead`"), "{gaps:#?}");
        assert_eq!(check.checked, 6, "four rows (esc falls back) and two targets");
    }

    #[test]
    fn the_keys_the_host_keeps_and_falls_back_on() {
        let k = |c| KeyEvent::new(c, KeyModifiers::NONE);
        for c in ['q', 'x', '?', 'b', 'w', 'h', 'l', '1', '9', crate::host::SETTINGS_KEY, crate::host::HISTORY_KEY] {
            assert!(host_keeps(k(KeyCode::Char(c))), "{c}");
        }
        for c in ['a', 'o', 'u', 'm', '/', '[', ']'] {
            assert!(!host_keeps(k(KeyCode::Char(c))), "{c}");
        }
        assert!(!host_keeps(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::CONTROL)));
        assert!(host_falls_back(k(KeyCode::Enter), Pane::Nav));
        assert!(!host_falls_back(k(KeyCode::Enter), Pane::Middle));
        assert!(host_falls_back(k(KeyCode::Esc), Pane::Detail));
    }
}
