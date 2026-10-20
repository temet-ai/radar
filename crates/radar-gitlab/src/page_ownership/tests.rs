//! Recorded providers only. Production dispatch futures are inspected and dropped,
//! never polled; replay futures below do not resolve credentials or use a network.

use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, DiffRead, MrDetailRead};
use crate::{GlMsg, demo, diff_history, page_history, pages};
use radar_core::journal::{ActionDescriptor, Outcome as JournalOutcome};
use radar_core::plugin::{Effect, Outcome, Pane};
use radar_core::view::{Chord, Key};
use radar_core::Plugin;
use serde_json::{Value, json};

const PROJECT: &str = "acme/team-a/api";
const IID: u64 = 263;
const HEAD: &str = "SECRET_HEAD";
const SKIP: &str = "gitlab.page.detail.application.skipped";
const DIFF_SKIP: &str = "gitlab.page.diffs.apply.skipped.stale";

#[derive(Clone)]
struct Pending {
    key: String,
    read: Read,
    requested: crate::page_identity::Requested,
    action: ActionDescriptor,
}
fn key(iid: u64) -> String {
    pages::mr_key(PROJECT, iid)
}
fn event(code: Key) -> Chord {
    Chord::new(code)
}
fn node(title: &str, head: &str, iid: u64) -> Value {
    json!({"iid": iid.to_string(), "title": title, "description": "SECRET_BODY",
        "project": {"name":"SECRET_PROJECT", "fullPath":PROJECT},
        "webUrl":"https://elsewhere.example/SECRET_URL",
        "diffRefs":{"headSha":head, "baseSha":"SECRET_BASE", "startSha":"SECRET_START"},
        "diffStats":[{"path":"SECRET_FILE", "additions":2,"deletions":1}],
        "commits":{"nodes":[]}, "discussions":{"nodes":[]},
        "jobs":{"iid":"1", "stages":{"nodes":[]}}})
}
fn recorded(n: Value) -> MrDetailRead {
    let replay = Replay::new(vec![(200, json!({"data":{"project":{"mergeRequest":n}}}))]);
    let got = block(Client::over(replay.clone()).mr_detail_observed(PROJECT, IID)).unwrap();
    assert_eq!(replay.asked().len(), 1);
    got
}
fn result(title: &str, head: &str, iid: u64) -> Result<Box<MrDetailRead>, String> {
    Ok(Box::new(recorded(node(title, head, iid))))
}
fn launch(p: &mut GitlabPlugin, iid: u64) -> Pending {
    let kind = pages::Kind::Mr {
        project: PROJECT.into(),
        iid,
    };
    let fx = if p.pages.last().is_some_and(|page| page.kind == kind) {
        let Outcome::Handled(fx) = pages::key(p, Pane::Middle, event(Key::Char('r'))) else {
            panic!("force handled")
        };
        fx
    } else {
        pages::push(p, kind)
    };
    let action = fx
        .into_iter()
        .find_map(|effect| match effect {
            Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.detail" => {
                Some(action)
            }
            _ => None,
        })
        .expect("an actual detail dispatch");
    Pending {
        key: key(iid),
        read: p.mr_page_reads[&key(iid)],
        requested: crate::page_identity::Requested::new(PROJECT, iid),
        action,
    }
}
fn receive(
    p: &mut GitlabPlugin,
    pending: &Pending,
    result: Result<Box<MrDetailRead>, String>,
) -> Vec<Effect> {
    p.on_msg(Box::new(GlMsg::MrPage {
        key: pending.key.clone(),
        read: pending.read,
        requested: pending.requested.clone(),
        action: pending.action.clone(),
        result,
    }))
}
fn installed() -> GitlabPlugin {
    let mut p = demo::sample();
    let pending = launch(&mut p, IID);
    receive(&mut p, &pending, result("SECRET_HELD", HEAD, IID));
    p
}
fn state(p: &GitlabPlugin) -> String {
    format!(
        "{:?}{:?}{:?}{:?}{:?}{:?}",
        p.mr_pages,
        p.mr_page_reads,
        p.diffs,
        p.pages,
        p.failed,
        p.snapshot()
    )
}
fn skipped(fx: &[Effect], scope: &ActionDescriptor, expected: JournalOutcome, kind: &str) {
    assert!(matches!(fx.first(), Some(Effect::Report(outcome)) if *outcome == expected));
    let Some(Effect::Observe {
        action,
        outcome: JournalOutcome::Blocked,
    }) = fx.last()
    else {
        panic!("skip after original evidence")
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
    assert!(!fx.iter().any(|e| matches!(
        e,
        Effect::Spawn(_) | Effect::SpawnAction { .. } | Effect::PersistCache | Effect::Emit { .. }
    )));
    assert!(!format!("{fx:?}").contains("SECRET"));
}
fn diff_launch(p: &mut GitlabPlugin) -> (Pending, String, usize) {
    p.pages.last_mut().unwrap().tab = 1;
    let fx = pages::changes::want(p);
    let [Effect::SpawnAction { action, .. }] = &fx[..] else {
        panic!("one diff dispatch: {fx:?}")
    };
    let ds = &p.diffs[&key(IID)];
    let (&page, &read) = ds.requests.iter().next().unwrap();
    (
        Pending {
            key: key(IID),
            read,
            requested: crate::page_identity::Requested::new(PROJECT, IID),
            action: action.clone(),
        },
        ds.head.clone(),
        page,
    )
}
fn diff_result(path: &str) -> Result<DiffRead, String> {
    let replay = Replay::new(vec![(
        200,
        json!([{"old_path":path,"new_path":path,"diff":"@@ -0,0 +1,2 @@\n+SECRET_LINE\n+SECRET_SECOND"}]),
    )]);
    Ok(block(Client::over(replay).diffs_observed(PROJECT, IID, 1)).unwrap())
}
fn diff_receive(
    p: &mut GitlabPlugin,
    pending: &(Pending, String, usize),
    result: Result<DiffRead, String>,
) -> Vec<Effect> {
    p.on_msg(Box::new(GlMsg::Diffs {
        key: pending.0.key.clone(),
        read: pending.0.read,
        head: pending.1.clone(),
        page: pending.2,
        action: pending.0.action.clone(),
        result,
    }))
}
/// Show the page (the part's view is what a renderer draws) and return the
/// targets it offers: the rows' own, `node:` for a pointer-pressed file
/// tree, and `line:` / `comment:` for each line of a diff (its numbers).
fn paint(p: &GitlabPlugin) -> Vec<String> {
    use radar_core::view::{Node, TreeRow};
    fn node_ids(rows: &[TreeRow], out: &mut Vec<String>) {
        for row in rows {
            out.push(format!("node:{}", row.id));
            node_ids(&row.children, out);
        }
    }
    fn targets(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(t) = map.get("target").and_then(|t| t.as_str()).filter(|t| !t.is_empty()) {
                    out.push(t.to_string());
                }
                map.values().for_each(|v| targets(v, out));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| targets(v, out)),
            _ => {}
        }
    }
    let vcx = radar_core::view::ViewCx {
        now: p.now,
        focus: Pane::Middle.into(),
        width: 160,
        height: 40,
        parts: radar_core::Parts::none(),
        icon_lead: 0,
        setup: false,
    };
    let view = Plugin::view(p, "repos", &vcx).expect("the screen has a view");
    let mut ids = Vec::new();
    let panes = view.page.as_ref().map_or(view.panes.as_slice(), |page| page.panes.as_slice());
    for pane in panes {
        match &pane.body {
            Node::Tree(tree) if tree.pointer_action => node_ids(&tree.rows, &mut ids),
            Node::Diff { lines, .. } => {
                for (i, line) in lines.iter().enumerate() {
                    ids.push(format!("line:{i}"));
                    if line.old.is_some() || line.new.is_some() {
                        ids.push(format!("comment:{i}"));
                    }
                }
            }
            _ => {}
        }
    }
    targets(&serde_json::to_value(&view).expect("the view serialises"), &mut ids);
    ids
}

