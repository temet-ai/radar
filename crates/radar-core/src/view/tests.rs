use super::*;

#[test]
fn the_contract_names_no_terminal_library() {
    for (file, source) in [
        ("view.rs", include_str!("../view.rs")),
        ("view/chrome.rs", include_str!("chrome.rs")),
    ] {
        for word in ["ratatui", "crossterm", "Buffer", "Rect", "KeyEvent"] {
            let used = source
                .lines()
                .filter(|l| !l.trim_start().starts_with("//"))
                .any(|l| l.contains(word));
            assert!(!used, "{file} uses {word}");
        }
    }
}

#[test]
fn the_plugin_trait_names_no_terminal_type() {
    let source = include_str!("../plugin.rs");
    let start = source.find("pub trait Plugin").expect("the trait");
    let end = start + source[start..].find("\n}\n").expect("its end");
    let body = &source[start..end];
    for word in [
        "ratatui", "crossterm", "Buffer", "Rect", "KeyEvent", "MouseEvent", "Frame", "Span", "Line<",
        "Style",
    ] {
        let used = body
            .lines()
            .filter(|l| !l.trim_start().starts_with("//"))
            .any(|l| l.contains(word));
        assert!(!used, "the Plugin trait names {word}");
    }
}

#[test]
fn a_screen_survives_the_wire_with_its_version_targets_and_icons() {
    let tree = Tree {
        header: Text::of("clones", Ink::Muted),
        columns: vec![ColumnSpec::new("ahead", 7, 5).icon(Icon::Commit)],
        rows: vec![
            TreeRow::new(
                "c:/a",
                Text::new()
                    .push(Seg::icon(Icon::Repo))
                    .push(Seg::new("a", Ink::Primary).strong()),
            )
            .cell("ahead", Text::of("↑1", Ink::Number))
            .child(TreeRow::new("b:main", "main")),
        ],
        selected: vec!["c:/a".into()],
        open: vec![vec!["c:/a".into()]],
        name_width: 30,
        ..Tree::default()
    };
    let mut view = ScreenView::new(
        "branches",
        vec![PaneView::new(PaneId::Middle, "all", Node::Tree(tree))],
    );
    view.actions
        .push(ActionSpec::named(
            "git.branches.delete",
            &crate::Binding::new("d", "delete the branch…"),
        ));
    let wire = serde_json::to_string(&view).unwrap();
    assert!(wire.contains("\"version\":1"), "{wire}");
    assert!(
        wire.contains("\"icon\":\"repo\""),
        "icons go by name: {wire}"
    );
    let back: ScreenView = serde_json::from_str(&wire).unwrap();
    assert_eq!(back, view);
    assert_eq!(back.actions[0].id, "git.branches.delete");
    assert_eq!(back.actions[0].key, Some(Chord::char('d')));
}

#[test]
fn an_unknown_icon_name_reads_as_none_not_a_failure() {
    let seg: Seg = serde_json::from_str(r#"{"text":"x","icon":"no-such-icon"}"#).unwrap();
    assert_eq!(seg.icon, None);
    assert_eq!(seg.text, "x");
}

#[test]
fn chords_read_back_their_names() {
    for name in ["d", "⏎", "space", "ctrl-r", "pgdn", "shift-tab", "↑"] {
        assert_eq!(Chord::parse(name).unwrap().name(), name);
    }
    assert_eq!(Chord::parse("is:review"), None);
}

#[test]
fn list_movement_skips_headings() {
    let rows = vec![
        ListRow::heading("group", "GROUP"),
        ListRow::new("first", "first"),
        ListRow::heading("next-group", "NEXT"),
        ListRow::new("second", "second"),
    ];
    assert_eq!(rows_step(&rows, 1, Chord::new(Key::Down)), Some(3));
    assert_eq!(rows_step(&rows, 3, Chord::new(Key::Up)), Some(1));
    assert_eq!(rows_step(&rows, 1, Chord::new(Key::Up)), None);
    assert_eq!(rows_step(&rows, 1, Chord::new(Key::End)), Some(3));
}

fn sample() -> Tree {
    Tree {
        rows: vec![
            TreeRow::new("a", "a")
                .child(TreeRow::new("a1", "a1"))
                .child(TreeRow::new("a2", "a2")),
            TreeRow::new("b", "b"),
        ],
        open: vec![vec!["a".into()]],
        selected: vec!["a".into()],
        ..Tree::default()
    }
}

#[test]
fn movement_keys_select_the_next_visible_row_and_fold_groups() {
    let t = sample();
    let path = |i: &Intent| match i {
        Intent::Select { target } | Intent::Fold { target, .. } => TreeRow::path_of(target),
        _ => vec![],
    };
    let down = tree_step(&t, Chord::char('j')).unwrap();
    assert_eq!(path(&down), ["a", "a1"]);
    assert_eq!(
        tree_step(&t, Chord::new(Key::Up)),
        None,
        "the first row's ↑ is the host's"
    );
    assert_eq!(path(&tree_step(&t, Chord::char('G')).unwrap()), ["b"]);
    let close = tree_step(&t, Chord::char('-')).unwrap();
    assert!(matches!(close, Intent::Fold { open: false, .. }));
    let mut child = t.clone();
    child.selected = vec!["a".into(), "a2".into()];
    assert_eq!(
        path(&tree_step(&child, Chord::char('-')).unwrap()),
        ["a"],
        "a child closes its group"
    );
    assert_eq!(tree_step(&t, Chord::char('x')), None);
}

#[test]
fn tree_movement_skips_group_headings() {
    let tree = Tree {
        rows: vec![TreeRow::heading("site", "site").children([
            TreeRow::new("add", "add"),
            TreeRow::new("project", "project"),
        ])],
        open: vec![vec!["site".into()]],
        selected: vec!["site".into(), "add".into()],
        ..Tree::default()
    };
    assert_eq!(tree_step(&tree, Chord::new(Key::Up)), None);
    assert_eq!(
        tree_step(&tree, Chord::new(Key::Down)),
        Some(Intent::Select {
            target: TreeRow::target_of(&["site".into(), "project".into()]),
        })
    );
}

#[test]
fn tree_nav_opens_new_groups_once_and_keeps_a_selection_that_is_there() {
    let t = sample();
    let mut nav = TreeNav::default();
    nav.sync(&t.rows);
    assert_eq!(nav.open, vec![vec!["a".to_string()]]);
    assert_eq!(nav.selected, ["a"]);
    assert!(nav.apply(&Intent::Select {
        target: TreeRow::target_of(&["a".into(), "a2".into()])
    }));
    nav.apply(&Intent::Fold {
        target: Target::new("a"),
        open: false,
    });
    assert_eq!(nav.selected, ["a"], "closing the group selects it");
    nav.sync(&t.rows);
    assert!(nav.open.is_empty(), "a group the user closed stays closed");
}
