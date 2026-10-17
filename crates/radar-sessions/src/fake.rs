//! A deterministic fake provider: scripted turns, no process, no threads,
//! no wall clock. Every action runs to completion inside the call that makes
//! it, so a test needs no sleeps and no waiting.
//!
//! [`FakeProvider`] is the provider's side (its sessions and persisted history
//! survive an adapter being dropped, like a real CLI's); [`FakeAdapter`] is
//! one radar-side handle on it; [`FakeHarness`] lets the contract kit drive it.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::mpsc;

use crate::adapter::*;
use crate::contract::{Harness, Scenario};
use crate::event::*;

/// One thing a turn does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    /// Stream the text word by word (ephemeral), then the persisted message.
    Say(String),
    /// The persisted message alone, with nothing streamed before it.
    Said(String),
    /// Start a shell tool and ask permission to run it; the turn waits for
    /// the operator.
    Shell(String),
    /// Ask the operator a question with these choices (free text allowed)
    /// and wait; the answer is said back ("You chose: …"), a refusal ends
    /// the question with "No answer".
    Ask { question: String, choices: Vec<String> },
    /// Stream reasoning word by word (ephemeral), then the persisted text.
    Think(String),
    /// A tool call the agent may make without asking (a read, a search, an
    /// edit the policy allows): started (`diff` with it, for an edit), then
    /// finished with `result`.
    Tool { name: String, summary: String, diff: Option<String>, result: String, ok: bool },
    /// Keep working until stopped or released.
    Hang,
    /// Fail the turn.
    Fail(String),
    /// Deliver the last persisted event a second time.
    Redeliver,
    /// Deliver the next persisted event after the one that follows it.
    Reorder,
}

/// The steps of one turn.
pub type Script = Vec<Step>;

/// Exact adapter invocations, in order. Available to owner and hosted tests;
/// these are requests, not claims that a provider turn or command succeeded.
#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FakeCall {
    Subscribe,
    Create(CreateSession),
    Send { session_id: RadarSessionId, input: SendInput },
    Permission { session_id: RadarSessionId, answer: PermissionAnswer },
    Resume(ResumeSession),
    Stop { session_id: RadarSessionId },
    Release,
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
struct Inspection {
    calls: Vec<FakeCall>,
    strict_scripts: bool,
    consumed_scripts: usize,
    failure: Option<String>,
}

/// What `fill` and `stream` say: a paragraph long enough to wrap.
const PARAGRAPH: &str = "The change moves the transcript out of the snapshot's hot path, so a streamed word costs the same \
    whether the session holds ten messages or five hundred; the renderer keeps what it already wrapped and draws only \
    the rows that show, and the host compares versions instead of writing the whole view out to find what changed.";

/// What a turn does when no script is queued: it reacts to the prompt, so a
/// person can drive the fake in the dock. A prompt starting with "ask" asks
/// a question, "run" asks to run a command, "long" keeps working until
/// stopped, "demo" reasons, reads, searches, edits and answers in markdown
/// (what the transcript draws, all at once), "fill N" answers N paragraphs at
/// once, "stream N" streams an answer of N words, anything else is echoed.
pub fn auto_script(prompt: &str) -> Script {
    let lower = prompt.to_lowercase();
    // `fill N`: N answers of a paragraph each (a long transcript); `stream N`: one answer of N words.
    let count = |word: &str| lower.trim().strip_prefix(word).and_then(|n| n.trim().parse::<usize>().ok()).map(|n| n.min(5000));
    if let Some(n) = count("fill") {
        return (1..=n).map(|i| Step::Said(format!("Answer {i}: {}", PARAGRAPH))).collect();
    }
    if let Some(n) = count("stream") {
        let words: Vec<&str> = PARAGRAPH.split(' ').collect();
        return vec![Step::Say((0..n).map(|i| words[i % words.len()]).collect::<Vec<_>>().join(" "))];
    }
    if lower.trim_start().starts_with("demo") {
        demo_script()
    } else if lower.trim_start().starts_with("ask") {
        vec![
            Step::Ask { question: "Which colour should it be?".into(), choices: vec!["red".into(), "blue".into()] },
            Step::Say("Thanks.".into()),
        ]
    } else if lower.contains("run") || lower.contains("tool") {
        vec![Step::Say("I will list the directory.".into()), Step::Shell("ls -la".into()), Step::Say("The command finished.".into())]
    } else if lower.contains("long") || lower.contains("hang") {
        vec![Step::Say("Working on it".into()), Step::Hang, Step::Say("Finished".into())]
    } else {
        vec![Step::Say(format!("You said: {prompt}"))]
    }
}

/// A review turn as a real agent runs one: reasoning, five reads, a search,
/// a command, an edit, and an answer in markdown (headings, bold, inline and
/// fenced code, nested lists, a quote, a link, a table).
pub fn demo_script() -> Script {
    let read = |path: &str, lines: usize| Step::Tool {
        name: "Read".into(),
        summary: path.into(),
        diff: None,
        result: format!("{lines} lines"),
        ok: true,
    };
    vec![
        Step::Think("The question is about the dock's transcript. Read the rows first, then the host's view of it.".into()),
        read("ink/src/dock/rows.ts", 309),
        read("ink/src/dock/Dock.tsx", 164),
        read("ink/src/dock/composer.ts", 351),
        read("crates/radar-core/src/dock.rs", 410),
        read("crates/radar-sessions/src/dock.rs", 516),
        Step::Tool { name: "Grep".into(), summary: "fn dock_view".into(), diff: None, result: "crates/radar-sessions/src/dock.rs:215".into(), ok: true },
        Step::Tool {
            name: "Bash".into(),
            summary: "cargo test -p radar-sessions dock".into(),
            diff: None,
            result: "test result: ok. 12 passed; 0 failed".into(),
            ok: true,
        },
        Step::Tool {
            name: "Edit".into(),
            summary: "crates/radar-sessions/src/dock.rs".into(),
            diff: Some(" fn lines(text: &str) -> Vec<String> {\n-    text.lines().map(str::to_string).collect()\n+    text.lines().map(String::from).collect()\n }".into()),
            result: "updated".into(),
            ok: true,
        },
        Step::Say(DEMO_ANSWER.into()),
    ]
}

