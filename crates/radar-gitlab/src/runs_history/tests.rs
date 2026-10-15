use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, Method, Resp, RunsEvidence, parse_runs};
use crate::{GitlabPlugin, demo, pages};
use radar_core::Plugin;
use radar_core::plugin::Pane;
use radar_core::ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{Value, json};

const PROJECT: &str = "acme/team-a/api";
const BASE: &str = "https://gitlab.example.com";
const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";

fn node() -> Value {
    json!({"iid": "SECRET_IID", "status": "SUCCESS", "createdAt": "SECRET_DATE",
        "duration": 23, "path": "/SECRET_PATH", "source": "SECRET_SOURCE",
        "user": {"name": "SECRET_NAME", "username": "SECRET_USERNAME"},
        "commit": {"shortId": "SECRET_SHA", "title": "SECRET_TITLE"}})
}
fn response(conn: Value) -> Value {
    json!({"data": {"project": {"pipelines": conn}}})
}
fn read(conn: Value) -> RunsRead {
    block(Client::over(Replay::new(vec![(200, response(conn))])).runs_observed(PROJECT, "main"))
        .unwrap()
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
fn effects(conn: Value) -> Vec<Effect> {
    observed(&action(URL, PROJECT, "main"), &Ok(read(conn)))
}
fn scoped(effects: &[Effect]) -> &ActionDescriptor {
    effects
        .iter()
        .find_map(|effect| match effect {
            Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.runs" => {
                Some(action)
            }
            _ => None,
        })
        .unwrap()
}

#[test]
fn public_and_observed_reads_keep_exact_query_variables_and_legacy_payload() {
    let conn = json!({"nodes": [node()]});
    let plain = Replay::new(vec![(200, response(conn.clone()))]);
    let replay = Replay::new(vec![(200, response(conn.clone()))]);
    let old = block(Client::over(plain.clone()).runs(PROJECT, "feature/A")).unwrap();
    let got = block(Client::over(replay.clone()).runs_observed(PROJECT, "feature/A")).unwrap();
    assert_eq!(old, got.runs);
    assert_eq!(got.runs, parse_runs(&conn, BASE));
    assert_eq!(got.evidence, RunsEvidence::default());
    assert_eq!(plain.asked(), replay.asked());
    let asked = replay.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].method, Method::Post);
    assert_eq!(asked[0].path, "/api/graphql");
    assert!(asked[0].range.is_none());
    let body = asked[0].body.as_ref().unwrap();
    assert_eq!(
        body["variables"],
        json!({"project": PROJECT, "ref": "feature/A"})
    );
    assert_eq!(
        body["query"],
        r#"
query($project: ID!, $ref: String) {
  project(fullPath: $project) {
    pipelines(ref: $ref, first: 10) {
      nodes {
        iid status createdAt duration path source
        user { name username }
        commit { shortId title }
      }
    }
  }
}"#
    );
}

#[test]
fn explicit_empty_short_full_duplicate_and_over_limit_arrays_are_not_cutoff_evidence() {
    for n in [0, 1, 9, 10, 11] {
        let fx = effects(json!({"nodes": vec![node(); n]}));
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        assert_eq!(
            observations(&fx),
            [(
                "gitlab.page.runs.entries",
                Some(n as u32),
                Outcome::Succeeded,
                false
            )]
        );
    }
}

#[test]
fn missing_null_or_nonarray_collection_is_partial_without_invented_rows() {
    for conn in [
        Value::Null,
        json!({}),
        json!(false),
        json!([]),
        json!(42),
        json!({"nodes": null}),
        json!({"nodes": false}),
        json!({"nodes": {}}),
        json!({"nodes": "SECRET"}),
    ] {
        let got = read(conn.clone());
        assert_eq!(got.runs, parse_runs(&conn, BASE));
        assert!(got.evidence.incomplete_response);
        assert_eq!(got.evidence.incomplete_rows, 0);
        let fx = observed(&action(URL, PROJECT, "main"), &Ok(got));
        assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
        assert_eq!(
            observations(&fx),
            [
                ("gitlab.page.runs.entries", Some(0), Outcome::Partial, false),
                (
                    "gitlab.page.runs.responses.incomplete",
                    Some(1),
                    Outcome::Partial,
                    false
                ),
            ]
        );
    }
}