#[test]
fn recorded_forced_overlap_in_both_orders_keeps_only_latest_exact_owner() {
    for reverse in [false, true] {
        let mut p = installed();
        let old = launch(&mut p, IID);
        let latest = launch(&mut p, IID);
        assert!(latest.read.id > old.read.id);
        let mut pending = vec![(old, "SECRET_OLD"), (latest.clone(), "SECRET_LATEST")];
        if reverse {
            pending.reverse();
        }
        for (read, title) in pending {
            let n = node(title, HEAD, IID);
            let replay = Replay::new(vec![(200, json!({"data":{"project":{"mergeRequest":n}}}))]);
            let client = Client::over(replay.clone());
            let effect = page_history::spawn(
                read.action.clone(),
                read.key.clone(),
                read.read,
                read.requested.clone(),
                async move {
                    client
                        .mr_detail_observed(PROJECT, IID)
                        .await
                        .map_err(|e| e.to_string())
                },
            );
            let before = state(&p);
            let Effect::SpawnAction { task, .. } = effect else {
                unreachable!()
            };
            let fx = p.on_msg(block(task));
            assert_eq!(replay.asked().len(), 1);
            if read.read != latest.read {
                assert_eq!(state(&p), before);
                skipped(&fx, &read.action, JournalOutcome::Succeeded, SKIP);
            }
        }
        assert_eq!(
            p.mr_pages[&key(IID)].value.as_ref().unwrap().0.title,
            "SECRET_LATEST"
        );
        assert!(!p.mr_pages[&key(IID)].loading);
        assert!(p.mr_page_reads.is_empty());
    }
}

#[test]
fn overlap_failure_partial_and_success_preserve_latest_loading_value_time_and_error() {
    for older_first in [false, true] {
        for old_mode in 0..3 {
            for new_mode in 0..3 {
                let mut p = installed();
                let held_at = p.mr_pages[&key(IID)].at;
                let old = launch(&mut p, IID);
                let latest = launch(&mut p, IID);
                p.now += 20;
                let mut completions = vec![
                    (old, old_mode, "SECRET_OLD"),
                    (latest.clone(), new_mode, "SECRET_NEW"),
                ];
                if !older_first {
                    completions.reverse();
                }
                for (read, mode, title) in completions {
                    let before = state(&p);
                    let mut n = node(title, HEAD, IID);
                    if mode == 1 {
                        n["commits"] = Value::Null;
                    }
                    let result = if mode == 2 {
                        Err("SECRET_ERROR".into())
                    } else {
                        Ok(Box::new(recorded(n)))
                    };
                    let fx = receive(&mut p, &read, result);
                    if read.read != latest.read {
                        assert_eq!(state(&p), before);
                        skipped(
                            &fx,
                            &read.action,
                            [
                                JournalOutcome::Succeeded,
                                JournalOutcome::Partial,
                                JournalOutcome::Failed,
                            ][mode],
                            SKIP,
                        );
                    }
                }
                let f = &p.mr_pages[&key(IID)];
                assert!(!f.loading && p.mr_page_reads.is_empty());
                assert_eq!(
                    f.value.as_ref().unwrap().0.title,
                    if new_mode == 2 {
                        "SECRET_HELD"
                    } else {
                        "SECRET_NEW"
                    }
                );
                assert_eq!(f.at, if new_mode == 2 { held_at } else { p.now });
                assert_eq!(
                    f.error.as_deref(),
                    (new_mode == 2).then_some("SECRET_ERROR")
                );
            }
        }
    }
}