const DEMO_ANSWER: &str = "## Review\n\n**Verdict**: Request changes. The branch is behind `origin/main` by three commits.\n\n- **Findings**:\n  - The tool rows take *two lines* each; one is enough.\n  - Reads in a row should fold together.\n- **Fixed** in this turn: `lines` uses `String::from`.\n\n1. Rebase on `origin/main`.\n2. Run the checks again.\n\n> The host owns all state; Ink only draws it.\n\n```rust\nfn lines(text: &str) -> Vec<String> {\n    text.lines().map(String::from).collect() // one per line\n}\n```\n\n| Check | Result |\n|---|---|\n| typecheck | ok |\n| tests | 12 passed |\n\nSee [the plan](https://example.test/agent-dock) for the dock's rules.";

#[derive(Clone, Debug, PartialEq, Eq)]
enum Blocked {
    Permission { request_id: String, call_id: String, command: String },
    Question { request_id: String },
    Hang,
}

struct Turn {
    steps: VecDeque<Step>,
    blocked: Option<Blocked>,
}

struct Sess {
    radar: RadarSessionId,
    provider_session_id: ProviderSessionId,
    cwd: String,
    seq: u64,
    next_event: u64,
    next_message: u64,
    /// What the provider keeps: persisted events only.
    history: Vec<Event>,
    turn: Option<Turn>,
    answered: BTreeSet<String>,
    /// Held back by `Step::Reorder`.
    held: Option<Event>,
    reorder_next: bool,
    /// The command a turn waited to run when the provider's process ended
    /// ([`FakeProvider::restart`]); [`FakeProvider::reask`] asks it again.
    interrupted: Option<String>,
}

impl Sess {
    fn new(radar: RadarSessionId, provider_session_id: ProviderSessionId, cwd: String) -> Self {
        Sess {
            radar,
            provider_session_id,
            cwd,
            seq: 0,
            next_event: 0,
            next_message: 0,
            history: Vec::new(),
            turn: None,
            answered: BTreeSet::new(),
            held: None,
            reorder_next: false,
            interrupted: None,
        }
    }
}

/// What the fake keeps on disk between radar's runs (`RADAR_FAKE_AGENT_STATE`):
/// each session's persisted history, as a real CLI keeps its own.
#[derive(serde::Serialize, serde::Deserialize)]
struct Saved {
    next_session: u64,
    sessions: Vec<SavedSess>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct SavedSess {
    radar: RadarSessionId,
    provider_session_id: ProviderSessionId,
    cwd: String,
    seq: u64,
    next_event: u64,
    next_message: u64,
    history: Vec<Event>,
    answered: BTreeSet<String>,
    /// The command the turn waited to run when that run's process ended.
    #[serde(default)]
    interrupted: Option<String>,
}

struct State {
    sessions: BTreeMap<ProviderSessionId, Sess>,
    next_session: u64,
    tx: Option<mpsc::UnboundedSender<Update>>,
    /// Updates made before anything subscribed.
    backlog: Vec<Update>,
    /// A CLI slow to listen ([`FakeProvider::hold_subscribe`]): `subscribe` answers once this
    /// reads true, and what is said before that is told to nobody (counted in `unheard`).
    held: Option<tokio::sync::watch::Sender<bool>>,
    unheard: usize,
    connected: bool,
    clock: i64,
    scripts: VecDeque<Script>,
    signed_in: bool,
    installed: bool,
    /// What each create and resume was set up with, and the provider setups, in order.
    setups: Vec<SessionSetup>,
    provider_setups: Vec<ProviderSetup>,
    /// What a new session reports as its commands, when set.
    commands: Option<Vec<SlashCommand>>,
    /// The provider's id (`fake`; a second fake provider has another).
    id: String,
    /// An unused session leaves nothing in the provider's list ([`Adapter::idle_session_is_free`]).
    idle_free: bool,
    /// How many times each call was made to the provider (`status`, `list_models`, `catalog`, `warm`,
    /// `create`, `list_sessions`), and the sessions radar discarded unused.
    calls: BTreeMap<&'static str, usize>,
    discarded: Vec<RadarSessionId>,
    /// Where the sessions' persisted history is kept between radar's runs, when anywhere.
    file: Option<std::path::PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    inspection: Inspection,
}

impl State {
    /// Write what the provider keeps (persisted history, no running turn) to its file. A turn
    /// that waits for a permission is kept as interrupted: the next run's process asks again.
    fn save(&self) {
        let Some(file) = &self.file else { return };
        let saved = Saved {
            next_session: self.next_session,
            sessions: self
                .sessions
                .values()
                .map(|s| SavedSess {
                    radar: s.radar.clone(),
                    provider_session_id: s.provider_session_id.clone(),
                    cwd: s.cwd.clone(),
                    seq: s.seq,
                    next_event: s.next_event,
                    next_message: s.next_message,
                    history: s.history.clone(),
                    answered: s.answered.clone(),
                    interrupted: match s.turn.as_ref().and_then(|t| t.blocked.as_ref()) {
                        Some(Blocked::Permission { command, .. }) => Some(command.clone()),
                        _ => s.interrupted.clone(),
                    },
                })
                .collect(),
        };
        if let Ok(text) = serde_json::to_string(&saved) {
            let tmp = file.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, file);
            }
        }
    }
}

