//! Which tenancies to sync and what to ask GitLab for, and how what comes
//! back is folded into what radar holds.
//!
//! radar draws from its cache at once and, when the cache was synced under a
//! day ago ([`RESTART_FRESH`]), takes it as the reference: it goes on with
//! changes only, as if it had not stopped. Otherwise the first sync of a
//! tenancy in a session is full: every open merge
//! request, the merged and closed ones of the last `history_days` if a view
//! shows them, and the projects. After that a sync asks only for merge
//! requests updated since the newest one held, in any state, so one that
//! was merged or closed is seen leaving; open ones with a running pipeline
//! are asked for again, each of its own project and for only its pipeline
//! and merge status (a pipeline changes without touching its merge
//! request). Every few minutes each project's last activity is asked (a
//! cheap query), and only the projects that are new, had a merge request
//! merged or run a pipeline on main are fetched in full, with the ones that
//! moved or were active within the hour (a push in that hour may not show
//! in `lastActivityAt`) that have nothing to compare with. The cache keeps
//! each project's reference: the full head commit id and main's newest
//! pipeline (number, state, time). A project that moved or was recently
//! active is asked only for those (branch, head commit, pipeline on it,
//! batched, a seventh of the bytes) and fetched in full only if they differ,
//! so a push to a feature branch, or a quiet hour, costs no full fetch. `r`
//! makes the next sync full again (it also catches a merge request that was
//! deleted).

use radar_core::time;

use crate::GitlabPlugin;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::{GroupData, ProjectsCheck, ProjectsSeen, Synced, Want};
use crate::model::{Mr, MrState, Pipeline, Project};

/// Projects (main's pipeline and newest commit) are fetched again at most
/// this often on a sync that is not full.
pub const PROJECTS_EVERY: i64 = 300;

/// A cached sync state younger than this is the reference on start: the
/// first sync asks only for what changed since, as later ones do. The
/// changes are read up to 4 pages (200), enough for a day of a busy group;
/// an older cache, or one from before this was saved, starts with a full
/// sync.
pub const RESTART_FRESH: i64 = 24 * 3600;

/// What the cache keeps of the sync state, beside the data it drew.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Saved {
    /// When the data saved with it was last synced good (the plugin's
    /// clock; 0 for never), which is how old the reference is.
    pub saved_at: i64,
    #[serde(default)]
    pub full_done: BTreeSet<String>,
    #[serde(default)]
    pub synced_at: BTreeMap<String, i64>,
}

/// The sync state for the cache: this session's, or the one restored and
/// not yet taken up.
pub(crate) fn saved(p: &GitlabPlugin) -> Saved {
    p.sync.restored.clone().unwrap_or_else(|| Saved {
        saved_at: p.sync.last_ok.unwrap_or(0),
        full_done: p.sync.full_done.clone(),
        synced_at: p.sync.synced_at.clone(),
    })
}

/// The sync state from a cache whose data was restored; an old cache
/// without one, or one that does not read, leaves nothing (a full sync).
pub(crate) fn restore(p: &mut GitlabPlugin, cache: &Value) {
    p.sync.restored = serde_json::from_value(cache["sync"].clone()).ok();
}

/// Before the first sync: a restored state young enough becomes this
/// session's, so that sync asks only for changes; an old one is dropped.
pub(crate) fn adopt(p: &mut GitlabPlugin) {
    let Some(s) = p.sync.restored.take() else {
        return;
    };
    if !(0..RESTART_FRESH).contains(&(p.now - s.saved_at)) {
        return;
    }
    for path in s.full_done {
        if p.data.contains_key(&path) {
            p.sync.full_done.insert(path);
        }
    }
    for (path, at) in s.synced_at {
        if p.data.contains_key(&path) {
            p.sync.synced_at.entry(path).or_insert(at);
        }
    }
}

/// GitLab moves a project's `lastActivityAt` at most this often
/// (`Event::RESET_PROJECT_ACTIVITY_INTERVAL`): a project active more
/// recently may have changed without it moving.
pub const ACTIVITY_LAG: i64 = 3600;

/// To be looked at whatever its `lastActivityAt` says: main's pipeline is
/// running or pending (its end does not move it), or the project was active
/// within [`ACTIVITY_LAG`] (or at a time that does not read): a push since
/// may not have moved it.
fn looked_at(p: &Project, now: i64) -> bool {
    matches!(p.main_pipeline, Some(Pipeline::Running | Pipeline::Pending))
        || time::parse_rfc3339(&p.last_activity).is_none_or(|t| now - t < ACTIVITY_LAG)
}

