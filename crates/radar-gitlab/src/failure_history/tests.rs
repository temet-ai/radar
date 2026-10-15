use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, FailedJobsEvidence, FailedJobsRead, Method, Resp, parse_job};
use crate::model::TracePart;
use crate::{GitlabPlugin, demo, why};
use crate::log_ownership::{self, JobKey, PipelineRead};
use radar_core::host::{Event, HostEffect, JournalRecord};
use radar_core::journal::{ActionId, Journal, Query};
use radar_core::{Host, Plugin};
use serde_json::{Value, json};
use std::sync::Arc;

const PROJECT: &str = "acme/team-a/api";
const PIPELINE: u64 = 901;
const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";
const KIND: &str = "gitlab.pipeline.diagnosis";

fn begin_read(p: &mut GitlabPlugin) -> PipelineRead {
    p.failed
        .entry(why::key(PROJECT, PIPELINE))
        .or_default()
        .loading = true;
    log_ownership::pipeline(p, PROJECT, PIPELINE)
}

fn apply(
    p: &mut GitlabPlugin,
    result: Result<Vec<crate::model::JobInfo>, String>,
    logs: Vec<(u64, Result<TracePart, String>)>,
) {
    let request = begin_read(p);
    why::on_failed(p, request, &action(URL, PROJECT, PIPELINE), result, logs);
}

fn row(id: u64, allowed: bool) -> Value {
    json!({
        "id": id, "allow_failure": allowed, "name": "SECRET_JOB", "stage": "SECRET_STAGE",
        "status": "failed", "failure_reason": "SECRET_REASON", "duration": 1.2,
        "web_url": "https://other.example/SECRET_URL", "pipeline": {"id": 999}
    })
}

fn response(status: u16, body: Value) -> Resp {
    Resp {
        status,
        body: body.to_string().into_bytes(),
        ..Default::default()
    }
}

fn tail(status: u16) -> Resp {
    Resp {
        status,
        body: b"SECRET_LOG error: SECRET_SHA Bearer SECRET_TOKEN".to_vec(),
        content_range: Some("bytes 44-89/90".into()),
        ..Default::default()
    }
}

async fn recorded(replay: Arc<Replay>, logs: bool) -> DiagnosisRead {
    let list = Client::over(replay.clone());
    why::read(
        logs,
        async move {
            list.failed_jobs_observed(PROJECT, PIPELINE)
                .await
                .map_err(|e| format!("{e:#}"))
        },
        move |id| {
            let client = Client::over(replay.clone());
            async move {
                client
                    .trace(PROJECT, id, None)
                    .await
                    .map_err(|e| format!("{e:#}"))
            }
        },
    )
    .await
}

fn observations(effects: &[Effect]) -> Vec<(&str, u32, Outcome, bool)> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } => Some((
                action.kind.as_str(),
                action.metadata.count.unwrap(),
                *outcome,
                action.metadata.truncated,
            )),
            _ => None,
        })
        .collect()
}

fn component(effects: &[Effect], suffix: &str) -> Option<(u32, Outcome)> {
    let kind = format!("{KIND}.{suffix}");
    observations(effects)
        .into_iter()
        .find(|(k, _, _, _)| *k == kind)
        .map(|(_, n, o, _)| (n, o))
}

fn evidence(read: &DiagnosisRead) -> Vec<Effect> {
    observed(&action(URL, PROJECT, PIPELINE), read)
}