/// The provider's side. Cheap to clone; clones share state.
#[derive(Clone)]
pub struct FakeProvider {
    state: Arc<Mutex<State>>,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeProvider {
    pub fn new() -> Self {
        Self::named("fake")
    }

    /// A fake provider with another id, for a test that has two.
    pub fn named(id: &str) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                sessions: BTreeMap::new(),
                next_session: 1,
                tx: None,
                backlog: vec![],
                held: None,
                unheard: 0,
                connected: true,
                clock: 1_000,
                scripts: VecDeque::new(),
                signed_in: true,
                installed: true,
                setups: vec![],
                provider_setups: vec![],
                commands: None,
                id: id.to_string(),
                idle_free: false,
                calls: BTreeMap::new(),
                discarded: vec![],
                file: None,
                #[cfg(any(test, feature = "test-support"))]
                inspection: Inspection::default(),
            })),
        }
    }

    /// A provider that keeps its sessions' persisted history in `file`, read
    /// back now: what a radar run before this one did is there to resume, and
    /// nothing of it is running (its process ended with that run).
    pub fn with_state_file(file: std::path::PathBuf) -> Self {
        let p = Self::new();
        {
            let mut st = p.lock();
            if let Some(saved) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Saved>(&t).ok()) {
                st.next_session = saved.next_session;
                for s in saved.sessions {
                    let mut sess = Sess::new(s.radar, s.provider_session_id.clone(), s.cwd);
                    sess.seq = s.seq;
                    sess.next_event = s.next_event;
                    sess.next_message = s.next_message;
                    sess.history = s.history;
                    sess.answered = s.answered;
                    sess.interrupted = s.interrupted;
                    st.sessions.insert(s.provider_session_id, sess);
                }
            }
            st.file = Some(file);
        }
        p
    }

    /// The provider's process ends, as it does when radar closes: persisted
    /// history stays, nothing runs any more, and what a turn waited for is
    /// asked of nobody (a command it waited to run is remembered for
    /// [`FakeProvider::reask`]). The next adapter subscribes afresh.
    pub fn restart(&self) {
        let mut st = self.lock();
        st.tx = None;
        st.backlog.clear();
        st.connected = true;
        for s in st.sessions.values_mut() {
            if let Some(Blocked::Permission { command, .. }) = s.turn.take().and_then(|t| t.blocked) {
                s.interrupted = Some(command);
            }
            s.held = None;
            s.reorder_next = false;
        }
    }

    /// A provider that asks the interrupted tool call again once resumed: a
    /// new turn asks to run the command the last one waited for, with a new
    /// request id.
    pub fn reask(&self) {
        let mut st = self.lock();
        let ids: Vec<ProviderSessionId> =
            st.sessions.iter().filter(|(_, s)| s.interrupted.is_some() && s.turn.is_none()).map(|(id, _)| id.clone()).collect();
        for id in ids {
            let Some(command) = session(&mut st, &id).interrupted.take() else { continue };
            emit(&mut st, &id, EventKind::TurnStarted, Durability::Persisted);
            session(&mut st, &id).turn = Some(Turn { steps: vec![Step::Shell(command), Step::Say("The command finished.".into())].into(), blocked: None });
            advance(&mut st, &id);
        }
    }

    /// How many times radar made this call to the provider (`status`, `list_models`, `catalog`, `warm`,
    /// `create`, `list_sessions`).
    pub fn calls(&self, what: &str) -> usize {
        self.lock().calls.get(what).copied().unwrap_or(0)
    }

    /// The sessions radar discarded without ever showing them.
    pub fn discarded(&self) -> Vec<RadarSessionId> {
        self.lock().discarded.clone()
    }

    /// Whether an unused session leaves nothing in the provider's list, so the part may keep one
    /// ready (off by default: a test that counts creates is not surprised by a spare).
    pub fn set_idle_free(&self, free: bool) {
        self.lock().idle_free = free;
    }

    /// The sessions the provider holds now, the ones radar discarded gone.
    pub fn session_count(&self) -> usize {
        self.lock().sessions.len()
    }

    /// The setups the sessions were created or resumed with, oldest first.
    pub fn setups(&self) -> Vec<SessionSetup> {
        self.lock().setups.clone()
    }

    /// The provider setups radar configured, oldest first.
    pub fn provider_setups(&self) -> Vec<ProviderSetup> {
        self.lock().provider_setups.clone()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("fake provider state")
    }

    /// The next turn (one per `send`) acts out `script`; with none queued a
    /// turn uses [`auto_script`], unless strict scripting was enabled.
    pub fn script(&self, script: Script) {
        self.lock().scripts.push_back(script);
    }

    /// An adapter has subscribed: events from now on are delivered (before it, a
    /// turn's events are lost, as a real CLI's are to nobody listening).
    pub fn listening(&self) -> bool {
        self.lock().tx.is_some()
    }

    /// Require a queued script for every send. An unexpected send fails closed
    /// and poisons later sends, so queueing a script cannot hide the mismatch.
    /// The interactive fake's default auto-script behaviour is unchanged.
    #[cfg(any(test, feature = "test-support"))]
    pub fn strict_scripts(&self) {
        self.lock().inspection.strict_scripts = true;
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn observations(&self) -> Vec<FakeCall> {
        self.lock().inspection.calls.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn remaining_scripts(&self) -> usize {
        self.lock().scripts.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn consumed_scripts(&self) -> usize {
        self.lock().inspection.consumed_scripts
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn script_failure(&self) -> Option<String> {
        self.lock().inspection.failure.clone()
    }

    /// A stored sender alone does not prove that its receiver survived teardown.
    #[cfg(any(test, feature = "test-support"))]
    pub fn has_live_subscriber(&self) -> bool {
        self.lock().tx.as_ref().is_some_and(|tx| !tx.is_closed())
    }

    /// Provider-persisted events for this radar session. Ephemeral deltas and
    /// radar's metadata-only cache are deliberately not part of this evidence.
    #[cfg(any(test, feature = "test-support"))]
    pub fn persisted_events(&self, radar: &RadarSessionId) -> Vec<Event> {
        self.lock().sessions.values().filter(|s| &s.radar == radar).flat_map(|s| s.history.clone()).collect()
    }

    /// A CLI slow to listen: `subscribe` answers only after [`FakeProvider::let_subscribe`], and
    /// what the provider says before anyone listens is lost, as a real CLI's is (no backlog), and
    /// counted ([`FakeProvider::unheard`]).
    pub fn hold_subscribe(&self) {
        self.lock().held = Some(tokio::sync::watch::channel(false).0);
    }

    /// The held `subscribe` answers now.
    pub fn let_subscribe(&self) {
        if let Some(held) = &self.lock().held {
            held.send_replace(true);
        }
    }

    /// How many updates were told to nobody while `subscribe` was held.
    pub fn unheard(&self) -> usize {
        self.lock().unheard
    }

    pub fn set_signed_in(&self, signed_in: bool) {
        self.lock().signed_in = signed_in;
    }

    /// Every session created from now on reports these as its commands, as a provider does
    /// once it has started the session.
    pub fn set_commands(&self, commands: Vec<SlashCommand>) {
        self.lock().commands = Some(commands);
    }

    pub fn set_installed(&self, installed: bool) {
        self.lock().installed = installed;
    }

    /// The connection drops: adapters hear it, and nothing more is delivered
    /// until [`FakeProvider::reconnect`]. Persisted events still happen.
    pub fn disconnect(&self) {
        let mut st = self.lock();
        if st.connected {
            st.connected = false;
            push_update(&mut st, Update::Disconnected { reason: "connection lost".into() });
        }
    }

    pub fn reconnect(&self) {
        let mut st = self.lock();
        if !st.connected {
            st.connected = true;
            push_update(&mut st, Update::Reconnected);
        }
    }

    /// Let a hanging turn finish by itself.
    pub fn release(&self) {
        let mut st = self.lock();
        #[cfg(any(test, feature = "test-support"))]
        st.inspection.calls.push(FakeCall::Release);
        let ids: Vec<ProviderSessionId> = st
            .sessions
            .iter()
            .filter(|(_, s)| matches!(s.turn.as_ref().and_then(|t| t.blocked.as_ref()), Some(Blocked::Hang)))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(turn) = st.sessions.get_mut(&id).and_then(|s| s.turn.as_mut()) {
                turn.blocked = None;
            }
            advance(&mut st, &id);
        }
    }

    /// How many sessions the provider still has a turn running in (a test asks: a closed tab leaves none).
    pub fn running(&self) -> usize {
        self.lock().sessions.values().filter(|s| s.turn.is_some()).count()
    }

    /// A handle for radar's side.
    pub fn adapter(&self) -> FakeAdapter {
        FakeAdapter { provider: self.clone(), subscribed: Arc::new(Mutex::new(false)), pace: None }
    }
}

fn session<'a>(st: &'a mut State, id: &ProviderSessionId) -> &'a mut Sess {
    st.sessions.get_mut(id).expect("fake session exists")
}

