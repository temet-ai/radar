//! Shared tree dispatch. Owners receive their existing Select/Fold operations;
//! the host owns pane precedence, held-data open-all and stale path admission.

use super::{Host, HostEffect, Pane, View, interaction_motion};
use crate::plugin::{Binding, Outcome};
use crate::view::{Intent, Key, Node, Tree, TreeRow};
use crate::view::Chord;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct MenuContext {
    view: View,
    pane: Pane,
    route: Option<crate::motion::transition::MotionContext>,
    rows: Vec<TreeRow>,
    selected: Vec<String>,
    open: Vec<Vec<String>>,
}

impl Host {
    /// A uniquely addressable, currently focused tree. Editors and overlays
    /// retain input ownership even when the part's legacy focus is elsewhere.
    pub(super) fn focused_tree(&self) -> Option<Tree> {
        if self.bar
            || self.sources
            || self.tabs
            || self.menu.is_some()
            || self.capturing()
            || (self.dock.focused && self.dock_has_place())
        {
            return None;
        }
        if self.view == View::Settings && self.focus == Pane::Nav {
            return Some(self.settings_nav_tree());
        }
        let p = self.target()?;
        let body = match self.view {
            View::Screen(_) => {
                let view = self.screen_view(p, self.screen()?.1.id)?;
                if crate::route::screen_captures_keys(&view) {
                    return None;
                }
                interaction_motion::screen_node(&view, self.focus)?.clone()
            }
            View::Settings if self.focus == Pane::Middle => {
                self.section_view(p, self.current_section()?.1.id)?.body
            }
            View::Settings => return None,
        };
        match body {
            Node::Tree(tree) if tree.editing.is_none() => Some(tree),
            _ => None,
        }
    }

    /// Run before the host's left/right pane switch, after menus, capture and
    /// top-bar/tab dispatch. A tree keeps left/right at its boundaries on a
    /// screen; on an item page they go on to the pane or tab beside. Up/k
    /// still reach the bar.
    pub(super) fn shared_tree_key(&mut self, key: Chord) -> Option<Vec<HostEffect>> {
        let chord = key;
        if !crate::view::tree_key(chord)
            || !matches!(
                chord.key,
                Key::Left | Key::Right | Key::Char(' ' | '+' | '-' | '=')
            )
            || self.bar
            || self.sources
            || self.tabs
        {
            return None;
        }
        // A model-declared field/overlay is an editor even if the plugin did
        // not also expose legacy capturing(). Do not steal its arrows.
        if let Some((p, spec)) = self.screen()
            && self.ready(p)
            && self
                .screen_view(p, spec.id)
                .is_some_and(|view| crate::route::screen_captures_keys(&view))
        {
            return Some(self.deliver(key).unwrap_or_default());
        }
        if self.view == View::Settings
            && self.focus == Pane::Middle
            && let Some(p) = self.target()
            && let Some((_, section)) = self.current_section()
            && self
                .section_view(p, section.id)
                .is_some_and(|section| match section.body {
                    Node::Tree(tree) => tree.editing.is_some(),
                    Node::List { editing, .. } => editing.is_some(),
                    _ => false,
                })
        {
            return Some(self.deliver(key).unwrap_or_default());
        }
        let tree = self.focused_tree()?;
        let intent = crate::view::tree_step(&tree, chord);
        // On an item page a left/right the tree has no use for goes on to
        // the pane or tab beside (the tab row is how the page is walked); on
        // a screen it stays, rather than leave for the screen beside.
        if intent.is_none() && matches!(chord.key, Key::Left | Key::Right) && self.page_tabs().is_some() {
            return None;
        }
        if self.view == View::Settings && self.focus == Pane::Nav {
            if let Some(intent) = intent {
                self.settings_tree_intent(&intent);
            }
            return Some(vec![]);
        }
        Some(intent.map_or_else(Vec::new, |intent| {
            self.deliver_input(super::Input::Intent(intent))
                .unwrap_or_default()
        }))
    }

    pub(super) fn settings_nav_tree(&self) -> Tree {
        let rows = self
            .nav_groups()
            .into_iter()
            .map(|(group, sections)| {
                TreeRow::new(group, self.group_name(group)).children(
                    sections
                        .into_iter()
                        .map(|i| TreeRow::new(i.to_string(), self.sections[i].1.name)),
                )
            })
            .collect();
        let (group, section) = self.nav_at();
        let mut selected = vec![group.to_string()];
        if let Some(section) = section {
            selected.push(section.to_string());
        }
        Tree {
            rows,
            selected,
            open: self
                .nav_groups()
                .into_iter()
                .filter(|(group, _)| !self.folded.contains(group))
                .map(|(group, _)| vec![group.to_string()])
                .collect(),
            ..Tree::default()
        }
    }

