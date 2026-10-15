//! Pure reducer fixtures. Real dispatch futures are inspected and dropped without
//! polling: these tests never resolve credentials, call providers, or write files.

use super::*;
use crate::client::{FailedJobsEvidence, FailedJobsRead};
use crate::model::{JobInfo, TracePart};
use crate::{GlMsg, failure_history, joblog::JobLog, why};
use radar_core::Plugin;
use serde_json::json;

const PROJECT: &str = "acme/team-a/api";
const OTHER: &str = "acme/team-b/api";
const PIPELINE: u64 = 901;
const JOB: u64 = 41;
const PIPELINE_SKIP: &str = "gitlab.pipeline.diagnosis.apply.skipped.stale";
const JOB_SKIP: &str = "gitlab.pipeline.diagnosis.jobs.apply.skipped.newer";
const TAIL_SKIP: &str = "gitlab.pipeline.diagnosis.tails.apply.skipped.newer";
const DIRECT_SKIP: &str = "gitlab.job.log.tail.apply.skipped.stale";
const EARLIER_SKIP: &str = "gitlab.job.log.earlier.apply.skipped.stale";

#[derive(Clone)]
struct Pending {
    request: PipelineRead,
    action: ActionDescriptor,
}

fn plugin() -> GitlabPlugin {
    GitlabPlugin {
        now: 1_000,
        env: Some(vec![]),
        ..Default::default()
    }
}

fn job(project: &str, id: u64, name: &str) -> JobInfo {
    crate::client::parse_job(
        &json!({"id":id, "name":name, "stage":"SECRET_STAGE", "status":"failed",
            "failure_reason":"SECRET_REASON", "web_url":"https://other.example/SECRET_URL",
            "pipeline":{"id":123456789}}),
        project,
    )
}

fn part(text: &str, start: u64) -> TracePart {
    TracePart {
        text: format!("{text}\n"),
        start,
        total: 1_000,
    }
}

fn launch_pipeline(p: &mut GitlabPlugin, project: &str, pipeline: u64) -> Pending {
    let activity = p.next_log_activity;
    let mut fx = why::diagnose(p, project, pipeline, true, true);
    assert_eq!(fx.len(), 1);
    let Effect::SpawnAction { action, .. } = fx.remove(0) else {
        panic!("actual diagnosis dispatch")
    };
    assert_eq!(p.next_log_activity, activity);
    Pending {
        request: PipelineRead {
            project: project.into(),
            pipeline,
            owner: p.failed_reads[&why::key(project, pipeline)],
            activity,
        },
        action,
    }
}

fn launch_log(p: &mut GitlabPlugin, project: &str, id: u64, is_earlier: bool) -> LogRead {
    let fx = if is_earlier {
        why::earlier(p, project, id)
    } else {
        why::open_log(p, project, id, true)
    };
    let [Effect::SpawnAction { action, .. }] = &fx[..] else {
        panic!("named log dispatch")
    };
    let key = JobKey::new(project, id);
    let reads = &p.log_reads[&key];
    let earlier = is_earlier.then(|| reads.earlier.expect("earlier dispatch owner"));
    LogRead {
        owner: earlier.map_or_else(|| reads.direct.expect("direct dispatch owner"), |r| r.owner),
        earlier,
        action: action.clone(),
        key,
    }
}

fn receive_pipeline_read(
    p: &mut GitlabPlugin,
    pending: &Pending,
    read: why::DiagnosisRead,
) -> Vec<Effect> {
    let evidence = failure_history::observed(&pending.action, &read);
    let fx = p.on_msg(Box::new(GlMsg::Failed {
        request: pending.request.clone(),
        action: pending.action.clone(),
        read,
    }));
    assert!(fx.len() >= evidence.len());
    assert_eq!(
        format!("{:?}", &fx[..evidence.len()]),
        format!("{evidence:?}")
    );
    fx
}

fn receive_pipeline(
    p: &mut GitlabPlugin,
    pending: &Pending,
    result: Result<Vec<JobInfo>, String>,
    logs: Vec<(u64, Result<TracePart, String>)>,
) -> Vec<Effect> {
    receive_pipeline_read(
        p,
        pending,
        why::DiagnosisRead {
            result: result.map(|jobs| FailedJobsRead {
                jobs,
                evidence: FailedJobsEvidence::default(),
            }),
            logs: Some(logs),
        },
    )
}

fn receive_log(
    p: &mut GitlabPlugin,
    request: &LogRead,
    info: Option<JobInfo>,
    result: Result<TracePart, String>,
) -> Vec<Effect> {
    let fx = p.on_msg(Box::new(GlMsg::Log {
        request: request.clone(),
        info,
        result,
    }));
    assert!(!fx.iter().any(|effect| matches!(effect, Effect::Report(_))));
    fx
}

fn install(p: &mut GitlabPlugin, project: &str, id: u64) {
    let request = launch_log(p, project, id, false);
    assert!(
        receive_log(
            p,
            &request,
            Some(job(project, id, "SECRET_HELD")),
            Ok(part("fragment\nheld", 100)),
        )
        .is_empty()
    );
}

fn state(p: &GitlabPlugin) -> String {
    format!(
        "{:?}{:?}{:?}{:?}{:?}{:?}{:?}{:?}",
        p.failed,
        p.failed_reads,
        p.jobs,
        p.logs,
        p.log_reads,
        p.next_page_read,
        p.next_log_activity,
        p.snapshot()
    )
}