/// Make an event of `session`, keep it in history when persisted, and deliver
/// it when connected.
fn emit(st: &mut State, id: &ProviderSessionId, kind: EventKind, durability: Durability) {
    st.clock += 1;
    let observed_at = st.clock;
    let provider = ProviderId::new(st.id.clone());
    let s = session(st, id);
    s.seq += 1;
    s.next_event += 1;
    let event = Event {
        radar_session_id: s.radar.clone(),
        provider,
        provider_session_id: s.provider_session_id.clone(),
        provider_event_id: Some(ProviderEventId::new(format!("{}-e{}", s.provider_session_id, s.next_event))),
        sequence: s.seq,
        observed_at,
        kind,
        durability,
        agent: None,
        parent: None,
        payload: serde_json::Value::Null,
    };
    let mut deliver = vec![event.clone()];
    if durability == Durability::Persisted {
        s.history.push(event.clone());
        if s.reorder_next {
            s.reorder_next = false;
            s.held = Some(event);
            deliver.clear();
        } else if let Some(held) = s.held.take() {
            deliver.push(held);
        }
    }
    deliver_all(st, deliver);
    if durability == Durability::Persisted {
        st.save();
    }
}

/// What the provider tells the part: at once when it listens, else kept for when it does (as a
/// real adapter's channel keeps it), so a session that speaks before the part has subscribed
/// is not lost.
fn push_update(st: &mut State, update: Update) {
    match &st.tx {
        Some(tx) => {
            let _ = tx.send(update);
        }
        None if st.held.is_some() => st.unheard += 1,
        None => st.backlog.push(update),
    }
}

fn deliver_all(st: &mut State, events: Vec<Event>) {
    if st.connected {
        for e in events {
            push_update(st, Update::Event(e));
        }
    }
}

/// Run the turn's steps until one waits or they end.
fn advance(st: &mut State, id: &ProviderSessionId) {
    loop {
        let step = {
            let s = session(st, id);
            let Some(turn) = s.turn.as_mut() else { return };
            if turn.blocked.is_some() {
                return;
            }
            turn.steps.pop_front()
        };
        match step {
            None => {
                emit(st, id, EventKind::TurnEnded, Durability::Persisted);
                emit(st, id, EventKind::Idle, Durability::Ephemeral);
                session(st, id).turn = None;
                return;
            }
            Some(Step::Say(text)) => {
                let s = session(st, id);
                s.next_message += 1;
                let message_id = format!("m{}", s.next_message);
                for word in text.split_inclusive(' ') {
                    emit(st, id, EventKind::AssistantDelta { message_id: message_id.clone(), text: word.to_string() }, Durability::Ephemeral);
                }
                emit(st, id, EventKind::AssistantMessage { message_id, text }, Durability::Persisted);
            }
            Some(Step::Think(text)) => {
                let s = session(st, id);
                s.next_message += 1;
                let message_id = format!("m{}", s.next_message);
                for word in text.split_inclusive(' ') {
                    emit(st, id, EventKind::ReasoningDelta { message_id: message_id.clone(), text: word.to_string() }, Durability::Ephemeral);
                }
                emit(st, id, EventKind::Reasoning { message_id, text }, Durability::Persisted);
            }
            Some(Step::Tool { name, summary, diff, result, ok }) => {
                let s = session(st, id);
                s.next_message += 1;
                let call_id = format!("call{}", s.next_message);
                emit(st, id, EventKind::ToolStarted { call_id: call_id.clone(), name, summary, diff }, Durability::Persisted);
                emit(st, id, EventKind::ToolCompleted { call_id, ok, summary: result }, Durability::Persisted);
            }
            Some(Step::Said(text)) => {
                let s = session(st, id);
                s.next_message += 1;
                let message_id = format!("m{}", s.next_message);
                emit(st, id, EventKind::AssistantMessage { message_id, text }, Durability::Persisted);
            }
            Some(Step::Shell(command)) => {
                let s = session(st, id);
                s.next_message += 1;
                let call_id = format!("call{}", s.next_message);
                let request_id = format!("req{}", s.next_message);
                emit(st, id, EventKind::ToolStarted { call_id: call_id.clone(), name: "shell".into(), summary: command.clone(), diff: None }, Durability::Persisted);
                let request = PermissionRequest {
                    request_id: request_id.clone(),
                    kind: PermissionKind::Shell,
                    target: PermissionTarget::Command { command: command.clone(), segments: command.split(' ').map(String::from).collect() },
                    tool_call_id: Some(call_id.clone()),
                    // The fake remembers nothing itself: an approval for the session is radar's.
                    can_offer_session_approval: true,
                };
                emit(st, id, EventKind::PermissionRequested { request }, Durability::Persisted);
                if let Some(turn) = session(st, id).turn.as_mut() {
                    turn.blocked = Some(Blocked::Permission { request_id, call_id, command });
                }
            }
            Some(Step::Ask { question, choices }) => {
                let s = session(st, id);
                s.next_message += 1;
                let request_id = format!("ask{}", s.next_message);
                let request = QuestionRequest {
                    request_id: request_id.clone(),
                    questions: vec![Question {
                        id: "0".into(),
                        header: None,
                        text: question,
                        choices: choices.into_iter().map(|label| QuestionChoice { label, description: None }).collect(),
                        free_text: true,
                        multiple: false,
                        secret: false,
                    }],
                    tool_call_id: None,
                };
                emit(st, id, EventKind::QuestionAsked { request }, Durability::Persisted);
                if let Some(turn) = session(st, id).turn.as_mut() {
                    turn.blocked = Some(Blocked::Question { request_id });
                }
            }
            Some(Step::Hang) => {
                if let Some(turn) = session(st, id).turn.as_mut() {
                    turn.blocked = Some(Blocked::Hang);
                }
            }
            Some(Step::Fail(message)) => {
                emit(st, id, EventKind::Error { message, retryable: false }, Durability::Persisted);
                emit(st, id, EventKind::TurnEnded, Durability::Persisted);
                emit(st, id, EventKind::Idle, Durability::Ephemeral);
                session(st, id).turn = None;
                return;
            }
            Some(Step::Redeliver) => {
                let last = session(st, id).history.last().cloned();
                if let Some(event) = last {
                    deliver_all(st, vec![event]);
                }
            }
            Some(Step::Reorder) => session(st, id).reorder_next = true,
        }
    }
}

