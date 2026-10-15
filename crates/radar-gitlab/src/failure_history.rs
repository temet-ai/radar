//! The existing failed-job list and optional bounded tail bundle only.
//! Returned occurrences are evidence, never verified failure causes or job identities.

use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, SafeUrl, StableCode, SubjectKey, SubjectRole,
    SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::{GlMsg, config, join, why::DiagnosisRead};

pub(super) fn action(url: &str, project: &str, pipeline: u64) -> ActionDescriptor {
    let mut action =
        ActionDescriptor::new("gitlab.pipeline.diagnosis", Origin::Plugin("gitlab".into()));
    let subject = SafeUrl::parse(url).ok().and_then(|url| {
        if pipeline == 0
            || config::group_path_problem(project).is_some()
            || project.split('/').any(|part| matches!(part, "." | ".."))
        {
            return None;
        }
        // Match the existing journal pipeline ID, not the UI's project#id key.
        SubjectKey::new(
            "gitlab",
            url.as_str(),
            join::PIPELINE,
            &format!("{project}/{pipeline}"),
        )
        .ok()
        .map(|key| SubjectSnapshot::new(key, SubjectRole::Primary))
    });
    match subject {
        Some(subject) => action.subjects.push(subject),
        None => {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects = 1;
        }
    }
    action
}

pub(super) fn spawn(
    action: ActionDescriptor,
    request: crate::log_ownership::PipelineRead,
    read: impl Future<Output = DiagnosisRead> + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(GlMsg::Failed {
                request,
                action: captured,
                read: read.await,
            }) as Msg
        }),
    }
}

fn count(scope: &ActionDescriptor, kind: &'static str, n: usize, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    // Neither per_page=100 nor the five-tail bound proves a truncated response.
    action.metadata.truncated = n > u32::MAX as usize;
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

pub(super) fn observed(scope: &ActionDescriptor, read: &DiagnosisRead) -> Vec<Effect> {
    let Ok(got) = &read.result else {
        // The required list failed, so no tails were selected or read.
        return vec![
            Effect::Report(Outcome::Failed),
            count(
                scope,
                "gitlab.pipeline.diagnosis.list.errors",
                1,
                Outcome::Failed,
            ),
        ];
    };
    let incomplete = got.evidence.incomplete_response || got.evidence.incomplete_rows != 0;
    let errors = read.logs.as_ref().map_or(0, |logs| {
        logs.iter().filter(|(_, result)| result.is_err()).count()
    });
    let list_outcome = if incomplete {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    };
    let mut effects = vec![
        Effect::Report(if incomplete || errors != 0 {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        }),
        count(
            scope,
            "gitlab.pipeline.diagnosis.jobs",
            got.jobs.len(),
            list_outcome,
        ),
    ];
    for (kind, n) in [
        (
            "gitlab.pipeline.diagnosis.responses.incomplete",
            usize::from(got.evidence.incomplete_response),
        ),
        (
            "gitlab.pipeline.diagnosis.rows.incomplete",
            got.evidence.incomplete_rows,
        ),
    ] {
        if n != 0 {
            effects.push(count(scope, kind, n, Outcome::Partial));
        }
    }
    // No tail observation when logs=false, even when the list contains jobs.
    if let Some(logs) = &read.logs {
        effects.extend([
            count(
                scope,
                "gitlab.pipeline.diagnosis.tails.selected",
                logs.len(),
                Outcome::Succeeded,
            ),
            count(
                scope,
                "gitlab.pipeline.diagnosis.tails.succeeded",
                logs.len() - errors,
                Outcome::Succeeded,
            ),
        ]);
        if errors != 0 {
            effects.push(count(
                scope,
                "gitlab.pipeline.diagnosis.tails.failed",
                errors,
                Outcome::Failed,
            ));
        }
        let unselected = got.jobs.len().saturating_sub(logs.len());
        if unselected != 0 {
            // Known returned occurrences outside this bundle's intentional bound.
            // This does not make its completed bounded reads partial.
            effects.push(count(
                scope,
                "gitlab.pipeline.diagnosis.tails.unselected.bound",
                unselected,
                Outcome::Blocked,
            ));
        }
    }
    effects
}

pub(super) fn skipped(scope: &ActionDescriptor, kind: &'static str, n: usize) -> Effect {
    count(scope, kind, n, Outcome::Blocked)
}

#[cfg(test)]
mod tests;
