//! One review, from "review locally" to a saved note, as a state machine.
//!
//! The journey does no I/O. It is told what happened ([`Input`]) and answers
//! with what to do next ([`Action`]); the part (and, for the two questions
//! that belong to other parts, the host) does it. That keeps every step and
//! every failure testable with no provider, no clock and no network:
//!
//! ```text
//! Start → Resolve (the change request's owner) → EnsureWorktree (the repository owner) → StartSession (default
//! provider, review-mr brief) → the turn streams → SaveNote
//! ```
//!
//! Any step may fail, and the operator may stop it; each ends in a state the
//! dock can say in words, and nothing is saved from a review that did not
//! finish. Nothing in this module posts anything.

use std::collections::BTreeMap;
use std::path::PathBuf;

use radar_core::agent_context::{
    AgentContext, ChangeResolution, ChangeResolveRequest, ContextField, Resolve, WorktreeError, WorktreeReady,
    WorktreeRequest,
};
use radar_core::journal::SubjectKey;

use crate::brief::{Brief, BriefRequest, Recipe, TEMPLATE_VERSION};
use crate::event::{Durability, Event, EventKind, RadarSessionId, ProviderEventId};

use super::note::ReviewNote;

/// Seconds a resolution or a worktree may take before the journey gives up.
pub const STEP_TIMEOUT_SECS: i64 = 60;

/// The recipe shipped with radar.
pub fn recipe() -> Recipe {
    Recipe::built_in(RECIPE).expect("agents/recipes/review-mr.md is a valid recipe")
}

/// The review recipe's name: the operator's `<recipes folder>/review-mr.md` replaces the built-in.
pub const RECIPE: &str = "review-mr";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Resolving,
    Worktree,
    Starting,
    Running,
    Done,
    Failed(String),
    Stopped,
}

impl Step {
    pub fn finished(&self) -> bool {
        matches!(self, Step::Done | Step::Failed(_) | Step::Stopped)
    }
    /// The step in words for the dock's notice line.
    pub fn say(&self, title: &str) -> String {
        match self {
            Step::Resolving => format!("review: reading {title}"),
            Step::Worktree => format!("review: making a worktree for {title}"),
            Step::Starting => format!("review: starting the reviewer for {title}"),
            Step::Running => format!("review: reviewing {title}"),
            Step::Done => format!("review of {title} saved locally; nothing was posted"),
            Step::Failed(why) => format!("review of {title} failed: {why}"),
            Step::Stopped => format!("review of {title} stopped; nothing was saved"),
        }
    }
}

/// What happened, from outside.
#[derive(Debug)]
// One input per step of a review, moved once.
#[allow(clippy::large_enum_variant)]
pub enum Input<'a> {
    /// The owning part's answer to [`Action::Resolve`] (asked again until it is not Pending).
    Resolved(Resolve),
    /// The git part's answer to [`Action::EnsureWorktree`].
    Worktree(Result<WorktreeReady, WorktreeError>),
    /// The session exists.
    Started { session: RadarSessionId, model: Option<String> },
    /// The session could not be made, or the brief could not be sent.
    StartFailed(String),
    /// An event of the review's session, in the order the store ingested it.
    Event(&'a Event),
    /// The operator stopped the review.
    Stop,
    /// Time passes (the part's tick).
    Tick,
}

/// What to do next.
#[derive(Debug, PartialEq)]
// One action per step of a review, moved once.
#[allow(clippy::large_enum_variant)]
pub enum Action {
    /// Ask the part that owns the change request.
    Resolve(ChangeResolveRequest),
    /// Ask the part that owns repositories.
    EnsureWorktree(WorktreeRequest),
    /// Create a session with the default provider in `cwd`, send `brief`, and
    /// apply [`super::policy`] to its permission requests.
    StartSession(SessionSpec),
    /// Abort the running turn of this session.
    StopSession(RadarSessionId),
    /// Write the note.
    SaveNote(Box<ReviewNote>),
    /// Say it in the dock's notice line.
    Notice(String),
    /// A failed operation, independent of its icon or wording.
    Error(String),
}

#[derive(Debug, PartialEq, Eq)]
pub struct SessionSpec {
    pub cwd: PathBuf,
    pub brief: String,
    pub title: String,
    /// The recipe's agent and skills for the reviewer, when it names them.
    pub agent: Option<String>,
    pub skills: Vec<String>,
}

/// One review in progress.
pub struct Journey {
    change: SubjectKey,
    origin: Option<AgentContext>,
    step: Step,
    since: i64,
    resolution: Option<ChangeResolution>,
    worktree: Option<PathBuf>,
    session: Option<RadarSessionId>,
    model: Option<String>,
    provider: String,
    /// Assistant messages of the review turn, in the order they began.
    messages: Vec<(String, String)>,
    seen: BTreeMap<ProviderEventId, ()>,
    recipe: Recipe,
    now: i64,
}

