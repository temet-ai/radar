//! Session store: identity, cursor and bounded metadata, plus the live
//! timeline the dock draws.
//!
//! Only [`SessionMeta`] is persisted (radar's private state). The timeline is
//! rebuilt from the provider on resume: transcripts stay provider-owned.
//!
//! [`Session::ingest`] is where the rules of the envelope live:
//! - an event seen before (same provider event id) is dropped;
//! - events apply in `sequence` order even when they arrive out of order;
//! - an ephemeral delta shows as unsaved text until the persisted message
//!   with the same message id replaces it, and a delta arriving after that
//!   message is dropped;
//! - a permission request delivered twice is one request, and nothing
//!   answers it but the operator.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::event::{
    Durability, Event, EventKind, RadarSessionId, PermissionRequest, ProviderEventId, ProviderId,
    ProviderSessionId, QuestionReply, QuestionRequest,
};

/// Items kept per session; older ones are dropped and `truncated` is set.
pub const TIMELINE_CAP: usize = 2000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Created, nothing sent yet.
    #[default]
    Idle,
    /// A turn is running.
    Running,
    /// A turn waits for the operator to answer a permission request.
    WaitingApproval,
    /// The last turn was stopped.
    Stopped,
    Error,
    /// The connection to the provider is lost.
    Disconnected,
}

/// What radar keeps about a session between runs. Bounded: no prompts, no
/// output, no arguments.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMeta {
    /// Read under its old name too: sessions saved before the rename to radar.
    #[serde(alias = "radar_session_id")]
    pub radar_session_id: RadarSessionId,
    pub provider: ProviderId,
    pub provider_session_id: ProviderSessionId,
    pub cwd: String,
    pub title: String,
    pub model: Option<String>,
    pub created_at: i64,
    /// The last persisted provider event seen: where a reconnect resumes.
    pub cursor: Option<ProviderEventId>,
    /// The agent the session runs as, when not the CLI's own: it is resumed as the same agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// A review session (the review journey's): it is resumed with radar's
    /// strict review policy, never the operator's own approvals.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review: bool,
}

/// Whether radar is attached to the provider's session: a session restored
/// from the last run is not, until it is resumed (its provider's process,
/// and whatever that process was asking, ended with the last run).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Attach {
    #[default]
    Attached,
    /// Restored, or its resume failed: a message or the resume key resumes it.
    Detached,
    /// radar reads it back from the provider; what arrives meanwhile is history.
    Resuming,
}

/// What [`Session::lapse`] found waiting when the history ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lapsed {
    pub requests: usize,
    pub questions: usize,
    /// The history ended inside a turn (waiting or still running).
    pub turn: bool,
}

impl Lapsed {
    pub fn any(&self) -> bool {
        self.requests + self.questions > 0 || self.turn
    }

    /// What the transcript says of it, if anything.
    pub fn words(&self) -> Option<&'static str> {
        if self.requests + self.questions > 0 {
            Some("the turn that was waiting was ended when radar closed; send a message to carry on")
        } else if self.turn {
            Some("the turn that was running was ended when radar closed; send a message to carry on")
        } else {
            None
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolState {
    Running,
    Ok,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PermissionState {
    Pending,
    Approved,
    Denied,
    /// It waited when radar closed: the process that asked is gone, so nothing
    /// can answer it now.
    Lapsed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuestionState {
    Pending,
    Answered,
    /// Declined by the operator, or cancelled (the turn stopped, the
    /// provider went away).
    Declined,
    /// It waited when radar closed, as [`PermissionState::Lapsed`].
    Lapsed,
}

/// One row of the transcript.
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    User { text: String },
    Assistant { message_id: String, text: String, saved: bool },
    Reasoning { message_id: String, text: String, saved: bool },
    Tool { call_id: String, name: String, summary: String, state: ToolState, result: Option<String>, diff: Option<String> },
    Permission { request: PermissionRequest, state: PermissionState },
    /// A question the agent asked; `replies` are what was answered, once it is.
    Question { request: QuestionRequest, state: QuestionState, replies: Vec<QuestionReply> },
    Error { message: String, retryable: bool },
    Notice { text: String },
}

/// Who holds a message the operator sent while a turn ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Holder {
    /// radar: the provider takes nothing mid-turn, so it is sent when the turn ends.
    Radar,
    /// The provider: it was sent and the provider runs it after this turn
    /// (or radar sent it and the provider has not shown it yet).
    Provider,
}

/// A message on its way: shown at the foot of the transcript, marked, until
/// the provider shows it as the operator's (its user message event).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Queued {
    pub text: String,
    pub holder: Holder,
}

/// What [`Session::ingest`] did with an event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ingested {
    Applied,
    /// Seen before (same provider event id, or same sequence without one).
    Duplicate,
    /// Late and no longer meaningful (a delta after its message).
    Dropped,
}

/// The turn running, or the last one, in the events' own terms (no clock:
/// the plugin times turns at its own clock by watching `seq` and `open`).
///
/// A turn is everything from the user's message, or the provider's turn
/// start, to the session going idle, being stopped or failing. A provider's
/// own turn end (Copilot ends one per model call) closes it softly: a turn
/// start or more text before the session is idle carries on the same turn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnMark {
    /// Turns started so far; 0 before the first.
    pub seq: u64,
    pub open: bool,
    /// Closed by the provider's turn end only: more may follow.
    soft: bool,
    /// Tokens (input and output) used before it started.
    usage_before: u64,
    /// The provider reported usage since it started.
    reported: bool,
    /// Something live (not kept by the provider) happened in it: streamed
    /// text, usage, the session going idle. A turn read back from history on
    /// resume has only persisted events, and is not news.
    pub live: bool,
}

