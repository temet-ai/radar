//! Copilot as a radar-sessions [`Adapter`].
//!
//! Local only: `/delegate`, a leading `&` and `--remote` are refused. Nothing answers a
//! permission request but the operator through [`Adapter::answer_permission`]; a request
//! waits while no renderer is attached.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use radar_sessions::adapter::{Catalog, ProviderSetup, SessionSetup, Updates};
use radar_sessions::event::AgentRef;
use radar_sessions::{
    Adapter, AdapterStatus, AuthStatus, BoxFuture, Capabilities, CreateSession, Durability, Event, EventKind, RadarSessionId,
    LoginHandle, ModelInfo, PermissionAnswer, PermissionDecision, ProviderEventId, ProviderId,
    ProviderSessionId, ProviderSessionInfo, QuestionAnswer, QuestionDecision, ResumeSession,
    SendInput, Unsupported, Update,
};
use tokio::sync::{mpsc, watch};

use crate::backend::{Backend, BackendStatus, Sink, StatusError};
use crate::commands::{self, CommandInfo, Invoked};
use crate::login::LoginFlow;
use crate::map::{map_kind, RawEvent};

/// The Copilot SDK/CLI protocol this adapter was written against.
pub const PROTOCOL: u32 = 3;

pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

pub fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    })
}

/// A request waiting for the operator, shared by the SDK handler and the dock's answer.
type Waiter = watch::Sender<Option<PermissionDecision>>;

/// A question waiting for the operator: the SDK's handler and the dock's answer meet here.
type QuestionWaiter = watch::Sender<Option<QuestionDecision>>;

/// A question the event stream announced (`user_input.requested`).
struct Asked {
    request_id: String,
    text: String,
    /// A handler waits on it, or it is settled.
    claimed: bool,
}

struct SessionState {
    radar: RadarSessionId,
    seq: AtomicU64,
    pending: Mutex<HashMap<String, Waiter>>,
    /// Every command the session lists, run by radar or not; empty until it has said.
    commands: Mutex<Vec<CommandInfo>>,
    asked: Mutex<Vec<Asked>>,
    /// Moves when a question is announced, for a handler still looking for its id.
    asked_tick: watch::Sender<u64>,
    answers: Mutex<HashMap<String, QuestionWaiter>>,
    /// The id of the last persisted event read, live or replayed.
    last_persisted: Mutex<Option<String>>,
}

impl SessionState {
    fn new(radar: RadarSessionId) -> Self {
        Self {
            radar,
            last_persisted: Mutex::new(None),
            seq: AtomicU64::new(0),
            pending: Mutex::new(HashMap::new()),
            commands: Mutex::new(Vec::new()),
            asked: Mutex::new(Vec::new()),
            asked_tick: watch::channel(0).0,
            answers: Mutex::new(HashMap::new()),
        }
    }

    fn waiter(&self, request_id: &str) -> Waiter {
        self.pending
            .lock()
            .unwrap()
            .entry(request_id.to_string())
            .or_insert_with(|| watch::channel(None).0)
            .clone()
    }

    fn question_waiter(&self, request_id: &str) -> QuestionWaiter {
        self.answers
            .lock()
            .unwrap()
            .entry(request_id.to_string())
            .or_insert_with(|| watch::channel(None).0)
            .clone()
    }

    /// What the event stream says of questions: a new one is announced, a settled one
    /// (answered here, or before, in the replayed history) is claimed for good.
    fn note_question(&self, raw: &RawEvent) {
        let id = raw.data.get("requestId").and_then(|v| v.as_str()).unwrap_or_default();
        let mut asked = self.asked.lock().unwrap();
        match raw.event_type.as_str() {
            "user_input.requested" if !asked.iter().any(|a| a.request_id == id) => {
                let text = raw.data.get("question").and_then(|v| v.as_str()).unwrap_or_default();
                asked.push(Asked { request_id: id.to_string(), text: text.to_string(), claimed: false });
                drop(asked);
                self.asked_tick.send_modify(|n| *n += 1);
            }
            "user_input.completed" => match asked.iter_mut().find(|a| a.request_id == id) {
                Some(a) => a.claimed = true,
                None => asked.push(Asked { request_id: id.to_string(), text: String::new(), claimed: true }),
            },
            _ => {}
        }
    }

    /// The request id of the newest unclaimed question asking `text`, once the event
    /// stream has announced it (the handler may run before the event is read).
    async fn claim(&self, text: &str) -> Option<String> {
        let mut tick = self.asked_tick.subscribe();
        loop {
            if let Some(a) = self.asked.lock().unwrap().iter_mut().rev().find(|a| !a.claimed && a.text == text) {
                a.claimed = true;
                return Some(a.request_id.clone());
            }
            tick.changed().await.ok()?;
        }
    }
}