#[test]
fn public_observed_methods_keep_request_order_parser_and_all_returned_occurrences() {
    for n in [0, 1, 5, 100, 101] {
        let rows = json!(vec![row(9, false); n]);
        let plain = Replay::new(vec![(200, rows.clone())]);
        let replay = Replay::new(vec![(200, rows.clone())]);
        let old = block(Client::over(plain.clone()).failed_jobs(PROJECT, PIPELINE)).unwrap();
        let got =
            block(Client::over(replay.clone()).failed_jobs_observed(PROJECT, PIPELINE)).unwrap();
        assert_eq!(got.jobs, old);
        assert_eq!(
            got.jobs,
            rows.as_array()
                .unwrap()
                .iter()
                .map(|v| parse_job(v, PROJECT))
                .collect::<Vec<_>>()
        );
        assert_eq!(got.evidence, FailedJobsEvidence::default());
        assert_eq!(plain.asked(), replay.asked());
        let asked = replay.asked();
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].method, Method::Get);
        assert_eq!(
            asked[0].path,
            "/api/v4/projects/acme%2Fteam-a%2Fapi/pipelines/901/jobs?scope[]=failed&per_page=100"
        );
        assert!(asked[0].body.is_none() && asked[0].range.is_none());
        let fx = evidence(&DiagnosisRead {
            result: Ok(got),
            logs: None,
        });
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        assert_eq!(component(&fx, "jobs"), Some((n as u32, Outcome::Succeeded)));
        assert!(observations(&fx).iter().all(|(_, _, _, cut)| !cut));
        assert_eq!(observations(&fx).len(), 1);
    }
}

#[test]
fn malformed_lists_keep_legacy_empty_results_and_partial_shape_evidence() {
    for body in [
        b"SECRET_INVALID_JSON".to_vec(),
        vec![],
        b"null".to_vec(),
        b"{}".to_vec(),
        b"false".to_vec(),
        b"42".to_vec(),
        b"\"SECRET\"".to_vec(),
    ] {
        let raw = Resp {
            status: 200,
            body,
            ..Default::default()
        };
        let plain = Replay::raw(vec![raw.clone()]);
        let replay = Replay::raw(vec![raw]);
        let old = block(Client::over(plain.clone()).failed_jobs(PROJECT, PIPELINE)).unwrap();
        let read = block(recorded(replay.clone(), true));
        let got = read.result.as_ref().unwrap();
        assert_eq!(got.jobs, old);
        assert!(got.jobs.is_empty());
        assert_eq!(
            got.evidence,
            FailedJobsEvidence {
                incomplete_response: true,
                incomplete_rows: 0
            }
        );
        assert_eq!(plain.asked(), replay.asked());
        assert!(read.logs.as_ref().unwrap().is_empty());
        let fx = evidence(&read);
        assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
        assert_eq!(component(&fx, "jobs"), Some((0, Outcome::Partial)));
        assert_eq!(
            component(&fx, "responses.incomplete"),
            Some((1, Outcome::Partial))
        );
        assert_eq!(
            component(&fx, "tails.selected"),
            Some((0, Outcome::Succeeded))
        );
        assert_eq!(
            component(&fx, "tails.succeeded"),
            Some((0, Outcome::Succeeded))
        );
        assert!(component(&fx, "rows.incomplete").is_none());
        assert!(!format!("{fx:?}").contains("SECRET"));
    }
}

#[test]
fn nonobject_rows_are_counted_once_without_rejecting_defaults_or_duplicate_zero_ids() {
    let rows = json!([null, false, 42, [], "SECRET", {}, {"id":"SECRET","pipeline":false,"allow_failure":"SECRET"}]);
    let mut replies = vec![response(200, rows.clone())];
    replies.extend((0..5).map(|_| tail(200)));
    let replay = Replay::raw(replies);
    let read = block(recorded(replay.clone(), true));
    let got = read.result.as_ref().unwrap();
    assert_eq!(
        got.jobs,
        rows.as_array()
            .unwrap()
            .iter()
            .map(|v| parse_job(v, PROJECT))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        got.evidence,
        FailedJobsEvidence {
            incomplete_response: false,
            incomplete_rows: 5
        }
    );
    assert_eq!(
        read.logs
            .as_ref()
            .unwrap()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        vec![0; 5]
    );
    assert!(
        replay.asked()[1..]
            .iter()
            .all(|r| r.path.ends_with("/jobs/0/trace"))
    );
    let fx = evidence(&read);
    assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    assert_eq!(component(&fx, "jobs"), Some((7, Outcome::Partial)));
    assert_eq!(
        component(&fx, "rows.incomplete"),
        Some((5, Outcome::Partial))
    );
    assert_eq!(
        component(&fx, "tails.succeeded"),
        Some((5, Outcome::Succeeded))
    );
    assert_eq!(
        component(&fx, "tails.unselected.bound"),
        Some((2, Outcome::Blocked))
    );
    let scalar = block(recorded(
        Replay::new(vec![(200, json!([{}, {"id": "SECRET", "pipeline": []}]))]),
        false,
    ));
    assert!(matches!(
        evidence(&scalar)[0],
        Effect::Report(Outcome::Succeeded)
    ));
}

