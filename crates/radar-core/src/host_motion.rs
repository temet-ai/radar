//! Host-owned causal verification. No clock, renderer geometry, or animation.

use super::{Host, Owner, Pane, View};
use crate::motion::transition::{BODY_SLOT, MotionContext, MotionFold, SETTINGS_NAV_SLOT};
use crate::plugin::Outcome;
use crate::view::{Intent, Key, Node, PaneId, Tree, TreeRow};

#[derive(Default)]
pub(super) struct State {
    enabled: bool,
    origin: Option<(u64, u64)>,
    pub(super) fold: Option<MotionFold>,
    // Lives only during this dispatch; final effects must leave the same tree.
    proof: Option<(usize, String, Pane, Tree)>,
}

impl Host {
    /// Enable semantic fold observation only at the frontend's effective full
    /// level. Switching it off releases pending evidence immediately.
    pub fn set_motion_enabled(&mut self, enabled: bool) {
        self.interaction_motion.enabled = enabled;
        if !enabled {
            self.interaction_motion.fold = None;
            self.interaction_motion.proof = None;
        }
    }

    /// Whether this frontend currently permits interaction motion.
    pub fn motion_enabled(&self) -> bool {
        self.interaction_motion.enabled
    }

    /// Lightweight identity: asks only owner state APIs, never builds a view.
    /// An item page without an explicit identity is intentionally ambiguous.
    pub fn motion_context(&self) -> Option<MotionContext> {
        if self.history.is_some() {
            return Some(MotionContext::History);
        }
        let section = || {
            self.current_section().map(|(owner, section)| {
                let plugin = match owner {
                    Owner::Plugin(p) => Some(self.plugins[p].id().to_string()),
                    _ => None,
                };
                (plugin, section.id.to_string())
            })
        };
        if let Some(setup) = &self.setup {
            let step = match setup.at {
                crate::setup::At::Welcome => "welcome".to_string(),
                crate::setup::At::Scan => "scan".to_string(),
                crate::setup::At::Choose => "choose".to_string(),
                crate::setup::At::Step(i) => format!("step:{i}"),
                crate::setup::At::Ready => "ready".to_string(),
            };
            let (plugin, section) = if self.setup_on_section() {
                section().map(|(p, s)| (p, Some(s))).unwrap_or_default()
            } else {
                (None, None)
            };
            return Some(MotionContext::Setup {
                step,
                plugin,
                section,
            });
        }
        match self.view {
            View::Settings => {
                let (plugin, section) = section()?;
                Some(MotionContext::Settings { plugin, section })
            }
            View::Screen(_) => {
                let (p, screen) = self.screen()?;
                let plugin = &self.plugins[p];
                if !self.ready(p) {
                    return Some(MotionContext::Setup {
                        step: "connection".into(),
                        plugin: Some(plugin.id().into()),
                        section: Some(screen.id.into()),
                    });
                }
                let depth = plugin.depth(screen.id);
                let page = plugin.page_identity(screen.id);
                if depth > 0 && page.is_none() {
                    return None;
                }
                let tab = if depth > 0 || page.is_some() {
                    plugin.tabs(screen.id).map(|tabs| tabs.on)
                } else {
                    None
                };
                Some(MotionContext::Screen {
                    plugin: plugin.id().into(),
                    screen: screen.id.into(),
                    page,
                    depth,
                    tab,
                })
            }
        }
    }

    /// Latest verified fold, until consumed or superseded by another event.
    pub fn motion_fold(&self) -> Option<&MotionFold> {
        self.interaction_motion.fold.as_ref()
    }

    /// Consume once after a committed frontend frame/snapshot. Reading it on
    /// animation ticks must not replay the same input.
    pub fn take_motion_fold(&mut self) -> Option<MotionFold> {
        self.interaction_motion.proof = None;
        self.interaction_motion.fold.take()
    }