    fn settings_tree_intent(&mut self, intent: &Intent) {
        match intent {
            Intent::OpenAll => {
                // Opening never changes the active section or heading cursor.
                self.on_heading = self.nav_at().1.is_none();
                self.folded.clear();
                let _ = self.take_motion_fold();
            }
            Intent::Select { target } => {
                let path = TreeRow::path_of(target);
                if !self.settings_nav_tree().selectable(&path) {
                    return;
                }
                let Some((group, sections)) = self
                    .nav_groups()
                    .into_iter()
                    .find(|(group, _)| path.first().is_some_and(|id| id == group))
                else {
                    return;
                };
                let section = path
                    .get(1)
                    .and_then(|id| id.parse::<usize>().ok())
                    .filter(|i| sections.contains(i));
                self.nav_go((group, section));
            }
            Intent::Fold { target, open } => {
                let path = TreeRow::path_of(target);
                if path.len() != 1 {
                    return;
                }
                let Some((group, _)) = self
                    .nav_groups()
                    .into_iter()
                    .find(|(group, _)| path[0] == *group)
                else {
                    return;
                };
                if self.folded.contains(group) == *open {
                    let before = self.settings_fold_before(group);
                    let selected = self.nav_at();
                    if *open {
                        self.folded.remove(group);
                        if selected == (group, None) {
                            self.on_heading = true;
                        }
                    } else {
                        self.folded.insert(group);
                        if selected.0 == group {
                            self.nav_go((group, None));
                        }
                    }
                    self.settings_fold_after(group, before);
                }
            }
            _ => {}
        }
    }

    /// Chrome group folds are valid only in the focused settings tree. An old
    /// section, unknown group or intent from another pane cannot retarget it.
    pub(super) fn settings_nav_fold(
        &mut self,
        target: &crate::view::Target,
        open: bool,
    ) -> Vec<HostEffect> {
        if self.view != View::Settings
            || self.focus != Pane::Nav
            || self.focused_tree().is_none()
        {
            return vec![];
        }
        if let Some((group, _)) = self.nav_groups().into_iter().find(|(group, _)| {
            target == &crate::view::chrome::targets::nav_group(group)
        }) {
            self.settings_tree_intent(&Intent::Fold {
                target: crate::view::Target::new(group),
                open,
            });
        }
        vec![]
    }

    /// OpenAll is host-owned even for host settings navigation. It must not
    /// fall through into a plugin's custom action or an underlying tree.
    pub(super) fn open_all_intent(&mut self) -> Vec<HostEffect> {
        if self.focused_tree().is_none() {
            return vec![];
        }
        if self.view == View::Settings && self.focus == Pane::Nav {
            self.settings_tree_intent(&Intent::OpenAll);
            return vec![];
        }
        self.deliver_input(super::Input::Intent(Intent::OpenAll))
            .unwrap_or_default()
    }

    /// Admit shared tree input before observing its effects. Keeping these
    /// layers separate lets disabled motion forward without projecting a view;
    /// real dispatch still validates every tree target at this boundary.
    pub(super) fn shared_part_input(
        &mut self,
        p: usize,
        scope: &str,
        pane: Pane,
        intent: &Intent,
        known: Option<&Node>,
    ) -> Outcome {
        if let Some(outcome) = self.shared_tree_input(p, scope, pane, intent, known) {
            return outcome;
        }
        self.motion_part_input(p, scope, pane, intent, known)
    }