/// radar's handle on a [`FakeProvider`].
pub struct FakeAdapter {
    provider: FakeProvider,
    subscribed: Arc<Mutex<bool>>,
    /// Live use only: each streamed delta is delivered this long after the
    /// one before, so the dock sees text arrive as a CLI streams it.
    pace: Option<std::time::Duration>,
}

/// The variable that paces the fake's streamed deltas, in ms (live checks).
pub const PACE_VAR: &str = "RADAR_FAKE_AGENT_PACE_MS";

impl FakeAdapter {
    pub fn new() -> Self {
        FakeProvider::new().adapter()
    }

    /// The fake as `RADAR_FAKE_AGENT` runs it: paced by `RADAR_FAKE_AGENT_PACE_MS`
    /// when that is set (a person watching, a latency trace), at once
    /// otherwise.
    pub fn from_env() -> Self {
        Self::new().paced_from_env()
    }

    /// This adapter, paced by `RADAR_FAKE_AGENT_PACE_MS` when that is set.
    pub fn paced_from_env(self) -> Self {
        let pace = std::env::var(PACE_VAR).ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|ms| *ms > 0);
        Self { pace: pace.map(std::time::Duration::from_millis), ..self }
    }

    pub fn provider(&self) -> &FakeProvider {
        &self.provider
    }

    fn find(st: &State, radar: &RadarSessionId) -> anyhow::Result<ProviderSessionId> {
        st.sessions
            .values()
            .find(|s| &s.radar == radar)
            .map(|s| s.provider_session_id.clone())
            .ok_or_else(|| anyhow::anyhow!("no such session: {radar}"))
    }
}

