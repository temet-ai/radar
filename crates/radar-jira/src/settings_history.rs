//! Bounded settings-check evidence, scoped to the configured request target.
//! Identity/display/error payloads stay transient and never enter the journal.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::JiraMsg;
use crate::client::{CheckError, CheckRead, Client};
use crate::config::{Site, is_project_key};

#[derive(Debug, Clone, Copy)]
pub(super) enum Kind {
    Site,
    Project,
}

impl Kind {
    fn codes(self) -> [&'static str; 7] {
        match self {
            Self::Site => [
                "jira.settings.check.site",
                "jira.settings.check.site.replies",
                "jira.settings.check.site.fields.incomplete",
                "jira.settings.check.site.fields.defaulted",
                "jira.settings.check.site.errors",
                "jira.settings.check.site.requests.skipped",
                "jira.settings.check.site.unavailable",
            ],
            Self::Project => [
                "jira.settings.check.project",
                "jira.settings.check.project.replies",
                "jira.settings.check.project.fields.incomplete",
                "jira.settings.check.project.fields.defaulted",
                "jira.settings.check.project.errors",
                "jira.settings.check.project.requests.skipped",
                "jira.settings.check.project.unavailable",
            ],
        }
    }
}

pub(super) fn action(kind: Kind, site: Option<&Site>, project: &str) -> ActionDescriptor {
    let mut action = ActionDescriptor::new(kind.codes()[0], Origin::Plugin("jira".into()));
    let subject = site.and_then(|site| {
        let (kind, id) = match kind {
            Kind::Site => ("site", "site"),
            Kind::Project if is_project_key(project) => ("project", project),
            Kind::Project => return None,
        };
        let instance = SafeUrl::parse(&site.base_url().ok()?).ok()?;
        SubjectKey::new("jira", instance.as_str(), kind, id).ok()
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

pub(super) fn spawn(
    target: crate::settings_checks::Target,
    request: crate::settings_checks::Request,
    action: ActionDescriptor,
    read: impl Future<Output = Result<CheckRead<String>, CheckError>> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(JiraMsg::Checked {
                target,
                request,
                action: captured,
                result: read.await,
            }) as Msg
        }),
    }
}

pub(super) async fn read_site(
    client: anyhow::Result<Client>,
) -> Result<CheckRead<String>, CheckError> {
    let got = client
        .map_err(CheckError::Setup)?
        .myself_observed()
        .await
        .map_err(CheckError::Read)?;
    Ok(CheckRead {
        value: got.value.name,
        evidence: got.evidence,
    })
}

pub(super) async fn read_project(
    client: anyhow::Result<Client>,
    project: &str,
) -> Result<CheckRead<String>, CheckError> {
    // Client setup preceded key validation before capture and still does.
    let got = client
        .map_err(CheckError::Setup)?
        .project_observed(project)
        .await?;
    Ok(CheckRead {
        value: format!("found {}", got.value),
        evidence: got.evidence,
    })
}

/// Preserve the read's original outcome; this independent observation only
/// describes local application refusal, using the captured safe request scope.
pub(super) fn skipped(kind: Kind, scope: &ActionDescriptor) -> Effect {
    let code = match kind {
        Kind::Site => "jira.settings.check.site.application.skipped",
        Kind::Project => "jira.settings.check.project.application.skipped",
    };
    let mut action = ActionDescriptor::new(code, Origin::Plugin("jira".into()));
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
    count: u32,
    outcome: Outcome,
    reason: Option<Reason>,
) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(count);
    action.metadata.reason = reason;
    Effect::Observe { action, outcome }
}

/// No configured row means no task or UI change, only an independent observation.
pub(super) fn unavailable(kind: Kind) -> Effect {
    count(
        &action(kind, None, ""),
        kind.codes()[6],
        1,
        Outcome::Blocked,
        Some(Reason::Unavailable),
    )
}

/// Observe the actual bounded result before the existing local application.
/// Counts describe fields/replies or one logical error, never HTTP attempts.
pub(super) fn observed(
    kind: Kind,
    scope: &ActionDescriptor,
    result: &Result<CheckRead<String>, CheckError>,
) -> Vec<Effect> {
    let codes = kind.codes();
    let got = match result {
        Ok(got) => got,
        Err(error) => {
            let (outcome, code, reason) = match error {
                CheckError::Setup(_) => (Outcome::Blocked, codes[5], Reason::Unavailable),
                CheckError::InvalidProject(_) => (Outcome::Blocked, codes[5], Reason::InvalidInput),
                CheckError::Read(_) => (Outcome::Failed, codes[4], Reason::Unspecified),
            };
            return vec![
                Effect::Report(outcome),
                count(scope, code, 1, outcome, Some(reason)),
            ];
        }
    };
    let outcome = if got.evidence.incomplete_fields != 0 {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    };
    let mut effects = vec![
        Effect::Report(outcome),
        count(scope, codes[1], 1, outcome, None),
    ];
    for (code, n) in [
        (codes[2], got.evidence.incomplete_fields),
        (codes[3], got.evidence.defaulted_fields),
    ] {
        if n != 0 {
            effects.push(count(scope, code, n, Outcome::Partial, None));
        }
    }
    effects
}

#[cfg(test)]
mod tests;