    /// Tree-specific validation and OpenAll, at the shared owner-dispatch
    /// boundary used by terminal keys, frontend intents and menu replay.
    pub(super) fn shared_tree_input(
        &mut self,
        p: usize,
        scope: &str,
        pane: Pane,
        intent: &Intent,
        known: Option<&Node>,
    ) -> Option<Outcome> {
        if !matches!(
            intent,
            Intent::OpenAll | Intent::Select { .. } | Intent::Context { .. } | Intent::Fold { .. }
        ) {
            return None;
        }
        let node = known.cloned().or_else(|| self.motion_node(p, scope, pane));
        let Some(Node::Tree(tree)) = node else {
            return matches!(intent, Intent::OpenAll | Intent::Fold { .. }).then(Outcome::done);
        };
        let blocked = || {
            if matches!(intent, Intent::Context { .. }) {
                Outcome::Ignored
            } else {
                Outcome::done()
            }
        };
        if self.plugins[p].capturing()
            || tree.editing.is_some()
            || (self.view != View::Settings
                && self.screen_view(p, scope).is_some_and(|view| {
                    crate::route::screen_captures_keys(&view)
                }))
        {
            return Some(blocked());
        }
        match intent {
            Intent::Select { target } | Intent::Context { target } => {
                // Custom cell/header targets retain the owner's action contract.
                if custom_target(&tree, target) {
                    return None;
                }
                (!tree.selectable(&TreeRow::path_of(target))).then(blocked)
            }
            Intent::Fold { target, .. } => (!tree
                .row(&TreeRow::path_of(target))
                .is_some_and(|row| !row.children.is_empty()))
            .then(Outcome::done),
            Intent::OpenAll => {
                // Invoke's key fallback reaches this boundary without on_key.
                // It must not expand a tree beneath the bar, tabs, dock or a
                // different focused owner merely because the action still exists.
                let current_scope = match self.view {
                    View::Screen(_) => self.screen().map(|(_, screen)| screen.id),
                    View::Settings => self.current_section().map(|(_, section)| section.id),
                };
                if self.focus != pane
                    || self.target() != Some(p)
                    || current_scope != Some(scope)
                    || self.focused_tree().as_ref() != Some(&tree)
                {
                    return Some(Outcome::done());
                }
                let Some(context) = self.motion_context() else {
                    // An item page without an owner identity cannot prove that
                    // successive folds still address the same held tree.
                    return Some(Outcome::done());
                };
                let rows = fold_identity(&tree.rows);
                let mut effects = vec![];
                for path in tree.groups() {
                    // Do not follow an owner into a replacement route, new data,
                    // new selection, or an editor while expanding a snapshot.
                    let Some(Node::Tree(current)) = self.motion_node(p, scope, pane) else {
                        break;
                    };
                    if self.motion_context().as_ref() != Some(&context)
                        || self.plugins[p].capturing()
                        || fold_identity(&current.rows) != rows
                        || current.selected != tree.selected
                        || current.editing.is_some()
                    {
                        break;
                    }
                    if current.open.contains(&path) {
                        continue;
                    }
                    let fold = Intent::Fold {
                        target: TreeRow::target_of(&path),
                        open: true,
                    };
                    match self.shared_part_input(p, scope, pane, &fold, Some(&Node::Tree(current)))
                    {
                        Outcome::Handled(fx) => {
                            // Effects may change focus, open a modal or refresh
                            // data when routed. Return them before any more folds.
                            let stop = !fx.is_empty();
                            effects.extend(fx);
                            if stop {
                                break;
                            }
                        }
                        Outcome::Ignored => break,
                    }
                }
                // A bulk expansion is not a single-row animation proof.
                let _ = self.take_motion_fold();
                Some(Outcome::Handled(effects))
            }
            _ => None,
        }
    }

    pub(super) fn tree_menu_context(&self, key: Chord) -> Option<MenuContext> {
        let chord = key;
        if !crate::view::tree_key(chord)
            || !matches!(
                chord.key,
                Key::Left | Key::Right | Key::Char(' ' | '+' | '-' | '=')
            )
        {
            return None;
        }
        let tree = self.focused_tree()?;
        Some(MenuContext {
            view: self.view,
            pane: self.focus,
            route: self.motion_context(),
            rows: tree.rows,
            selected: tree.selected,
            open: tree.open,
        })
    }

    pub(super) fn tree_menu_is_current(&self, key: Chord, context: &MenuContext) -> bool {
        context.route.is_some() && self.tree_menu_context(key).as_ref() == Some(context)
    }

    pub(super) fn shared_fold_keys(&self) -> Option<Vec<Binding>> {
        let tree = self.focused_tree()?;
        if tree.groups().is_empty() {
            return Some(vec![]);
        }
        Some(vec![
            Binding::new("space", "toggle the group"),
            Binding::new("+", "expand the group"),
            Binding::new("-", "collapse the group, or the one this row is in"),
            Binding::new("=", "expand every held group"),
        ])
    }
}

/// Folder glyphs express the fold just applied, not replacement data. Keep
/// every other field, child identity, subject payload and action target exact.
fn fold_identity(rows: &[TreeRow]) -> Vec<TreeRow> {
    fn normalize(rows: &mut [TreeRow]) {
        for row in rows {
            if !row.children.is_empty() {
                for seg in &mut row.name.0 {
                    if seg.icon == Some(crate::ui::icon::Icon::FolderOpen) {
                        seg.icon = Some(crate::ui::icon::Icon::Folder);
                    }
                }
                normalize(&mut row.children);
            }
        }
    }
    let mut rows = rows.to_vec();
    normalize(&mut rows);
    rows
}

fn custom_target(tree: &Tree, target: &crate::view::Target) -> bool {
    fn has(rows: &[TreeRow], target: &crate::view::Target, pointer: bool) -> bool {
        rows.iter().any(|row| {
            (pointer && target.as_str() == format!("node:{}", row.id))
                || row
                    .cells
                    .iter()
                    .any(|cell| cell.target.as_ref() == Some(target))
                || row
                    .name
                    .0
                    .iter()
                    .any(|seg| seg.target.as_ref() == Some(target))
                || has(&row.children, target, pointer)
        })
    }
    tree.header_target.as_ref() == Some(target) || has(&tree.rows, target, tree.pointer_action)
}

#[cfg(test)]
#[path = "host_tree_tests.rs"]
mod tests;
