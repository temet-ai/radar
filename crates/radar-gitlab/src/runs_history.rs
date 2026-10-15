//! One existing repository-page read, scoped to its exact requested ref.
//! Returned pipeline rows supply counts, never verified pipeline/revision identity.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::client::RunsRead;
use crate::{GlMsg, config, join};

pub(super) fn action(url: &str, project: &str, branch: &str) -> ActionDescriptor {
    let mut action = ActionDescriptor::new("gitlab.page.runs", Origin::Plugin("gitlab".into()));
    let repo = SafeUrl::parse(url).ok().and_then(|url| {
        if project.len() > 2048
            || config::group_path_problem(project).is_some()
            || project.split('/').any(|part| matches!(part, "." | ".."))
            || radar_core::log::contains_secret(project)
            || radar_core::log::contains_secret(url.as_str())
        {
            return None;
        }
        SubjectKey::new("gitlab", url.as_str(), join::REPO, project).ok()
    });
    let target = repo.as_ref().and_then(|repo| {
        if !supported_ref(branch) || radar_core::log::contains_secret(branch) {
            return None;
        }
        // Both local validators exclude ':', so this tuple is unambiguous.
        // This observation-only kind has no compose ownership or auto-binding.
        SubjectKey::new(
            "gitlab",
            &repo.instance,
            "branch",
            &format!("{project}:{branch}"),
        )
        .ok()
        .filter(|key| !radar_core::log::contains_secret(&key.id))
    });
    for (key, role) in [(repo, SubjectRole::Primary), (target, SubjectRole::Context)] {
        match key {
            Some(key) => action.subjects.push(SubjectSnapshot::new(key, role)),
            None => {
                action.metadata.redacted = true;
                action.metadata.omitted_subjects += 1;
            }
        }
    }
    action
}

/// A conservative history-identity subset, not a full provider ref validator.
/// Unsupported refs still reach the unchanged request, but cannot be journal IDs.
fn supported_ref(branch: &str) -> bool {
    !branch.is_empty()
        && branch.len() <= 2048
        && !branch.starts_with('-')
        && !branch.contains("..")
        && branch
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        && branch.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && !part.ends_with('.')
                && !part.ends_with(".lock")
        })
}

pub(super) fn spawn(
    action: ActionDescriptor,
    request: crate::runs_ownership::Request,
    read: impl Future<Output = Result<RunsRead, String>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(GlMsg::Runs {
                request,
                action: captured,
                result: read.await,
            }) as Msg
        }),
    }
}

fn count(scope: &ActionDescriptor, kind: &'static str, n: usize, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    // first:10, including exactly ten returned rows, proves no cutoff.
    action.metadata.truncated = n > u32::MAX as usize;
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

pub(super) fn observed(scope: &ActionDescriptor, result: &Result<RunsRead, String>) -> Vec<Effect> {
    let Ok(got) = result else {
        // A final task error, with no assumed attempts or intermediate progress.
        return vec![
            Effect::Report(Outcome::Failed),
            count(scope, "gitlab.page.runs.errors", 1, Outcome::Failed),
        ];
    };
    let e = &got.evidence;
    let outcome = if e.incomplete_response || e.incomplete_rows != 0 {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    };
    let mut effects = vec![
        Effect::Report(outcome),
        count(scope, "gitlab.page.runs.entries", got.runs.len(), outcome),
    ];
    for (kind, n) in [
        (
            "gitlab.page.runs.responses.incomplete",
            usize::from(e.incomplete_response),
        ),
        ("gitlab.page.runs.rows.incomplete", e.incomplete_rows),
    ] {
        if n != 0 {
            effects.push(count(scope, kind, n, Outcome::Partial));
        }
    }
    effects
}

/// Application evidence is bounded and keeps the original dispatch scope.
pub(super) fn skipped(scope: &ActionDescriptor) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new("gitlab.page.runs.apply.skipped.stale");
    action.metadata.count = Some(1);
    Effect::Observe { action, outcome: Outcome::Blocked }
}

#[cfg(test)]
mod tests;