#[test]
fn duplicate_detail_success_and_failure_leave_current_or_newer_state_untouched() {
    for newer in [false, true] {
        for failed in [false, true] {
            let mut p = installed();
            let read = launch(&mut p, IID);
            receive(&mut p, &read, result("SECRET_ACCEPTED", HEAD, IID));
            if newer {
                launch(&mut p, IID);
            }
            let before = state(&p);
            let fx = receive(
                &mut p,
                &read,
                if failed {
                    Err("SECRET_ERROR".into())
                } else {
                    result("SECRET_DUPLICATE", HEAD, IID)
                },
            );
            assert_eq!(state(&p), before);
            skipped(
                &fx,
                &read.action,
                if failed {
                    JournalOutcome::Failed
                } else {
                    JournalOutcome::Succeeded
                },
                SKIP,
            );
        }
    }
}

#[test]
fn distinct_keys_hidden_and_closed_pages_keep_their_own_pending_work() {
    let mut p = demo::sample();
    let first = launch(&mut p, IID);
    let second = launch(&mut p, 264);
    receive(&mut p, &first, result("SECRET_FIRST", HEAD, IID));
    assert!(p.mr_pages[&key(264)].loading);
    assert_eq!(p.mr_page_reads[&key(264)], second.read);
    p.pages.clear();
    receive(&mut p, &second, result("SECRET_SECOND", HEAD, 264));
    assert_eq!(
        p.mr_pages[&key(IID)].value.as_ref().unwrap().0.title,
        "SECRET_FIRST"
    );
    assert_eq!(
        p.mr_pages[&key(264)].value.as_ref().unwrap().0.title,
        "SECRET_SECOND"
    );
    assert!(p.mr_page_reads.is_empty());
}

#[test]
fn effective_url_token_aba_and_default_changes_retire_reads_without_resetting_sequence() {
    for change in 0..4 {
        let mut p = installed();
        if change == 3 {
            p.replace_connection(
                "https://configured.example".into(),
                "env:SECRET_CONFIGURED".into(),
            );
        }
        let old = launch(&mut p, IID);
        let cfg = p.cfg.clone();
        match change {
            0 => p.replace_connection("https://other.example".into(), cfg.token.clone()),
            1 => p.replace_connection(cfg.url.clone(), "env:SECRET_OTHER_REFERENCE".into()),
            2 => {
                p.replace_connection("https://other.example".into(), cfg.token.clone());
                p.replace_connection(cfg.url.clone(), cfg.token.clone());
            }
            _ => p.configure(None).unwrap(),
        }
        assert!(p.mr_pages.is_empty() && p.mr_page_reads.is_empty() && p.diffs.is_empty());
        let new = launch(&mut p, IID);
        assert!(new.read.id > old.read.id);
        let before = state(&p);
        let fx = receive(&mut p, &old, result("SECRET_OLD", HEAD, IID));
        assert_eq!(state(&p), before);
        skipped(&fx, &old.action, JournalOutcome::Succeeded, SKIP);
        receive(&mut p, &new, result("SECRET_NEW", HEAD, IID));
        assert_eq!(
            p.mr_pages[&key(IID)].value.as_ref().unwrap().0.title,
            "SECRET_NEW"
        );
    }
}

#[test]
fn identical_defaults_and_unrelated_configuration_preserve_pending_owners_and_caches() {
    for mode in 0..3 {
        let mut p = if mode == 0 {
            GitlabPlugin::default()
        } else {
            installed()
        };
        let read = launch(&mut p, IID);
        let before = state(&p);
        let stamp = p.connection_stamp();
        if mode == 0 {
            p.configure(None).unwrap();
        } else {
            let mut cfg = p.cfg.clone();
            if mode == 2 {
                cfg.base = "elsewhere".into();
                cfg.sources.clear();
                cfg.views.clear();
                cfg.refresh_secs += 3;
                cfg.side_by_side = !cfg.side_by_side;
            }
            p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
                .unwrap();
        }
        assert_eq!(p.connection_stamp(), stamp);
        assert_eq!(state(&p), before);
        receive(&mut p, &read, result("SECRET_CURRENT", HEAD, IID));
        assert!(!p.mr_pages[&key(IID)].loading);
    }
}

#[test]
fn invalid_url_and_absent_journal_subject_still_have_internal_ownership() {
    for url in ["", "not a URL", "file:///SECRET"] {
        let mut p = demo::sample();
        p.replace_connection(url.into(), "env:SECRET_REFERENCE".into());
        let old = launch(&mut p, IID);
        let current = launch(&mut p, IID);
        assert!(old.action.subjects.is_empty() && old.action.metadata.redacted);
        let before = state(&p);
        let fx = receive(&mut p, &old, result("SECRET_STALE", HEAD, IID));
        skipped(&fx, &old.action, JournalOutcome::Succeeded, SKIP);
        assert_eq!(state(&p), before);
        receive(&mut p, &current, result("SECRET_CURRENT", HEAD, IID));
        assert_eq!(
            p.mr_pages[&key(IID)].value.as_ref().unwrap().0.title,
            "SECRET_CURRENT"
        );
    }
}

#[test]
fn missing_detail_cache_cannot_be_recreated_even_by_its_pending_owner() {
    let mut p = installed();
    let pending = launch(&mut p, IID);
    p.mr_pages.clear();
    let before = state(&p);
    let fx = receive(&mut p, &pending, result("SECRET_STALE", HEAD, IID));
    assert_eq!(state(&p), before);
    skipped(&fx, &pending.action, JournalOutcome::Succeeded, SKIP);
}

#[test]
fn stale_red_detail_cannot_start_diagnosis_or_selected_pages_diff_continuations() {
    let mut p = installed();
    let old = launch(&mut p, IID);
    launch(&mut p, IID);
    p.pages.last_mut().unwrap().tab = 1;
    let mut n = node("SECRET_STALE", HEAD, IID);
    n["headPipeline"] = json!({"status":"FAILED","path":"/acme/team-a/api/-/pipelines/999"});
    let before = state(&p);
    let fx = receive(&mut p, &old, Ok(Box::new(recorded(n))));
    assert_eq!(state(&p), before);
    skipped(&fx, &old.action, JournalOutcome::Succeeded, SKIP);
    assert!(!p.failed.contains_key(&crate::why::key(PROJECT, 999)) && p.diffs.is_empty());
}

