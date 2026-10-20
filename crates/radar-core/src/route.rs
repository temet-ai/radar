//! Where a key or a click on a part's view goes: movement the host does along
//! a list, a table or a tree (it selects through the part), `⏎` activating the
//! selection, any other key passed on as the part's. No renderer is named
//! here: the view is data, a key is a [`Chord`], a click is a target.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use crate::plugin::Binding;
use crate::view::scroll::Scroll;
use crate::view::{ActionSpec, Chord, Intent, Key, Node, PaneId, ScreenView, SectionView, Target, TreeRow};

/// The id a click on a pane's empty space or border carries; a part's
/// `context_select` gets it for a click that only focuses.
pub const PANE_HIT: &str = "pane";

/// What the host keeps between keys: scroll offsets of the views it moves
/// along, the room the last view was laid out in, and how wide an icon's lead
/// is in the pack in use.
#[derive(Default)]
pub struct Frontend {
    scrolls: RefCell<HashMap<(String, PaneId), Scroll>>,
    room: Cell<(u16, u16)>,
    lead: Cell<usize>,
}

impl Frontend {
    /// How wide an icon's lead is in the pack in use (0 with icons off).
    pub fn lead(&self) -> usize {
        self.lead.get()
    }

    pub fn set_lead(&self, lead: usize) {
        self.lead.set(lead);
    }

    pub fn set_room(&self, width: u16, height: u16) {
        self.room.set((width, height));
    }

    /// The room the last frame had for the body, for views asked between
    /// frames (a key's).
    pub fn room(&self) -> (u16, u16) {
        match self.room.get() {
            (0, 0) => (120, 40),
            r => r,
        }
    }

    fn with_scroll<T>(&self, scope: &str, pane: PaneId, f: impl FnOnce(&Scroll) -> T) -> T {
        let mut all = self.scrolls.borrow_mut();
        f(all.entry((scope.to_string(), pane)).or_default())
    }
}

/// The panes the keys move in: a page's while one is open.
fn panes(view: &ScreenView) -> &[crate::view::PaneView] {
    match &view.page {
        Some(page) => &page.panes,
        None => &view.panes,
    }
}

fn pane_of(view: &ScreenView, pane: PaneId) -> Option<&crate::view::PaneView> {
    panes(view).iter().find(|p| p.pane == pane)
}

/// A field with a caret takes keys even when focus stays elsewhere.
fn field_owner(view: &ScreenView) -> Option<PaneId> {
    panes(view)
        .iter()
        .find(|pane| {
            pane.fields.iter().any(|field| field.caret.is_some())
                || pane.bottom_field.as_ref().is_some_and(|field| field.caret.is_some())
        })
        .map(|pane| pane.pane)
}

/// An editor or an overlay has precedence over movement.
pub(crate) fn screen_captures_keys(view: &ScreenView) -> bool {
    view.overlay.is_some()
        || field_owner(view).is_some()
        || panes(view).iter().any(|pane| match &pane.body {
            Node::List { editing, .. } => editing.is_some(),
            Node::Tree(tree) => tree.editing.is_some(),
            _ => false,
        })
}

/// The actions menu's rows from a view's actions.
pub fn bindings(actions: &[ActionSpec]) -> Vec<Binding> {
    actions
        .iter()
        .map(|a| {
            let mut b = if a.heading {
                Binding::heading(a.label.clone())
            } else {
                let keys = a.key.map_or_else(|| a.id.clone(), |c| c.name());
                Binding::new(keys, a.label.clone())
            };
            b.icon = a.icon;
            b.change = a.change.map(|c| match c {
                crate::view::Change::Add => crate::writes::Change::Add,
                crate::view::Change::Edit => crate::writes::Change::Edit,
            });
            b
        })
        .collect()
}

/// What a key or click comes to on a view.
#[derive(Debug, PartialEq)]
pub enum Routed {
    /// For the part.
    Intent(Intent),
    /// The host used it (it scrolled).
    Done,
    /// Nobody's here: the host's own keys may take it.
    Ignored,
}

