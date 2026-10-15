//! Completed For-you evidence. Query text and membership identities stay transient.

use super::*;
use crate::client::Found;

#[derive(Debug, Clone, Copy)]
pub(crate) enum SearchKind {
    Mentions,
    Direct,
    Group,
}

impl SearchKind {
    fn kind(self) -> &'static str {
        match self {
            Self::Mentions => "slack.sync.for_you.search.mentions",
            Self::Direct => "slack.sync.for_you.search.direct",
            Self::Group => "slack.sync.for_you.search.group",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SearchList {
    /// Returned entries before cross-query deduplication and final retention.
    pub returned: usize,
    pub evidence: ListEvidence,
}

#[derive(Debug)]
pub(crate) struct SearchObservation {
    pub kind: SearchKind,
    /// An error retains no invented results from earlier pages of this query.
    pub list: Option<SearchList>,
}

#[derive(Debug)]
pub(crate) enum GroupsEvidence {
    Cached {
        selected: usize,
    },
    Returned {
        returned: usize,
        selected: usize,
        incomplete_collection: bool,
        incomplete_entries: usize,
    },
    Failed {
        selected: usize,
    },
}

impl Default for GroupsEvidence {
    fn default() -> Self {
        Self::Cached { selected: 0 }
    }
}

impl GroupsEvidence {
    fn incomplete(&self) -> bool {
        match self {
            Self::Cached { .. } => false,
            Self::Returned {
                incomplete_collection,
                incomplete_entries,
                ..
            } => *incomplete_collection || *incomplete_entries != 0,
            Self::Failed { .. } => true,
        }
    }
}

#[derive(Debug, Default)]
pub(crate) struct ForYouEvidence {
    pub groups: GroupsEvidence,
    pub names: NamesEvidence,
    pub planned: usize,
    pub queries: Vec<SearchObservation>,
    pub duplicates: usize,
    /// Unique candidate entries before the existing final 80-entry retention.
    pub unique: usize,
    /// Safe descriptors for retained returned entries, not proof of cache merging.
    pub retained: Vec<ActionDescriptor>,
}

pub(crate) fn for_you_action(team: &str) -> ActionDescriptor {
    let mut action = followed_action(team);
    action.kind = StableCode::new("slack.sync.for_you");
    action
}

pub(crate) fn for_you_message(team: &str, found: &Found) -> ActionDescriptor {
    // Reuse the canonical team/channel/timestamp validation, without treating a
    // search message as a root just because the legacy UI opens it as a thread.
    let mut action = followed_target(team, &found.channel_id, &found.message.ts);
    action.kind = StableCode::new("slack.sync.for_you.message.observed");
    action.metadata.count = Some(1);
    for subject in &mut action.subjects {
        subject.key.kind = "message".into();
        subject.role = SubjectRole::Result;
    }
    if let Some(root) = found.message.thread_ts.as_deref() {
        let context = followed_target(team, &found.channel_id, root);
        if let Some(mut subject) = context.subjects.into_iter().next() {
            subject.role = SubjectRole::Context;
            action.subjects.push(subject);
        } else {
            action.metadata.redacted = true;
            action.metadata.omitted_subjects += 1;
        }
    }
    action
}

/// Feed failure does not erase successful auxiliary calls or earlier queries.
/// Every observation is made only after the whole task returns.
pub(crate) fn for_you(
    scope: &ActionDescriptor,
    evidence: &ForYouEvidence,
    feed_ok: bool,
) -> Vec<Effect> {
    let omitted = evidence.unique.saturating_sub(evidence.retained.len());
    let incomplete = omitted != 0
        || evidence.groups.incomplete()
        || evidence.names.incomplete()
        || evidence.queries.iter().any(|query| {
            query
                .list
                .is_none_or(|list| list_outcome(list.evidence) != Outcome::Succeeded)
        });
    let outcome = if !feed_ok {
        Outcome::Failed
    } else if incomplete {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    };
    let mut effects = vec![Effect::Report(outcome)];
    for (kind, n) in [
        ("slack.sync.for_you.queries.planned", evidence.planned),
        (
            "slack.sync.for_you.queries.attempted",
            evidence.queries.len(),
        ),
        (
            "slack.sync.for_you.queries.returned",
            evidence
                .queries
                .iter()
                .filter(|query| query.list.is_some())
                .count(),
        ),
        (
            "slack.sync.for_you.messages.returned",
            evidence
                .queries
                .iter()
                .filter_map(|query| query.list)
                .fold(0usize, |n, list| n.saturating_add(list.returned)),
        ),
        (
            "slack.sync.for_you.messages.duplicates",
            evidence.duplicates,
        ),
        ("slack.sync.for_you.messages.unique", evidence.unique),
    ] {
        effects.push(count(scope, kind, n, false, Outcome::Succeeded));
    }
    effects.push(count(
        scope,
        "slack.sync.for_you.messages.retained",
        evidence.retained.len(),
        omitted != 0,
        if omitted == 0 {
            Outcome::Succeeded
        } else {
            Outcome::Partial
        },
    ));
    if omitted != 0 {
        effects.push(count(
            scope,
            "slack.sync.for_you.messages.omitted",
            omitted,
            true,
            Outcome::Partial,
        ));
    }
    let deferred = evidence.planned.saturating_sub(evidence.queries.len());
    if deferred != 0 {
        effects.push(count(
            scope,
            "slack.sync.for_you.queries.deferred",
            deferred,
            false,
            Outcome::Partial,
        ));
    }
    for query in &evidence.queries {
        let mut action = scope.clone();
        action.kind = StableCode::new(query.kind.kind());
        match query.list {
            Some(list) => {
                effects.push(count(
                    &action,
                    query.kind.kind(),
                    list.returned,
                    list.evidence.truncated,
                    list_outcome(list.evidence),
                ));
                effects.extend(gaps(
                    &action,
                    list.evidence,
                    usize::from(list.evidence.truncated),
                    NamesEvidence::default(),
                ));
            }
            None => {
                action.metadata.reason = Some(Reason::Unspecified);
                effects.push(Effect::Observe {
                    action,
                    outcome: Outcome::Failed,
                });
            }
        }
    }
    match evidence.groups {
        GroupsEvidence::Cached { selected } => {
            effects.push(count(
                scope,
                "slack.sync.for_you.groups.cached",
                selected,
                false,
                Outcome::Succeeded,
            ));
        }
        GroupsEvidence::Returned {
            returned,
            selected,
            incomplete_collection,
            incomplete_entries,
        } => {
            effects.push(count(
                scope,
                "slack.sync.for_you.groups.attempted",
                1,
                false,
                Outcome::Succeeded,
            ));
            effects.push(count(
                scope,
                "slack.sync.for_you.groups.returned",
                returned,
                false,
                if incomplete_collection || incomplete_entries != 0 {
                    Outcome::Partial
                } else {
                    Outcome::Succeeded
                },
            ));
            effects.push(count(
                scope,
                "slack.sync.for_you.groups.selected",
                selected,
                false,
                Outcome::Succeeded,
            ));
            if incomplete_collection {
                effects.push(count(
                    scope,
                    "slack.sync.for_you.groups.collections.incomplete",
                    1,
                    false,
                    Outcome::Partial,
                ));
            }
            if incomplete_entries != 0 {
                effects.push(count(
                    scope,
                    "slack.sync.for_you.groups.entries.incomplete",
                    incomplete_entries,
                    false,
                    Outcome::Partial,
                ));
            }
        }
        GroupsEvidence::Failed { selected } => {
            effects.push(count(
                scope,
                "slack.sync.for_you.groups.attempted",
                1,
                false,
                Outcome::Succeeded,
            ));
            effects.push(count(
                scope,
                "slack.sync.for_you.groups.errors",
                1,
                false,
                Outcome::Failed,
            ));
            effects.push(count(
                scope,
                "slack.sync.for_you.groups.fallback",
                selected,
                false,
                Outcome::Partial,
            ));
        }
    }
    effects.extend(gaps(scope, ListEvidence::default(), 0, evidence.names));
    effects.extend(
        evidence
            .retained
            .iter()
            .cloned()
            .map(|action| Effect::Observe {
                action,
                outcome: Outcome::Succeeded,
            }),
    );
    effects
}
