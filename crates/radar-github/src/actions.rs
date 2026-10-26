//! Bounded, read-only Actions pages. Every held row belongs to its request scope.
use crate::{
    GithubPlugin, client, history,
    join::Target,
    model::{
        ApiError, Collection, Fresh, Marker, PullRequest, RepositoryId, RunPullMatch, RunsAsk,
        Validator, WorkflowJob, WorkflowRun, WorkflowStep,
    },
    sync,
};
use radar_core::journal::{ActionDescriptor, Outcome as ReadOutcome};
use radar_core::plugin::{Binding, Effect, Menu, MenuItem, Msg, Outcome};
use radar_core::ui::{self, rows::Move, scroll::Scroll};
use radar_core::view::{Chord, Key, Text};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Default)]
pub(crate) struct State {
    lists: BTreeMap<String, Runs>,
    runs: BTreeMap<String, Run>,
    attempts: Option<(u64, u64, Target, u64)>,
}
#[derive(Default)]
struct Runs {
    value: Option<Collection<WorkflowRun>>,
    pull: Option<PullRequest>,
    error: Option<String>,
    loading: bool,
    request: u64,
    row: usize,
    scroll: Scroll,
    /// What `value` was asked with, the validators it came with, the
    /// repository's moved state it was read at, and when. Reopening the tab
    /// on an unchanged repository asks nothing, or only whether it changed.
    ask: Option<RunsAsk>,
    tags: Vec<Validator>,
    marker: Option<Marker>,
    at: Option<i64>,
}
#[derive(Default)]
struct Run {
    value: Option<WorkflowRun>,
    jobs: Option<Collection<WorkflowJob>>,
    latest: Option<u64>,
    error: Option<String>,
    jobs_error: Option<String>,
    loading: bool,
    request: u64,
    row: usize,
    scroll: Scroll,
}
pub(crate) struct RunRead {
    run: WorkflowRun,
    latest: u64,
    jobs: Result<Collection<WorkflowJob>, String>,
}

/// What the runs read learned: the runs (or that they are unchanged), the
/// pull request a pull request page filters by, and what the runs were asked
/// with and at which moved state of the repository.
pub(crate) struct ListRead {
    runs: Fresh<Collection<WorkflowRun>>,
    pull: Option<PullRequest>,
    ask: RunsAsk,
    marker: Option<Marker>,
}

impl ListRead {
    #[cfg(test)]
    pub(crate) fn new(runs: Collection<WorkflowRun>, pull: Option<PullRequest>) -> Self {
        Self {
            runs: Fresh::New(runs, vec![]),
            pull,
            ask: RunsAsk {
                branch: None,
                per_page: 50,
            },
            marker: None,
        }
    }
}

pub(crate) enum Answer {
    Runs {
        generation: u64,
        page: u64,
        request: u64,
        target: Target,
        action: ActionDescriptor,
        result: Result<ListRead, String>,
        reply: sync::Reply,
    },
    Run {
        generation: u64,
        page: u64,
        request: u64,
        target: Target,
        action: ActionDescriptor,
        result: Result<RunRead, String>,
    },
}

/// Repositories whose runs the cache keeps (the most recently read).
const SAVED_MAX: usize = 20;

/// A repository's runs as the cache keeps them: drawn at once when its tab
/// opens after a start, then asked only whether they changed.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct SavedRuns {
    pub id: RepositoryId,
    value: Collection<WorkflowRun>,
    ask: RunsAsk,
    #[serde(default)]
    tags: Vec<Validator>,
    #[serde(default)]
    marker: Option<Marker>,
    at: i64,
}

pub(crate) fn saved(p: &GithubPlugin) -> Vec<SavedRuns> {
    let mut kept: Vec<_> = p
        .repos
        .iter()
        .filter_map(|repo| {
            let state = p
                .actions
                .lists
                .get(&Target::Repository(repo.id.clone()).url())?;
            Some(SavedRuns {
                id: repo.id.clone(),
                value: state.value.clone()?,
                ask: state.ask.clone()?,
                tags: state.tags.clone(),
                marker: state.marker.clone(),
                at: state.at?,
            })
        })
        .collect();
    kept.sort_by_key(|runs| std::cmp::Reverse(runs.at));
    kept.truncate(SAVED_MAX);
    kept
}

pub(crate) fn restore(p: &mut GithubPlugin, saved: SavedRuns) {
    let SavedRuns {
        id,
        value,
        ask,
        tags,
        marker,
        at,
    } = saved;
    // Another repository's rows never fill this one's tab.
    if !p.repos.iter().any(|r| r.id == id) || value.items.iter().any(|run| run.repository != id) {
        return;
    }
    p.actions.lists.insert(
        Target::Repository(id).url(),
        Runs {
            value: Some(value),
            ask: Some(ask),
            tags,
            marker,
            at: Some(at),
            ..Runs::default()
        },
    );
}