#[test]
fn ranked_five_tail_occurrences_remain_sequential_and_continue_after_failure() {
    let rows = json!([
        row(1, true),
        row(2, false),
        row(3, true),
        row(2, false),
        row(4, false),
        row(5, false),
        row(6, false),
        row(7, false)
    ]);
    let mut replies = vec![response(200, rows.clone())];
    replies.extend([tail(206), tail(500), tail(416), tail(200), tail(206)]);
    let replay = Replay::raw(replies);
    let read = block(recorded(replay.clone(), true));
    let got = read.result.as_ref().unwrap();
    assert_eq!(
        got.jobs,
        rows.as_array()
            .unwrap()
            .iter()
            .map(|v| parse_job(v, PROJECT))
            .collect::<Vec<_>>()
    );
    let logs = read.logs.as_ref().unwrap();
    assert_eq!(
        logs.iter().map(|(id, _)| *id).collect::<Vec<_>>(),
        [2, 2, 4, 5, 6]
    );
    assert!(logs[1].1.is_err() && logs[4].1.is_ok());
    assert_eq!(logs[2].1.as_ref().unwrap(), &TracePart::default());
    let asked = replay.asked();
    assert_eq!(asked.len(), 6);
    for (req, id) in asked[1..].iter().zip([2, 2, 4, 5, 6]) {
        assert_eq!(req.method, Method::Get);
        assert_eq!(
            req.path,
            format!("/api/v4/projects/acme%2Fteam-a%2Fapi/jobs/{id}/trace")
        );
        assert_eq!(req.range.as_deref(), Some("bytes=-262144"));
        assert!(req.body.is_none());
    }
    let fx = evidence(&read);
    assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    assert_eq!(component(&fx, "jobs"), Some((8, Outcome::Succeeded)));
    assert_eq!(
        component(&fx, "tails.selected"),
        Some((5, Outcome::Succeeded))
    );
    assert_eq!(
        component(&fx, "tails.succeeded"),
        Some((4, Outcome::Succeeded))
    );
    assert_eq!(component(&fx, "tails.failed"), Some((1, Outcome::Failed)));
    assert_eq!(
        component(&fx, "tails.unselected.bound"),
        Some((3, Outcome::Blocked))
    );
    assert!(observations(&fx).iter().all(|(_, _, _, cut)| !cut));
}

#[test]
fn allowed_failure_jobs_keep_stable_order_after_required_jobs() {
    let replay = Replay::raw(vec![
        response(
            200,
            json!([row(1, true), row(2, false), row(3, true), row(4, false)]),
        ),
        tail(200),
        tail(200),
        tail(200),
        tail(200),
    ]);
    let got = block(recorded(replay, true));
    assert_eq!(
        got.logs
            .unwrap()
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>(),
        [2, 4, 1, 3]
    );
}

#[test]
fn clean_empty_and_max_five_bundles_succeed_without_claiming_inventory_or_all_tails() {
    for n in [0, 1, 5, 6, 100, 101] {
        let mut replies = vec![response(200, json!(vec![row(9, false); n]))];
        replies.extend((0..n.min(5)).map(|_| tail(200)));
        let replay = Replay::raw(replies);
        let read = block(recorded(replay.clone(), true));
        assert_eq!(replay.asked().len(), 1 + n.min(5));
        let fx = evidence(&read);
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        assert_eq!(component(&fx, "jobs"), Some((n as u32, Outcome::Succeeded)));
        assert_eq!(
            component(&fx, "tails.selected"),
            Some((n.min(5) as u32, Outcome::Succeeded))
        );
        assert_eq!(
            component(&fx, "tails.unselected.bound"),
            (n > 5).then_some((n.saturating_sub(5) as u32, Outcome::Blocked))
        );
        assert!(observations(&fx).iter().all(|(_, _, _, cut)| !cut));
    }
}

