//! One existing auth/base/source check, captured before dispatch. The subject is
//! the requested server/source, never a returned user, group or repo identity.
//! Original evidence precedes the independent one-use application guard.

use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::{GlMsg, client::CheckRead, config};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Auth,
    Base,
    Source,
}

impl Kind {
    fn code(self) -> &'static str {
        match self {
            Self::Auth => "gitlab.settings.check.auth",
            Self::Base => "gitlab.settings.check.base",
            Self::Source => "gitlab.settings.check.source",
        }
    }
}

pub(super) fn action(kind: Kind, url: &str, path: &str) -> ActionDescriptor {
    let mut action = ActionDescriptor::new(kind.code(), Origin::Plugin("gitlab".into()));
    let subject = (|| {
        let instance = SafeUrl::parse(url).ok()?;
        let (subject_kind, id) = match kind {
            Kind::Auth => ("server", "server"),
            Kind::Base | Kind::Source => {
                if path.len() > 2048
                    || config::group_path_problem(path).is_some()
                    || path.split('/').any(|part| matches!(part, "." | ".."))
                {
                    return None;
                }
                ("source", path)
            }
        };
        let key = SubjectKey::new("gitlab", instance.as_str(), subject_kind, id).ok()?;
        // Check the bounded final stored identity, including URL normalization.
        if [&key.provider, &key.instance, &key.kind, &key.id]
            .into_iter()
            .any(|part| radar_core::log::contains_secret(part))
        {
            return None;
        }
        Some(key)
    })();
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

/// This only records entry to the existing auth callback. A renewal or second
/// token resolution failure remains a read failure once that callback ran.
#[derive(Default)]
pub(super) struct Attempt(AtomicBool);

impl Attempt {
    pub fn enter(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn finish(&self, result: anyhow::Result<CheckRead>) -> Result<CheckRead, CheckError> {
        result.map_err(|error| CheckError {
            entered: self.0.load(Ordering::Relaxed),
            message: format!("{error:#}"),
        })
    }
}

#[derive(Debug)]
pub(super) struct CheckError {
    entered: bool,
    pub message: String,
}

pub(super) fn spawn(
    action: ActionDescriptor,
    target: crate::settings_checks::Target,
    request: crate::settings_checks::Request,
    read: impl Future<Output = Result<CheckRead, CheckError>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(GlMsg::Checked {
                target,
                request,
                action: captured,
                result: read.await,
            }) as Msg
        }),
    }
}

fn count(
    kind: Kind,
    scope: &ActionDescriptor,
    suffix: &'static str,
    n: u32,
    outcome: Outcome,
    reason: Option<Reason>,
) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::try_new(&format!("{}.{suffix}", kind.code()))
        .expect("fixed settings-check vocabulary");
    action.metadata.count = Some(n);
    action.metadata.reason = reason;
    Effect::Observe { action, outcome }
}

/// One logical returned check, not HTTP-attempt counts. Display text, response
/// bodies and error strings are never used to construct journal evidence.
pub(super) fn observed(
    kind: Kind,
    scope: &ActionDescriptor,
    result: &Result<CheckRead, CheckError>,
) -> Vec<Effect> {
    let got = match result {
        Ok(got) => got,
        Err(error) => {
            let (outcome, suffix, reason) = if error.entered {
                (Outcome::Failed, "errors", Reason::Unspecified)
            } else {
                (Outcome::Blocked, "requests.skipped", Reason::Unavailable)
            };
            return vec![
                Effect::Report(outcome),
                count(kind, scope, suffix, 1, outcome, Some(reason)),
            ];
        }
    };
    let e = &got.evidence;
    let outcome = if e.incomplete() {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    };
    let mut effects = vec![
        Effect::Report(outcome),
        count(kind, scope, "replies", 1, outcome, None),
    ];
    for (suffix, n) in [
        ("objects.incomplete", e.incomplete_objects),
        ("fields.incomplete", e.incomplete_fields),
        ("fields.defaulted", e.defaulted_fields),
        ("numbers.lossy", e.lossy_numbers),
        ("paths.ambiguous", u32::from(e.ambiguous_path)),
    ] {
        if n != 0 {
            effects.push(count(kind, scope, suffix, n, Outcome::Partial, None));
        }
    }
    for (suffix, n) in [
        ("projects.reported", e.reported_projects),
        (
            "merge_requests.open.reported",
            e.reported_open_merge_requests,
        ),
    ] {
        if let Some(n) = n {
            effects.push(count(kind, scope, suffix, n, outcome, None));
        }
    }
    effects
}

/// Application evidence is separate from the read's original outcome and scope.
pub(super) fn skipped(kind: Kind, scope: &ActionDescriptor) -> Effect {
    count(
        kind,
        scope,
        "application.skipped",
        1,
        Outcome::Blocked,
        Some(Reason::Unavailable),
    )
}

#[cfg(test)]
mod tests;