#[test]
fn nonobject_rows_are_counted_once_and_retained_by_the_permissive_parser() {
    let conn = json!({"nodes": [node(), null, false, 42, [], "SECRET_ROW", {}]});
    let got = read(conn.clone());
    assert_eq!(got.runs, parse_runs(&conn, BASE));
    assert_eq!(
        got.evidence,
        RunsEvidence {
            incomplete_response: false,
            incomplete_rows: 5
        }
    );
    let fx = observed(&action(URL, PROJECT, "main"), &Ok(got));
    assert_eq!(
        observations(&fx),
        [
            ("gitlab.page.runs.entries", Some(7), Outcome::Partial, false),
            (
                "gitlab.page.runs.rows.incomplete",
                Some(5),
                Outcome::Partial,
                false
            ),
        ]
    );
}

#[test]
fn object_rows_with_missing_or_permissive_fields_are_not_new_validation_failures() {
    let conn = json!({"nodes": [{}, {"iid": 42, "status": "UNKNOWN", "duration": -1,
        "createdAt": [], "source": false, "path": 42, "user": false, "commit": []},
        {"user": {"name": "", "username": "SECRET_USERNAME"}, "commit": {"title": "a &amp; b"}}]});
    let got = read(conn.clone());
    assert_eq!(got.runs, parse_runs(&conn, BASE));
    assert_eq!(got.evidence, RunsEvidence::default());
    assert_eq!(got.runs[2].commit_title, "a & b");
    assert!(matches!(
        observed(&action(URL, PROJECT, "main"), &Ok(got))[0],
        Effect::Report(Outcome::Succeeded)
    ));
}

#[test]
fn errors_and_nonnull_malformed_projects_preserve_legacy_graphql_semantics() {
    for body in [
        json!({}),
        json!({"data": {"project": null}}),
        json!({"errors": [{"message": "SECRET_ERROR"}], "data": response(json!({"nodes": []}))["data"]}),
    ] {
        let plain = Replay::new(vec![(200, body.clone())]);
        let replay = Replay::new(vec![(200, body)]);
        let old = block(Client::over(plain.clone()).runs(PROJECT, "main")).unwrap_err();
        let result = block(Client::over(replay.clone()).runs_observed(PROJECT, "main"))
            .map_err(|e| e.to_string());
        assert_eq!(result.as_ref().unwrap_err(), &old.to_string());
        assert_eq!(plain.asked(), replay.asked());
        assert_eq!(
            observations(&observed(&action(URL, PROJECT, "main"), &result)),
            [("gitlab.page.runs.errors", Some(1), Outcome::Failed, false)]
        );
    }
    for project in [
        json!(false),
        json!(42),
        json!([]),
        json!("SECRET_PROJECT"),
        json!({}),
    ] {
        let body = json!({"data": {"project": project}});
        let got =
            block(Client::over(Replay::new(vec![(200, body)])).runs_observed(PROJECT, "main"))
                .unwrap();
        assert!(got.runs.is_empty() && got.evidence.incomplete_response);
    }
    for status in [200, 403, 404, 429, 500] {
        assert!(
            block(
                Client::over(Replay::new(vec![(status, response(json!({"nodes": []})))]))
                    .runs_observed(PROJECT, "main")
            )
            .is_ok()
        );
    }
    for raw in [vec![], b"SECRET_INVALID_JSON".to_vec()] {
        let result = block(
            Client::over(Replay::raw(vec![Resp {
                status: 200,
                body: raw,
                ..Default::default()
            }]))
            .runs_observed(PROJECT, "main"),
        );
        assert!(result.is_err());
    }
    assert!(block(Client::over(Replay::new(vec![])).runs_observed(PROJECT, "main")).is_err());
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
fn renewable_rejection_replays_only_the_existing_operation_once_without_real_auth() {
    for (renewable, statuses) in [(true, [401, 200]), (true, [401, 401]), (false, [401, 200])] {
        let replay = Replay::new(
            statuses
                .into_iter()
                .map(|status| (status, response(json!({"nodes": [node()]}))))
                .collect(),
        );
        let tokens = Tokens {
            renewable,
            renewals: Default::default(),
        };
        let got = block(crate::auth::with_token(&tokens, |_| {
            let client = Client::over(replay.clone());
            async move { client.runs_observed(PROJECT, "feature/a").await }
        }));
        assert_eq!(got.is_ok(), renewable && statuses[1] == 200);
        assert_eq!(tokens.renewals.load(std::sync::atomic::Ordering::SeqCst), 1);
        let asked = replay.asked();
        assert_eq!(asked.len(), if renewable { 2 } else { 1 });
        if renewable {
            assert_eq!(asked[0], asked[1]);
        }
        if got.is_err() {
            assert_eq!(
                observations(&observed(
                    &action(URL, PROJECT, "feature/a"),
                    &got.map_err(|e| e.to_string())
                )),
                [("gitlab.page.runs.errors", Some(1), Outcome::Failed, false)]
            );
        }
    }
}

#[test]
fn exact_requested_repo_and_ref_are_scoped_without_normalization_or_result_identity() {
    let scope = action(URL, PROJECT, "feature/A");
    assert_eq!(scope.subjects.len(), 2);
    let repo = &scope.subjects[0];
    let target = &scope.subjects[1];
    assert_eq!(
        (
            &*repo.key.provider,
            &*repo.key.instance,
            &*repo.key.kind,
            &*repo.key.id
        ),
        ("gitlab", "https://git.example/", "repo", PROJECT)
    );
    assert_eq!(repo.role, SubjectRole::Primary);
    assert_eq!(
        (&*target.key.kind, &*target.key.id),
        ("branch", "acme/team-a/api:feature/A")
    );
    assert_eq!(target.role, SubjectRole::Context);
    assert_eq!(repo.key.instance, target.key.instance);
    assert!(scope.metadata.count.is_none() && scope.metadata.url.is_none());
    for (url, project, branch) in [
        (URL, PROJECT, "feature/a"),
        (URL, PROJECT, "refs/heads/feature/A"),
        (URL, "acme/team-b/api", "feature/A"),
        ("https://other.example", PROJECT, "feature/A"),
    ] {
        assert_ne!(target.key, action(url, project, branch).subjects[1].key);
    }
    let fx = observed(&scope, &Ok(read(json!({"nodes": [node()]}))));
    for effect in &fx {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects, scope.subjects);
            assert!(
                action
                    .subjects
                    .iter()
                    .all(|s| s.role != SubjectRole::Result && s.url.is_none())
            );
        }
    }
    assert!(!format!("{fx:?}").contains("SECRET"));
}