#[test]
fn diff_duplicate_success_failure_and_retry_cannot_steal_newer_owner() {
    for duplicate_failure in [false, true] {
        let mut p = installed();
        let old = diff_launch(&mut p);
        diff_receive(&mut p, &old, Err("SECRET_FIRST_ERROR".into()));
        let current = diff_launch(&mut p);
        assert!(current.0.read.id > old.0.read.id);
        let before = state(&p);
        let fx = diff_receive(
            &mut p,
            &old,
            if duplicate_failure {
                Err("SECRET_STALE_ERROR".into())
            } else {
                diff_result("SECRET_FILE")
            },
        );
        assert_eq!(state(&p), before);
        skipped(
            &fx,
            &old.0.action,
            if duplicate_failure {
                JournalOutcome::Failed
            } else {
                JournalOutcome::Succeeded
            },
            DIFF_SKIP,
        );
        diff_receive(&mut p, &current, diff_result("SECRET_FILE"));
        let before = state(&p);
        let fx = diff_receive(&mut p, &current, diff_result("SECRET_FILE"));
        assert_eq!(state(&p), before);
        skipped(&fx, &current.0.action, JournalOutcome::Succeeded, DIFF_SKIP);
    }
}

#[test]
fn same_head_cross_connection_and_aba_diffs_never_apply_to_recreated_cache() {
    for aba in [false, true] {
        for empty in [false, true] {
            let mut p = installed();
            let head = if empty { "" } else { HEAD };
            if empty {
                let detail = launch(&mut p, IID);
                receive(&mut p, &detail, result("SECRET_HELD", head, IID));
            }
            let old = diff_launch(&mut p);
            let cfg = p.cfg.clone();
            p.replace_connection("https://other.example".into(), cfg.token.clone());
            if aba {
                p.replace_connection(cfg.url, cfg.token);
            }
            let detail = launch(&mut p, IID);
            receive(&mut p, &detail, result("SECRET_NEW", head, IID));
            // Accepted current detail may start the selected Changes tab's diff itself.
            let ds = &p.diffs[&key(IID)];
            let current = ds.requests[&1];
            assert_ne!(current, old.0.read);
            let before = state(&p);
            let fx = diff_receive(&mut p, &old, diff_result("SECRET_OLD_FILE"));
            assert_eq!(state(&p), before);
            skipped(&fx, &old.0.action, JournalOutcome::Succeeded, DIFF_SKIP);
        }
    }
}

#[test]
fn cleared_empty_head_diff_cache_is_never_recreated_by_late_success_or_error() {
    for failure in [false, true] {
        let mut p = installed();
        let detail = launch(&mut p, IID);
        receive(&mut p, &detail, result("SECRET_HELD", "", IID));
        let old = diff_launch(&mut p);
        p.diffs.clear();
        let before = state(&p);
        let fx = diff_receive(
            &mut p,
            &old,
            if failure {
                Err("SECRET_ERROR".into())
            } else {
                diff_result("SECRET_FILE")
            },
        );
        assert_eq!(state(&p), before);
        assert!(p.diffs.is_empty());
        skipped(
            &fx,
            &old.0.action,
            if failure {
                JournalOutcome::Failed
            } else {
                JournalOutcome::Succeeded
            },
            DIFF_SKIP,
        );
    }
}

#[test]
fn accepted_hidden_head_change_retires_diff_owner_and_head_aba_cannot_revive_it() {
    let mut p = installed();
    let old_diff = diff_launch(&mut p);
    let detail = launch(&mut p, IID);
    pages::push(&mut p, pages::Kind::Repo(PROJECT.into()));
    receive(&mut p, &detail, result("SECRET_NEW", "SECRET_HEAD_B", IID));
    assert!(!p.diffs.contains_key(&key(IID)));
    p.pages.pop();
    let back = launch(&mut p, IID);
    receive(&mut p, &back, result("SECRET_BACK", HEAD, IID));
    assert!(p.diffs[&key(IID)].requests[&1].id > old_diff.0.read.id);
    let before = state(&p);
    let fx = diff_receive(&mut p, &old_diff, diff_result("SECRET_FILE"));
    assert_eq!(state(&p), before);
    skipped(
        &fx,
        &old_diff.0.action,
        JournalOutcome::Succeeded,
        DIFF_SKIP,
    );
}

#[test]
fn same_head_detail_preserves_pending_diff_and_held_files_even_when_hidden() {
    let mut p = installed();
    let pending = diff_launch(&mut p);
    let detail = launch(&mut p, IID);
    pages::push(&mut p, pages::Kind::Repo(PROJECT.into()));
    let before = format!("{:?}", p.diffs);
    receive(&mut p, &detail, result("SECRET_REFRESH", HEAD, IID));
    assert_eq!(format!("{:?}", p.diffs), before);
    diff_receive(&mut p, &pending, diff_result("SECRET_FILE"));
    assert!(p.diffs[&key(IID)].files.contains_key("SECRET_FILE"));
    p.pages.pop();
    let detail = launch(&mut p, IID);
    let before = format!("{:?}", p.diffs);
    receive(&mut p, &detail, result("SECRET_REFRESH_AGAIN", HEAD, IID));
    assert_eq!(format!("{:?}", p.diffs), before);
}