impl State {
    /// A settings save keeps what was read of the repositories still
    /// configured (same credential): their runs, not what was open.
    pub(crate) fn keep(self, ids: &[RepositoryId]) -> State {
        let lists = self
            .lists
            .into_iter()
            .filter(|(key, state)| {
                state.value.is_some()
                    && ids
                        .iter()
                        .any(|id| *key == Target::Repository(id.clone()).url())
            })
            .map(|(key, state)| {
                let state = Runs {
                    value: state.value,
                    ask: state.ask,
                    tags: state.tags,
                    marker: state.marker,
                    at: state.at,
                    ..Runs::default()
                };
                (key, state)
            })
            .collect();
        State {
            lists,
            ..State::default()
        }
    }
}

pub(crate) fn leave(p: &mut GithubPlugin) {
    p.actions.attempts = None;
    for state in p.actions.lists.values_mut() {
        state.loading = false;
        state.request = 0;
    }
    for state in p.actions.runs.values_mut() {
        state.loading = false;
        state.request = 0;
    }
}
pub(crate) fn title(p: &GithubPlugin) -> Option<String> {
    let target @ Target::Run(_, id, attempt) = scope(p)? else {
        return None;
    };
    let name = p
        .actions
        .runs
        .get(&target.url())
        .and_then(|s| s.value.as_ref())
        .and_then(|r| r.name.as_deref())
        .unwrap_or("workflow run");
    Some(format!("{name} · run {id} · attempt {attempt}"))
}
fn scope(p: &GithubPlugin) -> Option<&Target> {
    p.pages.last().map(|page| &page.target)
}
pub(crate) fn active(p: &GithubPlugin) -> bool {
    p.pages
        .last()
        .is_some_and(|page| matches!(page.target, Target::Run(..)) || page.tab == 2)
}
pub(crate) fn busy(p: &GithubPlugin) -> bool {
    scope(p).is_some_and(|target| match target {
        Target::Run(..) => p.actions.runs.get(&target.url()).is_some_and(|s| s.loading),
        _ => p
            .actions
            .lists
            .get(&target.url())
            .is_some_and(|s| s.loading),
    })
}
pub(crate) fn incomplete(p: &GithubPlugin) -> bool {
    if !active(p) {
        return false;
    }
    scope(p).is_some_and(|target| match target {
        Target::Run(..) => p.actions.runs.get(&target.url()).is_some_and(|s| {
            s.error.is_some()
                || s.jobs_error.is_some()
                || s.jobs.as_ref().is_some_and(|v| !v.completion.complete)
        }),
        _ => p.actions.lists.get(&target.url()).is_some_and(|s| {
            s.error.is_some() || s.value.as_ref().is_some_and(|v| !v.completion.complete)
        }),
    })
}
fn rows(state: &Runs) -> Vec<&WorkflowRun> {
    state
        .value
        .as_ref()
        .map(|v| {
            v.items
                .iter()
                .filter(|run| {
                    state
                        .pull
                        .as_ref()
                        .is_none_or(|pull| run.pull_match(pull).is_some())
                })
                .collect()
        })
        .unwrap_or_default()
}
pub(crate) fn selected_run(p: &GithubPlugin) -> Option<&WorkflowRun> {
    let target = scope(p)?;
    if matches!(target, Target::Run(..)) {
        return None;
    }
    let state = p.actions.lists.get(&target.url())?;
    rows(state).get(state.row).copied()
}
fn job_rows(jobs: &Collection<WorkflowJob>) -> Vec<(&WorkflowJob, Option<&WorkflowStep>)> {
    jobs.items
        .iter()
        .flat_map(|job| {
            std::iter::once((job, None)).chain(job.steps.iter().map(move |step| (job, Some(step))))
        })
        .collect()
}
fn selected_job(p: &GithubPlugin) -> Option<&WorkflowJob> {
    let target @ Target::Run(repo, id, attempt) = scope(p)? else {
        return None;
    };
    if p.pages.last()?.tab != 1 {
        return None;
    }
    let state = p.actions.runs.get(&target.url())?;
    let (job, _) = job_rows(state.jobs.as_ref()?).get(state.row).copied()?;
    (&job.repository == repo && job.run_id == *id && job.run_attempt == *attempt).then_some(job)
}
pub(crate) fn selected_target(p: &GithubPlugin) -> Option<Target> {
    if !active(p) {
        return None;
    }
    match scope(p)? {
        Target::Run(..) => scope(p).cloned(),
        _ => {
            selected_run(p).map(|run| Target::Run(run.repository.clone(), run.id, run.run_attempt))
        }
    }
}
pub(crate) fn link(p: &GithubPlugin) -> Option<String> {
    if let Some(job) = selected_job(p) {
        return Some(job.web_url());
    }
    selected_target(p).map(|target| target.url())
}
pub(crate) fn context(p: &GithubPlugin) -> Option<String> {
    let target = scope(p)?;
    if !active(p) {
        return None;
    }
    Some(format!(
        "{}:{}:{}:{}",
        p.generation,
        target.url(),
        p.pages.last()?.tab,
        link(p).unwrap_or_default()
    ))
}

