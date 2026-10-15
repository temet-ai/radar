//! Count-only evidence from one configured GitLab source's actual sync reply.
//! Descriptors travel with the task, never through the persisted provider cache.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::backoff::Net;
use crate::client::{Synced, Want};
use crate::{GlMsg, config};

pub(super) fn action(url: &str, group: &str, want: &Want) -> ActionDescriptor {
    let kind = if want.changed_since.is_some() {
        "gitlab.sync.incremental"
    } else {
        "gitlab.sync.full"
    };
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("gitlab".into()));
    let subject = SafeUrl::parse(url).ok().and_then(|url| {
        if config::group_path_problem(group).is_some() {
            return None;
        }
        SubjectKey::new("gitlab", url.as_str(), "source", group).ok()
    });
    match subject {
        Some(key) => action
            .subjects
            .push(SubjectSnapshot::new(key, SubjectRole::Primary)),
        None => {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
    }
    action
}

pub(super) fn spawn(
    url: &str,
    group: String,
    want: Want,
    read: impl Future<Output = (Result<Synced, String>, Net)> + Send + 'static,
) -> Effect {
    let action = action(url, &group, &want);
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            let (result, net) = read.await;
            Box::new(GlMsg::Loaded {
                group,
                want,
                action: captured,
                result,
                net,
            }) as Msg
        }),
    }
}

fn count(
    scope: &ActionDescriptor,
    kind: &'static str,
    count: usize,
    truncated: bool,
    outcome: Outcome,
) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(count).unwrap_or(u32::MAX));
    action.metadata.truncated = truncated || count > u32::MAX as usize;
    if outcome == Outcome::Failed {
        // The provider currently returns display strings, not a safe typed cause.
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

pub(super) fn observed(scope: &ActionDescriptor, result: &Result<Synced, String>) -> Vec<Effect> {
    match result {
        Ok(got) => {
            let mr_outcome = if got.mrs_truncated {
                Outcome::Partial
            } else {
                Outcome::Succeeded
            };
            let outcome = if got.mrs_truncated || got.nudges.errors != 0 || got.nudges.deferred != 0
            {
                Outcome::Partial
            } else {
                Outcome::Succeeded
            };
            let mut effects = vec![
                Effect::Report(outcome),
                count(
                    scope,
                    "gitlab.sync.merge_requests",
                    got.mrs.len() + got.rechecked.len(),
                    got.mrs_truncated,
                    mr_outcome,
                ),
            ];
            // Not requested is different from a successful empty result.
            let projects = got
                .projects
                .as_ref()
                .map(Vec::len)
                .or_else(|| got.projects_seen.as_ref().map(|s| s.present.len()));
            if let Some(projects) = projects {
                effects.push(count(
                    scope,
                    "gitlab.sync.projects",
                    projects,
                    false,
                    Outcome::Succeeded,
                ));
            }
            if got.nudges.attempted != 0 {
                // This observes attempts only, not completion of merge checks.
                effects.push(count(
                    scope,
                    "gitlab.sync.nudges.attempted",
                    got.nudges.attempted,
                    false,
                    Outcome::Succeeded,
                ));
            }
            if got.nudges.errors != 0 {
                effects.push(count(
                    scope,
                    "gitlab.sync.nudges.errors",
                    got.nudges.errors,
                    false,
                    Outcome::Failed,
                ));
            }
            if got.nudges.deferred != 0 {
                effects.push(count(
                    scope,
                    "gitlab.sync.nudges.deferred",
                    got.nudges.deferred,
                    false,
                    Outcome::Blocked,
                ));
            }
            effects
        }
        Err(_) => vec![
            Effect::Report(Outcome::Failed),
            // One returned task error, not a count of HTTP failures or lost rows.
            count(scope, "gitlab.sync.errors", 1, false, Outcome::Failed),
        ],
    }
}

#[cfg(test)]
mod tests;