#[test]
fn different_diff_pages_and_mr_keys_do_not_consume_each_others_pending_slots() {
    let mut p = installed();
    let pending = diff_launch(&mut p);
    let other = next(&mut p);
    let another = next(&mut p);
    p.diffs
        .get_mut(&key(IID))
        .unwrap()
        .requests
        .insert(2, other);
    p.diffs.insert(
        key(264),
        pages::changes::Diffs {
            head: HEAD.into(),
            requests: std::collections::BTreeMap::from([(1, another)]),
            ..Default::default()
        },
    );
    let mut mismatched = pending.clone();
    mismatched.2 = 2;
    let before = state(&p);
    let fx = diff_receive(&mut p, &mismatched, diff_result("SECRET_FILE"));
    assert_eq!(state(&p), before);
    skipped(&fx, &pending.0.action, JournalOutcome::Succeeded, DIFF_SKIP);
    diff_receive(&mut p, &pending, diff_result("SECRET_FILE"));
    assert_eq!(p.diffs[&key(IID)].requests[&2], other);
    assert_eq!(p.diffs[&key(264)].requests[&1], another);
}

#[test]
fn connection_invalidation_clears_all_mr_buffers_but_preserves_stack_source_and_authored_state() {
    let mut p = installed();
    let pending = diff_launch(&mut p);
    diff_receive(&mut p, &pending, diff_result("SECRET_FILE"));
    paint(&p);
    crate::write::start(&mut p, PROJECT, IID, crate::write::Target::Note);
    p.paste("SECRET_AUTHORED_TEXT");
    p.recovery.test_keep("SECRET_KEPT_TEXT");
    p.runs.insert(PROJECT.into(), Default::default());
    p.failed.insert("preserved".into(), Default::default());
    p.logs.insert(crate::log_ownership::JobKey::new(PROJECT, 9), Default::default());
    p.jobs.insert(
        crate::log_ownership::JobKey::new(PROJECT, 9),
        crate::model::JobInfo {
            id: 9,
            project: PROJECT.into(),
            name: "SECRET_JOB".into(),
            stage: "test".into(),
            status: "failed".into(),
            duration_secs: None,
            failure_reason: String::new(),
            url: String::new(),
            allow_failure: false,
            pipeline: 1,
        },
    );
    p.log_reads.entry(crate::log_ownership::JobKey::new(PROJECT, 9)).or_default();
    pages::push(&mut p, pages::Kind::Repo(PROJECT.into()));
    let other = launch(&mut p, 264);
    receive(&mut p, &other, result("SECRET_OTHER", HEAD, 264));
    for page in &mut p.pages {
        if matches!(page.kind, pages::Kind::Mr { .. }) {
            page.extra
                .borrow_mut()
                .push(("SECRET_EXTRA".into(), vec![]));
            page.links.borrow_mut().push(radar_core::Subject::new(
                "link",
                "SECRET_LINK",
                "SECRET_LABEL",
            ));
            page.under.borrow_mut().push((
                "local",
                radar_core::Subject::new("repo", "SECRET_REPO", "SECRET_LABEL"),
                "SECRET_ACTION".into(),
            ));
            page.seen.borrow_mut().insert("SECRET_FILE".into());
            page.flipped.borrow_mut().insert("SECRET_THREAD".into());
            *page.jump.borrow_mut() = Some("SECRET_JUMP".into());
            page.row.set(3);
            page.file.set(3);
            page.diff.set(3);
        }
    }
    p.pages.last_mut().unwrap().tab = 5;
    let kinds = p
        .pages
        .iter()
        .map(|page| page.kind.clone())
        .collect::<Vec<_>>();
    let source = p.snapshot();
    let other_page = format!("{:?}", p.pages[1]);
    let held = format!(
        "{:?}{:?}", p.compose, p.recovery
    );
    let sequence = p.next_page_read;
    p.replace_connection("https://other.example".into(), "env:SECRET_OTHER".into());
    assert!(p.mr_pages.is_empty() && p.diffs.is_empty() && p.mr_page_reads.is_empty());
    assert!(p.runs.is_empty() && p.runs_scopes.is_empty());
    assert_eq!(p.next_page_read, sequence);
    assert!(p.failed.is_empty() && p.jobs.is_empty() && p.logs.is_empty() && p.log_reads.is_empty());
    assert_eq!(
        p.pages
            .iter()
            .map(|page| page.kind.clone())
            .collect::<Vec<_>>(),
        kinds
    );
    assert_eq!(p.snapshot(), source);
    assert_eq!(format!("{:?}", p.pages[1]), other_page);
    assert_eq!(
        format!(
            "{:?}{:?}", p.compose, p.recovery
        ),
        held
    );
    assert_eq!(p.pages[0].tab, 1);
    assert_eq!(p.pages.last().unwrap().tab, 4);
    assert_eq!(p.pages.last().unwrap().row.get(), 0);
    for page in &p.pages {
        if matches!(page.kind, pages::Kind::Mr { .. }) {
            assert!(
                page.extra.borrow().is_empty()
                    && page.links.borrow().is_empty()
                    && page.under.borrow().is_empty()
            );
            assert!(
                page.seen.borrow().is_empty()
                    && page.flipped.borrow().is_empty()
                    && page.jump.borrow().is_none()
            );
            assert_eq!((page.file.get(), page.diff.cursor()), (0, 0));
            assert!(!page.placed.get() && !page.tree_shown.get() && !page.split_shown.get());
            assert!(!page.detail_painted.get() && !page.diff_painted.get());
            assert!(!format!("{:?}", page.diff).contains("SECRET"));
        }
    }
    // The ordinary source-list fallback remains intentionally available.
    assert!(pages::find_mr(&p, PROJECT, IID).is_some());
    p.pages.pop();
    p.pages.pop();
    let fx = pages::key(&mut p, Pane::Middle, event(Key::Char('o')));
    assert!(matches!(fx,Outcome::Handled(ref fx) if fx.iter().any(|e|matches!(e,Effect::Open(_)))));
    assert_eq!(p.compose.as_ref().unwrap().text(), "SECRET_AUTHORED_TEXT");
}

