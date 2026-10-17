//! Local-first sync. What radar last read is the reference: it is drawn at
//! start from the cache, and each round asks GitHub only what moved. With a
//! token that is one GraphQL query for every repository's open pull
//! requests (one rate-limit point per 50 repositories), then a conditional
//! REST read (a 304 is free) for each repository that moved. Public reads
//! have no GraphQL, so each repository is one conditional read. A
//! repository with nothing held (a first start) is asked for in that same
//! query with its description and pull requests, and what GraphQL cannot
//! give exactly is read over REST a few repositories at a time. The
//! query's batches go out together. A pull request page is asked again only
//! while its checks are still running; the Actions tab's runs are held with
//! the repository's moved state and asked again conditionally.
//! GitHub's refusals are waited out until the time it names, never retried
//! into, and a bucket running low waits for its reset.

use std::collections::{BTreeMap, VecDeque};

use radar_core::journal::Outcome as ReadOutcome;
use radar_core::plugin::{Effect, Msg};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{
    ApiError, ApiErrorKind, CheckTags, Checks, Collection, Full, Marker, Probed, PullRequest,
    Repository, RepositoryId, Traffic, Validator,
};
use crate::{Answer, GithubPlugin, RepoState, client, history};

/// A round a minute while someone is there; the host does not tick a
/// paused radar, so an idle one asks nothing.
pub(crate) const EVERY: i64 = 60;
/// Repository descriptions and default branches are asked again daily.
pub(crate) const META_EVERY: i64 = 24 * 3600;
/// Under a tenth of a bucket left, wait for its reset.
const LOW_SHARE: u64 = 10;
/// A refusal that names no time (a secondary limit) waits at least this.
const LIMIT_WAIT: i64 = 60;
/// No wait GitHub names is taken past an hour (a bad header).
const WAIT_MAX: i64 = 3600;
const BACKOFF_MAX: i64 = 15 * 60;
/// The cache layout. Another version is discarded once and rewritten.
pub(crate) const VERSION: u32 = 1;
/// Pull request pages kept for an instant reopen.
const SEEN_MAX: usize = 50;
/// Repositories read over REST at once. A first start with nothing cached
/// has 76 of them to read; a burst of 152 requests is slow to finish and
/// invites GitHub's secondary limit, so they are fed through this many.
pub(crate) const READS_AT_ONCE: usize = 6;
/// Repository workflow runs are asked again at the latest this often, even
/// when nothing in the repository moved (a push to a branch changes no pull
/// request); the ask is conditional, so an unchanged answer is a 304.
pub(crate) const RUNS_EVERY: i64 = 15 * 60;

/// What a repository's held data was read with: the validators to ask
/// again with, when its description was last asked, and the marker of the
/// open pull requests it matches.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepoSync {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_tag: Option<Validator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pulls_tags: Vec<Validator>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub marker: Option<Marker>,
}

#[derive(Default)]
pub(crate) struct Sync {
    pub last_started: Option<i64>,
    pub probing: bool,
    pub request: u64,
    /// The round was asked for (`r`, a settings save): it always reports.
    pub manual: bool,
    /// The round could not check what moved: it does not count as synced.
    pub refused: bool,
    pub blocked_until: Option<i64>,
    pub failures: u32,
    /// Everything sent since start, and GitHub's last word on each bucket.
    pub traffic: Traffic,
    /// Held data changed since the cache was last written.
    pub dirty: bool,
    pub said_complete: Option<bool>,
    /// Repositories (by index) to read over REST once a slot is free, and
    /// the marker each read answers.
    pub queue: VecDeque<(usize, Option<Marker>)>,
}

/// A pull request page read before: drawn at once when it opens again,
/// with the validators to ask GitHub whether it changed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Seen {
    pub pull: PullRequest,
    #[serde(default)]
    pub tag: Option<Validator>,
    #[serde(default)]
    pub checks: Option<Checks>,
    #[serde(default)]
    pub check_tags: CheckTags,
    pub at: i64,
}

/// What a read task reports beside its data.
#[derive(Default)]
pub(crate) struct Reply {
    /// The probe's marker this read answers, kept once it is complete.
    pub marker: Option<Marker>,
    /// A refusal that asks radar to wait: a rate limit, a 5xx, the network.
    pub stop: Option<ApiError>,
    pub traffic: Traffic,
}

impl Reply {
    pub(crate) fn of<'a>(
        client: &client::Client,
        marker: Option<Marker>,
        errors: impl IntoIterator<Item = &'a ApiError>,
    ) -> Self {
        Self {
            marker,
            stop: errors.into_iter().find(|e| stops(e)).cloned(),
            traffic: client.traffic(),
        }
    }
}

