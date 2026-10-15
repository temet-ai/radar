//! Why a pipeline is red: its failed jobs, each with its stage, how long it
//! ran, GitLab's reason and the lines of its log around the first error.
//! Fetched for a merge request's page, a repo's page (main's pipeline) and,
//! without the logs, for the detail pane of a red one; the job logs are kept
//! for the job's page.
//!
//! GitLab's jobs API (REST) has what its GraphQL lacks here: a job's
//! `failure_reason`, and its log by byte range (the last part first).

use radar_core::plugin::Effect;

use crate::client::FailedJobsRead;
use crate::joblog::{self, JobLog};
use crate::log_ownership::{self, JobKey, LogRead, PipelineRead};
use crate::model::{JobInfo, Pipeline, TracePart};
use crate::GitlabPlugin;

/// At most this many failed jobs have their logs fetched for a diagnosis.
const LOGS: usize = 5;

/// One existing bounded bundle, not proof of complete logs or a root cause.
/// None distinguishes tails not requested from a selected empty collection.
pub(crate) struct DiagnosisRead {
    pub result: Result<FailedJobsRead, String>,
    pub logs: Option<Vec<(u64, Result<TracePart, String>)>>,
}

/// Keep list and tail authentication boundaries separate. Tail errors do not
/// prevent later selected occurrences (including duplicate IDs) from being read.
pub(super) async fn read<Fut>(
    logs: bool,
    list: impl std::future::Future<Output = Result<FailedJobsRead, String>>,
    mut trace: impl FnMut(u64) -> Fut,
) -> DiagnosisRead
where
    Fut: std::future::Future<Output = Result<TracePart, String>>,
{
    let result = list.await;
    let mut traces = logs.then(Vec::new);
    if let (Some(traces), Ok(got)) = (&mut traces, &result) {
        for job in ranked(&got.jobs).into_iter().take(LOGS) {
            traces.push((job.id, trace(job.id).await));
        }
    }
    DiagnosisRead {
        result,
        logs: traces,
    }
}

/// A selection rests on a row for this many host ticks (200 ms each: a
/// move lands between two, so 200 to 400 ms) before its failed jobs are asked.
const REST: u64 = 2;

/// What the plugin remembers about diagnoses: the host's tick count (the
/// only clock finer than a second), the diagnosis waiting for the selection
/// to rest, and what each held diagnosis was made from.
#[derive(Debug, Default)]
pub(crate) struct Watch {
    ticks: u64,
    /// Project, pipeline and the tick it is due at.
    wait: Option<(String, u64, u64)>,
    /// The pipeline as the list showed it when its jobs were fetched.
    seen: std::collections::BTreeMap<String, String>,
}

/// What the list shows of a pipeline that changes when it runs again (a
/// retry keeps its id): the merge request's duration and stages. Empty when
/// no merge request shows it.
fn signature(p: &GitlabPlugin, project: &str, pipeline: u64) -> String {
    p.data
        .values()
        .flat_map(|d| d.mrs.iter())
        .find(|m| {
            m.project_path == project
                && m.pipeline_url.as_deref().and_then(crate::client::pipeline_id_of) == Some(pipeline)
        })
        .map(|m| format!("{:?}{:?}", m.duration_secs, m.stages))
        .unwrap_or_default()
}

/// The failed jobs are held, or being fetched: nothing to ask.
fn held(p: &GitlabPlugin, project: &str, pipeline: u64) -> bool {
    let k = key(project, pipeline);
    p.failed.get(&k).is_some_and(|f| {
        f.loading
            || (f.value.is_some()
                && p.watch.seen.get(&k).map_or("", String::as_str) == signature(p, project, pipeline))
    })
}

/// The selection is on a red pipeline: ask which jobs failed once it has
/// rested there, not as it passes. Nothing is waited for when they are held.
pub(crate) fn rest_on(p: &mut GitlabPlugin, project: &str, pipeline: u64) {
    p.watch.wait = (!held(p, project, pipeline))
        .then(|| (project.to_string(), pipeline, p.watch.ticks + REST));
}

