//! Local accepted page pushes and independent MR-detail read evidence.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, StableCode, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::client::MrDetailRead;
use crate::{GlMsg, diff_history};

/// Only accepted local navigation, never fetched, painted or human-read content.
/// Borrow requested identity from the read helpers, never their metadata or cache.
pub(super) fn opened(p: &crate::GitlabPlugin, kind: &crate::pages::Kind) -> Effect {
    use crate::pages::Kind;
    let primary = |scope: ActionDescriptor| {
        scope
            .subjects
            .into_iter()
            .find(|s| s.role == SubjectRole::Primary)
    };
    let subject = match kind {
        Kind::Tenancy(group) => {
            p.cfg
                .sources
                .iter()
                .find(|s| &s.group == group)
                .and_then(|source| {
                    let path = p.cfg.group_path(source);
                    if path.split('/').any(|part| matches!(part, "." | "..")) {
                        return None;
                    }
                    primary(crate::sync_history::action(
                        &p.cfg.url,
                        &path,
                        &Default::default(),
                    ))
                })
        }
        // Kind has no branch: the donor's omitted Context is not open evidence.
        Kind::Repo(path) => primary(crate::runs_history::action(&p.cfg.url, path, "")),
        Kind::Mr { project, iid } => diff_history::requested_subject(&p.cfg.url, project, *iid),
        Kind::Job { project, id } => primary(crate::log_ownership::action(
            &p.cfg.url,
            &crate::log_ownership::JobKey::new(project, *id),
        )),
    }
    .filter(|s| {
        [&s.key.instance, &s.key.id]
            .into_iter()
            .all(|value| value.len() <= 2048 && !radar_core::log::contains_secret(value))
    });
    let mut action = ActionDescriptor::new("gitlab.page.open", Origin::Plugin("gitlab".into()));
    match subject {
        Some(subject) => action
            .subjects
            .push(SubjectSnapshot::new(subject.key, SubjectRole::Primary)),
        None => {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
    }
    Effect::Observe {
        action,
        outcome: Outcome::Succeeded,
    }
}

pub(super) fn action(url: &str, project: &str, iid: u64) -> ActionDescriptor {
    let mut action = ActionDescriptor::new("gitlab.page.detail", Origin::Plugin("gitlab".into()));
    match diff_history::requested_subject(url, project, iid) {
        Some(subject) => action.subjects.push(subject),
        None => {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
    }
    action
}

pub(super) fn spawn(
    action: ActionDescriptor,
    key: String,
    owner: crate::page_ownership::Read,
    requested: crate::page_identity::Requested,
    read: impl Future<Output = Result<MrDetailRead, String>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(GlMsg::MrPage {
                key,
                read: owner,
                requested,
                action: captured,
                result: read.await.map(Box::new),
            }) as Msg
        }),
    }
}

/// Independent application evidence, after the original read's actual outcome.
pub(super) fn skipped(scope: &ActionDescriptor) -> Effect {
    blocked(scope, "gitlab.page.detail.application.skipped")
}

pub(super) fn identity_rejected(scope: &ActionDescriptor) -> Effect {
    blocked(scope, "gitlab.page.detail.application.identity.rejected")
}

fn blocked(scope: &ActionDescriptor, kind: &'static str) -> Effect {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("gitlab".into()));
    action.subjects.clone_from(&scope.subjects);
    action.metadata.redacted = scope.metadata.redacted;
    action.metadata.omitted_subjects = scope.metadata.omitted_subjects;
    action.metadata.count = Some(1);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

fn count(scope: &ActionDescriptor, kind: &'static str, n: usize, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    // A requested first:100 is not evidence of a cutoff or complete inventory.
    action.metadata.truncated = n > u32::MAX as usize;
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

fn outcome(incomplete: bool) -> Outcome {
    if incomplete {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    }
}

pub(super) fn observed(
    scope: &ActionDescriptor,
    result: &Result<Box<MrDetailRead>, String>,
) -> Vec<Effect> {
    let Ok(got) = result else {
        // One final task error, with no assumed earlier progress or attempt count.
        return vec![
            Effect::Report(Outcome::Failed),
            count(scope, "gitlab.page.detail.errors", 1, Outcome::Failed),
        ];
    };
    let e = &got.evidence;
    let d = &got.detail;
    let pipeline = d.pipeline.as_ref();
    let components = [
        ("gitlab.page.detail.merge_requests", 1, e.mr_incomplete),
        (
            "gitlab.page.detail.files",
            d.files.len(),
            e.files_incomplete,
        ),
        (
            "gitlab.page.detail.commits",
            d.commits.len(),
            e.commits_incomplete,
        ),
        (
            "gitlab.page.detail.discussions",
            d.discussions.len(),
            e.discussions_incomplete || e.notes_incomplete,
        ),
        (
            "gitlab.page.detail.notes",
            d.discussions
                .iter()
                .map(|discussion| discussion.notes.len())
                .sum(),
            e.notes_incomplete,
        ),
        (
            "gitlab.page.detail.pipelines",
            usize::from(pipeline.is_some()),
            e.pipeline_incomplete,
        ),
        (
            "gitlab.page.detail.stages",
            pipeline.map_or(0, |pipeline| pipeline.stages.len()),
            e.stages_incomplete,
        ),
        (
            "gitlab.page.detail.jobs",
            pipeline.map_or(0, |pipeline| {
                pipeline.stages.iter().map(|stage| stage.jobs.len()).sum()
            }),
            e.jobs_incomplete,
        ),
    ];
    let incomplete = components.iter().any(|(_, _, gap)| *gap);
    let mut effects = vec![Effect::Report(outcome(incomplete))];
    effects.extend(components.map(|(kind, n, gap)| count(scope, kind, n, outcome(gap))));
    if incomplete {
        effects.push(count(
            scope,
            "gitlab.page.detail.responses.incomplete",
            1,
            Outcome::Partial,
        ));
    }
    effects
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod open_tests;