#[test]
fn logs_false_does_not_call_tails_or_invent_unselected_coverage() {
    let read = block(why::read(
        false,
        async {
            Client::over(Replay::new(vec![(200, json!(vec![row(1, false); 101]))]))
                .failed_jobs_observed(PROJECT, PIPELINE)
                .await
                .map_err(|e| e.to_string())
        },
        |_| async { panic!("tails were not requested") },
    ));
    assert!(read.logs.is_none());
    let fx = evidence(&read);
    assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
    assert_eq!(
        observations(&fx),
        [(
            "gitlab.pipeline.diagnosis.jobs",
            101,
            Outcome::Succeeded,
            false
        )]
    );
}

#[test]
fn list_errors_stop_tails_and_have_only_one_fixed_safe_error_fact() {
    for status in [401, 403, 404, 429, 500] {
        let replay = Replay::new(vec![(status, json!({"message":"SECRET_ERROR"}))]);
        let read = block(recorded(replay.clone(), true));
        assert!(read.result.is_err());
        assert!(read.logs.as_ref().unwrap().is_empty());
        assert_eq!(replay.asked().len(), 1);
        let fx = evidence(&read);
        assert!(matches!(fx[0], Effect::Report(Outcome::Failed)));
        assert_eq!(
            observations(&fx),
            [(
                "gitlab.pipeline.diagnosis.list.errors",
                1,
                Outcome::Failed,
                false
            )]
        );
        if let Effect::Observe { action, .. } = &fx[1] {
            assert_eq!(action.metadata.reason, Some(Reason::Unspecified));
        }
        assert!(!format!("{fx:?}").contains("SECRET"));
    }
    let read = block(recorded(Replay::new(vec![]), false));
    assert!(matches!(
        evidence(&read)[0],
        Effect::Report(Outcome::Failed)
    ));
}

struct Tokens {
    renewals: std::sync::atomic::AtomicUsize,
    reads: std::sync::atomic::AtomicUsize,
}
impl crate::auth::Tokens for Tokens {
    fn token(&self) -> anyhow::Result<String> {
        self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok("SECRET_TOKEN".into())
    }
    fn renew(&self) -> anyhow::Result<bool> {
        self.renewals
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(true)
    }
}

#[test]
fn each_list_and_tail_retains_its_own_once_only_renewable_rejection_boundary() {
    let rows = json!([row(1, false), row(2, false), row(3, false)]);
    let replay = Replay::raw(vec![
        response(401, rows.clone()),
        response(200, rows),
        tail(401),
        tail(200),
        tail(401),
        tail(401),
        tail(200),
    ]);
    let tokens = Tokens {
        renewals: Default::default(),
        reads: Default::default(),
    };
    let read = block(why::read(
        true,
        async {
            crate::auth::with_token(&tokens, |_| {
                let client = Client::over(replay.clone());
                async move { client.failed_jobs_observed(PROJECT, PIPELINE).await }
            })
            .await
            .map_err(|e| format!("{e:#}"))
        },
        |id| {
            let replay = replay.clone();
            let tokens = &tokens;
            async move {
                crate::auth::with_token(tokens, |_| {
                    let client = Client::over(replay.clone());
                    async move { client.trace(PROJECT, id, None).await }
                })
                .await
                .map_err(|e| format!("{e:#}"))
            }
        },
    ));
    assert_eq!(tokens.renewals.load(std::sync::atomic::Ordering::SeqCst), 3);
    assert_eq!(tokens.reads.load(std::sync::atomic::Ordering::SeqCst), 7);
    let asked = replay.asked();
    assert_eq!(asked.len(), 7);
    assert_eq!(asked[0], asked[1]);
    assert_eq!(asked[2], asked[3]);
    assert_eq!(asked[4], asked[5]);
    assert!(asked[6].path.ends_with("/jobs/3/trace"));
    let fx = evidence(&read);
    assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    assert_eq!(
        component(&fx, "tails.selected"),
        Some((3, Outcome::Succeeded))
    );
    assert_eq!(
        component(&fx, "tails.succeeded"),
        Some((2, Outcome::Succeeded))
    );
    assert_eq!(component(&fx, "tails.failed"), Some((1, Outcome::Failed)));
}

