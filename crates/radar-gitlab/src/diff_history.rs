//! One requested MR diff page. No returned revision or file identity is inferred.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::client::{DIFF_PAGE, DiffRead};
use crate::{GlMsg, config, join};

pub(super) fn action(url: &str, project: &str, iid: u64, page: usize) -> ActionDescriptor {
    let mut action = ActionDescriptor::new("gitlab.page.diffs", Origin::Plugin("gitlab".into()));
    // For this request action only, count is the raw requested page parameter.
    // Never replace an unrepresentable ordinal with a different exact page.
    action.metadata.count = u32::try_from(page).ok();
    if action.metadata.count.is_none() {
        action.metadata.redacted = true;
        action.metadata.truncated = true;
    }
    match requested_subject(url, project, iid) {
        Some(subject) => action.subjects.push(subject),
        None => {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
    }
    action
}

pub(super) fn requested_subject(url: &str, project: &str, iid: u64) -> Option<SubjectSnapshot> {
    SafeUrl::parse(url).ok().and_then(|url| {
        if iid == 0
            || config::group_path_problem(project).is_some()
            || project.split('/').any(|part| matches!(part, "." | ".."))
        {
            return None;
        }
        SubjectKey::new(
            "gitlab",
            url.as_str(),
            join::MERGE_REQUEST,
            &format!("{project}!{iid}"),
        )
        .ok()
        .map(|key| SubjectSnapshot::new(key, SubjectRole::Primary))
    })
}

pub(super) fn spawn(
    action: ActionDescriptor,
    key: String,
    owner: crate::page_ownership::Read,
    head: String,
    page: usize,
    read: impl Future<Output = Result<DiffRead, String>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(GlMsg::Diffs {
                key,
                read: owner,
                head,
                page,
                action: captured,
                result: read.await,
            }) as Msg
        }),
    }
}

fn count(scope: &ActionDescriptor, kind: &'static str, count: usize, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(count).unwrap_or(u32::MAX));
    // The requested page size is not evidence of unread rows.
    action.metadata.truncated = count > u32::MAX as usize;
    if matches!(outcome, Outcome::Failed | Outcome::Blocked) {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

pub(super) fn observed(scope: &ActionDescriptor, result: &Result<DiffRead, String>) -> Vec<Effect> {
    let outcome = match result {
        Ok(got)
            if got.evidence.incomplete_response
                || got.evidence.incomplete_rows != 0
                || got.evidence.withheld_rows != 0 =>
        {
            Outcome::Partial
        }
        Ok(_) => Outcome::Succeeded,
        Err(_) => Outcome::Failed,
    };
    let mut effects = vec![
        Effect::Report(outcome),
        // A known request parameter, not proof that an HTTP attempt succeeded.
        count(
            scope,
            "gitlab.page.diffs.limit.requested",
            DIFF_PAGE,
            Outcome::Succeeded,
        ),
    ];
    match result {
        Ok(got) => {
            effects.push(count(
                scope,
                "gitlab.page.diffs.entries",
                got.files.len(),
                outcome,
            ));
            for (kind, n) in [
                (
                    "gitlab.page.diffs.responses.incomplete",
                    usize::from(got.evidence.incomplete_response),
                ),
                (
                    "gitlab.page.diffs.rows.incomplete",
                    got.evidence.incomplete_rows,
                ),
                (
                    "gitlab.page.diffs.content.withheld",
                    got.evidence.withheld_rows,
                ),
            ] {
                if n != 0 {
                    effects.push(count(scope, kind, n, Outcome::Partial));
                }
            }
        }
        // One returned task error, never an HTTP-attempt or lost-row count.
        Err(_) => effects.push(count(scope, "gitlab.page.diffs.errors", 1, Outcome::Failed)),
    }
    effects
}

pub(super) fn stale(scope: &ActionDescriptor) -> Effect {
    count(
        scope,
        "gitlab.page.diffs.apply.skipped.stale",
        1,
        Outcome::Blocked,
    )
}

#[cfg(test)]
mod tests;
