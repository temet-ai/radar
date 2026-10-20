use super::*;
use crate::host::{Event, OpenMenu, Owner};
use crate::plugin::{Effect, Menu, Plugin, ScreenSpec, SectionSpec};
use crate::view::{
    ActionSpec, Chord, ListRow, PaneId, PaneView, ScreenView, SectionView, Target, TreeNav, ViewCx,
};
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::sync::{Arc, Mutex};

fn path(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).into()).collect()
}

fn tree() -> Tree {
    Tree {
        rows: vec![
            TreeRow::new("watch", "Watch").children([
                TreeRow::new("cases", "Cases").child(TreeRow::new("WATCH-12", "Case twelve")),
                TreeRow::new("vimes", "Sam Vimes"),
            ]),
            TreeRow::new("guild", "Guild").child(TreeRow::new("WATCH-13", "Case thirteen")),
        ],
        selected: path(&["watch"]),
        ..Tree::default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelCapture {
    Overlay,
    OtherPaneEditor,
}

struct State {
    tree: Tree,
    calls: Vec<Intent>,
    capture: bool,
    overlay: bool,
    capture_after_fold: Option<ModelCapture>,
    replace: bool,
    effect: bool,
    folder_icons: bool,
    replace_payload: bool,
    ambiguous: bool,
}

impl State {
    fn view_tree(&self) -> Tree {
        fn icons(rows: &mut [TreeRow], prefix: &[String], open: &[Vec<String>]) {
            for row in rows {
                if row.children.is_empty() { continue; }
                let mut path = prefix.to_vec();
                path.push(row.id.clone());
                row.name.0.insert(0, crate::view::Seg::icon(if open.contains(&path) {
                    crate::ui::icon::Icon::FolderOpen
                } else {
                    crate::ui::icon::Icon::Folder
                }));
                icons(&mut row.children, &path, open);
            }
        }
        let mut tree = self.tree.clone();
        if self.folder_icons { icons(&mut tree.rows, &[], &tree.open); }
        tree
    }
}

struct Probe(Arc<Mutex<State>>);
impl Plugin for Probe {
    fn id(&self) -> &'static str {
        "watch-tree"
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
            name: "Cases",
            group: "watch",
        }]
    }
    fn depth(&self, _: &str) -> usize {
        usize::from(self.0.lock().unwrap().ambiguous)
    }
    fn capturing(&self) -> bool {
        self.0.lock().unwrap().capture
    }
    fn view(&self, scope: &str, _: &ViewCx<'_>) -> Option<ScreenView> {
        let state = self.0.lock().unwrap();
        let capture = state
            .capture_after_fold
            .filter(|_| !state.tree.open.is_empty());
        let mut view = ScreenView::new(
            scope,
            vec![
                PaneView::new(
                    PaneId::Nav,
                    "Views",
                    Node::List {
                        rows: vec![ListRow::new("view:watch", "Watch")],
                        selected: Some(0),
                        editing: (capture == Some(ModelCapture::OtherPaneEditor)).then(|| {
                            crate::view::Editing {
                                row: 0,
                                at: 0,
                                text: "Watch".into(),
                                caret: 5,
                            }
                        }),
                    },
                ),
                PaneView::new(PaneId::Middle, "Cases", Node::Tree(state.view_tree())),
                PaneView::new(
                    PaneId::Detail,
                    "About",
                    Node::Document {
                        lines: vec![],
                        cut: false,
                    },
                ),
            ],
        );
        view.actions.push(ActionSpec::named(
            "watch.open_all",
            &Binding::new("=", "expand every held group"),
        ));
        if state.overlay || capture == Some(ModelCapture::Overlay) {
            view.overlay = Some(Box::new(PaneView::new(
                PaneId::Middle,
                "Question",
                Node::Document {
                    lines: vec![],
                    cut: false,
                },
            )));
        }
        Some(view)
    }
    fn section_view(&self, scope: &str, _: &ViewCx<'_>) -> Option<SectionView> {
        Some(SectionView {
            version: crate::view::VERSION,
            scope: scope.into(),
            body: Node::Tree(self.0.lock().unwrap().view_tree()),
            detail_title: String::new(),
            detail: vec![],
            detail_blocks: vec![],
            actions: vec![],
        })
    }
    fn intent(&mut self, _: &str, _: Pane, intent: &Intent) -> Outcome {
        let mut state = self.0.lock().unwrap();
        assert!(
            !matches!(intent, Intent::OpenAll),
            "open-all must be host-owned"
        );
        state.calls.push(intent.clone());
        if state.capture || state.overlay {
            return Outcome::done();
        }
        let mut nav = TreeNav::default();
        nav.selected = state.tree.selected.clone();
        nav.open = state.tree.open.clone();
        if !nav.apply(intent) {
            return Outcome::Ignored;
        }
        state.tree.selected = nav.selected;
        state.tree.open = nav.open;
        if matches!(intent, Intent::Fold { .. }) {
            if state.replace {
                state.tree.rows.pop();
            }
            if state.replace_payload {
                state.tree.rows[1].cells.push(crate::view::CellView {
                    heading: "Action".into(),
                    text: "changed action".into(),
                    show: None,
                    target: Some(Target::new("watch:replacement-action")),
                });
            }
            if state.effect {
                return Outcome::Handled(vec![Effect::Focus(Pane::Detail)]);
            }
        }
        Outcome::done()
    }
}

