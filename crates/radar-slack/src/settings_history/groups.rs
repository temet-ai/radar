//! Group-load results and existing no-request template-cache use. Counts only;
//! cached workspace identity cannot establish the account used by this request.

use radar_core::journal::{ActionDescriptor, Origin, Outcome, Reason, StableCode};
use radar_core::plugin::Effect;

use crate::client::GroupsRead;

pub(crate) fn groups_action() -> ActionDescriptor {
    ActionDescriptor::new("slack.settings.groups.load", Origin::Plugin("slack".into()))
}

fn count(scope: &ActionDescriptor, kind: &'static str, n: usize, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    action.metadata.truncated = n > u32::MAX as usize;
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

/// A refused invocation, not an attempted request or a report into its caller.
pub(crate) fn groups_unavailable() -> Effect {
    let mut action = groups_action();
    action.kind = StableCode::new("slack.settings.groups.load.unavailable");
    action.metadata.count = Some(1);
    action.metadata.reason = Some(Reason::Unavailable);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

/// Group-entry occurrences already held when opening the template editor. No
/// freshness, membership validation or remote inventory is established here.
pub(crate) fn groups_cached(entries: usize) -> Effect {
    count(
        &groups_action(),
        "slack.settings.groups.load.cached",
        entries,
        Outcome::Succeeded,
    )
}

/// One completed logical request, including legacy retries. Collection and
/// membership-row gaps retain the existing permissive parser's exact evidence.
pub(crate) fn groups(
    action: &ActionDescriptor,
    result: &Result<GroupsRead, String>,
) -> Vec<Effect> {
    let Ok(got) = result else {
        return vec![
            Effect::Report(Outcome::Failed),
            count(
                action,
                "slack.settings.groups.load.request",
                1,
                Outcome::Failed,
            ),
        ];
    };
    let outcome = if got.incomplete_collection || got.incomplete_entries != 0 {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    };
    let mut effects = vec![
        Effect::Report(outcome),
        count(
            action,
            "slack.settings.groups.load.request",
            1,
            Outcome::Succeeded,
        ),
        count(
            action,
            "slack.settings.groups.load.returned",
            got.groups.len(),
            outcome,
        ),
    ];
    if got.incomplete_collection {
        effects.push(count(
            action,
            "slack.settings.groups.load.collections.incomplete",
            1,
            Outcome::Partial,
        ));
    }
    if got.incomplete_entries != 0 {
        effects.push(count(
            action,
            "slack.settings.groups.load.entries.incomplete",
            got.incomplete_entries,
            Outcome::Partial,
        ));
    }
    effects
}