pub(crate) fn ensure(p: &mut GithubPlugin) -> Vec<Effect> {
    if !active(p) {
        return vec![];
    }
    let Some(target) = scope(p).cloned() else {
        return vec![];
    };
    let loaded = match target {
        Target::Run(..) => p
            .actions
            .runs
            .get(&target.url())
            .is_some_and(|s| s.value.is_some() || s.loading),
        _ => p
            .actions
            .lists
            .get(&target.url())
            .is_some_and(|s| s.loading || (s.value.is_some() && current(p, &target, s))),
    };
    if loaded { vec![] } else { refresh(p) }
}

/// Runs held that need no read now: read at the repository's present moved
/// state and not long ago. A run still going, or no moved state to compare
/// (public reads), is asked again after [`sync::EVERY`]; the ask is
/// conditional, so what has not changed costs a 304.
fn current(p: &GithubPlugin, target: &Target, state: &Runs) -> bool {
    let Some(value) = &state.value else {
        return false;
    };
    // Seeded or restored without a stamp: held as read.
    let Some(at) = state.at else {
        return true;
    };
    let marker = p
        .repos
        .iter()
        .find(|r| r.id == *target.repository())
        .and_then(|r| r.sync.marker.as_ref());
    if state.marker.as_ref() != marker {
        // The repository moved since (or the read has no moved state).
        return false;
    }
    let running = value.items.iter().any(|run| run.status != "completed");
    let limit = if marker.is_some() && !running {
        sync::RUNS_EVERY
    } else {
        sync::EVERY
    };
    p.now - at < limit
}

/// Runs asked for: about two screens of the view, a page of at most 100.
fn runs_page(view_rows: u16) -> usize {
    match usize::from(view_rows) {
        0 => 50,
        rows => (rows * 2).clamp(30, 100).next_multiple_of(10).min(100),
    }
}
pub(crate) fn refresh(p: &mut GithubPlugin) -> Vec<Effect> {
    let Some(target) = scope(p).cloned() else {
        return vec![];
    };
    if busy(p) {
        return vec![Effect::Notify("these Actions are already loading".into())];
    }
    p.next_request = p.next_request.wrapping_add(1);
    let (generation, page, request) = (p.generation, p.page_request, p.next_request);
    let ids = p.cfg.ids();
    let token = p.cfg.token.clone();
    let api = p.cfg.api.clone();
    let via = p.via.clone();
    let kind = if matches!(target, Target::Run(..)) {
        "github.actions.run.read"
    } else {
        "github.actions.runs.read"
    };
    let action = history::action(kind, &target);
    let captured = action.clone();
    match target.clone() {
        Target::Run(repo, id, attempt) => {
            let state = p.actions.runs.entry(target.url()).or_default();
            state.loading = true;
            state.request = request;
            state.error = None;
            state.jobs_error = None;
            vec![Effect::SpawnAction {
                action,
                task: Box::pin(async move {
                    let result = async {
                        let c =
                            via.client(&ids, &token, &api).map_err(|e| e.to_string())?;
                        let latest = c.workflow_run(&repo, id).await.map_err(|e| e.to_string())?;
                        if attempt == 0 || attempt > latest.run_attempt {
                            return Err("requested attempt is not in the observed run".into());
                        }
                        let run = c
                            .workflow_run_attempt(&repo, id, attempt)
                            .await
                            .map_err(|e| e.to_string())?;
                        let jobs = c.workflow_jobs(&run).await.map_err(|e| e.to_string());
                        Ok(RunRead {
                            run,
                            latest: latest.run_attempt,
                            jobs,
                        })
                    }
                    .await;
                    Box::new(crate::Answer::Actions(Box::new(Answer::Run {
                        generation,
                        page,
                        request,
                        target,
                        action: captured,
                        result,
                    }))) as Msg
                }),
            }]
        }
        _ => {
            let per_page = runs_page(p.view_rows.get());
            let marker = p
                .repos
                .iter()
                .find(|r| r.id == *target.repository())
                .and_then(|r| r.sync.marker.clone());
            // A pull request held before is asked after, not read again.
            let known_pull = match &target {
                Target::Pull(repo, number) => p
                    .seen
                    .get(&(repo.clone(), *number))
                    .and_then(|seen| Some((seen.pull.clone(), seen.tag.clone()?))),
                _ => None,
            };
            let state = p.actions.lists.entry(target.url()).or_default();
            state.loading = true;
            state.request = request;
            state.error = None;
            // The validators of the last complete read, for the same ask.
            let held = state
                .value
                .as_ref()
                .filter(|v| v.completion.complete)
                .and(state.ask.clone())
                .map(|ask| (ask, state.tags.clone()));
            vec![Effect::SpawnAction {
                action,
                task: Box::pin(async move {
                    let (read, reply) = match via.client(&ids, &token, &api) {
                        Ok(c) => {
                            let read =
                                read_runs(&c, &target, per_page, known_pull, held, marker).await;
                            let reply = sync::Reply::of(&c, None, read.as_ref().err());
                            (read.map_err(|e| e.to_string()), reply)
                        }
                        Err(e) => (Err(e.to_string()), sync::Reply::default()),
                    };
                    Box::new(crate::Answer::Actions(Box::new(Answer::Runs {
                        generation,
                        page,
                        request,
                        target,
                        action: captured,
                        result: read,
                        reply,
                    }))) as Msg
                }),
            }]
        }
    }
}