impl TurnMark {
    fn start(&mut self, usage: u64) {
        if self.open {
            return;
        }
        if self.soft {
            self.open = true;
            self.soft = false;
            return;
        }
        *self = TurnMark { seq: self.seq + 1, open: true, soft: false, usage_before: usage, reported: false, live: false };
    }

    /// It ended for good: the session went idle, was stopped or failed.
    pub fn settled(&self) -> bool {
        self.seq > 0 && !self.open && !self.soft
    }

    fn end(&mut self, soft: bool) {
        if self.open || self.soft {
            self.open = false;
            self.soft = soft;
        }
    }
}

/// A session in memory: its metadata and what the dock draws.
#[derive(Clone, Debug)]
pub struct Session {
    pub meta: SessionMeta,
    pub state: SessionState,
    pub items: Vec<Item>,
    pub truncated: bool,
    pub usage: (u64, u64),
    /// The turn running, or the last one.
    pub turn: TurnMark,
    /// The slash commands the provider reported for this session (not history: never stored,
    /// reported again when the session is resumed). `None` until it reports.
    pub commands: Option<Vec<crate::adapter::SlashCommand>>,
    /// Messages the operator sent that the provider has not shown yet, oldest
    /// first. Not history: never stored, gone with the session.
    pub queued: Vec<Queued>,
    /// radar sent a message and the provider has not said a turn started (or
    /// showed the message) yet: the session counts as busy meanwhile, so a
    /// second message waits instead of meeting a provider mid-start.
    sending: bool,
    /// Events applied so far, by sequence, kept to place a late event.
    applied: BTreeMap<u64, Event>,
    seen_ids: BTreeSet<ProviderEventId>,
    /// Messages already replaced by their persisted form.
    saved: BTreeSet<String>,
    connected: bool,
    /// The operator's answers to a request's questions so far, by request
    /// id: a request with several questions is answered one at a time and
    /// goes to the agent when every question has its reply. Kept across a
    /// rebuild, and after the answer, to show what was chosen.
    drafts: BTreeMap<String, Vec<QuestionReply>>,
    /// What the operator approved for this session ([`PermissionRequest::session_key`]),
    /// and the requests answered that way. In memory only: a session radar
    /// starts again asks again, as the provider's own session approval does.
    for_session: BTreeSet<String>,
    approved_for_session: BTreeSet<String>,
    /// Attached to the provider's session, or not yet (restored), or being read back.
    pub attach: Attach,
    /// When the plugin last began to resume it, at the plugin's clock.
    pub attach_at: i64,
    /// Requests and questions that lapsed, by request id, and the newest
    /// sequence applied when they did: a rebuild keeps them lapsed, and the
    /// provider asking one again after that is a request that waits.
    lapsed: BTreeSet<String>,
    lapsed_through: u64,
    /// The turn the history ended inside ([`TurnMark::seq`]): a rebuild keeps it ended.
    cut: Option<u64>,
    /// The tool calls that were running when it ended: they never finish.
    cut_calls: BTreeSet<String>,
}

impl Session {
    /// The history read back on resume has ended: whatever still waits was
    /// asked by a process that ended with the last run, so nothing can
    /// answer it. Each such request and question lapses (shown without
    /// buttons, never "needs you"), and a turn the history ends inside is
    /// over. Says what it found.
    pub fn lapse(&mut self) -> Lapsed {
        let mut found = Lapsed { turn: self.turn.open, ..Lapsed::default() };
        // A provider's own turn end (soft) at the end of the history is the turn's end: nothing more follows.
        if self.turn.soft {
            self.turn.end(false);
        }
        for item in &mut self.items {
            match item {
                Item::Permission { request, state } if *state == PermissionState::Pending => {
                    *state = PermissionState::Lapsed;
                    self.lapsed.insert(request.request_id.clone());
                    found.requests += 1;
                }
                Item::Question { request, state, .. } if *state == QuestionState::Pending => {
                    *state = QuestionState::Lapsed;
                    self.lapsed.insert(request.request_id.clone());
                    found.questions += 1;
                }
                Item::Tool { call_id, state: ToolState::Running, .. } => {
                    self.cut_calls.insert(call_id.clone());
                }
                _ => {}
            }
        }
        self.lapsed_through = self.applied.keys().next_back().copied().unwrap_or(0);
        if found.any() {
            self.cut = Some(self.turn.seq);
            self.end_cut_turn();
        } else {
            self.cut_calls.clear();
        }
        found
    }

    /// The turn the history ended inside stays over, and the calls it was running never finish; what was asked
    /// again after it still waits.
    fn end_cut_turn(&mut self) {
        for item in &mut self.items {
            if let Item::Tool { call_id, state, result, .. } = item
                && *state == ToolState::Running
                && self.cut_calls.contains(call_id)
            {
                *state = ToolState::Failed;
                *result = Some("radar closed before it finished".into());
            }
        }
        if self.cut.is_some() && self.cut == Some(self.turn.seq) {
            self.turn.end(false);
            if matches!(self.state, SessionState::Running | SessionState::WaitingApproval) {
                self.state = self.derived_state();
            }
        }
    }

    /// The request event `request_id` at `sequence` is one that lapsed (or a rebuild's replay of it).
    fn lapsed_ask(&self, request_id: &str, sequence: u64) -> bool {
        self.lapsed.contains(request_id) && sequence <= self.lapsed_through
    }

    /// Attached to the provider's session: what it asks can be answered.
    pub fn attached(&self) -> bool {
        self.attach == Attach::Attached
    }