struct Shared {
    /// False between a lost connection and the resume that follows it.
    connected: AtomicBool,
    clock: Clock,
    tx: mpsc::UnboundedSender<Update>,
    sessions: Mutex<HashMap<RadarSessionId, Arc<SessionState>>>,
}

impl Shared {
    fn emit(&self, st: &SessionState, raw: RawEvent) {
        st.note_question(&raw);
        let sequence = st.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let durability = raw.durability();
        if durability == Durability::Persisted {
            *st.last_persisted.lock().unwrap() = Some(raw.id.clone());
        }
        let kind = map_kind(&raw);
        let ev = Event {
            radar_session_id: st.radar.clone(),
            provider: ProviderId::new("copilot"),
            // radar's session id is Copilot's session id.
            provider_session_id: ProviderSessionId::new(st.radar.as_str()),
            provider_event_id: Some(ProviderEventId::new(raw.id)),
            sequence,
            observed_at: (self.clock)(),
            kind,
            durability,
            agent: raw.agent_id.map(|id| AgentRef { id, name: None }),
            parent: raw.parent_id.map(ProviderEventId::new),
            payload: serde_json::Value::Null,
        };
        let _ = self.tx.send(Update::Event(ev));
    }

    /// Something radar says in the transcript (a command's output), not Copilot: live only, as
    /// Copilot keeps no record of it.
    fn notice(&self, st: &SessionState, text: String) {
        self.say(st, EventKind::Notice { text }, None);
    }

    /// The replayed history ended after the event `last`: the session is idle. Part of the
    /// history (each replay of it says it again, under the same id).
    fn end_of_history(&self, st: &SessionState, last: &str) {
        self.say(st, EventKind::Idle, Some(ProviderEventId::new(format!("{last}#radar-end-of-history"))));
    }

    /// An event of radar's own about the session, in its sequence: live only, or with `id` part of
    /// what a replay of the history says.
    fn say(&self, st: &SessionState, kind: EventKind, id: Option<ProviderEventId>) {
        let sequence = st.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let durability = if id.is_some() { Durability::Persisted } else { Durability::Ephemeral };
        let ev = Event {
            radar_session_id: st.radar.clone(),
            provider: ProviderId::new("copilot"),
            provider_session_id: ProviderSessionId::new(st.radar.as_str()),
            provider_event_id: id,
            sequence,
            observed_at: (self.clock)(),
            kind,
            durability,
            agent: None,
            parent: None,
            payload: serde_json::Value::Null,
        };
        let _ = self.tx.send(Update::Event(ev));
    }
}

struct SessionSink {
    shared: Arc<Shared>,
    state: Arc<SessionState>,
}

impl Sink for SessionSink {
    fn event(&self, event: RawEvent) {
        self.shared.emit(&self.state, event);
    }

    fn permission(
        &self,
        request_id: String,
        _data: serde_json::Value,
    ) -> BoxFuture<'_, PermissionDecision> {
        Box::pin(async move {
            let mut rx = self.state.waiter(&request_id).subscribe();
            loop {
                if let Some(d) = rx.borrow_and_update().clone() {
                    return d;
                }
                if rx.changed().await.is_err() {
                    return PermissionDecision::Deny {
                        reason: Some("radar closed the session".into()),
                    };
                }
            }
        })
    }

    fn question(
        &self,
        question: String,
        _choices: Vec<String>,
        _allow_freeform: bool,
    ) -> BoxFuture<'_, QuestionDecision> {
        Box::pin(async move {
            let closed = || QuestionDecision::Decline {
                reason: Some("radar closed the session".into()),
            };
            let Some(request_id) = self.state.claim(&question).await else {
                return closed();
            };
            let mut rx = self.state.question_waiter(&request_id).subscribe();
            loop {
                if let Some(d) = rx.borrow_and_update().clone() {
                    return d;
                }
                if rx.changed().await.is_err() {
                    return closed();
                }
            }
        })
    }

    fn missed(&self, what: String) {
        self.shared.notice(&self.state, format!("radar missed some of copilot's live events ({what}); what copilot keeps is shown when the session is resumed"));
    }

    fn lost(&self, reason: String) {
        // One report per loss, however many sessions notice it.
        if self.shared.connected.swap(false, Ordering::SeqCst) {
            let _ = self.shared.tx.send(Update::Disconnected { reason });
        }
    }

    fn restored(&self) {
        if !self.shared.connected.swap(true, Ordering::SeqCst) {
            let _ = self.shared.tx.send(Update::Reconnected);
        }
    }
}

