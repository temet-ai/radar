//! Recorded responses only; production dispatch futures are dropped unpolled.

use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, parse_mr};
use crate::{GlMsg, demo, page_history, page_ownership};
use radar_core::Plugin;
use radar_core::journal::{ActionDescriptor, Outcome};
use radar_core::plugin::Effect;
use serde_json::{Value, json};

const PROJECT: &str = "acme/team-a/api";
const IID: u64 = 263;
const REJECTION: &str = "gitlab.page.detail.application.identity.rejected";

fn node() -> Value {
    json!({"iid":"263", "title":"SECRET_RETURNED_TITLE", "description":"SECRET_BODY",
        "project":{"fullPath":PROJECT},
        "diffRefs":{"headSha":"SECRET_CHANGED_HEAD", "baseSha":"SECRET_BASE", "startSha":"SECRET_START"},
        "diffStats":[{"path":"SECRET_RETURNED_FILE"}],
        "commits":{"nodes":[]}, "discussions":{"nodes":[]},
        "jobs":{"stages":{"nodes":[]}},
        "headPipeline":{"status":"FAILED", "path":"/acme/team-a/api/-/pipelines/902"}})
}

fn recorded(n: Value, project: &str, iid: u64) -> MrDetailRead {
    let replay = Replay::new(vec![(200, json!({"data":{"project":{"mergeRequest":n}}}))]);
    let got = block(Client::over(replay.clone()).mr_detail_observed(project, iid)).unwrap();
    assert_eq!(replay.asked().len(), 1);
    assert_eq!(
        replay.asked()[0].body.as_ref().unwrap()["variables"],
        json!({"project":project,"iid":iid.to_string()})
    );
    got
}

#[derive(Clone)]
struct Pending {
    key: String,
    owner: page_ownership::Read,
    requested: Requested,
    action: ActionDescriptor,
}

fn launch(p: &mut GitlabPlugin, project: &str, iid: u64) -> Pending {
    let fx = pages::push(
        p,
        pages::Kind::Mr {
            project: project.into(),
            iid,
        },
    );
    let action = fx
        .into_iter()
        .find_map(|fx| match fx {
            Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.detail" => {
                Some(action)
            }
            _ => None,
        })
        .expect("detail dispatched without running production transport");
    let key = pages::mr_key(project, iid);
    Pending {
        owner: p.mr_page_reads[&key],
        key,
        requested: Requested::new(project, iid),
        action,
    }
}

fn receive(p: &mut GitlabPlugin, pending: &Pending, n: Value) -> Vec<Effect> {
    p.on_msg(Box::new(GlMsg::MrPage {
        key: pending.key.clone(),
        read: pending.owner,
        requested: pending.requested.clone(),
        action: pending.action.clone(),
        result: Ok(Box::new(recorded(
            n,
            &pending.requested.project,
            pending.requested.iid,
        ))),
    }))
}