    pub(crate) fn set_motion_origin(&mut self, seq: u64, cursor: u64) {
        self.interaction_motion.origin = Some((seq, cursor));
    }

    pub(super) fn finish_motion_event(&mut self) {
        if self.interaction_motion.fold.as_ref().is_some_and(|fold| {
            self.motion_context().as_ref() != Some(&fold.context)
                || self.menu.is_some()
                || self.tour.is_some()
        }) {
            self.interaction_motion.fold = None;
        }
        // A route change must not ask the previous owner for a view under the
        // new route's kind (a screen id is not a settings section id).
        if let Some((plugin, scope, pane, after)) = self.interaction_motion.proof.take()
            && self.interaction_motion.fold.is_some()
            && self.motion_node(plugin, &scope, pane).as_ref() != Some(&Node::Tree(after))
        {
            self.interaction_motion.fold = None;
        }
        self.interaction_motion.origin = None;
    }

    fn fold_evidence(
        &self,
        context: MotionContext,
        pane: PaneId,
        slot: &str,
        path: Vec<String>,
        before_open: bool,
        after_open: bool,
    ) -> MotionFold {
        MotionFold {
            context,
            pane,
            slot: slot.into(),
            path,
            before_open,
            after_open,
            origin_seq: self.interaction_motion.origin.map(|v| v.0),
            origin_cursor: self.interaction_motion.origin.map(|v| v.1),
        }
    }

    pub(super) fn settings_fold_before(&self, group: &str) -> Option<(MotionContext, bool)> {
        if !self.interaction_motion.enabled
            || group.is_empty()
            || self.setup.is_some()
            || self.view != View::Settings
            || !self
                .sections
                .iter()
                .any(|(_, section)| section.group == group)
        {
            return None;
        }
        Some((self.motion_context()?, !self.folded.contains(group)))
    }

    pub(super) fn settings_fold_after(
        &mut self,
        group: &str,
        before: Option<(MotionContext, bool)>,
    ) {
        let Some((context, before_open)) = before else {
            return;
        };
        let after_open = !self.folded.contains(group);
        if before_open != after_open && self.motion_context().as_ref() == Some(&context) {
            self.interaction_motion.fold = Some(self.fold_evidence(
                context,
                PaneId::Nav,
                SETTINGS_NAV_SLOT,
                vec![crate::view::chrome::targets::nav_group(group).0],
                before_open,
                after_open,
            ));
        }
    }

    /// Observe around the existing dispatch exactly once. Fold and Activate
    /// preserve their original semantics, including activation side effects.
    /// Legacy raw fold keys are checked at this same boundary, never rewritten.
    pub(super) fn motion_part_input(
        &mut self,
        p: usize,
        scope: &str,
        pane: Pane,
        intent: &Intent,
        known: Option<&Node>,
    ) -> Outcome {
        // Action ids keep their dispatch semantics. Only their declared fold
        // key is used to choose what group to verify around that dispatch.
        let semantic = if self.interaction_motion.enabled {
            match intent {
                Intent::Invoke { action } => intent
                    .chord()
                    .or_else(|| self.action_chord(p, scope, action))
                    .map(|chord| Intent::Key { chord }),
                _ => None,
            }
        } else {
            None
        };
        let observation = semantic.as_ref().unwrap_or(intent);
        let observe = self.interaction_motion.enabled
            && fold_candidate(observation)
            && !self.plugins[p].capturing()
            && self.menu.is_none()
            && self.tour.is_none();
        let before = if observe {
            let node = known.cloned().or_else(|| self.motion_node(p, scope, pane));
            node.and_then(|node| match node {
                Node::Tree(tree) => {
                    let context = self.motion_context()?;
                    let (path, requested) = fold_target(&tree, observation)?;
                    Some((context, tree, path, requested))
                }
                _ => None,
            })
        } else {
            None
        };
        let outcome = crate::route::to_part(self.plugins[p].as_mut(), scope, pane, intent);
        if let Some((context, before, path, requested)) = before
            && matches!(outcome, Outcome::Handled(_))
            && self.motion_context().as_ref() == Some(&context)
            && let Some(Node::Tree(after)) = self.motion_node(p, scope, pane)
            && verified_fold(&before, &after, &path, requested)
        {
            self.interaction_motion.fold = Some(self.fold_evidence(
                context,
                pane.into(),
                BODY_SLOT,
                path.clone(),
                before.open.contains(&path),
                after.open.contains(&path),
            ));
            self.interaction_motion.proof = Some((p, scope.into(), pane, after));
        }
        outcome
    }