/// What a sync that is not full knows of a tenancy's projects, so it
/// fetches in full only the ones that need it; `None` when none are held
/// (then they are all fetched).
pub(crate) fn projects_check(d: &GroupData, now: i64) -> Option<ProjectsCheck> {
    if d.projects.is_empty() {
        return None;
    }
    Some(ProjectsCheck {
        seen: d
            .projects
            .iter()
            .map(|p| (p.full_path.clone(), p.last_activity.clone()))
            .collect(),
        again: d
            .projects
            .iter()
            .filter(|p| looked_at(p, now) && p.pin().is_none())
            .map(|p| p.full_path.clone())
            .collect(),
        pins: d
            .projects
            .iter()
            .filter_map(|p| Some((p.full_path.clone(), p.pin()?)))
            .collect(),
        look: d
            .projects
            .iter()
            .filter(|p| looked_at(p, now) && p.pin().is_some())
            .map(|p| p.full_path.clone())
            .collect(),
    })
}

/// The projects now: the ones fetched in full, and the held ones whose
/// activity did not move; ones gone (or archived) leave.
fn projects_now(held: &[Project], seen: ProjectsSeen) -> Vec<Project> {
    let mut fresh: BTreeMap<String, Project> = seen
        .fresh
        .into_iter()
        .map(|p| (p.full_path.clone(), p))
        .collect();
    let mut out: Vec<Project> = seen
        .present
        .into_iter()
        .filter_map(|(path, at)| {
            fresh.remove(&path).or_else(|| {
                held.iter().find(|p| p.full_path == path).map(|p| Project {
                    last_activity: at,
                    ..p.clone()
                })
            })
        })
        .collect();
    out.sort_by(|a, b| a.full_path.cmp(&b.full_path));
    out
}

/// A rechecked merge request's pipeline and merge status onto the held one
/// (the fields [`crate::client::RECHECK_FIELDS`] reads).
fn apply_recheck(held: &mut Mr, got: Mr) {
    held.pipeline = got.pipeline;
    held.pipeline_url = got.pipeline_url;
    held.stages = got.stages;
    held.duration_secs = got.duration_secs;
    held.merge_status = got.merge_status;
    held.conflicts = got.conflicts;
    held.rebase = got.rebase;
    held.checks = got.checks;
    held.merge_ongoing = got.merge_ongoing;
    held.merge_error = got.merge_error;
}

/// The newest change held for a tenancy: what the next sync asks from.
fn newest(d: &GroupData) -> Option<&str> {
    d.mrs.iter().map(|m| m.updated_at.as_str()).max()
}

/// How recently anything happened in a tenancy, for the order of syncing.
fn activity(d: Option<&GroupData>) -> i64 {
    let Some(d) = d else { return i64::MAX };
    d.mrs
        .iter()
        .map(|m| m.updated_at.as_str())
        .chain(d.projects.iter().map(|p| p.last_activity.as_str()))
        .filter_map(time::parse_rfc3339)
        .max()
        .unwrap_or(0)
}