#[test]
fn actual_pre_redraw_clicks_and_coordinate_keys_are_inert_after_connection_invalidation() {
    let mut p = installed();
    let diff = diff_launch(&mut p);
    diff_receive(&mut p, &diff, diff_result("SECRET_FILE"));
    let hits = paint(&p);
    let ids = hits
        .into_iter()
        .filter(|id| {
            id.starts_with("node:") || id.starts_with("line:") || id.starts_with("comment:")
        })
        .collect::<Vec<_>>();
    assert!(ids.iter().any(|id| id.starts_with("comment:")));
    p.replace_connection("https://other.example".into(), "env:SECRET_OTHER".into());
    let before = state(&p);
    for id in &ids {
        p.click("repos", id);
    }
    for c in ['c', 'd'] {
        pages::key(&mut p, Pane::Middle, event(Key::Char(c)));
    }
    pages::key(&mut p, Pane::Nav, event(Key::Enter));
    assert_eq!(state(&p), before);
    assert!(p.compose.is_none() && p.change.is_none());
    // Source fallback rendering does not certify removed dedicated payloads.
    paint(&p);
    assert!(!p.pages[0].detail_painted.get() && !p.pages[0].diff_painted.get());
    let read = launch(&mut p, IID);
    receive(&mut p, &read, result("SECRET_NEW", HEAD, IID));
    let before_file = p.pages[0].file.get();
    for id in &ids {
        p.click("repos", id);
    }
    assert_eq!(p.pages[0].file.get(), before_file);
    assert!(p.compose.is_none());
    let ds = &p.diffs[&key(IID)];
    let pending = (
        Pending {
            key: key(IID),
            read: ds.requests[&1],
            requested: crate::page_identity::Requested::new(PROJECT, IID),
            action: diff_history::action(&p.cfg.url, PROJECT, IID, 1),
        },
        HEAD.into(),
        1,
    );
    diff_receive(&mut p, &pending, diff_result("SECRET_FILE"));
    for id in &ids {
        p.click("repos", id);
    }
    assert!(p.compose.is_none());
    paint(&p);
    p.click("repos", "comment:1");
    assert!(p.compose.is_some());
}

#[test]
fn accepted_same_head_detail_replacement_invalidates_file_thread_and_diff_coordinates() {
    let mut p = installed();
    let diff = diff_launch(&mut p);
    diff_receive(&mut p, &diff, diff_result("SECRET_FILE"));
    paint(&p);
    let pending = launch(&mut p, IID);
    let mut n = node("SECRET_NEW", HEAD, IID);
    n["diffStats"] = json!([{"path":"SECRET_REORDERED_FILE","additions":0,"deletions":0},{"path":"SECRET_FILE","additions":0,"deletions":0}]);
    receive(&mut p, &pending, Ok(Box::new(recorded(n))));
    assert!(p.diffs[&key(IID)].files.contains_key("SECRET_FILE"));
    assert!(!p.pages[0].detail_painted.get() && !p.pages[0].diff_painted.get());
    for id in ["node:f:1", "line:2", "comment:1"] {
        p.click("repos", id);
    }
    pages::key(&mut p, Pane::Nav, event(Key::Enter));
    pages::key(&mut p, Pane::Middle, event(Key::Char('c')));
    assert_eq!(p.pages[0].file.get(), 0);
    assert!(p.compose.is_none());
    paint(&p);
    assert!(p.pages[0].detail_painted.get());
    assert!(
        !p.pages[0].diff_painted.get(),
        "the newly selected file has no painted diff"
    );
    // A loaded dedicated detail alone cannot validate line/gutter coordinates.
    p.click("repos", "comment:1");
    assert!(p.compose.is_none());
    p.click("repos", "node:f:1");
    paint(&p);
    p.click("repos", "comment:1");
    assert!(p.compose.is_some());
}

#[test]
fn accepted_diff_replacement_requires_fresh_diff_paint_but_keeps_file_navigation() {
    let mut p = installed();
    let diff = diff_launch(&mut p);
    paint(&p);
    assert!(p.pages[0].detail_painted.get());
    diff_receive(&mut p, &diff, diff_result("SECRET_FILE"));
    assert!(p.pages[0].detail_painted.get() && !p.pages[0].diff_painted.get());
    pages::key(&mut p, Pane::Middle, event(Key::Char('c')));
    assert!(p.compose.is_none());
    paint(&p);
    pages::key(&mut p, Pane::Middle, event(Key::Down));
    pages::key(&mut p, Pane::Middle, event(Key::Char('c')));
    assert!(p.compose.is_some());
}

#[test]
fn fresh_busy_noops_do_not_allocate_and_every_force_still_dispatches() {
    let only_open = |fx: Vec<Effect>| matches!(&fx[..],
        [Effect::Observe { action, outcome: JournalOutcome::Succeeded }]
        if action.kind.as_str() == "gitlab.page.open");
    let mut p = installed();
    let sequence = p.next_page_read;
    assert!(only_open(
        pages::push(
            &mut p,
            pages::Kind::Mr {
                project: PROJECT.into(),
                iid: IID
            }
        )
    ));
    assert_eq!(p.next_page_read, sequence);
    let first = launch(&mut p, IID);
    let sequence = p.next_page_read;
    assert!(only_open(
        pages::push(
            &mut p,
            pages::Kind::Mr {
                project: PROJECT.into(),
                iid: IID
            }
        )
    ));
    assert_eq!(p.next_page_read, sequence);
    let second = launch(&mut p, IID);
    assert_eq!(second.read.id, first.read.id + 1);
}

#[test]
#[should_panic(expected = "GitLab page read sequence exhausted")]
fn request_sequence_never_wraps() {
    let mut p = GitlabPlugin {
        next_page_read: u64::MAX,
        ..Default::default()
    };
    next(&mut p);
}

