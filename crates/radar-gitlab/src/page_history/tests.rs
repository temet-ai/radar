use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, Method, MrDetailEvidence, Resp, parse_detail, parse_mr};
use crate::{GitlabPlugin, demo, pages};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query, SubjectRole};
use radar_core::plugin::Pane;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};

const PROJECT: &str = "acme/team-a/api";
const IID: u64 = 263;
const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";
const BASE: &str = "https://gitlab.example.com";

fn node() -> Value {
    json!({
        "iid": "263", "title": "SECRET_TITLE", "description": "SECRET_BODY",
        "project": {"name": "SECRET_PROJECT", "fullPath": PROJECT},
        "webUrl": "https://elsewhere.example/SECRET_URL",
        "diffRefs": {"headSha": "SECRET_HEAD", "baseSha": "SECRET_BASE", "startSha": "SECRET_START"},
        "diffStats": [{"path": "SECRET_FILE", "additions": 2, "deletions": 1}],
        "commits": {"nodes": [{"shortId": "SECRET_SHA", "title": "SECRET_COMMIT", "authorName": "SECRET_AUTHOR"}]},
        "discussions": {"nodes": [{"id": "SECRET_DISCUSSION", "notes": {"nodes": [
            {"body": "SECRET_NOTE", "system": false, "author": {"name": "SECRET_NAME"}, "position": null}
        ]}}]},
        "jobs": {"iid": "SECRET_PIPELINE", "stages": {"nodes": [
            {"name": "SECRET_STAGE", "jobs": {"nodes": [{"name": "SECRET_JOB", "webPath": "/SECRET_JOB_URL"}]}}
        ]}}
    })
}

fn response(node: Value) -> Value {
    json!({"data": {"project": {"mergeRequest": node}}})
}

fn read(node: Value) -> MrDetailRead {
    block(Client::over(Replay::new(vec![(200, response(node))])).mr_detail_observed(PROJECT, IID))
        .unwrap()
}

fn effects(node: Value) -> Vec<Effect> {
    observed(&action(URL, PROJECT, IID), &Ok(Box::new(read(node))))
}

fn observations(effects: &[Effect]) -> Vec<(&str, Option<u32>, Outcome, bool)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((
                action.kind.as_str(),
                action.metadata.count,
                *outcome,
                action.metadata.truncated,
            )),
            _ => None,
        })
        .collect()
}

fn component(effects: &[Effect], kind: &str) -> (u32, Outcome) {
    let prefix = format!("gitlab.page.detail.{kind}");
    observations(effects)
        .into_iter()
        .find(|(kind, _, _, _)| *kind == prefix)
        .map(|(_, count, outcome, _)| (count.unwrap(), outcome))
        .unwrap()
}

#[test]
fn public_and_observed_methods_keep_the_exact_request_and_legacy_values() {
    let n = node();
    let plain = Replay::new(vec![(200, response(n.clone()))]);
    let recorded = Replay::new(vec![(200, response(n.clone()))]);
    let old = block(Client::over(plain.clone()).mr_detail(PROJECT, IID)).unwrap();
    let got = block(Client::over(recorded.clone()).mr_detail_observed(PROJECT, IID)).unwrap();
    assert_eq!((&got.mr, &got.detail), (&old.0, &old.1));
    assert_eq!(got.mr, parse_mr(&n, BASE));
    assert_eq!(got.detail, parse_detail(&n, BASE));
    assert_eq!(got.evidence, MrDetailEvidence::default());
    assert_eq!(plain.asked(), recorded.asked());
    let asked = recorded.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].method, Method::Post);
    assert_eq!(asked[0].path, "/api/graphql");
    assert!(asked[0].range.is_none());
    let body = asked[0].body.as_ref().unwrap();
    assert_eq!(body["variables"], json!({"project": PROJECT, "iid": "263"}));
    let query = body["query"].as_str().unwrap();
    for field in [
        "diffStats",
        "commits(first: 100)",
        "discussions(first: 100)",
        "notes(first: 100)",
        "jobs(first: 100)",
        "mergeabilityChecks",
    ] {
        assert!(query.contains(field));
    }
    assert!(!query.contains("pageInfo"));
    let fx = observed(&action(URL, PROJECT, IID), &Ok(Box::new(got)));
    assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
    assert_eq!(observations(&fx).len(), 8);
    assert!(
        observations(&fx)
            .iter()
            .all(|(_, n, out, cut)| *n == Some(1) && *out == Outcome::Succeeded && !cut)
    );
}