#[test]
fn unsafe_servers_projects_refs_and_over_budget_tuples_are_explicitly_omitted() {
    for (url, project) in [
        ("no URL", PROJECT),
        ("file:///SECRET", PROJECT),
        ("https://git.example/token=SECRET", PROJECT),
        ("https://git.example/%74oken%3DSECRET", PROJECT),
        (URL, ""),
        (URL, "acme/../api"),
        (URL, "acme/./api"),
        (URL, "acme//api"),
        (URL, "acme:api"),
        (URL, "acme/é"),
        (URL, "token=SECRET"),
    ] {
        let scope = action(url, project, "main");
        assert!(scope.subjects.is_empty() && scope.metadata.redacted);
        assert_eq!(scope.metadata.omitted_subjects, 2);
    }
    for branch in [
        "",
        " feature/a",
        "feature/a ",
        "é",
        "feature:a",
        "../a",
        "a..b",
        ".hidden",
        "-option",
        "/main",
        "main/",
        "a//b",
        "a/.b",
        "a.lock/b",
        "a.",
        "a./b",
        "@",
        "a@{b}",
        "feature?a",
        "a#b",
        "a\\b",
        "a~b",
        "a^b",
        "a*b",
        "a[b",
        "a!b",
        "token=SECRET",
        "a%20b",
        "a\nb",
    ] {
        let scope = action(URL, PROJECT, branch);
        assert_eq!(scope.subjects.len(), 1, "{branch:?}");
        assert!(scope.metadata.redacted);
        assert_eq!(scope.metadata.omitted_subjects, 1);
    }
    let project = "a".repeat(2048);
    assert_eq!(action(URL, &project, "main").metadata.omitted_subjects, 1);
    assert_eq!(
        action(URL, &"a".repeat(2049), "main")
            .metadata
            .omitted_subjects,
        2
    );
    assert_eq!(
        action(URL, PROJECT, &"a".repeat(2049))
            .metadata
            .omitted_subjects,
        1
    );
    let budget = 2048 - PROJECT.len() - 1;
    assert_eq!(action(URL, PROJECT, &"a".repeat(budget)).subjects.len(), 2);
    assert_eq!(
        action(URL, PROJECT, &"a".repeat(budget + 1))
            .metadata
            .omitted_subjects,
        1
    );
}