    pub(super) fn motion_node(&self, p: usize, scope: &str, pane: Pane) -> Option<Node> {
        if self.view == View::Settings {
            return Some(self.section_view(p, scope)?.body);
        }
        let view = self.screen_view(p, scope)?;
        screen_node(&view, pane).cloned()
    }
}

pub(super) fn screen_node(view: &crate::view::ScreenView, pane: Pane) -> Option<&Node> {
    let panes = view
        .page
        .as_ref()
        .map_or(view.panes.as_slice(), |page| &page.panes);
    let mut matching = panes.iter().filter(|view| view.pane == PaneId::from(pane));
    let node = &matching.next()?.body;
    // A pane id reused in a compound layout is not an addressable tree.
    matching.next().is_none().then_some(node)
}

fn fold_candidate(intent: &Intent) -> bool {
    match intent {
        Intent::Fold { .. } | Intent::Activate { .. } => true,
        Intent::Key { chord } => {
            !chord.ctrl
                && !chord.alt
                && matches!(chord.key, Key::Enter | Key::Left | Key::Right | Key::Char(' ' | '+' | '-'))
        }
        _ => false,
    }
}

fn group_at<'a>(tree: &'a Tree, path: &[String]) -> Option<&'a TreeRow> {
    let mut rows = tree.rows.as_slice();
    let mut group = None;
    for id in path {
        // Duplicate sibling ids are ambiguous, even if a renderer draws them.
        let mut matches = rows.iter().filter(|row| row.id == *id);
        let row = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        group = Some(row);
        rows = &row.children;
    }
    group.filter(|row| !row.children.is_empty())
}

fn fold_target(tree: &Tree, intent: &Intent) -> Option<(Vec<String>, Option<bool>)> {
    let (path, requested) = match intent {
        Intent::Fold { target, open } => (TreeRow::path_of(target), Some(*open)),
        Intent::Activate { target } => (TreeRow::path_of(target), None),
        Intent::Key { chord } if fold_candidate(intent) => {
            if let Some(Intent::Fold { target, open }) = crate::view::tree_step(tree, *chord) {
                (TreeRow::path_of(&target), Some(open))
            } else {
                let requested = match chord.key {
                    Key::Char('+') => Some(true),
                    Key::Char('-') => Some(false),
                    _ => None,
                };
                (tree.selected.clone(), requested)
            }
        }
        _ => return None,
    };
    group_at(tree, &path)?;
    Some((path, requested))
}

/// Pure verifier: only one existing group changed, in the requested direction.
/// Background discovery, new rows, reorder and selection alone are not folds.
fn verified_fold(before: &Tree, after: &Tree, path: &[String], requested: Option<bool>) -> bool {
    if group_at(before, path).is_none()
        || group_at(after, path).is_none()
        || before.rows != after.rows
    {
        return false;
    }
    let was = before.open.iter().any(|p| p == path);
    let now = after.open.iter().any(|p| p == path);
    if was == now || requested.is_some_and(|want| now != want) {
        return false;
    }
    let others = |tree: &Tree| {
        tree.open
            .iter()
            .filter(|p| p.as_slice() != path)
            .cloned()
            .collect::<std::collections::BTreeSet<_>>()
    };
    others(before) == others(after)
}

#[cfg(test)]
#[path = "host_motion_tests.rs"]
pub(crate) mod tests;
