use super::*;

mod mnemonic_tests {
use super::*;

#[test]
fn mnemonic_fields_default_on_legacy_data_and_fresh_producers_keep_real_keys() {
    let action = ActionSpec::named("watch.copy", &crate::Binding::new("Y", "copy item key"));
    assert_eq!(action.mnemonic_at, Some(3));
    assert!(action.mnemonic_shift);
    assert_eq!(action.key, Some(Chord::char('Y')));
    let mut wire = serde_json::to_value(&action).unwrap();
    assert_eq!(wire["mnemonic_at"], 3);
    assert_eq!(wire["mnemonic_shift"], true);
    wire.as_object_mut().unwrap().remove("mnemonic_at");
    wire.as_object_mut().unwrap().remove("mnemonic_shift");
    let old: ActionSpec = serde_json::from_value(wire).unwrap();
    assert_eq!(old.mnemonic_at, None);
    assert!(!old.mnemonic_shift);
    let text = Text::new().push(Seg::icon_or(Icon::Link, "open ", Ink::Muted)).push(Seg::new("refresh", Ink::Plain));
    let block = Block::keyed("watch.refresh", text, "r");
    assert!(matches!(block, Block::Line { mnemonic_at: Some(5), mnemonic_shift: false, .. }));
    assert!(matches!(Block::keyed("watch.open", Text::from(vec![Seg::icon_or(Icon::Link, "open", Ink::Muted)]), "o"), Block::Line { mnemonic_at: None, .. }));
    let plain = Text::plain("ordinary text");
    assert_eq!(plain.words(), plain.plain_words());
    assert!(serde_json::to_value(&plain).unwrap()[0].get("key_hint").is_none());
    let old: Seg = serde_json::from_str(r#"{"text":"old words"}"#).unwrap();
    assert_eq!(old.key_hint, None);
    let keyed = crate::mnemonic::text("A", "ask an agent", "text.secondary");
    assert_eq!(keyed.words(), "shift ask an agent");
    assert_eq!(keyed.plain_words(), "shift ask an agent [A]");
    assert_eq!(serde_json::from_str::<Text>(&serde_json::to_string(&keyed).unwrap()).unwrap(), keyed);
}

}

/// Every `.rs` file and `Cargo.toml` under `dir`, skipping build output.
fn sources_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name != "target" && name != "node_modules" {
                sources_under(&path, out);
            }
        } else if path.extension().is_some_and(|e| e == "rs") || name == "Cargo.toml" {
            out.push(path);
        }
    }
}