/// What to sync now: each tenancy's path and what to ask for, the most
/// recently active first (and ones with nothing held before them).
pub fn plan(p: &GitlabPlugin, full: bool) -> Vec<(String, Want)> {
    let ended_since = p
        .wants_ended()
        .then(|| time::to_rfc3339(p.now - p.cfg.history_days as i64 * 86_400));
    let projects_every = PROJECTS_EVERY.max(p.cfg.refresh_secs as i64);
    let mut out: Vec<(i64, String, Want)> = p
        .cfg
        .sources
        .iter()
        .map(|s| {
            let path = p.cfg.group_path(s);
            let held = p.data.get(&path);
            let since = held.and_then(|d| match newest(d) {
                Some(t) => Some(t.to_string()),
                // Nothing open: ask from when the last sync started.
                None => p
                    .sync
                    .synced_at
                    .get(&path)
                    .map(|t| time::to_rfc3339(t - 60)),
            });
            let first = !p.sync.full_done.contains(&path);
            let missing_ended =
                ended_since.is_some() && held.is_none_or(|d| d.ended_since.is_none());
            let want = match since {
                Some(since) if !full && !first && !missing_ended => Want {
                    changed_since: Some(since),
                    ended_since: None,
                    projects: p
                        .sync
                        .projects_at
                        .get(&path)
                        .is_none_or(|at| p.now - at >= projects_every),
                    projects_check: held.and_then(|d| projects_check(d, p.now)),
                    recheck: held
                        .map(|d| {
                            d.mrs
                                .iter()
                                .filter(|m| m.state == MrState::Opened)
                                // A running pipeline, GitLab still checking
                                // (after a rebase), or a merge under way, failed
                                // or followed (merge_follow): none of these
                                // touches the merge request's update time.
                                .filter(|m| {
                                    matches!(
                                        m.pipeline,
                                        Some(Pipeline::Running | Pipeline::Pending)
                                    ) || crate::ready::blocks(m)
                                        .contains(&crate::ready::Block::Checking)
                                        || m.merge_ongoing
                                        || !m.merge_error.is_empty()
                                        || crate::merge_follow::following(p, &m.project_path, m.iid)
                                })
                                .map(|m| (m.project_path.clone(), m.iid))
                                .collect()
                        })
                        .unwrap_or_default(),
                    nudge: held
                        .map(|d| {
                            d.mrs
                                .iter()
                                .filter(|m| m.state == MrState::Opened)
                                .filter(|m| m.merge_status.eq_ignore_ascii_case("UNCHECKED"))
                                .map(|m| (m.project_path.clone(), m.iid))
                                .collect()
                        })
                        .unwrap_or_default(),
                    repo: held.map(|d| d.repo),
                },
                _ => Want {
                    changed_since: None,
                    ended_since: ended_since.clone(),
                    projects: true,
                    projects_check: None,
                    recheck: vec![],
                    nudge: vec![],
                    repo: held.map(|d| d.repo),
                },
            };
            (activity(held), path, want)
        })
        .collect();
    out.sort_by_key(|o| std::cmp::Reverse(o.0));
    out.into_iter()
        .map(|(_, path, want)| (path, want))
        .collect()
}

