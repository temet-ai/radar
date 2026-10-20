//! Owned synthetic provider completions; production tasks are dropped unpolled.

use super::*;
use crate::client::recorded::{Replay, block};
use crate::client::{Client, FailedJobsRead, MrDetailRead};
use crate::log_ownership::PipelineRead;
use radar_core::host::{Event, HostEffect};
use radar_core::view::{Intent, Target};
use serde_json::json;

const API: &str = "acme/team-a/api";
const WEB: &str = "acme/team-a/web";
const IID: u64 = 263;
const DETAILS_ERROR: &str = "details unavailable";
const DIAGNOSIS_ERROR: &str = "diagnosis unavailable";
const DETAILS_NOTICE: &str = "merge request details refresh failed:";

fn plugin(host: &mut radar_core::Host) -> &mut GitlabPlugin {
    assert_eq!(host.plugins[0].id(), "gitlab");
    // SAFETY: these hosts contain exactly this plugin type.
    unsafe { &mut *(host.plugins[0].as_mut() as *mut dyn Plugin as *mut GitlabPlugin) }
}

fn host(p: GitlabPlugin) -> radar_core::Host {
    let mut host = radar_core::Host::new(vec![Box::new(sample())], p.now).unwrap();
    host.load_config(Some("")).unwrap();
    host.plugins[0] = Box::new(p);
    host
}

fn kind() -> Kind {
    Kind::Mr {
        project: API.into(),
        iid: IID,
    }
}