fn seeded() -> (GitlabPlugin, Pending) {
    let mut p = demo::sample();
    let pending = launch(&mut p, PROJECT, IID);
    let mut held = node();
    held["title"] = json!("SECRET_HELD_TITLE");
    held["diffRefs"]["headSha"] = json!("SECRET_HELD_HEAD");
    held["headPipeline"] = Value::Null;
    let got = recorded(held, PROJECT, IID);
    pages::on_mr_page(&mut p, pending.key.clone(), Ok((got.mr, got.detail)));
    let fetched = p.mr_pages.get_mut(&pending.key).unwrap();
    fetched.at = p.now - 1; // A forced fresh-value refresh must keep this timestamp.
    fetched.loading = true;
    fetched.error = Some("SECRET_OLD_ERROR".into());
    let diff_owner = page_ownership::next(&mut p);
    p.diffs.insert(
        pending.key.clone(),
        pages::changes::Diffs {
            head: "SECRET_HELD_HEAD".into(),
            requests: [(2, diff_owner)].into(),
            error: Some("SECRET_DIFF_ERROR".into()),
            ..Default::default()
        },
    );
    p.failed
        .insert("SECRET_DIAGNOSIS".into(), Default::default());
    p.logs.insert(crate::log_ownership::JobKey::new(PROJECT, 9), Default::default());
    p.log_reads.entry(crate::log_ownership::JobKey::new(PROJECT, 9)).or_default();
    p.runs.insert(PROJECT.into(), Default::default());
    crate::write::start(&mut p, PROJECT, IID, crate::write::Target::Note);
    p.paste("SECRET_AUTHORED_TEXT");
    p.recovery.test_keep("SECRET_KEPT_TEXT");
    let page = p.pages.last_mut().unwrap();
    page.tab = 1; // Ordinary Err application would continue Changes.
    page.row.set(3);
    page.file.set(0);
    page.diff.set(2);
    page.placed.set(false);
    page.tree_shown.set(true);
    page.split_shown.set(true);
    page.detail_painted.set(true);
    page.diff_painted.set(true);
    page.extra
        .borrow_mut()
        .push(("SECRET_EXTRA".into(), vec![]));
    page.links
        .borrow_mut()
        .push(radar_core::Subject::link("https://example.com/SECRET_LINK"));
    page.under.borrow_mut().push((
        "local",
        radar_core::Subject::new("repo", "SECRET_REPO", "SECRET_LABEL"),
        "SECRET_ACTION".into(),
    ));
    page.seen.borrow_mut().insert("SECRET_SEEN".into());
    page.flipped.borrow_mut().insert("SECRET_THREAD".into());
    *page.jump.borrow_mut() = Some("SECRET_JUMP".into());
    (p, pending)
}

fn assert_evidence(fx: &[Effect], scope: &ActionDescriptor, expected: Outcome, kind: &str) {
    assert!(matches!(fx.first(), Some(Effect::Report(got)) if *got == expected));
    assert_eq!(
        fx.iter().filter(|e| matches!(e, Effect::Report(_))).count(),
        1
    );
    let Some(Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }) = fx.last()
    else {
        panic!("one application observation after original read evidence")
    };
    assert_eq!(action.kind.as_str(), kind);
    assert_eq!(action.subjects, scope.subjects);
    assert_eq!(action.metadata.count, Some(1));
    assert_eq!(action.metadata.redacted, scope.metadata.redacted);
    assert_eq!(
        action.metadata.omitted_subjects,
        scope.metadata.omitted_subjects
    );
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::Observe { action, .. } if action.kind.as_str() == kind))
            .count(),
        1
    );
    assert!(
        fx.iter()
            .all(|fx| matches!(fx, Effect::Report(_) | Effect::Observe { .. }))
    );
    assert!(!format!("{fx:?}").contains("SECRET"));
}

fn rejected_unchanged(p: &mut GitlabPlugin, pending: &Pending, n: Value, outcome: Outcome) {
    // Construct the complete expected state by applying only the three allowed
    // changes, then restore the pending state before delivering the reply.
    let f = p.mr_pages.get_mut(&pending.key).unwrap();
    let (loading, error) = (f.loading, f.error.clone());
    f.loading = false;
    f.error = Some(REJECTED.into());
    let owner = p.mr_page_reads.remove(&pending.key).unwrap();
    let expected = format!("{p:?}");
    p.mr_page_reads.insert(pending.key.clone(), owner);
    let f = p.mr_pages.get_mut(&pending.key).unwrap();
    f.loading = loading;
    f.error = error;
    let fx = receive(p, pending, n);
    assert_eq!(format!("{p:?}"), expected);
    assert_evidence(&fx, &pending.action, outcome, REJECTION);
}