#[test]
fn empty_short_full_oversized_and_duplicate_collections_are_occurrences_without_cutoff() {
    for count in [0, 1, 100, 101] {
        let mut n = node();
        n["diffStats"] = json!(vec![n["diffStats"][0].clone(); count]);
        n["commits"]["nodes"] = json!(vec![n["commits"]["nodes"][0].clone(); count]);
        n["discussions"]["nodes"] = json!(vec![n["discussions"]["nodes"][0].clone(); count]);
        n["jobs"]["stages"]["nodes"][0]["jobs"]["nodes"] = json!(vec![
            n["jobs"]["stages"]["nodes"]
                [0]["jobs"]["nodes"][0]
                .clone();
            count
        ]);
        let got = read(n.clone());
        assert_eq!(got.detail, parse_detail(&n, BASE));
        let fx = observed(&action(URL, PROJECT, IID), &Ok(Box::new(got)));
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        for kind in ["files", "commits", "discussions", "notes", "jobs"] {
            assert_eq!(component(&fx, kind), (count as u32, Outcome::Succeeded));
        }
        assert!(observations(&fx).iter().all(|(_, _, _, cut)| !cut));
    }
    let mut n = node();
    n["discussions"]["nodes"][0]["notes"]["nodes"] = json!(vec![
            n["discussions"]["nodes"][0]["notes"]["nodes"][0]
                .clone();
            101
        ]);
    n["jobs"]["stages"]["nodes"] = json!(vec![n["jobs"]["stages"]["nodes"][0].clone(); 101]);
    let fx = effects(n);
    assert_eq!(component(&fx, "notes"), (101, Outcome::Succeeded));
    assert_eq!(component(&fx, "stages"), (101, Outcome::Succeeded));
    assert_eq!(component(&fx, "jobs"), (101, Outcome::Succeeded));
    assert!(observations(&fx).iter().all(|(_, _, _, cut)| !cut));
}

#[test]
fn missing_nonarray_collections_are_partial_without_rejecting_defaulted_parser_results() {
    for (pointer, kind) in [
        ("/diffStats", "files"),
        ("/commits/nodes", "commits"),
        ("/discussions/nodes", "discussions"),
        ("/discussions/nodes/0/notes/nodes", "notes"),
        ("/jobs/stages/nodes", "stages"),
        ("/jobs/stages/nodes/0/jobs/nodes", "jobs"),
    ] {
        for malformed in [Value::Null, json!({}), json!(false), json!("SECRET")] {
            let mut n = node();
            *n.pointer_mut(pointer).unwrap() = malformed;
            let got = read(n.clone());
            assert_eq!(got.detail, parse_detail(&n, BASE));
            let fx = observed(&action(URL, PROJECT, IID), &Ok(Box::new(got)));
            assert!(
                matches!(fx[0], Effect::Report(Outcome::Partial)),
                "{pointer}"
            );
            assert_eq!(component(&fx, kind), (0, Outcome::Partial));
            assert_eq!(
                component(&fx, "responses.incomplete"),
                (1, Outcome::Partial)
            );
            if kind != "files" {
                assert_eq!(component(&fx, "files"), (1, Outcome::Succeeded));
            }
        }
    }
    for field in ["diffStats", "commits", "discussions"] {
        let mut n = node();
        n.as_object_mut().unwrap().remove(field);
        let fx = effects(n);
        assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    }
}

#[test]
fn malformed_rows_are_retained_by_the_same_parser_and_no_descendant_gaps_are_counted() {
    for (pointer, kind) in [
        ("/diffStats/0", "files"),
        ("/commits/nodes/0", "commits"),
        ("/discussions/nodes/0", "discussions"),
        ("/discussions/nodes/0/notes/nodes/0", "notes"),
        ("/jobs/stages/nodes/0", "stages"),
        ("/jobs/stages/nodes/0/jobs/nodes/0", "jobs"),
    ] {
        for malformed in [Value::Null, json!(false), json!(42), json!([])] {
            let mut n = node();
            *n.pointer_mut(pointer).unwrap() = malformed;
            let got = read(n.clone());
            assert_eq!(got.mr, parse_mr(&n, BASE));
            assert_eq!(got.detail, parse_detail(&n, BASE));
            let fx = observed(&action(URL, PROJECT, IID), &Ok(Box::new(got)));
            assert!(
                matches!(fx[0], Effect::Report(Outcome::Partial)),
                "{pointer}"
            );
            assert_eq!(component(&fx, kind).1, Outcome::Partial);
            assert_eq!(
                component(&fx, "responses.incomplete"),
                (1, Outcome::Partial)
            );
            assert_eq!(
                observations(&fx).len(),
                9,
                "one response gap, no invented descendant gap multiplicity"
            );
        }
    }
}

