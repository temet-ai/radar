//! The reviews in progress, between the journey (pure) and the part (which
//! owns the sessions and the tasks). The driver holds the journeys, the
//! questions waiting for another part, and the notes saved; it does no
//! session I/O: it says what the part should do ([`Todo`]).

use std::path::PathBuf;

use radar_core::agent_context::AgentContext;
use radar_core::journal::SubjectKey;
use radar_core::plugin::{NoteRequest, ReviewAnswer, ReviewAsk};

use super::journey::{Action, Input, Journey, SessionSpec};
use super::note::{ReviewNote, Reviews};
use crate::event::{Event, RadarSessionId};

/// What the part does for the driver.
#[derive(Debug, PartialEq)]
pub enum Todo {
    /// Create a session with the default provider in the spec's folder,
    /// send the brief, and tell the driver ([`Driver::bind`]) its id.
    StartSession { run: u64, spec: SessionSpec },
    /// Abort the running turn of this session.
    StopSession(RadarSessionId),
    /// Say it in the notice line.
    Notice(String),
    /// A failed operation, independent of its icon or wording.
    Error(String),
}

struct Run {
    id: u64,
    journey: Journey,
    session: Option<RadarSessionId>,
    /// The note, held until a part (or the fallback) has written it.
    note: Option<ReviewNote>,
}

#[derive(Default)]
pub struct Driver {
    next: u64,
    runs: Vec<Run>,
    /// Every session a review started, kept after its review ends: a
    /// follow-up message in the reviewer's tab is still the reviewer's.
    reviewers: std::collections::BTreeSet<RadarSessionId>,
    asks: Vec<(u64, ReviewAsk)>,
    pub reviews: Reviews,
    /// Where a note goes when no part keeps notes.
    pub fallback_dir: Option<PathBuf>,
    /// The operator's recipes folder: its `review-mr.md` replaces the built-in.
    pub recipes_dir: Option<PathBuf>,
}

impl Driver {
    pub fn start(&mut self, change: SubjectKey, origin: Option<AgentContext>, provider: &str, now: i64) -> Vec<Todo> {
        // One review of a change request at a time.
        if self.runs.iter().any(|r| r.journey.change() == &change && !r.journey.step().finished()) {
            return vec![Todo::Notice("a review of this change request is already running".into())];
        }
        let (recipe, _, broken) = crate::brief::Recipe::load(super::journey::RECIPE, self.recipes_dir.as_deref());
        let Some(recipe) = recipe else { return vec![Todo::Error("no review recipe".into())] };
        self.next += 1;
        let (journey, mut actions) = Journey::with_recipe(change, origin, provider, recipe, now);
        if let Some(why) = broken {
            actions.insert(0, Action::Error(format!("your review recipe is not used: {why}")));
        }
        self.runs.push(Run { id: self.next, journey, session: None, note: None });
        let id = self.next;
        self.apply(id, actions)
    }

    pub fn take_asks(&mut self) -> Vec<(u64, ReviewAsk)> {
        std::mem::take(&mut self.asks)
    }

    pub fn is_review(&self, radar: &RadarSessionId) -> bool {
        self.reviewers.contains(radar)
    }

    /// What the owning part answered.
    pub fn answer(&mut self, run: u64, answer: ReviewAnswer, now: i64) -> Vec<Todo> {
        let input = match answer {
            ReviewAnswer::Resolved(r) => Input::Resolved(r),
            ReviewAnswer::Worktree(w) => Input::Worktree(w),
            ReviewAnswer::NoteSaved(path) => return self.note_saved(run, path),
        };
        self.feed(run, input, now)
    }