#[test]
fn registered_values_and_shapes_are_omitted_without_looking_up_credentials() {
    let _plugin = GitlabPlugin::new();
    for prefix in ["glpat-", "gloas-", "glptt-", "glrt-", "gldt-"] {
        let token = format!("{prefix}RUNS_SYNTHETIC_CANARY");
        assert_eq!(
            action(URL, PROJECT, &format!("feature/{token}"))
                .metadata
                .omitted_subjects,
            1
        );
        assert_eq!(
            action(URL, &format!("acme/{token}"), "main")
                .metadata
                .omitted_subjects,
            2
        );
        assert_eq!(
            action(&format!("https://git.example/{token}"), PROJECT, "main")
                .metadata
                .omitted_subjects,
            2
        );
    }
    let value = "RUNS_REGISTERED_CANARY_59c8ce34";
    radar_core::log::secret(value);
    assert_eq!(action(URL, PROJECT, value).metadata.omitted_subjects, 1);
    assert_eq!(
        action(URL, &format!("acme/{value}"), "main")
            .metadata
            .omitted_subjects,
        2
    );
    assert_eq!(
        action(&format!("https://git.example/{value}"), PROJECT, "main")
            .metadata
            .omitted_subjects,
        2
    );
}

#[test]
fn secret_registered_only_as_combined_context_omits_that_subject_without_rewriting() {
    let project = "combined-runs-repo-08499c1e";
    let branch = "combined-runs-ref-3286a5cb";
    let combined = format!("{project}:{branch}");
    radar_core::log::secret(&combined);
    assert!(!radar_core::log::contains_secret(project));
    assert!(!radar_core::log::contains_secret(branch));
    let scope = action(URL, project, branch);
    assert_eq!(scope.subjects.len(), 1);
    assert_eq!(scope.subjects[0].key.id, project);
    assert!(scope.metadata.redacted);
    assert_eq!(scope.metadata.omitted_subjects, 1);
}

#[test]
fn omitted_scope_never_changes_exact_query_inputs() {
    for (project, branch) in [
        (PROJECT, "feature:a"),
        (PROJECT, " feature/é "),
        ("acme:repo", "main"),
        (PROJECT, "token=SECRET"),
    ] {
        assert!(action(URL, project, branch).metadata.redacted);
        let replay = Replay::new(vec![(200, response(json!({"nodes": []})))]);
        block(Client::over(replay.clone()).runs_observed(project, branch)).unwrap();
        assert_eq!(
            replay.asked()[0].body.as_ref().unwrap()["variables"],
            json!({"project": project, "ref": branch})
        );
    }
}

fn page_plugin() -> GitlabPlugin {
    let mut p = demo::sample();
    p.cfg.url = URL.into();
    p
}
fn refresh(p: &mut GitlabPlugin) -> Vec<Effect> {
    let radar_core::plugin::Outcome::Handled(fx) = pages::key(
        p,
        Pane::Middle,
        KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
    ) else {
        panic!("refresh handled")
    };
    fx
}

#[test]
fn actual_dispatch_captures_exact_default_or_fallback_ref_without_proving_it_exists() {
    for branch in ["main", "feature/A", "", "feature:a"] {
        let mut p = page_plugin();
        for data in p.data.values_mut() {
            for project in &mut data.projects {
                if project.full_path == PROJECT {
                    project.default_branch = branch.into();
                }
            }
        }
        let fx = pages::push(&mut p, pages::Kind::Repo(PROJECT.into()));
        let scope = scoped(&fx);
        if branch == "feature:a" {
            assert_eq!(scope.metadata.omitted_subjects, 1);
        } else {
            assert_eq!(
                scope.subjects[1].key.id,
                format!(
                    "{PROJECT}:{}",
                    if branch.is_empty() { "main" } else { branch }
                )
            );
        }
        assert!(!fx.iter().any(|e| matches!(e, Effect::Spawn(_))));
    }
    let mut p = page_plugin();
    let fx = pages::push(&mut p, pages::Kind::Repo("acme/unknown".into()));
    assert_eq!(scoped(&fx).subjects[1].key.id, "acme/unknown:main");
}