#[test]
fn reordered_threads_are_inert_before_redraw_and_new_row_works_after_render() {
    let mut p = installed();
    let mut n = node("SECRET_THREADS", HEAD, IID);
    let thread = |id: &str| {
        json!({"id":id,"resolvable":true,"resolved":false,"notes":{"nodes":[
        {"system":false,"author":{"name":"SECRET_AUTHOR"},"body":"SECRET_NOTE",
         "position":{"filePath":"SECRET_FILE","newLine":1}}]}})
    };
    n["discussions"] = json!({"nodes":[thread("SECRET_OLD_THREAD"),thread("SECRET_NEW_THREAD")]});
    let pending = launch(&mut p, IID);
    receive(&mut p, &pending, Ok(Box::new(recorded(n.clone()))));
    p.pages[0].tab = 2;
    p.pages[0].row.set(1);
    let hits = paint(&p);
    assert!(hits.iter().any(|id| id == "row:1"));
    n["discussions"]["nodes"] = json!([thread("SECRET_NEW_THREAD"), thread("SECRET_OLD_THREAD")]);
    let pending = launch(&mut p, IID);
    receive(&mut p, &pending, Ok(Box::new(recorded(n))));
    assert!(!p.pages[0].detail_painted.get());
    p.click("repos", "row:2");
    assert_eq!(p.pages[0].row.get(), 1);
    for key in [Key::Char('c'), Key::Char('d'), Key::Enter] {
        pages::key(&mut p, Pane::Middle, event(key));
        assert!(p.compose.is_none() && p.change.is_none());
        assert_eq!(p.pages[0].tab, 2);
    }
    // Refresh and back/tab navigation still work while coordinate actions wait.
    let next = launch(&mut p, IID);
    assert_eq!(p.mr_page_reads[&key(IID)], next.read);
    paint(&p);
    pages::key(&mut p, Pane::Middle, event(Key::Char('c')));
    assert!(
        matches!(&p.compose.as_ref().unwrap().target,crate::write::Target::Reply{discussion,..} if discussion=="SECRET_NEW_THREAD")
    );
}

#[test]
fn removed_extra_tab_and_under_link_buffers_are_inert_before_next_render() {
    let mut p = installed();
    let page = &mut p.pages[0];
    page.extra.borrow_mut().push((
        "SECRET_EXTRA".into(),
        vec![radar_core::Row {
            line: "SECRET_ROW".into(),
            subject: Some(
                radar_core::Subject::new("link", "SECRET_DESTINATION", "SECRET_LABEL")
                    .with_url("https://elsewhere.example/SECRET_URL"),
            ),
        }],
    ));
    page.under.borrow_mut().push((
        "local",
        radar_core::Subject::new("repo", "SECRET_REPO", "SECRET_LABEL"),
        "SECRET_ACTION".into(),
    ));
    page.links
        .borrow_mut()
        .push(radar_core::Subject::new("link", "SECRET_LINK", "SECRET_LABEL"));
    page.tab = 5;
    p.replace_connection("https://other.example".into(), "env:SECRET_OTHER".into());
    assert_eq!(p.pages[0].tab, 4);
    let Outcome::Handled(fx) = p.click("repos", "under:0") else {
        panic!("click handled")
    };
    assert!(!fx.iter().any(|effect| matches!(
        effect,
        Effect::Act { .. } | Effect::Open(_) | Effect::Show(_)
    )));
    let Outcome::Handled(fx) = pages::key(&mut p, Pane::Middle, event(Key::Enter)) else {
        panic!("enter handled")
    };
    assert!(!fx.iter().any(|effect| matches!(
        effect,
        Effect::Act { .. } | Effect::Open(_) | Effect::Show(_)
    )));
    assert!(!format!("{:?}", pages::act::links_menu(&p, IID)).contains("SECRET"));
    pages::key(&mut p, Pane::Middle, event(Key::Char('[')));
    assert_eq!(p.pages[0].tab, 3);
    pages::key(&mut p, Pane::Middle, event(Key::Esc));
    assert!(p.pages.is_empty());
}

