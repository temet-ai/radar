//! Typed read evidence for Slack refreshes and independent local page opens.
//! Descriptors and evidence travel with tasks and never enter the provider cache.

use std::collections::BTreeMap;
use std::future::Future;

use radar_core::journal::{
    ActionDescriptor, Origin, Outcome, Reason, StableCode, SubjectKey, SubjectRole, SubjectSnapshot,
};
use radar_core::plugin::{Effect, Msg};

use crate::client::{ChannelInfo, ListEvidence, Message, MessagesRead, Reaction};
use crate::{SlackMsg, config, read::Read};

mod bootstrap;
pub(crate) use bootstrap::{
    auth_identity_complete, auth_workspace, bootstrap, bootstrap_action, bootstrap_scope,
};

mod for_you;
pub(crate) use for_you::{
    ForYouEvidence, GroupsEvidence, SearchKind, SearchList, SearchObservation, for_you,
    for_you_action, for_you_message,
};

#[derive(Debug)]
pub(crate) struct ChannelRead {
    pub info: Option<ChannelInfo>,
    pub list: MessagesRead,
}

#[derive(Debug, Default)]
pub(crate) struct ThreadsRead {
    pub threads: BTreeMap<String, Vec<Message>>,
    pub evidence: ListEvidence,
    /// Number of individual replies lists stopped at an actual bound.
    pub cutoffs: usize,
    /// Stale parent-message candidates outside the existing selection bound.
    pub deferred: usize,
    /// Threads read only past their newest held reply: folded into what is
    /// held rather than replacing it.
    pub merged: std::collections::BTreeSet<String>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NamesEvidence {
    pub attempted: usize,
    pub errors: usize,
    /// Eligible unknown users left unattempted, after a bound or first error.
    pub deferred: usize,
}

impl NamesEvidence {
    fn incomplete(self) -> bool {
        self.errors != 0 || self.deferred != 0
    }
}

fn team_is_id(team: &str) -> bool {
    team.strip_prefix('T').is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
    })
}

fn scoped(kind: &'static str, subject: Option<SubjectKey>) -> ActionDescriptor {
    let mut action = ActionDescriptor::new(kind, Origin::Plugin("slack".into()));
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

pub(super) fn action(kind: &'static str, team: &str, channel: &str) -> ActionDescriptor {
    let subject = (team_is_id(team) && config::is_channel_id(channel))
        .then(|| SubjectKey::new("slack", team, "channel", channel).ok())
        .flatten();
    scoped(kind, subject)
}

pub(super) fn followed_action(team: &str) -> ActionDescriptor {
    let subject = team_is_id(team)
        .then(|| SubjectKey::new("slack", team, "workspace", "workspace").ok())
        .flatten();
    scoped("slack.sync.followed", subject)
}

pub(super) fn followed_target(team: &str, channel: &str, ts: &str) -> ActionDescriptor {
    thread_action("slack.sync.followed.thread", team, channel, ts)
}

pub(super) fn page_thread_action(team: &str, channel: &str, ts: &str) -> ActionDescriptor {
    thread_action("slack.page.thread", team, channel, ts)
}

/// Only the accepted local page push, independent of fetching, painting or
/// reading its content. Unsafe scope omission must never change navigation.
pub(super) fn page_opened(team: &str, channel: &str, ts: &str) -> Effect {
    let mut action = page_thread_action(team, channel, ts);
    action.kind = StableCode::new("slack.page.open");
    // Check the exact retained identity after validation, with bounded scans.
    // This additional protection applies only to the new open observation.
    if action.subjects.is_empty()
        || action.subjects.iter().any(|subject| {
            [&subject.key.instance, &subject.key.id]
                .into_iter()
                .any(|value| value.len() > 2048 || radar_core::log::contains_secret(value))
        })
    {
        action.subjects.clear();
        action.metadata.redacted = true;
        action.metadata.omitted_subjects = 1;
    }
    Effect::Observe {
        action,
        outcome: Outcome::Succeeded,
    }
}

fn thread_action(kind: &'static str, team: &str, channel: &str, ts: &str) -> ActionDescriptor {
    // The same canonical channel/root-ts identity used by Slack thread pages.
    let valid_ts = ts.split_once('.').is_some_and(|(seconds, fraction)| {
        !seconds.is_empty()
            && !fraction.is_empty()
            && seconds
                .bytes()
                .chain(fraction.bytes())
                .all(|b| b.is_ascii_digit())
    });
    let subject = (team_is_id(team) && config::is_channel_id(channel) && valid_ts)
        .then(|| SubjectKey::new("slack", team, "thread", &format!("{channel}/{ts}")).ok())
        .flatten();
    scoped(kind, subject)
}

/// A refused invocation is independent of the write or read that triggered it.
/// Its count is not an attempted network request and it has no started event.
pub(super) fn page_thread_blocked(scope: &ActionDescriptor, unavailable: bool) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(if unavailable {
        "slack.page.thread.unavailable"
    } else {
        "slack.page.thread.suppressed"
    });
    action.metadata.count = Some(1);
    action.metadata.reason = Some(if unavailable {
        Reason::Unavailable
    } else {
        Reason::Unspecified
    });
    Effect::Observe {
        action,
        outcome: Outcome::Blocked,
    }
}

