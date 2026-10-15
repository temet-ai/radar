use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{JobEvidence, Method, Resp, TRACE_PART, parse_job, trace_part, trace_range};
use crate::log_ownership::{self, JobKey};
use crate::{GitlabPlugin, demo, why};
use radar_core::Plugin;
use serde_json::{Value, json};
use std::sync::Arc;

const PROJECT: &str = "acme/team-a/api";
const JOB: u64 = 41;
const URL: &str =
    "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT";
const KIND: &str = "gitlab.job.log";

fn row(id: u64) -> Value {
    json!({"id":id, "name":"SECRET_NAME", "stage":"SECRET_STAGE", "status":"failed",
        "web_url":"https://other.example/SECRET_URL", "failure_reason":"SECRET_REASON",
        "pipeline":{"id":987654321}, "commit":{"id":"SECRET_SHA"}})
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
        body: b"SECRET_LOG error SECRET_SHA Bearer SECRET_TOKEN\n".to_vec(),
        content_range: Some("bytes 100-SECRET_END/200".into()),
        ..Default::default()
    }
}

async fn recorded(replay: Arc<Replay>) -> Result<JobLogRead, String> {
    read(&Client::over(replay), PROJECT, JOB)
        .await
        .map_err(|e| format!("{e:#}"))
}

fn scope() -> ActionDescriptor {
    log_ownership::action(URL, &JobKey::new(PROJECT, JOB))
}

fn evidence(result: &Result<JobLogRead, String>) -> Vec<Effect> {
    observed(&scope(), result)
}

fn component<'a>(effects: &'a [Effect], suffix: &str) -> Option<(&'a ActionDescriptor, Outcome)> {
    let kind = format!("{KIND}.{suffix}");
    effects.iter().find_map(|e| match e {
        Effect::Observe { action, outcome } if action.kind.as_str() == kind => {
            Some((action, *outcome))
        }
        _ => None,
    })
}

fn count_is(effects: &[Effect], suffix: &str, n: u32, outcome: Outcome) {
    let (action, got) = component(effects, suffix).unwrap();
    assert_eq!(action.metadata.count, Some(n));
    assert_eq!(got, outcome);
}

fn begin(p: &mut GitlabPlugin, project: &str, id: u64) -> LogRead {
    let key = JobKey::new(project, id);
    p.logs.entry(key.clone()).or_default().loading = true;
    log_ownership::direct(p, key)
}

fn apply(
    p: &mut GitlabPlugin,
    request: LogRead,
    result: Result<JobLogRead, String>,
) -> Vec<Effect> {
    let expected = observed(&request.action, &result);
    let effects = p.on_msg(Box::new(GlMsg::JobLog { request, result }));
    assert_eq!(
        format!("{:?}", &effects[..expected.len()]),
        format!("{expected:?}")
    );
    effects
}

fn state(p: &GitlabPlugin) -> String {
    format!(
        "{:?}{:?}{:?}{:?}{:?}",
        p.jobs,
        p.logs,
        p.log_reads,
        p.next_log_activity,
        p.snapshot()
    )
}

#[test]
fn public_job_wrapper_keeps_requests_defaults_and_only_nonobject_evidence() {
    for value in [
        row(987654321),
        json!({}),
        json!({"id":"SECRET","pipeline":false}),
        Value::Null,
        json!(false),
        json!(42),
        json!([]),
        json!("SECRET"),
    ] {
        let plain = Replay::new(vec![(200, value.clone())]);
        let replay = Replay::new(vec![(200, value.clone())]);
        let old = block(Client::over(plain.clone()).job(PROJECT, JOB)).unwrap();
        let got = block(Client::over(replay.clone()).job_observed(PROJECT, JOB)).unwrap();
        assert_eq!(old, got.info);
        assert_eq!(got.info, parse_job(&value, PROJECT));
        assert_eq!(
            got.evidence,
            JobEvidence {
                nonobject_response: !value.is_object()
            }
        );
        assert_eq!(plain.asked(), replay.asked());
        let requests = replay.asked();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, Method::Get);
        assert_eq!(
            requests[0].path,
            "/api/v4/projects/acme%2Fteam-a%2Fapi/jobs/41"
        );
        assert!(requests[0].body.is_none() && requests[0].range.is_none());
    }
}