#[test]
fn fresh_busy_forced_refresh_and_independent_red_diagnosis_behavior_is_unchanged() {
    let mut p = page_plugin();
    let kind = pages::Kind::Repo(PROJECT.into());
    without_open(pages::push(&mut p, kind.clone()));
    assert!(without_open(pages::push(&mut p, kind.clone())).is_empty());
    pages::on_runs(&mut p, PROJECT.into(), Ok(vec![]));
    assert!(without_open(pages::push(&mut p, kind.clone())).is_empty());
    for _ in 0..2 {
        assert_eq!(scoped(&refresh(&mut p)).kind.as_str(), "gitlab.page.runs");
    }
    pages::on_runs(&mut p, PROJECT.into(), Ok(vec![]));
    for data in p.data.values_mut() {
        for project in &mut data.projects {
            if project.full_path == PROJECT {
                project.main_pipeline = Some(crate::model::Pipeline::Failed);
                project.main_pipeline_url = Some(format!("{BASE}/{PROJECT}/-/pipelines/901"));
            }
        }
    }
    let fx = without_open(pages::push(&mut p, kind.clone()));
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.kind.as_str() == "gitlab.pipeline.diagnosis")
    );
    assert!(without_open(pages::push(&mut p, kind)).is_empty());
    let fx = p.tick(p.now + p.cfg.refresh_secs as i64 + 1);
    assert!(!fx.iter().any(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.runs")));
}

#[test]
fn original_report_and_evidence_precede_unchanged_cache_success_partial_and_error_application() {
    for expected in [Outcome::Succeeded, Outcome::Partial, Outcome::Failed] {
        let mut p = page_plugin();
        let mut legacy = page_plugin();
        for plugin in [&mut p, &mut legacy] {
            pages::push(plugin, pages::Kind::Repo(PROJECT.into()));
            pages::on_runs(
                plugin,
                PROJECT.into(),
                Ok(read(json!({"nodes": [node()]})).runs),
            );
            plugin.now += 10;
            refresh(plugin);
        }
        let got = read(if expected == Outcome::Partial {
            json!({})
        } else {
            json!({"nodes": [node(), node()]})
        });
        let plain = if expected == Outcome::Failed {
            Err("SECRET_ERROR".into())
        } else {
            Ok(got.runs.clone())
        };
        pages::on_runs(&mut legacy, PROJECT.into(), plain);
        let result = if expected == Outcome::Failed {
            Err("SECRET_ERROR".into())
        } else {
            Ok(got)
        };
        let request = crate::runs_ownership::pending(&p, PROJECT);
        let fx = p.on_msg(Box::new(GlMsg::Runs {
            request,
            action: action(URL, PROJECT, "main"),
            result,
        }));
        assert!(matches!(fx[0], Effect::Report(outcome) if outcome == expected));
        assert!(
            fx.iter()
                .skip(1)
                .all(|e| matches!(e, Effect::Observe { .. }))
        );
        assert_eq!(format!("{:?}", p.runs), format!("{:?}", legacy.runs));
        assert_eq!(p.snapshot(), legacy.snapshot());
        assert!(!format!("{fx:?}").contains("SECRET"));
    }
}

#[test]
fn numeric_saturation_only_marks_real_count_overflow() {
    let Effect::Observe { action, .. } = count(
        &action(URL, PROJECT, "main"),
        "gitlab.page.runs.entries",
        usize::MAX,
        Outcome::Succeeded,
    ) else {
        unreachable!()
    };
    assert_eq!(action.metadata.count, Some(u32::MAX));
    assert_eq!(action.metadata.truncated, usize::BITS > 32);
}

mod host;

fn without_open(effects: Vec<Effect>) -> Vec<Effect> {
    assert_eq!(effects.iter().filter(|e| matches!(e, Effect::Observe { action, outcome: Outcome::Succeeded } if action.kind.as_str() == "gitlab.page.open")).count(), 1);
    effects.into_iter().filter(|e| !matches!(e, Effect::Observe { action, .. } if action.kind.as_str() == "gitlab.page.open")).collect()
}

#[test]
fn recorded_spawn_carries_frozen_owner_and_scope_across_reversed_completions() {
    let mut p = page_plugin();
    let mut pending = Vec::new();
    for title in ["SECRET_OLD", "SECRET_NEW"] {
        let request = crate::runs_ownership::begin(&mut p, PROJECT, "main");
        let mut n = node();
        n["commit"]["title"] = json!(title);
        let client = Client::over(Replay::new(vec![(200, response(json!({"nodes": [n]})))]));
        pending.push(spawn(action(URL, PROJECT, "main"), request, async move {
            client.runs_observed(PROJECT, "main").await.map_err(|e| e.to_string())
        }));
    }
    for (index, effect) in pending.into_iter().rev().enumerate() {
        let Effect::SpawnAction { action: scope, task } = effect else { panic!("scoped read") };
        let fx = p.on_msg(block(task));
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        for effect in &fx {
            if let Effect::Observe { action, .. } = effect { assert_eq!(action.subjects, scope.subjects); }
        }
        assert_eq!(p.runs[PROJECT].value.as_ref().unwrap()[0].commit_title, "SECRET_NEW");
        assert_eq!(fx.iter().filter(|effect| matches!(effect, Effect::Observe { action, .. }
            if action.kind.as_str() == "gitlab.page.runs.apply.skipped.stale")).count(), index);
        assert!(!format!("{fx:?}").contains("SECRET"));
    }
}