    /// The operator approved `request` for the rest of this session.
    pub fn approve_for_session(&mut self, request: &PermissionRequest) {
        self.for_session.insert(request.session_key());
        self.approved_for_session.insert(request.request_id.clone());
    }

    /// The same kind and target was approved for this session before.
    pub fn approved_for_session(&self, request: &PermissionRequest) -> bool {
        self.for_session.contains(&request.session_key())
    }

    /// Request `request_id` was answered "approve for session".
    pub fn answered_for_session(&self, request_id: &str) -> bool {
        self.approved_for_session.contains(request_id)
    }

    /// Tokens the current or last turn used, when the provider reported usage in it.
    pub fn turn_tokens(&self) -> Option<u64> {
        self.turn.reported.then(|| (self.usage.0 + self.usage.1).saturating_sub(self.turn.usage_before))
    }

    pub fn new(meta: SessionMeta) -> Self {
        Self {
            meta,
            state: SessionState::Idle,
            items: Vec::new(),
            truncated: false,
            usage: (0, 0),
            turn: TurnMark::default(),
            commands: None,
            queued: Vec::new(),
            sending: false,
            applied: BTreeMap::new(),
            seen_ids: BTreeSet::new(),
            saved: BTreeSet::new(),
            connected: true,
            drafts: BTreeMap::new(),
            for_session: BTreeSet::new(),
            approved_for_session: BTreeSet::new(),
            attach: Attach::Attached,
            attach_at: 0,
            lapsed: BTreeSet::new(),
            lapsed_through: 0,
            cut: None,
            cut_calls: BTreeSet::new(),
        }
    }