/// Where the adapter's own notes go (the host's log); nothing by default.
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

pub struct CopilotAdapter<B: Backend> {
    backend: B,
    /// The `copilot` and `COPILOT_HOME` sign-in uses; the backend has its own copy,
    /// both set by [`Adapter::configure`].
    provider: Mutex<ProviderSetup>,
    login_home: Option<PathBuf>,
    log: Log,
    shared: Arc<Shared>,
    updates: Mutex<Option<Updates>>,
}

impl<B: Backend> CopilotAdapter<B> {
    /// `program` is the `copilot` used for sign-in (the backend has its own).
    pub fn new(backend: B, program: Option<PathBuf>, clock: Clock) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            backend,
            provider: Mutex::new(ProviderSetup {
                executable: program,
                config_dir: None,
            }),
            login_home: None,
            log: Arc::new(|_| {}),
            shared: Arc::new(Shared {
                connected: AtomicBool::new(true),
                clock,
                tx,
                sessions: Mutex::new(HashMap::new()),
            }),
            updates: Mutex::new(Some(rx)),
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    /// Sign-in runs with this `COPILOT_HOME` (tests and recordings).
    pub fn with_login_home(mut self, home: PathBuf) -> Self {
        self.login_home = Some(home);
        self
    }

    /// Notes of the adapter (a setting that waits for open sessions) go here.
    pub fn with_log(mut self, log: Log) -> Self {
        self.log = log;
        self
    }

    fn state(&self, id: &RadarSessionId) -> Arc<SessionState> {
        self.shared
            .sessions
            .lock()
            .unwrap()
            .entry(id.clone())
            .or_insert_with(|| Arc::new(SessionState::new(id.clone())))
            .clone()
    }

    fn sink(&self, st: &Arc<SessionState>) -> Arc<dyn Sink> {
        Arc::new(SessionSink {
            shared: self.shared.clone(),
            state: st.clone(),
        })
    }

    /// Ask the open session which commands it has, keep them all, and tell the part which
    /// ones radar offers. A session that cannot say keeps an empty menu.
    async fn report_commands(&self, id: &RadarSessionId) {
        let Ok(list) = self.backend.commands(id).await else {
            return;
        };
        if list.is_empty() {
            return;
        }
        let offered = commands::advertised(&list);
        *self.state(id).commands.lock().unwrap() = list;
        let _ = self.shared.tx.send(Update::Commands {
            session: id.clone(),
            commands: offered,
        });
    }

    /// Runs `text` when it is `/name args` for a command the session has: its answer goes in
    /// the transcript, or its prompt to the model. `false` when it is not a command (a path,
    /// a sentence), which is then sent as written.
    async fn run_command(&self, id: &RadarSessionId, text: &str) -> anyhow::Result<bool> {
        let Some((word, input)) = radar_sessions::parse_slash(text) else {
            return Ok(false);
        };
        let st = self.state(id);
        let found = st
            .commands
            .lock()
            .unwrap()
            .iter()
            .find(|c| c.answers_to(word))
            .cloned();
        let Some(cmd) = found else {
            return Ok(false);
        };
        let typed = text.trim().lines().next().unwrap_or_default().to_string();
        let say = |what: String| self.shared.notice(&st, format!("{typed}\n{what}"));
        if let Some(why) = commands::why_not(&cmd.name) {
            say(format!("radar does not run /{}: {why}", cmd.name));
            return Ok(true);
        }
        match self.backend.invoke(id, &cmd.name, input).await {
            Err(e) => say(commands::reason(&e.to_string())),
            Ok(Invoked::Text(t)) => say(commands::cap(&t)),
            Ok(Invoked::Done(m)) => say(m.unwrap_or_else(|| "done".into())),
            Ok(Invoked::Choose { title, options }) => {
                let rows: Vec<String> = options
                    .iter()
                    .map(|(n, d)| format!("  {n}  {d}").trim_end().to_string())
                    .collect();
                say(format!("{title}\n{}", rows.join("\n")));
            }
            Ok(Invoked::Prompt {
                prompt,
                display,
                plan,
            }) => self.backend.send_prompt(id, &prompt, &display, plan).await?,
        }
        Ok(true)
    }
}

