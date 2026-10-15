//! Reading Slack: who you are, then each watched channel (its messages
//! first, then the threads with replies radar has not read), what is for you,
//! and the review asks being followed. Each is a task of its own, so the
//! screen fills as they come back rather than after the slowest.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use radar_core::journal::{ActionDescriptor, Outcome};
use radar_core::log;
use radar_core::plugin::Effect;

use crate::client::{BootstrapRead, Client, Found, Message, Stop, UserGroup, all};
use crate::model::{merge_messages, newer, ts_key, ts_secs};
use crate::sync_history::{
    self, ChannelRead, FollowedBatch, FollowedResponse, FollowedTarget, ForYouEvidence,
    GroupsEvidence, NamesEvidence, SearchKind, SearchList, SearchObservation, ThreadsRead,
};
use crate::{SlackPlugin, SyncErrorKey, mrkdwn, view};

/// How far back For you looks, at most.
pub(crate) const FOR_YOU_DAYS: u64 = 7;
/// The user groups (`usergroups.list` with users: over half a megabyte in a
/// large workspace) are read this often: a day. They are held in the cache
/// with the time they were read, so a start within a day asks nothing.
pub(crate) const GROUPS_EVERY: i64 = 86_400;
/// A read of them that failed is tried again after this long, not a day.
pub(crate) const GROUPS_RETRY: i64 = 3_600;
/// Messages found for you that are kept, at most.
const FOR_YOU_KEPT: usize = 80;
/// Threads whose replies are read per channel and read, at most.
const REPLIES_PER_READ: usize = 20;
/// Names looked up per task, at most.
const NAMES_PER_TASK: usize = 40;
/// Review asks are followed this long.
const FOLLOW_DAYS: i64 = 14;
/// A channel's whole window is read again every this many refreshes (and
/// on `r`): Slack says nothing of new replies, reactions or edits on older
/// messages to a read of only what is new.
const SWEEP_REFRESHES: i64 = 5;

/// What one task brought back.
#[derive(Debug)]
pub(crate) enum Read {
    Me {
        action: ActionDescriptor,
        result: Result<BootstrapRead, String>,
    },
    /// A channel's info (when it was asked for) and its messages.
    Channel {
        id: String,
        result: Result<ChannelRead, String>,
        names: Vec<(String, String)>,
        names_evidence: NamesEvidence,
        action: ActionDescriptor,
    },
    /// Replies of a channel's threads, by the thread's ts.
    Threads {
        id: String,
        result: Result<ThreadsRead, String>,
        names: Vec<(String, String)>,
        names_evidence: NamesEvidence,
        action: ActionDescriptor,
    },
    ForYou {
        groups: Option<Vec<UserGroup>>,
        result: Result<Vec<Found>, String>,
        names: Vec<(String, String)>,
        action: ActionDescriptor,
        evidence: ForYouEvidence,
    },
    /// Original cache updates plus end-of-batch per-target observations.
    Followed {
        action: ActionDescriptor,
        batch: FollowedBatch,
    },
}

impl Read {
    fn history(&self) -> Vec<Effect> {
        match self {
            Self::Me { action, result } => sync_history::bootstrap(action, result),
            Self::Channel {
                action,
                result,
                names_evidence,
                ..
            } => sync_history::channel(action, result, *names_evidence),
            Self::Threads {
                action,
                result,
                names_evidence,
                ..
            } => sync_history::threads(action, result, *names_evidence),
            Self::ForYou {
                action,
                result,
                evidence,
                ..
            } => sync_history::for_you(action, evidence, result.is_ok()),
            Self::Followed { action, batch } => sync_history::followed(action, batch),
        }
    }

    pub(crate) fn stale_effects(&self) -> Vec<Effect> {
        let scope = match self {
            Self::Me { action, result } => sync_history::bootstrap_scope(action, result),
            Self::Channel { action, .. }
            | Self::Threads { action, .. }
            | Self::ForYou { action, .. }
            | Self::Followed { action, .. } => action.clone(),
        };
        let mut effects = self.history();
        effects.push(crate::read_context::skipped(Some(&scope)));
        effects
    }
}

