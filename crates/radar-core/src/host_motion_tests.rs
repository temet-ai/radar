//! Pure verification cases and a small host/plugin dispatch contract.

use super::*;
use crate::host::{Event, Hit, MouseKind};
use crate::plugin::{Effect, PageTabs, Plugin, ScreenSpec, SectionSpec};
use crate::view::{Chord, PageView, PaneView, ScreenView, SectionView, Target, TreeNav};
use std::sync::{Arc, Mutex};

fn path(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| (*s).into()).collect()
}

fn tree() -> Tree {
    Tree {
        rows: vec![
            TreeRow::new("watch", "Watch").children([
                TreeRow::new("cases", "Cases").child(TreeRow::new("WATCH-12", "Case twelve")),
                TreeRow::new("vimes", "Sam Vimes"),
            ]),
            TreeRow::new("guild", "Guild").child(
                TreeRow::new("cases", "Cases").child(TreeRow::new("WATCH-13", "Case thirteen")),
            ),
        ],
        open: vec![
            path(&["watch"]),
            path(&["watch", "cases"]),
            path(&["guild"]),
            path(&["guild", "cases"]),
        ],
        selected: path(&["watch"]),
        ..Tree::default()
    }
}

mod unit {
    use super::*;

    #[test]
    fn only_the_exact_existing_group_and_requested_direction_verify() {
        let before = tree();
        let group = path(&["watch", "cases"]);
        let mut after = before.clone();
        after.open.retain(|p| p != &group);
        assert!(verified_fold(&before, &after, &group, Some(false)));
        assert!(!verified_fold(&before, &after, &group, Some(true)));
        assert!(
            !verified_fold(&before, &after, &path(&["guild", "cases"]), None),
            "duplicate group id in a different full path"
        );
        assert!(!verified_fold(&before, &after, &path(&["cases"]), None));
        assert!(
            !verified_fold(&before, &before, &group, None),
            "accepted no-op is not a fold"
        );
        assert!(verified_fold(&after, &before, &group, Some(true)));
    }

    #[test]
    fn selection_refresh_reorder_and_auto_open_never_verify_as_a_fold() {
        let before = tree();
        let group = path(&["watch"]);
        let mut after = before.clone();
        after.selected = path(&["watch", "vimes"]);
        assert!(!verified_fold(&before, &after, &group, None));
        after.open.retain(|p| p != &group);
        after.rows.swap(0, 1);
        assert!(!verified_fold(&before, &after, &group, None));
        after = before.clone();
        after.open.retain(|p| p != &group);
        after.rows[0].name = "Renamed Watch".into();
        assert!(!verified_fold(&before, &after, &group, None));
        after = before.clone();
        after.open.retain(|p| p != &group);
        after.open.push(path(&["newly-discovered"]));
        assert!(!verified_fold(&before, &after, &group, None));
        after = before.clone();
        after.rows.push(after.rows[0].clone());
        assert!(
            group_at(&after, &group).is_none(),
            "duplicate sibling identity is ambiguous"
        );
    }

    #[test]
    fn pane_identity_selects_one_tree_and_reused_pane_ids_are_ambiguous() {
        let view = ScreenView::new(
            "watch",
            vec![
                PaneView::new(PaneId::Nav, "left", Node::Tree(tree())),
                PaneView::new(PaneId::Middle, "right", Node::Tree(Tree::default())),
            ],
        );
        assert!(
            matches!(screen_node(&view, Pane::Nav), Some(Node::Tree(tree)) if !tree.rows.is_empty())
        );
        assert!(
            matches!(screen_node(&view, Pane::Middle), Some(Node::Tree(tree)) if tree.rows.is_empty())
        );
        let mut ambiguous = view;
        ambiguous.panes.push(ambiguous.panes[0].clone());
        assert!(screen_node(&ambiguous, Pane::Nav).is_none());
        assert!(screen_node(&ambiguous, Pane::Detail).is_none());
    }