impl Default for FakeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Adapter for FakeAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new(self.provider.lock().id.clone())
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            resume: true,
            stop: true,
            steer: false,
            queue: false,
            list_sessions: true,
            delete_session: true,
            list_models: true,
            permissions: true,
            questions: true,
            structured_events: true,
            slash_commands: vec![SlashCommand::new("model", "choose the model")],
            agents: true,
            skills: true,
            unsupported: vec![Unsupported { control: "steer".into(), reason: "the fake runs one turn at a time".into() }],
        }
    }

    fn configure(&self, setup: ProviderSetup) {
        self.provider.lock().provider_setups.push(setup);
    }

    fn warm(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            let mut st = self.provider.lock();
            *st.calls.entry("warm").or_default() += 1;
            if !st.installed {
                anyhow::bail!("the fake CLI is not installed");
            }
            Ok(())
        })
    }

    fn idle_session_is_free(&self) -> bool {
        self.provider.lock().idle_free
    }

    fn discard(&self, session_id: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let mut st = self.provider.lock();
            st.sessions.retain(|_, s| s.radar != session_id);
            st.discarded.push(session_id);
            Ok(())
        })
    }

    /// Agents `reviewer` and `writer`; a skill `notes`, and one per skill folder of the setup.
    fn catalog(&self, _cwd: std::path::PathBuf, setup: SessionSetup) -> BoxFuture<'_, anyhow::Result<Catalog>> {
        *self.provider.lock().calls.entry("catalog").or_default() += 1;
        Box::pin(async move {
            let entry = |name: &str, source: &str| CatalogEntry { name: name.into(), description: None, source: Some(source.into()) };
            let mut skills = vec![entry("notes", "user")];
            skills.extend(setup.skill_dirs.iter().map(|d| entry(&d.file_name().unwrap_or_default().to_string_lossy(), "folder")));
            Ok(Catalog { agents: vec![entry("reviewer", "project"), entry("writer", "user")], skills })
        })
    }

    fn status(&self) -> BoxFuture<'_, AdapterStatus> {
        let (installed, signed_in) = {
            let mut st = self.provider.lock();
            *st.calls.entry("status").or_default() += 1;
            (st.installed, st.signed_in)
        };
        let caps = self.capabilities();
        Box::pin(async move {
            AdapterStatus {
                installed,
                version: installed.then(|| "fake-1".to_string()),
                protocol: installed.then(|| "1".to_string()),
                auth: if !installed {
                    AuthStatus::Unknown
                } else if signed_in {
                    AuthStatus::SignedIn { as_user: "fake-user".into() }
                } else {
                    AuthStatus::SignedOut
                },
                error: (!installed).then(|| "the fake CLI is not installed".to_string()),
                capabilities: caps,
            }
        })
    }

    fn list_models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
        *self.provider.lock().calls.entry("list_models").or_default() += 1;
        Box::pin(async {
            Ok(vec![
                ModelInfo { id: "fake-small".into(), name: "Fake small".into() },
                ModelInfo { id: "fake-large".into(), name: "Fake large".into() },
            ])
        })
    }

    fn subscribe(&self) -> BoxFuture<'_, anyhow::Result<Updates>> {
        Box::pin(async move {
            #[cfg(any(test, feature = "test-support"))]
            self.provider.lock().inspection.calls.push(FakeCall::Subscribe);
            {
                let mut done = self.subscribed.lock().expect("flag");
                if *done {
                    anyhow::bail!("already subscribed");
                }
                *done = true;
            }
            let held = self.provider.lock().held.as_ref().map(|h| h.subscribe());
            if let Some(mut held) = held {
                let _ = held.wait_for(|open| *open).await;
            }
            let (tx, rx) = mpsc::unbounded_channel();
            let mut st = self.provider.lock();
            // What happened before the part listened comes first, in order.
            for update in std::mem::take(&mut st.backlog) {
                let _ = tx.send(update);
            }
            // A session made before the part listened still has its commands to report.
            if let Some(commands) = st.commands.clone() {
                for s in st.sessions.values() {
                    let _ = tx.send(Update::Commands { session: s.radar.clone(), commands: commands.clone() });
                }
            }
            st.tx = Some(tx);
            let Some(pace) = self.pace else { return Ok(rx) };
            // Paced: a thread passes the updates on, waiting `pace` before each
            // streamed delta, in order.
            let (paced, out) = mpsc::unbounded_channel();
            let mut rx = rx;
            std::thread::spawn(move || {
                while let Some(update) = rx.blocking_recv() {
                    if matches!(&update, Update::Event(e) if matches!(e.kind, EventKind::AssistantDelta { .. })) {
                        std::thread::sleep(pace);
                    }
                    if paced.send(update).is_err() {
                        break;
                    }
                }
            });
            Ok(out)
        })
    }

    fn create(&self, req: CreateSession) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
        Box::pin(async move {
            let mut st = self.provider.lock();
            #[cfg(any(test, feature = "test-support"))]
            st.inspection.calls.push(FakeCall::Create(req.clone()));
            if !st.installed {
                anyhow::bail!("the fake CLI is not installed");
            }
            st.setups.push(req.setup.clone());
            *st.calls.entry("create").or_default() += 1;
            let n = st.next_session;
            st.next_session += 1;
            let id = ProviderSessionId::new(format!("fake-session-{n}"));
            st.sessions.insert(id.clone(), Sess::new(req.radar_session_id, id.clone(), req.cwd.display().to_string()));
            st.save();
            if let Some(commands) = st.commands.clone() {
                let session = st.sessions[&id].radar.clone();
                push_update(&mut st, Update::Commands { session, commands });
            }
            Ok(id)
        })
    }

    fn send(&self, session_id: &RadarSessionId, input: SendInput) -> BoxFuture<'_, anyhow::Result<()>> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let mut st = self.provider.lock();
            #[cfg(any(test, feature = "test-support"))]
            {
                st.inspection.calls.push(FakeCall::Send { session_id: session_id.clone(), input: input.clone() });
                if let Some(failure) = &st.inspection.failure {
                    anyhow::bail!("{failure}");
                }
            }
            if !st.connected {
                anyhow::bail!("not connected to the provider");
            }
            let id = Self::find(&st, &session_id)?;
            if session(&mut st, &id).turn.is_some() {
                anyhow::bail!("a turn is running; the fake cannot take text mid-turn");
            }
            #[cfg(any(test, feature = "test-support"))]
            if st.inspection.strict_scripts && st.scripts.is_empty() {
                let failure = format!("unexpected send for {session_id}: no queued fake script");
                st.inspection.failure = Some(failure.clone());
                anyhow::bail!("{failure}");
            }
            let script = match st.scripts.pop_front() {
                Some(script) => {
                    #[cfg(any(test, feature = "test-support"))]
                    { st.inspection.consumed_scripts += 1; }
                    script
                }
                None => auto_script(&input.text),
            };
            emit(&mut st, &id, EventKind::UserMessage { text: input.text }, Durability::Persisted);
            emit(&mut st, &id, EventKind::TurnStarted, Durability::Persisted);
            session(&mut st, &id).turn = Some(Turn { steps: script.into(), blocked: None });
            advance(&mut st, &id);
            Ok(())
        })
    }

    fn stop(&self, session_id: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let mut st = self.provider.lock();
            #[cfg(any(test, feature = "test-support"))]
            st.inspection.calls.push(FakeCall::Stop { session_id: session_id.clone() });
            let id = Self::find(&st, &session_id)?;
            let Some(turn) = session(&mut st, &id).turn.take() else { return Ok(()) };
            match turn.blocked {
                Some(Blocked::Permission { request_id, call_id, .. }) => {
                    session(&mut st, &id).answered.insert(request_id.clone());
                    emit(&mut st, &id, EventKind::PermissionResolved { request_id, approved: false }, Durability::Persisted);
                    emit(&mut st, &id, EventKind::ToolCompleted { call_id, ok: false, summary: "stopped".into() }, Durability::Persisted);
                }
                Some(Blocked::Question { request_id }) => {
                    session(&mut st, &id).answered.insert(request_id.clone());
                    emit(&mut st, &id, EventKind::QuestionResolved { request_id, replies: None }, Durability::Persisted);
                }
                _ => {}
            }
            emit(&mut st, &id, EventKind::Aborted, Durability::Persisted);
            emit(&mut st, &id, EventKind::TurnEnded, Durability::Persisted);
            emit(&mut st, &id, EventKind::Idle, Durability::Ephemeral);
            Ok(())
        })
    }

    fn resume(&self, req: ResumeSession) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            let mut st = self.provider.lock();
            #[cfg(any(test, feature = "test-support"))]
            st.inspection.calls.push(FakeCall::Resume(req.clone()));
            st.setups.push(req.setup.clone());
            let Some(s) = st.sessions.get_mut(&req.provider_session_id) else {
                anyhow::bail!("the provider no longer has session {}", req.provider_session_id);
            };
            s.radar = req.radar_session_id.clone();
            let after = match &req.cursor {
                Some(cursor) => s
                    .history
                    .iter()
                    .position(|e| e.provider_event_id.as_ref() == Some(cursor))
                    .map(|i| i + 1)
                    .unwrap_or(0),
                None => 0,
            };
            let replay: Vec<Event> = s
                .history
                .iter()
                .skip(after)
                .cloned()
                .map(|mut e| {
                    e.radar_session_id = req.radar_session_id.clone();
                    e
                })
                .collect();
            deliver_all(&mut st, replay);
            Ok(())
        })
    }

    fn list_sessions(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async move {
            let mut st = self.provider.lock();
            *st.calls.entry("list_sessions").or_default() += 1;
            Ok(st
                .sessions
                .values()
                .map(|s| ProviderSessionInfo {
                    provider_session_id: s.provider_session_id.clone(),
                    title: None,
                    cwd: Some(s.cwd.clone()),
                    updated_at: None,
                })
                .collect())
        })
    }

    fn delete_session(&self, session_id: &ProviderSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let mut st = self.provider.lock();
            st.sessions.remove(&session_id);
            st.save();
            Ok(())
        })
    }

    fn answer_permission(&self, session_id: &RadarSessionId, answer: PermissionAnswer) -> BoxFuture<'_, anyhow::Result<()>> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let mut st = self.provider.lock();
            #[cfg(any(test, feature = "test-support"))]
            st.inspection.calls.push(FakeCall::Permission { session_id: session_id.clone(), answer: answer.clone() });
            let id = Self::find(&st, &session_id)?;
            if session(&mut st, &id).answered.contains(&answer.request_id) {
                return Ok(());
            }
            let blocked = session(&mut st, &id).turn.as_ref().and_then(|t| t.blocked.clone());
            let Some(Blocked::Permission { request_id, call_id, command }) = blocked else {
                anyhow::bail!("no permission request {} is waiting", answer.request_id);
            };
            if request_id != answer.request_id {
                anyhow::bail!("no permission request {} is waiting", answer.request_id);
            }
            session(&mut st, &id).answered.insert(request_id.clone());
            let (approved, summary) = match &answer.decision {
                PermissionDecision::ApproveOnce | PermissionDecision::ApproveForSession => (true, format!("ran {command}")),
                PermissionDecision::Deny { reason } => (false, format!("denied: {}", reason.clone().unwrap_or_default())),
            };
            emit(&mut st, &id, EventKind::PermissionResolved { request_id, approved }, Durability::Persisted);
            emit(&mut st, &id, EventKind::ToolCompleted { call_id, ok: approved, summary }, Durability::Persisted);
            if let Some(turn) = session(&mut st, &id).turn.as_mut() {
                turn.blocked = None;
            }
            advance(&mut st, &id);
            Ok(())
        })
    }

    fn answer_question(&self, session_id: &RadarSessionId, answer: QuestionAnswer) -> BoxFuture<'_, anyhow::Result<()>> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let mut st = self.provider.lock();
            let id = Self::find(&st, &session_id)?;
            if session(&mut st, &id).answered.contains(&answer.request_id) {
                return Ok(());
            }
            let blocked = session(&mut st, &id).turn.as_ref().and_then(|t| t.blocked.clone());
            let Some(Blocked::Question { request_id }) = blocked.filter(|b| matches!(b, Blocked::Question { request_id } if *request_id == answer.request_id)) else {
                anyhow::bail!("no question {} is waiting", answer.request_id);
            };
            session(&mut st, &id).answered.insert(request_id.clone());
            let (replies, said) = match answer.decision {
                QuestionDecision::Answer { replies } => {
                    let said = format!("You chose: {}", replies.iter().flat_map(|r| r.values.iter().cloned()).collect::<Vec<_>>().join(", "));
                    (Some(replies), said)
                }
                QuestionDecision::Decline { .. } => (None, "No answer".to_string()),
            };
            emit(&mut st, &id, EventKind::QuestionResolved { request_id, replies }, Durability::Persisted);
            if let Some(turn) = session(&mut st, &id).turn.as_mut() {
                turn.blocked = None;
                turn.steps.push_front(Step::Say(said));
            }
            advance(&mut st, &id);
            Ok(())
        })
    }
}