/// Plan decision 6: the host and the parts hold radar's own colour, key and
/// text types and no terminal library's. The words are written in pieces so
/// this file does not name them itself.
#[test]
fn the_host_and_every_part_name_no_terminal_library() {
    let libraries = [
        ["rata", "tui"].concat(),
        ["cross", "term"].concat(),
        ["tui", "-tree-widget"].concat(),
        ["tui", "_tree_widget"].concat(),
    ];
    let types = [["Buf", "fer"].concat(), ["Re", "ct"].concat(), ["Key", "Event"].concat()];
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("crates/");
    let mut files = Vec::new();
    for member in std::fs::read_dir(crates).expect("the workspace's crates").flatten() {
        sources_under(&member.path(), &mut files);
    }
    assert!(files.len() > 100, "the walk found {} files", files.len());
    let mut found = Vec::new();
    for file in &files {
        let Ok(source) = std::fs::read_to_string(file) else { continue };
        let shown = file.strip_prefix(crates).unwrap_or(file).display().to_string();
        for (n, line) in source.lines().enumerate() {
            let lower = line.to_lowercase();
            if let Some(word) = libraries.iter().find(|w| lower.contains(w.as_str())) {
                found.push(format!("{shown}:{} names {word}", n + 1));
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            let mut tokens = line.split(|c: char| !(c.is_alphanumeric() || c == '_'));
            if let Some(word) = tokens.find(|t| types.iter().any(|w| w == t)) {
                found.push(format!("{shown}:{} uses {word}", n + 1));
            }
        }
    }
    assert!(found.is_empty(), "terminal types in the host or a part:\n{}", found.join("\n"));
}

#[test]
fn the_plugin_trait_names_no_terminal_type() {
    let source = include_str!("../plugin.rs");
    let start = source.find("pub trait Plugin").expect("the trait");
    let end = start + source[start..].find("\n}\n").expect("its end");
    let body = &source[start..end];
    for word in ["MouseEvent", "Frame", "Span", "Line<", "Style"] {
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

mod tab_attention {
    use super::*;

    #[test]
    fn attention_is_explicit_and_defaults_off_on_the_wire() {
        let tab = TabView::new("Notes", Some(Icon::Thread));
        let wire = serde_json::to_value(&tab).unwrap();
        assert!(wire.get("attention").is_none());
        assert!(!serde_json::from_value::<TabView>(wire).unwrap().attention);
        let tab = tab.attention(true);
        let wire = serde_json::to_value(&tab).unwrap();
        assert_eq!(wire["attention"], true);
        assert_eq!(wire["label"], "Notes");
        assert_eq!(serde_json::from_value::<TabView>(wire).unwrap(), tab);
        let old: crate::view::chrome::BarTab = serde_json::from_str(
            r#"{"label":"Watch","mnemonic":"W","target":"chrome:screen:0"}"#,
        ).unwrap();
        assert!(!old.attention);
    }
}

#[test]
fn shared_tree_horizontal_navigation_and_open_all_have_distinct_meanings() {
    let mut tree = Tree {
        rows: vec![TreeRow::new("watch", "Watch").child(
            TreeRow::heading("cases", "Cases").child(TreeRow::new("WATCH-12", "Case twelve"))
        )],
        selected: vec!["watch".into()],
        open: vec![vec!["watch".into(), "cases".into()]],
        ..Tree::default()
    };
    let fold = |ids: &[&str], open| Intent::Fold {
        target: TreeRow::target_of(&ids.iter().map(|id| (*id).to_string()).collect::<Vec<_>>()), open,
    };
    assert_eq!(tree_step(&tree, Chord::char('=')), Some(Intent::OpenAll));
    assert_eq!(tree_step(&tree, Chord::char('+')), Some(fold(&["watch"], true)));
    assert_eq!(tree_step(&tree, Chord::new(Key::Right)), Some(fold(&["watch"], true)));
    tree.open.push(vec!["watch".into()]);
    let child = vec!["watch".into(), "cases".into(), "WATCH-12".into()];
    assert_eq!(tree_step(&tree, Chord::new(Key::Right)), Some(Intent::Select { target: TreeRow::target_of(&child) }));
    tree.selected = child;
    assert_eq!(tree_step(&tree, Chord::new(Key::Left)), Some(Intent::Select { target: Target::new("watch") }));
    assert_eq!(tree_step(&tree, Chord::char('-')), Some(fold(&["watch"], false)));
    assert_eq!(tree_step(&tree, Chord::new(Key::Right)), None, "leaf boundary is owned but does not mutate");
    assert!(tree_key(Chord::new(Key::Right)));
}

#[test]
fn tree_stale_selection_does_not_redirect_a_fold_and_empty_open_all_is_explicit() {
    let mut tree = sample();
    tree.selected = vec!["missing".into()];
    for key in [Key::Left, Key::Right, Key::Char(' '), Key::Char('+'), Key::Char('-')] {
        assert_eq!(tree_step(&tree, Chord::new(key)), None, "{key:?}");
    }
    assert_eq!(tree_step(&tree, Chord::new(Key::Down)), Some(Intent::Select { target: Target::new("a") }));
    assert_eq!(tree_step(&Tree::default(), Chord::char('=')), Some(Intent::OpenAll));
    assert_eq!(serde_json::to_value(Intent::OpenAll).unwrap(), serde_json::json!({"intent": "open_all"}));
}

#[test]
fn held_groups_are_parent_first_and_ambiguous_paths_are_never_addressable() {
    let mut tree = Tree {
        rows: vec![TreeRow::new("watch", "Watch").child(
            TreeRow::new("cases", "Cases").child(TreeRow::new("WATCH-12", "Case twelve"))
        )],
        ..Tree::default()
    };
    assert_eq!(tree.groups(), vec![vec!["watch".to_string()], vec!["watch".to_string(), "cases".to_string()]]);
    assert!(!tree.selectable(&["watch".into(), "cases".into()]));
    tree.rows.push(tree.rows[0].clone());
    assert!(tree.row(&["watch".into()]).is_none());
    assert!(tree.groups().is_empty());
}

#[test]
fn tree_nav_sync_prunes_removed_groups_and_never_selects_headings() {
    let mut nav = TreeNav::default();
    let rows = vec![TreeRow::heading("watch", "Watch").child(TreeRow::new("vimes", "Sam Vimes"))];
    nav.sync(&rows);
    assert_eq!(nav.selected, ["watch", "vimes"]);
    nav.sync(&[TreeRow::heading("empty", "Empty")]);
    assert!(nav.selected.is_empty());
    assert!(nav.open.is_empty());
}

#[test]
fn shifted_navigation_is_additive_on_the_wire_and_not_tree_owned() {
    let old: Chord = serde_json::from_str(r#"{"key":"right"}"#).unwrap();
    assert_eq!(old, Chord::new(Key::Right));
    assert_eq!(serde_json::to_value(old).unwrap(), serde_json::json!({"key": "right"}));
    let shifted = Chord { shift: true, ..old };
    assert!(!tree_key(shifted));
    assert_eq!(tree_step(&sample(), shifted), None);
    assert_eq!(serde_json::to_value(shifted).unwrap(), serde_json::json!({"key": "right", "shift": true}));
}