    #[test]
    fn raw_fold_keys_and_activate_preserve_the_selected_group_path() {
        let mut tree = tree();
        let activate = Intent::Activate {
            target: TreeRow::target_of(&path(&["watch", "cases"])),
        };
        assert_eq!(
            fold_target(&tree, &activate),
            Some((path(&["watch", "cases"]), None))
        );
        tree.selected = path(&["watch", "vimes"]);
        assert_eq!(
            fold_target(
                &tree,
                &Intent::Key {
                    chord: Chord::parse("-").unwrap()
                }
            ),
            Some((path(&["watch"]), Some(false)))
        );
        assert!(
            fold_target(
                &tree,
                &Intent::Activate {
                    target: TreeRow::target_of(&tree.selected)
                }
            )
            .is_none()
        );
        assert!(!fold_candidate(&Intent::Key {
            chord: Chord::ctrl('-')
        }));
        assert!(!fold_candidate(&Intent::Select {
            target: Target::new("watch")
        }));
    }
}

/// Owned offline values only: no demo constructor, provider, clock, filesystem,
/// environment discovery, task or full application frame.
#[derive(Clone)]
pub(crate) struct ProbeState {
    pub(crate) tree: Tree,
    pub(crate) calls: Vec<Intent>,
    pub(crate) panes: Vec<Pane>,
    pub(crate) hidden_root: bool,
    pub(crate) empty_page: bool,
    pub(crate) page: Option<String>,
    pub(crate) keyed: bool,
    pub(crate) tab: usize,
    pub(crate) title: String,
    pub(crate) ignored: bool,
    pub(crate) change_page: bool,
    pub(crate) refresh_after_fold: bool,
    pub(crate) views: usize,
}

struct Probe(Arc<Mutex<ProbeState>>);

impl Plugin for Probe {
    fn id(&self) -> &'static str {
        "watch-probe"
    }
    fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
        Ok(())
    }
    fn config(&self) -> Option<toml::Value> {
        None
    }
    fn screens(&self) -> Vec<ScreenSpec> {
        vec![ScreenSpec {
            id: "watch",
            name: "Watch",
            key: 'W',
        }]
    }
    fn sections(&self) -> Vec<SectionSpec> {
        vec![SectionSpec {
            id: "cases",
            name: "cases",
            group: "watch",
        }]
    }
    fn depth(&self, _: &str) -> usize {
        usize::from(self.0.lock().unwrap().page.is_some())
    }
    fn page_identity(&self, _: &str) -> Option<String> {
        let state = self.0.lock().unwrap();
        state.keyed.then(|| state.page.clone()).flatten()
    }
    fn tabs(&self, _: &str) -> Option<PageTabs> {
        let state = self.0.lock().unwrap();
        state.page.as_ref().map(|_| PageTabs {
            names: vec!["case".into(), "history".into()],
            on: state.tab,
        })
    }
    fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<ScreenView> {
        let mut state = self.0.lock().unwrap();
        state.views += 1;
        let panes = vec![PaneView::new(
            PaneId::Middle,
            &state.title,
            Node::Tree(state.tree.clone()),
        )];
        let mut view = if state.page.is_some() {
            ScreenView::page(
                scope,
                PageView {
                    crumbs: vec![],
                    title: state.title.clone().into(),
                    details: Default::default(),
                    under: None,
                    tabs: vec![],
                    tab: state.tab,
                    notice: None,
                    panes: if state.empty_page { vec![] } else { panes },
                    layout: None,
                },
            )
        } else {
            ScreenView::new(scope, panes)
        };
        if state.page.is_some() && state.hidden_root {
            let mut root = state.tree.clone();
            root.rows.push(TreeRow::new("root-only", "Hidden root row"));
            view.panes
                .push(PaneView::new(PaneId::Nav, "hidden root", Node::Tree(root)));
        }
        view.actions.push(crate::view::ActionSpec {
                mnemonic_at: None,
                mnemonic_shift: false,            id: "watch.toggle".into(),
            label: "toggle group".into(),
            key: Some(Chord::char(' ')),
            heading: false,
            icon: None,
            change: None,
        });
        Some(view)
    }
    fn section_view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<SectionView> {
        let mut state = self.0.lock().unwrap();
        state.views += 1;
        Some(SectionView {
            version: crate::view::VERSION,
            scope: scope.into(),
            body: Node::Tree(state.tree.clone()),
            detail_title: String::new(),
            detail: vec![],
            detail_blocks: vec![],
            actions: vec![],
        })
    }
    fn intent(&mut self, _: &str, pane: Pane, intent: &Intent) -> Outcome {
        let mut state = self.0.lock().unwrap();
        state.calls.push(intent.clone());
        state.panes.push(pane);
        if state.ignored {
            return Outcome::Ignored;
        }
        if matches!(intent, Intent::Key { chord } if chord.key == Key::Char('r')) {
            state.tree.rows[0].name = "Watch refreshed by routed effect".into();
            return Outcome::done();
        }
        let mut nav = TreeNav::default();
        nav.selected = state.tree.selected.clone();
        nav.open = state.tree.open.clone();
        let handled = match intent {
            Intent::Activate { target } => {
                nav.select(TreeRow::path_of(target));
                if group_at(&state.tree, nav.selected()).is_some() {
                    nav.toggle_selected();
                }
                true
            }
            Intent::Key { .. } | Intent::Invoke { .. } => {
                let semantic = match intent {
                    Intent::Invoke { action } if action == "watch.toggle" => Intent::Key {
                        chord: Chord::char(' '),
                    },
                    _ => intent.clone(),
                };
                if let Some((path, requested)) = fold_target(&state.tree, &semantic) {
                    let open = requested.unwrap_or_else(|| !nav.is_open(&path));
                    nav.apply(&Intent::Fold {
                        target: TreeRow::target_of(&path),
                        open,
                    })
                } else {
                    false
                }
            }
            _ => nav.apply(intent),
        };
        if !handled {
            return Outcome::Ignored;
        }
        state.tree.selected = nav.selected.clone();
        state.tree.open = nav.open.clone();
        if state.change_page {
            state.page = Some("WATCH-13".into());
        }
        // Activation's side effect must survive unchanged, exactly once.
        Outcome::Handled(if matches!(intent, Intent::Activate { .. }) {
            vec![Effect::Notify("case activated".into())]
        } else if state.refresh_after_fold && matches!(intent, Intent::Fold { .. }) {
            vec![Effect::Press('r')]
        } else {
            vec![]
        })
    }
}