/// Drives a [`FakeProvider`] for the contract kit.
pub struct FakeHarness {
    provider: FakeProvider,
}

impl Default for FakeHarness {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeHarness {
    pub fn new() -> Self {
        Self { provider: FakeProvider::new() }
    }

    pub fn provider(&self) -> &FakeProvider {
        &self.provider
    }
}

impl Harness for FakeHarness {
    fn adapter(&mut self, scenario: Scenario) -> BoxFuture<'_, Box<dyn Adapter>> {
        self.provider = FakeProvider::new();
        let say = |t: &str| Step::Say(t.to_string());
        match scenario {
            Scenario::Text => self.provider.script(vec![say("Hello from the fake provider")]),
            Scenario::TwoTurns => {
                self.provider.script(vec![say("First answer")]);
                self.provider.script(vec![say("Second answer")]);
            }
            Scenario::Tool => self.provider.script(vec![
                Step::Shell("ls -la".into()),
                say("Done with the command"),
            ]),
            Scenario::LongTurn => self.provider.script(vec![say("Working on it"), Step::Hang, say("Finished")]),
            Scenario::Redelivery => self.provider.script(vec![
                say("Once"),
                Step::Redeliver,
                Step::Shell("ls".into()),
                Step::Redeliver,
                say("Twice delivered"),
            ]),
            Scenario::Reordered => self.provider.script(vec![Step::Reorder, say("Late first"), say("Then second")]),
            Scenario::Question => self.provider.script(vec![Step::Ask {
                question: "Which colour should it be?".into(),
                choices: vec!["red".into(), "blue".into()],
            }]),
        }
        let adapter: Box<dyn Adapter> = Box::new(self.provider.adapter());
        Box::pin(async move { adapter })
    }

    fn restart(&mut self) -> BoxFuture<'_, Box<dyn Adapter>> {
        // radar's restart ends the provider's process with it: nothing of a turn survives but its history.
        self.provider.restart();
        let adapter: Box<dyn Adapter> = Box::new(self.provider.adapter());
        Box::pin(async move { adapter })
    }

    fn settle(&mut self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn disconnect(&mut self) -> BoxFuture<'_, ()> {
        self.provider.disconnect();
        Box::pin(async {})
    }

    fn reconnect(&mut self) -> BoxFuture<'_, ()> {
        self.provider.reconnect();
        Box::pin(async {})
    }

    fn release(&mut self) -> BoxFuture<'_, ()> {
        self.provider.release();
        Box::pin(async {})
    }
}

#[cfg(test)]
mod long_tests {
    use super::*;

    /// `fill N` and `stream N` make the long transcripts a latency check needs.
    #[test]
    fn fill_answers_n_times_at_once_and_stream_streams_n_words() {
        let fill = auto_script("fill 3");
        assert_eq!(fill.len(), 3);
        assert!(fill.iter().all(|s| matches!(s, Step::Said(t) if t.starts_with("Answer "))));
        let stream = auto_script("stream 40");
        let [Step::Say(text)] = stream.as_slice() else { panic!("one streamed answer: {stream:?}") };
        assert_eq!(text.split(' ').count(), 40);
        assert_eq!(auto_script("hello"), vec![Step::Say("You said: hello".into())]);
    }
}

#[cfg(test)]
mod inspection_tests {
    use super::*;