fn target(p: &GitlabPlugin, key: &JobKey) -> String {
    format!(
        "{:?}{:?}{:?}",
        p.jobs.get(key),
        p.logs.get(key),
        p.log_reads.get(key)
    )
}

fn assert_skip(fx: &[Effect], scope: &ActionDescriptor, kind: &str, count: u32) {
    let matches: Vec<_> = fx
        .iter()
        .filter_map(|effect| match effect {
            Effect::Observe { action, outcome } if action.kind.as_str() == kind => {
                Some((action, outcome))
            }
            _ => None,
        })
        .collect();
    let [(action, outcome)] = &matches[..] else {
        panic!("exactly one {kind}: {fx:?}")
    };
    assert_eq!(**outcome, Outcome::Blocked);
    assert_eq!(action.metadata.count, Some(count));
    assert_eq!(action.subjects, scope.subjects);
    assert_eq!(action.metadata.redacted, scope.metadata.redacted);
    assert_eq!(
        action.metadata.omitted_subjects,
        scope.metadata.omitted_subjects
    );
    assert!(!fx.iter().any(|effect| matches!(
        effect,
        Effect::Spawn(_) | Effect::SpawnAction { .. } | Effect::PersistCache | Effect::Emit { .. }
    )));
    assert!(!format!("{fx:?}").contains("SECRET"));
}

#[test]
fn forced_diagnoses_are_latest_owner_wins_for_success_error_and_duplicate_replies() {
    for newer_ok in [false, true] {
        for older_ok in [false, true] {
            for old_first in [false, true] {
                let mut p = plugin();
                let seed = launch_pipeline(&mut p, PROJECT, PIPELINE);
                receive_pipeline(
                    &mut p,
                    &seed,
                    Ok(vec![job(PROJECT, JOB, "SECRET_SEED")]),
                    vec![(JOB, Ok(part("seed", 0)))],
                );
                let older = launch_pipeline(&mut p, PROJECT, PIPELINE);
                let newer = launch_pipeline(&mut p, PROJECT, PIPELINE);
                let stale = |p: &mut GitlabPlugin| {
                    let before = state(p);
                    let fx = receive_pipeline(
                        p,
                        &older,
                        if older_ok {
                            Ok(vec![job(PROJECT, JOB, "SECRET_OLD")])
                        } else {
                            Err("SECRET_OLD_ERROR".into())
                        },
                        vec![(JOB, Ok(part("SECRET_OLD_TAIL", 0)))],
                    );
                    assert_skip(&fx, &older.action, PIPELINE_SKIP, 1);
                    assert_eq!(state(p), before);
                };
                if old_first {
                    stale(&mut p);
                }
                let fx = receive_pipeline(
                    &mut p,
                    &newer,
                    if newer_ok {
                        Ok(vec![job(PROJECT, JOB, "SECRET_NEW")])
                    } else {
                        Err("SECRET_NEW_ERROR".into())
                    },
                    vec![],
                );
                assert!(!format!("{fx:?}").contains("skipped"));
                let held = &p.failed[&newer.request.key()];
                assert!(!held.loading);
                assert_eq!(
                    held.value.as_ref().unwrap()[0].name,
                    if newer_ok {
                        "SECRET_NEW"
                    } else {
                        "SECRET_SEED"
                    }
                );
                assert_eq!(
                    held.error.as_deref(),
                    (!newer_ok).then_some("SECRET_NEW_ERROR")
                );
                stale(&mut p);
                stale(&mut p);
                let before = state(&p);
                let fx = receive_pipeline(&mut p, &newer, Err("SECRET_DUPLICATE".into()), vec![]);
                assert_skip(&fx, &newer.action, PIPELINE_SKIP, 1);
                assert_eq!(state(&p), before);
            }
        }
    }
}

#[test]
fn stale_diagnosis_keeps_all_original_partial_observations_before_its_skip() {
    let mut p = plugin();
    let old = launch_pipeline(&mut p, PROJECT, PIPELINE);
    let _new = launch_pipeline(&mut p, PROJECT, PIPELINE);
    let before = state(&p);
    let fx = receive_pipeline_read(
        &mut p,
        &old,
        why::DiagnosisRead {
            result: Ok(FailedJobsRead {
                jobs: vec![
                    job(PROJECT, JOB, "SECRET_ONE"),
                    job(PROJECT, JOB, "SECRET_TWO"),
                ],
                evidence: FailedJobsEvidence {
                    incomplete_response: true,
                    incomplete_rows: 2,
                },
            }),
            logs: Some(vec![
                (JOB, Ok(part("SECRET_TRACE", 0))),
                (JOB, Err("SECRET_ERROR".into())),
            ]),
        },
    );
    assert!(matches!(fx.first(), Some(Effect::Report(Outcome::Partial))));
    assert!(
        matches!(fx.last(), Some(Effect::Observe { action, .. }) if action.kind.as_str() == PIPELINE_SKIP)
    );
    assert_skip(&fx, &old.action, PIPELINE_SKIP, 1);
    assert_eq!(state(&p), before);
}