impl Journey {
    /// `origin` is what the row it was asked on holds (the chat message that
    /// linked the change request): material for the brief.
    pub fn new(change: SubjectKey, origin: Option<AgentContext>, provider: &str, now: i64) -> (Journey, Vec<Action>) {
        Self::with_recipe(change, origin, provider, recipe(), now)
    }

    /// A review run with `recipe` (the operator's own when there is one).
    pub fn with_recipe(
        change: SubjectKey,
        origin: Option<AgentContext>,
        provider: &str,
        recipe: Recipe,
        now: i64,
    ) -> (Journey, Vec<Action>) {
        let journey = Journey {
            change: change.clone(),
            origin,
            step: Step::Resolving,
            since: now,
            resolution: None,
            worktree: None,
            session: None,
            model: None,
            provider: provider.to_owned(),
            messages: vec![],
            seen: BTreeMap::new(),
            recipe,
            now,
        };
        let first = vec![Action::Resolve(ChangeResolveRequest { subject: change })];
        (journey, first)
    }

    pub fn step(&self) -> &Step {
        &self.step
    }
    pub fn change(&self) -> &SubjectKey {
        &self.change
    }
    pub fn session(&self) -> Option<&RadarSessionId> {
        self.session.as_ref()
    }
    pub fn worktree(&self) -> Option<&PathBuf> {
        self.worktree.as_ref()
    }
    pub fn title(&self) -> &str {
        self.resolution.as_ref().map_or(self.change.id.as_str(), |r| r.title.as_str())
    }
    pub fn recipe_version(&self) -> u32 {
        self.recipe.version
    }

    /// Feed one input; `now` is the injected clock in unix seconds.
    pub fn feed(&mut self, input: Input<'_>, now: i64) -> Vec<Action> {
        self.now = now;
        if self.step.finished() {
            return vec![];
        }
        match input {
            Input::Resolved(resolve) => self.resolved(resolve),
            Input::Worktree(result) => self.worktree_made(result),
            Input::Started { session, model } => self.started(session, model),
            Input::StartFailed(why) => self.fail(format!("could not start the reviewer: {why}")),
            Input::Event(event) => self.event(event),
            Input::Stop => self.stop(),
            Input::Tick => self.tick(),
        }
    }

    fn enter(&mut self, step: Step) {
        self.step = step;
        self.since = self.now;
    }

    fn fail(&mut self, why: String) -> Vec<Action> {
        self.enter(Step::Failed(why));
        vec![Action::Error(self.step.say(self.title()))]
    }

    fn tick(&mut self) -> Vec<Action> {
        let waiting = matches!(self.step, Step::Resolving | Step::Worktree | Step::Starting);
        if waiting && self.now - self.since >= STEP_TIMEOUT_SECS {
            let at = match self.step {
                Step::Resolving => "reading the change request",
                Step::Worktree => "making the worktree",
                _ => "starting the reviewer",
            };
            return self.fail(format!("timed out {at}"));
        }
        vec![]
    }

    fn resolved(&mut self, resolve: Resolve) -> Vec<Action> {
        if self.step != Step::Resolving {
            return vec![];
        }
        match resolve {
            // Ask again; the timeout is the tick's.
            Resolve::Pending => vec![Action::Resolve(ChangeResolveRequest { subject: self.change.clone() })],
            Resolve::Failed(why) => self.fail(why),
            Resolve::Ready(r) if r.head_sha.trim().is_empty() => self.fail("the change request has no head commit".into()),
            Resolve::Ready(r) => {
                let request = WorktreeRequest {
                    repository: r.repository.clone(),
                    branch: r.source_branch.clone(),
                    sha: r.head_sha.clone(),
                    for_subject: Some(r.subject.clone()),
                };
                self.resolution = Some(r);
                self.enter(Step::Worktree);
                vec![Action::EnsureWorktree(request), Action::Notice(self.step.say(self.title()))]
            }
        }
    }