#[test]
fn optional_nulls_are_clean_and_malformed_nonnull_parents_are_partial() {
    for field in ["jobs", "diffRefs"] {
        for absent in [true, false] {
            let mut n = node();
            if absent {
                n.as_object_mut().unwrap().remove(field);
            } else {
                n[field] = Value::Null;
            }
            assert!(matches!(effects(n)[0], Effect::Report(Outcome::Succeeded)));
        }
        for bad in [json!(false), json!(42), json!([]), json!("SECRET")] {
            let mut n = node();
            n[field] = bad;
            let got = read(n.clone());
            assert_eq!(got.detail, parse_detail(&n, BASE));
            let fx = observed(&action(URL, PROJECT, IID), &Ok(Box::new(got)));
            assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
            assert_eq!(component(&fx, "files"), (1, Outcome::Succeeded));
            assert_eq!(
                component(
                    &fx,
                    if field == "jobs" {
                        "pipelines"
                    } else {
                        "merge_requests"
                    }
                )
                .1,
                Outcome::Partial
            );
        }
    }
    for field in ["author", "position"] {
        let mut n = node();
        n["discussions"]["nodes"][0]["notes"]["nodes"][0][field] = Value::Null;
        assert!(matches!(
            effects(n.clone())[0],
            Effect::Report(Outcome::Succeeded)
        ));
        n["discussions"]["nodes"][0]["notes"]["nodes"][0][field] = json!([]);
        assert_eq!(component(&effects(n), "notes"), (1, Outcome::Partial));
    }
    for malformed in [json!(false), json!(42), json!([]), json!("SECRET")] {
        let got = read(malformed.clone());
        assert_eq!(got.mr, parse_mr(&malformed, BASE));
        assert_eq!(got.detail, parse_detail(&malformed, BASE));
        let fx = observed(&action(URL, PROJECT, IID), &Ok(Box::new(got)));
        assert_eq!(component(&fx, "merge_requests"), (1, Outcome::Partial));
        assert_eq!(
            component(&fx, "responses.incomplete"),
            (1, Outcome::Partial)
        );
        assert!(
            observations(&fx)
                .iter()
                .all(|(_, _, outcome, _)| *outcome == Outcome::Partial)
        );
    }
}

#[test]
fn intentional_filters_and_scalar_defaults_do_not_create_false_gaps() {
    let mut n = node();
    n["discussions"]["nodes"] = json!([
        {"id": "SECRET_EMPTY", "notes": {"nodes": []}},
        {"id": "SECRET_SYSTEM", "notes": {"nodes": [{"system": true, "author": false, "position": []}]}},
        {"id": 9, "notes": {"nodes": [{"system": "true", "body": 42}, {}]}}
    ]);
    n["diffStats"] = json!([{}]);
    n["commits"]["nodes"] = json!([{}]);
    n["jobs"]["stages"]["nodes"][0]["jobs"]["nodes"] = json!([{}]);
    n["diffRefs"] = json!({});
    n["mergeabilityChecks"] = Value::Null;
    n["reviewers"] = json!("SECRET_UNOBSERVED_BASE_FIELD");
    let got = read(n.clone());
    assert_eq!(got.detail, parse_detail(&n, BASE));
    let fx = observed(&action(URL, PROJECT, IID), &Ok(Box::new(got)));
    assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
    assert_eq!(component(&fx, "discussions"), (1, Outcome::Succeeded));
    assert_eq!(component(&fx, "notes"), (2, Outcome::Succeeded));
    n["discussions"]["nodes"] =
        json!([{ "notes": {"nodes": []}}, {"notes": {"nodes": [{"system": true}]}}]);
    let fx = effects(n);
    assert_eq!(component(&fx, "discussions"), (0, Outcome::Succeeded));
    assert_eq!(component(&fx, "notes"), (0, Outcome::Succeeded));
}

#[test]
fn compatibility_retry_only_removes_checks_and_final_errors_have_no_progress() {
    let failure = json!({"errors": [{"message": "SECRET mergeabilityChecks unavailable"}]});
    for succeeds in [true, false] {
        let answers = vec![
            (200, failure.clone()),
            (
                200,
                if succeeds {
                    response(node())
                } else {
                    failure.clone()
                },
            ),
        ];
        let plain = Replay::new(answers.clone());
        let replay = Replay::new(answers);
        let old = block(Client::over(plain.clone()).mr_detail(PROJECT, IID));
        let got = block(Client::over(replay.clone()).mr_detail_observed(PROJECT, IID));
        assert_eq!(old.is_ok(), succeeds);
        assert_eq!(got.is_ok(), succeeds);
        assert_eq!(plain.asked(), replay.asked());
        let asked = replay.asked();
        assert_eq!(asked.len(), 2);
        let first = asked[0].body.as_ref().unwrap();
        let second = asked[1].body.as_ref().unwrap();
        assert_eq!(first["variables"], second["variables"]);
        assert_eq!(
            first["query"]
                .as_str()
                .unwrap()
                .replace("mergeabilityChecks { identifier status }", ""),
            second["query"]
        );
        let result = got.map(Box::new).map_err(|e| format!("{e:#}"));
        let fx = observed(&action(URL, PROJECT, IID), &result);
        if !succeeds {
            assert_eq!(
                observations(&fx),
                [("gitlab.page.detail.errors", Some(1), Outcome::Failed, false)]
            );
        }
    }
}

struct Tokens {
    renewable: bool,
    renewals: std::sync::atomic::AtomicUsize,
}
impl crate::auth::Tokens for Tokens {
    fn token(&self) -> anyhow::Result<String> {
        Ok("SECRET_TOKEN".into())
    }
    fn renew(&self) -> anyhow::Result<bool> {
        self.renewals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.renewable)
    }
}

