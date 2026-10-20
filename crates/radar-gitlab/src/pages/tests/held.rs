//! What is fetched is kept by what it was made from, not by a timer: a
//! merge request's page by the `updatedAt` the sync shows, a finished
//! pipeline's failed jobs for the session; a diagnosis waits for the
//! selection to rest; diffs come twenty files a request.

use super::*;
use crate::client::DIFF_PAGE;
use crate::model::{FileDiff, FileStat, Pipeline, Stage};

const API: &str = "acme/team-a/api";
const WORKER: &str = "acme/team-b/worker";

fn kind() -> Kind {
    Kind::Mr { project: API.into(), iid: 263 }
}

fn asks(fx: &[Effect], what: &str) -> usize {
    fx.iter()
        .filter(|e| matches!(e, Effect::SpawnAction { action, .. } if action.kind.as_str() == what))
        .count()
}

fn listed(p: &mut GitlabPlugin) -> &mut Mr {
    p.data
        .values_mut()
        .flat_map(|d| d.mrs.iter_mut())
        .find(|m| m.project_path == API && m.iid == 263)
        .unwrap()
}

/// !263's page, its data held, closed again, a long time later.
fn closed_and_later() -> GitlabPlugin {
    let mut p = mr_page();
    listed(&mut p).pipeline = Some(Pipeline::Passed);
    // The detail was made from the merge request as listed now.
    let mr = listed(&mut p).clone();
    p.mr_pages.get_mut(&mr_key(API, 263)).unwrap().value.as_mut().unwrap().0 = mr;
    p.pages.clear();
    p.now += 100 * p.cfg.refresh_secs as i64;
    p
}

#[test]
fn reopening_a_merge_request_the_sync_shows_unchanged_costs_no_request() {
    let mut p = closed_and_later();
    let fx = push(&mut p, kind());
    assert_eq!(asks(&fx, "gitlab.page.detail"), 0, "{fx:?}");
}

#[test]
fn a_changed_updated_at_or_pipeline_asks_again() {
    let mut p = closed_and_later();
    listed(&mut p).updated_at = "2027-01-01T00:00:00Z".into();
    let fx = push(&mut p, kind());
    assert_eq!(asks(&fx, "gitlab.page.detail"), 1, "{fx:?}");
    // The sync shows a pipeline that went red since.
    let mut p = closed_and_later();
    listed(&mut p).pipeline = Some(Pipeline::Failed);
    let fx = push(&mut p, kind());
    assert_eq!(asks(&fx, "gitlab.page.detail"), 1, "{fx:?}");
}

#[test]
fn r_on_the_page_asks_though_nothing_changed() {
    let mut p = closed_and_later();
    push(&mut p, kind());
    let fx = press(&mut p, Key::Char('r'));
    assert_eq!(asks(&fx, "gitlab.page.detail"), 1, "{fx:?}");
}

#[test]
fn a_running_pipeline_keeps_the_timer() {
    let mut p = closed_and_later();
    let key = mr_key(API, 263);
    p.mr_pages.get_mut(&key).unwrap().value.as_mut().unwrap().0.pipeline = Some(Pipeline::Running);
    listed(&mut p).pipeline = Some(Pipeline::Running);
    p.mr_pages.get_mut(&key).unwrap().at = p.now;
    assert_eq!(asks(&push(&mut p, kind()), "gitlab.page.detail"), 0, "held a moment");
    p.pages.clear();
    p.now += 100 * p.cfg.refresh_secs as i64;
    assert_eq!(asks(&push(&mut p, kind()), "gitlab.page.detail"), 1, "then asked again");
}

