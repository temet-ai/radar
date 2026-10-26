//! Configured local workspaces and the repositories found inside them.

use radar_core::plugin::{Binding, Outcome, Pane, ScreenSpec};
use radar_core::view::{self as v, Ink, Node, PaneId, PaneView, Text, TreeRow};

use crate::{Target, WorkspacePlugin};

pub const SCREEN: ScreenSpec = ScreenSpec {
    id: "local-repositories",
    name: "local workspaces",
    key: 'W',
};

#[derive(Clone)]
struct Row {
    space: usize,
    repo: Option<usize>,
    path: Vec<String>,
    label: String,
    ambiguous: bool,
}

// Folder identities, not row positions or display names, survive index refreshes.
fn id(kind: &str, dir: &str) -> String {
    format!("{kind}:{}", dir.replace('%', "%25").replace(v::PATH_SEP, "%1F"))
}

fn rows(p: &WorkspacePlugin) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut space_counts = std::collections::BTreeMap::new();
    for source in &p.cfg.spaces {
        *space_counts.entry(source.dir.as_str()).or_insert(0usize) += 1;
    }
    for (space, source) in p.cfg.spaces.iter().enumerate() {
        let Some(count) = space_counts.remove(source.dir.as_str()) else { continue; };
        let path = vec![id("workspace", &source.dir)];
        let ambiguous = count > 1;
        rows.push(Row {
            space, repo: None, path: path.clone(),
            label: if ambiguous { source.dir.clone() } else { source.label() }, ambiguous,
        });
        // Configuration loading permits duplicates. Never choose one entry's
        // editor/settings on behalf of another with the same directory identity.
        if ambiguous { continue; }
        if let Some(index) = p.state.indexes.get(&source.dir) {
            let mut repository_counts = std::collections::BTreeMap::new();
            for found in &index.repositories {
                *repository_counts.entry(found.dir.as_str()).or_insert(0usize) += 1;
            }
            for (repo, found) in index.repositories.iter().enumerate() {
                let Some(count) = repository_counts.remove(found.dir.as_str()) else { continue; };
                rows.push(Row {
                    space,
                    repo: Some(repo),
                    path: vec![path[0].clone(), id("repository", &found.dir)],
                    label: found.dir.rsplit('/').next().unwrap_or(&found.dir).into(),
                    ambiguous: count > 1,
                });
            }
        }
    }
    rows
}

fn visible(p: &WorkspacePlugin, row: &Row) -> bool {
    row.repo.is_none() || !p.state.screen_folded.contains(&row.path[0])
}

fn selected<'a>(p: &WorkspacePlugin, rows: &'a [Row]) -> Option<&'a Row> {
    let mut path = p.state.screen_selected.clone();
    while !path.is_empty() {
        if let Some(row) = rows.iter().find(|row| row.path == path) {
            if row.ambiguous { return None; }
            if visible(p, row) { return Some(row); }
        }
        path.pop();
    }
    rows.iter().find(|row| !row.ambiguous && visible(p, row))
}

fn tree(p: &WorkspacePlugin) -> v::Tree {
    let items = rows(p);
    let selected = selected(p, &items).map(|row| row.path.clone()).unwrap_or_default();
    let mut open = Vec::new();
    let mut roots: Vec<TreeRow> = Vec::new();
    for row in items {
        let mut node = TreeRow::new(row.path.last().unwrap(), Text::of(row.label, if row.repo.is_none() { Ink::Emphasis } else { Ink::Secondary }));
        if row.ambiguous {
            node.heading = true;
            node.detail = Some(Text::of(if row.repo.is_none() {
                "duplicate folder in Settings; cannot open"
            } else {
                "duplicate folder in held index; cannot open"
            }, Ink::Warn));
        }
        if row.repo.is_none() {
            if !row.ambiguous && !p.state.screen_folded.contains(&row.path[0]) {
                open.push(row.path);
            }
            roots.push(node);
        } else if let Some(parent) = roots.last_mut() {
            parent.children.push(node);
        }
    }
    v::Tree { rows: roots, selected, open, ..v::Tree::default() }
}