/// `held` with what a sync asked for (`want`) and brought back (`got`).
/// `now` and `history_days` move the window of merged and closed ones.
pub fn merge(
    held: Option<&GroupData>,
    want: &Want,
    got: Synced,
    now: i64,
    history_days: u64,
) -> GroupData {
    let held_projects = held.map(|h| h.projects.as_slice()).unwrap_or_default();
    let projects = got
        .projects
        .or_else(|| got.projects_seen.map(|seen| projects_now(held_projects, seen)));
    let Some(_) = want.changed_since else {
        // A full sync replaces everything.
        return GroupData {
            mrs: got.mrs,
            projects: projects.unwrap_or_else(|| held_projects.to_vec()),
            repo: got.repo,
            ended_since: want.ended_since.clone(),
        };
    };
    let mut data = held.cloned().unwrap_or_default();
    data.repo = got.repo;
    if let Some(projects) = projects {
        data.projects = projects;
    }
    for mr in got.rechecked {
        if let Some(m) = data
            .mrs
            .iter_mut()
            .find(|m| m.project_path == mr.project_path && m.iid == mr.iid)
        {
            apply_recheck(m, mr);
        }
    }
    // The window of merged and closed ones moves on with time.
    if data.ended_since.is_some() {
        data.ended_since = Some(time::to_rfc3339(now - history_days as i64 * 86_400));
    }
    let window = data.ended_since.as_deref().and_then(time::parse_rfc3339);
    for mr in got.mrs {
        data.mrs
            .retain(|m| !(m.project_path == mr.project_path && m.iid == mr.iid));
        let keep = mr.state == MrState::Opened
            || window.is_some_and(|w| time::parse_rfc3339(&mr.updated_at).is_some_and(|t| t >= w));
        if keep {
            data.mrs.push(mr);
        }
    }
    data.mrs.retain(|m| {
        m.state == MrState::Opened
            || window.is_some_and(|w| time::parse_rfc3339(&m.updated_at).is_some_and(|t| t >= w))
    });
    data
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::fixtures::mr;
    use crate::tests::sample;

    const A: &str = "acme/team-a";
    const NOW: i64 = 1_791_271_000;

    fn incremental() -> Want {
        Want {
            changed_since: Some("2026-10-06T00:00:00Z".into()),
            repo: Some(false),
            ..Default::default()
        }
    }

    #[test]
    fn the_first_sync_is_full_and_later_ones_ask_only_for_changes() {
        let mut p = sample();
        let plans = plan(&p, false);
        assert_eq!(plans.len(), 2);
        assert!(
            plans
                .iter()
                .all(|(_, w)| w.changed_since.is_none() && w.projects)
        );
        assert!(
            plans.iter().all(|(_, w)| w.ended_since.is_none()),
            "no view shows merged ones"
        );
        p.sync.full_done.insert(A.to_string());
        p.sync.projects_at.insert(A.into(), p.now);
        let plans = plan(&p, false);
        let (_, a) = plans.iter().find(|(path, _)| path == A).unwrap();
        assert_eq!(
            a.changed_since.as_deref(),
            Some("2026-10-06T07:00:00Z"),
            "the newest held"
        );
        assert!(!a.projects, "projects were fetched a moment ago");
        assert_eq!(a.repo, Some(false));
        let (_, b) = plans.iter().find(|(path, _)| path != A).unwrap();
        assert!(
            b.changed_since.is_none(),
            "team-b has not had its full sync"
        );
        // r: full again.
        assert!(
            plan(&p, true)
                .iter()
                .all(|(_, w)| w.changed_since.is_none())
        );
        // Projects come again after a while.
        p.now += PROJECTS_EVERY;
        assert!(
            plan(&p, false)
                .iter()
                .find(|(path, _)| path == A)
                .unwrap()
                .1
                .projects
        );
    }

    #[test]
    fn running_pipelines_are_asked_for_again() {
        let mut p = sample();
        p.sync.full_done.insert(A.into());
        p.data.get_mut(A).unwrap().mrs[0].pipeline = Some(Pipeline::Running);
        let want = plan(&p, false)
            .into_iter()
            .find(|(path, _)| path == A)
            .unwrap()
            .1;
        assert_eq!(want.recheck, vec![(format!("{A}/web"), 198)]);
    }

    /// After a rebase GitLab checks the merge request again; when it is
    /// done the merge request's update time has not moved, so it is asked
    /// for by number, or it would say "checking" for ever.
    #[test]
    fn a_merge_request_gitlab_is_still_checking_is_asked_for_again() {
        let mut p = sample();
        p.sync.full_done.insert(A.into());
        let mr = &mut p.data.get_mut(A).unwrap().mrs[0];
        mr.merge_status = "CHECKING".into();
        let want = plan(&p, false).into_iter().find(|(path, _)| path == A).unwrap().1;
        assert_eq!(want.recheck, vec![(format!("{A}/web"), 198)]);
        assert!(want.nudge.is_empty(), "GitLab is checking it already");
        // Unchecked: GitLab checks only when asked, so it is asked to.
        p.data.get_mut(A).unwrap().mrs[0].merge_status = "UNCHECKED".into();
        let want = plan(&p, false).into_iter().find(|(path, _)| path == A).unwrap().1;
        assert_eq!(want.nudge, vec![(format!("{A}/web"), 198)]);
        assert_eq!(want.recheck, vec![(format!("{A}/web"), 198)]);
    }

    /// GitLab's merge fields and radar's own follow-up change without the
    /// merge request's update time moving.
    #[test]
    fn a_merge_under_way_or_failed_or_being_checked_is_asked_again() {
        let mut p = sample();
        p.sync.full_done.insert(A.into());
        let recheck = |p: &GitlabPlugin| plan(p, false).into_iter().find(|(path, _)| path == A).unwrap().1.recheck;
        assert!(recheck(&p).is_empty(), "nothing to ask again");
        p.data.get_mut(A).unwrap().mrs[0].merge_ongoing = true;
        assert_eq!(recheck(&p), vec![(format!("{A}/web"), 198)]);
        let mr = &mut p.data.get_mut(A).unwrap().mrs[0];
        mr.merge_ongoing = false;
        mr.merge_error = "Branch cannot be merged".into();
        assert_eq!(recheck(&p), vec![(format!("{A}/web"), 198)]);
        p.data.get_mut(A).unwrap().mrs[0].merge_error.clear();
        p.merging.insert(
            crate::pages::mr_key(&format!("{A}/web"), 198),
            crate::merge_follow::Merging {
                group: A.into(),
                since: p.now,
                watch: crate::model::MergeWatch::Checking,
                connection: p.connection_stamp(),
            },
        );
        assert_eq!(recheck(&p), vec![(format!("{A}/web"), 198)]);
    }

    #[test]
    fn a_view_showing_merged_ones_makes_a_full_sync_with_them_once() {
        let mut p = sample();
        p.sync.full_done.insert(A.into());
        p.data.get_mut(A).unwrap().ended_since = None;
        p.cfg.views[0].filter = "state:merged".into();
        let want = plan(&p, false)
            .into_iter()
            .find(|(path, _)| path == A)
            .unwrap()
            .1;
        assert!(want.changed_since.is_none());
        assert_eq!(
            want.ended_since.as_deref(),
            Some(time::to_rfc3339(p.now - 14 * 86_400).as_str())
        );
        p.data.get_mut(A).unwrap().ended_since = want.ended_since.clone();
        let want = plan(&p, false)
            .into_iter()
            .find(|(path, _)| path == A)
            .unwrap()
            .1;
        assert!(want.changed_since.is_some(), "held now: changes only");
    }

    #[test]
    fn the_most_recently_active_tenancy_syncs_first_and_new_ones_before_all() {
        let mut p = sample();
        // team-b's draft is newer than everything in team-a.
        p.data.get_mut("acme/team-b").unwrap().mrs[0].updated_at = "2026-10-06T09:00:00Z".into();
        let order: Vec<String> = plan(&p, false).into_iter().map(|(path, _)| path).collect();
        assert_eq!(order, ["acme/team-b", A]);
        p.data.get_mut("acme/team-b").unwrap().mrs[0].updated_at = "2026-01-01T00:00:00Z".into();
        let order: Vec<String> = plan(&p, false).into_iter().map(|(path, _)| path).collect();
        assert_eq!(order, [A, "acme/team-b"]);
        p.data.remove(A);
        let order: Vec<String> = plan(&p, false).into_iter().map(|(path, _)| path).collect();
        assert_eq!(order[0], A, "nothing held yet: first");
    }

    #[test]
    fn changes_replace_held_ones_and_merged_ones_leave_unless_a_view_keeps_them() {
        let p = sample();
        // As held when no view shows merged ones: open ones only.
        let mut held = p.data[A].clone();
        held.ended_since = None;
        held.mrs.retain(|m| m.state == MrState::Opened);
        let held = &held;
        let n = held.mrs.len();
        let mut changed = mr(263, "alice", &format!("{A}/api"));
        changed.title = "renamed".into();
        changed.updated_at = "2026-10-06T08:00:00Z".into();
        let mut merged = mr(187, "me", &format!("{A}/web"));
        merged.state = MrState::Merged;
        merged.updated_at = "2026-10-06T08:00:00Z".into();
        let fresh = mr(400, "bob", &format!("{A}/docs"));
        let got = Synced {
            mrs: vec![changed, merged.clone(), fresh],
            repo: false,
            ..Default::default()
        };
        let data = merge(Some(held), &incremental(), got.clone(), NOW, 14);
        assert_eq!(data.mrs.len(), n, "one changed, one left, one new");
        assert!(
            data.mrs
                .iter()
                .any(|m| m.iid == 263 && m.title == "renamed")
        );
        assert!(
            !data.mrs.iter().any(|m| m.iid == 187),
            "merged: left the open ones"
        );
        assert!(data.mrs.iter().any(|m| m.iid == 400));
        assert_eq!(
            data.projects, held.projects,
            "projects kept when not asked for"
        );

        // With a view of merged ones held, it stays, as merged.
        let mut with_history = held.clone();
        with_history.ended_since = Some("2026-09-22T00:00:00Z".into());
        let data = merge(Some(&with_history), &incremental(), got, NOW, 14);
        assert_eq!(
            data.mrs.iter().find(|m| m.iid == 187).map(|m| m.state),
            Some(MrState::Merged)
        );
    }

    #[test]
    fn merged_ones_older_than_the_window_are_dropped() {
        let mut held = GroupData {
            ended_since: Some("2026-09-01T00:00:00Z".into()),
            ..Default::default()
        };
        let mut old = mr(1, "alice", "acme/a/api");
        old.state = MrState::Closed;
        old.updated_at = "2026-09-10T00:00:00Z".into();
        held.mrs.push(old);
        let data = merge(Some(&held), &incremental(), Synced::default(), NOW, 14);
        assert!(data.mrs.is_empty(), "older than 14 days before now");
        assert_eq!(data.ended_since, Some(time::to_rfc3339(NOW - 14 * 86_400)));
    }

    /// radar as it starts: the configuration, nothing held, nothing synced.
    fn started(now: i64) -> GitlabPlugin {
        let mut p = sample();
        p.data.clear();
        p.sync = Default::default();
        p.now = now;
        p
    }

    /// What the first sync after a start asks of team-a.
    fn first_sync_of_a(p: &mut GitlabPlugin) -> Want {
        use radar_core::Plugin;
        let now = p.now;
        assert!(!p.tick(now).is_empty(), "a sync starts");
        p.sync.in_flight = 0;
        plan(p, false).into_iter().find(|(path, _)| path == A).unwrap().1
    }

    /// The cache is the reference: started from one saved under a day ago,
    /// the first sync asks only for what changed since, its projects only
    /// looked at; from an older one, or one saved before the sync state
    /// was, it is full.
    #[test]
    fn a_fresh_cache_starts_with_changes_only_and_a_stale_or_old_one_in_full() {
        use radar_core::Plugin;
        let mut was = sample();
        was.sync.full_done.insert(A.into());
        was.sync.synced_at.insert(A.into(), was.now - 120);
        was.sync.last_ok = Some(was.now);
        let cache = was.snapshot().unwrap();
        assert_eq!(cache["sync"]["saved_at"], was.now);

        let mut p = started(was.now + 600);
        p.restore(cache.clone());
        assert_eq!(p.data, was.data, "drawn from the cache at once");
        let want = first_sync_of_a(&mut p);
        assert_eq!(want.changed_since.as_deref(), Some("2026-10-06T07:00:00Z"));
        assert!(want.projects && want.projects_check.is_some(), "projects only looked at");
        assert_eq!(p.sync.synced_at.get(A), Some(&(was.now - 120)));
        // team-b had no full sync before: it has one now.
        let b = plan(&p, false).into_iter().find(|(path, _)| path != A).unwrap().1;
        assert!(b.changed_since.is_none());

        let mut p = started(was.now + RESTART_FRESH);
        p.restore(cache.clone());
        assert!(first_sync_of_a(&mut p).changed_since.is_none(), "a day old: full");

        let mut old = cache;
        old.as_object_mut().unwrap().remove("sync");
        let mut p = started(was.now + 600);
        p.restore(old);
        assert_eq!(p.data, was.data, "an old cache still draws");
        assert!(first_sync_of_a(&mut p).changed_since.is_none(), "no sync state saved: full");
    }

    /// Saved again before the first sync took it up, the cache keeps the
    /// state it was restored with, and its time.
    #[test]
    fn a_cache_saved_before_the_first_sync_keeps_its_state() {
        use radar_core::Plugin;
        let mut was = sample();
        was.sync.full_done.insert(A.into());
        was.sync.last_ok = Some(was.now);
        let cache = was.snapshot().unwrap();
        let mut p = started(was.now + 600);
        p.restore(cache.clone());
        assert_eq!(p.snapshot().unwrap()["sync"], cache["sync"]);
    }

    fn project(path: &str, at: &str, main: Option<Pipeline>) -> Project {
        Project {
            name: path.rsplit('/').next().unwrap().into(),
            full_path: path.into(),
            last_activity: at.into(),
            main_pipeline: main,
            ..Default::default()
        }
    }

    /// Projects held: the next look asks their activity and names the ones
    /// to fetch whatever it says: main's pipeline running or pending (its
    /// end does not move the activity) or active within the hour. A quiet
    /// one is not named.
    #[test]
    fn a_projects_look_names_running_and_recently_active_ones() {
        let mut p = sample();
        p.sync.full_done.insert(A.into());
        let hour_ago = time::to_rfc3339(p.now - ACTIVITY_LAG - 1);
        let minute_ago = time::to_rfc3339(p.now - 60);
        p.data.get_mut(A).unwrap().projects = vec![
            project(&format!("{A}/quiet"), &hour_ago, Some(Pipeline::Passed)),
            project(&format!("{A}/running"), &hour_ago, Some(Pipeline::Running)),
            project(&format!("{A}/pending"), &hour_ago, Some(Pipeline::Pending)),
            project(&format!("{A}/busy"), &minute_ago, Some(Pipeline::Failed)),
        ];
        let want = plan(&p, false).into_iter().find(|(path, _)| path == A).unwrap().1;
        assert!(want.projects, "never fetched this session");
        let check = want.projects_check.unwrap();
        assert_eq!(check.seen.len(), 4);
        assert_eq!(check.seen[&format!("{A}/quiet")], hour_ago);
        let again: Vec<&str> = check.again.iter().map(String::as_str).collect();
        assert_eq!(again, [format!("{A}/busy"), format!("{A}/pending"), format!("{A}/running")]);
        // Nothing held: every project in full.
        p.data.get_mut(A).unwrap().projects.clear();
        let want = plan(&p, false).into_iter().find(|(path, _)| path == A).unwrap().1;
        assert_eq!(want.projects_check, None);
        // A full sync fetches them all.
        assert!(plan(&p, true).iter().all(|(_, w)| w.projects_check.is_none()));
    }

    /// A project as a full fetch leaves it, for the reference to keep.
    fn settled(path: &str, at: &str, head: &str, id: u64) -> Project {
        Project {
            default_branch: "main".into(),
            main_pipeline_id: Some(id),
            main_pipeline_at: "2026-10-05T12:30:00Z".into(),
            main_commit: Some(crate::model::MainCommit {
                sha: head.into(),
                short_id: head[..3].into(),
                author: "a".into(),
                at: at.into(),
                title: "t".into(),
            }),
            ..project(path, at, Some(Pipeline::Passed))
        }
    }

    /// Inside the hour GitLab's activity may not show a push, so the project
    /// is looked at; with a reference it is looked at by that (head commit
    /// and main's pipeline), without one it is fetched in full as before.
    #[test]
    fn a_recent_project_with_a_reference_is_looked_at_by_it_and_one_without_is_fetched() {
        let mut p = sample();
        p.sync.full_done.insert(A.into());
        let hour_ago = time::to_rfc3339(p.now - ACTIVITY_LAG - 1);
        let minute_ago = time::to_rfc3339(p.now - 60);
        let mut running = settled(&format!("{A}/running"), &minute_ago, "ccc333", 3);
        running.main_pipeline = Some(Pipeline::Running);
        let mut old_running = project(&format!("{A}/old-running"), &hour_ago, Some(Pipeline::Running));
        old_running.main_pipeline_id = None;
        p.data.get_mut(A).unwrap().projects = vec![
            settled(&format!("{A}/busy"), &minute_ago, "aaa111", 1),
            settled(&format!("{A}/quiet"), &hour_ago, "bbb222", 2),
            running,
            old_running,
            project(&format!("{A}/old-cache"), &minute_ago, Some(Pipeline::Passed)),
        ];
        let want = plan(&p, false).into_iter().find(|(path, _)| path == A).unwrap().1;
        let check = want.projects_check.unwrap();
        let look: Vec<&str> = check.look.iter().map(String::as_str).collect();
        assert_eq!(
            look,
            [format!("{A}/busy"), format!("{A}/running")],
            "active within the hour, or a pipeline running, with a reference"
        );
        let again: Vec<&str> = check.again.iter().map(String::as_str).collect();
        assert_eq!(
            again,
            [format!("{A}/old-cache"), format!("{A}/old-running")],
            "looked at but nothing to compare with: in full"
        );
        // The reference is each project's head commit and pipeline on main.
        let busy = &check.pins[&format!("{A}/busy")];
        assert_eq!((busy.head_sha.as_str(), busy.pipeline_id), ("aaa111", Some(1)));
        assert_eq!(busy.pipeline, Some(Pipeline::Passed));
        assert!(check.pins.contains_key(&format!("{A}/quiet")), "kept for when its activity moves");
        let running = &check.pins[&format!("{A}/running")];
        assert_eq!(running.pipeline, Some(Pipeline::Running), "its end shows as a difference");
        assert!(!check.pins.contains_key(&format!("{A}/old-running")), "no pipeline number held");
        assert!(!check.pins.contains_key(&format!("{A}/old-cache")), "no head commit id held");
    }

    /// The reference is the cache: saved with the projects, restored from
    /// it; a cache from before it was kept still reads and has none, so
    /// those projects are fetched in full once (which fills it).
    #[test]
    fn the_reference_is_saved_in_the_cache_and_an_old_cache_has_none() {
        use radar_core::Plugin;
        let mut was = sample();
        let held = settled(&format!("{A}/api"), "2026-10-06T00:00:00Z", "aaa111", 70);
        was.data.get_mut(A).unwrap().projects = vec![held.clone()];
        let cache = was.snapshot().unwrap();
        let saved = &cache["data"][A]["projects"][0];
        assert_eq!(saved["main_pipeline_id"], 70);
        assert_eq!(saved["main_pipeline_at"], "2026-10-05T12:30:00Z");
        assert_eq!(saved["main_commit"]["sha"], "aaa111");
        let mut p = started(was.now);
        p.restore(cache.clone());
        assert_eq!(p.data[A].projects[0], held);
        assert!(p.data[A].projects[0].pin().is_some());

        let mut old = cache;
        let project = old["data"][A]["projects"][0].as_object_mut().unwrap();
        project.remove("main_pipeline_id");
        project.remove("main_pipeline_at");
        project["main_commit"].as_object_mut().unwrap().remove("sha");
        let mut p = started(was.now);
        p.restore(old);
        let got = &p.data[A].projects[0];
        assert_eq!(got.main_pipeline, Some(Pipeline::Passed), "the rest still draws");
        assert_eq!((got.main_pipeline_id, got.pin()), (None, None));
    }

    /// What a look brings back: fetched ones replace held ones, quiet ones
    /// stay as held with their activity, gone ones leave.
    #[test]
    fn a_projects_look_keeps_quiet_ones_and_takes_fresh_ones() {
        let held = GroupData {
            projects: vec![
                project("a/gone", "t0", None),
                project("a/moved", "t0", Some(Pipeline::Running)),
                project("a/quiet", "t0", Some(Pipeline::Passed)),
            ],
            ..Default::default()
        };
        let got = Synced {
            projects_seen: Some(ProjectsSeen {
                present: vec![
                    ("a/moved".into(), "t1".into()),
                    ("a/new".into(), "t1".into()),
                    ("a/quiet".into(), "t0".into()),
                ],
                fresh: vec![
                    project("a/new", "t1", None),
                    project("a/moved", "t1", Some(Pipeline::Passed)),
                ],
            }),
            ..Default::default()
        };
        let data = merge(Some(&held), &incremental(), got, NOW, 14);
        let got: Vec<(&str, &str, Option<Pipeline>)> = data
            .projects
            .iter()
            .map(|p| (p.full_path.as_str(), p.last_activity.as_str(), p.main_pipeline))
            .collect();
        assert_eq!(
            got,
            [
                ("a/moved", "t1", Some(Pipeline::Passed)),
                ("a/new", "t1", None),
                ("a/quiet", "t0", Some(Pipeline::Passed)),
            ]
        );
    }

    /// A recheck brings back only a merge request's pipeline and merge
    /// status: those change on the held one, the rest stays.
    #[test]
    fn a_recheck_changes_only_the_pipeline_and_merge_status() {
        let p = sample();
        let held = p.data[A].clone();
        let before = held.mrs[0].clone();
        // As the recheck reads it: the rest at its defaults.
        let mut got_mr = crate::client::parse_mr(&serde_json::json!({}), "");
        got_mr.project_path = before.project_path.clone();
        got_mr.iid = before.iid;
        got_mr.pipeline = Some(Pipeline::Failed);
        got_mr.merge_status = "CI_MUST_PASS".into();
        got_mr.stages = vec![crate::model::Stage { name: "test".into(), status: "failed".into() }];
        let got = Synced {
            rechecked: vec![got_mr],
            ..Default::default()
        };
        let data = merge(Some(&held), &incremental(), got, NOW, 14);
        let after = data.mrs.iter().find(|m| m.iid == before.iid && m.project_path == before.project_path).unwrap();
        assert_eq!(after.pipeline, Some(Pipeline::Failed));
        assert_eq!(after.merge_status, "CI_MUST_PASS");
        assert_eq!(after.stages.len(), 1);
        assert_eq!((&after.title, &after.author, after.updated_at.as_str()), (&before.title, &before.author, before.updated_at.as_str()));
    }

    #[test]
    fn a_full_sync_replaces_everything() {
        let p = sample();
        let want = Want {
            ended_since: Some("2026-09-22T00:00:00Z".into()),
            projects: true,
            ..Default::default()
        };
        let got = Synced {
            mrs: vec![mr(9, "alice", "acme/x")],
            repo: true,
            projects: Some(vec![]),
            ..Default::default()
        };
        let data = merge(Some(&p.data[A]), &want, got, NOW, 14);
        assert_eq!(data.mrs.len(), 1);
        assert!(data.repo && data.projects.is_empty());
        assert_eq!(data.ended_since, want.ended_since);
    }
}
