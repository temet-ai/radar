//! Local page opens and independent detail reads, scoped to the configured server
//! and requested ticket. Read evidence describes entries, never remote inventory.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::JiraMsg;
use crate::client::{DetailRead, LinkEvidence};
use crate::config::{Site, is_ticket_key};

pub(super) fn action(site: &Site, ticket: &str) -> ActionDescriptor {
    let mut action = ActionDescriptor::new("jira.page.detail", Origin::Plugin("jira".into()));
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

/// The page was pushed locally. Fetching, painting and reading it are separate
/// facts; a missing/unsafe scope must not change whether navigation succeeds.
pub(super) fn opened(site: Option<&Site>, ticket: &str) -> Effect {
    let mut action = site.map_or_else(
        || ActionDescriptor::new("jira.page.open", Origin::Plugin("jira".into())),
        |site| action(site, ticket),
    );
    action.kind = StableCode::new("jira.page.open");
    // Scan the exact persisted identity components, after existing validation,
    // with explicit bounds. This protects only the new observation's scope.
    if action.subjects.is_empty()
        || action.subjects.iter().any(|subject| {
            [&subject.key.instance, &subject.key.id]
                .into_iter()
                .any(|value| value.len() > 2048 || radar_core::log::contains_secret(value))
        })
    {
        action.subjects.clear();
        action.metadata.redacted = true;
        action.metadata.omitted_subjects = 1;
    }
    Effect::Observe {
        action,
        outcome: Outcome::Succeeded,
    }
}

pub(super) fn spawn(
    action: ActionDescriptor,
    site: String,
    key: String,
    owner: crate::page::Read,
    read: impl Future<Output = Result<DetailRead, String>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(JiraMsg::Detail {
                site,
                key,
                action: captured,
                read: owner,
                result: read.await.map(Box::new),
            }) as Msg
        }),
    }
}

/// The task's actual outcome was already observed. This only says its answer
/// did not own current page state; it never revises the original read outcome.
pub(super) fn skipped(scope: &ActionDescriptor) -> Effect {
    let mut action = ActionDescriptor::new(
        "jira.page.detail.application.skipped",
        Origin::Plugin("jira".into()),
    );
    action.subjects.clone_from(&scope.subjects);
    action.metadata.redacted = scope.metadata.redacted;
    action.metadata.omitted_subjects = scope.metadata.omitted_subjects;
    action.metadata.count = Some(1);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

fn count(
    scope: &ActionDescriptor,
    kind: &'static str,
    n: usize,
    truncated: bool,
    outcome: Outcome,
) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    action.metadata.truncated = truncated || n > u32::MAX as usize;
    action.metadata.reason = match outcome {
        Outcome::Failed => Some(Reason::Unspecified),
        Outcome::Blocked => Some(Reason::InvalidInput),
        _ => None,
    };
    Effect::Observe { action, outcome }
}

fn outcome(gap: bool) -> Outcome {
    if gap {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    }
}

fn link_gap(evidence: &LinkEvidence) -> bool {
    evidence.omitted_rows != 0 || evidence.incomplete_collection
}

