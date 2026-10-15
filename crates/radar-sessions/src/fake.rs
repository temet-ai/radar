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

/// What a turn does when no script is queued: it reacts to the prompt, so a
/// person can drive the fake in the dock. A prompt starting with "ask" asks
/// a question, "run" asks to run a command, "long" keeps working until
/// stopped, "demo" reasons, reads, searches, edits and answers in markdown
/// (what the transcript draws, all at once), anything else is echoed.
pub fn auto_script(prompt: &str) -> Script {
    let lower = prompt.to_lowercase();
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
}

struct State {
    sessions: BTreeMap<ProviderSessionId, Sess>,
    next_session: u64,
    tx: Option<mpsc::UnboundedSender<Update>>,
    /// Updates made before anything subscribed.
    backlog: Vec<Update>,
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
            })),
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
    /// turn says "ok".
    pub fn script(&self, script: Script) {
        self.lock().scripts.push_back(script);
    }

    /// An adapter has subscribed: events from now on are delivered (before it, a
    /// turn's events are lost, as a real CLI's are to nobody listening).
    pub fn listening(&self) -> bool {
        self.lock().tx.is_some()
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
        FakeAdapter { provider: self.clone(), subscribed: Arc::new(Mutex::new(false)) }
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
}

/// What the provider tells the part: at once when it listens, else kept for when it does (as a
/// real adapter's channel keeps it), so a session that speaks before the part has subscribed
/// is not lost.
fn push_update(st: &mut State, update: Update) {
    match &st.tx {
        Some(tx) => {
            let _ = tx.send(update);
        }
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
}

impl FakeAdapter {
    pub fn new() -> Self {
        FakeProvider::new().adapter()
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
            let mut done = self.subscribed.lock().expect("flag");
            if *done {
                anyhow::bail!("already subscribed");
            }
            *done = true;
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
            Ok(rx)
        })
    }

    fn create(&self, req: CreateSession) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
        Box::pin(async move {
            let mut st = self.provider.lock();
            if !st.installed {
                anyhow::bail!("the fake CLI is not installed");
            }
            st.setups.push(req.setup.clone());
            *st.calls.entry("create").or_default() += 1;
            let n = st.next_session;
            st.next_session += 1;
            let id = ProviderSessionId::new(format!("fake-session-{n}"));
            st.sessions.insert(
                id.clone(),
                Sess {
                    radar: req.radar_session_id,
                    provider_session_id: id.clone(),
                    cwd: req.cwd.display().to_string(),
                    seq: 0,
                    next_event: 0,
                    next_message: 0,
                    history: Vec::new(),
                    turn: None,
                    answered: BTreeSet::new(),
                    held: None,
                    reorder_next: false,
                },
            );
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
            if !st.connected {
                anyhow::bail!("not connected to the provider");
            }
            let id = Self::find(&st, &session_id)?;
            if session(&mut st, &id).turn.is_some() {
                anyhow::bail!("a turn is running; the fake cannot take text mid-turn");
            }
            let script = st.scripts.pop_front().unwrap_or_else(|| auto_script(&input.text));
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
            self.provider.lock().sessions.remove(&session_id);
            Ok(())
        })
    }

    fn answer_permission(&self, session_id: &RadarSessionId, answer: PermissionAnswer) -> BoxFuture<'_, anyhow::Result<()>> {
        let session_id = session_id.clone();
        Box::pin(async move {
            let mut st = self.provider.lock();
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
