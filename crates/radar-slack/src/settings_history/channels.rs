//! Bounded channel-resolution observations. The common API endpoint and cached
//! account are not workspace identity: every descriptor here is unscoped.

use radar_core::journal::{ActionDescriptor, Origin, Outcome, Reason, StableCode};
use radar_core::plugin::Effect;

use crate::client::{ChannelEnd, ChannelEvidence, ChannelKind, ChannelMembership};

pub(crate) fn channel_action() -> ActionDescriptor {
    ActionDescriptor::new(
        "slack.settings.channel.resolve",
        Origin::Plugin("slack".into()),
    )
}

pub(crate) fn channel_unavailable() -> Effect {
    let mut action = channel_action();
    action.kind = StableCode::new("slack.settings.channel.resolve.unavailable");
    action.metadata.count = Some(1);
    action.metadata.reason = Some(Reason::Unavailable);
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

fn count(
    scope: &ActionDescriptor,
    phase: Option<ChannelKind>,
    suffix: &'static str,
    n: usize,
    outcome: Outcome,
) -> Effect {
    let mut action = scope.clone();
    let phase = match phase {
        Some(ChannelKind::Info) => ".info",
        Some(ChannelKind::Yours) => ".yours",
        Some(ChannelKind::Public) => ".public",
        None => "",
    };
    // Only the closed enum and static suffix vocabulary can reach this code.
    action.kind = StableCode::try_new(&format!("slack.settings.channel.resolve{phase}.{suffix}"))
        .expect("static channel-resolution vocabulary");
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    action.metadata.truncated = n > u32::MAX as usize || suffix == "cap";
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

/// The lookup may have succeeded while its identity cannot safely be applied.
/// Keep this local refusal separate from both network evidence and stale replies.
pub(crate) fn channel_application_blocked(action: &ActionDescriptor) -> Effect {
    count(action, None, "apply.blocked.identity", 1, Outcome::Blocked)
}

/// Evidence describes only the finished lookup, never config persistence or the
/// add's subsequent refresh. A failed request may retain genuine earlier page
/// responses; an interrupted task never delivers this end-of-task evidence.
pub(crate) fn channel(action: &ActionDescriptor, evidence: &ChannelEvidence) -> Vec<Effect> {
    let gaps = evidence.has_gaps();
    let outcome = match evidence.end() {
        Some(ChannelEnd::Request) => Outcome::Failed,
        Some(ChannelEnd::LocalCap) => Outcome::Partial,
        Some(ChannelEnd::Found | ChannelEnd::NoMatch) if gaps => Outcome::Partial,
        Some(ChannelEnd::Found) => Outcome::Succeeded,
        Some(ChannelEnd::NoMatch) => Outcome::Failed,
        None => Outcome::Unknown,
    };
    let mut effects = vec![Effect::Report(outcome)];
    for phase in evidence.phases.iter().flatten() {
        let kind = Some(phase.kind);
        let quality = if phase.has_gaps() {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        };
        if phase.requests != 0 {
            effects.push(count(
                action,
                kind,
                "requests",
                phase.requests,
                Outcome::Succeeded,
            ));
            effects.push(count(action, kind, "returned", phase.returned, quality));
        }
        for (suffix, n) in [
            ("fields.defaulted", phase.defaulted_fields),
            ("identity.incomplete", phase.identity_gaps),
            ("pages.incomplete", phase.incomplete_pages),
        ] {
            if n != 0 {
                effects.push(count(action, kind, suffix, n, Outcome::Partial));
            }
        }
        match phase.end {
            ChannelEnd::Found => effects.push(count(action, kind, "found", 1, quality)),
            ChannelEnd::NoMatch => effects.push(count(
                action,
                kind,
                if phase.has_gaps() {
                    "no_match.incomplete"
                } else {
                    "no_match"
                },
                1,
                quality,
            )),
            ChannelEnd::Request => {
                effects.push(count(action, kind, "request.failed", 1, Outcome::Failed))
            }
            ChannelEnd::LocalCap => effects.push(count(action, kind, "cap", 1, Outcome::Partial)),
        }
        if let Some(membership) = phase.membership {
            let (suffix, outcome) = match membership {
                ChannelMembership::Explicit => ("membership.explicit", Outcome::Succeeded),
                ChannelMembership::ListDerived => ("membership.list_derived", Outcome::Succeeded),
                ChannelMembership::Defaulted => ("membership.defaulted", Outcome::Partial),
            };
            effects.push(count(action, kind, suffix, 1, outcome));
        }
    }
    // A complete negative search failed to resolve a channel, but none of its
    // requests failed and it does not establish absence from a workspace.
    if evidence.end() == Some(ChannelEnd::NoMatch) {
        effects.push(count(
            action,
            None,
            if gaps {
                "no_match.incomplete"
            } else {
                "no_match"
            },
            1,
            outcome,
        ));
    }
    effects
}