#[test]
fn renewable_rejection_replays_the_same_operation_only_once_without_real_auth() {
    for (renewable, statuses) in [(true, [401, 200]), (true, [401, 401]), (false, [401, 200])] {
        let replay = Replay::new(
            statuses
                .into_iter()
                .map(|status| (status, response(node())))
                .collect(),
        );
        let tokens = Tokens {
            renewable,
            renewals: Default::default(),
        };
        let got = block(crate::auth::with_token(&tokens, |_| {
            let client = Client::over(replay.clone());
            async move { client.mr_detail_observed(PROJECT, IID).await }
        }));
        assert_eq!(got.is_ok(), renewable && statuses[1] == 200);
        assert_eq!(tokens.renewals.load(std::sync::atomic::Ordering::SeqCst), 1);
        let asked = replay.asked();
        assert_eq!(asked.len(), if renewable { 2 } else { 1 });
        if renewable {
            assert_eq!(asked[0], asked[1]);
        }
    }
}

#[test]
fn existing_graphql_errors_nulls_json_and_http_status_behavior_are_unchanged() {
    for body in [
        json!({}),
        response(Value::Null),
        json!({"data": {"project": null}}),
        json!({"errors": [{"message": "SECRET_ERROR"}], "data": response(node())["data"]}),
    ] {
        let replay = Replay::new(vec![(200, body)]);
        let result = block(Client::over(replay.clone()).mr_detail_observed(PROJECT, IID))
            .map(Box::new)
            .map_err(|e| e.to_string());
        assert!(result.is_err());
        assert_eq!(replay.asked().len(), 1);
        assert_eq!(
            observations(&observed(&action(URL, PROJECT, IID), &result)),
            [("gitlab.page.detail.errors", Some(1), Outcome::Failed, false)]
        );
    }
    for status in [200, 403, 404, 429, 500] {
        let replay = Replay::new(vec![(status, response(node()))]);
        assert!(
            block(Client::over(replay.clone()).mr_detail_observed(PROJECT, IID)).is_ok(),
            "legacy accepts usable GraphQL data at {status}"
        );
        assert_eq!(replay.asked().len(), 1);
    }
    for raw in [b"SECRET_INVALID_JSON".to_vec(), vec![]] {
        let replay = Replay::raw(vec![Resp {
            status: 200,
            body: raw,
            ..Default::default()
        }]);
        assert!(block(Client::over(replay.clone()).mr_detail_observed(PROJECT, IID)).is_err());
        assert_eq!(replay.asked().len(), 1);
    }
    assert!(block(Client::over(Replay::new(vec![])).mr_detail_observed(PROJECT, IID)).is_err());
}

#[test]
fn requested_identity_is_canonical_shared_with_diff_and_never_uses_returned_data() {
    let scope = action(URL, PROJECT, IID);
    assert_eq!(
        scope.subjects,
        crate::diff_history::action(URL, PROJECT, IID, 1).subjects
    );
    assert_eq!(scope.subjects[0].key.instance, "https://git.example/");
    assert_eq!(scope.subjects[0].key.id, "acme/team-a/api!263");
    assert_eq!(scope.subjects[0].role, SubjectRole::Primary);
    assert!(
        scope.metadata.count.is_none()
            && scope.metadata.url.is_none()
            && scope.subjects[0].url.is_none()
    );
    for (url, project, iid) in [
        ("no URL", PROJECT, IID),
        ("file:///SECRET", PROJECT, IID),
        (URL, "acme/../api", IID),
        (URL, "acme/./api", IID),
        (URL, "acme?token=SECRET", IID),
        (URL, PROJECT, 0),
    ] {
        let scope = action(url, project, iid);
        assert!(scope.subjects.is_empty() && scope.metadata.redacted);
        assert_eq!(scope.metadata.omitted_subjects, 1);
    }
    let mut n = node();
    n["project"]["fullPath"] = json!("SECRET_RETURNED_PROJECT");
    n["iid"] = json!(999);
    let fx = effects(n);
    for effect in &fx {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects, scope.subjects);
        }
    }
    assert!(!format!("{fx:?}").contains("SECRET"));
    let saturated = count(
        &scope,
        "gitlab.page.detail.files",
        usize::MAX,
        Outcome::Succeeded,
    );
    if let Effect::Observe { action, .. } = saturated {
        assert_eq!(action.metadata.count, Some(u32::MAX));
        assert_eq!(action.metadata.truncated, usize::BITS > 32);
    }
}

fn kind() -> pages::Kind {
    pages::Kind::Mr {
        project: PROJECT.into(),
        iid: IID,
    }
}
fn key() -> String {
    pages::mr_key(PROJECT, IID)
}

fn page_plugin() -> GitlabPlugin {
    let mut plugin = demo::sample();
    plugin.cfg.url = URL.into();
    // Production tasks here are deliberately never polled.
    pages::push(&mut plugin, kind());
    let got = read(node());
    pages::on_mr_page(&mut plugin, key(), Ok((got.mr, got.detail)));
    plugin
}