/// Refusals that are waited out rather than read past.
pub(crate) fn stops(err: &ApiError) -> bool {
    matches!(
        err.kind,
        ApiErrorKind::RateLimit | ApiErrorKind::Unavailable | ApiErrorKind::Network
    )
}

/// Refusals no other read would get past this round.
fn ends_round(err: &ApiError) -> bool {
    stops(err)
        || matches!(
            err.kind,
            ApiErrorKind::Authentication | ApiErrorKind::MissingToken | ApiErrorKind::InvalidToken
        )
}

impl Sync {
    /// When reads may go again: after a refusal's wait, or the reset of a
    /// bucket under a tenth left. `None` when they may go now.
    pub(crate) fn wait_until(&self, now: i64) -> Option<i64> {
        let low = self
            .traffic
            .rates
            .iter()
            .filter(|r| r.limit > 0 && r.remaining * LOW_SHARE < r.limit && r.reset > now)
            .map(|r| r.reset.min(now + WAIT_MAX));
        self.blocked_until
            .filter(|t| *t > now)
            .into_iter()
            .chain(low)
            .max()
    }

    /// A task's report: its traffic, and a refusal to wait out.
    pub(crate) fn take(&mut self, reply: Traffic, stop: Option<ApiError>, now: i64) {
        self.traffic.add(reply);
        match stop {
            Some(err) => self.stop(&err, now),
            None => self.failures = 0,
        }
    }

    /// Wait as GitHub asks: `Retry-After`, else the bucket's reset, else a
    /// minute. A 5xx or the network backs off 1, 2, 4… minutes up to 15.
    pub(crate) fn stop(&mut self, err: &ApiError, now: i64) {
        let until = if err.kind == ApiErrorKind::RateLimit {
            err.retry_after_secs
                .map(|s| now + (s as i64).clamp(1, WAIT_MAX))
                .or_else(|| {
                    err.rate_limit_reset
                        .and_then(|t| i64::try_from(t).ok())
                        .filter(|t| *t > now)
                        .map(|t| t.min(now + WAIT_MAX))
                })
                .unwrap_or(now + LIMIT_WAIT)
        } else {
            self.failures = self.failures.saturating_add(1);
            now + (30i64 << self.failures.min(5)).min(BACKOFF_MAX)
        };
        self.blocked_until = Some(self.blocked_until.map_or(until, |t| t.max(until)));
    }
}

/// `2m`, `45s`: how long until reads resume.
pub(crate) fn wait_words(secs: i64) -> String {
    match secs.max(1) {
        s if s >= 3600 => format!("{}h {}m", s / 3600, s % 3600 / 60),
        s if s >= 60 => format!("{}m", (s + 59) / 60),
        s => format!("{s}s"),
    }
}

pub(crate) fn waiting(p: &GithubPlugin) -> Option<String> {
    p.sync.wait_until(p.now).map(|until| {
        format!(
            "GitHub asked radar to wait · reads resume in {}",
            wait_words(until - p.now)
        )
    })
}

/// A round is due on the first tick, then each [`EVERY`] seconds after the
/// last one, unless a read is running or GitHub asked radar to wait. A settings
/// save that deferred reads (started, no round yet) waits to be asked.
pub(crate) fn due(p: &GithubPlugin, now: i64) -> bool {
    !p.repos.is_empty()
        && !p.sync.probing
        && p.sync.queue.is_empty()
        && !p.repos.iter().any(|r| r.loading)
        && p.sync.wait_until(now).is_none()
        && (!p.started || p.sync.last_started.is_some_and(|t| now - t >= EVERY))
}

/// Start a round: the probe with a token, else a conditional read of each
/// repository.
pub(crate) fn start(p: &mut GithubPlugin, manual: bool) -> Vec<Effect> {
    p.started = true;
    if p.repos.is_empty() {
        return vec![];
    }
    if p.sync.probing || !p.sync.queue.is_empty() || p.repos.iter().any(|r| r.loading) {
        return if manual {
            vec![Effect::Notify("GitHub reads are already running".into())]
        } else {
            vec![]
        };
    }
    if let Some(said) = waiting(p) {
        return if manual {
            vec![Effect::Notify(said)]
        } else {
            vec![]
        };
    }
    p.sync.last_started = Some(p.now);
    p.sync.manual = manual;
    p.sync.refused = false;
    if p.cfg.token.is_empty() {
        return read_all(p);
    }
    p.next_request = p.next_request.wrapping_add(1);
    p.sync.request = p.next_request;
    p.sync.probing = true;
    let (generation, request) = (p.generation, p.sync.request);
    let ids = p.cfg.ids();
    let token = p.cfg.token.clone();
    let api = p.cfg.api.clone();
    let via = p.via.clone();
    let asks = asks(p);
    let action = history::probe_action();
    let scope = action.clone();
    vec![Effect::SpawnAction {
        action,
        task: Box::pin(async move {
            let (result, traffic) = match via.client(&ids, &token, &api) {
                Ok(client) => (client.probe(&asks).await, client.traffic()),
                Err(err) => (Err(err), Traffic::default()),
            };
            Box::new(Answer::Probe {
                generation,
                request,
                action: scope,
                result,
                traffic,
            }) as Msg
        }),
    }]
}