fn links(
    effects: &mut Vec<Effect>,
    scope: &ActionDescriptor,
    kinds: [&'static str; 3],
    n: usize,
    evidence: &LinkEvidence,
) {
    effects.push(count(
        scope,
        kinds[0],
        n,
        false,
        outcome(link_gap(evidence)),
    ));
    if evidence.omitted_rows != 0 {
        effects.push(count(
            scope,
            kinds[1],
            evidence.omitted_rows,
            false,
            Outcome::Partial,
        ));
    }
    if evidence.incomplete_collection {
        effects.push(count(scope, kinds[2], 1, false, Outcome::Partial));
    }
}

pub(super) fn observed(
    scope: &ActionDescriptor,
    result: &Result<Box<DetailRead>, String>,
) -> Vec<Effect> {
    let Ok(got) = result else {
        // The existing fatal Result discards earlier issue/joined progress.
        // One returned task error is not a request/retry or missing-row count.
        return vec![
            Effect::Report(Outcome::Failed),
            count(scope, "jira.page.detail.errors", 1, false, Outcome::Failed),
        ];
    };
    let e = &got.evidence;
    let children_gap =
        e.children.truncated || e.children.omitted != 0 || e.children.incomplete_pages != 0;
    let comments_gap = e.comments.incomplete_collection || e.comments.total_defaulted;
    let code_gap =
        e.code.skipped_invalid_id || e.code.omitted_rows != 0 || e.code.incomplete_responses != 0;
    let gap = e.children_failed
        || children_gap
        || link_gap(&e.issue_links)
        || e.web_failed
        || link_gap(&e.web)
        || comments_gap
        || e.code_failed
        || code_gap;
    let mut effects = vec![
        Effect::Report(outcome(gap)),
        count(
            scope,
            "jira.page.detail.tickets",
            1,
            false,
            Outcome::Succeeded,
        ),
        count(
            scope,
            "jira.page.detail.parents",
            usize::from(got.detail.ticket.parent.is_some()),
            false,
            Outcome::Succeeded,
        ),
    ];
    if e.children_failed {
        effects.push(count(
            scope,
            "jira.page.detail.children.errors",
            1,
            false,
            Outcome::Failed,
        ));
    } else {
        effects.push(count(
            scope,
            "jira.page.detail.children",
            e.children_count,
            e.children.truncated,
            outcome(children_gap),
        ));
        for (kind, n, truncated) in [
            (
                "jira.page.detail.children.limits",
                usize::from(e.children.truncated),
                true,
            ),
            (
                "jira.page.detail.children.rows.omitted",
                e.children.omitted,
                false,
            ),
            (
                "jira.page.detail.children.pages.incomplete",
                e.children.incomplete_pages,
                false,
            ),
        ] {
            if n != 0 {
                effects.push(count(scope, kind, n, truncated, Outcome::Partial));
            }
        }
    }
    links(
        &mut effects,
        scope,
        [
            "jira.page.detail.issue.links",
            "jira.page.detail.issue.rows.omitted",
            "jira.page.detail.issue.collections.incomplete",
        ],
        e.issue_link_count,
        &e.issue_links,
    );
    if e.web_failed {
        effects.push(count(
            scope,
            "jira.page.detail.web.errors",
            1,
            false,
            Outcome::Failed,
        ));
    } else {
        links(
            &mut effects,
            scope,
            [
                "jira.page.detail.web.links",
                "jira.page.detail.web.rows.omitted",
                "jira.page.detail.web.collections.incomplete",
            ],
            e.web_link_count,
            &e.web,
        );
    }
    effects.push(count(
        scope,
        "jira.page.detail.comments",
        got.detail.comments.len(),
        false,
        outcome(comments_gap),
    ));
    for (kind, present) in [
        (
            "jira.page.detail.comments.collections.incomplete",
            e.comments.incomplete_collection,
        ),
        (
            "jira.page.detail.comments.total.defaulted",
            e.comments.total_defaulted,
        ),
    ] {
        if present {
            effects.push(count(scope, kind, 1, false, Outcome::Partial));
        }
    }
    if e.code_failed {
        effects.push(count(
            scope,
            "jira.page.detail.code.errors",
            1,
            false,
            Outcome::Failed,
        ));
    } else if e.code.skipped_invalid_id {
        effects.push(count(
            scope,
            "jira.page.detail.code.requests.skipped",
            1,
            false,
            Outcome::Blocked,
        ));
    } else {
        effects.push(count(
            scope,
            "jira.page.detail.code.links",
            got.detail.code.len(),
            false,
            outcome(code_gap),
        ));
        for (kind, n) in [
            ("jira.page.detail.code.rows.omitted", e.code.omitted_rows),
            (
                "jira.page.detail.code.responses.incomplete",
                e.code.incomplete_responses,
            ),
        ] {
            if n != 0 {
                effects.push(count(scope, kind, n, false, Outcome::Partial));
            }
        }
    }
    effects
}

#[cfg(test)]
mod tests;