#[test]
fn pipeline_and_project_targets_remain_independent() {
    let mut p = plugin();
    let a = launch_pipeline(&mut p, PROJECT, PIPELINE);
    let b = launch_pipeline(&mut p, PROJECT, PIPELINE + 1);
    let c = launch_pipeline(&mut p, OTHER, PIPELINE);
    for (pending, id, title) in [
        (&c, JOB, "other"),
        (&b, JOB + 1, "second"),
        (&a, JOB, "first"),
    ] {
        let fx = receive_pipeline(
            &mut p,
            pending,
            Ok(vec![job(&pending.request.project, id, title)]),
            vec![(id, Ok(part(title, 0)))],
        );
        assert!(!format!("{fx:?}").contains("skipped"));
    }
    assert!(p.failed_reads.is_empty());
    assert_eq!(p.failed.len(), 3);
    for (project, id, title) in [
        (PROJECT, JOB, "first"),
        (PROJECT, JOB + 1, "second"),
        (OTHER, JOB, "other"),
    ] {
        let key = JobKey::new(project, id);
        assert_eq!(p.jobs[&key].name, title);
        assert_eq!(
            p.logs[&key].value.as_ref().unwrap().raw,
            format!("{title}\n")
        );
    }
}

#[test]
fn direct_dispatch_noops_and_consumed_callbacks_cannot_steal_later_owners() {
    for first_ok in [false, true] {
        for later_ok in [false, true] {
            let mut p = plugin();
            let first = launch_log(&mut p, PROJECT, JOB, false);
            let before = state(&p);
            assert!(why::open_log(&mut p, PROJECT, JOB, true).is_empty());
            assert!(why::open_log(&mut p, PROJECT, JOB, false).is_empty());
            assert_eq!(state(&p), before);
            assert!(
                receive_log(
                    &mut p,
                    &first,
                    first_ok.then(|| job(PROJECT, JOB, "first")),
                    if first_ok {
                        Ok(part("first", 0))
                    } else {
                        Err("SECRET_FIRST".into())
                    }
                )
                .is_empty()
            );
            let before = state(&p);
            assert!(why::open_log(&mut p, PROJECT, JOB, false).is_empty());
            assert_eq!(state(&p), before);
            let later = launch_log(&mut p, PROJECT, JOB, false);
            for stale_error in [false, true] {
                let before = state(&p);
                let fx = receive_log(
                    &mut p,
                    &first,
                    Some(job(OTHER, 0, "SECRET_STALE_INFO")),
                    if stale_error {
                        Err("SECRET_STALE".into())
                    } else {
                        Ok(part("SECRET_STALE", 0))
                    },
                );
                assert_eq!(fx.len(), 1);
                assert_skip(&fx, &first.action, DIRECT_SKIP, 1);
                assert_eq!(state(&p), before);
                assert_eq!(p.log_reads[&later.key].direct, Some(later.owner));
                assert!(p.logs[&later.key].loading);
            }
            assert!(
                receive_log(
                    &mut p,
                    &later,
                    later_ok.then(|| job(PROJECT, JOB, "later")),
                    if later_ok {
                        Ok(part("later", 0))
                    } else {
                        Err("SECRET_LATER".into())
                    }
                )
                .is_empty()
            );
            let before = state(&p);
            for request in [&first, &later] {
                let fx = receive_log(&mut p, request, None, Err("SECRET_DUPLICATE".into()));
                assert_skip(&fx, &request.action, DIRECT_SKIP, 1);
                assert_eq!(state(&p), before);
            }
            assert!(!p.logs[&later.key].loading);
            assert!(p.log_reads[&later.key].direct.is_none());
        }
    }
}

#[test]
fn direct_and_earlier_reads_are_independent_across_project_and_job() {
    let mut p = plugin();
    for (project, id) in [(PROJECT, JOB), (OTHER, JOB), (PROJECT, JOB + 1)] {
        install(&mut p, project, id);
    }
    let a = launch_log(&mut p, PROJECT, JOB, true);
    let b = launch_log(&mut p, OTHER, JOB, true);
    let c = launch_log(&mut p, PROJECT, JOB + 1, false);
    assert!(receive_log(&mut p, &b, None, Ok(part("other earlier", 0))).is_empty());
    assert!(earlier_loading(&p, &a.key));
    assert!(p.logs[&c.key].loading);
    assert!(
        receive_log(
            &mut p,
            &c,
            Some(job(PROJECT, JOB + 1, "direct")),
            Ok(part("direct", 0))
        )
        .is_empty()
    );
    assert!(earlier_loading(&p, &a.key));
    assert!(receive_log(&mut p, &a, None, Err("SECRET_EARLIER_ERROR".into())).is_empty());
    assert_eq!(p.logs[&a.key].value.as_ref().unwrap().start, 100);
    assert_eq!(p.logs[&b.key].value.as_ref().unwrap().start, 0);
    assert_eq!(p.jobs[&c.key].name, "direct");
}