fn start_detail(p: &mut GitlabPlugin) {
    assert!(fetch(p, &kind(), true).iter().any(|fx|
        matches!(fx, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.page.detail")));
}

fn receive_detail(p: &mut GitlabPlugin, detail: Result<MrDetail, String>) {
    let key = mr_key(API, IID);
    let result = detail.map(|detail| {
        // Keep the real parser's lossless requested-identity evidence.
        let replay = Replay::new(vec![(
            200,
            json!({"data":{"project":{"mergeRequest":{
                "iid": IID.to_string(), "project":{"fullPath":API}
            }}}}),
        )]);
        let mut got: MrDetailRead =
            block(Client::over(replay).mr_detail_observed(API, IID)).unwrap();
        got.mr = find_mr(p, API, IID).unwrap().clone();
        got.detail = detail;
        Box::new(got)
    });
    p.on_msg(Box::new(GlMsg::MrPage {
        read: p.mr_page_reads[&key],
        key,
        requested: crate::page_identity::Requested::new(API, IID),
        action: crate::page_history::action(&p.cfg.url, API, IID),
        result,
    }));
}

fn detail_fixture(empty: bool) -> MrDetail {
    let mut d = fixtures::detail();
    if empty {
        d.files.clear();
        d.discussions.clear();
    }
    d
}

fn receive_diffs(p: &mut GitlabPlugin, result: Result<Vec<crate::model::FileDiff>, String>) {
    let key = mr_key(API, IID);
    let ds = &p.diffs[&key];
    p.on_msg(Box::new(GlMsg::Diffs {
        read: ds.requests[&1],
        head: ds.head.clone(),
        key,
        page: 1,
        action: crate::diff_history::action(&p.cfg.url, API, IID, 1),
        result: result.map(|files| crate::client::DiffRead {
            files,
            evidence: Default::default(),
        }),
    }));
}

fn mr_host(tab: usize, empty: bool) -> radar_core::Host {
    let mut p = sample();
    push(&mut p, kind());
    receive_detail(&mut p, Ok(detail_fixture(empty)));
    p.pages[0].tab = tab;
    p.pages[0].placed.set(true);
    if tab == mr::CHANGES && !empty {
        drop(changes::want(&mut p));
        receive_diffs(&mut p, Ok(fixtures::diffs()));
    }
    host(p)
}

fn fail_detail(p: &mut GitlabPlugin) {
    let key = mr_key(API, IID);
    let held = format!("{:?}", p.mr_pages[&key].value);
    let at = p.mr_pages[&key].at;
    start_detail(p);
    receive_detail(p, Err(DETAILS_ERROR.into()));
    assert_eq!(format!("{:?}", p.mr_pages[&key].value), held);
    assert_eq!(p.mr_pages[&key].at, at);
    assert!(!p.mr_pages[&key].loading);
}

fn start_diagnosis(p: &mut GitlabPlugin) -> PipelineRead {
    assert!(why::diagnose(p, WEB, 8001, true, true).iter().any(|fx|
        matches!(fx, Effect::SpawnAction { action, .. } if action.kind.as_str() == "gitlab.pipeline.diagnosis")));
    PipelineRead {
        project: WEB.into(),
        pipeline: 8001,
        owner: p.failed_reads[&why::key(WEB, 8001)],
        activity: p.next_log_activity,
    }
}

fn receive_diagnosis(
    p: &mut GitlabPlugin,
    request: PipelineRead,
    result: Result<Vec<crate::model::JobInfo>, String>,
) {
    p.on_msg(Box::new(GlMsg::Failed {
        request,
        action: crate::failure_history::action(&p.cfg.url, WEB, 8001),
        read: why::DiagnosisRead {
            result: result.map(|jobs| FailedJobsRead {
                jobs,
                evidence: Default::default(),
            }),
            logs: None,
        },
    }));
}

fn repo_host(empty: bool) -> radar_core::Host {
    let mut p = sample();
    let jobs = if empty {
        vec![]
    } else {
        why::failed_rows(&p, WEB, 8001)
    };
    push(&mut p, Kind::Repo(WEB.into()));
    p.pages[0].tab = repo::PIPELINES;
    let request = start_diagnosis(&mut p);
    receive_diagnosis(&mut p, request, Ok(jobs));
    p.runs.insert(
        WEB.into(),
        Fetched {
            value: Some(fixtures::runs()),
            at: p.now,
            ..Default::default()
        },
    );
    host(p)
}

fn fail_diagnosis(p: &mut GitlabPlugin) {
    let key = why::key(WEB, 8001);
    let held = format!("{:?}", p.failed[&key].value);
    let at = p.failed[&key].at;
    let request = start_diagnosis(p);
    receive_diagnosis(p, request, Err(DIAGNOSIS_ERROR.into()));
    assert_eq!(format!("{:?}", p.failed[&key].value), held);
    assert_eq!(p.failed[&key].at, at);
    assert!(!p.failed[&key].loading);
}

fn body(text: &str) -> Vec<&str> {
    text.lines().skip(1).collect()
}

#[test]
fn mr_detail_refresh_error_survives_cached_empty_and_nonempty_retry_then_clears() {
    let (w, h) = (160, 40);
    for (tab, name) in [(mr::THREADS, "threads"), (mr::CHANGES, "changes")] {
        for empty in [false, true] {
            let mut host = mr_host(tab, empty);
            let before = crate::tests::shown(&host, w, h);
            start_detail(plugin(&mut host));
            assert_eq!(
                body(&crate::tests::shown(&host, w, h)),
                body(&before),
                "healthy cached reads add no loading line"
            );
            receive_detail(plugin(&mut host), Err(DETAILS_ERROR.into()));
            let shown = crate::tests::shown(&host, w, h);
            assert!(
                shown.contains(DETAILS_NOTICE) && shown.contains(DETAILS_ERROR),
                "{shown}"
            );
            assert!(
                shown.contains(match (tab, empty) {
                    (mr::THREADS, true) => "comment on !263",
                    (mr::THREADS, false) => "Should this read from the cached view?",
                    (_, true) => "This merge request changes no files.",
                    (_, false) => "package acme.stats;",
                }),
                "{shown}"
            );
            // The notice is in the error tone, as the view names it.
            assert_eq!(crate::tests::ink_of(&host, DETAILS_NOTICE).as_deref(), Some("bad"));
            let label = format!("{name}-{}", if empty { "empty" } else { "cached" });
            insta::assert_snapshot!(format!("detail_refresh_{label}"), shown);
            start_detail(plugin(&mut host));
            assert!(crate::tests::shown(&host, w, h).contains(DETAILS_ERROR));
            receive_detail(plugin(&mut host), Ok(detail_fixture(empty)));
            assert!(
                plugin(&mut host).mr_pages[&mr_key(API, IID)]
                    .error
                    .is_none()
            );
            assert_eq!(body(&crate::tests::shown(&host, w, h)), body(&before));
        }
    }
}

#[test]
fn repository_diagnosis_error_survives_cached_empty_and_nonempty_retry_then_clears() {
    let (w, h) = (160, 40);
    for empty in [false, true] {
        let mut host = repo_host(empty);
        let jobs = why::failed_rows(plugin(&mut host), WEB, 8001);
        let before = crate::tests::shown(&host, w, h);
        fail_diagnosis(plugin(&mut host));
        let shown = crate::tests::shown(&host, w, h);
        assert!(
            shown.contains(DIAGNOSIS_ERROR) && shown.contains("#1842"),
            "{shown}"
        );
        if !empty {
            assert!(shown.contains("unit-test"), "{shown}");
        }
        assert_eq!(crate::tests::ink_of(&host, DIAGNOSIS_ERROR).as_deref(), Some("bad"));
        let label = if empty {
            "diagnosis-empty"
        } else {
            "diagnosis-cached"
        };
        insta::assert_snapshot!(format!("detail_refresh_{label}"), shown);
        let request = start_diagnosis(plugin(&mut host));
        assert!(crate::tests::shown(&host, w, h).contains(DIAGNOSIS_ERROR));
        receive_diagnosis(plugin(&mut host), request, Ok(jobs));
        assert_eq!(body(&crate::tests::shown(&host, w, h)), body(&before));
    }
}

#[test]
fn cold_detail_and_diagnosis_failures_still_show_and_diff_failure_is_independent() {
    let (w, h) = (160, 40);
    for tab in [mr::THREADS, mr::CHANGES] {
        let mut p = sample();
        push(&mut p, kind());
        p.pages[0].tab = tab;
        receive_detail(&mut p, Err(DETAILS_ERROR.into()));
        let shown = draw(&p, w, h);
        assert!(
            shown.contains("could not load: details unavailable"),
            "{shown}"
        );
        assert!(!shown.contains(DETAILS_NOTICE));
    }
    let mut host = repo_host(false);
    plugin(&mut host)
        .failed
        .get_mut(&why::key(WEB, 8001))
        .unwrap()
        .value = None;
    fail_diagnosis(plugin(&mut host));
    let shown = crate::tests::shown(&host, w, h);
    assert!(
        shown.contains(DIAGNOSIS_ERROR) && !shown.contains("unit-test"),
        "{shown}"
    );

    let mut host = mr_host(mr::THREADS, false);
    let p = plugin(&mut host);
    p.pages[0].tab = mr::CHANGES;
    drop(changes::want(p));
    fail_detail(p);
    let loading = crate::tests::shown(&host, w, h);
    assert!(loading.contains(DETAILS_NOTICE) && loading.contains("loading…"));
    receive_diffs(plugin(&mut host), Err("diff unavailable".into()));
    let shown = crate::tests::shown(&host, w, h);
    assert!(
        shown.contains(DETAILS_NOTICE) && shown.contains("could not load: diff unavailable"),
        "{shown}"
    );
}

/// The `row:<n>` targets the text form shows, with the line of each.
fn row_targets(shown: &str) -> Vec<(String, String)> {
    shown
        .lines()
        .filter_map(|line| {
            let (_, target) = line.rsplit_once('{')?;
            let target = target.strip_suffix('}')?;
            target.starts_with("row:").then(|| (target.to_string(), line.to_string()))
        })
        .collect()
}

fn select(host: &mut radar_core::Host, target: &str) {
    host.update(Event::Intent(Intent::Select { target: Target::new(target) }));
}

fn enter(host: &mut radar_core::Host) -> Vec<HostEffect> {
    host.update(Event::Key(KeyEvent::new(
        KeyCode::Enter,
        KeyModifiers::NONE,
    )))
}

#[test]
fn detail_error_is_not_a_row_and_thread_comment_and_file_targets_keep_their_identity() {
    // The files pane is shown at the wide size and folded away at the narrow
    // one, so the view differs: both sizes.
    for (w, h) in [(80, 24), (160, 40)] {
        let mut host = mr_host(mr::THREADS, false);
        fail_detail(plugin(&mut host));
        let shown = crate::tests::shown(&host, w, h);
        let rows = row_targets(&shown);
        assert!(
            rows.iter().all(|(_, line)| !line.contains(DETAILS_NOTICE)),
            "the error is not a row:\n{shown}"
        );
        assert!(rows.iter().any(|(id, _)| id == "row:1"), "{shown}");
        select(&mut host, "row:1");
        assert_eq!(plugin(&mut host).pages[0].row.get(), 1);
        enter(&mut host);
        let p = plugin(&mut host);
        assert_eq!(p.pages[0].tab, mr::CHANGES);
        assert_eq!(
            p.pages[0].file.get(),
            0,
            "thread still opens its controller file"
        );
        receive_diffs(p, Ok(fixtures::diffs()));
        let shown = crate::tests::shown(&host, w, h);
        assert!(shown.contains(DETAILS_NOTICE));
        if w == 160 {
            assert!(files_beside_the_diff(plugin(&mut host), w));
            assert!(shown.contains("/f:2}"), "{shown}");
            select(&mut host, "node:f:2");
            assert_eq!(plugin(&mut host).pages[0].file.get(), 2);
            enter(&mut host);
            assert_eq!(plugin(&mut host).pages[0].file.get(), 2);
        } else {
            // At narrow width files collapse; the diff's line targets remain exact.
            assert!(!files_beside_the_diff(plugin(&mut host), w));
            let index = 2;
            select(&mut host, &format!("line:{index}"));
            assert_eq!(plugin(&mut host).pages[0].diff.cursor(), index);
            enter(&mut host);
            assert!(matches!(plugin(&mut host).compose.as_ref().unwrap().target,
                crate::write::Target::Line(ref at) if at.new == Some(index as u32)));
        }
        let mut host = mr_host(mr::THREADS, true);
        fail_detail(plugin(&mut host));
        crate::tests::shown(&host, w, h);
        select(&mut host, "row:0");
        enter(&mut host);
        assert!(
            plugin(&mut host).compose.is_none(),
            "Enter on comment row stays a no-op"
        );
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::NONE,
        )));
        assert!(matches!(
            plugin(&mut host).compose.as_ref().unwrap().target,
            crate::write::Target::Note
        ));
    }
}

#[test]
fn diagnosis_error_is_not_a_row_and_job_and_pipeline_targets_keep_their_offsets() {
    let mut host = repo_host(false);
    fail_diagnosis(plugin(&mut host));
    let shown = crate::tests::shown(&host, 160, 40);
    let rows = row_targets(&shown);
    assert!(
        rows.iter().all(|(_, line)| !line.contains(DIAGNOSIS_ERROR)),
        "the error is not a row:\n{shown}"
    );
    let (id, line) = rows.iter().find(|(id, _)| id == "row:2").unwrap();
    assert!(line.contains("#1838"), "{line}");
    select(&mut host, id);
    assert_eq!(plugin(&mut host).pages[0].row.get(), 2);
    assert!(
        enter(&mut host)
            .iter()
            .any(|fx| matches!(fx, HostEffect::Open(url) if url.ends_with("/-/pipelines/1")))
    );
    select(&mut host, "row:0");
    enter(&mut host);
    assert_eq!(
        plugin(&mut host).pages.last().unwrap().kind,
        Kind::Job {
            project: WEB.into(),
            id: 5001
        }
    );
}
