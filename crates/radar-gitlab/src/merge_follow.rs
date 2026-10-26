//! A merge GitLab accepted but has not done yet (`mergeRequestAccept` answers
//! `opened` and merges in the background), followed until GitLab says how it
//! went: merged, refused with its reason (`mergeError`), or not happening.
//! Each follow-up drives the change-only sync every few seconds, writes the
//! merge request's `merge_watch` (its only writer), and ends with one journal
//! record (`gitlab.merge.confirm`), one notice and one log line.

use radar_core::journal::{Outcome as JournalOutcome, Reason};
use radar_core::plugin::Effect;
use radar_core::{log, writes};

use crate::GitlabPlugin;
use crate::client::Synced;
use crate::model::{MergeWatch, MrState};
use crate::mr_actions::{Act, Acting};
use crate::pages::mr_key;
use crate::write::SERVICE;
use crate::write_context::ConnectionStamp;

/// How often a merge GitLab accepted is asked about again.
pub const CHECK_EVERY: i64 = 3;
/// Accepted, and neither under way nor refused after this long: not
/// happening. Every merge GitLab did from radar finished within 5 s (the
/// journal, 19 to 26 October 2026).
pub const QUIET_AFTER: i64 = 15;
/// With the tenancy's syncs failing this long, radar stops asking.
pub const NO_ANSWER_AFTER: i64 = 120;

/// A merge GitLab accepted, followed until it says how it went.
#[derive(Debug, Clone)]
pub struct Merging {
    /// The tenancy that holds it.
    pub group: String,
    pub since: i64,
    pub watch: MergeWatch,
    pub connection: ConnectionStamp,
}

/// Start following the merge of `key` that GitLab accepted; `None` when no
/// tenancy holds it (the answer is then shown as it came).
pub(crate) fn begin(p: &mut GitlabPlugin, key: &str, connection: ConnectionStamp) -> Option<Vec<Effect>> {
    let group = group_of(p, key)?;
    p.merging.insert(
        key.to_string(),
        Merging {
            group,
            since: p.now,
            watch: MergeWatch::Checking,
            connection,
        },
    );
    p.acting.insert(
        key.to_string(),
        Act {
            what: "merge",
            state: Acting::Running,
            at: p.now,
        },
    );
    mark(p);
    let iid = iid_of(key);
    log::info(format!("gitlab merge !{iid}: accepted, checking"));
    Some(vec![
        Effect::Report(JournalOutcome::Unknown),
        Effect::Notify(SERVICE.taken(&format!("merge !{iid}"))),
    ])
}

/// Stop following `key` (a new merge was asked for).
pub(crate) fn forget(p: &mut GitlabPlugin, key: &str) {
    if p.merging.remove(key).is_some() {
        mark(p);
    }
}

/// Whether radar is following a merge of this merge request.
pub(crate) fn following(p: &GitlabPlugin, project: &str, iid: u64) -> bool {
    p.merging.contains_key(&mr_key(project, iid))
}

/// Whether a followed merge wants the next sync now rather than at the
/// refresh interval.
pub(crate) fn check_due(p: &GitlabPlugin) -> bool {
    p.merging
        .values()
        .any(|m| m.watch == MergeWatch::Checking && p.now - m.since < NO_ANSWER_AFTER)
        && p.sync.last_started.is_none_or(|t| p.now - t >= CHECK_EVERY)
}

/// The merge requests a sync saw leave the open state, by key, taken before
/// the sync's data is folded in (which drops them outside the history window).
pub(crate) fn ended(got: &Synced) -> Vec<(String, MrState)> {
    got.mrs
        .iter()
        .filter(|m| m.state != MrState::Opened)
        .map(|m| (mr_key(&m.project_path, m.iid), m.state))
        .collect()
}