/// A key on a screen's view: movement the host does itself (scrolling the
/// detail, moving along a list or tree, which selects through the part), `⏎`
/// activating the selection, any other key passed on.
pub fn key(fe: &Frontend, view: &ScreenView, focus: PaneId, chord: Chord) -> Routed {
    // While something is written in, every key is the part's.
    if screen_captures_keys(view) {
        return Routed::Intent(Intent::Key { chord });
    }
    let page = matches!(chord.key, Key::PageUp | Key::PageDown) && !chord.ctrl;
    let down = chord.key == Key::PageDown;
    if page
        && view.page.is_none()
        && let Some(det) = view.pane(PaneId::Detail)
        && matches!(det.body, Node::Document { .. })
    {
        return if fe.with_scroll(&view.scope, PaneId::Detail, |s| s.page(down)) {
            Routed::Done
        } else {
            Routed::Ignored
        };
    }
    let Some(pane) = pane_of(view, focus) else {
        return Routed::Intent(Intent::Key { chord });
    };
    // Rows to move along: a list's or a table's.
    let (rows, headings) = match &pane.body {
        Node::List { rows, selected, .. } => (
            Some((rows.iter().map(|r| &r.target).collect::<Vec<_>>(), *selected)),
            rows.iter().map(|r| r.heading).collect(),
        ),
        Node::Table { rows, selected, .. } => (Some((rows.iter().map(|r| &r.target).collect::<Vec<_>>(), *selected)), vec![]),
        Node::Entries { rows, selected, .. } => (Some((rows.iter().map(|r| &r.target).collect::<Vec<_>>(), *selected)), vec![]),
        _ => (None, vec![]),
    };
    if let Some((rows, selected)) = rows {
        if selected.is_none() && !rows.is_empty() {
            return Routed::Intent(Intent::Key { chord });
        }
        let at = selected.unwrap_or(0);
        // On a page a list moves by a page; when it cannot move the key
        // is nobody's.
        if page && view.page.is_some() && !rows.is_empty() {
            let step = fe.with_scroll(&view.scope, focus, |s| s.height()).saturating_sub(1).max(1) as usize;
            let to = if down {
                (at + step).min(rows.len() - 1)
            } else {
                at.saturating_sub(step)
            };
            return if to == at {
                Routed::Ignored
            } else {
                Routed::Intent(Intent::Select {
                    target: rows[to].clone(),
                })
            };
        }
        let heading = |i: usize| headings.get(i).copied().unwrap_or(false);
        if let Some(i) = crate::view::rows_step_by(rows.len(), at, chord, heading) {
            return Routed::Intent(Intent::Select {
                target: rows[i].clone(),
            });
        }
        if chord == Chord::new(Key::Enter)
            && let Some(r) = rows.get(at)
        {
            return Routed::Intent(Intent::Activate { target: (*r).clone() });
        }
        return Routed::Intent(Intent::Key { chord });
    }
    fn scrolling(node: &Node) -> &Node {
        match node {
            Node::Extension { capability, fallback, .. } if capability == crate::view::detail::CAPABILITY => scrolling(fallback),
            _ => node,
        }
    }
    match scrolling(&pane.body) {
        Node::List { .. } | Node::Table { .. } | Node::Entries { .. } | Node::Log { .. } | Node::Diff { .. } => {}
        Node::Rich { .. } => {
            // Scrolled to the end: the part may bring more.
            let (moved, end) = fe.with_scroll(&view.scope, focus, |s| (s.key(&chord), s.at() >= s.max()));
            match (moved, end) {
                (true, true) => return Routed::Intent(Intent::End { pane: focus }),
                (true, false) => return Routed::Done,
                _ => {}
            }
        }
        Node::Tree(tree) => {
            if let Some(i) = crate::view::tree_step(tree, chord) {
                return Routed::Intent(i);
            }
            if crate::view::tree_key(chord) && matches!(chord.key, Key::Left | Key::Right) {
                return Routed::Done;
            }
            if chord == Chord::new(Key::Enter) && !tree.selected.is_empty() {
                return Routed::Intent(Intent::Activate {
                    target: TreeRow::target_of(&tree.selected),
                });
            }
        }
        Node::Document { .. } => {
            if fe.with_scroll(&view.scope, focus, |s| s.key(&chord)) {
                return Routed::Done;
            }
        }
        Node::State { .. } | Node::Extension { .. } => {}
    }
    Routed::Intent(Intent::Key { chord })
}