/// The response/list contract only: even an empty complete list is successful.
/// This does not establish that the requested parent exists or was returned.
pub(super) fn page_thread(
    scope: &ActionDescriptor,
    result: &Result<MessagesRead, String>,
) -> Vec<Effect> {
    let Ok(got) = result else {
        return vec![
            Effect::Report(Outcome::Failed),
            count(scope, "slack.page.thread.errors", 1, false, Outcome::Failed),
        ];
    };
    let outcome = list_outcome(got.evidence);
    let mut effects = vec![
        Effect::Report(outcome),
        // Actual returned entries, including any parent, before cache application.
        count(
            scope,
            "slack.page.thread.messages",
            got.messages.len(),
            got.evidence.truncated,
            outcome,
        ),
    ];
    if got.evidence.truncated {
        effects.push(count(
            scope,
            "slack.page.thread.lists.truncated",
            1,
            true,
            Outcome::Partial,
        ));
    }
    if got.evidence.incomplete_pages != 0 {
        effects.push(count(
            scope,
            "slack.page.thread.pages.incomplete",
            got.evidence.incomplete_pages,
            false,
            Outcome::Partial,
        ));
    }
    effects
}

#[derive(Debug, Default)]
pub(crate) struct FollowedBatch {
    /// The unchanged legacy cache updates, including declared/fallback counts.
    pub updates: Vec<(String, String, u32, Vec<Reaction>)>,
    /// One result per attempted candidate, preserving duplicates and order.
    pub targets: Vec<FollowedTarget>,
}

impl FollowedBatch {
    /// The same completed-batch evidence governs history and sync health.
    /// Cache merging, declared reply counts and UI state are not evidence.
    pub(crate) fn outcome(&self) -> Outcome {
        if !self
            .targets
            .iter()
            .any(|target| target.response.is_some_and(|r| r.parent_found))
        {
            Outcome::Failed
        } else if self.targets.iter().any(|target| {
            target
                .response
                .is_none_or(|r| !r.parent_found || list_outcome(r.evidence) != Outcome::Succeeded)
        }) {
            Outcome::Partial
        } else {
            Outcome::Succeeded
        }
    }
}

#[derive(Debug)]
pub(crate) struct FollowedTarget {
    pub action: ActionDescriptor,
    /// None means the target's replies read returned an error, not an empty list.
    pub response: Option<FollowedResponse>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct FollowedResponse {
    pub parent_found: bool,
    /// Actual legacy entries returned by replies, including any parent.
    pub messages: usize,
    pub evidence: ListEvidence,
}

pub(super) fn spawn<F: Future<Output = Read> + Send + 'static>(
    stamp: crate::read_context::ReadStamp,
    action: ActionDescriptor,
    read: impl FnOnce(ActionDescriptor) -> F + Send + 'static,
) -> Effect {
    let captured = action.clone();
    Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            Box::new(SlackMsg::Read {
                stamp,
                read: Box::new(read(captured).await),
            }) as Msg
        }),
    }
}

