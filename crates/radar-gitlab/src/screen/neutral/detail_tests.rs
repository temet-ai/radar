//! Fixed owned preview data: no Host, renderer, constructor clock, or provider.

use super::*;
use radar_core::{Plugin, compose::Parts, plugin::DEMO_NOW};

const GROUP: &str = "ankh-morpork";
const PROJECT: &str = "ankh-morpork/watch";

fn plugin(mr: Mr) -> GitlabPlugin {
    let p = GitlabPlugin {
        now: DEMO_NOW,
        me: "vimes".into(),
        cfg: crate::Settings {
            url: "https://gitlab.example.test".into(),
            token: "env:WATCH_DETAIL_TOKEN_NEVER_RESOLVE".into(),
            sources: vec![Source {
                group: GROUP.into(),
                code: "WATCH".into(),
                color: "cyan".into(),
                hide: vec![],
            }],
            views: vec![crate::config::View {
                name: "Watch".into(),
                filter: String::new(),
                repos: Default::default(),
            }],
            ..Default::default()
        },
        data: [(
            GROUP.into(),
            crate::GroupData {
                mrs: vec![mr],
                ..Default::default()
            },
        )]
        .into(),
        ..Default::default()
    };
    p.tree.borrow_mut().select(vec![
        tenancy_id(&p.cfg.sources[0]),
        repo_id(PROJECT),
        mr_id(PROJECT, 12),
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

fn content(p: &GitlabPlugin) -> DetailContent {
    DetailContent::of(&detail_pane(p, &cx(PaneId::Nav)).body).expect("selected MR detail")
}

fn section<'a>(content: &'a DetailContent, name: &str) -> &'a Block {
    content
        .sections
        .iter()
        .find(|b| matches!(b, Block::Section { title, .. } if title == name))
        .unwrap()
}

#[test]
fn common_structure_keeps_mr_role_blockers_pipeline_changes_and_whole_comment() {
    let mut mr = crate::test_support::MrBuilder::default().build();
    mr.rebase = true;
    mr.merge_status = "NEED_REBASE".into();
    mr.labels = vec!["night-watch".into()];
    mr.reviewers = vec!["carrot".into()];
    mr.pipeline = Some(Pipeline::Failed);
    mr.duration_secs = Some(0);
    mr.stages = vec![crate::model::Stage {
        name: "patrol".into(),
        status: "failed".into(),
    }];
    mr.last_comment = Some(crate::model::Comment {
        author: "Carrot Ironfoundersson".into(),
        body: "First watch\nSecond watch".into(),
        at: mr.updated_at.clone(),
    });
    let p = plugin(mr);
    let detail = content(&p);
    assert_eq!(
        detail.header.path.words(),
        "WATCH / ankh-morpork/watch / !12"
    );
    assert_eq!(detail.header.title.words(), "Repair the Watch bell");
    assert_eq!(detail.header.branch.unwrap().words(), "watch-bell → main");
    assert_eq!(detail.header.author.unwrap().words(), "you");
    assert!(detail.header.age.is_some());
    let detail = content(&p);
    assert!(
        detail
            .sections
            .iter()
            .all(|b| matches!(b, Block::Section { .. }))
    );
    assert_eq!(
        detail
            .facts
            .iter()
            .find(|f| f.label == "approvals")
            .unwrap()
            .value
            .words(),
        "0 required · 0 left"
    );
    assert_eq!(
        detail
            .facts
            .iter()
            .find(|f| f.label == "reviewers")
            .unwrap()
            .value
            .words(),
        "carrot"
    );
    assert_eq!(
        detail
            .facts
            .iter()
            .find(|f| f.label == "labels")
            .unwrap()
            .value
            .words(),
        "#night-watch"
    );
    let Block::Section { blocks, .. } = section(&detail, "STATE") else {
        unreachable!()
    };
    assert!(blocks.len() >= 2, "role plus blockers");
    assert!(
        blocks.iter().any(
            |b| matches!(b, Block::Line { text, .. } if text.words().contains("needs rebase"))
        )
    );
    let Block::Section {
        detail: summary,
        blocks,
        ..
    } = section(&detail, "PIPELINE")
    else {
        unreachable!()
    };
    assert_eq!(summary.as_deref(), Some("failed · 0s"));
    assert!(matches!(&blocks[0], Block::Meter { stages } if stages[0].name == "patrol"));
    let Block::Section { blocks, .. } = section(&detail, "CHANGES") else {
        unreachable!()
    };
    assert!(matches!(&blocks[0], Block::Line { text, .. } if text.words() == "0 files  +0 −0"));
    let Block::Section { blocks, .. } = section(&detail, "THREADS") else {
        unreachable!()
    };
    assert!(
        blocks.iter().any(
            |b| matches!(b, Block::Wrapped { text, .. } if text == "First watch\nSecond watch")
        )
    );
}

#[test]
fn missing_optional_identity_and_facts_are_omitted_but_zero_is_retained() {
    let mut mr = crate::test_support::MrBuilder::default().build();
    mr.author.username.clear();
    mr.author.name.clear();
    mr.updated_at.clear();
    mr.source_branch.clear();
    mr.target_branch.clear();
    mr.project_name.clear();
    let detail = content(&plugin(mr));
    assert!(
        detail.header.branch.is_none()
            && detail.header.author.is_none()
            && detail.header.age.is_none()
    );
    assert_eq!(
        detail
            .facts
            .iter()
            .map(|f| f.label.as_str())
            .collect::<Vec<_>>(),
        ["state", "approvals"]
    );
    assert!(
        detail
            .facts
            .iter()
            .all(|f| !f.value.words().trim().is_empty())
    );
    let mut missing_path = crate::test_support::MrBuilder::default().build();
    missing_path.project_path.clear();
    let p = plugin(crate::test_support::MrBuilder::default().build());
    let mut tenant = p.cfg.sources[0].clone();
    tenant.code.clear();
    let item = Item {
        mr: &missing_path,
        role: Role::Team,
        tenant: &tenant,
    };
    assert_eq!(
        mr_detail(&p, &item, &cx(PaneId::Detail))
            .header
            .path
            .words(),
        "!12"
    );
}

#[test]
fn foot_uses_detail_rows_with_truthful_inverse_and_disabled_reasons() {
    let mut mr = crate::test_support::MrBuilder::default().build();
    mr.approved_by = vec!["vimes".into()];
    let mut p = plugin(mr);
    let pane = detail_pane(&p, &cx(PaneId::Nav));
    let foot = pane.foot_actions.unwrap();
    let canonical = crate::actions(&p.rows(Pane::Detail));
    assert_eq!(foot.actions.len(), 6, "pipeline absent, page included");
    for control in &foot.actions {
        let action = canonical
            .iter()
            .find(|action| action.id == control.action.id)
            .unwrap();
        assert_eq!(control.action.key, action.key);
        assert_eq!(control.action.change, action.change);
        assert!(control.target.is_none());
        let key = action.key.unwrap().name();
        assert_eq!(
            control.action.mnemonic_at,
            radar_core::mnemonic::at(&key, &control.action.label)
        );
    }
    let approve = foot
        .actions
        .iter()
        .find(|a| a.action.id == "gitlab.repos.approve")
        .unwrap();
    assert_eq!(approve.action.label, "take back approval");
    assert!(approve.disabled.is_none());
    let update = foot
        .actions
        .iter()
        .find(|a| a.action.id == "gitlab.repos.rebase")
        .unwrap();
    assert!(update.disabled.is_some());
    p.data.get_mut(GROUP).unwrap().mrs[0].pipeline_url =
        Some("https://gitlab.example.test/ankh-morpork/watch/-/pipelines/7".into());
    assert!(
        detail_pane(&p, &cx(PaneId::Nav))
            .foot_actions
            .unwrap()
            .actions
            .iter()
            .any(|a| a.action.id == "gitlab.repos.open_pipeline")
    );
    p.acting.insert(
        crate::pages::mr_key(PROJECT, 12),
        crate::mr_actions::Act {
            what: "rebase",
            state: crate::mr_actions::Acting::Running,
            at: DEMO_NOW,
        },
    );
    assert_eq!(
        detail_pane(&p, &cx(PaneId::Nav))
            .foot_actions
            .unwrap()
            .actions[0]
            .disabled
            .as_deref(),
        Some("asked, waiting for GitLab")
    );
}

#[test]
fn identity_is_selected_mr_only_and_tracks_destinations_branches_and_connection_aba() {
    let mut p = plugin(crate::test_support::MrBuilder::default().build());
    let identity = p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap();
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Nav).is_none());
    assert!(p.detail_identity("connection", Pane::Detail).is_none());
    p.me = "carrot".into();
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap(),
        identity
    );
    p.me = "vimes".into();
    p.data.get_mut(GROUP).unwrap().mrs[0]
        .web_url
        .push_str("?view=changes");
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap(),
        identity
    );
    p.data.get_mut(GROUP).unwrap().mrs[0].web_url = identity.subject[3].clone();
    p.data.get_mut(GROUP).unwrap().mrs[0].target_branch = "trunk".into();
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail).unwrap(),
        identity
    );
    p.data.get_mut(GROUP).unwrap().mrs[0].target_branch = "main".into();
    let (url, token) = (p.cfg.url.clone(), p.cfg.token.clone());
    p.replace_connection("https://other.example.test".into(), token.clone());
    p.replace_connection(url, token);
    assert_ne!(
        p.detail_identity(crate::SCREEN.id, Pane::Detail)
            .unwrap()
            .connection,
        identity.connection
    );
    p.data.get_mut(GROUP).unwrap().mrs[0].iid = 0;
    p.tree.borrow_mut().select(vec![
        tenancy_id(&p.cfg.sources[0]),
        repo_id(PROJECT),
        mr_id(PROJECT, 0),
    ]);
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
    p.data.get_mut(GROUP).unwrap().mrs[0].iid = 12;
    p.tree
        .borrow_mut()
        .select(vec![tenancy_id(&p.cfg.sources[0])]);
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
    assert!(detail_pane(&p, &cx(PaneId::Detail)).foot_actions.is_none());
    p.tree.borrow_mut().select(vec![
        tenancy_id(&p.cfg.sources[0]),
        repo_id(PROJECT),
        mr_id(PROJECT, 12),
    ]);
    // Tenancy page creates only local page state and effect envelopes, no reads.
    drop(crate::pages::push(
        &mut p,
        crate::pages::Kind::Tenancy(GROUP.into()),
    ));
    assert!(p.detail_identity(crate::SCREEN.id, Pane::Detail).is_none());
}