#[test]
fn actual_scoped_tasks_keep_both_original_servers_during_reversed_completions() {
    let mut plugin = page_plugin();
    let mut pending = Vec::new();
    for (url, title) in [
        ("https://first.example", "SECRET_OLD_TITLE"),
        ("https://second.example", "SECRET_NEW_TITLE"),
    ] {
        let mut n = node();
        n["title"] = json!(title);
        let client = Client::over(Replay::new(vec![(200, response(n))]));
        let owner = crate::page_ownership::next(&mut plugin);
        plugin.mr_page_reads.insert(key(), owner);
        pending.push(spawn(
            action(url, PROJECT, IID),
            key(),
            owner,
            crate::page_identity::Requested::new(PROJECT, IID),
            async move {
                client
                    .mr_detail_observed(PROJECT, IID)
                    .await
                    .map_err(|e| e.to_string())
            },
        ));
    }
    let source = plugin.snapshot();
    plugin.replace_connection(
        "https://third.example".into(),
        "env:SECRET_OTHER_TOKEN".into(),
    );
    plugin.now += 1;
    for (effect, url, _title) in pending
        .into_iter()
        .rev()
        .zip(["https://second.example/", "https://first.example/"])
        .zip(["SECRET_NEW_TITLE", "SECRET_OLD_TITLE"])
        .map(|((a, b), c)| (a, b, c))
    {
        let Effect::SpawnAction { action, task } = effect else {
            panic!("scoped detail task")
        };
        assert_eq!(action.subjects[0].key.instance, url);
        let fx = plugin.on_msg(block(task));
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        for effect in &fx {
            if let Effect::Observe { action, .. } = effect {
                assert_eq!(action.subjects[0].key.instance, url);
                assert_eq!(action.subjects[0].key.id, key());
            }
        }
        assert!(plugin.mr_pages.is_empty());
        assert!(plugin.mr_page_reads.is_empty());
        assert_eq!(plugin.snapshot(), source);
        assert!(
            matches!(fx.last(), Some(Effect::Observe { action, outcome: Outcome::Blocked })
            if action.kind.as_str() == "gitlab.page.detail.application.skipped")
        );
    }
    let snapshot = plugin.snapshot().unwrap().to_string();
    for transient in [
        "gitlab.page.detail",
        "MrDetailEvidence",
        "files_incomplete",
        "notes_incomplete",
    ] {
        assert!(!snapshot.contains(transient));
    }
}

#[test]
fn original_report_precedes_unchanged_application_tree_cache_error_and_follow_on_effects() {
    for mode in [0, 1, 2] {
        let mut observed_plugin = page_plugin();
        let mut legacy_plugin = page_plugin();
        observed_plugin.pages.last_mut().unwrap().tab = 1;
        legacy_plugin.pages.last_mut().unwrap().tab = 1;
        observed_plugin.now += 10;
        legacy_plugin.now += 10;
        observed_plugin.mr_pages.get_mut(&key()).unwrap().loading = true;
        legacy_plugin.mr_pages.get_mut(&key()).unwrap().loading = true;
        let mut n = node();
        n["title"] = json!("SECRET_NEW_TITLE");
        n["headPipeline"] = json!({"status": "FAILED", "path": "/acme/team-a/api/-/pipelines/902"});
        if mode == 1 {
            n["commits"] = Value::Null;
        }
        let got = read(n);
        let plain = if mode == 2 {
            Err("SECRET_ERROR".into())
        } else {
            Ok((got.mr.clone(), got.detail.clone()))
        };
        let expected = pages::on_mr_page(&mut legacy_plugin, key(), plain);
        let result = if mode == 2 {
            Err("SECRET_ERROR".into())
        } else {
            Ok(Box::new(got))
        };
        let actual = observed_plugin.on_msg(Box::new(GlMsg::MrPage {
            key: key(),
            read: observed_plugin.mr_page_reads[&key()],
            requested: crate::page_identity::Requested::new(PROJECT, IID),
            action: action(URL, PROJECT, IID),
            result,
        }));
        let outcome = [Outcome::Succeeded, Outcome::Partial, Outcome::Failed][mode];
        assert!(matches!(actual[0], Effect::Report(got) if got == outcome));
        let application: Vec<_> = actual
            .into_iter()
            .filter(|e| !matches!(e, Effect::Report(_) | Effect::Observe { .. }))
            .collect();
        assert_eq!(format!("{application:?}"), format!("{expected:?}"));
        assert_eq!(
            format!("{:?}", observed_plugin.mr_pages),
            format!("{:?}", legacy_plugin.mr_pages)
        );
        assert_eq!(
            format!("{:?}", observed_plugin.diffs),
            format!("{:?}", legacy_plugin.diffs)
        );
        assert_eq!(
            format!("{:?}", observed_plugin.failed),
            format!("{:?}", legacy_plugin.failed)
        );
        assert_eq!(observed_plugin.snapshot(), legacy_plugin.snapshot());
        assert!(
            !application
                .iter()
                .any(|e| matches!(e, Effect::PersistCache)),
            "detail has no new cache persistence"
        );
    }
}