pub fn view(p: &WorkspacePlugin, cx: &v::ViewCx<'_>) -> v::ScreenView {
    let list = rows(p);
    let detail = selected(p, &list).map_or_else(
        || vec![Text::of(if list.iter().any(|row| row.ambiguous) {
            "Duplicate folder identities cannot be opened. Resolve duplicate workspace entries in Settings or refresh the repository index."
        } else {
            "Add a local workspace in Settings."
        }, Ink::Secondary)],
        |row| {
            let space = &p.cfg.spaces[row.space];
            if let Some(repo) = row.repo.and_then(|i| p.state.indexes.get(&space.dir)?.repositories.get(i)) {
                vec![Text::plain(&repo.project), Text::plain(&repo.dir), Text::new(), Text::of("Enter opens this folder in your editor", Ink::Muted)]
            } else {
                vec![Text::plain(&space.dir), Text::new(), Text::of("Repositories are read from this workspace's clones folder.", Ink::Muted), Text::of("Enter opens the workspace in your editor", Ink::Muted)]
            }
        });
    let mut screen = v::ScreenView::new(SCREEN.id, vec![
        PaneView::new(PaneId::Middle, "local repositories", Node::Tree(tree(p))),
        PaneView::new(PaneId::Detail, "details", Node::Document { lines: detail, cut: false }),
    ]);
    screen.actions = keys(p, Pane::from(cx.focus)).iter().map(|b| v::ActionSpec::named(action_id(b), b)).collect();
    screen
}

/// Stable action ids preserve the screen's existing editor and refresh actions.
fn action_id(b: &Binding) -> &'static str {
    if b.is_heading() { return ""; }
    match b.keys.as_str() {
        "⏎" => "workspace.local_repositories.open",
        "r" => "workspace.local_repositories.look_again",
        other => unreachable!("local repositories has no stable id for key {other:?}"),
    }
}

fn select(p: &mut WorkspacePlugin, target: &v::Target) -> Outcome {
    let path = TreeRow::path_of(target);
    if !rows(p).iter().any(|row| row.path == path && !row.ambiguous && visible(p, row)) {
        return Outcome::Ignored;
    }
    p.state.screen_selected = path;
    Outcome::done()
}

fn fold(p: &mut WorkspacePlugin, target: &v::Target, open: bool) -> Outcome {
    let path = TreeRow::path_of(target);
    let model = tree(p);
    if !model.visible().iter().any(|(shown, row)| *shown == path && !row.children.is_empty()) {
        return Outcome::Ignored;
    }
    if open {
        p.state.screen_folded.remove(&path[0]);
    } else {
        p.state.screen_folded.insert(path[0].clone());
        if model.selected.starts_with(&path) {
            p.state.screen_selected = path;
        }
    }
    Outcome::done()
}

pub fn intent(p: &mut WorkspacePlugin, pane: Pane, intent: &v::Intent) -> Outcome {
    if pane != Pane::Middle { return Outcome::Ignored; }
    match intent {
        v::Intent::Select { target } | v::Intent::Context { target } | v::Intent::Activate { target } => {
            let out = select(p, target);
            if matches!(intent, v::Intent::Activate { .. }) && matches!(out, Outcome::Handled(_)) {
                return key(p, v::Chord::new(v::Key::Enter));
            }
            out
        }
        v::Intent::Fold { target, open } => fold(p, target, *open),
        v::Intent::Key { chord } => key(p, *chord),
        v::Intent::Invoke { action } => {
            let chord = keys(p, pane).iter().find(|binding| action_id(binding) == action).and_then(|binding| v::Chord::parse(&binding.keys));
            chord.map_or(Outcome::Ignored, |chord| key(p, chord))
        }
        _ => Outcome::Ignored,
    }
}

fn key(p: &mut WorkspacePlugin, chord: v::Chord) -> Outcome {
    if chord.ctrl || chord.alt { return Outcome::Ignored; }
    if let Some(step) = v::tree_step(&tree(p), chord) {
        return intent(p, Pane::Middle, &step);
    }
    match chord.key {
        v::Key::Enter => {
            let list = rows(p);
            let Some(row) = selected(p, &list) else { return Outcome::done(); };
            let space = &p.cfg.spaces[row.space];
            let dir = row.repo
                .and_then(|i| p.state.indexes.get(&space.dir)?.repositories.get(i))
                .map_or_else(|| space.dir.clone(), |repo| repo.dir.clone());
            Outcome::Handled(p.open_editor(space, Target::Folder(dir)))
        }
        v::Key::Char('r') => {
            let mut effects = Vec::new();
            for dir in p.cfg.spaces.iter().map(|s| s.dir.clone()).collect::<Vec<_>>() {
                effects.extend(p.reindex(&dir, true));
            }
            Outcome::Handled(effects)
        }
        _ => Outcome::Ignored,
    }
}