#[test]
fn recorded_project_iid_and_combined_mismatches_preserve_all_owned_state() {
    for (project, iid) in [
        ("acme/team-a/other", IID),
        (PROJECT, IID + 1),
        ("acme/team-b/ui", IID + 1),
    ] {
        for partial in [false, true] {
            let (mut p, pending) = seeded();
            let mut n = node();
            n["project"]["fullPath"] = json!(project);
            n["iid"] = json!(iid);
            if partial {
                n["commits"] = Value::Null;
            }
            rejected_unchanged(
                &mut p,
                &pending,
                n,
                if partial {
                    Outcome::Partial
                } else {
                    Outcome::Succeeded
                },
            );
        }
    }
}

#[test]
fn raw_overflow_cannot_impersonate_the_legacy_narrowed_iid() {
    let overflow = (1_u64 << 32) + IID;
    for raw in [json!(overflow), json!(overflow.to_string())] {
        let mut n = node();
        n["iid"] = raw;
        let got = recorded(n.clone(), PROJECT, IID);
        let plain = block(
            Client::over(Replay::new(vec![(
                200,
                json!({"data":{"project":{"mergeRequest":n.clone()}}}),
            )]))
            .mr_detail(PROJECT, IID),
        )
        .unwrap();
        assert_eq!((&got.mr, &got.detail), (&plain.0, &plain.1));
        assert_eq!(
            got.mr.iid, IID,
            "public parser intentionally retains narrowing"
        );
        assert_eq!(got.mr, parse_mr(&n, "https://gitlab.example.com"));
        assert!(!got.matches_iid(IID));
        assert!(!got.matches_iid(overflow));
        let (mut p, pending) = seeded();
        rejected_unchanged(&mut p, &pending, n, Outcome::Succeeded);
    }
    let mut p = demo::sample();
    let pending = launch(&mut p, PROJECT, overflow);
    let mut n = node();
    n["iid"] = json!(overflow);
    rejected_unchanged(&mut p, &pending, n, Outcome::Succeeded);
}

#[test]
fn absent_zero_malformed_and_unrepresentable_raw_iids_are_rejected() {
    for raw in [
        None,
        Some(Value::Null),
        Some(json!(0)),
        Some(json!("0")),
        Some(json!(-1)),
        Some(json!("-1")),
        Some(json!(false)),
        Some(json!([])),
        Some(json!({})),
        Some(json!(1.5)),
        Some(json!("263.0")),
        Some(json!(" 263")),
        Some(json!("SECRET_BAD_IID")),
        Some(json!("18446744073709551616")),
    ] {
        let (mut p, pending) = seeded();
        let mut n = node();
        match raw {
            Some(raw) => n["iid"] = raw,
            None => {
                n.as_object_mut().unwrap().remove("iid");
            }
        }
        rejected_unchanged(&mut p, &pending, n, Outcome::Succeeded);
    }
}

#[test]
fn missing_malformed_and_unusable_returned_paths_are_rejected_without_repairs() {
    for path in [
        None,
        Some(Value::Null),
        Some(json!(42)),
        Some(json!([])),
        Some(json!({})),
        Some(json!("")),
        Some(json!(" ")),
        Some(json!(" acme/team-a/api")),
        Some(json!("acme/team-a/api/")),
        Some(json!("acme//api")),
        Some(json!("acme/./api")),
        Some(json!("acme/../api")),
        Some(json!("acme/%61pi")),
        Some(json!("acme/äpi")),
        Some(json!("https://example.com/acme/team-a/api")),
        Some(json!("acme/api?SECRET")),
    ] {
        let (mut p, pending) = seeded();
        let mut n = node();
        match path {
            Some(path) => n["project"]["fullPath"] = path,
            None => {
                n["project"].as_object_mut().unwrap().remove("fullPath");
            }
        }
        rejected_unchanged(&mut p, &pending, n, Outcome::Succeeded);
    }
    for project in [
        None,
        Some(Value::Null),
        Some(json!(false)),
        Some(json!("SECRET_PROJECT")),
    ] {
        let (mut p, pending) = seeded();
        let mut n = node();
        match project {
            Some(project) => n["project"] = project,
            None => {
                n.as_object_mut().unwrap().remove("project");
            }
        }
        rejected_unchanged(&mut p, &pending, n, Outcome::Succeeded);
    }
    for n in [json!(false), json!(42), json!([]), json!("SECRET_NODE")] {
        let (mut p, pending) = seeded();
        rejected_unchanged(&mut p, &pending, n, Outcome::Partial);
    }
}