/// A pull request page shows the runs of its source branch (and only those
/// that match it), so that branch's newest runs are what it asks for; the
/// repository's tab shows every branch.
fn ask_for(pull: Option<&PullRequest>, per_page: usize) -> RunsAsk {
    RunsAsk {
        branch: pull
            .map(|pull| pull.head_branch.clone())
            .filter(|branch| !branch.is_empty()),
        per_page,
    }
}

/// The validators of the last read, when it was this very ask: another page
/// size or branch is another URL, whose tag says nothing about it.
fn tags_for(held: Option<(RunsAsk, Vec<Validator>)>, ask: &RunsAsk) -> Vec<Validator> {
    held.filter(|(held, _)| held == ask)
        .map(|(_, tags)| tags)
        .unwrap_or_default()
}

/// The runs a list shows: for a pull request page the pull request first
/// (a 304 when it was seen), then its source branch's newest runs, else the
/// repository's newest runs; one page each, conditional on the last ask.
async fn read_runs(
    c: &client::Client,
    target: &Target,
    per_page: usize,
    known_pull: Option<(PullRequest, Validator)>,
    held: Option<(RunsAsk, Vec<Validator>)>,
    marker: Option<Marker>,
) -> Result<ListRead, ApiError> {
    let pull = match target {
        Target::Pull(repo, number) => {
            let known = known_pull.as_ref().map(|(pull, tag)| (pull, tag));
            match c.pull_if(repo, *number, known).await? {
                Fresh::New(pull, _) => Some(pull),
                Fresh::Same => known_pull.map(|(pull, _)| pull),
            }
        }
        _ => None,
    };
    let ask = ask_for(pull.as_ref(), per_page);
    let tags = tags_for(held, &ask);
    let runs = c.workflow_runs_if(target.repository(), &ask, &tags).await?;
    Ok(ListRead {
        runs,
        pull,
        ask,
        marker,
    })
}

pub(crate) fn receive(p: &mut GithubPlugin, answer: Answer) -> Vec<Effect> {
    match answer {
        Answer::Runs {
            generation,
            page,
            request,
            target,
            action,
            result,
            reply,
        } => {
            // What was spent counts, whichever page asked.
            let sync::Reply { stop, traffic, .. } = reply;
            p.sync.take(traffic, stop, p.now);
            // A 304 leaves the runs held, which are what the read answers.
            let held = p
                .actions
                .lists
                .get(&target.url())
                .and_then(|s| s.value.as_ref());
            let counted = result.as_ref().ok().and_then(|read| match &read.runs {
                Fresh::New(runs, _) => Some(runs),
                Fresh::Same => held,
            });
            let complete = counted.is_some_and(|v| v.completion.complete);
            let outcome = if result.is_err() {
                ReadOutcome::Failed
            } else if complete {
                ReadOutcome::Succeeded
            } else {
                ReadOutcome::Partial
            };
            let mut fx = history::finished(
                &action,
                outcome,
                counted.map(|v| v.items.len()),
                result.is_ok() && !complete,
            );
            let current = p
                .actions
                .lists
                .get(&target.url())
                .is_some_and(|s| s.request == request);
            if generation != p.generation
                || page != p.page_request
                || scope(p) != Some(&target)
                || !current
            {
                if generation == p.generation && current {
                    p.actions.lists.get_mut(&target.url()).unwrap().loading = false;
                }
                fx.push(history::skipped(&action));
                return fx;
            }
            let state = p.actions.lists.get_mut(&target.url()).unwrap();
            let selected = rows(state).get(state.row).map(|r| (r.id, r.run_attempt));
            state.loading = false;
            match result {
                Ok(ListRead {
                    runs,
                    pull,
                    ask,
                    marker,
                }) => {
                    let moved = matches!(runs, Fresh::New(..)) || state.marker != marker;
                    match runs {
                        Fresh::New(value, tags) => {
                            state.value = Some(value);
                            state.tags = tags;
                            state.ask = Some(ask);
                        }
                        // The runs held are current. Nothing held is a 304
                        // nobody asked for, which the client refuses.
                        Fresh::Same => {}
                    }
                    state.pull = pull;
                    state.marker = marker;
                    state.at = Some(p.now);
                    state.error = None;
                    state.row = selected
                        .and_then(|id| rows(state).iter().position(|r| (r.id, r.run_attempt) == id))
                        .unwrap_or(0);
                    // Written with the cache, for the next start's instant
                    // tab, when the rows or the moved state they match changed.
                    if moved && matches!(target, Target::Repository(_)) {
                        p.sync.dirty = true;
                    }
                }
                Err(error) => state.error = Some(error),
            }
            if std::mem::take(&mut p.sync.dirty) {
                fx.push(Effect::PersistCache);
            }
            fx
        }
        Answer::Run {
            generation,
            page,
            request,
            target,
            action,
            result,
        } => {
            let complete = result
                .as_ref()
                .is_ok_and(|got| got.jobs.as_ref().is_ok_and(|j| j.completion.complete));
            let outcome = if result.is_err() {
                ReadOutcome::Failed
            } else if complete {
                ReadOutcome::Succeeded
            } else {
                ReadOutcome::Partial
            };
            let mut fx = history::finished(
                &action,
                outcome,
                result
                    .as_ref()
                    .ok()
                    .and_then(|got| got.jobs.as_ref().ok())
                    .map(|j| j.items.len()),
                result.is_ok() && !complete,
            );
            let current = p
                .actions
                .runs
                .get(&target.url())
                .is_some_and(|s| s.request == request);
            if generation != p.generation
                || page != p.page_request
                || scope(p) != Some(&target)
                || !current
            {
                if generation == p.generation && current {
                    p.actions.runs.get_mut(&target.url()).unwrap().loading = false;
                }
                fx.push(history::skipped(&action));
                return fx;
            }
            let state = p.actions.runs.get_mut(&target.url()).unwrap();
            state.loading = false;
            match result {
                Ok(RunRead { run, latest, jobs }) => {
                    state.value = Some(run);
                    state.latest = Some(latest);
                    state.error = None;
                    match jobs {
                        Ok(value) => {
                            let selected = state.jobs.as_ref().and_then(|v| {
                                job_rows(v)
                                    .get(state.row)
                                    .map(|(j, s)| (j.id, s.map(|s| s.number)))
                            });
                            state.row = selected
                                .and_then(|id| {
                                    job_rows(&value)
                                        .iter()
                                        .position(|(j, s)| (j.id, s.map(|s| s.number)) == id)
                                })
                                .unwrap_or(0);
                            state.jobs = Some(value);
                            state.jobs_error = None;
                        }
                        Err(error) => state.jobs_error = Some(error),
                    }
                }
                Err(error) => state.error = Some(error),
            }
            fx
        }
    }
}