pub fn keys(p: &WorkspacePlugin, pane: Pane) -> Vec<Binding> {
    if pane == Pane::Middle {
        let mut bindings = vec![Binding::heading("local repositories")];
        if selected(p, &rows(p)).is_some() {
            bindings.push(Binding::new("⏎", "open the selected folder"));
        }
        bindings.push(Binding::new("r", "look for repositories again").pane());
        bindings
    } else {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Space, find::Repo, notes::Index};

    fn fixture() -> WorkspacePlugin {
        let mut p = WorkspacePlugin::new();
        for (dir, names) in [("/work/ankh-morpork", vec!["watch", "guilds"]), ("/work/lancre", vec!["witches"])] {
            p.cfg.spaces.push(Space::new(dir));
            p.state.indexes.insert(dir.into(), Index {
                repositories: names.into_iter().map(|name| Repo { dir: format!("{dir}/{name}"), project: name.into() }).collect(),
                ..Index::default()
            });
        }
        p
    }

    #[test]
    fn paths_selection_and_folds_survive_workspace_and_repository_reordering() {
        let mut p = fixture();
        let path = tree(&p).visible()[2].0.clone();
        let target = TreeRow::target_of(&path);
        assert!(matches!(select(&mut p, &target), Outcome::Handled(_)));
        p.cfg.spaces.reverse();
        p.state.indexes.get_mut("/work/ankh-morpork").unwrap().repositories.reverse();
        assert_eq!(tree(&p).selected, path);
        let step = v::tree_step(&tree(&p), v::Chord::char('-')).unwrap();
        assert!(matches!(intent(&mut p, Pane::Middle, &step), Outcome::Handled(_)));
        assert_eq!(tree(&p).selected, path[..1]);
        assert_eq!(tree(&p).visible().len(), 3);
        assert!(matches!(select(&mut p, &target), Outcome::Ignored));
        let parent = TreeRow::target_of(&path[..1]);
        assert!(matches!(fold(&mut p, &parent, true), Outcome::Handled(_)));
        assert!(matches!(select(&mut p, &target), Outcome::Handled(_)));
        p.state.indexes.get_mut("/work/ankh-morpork").unwrap().repositories.retain(|r| r.project != "guilds");
        assert!(matches!(select(&mut p, &target), Outcome::Ignored));
        assert_eq!(tree(&p).selected, path[..1]);
    }

    #[test]
    fn stale_wrong_ancestor_and_leaf_targets_are_ignored() {
        let mut p = fixture();
        let path = tree(&p).visible()[1].0.clone();
        let leaf = TreeRow::target_of(&path);
        assert!(matches!(fold(&mut p, &leaf, false), Outcome::Ignored));
        let wrong = TreeRow::target_of(&[id("workspace", "/work/lancre"), path[1].clone()]);
        assert!(matches!(select(&mut p, &wrong), Outcome::Ignored));
        p.cfg.spaces.clear();
        assert!(tree(&p).rows.is_empty());
        assert!(tree(&p).selected.is_empty());
        assert!(matches!(select(&mut p, &leaf), Outcome::Ignored));
        assert!(matches!(key(&mut p, v::Chord::new(v::Key::Enter)), Outcome::Handled(effects) if effects.is_empty()));
    }

    #[test]
    fn open_preserves_another_selection_and_identity_escapes_path_delimiters() {
        let mut p = fixture();
        let paths: Vec<_> = tree(&p).visible().into_iter().map(|(path, _)| path).collect();
        select(&mut p, &TreeRow::target_of(&paths[1]));
        let other = TreeRow::target_of(&paths[3]);
        fold(&mut p, &other, false);
        fold(&mut p, &other, true);
        assert_eq!(tree(&p).selected, paths[1]);
        assert_ne!(id("workspace", "/work/%1F"), id("workspace", "/work/\u{1f}"));
        let path = vec![id("workspace", "/work/\u{1f}%")];
        assert_eq!(TreeRow::path_of(&TreeRow::target_of(&path)), path);
    }
    #[test]
    fn activation_and_named_open_action_keep_the_exact_folder_target() {
        use radar_core::plugin::Effect;
        let mut p = fixture();
        let path = tree(&p).visible()[1].0.clone();
        let target = TreeRow::target_of(&path);
        p.cfg.spaces.reverse();
        let Outcome::Handled(effects) = intent(&mut p, Pane::Middle, &v::Intent::Activate { target: target.clone() }) else {
            panic!("open folder effects")
        };
        let Some(Effect::Journal(action)) = effects.first() else { panic!("folder launch descriptor") };
        assert_eq!(action.subjects[0].key.id, "/work/ankh-morpork");
        assert_eq!(action.subjects[1].key.id, "/work/ankh-morpork/watch");
        assert!(matches!(effects.get(1), Some(Effect::Spawn(_))));
        let Outcome::Handled(effects) = intent(&mut p, Pane::Middle, &v::Intent::Invoke { action: "workspace.local_repositories.open".into() }) else {
            panic!("named open action")
        };
        assert!(matches!(effects.first(), Some(Effect::Journal(action)) if action.subjects[1].key.id == "/work/ankh-morpork/watch"));
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Invoke { action: "workspace.local_repositories.unknown".into() }), Outcome::Ignored));
        p.state.indexes.get_mut("/work/ankh-morpork").unwrap().repositories.clear();
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Activate { target }), Outcome::Ignored));
    }

    #[test]
    fn configured_duplicate_workspaces_fail_closed_before_and_after_reordering() {
        use radar_core::Plugin;
        let mut p = WorkspacePlugin::new();
        let config: toml::Value = toml::from_str(r#"
            [[space]]
            dir = "/work/ankh-morpork"
            name = "Watch"
            editor = "watch-editor"
            repos = "watch-clones"
            tickets = ["WATCH"]
            [[space]]
            dir = "/work/ankh-morpork"
            name = "Guilds"
            editor = "guild-editor"
            repos = "guild-clones"
            tickets = ["GUILD"]
        "#).unwrap();
        p.configure(Some(&config)).unwrap();
        p.state.indexes.insert("/work/ankh-morpork".into(), Index {
            repositories: vec![Repo { dir: "/work/ankh-morpork/watch".into(), project: "watch".into() }],
            ..Index::default()
        });
        let root = vec![id("workspace", "/work/ankh-morpork")];
        let child = vec![root[0].clone(), id("repository", "/work/ankh-morpork/watch")];
        p.state.screen_selected = child.clone();
        let before = tree(&p);
        for _ in 0..2 {
            let model = tree(&p);
            assert_eq!(model, before);
            assert_eq!(model.rows.len(), 1);
            assert!(model.rows[0].heading && model.rows[0].children.is_empty());
            assert!(model.selected.is_empty());
            for path in [&root, &child] {
                let target = TreeRow::target_of(path);
                for event in [
                    v::Intent::Select { target: target.clone() },
                    v::Intent::Context { target: target.clone() },
                    v::Intent::Activate { target: target.clone() },
                    v::Intent::Fold { target, open: true },
                ] {
                    assert!(matches!(intent(&mut p, Pane::Middle, &event), Outcome::Ignored));
                }
            }
            assert!(matches!(key(&mut p, v::Chord::new(v::Key::Enter)), Outcome::Handled(effects) if effects.is_empty()));
            assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Invoke { action: "workspace.local_repositories.open".into() }), Outcome::Ignored));
            p.cfg.spaces.reverse();
        }
        assert_eq!(p.cfg.spaces.len(), 2);
        assert_eq!(p.cfg.spaces[0].editor, "watch-editor");
        assert_eq!(p.cfg.spaces[1].editor, "guild-editor");
        assert_eq!(p.cfg.spaces[0].repos, "watch-clones");
        assert_eq!(p.cfg.spaces[1].repos, "guild-clones");
    }

    #[test]
    fn restored_duplicate_repository_identities_never_launch_or_open_context() {
        use radar_core::Plugin;
        let mut p = fixture();
        let root = id("workspace", "/work/ankh-morpork");
        let child = vec![root, id("repository", "/work/ankh-morpork/watch")];
        p.state.screen_selected = child.clone();
        let index = Index {
            repositories: vec![
                Repo { dir: "/work/ankh-morpork/watch".into(), project: "watch".into() },
                Repo { dir: "/work/ankh-morpork/watch".into(), project: "guilds".into() },
            ], ..Index::default()
        };
        p.restore(serde_json::json!({ "indexes": { "/work/ankh-morpork": index } }));
        for _ in 0..2 {
            let model = tree(&p);
            assert!(model.selected.is_empty());
            assert_eq!(model.rows[0].children.len(), 1);
            assert!(model.rows[0].children[0].heading);
            let target = TreeRow::target_of(&child);
            assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Context { target: target.clone() }), Outcome::Ignored));
            assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Activate { target }), Outcome::Ignored));
            assert!(matches!(key(&mut p, v::Chord::new(v::Key::Enter)), Outcome::Handled(effects) if effects.is_empty()));
            p.state.indexes.get_mut("/work/ankh-morpork").unwrap().repositories.reverse();
        }
    }

    #[test]
    fn hidden_and_removed_context_targets_do_not_open_a_fallback_menu() {
        let mut p = fixture();
        let child = tree(&p).visible()[1].0.clone();
        let target = TreeRow::target_of(&child);
        fold(&mut p, &TreeRow::target_of(&child[..1]), false);
        let selected = tree(&p).selected;
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Context { target: target.clone() }), Outcome::Ignored));
        assert_eq!(tree(&p).selected, selected);
        p.cfg.spaces.clear();
        assert!(matches!(intent(&mut p, Pane::Middle, &v::Intent::Context { target }), Outcome::Ignored));
    }

}
