//! Fixed owned ticket/config state; no live constructors, host, frame or provider.

use super::*;
use crate::{SiteData, test_support::TicketBuilder};
use radar_core::{Plugin, plugin::DEMO_NOW};

fn plugin(ticket: Ticket) -> JiraPlugin {
    let mut p = JiraPlugin {
        now: DEMO_NOW,
        cfg: crate::test_support::fixture_settings(),
        ..Default::default()
    };
    let key = ticket.key.clone();
    let status = ticket.status.clone();
    let jql = p.jql_for(0, &p.cfg.views[0]).unwrap();
    p.data.insert(
        "watch".into(),
        SiteData {
            tickets: [(key.clone(), ticket)].into(),
            found: [(jql, vec![key.clone()])].into(),
            me: Some(crate::model::Me {
                account_id: "vimes".into(),
                name: "Sam Vimes".into(),
            }),
            ..Default::default()
        },
    );
    p.connections.reconcile(&p.cfg.sites);
    p.tree.borrow_mut().select(vec![
        site_id("watch"),
        group_id("watch", &status),
        ticket_id("watch", &key),
    ]);
    p
}

fn cx(focus: PaneId) -> v::ViewCx<'static> {
    v::ViewCx {
        now: DEMO_NOW,
        focus,
        width: 160,
        height: 40,
        parts: Parts::none(),
        icon_lead: 0,
        setup: false,
    }
}

fn content(p: &JiraPlugin) -> DetailContent {
    DetailContent::of(&detail_pane(p, &cx(PaneId::Nav)).body).expect("selected ticket detail")
}

fn section<'a>(content: &'a DetailContent, name: &str) -> &'a [Block] {
    match content
        .sections
        .iter()
        .find(|b| matches!(b, Block::Section { title, .. } if title == name))
        .unwrap()
    {
        Block::Section { blocks, .. } => blocks,
        _ => unreachable!(),
    }
}

#[test]
fn common_structure_keeps_reporter_parent_metadata_feedback_and_preview_limits() {
    let mut ticket = TicketBuilder::new()
        .assignee("carrot", "Carrot Ironfoundersson")
        .build();
    ticket.reporter = "Sam Vimes".into();
    ticket.priority = "High".into();
    ticket.parent = Some(crate::model::Parent {
        key: "WATCH-1".into(),
        summary: "Keep the streets safe".into(),
        ..Default::default()
    });
    ticket.labels = vec!["night-watch".into()];
    ticket.components = vec!["Shades".into()];
    ticket.fix_versions = vec!["Watch 2".into()];
    ticket.description =
        radar_core::ui::rich::Doc::new(vec![radar_core::ui::rich::Block::para(vec![
            radar_core::ui::rich::Inline::text("First patrol\nSecond patrol"),
        ])]);
    ticket.ac = ticket.description.clone();
    let mut p = plugin(ticket);
    p.sent.insert(
        crate::detail_key("watch", "WATCH-12"),
        write::Sent {
            dispatch: None,
            words: "move WATCH-12".into(),
            result: Some(Err("The move could not be confirmed".into())),
        },
    );
    let detail = content(&p);
    assert_eq!(detail.header.path.words(), "watch / WATCH / WATCH-12");
    assert_eq!(detail.header.title.words(), "Patrol the Shades");
    assert_eq!(detail.header.author.as_ref().unwrap().words(), "Sam Vimes");
    assert!(detail.header.branch.is_none());
    assert!(detail.header.age.is_some());
    let value = |label| {
        detail
            .facts
            .iter()
            .find(|f| f.label == label)
            .unwrap()
            .value
            .words()
    };
    assert_eq!(value("assignee"), "Carrot Ironfoundersson");
    assert_eq!(value("priority"), "High", "priority survives icons off");
    assert_eq!(value("parent"), "WATCH-1: Keep the streets safe");
    assert_eq!(value("components"), "Shades");
    assert_eq!(value("fix versions"), "Watch 2");
    assert!(value("labels").contains("night-watch"));
    assert!(
        detail
            .sections
            .iter()
            .all(|b| matches!(b, Block::Section { .. }))
    );
    assert!(
        matches!(&section(&detail, "LATEST CHANGE")[0], Block::Line { text, .. } if text.words() == "The move could not be confirmed")
    );
    for name in ["DESCRIPTION", "ACCEPTANCE CRITERIA"] {
        assert!(
            matches!(&section(&detail, name)[0], Block::Doc { most: Some(12), doc, .. } if !doc.is_empty())
        );
    }
}