fn count(
    scope: &ActionDescriptor,
    kind: &'static str,
    n: usize,
    truncated: bool,
    outcome: Outcome,
) -> Effect {
    let mut action = scope.clone();
    action.kind = StableCode::new(kind);
    action.metadata.count = Some(u32::try_from(n).unwrap_or(u32::MAX));
    action.metadata.truncated = truncated || n > u32::MAX as usize;
    if outcome == Outcome::Failed {
        action.metadata.reason = Some(Reason::Unspecified);
    }
    Effect::Observe { action, outcome }
}

fn list_outcome(evidence: ListEvidence) -> Outcome {
    if evidence.truncated || evidence.incomplete_pages != 0 {
        Outcome::Partial
    } else {
        Outcome::Succeeded
    }
}

fn gaps(
    scope: &ActionDescriptor,
    evidence: ListEvidence,
    cutoffs: usize,
    names: NamesEvidence,
) -> Vec<Effect> {
    let mut effects = Vec::new();
    if cutoffs != 0 {
        effects.push(count(
            scope,
            "slack.sync.lists.truncated",
            cutoffs,
            true,
            Outcome::Partial,
        ));
    }
    if evidence.incomplete_pages != 0 {
        effects.push(count(
            scope,
            "slack.sync.pages.incomplete",
            evidence.incomplete_pages,
            false,
            Outcome::Partial,
        ));
    }
    if names.attempted != 0 {
        effects.push(count(
            scope,
            "slack.sync.names.attempted",
            names.attempted,
            false,
            Outcome::Succeeded,
        ));
        // Completed calls, not proof of nonempty or semantically valid profiles.
        effects.push(count(
            scope,
            "slack.sync.names.returned",
            names.attempted.saturating_sub(names.errors),
            false,
            Outcome::Succeeded,
        ));
    }
    if names.errors != 0 {
        effects.push(count(
            scope,
            "slack.sync.names.errors",
            names.errors,
            false,
            Outcome::Failed,
        ));
    }
    if names.deferred != 0 {
        effects.push(count(
            scope,
            "slack.sync.names.deferred",
            names.deferred,
            false,
            Outcome::Blocked,
        ));
    }
    effects
}

fn failed(scope: &ActionDescriptor) -> Vec<Effect> {
    vec![
        Effect::Report(Outcome::Failed),
        // One returned task error. Earlier pages/threads were not returned.
        count(scope, "slack.sync.errors", 1, false, Outcome::Failed),
    ]
}

pub(super) fn channel(
    scope: &ActionDescriptor,
    result: &Result<ChannelRead, String>,
    names: NamesEvidence,
) -> Vec<Effect> {
    let Ok(got) = result else {
        return failed(scope);
    };
    let evidence = got.list.evidence;
    let list = list_outcome(evidence);
    let aggregate = if names.incomplete() {
        Outcome::Partial
    } else {
        list
    };
    let mut effects = vec![
        Effect::Report(aggregate),
        // Legacy returned entries after intentional join/leave filtering. The
        // permissive parser and optional channel-info payload are not validated.
        count(
            scope,
            "slack.sync.channel.messages",
            got.list.messages.len(),
            evidence.truncated,
            list,
        ),
    ];
    effects.extend(gaps(
        scope,
        evidence,
        usize::from(evidence.truncated),
        names,
    ));
    effects
}

