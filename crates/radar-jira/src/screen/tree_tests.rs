//! Pure loaded-model tests: no full frames, host, network, or invented ancestry.
use super::*;
use crate::{SiteData, model::Parent, test_support::TicketBuilder};
use radar_core::plugin::DEMO_NOW;

fn ticket(key: &str, parent: Option<&str>, status: &str, category: Category) -> Ticket {
    let mut ticket = TicketBuilder::new()
        .key(key)
        .status(status, category)
        .build();
    ticket.parent = parent.map(|key| Parent {
        key: key.into(),
        ..Default::default()
    });
    ticket
}

fn plugin(tickets: Vec<Ticket>) -> JiraPlugin {
    let mut p = JiraPlugin {
        now: DEMO_NOW,
        cfg: crate::test_support::fixture_settings(),
        ..Default::default()
    };
    let jql = p.jql_for(0, &p.cfg.views[0]).unwrap();
    p.data.insert(
        "watch".into(),
        SiteData {
            found: [(
                jql,
                tickets.iter().map(|ticket| ticket.key.clone()).collect(),
            )]
            .into(),
            tickets: tickets
                .into_iter()
                .map(|ticket| (ticket.key.clone(), ticket))
                .collect(),
            ..Default::default()
        },
    );
    p
}

fn ancestry() -> JiraPlugin {
    plugin(vec![
        ticket("WATCH-4", Some("WATCH-3"), "In Review", Category::Doing),
        ticket("WATCH-2", Some("WATCH-1"), "To Do", Category::Todo),
        ticket("WATCH-1", None, "In Progress", Category::Doing),
        ticket("WATCH-3", Some("WATCH-2"), "Done", Category::Done),
    ])
}

fn ticket_path(p: &JiraPlugin, key: &str) -> Vec<String> {
    targets(p)
        .into_iter()
        .find(|(path, _, _)| path.last() == Some(&ticket_id("watch", key)))
        .unwrap()
        .0
}

fn refresh(p: &JiraPlugin) {
    open_new_nodes(p, &nodes(p));
}

#[test]
fn full_ancestry_keeps_three_generations_and_cross_status_children_together() {
    let p = ancestry();
    let sites = nodes(&p);
    assert_eq!(sites.len(), 1);
    let groups = &sites[0].groups;
    assert_eq!(groups.len(), 1, "only the root's status makes a group");
    assert_eq!(groups[0].status, "In Progress");
    assert_eq!(groups[0].len(), 4);
    assert_eq!(
        groups[0]
            .tickets()
            .map(|t| t.key.as_str())
            .collect::<Vec<_>>(),
        ["WATCH-1", "WATCH-2", "WATCH-3", "WATCH-4"]
    );
    assert_eq!(
        ticket_path(&p, "WATCH-4"),
        [
            "s:watch",
            "g:watch:In Progress",
            "t:watch:WATCH-1",
            "t:watch:WATCH-2",
            "t:watch:WATCH-3",
            "t:watch:WATCH-4"
        ]
    );
    let rows = tree_rows(&p, &sites, DEMO_NOW, &Parts::none());
    let tree = v::Tree {
        rows,
        ..Default::default()
    };
    let leaf = tree.row(&ticket_path(&p, "WATCH-4")).unwrap();
    assert!(
        leaf.cells
            .iter()
            .find(|cell| cell.heading == "status")
            .unwrap()
            .text
            .words()
            .contains("In Review")
    );
    assert!(
        group_detail(&p, 0, "In Progress")
            .iter()
            .any(|line| line.words().contains("WATCH-4"))
    );
}

#[test]
fn missing_or_filtered_parent_is_a_root_without_fetching_or_synthesizing_rows() {
    let mut p = ancestry();
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-4")
        .unwrap()
        .summary = "missing dragon".into();
    p.filter = "missing dragon".into();
    assert_eq!(targets(&p).len(), 1);
    assert_eq!(
        ticket_path(&p, "WATCH-4"),
        ["s:watch", "g:watch:In Review", "t:watch:WATCH-4"]
    );
    let missing = ticket("WATCH-5", Some("WATCH-404"), "To Do", Category::Todo);
    let groups = forest(vec![&missing]);
    assert_eq!(groups[0].tops[0].ticket.key, "WATCH-5");
    assert!(groups[0].tops[0].children.is_empty());
    assert!(p.sync.in_flight.is_empty());
    assert!(p.details.is_empty());
}