#[test]
fn trace_ok_including_empty_and_range_defaults_means_only_the_legacy_wrapper_result() {
    let rows = json!([row(1, false), row(2, false), row(3, false), row(4, false)]);
    let replay = Replay::raw(vec![
        response(200, rows),
        Resp {
            status: 416,
            ..Default::default()
        },
        Resp {
            status: 206,
            body: vec![255, 0],
            content_range: Some("SECRET_MALFORMED".into()),
            ..Default::default()
        },
        Resp {
            status: 200,
            body: vec![],
            ..Default::default()
        },
        tail(206),
    ]);
    let read = block(recorded(replay, true));
    assert!(read.logs.as_ref().unwrap().iter().all(|(_, r)| r.is_ok()));
    let fx = evidence(&read);
    assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
    assert_eq!(
        component(&fx, "tails.succeeded"),
        Some((4, Outcome::Succeeded))
    );
    assert!(!format!("{fx:?}").contains("SECRET"));
}

#[test]
fn requested_pipeline_identity_is_canonical_safe_and_never_inferred_from_results() {
    let scope = action(URL, PROJECT, PIPELINE);
    assert_eq!(scope.subjects[0].key.id, "acme/team-a/api/901");
    assert_eq!(scope.subjects[0].key.instance, "https://git.example/");
    assert_eq!(scope.subjects[0].key.kind, "pipeline");
    assert_eq!(scope.subjects[0].role, SubjectRole::Primary);
    assert!(
        scope.subjects[0].url.is_none()
            && scope.metadata.url.is_none()
            && scope.metadata.count.is_none()
    );
    let mut plugin = demo::sample();
    plugin.cfg.url = URL.into();
    let existing = plugin
        .journal_subject(
            &radar_core::Subject::new("pipeline", "acme/team-a/api#901", "SECRET_TITLE")
                .with_url("https://git.example/acme/team-a/api/-/pipelines/901"),
        )
        .unwrap();
    assert_eq!(scope.subjects[0].key, existing.key);
    for (url, project, pipeline) in [
        ("file:///SECRET", PROJECT, PIPELINE),
        ("not URL", PROJECT, PIPELINE),
        (URL, "acme/../api", PIPELINE),
        (URL, "acme/./api", PIPELINE),
        (URL, "acme?token=SECRET", PIPELINE),
        (URL, PROJECT, 0),
    ] {
        let scope = action(url, project, pipeline);
        assert!(scope.subjects.is_empty() && scope.metadata.redacted);
        assert_eq!(scope.metadata.omitted_subjects, 1);
        assert!(!format!("{scope:?}").contains("SECRET"));
    }
    let fx = evidence(&block(recorded(
        Replay::raw(vec![response(200, json!([row(88, false)])), tail(200)]),
        true,
    )));
    for effect in &fx {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects, scope.subjects);
        }
    }
    assert!(!format!("{fx:?}").contains("SECRET"));
    if let Effect::Observe { action, .. } = count(
        &scope,
        "gitlab.pipeline.diagnosis.jobs",
        usize::MAX,
        Outcome::Succeeded,
    ) {
        assert_eq!(action.metadata.count, Some(u32::MAX));
        assert_eq!(action.metadata.truncated, usize::BITS > 32);
    }
}