#[test]
fn duplicate_earlier_success_or_error_never_prepends_twice_or_clears_new_loading() {
    for first_ok in [false, true] {
        let mut p = plugin();
        install(&mut p, PROJECT, JOB);
        let first = launch_log(&mut p, PROJECT, JOB, true);
        let before = state(&p);
        assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
        assert_eq!(state(&p), before);
        assert!(
            receive_log(
                &mut p,
                &first,
                None,
                if first_ok {
                    Ok(part("prefix", 50))
                } else {
                    Err("SECRET_EARLIER_ERROR".into())
                }
            )
            .is_empty()
        );
        let next = launch_log(&mut p, PROJECT, JOB, true);
        for stale_error in [false, true] {
            let before = state(&p);
            let fx = receive_log(
                &mut p,
                &first,
                Some(job(OTHER, 9, "SECRET_IGNORED")),
                if stale_error {
                    Err("SECRET_STALE".into())
                } else {
                    Ok(part("SECRET_DUPLICATE", 0))
                },
            );
            assert_eq!(fx.len(), 1);
            assert_skip(&fx, &first.action, EARLIER_SKIP, 1);
            assert_eq!(state(&p), before);
            assert_eq!(p.log_reads[&next.key].earlier, next.earlier);
        }
        assert!(receive_log(&mut p, &next, None, Ok(part("beginning", 0))).is_empty());
        assert!(!earlier_loading(&p, &next.key));
    }
}

#[test]
fn direct_reload_retires_earlier_and_suppresses_pagination_until_completion() {
    for direct_ok in [false, true] {
        let mut p = plugin();
        install(&mut p, PROJECT, JOB);
        let earlier = launch_log(&mut p, PROJECT, JOB, true);
        let direct = launch_log(&mut p, PROJECT, JOB, false);
        assert!(p.log_reads[&direct.key].earlier.is_none());
        let before = state(&p);
        assert!(why::earlier(&mut p, PROJECT, JOB).is_empty());
        assert_eq!(state(&p), before);
        for stale_error in [false, true] {
            let fx = receive_log(
                &mut p,
                &earlier,
                None,
                if stale_error {
                    Err("SECRET_OLD_ERROR".into())
                } else {
                    Ok(part("SECRET_OLD_PREFIX", 0))
                },
            );
            assert_skip(&fx, &earlier.action, EARLIER_SKIP, 1);
            assert_eq!(state(&p), before);
        }
        assert!(
            receive_log(
                &mut p,
                &direct,
                None,
                if direct_ok {
                    Ok(part("replacement", 80))
                } else {
                    Err("SECRET_RELOAD_ERROR".into())
                }
            )
            .is_empty()
        );
        let next = launch_log(&mut p, PROJECT, JOB, true);
        assert_eq!(
            next.earlier.unwrap().before,
            if direct_ok { 80 } else { 100 }
        );
        let before = state(&p);
        let fx = receive_log(&mut p, &earlier, None, Err("SECRET_OLD_ERROR".into()));
        assert_skip(&fx, &earlier.action, EARLIER_SKIP, 1);
        assert_eq!(state(&p), before);
    }
}

