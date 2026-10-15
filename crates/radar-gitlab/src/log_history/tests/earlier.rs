use super::*;
use crate::joblog::JobLog;

const EARLIER: &str = "gitlab.job.log.earlier";

fn held(p: &mut GitlabPlugin, before: u64) {
    p.logs.entry(JobKey::new(PROJECT, JOB)).or_default().value =
        Some(JobLog::new(crate::model::TracePart {
            text: "cut\nheld\n".into(),
            start: before,
            total: before,
        }));
}

pub(super) fn begin_earlier(p: &mut GitlabPlugin, before: u64) -> LogRead {
    p.cfg.url = URL.into();
    held(p, before);
    log_ownership::earlier(p, JobKey::new(PROJECT, JOB)).unwrap()
}

pub(super) async fn earlier_read(replay: Arc<Replay>, before: u64) -> Result<TraceRead, String> {
    Client::over(replay)
        .trace_observed(PROJECT, JOB, Some(before))
        .await
        .map_err(|e| format!("{e:#}"))
}

fn apply_earlier(
    p: &mut GitlabPlugin,
    request: LogRead,
    result: Result<TraceRead, String>,
) -> Vec<Effect> {
    let expected = earlier_observed(&request.action, &result);
    let effects = p.on_msg(Box::new(GlMsg::EarlierLog { request, result }));
    assert_eq!(
        format!("{:?}", &effects[..expected.len()]),
        format!("{expected:?}")
    );
    assert!(!format!("{effects:?}").contains("SECRET"));
    effects
}

#[test]
fn original_boundary_scope_and_full_u64_range_survive_saturated_capture() {
    for before in [
        0,
        1,
        100,
        TRACE_PART,
        TRACE_PART + 10,
        u32::MAX as u64,
        u32::MAX as u64 + 1,
        u64::MAX,
    ] {
        let mut p = demo::sample();
        held(&mut p, before);
        if before == 0 {
            let old = state(&p);
            assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
            assert_eq!(state(&p), old);
            // Zero cannot dispatch in production, but the preserved client boundary is pinned.
            let replay = Replay::raw(vec![tail(206)]);
            block(earlier_read(replay.clone(), before)).unwrap();
            assert_eq!(replay.asked()[0].range.as_deref(), Some("bytes=0-0"));
            continue;
        }
        let request = begin_earlier(&mut p, before);
        assert_eq!(request.earlier.unwrap().before, before);
        let replay = Replay::raw(vec![tail(206)]);
        let Effect::SpawnAction { action, task } =
            earlier_spawn(request.clone(), earlier_read(replay.clone(), before))
        else {
            panic!()
        };
        assert_eq!(action.kind.as_str(), EARLIER);
        assert_eq!(action.subjects, scope().subjects);
        assert_eq!(
            action.metadata.count,
            Some(before.min(u32::MAX.into()) as u32)
        );
        assert_eq!(action.metadata.truncated, before > u32::MAX.into());
        // The held start may change during I/O; the request still uses its original boundary.
        p.logs
            .get_mut(&request.key)
            .unwrap()
            .value
            .as_mut()
            .unwrap()
            .start = 19;
        let fx = p.on_msg(block(task));
        let requests = replay.asked();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, Method::Get);
        assert_eq!(
            requests[0].path,
            "/api/v4/projects/acme%2Fteam-a%2Fapi/jobs/41/trace"
        );
        assert_eq!(requests[0].range, Some(trace_range(Some(before))));
        assert!(requests[0].body.is_none());
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        count_is(&fx, "earlier.apply.skipped.stale", 1, Outcome::Blocked);
        assert!(
            fx.iter()
                .all(|e| !matches!(e, Effect::Observe { action, .. } if action.metadata.truncated))
        );
        assert!(!format!("{fx:?}").contains("SECRET"));
    }
}