    /// The session of `run` exists.
    pub fn bind(&mut self, run: u64, session: RadarSessionId, model: Option<String>, now: i64) -> Vec<Todo> {
        self.reviewers.insert(session.clone());
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == run) {
            r.session = Some(session.clone());
        }
        self.feed(run, Input::Started { session, model }, now)
    }

    pub fn start_failed(&mut self, session: &RadarSessionId, why: String, now: i64) -> Vec<Todo> {
        match self.run_of(session) {
            Some(run) => self.feed(run, Input::StartFailed(why), now),
            None => vec![],
        }
    }

    pub fn event(&mut self, event: &Event, now: i64) -> Vec<Todo> {
        match self.run_of(&event.radar_session_id) {
            Some(run) => self.feed(run, Input::Event(event), now),
            None => vec![],
        }
    }

    /// The operator stopped the turn of `session` (the part aborts it).
    pub fn stopped(&mut self, session: &RadarSessionId, now: i64) -> Vec<Todo> {
        let Some(run) = self.run_of(session) else { return vec![] };
        self.feed(run, Input::Stop, now).into_iter().filter(|t| !matches!(t, Todo::StopSession(_))).collect()
    }

    pub fn tick(&mut self, now: i64) -> Vec<Todo> {
        let ids: Vec<u64> = self.runs.iter().map(|r| r.id).collect();
        ids.into_iter().flat_map(|id| self.feed(id, Input::Tick, now)).collect()
    }

    fn run_of(&self, session: &RadarSessionId) -> Option<u64> {
        self.runs.iter().find(|r| r.session.as_ref() == Some(session)).map(|r| r.id)
    }

    fn feed(&mut self, run: u64, input: Input<'_>, now: i64) -> Vec<Todo> {
        let Some(r) = self.runs.iter_mut().find(|r| r.id == run) else { return vec![] };
        let actions = r.journey.feed(input, now);
        self.apply(run, actions)
    }

    fn apply(&mut self, run: u64, actions: Vec<Action>) -> Vec<Todo> {
        let mut todo = Vec::new();
        for action in actions {
            match action {
                Action::Resolve(r) => self.asks.push((run, ReviewAsk::Resolve(r))),
                Action::EnsureWorktree(w) => self.asks.push((run, ReviewAsk::Worktree(w))),
                Action::StartSession(spec) => todo.push(Todo::StartSession { run, spec }),
                Action::StopSession(s) => todo.push(Todo::StopSession(s)),
                Action::Notice(text) => todo.push(Todo::Notice(text)),
                Action::Error(text) => todo.push(Todo::Error(text)),
                Action::SaveNote(note) => {
                    if let Some(r) = self.runs.iter_mut().find(|r| r.id == run) {
                        let request = NoteRequest {
                            subject: r.journey.change().clone(),
                            file_name: note.file_name(),
                            text: note.render(),
                        };
                        r.note = Some(*note);
                        self.asks.push((run, ReviewAsk::SaveNote(request)));
                    }
                }
            }
        }
        todo
    }

    fn note_saved(&mut self, run: u64, path: Option<PathBuf>) -> Vec<Todo> {
        let Some(ix) = self.runs.iter().position(|r| r.id == run) else { return vec![] };
        let Some(note) = self.runs[ix].note.take() else { return vec![] };
        let path = match path {
            Some(path) => Ok(path),
            None => self.write_fallback(&note),
        };
        self.runs.remove(ix);
        match path {
            Ok(path) => {
                self.reviews.add(note.record(Some(path.display().to_string())));
                vec![Todo::Notice(format!("review of {} saved at {}; nothing was posted", note.title, path.display()))]
            }
            Err(why) => vec![Todo::Error(format!("the review of {} could not be saved: {why}", note.title))],
        }
    }

    fn write_fallback(&self, note: &ReviewNote) -> Result<PathBuf, String> {
        let dir = self.fallback_dir.clone().ok_or("no place to keep notes")?;
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(note.file_name());
        std::fs::write(&path, note.render()).map_err(|e| e.to_string())?;
        Ok(path)
    }
}

/// Reviews saved by earlier runs are read back from their notes, so a
/// restart still knows what was reviewed.
pub fn load(dir: &std::path::Path, into: &mut Reviews) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "md")
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Some(record) = ReviewNote::parse(&text)
        {
            into.add(super::note::ReviewRecord { path: Some(path.display().to_string()), ..record });
        }
    }
}