/// Text that would leave the machine or hand the work to Copilot's cloud.
pub fn refuse_remote(text: &str) -> Option<&'static str> {
    let t = text.trim_start();
    let word = t.split_whitespace().next().unwrap_or("");
    if word == "/delegate" {
        Some("/delegate hands the work to Copilot's cloud agent; radar runs Copilot locally only")
    } else if t.starts_with('&') {
        Some("a leading & hands the work to Copilot's cloud agent; radar runs Copilot locally only")
    } else {
        None
    }
}

fn bare_host(host: &str) -> &str {
    host.trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
}

pub fn capabilities() -> Capabilities {
    Capabilities {
        resume: true,
        stop: true,
        steer: false,
        // A message sent during a turn is Copilot's to keep (`mode: enqueue`, the SDK's default): it runs it as
        // the next turn (`user.message` with `delivery: queued`).
        queue: true,
        list_sessions: true,
        delete_session: true,
        list_models: true,
        permissions: true,
        questions: true,
        structured_events: true,
        slash_commands: vec![],
        agents: true,
        skills: true,
        unsupported: vec![
            Unsupported {
                control: "steer".into(),
                reason: "a message sent during a turn is queued by Copilot and runs when the turn ends".into(),
            },
            Unsupported {
                control: "/delegate".into(),
                reason:
                    "radar runs Copilot locally only; cloud delegation and remote sessions are refused"
                        .into(),
            },
        ],
    }
}

fn status_from(r: Result<BackendStatus, StatusError>) -> AdapterStatus {
    let capabilities = capabilities();
    match r {
        Err(StatusError::Missing) => AdapterStatus {
            installed: false,
            version: None,
            protocol: None,
            auth: AuthStatus::Unknown,
            error: None,
            capabilities,
        },
        Err(StatusError::Connection(e)) => AdapterStatus {
            installed: true,
            version: None,
            protocol: None,
            auth: AuthStatus::Unknown,
            error: Some(format!("could not talk to copilot: {e}")),
            capabilities,
        },
        Ok(s) => {
            let mismatch = (s.protocol != PROTOCOL).then(|| {
                format!(
                    "this copilot speaks protocol {} and radar needs {}; update radar or install a matching copilot",
                    s.protocol, PROTOCOL
                )
            });
            let auth = if s.authenticated {
                let who = s.login.clone().unwrap_or_else(|| "signed in".into());
                AuthStatus::SignedIn {
                    as_user: match &s.host {
                        Some(h) => format!("{who}@{}", bare_host(h)),
                        None => who,
                    },
                }
            } else {
                let m = s.message.clone().unwrap_or_default();
                let lower = m.to_lowercase();
                if lower.contains("policy") || lower.contains("organization") {
                    AuthStatus::PolicyBlocked { message: m }
                } else {
                    AuthStatus::SignedOut
                }
            };
            AdapterStatus {
                installed: true,
                version: Some(s.version),
                protocol: Some(s.protocol.to_string()),
                auth,
                error: mismatch,
                capabilities,
            }
        }
    }
}

impl<B: Backend> Adapter for CopilotAdapter<B> {
    fn id(&self) -> ProviderId {
        ProviderId::new("copilot")
    }

    fn capabilities(&self) -> Capabilities {
        capabilities()
    }

    /// Starts the client. A session made ahead is not kept: Copilot creates the session's folder
    /// in its state at once, so the new tab only finds the client running.
    fn warm(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        self.backend.warm()
    }