#[test]
fn actual_dispatch_keeps_noop_fresh_cache_and_separate_open_log_task_behavior() {
    let mut plugin = demo::sample();
    plugin.cfg.url = URL.into();
    let fx = why::diagnose(&mut plugin, PROJECT, PIPELINE, false, false);
    assert!(matches!(&fx[..],[Effect::SpawnAction{action,..}] if action.kind.as_str()==KIND));
    // All production futures in this test are deliberately unpolled.
    drop(fx);
    assert!(why::diagnose(&mut plugin, PROJECT, PIPELINE, false, false).is_empty());
    assert!(why::diagnose(&mut plugin, PROJECT, PIPELINE, true, false).is_empty());
    let fx = why::diagnose(&mut plugin, PROJECT, PIPELINE, true, true);
    assert!(matches!(&fx[..], [Effect::SpawnAction { .. }]));
    drop(fx);
    let jobs = vec![parse_job(&row(9, false), PROJECT); 7];
    apply(&mut plugin, Ok(jobs), vec![]);
    assert!(why::diagnose(&mut plugin, PROJECT, PIPELINE, false, false).is_empty());
    let fx = why::diagnose(&mut plugin, PROJECT, PIPELINE, true, false);
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.kind.as_str() == "gitlab.job.log"),
        "fresh branch still uses open_log and suppresses duplicate in-flight IDs"
    );
    drop(fx);
    assert!(why::diagnose(&mut plugin, PROJECT, PIPELINE, true, false).is_empty());
    // A finished pipeline's failed jobs do not change: no timer asks again.
    plugin.now += 10 * plugin.cfg.refresh_secs as i64;
    assert!(why::diagnose(&mut plugin, PROJECT, PIPELINE, false, false).is_empty());
}

#[test]
fn actual_replies_keep_original_scope_after_reconfiguration_and_skip_stale_application() {
    let mut plugin = demo::sample();
    let snapshot = plugin.snapshot();
    let mut pending = Vec::new();
    for (url, id) in [("https://first.example", 1), ("https://second.example", 2)] {
        let replay = Replay::raw(vec![response(200, json!([row(id, false)])), tail(200)]);
        plugin.replace_connection(url.into(), "env:SECRET_TOKEN".into());
        let request = begin_read(&mut plugin);
        pending.push(spawn(
            action(url, PROJECT, PIPELINE),
            request,
            recorded(replay, true),
        ));
    }
    plugin.replace_connection("https://third.example".into(), "env:SECRET_TOKEN".into());
    for ((effect, url), id) in pending
        .into_iter()
        .rev()
        .zip(["https://second.example/", "https://first.example/"])
        .zip([2, 1])
    {
        let Effect::SpawnAction { action, task } = effect else {
            panic!("scoped task")
        };
        assert_eq!(action.subjects[0].key.instance, url);
        let fx = plugin.on_msg(block(task));
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        for effect in &fx {
            if let Effect::Observe { action, .. } = effect {
                assert_eq!(action.subjects[0].key.instance, url);
            }
        }
        assert!(why::failed(&plugin, PROJECT, PIPELINE).is_none());
        assert!(!plugin.logs.contains_key(&JobKey::new(PROJECT, id)));
        assert_eq!(component(&fx, "apply.skipped.stale"), Some((1, Outcome::Blocked)));
        assert_eq!(plugin.snapshot(), snapshot);
    }
}

#[test]
fn observed_application_matches_legacy_success_partial_and_failure_cache_state() {
    for mode in [0, 1, 2] {
        let mut actual = demo::sample();
        let mut legacy = demo::sample();
        let prior = vec![parse_job(&row(4, false), PROJECT)];
        for p in [&mut actual, &mut legacy] {
            apply(
                p,
                Ok(prior.clone()),
                vec![(
                    4,
                    Ok(TracePart {
                        text: "old".into(),
                        ..Default::default()
                    }),
                )],
            );
            p.now += 10;
            p.failed
                .get_mut(&why::key(PROJECT, PIPELINE))
                .unwrap()
                .loading = true;
        }
        let replies = if mode == 2 {
            vec![response(500, json!({"message":"SECRET_ERROR"}))]
        } else {
            vec![
                response(200, json!([row(4, false)])),
                tail(if mode == 1 { 500 } else { 200 }),
            ]
        };
        let read = block(recorded(Replay::raw(replies), true));
        let old_result = read
            .result
            .as_ref()
            .map(|r| r.jobs.clone())
            .map_err(Clone::clone);
        apply(
            &mut legacy,
            old_result,
            read.logs.clone().unwrap(),
        );
        let request = begin_read(&mut actual);
        let fx = actual.on_msg(Box::new(GlMsg::Failed {
            request,
            action: action(URL, PROJECT, PIPELINE),
            read,
        }));
        let expected = [Outcome::Succeeded, Outcome::Partial, Outcome::Failed][mode];
        assert!(matches!(fx[0],Effect::Report(o) if o==expected));
        assert!(
            fx.iter()
                .all(|e| matches!(e, Effect::Report(_) | Effect::Observe { .. }))
        );
        assert_eq!(
            format!("{:?}", actual.failed),
            format!("{:?}", legacy.failed)
        );
        assert_eq!(format!("{:?}", actual.logs), format!("{:?}", legacy.logs));
        assert_eq!(actual.jobs, legacy.jobs);
        assert_eq!(actual.snapshot(), legacy.snapshot());
    }
}