/// The same held Actions snapshot exposed to the renderer-neutral page.
pub(crate) fn neutral_blocks(p: &GithubPlugin) -> Vec<radar_core::view::Block> {
    use radar_core::view::{self as v, Block, Ink, Text};
    let Some(target) = scope(p) else {
        return vec![];
    };
    let mut blocks = Vec::new();
    if let Target::Run(..) = target {
        let Some(state) = p.actions.runs.get(&target.url()) else {
            return vec![Block::line(Text::of(
                "Run attempt has not been loaded.",
                Ink::Muted,
            ))];
        };
        if state.loading {
            blocks.push(Block::line(Text::of(
                "Loading this run attempt…",
                Ink::Info,
            )));
        }
        if let Some(error) = &state.error {
            blocks.push(Block::line(Text::of(error, Ink::Bad)));
        }
        if let Some(run) = &state.value {
            for line in [
                format!("workflow  {}", run.name.as_deref().unwrap_or("unknown")),
                format!("result  {}", run.status_label()),
                format!(
                    "run  #{} · id {} · attempt {}",
                    run.run_number, run.id, run.run_attempt
                ),
                format!(
                    "latest observed attempt  {}",
                    state
                        .latest
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "unknown".into())
                ),
                format!("event  {}", run.event),
                format!(
                    "branch  {}",
                    run.head_branch.as_deref().unwrap_or("unknown")
                ),
                format!("commit  {}", run.head_sha),
                format!("created  {}", run.created_at),
                format!("updated  {}", run.updated_at),
                run.display_title.clone(),
            ] {
                blocks.push(Block::line(Text::plain(line)));
            }
        }
        if p.pages.last().is_some_and(|page| page.tab == 1) {
            blocks.push(Block::line(v::kit::heading("Jobs and steps")));
            blocks.push(Block::line(Text::of("Jobs and steps belong to this exact run attempt. Step metadata contains no raw logs.", Ink::Muted)));
            if let Some(error) = &state.jobs_error {
                blocks.push(Block::line(Text::of(error, Ink::Bad)));
            }
            if let Some(jobs) = &state.jobs {
                if !jobs.completion.complete {
                    blocks.push(Block::line(Text::of("Partial jobs result.", Ink::Warn)));
                    if let Some(reason) = &jobs.completion.reason {
                        blocks.push(Block::line(Text::of(reason, Ink::Warn)));
                    }
                }
                for (i, (job, step)) in job_rows(jobs).into_iter().enumerate() {
                    let label = step.map_or_else(
                        || format!("{}  {}", job.status_label(), job.name),
                        |step| format!("  {}. {}  {}", step.number, step.status_label(), step.name),
                    );
                    blocks.push(Block::action(
                        format!("actions-job:{i}"),
                        Text::plain(label),
                    ));
                    if step.is_none() {
                        blocks.push(Block::line(Text::of(
                            format!(
                                "  started {} · completed {}",
                                job.started_at.as_deref().unwrap_or("unknown"),
                                job.completed_at.as_deref().unwrap_or("not reported")
                            ),
                            Ink::Muted,
                        )));
                    }
                }
            } else {
                blocks.push(Block::line(Text::of(
                    "Jobs are unknown; no results loaded.",
                    Ink::Warn,
                )));
            }
        }
    } else {
        let Some(state) = p.actions.lists.get(&target.url()) else {
            return vec![Block::line(Text::of(
                "Actions have not been loaded.",
                Ink::Muted,
            ))];
        };
        if state.loading {
            blocks.push(Block::line(Text::of("Loading workflow runs…", Ink::Info)));
        }
        if let Some(error) = &state.error {
            blocks.push(Block::line(Text::of(error, Ink::Bad)));
        }
        if state.value.as_ref().is_some_and(|v| !v.completion.complete) {
            blocks.push(Block::line(Text::of(
                "Partial result; more Actions may exist.",
                Ink::Warn,
            )));
            if let Some(reason) = state
                .value
                .as_ref()
                .and_then(|v| v.completion.reason.as_ref())
            {
                blocks.push(Block::line(Text::of(reason, Ink::Warn)));
            }
        }
        // Asked for the newest only: said, so a short list is not taken for all.
        if let Some(reason) = state
            .value
            .as_ref()
            .filter(|v| v.completion.complete)
            .and_then(|v| v.completion.reason.as_ref())
        {
            blocks.push(Block::line(Text::of(reason, Ink::Muted)));
        }
        if let Some(pull) = &state.pull {
            blocks.push(Block::line(Text::of(
                format!(
                    "Recent base-repository runs for #{} · head {}",
                    pull.number, pull.head_sha
                ),
                Ink::Muted,
            )));
            blocks.push(Block::line(Text::of("Declared PR associations and exact source/head matches are labeled separately. Merge-ref and older runs may be missing; no required-check or merge-readiness proof.", Ink::Muted)));
        } else {
            blocks.push(Block::line(Text::of(
                "Recent workflow runs · read-only · a run is not overall repository health.",
                Ink::Muted,
            )));
        }
        for (i, run) in rows(state).into_iter().enumerate() {
            let matched = state
                .pull
                .as_ref()
                .and_then(|pull| run.pull_match(pull))
                .map(|m| match m {
                    RunPullMatch::Associated => " · PR association",
                    RunPullMatch::HeadSha => " · exact head match",
                })
                .unwrap_or("");
            blocks.push(Block::action(
                format!("actions-run:{i}"),
                Text::plain(format!(
                    "{}  {} #{} · attempt {}{} · {} · {} · {}",
                    run.status_label(),
                    run.name.as_deref().unwrap_or("workflow"),
                    run.run_number,
                    run.run_attempt,
                    matched,
                    run.event,
                    run.head_branch.as_deref().unwrap_or("unknown branch"),
                    run.display_title
                )),
            ));
        }
        if rows(state).is_empty() {
            let words = if state.value.is_none() {
                "Workflow runs are unavailable."
            } else if state.pull.is_some() {
                "No associated or exact-head runs in this observed base-repository result. This is not proof of no PR activity."
            } else if state.value.as_ref().is_some_and(|v| !v.completion.complete) {
                "No workflow runs in this partial result."
            } else {
                "No workflow runs reported."
            };
            blocks.push(Block::line(Text::of(words, Ink::Muted)));
        }
    }
    blocks
}
pub(crate) fn repository_summary(p: &GithubPlugin, repo: &crate::RepoState) -> Vec<Text> {
    let key = Target::Repository(repo.id.clone()).url();
    let Some(state) = p.actions.lists.get(&key) else {
        return vec![Text::plain("Actions have not been loaded.")];
    };
    let Some(branch) = repo.repository.as_ref().map(|r| r.default_branch.as_str()) else {
        return vec![Text::plain(
            "Default branch is unknown; Actions health is unknown.",
        )];
    };
    let Some(value) = &state.value else {
        return vec![Text::plain(format!("Actions on {branch} are unknown."))];
    };
    let mut out = vec![Text::plain(format!("Latest observed Actions on {branch}"))];
    let mut observed = std::collections::BTreeMap::<u64, &WorkflowRun>::new();
    for run in value.items.iter().filter(|r| {
        r.head_branch.as_deref() == Some(branch)
            && r.head_repository.as_ref() == Some(&repo.id)
            && !matches!(
                r.event.as_str(),
                "pull_request" | "pull_request_target" | "merge_group"
            )
    }) {
        let old = observed.entry(run.workflow_id).or_insert(run);
        if (run.run_number, run.run_attempt) > (old.run_number, old.run_attempt) {
            *old = run;
        }
    }
    for run in observed.values() {
        out.push(Text::plain(format!(
            "{}  {} · #{} attempt {} · updated {}",
            run.status_label(),
            run.name.as_deref().unwrap_or("workflow"),
            run.run_number,
            run.run_attempt,
            run.updated_at
        )));
    }
    if observed.is_empty() {
        out.push(Text::plain(
            "No default-branch runs in this observed result.",
        ));
    }
    if state.error.is_some() || !value.completion.complete {
        out.push(Text::plain(
            "Results are stale or incomplete; current overall health is unknown.",
        ));
    }
    out.push(Text::plain(
        "Observed workflows only; this does not establish overall repository health.",
    ));
    out
}