/// Held data that needs no read for this marker.
fn current(repo: &RepoState, marker: Option<&Marker>, now: i64) -> bool {
    marker.is_some()
        && repo.sync.marker.as_ref() == marker
        && repo.error.is_none()
        && repo.repository.is_some()
        && repo
            .sync
            .repository_at
            .is_some_and(|t| now - t < META_EVERY)
        && repo
            .pulls
            .as_ref()
            .is_some_and(|pulls| pulls.completion.complete)
}

/// What the probe asks of each repository: its marker, and where nothing is
/// held yet its description and pull requests too, in this very query, which
/// saves the two REST reads each would cost.
pub(crate) fn asks(p: &GithubPlugin) -> Vec<(RepositoryId, bool)> {
    p.repos
        .iter()
        .map(|r| (r.id.clone(), r.repository.is_none() || r.pulls.is_none()))
        .collect()
}

/// Read every repository, a few at a time.
fn read_all(p: &mut GithubPlugin) -> Vec<Effect> {
    p.sync.queue = (0..p.repos.len()).map(|i| (i, None)).collect();
    pump(p)
}

/// Start queued repository reads while fewer than [`READS_AT_ONCE`] run. A
/// refusal that asks radar to wait ends the round: what is still queued is
/// not read into it.
pub(crate) fn pump(p: &mut GithubPlugin) -> Vec<Effect> {
    if p.sync.wait_until(p.now).is_some() {
        p.sync.refused |= !p.sync.queue.is_empty();
        p.sync.queue.clear();
        return vec![];
    }
    let mut effects = vec![];
    while p.repos.iter().filter(|r| r.loading).count() < READS_AT_ONCE {
        let Some((index, marker)) = p.sync.queue.pop_front() else {
            break;
        };
        effects.extend(p.refresh_repository(index, marker, false));
    }
    effects
}

/// Take what a probe read in full as the repository's data, as a complete
/// REST read would leave it (the validators come with the next REST read).
fn adopt(p: &mut GithubPlugin, index: usize, marker: Option<Marker>, full: Full) {
    let now = p.now;
    let repo = &mut p.repos[index];
    repo.repository = Some(full.repository);
    repo.sync.repository_tag = None;
    repo.sync.repository_at = Some(now);
    repo.sync.pulls_tags = vec![];
    repo.sync.marker = marker;
    repo.pulls = Some(full.pulls);
    repo.error = None;
    p.sync.dirty = true;
    p.links.borrow_mut().clear();
    p.under_actions.borrow_mut().clear();
    p.link_menu = None;
}

/// What moved came back: read only the repositories whose marker changed.
pub(crate) fn probed(
    p: &mut GithubPlugin,
    generation: u64,
    request: u64,
    action: radar_core::journal::ActionDescriptor,
    result: Result<Vec<Probed>, ApiError>,
    traffic: Traffic,
) -> Vec<Effect> {
    let outcome = if result.is_ok() {
        ReadOutcome::Succeeded
    } else {
        ReadOutcome::Failed
    };
    let mut effects =
        history::finished(&action, outcome, result.as_ref().ok().map(Vec::len), false);
    if generation != p.generation || request != p.sync.request || !p.sync.probing {
        effects.push(history::skipped(&action));
        return effects;
    }
    p.sync.probing = false;
    let now = p.now;
    match result {
        Ok(probes) => {
            p.sync.take(traffic, None, now);
            let mut probes: BTreeMap<_, _> = probes
                .into_iter()
                .map(|probed| (probed.id.clone(), probed))
                .collect();
            for i in 0..p.repos.len() {
                let probed = probes.remove(&p.repos[i].id);
                let marker = probed.as_ref().and_then(|probed| probed.marker.clone());
                if current(&p.repos[i], marker.as_ref(), now) {
                    continue;
                }
                match probed.and_then(|probed| probed.full) {
                    Some(full) => adopt(p, i, marker, full),
                    None => p.sync.queue.push_back((i, marker)),
                }
            }
            effects.extend(pump(p));
        }
        Err(err) if ends_round(&err) => {
            p.sync.take(traffic, stops(&err).then(|| err.clone()), now);
            p.sync.refused = true;
            if !stops(&err) {
                for repo in &mut p.repos {
                    repo.error = Some(err.to_string());
                }
            }
        }
        // GraphQL refused for another reason (a token without it): each
        // repository is one conditional read instead.
        Err(_) => {
            p.sync.take(traffic, None, now);
            effects.extend(read_all(p));
        }
    }
    effects.extend(p.finish_round());
    effects
}