#[cfg(test)]
#[path = "read_incremental_tests.rs"]
mod incremental_tests;

#[cfg(test)]
#[path = "read_concurrent_tests.rs"]
mod concurrent_tests;

/// Names for the user ids among `ids` radar does not know yet.
async fn names_observed(
    c: &Client,
    ids: BTreeSet<String>,
    known: &BTreeSet<String>,
) -> (Vec<(String, String)>, NamesEvidence) {
    let eligible: Vec<String> = ids
        .into_iter()
        .filter(|u| !known.contains(u) && u.starts_with(['U', 'W']))
        .collect();
    let mut evidence = NamesEvidence::default();
    let mut out = Vec::new();
    // A few at a time (users.info is Tier 4), each person once across the
    // tasks reading together; the first failure leaves out those not yet sent.
    let stop = Stop::default();
    let lookups = eligible.iter().take(NAMES_PER_TASK).map(|u| async {
        let got = c.user_name(u, &stop).await;
        if matches!(got, Some(Err(_))) {
            stop.set();
        }
        got
    });
    for (u, got) in eligible.iter().zip(all(lookups).await) {
        match got {
            Some(Ok(name)) => {
                evidence.attempted += 1;
                out.push((u.clone(), name));
            }
            Some(Err(e)) => {
                evidence.attempted += 1;
                evidence.errors += 1;
                log::warn(format!("slack: user {u}: {e}"));
            }
            None => {}
        }
    }
    evidence.deferred = eligible.len().saturating_sub(evidence.attempted);
    (out, evidence)
}

/// The people a message names: who wrote it, who replied, who it mentions.
fn people(m: &Message, into: &mut BTreeSet<String>) {
    into.extend(m.user.iter().cloned());
    into.extend(m.reply_users.iter().cloned());
    into.extend(mrkdwn::mentions(&m.text).users);
}

impl SlackPlugin {
    /// Start a read: who you are first, when that is not known yet.
    pub(crate) fn read(&mut self) -> Vec<Effect> {
        // An explicit connection check owns authentication until it completes.
        // Do not let the timer bootstrap supersede its check-all continuation.
        if matches!(self.checks.get("workspace"), Some((crate::Check::Running, _))) {
            return vec![];
        }
        let Ok(c) = self.client() else {
            self.revoke_session();
            return vec![];
        };
        self.sync.started = Some(self.now);
        // Persisted Me is never a login. Every newly configured session
        // authenticates before account-scoped work, even with a warm cache.
        if !self.authenticated() || self.data.me.as_ref().is_none_or(|m| m.tz_offset.is_none()) {
            self.sync.reading += 1;
            let stamp = self.read_stamp();
            return vec![sync_history::spawn(
                stamp,
                sync_history::bootstrap_action(),
                move |action| async move {
                    Read::Me {
                        action,
                        result: c.whoami_observed().await.map_err(|e| format!("{e:#}")),
                    }
                },
            )];
        }
        self.read_all(c)
    }