pub(crate) fn fixture() -> (Host, Arc<Mutex<ProbeState>>) {
    let state = Arc::new(Mutex::new(ProbeState {
        tree: tree(),
        calls: vec![],
        panes: vec![],
        hidden_root: false,
        empty_page: false,
        page: None,
        keyed: true,
        tab: 0,
        title: "Watch cases".into(),
        ignored: false,
        change_page: false,
        refresh_after_fold: false,
        views: 0,
    }));
    let mut host = Host::new(vec![Box::new(Probe(state.clone()))], 0).unwrap();
    host.colours.cfg.motion = "full".into();
    host.set_motion_enabled(true);
    host.set_view_room(80, 24);
    (host, state)
}

mod contract {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn native_keys_and_ink_intents_verify_once_without_changing_activation_semantics() {
        let (mut host, state) = fixture();
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('-'),
            KeyModifiers::NONE,
        )));
        let closed = host.take_motion_fold().expect("native routed fold");
        assert_eq!(
            (closed.pane, closed.slot.as_str(), closed.path),
            (PaneId::Middle, BODY_SLOT, path(&["watch"]))
        );
        assert!(closed.before_open && !closed.after_open);
        assert!(closed.origin_seq.is_none() && closed.origin_cursor.is_none());
        assert_eq!(state.lock().unwrap().calls.len(), 1);
        host.update(Event::Intent(Intent::Activate {
            target: Target::new("watch"),
        }));
        assert!(!host.motion_fold().unwrap().before_open && host.motion_fold().unwrap().after_open);
        assert_eq!(host.message.as_ref().unwrap().text, "case activated");
        assert_eq!(state.lock().unwrap().calls.len(), 2);
        // Native Enter translates to group Activate rather than replacing it
        // with Fold, preserving the same effect and actual final selection.
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        )));
        assert!(!host.motion_fold().unwrap().after_open);
        let state = state.lock().unwrap();
        assert_eq!(state.calls.len(), 3);
        assert!(matches!(state.calls.last(), Some(Intent::Activate { .. })));
        assert_eq!(state.tree.selected, path(&["watch"]));
    }

    #[test]
    fn rejected_noop_leaf_route_change_and_newer_input_discard_evidence() {
        let (mut host, state) = fixture();
        let close = || Intent::Fold {
            target: Target::new("watch"),
            open: false,
        };
        state.lock().unwrap().ignored = true;
        host.update(Event::Intent(close()));
        assert!(host.motion_fold().is_none());
        state.lock().unwrap().ignored = false;
        host.update(Event::Intent(close()));
        assert!(host.motion_fold().is_some());
        host.update(Event::Intent(close()));
        assert!(
            host.motion_fold().is_none(),
            "no-op acknowledgement does not retain prior fold"
        );
        host.update(Event::Intent(Intent::Activate {
            target: TreeRow::target_of(&path(&["guild", "cases", "WATCH-13"])),
        }));
        assert!(
            host.motion_fold().is_none(),
            "leaf activation is not a group fold"
        );
        state.lock().unwrap().change_page = true;
        host.update(Event::Intent(Intent::Activate {
            target: Target::new("watch"),
        }));
        assert!(
            host.motion_fold().is_none(),
            "legitimate activation can replace the page"
        );
        state.lock().unwrap().change_page = false;
        host.update(Event::Intent(close()));
        assert!(host.motion_fold().is_some());
        host.update(Event::Tick(1));
        assert!(
            host.motion_fold().is_none(),
            "background work never borrows input causality"
        );
    }

    #[test]
    fn context_tracks_page_identity_and_tab_without_building_views() {
        let (mut host, state) = fixture();
        let initial = host.motion_context();
        let views = state.lock().unwrap().views;
        state.lock().unwrap().title = "Renamed Watch".into();
        assert_eq!(host.motion_context(), initial);
        state.lock().unwrap().page = Some("WATCH-12".into());
        let first = host.motion_context();
        state.lock().unwrap().tree.selected = path(&["guild"]);
        assert_eq!(host.motion_context(), first);
        state.lock().unwrap().page = Some("WATCH-13".into());
        assert_ne!(
            host.motion_context(),
            first,
            "same depth, distinct actual page"
        );
        let second = host.motion_context();
        state.lock().unwrap().tab = 1;
        assert_ne!(host.motion_context(), second);
        state.lock().unwrap().keyed = false;
        assert!(host.motion_context().is_none());
        host.update(Event::Tick(1));
        assert_eq!(
            state.lock().unwrap().views,
            views,
            "identity and idle ticks never reconstruct a view"
        );
    }

    #[test]
    fn page_tree_targets_route_to_actual_page_panes_without_hidden_root_fallback() {
        let (mut host, state) = fixture();
        {
            let mut state = state.lock().unwrap();
            state.page = Some("WATCH-12".into());
            state.hidden_root = true;
        }
        host.focus = Pane::Nav;
        host.update(Event::Intent(Intent::Select {
            target: Target::new("watch"),
        }));
        assert_eq!(
            host.focus,
            Pane::Middle,
            "actual page beats hidden root's identical target"
        );
        host.focus = Pane::Detail;
        host.update(Event::Intent(Intent::Activate {
            target: Target::new("watch"),
        }));
        assert_eq!(host.focus, Pane::Middle);
        assert_eq!(host.motion_fold().unwrap().pane, PaneId::Middle);
        assert!(!host.motion_fold().unwrap().after_open);
        assert_eq!(
            state.lock().unwrap().panes,
            vec![Pane::Middle, Pane::Middle]
        );
        assert_eq!(
            state.lock().unwrap().calls.len(),
            2,
            "selection and activation each dispatch once"
        );
        host.focus = Pane::Detail;
        host.update(Event::Intent(Intent::Select {
            target: Target::new("root-only"),
        }));
        assert_eq!(
            host.focus,
            Pane::Detail,
            "a hidden root target does not steal focus"
        );
        state.lock().unwrap().empty_page = true;
        host.update(Event::Intent(Intent::Select {
            target: Target::new("watch"),
        }));
        assert_eq!(
            host.focus,
            Pane::Detail,
            "an empty actual page does not fall back to hidden root panes"
        );
        state.lock().unwrap().page = None;
        host.update(Event::Intent(Intent::Select {
            target: Target::new("watch"),
        }));
        assert_eq!(
            host.focus,
            Pane::Middle,
            "without a page, ordinary root routing is unchanged"
        );
    }

    #[test]
    fn native_section_and_settings_group_paths_have_distinct_tree_slots() {
        let (mut host, state) = fixture();
        host.view = View::Settings;
        host.section = host
            .sections
            .iter()
            .position(|(_, section)| section.id == "cases")
            .unwrap();
        host.focus = Pane::Middle;
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('-'),
            KeyModifiers::NONE,
        )));
        let fold = host.take_motion_fold().unwrap();
        assert_eq!((fold.pane, fold.slot.as_str()), (PaneId::Middle, BODY_SLOT));
        assert_eq!(state.lock().unwrap().calls.len(), 1);
        host.focus = Pane::Nav;
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('-'),
            KeyModifiers::NONE,
        )));
        let fold = host.take_motion_fold().unwrap();
        assert_eq!(
            (fold.pane, fold.slot.as_str(), fold.path),
            (
                PaneId::Nav,
                SETTINGS_NAV_SLOT,
                path(&["chrome:group:watch"])
            )
        );
        host.hits
            .borrow_mut()
            .push((ratatui::layout::Rect::new(0, 0, 10, 1), Hit::Group("watch")));
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: 1,
            row: 0,
            at_ms: 1,
        });
        assert!(
            host.motion_fold().unwrap().after_open,
            "native settings group click verified"
        );
    }

    #[test]
    fn verification_is_disabled_without_full_and_raw_keys_keep_their_semantics() {
        let (mut host, state) = fixture();
        host.set_motion_enabled(false);
        let close = Intent::Fold {
            target: Target::new("watch"),
            open: false,
        };
        let views = state.lock().unwrap().views;
        host.motion_part_input(0, "watch", Pane::Middle, &close, None);
        assert!(host.motion_fold().is_none());
        assert_eq!(
            state.lock().unwrap().views,
            views,
            "no additional views while disabled"
        );
        // The real safety wrapper must also avoid projections for an
        // unrelated raw key; tree admission does not make every key a view.
        let unrelated = Intent::Key { chord: Chord::char('v') };
        host.shared_part_input(0, "watch", Pane::Middle, &unrelated, None);
        assert_eq!(state.lock().unwrap().views, views);
        assert_eq!(state.lock().unwrap().calls.last(), Some(&unrelated));
        host.set_motion_enabled(true);
        let raw = Intent::Key {
            chord: Chord::parse(" ").unwrap(),
        };
        host.motion_part_input(0, "watch", Pane::Middle, &raw, None);
        assert!(host.motion_fold().unwrap().after_open);
        assert_eq!(
            state.lock().unwrap().calls.last(),
            Some(&raw),
            "legacy raw key forwarded unchanged"
        );
        let invoked = Intent::Invoke {
            action: "watch.toggle".into(),
        };
        host.update(Event::Intent(invoked.clone()));
        assert!(!host.motion_fold().unwrap().after_open);
        assert_eq!(
            state.lock().unwrap().calls.last(),
            Some(&invoked),
            "declared fold action forwarded unchanged"
        );
    }

    #[test]
    fn synchronous_routed_effect_replacement_invalidates_fold_evidence() {
        let (mut host, state) = fixture();
        state.lock().unwrap().refresh_after_fold = true;
        host.update(Event::Intent(Intent::Fold {
            target: Target::new("watch"),
            open: false,
        }));
        let state = state.lock().unwrap();
        assert_eq!(
            state.calls.len(),
            2,
            "one fold and its existing follow-on action"
        );
        assert_eq!(
            state.tree.rows[0].name.words(),
            "Watch refreshed by routed effect"
        );
        assert!(
            !state.tree.open.contains(&path(&["watch"])),
            "real fold still happened"
        );
        assert!(
            host.motion_fold().is_none(),
            "stale pre-effect picture is never announced"
        );
    }
}