#[test]
fn requested_target_must_be_usable_and_name_the_exact_owned_key() {
    for (project, iid) in [
        ("", IID),
        ("acme/../api", IID),
        ("acme/./api", IID),
        ("acme//api", IID),
        (" acme/team-a/api", IID),
        (PROJECT, 0),
    ] {
        let mut p = demo::sample();
        let pending = launch(&mut p, project, iid);
        rejected_unchanged(&mut p, &pending, node(), Outcome::Succeeded);
    }
    let (mut p, mut pending) = seeded();
    pending.requested = Requested::new("acme/team-a/other", IID);
    let mut n = node();
    n["project"]["fullPath"] = json!("acme/team-a/other");
    rejected_unchanged(&mut p, &pending, n, Outcome::Succeeded);
}

#[test]
fn matching_case_numeric_and_string_iids_preserve_legacy_application_parity() {
    for project in [PROJECT, "ACME/Team-A/API"] {
        for iid in [json!(IID), json!(IID.to_string())] {
            for partial in [false, true] {
                let (mut p, pending) = seeded();
                let (mut legacy, _) = seeded();
                let mut n = node();
                n["project"]["fullPath"] = json!(project);
                n["iid"] = iid.clone();
                if partial {
                    n["commits"] = Value::Null;
                }
                let got = recorded(n.clone(), PROJECT, IID);
                let parsed = (got.mr.clone(), got.detail.clone());
                legacy.mr_page_reads.remove(&pending.key);
                let expected =
                    pages::on_mr_page(&mut legacy, pending.key.clone(), Ok(parsed.clone()));
                let actual = receive(&mut p, &pending, n);
                let application = actual
                    .iter()
                    .filter(|fx| !matches!(fx, Effect::Report(_) | Effect::Observe { .. }))
                    .collect::<Vec<_>>();
                assert_eq!(format!("{application:?}"), format!("{expected:?}"));
                assert_eq!(format!("{p:?}"), format!("{legacy:?}"));
                assert_eq!(p.mr_pages[&pending.key].value.as_ref(), Some(&parsed));
                assert_eq!(
                    p.mr_pages[&pending.key]
                        .value
                        .as_ref()
                        .unwrap()
                        .0
                        .project_path,
                    project
                );
                assert!(!actual.iter().any(|fx| matches!(fx, Effect::Observe { action, .. } if action.kind.as_str() == REJECTION)));
            }
        }
    }
}

#[test]
fn journal_only_url_omission_does_not_reject_a_usable_matching_business_target() {
    for url in [
        "not a URL",
        "file:///SECRET_LOCAL",
        "ssh://git.example/acme",
    ] {
        for matches in [false, true] {
            let mut p = demo::sample();
            p.cfg.url = url.into();
            let pending = launch(&mut p, PROJECT, IID);
            assert!(pending.action.subjects.is_empty() && pending.action.metadata.redacted);
            assert_eq!(pending.action.metadata.omitted_subjects, 1);
            let mut n = node();
            if !matches {
                n["iid"] = json!(IID + 1);
            }
            let fx = receive(&mut p, &pending, n);
            assert!(!p.mr_pages[&pending.key].loading);
            assert_eq!(p.mr_pages[&pending.key].value.is_some(), matches);
            if !matches {
                assert_evidence(&fx, &pending.action, Outcome::Succeeded, REJECTION);
            }
            for fx in &fx {
                if let Effect::Observe { action, .. } = fx {
                    assert!(action.subjects.is_empty() && action.metadata.redacted);
                    assert_eq!(action.metadata.omitted_subjects, 1);
                }
            }
            assert!(!format!("{fx:?}").contains("SECRET"));
        }
    }
}