/// Ask the open pull request page again while its checks are running, once
/// rounds run (not after a save that deferred reads, nor in the demo).
pub(crate) fn poll_page(p: &mut GithubPlugin, now: i64) -> Vec<Effect> {
    let on_pull = p
        .pages
        .last()
        .is_some_and(|page| matches!(page.target, crate::join::Target::Pull(..)));
    if !on_pull
        || p.sync.last_started.is_none()
        || crate::actions::active(p)
        || p.page_loading
        || p.sync.wait_until(now).is_some()
        || !p.checks.as_ref().is_some_and(Checks::running)
        || p.page_polled.is_some_and(|t| now - t < EVERY)
    {
        return vec![];
    }
    p.refresh_page()
}

/// Keep a pull request page for an instant reopen, newest [`SEEN_MAX`].
pub(crate) fn remember(p: &mut GithubPlugin, seen: Seen) {
    let key = (seen.pull.repository.clone(), seen.pull.number);
    p.seen.insert(key, seen);
    while p.seen.len() > SEEN_MAX {
        let Some(oldest) = p
            .seen
            .iter()
            .min_by_key(|(_, s)| s.at)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        p.seen.remove(&oldest);
    }
}

#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    #[serde(default)]
    last_sync: Option<i64>,
    #[serde(default)]
    repos: Vec<SavedRepo>,
    #[serde(default)]
    seen: Vec<Seen>,
    /// Workflow runs of the repositories whose Actions tab was read. Added
    /// to this layout: a cache without it reads as having none.
    #[serde(default)]
    runs: Vec<crate::actions::SavedRuns>,
}

#[derive(Serialize, Deserialize)]
struct SavedRepo {
    id: RepositoryId,
    #[serde(default)]
    repository: Option<Repository>,
    #[serde(default)]
    pulls: Option<Collection<PullRequest>>,
    #[serde(default)]
    sync: RepoSync,
}

/// What radar shows, with what it was read with, for the next start.
pub(crate) fn snapshot(p: &GithubPlugin) -> Option<Value> {
    let repos = p
        .repos
        .iter()
        .filter(|r| r.repository.is_some() || r.pulls.is_some())
        .map(|r| SavedRepo {
            id: r.id.clone(),
            repository: r.repository.clone(),
            pulls: r.pulls.clone(),
            sync: r.sync.clone(),
        })
        .collect();
    serde_json::to_value(Saved {
        version: VERSION,
        last_sync: p.last_sync,
        repos,
        seen: p.seen.values().cloned().collect(),
        runs: crate::actions::saved(p),
    })
    .ok()
}

/// Take the cache as the reference for the configured repositories. A cache
/// of another layout, or one that does not read, is discarded once: the
/// next round writes a new one.
pub(crate) fn restore(p: &mut GithubPlugin, cache: Value) {
    let version = cache.get("version").and_then(Value::as_u64);
    let saved = match serde_json::from_value::<Saved>(cache) {
        Ok(saved) if version == Some(u64::from(VERSION)) => saved,
        _ => {
            radar_core::log::info(format!(
                "github cache ({}) discarded; the next sync writes a new one",
                version.map_or("unreadable".into(), |v| format!("version {v}"))
            ));
            p.sync.dirty = true;
            return;
        }
    };
    let mut restored = false;
    for saved in saved.repos {
        let Some(state) = p.repos.iter_mut().find(|s| s.id == saved.id) else {
            continue;
        };
        let consistent = saved.repository.as_ref().is_none_or(|r| r.id == saved.id)
            && saved
                .pulls
                .as_ref()
                .is_none_or(|pulls| pulls.items.iter().all(|pull| pull.repository == saved.id));
        if consistent {
            state.repository = saved.repository;
            state.pulls = saved.pulls;
            state.sync = saved.sync;
            restored = true;
        }
    }
    for mut seen in saved.seen {
        if !p.repos.iter().any(|r| r.id == seen.pull.repository) {
            continue;
        }
        if seen
            .checks
            .as_ref()
            .is_some_and(|c| c.head_sha != seen.pull.head_sha)
        {
            seen.checks = None;
            seen.check_tags.clear();
        }
        remember(p, seen);
    }
    for runs in saved.runs {
        crate::actions::restore(p, runs);
    }
    if restored {
        p.last_sync = saved.last_sync;
    }
}