/// Jobs put in `failed` by hand (the demo): they were made from the list as it is.
#[cfg(any(test, feature = "demo"))]
pub(crate) fn made_from_list(p: &mut GitlabPlugin, project: &str, pipeline: u64) {
    let sig = signature(p, project, pipeline);
    p.watch.seen.insert(key(project, pipeline), sig);
}

/// Held diagnoses are gone: nothing is made from them or waited for.
pub(crate) fn reset(p: &mut GitlabPlugin) {
    p.watch.wait = None;
    p.watch.seen.clear();
}

/// The selection is on something that is not red.
pub(crate) fn rest_off(p: &mut GitlabPlugin) {
    p.watch.wait = None;
}

/// A host tick: the diagnosis whose row the selection rested on is asked.
pub(crate) fn rested(p: &mut GitlabPlugin) -> Vec<Effect> {
    p.watch.ticks += 1;
    if p.watch.wait.as_ref().is_none_or(|(_, _, due)| p.watch.ticks < *due) {
        return vec![];
    }
    let (project, pipeline, _) = p.watch.wait.take().expect("checked");
    diagnose(p, &project, pipeline, false, false)
}

/// The key a pipeline's diagnosis is kept by.
pub(crate) fn key(project: &str, pipeline: u64) -> String {
    format!("{project}#{pipeline}")
}

/// A pipeline's failed jobs, as last fetched.
pub(crate) fn failed<'a>(p: &'a GitlabPlugin, project: &str, pipeline: u64) -> Option<&'a [JobInfo]> {
    p.failed
        .get(&key(project, pipeline))
        .and_then(|f| f.value.as_deref())
}

/// The ones that made it red first (not allowed to fail), GitLab's order kept.
fn ranked(jobs: &[JobInfo]) -> Vec<&JobInfo> {
    let mut out: Vec<&JobInfo> = jobs.iter().collect();
    out.sort_by_key(|j| j.allow_failure);
    out
}

/// Fetch a pipeline's failed jobs and, with `logs`, the last part of the
/// first few one's logs; unless they are held (or `force`). Only red
/// pipelines are asked about, and a finished pipeline's failed jobs do not
/// change: they are held for the session, not for a timer, unless the list
/// shows the pipeline has run again (a retry).
pub(crate) fn diagnose(
    p: &mut GitlabPlugin,
    project: &str,
    pipeline: u64,
    logs: bool,
    force: bool,
) -> Vec<Effect> {
    let k = key(project, pipeline);
    let sig = signature(p, project, pipeline);
    let same = p.watch.seen.get(&k).map_or("", String::as_str) == sig;
    let f = p.failed.entry(k.clone()).or_default();
    let fresh = f.loading || (f.value.is_some() && same);
    if fresh && !force {
        if !logs || f.loading {
            return vec![];
        }
        // The jobs are known: fetch the logs still missing.
        let want: Vec<u64> = ranked(f.value.as_deref().unwrap_or_default())
            .into_iter()
            .take(LOGS)
            .map(|j| j.id)
            .collect();
        return want
            .into_iter()
            .flat_map(|id| open_log(p, project, id, false))
            .collect();
    }
    f.loading = true;
    p.watch.seen.insert(k, sig);
    let request = log_ownership::pipeline(p, project, pipeline);
    let (url, token, project) = (p.cfg.url.clone(), p.cfg.token.clone(), project.to_string());
    let action = crate::failure_history::action(&url, &project, pipeline);
    vec![crate::failure_history::spawn(action, request, async move {
        let list = async {
            crate::auth::run(&url, &token, |c| {
                let project = project.clone();
                async move { c.failed_jobs_observed(&project, pipeline).await }
            })
            .await
            .map_err(|e| format!("{e:#}"))
        };
        read(logs, list, |id| {
            let url = url.clone();
            let token = token.clone();
            let project = project.clone();
            async move {
                crate::auth::run(&url, &token, |c| {
                    let project = project.clone();
                    async move { c.trace(&project, id, None).await }
                })
                .await
                .map_err(|e| format!("{e:#}"))
            }
        })
        .await
    })]
}