#[test]
fn supported_path_grammar_is_not_coupled_to_journal_identity_bounds() {
    for project in [
        "api".to_string(),
        "acme/API.v2_tools-repo".into(),
        format!("acme/{}", "a".repeat(2048)),
    ] {
        let mut n = node();
        n["project"]["fullPath"] = json!(project);
        let got = recorded(n, &project, IID);
        assert!(Requested::new(&project, IID).accepts(&pages::mr_key(&project, IID), &got));
    }
}

#[test]
fn identity_rejection_consumes_once_and_stale_mismatch_cannot_clear_a_new_owner() {
    let (mut p, old) = seeded();
    let newer = page_ownership::next(&mut p);
    p.mr_page_reads.insert(old.key.clone(), newer);
    let before = format!("{p:?}");
    let mut n = node();
    n["iid"] = json!(IID + 1);
    let fx = receive(&mut p, &old, n.clone());
    assert_eq!(format!("{p:?}"), before);
    assert_evidence(
        &fx,
        &old.action,
        Outcome::Succeeded,
        "gitlab.page.detail.application.skipped",
    );
    let mut current = old.clone();
    current.owner = newer;
    rejected_unchanged(&mut p, &current, n.clone(), Outcome::Succeeded);
    let before = format!("{p:?}");
    let fx = receive(&mut p, &current, n);
    assert_eq!(format!("{p:?}"), before);
    assert_evidence(
        &fx,
        &current.action,
        Outcome::Succeeded,
        "gitlab.page.detail.application.skipped",
    );
    assert!(!p.mr_page_reads.contains_key(&old.key));
}

#[test]
fn explicit_refresh_after_rejection_can_replace_a_still_fresh_held_value() {
    let (mut p, pending) = seeded();
    let mut n = node();
    n["iid"] = json!(IID + 1);
    rejected_unchanged(&mut p, &pending, n, Outcome::Succeeded);
    p.compose = None; // The editor's existing key capture is unrelated to refresh.
    let radar_core::plugin::Outcome::Handled(fx) = pages::key(
        &mut p,
        radar_core::plugin::Pane::Middle,
        Chord::char('r'),
    ) else {
        panic!("explicit refresh handled")
    };
    assert!(fx.iter().any(|fx| matches!(fx, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.detail")));
    let mut refreshed = pending;
    refreshed.owner = p.mr_page_reads[&refreshed.key];
    assert!(refreshed.owner.id > 1);
    assert!(p.mr_pages[&refreshed.key].loading);
    receive(&mut p, &refreshed, node());
    assert!(p.mr_pages[&refreshed.key].error.is_none());
    assert_eq!(p.mr_pages[&refreshed.key].at, p.now);
}

#[test]
fn transient_target_is_carried_by_recorded_task_without_journal_scope() {
    let mut p = demo::sample();
    let pending = launch(&mut p, PROJECT, IID);
    let action = page_history::action("not a URL", PROJECT, IID);
    let got = recorded(node(), PROJECT, IID);
    let Effect::SpawnAction { task, .. } = page_history::spawn(
        action,
        pending.key.clone(),
        pending.owner,
        Requested::new(PROJECT, IID),
        async move { Ok(got) },
    ) else {
        unreachable!()
    };
    let fx = p.on_msg(block(task));
    assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
    assert!(p.mr_pages[&pending.key].value.is_some());
    assert!(p.mr_pages[&pending.key].error.is_none());
    let snapshot = p.snapshot().unwrap().to_string();
    for transient in ["raw_iid", "Requested", REJECTION] {
        assert!(!snapshot.contains(transient));
    }
}