#[test]
fn missing_fields_are_omitted_and_unassigned_and_ac_guidance_remain_explicit() {
    let mut ticket = TicketBuilder::new().build();
    ticket.project.clear();
    ticket.kind.clear();
    ticket.status.clear();
    ticket.updated.clear();
    let mut p = plugin(ticket);
    let detail = content(&p);
    assert_eq!(detail.header.path.words(), "watch / WATCH-12");

    assert!(
        detail.header.author.is_none()
            && detail.header.age.is_none()
            && detail.header.branch.is_none()
    );
    assert_eq!(
        detail
            .facts
            .iter()
            .map(|f| f.label.as_str())
            .collect::<Vec<_>>(),
        ["assignee"]
    );
    assert_eq!(detail.facts[0].value.words(), "unassigned");
    assert!(
        matches!(&section(&detail, "DESCRIPTION")[0], Block::Line { text, .. } if text.words() == "no description")
    );
    assert!(
        matches!(&section(&detail, "ACCEPTANCE CRITERIA")[0], Block::Line { text, .. } if text.words() == "no field set for this project: settings, projects")
    );
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-12")
        .unwrap()
        .project = "WATCH".into();
    p.cfg.sites[0].projects[0].ac_field = "customfield_10001".into();
    assert!(
        matches!(&section(&content(&p), "ACCEPTANCE CRITERIA")[0], Block::Line { text, .. } if text.words() == "none written")
    );
}

#[test]
fn foot_uses_actual_detail_actions_and_preserves_keys_changes_and_assign_blocker() {
    let p = plugin(TicketBuilder::new().assignee("vimes", "Sam Vimes").build());
    let pane = detail_pane(&p, &cx(PaneId::Nav));
    let foot = pane.foot_actions.unwrap();
    let rows = p.screen_rows(Pane::Detail);
    assert_eq!(foot.actions.len(), 7);
    assert!(
        !p.screen_rows(Pane::Nav)
            .iter()
            .any(|(id, _)| id == "jira.tickets.comment")
    );
    for control in &foot.actions {
        let (id, binding) = rows
            .iter()
            .find(|(id, _)| *id == control.action.id)
            .unwrap();
        let canonical = v::ActionSpec::named(id.clone(), binding);
        assert_eq!(control.action.key, canonical.key);
        assert_eq!(control.action.change, canonical.change);
        assert!(control.target.is_none());
        assert_eq!(
            control.action.mnemonic_at,
            radar_core::mnemonic::at(&binding.keys, &control.action.label)
        );
        assert_eq!(
            control.action.mnemonic_shift,
            control.action.mnemonic_at.is_some() && radar_core::mnemonic::shifted(&binding.keys)
        );
    }
    let assign = foot
        .actions
        .iter()
        .find(|c| c.action.id == "jira.tickets.assign_me")
        .unwrap();
    assert_eq!(
        assign.disabled.as_deref(),
        Some("WATCH-12 is already yours")
    );
    let copy_key = foot
        .actions
        .iter()
        .find(|c| c.action.id == "jira.tickets.copy_key")
        .unwrap();
    assert!(copy_key.action.mnemonic_shift);
    assert_eq!(foot.menu.action.key.unwrap().name(), "x");
    assert!(foot.menu.target.is_none());
}

#[test]
fn identity_requires_selected_ticket_and_changes_for_provider_id_and_connection_aba() {
    let mut p = plugin(TicketBuilder::new().build());
    let identity = p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap();
    assert_eq!(identity.subject, ["watch", "WATCH-12", "10012"]);
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Middle).is_none());
    assert!(p.detail_identity("connection", Pane::Detail).is_none());
    for missing in ["", "   "] {
        p.data
            .get_mut("watch")
            .unwrap()
            .tickets
            .get_mut("WATCH-12")
            .unwrap()
            .id = missing.into();
        assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
    }
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-12")
        .unwrap()
        .id = "10013".into();
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap(),
        identity
    );
    p.data
        .get_mut("watch")
        .unwrap()
        .tickets
        .get_mut("WATCH-12")
        .unwrap()
        .id = "10012".into();
    let original = p.cfg.sites[0].url.clone();
    p.cfg.sites[0].url = "https://other.example.test".into();
    p.connections.reconcile(&p.cfg.sites);
    p.cfg.sites[0].url = original;
    p.connections.reconcile(&p.cfg.sites);
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail)
            .unwrap()
            .connection,
        identity.connection
    );
    p.tree.borrow_mut().select(vec![site_id("watch")]);
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
    assert!(detail_pane(&p, &cx(PaneId::Detail)).foot_actions.is_none());
    p.tree
        .borrow_mut()
        .select(vec![ticket_id("watch", "WATCH-12")]);
    p.pages.push(page::Page {
        site: "watch".into(),
        key: "WATCH-12".into(),
        tab: 0,
        row: 0.into(),
        extra: Default::default(),
    });
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
    p.pages.clear();
    p.cfg.sites.clear();
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
}

#[test]
fn shared_missing_id_ticket_is_still_refused_a_detail_identity() {
    let ticket = crate::model::fixtures::ticket(
        "WATCH-12",
        "Patrol the Shades",
        "To Do",
        crate::model::Category::Todo,
    );
    assert!(
        ticket.id.is_empty(),
        "the shared missing-ID fixture stays missing"
    );
    let p = plugin(ticket);
    assert!(matches!(selected(&p), Some(Selected::Ticket(_))));
    assert!(p.connections.generation("watch").is_some());
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
}