#[test]
fn bundles_yield_to_direct_and_earlier_activity_before_or_after_bundle_launch() {
    for is_earlier in [false, true] {
        for before_bundle in [false, true] {
            for finished_before_reply in [false, true] {
                for read_ok in [false, true] {
                    let mut p = plugin();
                    install(&mut p, PROJECT, JOB);
                    let (request, bundle) = if before_bundle {
                        let request = launch_log(&mut p, PROJECT, JOB, is_earlier);
                        let bundle = launch_pipeline(&mut p, PROJECT, PIPELINE);
                        (request, bundle)
                    } else {
                        let bundle = launch_pipeline(&mut p, PROJECT, PIPELINE);
                        let request = launch_log(&mut p, PROJECT, JOB, is_earlier);
                        (request, bundle)
                    };
                    if finished_before_reply {
                        receive_log(
                            &mut p,
                            &request,
                            (!is_earlier && read_ok).then(|| job(PROJECT, JOB, "SECRET_DIRECT")),
                            if read_ok {
                                Ok(part("SECRET_INTERVENING", 50))
                            } else {
                                Err("SECRET_INTERVENING_ERROR".into())
                            },
                        );
                    }
                    let before = target(&p, &request.key);
                    let activity = p.next_log_activity;
                    let fx = receive_pipeline(
                        &mut p,
                        &bundle,
                        Ok(vec![job(PROJECT, JOB, "SECRET_BUNDLE")]),
                        vec![(JOB, Ok(part("SECRET_BUNDLE", 0)))],
                    );
                    assert_skip(&fx, &bundle.action, JOB_SKIP, 1);
                    assert_skip(&fx, &bundle.action, TAIL_SKIP, 1);
                    assert_eq!(target(&p, &request.key), before);
                    assert_eq!(p.next_log_activity, activity);
                    assert_eq!(
                        p.failed[&bundle.request.key()].value.as_ref().unwrap()[0].name,
                        "SECRET_BUNDLE"
                    );
                    assert!(!p.failed[&bundle.request.key()].loading);
                    if !finished_before_reply {
                        assert!(
                            receive_log(
                                &mut p,
                                &request,
                                None,
                                if read_ok {
                                    Ok(part("accepted", 50))
                                } else {
                                    Err("SECRET_ACCEPTED_ERROR".into())
                                }
                            )
                            .is_empty()
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn held_expanded_logs_survive_dispatch_and_refresh_if_activity_precedes_launch() {
    let mut p = plugin();
    install(&mut p, PROJECT, JOB);
    let earlier = launch_log(&mut p, PROJECT, JOB, true);
    receive_log(&mut p, &earlier, None, Ok(part("prefix", 0)));
    let before = target(&p, &earlier.key);
    let bundle = launch_pipeline(&mut p, PROJECT, PIPELINE);
    assert_eq!(target(&p, &earlier.key), before);
    assert_eq!(p.logs[&earlier.key].value.as_ref().unwrap().start, 0);
    let fx = receive_pipeline(
        &mut p,
        &bundle,
        Ok(vec![job(PROJECT, JOB, "refreshed")]),
        vec![(JOB, Ok(part("new tail", 80)))],
    );
    assert!(!format!("{fx:?}").contains("skipped"));
    assert_eq!(p.jobs[&earlier.key].name, "refreshed");
    assert_eq!(
        p.logs[&earlier.key].value.as_ref().unwrap().raw,
        "new tail\n"
    );
    assert_eq!(p.logs[&earlier.key].value.as_ref().unwrap().start, 80);
}

#[test]
fn bundle_eligibility_is_frozen_across_duplicate_job_and_tail_occurrences() {
    for results in [
        vec![true, false],
        vec![false, true],
        vec![true, false, true],
    ] {
        let mut p = plugin();
        let bundle = launch_pipeline(&mut p, PROJECT, PIPELINE);
        let rows = vec![job(PROJECT, JOB, "first"), job(PROJECT, JOB, "last")];
        let tails = results
            .iter()
            .enumerate()
            .map(|(n, ok)| {
                (
                    JOB,
                    if *ok {
                        Ok(part(&format!("tail {n}"), n as u64))
                    } else {
                        Err(format!("SECRET_ERROR_{n}"))
                    },
                )
            })
            .collect();
        let fx = receive_pipeline(&mut p, &bundle, Ok(rows.clone()), tails);
        assert!(!format!("{fx:?}").contains("skipped"));
        assert_eq!(p.failed[&bundle.request.key()].value.as_ref(), Some(&rows));
        let key = JobKey::new(PROJECT, JOB);
        assert_eq!(p.jobs[&key].name, "last");
        let last_success = results.iter().rposition(|ok| *ok).unwrap();
        assert_eq!(
            p.logs[&key].value.as_ref().unwrap().raw,
            format!("tail {last_success}\n")
        );
        assert_eq!(
            p.logs[&key].error.as_deref(),
            if *results.last().unwrap() {
                None
            } else {
                Some("SECRET_ERROR_1")
            }
        );
        assert_eq!(p.next_log_activity, 2 + results.len() as u64);
    }
}

#[test]
fn fenced_bundle_counts_occurrences_and_does_not_block_an_independent_tail() {
    let mut p = plugin();
    let bundle = launch_pipeline(&mut p, PROJECT, PIPELINE);
    let direct = launch_log(&mut p, PROJECT, JOB, false);
    let before = target(&p, &direct.key);
    let fx = receive_pipeline(
        &mut p,
        &bundle,
        Ok(vec![
            job(PROJECT, JOB, "SECRET_ONE"),
            job(PROJECT, JOB, "SECRET_TWO"),
        ]),
        vec![
            (JOB, Err("SECRET_ERROR".into())),
            (JOB, Ok(part("SECRET_DUPLICATE", 0))),
            (JOB + 1, Ok(part("independent tail", 0))),
        ],
    );
    assert_skip(&fx, &bundle.action, JOB_SKIP, 2);
    assert_skip(&fx, &bundle.action, TAIL_SKIP, 2);
    assert_eq!(target(&p, &direct.key), before);
    assert_eq!(
        p.logs[&JobKey::new(PROJECT, JOB + 1)]
            .value
            .as_ref()
            .unwrap()
            .raw,
        "independent tail\n"
    );
    assert_eq!(
        p.failed[&bundle.request.key()]
            .value
            .as_ref()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn earlier_exact_base_and_byte_start_are_required_and_own_owner_is_retired() {
    for change in ["base", "start", "value", "cache"] {
        for reply_ok in [false, true] {
            let mut p = plugin();
            install(&mut p, PROJECT, JOB);
            let request = launch_log(&mut p, PROJECT, JOB, true);
            match change {
                "base" => p.log_reads.get_mut(&request.key).unwrap().base += 1,
                "start" => {
                    p.logs
                        .get_mut(&request.key)
                        .unwrap()
                        .value
                        .as_mut()
                        .unwrap()
                        .start += 1
                }
                "value" => p.logs.get_mut(&request.key).unwrap().value = None,
                "cache" => {
                    p.logs.remove(&request.key);
                }
                _ => unreachable!(),
            }
            let held = format!("{:?}{:?}", p.logs, p.jobs);
            let activity = p.next_log_activity;
            let fx = receive_log(
                &mut p,
                &request,
                Some(job(OTHER, 0, "SECRET_IGNORED")),
                if reply_ok {
                    Ok(part("SECRET_PREFIX", 0))
                } else {
                    Err("SECRET_ERROR".into())
                },
            );
            assert_skip(&fx, &request.action, EARLIER_SKIP, 1);
            assert!(p.log_reads[&request.key].earlier.is_none());
            assert_eq!(format!("{:?}{:?}", p.logs, p.jobs), held);
            assert_eq!(p.next_log_activity, activity);
            let before = state(&p);
            receive_log(&mut p, &request, None, Ok(part("SECRET_DUPLICATE", 0)));
            assert_eq!(state(&p), before);
            if change == "cache" {
                assert!(!p.logs.contains_key(&request.key));
            }
        }
    }
}

#[test]
fn removed_pipeline_or_direct_cache_cannot_be_resurrected_by_its_pending_reply() {
    let mut p = plugin();
    let pipeline = launch_pipeline(&mut p, PROJECT, PIPELINE);
    p.failed.remove(&pipeline.request.key());
    let fx = receive_pipeline(
        &mut p,
        &pipeline,
        Ok(vec![job(PROJECT, JOB, "SECRET_IGNORED")]),
        vec![(JOB, Ok(part("SECRET_TAIL", 0)))],
    );
    assert_skip(&fx, &pipeline.action, PIPELINE_SKIP, 1);
    assert!(
        p.failed.is_empty() && p.failed_reads.is_empty() && p.logs.is_empty() && p.jobs.is_empty()
    );
    let direct = launch_log(&mut p, PROJECT, JOB, false);
    p.logs.remove(&direct.key);
    let fx = receive_log(
        &mut p,
        &direct,
        Some(job(PROJECT, JOB, "SECRET_IGNORED")),
        Ok(part("SECRET_TAIL", 0)),
    );
    assert_skip(&fx, &direct.action, DIRECT_SKIP, 1);
    assert!(p.logs.is_empty() && p.jobs.is_empty());
    assert!(p.log_reads[&direct.key].direct.is_none());
}

fn configure(p: &mut GitlabPlugin, edit: impl FnOnce(&mut crate::config::Settings)) {
    let mut cfg = p.cfg.clone();
    edit(&mut cfg);
    p.configure(Some(&toml::Value::try_from(cfg).unwrap()))
        .unwrap();
}

#[test]
fn same_and_unrelated_configuration_preserve_pending_owners_and_held_logs() {
    let mut p = plugin();
    install(&mut p, PROJECT, JOB);
    let earlier = launch_log(&mut p, PROJECT, JOB, true);
    let direct = launch_log(&mut p, OTHER, JOB, false);
    let pipeline = launch_pipeline(&mut p, PROJECT, PIPELINE);
    let before = state(&p);
    let stamp = p.connection_stamp();
    configure(&mut p, |_| {});
    configure(&mut p, |cfg| {
        cfg.refresh_secs += 1;
        cfg.base = "another/group".into();
        cfg.side_by_side = !cfg.side_by_side;
        cfg.history_days += 1;
    });
    assert_eq!(state(&p), before);
    assert_eq!(p.connection_stamp(), stamp);
    assert!(receive_log(&mut p, &earlier, None, Ok(part("prefix", 0))).is_empty());
    assert!(receive_log(&mut p, &direct, None, Ok(part("direct", 0))).is_empty());
    let fx = receive_pipeline(&mut p, &pipeline, Ok(vec![]), vec![]);
    assert!(!format!("{fx:?}").contains("skipped"));
}

#[test]
fn token_and_url_aba_clear_remote_caches_but_do_not_revalidate_old_owners() {
    for token_change in [false, true] {
        let mut p = plugin();
        install(&mut p, PROJECT, JOB);
        let earlier = launch_log(&mut p, PROJECT, JOB, true);
        let direct = launch_log(&mut p, OTHER, JOB, false);
        let pipeline = launch_pipeline(&mut p, PROJECT, PIPELINE);
        let original = p.cfg.clone();
        let old_stamp = p.connection_stamp();
        let next = p.next_page_read;
        let activity = p.next_log_activity;
        configure(&mut p, |cfg| {
            if token_change {
                cfg.token = "env:SECRET_OTHER_TOKEN".into();
            } else {
                cfg.url = "https://different.example".into();
            }
        });
        configure(&mut p, |cfg| {
            cfg.url = original.url.clone();
            cfg.token = original.token.clone();
        });
        assert_ne!(p.connection_stamp(), old_stamp);
        assert_eq!(p.next_page_read, next);
        assert_eq!(p.next_log_activity, activity);
        assert!(
            p.failed.is_empty()
                && p.failed_reads.is_empty()
                && p.jobs.is_empty()
                && p.logs.is_empty()
                && p.log_reads.is_empty()
        );
        let current_pipeline = launch_pipeline(&mut p, PROJECT, PIPELINE);
        let current_direct = launch_log(&mut p, OTHER, JOB, false);
        let before = state(&p);
        for old in [&earlier, &direct] {
            let fx = receive_log(
                &mut p,
                old,
                Some(job(PROJECT, JOB, "SECRET_IGNORED")),
                Err("SECRET_OLD_CONNECTION".into()),
            );
            assert_skip(
                &fx,
                &old.action,
                if old.earlier.is_some() {
                    EARLIER_SKIP
                } else {
                    DIRECT_SKIP
                },
                1,
            );
            assert_eq!(state(&p), before);
        }
        let fx = receive_pipeline(
            &mut p,
            &pipeline,
            Ok(vec![job(PROJECT, JOB, "SECRET_IGNORED")]),
            vec![(JOB, Ok(part("SECRET_OLD_CONNECTION", 0)))],
        );
        assert_skip(&fx, &pipeline.action, PIPELINE_SKIP, 1);
        assert_eq!(state(&p), before);
        assert!(receive_log(&mut p, &current_direct, None, Ok(part("current", 0))).is_empty());
        receive_pipeline(&mut p, &current_pipeline, Ok(vec![]), vec![]);
    }
}

#[test]
fn transient_owners_logs_and_failures_never_change_the_me_data_snapshot() {
    let mut p = plugin();
    p.me = "fixture-user".into();
    p.data.insert(
        PROJECT.into(),
        crate::client::GroupData {
            repo: true,
            ended_since: Some("2026-10-01T00:00:00Z".into()),
            ..Default::default()
        },
    );
    let snapshot = p.snapshot().unwrap();
    assert_eq!(
        snapshot,
        json!({"me":"fixture-user", "data":{(PROJECT): {
            "mrs":[], "projects":[], "repo":true, "ended_since":"2026-10-01T00:00:00Z"
        }}, "sync": {"saved_at": 0, "full_done": [], "synced_at": {}}})
    );
    install(&mut p, PROJECT, JOB);
    let earlier = launch_log(&mut p, PROJECT, JOB, true);
    let pipeline = launch_pipeline(&mut p, PROJECT, PIPELINE);
    receive_pipeline(
        &mut p,
        &pipeline,
        Ok(vec![job(PROJECT, JOB, "SECRET_TITLE")]),
        vec![(JOB, Err("SECRET_ERROR".into()))],
    );
    assert_eq!(p.snapshot(), Some(snapshot.clone()));
    receive_log(&mut p, &earlier, None, Ok(part("SECRET_PREFIX", 0)));
    assert_eq!(p.snapshot(), Some(snapshot.clone()));
    configure(&mut p, |cfg| {
        cfg.token = "env:SECRET_REPLACEMENT_TOKEN".into()
    });
    assert_eq!(p.snapshot(), Some(snapshot.clone()));
    let mut restored = plugin();
    restored.restore(snapshot.clone());
    assert_eq!(restored.snapshot(), Some(snapshot));
    assert!(
        restored.failed.is_empty()
            && restored.failed_reads.is_empty()
            && restored.jobs.is_empty()
            && restored.logs.is_empty()
            && restored.log_reads.is_empty()
    );
    assert_eq!(restored.next_page_read, 0);
    assert_eq!(restored.next_log_activity, 0);
}

#[test]
fn job_skip_uses_original_sanitized_request_scope_without_local_ids_or_payloads() {
    for is_earlier in [false, true] {
        let mut p = plugin();
        p.cfg.url =
            "https://SECRET_USER:SECRET_PASSWORD@git.example/?token=SECRET_TOKEN#SECRET_FRAGMENT"
                .into();
        p.cfg.token = "SECRET_CONFIGURED_TOKEN".into();
        install(&mut p, PROJECT, JOB);
        p.next_page_read = 876_543_210;
        p.next_log_activity = 987_654_320;
        let request = launch_log(&mut p, PROJECT, JOB, is_earlier);
        let scope = request.action.clone();
        configure(&mut p, |cfg| cfg.url = "https://replacement.example".into());
        let fx = receive_log(
            &mut p,
            &request,
            Some(job(
                "SECRET_RESPONSE_PROJECT",
                123456789,
                "SECRET_RESPONSE_NAME",
            )),
            Err("SECRET_PROVIDER_PAYLOAD".into()),
        );
        assert_eq!(fx.len(), 1);
        assert_skip(
            &fx,
            &scope,
            if is_earlier {
                EARLIER_SKIP
            } else {
                DIRECT_SKIP
            },
            1,
        );
        assert_eq!(scope.subjects.len(), 1);
        assert_eq!(scope.subjects[0].key.instance, "https://git.example/");
        assert_eq!(scope.subjects[0].key.id, format!("{PROJECT}/{JOB}"));
        assert_eq!(scope.subjects[0].key.kind, crate::join::JOB);
        assert!(scope.subjects[0].url.is_none());
        let rendered = format!("{fx:?}");
        for forbidden in [
            "SECRET",
            "replacement.example",
            "876543211",
            "987654321",
            "123456789",
            "owner",
            "connection",
            "activity",
            "before",
            "base",
        ] {
            assert!(
                !rendered.contains(forbidden),
                "leaked {forbidden}: {rendered}"
            );
        }
    }
}

#[test]
fn unsafe_job_scope_is_omitted_without_affecting_stale_read_outcome() {
    for (project, id) in [("acme/../SECRET_PATH", JOB), (PROJECT, 0)] {
        let mut p = plugin();
        let request = launch_log(&mut p, project, id, false);
        p.log_reads.get_mut(&request.key).unwrap().direct = None;
        let fx = receive_log(&mut p, &request, None, Err("SECRET_ERROR".into()));
        assert_skip(&fx, &request.action, DIRECT_SKIP, 1);
        let [Effect::Observe { action, .. }] = &fx[..] else {
            panic!("one stale observation")
        };
        assert!(action.subjects.is_empty());
        assert!(action.metadata.redacted);
        assert_eq!(action.metadata.omitted_subjects, 1);
    }
}

#[test]
fn seed_metadata_is_one_time_activity_and_never_replaces_a_known_remote_job() {
    let mut p = plugin();
    let key = JobKey::new(PROJECT, JOB);
    seed(&mut p, key.clone(), job(PROJECT, JOB, "seed"));
    let activity = p.next_log_activity;
    assert!(activity > 0);
    seed(
        &mut p,
        key.clone(),
        job(PROJECT, JOB, "SECRET_REPLACEMENT_SEED"),
    );
    assert_eq!(p.next_log_activity, activity);
    assert_eq!(p.jobs[&key].name, "seed");
    let request = launch_log(&mut p, PROJECT, JOB, false);
    receive_log(
        &mut p,
        &request,
        Some(job(PROJECT, JOB, "remote")),
        Ok(part("log", 0)),
    );
    let before = state(&p);
    seed(&mut p, key.clone(), job(PROJECT, JOB, "SECRET_STALE_SEED"));
    assert_eq!(state(&p), before);
    assert_eq!(p.jobs[&key].name, "remote");
    assert_eq!(
        p.logs[&key].value.as_ref(),
        Some(&JobLog::new(part("log", 0)))
    );
}

#[test]
fn overlapping_pipeline_bundles_share_job_activity_without_sharing_pipeline_owners() {
    for order in [
        "first completes first",
        "second completes first",
        "second launched later",
    ] {
        let mut p = plugin();
        let first = launch_pipeline(&mut p, PROJECT, PIPELINE);
        let second = if order == "second launched later" {
            receive_pipeline(
                &mut p,
                &first,
                Ok(vec![job(PROJECT, JOB, "first")]),
                vec![(JOB, Ok(part("first", 0)))],
            );
            launch_pipeline(&mut p, PROJECT, PIPELINE + 1)
        } else {
            launch_pipeline(&mut p, PROJECT, PIPELINE + 1)
        };
        let (completes_first, finishes_last, held_title, last_title) =
            if order == "second completes first" {
                (&second, &first, "second", "first")
            } else {
                (&first, &second, "first", "second")
            };
        if order != "second launched later" {
            let fx = receive_pipeline(
                &mut p,
                completes_first,
                Ok(vec![job(PROJECT, JOB, held_title)]),
                vec![(JOB, Ok(part(held_title, 0)))],
            );
            assert!(!format!("{fx:?}").contains("skipped"));
        }
        let key = JobKey::new(PROJECT, JOB);
        let held = target(&p, &key);
        let fx = receive_pipeline(
            &mut p,
            finishes_last,
            Ok(vec![job(PROJECT, JOB, last_title)]),
            vec![(JOB, Ok(part(last_title, 0)))],
        );
        if order == "second launched later" {
            assert!(!format!("{fx:?}").contains("skipped"));
            assert_eq!(p.jobs[&key].name, "second");
            assert_eq!(p.logs[&key].value.as_ref().unwrap().raw, "second\n");
        } else {
            assert_skip(&fx, &finishes_last.action, JOB_SKIP, 1);
            assert_skip(&fx, &finishes_last.action, TAIL_SKIP, 1);
            assert_eq!(target(&p, &key), held);
        }
        assert!(p.failed_reads.is_empty());
        assert_eq!(
            p.failed[&first.request.key()].value.as_ref().unwrap()[0].name,
            "first"
        );
        assert_eq!(
            p.failed[&second.request.key()].value.as_ref().unwrap()[0].name,
            "second"
        );
    }
}

#[test]
fn equal_parsed_tail_and_start_do_not_make_a_replaced_earlier_base_current() {
    let mut p = plugin();
    install(&mut p, PROJECT, JOB);
    let request = launch_log(&mut p, PROJECT, JOB, true);
    let replacement = JobLog::new(part("different fragment\nheld", 100));
    let original = p.logs[&request.key].value.as_ref().unwrap();
    assert_eq!(replacement.text, original.text);
    assert_eq!(replacement.start, original.start);
    assert_ne!(replacement.raw, original.raw);
    // Model an independently replaced held revision. The reducer normally also
    // retires this owner, but equal parsed text alone must never validate it.
    p.logs.get_mut(&request.key).unwrap().value = Some(replacement);
    activity(&mut p, &request.key, true);
    assert_ne!(
        p.log_reads[&request.key].base,
        request.earlier.unwrap().base
    );
    let held = format!("{:?}{:?}", p.logs, p.jobs);
    let current_activity = p.next_log_activity;
    let fx = receive_log(&mut p, &request, None, Ok(part("SECRET_OLD_PREFIX", 0)));
    assert_skip(&fx, &request.action, EARLIER_SKIP, 1);
    assert_eq!(format!("{:?}{:?}", p.logs, p.jobs), held);
    assert_eq!(p.next_log_activity, current_activity);
    assert!(!earlier_loading(&p, &request.key));
}

#[test]
fn nonforced_diagnosis_noops_preserve_request_ownership_and_activity() {
    let mut p = plugin();
    let request = launch_pipeline(&mut p, PROJECT, PIPELINE);
    let before = state(&p);
    for logs in [false, true] {
        assert!(why::diagnose(&mut p, PROJECT, PIPELINE, logs, false).is_empty());
        assert_eq!(state(&p), before);
    }
    receive_pipeline(
        &mut p,
        &request,
        Ok(vec![job(PROJECT, JOB, "held")]),
        vec![(JOB, Ok(part("held", 0)))],
    );
    let before = state(&p);
    for logs in [false, true] {
        assert!(why::diagnose(&mut p, PROJECT, PIPELINE, logs, false).is_empty());
        assert_eq!(state(&p), before);
    }
}