/// A key in a settings section's view: movement along its rows selects
/// through the part (`↑` on the first row stays there: a section's rows
/// keep the keys, as they always have); anything else is passed on.
pub fn section_key(view: &SectionView, chord: Chord) -> Intent {
    if let Node::List {
        rows,
        selected,
        editing: None,
    } = &view.body
        && !rows.is_empty()
    {
        let at = selected.unwrap_or(0);
        let up = matches!(chord.key, Key::Up | Key::Char('k')) && !chord.ctrl && !chord.alt;
        let to = crate::view::rows_step(rows, at, chord).or_else(|| up.then_some(at));
        if let Some(i) = to {
            return Intent::Select {
                target: rows[i].target.clone(),
            };
        }
    }
    // A section's tree moves and folds as a screen's does; `↑` on its
    // first row stays there. While a row is typed into, keys are the part's.
    if let Node::Tree(tree) = &view.body
        && tree.editing.is_none()
    {
        if let Some(i) = crate::view::tree_step(tree, chord) {
            return i;
        }
        if matches!(chord.key, Key::Up | Key::Char('k')) && !chord.ctrl && !chord.alt
            && !chord.shift && tree.selectable(&tree.selected)
        {
            return Intent::Select {
                target: TreeRow::target_of(&tree.selected),
            };
        }
    }
    Intent::Key { chord }
}

/// A click (or a right click, `context`) on the target `id`.
pub fn click(id: &str, context: bool) -> Routed {
    // Only host admission may dispatch pinned controls. Never turn a reserved
    // target into a provider row Select/Context.
    if id.starts_with(crate::view::detail::TARGET_PREFIX) {
        return Routed::Done;
    }
    // A pane's background only focuses it; it has no menu of its own.
    if id == PANE_HIT {
        return if context { Routed::Ignored } else { Routed::Done };
    }
    let target = Target::new(id);
    Routed::Intent(if context {
        Intent::Context { target }
    } else {
        Intent::Select { target }
    })
}