fn fixture() -> (Host, Arc<Mutex<State>>) {
    let state = Arc::new(Mutex::new(State {
        tree: tree(),
        calls: vec![],
        capture: false,
        overlay: false,
        capture_after_fold: None,
        replace: false,
        effect: false,
        folder_icons: false,
        replace_payload: false,
        ambiguous: false,
    }));
    let mut host = Host::new(vec![Box::new(Probe(state.clone()))], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host.view = View::Screen(0);
    host.focus = Pane::Middle;
    host.bar = false;
    (host, state)
}

fn press(host: &mut Host, code: KeyCode) {
    host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

#[test]
fn raw_keys_intents_action_fallback_and_menu_expand_the_same_held_groups() {
    for route in 0..4 {
        let (mut host, state) = fixture();
        match route {
            0 => press(&mut host, KeyCode::Char('=')),
            1 => {
                host.update(Event::Intent(Intent::OpenAll));
            }
            2 => {
                host.update(Event::Intent(Intent::Invoke {
                    action: "watch.open_all".into(),
                }));
            }
            _ => {
                press(&mut host, KeyCode::Char('x'));
                press(&mut host, KeyCode::Char('='));
            }
        }
        let state = state.lock().unwrap();
        assert_eq!(state.tree.open, state.tree.groups(), "route {route}");
        assert_eq!(state.tree.selected, path(&["watch"]));
        let folds: Vec<_> = state
            .calls
            .iter()
            .filter_map(|intent| match intent {
                Intent::Fold { target, open: true } => Some(TreeRow::path_of(target)),
                _ => None,
            })
            .collect();
        assert_eq!(
            folds,
            state.tree.groups(),
            "parent-first, once per held group"
        );
    }
}

#[test]
fn horizontal_arrows_keep_tree_ownership_and_buffered_keys_use_current_folds() {
    let (mut host, state) = fixture();
    press(&mut host, KeyCode::Left);
    assert_eq!(host.focus, Pane::Middle);
    press(&mut host, KeyCode::Right); // Open Watch.
    press(&mut host, KeyCode::Right); // Select Cases.
    press(&mut host, KeyCode::Right); // Open Cases.
    press(&mut host, KeyCode::Right); // Select WATCH-12.
    assert_eq!(
        state.lock().unwrap().tree.selected,
        path(&["watch", "cases", "WATCH-12"])
    );
    press(&mut host, KeyCode::Right); // Leaf: no pane jump.
    assert_eq!(host.focus, Pane::Middle);
    press(&mut host, KeyCode::Left); // Select parent; keep it open.
    assert_eq!(
        state.lock().unwrap().tree.selected,
        path(&["watch", "cases"])
    );
    assert!(
        state
            .lock()
            .unwrap()
            .tree
            .open
            .contains(&path(&["watch", "cases"]))
    );
    press(&mut host, KeyCode::Left); // Close selected Cases.
    press(&mut host, KeyCode::Down); // Visible next row, not now-hidden WATCH-12.
    assert_eq!(
        state.lock().unwrap().tree.selected,
        path(&["watch", "vimes"])
    );
    press(&mut host, KeyCode::Char('-')); // Close/select Watch.
    assert_eq!(state.lock().unwrap().tree.selected, path(&["watch"]));
    assert!(state.lock().unwrap().tree.open.is_empty());
}

#[test]
fn up_and_k_still_reach_the_bar_and_pane_keys_keep_their_original_route() {
    for key in [KeyCode::Up, KeyCode::Char('k')] {
        let (mut host, _) = fixture();
        press(&mut host, key);
        assert!(host.bar);
    }
    for (key, pane) in [
        (KeyCode::Char('h'), Pane::Nav),
        (KeyCode::Char('l'), Pane::Detail),
        (KeyCode::Tab, Pane::Detail),
        (KeyCode::BackTab, Pane::Nav),
        (KeyCode::Char('1'), Pane::Nav),
    ] {
        let (mut host, _) = fixture();
        press(&mut host, key);
        assert_eq!(host.focus, pane, "{key:?}");
    }
}

#[test]
fn empty_trees_and_leaf_boundaries_do_not_leak_horizontal_arrows() {
    let (mut host, state) = fixture();
    state.lock().unwrap().tree = Tree::default();
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::Char('='),
        KeyCode::Char('-'),
    ] {
        press(&mut host, code);
        assert_eq!(host.focus, Pane::Middle);
    }
    assert!(state.lock().unwrap().calls.is_empty());
}

#[test]
fn stale_hidden_heading_and_non_group_targets_cannot_change_a_tree() {
    let (mut host, state) = fixture();
    for intent in [
        Intent::Select {
            target: TreeRow::target_of(&path(&["watch", "vimes"])),
        },
        Intent::Select {
            target: Target::new("missing"),
        },
        Intent::Fold {
            target: Target::new("missing"),
            open: true,
        },
        Intent::Fold {
            target: TreeRow::target_of(&path(&["watch", "vimes"])),
            open: true,
        },
    ] {
        host.update(Event::Intent(intent));
    }
    assert!(state.lock().unwrap().calls.is_empty());
    assert_eq!(state.lock().unwrap().tree.selected, path(&["watch"]));
    state.lock().unwrap().tree.rows[0].heading = true;
    host.update(Event::Intent(Intent::Select {
        target: Target::new("watch"),
    }));
    assert!(state.lock().unwrap().calls.is_empty());
}

#[test]
fn menu_replay_does_not_retarget_after_selection_or_held_data_changes() {
    for replace in [false, true] {
        let (mut host, state) = fixture();
        press(&mut host, KeyCode::Char('x'));
        if replace {
            state.lock().unwrap().tree.rows.pop();
        } else {
            state.lock().unwrap().tree.selected = path(&["guild"]);
        }
        press(&mut host, KeyCode::Char('='));
        assert!(state.lock().unwrap().calls.is_empty());
        assert!(state.lock().unwrap().tree.open.is_empty());
    }
}

#[test]
fn open_all_stops_before_reusing_paths_after_owner_data_or_effect_changes() {
    for effect in [false, true] {
        let (mut host, state) = fixture();
        {
            let mut state = state.lock().unwrap();
            state.effect = effect;
            state.replace = !effect;
        }
        host.update(Event::Intent(Intent::OpenAll));
        let state = state.lock().unwrap();
        assert_eq!(state.calls.len(), 1);
        assert_eq!(state.tree.open, vec![path(&["watch"])]);
        if effect {
            assert_eq!(host.focus, Pane::Detail);
        }
    }
}

#[test]
fn open_all_stops_when_a_fold_opens_model_only_capture_without_effects() {
    for capture in [ModelCapture::Overlay, ModelCapture::OtherPaneEditor] {
        let (mut host, state) = fixture();
        state.lock().unwrap().capture_after_fold = Some(capture);
        let before = state.lock().unwrap().tree.clone();
        host.update(Event::Intent(Intent::OpenAll));
        let state = state.lock().unwrap();
        assert!(!state.capture, "only the view declares {capture:?}");
        assert_eq!(state.tree.rows, before.rows);
        assert_eq!(state.tree.selected, before.selected);
        assert_eq!(state.tree.open, vec![path(&["watch"])]);
        assert_eq!(
            state.calls,
            vec![Intent::Fold {
                target: TreeRow::target_of(&path(&["watch"])),
                open: true,
            }],
            "no subsequent fold reaches the owner behind {capture:?}"
        );
        assert_eq!(host.focus, Pane::Middle);
        assert!(host.menu.is_none());
    }
}

#[test]
fn editors_menus_topbar_dock_and_modified_arrows_keep_precedence() {
    for overlay in [false, true] {
        let (mut host, state) = fixture();
        {
            let mut state = state.lock().unwrap();
            state.capture = !overlay;
            state.overlay = overlay;
        }
        press(&mut host, KeyCode::Right);
        host.update(Event::Intent(Intent::OpenAll));
        let state = state.lock().unwrap();
        assert!(state.tree.open.is_empty());
        assert_eq!(
            state.calls,
            vec![Intent::Key {
                chord: Chord::new(Key::Right)
            }]
        );
    }
    let (mut host, state) = fixture();
    host.menu = Some(OpenMenu::open(
        Owner::Host,
        Menu::confirm("watch-question", "Continue?", "yes", "no"),
    ));
    press(&mut host, KeyCode::Right);
    host.update(Event::Intent(Intent::OpenAll));
    assert!(state.lock().unwrap().calls.is_empty());
    host.menu = None;
    host.bar = true;
    press(&mut host, KeyCode::Right);
    assert!(state.lock().unwrap().calls.is_empty());
    host.bar = false;
    host.dock.visible = true;
    host.dock.focused = true;
    host.update(Event::Intent(Intent::OpenAll));
    assert!(state.lock().unwrap().tree.open.is_empty());
    host.dock.focused = false;
    host.view = View::Screen(0);
    host.focus = Pane::Middle;
    host.update(Event::Intent(Intent::Key {
        chord: Chord {
            shift: true,
            ..Chord::new(Key::Right)
        },
    }));
    assert_eq!(
        host.focus,
        Pane::Detail,
        "modified arrow keeps old pane behavior"
    );
}

#[test]
fn settings_sections_and_navigation_share_open_all_and_parent_operations() {
    let (mut host, state) = fixture();
    host.view = View::Settings;
    host.section = host
        .sections
        .iter()
        .position(|(_, section)| section.id == "cases")
        .unwrap();
    host.focus = Pane::Middle;
    press(&mut host, KeyCode::Char('='));
    {
        let state = state.lock().unwrap();
        assert_eq!(state.tree.open, state.tree.groups());
    }
    host.focus = Pane::Nav;
    host.on_heading = false;
    press(&mut host, KeyCode::Char('-'));
    assert!(host.folded.contains("watch"));
    assert!(host.on_heading);
    press(&mut host, KeyCode::Right);
    assert!(!host.folded.contains("watch"));
    assert!(host.on_heading);
    press(&mut host, KeyCode::Right);
    assert!(!host.on_heading);
    let selected = host.section;
    for (group, _) in host.nav_groups() {
        host.folded.insert(group);
    }
    host.on_heading = true;
    host.update(Event::Intent(Intent::OpenAll));
    assert!(host.folded.is_empty());
    assert_eq!(host.section, selected);
    assert!(host.on_heading);
}

#[test]
fn direct_settings_navigation_folds_validate_group_scope_and_preserve_selection() {
    let (mut host, state) = fixture();
    host.view = View::Settings;
    host.section = host
        .sections
        .iter()
        .position(|(_, section)| section.id == "cases")
        .unwrap();
    host.focus = Pane::Nav;
    host.on_heading = false;
    let group = crate::view::chrome::targets::nav_group("watch");
    let fold = |target: Target, open| Event::Intent(Intent::Fold { target, open });
    host.update(fold(group.clone(), false));
    assert!(host.folded.contains("watch"));
    assert!(host.on_heading);
    let section = host.section;
    host.update(fold(group.clone(), true));
    assert!(!host.folded.contains("watch"));
    assert!(host.on_heading);
    assert_eq!(host.section, section);
    for target in [
        crate::view::chrome::targets::nav_group("missing"),
        crate::view::chrome::targets::nav_section(section),
    ] {
        host.update(fold(target, false));
        assert!(host.folded.is_empty());
    }
    host.focus = Pane::Middle;
    host.update(fold(group.clone(), false));
    assert!(host.folded.is_empty());
    host.focus = Pane::Nav;
    state.lock().unwrap().capture = true;
    host.update(fold(group, false));
    assert!(host.folded.is_empty());
    assert!(state.lock().unwrap().calls.is_empty());
}

#[test]
fn stale_hidden_or_heading_context_never_opens_another_rows_actions() {
    for target in [
        Target::new("missing"),
        TreeRow::target_of(&path(&["watch", "vimes"])),
    ] {
        let (mut host, state) = fixture();
        host.update(Event::Intent(Intent::Context { target }));
        assert!(host.menu.is_none());
        assert!(state.lock().unwrap().calls.is_empty());
        assert_eq!(state.lock().unwrap().tree.selected, path(&["watch"]));
    }
    let (mut host, state) = fixture();
    state.lock().unwrap().tree.rows[0].heading = true;
    host.update(Event::Intent(Intent::Context {
        target: Target::new("watch"),
    }));
    assert!(host.menu.is_none());
    assert!(state.lock().unwrap().calls.is_empty());
    state.lock().unwrap().tree.rows[0].heading = false;
    state.lock().unwrap().overlay = true;
    host.update(Event::Intent(Intent::Context {
        target: Target::new("watch"),
    }));
    assert!(host.menu.is_none());
    assert!(state.lock().unwrap().calls.is_empty());
}

#[test]
fn open_all_action_fallback_obeys_the_same_focus_fence_as_direct_intent() {
    for mask in ["bar", "tabs", "sources", "dock", "capture", "overlay"] {
        for invoked in [false, true] {
            let (mut host, state) = fixture();
            match mask {
                "bar" => host.bar = true,
                "tabs" => host.tabs = true,
                "sources" => host.sources = true,
                "dock" => {
                    host.dock.visible = true;
                    host.dock.focused = true;
                }
                "capture" => state.lock().unwrap().capture = true,
                "overlay" => state.lock().unwrap().overlay = true,
                _ => unreachable!(),
            }
            let before = state.lock().unwrap().tree.clone();
            host.update(Event::Intent(if invoked {
                Intent::Invoke {
                    action: "watch.open_all".into(),
                }
            } else {
                Intent::OpenAll
            }));
            let state = state.lock().unwrap();
            assert_eq!(state.tree, before, "mask {mask}, invoked {invoked}");
            assert!(state.calls.iter().all(|intent| !matches!(
                intent,
                Intent::Fold { .. } | Intent::OpenAll
            )), "no owner fold behind {mask}, invoked {invoked}");
        }
    }
}

#[test]
fn open_all_accepts_fold_derived_folder_icons_but_not_changed_action_payloads() {
    for replace_payload in [false, true] {
        let (mut host, state) = fixture();
        {
            let mut state = state.lock().unwrap();
            state.folder_icons = true;
            state.replace_payload = replace_payload;
        }
        host.update(Event::Intent(Intent::OpenAll));
        let state = state.lock().unwrap();
        if replace_payload {
            assert_eq!(state.tree.open, vec![path(&["watch"])]);
            assert_eq!(state.calls.len(), 1);
        } else {
            assert_eq!(state.tree.open, state.tree.groups());
            assert_eq!(state.calls.len(), state.tree.groups().len());
        }
        assert_eq!(state.tree.selected, path(&["watch"]));
    }
}

#[test]
fn open_all_and_menu_replay_require_a_concrete_item_page_identity() {
    for menu in [false, true] {
        let (mut host, state) = fixture();
        state.lock().unwrap().ambiguous = true;
        assert!(host.motion_context().is_none());
        if menu {
            press(&mut host, KeyCode::Char('x'));
            press(&mut host, KeyCode::Char('='));
        } else {
            host.update(Event::Intent(Intent::OpenAll));
        }
        let state = state.lock().unwrap();
        assert!(state.tree.open.is_empty());
        assert!(state.calls.is_empty());
    }
}

#[test]
fn a_part_s_views_leave_the_host_without_terminal_controls() {
    let (host, state) = fixture();
    {
        let mut state = state.lock().unwrap();
        let row = &mut state.tree.rows[0];
        row.name = crate::view::Text::plain("Watch\u{1b}[31m\u{7}\u{85}\u{9b}\u{7f}\tduty");
        row.cells.push(crate::view::CellView {
            heading: "State\u{1b}".into(),
            text: crate::view::Text::plain("on\u{1b}]52;c;x\u{7}"),
            show: None,
            target: None,
        });
    }
    let name_in = |tree: &Tree| (tree.rows[0].name.words(), tree.rows[0].cells[0].text.words(), tree.rows[0].cells[0].heading.clone());
    let want = ("Watch[31m duty".to_string(), "on]52;c;x".to_string(), "State".to_string());

    let screen = crate::view::screen_showing(&host, 160, 40).expect("the screen");
    let Node::Tree(tree) = &screen.panes[1].body else { panic!("a tree") };
    assert_eq!(name_in(tree), want, "the screen the host shows");
    let screen = host.screen_view(0, "watch").expect("the screen");
    let Node::Tree(tree) = &screen.panes[1].body else { panic!("a tree") };
    assert_eq!(name_in(tree), want, "a screen taken by key handling");
    let section = host.section_view(0, "cases").expect("the section");
    let Node::Tree(tree) = &section.body else { panic!("a tree") };
    assert_eq!(name_in(tree), want, "a settings section");
    assert_eq!(tree.rows[0].id, "watch", "ids are as the part gave them");
}