    fn worktree_made(&mut self, result: Result<WorktreeReady, WorktreeError>) -> Vec<Action> {
        if self.step != Step::Worktree {
            return vec![];
        }
        let ready = match result {
            Ok(ready) => ready,
            Err(WorktreeError::UnknownRepository) => {
                return self.fail("no clone of this repository is known to radar: add it to a workspace first".into());
            }
            Err(WorktreeError::ShaNotFound) => {
                return self.fail("the head commit is not on the remote branch any more (the branch moved or was deleted)".into());
            }
            Err(WorktreeError::Failed(why)) => return self.fail(format!("could not make the worktree: {why}")),
        };
        let Some(resolution) = self.resolution.clone() else { return self.fail("lost the change request".into()) };
        let brief = Brief::build(&BriefRequest {
            recipe: &self.recipe,
            task: String::new(),
            contexts: self.contexts(&resolution),
        });
        self.worktree = Some(ready.path.clone());
        self.enter(Step::Starting);
        vec![Action::StartSession(SessionSpec {
            cwd: ready.path,
            brief: brief.text,
            title: format!("review: {}", resolution.title),
            agent: self.recipe.agent_for(&self.provider),
            skills: self.recipe.skills.clone(),
        })]
    }

    fn contexts(&self, r: &ChangeResolution) -> Vec<AgentContext> {
        let mut change = AgentContext::new("change request", r.subject.clone(), &r.title)
            .link(r.link.clone())
            .field(ContextField::new("source_branch", &r.source_branch))
            .field(ContextField::new("target_branch", &r.target_branch))
            .field(ContextField::new("head_sha", &r.head_sha))
            .field(ContextField::new(
                "diff_stats",
                format!("{} files, +{} -{}", r.diff_stats.files, r.diff_stats.additions, r.diff_stats.deletions),
            ))
            .field(ContextField::new("description", &r.description));
        for (i, t) in r.threads.iter().enumerate() {
            let place = match (&t.file, t.line) {
                (Some(f), Some(l)) => format!("{f}:{l}"),
                (Some(f), None) => f.clone(),
                _ => "general".into(),
            };
            change = change.field(ContextField::new(&format!("open_thread_{}", i + 1), format!("{} on {place}: {}", t.author, t.body)));
        }
        let mut all = vec![change];
        all.extend(self.origin.clone());
        all
    }

    fn started(&mut self, session: RadarSessionId, model: Option<String>) -> Vec<Action> {
        if self.step != Step::Starting {
            return vec![];
        }
        self.session = Some(session);
        self.model = model;
        self.enter(Step::Running);
        vec![Action::Notice(self.step.say(self.title()))]
    }

    fn event(&mut self, event: &Event) -> Vec<Action> {
        if self.step != Step::Running || Some(&event.radar_session_id) != self.session.as_ref() {
            return vec![];
        }
        // Reconnects replay events; the store dedupes, the journey does too.
        if let Some(id) = &event.provider_event_id
            && self.seen.insert(id.clone(), ()).is_some()
        {
            return vec![];
        }
        match &event.kind {
            EventKind::ModelChanged { model } => {
                self.model = Some(model.clone());
                vec![]
            }
            EventKind::AssistantMessage { message_id, text } => {
                self.put(message_id, text.clone(), true);
                vec![]
            }
            EventKind::AssistantDelta { message_id, text } if event.durability == Durability::Ephemeral => {
                self.put(message_id, text.clone(), false);
                vec![]
            }
            EventKind::Error { message, retryable: false } => self.fail(format!("the reviewer failed: {message}")),
            EventKind::Aborted => self.stop_seen(),
            EventKind::TurnEnded | EventKind::Idle => self.finish(),
            _ => vec![],
        }
    }

    /// The finished message replaces what its deltas built; a delta appends.
    fn put(&mut self, id: &str, text: String, whole: bool) {
        match self.messages.iter_mut().find(|(m, _)| m == id) {
            Some((_, held)) if whole => *held = text,
            Some((_, held)) => held.push_str(&text),
            None => self.messages.push((id.to_owned(), text)),
        }
    }

    fn finish(&mut self) -> Vec<Action> {
        let body = self.messages.iter().map(|(_, t)| t.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join("\n\n");
        // The turn ended before the model said anything: not a review.
        if body.is_empty() {
            return vec![];
        }
        let Some(resolution) = &self.resolution else { return vec![] };
        let mut note = ReviewNote::from(resolution);
        note.provider = self.provider.clone();
        note.model = self.model.clone();
        note.recipe = self.recipe.name.clone();
        note.recipe_version = self.recipe.version;
        note.template_version = TEMPLATE_VERSION;
        note.reviewed_at = self.now;
        note.body = body;
        self.enter(Step::Done);
        vec![Action::SaveNote(Box::new(note)), Action::Notice(self.step.say(self.title()))]
    }

    fn stop(&mut self) -> Vec<Action> {
        let session = self.session.clone();
        let mut out = self.stop_seen();
        if let (Some(s), Step::Stopped) = (session, &self.step) {
            out.insert(0, Action::StopSession(s));
        }
        out
    }

    fn stop_seen(&mut self) -> Vec<Action> {
        self.enter(Step::Stopped);
        vec![Action::Notice(self.step.say(self.title()))]
    }
}
