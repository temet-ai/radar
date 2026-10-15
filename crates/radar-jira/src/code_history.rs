//! Ticket-scoped observations of background development-panel reads.
//! Returned URLs and display fields are deliberately not subject identities.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::JiraMsg;
use crate::client::CodeRead;
use crate::config::{Site, is_ticket_key};

pub(super) fn action(site: &Site, ticket: &str) -> ActionDescriptor {
    let mut action = ActionDescriptor::new("jira.sync.code", Origin::Plugin("jira".into()));
    let subject = is_ticket_key(ticket)
        .then(|| site.base_url().ok())
        .flatten()
        .and_then(|instance| SafeUrl::parse(&instance).ok())
        .and_then(|instance| SubjectKey::new("jira", instance.as_str(), "ticket", ticket).ok());
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
    action: ActionDescriptor,
    site: String,
    key: String,
    updated: String,
    read: impl Future<Output = Result<CodeRead, String>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(JiraMsg::Code {
                site,
                key,
                updated,
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
    // Numeric saturation only; these endpoints have no observed paging bound.
    action.metadata.truncated = count > u32::MAX as usize;
    action.metadata.reason = match outcome {
        Outcome::Failed => Some(Reason::Unspecified),
        Outcome::Blocked => Some(Reason::InvalidInput),
        _ => None,
    };
    Effect::Observe { action, outcome }
}

pub(super) fn observed(scope: &ActionDescriptor, result: &Result<CodeRead, String>) -> Vec<Effect> {
    match result {
        Ok(got) if got.evidence.skipped_invalid_id => vec![
            Effect::Report(Outcome::Blocked),
            count(
                scope,
                "jira.sync.code.requests.skipped",
                1,
                Outcome::Blocked,
            ),
        ],
        Ok(got) => {
            let evidence = &got.evidence;
            let outcome = if evidence.omitted_rows != 0 || evidence.incomplete_responses != 0 {
                Outcome::Partial
            } else {
                Outcome::Succeeded
            };
            let mut effects = vec![
                Effect::Report(outcome),
                count(scope, "jira.sync.code.links", got.links.len(), outcome),
            ];
            for (kind, n) in [
                ("jira.sync.code.rows.omitted", evidence.omitted_rows),
                (
                    "jira.sync.code.responses.incomplete",
                    evidence.incomplete_responses,
                ),
            ] {
                if n != 0 {
                    effects.push(count(scope, kind, n, Outcome::Partial));
                }
            }
            effects
        }
        // A later host error discards earlier results in the existing client.
        // No earlier-host, retry or returned-link progress can be inferred.
        Err(_) => vec![
            Effect::Report(Outcome::Failed),
            count(scope, "jira.sync.code.errors", 1, Outcome::Failed),
        ],
    }
}

#[cfg(test)]
mod tests;