#[test]
fn earlier_parser_facts_and_permissive_prepend_match_legacy_for_every_branch() {
    for (status, header) in [
        (200, None),
        (206, Some("SECRET_HEADER")),
        (204, Some("bytes 0-0/0")),
        (206, Some("bytes 900-SECRET_END/1")),
        (299, Some("bytes 1--9/0")),
        (416, Some("SECRET_IGNORED")),
    ] {
        let raw = Resp {
            status,
            content_range: header.map(String::from),
            body: vec![0xff; TRACE_PART as usize + 1],
            ..Default::default()
        };
        let result = block(earlier_read(Replay::raw(vec![raw.clone()]), 100));
        let got = result.as_ref().unwrap();
        let mut actual = demo::sample();
        let mut legacy = demo::sample();
        let a = begin_earlier(&mut actual, 100);
        let b = begin_earlier(&mut legacy, 100);
        why::on_log(&mut legacy, b, None, Ok(got.part.clone()));
        let fx = apply_earlier(&mut actual, a, result.clone());
        assert_eq!(state(&actual), state(&legacy));
        assert!(matches!(fx[0], Effect::Report(Outcome::Succeeded)));
        count_is(
            &fx,
            "earlier.body.supplied.bytes",
            TRACE_PART as u32 + 1,
            Outcome::Succeeded,
        );
        assert!(
            fx.iter()
                .all(|e| !matches!(e, Effect::Observe { action, .. } if action.metadata.truncated))
        );
        let standalone = observed(
            &scope(),
            &Ok(JobLogRead {
                job: crate::client::JobRead {
                    info: parse_job(&row(JOB), PROJECT),
                    evidence: JobEvidence::default(),
                },
                trace: got.clone(),
            }),
        );
        for (earlier, tail) in fx.iter().skip(1).zip(standalone.iter().skip(1)) {
            assert_eq!(
                format!("{earlier:?}").replace(".earlier.", ".tail."),
                format!("{tail:?}")
            );
        }
        assert_eq!(fx.len(), standalone.len());
        if status == 416 {
            assert_eq!(got.part, Default::default());
            count_is(&fx, "earlier.default.status416", 1, Outcome::Succeeded);
            assert_eq!(fx.len(), 3);
        } else {
            count_is(&fx, "earlier.body.accepted", 1, Outcome::Succeeded);
            count_is(&fx, "earlier.text.lossy", 1, Outcome::Succeeded);
        }
    }
}

#[test]
fn fake_auth_repeats_only_one_trace_and_final_errors_have_no_fabricated_progress() {
    for (replies, renewable, requests, failed, renewals) in [
        (vec![tail(401), tail(206)], true, 2, false, 1),
        (vec![tail(401), tail(416)], true, 2, false, 1),
        (vec![tail(401), tail(401)], true, 2, true, 1),
        (vec![tail(401), tail(500)], true, 2, true, 1),
        (vec![tail(401)], false, 1, true, 1),
        (
            vec![response(403, json!({"message":"SECRET_ERROR"}))],
            true,
            1,
            true,
            0,
        ),
        (vec![], true, 1, true, 0),
    ] {
        let replay = Replay::raw(replies);
        let tokens = Tokens {
            renewable,
            renewals: Default::default(),
        };
        let before = u64::MAX;
        let result = block(crate::auth::with_token(&tokens, |_| {
            let client = Client::over(replay.clone());
            async move { client.trace_observed(PROJECT, JOB, Some(before)).await }
        }))
        .map_err(|e| format!("{e:#}"));
        assert_eq!(result.is_err(), failed);
        assert_eq!(
            tokens.renewals.load(std::sync::atomic::Ordering::SeqCst),
            renewals
        );
        let asked = replay.asked();
        assert_eq!(asked.len(), requests);
        assert!(asked.iter().all(|r| r == &asked[0]
            && r.range == Some(trace_range(Some(before)))
            && r.path.ends_with("/trace")));
        let mut p = demo::sample();
        let request = begin_earlier(&mut p, before);
        let fx = apply_earlier(&mut p, request, result);
        if failed {
            assert!(matches!(fx[0], Effect::Report(Outcome::Failed)));
            count_is(&fx, "earlier.errors", 1, Outcome::Failed);
            let (error, _) = component(&fx, "earlier.errors").unwrap();
            assert_eq!(error.metadata.reason, Some(Reason::Unspecified));
            assert!(!error.metadata.truncated);
            assert_eq!(fx.len(), 2);
        }
    }
}

