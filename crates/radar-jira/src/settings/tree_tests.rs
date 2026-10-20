//! Projects tree navigation is local state; owner actions retain their targets.
use super::*;
use radar_core::plugin::DEMO_NOW;

fn plugin() -> JiraPlugin {
    let mut cfg = crate::test_support::fixture_settings();
    cfg.sites[0].projects.push(Project {
        key: "SHADES".into(),
        ac_field: String::new(),
    });
    let mut other = cfg.sites[0].clone();
    other.name = "witches".into();
    other.url = "https://witches.invalid".into();
    other.projects = vec![Project {
        key: "LANCRE".into(),
        ac_field: String::new(),
    }];
    cfg.sites.push(other);
    JiraPlugin {
        cfg,
        now: DEMO_NOW,
        section_rows: [0, 1, 0, 1],
        env: Some(vec![]),
        ..Default::default()
    }
}

fn tree(p: &JiraPlugin) -> v::Tree {
    let v::Node::Tree(tree) = projects_view(p, 100, DEMO_NOW).body else {
        panic!("projects tree")
    };
    tree
}

fn select(p: &mut JiraPlugin, row: ProjRow) {
    let target = v::TreeRow::target_of(&project_path(p, &row));
    assert!(
        matches!(projects_intent(p, &v::Intent::Select { target }), Outcome::Handled(fx) if fx.is_empty())
    );
}

fn fold(p: &mut JiraPlugin, site: usize, open: bool) {
    let target = v::TreeRow::target_of(&project_path(p, &ProjRow::Site(site)));
    assert!(
        matches!(projects_intent(p, &v::Intent::Fold { target, open }), Outcome::Handled(fx) if fx.is_empty())
    );
}

#[test]
fn projects_headers_are_selectable_and_folds_persist_across_views() {
    let mut p = plugin();
    let initial = tree(&p);
    assert_eq!(initial.selected, ["s:watch", "add"]);
    assert!(!initial.rows[0].heading);
    select(&mut p, ProjRow::Site(0));
    assert_eq!(proj_row(&p), Some(ProjRow::Site(0)));
    fold(&mut p, 0, false);
    for _ in 0..3 {
        let drawn = tree(&p);
        assert_eq!(drawn.selected, ["s:watch"]);
        assert!(!drawn.open.contains(&vec!["s:watch".into()]));
        assert_eq!(
            drawn.visible().len(),
            5,
            "closed watch, open witches and three children"
        );
        let _ = views_view(&p, 100);
    }
    let enter = v::Intent::Activate {
        target: v::TreeRow::target_of(&["s:watch".into()]),
    };
    assert!(
        matches!(projects_intent(&mut p, &enter), Outcome::Handled(fx)
        if matches!(fx.as_slice(), [Effect::Menu(menu)] if menu.id == "psite:0"))
    );
}

#[test]
fn opening_an_earlier_branch_never_shifts_the_selected_project_or_its_actions() {
    let mut p = plugin();
    tree(&p);
    select(&mut p, ProjRow::Project(1, 0));
    let selected = tree(&p).selected;
    for open in [false, true, false, true] {
        fold(&mut p, 0, open);
        assert_eq!(tree(&p).selected, selected);
        assert_eq!(proj_row(&p), Some(ProjRow::Project(1, 0)));
        assert_eq!(row_menu(&p, "projects").unwrap().id, "project:1:0");
    }
    fold(&mut p, 1, false);
    assert_eq!(tree(&p).selected, ["s:witches"]);
    assert_eq!(proj_row(&p), Some(ProjRow::Site(1)));
}

#[test]
fn stable_project_selection_and_folds_survive_site_and_project_reordering() {
    let mut p = plugin();
    tree(&p);
    select(&mut p, ProjRow::Project(0, 1));
    fold(&mut p, 1, false);
    p.cfg.sites.swap(0, 1);
    p.cfg.sites[1].projects.swap(0, 1);
    let drawn = tree(&p);
    assert_eq!(drawn.selected, ["s:watch", "SHADES"]);
    assert!(!drawn.open.contains(&vec!["s:witches".into()]));
    assert_eq!(proj_row(&p), Some(ProjRow::Project(1, 0)));
    assert_eq!(row_menu(&p, "projects").unwrap().id, "project:1:0");
    assert_eq!(detail_title(&p, "projects"), "SHADES");
}