    /// Every watched channel, For you and the asks followed, at once.
    fn read_all(&mut self, c: Client) -> Vec<Effect> {
        if !self.authenticated() { return vec![]; }
        // Everyone not known is asked for again this round, once.
        self.pace.new_round();
        let stamp = self.read_stamp();
        let Some(me) = self.data.me.clone() else {
            return vec![];
        };
        let known: Arc<BTreeSet<String>> = Arc::new(self.data.users.keys().cloned().collect());
        let window = self.now - self.cfg.days as i64 * 86_400;
        let refresh = self.cfg.refresh_secs as i64;
        let full_now = std::mem::take(&mut self.sync.full_now);
        let resumed = std::mem::take(&mut self.sync.resumed);
        // What is held is the reference: when radar starts again, what was read
        // less than a refresh ago is not asked again.
        let fresh = |at: i64| resumed && !full_now && at != 0 && self.now - at < refresh;
        let mut held_since: Option<i64> = None;
        let mut fx = Vec::new();
        for ch in self.cfg.watched() {
            let held = self.data.channels.get(&ch.id);
            if let Some(d) = held.filter(|d| d.info.is_some() && fresh(d.read_at)) {
                held_since = Some(held_since.map_or(d.read_at, |t| t.min(d.read_at)));
                continue;
            }
            // Between whole-window reads (every few refreshes, and on `r`),
            // only what is newer than the newest message held.
            let since = held
                .filter(|d| {
                    !full_now && d.swept_at != 0 && self.now - d.swept_at < SWEEP_REFRESHES * refresh
                })
                .and_then(|d| d.newest())
                .filter(|ts| ts_secs(ts) >= window)
                .map(str::to_string);
            let oldest = since.clone().unwrap_or_else(|| format!("{window}.000000"));
            self.sync
                .channel_reads
                .insert(ch.id.clone(), (since, self.now));
            let (c, known, id) = (c.clone(), known.clone(), ch.id.clone());
            let ask_info = self
                .data
                .channels
                .get(&ch.id)
                .is_none_or(|d| d.info.is_none());
            let action = sync_history::action("slack.sync.channel", &me.team_id, &id);
            fx.push(sync_history::spawn(
                stamp,
                action,
                move |action| async move {
                    let result = async {
                        let info = if ask_info {
                            Some(c.channel_info(&id).await?)
                        } else {
                            None
                        };
                        let mut list = c.history_observed(&id, Some(&oldest), 300).await?;
                        list.messages.retain(|m| {
                            !matches!(
                                m.subtype.as_deref(),
                                Some(
                                    "channel_join" | "channel_leave" | "group_join" | "group_leave"
                                )
                            )
                        });
                        list.messages.sort_by_key(|m| ts_key(&m.ts));
                        anyhow::Ok(ChannelRead { info, list })
                    }
                    .await
                    .map_err(|e| format!("{e:#}"));
                    let mut ids = BTreeSet::new();
                    if let Ok(got) = &result {
                        got.list.messages.iter().for_each(|m| people(m, &mut ids));
                    }
                    let (names, names_evidence) = names_observed(&c, ids, &known).await;
                    Read::Channel {
                        id,
                        result,
                        names,
                        names_evidence,
                        action,
                    }
                },
            ));
        }
        // For you: mentions of you and your groups, and messages to you.
        let groups_at = self.sync.groups_at.max(self.data.groups_at);
        let groups_due = self.now - groups_at >= GROUPS_EVERY || self.data.groups.is_empty();
        if groups_due {
            self.sync.groups_at = self.now;
            self.data.groups_at = self.now;
        }
        let mine: Vec<String> = self.data.my_groups().iter().map(|g| g.id.clone()).collect();
        let after = view::date(self.now - FOR_YOU_DAYS as i64 * 86_400);
        if fresh(self.data.for_you_at) && !groups_due {
            let at = self.data.for_you_at;
            held_since = Some(held_since.map_or(at, |t| t.min(at)));
        } else {
            self.data.for_you_at = self.now;
            // Between whole reads (every few refreshes, on `r`, and when the
            // groups changed what is asked), ask only whether anything is
            // newer than what is held: a search answers in tens of KB.
            let incremental = !full_now
                && !groups_due
                && !self.data.for_you.is_empty()
                && self.data.for_you_swept_at != 0
                && self.now - self.data.for_you_swept_at < SWEEP_REFRESHES * refresh;
            self.sync.for_you_read = Some((incremental, self.now));
            let held: Arc<BTreeSet<(String, String)>> = Arc::new(if incremental {
                self.data
                    .for_you
                    .iter()
                    .map(|f| (f.channel_id.clone(), f.message.ts.clone()))
                    .collect()
            } else {
                BTreeSet::new()
            });
            let (c, known) = (c.clone(), known.clone());
            let action = sync_history::for_you_action(&me.team_id);
            fx.push(sync_history::spawn(
                stamp,
                action,
                move |action| async move {
                    let mut groups = None;
                    let mut mine = mine;
                    let mut evidence = ForYouEvidence {
                        groups: GroupsEvidence::Cached {
                            selected: mine.len(),
                        },
                        ..Default::default()
                    };
                    if groups_due {
                        match c.usergroups_observed().await {
                            Ok(got) => {
                                let g = got.groups;
                                mine = g
                                    .iter()
                                    .filter(|g| g.users.contains(&me.user_id))
                                    .map(|g| g.id.clone())
                                    .collect();
                                evidence.groups = GroupsEvidence::Returned {
                                    returned: g.len(),
                                    selected: mine.len(),
                                    incomplete_collection: got.incomplete_collection,
                                    incomplete_entries: got.incomplete_entries,
                                };
                                groups = Some(g);
                            }
                            Err(e) => {
                                log::warn(format!("slack: user groups: {e:#}"));
                                evidence.groups = GroupsEvidence::Failed {
                                    selected: mine.len(),
                                };
                            }
                        }
                    }
                    let mut queries = vec![
                        (
                            SearchKind::Mentions,
                            format!("<@{}> after:{after}", me.user_id),
                        ),
                        (SearchKind::Direct, format!("to:me after:{after}")),
                    ];
                    queries.extend(
                        mine.iter()
                            .map(|g| (SearchKind::Group, format!("<!subteam^{g}> after:{after}"))),
                    );
                    evidence.planned = queries.len();
                    let mut found: Vec<Found> = Vec::new();
                    let mut failed = None;
                    for (kind, q) in queries {
                        let read = if incremental {
                            let is_held = |f: &Found| {
                                held.contains(&(f.channel_id.clone(), f.message.ts.clone()))
                            };
                            c.search_since(&q, 40, &is_held).await
                        } else {
                            c.search_observed(&q, 40).await
                        };
                        match read {
                            Ok(got) => {
                                evidence.queries.push(SearchObservation {
                                    kind,
                                    list: Some(SearchList {
                                        returned: got.matches.len(),
                                        evidence: got.evidence,
                                    }),
                                });
                                for x in got.matches {
                                    if !found.iter().any(|y| {
                                        y.channel_id == x.channel_id && y.message.ts == x.message.ts
                                    }) {
                                        found.push(x);
                                    } else {
                                        evidence.duplicates += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn(format!("slack search failed: {e:#}"));
                                evidence
                                    .queries
                                    .push(SearchObservation { kind, list: None });
                                failed = Some(format!("{e:#}"));
                                break;
                            }
                        }
                    }
                    found.sort_by_key(|f| std::cmp::Reverse(ts_key(&f.message.ts)));
                    evidence.unique = found.len();
                    found.truncate(FOR_YOU_KEPT);
                    evidence.retained = found
                        .iter()
                        .map(|f| sync_history::for_you_message(&me.team_id, f))
                        .collect();
                    let mut ids = BTreeSet::new();
                    found.iter().for_each(|f| people(&f.message, &mut ids));
                    let (names, names_evidence) = names_observed(&c, ids, &known).await;
                    evidence.names = names_evidence;
                    let result = match failed {
                        Some(e) => Err(e),
                        None => Ok(found),
                    };
                    Read::ForYou {
                        groups,
                        result,
                        names,
                        action,
                        evidence,
                    }
                },
            ));
        }
        // Review asks followed.
        let asked: Vec<(String, String)> = self
            .data
            .asked
            .iter()
            .filter(|a| self.now - ts_secs(&a.ts) < FOLLOW_DAYS * 86_400)
            .map(|a| (a.channel.clone(), a.ts.clone()))
            .collect();
        if asked.is_empty() {
            // No eligible work remains (including asks that aged out). An old
            // followed-read failure must not hold later complete reads back.
            self.sync.errors.remove(&SyncErrorKey::Followed);
        } else {
            let c = c.clone();
            let team = self
                .data
                .me
                .as_ref()
                .map(|me| me.team_id.as_str())
                .unwrap_or_default();
            let action = sync_history::followed_action(team);
            let targets: Vec<_> = asked
                .into_iter()
                .map(|(channel, ts)| {
                    let scope = sync_history::followed_target(team, &channel, &ts);
                    (channel, ts, scope)
                })
                .collect();
            fx.push(sync_history::spawn(
                stamp,
                action,
                move |action| async move {
                    let mut batch = FollowedBatch::default();
                    // A few at a time (conversations.replies is Tier 3).
                    let reads = all(targets.iter().map(|(channel, ts, _)| async {
                        let _turn = c.replies_turn().await;
                        c.replies_observed(channel, ts, 200).await
                    }))
                    .await;
                    for ((channel, ts, scope), read) in targets.into_iter().zip(reads) {
                        let response = match read {
                            Ok(got) => {
                                let parent = got.messages.iter().find(|x| x.ts == ts);
                                if let Some(parent) = parent {
                                    let n = parent
                                        .reply_count
                                        .max(got.messages.len().saturating_sub(1) as u32);
                                    batch.updates.push((
                                        channel,
                                        ts.clone(),
                                        n,
                                        parent.reactions.clone(),
                                    ));
                                }
                                Some(FollowedResponse {
                                    parent_found: parent.is_some(),
                                    messages: got.messages.len(),
                                    evidence: got.evidence,
                                })
                            }
                            Err(e) => {
                                log::warn(format!("slack: following {channel} {ts}: {e:#}"));
                                None
                            }
                        };
                        batch.targets.push(FollowedTarget {
                            action: scope,
                            response,
                        });
                    }
                    Read::Followed { action, batch }
                },
            ));
        }
        if fx.is_empty()
            && let Some(at) = held_since
        {
            // Nothing to ask: what is held is as fresh as its oldest read.
            log::info("slack: everything held was read less than a refresh ago");
            self.sync.last_ok = Some(at);
            self.sync.held_at = Some(at);
        }
        self.sync.reading += fx.len();
        fx
    }

    /// The threads of a channel whose newest reply radar has not read, newest
    /// first, as one task.
    fn read_threads(&mut self, id: &str) -> Vec<Effect> {
        let stamp = self.read_stamp();
        let Some(held) = self.data.channels.get(id) else {
            return vec![];
        };
        let mut todo: Vec<&Message> = held
            .messages
            .iter()
            .filter(|m| m.reply_count > 0)
            .filter(|m| {
                match (
                    m.latest_reply.as_deref(),
                    held.replies.get(&m.ts).and_then(|r| r.last()),
                ) {
                    (Some(l), Some(got)) => newer(l, &got.ts),
                    _ => true,
                }
            })
            .collect();
        if todo.is_empty() {
            return vec![];
        }
        todo.sort_by_key(|m| std::cmp::Reverse(ts_key(m.latest_reply.as_deref().unwrap_or(&m.ts))));
        // Each from its newest reply held, when it has one.
        let threads: Vec<(String, Option<String>)> = todo
            .iter()
            .take(REPLIES_PER_READ)
            .map(|m| {
                let since = held
                    .replies
                    .get(&m.ts)
                    .and_then(|r| r.iter().map(|r| r.ts.as_str()).max_by_key(|ts| ts_key(ts)))
                    .map(str::to_string);
                (m.ts.clone(), since)
            })
            .collect();
        let deferred = todo.len().saturating_sub(threads.len());
        let Ok(c) = self.client() else {
            return vec![];
        };
        let known: BTreeSet<String> = self.data.users.keys().cloned().collect();
        let id = id.to_string();
        self.sync.reading += 1;
        let team = self
            .data
            .me
            .as_ref()
            .map(|me| me.team_id.as_str())
            .unwrap_or_default();
        let action = sync_history::action("slack.sync.threads", team, &id);
        vec![sync_history::spawn(
            stamp,
            action,
            move |action| async move {
                let result = async {
                    let mut out = ThreadsRead {
                        deferred,
                        ..Default::default()
                    };
                    // A few at a time (conversations.replies is Tier 3); the
                    // first failure leaves out those not yet sent.
                    let stop = Stop::default();
                    let reads = threads.iter().map(|(ts, since)| async {
                        let _turn = c.replies_turn().await;
                        if stop.is_set() {
                            return None;
                        }
                        let got = c.replies_after(&id, ts, since.as_deref(), 200).await;
                        if got.is_err() {
                            stop.set();
                        }
                        Some(got)
                    });
                    for ((ts, since), got) in threads.iter().zip(all(reads).await) {
                        let Some(got) = got else { continue };
                        let ts = ts.clone();
                        let got = got?;
                        out.cutoffs += usize::from(got.evidence.truncated);
                        out.evidence.merge(got.evidence);
                        if since.is_some() && !got.evidence.truncated {
                            out.merged.insert(ts.clone());
                        }
                        let replies = got.messages.into_iter().filter(|r| r.ts != ts).collect();
                        out.threads.insert(ts, replies);
                    }
                    anyhow::Ok(out)
                }
                .await
                .map_err(|e| format!("{e:#}"));
                let mut ids = BTreeSet::new();
                if let Ok(t) = &result {
                    t.threads
                        .values()
                        .flatten()
                        .for_each(|m| people(m, &mut ids));
                }
                let (names, names_evidence) = names_observed(&c, ids, &known).await;
                Read::Threads {
                    id,
                    result,
                    names,
                    names_evidence,
                    action,
                }
            },
        )]
    }

    /// Every thread held, by its motion key, with its first message: its
    /// text, reactions, reply count and latest reply say what a read
    /// changed. A message found for you stands for its thread when the
    /// thread is not held.
    pub(crate) fn threads(&self) -> BTreeMap<String, Message> {
        let mut out = BTreeMap::new();
        for (id, c) in &self.data.channels {
            for m in &c.messages {
                out.insert(crate::join::motion_key(id, &m.ts), m.clone());
            }
        }
        for f in &self.data.for_you {
            out.entry(crate::join::motion_key(
                &f.channel_id,
                crate::screen::found_thread(f),
            ))
            .or_insert_with(|| f.message.clone());
        }
        out
    }

    /// Fold one task's answer into what is held.
    pub(crate) fn on_read(&mut self, r: Read) -> Vec<Effect> {
        self.sync.reading = self.sync.reading.saturating_sub(1);
        let mut fx = r.history();
        match r {
            Read::Me {
                result: Ok(got), ..
            } => {
                self.sync.errors.remove(&SyncErrorKey::Bootstrap);
                let stamp = self.read_stamp();
                if !self.install_me(got.me) {
                    self.sync.errors.insert(SyncErrorKey::Bootstrap, "Slack returned an incomplete account identity".into());
                    return fx;
                }
                if let Ok(c) = self.client() {
                    if !self.is_current_read(stamp) {
                        self.sync.started = Some(self.now);
                    }
                    fx.extend(self.read_all(c));
                }
            }
            Read::Me { result: Err(e), .. } => {
                self.revoke_session();
                self.sync.started = Some(self.now);
                log::error(format!("slack: who am I failed: {e}"));
                self.sync.errors.insert(SyncErrorKey::Bootstrap, e);
            }
            Read::Channel {
                id, result, names, ..
            } => {
                self.data.users.extend(names);
                let alias = self.alias(&id);
                match result {
                    Ok(ChannelRead { info, list }) => {
                        let (since, asked) = self
                            .sync
                            .channel_reads
                            .remove(&id)
                            .unwrap_or((None, self.now));
                        let messages = list.messages;
                        let window = self.now - self.cfg.days as i64 * 86_400;
                        log::info(format!(
                            "slack #{alias}: {} messages{}",
                            messages.len(),
                            if since.is_some() { " new" } else { "" }
                        ));
                        self.sync.errors.remove(&SyncErrorKey::Channel(alias));
                        let c = self.data.channels.entry(id.clone()).or_default();
                        if info.is_some() {
                            c.info = info;
                        }
                        match since {
                            // Everything newer than what is held: folded in,
                            // and what has left the window let go.
                            Some(_) if !list.evidence.truncated => {
                                merge_messages(&mut c.messages, messages);
                                c.messages.retain(|m| ts_secs(&m.ts) >= window);
                            }
                            _ => {
                                c.messages = messages;
                                c.swept_at = asked;
                            }
                        }
                        c.read_at = asked;
                        let keep: BTreeSet<&str> =
                            c.messages.iter().map(|m| m.ts.as_str()).collect();
                        c.replies.retain(|ts, _| keep.contains(ts.as_str()));
                        fx.extend(self.read_threads(&id));
                        // What an alert reader could not read is said in the
                        // log, once.
                        self.changed();
                        self.log_refusals();
                    }
                    Err(e) => {
                        log::error(format!("slack #{alias}: {e}"));
                        self.sync.errors.insert(SyncErrorKey::Channel(alias), e);
                    }
                }
            }
            Read::Threads {
                id, result, names, ..
            } => {
                self.data.users.extend(names);
                let alias = self.alias(&id);
                match result {
                    Ok(ThreadsRead {
                        threads, merged, ..
                    }) => {
                        log::info(format!("slack #{alias}: {} threads read", threads.len()));
                        let c = self.data.channels.entry(id).or_default();
                        for (ts, replies) in threads {
                            if merged.contains(&ts) {
                                merge_messages(c.replies.entry(ts).or_default(), replies);
                            } else {
                                c.replies.insert(ts, replies);
                            }
                        }
                    }
                    Err(e) => {
                        log::error(format!("slack #{alias} threads: {e}"));
                        self.sync.errors.insert(SyncErrorKey::Channel(alias), e);
                    }
                }
            }
            Read::ForYou {
                groups,
                result,
                names,
                evidence,
                ..
            } => {
                self.data.users.extend(names);
                if matches!(evidence.groups, GroupsEvidence::Failed { .. }) {
                    // Not a day: due again in GROUPS_RETRY.
                    let at = self.now - (GROUPS_EVERY - GROUPS_RETRY);
                    self.sync.groups_at = at;
                    self.data.groups_at = at;
                }
                if let Some(g) = groups {
                    self.data.groups = g;
                }
                match result {
                    Ok(f) => {
                        self.sync.errors.remove(&SyncErrorKey::ForYou);
                        let (incremental, asked) =
                            self.sync.for_you_read.take().unwrap_or((false, self.now));
                        if incremental {
                            // What is newer, ahead of what is held; what has
                            // left the window let go.
                            let window = self.now - FOR_YOU_DAYS as i64 * 86_400;
                            let mut all = f;
                            for old in std::mem::take(&mut self.data.for_you) {
                                let same = |x: &Found| {
                                    x.channel_id == old.channel_id && x.message.ts == old.message.ts
                                };
                                if !all.iter().any(same) {
                                    all.push(old);
                                }
                            }
                            all.retain(|x| ts_secs(&x.message.ts) >= window);
                            all.sort_by_key(|x| std::cmp::Reverse(ts_key(&x.message.ts)));
                            all.truncate(FOR_YOU_KEPT);
                            self.data.for_you = all;
                        } else {
                            self.data.for_you = f;
                            self.data.for_you_swept_at = asked;
                        }
                    }
                    Err(e) => {
                        self.sync.for_you_read = None;
                        self.data.for_you_at = 0;
                        self.sync.errors.insert(SyncErrorKey::ForYou, e);
                    }
                }
            }
            Read::Followed { batch, .. } => {
                match batch.outcome() {
                    Outcome::Succeeded => {
                        self.sync.errors.remove(&SyncErrorKey::Followed);
                    }
                    outcome => {
                        let error = if outcome == Outcome::Partial {
                            "some followed review threads could not be fully refreshed"
                        } else {
                            "followed review threads could not be refreshed"
                        };
                        self.sync
                            .errors
                            .insert(SyncErrorKey::Followed, error.into());
                    }
                }
                for (channel, ts, replies, reactions) in batch.updates {
                    if let Some(a) = self
                        .data
                        .asked
                        .iter_mut()
                        .find(|a| a.channel == channel && a.ts == ts)
                    {
                        a.replies = replies;
                        a.reactions = reactions;
                    }
                }
            }
        }
        if self.sync.reading == 0 {
            // What was read in threads a month old no longer matters.
            self.data.forget_read_before(self.now - 30 * 86_400);
            if self.sync.errors.is_empty() {
                self.sync.last_ok = Some(self.sync.held_at.take().unwrap_or(self.now));
            }
            if let Some(started) = self.sync.started {
                log::info(format!("slack read in {}s", self.now - started));
            }
            fx.push(Effect::PersistCache);
        }
        fx
    }
}
