//! Required job-metadata + tail and earlier-chunk reads, observed at completion.
//! No per-attempt progress, remote identity, range validity or log completeness.

use std::future::Future;

use radar_core::journal::{ActionDescriptor, Outcome, Reason, StableCode};
use radar_core::plugin::{Effect, Msg};

use crate::client::{Client, JobRead, TraceEvidence, TraceInterpretation, TraceRange, TraceRead};
use crate::{GlMsg, log_ownership::LogRead};

#[derive(Debug, Clone)]
pub(crate) struct JobLogRead {
    pub job: JobRead,
    pub trace: TraceRead,
}

/// Both reads must remain inside one auth closure. A rejected tail retries the
/// metadata too, and a final error discards all intermediate successful metadata.
pub(super) async fn read(client: &Client, project: &str, id: u64) -> anyhow::Result<JobLogRead> {
    let job = client.job_observed(project, id).await?;
    let trace = client.trace_observed(project, id, None).await?;
    Ok(JobLogRead { job, trace })
}

pub(super) fn spawn(
    request: LogRead,
    read: impl Future<Output = Result<JobLogRead, String>> + Send + 'static,
) -> Effect {
    Effect::SpawnAction {
        action: request.action.clone(),
        task: Box::pin(async move {
            Box::new(GlMsg::JobLog {
                request,
                result: read.await,
            }) as Msg
        }),
    }
}

/// A single required earlier trace, in its own action and auth closure.
pub(super) fn earlier_spawn(
    request: LogRead,
    read: impl Future<Output = Result<TraceRead, String>> + Send + 'static,
) -> Effect {
    Effect::SpawnAction {
        action: request.action.clone(),
        task: Box::pin(async move {
            Box::new(GlMsg::EarlierLog {
                request,
                result: read.await,
            }) as Msg
        }),
    }
}

fn count(scope: &ActionDescriptor, kind: &str, n: u64, outcome: Outcome) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::try_new(kind).expect("declared log-history vocabulary");
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    // Only numeric representation saturation, never a requested log-size bound.
    action.metadata.truncated = n > u32::MAX.into();
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

pub(super) fn observed(
    scope: &ActionDescriptor,
    result: &Result<JobLogRead, String>,
) -> Vec<Effect> {
    let Ok(got) = result else {
        return vec![
            Effect::Report(Outcome::Failed),
            count(scope, "gitlab.job.log.errors", 1, Outcome::Failed),
        ];
    };
    let mut effects = vec![Effect::Report(if got.job.evidence.nonobject_response {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    })];
    if got.job.evidence.nonobject_response {
        effects.push(count(
            scope,
            "gitlab.job.log.metadata.responses.nonobject",
            1,
            Outcome::Partial,
        ));
    }
    effects.extend(trace_observed(scope, got.trace.evidence, TraceScope::Tail));
    effects
}

pub(super) fn earlier_observed(
    scope: &ActionDescriptor,
    result: &Result<TraceRead, String>,
) -> Vec<Effect> {
    let Ok(got) = result else {
        return vec![
            Effect::Report(Outcome::Failed),
            count(scope, "gitlab.job.log.earlier.errors", 1, Outcome::Failed),
        ];
    };
    let mut effects = vec![Effect::Report(Outcome::Succeeded)];
    effects.extend(trace_observed(scope, got.evidence, TraceScope::Earlier));
    effects
}

/// Closed component vocabulary; never derived from a request or response string.
enum TraceScope {
    Tail,
    Earlier,
}

fn trace_observed(
    scope: &ActionDescriptor,
    evidence: TraceEvidence,
    part: TraceScope,
) -> Vec<Effect> {
    let prefix = match part {
        TraceScope::Tail => "gitlab.job.log.tail",
        TraceScope::Earlier => "gitlab.job.log.earlier",
    };
    let fact = |suffix: &str, n| count(scope, &format!("{prefix}.{suffix}"), n, Outcome::Succeeded);
    let mut effects = vec![fact("body.supplied.bytes", evidence.supplied_bytes as u64)];
    match evidence.interpretation {
        TraceInterpretation::Legacy416Default => effects.push(fact("default.status416", 1)),
        TraceInterpretation::AcceptedBody { range, lossy_utf8 } => {
            effects.push(fact("body.accepted", 1));
            match range {
                TraceRange::Parsed { start, total } => {
                    effects.push(fact("range.parsed.start", start));
                    effects.push(fact("range.parsed.total", total));
                }
                TraceRange::Missing => effects.push(fact("range.fallback.missing", 1)),
                TraceRange::Unparseable => effects.push(fact("range.fallback.unparseable", 1)),
            }
            if lossy_utf8 {
                effects.push(fact("text.lossy", 1));
            }
        }
    }
    effects
}

#[cfg(test)]
mod tests;
