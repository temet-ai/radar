//! The list of the channels the token is in, for the picker. Counts only: no
//! channel name, identifier or error text is kept.

use radar_core::journal::{ActionDescriptor, Origin, Outcome, Reason, StableCode};
use radar_core::plugin::Effect;

use crate::client::JoinedRead;

pub(crate) fn joined_action() -> ActionDescriptor {
    ActionDescriptor::new("slack.settings.joined.load", Origin::Plugin("slack".into()))
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

/// A refused invocation, not an attempted request.
pub(crate) fn joined_unavailable() -> Effect {
    let mut action = joined_action();
    action.kind = StableCode::new("slack.settings.joined.load.unavailable");
    action.metadata.count = Some(1);
    action.metadata.reason = Some(Reason::Unavailable);
    Effect::Observe { action, outcome: Outcome::Blocked }
}

/// One completed list. A list cut short, or without the private channels,
/// is partial.
pub(crate) fn joined(action: &ActionDescriptor, result: &Result<JoinedRead, String>) -> Vec<Effect> {
    let Ok(got) = result else {
        return vec![
            Effect::Report(Outcome::Failed),
            count(action, "slack.settings.joined.load.request", 1, Outcome::Failed),
        ];
    };
    let outcome = if got.truncated || got.private_skipped { Outcome::Partial } else { Outcome::Succeeded };
    vec![
        Effect::Report(outcome),
        count(action, "slack.settings.joined.load.request", 1, Outcome::Succeeded),
        count(action, "slack.settings.joined.load.returned", got.channels.len(), outcome),
    ]
}