#[test]
fn invalid_job_json_keeps_default_metadata_and_partial_result() {
    for body in [vec![], b"SECRET_INVALID_JSON".to_vec()] {
        let result = block(recorded(Replay::raw(vec![
            Resp {
                status: 200,
                body,
                ..Default::default()
            },
            tail(200),
        ])));
        let got = result.as_ref().unwrap();
        assert_eq!(got.job.info, parse_job(&Value::Null, PROJECT));
        let effects = evidence(&result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Partial)));
        count_is(
            &effects,
            "metadata.responses.nonobject",
            1,
            Outcome::Partial,
        );
        assert!(!format!("{effects:?}").contains("SECRET"));
    }
}

#[test]
fn public_trace_wrapper_and_parser_keep_status_range_and_payload_parity() {
    let headers = [
        None,
        Some("bytes 100-109/200"),
        Some("SECRET_BAD_HEADER"),
        Some("bytes 900-SECRET_END/1"),
        Some("bytes 1--999/0"),
        Some("bytes 0-10/*"),
        Some("bytes */123"),
        Some("Bytes 1-2/3"),
    ];
    for status in [200, 201, 204, 206, 299, 416] {
        for header in headers {
            for before in [None, Some(0), Some(100), Some(TRACE_PART + 10)] {
                let raw = Resp {
                    status,
                    body: b"SECRET_LOG\n".to_vec(),
                    content_range: header.map(String::from),
                    ..Default::default()
                };
                let plain = Replay::raw(vec![raw.clone()]);
                let replay = Replay::raw(vec![raw.clone()]);
                let old = block(Client::over(plain.clone()).trace(PROJECT, JOB, before)).unwrap();
                let got = block(Client::over(replay.clone()).trace_observed(PROJECT, JOB, before))
                    .unwrap();
                let expected = if status == 416 {
                    Default::default()
                } else {
                    trace_part(&raw.body, header)
                };
                assert_eq!(old, expected);
                assert_eq!(got.part, old);
                assert_eq!(got.evidence.supplied_bytes, raw.body.len());
                assert_eq!(plain.asked(), replay.asked());
                let requests = replay.asked();
                assert_eq!(requests.len(), 1);
                assert_eq!(requests[0].method, Method::Get);
                assert_eq!(
                    requests[0].path,
                    "/api/v4/projects/acme%2Fteam-a%2Fapi/jobs/41/trace"
                );
                assert_eq!(requests[0].range, Some(trace_range(before)));
                assert!(requests[0].body.is_none());
            }
        }
    }
}