pub(crate) fn on_failed(
    p: &mut GitlabPlugin,
    request: PipelineRead,
    action: &radar_core::journal::ActionDescriptor,
    result: Result<Vec<JobInfo>, String>,
    logs: Vec<(u64, Result<TracePart, String>)>,
) -> Vec<Effect> {
    if !log_ownership::claim_pipeline(p, &request) {
        return vec![crate::failure_history::skipped(
            action,
            "gitlab.pipeline.diagnosis.apply.skipped.stale",
            1,
        )];
    }
    let eligible = log_ownership::eligible(
        p,
        &request,
        result
            .as_ref()
            .into_iter()
            .flatten()
            .map(|job| job.id)
            .chain(logs.iter().map(|(id, _)| *id)),
    );
    let mut jobs_skipped = 0;
    let mut tails_skipped = 0;
    let key = request.key();
    p.failed.get_mut(&key).expect("claimed cache").loading = false;
    match result {
        Ok(jobs) => {
            for job in &jobs {
                let key = JobKey::new(&request.project, job.id);
                if eligible.contains(&key) {
                    p.jobs.insert(key.clone(), job.clone());
                    log_ownership::activity(p, &key, false);
                } else {
                    jobs_skipped += 1;
                }
            }
            let f = p.failed.get_mut(&key).expect("claimed cache");
            f.value = Some(jobs);
            f.error = None;
            f.at = p.now;
        }
        Err(error) => p.failed.get_mut(&key).expect("claimed cache").error = Some(error),
    }
    for (id, result) in logs {
        let key = JobKey::new(&request.project, id);
        if eligible.contains(&key) {
            log_ownership::activity(p, &key, result.is_ok());
            got_log(p, &key, result);
        } else {
            tails_skipped += 1;
        }
    }
    [
        (
            "gitlab.pipeline.diagnosis.jobs.apply.skipped.newer",
            jobs_skipped,
        ),
        (
            "gitlab.pipeline.diagnosis.tails.apply.skipped.newer",
            tails_skipped,
        ),
    ]
    .into_iter()
    .filter(|(_, count)| *count != 0)
    .map(|(kind, count)| crate::failure_history::skipped(action, kind, count))
    .collect()
}

/// Fetch a job (what its page's title shows) and the last part of its log,
/// unless held (or `force`).
pub(crate) fn open_log(p: &mut GitlabPlugin, project: &str, id: u64, force: bool) -> Vec<Effect> {
    let key = JobKey::new(project, id);
    let f = p.logs.entry(key.clone()).or_default();
    if f.loading || (!force && (f.value.is_some() || f.error.is_some())) {
        return vec![];
    }
    f.loading = true;
    let request = log_ownership::direct(p, key);
    let (url, token, project) = (p.cfg.url.clone(), p.cfg.token.clone(), project.to_string());
    vec![crate::log_history::spawn(request, async move {
        crate::auth::run(&url, &token, |c| {
            let project = project.clone();
            async move { crate::log_history::read(&c, &project, id).await }
        })
        .await
        .map_err(|e| format!("{e:#}"))
    })]
}

/// Fetch the part of a job's log before what is held, if there is one and
/// it is not being fetched.
pub(crate) fn earlier(p: &mut GitlabPlugin, project: &str, id: u64) -> Vec<Effect> {
    let Some(request) = log_ownership::earlier(p, JobKey::new(project, id)) else {
        return vec![];
    };
    let before = request.earlier.expect("earlier request").before;
    let (url, token, project) = (p.cfg.url.clone(), p.cfg.token.clone(), project.to_string());
    vec![crate::log_history::earlier_spawn(request, async move {
        crate::auth::run(&url, &token, |c| {
            let project = project.clone();
            async move { c.trace_observed(&project, id, Some(before)).await }
        })
        .await
        .map_err(|e| format!("{e:#}"))
    })]
}

