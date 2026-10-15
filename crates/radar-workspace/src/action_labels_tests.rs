//! Cached action wording allows for unavailable folders; launch tasks are never polled.

use super::*;
use crate::demo::KEY;
use crate::find::fixture::Scratch;

fn assert_terminal(p: &mut WorkspacePlugin, expected_label: &str, expected_dir: &Path) {
    let ticket = Subject::new("ticket", KEY, KEY).with("project", "ABC");
    let Some(Contribution::Actions(actions)) = p.contribute(Point::Actions, &ticket) else {
        panic!("ticket actions");
    };
    let terminal = actions
        .iter()
        .find(|action| action.id == "terminal")
        .unwrap();
    assert_eq!(terminal.what, expected_label);
    assert_eq!(terminal.key, None, "the terminal stays a menu-only action");
    assert_eq!(Path::new(&p.open_dir(&p.cfg.spaces[0], KEY)), expected_dir);

    let effects = p.act(&ticket, "terminal");
    let [Effect::Journal(action), Effect::Spawn(_)] = effects.as_slice() else {
        panic!("an unpolled terminal launch: {effects:?}");
    };
    assert_eq!(action.kind.as_str(), "workspace.terminal.launch");
    let destination = action
        .subjects
        .iter()
        .find(|subject| subject.role == SubjectRole::Destination)
        .unwrap();
    assert_eq!(Path::new(&destination.key.id), expected_dir);
}

#[test]
fn contextual_terminal_label_covers_notes_folder_and_unavailable_fallbacks_from_cache() {
    let scratch = Scratch::new("terminal-action-label");
    let folder = "notes/ABC-12-stats";
    scratch.file(&format!("{folder}/README.md"), "# ABC-12");
    let top = scratch.0.clone();
    let mut p = WorkspacePlugin::new();
    let mut space = Space::new(top.display().to_string());
    space.name = "acme".into();
    space.tickets = vec!["ABC".into()];
    p.state.indexes.insert(
        space.dir.clone(),
        notes::Index {
            notes: vec![notes::Note {
                key: KEY.into(),
                folder: folder.into(),
                readme: true,
                ..notes::Note::default()
            }],
            ..notes::Index::default()
        },
    );
    p.cfg.spaces.push(space);

    assert_terminal(&mut p, "open a terminal at the top of acme", &top);
    p.cfg.spaces[0].open_at = "repo".into();
    assert_terminal(
        &mut p,
        "open a terminal for ABC-12 in acme",
        &top.join(folder),
    );

    let index = p.state.indexes.get_mut(&p.cfg.spaces[0].dir).unwrap();
    index.notes[0].folder = "notes/missing".into();
    assert_terminal(&mut p, "open a terminal for ABC-12 in acme", &top);
    p.state.indexes.clear();
    assert_terminal(&mut p, "open a terminal at the top of acme", &top);
}