pub(crate) fn key(p: &mut GithubPlugin, event: Chord) -> Option<Outcome> {
    if !active(p) {
        return None;
    }
    match event.key {
        Key::Char('r') => Some(Outcome::Handled(refresh(p))),
        Key::Char('o' | 'y') => Some(Outcome::Handled(
            link(p)
                .map(|url| {
                    if event.key == Key::Char('o') {
                        vec![Effect::Open(url)]
                    } else {
                        vec![Effect::Copy(url), Effect::Notify("link copied".into())]
                    }
                })
                .unwrap_or_else(|| vec![Effect::Notify("no Actions target is selected".into())]),
        )),
        Key::Enter => Some(Outcome::Handled(if let Some(job) = selected_job(p) {
            vec![Effect::Open(job.web_url())]
        } else if let Some(run) = selected_run(p) {
            p.open_target(Target::Run(run.repository.clone(), run.id, run.run_attempt))
        } else {
            vec![Effect::Notify("no Actions row is selected".into())]
        })),
        Key::Char('a') if matches!(scope(p), Some(Target::Run(..))) => Some(attempts_menu(p)),
        _ => {
            let target = scope(p)?.clone();
            if matches!(target, Target::Run(..)) && p.pages.last()?.tab == 0 {
                return None;
            }
            let (at, len, height) = match &target {
                Target::Run(..) => {
                    let s = p.actions.runs.get(&target.url())?;
                    (
                        s.row,
                        s.jobs.as_ref().map_or(0, |j| job_rows(j).len()),
                        s.scroll.height(),
                    )
                }
                _ => {
                    let s = p.actions.lists.get(&target.url())?;
                    (s.row, rows(s).len(), s.scroll.height())
                }
            };
            let movement = Move::from_key(event, height)?;
            let to = ui::rows::select(len, at, movement, |_| true);
            if to == at {
                return Some(Outcome::Ignored);
            }
            match target {
                Target::Run(..) => p.actions.runs.get_mut(&target.url()).unwrap().row = to,
                _ => p.actions.lists.get_mut(&target.url()).unwrap().row = to,
            };
            Some(Outcome::done())
        }
    }
}
pub(crate) fn click(p: &mut GithubPlugin, id: &str) -> Option<Outcome> {
    if !active(p) {
        return None;
    }
    let target = scope(p)?.clone();
    if let Some(index) = id
        .strip_prefix("actions-run:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        let state = p.actions.lists.get_mut(&target.url())?;
        if index >= rows(state).len() {
            return Some(Outcome::Ignored);
        }
        state.row = index;
        return Some(Outcome::done());
    }
    if let Some(index) = id
        .strip_prefix("actions-job:")
        .and_then(|n| n.parse::<usize>().ok())
    {
        let state = p.actions.runs.get_mut(&target.url())?;
        if index >= job_rows(state.jobs.as_ref()?).len() {
            return Some(Outcome::Ignored);
        }
        state.row = index;
        return Some(Outcome::done());
    }
    None
}
pub(crate) fn keys(p: &GithubPlugin) -> Vec<crate::Act> {
    let act = |verb: &str, b: Binding| crate::act("actions", verb, b);
    let mut rows = vec![crate::head(Binding::heading(
        selected_job(p)
            .map(|j| j.name.clone())
            .or_else(|| selected_target(p).map(|t| t.subject().label))
            .unwrap_or_else(|| "Actions".into()),
    ))];
    if link(p).is_some() {
        if selected_run(p).is_some() {
            rows.push(act("open_run", Binding::new("⏎", "open this run attempt")));
        } else if selected_job(p).is_some() {
            rows.push(act(
                "open_job",
                Binding::new("⏎", "open this job on GitHub"),
            ));
        }
        rows.extend([
            act("open_on_github", Binding::new("o", "open on GitHub")),
            act("copy_link", Binding::new("y", "copy the link")),
        ]);
    }
    if matches!(scope(p), Some(Target::Run(..))) {
        rows.push(act(
            "choose_attempt",
            Binding::new("a", "choose an observed run attempt…"),
        ));
    }
    rows.extend([
        act("refresh", Binding::new("r", "refresh these Actions").pane()),
        act("previous_tab", Binding::new("[", "previous tab").pane()),
        act("next_tab", Binding::new("]", "next tab").pane()),
        act("back", Binding::new("esc", "back to the previous page")),
    ]);
    rows
}
fn attempts_menu(p: &mut GithubPlugin) -> Outcome {
    let Some(target @ Target::Run(_, _, attempt)) = scope(p).cloned() else {
        return Outcome::Ignored;
    };
    let Some(latest) = p.actions.runs.get(&target.url()).and_then(|s| s.latest) else {
        return Outcome::Handled(vec![Effect::Notify(
            "latest attempt is unknown; load this run first".into(),
        )]);
    };
    // Do not allocate a provider-sized menu. Previous/current/latest are bounded choices.
    let mut choices = vec![1, attempt, latest];
    if attempt > 1 {
        choices.push(attempt - 1);
    }
    if attempt < latest {
        choices.push(attempt + 1);
    }
    choices.sort_unstable();
    choices.dedup();
    let items = choices
        .into_iter()
        .map(|n| {
            MenuItem::pick(
                format!("attempt {n}"),
                if n == attempt {
                    "current attempt"
                } else {
                    "read this exact attempt"
                },
                n.to_string(),
            )
        })
        .collect();
    p.actions.attempts = Some((p.generation, p.page_request, target, latest));
    Outcome::Handled(vec![Effect::Menu(Menu::new(
        "github-attempts",
        "run attempts · first, adjacent and latest observed",
        items,
    ))])
}
pub(crate) fn choose_attempt(p: &mut GithubPlugin, value: &str) -> Vec<Effect> {
    let Some((generation, page, target, latest)) = p.actions.attempts.take() else {
        return vec![];
    };
    if generation != p.generation || page != p.page_request || scope(p) != Some(&target) {
        return vec![Effect::Notify("run changed; reopen its attempts".into())];
    }
    let Target::Run(repo, id, current) = target else {
        return vec![];
    };
    let Some(attempt) = value.parse::<u64>().ok().filter(|n| *n > 0 && *n <= latest) else {
        return vec![];
    };
    if attempt == current {
        return vec![];
    }
    p.open_target(Target::Run(repo, id, attempt))
}