pub(crate) fn on_log(
    p: &mut GitlabPlugin,
    request: LogRead,
    info: Option<JobInfo>,
    result: Result<TracePart, String>,
) -> Vec<Effect> {
    if !log_ownership::claim_log(p, &request) {
        return vec![log_ownership::skipped(&request)];
    }
    let key = &request.key;
    log_ownership::activity(p, key, result.is_ok());
    if let Some(info) = info {
        p.jobs.insert(key.clone(), info);
    }
    if request.earlier.is_none() {
        p.logs.get_mut(key).expect("claimed cache").loading = false;
        let changed = got_log(p, key, result);
        // A page waiting for it opens at its first error.
        if changed
            && let Some(log) = p.logs.get(key).and_then(|f| f.value.as_ref())
            && let Some(line) = log.excerpt.as_ref().and_then(|e| e.error)
        {
            for page in p
                .pages
                .iter()
                .filter(|page| page.kind.job().as_ref() == Some(key))
            {
                page.log.goto(&log.text, line);
            }
        }
        return vec![];
    }
    let log = p
        .logs
        .get_mut(key)
        .and_then(|f| f.value.as_mut())
        .expect("claimed earlier base");
    match result {
        Ok(part) => {
            // This checks ownership of the held base, not the server's range
            // contiguity. TracePart retains neither end nor raw byte length.
            let added = log.prepend(part);
            for page in &p.pages {
                if page.kind.job().as_ref() == Some(key) {
                    page.log.shift(added);
                    page.log.research(&log.text);
                    page.log_painted.set(false);
                }
            }
        }
        Err(error) => p.logs.get_mut(key).expect("claimed cache").error = Some(error),
    }
    vec![]
}

/// Replace a tail and reconcile coordinates only when its parsed text changed.
/// Equal text and failed reads keep the user's position, folds and search state.
fn got_log(p: &mut GitlabPlugin, key: &JobKey, result: Result<TracePart, String>) -> bool {
    let f = p.logs.entry(key.clone()).or_default();
    match result {
        Ok(part) => {
            let log = JobLog::new(part);
            let changed = f.value.as_ref().is_none_or(|old| old.text != log.text);
            if changed {
                for page in p
                    .pages
                    .iter_mut()
                    .filter(|pg| pg.kind.job().as_ref() == Some(key))
                {
                    let query = page.log.query();
                    page.log = radar_core::ui::log::LogState::new();
                    page.log.search(&log.text, &query);
                    page.log_painted.set(false);
                }
            }
            f.value = Some(log);
            f.error = None;
            f.at = p.now;
            changed
        }
        Err(e) => {
            f.error = Some(e);
            false
        }
    }
}

/// True while a diagnosis or a log is being fetched.
pub(crate) fn loading(p: &GitlabPlugin) -> bool {
    p.failed.values().any(|f| f.loading)
        || p.logs.values().any(|f| f.loading)
        || p.log_reads.values().any(|reads| reads.earlier.is_some())
}

// --- what is shown -----------------------------------------------------------

/// `unit-test failed: the script failed`, `deploy and 2 more failed: …`:
/// a red pipeline in a line, once its failed jobs are known.
pub(crate) fn summary(p: &GitlabPlugin, project: &str, pipeline: u64) -> Option<String> {
    let jobs = failed(p, project, pipeline)?;
    let ranked = ranked(jobs);
    let first = ranked.first()?;
    let more = match ranked.len() {
        1 => String::new(),
        n => format!(" and {} more", n - 1),
    };
    let why = joblog::reason(&first.failure_reason);
    Some(if why.is_empty() {
        format!("{}{more} failed", first.name)
    } else {
        format!("{}{more} failed: {why}", first.name)
    })
}

/// The failed jobs, in the order shown: those that made it red first.
pub(crate) fn failed_rows(p: &GitlabPlugin, project: &str, pipeline: u64) -> Vec<JobInfo> {
    failed(p, project, pipeline)
        .map(|jobs| ranked(jobs).into_iter().cloned().collect())
        .unwrap_or_default()
}

/// The excerpt as plain text, for copying.
pub(crate) fn excerpt_text(p: &GitlabPlugin, project: &str, id: u64) -> Option<String> {
    let log = p.logs.get(&JobKey::new(project, id))?.value.as_ref()?;
    let ex = log.excerpt.as_ref()?;
    Some(
        ex.lines
            .clone()
            .map(|i| log.text.lines[i].text())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// A pipeline's id from its link, when it is red.
pub(crate) fn red(status: Option<Pipeline>, url: Option<&str>) -> Option<u64> {
    (status == Some(Pipeline::Failed))
        .then(|| url.and_then(crate::client::pipeline_id_of))
        .flatten()
}