    fn status(&self) -> BoxFuture<'_, AdapterStatus> {
        Box::pin(async move { status_from(self.backend.status().await) })
    }

    fn list_models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
        self.backend.models()
    }

    fn subscribe(&self) -> BoxFuture<'_, anyhow::Result<Updates>> {
        Box::pin(async move {
            self.updates
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| anyhow::anyhow!("subscribe was already called"))
        })
    }

    fn create(&self, req: CreateSession) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
        Box::pin(async move {
            let st = self.state(&req.radar_session_id);
            self.backend
                .create(
                    &req.radar_session_id,
                    &req.cwd,
                    req.model.as_deref(),
                    &req.setup,
                    self.sink(&st),
                )
                .await?;
            self.report_commands(&req.radar_session_id).await;
            Ok(ProviderSessionId::new(req.radar_session_id.as_str()))
        })
    }

    fn send(&self, session: &RadarSessionId, input: SendInput) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            if let Some(why) = refuse_remote(&input.text) {
                anyhow::bail!(why);
            }
            if !self.shared.connected.load(Ordering::SeqCst) {
                anyhow::bail!("the connection to copilot is lost; resume the session first");
            }
            // The server does not read a slash in a message: a command runs through its own call.
            if self.run_command(&session, &input.text).await? {
                return Ok(());
            }
            self.backend.send(&session, &input.text).await
        })
    }

    fn stop(&self, session: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move { self.backend.abort(&session).await })
    }

    fn resume(&self, req: ResumeSession) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            let st = self.state(&req.radar_session_id);
            // The persisted history is replayed whole (Copilot's own events, in its order)
            // and the store drops what it already holds by provider event id, so the
            // cursor needs no special handling here.
            let sink = self.sink(&st);
            self.backend
                .resume(&req.radar_session_id, &req.cwd, &req.setup, sink.clone())
                .await?;
            // The history ends where the CLI that ran it stopped, perhaps mid-turn, and it keeps no
            // `session.idle` (live only). A session just resumed runs nothing: say so after it, or the
            // replayed turn would look as if it were still running. Its id comes from the last event
            // replayed, so a second resume of the same history says it again under the same id.
            let last = st.last_persisted.lock().unwrap().clone();
            if let Some(last) = last {
                self.shared.end_of_history(&st, &last);
            }
            sink.restored();
            self.report_commands(&req.radar_session_id).await;
            Ok(())
        })
    }

    fn list_sessions(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        self.backend.list()
    }

    fn delete_session(&self, session: &ProviderSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move { self.backend.delete(session.as_str()).await })
    }

    fn start_login(
        &self,
        host: Option<String>,
    ) -> BoxFuture<'_, anyhow::Result<Box<dyn LoginHandle>>> {
        Box::pin(async move {
            let provider = self.provider.lock().unwrap().clone();
            let program = crate::login::resolve_program(provider.executable.as_deref())
                .ok_or_else(|| anyhow::anyhow!("copilot is not installed"))?;
            // Sign-in goes to the same home the sessions use.
            let home = provider.config_dir.or_else(|| self.login_home.clone());
            let flow = LoginFlow::start(&program, host.as_deref(), home.as_deref())?;
            Ok(Box::new(flow) as Box<dyn LoginHandle>)
        })
    }

    fn configure(&self, setup: ProviderSetup) {
        *self.provider.lock().unwrap() = setup.clone();
        if let Some(note) = self.backend.configure(setup) {
            (self.log)(&note);
        }
    }

    fn catalog(
        &self,
        cwd: PathBuf,
        setup: SessionSetup,
    ) -> BoxFuture<'_, anyhow::Result<Catalog>> {
        Box::pin(async move { self.backend.catalog(&cwd, &setup).await })
    }

    fn answer_permission(
        &self,
        session: &RadarSessionId,
        answer: PermissionAnswer,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        let st = self.state(session);
        Box::pin(async move {
            let w = st.waiter(&answer.request_id);
            // Idempotent: the first answer stands; a repeat changes nothing.
            w.send_if_modified(|v| {
                if v.is_none() {
                    *v = Some(answer.decision.clone());
                    true
                } else {
                    false
                }
            });
            Ok(())
        })
    }

    fn answer_question(
        &self,
        session: &RadarSessionId,
        answer: QuestionAnswer,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        let st = self.state(session);
        Box::pin(async move {
            // Idempotent, as permission answers are: the first answer stands.
            st.question_waiter(&answer.request_id).send_if_modified(|v| {
                if v.is_none() {
                    *v = Some(answer.decision.clone());
                    true
                } else {
                    false
                }
            });
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(ty: &str, data: serde_json::Value) -> RawEvent {
        RawEvent {
            id: format!("{ty}-1"),
            timestamp: String::new(),
            parent_id: None,
            ephemeral: false,
            agent_id: None,
            event_type: ty.into(),
            data,
        }
    }

    #[tokio::test]
    async fn a_question_handler_that_runs_before_its_event_waits_for_the_id_and_a_settled_one_is_never_claimed() {
        let st = Arc::new(SessionState::new(RadarSessionId::new("g")));
        // A question asked and answered before (replayed history) is settled for good.
        st.note_question(&ev("user_input.requested", serde_json::json!({"requestId":"old","question":"Which?"})));
        st.note_question(&ev("user_input.completed", serde_json::json!({"requestId":"old","answer":"a"})));
        let s2 = st.clone();
        let claim = tokio::spawn(async move { s2.claim("Which?").await });
        tokio::task::yield_now().await;
        assert!(!claim.is_finished(), "the handler waits for the event that names the request");
        st.note_question(&ev("user_input.requested", serde_json::json!({"requestId":"q1","question":"Which?"})));
        assert_eq!(claim.await.unwrap().as_deref(), Some("q1"));
    }
}