#[test]
fn dispatch_noops_busy_and_direct_reload_keep_the_same_ownership_contract() {
    let mut p = demo::sample();
    assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
    held(&mut p, 0);
    assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
    held(&mut p, 100);
    let fx = why::earlier(&mut p, PROJECT, JOB);
    assert!(
        matches!(&fx[..], [Effect::SpawnAction { action, .. }] if action.kind.as_str() == EARLIER && action.metadata.count == Some(100))
    );
    drop(fx); // No credentials or production transport in fixtures.
    assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
    assert!(why::open_log(&mut p, PROJECT, JOB, false).is_empty());
    let fx = why::open_log(&mut p, PROJECT, JOB, true);
    assert!(matches!(&fx[..], [Effect::SpawnAction { .. }]));
    drop(fx);
    assert!(p.log_reads[&JobKey::new(PROJECT, JOB)].earlier.is_none());
    assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
}

#[test]
fn actual_success_or_error_precedes_skip_for_replaced_base_duplicate_or_connection() {
    for change in [
        "direct",
        "base",
        "start",
        "value",
        "cache",
        "connection",
        "duplicate",
    ] {
        for failed in [false, true] {
            let mut p = demo::sample();
            let request = begin_earlier(&mut p, u64::MAX);
            let result = if failed {
                Err("SECRET_ERROR".into())
            } else {
                block(earlier_read(Replay::raw(vec![tail(206)]), u64::MAX))
            };
            match change {
                "direct" => {
                    begin(&mut p, PROJECT, JOB);
                }
                "base" => p.log_reads.get_mut(&request.key).unwrap().base += 1,
                "start" => {
                    p.logs
                        .get_mut(&request.key)
                        .unwrap()
                        .value
                        .as_mut()
                        .unwrap()
                        .start -= 1
                }
                "value" => p.logs.get_mut(&request.key).unwrap().value = None,
                "cache" => {
                    p.logs.remove(&request.key);
                }
                "connection" => {
                    p.replace_connection("https://new.example".into(), "SECRET_NEW_TOKEN".into())
                }
                "duplicate" => {
                    apply_earlier(&mut p, request.clone(), result.clone());
                }
                _ => unreachable!(),
            }
            let held = format!("{:?}{:?}", p.jobs, p.logs);
            let activity = p.next_log_activity;
            let fx = apply_earlier(&mut p, request.clone(), result);
            count_is(&fx, "earlier.apply.skipped.stale", 1, Outcome::Blocked);
            assert_eq!(format!("{:?}{:?}", p.jobs, p.logs), held);
            assert_eq!(p.next_log_activity, activity);
            for e in &fx {
                if let Effect::Observe { action, .. } = e {
                    assert_eq!(action.subjects, request.action.subjects);
                    assert!(!action.metadata.truncated);
                }
            }
        }
    }
}

#[test]
fn earlier_facts_saturate_only_their_own_numbers_and_keep_safe_scope_omissions() {
    let mut p = demo::sample();
    let mut request = begin_earlier(&mut p, u64::MAX);
    request.action.subjects.clear();
    request.action.metadata.redacted = true;
    request.action.metadata.omitted_subjects = 1;
    let result = Ok(TraceRead {
        part: Default::default(),
        evidence: TraceEvidence {
            supplied_bytes: 1,
            interpretation: TraceInterpretation::AcceptedBody {
                range: TraceRange::Parsed {
                    start: u64::MAX,
                    total: u32::MAX as u64,
                },
                lossy_utf8: false,
            },
        },
    });
    let fx = apply_earlier(&mut p, request, result);
    let (start, _) = component(&fx, "earlier.range.parsed.start").unwrap();
    assert_eq!(start.metadata.count, Some(u32::MAX));
    assert!(start.metadata.truncated);
    for e in &fx {
        if let Effect::Observe { action, .. } = e {
            assert!(action.subjects.is_empty() && action.metadata.redacted);
            assert_eq!(action.metadata.omitted_subjects, 1);
            if action.kind != start.kind {
                assert!(!action.metadata.truncated);
            }
        }
    }
}