#[test]
fn all_optional_tail_errors_leave_successful_list_evidence_and_partial_bundle() {
    let mut replies = vec![response(200, json!([row(1, false), row(2, false)]))];
    replies.extend([tail(500), tail(404)]);
    let read = block(recorded(Replay::raw(replies), true));
    let fx = evidence(&read);
    assert!(matches!(fx[0], Effect::Report(Outcome::Partial)));
    assert_eq!(component(&fx, "jobs"), Some((2, Outcome::Succeeded)));
    assert_eq!(
        component(&fx, "tails.selected"),
        Some((2, Outcome::Succeeded))
    );
    assert_eq!(
        component(&fx, "tails.succeeded"),
        Some((0, Outcome::Succeeded))
    );
    assert_eq!(component(&fx, "tails.failed"), Some((2, Outcome::Failed)));
}

struct Begin(Option<Vec<Resp>>);
struct Sibling;
struct Reads(GitlabPlugin);
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
        let msg = match msg.downcast::<Begin>() {
            Ok(begin) => {
                let scope = action(URL, PROJECT, PIPELINE);
                let request = begin_read(&mut self.0);
                let bundle = match begin.0 {
                    Some(replies) => spawn(scope, request, recorded(Replay::raw(replies), true)),
                    None => spawn(
                        scope,
                        request,
                        why::read(
                            true,
                            std::future::ready(Ok(FailedJobsRead {
                                jobs: vec![parse_job(&row(9, false), PROJECT)],
                                evidence: FailedJobsEvidence::default(),
                            })),
                            |_| std::future::pending(),
                        ),
                    ),
                };
                return vec![
                    Effect::Journal(ActionDescriptor::new(
                        "gitlab.test.parent",
                        Origin::Plugin("gitlab".into()),
                    )),
                    Effect::Report(Outcome::Succeeded),
                    bundle,
                    Effect::SpawnAction {
                        action: ActionDescriptor::new(
                            "gitlab.test.sibling",
                            Origin::Plugin("gitlab".into()),
                        ),
                        task: Box::pin(async { Box::new(Sibling) as Msg }),
                    },
                ];
            }
            Err(msg) => msg,
        };
        match msg.downcast::<Sibling>() {
            Ok(_) => vec![Effect::Report(Outcome::Failed)],
            Err(msg) => self.0.on_msg(msg),
        }
    }
}

fn records(effects: &[HostEffect]) -> Vec<JournalRecord> {
    effects
        .iter()
        .filter_map(|e| match e {
            HostEffect::Journal(record) => Some(record.clone()),
            _ => None,
        })
        .collect()
}