    /// Questions waiting for the operator, oldest first.
    pub fn pending_questions(&self) -> Vec<&QuestionRequest> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Question { request, state: QuestionState::Pending, .. } => Some(request),
                _ => None,
            })
            .collect()
    }

    /// The operator's replies so far to request `request_id`.
    pub fn drafts(&self, request_id: &str) -> &[QuestionReply] {
        self.drafts.get(request_id).map(Vec::as_slice).unwrap_or_default()
    }

    /// Keep the operator's reply to one question of a waiting request (a
    /// second reply to the same question replaces the first). Returns every
    /// reply, in the request's order, once each question has one; `None`
    /// while some are still unanswered or the request is not waiting.
    pub fn draft(&mut self, request_id: &str, reply: QuestionReply) -> Option<Vec<QuestionReply>> {
        let request = self.pending_questions().into_iter().find(|r| r.request_id == request_id)?.clone();
        if !request.questions.iter().any(|q| q.id == reply.question_id) {
            return None;
        }
        let drafts = self.drafts.entry(request_id.to_string()).or_default();
        drafts.retain(|r| r.question_id != reply.question_id);
        drafts.push(reply);
        let all: Option<Vec<QuestionReply>> = request
            .questions
            .iter()
            .map(|q| drafts.iter().find(|r| r.question_id == q.id).cloned())
            .collect();
        all
    }

    /// Permission requests waiting for the operator, oldest first.
    pub fn pending(&self) -> Vec<&PermissionRequest> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Permission { request, state: PermissionState::Pending } => Some(request),
                _ => None,
            })
            .collect()
    }

    /// Something radar says in the transcript (resumed, a failed call), not the
    /// agent.
    pub fn note(&mut self, text: impl Into<String>) {
        self.push(Item::Notice { text: text.into() });
    }

    /// A failure of radar's own call to the provider (send refused, resume
    /// failed): shown in the transcript, and the session is in error.
    pub fn fail(&mut self, message: impl Into<String>) {
        self.state = SessionState::Error;
        self.sending = false;
        self.turn.end(false);
        self.push(Item::Error { message: message.into(), retryable: true });
    }

    /// The connection to the provider was lost or came back.
    pub fn set_connected(&mut self, connected: bool) {
        self.connected = connected;
        if !connected {
            self.state = SessionState::Disconnected;
        } else if self.state == SessionState::Disconnected {
            self.state = self.derived_state();
        }
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    /// A turn runs, waits for the operator, or is being started by radar.
    pub fn busy(&self) -> bool {
        self.sending || matches!(self.state, SessionState::Running | SessionState::WaitingApproval)
    }

    /// radar is sending a message: busy until the provider shows a turn.
    pub fn sending(&mut self) {
        self.sending = true;
    }

    /// Keep a message the operator sent while the session was busy.
    pub fn queue(&mut self, text: String, holder: Holder) {
        self.queued.push(Queued { text, holder });
    }

    /// The message radar holds that is due now: the oldest, once the session is
    /// free (connected, started, no turn). It stays shown, now the
    /// provider's, until the provider shows it; radar is sending it.
    pub fn due(&mut self) -> Option<String> {
        if self.busy() || !self.connected || !self.attached() || self.meta.provider_session_id.as_str().is_empty() {
            return None;
        }
        let q = self.queued.iter_mut().find(|q| q.holder == Holder::Radar)?;
        q.holder = Holder::Provider;
        self.sending = true;
        Some(q.text.clone())
    }

    /// Sending `text` failed: it is no longer on its way.
    pub fn send_failed(&mut self, text: &str) {
        if let Some(i) = self.queued.iter().position(|q| q.holder == Holder::Provider && q.text == text) {
            self.queued.remove(i);
        }
        self.sending = false;
    }

    /// What a newly applied event says of the messages on their way. Only
    /// for an event as it arrives: a rebuild replays old user messages,
    /// which must not take a queued message's place.
    fn settle(&mut self, event: &Event) {
        match &event.kind {
            // The operator's message, as the provider shows it (a sub-agent's prompt is not theirs).
            EventKind::UserMessage { text } if event.agent.is_none() => {
                self.sending = false;
                // By its text: the echo of a message sent before this one may still be on its way.
                if let Some(i) = self.queued.iter().position(|q| q.holder == Holder::Provider && q.text.trim() == text.trim()) {
                    self.queued.remove(i);
                }
            }
            EventKind::TurnStarted | EventKind::TurnEnded | EventKind::Aborted | EventKind::Error { .. } => self.sending = false,
            // The provider is idle: nothing it was handed is still waiting in it.
            EventKind::Idle => {
                self.sending = false;
                self.queued.retain(|q| q.holder == Holder::Radar);
            }
            _ => {}
        }
    }

    pub fn ingest(&mut self, event: Event) -> Ingested {
        match &event.provider_event_id {
            Some(id) if self.seen_ids.contains(id) => return Ingested::Duplicate,
            None if self.applied.contains_key(&event.sequence) => return Ingested::Duplicate,
            _ => {}
        }
        // A sequence already applied under another id is a re-delivery too.
        if let Some(prev) = self.applied.get(&event.sequence)
            && prev.kind == event.kind
        {
            return Ingested::Duplicate;
        }
        if let EventKind::AssistantDelta { message_id, .. }
        | EventKind::ReasoningDelta { message_id, .. } = &event.kind
            && self.saved.contains(message_id)
        {
            return Ingested::Dropped;
        }
        if let Some(id) = &event.provider_event_id {
            self.seen_ids.insert(id.clone());
        }
        if event.durability == Durability::Persisted
            && let Some(id) = &event.provider_event_id
        {
            let newest = self
                .applied
                .values()
                .filter(|e| e.durability == Durability::Persisted)
                .map(|e| e.sequence)
                .max()
                .unwrap_or(0);
            if event.sequence >= newest {
                self.meta.cursor = Some(id.clone());
            }
        }
        let late = self.applied.range(event.sequence + 1..).next().is_some();
        self.settle(&event);
        self.applied.insert(event.sequence, event);
        if late {
            // Out of order: rebuild in sequence order. Rare, and bounded by
            // the timeline cap.
            self.rebuild();
        } else {
            let event = self.applied.values().next_back().cloned().expect("just inserted");
            self.apply(&event);
        }
        Ingested::Applied
    }

    fn rebuild(&mut self) {
        let events: Vec<Event> = self.applied.values().cloned().collect();
        self.items.clear();
        self.saved.clear();
        self.usage = (0, 0);
        self.turn = TurnMark::default();
        self.state = SessionState::Idle;
        for e in &events {
            self.apply(e);
        }
        self.end_cut_turn();
    }

    fn apply(&mut self, event: &Event) {
        let used = self.usage.0 + self.usage.1;
        match &event.kind {
            EventKind::UserMessage { .. } | EventKind::TurnStarted | EventKind::AssistantDelta { .. } | EventKind::ReasoningDelta { .. } => {
                self.turn.start(used)
            }
            EventKind::ToolStarted { .. } | EventKind::PermissionRequested { .. } | EventKind::QuestionAsked { .. } if self.turn.soft => {
                self.turn.start(used)
            }
            EventKind::TurnEnded => self.turn.end(true),
            EventKind::Idle | EventKind::Aborted | EventKind::Error { .. } => self.turn.end(false),
            EventKind::Usage { .. } => self.turn.reported = true,
            _ => {}
        }
        if event.durability == Durability::Ephemeral && self.turn.seq > 0 {
            self.turn.live = true;
        }
        match &event.kind {
            EventKind::UserMessage { text } => self.push(Item::User { text: text.clone() }),
            EventKind::TurnStarted => self.state = SessionState::Running,
            EventKind::TurnEnded | EventKind::Idle => {
                if self.state != SessionState::Stopped && self.state != SessionState::Error {
                    self.state = SessionState::Idle;
                }
            }
            EventKind::AssistantDelta { message_id, text } => {
                self.state = SessionState::Running;
                self.append(message_id, text, false, false);
            }
            EventKind::AssistantMessage { message_id, text } => {
                self.saved.insert(message_id.clone());
                self.append(message_id, text, true, false);
            }
            EventKind::ReasoningDelta { message_id, text } => self.append(message_id, text, false, true),
            EventKind::Reasoning { message_id, text } => {
                self.saved.insert(message_id.clone());
                self.append(message_id, text, true, true);
            }
            EventKind::ToolStarted { call_id, name, summary, diff } => {
                if !self.items.iter().any(|i| matches!(i, Item::Tool { call_id: c, .. } if c == call_id)) {
                    self.push(Item::Tool {
                        call_id: call_id.clone(),
                        name: name.clone(),
                        summary: summary.clone(),
                        state: ToolState::Running,
                        result: None,
                        diff: diff.clone(),
                    });
                }
            }
            EventKind::ToolCompleted { call_id, ok, summary } => {
                for item in &mut self.items {
                    if let Item::Tool { call_id: c, state, result, .. } = item
                        && c == call_id
                    {
                        *state = if *ok { ToolState::Ok } else { ToolState::Failed };
                        *result = Some(summary.clone());
                    }
                }
            }
            EventKind::PermissionRequested { request } => {
                let old = self.lapsed_ask(&request.request_id, event.sequence);
                let known = self.items.iter_mut().find_map(|i| match i {
                    Item::Permission { request: r, state } if r.request_id == request.request_id => Some(state),
                    _ => None,
                });
                match known {
                    None if old => self.push(Item::Permission { request: request.clone(), state: PermissionState::Lapsed }),
                    None => {
                        self.push(Item::Permission { request: request.clone(), state: PermissionState::Pending });
                        self.state = SessionState::WaitingApproval;
                    }
                    // Asked again after the resume, under the same id: it waits now, and can be answered.
                    Some(state) if !old && *state == PermissionState::Lapsed => {
                        *state = PermissionState::Pending;
                        self.state = SessionState::WaitingApproval;
                    }
                    Some(_) => {}
                }
            }
            EventKind::PermissionResolved { request_id, approved } => {
                for item in &mut self.items {
                    if let Item::Permission { request, state } = item
                        && request.request_id == *request_id
                    {
                        *state = if *approved { PermissionState::Approved } else { PermissionState::Denied };
                    }
                }
                if self.pending().is_empty() && self.pending_questions().is_empty() && self.state == SessionState::WaitingApproval {
                    self.state = SessionState::Running;
                }
            }
            EventKind::QuestionAsked { request } => {
                let old = self.lapsed_ask(&request.request_id, event.sequence);
                let known = self.items.iter_mut().find_map(|i| match i {
                    Item::Question { request: r, state, .. } if r.request_id == request.request_id => Some(state),
                    _ => None,
                });
                match known {
                    None if old => {
                        self.push(Item::Question { request: request.clone(), state: QuestionState::Lapsed, replies: Vec::new() })
                    }
                    None => {
                        self.push(Item::Question { request: request.clone(), state: QuestionState::Pending, replies: Vec::new() });
                        self.state = SessionState::WaitingApproval;
                    }
                    Some(state) if !old && *state == QuestionState::Lapsed => {
                        *state = QuestionState::Pending;
                        self.state = SessionState::WaitingApproval;
                    }
                    Some(_) => {}
                }
            }
            EventKind::QuestionResolved { request_id, replies } => {
                let mine = self.drafts.get(request_id).cloned().unwrap_or_default();
                for item in &mut self.items {
                    if let Item::Question { request, state, replies: shown } = item
                        && request.request_id == *request_id
                    {
                        match replies {
                            Some(given) => {
                                *state = QuestionState::Answered;
                                *shown = if given.is_empty() { mine.clone() } else { given.clone() };
                            }
                            None => *state = QuestionState::Declined,
                        }
                    }
                }
                if self.pending().is_empty() && self.pending_questions().is_empty() && self.state == SessionState::WaitingApproval {
                    self.state = SessionState::Running;
                }
            }
            EventKind::Aborted => {
                self.state = SessionState::Stopped;
                // The transcript says so; a stopped turn must not look like a quiet one.
                self.push(Item::Notice { text: "stopped".into() });
            }
            EventKind::Error { message, retryable } => {
                self.state = SessionState::Error;
                self.push(Item::Error { message: message.clone(), retryable: *retryable });
            }
            EventKind::Notice { text } => self.push(Item::Notice { text: text.clone() }),
            EventKind::Usage { input_tokens, output_tokens } => {
                self.usage.0 += input_tokens.unwrap_or(0);
                self.usage.1 += output_tokens.unwrap_or(0);
            }
            EventKind::ModelChanged { model } => self.meta.model = Some(model.clone()),
            EventKind::Other { .. } => {}
        }
    }

    /// Add text to the assistant (or reasoning) item for `message_id`. A
    /// saved message replaces the streamed text; a delta extends it.
    fn append(&mut self, message_id: &str, text: &str, saved: bool, reasoning: bool) {
        for item in &mut self.items {
            match item {
                Item::Assistant { message_id: m, text: t, saved: s } if !reasoning && m == message_id => {
                    if saved {
                        *t = text.to_string();
                        *s = true;
                    } else {
                        t.push_str(text);
                    }
                    return;
                }
                Item::Reasoning { message_id: m, text: t, saved: s } if reasoning && m == message_id => {
                    if saved {
                        *t = text.to_string();
                        *s = true;
                    } else {
                        t.push_str(text);
                    }
                    return;
                }
                _ => {}
            }
        }
        let (message_id, text) = (message_id.to_string(), text.to_string());
        self.push(if reasoning {
            Item::Reasoning { message_id, text, saved }
        } else {
            Item::Assistant { message_id, text, saved }
        });
    }

    fn push(&mut self, item: Item) {
        self.items.push(item);
        if self.items.len() > TIMELINE_CAP {
            self.items.remove(0);
            self.truncated = true;
        }
    }

    fn derived_state(&self) -> SessionState {
        if !self.pending().is_empty() || !self.pending_questions().is_empty() {
            SessionState::WaitingApproval
        } else {
            SessionState::Idle
        }
    }
}

