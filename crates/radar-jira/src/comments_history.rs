//! One newest-first older-comment page, scoped to the requested ticket and offset.
//! Returned rows are occurrences, not validated remote comment identities.

use std::future::Future;

use radar_core::journal::{ActionDescriptor, Outcome, Reason, StableCode};
use radar_core::plugin::{Effect, Msg};

use crate::JiraMsg;
use crate::client::{COMMENTS_PAGE, CommentsRead};
use crate::config::Site;

pub(super) fn action(site: &Site, ticket: &str, start: usize) -> ActionDescriptor {
    let mut action = crate::page_history::action(site, ticket);
    action.kind = StableCode::new("jira.page.comments");
    // For this action only, count is the captured requested start offset.
    // Do not record a different exact offset when it cannot be represented.
    action.metadata.count = u32::try_from(start).ok();
    if action.metadata.count.is_none() {
        action.metadata.redacted = true;
        action.metadata.truncated = true;
    }
    action
}

pub(super) fn spawn(
    action: ActionDescriptor,
    site: String,
    key: String,
    owner: crate::page::More,
    read: impl Future<Output = Result<CommentsRead, String>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(JiraMsg::More {
                site,
                key,
                action: captured,
                read: owner,
                result: read.await,
            }) as Msg
        }),
    }
}

fn count(scope: &ActionDescriptor, kind: &'static str, n: usize, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    // Advertised older rows and oversized replies are not an observed cutoff.
    action.metadata.truncated = n > u32::MAX as usize;
    action.metadata.reason = (outcome == Outcome::Failed).then_some(Reason::Unspecified);
    Effect::Observe { action, outcome }
}

pub(super) fn observed(
    scope: &ActionDescriptor,
    result: &Result<CommentsRead, String>,
) -> Vec<Effect> {
    let outcome = match result {
        Ok(got) if got.evidence.incomplete_collection || got.evidence.total_defaulted => {
            Outcome::Partial
        }
        Ok(_) => Outcome::Succeeded,
        Err(_) => Outcome::Failed,
    };
    let mut effects = vec![
        Effect::Report(outcome),
        // A captured request parameter, not proof that an HTTP attempt ran.
        count(
            scope,
            "jira.page.comments.limit.requested",
            COMMENTS_PAGE,
            Outcome::Succeeded,
        ),
    ];
    match result {
        Ok(got) => {
            effects.push(count(
                scope,
                "jira.page.comments.entries",
                got.comments.len(),
                outcome,
            ));
            for (kind, present) in [
                (
                    "jira.page.comments.collections.incomplete",
                    got.evidence.incomplete_collection,
                ),
                (
                    "jira.page.comments.total.defaulted",
                    got.evidence.total_defaulted,
                ),
            ] {
                if present {
                    effects.push(count(scope, kind, 1, Outcome::Partial));
                }
            }
        }
        // One logical task error, never an HTTP-attempt or missing-row count.
        Err(_) => effects.push(count(
            scope,
            "jira.page.comments.errors",
            1,
            Outcome::Failed,
        )),
    }
    effects
}

/// The actual read outcome is already reported. This only describes local refusal.
pub(super) fn skipped(scope: &ActionDescriptor) -> Effect {
    count(
        scope,
        "jira.page.comments.application.skipped",
        1,
        Outcome::Blocked,
    )
}

#[cfg(test)]
mod tests;