#[test]
fn returned_identity_cannot_contaminate_requested_page_or_another_tree_row() {
    let mut plugin = page_plugin();
    let other = plugin
        .data
        .values()
        .flat_map(|data| &data.mrs)
        .find(|mr| mr.iid != IID)
        .unwrap()
        .clone();
    let original = pages::find_mr(&plugin, PROJECT, IID).unwrap().clone();
    let held = plugin.mr_pages[&key()].value.clone();
    let at = plugin.mr_pages[&key()].at;
    let mut n = node();
    n["project"]["fullPath"] = json!(other.project_path);
    n["iid"] = json!(other.iid);
    n["title"] = json!("SECRET_RETURNED_TITLE");
    let fx = plugin.on_msg(Box::new(GlMsg::MrPage {
        key: key(),
        read: plugin.mr_page_reads[&key()],
        requested: crate::page_identity::Requested::new(PROJECT, IID),
        action: action(URL, PROJECT, IID),
        result: Ok(Box::new(read(n))),
    }));
    assert_eq!(pages::find_mr(&plugin, PROJECT, IID).unwrap(), &original);
    assert_eq!(
        pages::find_mr(&plugin, &other.project_path, other.iid).unwrap(),
        &other
    );
    assert_eq!(plugin.mr_pages[&key()].value, held);
    assert_eq!(plugin.mr_pages[&key()].at, at);
    assert!(!plugin.mr_pages[&key()].loading);
    assert_eq!(
        plugin.mr_pages[&key()].error.as_deref(),
        Some(crate::page_identity::REJECTED)
    );
    assert!(
        matches!(fx.last(), Some(Effect::Observe { action, outcome: Outcome::Blocked })
        if action.kind.as_str() == "gitlab.page.detail.application.identity.rejected")
    );
    for effect in fx {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects[0].key.id, key());
        }
    }
}

#[test]
fn fresh_and_busy_noops_forced_overlap_independent_red_diagnosis_and_ticks_are_preserved() {
    let mut plugin = page_plugin();
    assert!(without_open(pages::push(&mut plugin, kind())).is_empty());
    let fetched = plugin.mr_pages.get_mut(&key()).unwrap();
    fetched.value = None;
    fetched.loading = true;
    assert!(without_open(pages::push(&mut plugin, kind())).is_empty());
    for _ in 0..2 {
        let radar_core::plugin::Outcome::Handled(fx) = pages::key(
            &mut plugin,
            Pane::Middle,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        ) else {
            panic!("refresh handled")
        };
        assert!(
            matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.kind.as_str() == "gitlab.page.detail"),
            "forced overlap remains dispatched: {fx:?}"
        );
    }
    let mut n = node();
    n["headPipeline"] = json!({"status": "FAILED", "path": "/acme/team-a/api/-/pipelines/901"});
    let got = read(n);
    pages::on_mr_page(&mut plugin, key(), Ok((got.mr, got.detail)));
    plugin.failed.clear();
    let fx = without_open(pages::push(&mut plugin, kind()));
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.kind.as_str() == "gitlab.pipeline.diagnosis"),
        "fresh detail still allows its independent red diagnosis: {fx:?}"
    );
    assert!(
        without_open(pages::push(&mut plugin, kind())).is_empty(),
        "both now busy/fresh"
    );
    let now = plugin.now + plugin.cfg.refresh_secs as i64 + 1;
    let fx = plugin.tick(now);
    assert!(!fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.detail")), "source ticks do not acquire detail reads");
}

fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Journal(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}
fn tasks(effects: &[HostEffect]) -> Vec<(ActionId, Option<ActionId>)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } if matches!(&**effect, HostEffect::Spawn { .. }) => {
                Some((action.clone(), semantic.clone()))
            }
            _ => None,
        })
        .collect()
}
fn descriptor(records: &[JournalRecord], id: &ActionId) -> ActionDescriptor {
    records
        .iter()
        .find_map(|record| match record {
            JournalRecord::Begin {
                id: got,
                descriptor,
                ..
            } if got == id => Some(descriptor.clone()),
            _ => None,
        })
        .unwrap()
}
fn terminal(records: &[JournalRecord], id: &ActionId) -> Vec<Outcome> {
    records
        .iter()
        .filter_map(|record| match record {
            JournalRecord::Outcome {
                id: got, outcome, ..
            } if got == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}
fn deliver(host: &mut Host, task: &(ActionId, Option<ActionId>), msg: Msg) -> Vec<HostEffect> {
    host.update(Event::Tracked {
        action: task.0.clone(),
        semantic: task.1.clone(),
        event: Box::new(Event::Msg { plugin: 0, msg }),
    })
}

enum Begin {
    Write,
}
type Captured = std::sync::Arc<std::sync::Mutex<[Option<crate::page_ownership::Read>; 4]>>;
struct Reads(GitlabPlugin, Captured);
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
        let effects = match msg.downcast::<Begin>() {
            Ok(begin) => match *begin {
                Begin::Write => {
                    let mut effects = vec![Effect::Journal(ActionDescriptor::new(
                        "gitlab.test.write",
                        Origin::Plugin("gitlab".into()),
                    ))];
                    let connection = self.0.connection_stamp();
                    effects.extend(pages::act::on_msg(
                        &mut self.0,
                        Box::new(crate::mr_actions::ActMsg {
                            connection,
                            key: key(),
                            what: "rebase",
                            result: Ok("SECRET_SUCCESS".into()),
                            outcome: Outcome::Succeeded,
                        }),
                    ));
                    effects
                }
            },
            Err(msg) => self.0.on_msg(msg),
        };
        let mut captured = self.1.lock().unwrap();
        if let Some(read) = self.0.mr_page_reads.get(&key()) {
            captured[0] = Some(*read);
        }
        if let Some(read) = self.0.diffs.get(&key()).and_then(|d| d.requests.get(&1)) {
            captured[1] = Some(*read);
        }
        for (slot, pipeline) in [(2, 901), (3, 902)] {
            if let Some(read) = self.0.failed_reads.get(&crate::why::key(PROJECT, pipeline)) {
                captured[slot] = Some(*read);
            }
        }
        effects
    }
}