/// After a good sync of `group`: what GitLab now says of each merge followed
/// there.
pub(crate) fn settle(p: &mut GitlabPlugin, group: &str, ended: &[(String, MrState)]) -> Vec<Effect> {
    let keys: Vec<String> = p
        .merging
        .iter()
        .filter(|(_, m)| m.group == group)
        .map(|(k, _)| k.clone())
        .collect();
    let mut fx = Vec::new();
    for key in keys {
        let Some(m) = p.merging.get(&key).cloned() else { continue };
        if !p.is_current_connection(m.connection) {
            p.merging.remove(&key);
            continue;
        }
        let iid = iid_of(&key);
        let what = format!("merge !{iid}");
        let held = p
            .data
            .get(group)
            .and_then(|d| d.mrs.iter().find(|mr| mr_key(&mr.project_path, mr.iid) == key));
        let state = ended
            .iter()
            .find(|(k, _)| *k == key)
            .map(|(_, s)| *s)
            .or(held.map(|mr| mr.state));
        let ongoing = held.is_some_and(|mr| mr.merge_ongoing);
        let error = held.map(|mr| mr.merge_error.clone()).unwrap_or_default();
        match state {
            Some(MrState::Merged) => fx.extend(conclude(p, &key, Ok(format!("merged !{iid}")), JournalOutcome::Succeeded)),
            Some(MrState::Closed) => {
                let words = SERVICE.did_not(&what, "it was closed");
                fx.extend(conclude(p, &key, Err(words), JournalOutcome::Failed));
            }
            None => {
                let words = SERVICE.did_not(&what, "GitLab no longer shows it");
                fx.extend(conclude(p, &key, Err(words), JournalOutcome::Failed));
            }
            Some(MrState::Opened) if !error.is_empty() => {
                let words = SERVICE.did_not(&what, &error);
                fx.extend(conclude(p, &key, Err(words), JournalOutcome::Failed));
            }
            Some(MrState::Opened) if ongoing => {
                if let Some(m) = p.merging.get_mut(&key) {
                    m.watch = MergeWatch::Checking;
                }
            }
            Some(MrState::Opened) => match m.watch {
                MergeWatch::Checking if p.now - m.since >= QUIET_AFTER => {
                    let words = SERVICE.did_not(&what, "it accepted the merge but is not merging it; merge again");
                    if let Some(m) = p.merging.get_mut(&key) {
                        m.watch = MergeWatch::NotMerging;
                    }
                    fx.extend(said(p, &key, Err(words), JournalOutcome::Failed));
                }
                // An answer at last: GitLab's own fields say the rest.
                MergeWatch::Unanswered => {
                    p.merging.remove(&key);
                }
                _ => {}
            },
        }
    }
    mark(p);
    fx
}

/// Each followed merge whose tenancy has failed to sync for
/// [`NO_ANSWER_AFTER`]: radar stops asking and says so. A clock that ran
/// while radar was paused concludes nothing: only a failed sync does.
pub(crate) fn unanswered(p: &mut GitlabPlugin) -> Vec<Effect> {
    let keys: Vec<String> = p
        .merging
        .iter()
        .filter(|(_, m)| {
            m.watch == MergeWatch::Checking
                && p.now - m.since >= NO_ANSWER_AFTER
                && p.sync.errors.contains_key(&m.group)
        })
        .map(|(k, _)| k.clone())
        .collect();
    let mut fx = Vec::new();
    for key in keys {
        if let Some(m) = p.merging.get_mut(&key) {
            m.watch = MergeWatch::Unanswered;
        }
        let words = SERVICE.no_answer(&format!("merge !{}", iid_of(&key)));
        fx.extend(said(p, &key, Err(words), JournalOutcome::Unknown));
    }
    if !fx.is_empty() {
        mark(p);
    }
    fx
}

/// End the follow-up of `key` and say how it went.
fn conclude(p: &mut GitlabPlugin, key: &str, how: Result<String, String>, outcome: JournalOutcome) -> Vec<Effect> {
    p.merging.remove(key);
    said(p, key, how, outcome)
}

/// Record and say how a followed merge went: the page's line, a notice, the
/// journal and the log.
fn said(p: &mut GitlabPlugin, key: &str, how: Result<String, String>, outcome: JournalOutcome) -> Vec<Effect> {
    let (state, notice, words) = match how {
        Ok(words) => (Acting::Done(words.clone()), writes::done(&words), words),
        Err(words) if outcome == JournalOutcome::Unknown => {
            (Acting::Failed(words.clone()), Effect::Notify(words.clone()), words)
        }
        Err(words) => (Acting::Failed(words.clone()), writes::failed(&words), words),
    };
    p.acting.insert(key.to_string(), Act { what: "merge", state, at: p.now });
    log::info(format!("gitlab {words}"));
    vec![observed(p, key, outcome), notice]
}

/// The journal record of how a followed merge went.
fn observed(p: &GitlabPlugin, key: &str, outcome: JournalOutcome) -> Effect {
    let (project, iid) = key.rsplit_once('!').unwrap_or((key, ""));
    let mut action = crate::mr_actions::descriptor(p, "gitlab.merge.confirm", project, iid.parse().unwrap_or(0));
    action.metadata.reason = match outcome {
        JournalOutcome::Failed => Some(Reason::Unspecified),
        JournalOutcome::Unknown => Some(Reason::NoTerminalObservation),
        _ => None,
    };
    Effect::Observe { action, outcome }
}

/// Each held merge request's `merge_watch`, from what is followed.
fn mark(p: &mut GitlabPlugin) {
    let merging = &p.merging;
    for mr in p.data.values_mut().flat_map(|d| d.mrs.iter_mut()) {
        mr.merge_watch = merging
            .get(&mr_key(&mr.project_path, mr.iid))
            .map_or(MergeWatch::None, |m| m.watch);
    }
}

/// The tenancy that holds `key`.
fn group_of(p: &GitlabPlugin, key: &str) -> Option<String> {
    p.data
        .iter()
        .find(|(_, d)| d.mrs.iter().any(|mr| mr_key(&mr.project_path, mr.iid) == key))
        .map(|(group, _)| group.clone())
}

fn iid_of(key: &str) -> &str {
    key.rsplit_once('!').map_or("", |(_, iid)| iid)
}

#[cfg(test)]
mod tests;