#[cfg(any(test, feature = "demo"))]
pub(crate) fn seed(p: &mut GithubPlugin) {
    use crate::model::{Completion, RunPullAssociation};
    let Some(repo) = p.repos.first() else {
        return;
    };
    let id = repo.id.clone();
    let Some(pull) = repo.pulls.as_ref().and_then(|v| v.items.first()).cloned() else {
        return;
    };
    let run = WorkflowRun {
        repository: id.clone(),
        id: 501,
        workflow_id: 301,
        run_number: 88,
        run_attempt: 2,
        name: Some("Build and test".into()),
        display_title: "Keep responses with their request".into(),
        path: ".github/workflows/ci.yml".into(),
        event: "push".into(),
        status: "completed".into(),
        conclusion: Some("failure".into()),
        head_sha: "a".repeat(40),
        head_branch: Some("main".into()),
        head_repository: Some(id.clone()),
        created_at: "2026-10-06T10:00:00Z".into(),
        updated_at: "2026-10-06T10:05:00Z".into(),
        run_started_at: Some("2026-10-06T10:00:10Z".into()),
        pull_requests: vec![RunPullAssociation {
            repository: id.clone(),
            number: 42,
            head_sha: pull.head_sha.clone(),
            head_repository: pull.head_repository.clone(),
        }],
    };
    let mut next = run.clone();
    next.id = 502;
    next.workflow_id = 302;
    next.run_number = 19;
    next.run_attempt = 1;
    next.name = Some("Documentation".into());
    next.status = "in_progress".into();
    next.conclusion = None;
    next.pull_requests.clear();
    next.head_repository = pull.head_repository.clone();
    next.head_branch = Some(pull.head_branch.clone());
    let collection = Collection {
        items: vec![run.clone(), next.clone()],
        completion: Completion::complete(1),
    };
    p.actions.lists.insert(
        Target::Repository(id.clone()).url(),
        Runs {
            value: Some(collection.clone()),
            ..Runs::default()
        },
    );
    p.actions.lists.insert(
        Target::Pull(id.clone(), 42).url(),
        Runs {
            value: Some(collection),
            pull: Some(pull),
            ..Runs::default()
        },
    );
    let jobs = vec![
        WorkflowJob {
            repository: id.clone(),
            id: 701,
            run_id: 501,
            run_attempt: 2,
            head_sha: run.head_sha.clone(),
            name: "Linux tests".into(),
            status: "completed".into(),
            conclusion: Some("failure".into()),
            started_at: Some("2026-10-06T10:00:10Z".into()),
            completed_at: Some("2026-10-06T10:05:00Z".into()),
            steps: vec![
                WorkflowStep {
                    number: 1,
                    name: "Checkout".into(),
                    status: "completed".into(),
                    conclusion: Some("success".into()),
                    started_at: None,
                    completed_at: None,
                },
                WorkflowStep {
                    number: 2,
                    name: "Run unit tests".into(),
                    status: "completed".into(),
                    conclusion: Some("failure".into()),
                    started_at: None,
                    completed_at: None,
                },
                WorkflowStep {
                    number: 3,
                    name: "Publish results".into(),
                    status: "completed".into(),
                    conclusion: Some("skipped".into()),
                    started_at: None,
                    completed_at: None,
                },
            ],
        },
        WorkflowJob {
            repository: id.clone(),
            id: 702,
            run_id: 501,
            run_attempt: 2,
            head_sha: run.head_sha.clone(),
            name: "Formatting".into(),
            status: "completed".into(),
            conclusion: Some("success".into()),
            started_at: None,
            completed_at: None,
            steps: vec![],
        },
    ];
    p.actions.runs.insert(
        Target::Run(id, 501, 2).url(),
        Run {
            value: Some(run),
            latest: Some(2),
            jobs: Some(Collection {
                items: jobs,
                completion: Completion::complete(1),
            }),
            ..Run::default()
        },
    );
}

#[cfg(test)]
#[path = "actions_tests.rs"]
mod tests;