#[test]
fn real_host_keeps_write_detail_diagnosis_diff_and_source_outcomes_independent_and_private() {
    for expected in [Outcome::Succeeded, Outcome::Partial, Outcome::Failed] {
        let mut plugin = page_plugin();
        plugin.pages.last_mut().unwrap().tab = 1;
        // Known-red diagnosis is a sibling before the independently scoped detail.
        for data in plugin.data.values_mut() {
            if let Some(mr) = data
                .mrs
                .iter_mut()
                .find(|mr| mr.project_path == PROJECT && mr.iid == IID)
            {
                mr.pipeline = Some(crate::model::Pipeline::Failed);
                mr.pipeline_url = Some(format!("{BASE}/{PROJECT}/-/pipelines/901"));
            }
        }
        let captured = Captured::default();
        let mut host = Host::new(vec![Box::new(Reads(plugin, captured.clone()))], 1).unwrap();
        host.enable_journal();
        let initial = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Begin::Write),
        });
        let pending = tasks(&initial);
        let mut all = records(&initial);
        let write = all
            .iter()
            .find_map(|record| match record {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == "gitlab.test.write" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(terminal(&all, &write), [Outcome::Succeeded]);
        let detail = pending
            .iter()
            .find(|(_, semantic)| {
                semantic
                    .as_ref()
                    .is_some_and(|id| descriptor(&all, id).kind.as_str() == "gitlab.page.detail")
            })
            .unwrap()
            .clone();
        let detail_id = detail.1.as_ref().unwrap().clone();
        let scope = descriptor(&all, &detail_id);
        let source = pending
            .iter()
            .find(|(_, semantic)| {
                semantic.as_ref().is_some_and(|id| {
                    descriptor(&all, id)
                        .kind
                        .as_str()
                        .starts_with("gitlab.sync.")
                })
            })
            .unwrap()
            .clone();
        let source_scope = descriptor(&all, source.1.as_ref().unwrap());
        let diagnosis = pending
            .iter()
            .find(|(_, semantic)| semantic.as_ref().is_some_and(|id| {
                descriptor(&all, id).kind.as_str() == "gitlab.pipeline.diagnosis"
            }))
            .unwrap();
        let diagnosis_scope = descriptor(&all, diagnosis.1.as_ref().unwrap());
        let diagnosis_read = captured.lock().unwrap()[2].unwrap();
        let done = deliver(
            &mut host,
            diagnosis,
            Box::new(GlMsg::Failed {
                request: crate::log_ownership::PipelineRead {
                    project: PROJECT.into(), pipeline: 901,
                    owner: diagnosis_read, activity: 0,
                },
                action: diagnosis_scope,
                read: crate::why::DiagnosisRead {
                    result: Err("SECRET_DIAGNOSIS_ERROR".into()),
                    logs: Some(vec![]),
                },
            }),
        );
        let done_records = records(&done);
        assert!(
            terminal(&done_records, &write).is_empty()
                && terminal(&done_records, &detail_id).is_empty()
        );
        all.extend(done_records);
        let mut n = node();
        n["headPipeline"] = json!({"status": "FAILED", "path": "/acme/team-a/api/-/pipelines/902"});
        if expected == Outcome::Partial {
            n["commits"] = Value::Null;
        }
        let result = if expected == Outcome::Failed {
            Err("SECRET_DETAIL_ERROR".into())
        } else {
            Ok(Box::new(read(n)))
        };
        let detail_read = captured.lock().unwrap()[0].unwrap();
        let reply = deliver(
            &mut host,
            &detail,
            Box::new(GlMsg::MrPage {
                key: key(),
                read: detail_read,
                requested: crate::page_identity::Requested::new(PROJECT, IID),
                action: scope.clone(),
                result,
            }),
        );
        let detail_records = records(&reply);
        assert_eq!(terminal(&detail_records, &detail_id), [expected]);
        assert!(terminal(&detail_records, &write).is_empty());
        for record in &detail_records {
            if let JournalRecord::Begin {
                descriptor, parent, ..
            } = record
                && descriptor.kind.as_str().starts_with("gitlab.page.detail.")
            {
                assert_eq!(parent.as_ref(), Some(&detail.0));
                assert_eq!(descriptor.subjects, scope.subjects);
                assert_eq!(descriptor.subjects[0].key.instance, "https://git.example/");
            }
        }
        all.extend(detail_records);
        let source_reply = deliver(
            &mut host,
            &source,
            Box::new(GlMsg::Loaded {
                group: source_scope.subjects[0].key.id.clone(),
                want: crate::client::Want::default(),
                action: source_scope,
                result: Err("SECRET_SOURCE_ERROR".into()),
                net: Default::default(),
            }),
        );
        let source_records = records(&source_reply);
        assert_eq!(
            terminal(&source_records, source.1.as_ref().unwrap()),
            [Outcome::Failed]
        );
        assert!(
            terminal(&source_records, &detail_id).is_empty()
                && terminal(&source_records, &write).is_empty()
        );
        all.extend(source_records);
        let follow = tasks(&reply);
        let diff = follow
            .iter()
            .find(|(_, semantic)| {
                semantic
                    .as_ref()
                    .is_some_and(|id| descriptor(&all, id).kind.as_str() == "gitlab.page.diffs")
            })
            .unwrap()
            .clone();
        let diff_id = diff.1.as_ref().unwrap();
        assert_ne!(diff_id, &detail_id);
        let diff_scope = descriptor(&all, diff_id);
        assert_eq!(diff_scope.subjects[0].key.instance, "https://git.example/");
        let diff_read = captured.lock().unwrap()[1].unwrap();
        let diff_reply = deliver(
            &mut host,
            &diff,
            Box::new(GlMsg::Diffs {
                key: key(),
                read: diff_read,
                head: "SECRET_HEAD".into(),
                page: 1,
                action: diff_scope,
                result: Err("SECRET_DIFF_ERROR".into()),
            }),
        );
        let diff_records = records(&diff_reply);
        assert_eq!(terminal(&diff_records, diff_id), [Outcome::Failed]);
        assert!(
            terminal(&diff_records, &detail_id).is_empty()
                && terminal(&diff_records, &write).is_empty()
        );
        all.extend(diff_records);
        if expected != Outcome::Failed {
            let diagnosis = follow
                .iter()
                .find(|(_, semantic)| semantic.as_ref().is_some_and(|id| {
                    descriptor(&all, id).kind.as_str() == "gitlab.pipeline.diagnosis"
                }))
                .unwrap();
            let diagnosis_scope = descriptor(&all, diagnosis.1.as_ref().unwrap());
            let diagnosis_read = captured.lock().unwrap()[3].unwrap();
            let reply = deliver(
                &mut host,
                diagnosis,
                Box::new(GlMsg::Failed {
                    request: crate::log_ownership::PipelineRead {
                    project: PROJECT.into(), pipeline: 902,
                    owner: diagnosis_read, activity: 0,
                },
                    action: diagnosis_scope,
                    read: crate::why::DiagnosisRead {
                        result: Err("SECRET_NEW_DIAGNOSIS_ERROR".into()),
                        logs: Some(vec![]),
                    },
                }),
            );
            let records = records(&reply);
            assert!(
                terminal(&records, &detail_id).is_empty() && terminal(&records, &write).is_empty()
            );
            all.extend(records);
        }
        // A second actual forced refresh remains unreturned and becomes Unknown.
        let unfinished = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Begin::Write),
        });
        all.extend(records(&unfinished));
        let dir = std::env::temp_dir().join(format!(
            "radar-gitlab-detail-history-{}",
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
                JournalRecord::Gap { .. } => panic!("unexpected gap"),
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
        assert!(!exported.contains("SECRET"), "{exported}");
        assert!(
            events
                .iter()
                .any(|e| e.kind.as_str() == "gitlab.page.detail" && e.outcome == Outcome::Unknown)
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.action_id == detail_id && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [expected]
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.action_id == write && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [Outcome::Succeeded]
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind.as_str() == "gitlab.page.detail.merge_requests"
                    && e.outcome.is_terminal())
                .count(),
            usize::from(expected != Outcome::Failed)
        );
        drop(journal);
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn without_open(effects: Vec<Effect>) -> Vec<Effect> {
    assert_eq!(effects.iter().filter(|e| matches!(e, Effect::Observe { action, outcome: Outcome::Succeeded } if action.kind.as_str() == "gitlab.page.open")).count(), 1);
    effects.into_iter().filter(|e| !matches!(e, Effect::Observe { action, .. } if action.kind.as_str() == "gitlab.page.open")).collect()
}
