//! Bootstrap learns its workspace from the response, never from the token's
//! configured name or an identity left in the cache before authentication.

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, SubjectKey, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::Effect;

use crate::client::{BootstrapRead, Me, TimezoneEvidence};

use super::{count, team_is_id};

pub(crate) fn bootstrap_action() -> ActionDescriptor {
    ActionDescriptor::new("slack.sync.bootstrap", Origin::Plugin("slack".into()))
}

pub(crate) fn auth_workspace(team: &str) -> Option<SubjectKey> {
    team_is_id(team)
        .then(|| SubjectKey::new("slack", team, "workspace", "workspace").ok())
        .flatten()
}

pub(crate) fn auth_identity_complete(me: &Me) -> bool {
    // Validate the auth response without retaining a user identity in history.
    let user = &me.user_id;
    auth_workspace(&me.team_id).is_some()
        && user.strip_prefix(['U', 'W']).is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        })
        && SubjectKey::new("slack", &me.team_id, "user", user).is_ok()
}

pub(crate) fn bootstrap_scope(
    action: &ActionDescriptor,
    result: &Result<BootstrapRead, String>,
) -> ActionDescriptor {
    let mut scope = action.clone();
    if let Ok(got) = result {
        match auth_workspace(&got.me.team_id) {
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

pub(crate) fn bootstrap(
    action: &ActionDescriptor,
    result: &Result<BootstrapRead, String>,
) -> Vec<Effect> {
    let Ok(got) = result else {
        return vec![
            Effect::Report(Outcome::Failed),
            count(
                action,
                "slack.sync.bootstrap.auth",
                1,
                false,
                Outcome::Failed,
            ),
        ];
    };
    let complete = auth_identity_complete(&got.me);
    let outcome = if complete && got.timezone == TimezoneEvidence::Explicit {
        Outcome::Succeeded
    } else {
        Outcome::Partial
    };
    let scope = bootstrap_scope(action, result);
    let (kind, timezone_outcome) = match got.timezone {
        TimezoneEvidence::NotRequested => (
            "slack.sync.bootstrap.timezone.not_requested",
            Outcome::Blocked,
        ),
        TimezoneEvidence::Explicit => {
            ("slack.sync.bootstrap.timezone.returned", Outcome::Succeeded)
        }
        TimezoneEvidence::Defaulted => {
            ("slack.sync.bootstrap.timezone.defaulted", Outcome::Partial)
        }
        TimezoneEvidence::Failed => ("slack.sync.bootstrap.timezone.failed", Outcome::Failed),
    };
    vec![
        Effect::Report(outcome),
        count(
            action,
            "slack.sync.bootstrap.auth",
            1,
            false,
            Outcome::Succeeded,
        ),
        count(
            &scope,
            "slack.sync.bootstrap.identity",
            1,
            false,
            if complete {
                Outcome::Succeeded
            } else {
                Outcome::Partial
            },
        ),
        count(&scope, kind, 1, false, timezone_outcome),
    ]
}