#[test]
fn duplicate_query_keys_have_one_row_and_every_cycle_has_a_deterministic_root() {
    let tickets = [
        ticket("WATCH-2", Some("WATCH-3"), "To Do", Category::Todo),
        ticket("WATCH-3", Some("WATCH-1"), "Done", Category::Done),
        ticket("WATCH-1", Some("WATCH-2"), "In Progress", Category::Doing),
        ticket("WATCH-4", Some("WATCH-4"), "To Do", Category::Todo),
        ticket("WATCH-5", Some("WATCH-2"), "To Do", Category::Todo),
    ];
    for input in [
        vec![
            &tickets[0],
            &tickets[1],
            &tickets[2],
            &tickets[3],
            &tickets[4],
            &tickets[0],
        ],
        vec![
            &tickets[4],
            &tickets[3],
            &tickets[2],
            &tickets[1],
            &tickets[0],
        ],
    ] {
        let groups = forest(input);
        assert_eq!(groups.iter().map(GroupNode::len).sum::<usize>(), 5);
        let roots: Vec<_> = groups
            .iter()
            .flat_map(|group| group.tops.iter())
            .map(|node| node.ticket.key.as_str())
            .collect();
        assert_eq!(roots, ["WATCH-1", "WATCH-4"]);
        let keys: HashSet<_> = groups
            .iter()
            .flat_map(GroupNode::tickets)
            .map(|t| &t.key)
            .collect();
        assert_eq!(keys.len(), 5);
    }
    let mut p = plugin(tickets.to_vec());
    let jql = p.jql_for(0, &p.cfg.views[0]).unwrap();
    p.data
        .get_mut("watch")
        .unwrap()
        .found
        .get_mut(&jql)
        .unwrap()
        .extend(["WATCH-1".into(), "WATCH-5".into()]);
    assert_eq!(targets(&p).len(), 5);
}

#[test]
fn every_small_parent_graph_keeps_each_ticket_exactly_once() {
    // Four tickets each choose no parent, any held ticket, or a missing parent.
    // This covers self-cycles, disjoint cycles, tails into cycles, and orphans.
    for mut code in 0..6_usize.pow(4) {
        let tickets: Vec<_> = (1..=4)
            .map(|i| {
                let parent = code % 6;
                code /= 6;
                let key = format!("WATCH-{parent}");
                ticket(
                    &format!("WATCH-{i}"),
                    (parent != 0).then_some(key.as_str()),
                    "To Do",
                    Category::Todo,
                )
            })
            .collect();
        let groups = forest(tickets.iter().collect());
        let keys: Vec<_> = groups
            .iter()
            .flat_map(GroupNode::tickets)
            .map(|t| t.key.as_str())
            .collect();
        assert_eq!(keys.len(), 4);
        assert_eq!(keys.into_iter().collect::<HashSet<_>>().len(), 4);
    }
}

#[test]
fn selection_and_folds_follow_ticket_identity_after_status_ancestry_and_order_changes() {
    let mut p = ancestry();
    refresh(&p);
    let old_child = ticket_path(&p, "WATCH-2");
    intent(
        &mut p,
        Pane::Middle,
        &Intent::Fold {
            target: TreeRow::target_of(&old_child),
            open: false,
        },
    );
    let old_leaf = ticket_path(&p, "WATCH-4");
    // A selected descendant is deliberately made visible before the refresh.
    p.tree.borrow_mut().open(old_child);
    p.tree.borrow_mut().select(old_leaf.clone());
    let parent = p
        .data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-1")
        .unwrap();
    parent.status = "Done".into();
    parent.category = Category::Done;
    let jql = p.jql_for(0, &p.cfg.views[0]).unwrap();
    p.data
        .get_mut("watch")
        .unwrap()
        .found
        .get_mut(&jql)
        .unwrap()
        .reverse();
    refresh(&p);
    let path = ticket_path(&p, "WATCH-4");
    assert_eq!(p.tree.borrow().selected(), path);
    assert_ne!(path, old_leaf);
    assert!((1..path.len()).all(|depth| p.tree.borrow().is_open(&path[..depth])));
    assert!(matches!(selected(&p), Some(Selected::Ticket(item)) if item.t.key == "WATCH-4"));
    // Detaching a selected ticket must move the path, never the action subject.
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-4")
        .unwrap()
        .parent = None;
    refresh(&p);
    assert_eq!(p.tree.borrow().selected(), ticket_path(&p, "WATCH-4"));
    assert!(matches!(selected(&p), Some(Selected::Ticket(item)) if item.t.key == "WATCH-4"));
    // A closed branch retains that choice when its root's status changes again.
    let child = ticket_path(&p, "WATCH-2");
    intent(
        &mut p,
        Pane::Middle,
        &Intent::Fold {
            target: TreeRow::target_of(&child),
            open: false,
        },
    );
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-1")
        .unwrap()
        .status = "Released".into();
    refresh(&p);
    assert!(!p.tree.borrow().is_open(&ticket_path(&p, "WATCH-2")));
}

