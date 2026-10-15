//! Settings read evidence, independent of cached account identity.
//! No configuration reference, credential, user identity or display text is kept.

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, StableCode, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::Effect;

use crate::client::Me;
use crate::sync_history::{auth_identity_complete, auth_workspace};

mod channels;
pub(crate) use channels::{
    channel, channel_action, channel_application_blocked, channel_unavailable,
};

mod groups;
pub(crate) use groups::{groups, groups_action, groups_cached, groups_unavailable};

pub(crate) fn workspace_action() -> ActionDescriptor {
    ActionDescriptor::new(
        "slack.settings.check.workspace",
        Origin::Plugin("slack".into()),
    )
}

/// A local refusal, not an attempted authentication request.
pub(crate) fn workspace_unavailable() -> Effect {
    let mut action = workspace_action();
    action.kind = StableCode::new("slack.settings.check.workspace.unavailable");
    action.metadata.count = Some(1);
    action.metadata.reason = Some(Reason::Unavailable);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

pub(crate) fn workspace_scope(
    action: &ActionDescriptor,
    result: &Result<Me, String>,
) -> ActionDescriptor {
    let mut scope = action.clone();
    if let Ok(me) = result {
        match auth_workspace(&me.team_id) {
            Some(key) => scope
                .subjects
                .push(SubjectSnapshot::new(key, SubjectRole::Result)),
            None => {
                scope.metadata.redacted = true;
                scope.metadata.omitted_subjects = scope.metadata.omitted_subjects.saturating_add(1);
            }
        }
    }
    scope
}

fn observation(scope: &ActionDescriptor, kind: &'static str, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(1);
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

/// One logical auth result, not a count of HTTP attempts or a permissions check.
pub(crate) fn workspace(action: &ActionDescriptor, result: &Result<Me, String>) -> Vec<Effect> {
    let Ok(me) = result else {
        return vec![
            Effect::Report(Outcome::Failed),
            observation(
                action,
                "slack.settings.check.workspace.auth",
                Outcome::Failed,
            ),
        ];
    };
    let outcome = if auth_identity_complete(me) {
        Outcome::Succeeded
    } else {
        Outcome::Partial
    };
    let scope = workspace_scope(action, result);
    vec![
        Effect::Report(outcome),
        observation(
            action,
            "slack.settings.check.workspace.auth",
            Outcome::Succeeded,
        ),
        observation(&scope, "slack.settings.check.workspace.identity", outcome),
    ]
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod groups_tests;

#[cfg(test)]
mod channels_tests;