pub(super) fn threads(
    scope: &ActionDescriptor,
    result: &Result<ThreadsRead, String>,
    names: NamesEvidence,
) -> Vec<Effect> {
    let Ok(got) = result else {
        return failed(scope);
    };
    let list = list_outcome(got.evidence);
    let aggregate = if names.incomplete() || got.deferred != 0 {
        Outcome::Partial
    } else {
        list
    };
    let replies = got
        .threads
        .values()
        .fold(0usize, |n, rows| n.saturating_add(rows.len()));
    let mut effects = vec![
        Effect::Report(aggregate),
        // Returned map entries, not all threads in the channel.
        count(
            scope,
            "slack.sync.threads.returned",
            got.threads.len(),
            false,
            list,
        ),
        count(
            scope,
            "slack.sync.threads.replies",
            replies,
            got.evidence.truncated,
            list,
        ),
    ];
    effects.extend(gaps(scope, got.evidence, got.cutoffs, names));
    if got.deferred != 0 {
        effects.push(count(
            scope,
            "slack.sync.threads.deferred",
            got.deferred,
            false,
            Outcome::Blocked,
        ));
    }
    effects
}

/// Completion observations made only when the whole followed-read task returns.
/// These do not fabricate start times or partial progress for interrupted tasks.
pub(super) fn followed(scope: &ActionDescriptor, batch: &FollowedBatch) -> Vec<Effect> {
    let parents = batch
        .targets
        .iter()
        .filter(|target| target.response.is_some_and(|r| r.parent_found))
        .count();
    let errors = batch
        .targets
        .iter()
        .filter(|target| target.response.is_none())
        .count();
    let missing = batch
        .targets
        .len()
        .saturating_sub(parents)
        .saturating_sub(errors);
    let mut effects = vec![
        Effect::Report(batch.outcome()),
        count(
            scope,
            "slack.sync.followed.targets.attempted",
            batch.targets.len(),
            false,
            Outcome::Succeeded,
        ),
        // Requested parents present in returned results, not proof cache merging
        // or review happened; neither declared reply counts nor reactions leak.
        count(
            scope,
            "slack.sync.followed.parents.observed",
            parents,
            false,
            Outcome::Succeeded,
        ),
    ];
    if errors != 0 {
        effects.push(count(
            scope,
            "slack.sync.followed.targets.errors",
            errors,
            false,
            Outcome::Failed,
        ));
    }
    if missing != 0 {
        effects.push(count(
            scope,
            "slack.sync.followed.parents.missing",
            missing,
            false,
            Outcome::Failed,
        ));
    }
    for target in &batch.targets {
        let mut action = target.action.clone();
        let outcome = match target.response {
            None => {
                action.metadata.reason = Some(Reason::Unspecified);
                Outcome::Failed
            }
            Some(response) => {
                // Keep a duplicate candidate's own outcome and actual returned
                // entry count together, without pairing sibling observations.
                action.metadata.count = Some(u32::try_from(response.messages).unwrap_or(u32::MAX));
                action.metadata.truncated =
                    response.evidence.truncated || response.messages > u32::MAX as usize;
                if !response.parent_found {
                    // Unavailable in this returned result, not remotely deleted
                    // or denied. The successful returned list is observed below.
                    action.metadata.reason = Some(Reason::Unavailable);
                    Outcome::Failed
                } else {
                    list_outcome(response.evidence)
                }
            }
        };
        effects.push(Effect::Observe { action, outcome });
        if let Some(response) = target.response {
            effects.push(count(
                &target.action,
                "slack.sync.followed.messages",
                response.messages,
                response.evidence.truncated,
                list_outcome(response.evidence),
            ));
            effects.extend(gaps(
                &target.action,
                response.evidence,
                usize::from(response.evidence.truncated),
                NamesEvidence::default(),
            ));
        }
    }
    effects
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[path = "sync_history/page_thread_tests.rs"]
mod page_thread_tests;

#[cfg(test)]
#[path = "sync_history/read_tests.rs"]
mod read_tests;

#[cfg(test)]
#[path = "sync_history/followed_tests.rs"]
mod followed_tests;

#[cfg(test)]
#[path = "sync_history/status_tests.rs"]
mod status_tests;

#[cfg(test)]
#[path = "sync_history/for_you_tests.rs"]
mod for_you_tests;

#[cfg(test)]
#[path = "sync_history/bootstrap_tests.rs"]
mod bootstrap_tests;