#[test]
fn bulk_open_reaches_grandparents_and_closing_a_branch_selects_its_ancestor() {
    let mut p = ancestry();
    refresh(&p);
    on_bulk(&mut p, "j-close").unwrap();
    on_bulk(&mut p, "j-open").unwrap();
    let leaf = ticket_path(&p, "WATCH-4");
    assert!((1..leaf.len()).all(|depth| p.tree.borrow().is_open(&leaf[..depth])));
    intent(
        &mut p,
        Pane::Middle,
        &Intent::Select {
            target: TreeRow::target_of(&leaf),
        },
    );
    let child = ticket_path(&p, "WATCH-2");
    intent(
        &mut p,
        Pane::Middle,
        &Intent::Fold {
            target: TreeRow::target_of(&child),
            open: false,
        },
    );
    refresh(&p);
    assert_eq!(p.tree.borrow().selected(), child);
    assert!(!p.tree.borrow().is_open(&child));
}

#[test]
fn stale_or_hidden_tree_targets_cannot_move_or_activate_another_ticket() {
    let mut p = ancestry();
    refresh(&p);
    let old = ticket_path(&p, "WATCH-4");
    intent(
        &mut p,
        Pane::Middle,
        &Intent::Select {
            target: TreeRow::target_of(&old),
        },
    );
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-4")
        .unwrap()
        .parent = None;
    for stale in [
        Intent::Select {
            target: TreeRow::target_of(&old),
        },
        Intent::Activate {
            target: TreeRow::target_of(&old),
        },
        Intent::Fold {
            target: TreeRow::target_of(&old),
            open: true,
        },
    ] {
        assert!(
            matches!(intent(&mut p, Pane::Middle, &stale), Outcome::Handled(fx) if fx.is_empty())
        );
        assert_eq!(p.tree.borrow().selected(), ticket_path(&p, "WATCH-4"));
        assert!(p.pages.is_empty());
    }
    let child = ticket_path(&p, "WATCH-2");
    intent(
        &mut p,
        Pane::Middle,
        &Intent::Fold {
            target: TreeRow::target_of(&child[..2]),
            open: false,
        },
    );
    assert!(
        matches!(intent(&mut p, Pane::Middle, &Intent::Activate { target: TreeRow::target_of(&child) }),
        Outcome::Handled(fx) if fx.is_empty())
    );
    assert!(p.pages.is_empty());
}

#[test]
fn removing_a_selected_ticket_recovers_to_its_held_ancestor_without_retargeting_a_peer() {
    let mut p = ancestry();
    refresh(&p);
    p.tree.borrow_mut().select(ticket_path(&p, "WATCH-4"));
    p.data.get_mut("watch").unwrap().tickets.remove("WATCH-4");
    refresh(&p);
    assert_eq!(p.tree.borrow().selected(), ticket_path(&p, "WATCH-3"));
}

