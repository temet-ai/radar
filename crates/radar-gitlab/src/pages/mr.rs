//! The merge request page: Overview (description, latest activity, and
//! whether it can merge), Changes (files with +/−), Threads (every
//! discussion with its replies), Pipeline (stages and jobs), Commits.

use super::{Fetched, find_mr, mr_key};
use crate::model::{Job, JobInfo, Mr, MrDetail};
use crate::{GitlabPlugin, why};

pub(super) const TABS: [&str; 5] = ["Overview", "Changes", "Threads", "Pipeline", "Commits"];
pub(super) const OVERVIEW: usize = 0;
pub(super) const CHANGES: usize = 1;
pub(super) const THREADS: usize = 2;
pub(super) const PIPELINE: usize = 3;

type Loaded = Fetched<(Mr, MrDetail)>;

/// A row of the Pipeline tab: a failed job (why it is red), then every
/// job stage by stage.
pub(super) enum JobRow {
    Failed(JobInfo),
    Job {
        stage: String,
        first: bool,
        job: Job,
    },
}

impl JobRow {
    pub(super) fn id(&self) -> Option<u64> {
        match self {
            Self::Failed(j) => Some(j.id),
            Self::Job { job, .. } => job.id,
        }
    }

    pub(super) fn name(&self) -> &str {
        match self {
            Self::Failed(j) => &j.name,
            Self::Job { job, .. } => &job.name,
        }
    }

    /// What the row knows of its job, for its page's title.
    pub(super) fn info(&self, project: &str) -> Option<JobInfo> {
        match self {
            Self::Failed(j) => Some(j.clone()),
            Self::Job { stage, job, .. } => Some(JobInfo {
                id: job.id?,
                project: project.to_string(),
                name: job.name.clone(),
                stage: stage.clone(),
                status: job.status.clone(),
                duration_secs: job.duration_secs,
                failure_reason: String::new(),
                url: job.url.clone().unwrap_or_default(),
                allow_failure: job.allow_failure,
                pipeline: 0,
            }),
        }
    }
}

/// The red pipeline's id, when the merge request's pipeline failed.
fn red(mr: &Mr) -> Option<u64> {
    why::red(mr.pipeline, mr.pipeline_url.as_deref())
}

pub(super) fn job_rows(p: &GitlabPlugin, project: &str, iid: u64) -> Vec<JobRow> {
    let mut rows = Vec::new();
    if let Some(pl) = find_mr(p, project, iid).and_then(red) {
        rows.extend(why::failed_rows(p, project, pl).into_iter().map(JobRow::Failed));
    }
    if let Some(pl) = detail(p, project, iid).and_then(|d| d.pipeline.as_ref()) {
        for s in &pl.stages {
            rows.extend(s.jobs.iter().enumerate().map(|(i, j)| JobRow::Job {
                stage: s.name.clone(),
                first: i == 0,
                job: j.clone(),
            }));
        }
    }
    rows
}

fn fetched<'a>(p: &'a GitlabPlugin, project: &str, iid: u64) -> Option<&'a Loaded> {
    p.mr_pages.get(&mr_key(project, iid))
}

fn detail<'a>(p: &'a GitlabPlugin, project: &str, iid: u64) -> Option<&'a MrDetail> {
    fetched(p, project, iid)
        .and_then(|f| f.value.as_ref())
        .map(|(_, d)| d)
}

/// Open threads: from the page's discussions once loaded, else the list's count.
fn open_threads(mr: &Mr, d: Option<&MrDetail>) -> usize {
    match d {
        Some(d) => d.discussions.iter().filter(|x| x.is_open()).count(),
        None => mr.threads.saturating_sub(mr.threads_resolved) as usize,
    }
}

pub(super) fn tabs() -> Vec<String> {
    TABS.iter().map(|name| (*name).to_string()).collect()
}

/// Open discussions and failed pipelines need attention; totals do not.
pub(super) fn attention(p: &GitlabPlugin, project: &str, iid: u64) -> [bool; 5] {
    attention_of(find_mr(p, project, iid), detail(p, project, iid))
}

fn attention_of(mr: Option<&Mr>, d: Option<&MrDetail>) -> [bool; 5] {
    let open = match (mr, d) {
        (Some(m), d) => open_threads(m, d),
        (None, Some(d)) => d.discussions.iter().filter(|x| x.is_open()).count(),
        (None, None) => 0,
    };
    let failed = mr.is_some_and(|mr| mr.pipeline == Some(crate::model::Pipeline::Failed));
    [false, false, open > 0, failed, false]
}

#[cfg(test)]
mod tab_attention_tests {
    use super::*;
    use crate::model::{Discussion, Pipeline};
    use crate::test_support::MrBuilder;

    #[test]
    fn names_ignore_totals_and_attention_tracks_only_unresolved_or_failed_state() {
        let mut mr = MrBuilder::default().build();
        mr.files = 42;
        mr.notes = 17;
        mr.threads = 3;
        mr.threads_resolved = 2;
        mr.pipeline = Some(Pipeline::Failed);
        assert_eq!(tabs(), ["Overview", "Changes", "Threads", "Pipeline", "Commits"]);
        assert_eq!(attention_of(Some(&mr), None), [false, false, true, true, false]);
        mr.pipeline = Some(Pipeline::Passed);
        let mut detail = MrDetail::default();
        detail.discussions.push(Discussion {
            id: "watch-note".into(), resolvable: true, resolved: true, notes: vec![],
        });
        assert_eq!(attention_of(Some(&mr), Some(&detail)), [false; 5], "loaded resolutions override the old count");
        detail.discussions[0].resolved = false;
        assert_eq!(attention_of(None, Some(&detail)), [false, false, true, false, false]);
        detail.discussions[0].resolvable = false;
        assert_eq!(attention_of(None, Some(&detail)), [false; 5], "ordinary comments are not unresolved work");
    }
}