#[test]
fn add_reveals_its_site_and_editing_ignores_tree_selection_and_folding() {
    let mut p = plugin();
    tree(&p);
    fold(&mut p, 1, false);
    start_project_add(&mut p, 1);
    assert_eq!(tree(&p).selected, ["s:witches", "add"]);
    assert!(tree(&p).open.contains(&vec!["s:witches".into()]));
    let selected = tree(&p).selected;
    select(&mut p, ProjRow::Site(0));
    fold(&mut p, 1, false);
    let activate = v::Intent::Activate {
        target: v::TreeRow::target_of(&["s:watch".into(), "add".into()]),
    };
    assert!(matches!(projects_intent(&mut p, &activate), Outcome::Handled(fx) if fx.is_empty()));
    assert_eq!(tree(&p).selected, selected);
    assert!(matches!(p.edit, Some((Target::ProjectAdd(1), _))));
    assert!(tree(&p).open.contains(&vec!["s:witches".into()]));
    let edit = tree(&p).editing.unwrap();
    assert_eq!(edit.path, selected);
}

#[test]
fn deleted_project_recovers_on_the_same_site_and_keeps_confirmation_payloads() {
    let mut p = plugin();
    tree(&p);
    select(&mut p, ProjRow::Project(1, 0));
    let effects = on_menu(&mut p, "project:1:0", "remove");
    assert!(matches!(effects.as_slice(), [Effect::Menu(menu)] if menu.id == "project-remove:1:0"));
    assert_eq!(
        p.cfg.sites[1].projects.len(),
        1,
        "the question does not remove anything"
    );
    on_menu(&mut p, "project-remove:1:0", "yes");
    assert!(p.cfg.sites[1].projects.is_empty());
    assert_eq!(proj_row(&p), Some(ProjRow::Colour(1)));
    assert_eq!(tree(&p).selected, ["s:witches", "colour"]);
    assert_eq!(row_menu(&p, "projects").unwrap().id, "psite:1");
}

#[test]
fn missing_and_hidden_projects_cannot_activate_the_previous_row() {
    let mut p = plugin();
    tree(&p);
    select(&mut p, ProjRow::Project(0, 0));
    fold(&mut p, 1, false);
    for path in [
        vec!["0", "add"],
        vec!["s:witches", "add"],
        vec!["s:watch", "MISSING"],
    ] {
        let target = v::TreeRow::target_of(&path.into_iter().map(String::from).collect::<Vec<_>>());
        for intent in [
            v::Intent::Select {
                target: target.clone(),
            },
            v::Intent::Activate { target },
        ] {
            assert!(
                matches!(projects_intent(&mut p, &intent), Outcome::Handled(fx) if fx.is_empty())
            );
            assert_eq!(proj_row(&p), Some(ProjRow::Project(0, 0)));
            assert!(p.edit.is_none());
        }
    }
}

#[test]
fn raw_vertical_navigation_skips_closed_children_and_selects_site_rows() {
    let mut p = plugin();
    tree(&p);
    fold(&mut p, 0, false);
    select(&mut p, ProjRow::Site(0));
    key(&mut p, "projects", Chord::new(Key::Down));
    assert_eq!(proj_row(&p), Some(ProjRow::Site(1)));
    key(&mut p, "projects", Chord::new(Key::Home));
    assert_eq!(proj_row(&p), Some(ProjRow::Site(0)));
    key(&mut p, "projects", Chord::new(Key::End));
    assert_eq!(proj_row(&p), Some(ProjRow::Project(1, 0)));
}

#[test]
fn empty_projects_and_stale_fold_intents_remain_safe() {
    let mut p = plugin();
    tree(&p);
    p.cfg.sites.clear();
    let target = v::TreeRow::target_of(&["s:watch".into()]);
    projects_intent(&mut p, &v::Intent::Fold { target, open: true });
    let drawn = tree(&p);
    assert!(drawn.rows.is_empty() && drawn.selected.is_empty());
    assert!(proj_row(&p).is_none());
}

#[test]
fn stale_and_hidden_context_never_open_another_project_menu_through_the_host() {
    use radar_core::{
        Host, Pane,
        host::{Event, View},
    };
    let mut p = plugin();
    tree(&p);
    select(&mut p, ProjRow::Project(0, 0));
    fold(&mut p, 1, false);
    let current = tree(&p).selected;
    let hidden = project_path(&p, &ProjRow::Add(1));
    let mut host = Host::new(vec![Box::new(p)], DEMO_NOW).unwrap();
    host.view = View::Settings;
    host.section = host
        .sections
        .iter()
        .position(|(_, section)| section.id == "projects")
        .unwrap();
    host.focus = Pane::Middle;
    host.set_view_room(100, 40);
    for path in [vec!["s:watch".into(), "MISSING".into()], hidden] {
        host.update(Event::Intent(v::Intent::Context {
            target: v::TreeRow::target_of(&path),
        }));
        assert!(
            host.menu.is_none(),
            "invalid context must not open the current project's menu"
        );
        let v::Node::Tree(tree) = host.section_view(0, "projects").unwrap().body else {
            panic!("projects tree")
        };
        assert_eq!(tree.selected, current);
    }
    host.update(Event::Intent(v::Intent::Context {
        target: v::TreeRow::target_of(&current),
    }));
    assert!(
        host.menu.is_some(),
        "valid context keeps its original action"
    );
}