#[test]
fn a_finished_pipelines_failed_jobs_are_not_asked_again_by_time() {
    let mut p = sample();
    assert!(p.failed.contains_key(&why::key(WORKER, 9001)), "held");
    p.now += 1000 * p.cfg.refresh_secs as i64;
    assert!(why::diagnose(&mut p, WORKER, 9001, false, false).is_empty());
    // It ran again (a retry keeps the id): the list shows other stages.
    p.data
        .values_mut()
        .flat_map(|d| d.mrs.iter_mut())
        .find(|m| m.iid == 342)
        .unwrap()
        .stages = vec![Stage { name: "test".into(), status: "running".into() }];
    let fx = why::diagnose(&mut p, WORKER, 9001, false, false);
    assert_eq!(asks(&fx, "gitlab.pipeline.diagnosis"), 1, "{fx:?}");
}

fn files(n: usize) -> Vec<FileStat> {
    (0..n)
        .map(|i| FileStat { path: format!("f{i}"), additions: 1, deletions: 0 })
        .collect()
}

/// !263's Changes tab over `n` files, the open one `at`.
fn changes_of(n: usize, at: usize) -> GitlabPlugin {
    let mut p = mr_page();
    let key = mr_key(API, 263);
    let (_, d) = p.mr_pages.get_mut(&key).unwrap().value.as_mut().unwrap();
    d.files = files(n);
    d.refs = Some(crate::model::DiffRefs { head: "h".into(), ..Default::default() });
    let page = p.pages.last_mut().unwrap();
    page.tab = mr::CHANGES;
    page.placed.set(true);
    page.file.set(at);
    p
}

fn arrive(p: &mut GitlabPlugin, page: usize, paths: impl Iterator<Item = String>) -> Vec<Effect> {
    let key = mr_key(API, 263);
    let rows = paths.map(|path| FileDiff { new_path: path.clone(), old_path: path, ..Default::default() });
    changes::on_diffs(p, key, "h".into(), page, Ok(rows.collect()))
}

#[test]
fn the_page_of_the_open_file_is_asked_with_twenty_files_a_request() {
    assert_eq!(DIFF_PAGE, 20);
    let mut p = changes_of(100, 45);
    drop(changes::want(&mut p));
    let ds = &p.diffs[&mr_key(API, 263)];
    assert_eq!(ds.asked.iter().copied().collect::<Vec<_>>(), [3], "file 45 is on page 3");
}

#[test]
fn a_hundred_files_take_five_requests_not_twenty() {
    // The open file never arrives, so every page is asked once.
    let mut p = changes_of(100, 99);
    let mut requests = asks(&changes::want(&mut p), "gitlab.page.diffs");
    for page in [5, 1, 2, 3, 4] {
        let at = (page - 1) * DIFF_PAGE;
        let fx = arrive(&mut p, page, (at..at + DIFF_PAGE).map(|i| format!("x{i}")));
        requests += asks(&fx, "gitlab.page.diffs");
    }
    // Full pages never end the paging; five of them cover a hundred files.
    assert_eq!(requests, 5);
    assert_eq!(p.diffs[&mr_key(API, 263)].asked.len(), 5);
}

#[test]
fn moving_past_red_rows_quickly_asks_only_about_the_one_it_rests_on() {
    let mut p = sample();
    p.sync.last_started = Some(p.now);
    p.failed.clear();
    why::reset(&mut p);
    why::rest_on(&mut p, "acme/team-a/web", 8001);
    p.tick(p.now);
    why::rest_on(&mut p, WORKER, 9001);
    let mut fx = vec![];
    for _ in 0..4 {
        fx.extend(p.tick(p.now));
    }
    assert_eq!(asks(&fx, "gitlab.pipeline.diagnosis"), 1, "{fx:?}");
    assert!(p.failed.contains_key(&why::key(WORKER, 9001)));
    assert!(!p.failed.contains_key(&why::key("acme/team-a/web", 8001)));
    // Held ones are not waited for or asked.
    why::rest_on(&mut p, WORKER, 9001);
    let fx: Vec<_> = (0..4).flat_map(|_| p.tick(p.now)).collect();
    assert_eq!(asks(&fx, "gitlab.pipeline.diagnosis"), 0);
}