#[test]
fn trace_evidence_records_only_start_total_parser_outputs_without_validation() {
    for (header, range, start, total) in [
        (None, TraceRange::Missing, 0, 4),
        (Some("SECRET"), TraceRange::Unparseable, 0, 4),
        (
            Some("bytes 900-SECRET_END/1"),
            TraceRange::Parsed {
                start: 900,
                total: 1,
            },
            900,
            1,
        ),
        (
            Some("bytes 1--999/0"),
            TraceRange::Parsed { start: 1, total: 0 },
            1,
            0,
        ),
    ] {
        let got = block(
            Client::over(Replay::raw(vec![Resp {
                status: 200,
                body: b"text".to_vec(),
                content_range: header.map(String::from),
                ..Default::default()
            }]))
            .trace_observed(PROJECT, JOB, None),
        )
        .unwrap();
        assert_eq!((got.part.start, got.part.total), (start, total));
        assert_eq!(
            got.evidence.interpretation,
            TraceInterpretation::AcceptedBody {
                range,
                lossy_utf8: false
            }
        );
        let result = Ok(JobLogRead {
            job: crate::client::JobRead {
                info: parse_job(&row(JOB), PROJECT),
                evidence: JobEvidence::default(),
            },
            trace: got,
        });
        let effects = evidence(&result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
        match range {
            TraceRange::Parsed { start, total } => {
                count_is(
                    &effects,
                    "tail.range.parsed.start",
                    start as u32,
                    Outcome::Succeeded,
                );
                count_is(
                    &effects,
                    "tail.range.parsed.total",
                    total as u32,
                    Outcome::Succeeded,
                );
            }
            TraceRange::Missing => count_is(
                &effects,
                "tail.range.fallback.missing",
                1,
                Outcome::Succeeded,
            ),
            TraceRange::Unparseable => count_is(
                &effects,
                "tail.range.fallback.unparseable",
                1,
                Outcome::Succeeded,
            ),
        }
    }
}

#[test]
fn lossy_large_accepted_body_is_not_a_cutoff_and_416_does_not_decode() {
    for status in [200, 206, 416] {
        let mut raw = tail(status);
        raw.body = vec![0xff; TRACE_PART as usize + 1];
        let result = block(recorded(Replay::raw(vec![response(200, row(JOB)), raw])));
        let got = result.as_ref().unwrap();
        let effects = evidence(&result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
        count_is(
            &effects,
            "tail.body.supplied.bytes",
            TRACE_PART as u32 + 1,
            Outcome::Succeeded,
        );
        assert!(
            effects
                .iter()
                .all(|e| !matches!(e,Effect::Observe{action,..} if action.metadata.truncated))
        );
        if status == 416 {
            assert_eq!(got.trace.part, Default::default());
            assert_eq!(
                got.trace.evidence.interpretation,
                TraceInterpretation::Legacy416Default
            );
            count_is(&effects, "tail.default.status416", 1, Outcome::Succeeded);
            assert_eq!(effects.len(), 3);
        } else {
            assert_eq!(got.trace.part.text.len(), 3 * (TRACE_PART as usize + 1));
            count_is(&effects, "tail.text.lossy", 1, Outcome::Succeeded);
            assert!(component(&effects, "tail.default.status416").is_none());
        }
    }
}

#[test]
fn combined_failures_do_not_invent_metadata_or_tail_progress() {
    for (replies, requests) in [
        (
            vec![response(403, json!({"message":"SECRET_ERROR"})), tail(200)],
            1,
        ),
        (vec![response(200, row(JOB)), tail(500)], 2),
        (vec![], 1),
        (vec![response(200, row(JOB))], 2),
    ] {
        let replay = Replay::raw(replies);
        let result = block(recorded(replay.clone()));
        assert!(result.is_err());
        assert_eq!(replay.asked().len(), requests);
        let effects = evidence(&result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Failed)));
        count_is(&effects, "errors", 1, Outcome::Failed);
        assert_eq!(
            component(&effects, "errors").unwrap().0.metadata.reason,
            Some(Reason::Unspecified)
        );
        assert_eq!(effects.len(), 2);
        assert!(!format!("{effects:?}").contains("SECRET"));
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
fn tail_401_replays_the_whole_closure_and_keeps_only_final_attempt_metadata() {
    for first_metadata in [row(1), Value::Null] {
        for final_metadata in [row(2), Value::Null] {
            let replay = Replay::raw(vec![
                response(200, first_metadata.clone()),
                tail(401),
                response(200, final_metadata.clone()),
                tail(206),
            ]);
            let tokens = Tokens {
                renewable: true,
                renewals: Default::default(),
            };
            let result = block(crate::auth::with_token(&tokens, |_| {
                let client = Client::over(replay.clone());
                async move { read(&client, PROJECT, JOB).await }
            }))
            .map_err(|e| format!("{e:#}"));
            let got = result.as_ref().unwrap();
            assert_eq!(got.job.info, parse_job(&final_metadata, PROJECT));
            assert_eq!(
                got.job.evidence.nonobject_response,
                !final_metadata.is_object()
            );
            assert_eq!(tokens.renewals.load(std::sync::atomic::Ordering::SeqCst), 1);
            let requests = replay.asked();
            assert_eq!(requests.len(), 4);
            assert_eq!(requests[0], requests[2]);
            assert_eq!(requests[1], requests[3]);
            assert!(requests[0].path.ends_with("/jobs/41"));
            assert!(requests[1].path.ends_with("/jobs/41/trace"));
            let effects = evidence(&result);
            assert!(
                matches!(effects[0],Effect::Report(outcome) if outcome == if final_metadata.is_object(){Outcome::Succeeded}else{Outcome::Partial})
            );
        }
    }
}

#[test]
fn metadata_rejection_retry_and_final_tail_rejection_keep_once_only_boundary() {
    for (replies, renewable, calls, failed) in [
        (
            vec![
                response(401, Value::Null),
                response(200, row(JOB)),
                tail(200),
            ],
            true,
            3,
            false,
        ),
        (
            vec![
                response(200, row(1)),
                tail(401),
                response(200, row(2)),
                tail(401),
            ],
            true,
            4,
            true,
        ),
        (
            vec![response(200, row(1)), tail(401), response(500, Value::Null)],
            true,
            3,
            true,
        ),
        (vec![response(200, row(1)), tail(401)], false, 2, true),
    ] {
        let replay = Replay::raw(replies);
        let tokens = Tokens {
            renewable,
            renewals: Default::default(),
        };
        let result = block(crate::auth::with_token(&tokens, |_| {
            let client = Client::over(replay.clone());
            async move { read(&client, PROJECT, JOB).await }
        }))
        .map_err(|e| format!("{e:#}"));
        assert_eq!(replay.asked().len(), calls);
        assert_eq!(tokens.renewals.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(result.is_err(), failed);
        if failed {
            assert_eq!(evidence(&result).len(), 2);
        }
    }
}

#[test]
fn requested_scope_and_numeric_saturation_never_retain_returned_identity_or_secrets() {
    let result = block(recorded(Replay::raw(vec![
        response(200, row(987654321)),
        tail(206),
    ])));
    let scope = scope();
    assert_eq!(scope.subjects.len(), 1);
    assert_eq!(scope.subjects[0].key.id, "acme/team-a/api/41");
    assert_eq!(scope.subjects[0].key.instance, "https://git.example/");
    let effects = observed(&scope, &result);
    for effect in &effects {
        if let Effect::Observe { action, .. } = effect {
            assert_eq!(action.subjects, scope.subjects);
        }
    }
    let text = format!("{effects:?}");
    assert!(!text.contains("SECRET") && !text.contains("987654321"));
    for (url, project, id) in [
        ("file:///SECRET", PROJECT, JOB),
        (URL, "a/../SECRET", JOB),
        (URL, PROJECT, 0),
    ] {
        let action = log_ownership::action(url, &JobKey::new(project, id));
        assert!(action.subjects.is_empty() && action.metadata.redacted);
        let effects = observed(&action, &result);
        assert!(matches!(effects[0], Effect::Report(Outcome::Succeeded)));
        assert!(!format!("{effects:?}").contains("SECRET"));
    }
    for n in [u32::MAX as u64, u32::MAX as u64 + 1, u64::MAX] {
        let Effect::Observe { action, .. } = count(
            &scope,
            "gitlab.job.log.tail.range.parsed.total",
            n,
            Outcome::Succeeded,
        ) else {
            panic!()
        };
        assert_eq!(action.metadata.count, Some(u32::MAX));
        assert_eq!(action.metadata.truncated, n > u32::MAX as u64);
    }
}

#[test]
fn dispatch_noops_are_unchanged_while_direct_and_earlier_actions_are_named() {
    let mut p = demo::sample();
    p.cfg.url = URL.into();
    let fx = why::open_log(&mut p, PROJECT, JOB, false);
    assert!(
        matches!(&fx[..],[Effect::SpawnAction{action,..}] if action.kind.as_str() == KIND && action.subjects == scope().subjects)
    );
    drop(fx); // Production futures never execute in fixtures.
    assert!(why::open_log(&mut p, PROJECT, JOB, false).is_empty());
    assert!(why::open_log(&mut p, PROJECT, JOB, true).is_empty());
    let request = begin(&mut p, PROJECT, JOB);
    apply(
        &mut p,
        request,
        block(recorded(Replay::raw(vec![
            response(200, row(JOB)),
            tail(206),
        ]))),
    );
    assert!(why::open_log(&mut p, PROJECT, JOB, false).is_empty());
    let fx = why::earlier(&mut p, PROJECT, JOB);
    assert!(matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.kind.as_str() == "gitlab.job.log.earlier"));
    drop(fx);
    assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
    let mut p = demo::sample();
    let request = begin(&mut p, PROJECT, JOB);
    apply(&mut p, request, Err("SECRET_ERROR".into()));
    assert!(why::open_log(&mut p, PROJECT, JOB, false).is_empty());
    assert!(matches!(
        &why::open_log(&mut p, PROJECT, JOB, true)[..],
        [Effect::SpawnAction { .. }]
    ));
}

#[test]
fn accepted_typed_reply_matches_legacy_metadata_log_and_error_application() {
    for replies in [
        vec![response(200, row(JOB)), tail(206)],
        vec![response(200, Value::Null), tail(416)],
        vec![response(200, row(JOB)), tail(500)],
        vec![response(403, Value::Null)],
    ] {
        let mut actual = demo::sample();
        let mut legacy = demo::sample();
        let result = block(recorded(Replay::raw(replies)));
        let a = begin(&mut actual, PROJECT, JOB);
        let b = begin(&mut legacy, PROJECT, JOB);
        let (info, part) = match result.clone() {
            Ok(got) => (Some(got.job.info), Ok(got.trace.part)),
            Err(e) => (None, Err(e)),
        };
        why::on_log(&mut legacy, b, info, part);
        apply(&mut actual, a, result);
        assert_eq!(state(&actual), state(&legacy));
    }
}

#[test]
fn stale_success_error_duplicate_and_missing_cache_keep_evidence_without_application() {
    for failed in [false, true] {
        let mut p = demo::sample();
        p.cfg.url = URL.into();
        let old = begin(&mut p, PROJECT, JOB);
        let current = begin(&mut p, PROJECT, JOB);
        let result = if failed {
            Err("SECRET_ERROR".into())
        } else {
            block(recorded(Replay::raw(vec![
                response(200, row(JOB)),
                tail(200),
            ])))
        };
        let before = state(&p);
        let fx = apply(&mut p, old.clone(), result.clone());
        assert_eq!(state(&p), before);
        count_is(&fx, "tail.apply.skipped.stale", 1, Outcome::Blocked);
        apply(&mut p, current.clone(), result.clone());
        let before = state(&p);
        let fx = apply(&mut p, current, result.clone());
        assert_eq!(state(&p), before);
        count_is(&fx, "tail.apply.skipped.stale", 1, Outcome::Blocked);
        let request = begin(&mut p, PROJECT, JOB);
        p.logs.remove(&request.key);
        let fx = apply(&mut p, request, result);
        count_is(&fx, "tail.apply.skipped.stale", 1, Outcome::Blocked);
        assert!(!p.logs.contains_key(&JobKey::new(PROJECT, JOB)));
    }
}

#[test]
fn original_server_scope_survives_reconfiguration_and_project_scopes_do_not_collide() {
    let mut p = demo::sample();
    p.replace_connection(URL.into(), "SECRET_TOKEN".into());
    let old = begin(&mut p, PROJECT, JOB);
    let result = block(recorded(Replay::raw(vec![
        response(200, row(JOB)),
        tail(200),
    ])));
    p.replace_connection("https://new.example".into(), "SECRET_NEW_TOKEN".into());
    let before = state(&p);
    let fx = apply(&mut p, old, result.clone());
    assert_eq!(state(&p), before);
    for e in &fx {
        if let Effect::Observe { action, .. } = e {
            assert_eq!(action.subjects, scope().subjects);
        }
    }
    let a = begin(&mut p, PROJECT, JOB);
    let b = begin(&mut p, "acme/other", JOB);
    apply(&mut p, a, result.clone());
    assert!(p.logs[&b.key].loading);
    apply(&mut p, b, result);
    assert_eq!(p.logs.len(), 2);
    assert_eq!(p.jobs.len(), 2);
}

mod host;

mod earlier;
