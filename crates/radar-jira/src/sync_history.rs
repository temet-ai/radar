//! Count-only observations of one site's bounded view-list refresh.
//! The captured descriptor and list evidence never enter the provider cache.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::JiraMsg;
use crate::client::TicketsRead;
use crate::config::Site;
use crate::model::Me;

#[derive(Debug, Default)]
pub(super) struct Synced {
    pub me: Option<Me>,
    pub list: TicketsRead,
    /// `list` is only what changed since the last read, to merge into what
    /// is held; otherwise it is the whole view.
    pub changed_only: bool,
    /// The whole view's keys in Jira's order, from a sweep beside a
    /// changed-only read.
    pub swept: Option<Vec<String>>,
}

/// A sync that failed: what to say, and how long Jira asked radar to wait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failed {
    pub text: String,
    pub retry_after: Option<u64>,
}

impl Failed {
    pub(crate) fn of(e: &anyhow::Error) -> Self {
        Self {
            text: format!("{e:#}"),
            retry_after: crate::client::Busy::wait(e),
        }
    }
}

impl From<String> for Failed {
    fn from(text: String) -> Self {
        Self { text, retry_after: None }
    }
}

impl From<&str> for Failed {
    fn from(text: &str) -> Self {
        text.to_string().into()
    }
}

pub(super) fn action(site: &Site) -> ActionDescriptor {
    let mut action = ActionDescriptor::new("jira.sync.view", Origin::Plugin("jira".into()));
    let subject = site.base_url().ok().and_then(|url| {
        let url = SafeUrl::parse(&url).ok()?;
        // An action identifies a single request. Neither the configured display
        // name nor the query is a safe, stable identity for a saved view.
        SubjectKey::new("jira", url.as_str(), "site", "site").ok()
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

pub(super) fn spawn<E: Into<Failed>>(
    action: ActionDescriptor,
    site: String,
    jql: String,
    read: impl Future<Output = Result<Synced, E>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(JiraMsg::Loaded {
                site,
                jql,
                action: captured,
                result: read.await.map_err(Into::into),
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
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

pub(super) fn observed(scope: &ActionDescriptor, result: &Result<Synced, Failed>) -> Vec<Effect> {
    match result {
        Ok(got) => {
            let evidence = &got.list.evidence;
            let outcome =
                if evidence.truncated || evidence.omitted != 0 || evidence.incomplete_pages != 0 {
                    Outcome::Partial
                } else {
                    Outcome::Succeeded
                };
            let mut effects = vec![
                Effect::Report(outcome),
                // Successfully parsed rows in this returned result, before
                // merging. A partial zero is not proof of an empty inventory.
                count(
                    scope,
                    "jira.sync.tickets",
                    got.list.tickets.len(),
                    evidence.truncated,
                    outcome,
                ),
            ];
            if evidence.truncated {
                effects.push(count(scope, "jira.sync.limits", 1, true, Outcome::Partial));
            }
            if evidence.omitted != 0 {
                effects.push(count(
                    scope,
                    "jira.sync.issues.omitted",
                    evidence.omitted,
                    false,
                    Outcome::Partial,
                ));
            }
            if evidence.incomplete_pages != 0 {
                effects.push(count(
                    scope,
                    "jira.sync.pages.incomplete",
                    evidence.incomplete_pages,
                    false,
                    Outcome::Partial,
                ));
            }
            effects
        }
        Err(_) => vec![
            Effect::Report(Outcome::Failed),
            // One returned task error, not an HTTP-attempt or lost-row count.
            count(scope, "jira.sync.errors", 1, false, Outcome::Failed),
        ],
    }
}

#[cfg(test)]
mod tests;