/// Every session radar knows. Persists only [`SessionMeta`].
#[derive(Default)]
pub struct Store {
    sessions: BTreeMap<RadarSessionId, Session>,
}

impl Store {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, meta: SessionMeta) -> &mut Session {
        let id = meta.radar_session_id.clone();
        self.sessions.entry(id).or_insert_with(|| Session::new(meta))
    }

    pub fn get(&self, id: &RadarSessionId) -> Option<&Session> {
        self.sessions.get(id)
    }

    pub fn get_mut(&mut self, id: &RadarSessionId) -> Option<&mut Session> {
        self.sessions.get_mut(id)
    }

    pub fn remove(&mut self, id: &RadarSessionId) -> Option<Session> {
        self.sessions.remove(id)
    }

    pub fn sessions(&self) -> impl Iterator<Item = &Session> {
        self.sessions.values()
    }

    /// Route an adapter's event to its session; `None` when radar has no such
    /// session (deleted meanwhile).
    pub fn ingest(&mut self, event: Event) -> Option<Ingested> {
        let session = self.sessions.get_mut(&event.radar_session_id)?;
        Some(session.ingest(event))
    }

    /// The connection of every session of `provider`.
    pub fn set_connected(&mut self, provider: &ProviderId, connected: bool) {
        for s in self.sessions.values_mut().filter(|s| &s.meta.provider == provider) {
            s.set_connected(connected);
        }
    }

    /// The metadata to persist, as JSON.
    pub fn snapshot(&self) -> serde_json::Value {
        let metas: Vec<&SessionMeta> = self.sessions.values().map(|s| &s.meta).collect();
        serde_json::to_value(metas).unwrap_or_default()
    }

    /// Load persisted metadata. The cursor is cleared: the timeline is
    /// gone, so the next resume replays the provider's whole history. Each
    /// session is detached: the provider's process ended with the last run.
    pub fn restore(&mut self, value: serde_json::Value) {
        let metas: Vec<SessionMeta> = serde_json::from_value(value).unwrap_or_default();
        for mut meta in metas {
            meta.cursor = None;
            self.insert(meta).attach = Attach::Detached;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{PermissionKind, PermissionTarget};

    fn meta() -> SessionMeta {
        SessionMeta {
            radar_session_id: RadarSessionId::new("s1"),
            provider: ProviderId::new("fake"),
            provider_session_id: ProviderSessionId::new("p1"),
            cwd: "/tmp".into(),
            title: "t".into(),
            model: None,
            created_at: 1,
            cursor: None,
            agent: None,
            review: false,
        }
    }

    #[test]
    fn sessions_saved_under_the_old_name_are_restored() {
        let mut saved = serde_json::to_value(vec![meta()]).unwrap();
        let id = saved[0].as_object_mut().unwrap().remove("radar_session_id").unwrap();
        saved[0]["radar_session_id"] = id;
        let mut store = Store::default();
        store.restore(saved);
        assert_eq!(store.snapshot()[0]["radar_session_id"], "s1");
    }

    fn shell(id: &str) -> PermissionRequest {
        PermissionRequest {
            request_id: id.into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: "ls".into(), segments: vec!["ls".into()] },
            tool_call_id: None,
            can_offer_session_approval: false,
        }
    }

    #[test]
    fn what_waits_when_the_history_ends_lapses_and_stays_lapsed_through_a_rebuild() {
        let mut s = Session::new(meta());
        s.ingest(user(1, "e1", "run it"));
        s.ingest(ev(2, Some("e2"), Durability::Persisted, EventKind::TurnStarted));
        let call = EventKind::ToolStarted { call_id: "c1".into(), name: "shell".into(), summary: "ls".into(), diff: None };
        s.ingest(ev(3, Some("e3c"), Durability::Persisted, call));
        s.ingest(ev(4, Some("e4"), Durability::Persisted, EventKind::PermissionRequested { request: shell("r1") }));
        assert_eq!(s.pending().len(), 1);
        let found = s.lapse();
        let ended = |s: &Session| {
            s.items.iter().any(|i| matches!(i, Item::Tool { state: ToolState::Failed, result: Some(r), .. } if r == "radar closed before it finished"))
        };
        assert!(ended(&s), "the call it waited to run never finishes: {:?}", s.items);
        assert_eq!(found, Lapsed { requests: 1, questions: 0, turn: true });
        assert!(found.words().unwrap().starts_with("the turn that was waiting was ended when radar closed"));
        assert!(s.pending().is_empty() && !s.busy() && !s.turn.open, "{:?} {:?}", s.state, s.turn);
        assert!(matches!(s.items.last(), Some(Item::Permission { state: PermissionState::Lapsed, .. })));
        // A late event rebuilds the timeline: the request stays lapsed and the turn over.
        s.ingest(ev(0, Some("e0"), Durability::Persisted, EventKind::Notice { text: "late".into() }));
        assert!(s.pending().is_empty() && !s.busy(), "{:?}", s.items);
        assert!(ended(&s), "a rebuild keeps it ended: {:?}", s.items);
        assert!(s.items.iter().any(|i| matches!(i, Item::Permission { state: PermissionState::Lapsed, .. })));
        // Nothing waits a second time round.
        assert_eq!(s.lapse(), Lapsed::default());
        // A history that ended between turns has nothing to lapse.
        let mut quiet = Session::new(meta());
        quiet.ingest(user(1, "e1", "hi"));
        quiet.ingest(ev(2, Some("e2"), Durability::Persisted, EventKind::TurnEnded));
        assert!(!quiet.lapse().any());
        assert_eq!(quiet.lapse().words(), None);
    }

    #[test]
    fn a_request_the_provider_asks_again_after_the_resume_waits_even_under_the_same_id() {
        let mut s = Session::new(meta());
        s.ingest(user(1, "e1", "run it"));
        s.ingest(ev(2, Some("e2"), Durability::Persisted, EventKind::PermissionRequested { request: shell("r1") }));
        s.lapse();
        assert!(s.pending().is_empty());
        // The resumed provider asks again: a new event, the same request id.
        s.ingest(ev(3, Some("e3"), Durability::Persisted, EventKind::PermissionRequested { request: shell("r1") }));
        assert_eq!(s.pending().len(), 1, "{:?}", s.items);
        assert_eq!(s.state, SessionState::WaitingApproval);
        assert_eq!(s.items.iter().filter(|i| matches!(i, Item::Permission { .. })).count(), 1, "one row for it");
        // A rebuild keeps it waiting (the first ask stays the lapsed one, the second revives it).
        s.ingest(ev(0, Some("e0"), Durability::Persisted, EventKind::Notice { text: "late".into() }));
        assert_eq!(s.pending().len(), 1, "{:?}", s.items);
        assert_eq!(s.state, SessionState::WaitingApproval);
        // A new id asked after the resume is simply a request that waits.
        s.ingest(ev(4, Some("e4"), Durability::Persisted, EventKind::PermissionRequested { request: shell("r2") }));
        assert_eq!(s.pending().len(), 2);
    }

    #[test]
    fn a_restored_session_is_not_attached_until_resumed() {
        let mut store = Store::new();
        store.insert(meta());
        assert!(store.get(&RadarSessionId::new("s1")).unwrap().attached());
        let mut back = Store::new();
        back.restore(store.snapshot());
        let s = back.get_mut(&RadarSessionId::new("s1")).unwrap();
        assert_eq!(s.attach, Attach::Detached);
        s.queue("later".into(), Holder::Radar);
        assert_eq!(s.due(), None, "nothing goes to a provider radar is not attached to");
        s.attach = Attach::Attached;
        assert_eq!(s.due().as_deref(), Some("later"));
    }

    fn ev(seq: u64, id: Option<&str>, durability: Durability, kind: EventKind) -> Event {
        Event {
            radar_session_id: RadarSessionId::new("s1"),
            provider: ProviderId::new("fake"),
            provider_session_id: ProviderSessionId::new("p1"),
            provider_event_id: id.map(ProviderEventId::new),
            sequence: seq,
            observed_at: seq as i64,
            kind,
            durability,
            agent: None,
            parent: None,
            payload: serde_json::Value::Null,
        }
    }

    fn delta(seq: u64, id: &str, text: &str) -> Event {
        ev(seq, Some(id), Durability::Ephemeral, EventKind::AssistantDelta { message_id: "m".into(), text: text.into() })
    }

    fn message(seq: u64, id: &str, text: &str) -> Event {
        ev(seq, Some(id), Durability::Persisted, EventKind::AssistantMessage { message_id: "m".into(), text: text.into() })
    }

    #[test]
    fn deltas_show_unsaved_until_the_persisted_message_replaces_them() {
        let mut s = Session::new(meta());
        s.ingest(delta(1, "e1", "Hel"));
        s.ingest(delta(2, "e2", "lo"));
        assert_eq!(s.items, vec![Item::Assistant { message_id: "m".into(), text: "Hello".into(), saved: false }]);
        s.ingest(message(3, "e3", "Hello there"));
        assert_eq!(s.items, vec![Item::Assistant { message_id: "m".into(), text: "Hello there".into(), saved: true }]);
        assert_eq!(s.ingest(delta(4, "e4", "late")), Ingested::Dropped);
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn the_same_provider_event_id_applies_once() {
        let mut s = Session::new(meta());
        assert_eq!(s.ingest(message(1, "e1", "a")), Ingested::Applied);
        assert_eq!(s.ingest(message(2, "e1", "a")), Ingested::Duplicate);
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn an_event_without_an_id_is_deduped_by_its_sequence() {
        let mut s = Session::new(meta());
        let note = |seq| ev(seq, None, Durability::Persisted, EventKind::Notice { text: "n".into() });
        assert_eq!(s.ingest(note(1)), Ingested::Applied);
        assert_eq!(s.ingest(note(1)), Ingested::Duplicate);
    }

    #[test]
    fn events_apply_in_sequence_order_when_they_arrive_out_of_order() {
        let mut s = Session::new(meta());
        let user = |seq, id, t: &str| ev(seq, Some(id), Durability::Persisted, EventKind::UserMessage { text: t.into() });
        s.ingest(user(2, "e2", "second"));
        s.ingest(user(1, "e1", "first"));
        assert_eq!(
            s.items,
            vec![Item::User { text: "first".into() }, Item::User { text: "second".into() }]
        );
    }

    #[test]
    fn a_permission_request_delivered_twice_is_one_and_waits() {
        let mut s = Session::new(meta());
        let req = PermissionRequest {
            request_id: "r1".into(),
            kind: PermissionKind::Shell,
            target: PermissionTarget::Command { command: "ls".into(), segments: vec!["ls".into()] },
            tool_call_id: None,
            can_offer_session_approval: false,
        };
        let asked = |seq, id| ev(seq, Some(id), Durability::Persisted, EventKind::PermissionRequested { request: req.clone() });
        s.ingest(asked(1, "e1"));
        s.ingest(asked(2, "e2"));
        assert_eq!(s.pending().len(), 1);
        assert_eq!(s.state, SessionState::WaitingApproval);
        s.ingest(ev(3, Some("e3"), Durability::Persisted, EventKind::PermissionResolved { request_id: "r1".into(), approved: false }));
        assert!(s.pending().is_empty());
        assert!(matches!(s.items[0], Item::Permission { state: PermissionState::Denied, .. }));
    }

    #[test]
    fn a_question_of_two_parts_delivered_twice_is_one_and_goes_out_when_both_are_answered() {
        use crate::event::{Question, QuestionChoice};
        let q = |id: &str| Question {
            id: id.into(),
            header: None,
            text: format!("{id}?"),
            choices: vec![QuestionChoice { label: "yes".into(), description: None }, QuestionChoice { label: "no".into(), description: None }],
            free_text: false,
            multiple: false,
            secret: false,
        };
        let req = QuestionRequest { request_id: "q1".into(), questions: vec![q("a"), q("b")], tool_call_id: None };
        let asked = |seq, id| ev(seq, Some(id), Durability::Persisted, EventKind::QuestionAsked { request: req.clone() });
        let mut s = Session::new(meta());
        s.ingest(asked(1, "e1"));
        s.ingest(asked(2, "e2"));
        assert_eq!(s.pending_questions().len(), 1);
        assert_eq!(s.state, SessionState::WaitingApproval);
        let reply = |q: &str, v: &str| QuestionReply { question_id: q.into(), values: vec![v.into()], own_words: false };
        assert_eq!(s.draft("q1", reply("a", "no")), None, "b is still unanswered");
        assert_eq!(s.draft("q1", reply("zz", "no")), None, "a question the request does not hold");
        assert_eq!(s.draft("q1", reply("a", "yes")), None, "a second reply replaces the first");
        let all = s.draft("q1", reply("b", "no")).expect("both answered");
        assert_eq!(all, vec![reply("a", "yes"), reply("b", "no")]);
        // The provider says it is settled without saying what: the operator's replies are shown.
        s.ingest(ev(3, Some("e3"), Durability::Persisted, EventKind::QuestionResolved { request_id: "q1".into(), replies: Some(vec![]) }));
        assert!(s.pending_questions().is_empty());
        assert_eq!(s.state, SessionState::Running);
        assert!(matches!(&s.items[0], Item::Question { state: QuestionState::Answered, replies, .. } if *replies == all));
    }

    #[test]
    fn an_aborted_turn_leaves_a_stopped_notice_in_the_transcript() {
        let mut s = Session::new(meta());
        s.ingest(ev(1, Some("e1"), Durability::Persisted, EventKind::Aborted));
        assert_eq!(s.state, SessionState::Stopped);
        assert_eq!(s.items, vec![Item::Notice { text: "stopped".into() }]);
        // The same event delivered again changes nothing.
        s.ingest(ev(2, Some("e1"), Durability::Persisted, EventKind::Aborted));
        assert_eq!(s.items.len(), 1);
    }

    #[test]
    fn the_cursor_follows_persisted_events_only() {
        let mut s = Session::new(meta());
        s.ingest(message(1, "e1", "a"));
        s.ingest(delta(2, "e2", "b"));
        assert_eq!(s.meta.cursor, Some(ProviderEventId::new("e1")));
    }

    #[test]
    fn a_restored_store_keeps_identity_but_not_the_cursor() {
        let mut store = Store::new();
        let mut m = meta();
        m.cursor = Some(ProviderEventId::new("e9"));
        store.insert(m);
        let mut back = Store::new();
        back.restore(store.snapshot());
        let s = back.get(&RadarSessionId::new("s1")).expect("kept");
        assert_eq!(s.meta.provider_session_id, ProviderSessionId::new("p1"));
        assert_eq!(s.meta.cursor, None);
    }

    fn user(seq: u64, id: &str, text: &str) -> Event {
        ev(seq, Some(id), Durability::Persisted, EventKind::UserMessage { text: text.into() })
    }

    #[test]
    fn a_message_the_provider_queues_shows_until_it_is_the_operators_and_an_idle_provider_holds_none() {
        let mut s = Session::new(meta());
        s.ingest(ev(1, Some("e1"), Durability::Persisted, EventKind::TurnStarted));
        assert!(s.busy());
        s.queue("and?".into(), Holder::Provider);
        s.queue("later".into(), Holder::Provider);
        // A sub-agent's prompt is not the operator's message.
        let mut sub = user(2, "e2", "and?");
        sub.agent = Some(crate::event::AgentRef { id: "a1".into(), name: None });
        s.ingest(sub);
        assert_eq!(s.queued.len(), 2);
        s.ingest(user(3, "e3", " and? "));
        assert_eq!(s.queued, vec![Queued { text: "later".into(), holder: Holder::Provider }]);
        // A late event rebuilds the timeline; its old user messages do not take a queued one's place.
        s.queue("and?".into(), Holder::Provider);
        s.ingest(ev(5, Some("e5"), Durability::Persisted, EventKind::Notice { text: "n".into() }));
        s.ingest(ev(4, Some("e4"), Durability::Persisted, EventKind::Notice { text: "late".into() }));
        assert_eq!(s.queued.len(), 2);
        s.ingest(ev(6, Some("e6"), Durability::Ephemeral, EventKind::Idle));
        assert!(s.queued.is_empty(), "an idle provider is running nothing it was handed");
    }

    #[test]
    fn a_message_radar_keeps_is_due_when_the_turn_ends_one_at_a_time() {
        let mut s = Session::new(meta());
        s.ingest(ev(1, Some("e1"), Durability::Persisted, EventKind::TurnStarted));
        s.queue("a".into(), Holder::Radar);
        s.queue("b".into(), Holder::Radar);
        assert_eq!(s.due(), None, "the turn runs");
        s.ingest(ev(2, Some("e2"), Durability::Persisted, EventKind::TurnEnded));
        assert_eq!(s.due().as_deref(), Some("a"));
        assert!(s.busy(), "radar is sending it");
        assert_eq!(s.due(), None, "one at a time");
        assert_eq!(s.queued[0], Queued { text: "a".into(), holder: Holder::Provider });
        s.ingest(user(3, "e3", "a"));
        s.ingest(ev(4, Some("e4"), Durability::Persisted, EventKind::TurnStarted));
        s.ingest(ev(5, Some("e5"), Durability::Persisted, EventKind::TurnEnded));
        assert_eq!(s.due().as_deref(), Some("b"));
        // The provider refused it: no longer on its way, and nothing is due.
        s.send_failed("b");
        assert!(s.queued.is_empty() && !s.busy());
    }

    #[test]
    fn the_timeline_is_bounded() {
        let mut s = Session::new(meta());
        for i in 0..(TIMELINE_CAP as u64 + 5) {
            s.ingest(ev(i + 1, Some(&format!("e{i}")), Durability::Persisted, EventKind::Notice { text: "n".into() }));
        }
        assert_eq!(s.items.len(), TIMELINE_CAP);
        assert!(s.truncated);
    }
}
