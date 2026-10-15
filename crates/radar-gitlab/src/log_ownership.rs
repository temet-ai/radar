//! Session-local pipeline/job read ownership. Transport ordering and permissive
//! payload parsing stay unchanged; these guards only decide where replies apply.

use std::collections::BTreeSet;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::Effect;

use crate::{
    GitlabPlugin, config, join,
    page_ownership::{self, Read},
};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct JobKey {
    pub project: String,
    pub id: u64,
}

impl JobKey {
    pub(crate) fn new(project: &str, id: u64) -> Self {
        Self {
            project: project.into(),
            id,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PipelineRead {
    pub project: String,
    pub pipeline: u64,
    pub owner: Read,
    pub activity: u64,
}

impl PipelineRead {
    pub(crate) fn key(&self) -> String {
        crate::why::key(&self.project, self.pipeline)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Earlier {
    pub owner: Read,
    pub base: u64,
    pub before: u64,
}

#[derive(Debug, Default)]
pub(crate) struct JobReads {
    pub direct: Option<Read>,
    pub earlier: Option<Earlier>,
    pub base: u64,
    pub activity: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct LogRead {
    pub key: JobKey,
    pub owner: Read,
    pub earlier: Option<Earlier>,
    pub action: ActionDescriptor,
}

pub(crate) fn pipeline(p: &mut GitlabPlugin, project: &str, pipeline: u64) -> PipelineRead {
    let request = PipelineRead {
        project: project.into(),
        pipeline,
        owner: page_ownership::next(p),
        activity: p.next_log_activity,
    };
    p.failed_reads.insert(request.key(), request.owner);
    request
}

/// Retire only this exact owner even if its cache was independently removed.
pub(crate) fn claim_pipeline(p: &mut GitlabPlugin, request: &PipelineRead) -> bool {
    let key = request.key();
    if !p.is_current_connection(request.owner.connection)
        || p.failed_reads.get(&key) != Some(&request.owner)
    {
        return false;
    }
    p.failed_reads.remove(&key);
    p.failed.contains_key(&key)
}

/// Dispatch and accepted completion are activity; no-op and stale replies are not.
pub(crate) fn activity(p: &mut GitlabPlugin, key: &JobKey, replacement: bool) {
    p.next_log_activity = p
        .next_log_activity
        .checked_add(1)
        .expect("GitLab log activity sequence exhausted");
    let reads = p.log_reads.entry(key.clone()).or_default();
    reads.activity = p.next_log_activity;
    if replacement {
        reads.base = reads.activity;
    }
}

pub(crate) fn direct(p: &mut GitlabPlugin, key: JobKey) -> LogRead {
    let owner = page_ownership::next(p);
    let action = action(&p.cfg.url, &key);
    activity(p, &key, false);
    let reads = p.log_reads.get_mut(&key).expect("activity installed");
    reads.direct = Some(owner);
    reads.earlier = None;
    LogRead {
        key,
        owner,
        earlier: None,
        action,
    }
}

pub(crate) fn earlier(p: &mut GitlabPlugin, key: JobKey) -> Option<LogRead> {
    let log = p
        .logs
        .get(&key)?
        .value
        .as_ref()
        .filter(|log| log.earlier())?;
    let before = log.start;
    let reads = p.log_reads.entry(key.clone()).or_default();
    if reads.direct.is_some() || reads.earlier.is_some() {
        return None;
    }
    let base = reads.base;
    let owner = page_ownership::next(p);
    let mut action = action(&p.cfg.url, &key);
    action.kind = StableCode::new("gitlab.job.log.earlier");
    // Original exclusive byte boundary, not a returned-byte or completeness count.
    action.metadata.count = Some(u32::try_from(before).unwrap_or(u32::MAX));
    action.metadata.truncated = before > u32::MAX.into();
    let earlier = Earlier {
        owner,
        base,
        before,
    };
    activity(p, &key, false);
    p.log_reads
        .get_mut(&key)
        .expect("activity installed")
        .earlier = Some(earlier);
    Some(LogRead {
        key,
        owner,
        earlier: Some(earlier),
        action,
    })
}

/// Consume one exact callback, never another request's loading state. Earlier
/// reads own the held revision and byte start, not the user's current page.
pub(crate) fn claim_log(p: &mut GitlabPlugin, request: &LogRead) -> bool {
    if !p.is_current_connection(request.owner.connection) {
        return false;
    }
    let Some(reads) = p.log_reads.get_mut(&request.key) else {
        return false;
    };
    match request.earlier {
        None => {
            if reads.direct != Some(request.owner) {
                return false;
            }
            reads.direct = None;
            p.logs.contains_key(&request.key)
        }
        Some(earlier) => {
            if reads.earlier != Some(earlier) || earlier.owner != request.owner {
                return false;
            }
            reads.earlier = None;
            reads.base == earlier.base
                && p.logs
                    .get(&request.key)
                    .and_then(|f| f.value.as_ref())
                    .is_some_and(|log| log.start == earlier.before)
        }
    }
}

pub(crate) fn earlier_loading(p: &GitlabPlugin, key: &JobKey) -> bool {
    p.log_reads
        .get(key)
        .is_some_and(|reads| reads.earlier.is_some())
}

/// Freeze before *any* bundle metadata/tail write. Duplicate occurrences must
/// retain their existing order, rather than fence each other within one reply.
pub(crate) fn eligible(
    p: &GitlabPlugin,
    request: &PipelineRead,
    ids: impl Iterator<Item = u64>,
) -> BTreeSet<JobKey> {
    ids.map(|id| JobKey::new(&request.project, id))
        .filter(|key| {
            p.is_current_connection(request.owner.connection)
                && p.log_reads.get(key).is_none_or(|reads| {
                    reads.direct.is_none()
                        && reads.earlier.is_none()
                        && reads.activity <= request.activity
                })
        })
        .collect()
}

/// Existing source metadata is only a seed. Opening an already known job is
/// not activity and can never replace a current remote title/link.
pub(crate) fn seed(p: &mut GitlabPlugin, key: JobKey, info: crate::model::JobInfo) {
    if let std::collections::btree_map::Entry::Vacant(slot) = p.jobs.entry(key.clone()) {
        slot.insert(info);
        activity(p, &key, false);
    }
}

/// Original requested scope only; no credentials, payload or local owner IDs.
pub(crate) fn action(url: &str, key: &JobKey) -> ActionDescriptor {
    let mut action = ActionDescriptor::new("gitlab.job.log", Origin::Plugin("gitlab".into()));
    let subject = SafeUrl::parse(url).ok().and_then(|url| {
        if key.id == 0
            || config::group_path_problem(&key.project).is_some()
            || key
                .project
                .split('/')
                .any(|part| matches!(part, "." | ".."))
        {
            return None;
        }
        SubjectKey::new(
            "gitlab",
            url.as_str(),
            join::JOB,
            &format!("{}/{}", key.project, key.id),
        )
        .ok()
        .map(|key| SubjectSnapshot::new(key, SubjectRole::Primary))
    });
    match subject {
        Some(subject) => action.subjects.push(subject),
        None => {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
    }
    action
}

pub(crate) fn skipped(request: &LogRead) -> Effect {
    let mut action = request.action.clone();
    action.kind = StableCode::new(if request.earlier.is_some() {
        "gitlab.job.log.earlier.apply.skipped.stale"
    } else {
        "gitlab.job.log.tail.apply.skipped.stale"
    });
    action.metadata.count = Some(1);
    action.metadata.truncated = false;
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

#[cfg(test)]
mod tests;