    async fn created(fake: &FakeProvider) -> (FakeAdapter, CreateSession, ProviderSessionId) {
        let adapter = fake.adapter();
        let request = CreateSession {
            radar_session_id: RadarSessionId::new("watch-patrol"),
            cwd: std::path::PathBuf::from("/ankh-morpork/watch"),
            model: None,
            setup: SessionSetup { local_only: true, ..Default::default() },
        };
        let provider_session = adapter.create(request.clone()).await.unwrap();
        (adapter, request, provider_session)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn strict_unexpected_send_is_sticky_and_consumes_no_script() {
        let fake = FakeProvider::new();
        fake.strict_scripts();
        let (adapter, request, _) = created(&fake).await;
        let input = SendInput { text: "Check the patrol notes".into() };
        let first = adapter.send(&request.radar_session_id, input.clone()).await.unwrap_err().to_string();
        assert!(first.contains("no queued fake script"));
        // Adding a reply after the mismatch must not turn a failed fixture green.
        let observer = fake.clone();
        observer.script(vec![Step::Say("Too late".into())]);
        let again = adapter.send(&request.radar_session_id, input.clone()).await.unwrap_err().to_string();
        assert_eq!(again, first);
        assert_eq!(observer.script_failure(), Some(first));
        assert_eq!(fake.remaining_scripts(), 1);
        assert_eq!(fake.consumed_scripts(), 0);
        assert_eq!(fake.running(), 0);
        assert!(fake.persisted_events(&request.radar_session_id).is_empty());
        assert_eq!(fake.observations(), vec![
            FakeCall::Create(request.clone()),
            FakeCall::Send { session_id: request.radar_session_id.clone(), input: input.clone() },
            FakeCall::Send { session_id: request.radar_session_id, input },
        ]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn default_fake_still_auto_scripts_without_a_queued_turn() {
        let fake = FakeProvider::new();
        let (adapter, request, _) = created(&fake).await;
        adapter.send(&request.radar_session_id, SendInput { text: "Hello Watch".into() }).await.unwrap();
        assert!(fake.persisted_events(&request.radar_session_id).iter().any(|event| matches!(&event.kind,
            EventKind::AssistantMessage { text, .. } if text == "You said: Hello Watch")));
        assert_eq!(fake.script_failure(), None);
        assert_eq!(fake.consumed_scripts(), 0);
        assert_eq!(fake.running(), 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn release_cannot_answer_permission_and_duplicate_answer_does_not_complete_twice() {
        let fake = FakeProvider::new();
        fake.strict_scripts();
        fake.script(vec![Step::Shell("cargo test".into()), Step::Say("The fake continued".into())]);
        let (adapter, request, _) = created(&fake).await;
        let radar = &request.radar_session_id;
        adapter.send(radar, SendInput { text: "Check the patrol notes".into() }).await.unwrap();
        let waiting = fake.persisted_events(radar);
        let permission = waiting.iter().find_map(|event| match &event.kind {
            EventKind::PermissionRequested { request } => Some(request.clone()),
            _ => None,
        }).unwrap();
        fake.release();
        assert_eq!(fake.persisted_events(radar), waiting, "release only releases Hang, never a permission");
        assert_eq!(fake.running(), 1);
        let wrong = PermissionAnswer { request_id: "another-request".into(), decision: PermissionDecision::ApproveOnce };
        assert!(adapter.answer_permission(radar, wrong).await.is_err());
        assert_eq!(fake.persisted_events(radar), waiting);
        let answer = PermissionAnswer { request_id: permission.request_id, decision: PermissionDecision::Deny { reason: Some("not now".into()) } };
        adapter.answer_permission(radar, answer.clone()).await.unwrap();
        let completed = fake.persisted_events(radar);
        adapter.answer_permission(radar, answer.clone()).await.unwrap();
        assert_eq!(fake.persisted_events(radar), completed, "duplicate answers are idempotent");
        assert_eq!(completed.iter().filter(|event| matches!(event.kind, EventKind::PermissionResolved { .. })).count(), 1);
        assert_eq!(completed.iter().filter(|event| matches!(event.kind, EventKind::ToolCompleted { ok: false, .. })).count(), 1);
        assert_eq!(completed.iter().filter(|event| matches!(event.kind, EventKind::TurnEnded)).count(), 1);
        assert_eq!(fake.observations().iter().filter(|call| matches!(call,
            FakeCall::Permission { session_id, answer: actual } if session_id == radar && actual == &answer)).count(), 2);
        assert_eq!(fake.running(), 0);
        assert_eq!(fake.remaining_scripts(), 0);
        assert_eq!(fake.consumed_scripts(), 1);
        assert_eq!(fake.script_failure(), None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn observations_track_subscription_resume_stop_and_receiver_closure() {
        let fake = FakeProvider::new();
        fake.strict_scripts();
        fake.script(vec![Step::Hang, Step::Say("Never reached".into())]);
        let (adapter, create, provider_session_id) = created(&fake).await;
        assert!(!fake.has_live_subscriber());
        let receiver = adapter.subscribe().await.unwrap();
        assert!(fake.has_live_subscriber());
        assert!(adapter.subscribe().await.is_err(), "the same handle cannot subscribe twice");
        assert!(fake.has_live_subscriber(), "rejected duplicate leaves the original stream open");
        let radar = &create.radar_session_id;
        let input = SendInput { text: "Keep checking".into() };
        adapter.send(radar, input.clone()).await.unwrap();
        assert_eq!(fake.running(), 1);
        adapter.stop(radar).await.unwrap();
        assert_eq!(fake.running(), 0);
        let stopped = fake.persisted_events(radar);
        fake.release();
        assert_eq!(fake.persisted_events(radar), stopped, "release cannot revive a stopped turn");
        let resume = ResumeSession {
            radar_session_id: radar.clone(), provider_session_id,
            cwd: create.cwd.clone(), cursor: None, setup: create.setup.clone(),
        };
        adapter.resume(resume.clone()).await.unwrap();
        assert_eq!(fake.persisted_events(radar), stopped, "resume replays rather than regenerating history");
        assert_eq!(fake.observations(), vec![
            FakeCall::Create(create.clone()), FakeCall::Subscribe, FakeCall::Subscribe,
            FakeCall::Send { session_id: radar.clone(), input },
            FakeCall::Stop { session_id: radar.clone() }, FakeCall::Release, FakeCall::Resume(resume),
        ]);
        drop(receiver);
        assert!(fake.listening(), "legacy listening reports a stored sender");
        assert!(!fake.has_live_subscriber(), "the lifecycle check observes receiver closure");
        assert_eq!(fake.remaining_scripts(), 0);
        assert_eq!(fake.script_failure(), None);
    }
}