#[test]
fn fold_hints_follow_repaired_children_and_never_alias_another_site() {
    let mut p = plugin(vec![
        ticket("WATCH-1", Some("WATCH-2"), "To Do", Category::Todo),
        ticket("WATCH-2", Some("WATCH-1"), "To Do", Category::Todo),
        ticket("WATCH-3", Some("WATCH-3"), "To Do", Category::Todo),
        ticket("WATCH-4", None, "To Do", Category::Todo),
    ]);
    let mut other = p.cfg.sites[0].clone();
    other.name = "witches".into();
    other.url = "https://witches.invalid".into();
    p.cfg.sites.push(other);
    p.cfg.views[0].site.clear();
    let child = ticket("WATCH-5", Some("WATCH-4"), "To Do", Category::Todo);
    let jql = p.jql_for(1, &p.cfg.views[0]).unwrap();
    p.data.insert(
        "witches".into(),
        SiteData {
            found: [(jql, vec![child.key.clone()])].into(),
            tickets: [(child.key.clone(), child)].into(),
            ..Default::default()
        },
    );
    refresh(&p);
    for (key, expected) in [
        ("WATCH-1", true),
        ("WATCH-2", false),
        ("WATCH-3", false),
        ("WATCH-4", false),
    ] {
        p.tree.borrow_mut().select(ticket_path(&p, key));
        let fold = keys(&p, Pane::Middle)
            .iter()
            .any(|binding| binding.keys == "space");
        assert_eq!(
            fold, expected,
            "{key}: only held children in the repaired site tree are foldable"
        );
    }
}

#[test]
fn filter_enter_after_rendering_chooses_a_result_when_the_original_ticket_disappears() {
    let mut p = plugin(vec![
        ticket("WATCH-4", None, "To Do", Category::Todo),
        ticket("WATCH-5", None, "To Do", Category::Todo),
    ]);
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-5")
        .unwrap()
        .summary = "Find the dragon".into();
    refresh(&p);
    p.tree.borrow_mut().select(ticket_path(&p, "WATCH-4"));
    key(&mut p, Pane::Middle, Chord::new(Key::Char('/')));
    for ch in "dragon".chars() {
        key(&mut p, Pane::Middle, Chord::new(Key::Char(ch)));
        // This is the render-time state reconciliation, without drawing a frame.
        refresh(&p);
    }
    assert!(
        !matches!(selected(&p), Some(Selected::Ticket(_))),
        "rendering recovered to an ancestor"
    );
    key(&mut p, Pane::Middle, Chord::new(Key::Enter));
    assert_eq!(p.tree.borrow().selected(), ticket_path(&p, "WATCH-5"));
    assert!(p.filter_before.is_none() && p.filter_input.is_none());
}

#[test]
fn cancelling_a_rendered_filter_restores_the_original_ticket_identity() {
    let mut p = ancestry();
    refresh(&p);
    let before = ticket_path(&p, "WATCH-4");
    p.tree.borrow_mut().select(before.clone());
    key(&mut p, Pane::Middle, Chord::new(Key::Char('/')));
    key(&mut p, Pane::Middle, Chord::new(Key::Char('z')));
    refresh(&p);
    assert!(p.tree.borrow().selected().is_empty());
    key(&mut p, Pane::Middle, Chord::new(Key::Esc));
    assert_eq!(p.tree.borrow().selected(), before);
    assert!(p.filter_before.is_none() && p.filter_input.is_none() && p.filter.is_empty());
}

#[test]
fn stale_and_hidden_context_never_open_the_current_ticket_menu_through_the_host() {
    use radar_core::{
        Host,
        host::{Event, View},
    };
    let mut p = ancestry();
    refresh(&p);
    let stale = ticket_path(&p, "WATCH-4");
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-4")
        .unwrap()
        .parent = None;
    refresh(&p);
    let current = ticket_path(&p, "WATCH-4");
    p.tree.borrow_mut().select(current.clone());
    let hidden = ticket_path(&p, "WATCH-2");
    intent(
        &mut p,
        Pane::Middle,
        &Intent::Fold {
            target: TreeRow::target_of(&hidden[..2]),
            open: false,
        },
    );
    let mut host = Host::new(vec![Box::new(p)], DEMO_NOW).unwrap();
    host.view = View::Screen(0);
    host.focus = Pane::Middle;
    host.set_view_room(100, 40);
    for path in [stale, hidden] {
        host.update(Event::Intent(Intent::Context {
            target: TreeRow::target_of(&path),
        }));
        assert!(
            host.menu.is_none(),
            "invalid context must not open another ticket's menu"
        );
        let view = host.screen_view(0, crate::SCREEN.id).unwrap();
        let Node::Tree(tree) = &view
            .panes
            .iter()
            .find(|pane| pane.pane == PaneId::Middle)
            .unwrap()
            .body
        else {
            panic!("tree")
        };
        assert_eq!(tree.selected, current);
    }
    host.update(Event::Intent(Intent::Context {
        target: TreeRow::target_of(&current),
    }));
    assert!(
        host.menu.is_some(),
        "valid context keeps its original action"
    );
}