fn descriptor(records: &[JournalRecord], id: &ActionId) -> ActionDescriptor {
    records
        .iter()
        .find_map(|r| match r {
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
        .filter_map(|r| match r {
            JournalRecord::Outcome {
                id: got, outcome, ..
            } if got == id && outcome.is_terminal() => Some(*outcome),
            _ => None,
        })
        .collect()
}

type Pending = (ActionId, Option<ActionId>, radar_core::plugin::Task);
fn tasks(effects: Vec<HostEffect>) -> Vec<Pending> {
    effects
        .into_iter()
        .filter_map(|e| match e {
            HostEffect::Tracked {
                action,
                semantic,
                effect,
            } => match *effect {
                HostEffect::Spawn { task, .. } => Some((action, semantic, task)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

fn persist(records: Vec<JournalRecord>) -> Vec<radar_core::journal::ActionEvent> {
    let dir = std::env::temp_dir().join(format!(
        "radar-gitlab-failure-history-{}",
        ActionId::new().as_str()
    ));
    let mut journal = Journal::open(&dir.join("actions.sqlite3")).unwrap();
    let session = journal.start_session().unwrap();
    for record in records {
        match record {
            JournalRecord::Begin {
                id,
                descriptor,
                parent,
            } => {
                journal
                    .begin_with_id(&session, &id, descriptor, parent.as_ref())
                    .unwrap();
            }
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
            .flat_map(|e| &e.subjects)
            .all(|s| s.role == SubjectRole::Primary && s.key.kind == "pipeline")
    );
    drop(journal);
    std::fs::remove_dir_all(dir).unwrap();
    events
}

#[test]
fn real_host_keeps_bundle_parent_and_sibling_outcomes_independent_and_sqlite_private() {
    for (status, expected) in [
        (200, Outcome::Succeeded),
        (500, Outcome::Partial),
        (403, Outcome::Failed),
    ] {
        let replies = if expected == Outcome::Failed {
            vec![response(403, json!({"message": "SECRET_ERROR"}))]
        } else {
            vec![response(200, json!([row(9, false)])), tail(status)]
        };
        let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
        host.enable_journal();
        let initial = host.update(Event::Msg {
            plugin: 0,
            msg: Box::new(Begin(Some(replies))),
        });
        let mut all = records(&initial);
        let parent = all
            .iter()
            .find_map(|r| match r {
                JournalRecord::Begin { id, descriptor, .. }
                    if descriptor.kind.as_str() == "gitlab.test.parent" =>
                {
                    Some(id.clone())
                }
                _ => None,
            })
            .unwrap();
        assert_eq!(terminal(&all, &parent), [Outcome::Succeeded]);
        let mut ids = Vec::new();
        for (id, semantic, task) in tasks(initial) {
            let semantic = semantic.unwrap();
            let scope = descriptor(&all, &semantic);
            let bundle = scope.kind.as_str() == KIND;
            let fx = host.update(Event::Tracked {
                action: id.clone(),
                semantic: Some(semantic.clone()),
                event: Box::new(Event::Msg {
                    plugin: 0,
                    msg: block(task),
                }),
            });
            let got = records(&fx);
            assert_eq!(
                terminal(&got, &semantic),
                [if bundle { expected } else { Outcome::Failed }]
            );
            assert!(terminal(&got, &parent).is_empty());
            for prior in &ids {
                assert!(terminal(&got, prior).is_empty());
            }
            for record in &got {
                if let JournalRecord::Begin {
                    descriptor, parent, ..
                } = record
                    && descriptor
                        .kind
                        .as_str()
                        .starts_with("gitlab.pipeline.diagnosis.")
                {
                    assert_eq!(parent.as_ref(), Some(&id));
                    assert_eq!(descriptor.subjects, scope.subjects);
                }
            }
            ids.push(semantic);
            all.extend(got);
        }
        let events = persist(all);
        assert_eq!(
            events
                .iter()
                .filter(|e| e.kind.as_str() == KIND && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [expected]
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| e.action_id == parent && e.outcome.is_terminal())
                .map(|e| e.outcome)
                .collect::<Vec<_>>(),
            [Outcome::Succeeded]
        );
    }
}

#[test]
fn interruption_during_first_tail_records_unknown_without_list_or_completion_summaries() {
    let mut host = Host::new(vec![Box::new(Reads(demo::sample()))], 1).unwrap();
    host.enable_journal();
    let fx = host.update(Event::Msg {
        plugin: 0,
        msg: Box::new(Begin(None)),
    });
    let all = records(&fx);
    let mut found = false;
    for (_, semantic, mut task) in tasks(fx) {
        let semantic = semantic.unwrap();
        if descriptor(&all, &semantic).kind.as_str() != KIND {
            continue;
        }
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(task.as_mut().poll(&mut context).is_pending());
        assert!(terminal(&all, &semantic).is_empty());
        found = true;
    }
    assert!(found);
    let events = persist(all);
    assert!(
        events
            .iter()
            .any(|e| e.kind.as_str() == KIND && e.outcome == Outcome::Unknown)
    );
    assert!(
        !events
            .iter()
            .any(|e| e.kind.as_str().starts_with("gitlab.pipeline.diagnosis."))
    );
}