#[test]
fn tracked_stale_outcomes_keep_original_scopes_siblings_and_private_sqlite_history() {
    use radar_core::Host;
    use radar_core::host::{Event, HostEffect, JournalRecord};
    use radar_core::journal::{ActionId, Journal, Origin, Query};
    use radar_core::plugin::Msg;
    use std::sync::{Arc, Mutex};
    enum Begin {
        Reads,
        Change,
    }
    type CapturedReads = Arc<Mutex<Vec<(Pending, Option<(String, usize)>)>>>;
    struct Reads {
        p: GitlabPlugin,
        pending: CapturedReads,
    }
    impl Plugin for Reads {
        fn id(&self) -> &'static str {
            "gitlab"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
            match msg.downcast::<Begin>() {
                Ok(begin) => match *begin {
                    Begin::Reads => {
                        let mut out = vec![Effect::Journal(ActionDescriptor::new(
                            "gitlab.test.write",
                            Origin::Plugin("gitlab".into()),
                        ))];
                        for _ in 0..2 {
                            let Outcome::Handled(fx) =
                                pages::key(&mut self.p, Pane::Middle, event(Key::Char('r')))
                            else {
                                unreachable!()
                            };
                            let action = fx
                                .iter()
                                .find_map(|fx| match fx {
                                    Effect::SpawnAction { action, .. }
                                        if action.kind.as_str() == "gitlab.page.detail" =>
                                    {
                                        Some(action.clone())
                                    }
                                    _ => None,
                                })
                                .unwrap();
                            self.pending.lock().unwrap().push((
                                Pending {
                                    key: key(IID),
                                    read: self.p.mr_page_reads[&key(IID)],
                                    requested: crate::page_identity::Requested::new(PROJECT, IID),
                                    action,
                                },
                                None,
                            ));
                            out.extend(fx);
                        }
                        self.p.pages[0].tab = 1;
                        let fx = pages::changes::want(&mut self.p);
                        let [Effect::SpawnAction { action, .. }] = &fx[..] else {
                            unreachable!()
                        };
                        let ds = &self.p.diffs[&key(IID)];
                        self.pending.lock().unwrap().push((
                            Pending {
                                key: key(IID),
                                read: ds.requests[&1],
                                requested: crate::page_identity::Requested::new(PROJECT, IID),
                                action: action.clone(),
                            },
                            Some((ds.head.clone(), 1)),
                        ));
                        out.extend(fx);
                        out.push(Effect::Report(JournalOutcome::Succeeded));
                        out
                    }
                    Begin::Change => {
                        self.p.replace_connection(
                            "https://new.example".into(),
                            "env:SECRET_NEW_TOKEN".into(),
                        );
                        vec![]
                    }
                },
                Err(msg) => self.p.on_msg(msg),
            }
        }
    }
    let records = |fx: &[HostEffect]| {
        fx.iter()
            .filter_map(|fx| match fx {
                HostEffect::Journal(record) => Some(record.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let terminals = |records: &[JournalRecord], id: &ActionId| {
        records
            .iter()
            .filter_map(|r| match r {
                JournalRecord::Outcome {
                    id: got, outcome, ..
                } if got == id && outcome.is_terminal() => Some(*outcome),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let mut p = installed();
    p.cfg.url =
        "https://SECRET_USER:SECRET_PASSWORD@original.example/?token=SECRET_TOKEN#SECRET_FRAGMENT"
            .into();
    let pending = Arc::new(Mutex::new(vec![]));
    let mut host = Host::new(
        vec![Box::new(Reads {
            p,
            pending: pending.clone(),
        })],
        1,
    )
    .unwrap();
    host.enable_journal();
    let start = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Reads),
    });
    let tasks = start
        .iter()
        .filter_map(|fx| match fx {
            HostEffect::Tracked {
                action,
                semantic: Some(semantic),
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(tasks.len(), 3);
    let pending = pending.lock().unwrap().clone();
    let mut all = records(&start);
    let write = all
        .iter()
        .find_map(|r| match r {
            JournalRecord::Begin { id, descriptor, .. }
                if descriptor.kind.as_str() == "gitlab.test.write" =>
            {
                Some(id.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(terminals(&all, &write), [JournalOutcome::Succeeded]);
    all.extend(records(&host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin::Change),
    })));
    for index in [1, 0, 2] {
        let (pending, diff) = &pending[index];
        let (message, expected, skip_kind): (Msg, _, _) = if let Some((head, page)) = diff {
            (
                Box::new(GlMsg::Diffs {
                    key: pending.key.clone(),
                    read: pending.read,
                    head: head.clone(),
                    page: *page,
                    action: pending.action.clone(),
                    result: Err("SECRET_DIFF_ERROR".into()),
                }),
                JournalOutcome::Failed,
                DIFF_SKIP,
            )
        } else {
            let mut n = node("SECRET_TITLE", HEAD, IID);
            if index == 0 {
                n["commits"] = Value::Null;
            }
            (
                Box::new(GlMsg::MrPage {
                    key: pending.key.clone(),
                    read: pending.read,
                    requested: pending.requested.clone(),
                    action: pending.action.clone(),
                    result: Ok(Box::new(recorded(n))),
                }),
                if index == 0 {
                    JournalOutcome::Partial
                } else {
                    JournalOutcome::Succeeded
                },
                SKIP,
            )
        };
        let reply = host.update(Event::Tracked {
            action: tasks[index].0.clone(),
            semantic: Some(tasks[index].1.clone()),
            event: Box::new(Event::Msg {
                plugin: 0,
                msg: message,
            }),
        });
        assert!(!reply.iter().any(|fx|matches!(fx,HostEffect::Tracked{effect,..}if matches!(&**effect,HostEffect::Spawn{..}))));
        let got = records(&reply);
        assert_eq!(terminals(&got, &tasks[index].1), [expected]);
        assert!(terminals(&got, &write).is_empty());
        for (i, (_, sibling)) in tasks.iter().enumerate() {
            if i != index {
                assert!(terminals(&got, sibling).is_empty());
            }
        }
        let skips = got
            .iter()
            .filter_map(|r| match r {
                JournalRecord::Begin {
                    id,
                    descriptor,
                    parent,
                } if descriptor.kind.as_str() == skip_kind => Some((id, descriptor, parent)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(skips.len(), 1);
        let (skip, descriptor, parent) = skips[0];
        assert_eq!(descriptor.subjects, pending.action.subjects);
        assert_eq!(
            descriptor.subjects[0].key.instance,
            "https://original.example/"
        );
        assert_eq!(parent.as_ref(), Some(&tasks[index].0));
        assert_eq!(terminals(&got, skip), [JournalOutcome::Blocked]);
        all.extend(got);
    }
    let dir = std::env::temp_dir().join(format!(
        "radar-gitlab-ownership-history-{}",
        ActionId::new().as_str()
    ));
    let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = journal.start_session().unwrap();
    for record in all {
        match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => journal
                .begin_with_id(&session, &id, descriptor, parent.as_ref())
                .unwrap(),
            JournalRecord::Outcome {
                id,
                outcome,
                metadata,
            } => {
                journal.record(&id, outcome, metadata).unwrap();
            }
            JournalRecord::Gap { .. } => panic!("unexpected history gap"),
        }
    }
    journal.finish_session(&session).unwrap();
    let events = journal
        .query(&Query {
            limit: 1000,
            ..Default::default()
        })
        .unwrap();
    let exported = serde_json::to_string(&events).unwrap();
    for secret in [
        "SECRET",
        "ConnectionStamp",
        "next_page_read",
        "\"requests\"",
        "detail_painted",
    ] {
        assert!(!exported.contains(secret), "{exported}");
    }
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind.as_str() == SKIP && e.outcome == JournalOutcome::Blocked)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind.as_str() == DIFF_SKIP && e.outcome == JournalOutcome::Blocked)
            .count(),
        1
    );
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
}