/// An intent to its part, as the host gives it: a contributed cell chosen
/// ([`Intent::Show`]) selects its row through the owner, then shows the
/// subject as the owner's effect.
pub fn to_part(p: &mut dyn crate::Plugin, scope: &str, pane: crate::plugin::Pane, intent: &Intent) -> crate::Outcome {
    let Intent::Show { target, subject } = intent else {
        return p.intent(scope, pane, intent);
    };
    let select = Intent::Select {
        target: target.clone(),
    };
    let mut fx = match p.intent(scope, crate::plugin::Pane::Middle, &select) {
        crate::Outcome::Handled(fx) => fx,
        crate::Outcome::Ignored => vec![],
    };
    fx.push(crate::Effect::Show(subject.clone().into()));
    crate::Outcome::Handled(fx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{ListRow, PaneView, Tree};

    fn screen() -> ScreenView {
        let nav = Node::List {
            rows: vec![ListRow::new("view:0", "all"), ListRow::new("view:1", "dirty")],
            selected: Some(0),
            editing: None,
        };
        let tree = Tree {
            rows: vec![TreeRow::new("a", "a").child(TreeRow::new("b", "b"))],
            open: vec![vec!["a".into()]],
            selected: vec!["a".into()],
            ..Tree::default()
        };
        ScreenView::new(
            "s",
            vec![
                PaneView::new(PaneId::Nav, "views", nav),
                PaneView::new(PaneId::Middle, "all", Node::Tree(tree)),
                PaneView::new(PaneId::Detail, "a", Node::Document { lines: vec!["x".into()], cut: false }),
            ],
        )
    }

    #[test]
    fn movement_selects_through_the_part_and_enter_activates() {
        let fe = Frontend::default();
        let v = screen();
        let sel = |id: &str| Routed::Intent(Intent::Select { target: Target::new(id) });
        assert_eq!(key(&fe, &v, PaneId::Nav, Chord::char('j')), sel("view:1"));
        assert_eq!(
            key(&fe, &v, PaneId::Middle, Chord::new(Key::Down)),
            Routed::Intent(Intent::Select { target: TreeRow::target_of(&["a".into(), "b".into()]) })
        );
        assert_eq!(
            key(&fe, &v, PaneId::Middle, Chord::new(Key::Enter)),
            Routed::Intent(Intent::Activate { target: Target::new("a") })
        );
        assert_eq!(
            key(&fe, &v, PaneId::Middle, Chord::char('d')),
            Routed::Intent(Intent::Key { chord: Chord::char('d') }),
            "anything else is the part's"
        );
    }

    /// `/` typed from the views rail opens the tree's field: ↓ is the field's,
    /// not a move along the rail.
    fn typing_in_the_tree(mut v: ScreenView) -> ScreenView {
        v.panes[1].fields.push(crate::view::FieldBar {
            mark: " / ".into(),
            text: "ti".into(),
            caret: Some(2),
            hint: crate::view::Text::default(),
        });
        v
    }

    #[test]
    fn a_field_being_typed_into_takes_every_key_whichever_pane_has_the_focus() {
        let fe = Frontend::default();
        let v = typing_in_the_tree(screen());
        for chord in [Chord::new(Key::Down), Chord::char('j'), Chord::new(Key::PageDown)] {
            assert_eq!(key(&fe, &v, PaneId::Nav, chord), Routed::Intent(Intent::Key { chord }), "{chord:?}");
        }
    }

    #[test]
    fn clicks_select_their_target_and_the_pane_only_focuses() {
        assert_eq!(click(PANE_HIT, false), Routed::Done);
        assert_eq!(click(PANE_HIT, true), Routed::Ignored);
        assert_eq!(
            click("view:1", true),
            Routed::Intent(Intent::Context { target: Target::new("view:1") })
        );
        assert_eq!(
            click("view:1", false),
            Routed::Intent(Intent::Select { target: Target::new("view:1") })
        );
    }

    #[test]
    fn actions_become_the_menu_rows_with_their_keys() {
        let rows = bindings(&[
            ActionSpec::named("", &Binding::heading("main")),
            ActionSpec::named("git.branch.delete", &Binding::new("d", "delete the branch…").edits()),
        ]);
        assert!(rows[0].is_heading());
        assert_eq!(crate::plugin::parse_chord(&rows[1].keys), Some(Chord::char('d')));
        assert_eq!(rows[1].change, Some(crate::writes::Change::Edit));
    }

    #[test]
    fn every_view_action_chord_remains_pressable_in_a_menu() {
        for key in [Key::Backspace, Key::Delete, Key::Home, Key::End, Key::PageUp, Key::PageDown] {
            let chord = Chord::new(key);
            let mut action = ActionSpec::named("t.s.action", &Binding::new("a", "action"));
            action.key = Some(chord);
            let row = bindings(&[action]).remove(0);
            assert_eq!(crate::plugin::parse_chord(&row.keys), Some(chord));
        }
    }

    #[test]
    fn editing_a_tree_row_sends_keys_to_the_part() {
        let view = SectionView {
            version: crate::view::VERSION,
            scope: "projects".into(),
            body: Node::Tree(Tree {
                rows: vec![TreeRow::new("site", "site")],
                selected: vec!["site".into()],
                editing: Some(Box::new(crate::view::TreeEditing {
                    path: vec!["site".into()],
                    at: 2,
                    text: "acme".into(),
                    caret: 4,
                })),
                ..Tree::default()
            }),
            detail_title: String::new(),
            detail: vec![],
            detail_blocks: vec![],
            actions: vec![],
        };
        let chord = Chord::new(Key::Down);
        assert_eq!(section_key(&view, chord), Intent::Key { chord });
    }

    #[test]
    fn an_empty_settings_tree_still_translates_open_all_without_a_fake_selection() {
        let view = SectionView {
            version: crate::view::VERSION,
            scope: "watch".into(),
            body: Node::Tree(Tree::default()),
            detail_title: String::new(),
            detail: vec![],
            detail_blocks: vec![],
            actions: vec![],
        };
        assert_eq!(section_key(&view, Chord::char('=')), Intent::OpenAll);
        assert_eq!(section_key(&view, Chord::new(Key::Up)), Intent::Key { chord: Chord::new(Key::Up) });
    }
}
