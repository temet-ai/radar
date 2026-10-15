//! The Codex adapter: `codex app-server` as a radar-sessions [`Adapter`].
//!
//! An ordinary thread takes the operator's own Codex settings: no approval
//! policy or sandbox is sent, so `~/.codex/config.toml` (its profile, the
//! project's `trust_level`, its execpolicy rules and prefixes) decides what
//! Codex asks about, as it does in a terminal. With nothing set Codex asks
//! (`on-request`, a read-only sandbox in an untrusted folder); a policy that
//! never asks is the operator's own, and the session says so. A review
//! session keeps radar's strict [`SessionPolicy`] whatever the config says. A
//! permission request waits until the dock answers it; nothing here answers
//! for the operator.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use radar_sessions::adapter::{Catalog, ProviderSetup, SessionSetup, Updates};
use radar_sessions::{
    Adapter, AdapterStatus, AuthStatus, BoxFuture, Capabilities, CreateSession, Event, EventKind,
    RadarSessionId, ModelInfo, PermissionAnswer, PermissionDecision, ProviderEventId, ProviderId,
    ProviderSessionId, ProviderSessionInfo, QuestionAnswer, QuestionDecision, ResumeSession,
    SendInput, SlashCommand, Unsupported, Update,
};
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::commands::{self, Cmd, Run};
use crate::map::{self, Mapped, RequestKind};
use crate::rpc::{self, Conn, Incoming};
use crate::setup::{self, SkillStep};
use crate::wire::{ChildSpawner, Spawner};

/// What a thread may do. Sent on every thread start and resume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPolicy {
    /// `untrusted` (ask for everything not known safe), `on-request` or `never`.
    pub approval_policy: String,
    /// `read-only`, `workspace-write` or `danger-full-access`.
    pub sandbox: String,
}

impl Default for SessionPolicy {
    /// Ask about anything Codex does not know to be safe; writes stay inside
    /// the working directory.
    fn default() -> Self {
        Self {
            approval_policy: "untrusted".into(),
            sandbox: "workspace-write".into(),
        }
    }
}

impl SessionPolicy {
    /// A reviewer: asks before anything unknown, and cannot write at all.
    pub fn read_only() -> Self {
        Self {
            approval_policy: "untrusted".into(),
            sandbox: "read-only".into(),
        }
    }
}

pub struct CodexConfig {
    /// radar's own policy: a review session's, and every session's when
    /// [`Self::user_policy`] is off.
    pub policy: SessionPolicy,
    /// An ordinary session takes the operator's own Codex config (approval
    /// policy, sandbox, project trust) instead of [`Self::policy`].
    pub user_policy: bool,
    /// How long a request may go unanswered before it fails.
    pub request_timeout: Duration,
    /// Unix seconds, injected so tests need no wall clock.
    pub clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl Default for CodexConfig {
    fn default() -> Self {
        Self {
            policy: SessionPolicy::default(),
            user_policy: true,
            request_timeout: Duration::from_secs(60),
            clock: Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            }),
        }
    }
}

struct Sess {
    thread: String,
    provider: ProviderSessionId,
    seq: u64,
    turn: Option<String>,
    /// radar's policy for the thread; `None`: the operator's own Codex config.
    policy: Option<SessionPolicy>,
    cwd: PathBuf,
    /// What the session was set up with, sent again whenever it is resumed.
    setup: SessionSetup,
    /// The connection this thread is loaded in; a new process must resume it.
    loaded_in: u64,
    /// Changes announced by a file-change item, shown with its approval.
    file_changes: HashMap<String, Value>,
    /// What `/` offers: the built-ins, then the skills once `skills/list` has answered.
    commands: Vec<Cmd>,
}

struct Perm {
    session: RadarSessionId,
    kind: RequestKind,
    available: Vec<String>,
    requested: Value,
    rpc_ids: Vec<Value>,
    generation: u64,
    decided: Option<map::Answer>,
}

/// A question (`item/tool/requestUserInput`) waiting for the operator.
struct Ask {
    session: RadarSessionId,
    rpc_ids: Vec<Value>,
    generation: u64,
    /// The result sent, once the operator answered: a repeat of the request gets it too.
    sent: Option<Value>,
    answered: bool,
}

#[derive(Default)]
struct State {
    asks: HashMap<String, Ask>,
    /// Questions settled (answered, declined, resolved by Codex): a repeat answer is a no-op.
    asks_done: HashSet<String>,
    conn: Option<Arc<Conn>>,
    generation: u64,
    /// The process died and has not come back: calls fail until it does.
    lost: bool,
    sessions: HashMap<RadarSessionId, Sess>,
    by_thread: HashMap<String, RadarSessionId>,
    perms: HashMap<String, Perm>,
    answered: HashSet<String>,
    /// The skill folders last given to the app-server, and for which process.
    roots_applied: Option<(u64, BTreeSet<PathBuf>)>,
    /// Where Codex is and keeps its state, as last configured.
    provider_setup: ProviderSetup,
    /// The setup changed: the next use starts a new app-server once no
    /// session is mid-turn.
    restart_pending: bool,
}

struct Inner {
    spawner: Box<dyn Spawner>,
    cfg: CodexConfig,
    state: Mutex<State>,
    /// One connection attempt at a time.
    connecting: tokio::sync::Mutex<()>,
    /// The app-server's skill folders are one list for every thread; whoever
    /// sets them holds this until its thread has started or its listing is read.
    roots_lock: tokio::sync::Mutex<()>,
    updates_tx: mpsc::UnboundedSender<Update>,
    updates_rx: Mutex<Option<Updates>>,
}

pub struct CodexAdapter {
    inner: Arc<Inner>,
}

const PROVIDER: &str = "codex";

/// What Codex can and cannot do through radar.
pub fn capabilities() -> Capabilities {
    let no = |control: &str, reason: &str| Unsupported {
        control: control.into(),
        reason: reason.into(),
    };
    Capabilities {
        resume: true,
        stop: true,
        steer: true,
        queue: false,
        list_sessions: true,
        delete_session: true,
        list_models: true,
        permissions: true,
        questions: true,
        structured_events: true,
        slash_commands: vec![],
        // Codex lists skills (`skills/list`) and takes extra skill folders; it
        // has no named agents to list or select.
        agents: false,
        skills: true,
        unsupported: vec![
            no(
                "agents",
                "Codex has no named custom agents the app-server can list or select; a session runs as plain Codex",
            ),
            no(
                "instruction folders",
                "Codex reads AGENTS.md from the project and its own home only; it takes no extra folders of instruction files",
            ),
            no(
                "plugin folders",
                "Codex installs plugins from a marketplace into its own home; it cannot load a plugin folder for one session",
            ),
            no(
                "project configuration off",
                "turning it off skips the project's AGENTS.md; Codex's config.toml, global AGENTS.md, MCP servers and hooks still load",
            ),
            no(
                "Codex's own slash commands",
                "they belong to its terminal UI and the app-server has none to list; radar offers /compact, /review and the session's skills",
            ),
            no("cloud tasks", "radar runs Codex locally only"),
            no(
                "elicitations",
                "an MCP server's form or a dynamic tool call is refused, not guessed",
            ),
        ],
    }
}

/// The command that signs Codex in, for a terminal tab (Settings → Agents).
/// The device-code form works in a terminal without opening a local callback
/// server. Credentials stay in Codex's own store.
pub fn login_command() -> (String, Vec<String>) {
    ("codex".into(), vec!["login".into(), "--device-auth".into()])
}

/// `account/read` → what Settings shows. Never anything secret.
pub fn auth_from_account(result: &Value) -> AuthStatus {
    match result.get("account") {
        Some(a) if a.is_object() => match a.get("type").and_then(Value::as_str) {
            Some("chatgpt") => AuthStatus::SignedIn {
                as_user: a
                    .get("email")
                    .and_then(Value::as_str)
                    .unwrap_or("ChatGPT account")
                    .to_string(),
            },
            Some("apiKey") => AuthStatus::SignedIn {
                as_user: "API key".into(),
            },
            Some("amazonBedrock") => AuthStatus::SignedIn {
                as_user: "Amazon Bedrock".into(),
            },
            _ => AuthStatus::Unknown,
        },
        _ if result.get("requiresOpenaiAuth").and_then(Value::as_bool) == Some(true) => {
            AuthStatus::SignedOut
        }
        _ => AuthStatus::Unknown,
    }
}

impl CodexAdapter {
    /// Drive the `codex` on `PATH`.
    pub fn new() -> Self {
        Self::with(Box::new(ChildSpawner::default()), CodexConfig::default())
    }

    pub fn with(spawner: Box<dyn Spawner>, cfg: CodexConfig) -> Self {
        let (updates_tx, updates_rx) = mpsc::unbounded_channel();
        Self {
            inner: Arc::new(Inner {
                spawner,
                cfg,
                state: Mutex::new(State::default()),
                connecting: tokio::sync::Mutex::new(()),
                roots_lock: tokio::sync::Mutex::new(()),
                updates_tx,
                updates_rx: Mutex::new(Some(updates_rx)),
            }),
        }
    }

    /// Create a session with a policy other than the configured default (a
    /// reviewer is `SessionPolicy::read_only()`).
    pub async fn create_with(
        &self,
        req: CreateSession,
        policy: Option<SessionPolicy>,
    ) -> anyhow::Result<ProviderSessionId> {
        let inner = &self.inner;
        let conn = inner.ensure_conn().await?;
        let policy = policy.or_else(|| inner.cfg.policy_for(&req.setup));
        let users = policy.is_none();
        let mut params = json!({ "cwd": req.cwd });
        put_policy(&mut params, policy.as_ref());
        if let Some(m) = &req.model {
            params["model"] = json!(m);
        }
        if let Some(config) = setup::thread_config(policy.as_ref(), &req.setup) {
            params["config"] = config;
        }
        // Skill folders and named skills are in place before the thread starts.
        let _roots = inner.roots_lock.lock().await;
        let mut notes = inner
            .prepare_skills(&conn, &req.cwd, &req.setup)
            .await;
        let resp = conn
            .request("thread/start", params, inner.cfg.request_timeout)
            .await?;
        let thread = resp
            .get("thread")
            .ok_or_else(|| anyhow::anyhow!("Codex answered thread/start without a thread"))?;
        let id = map::str_at(thread, "id")
            .ok_or_else(|| anyhow::anyhow!("Codex gave the thread no id"))?
            .to_string();
        let provider = ProviderSessionId::new(id.clone());
        {
            let mut st = inner.state.lock().unwrap();
            st.by_thread.insert(id.clone(), req.radar_session_id.clone());
            st.sessions.insert(
                req.radar_session_id.clone(),
                Sess {
                    thread: id,
                    provider: provider.clone(),
                    seq: 0,
                    turn: None,
                    policy,
                    cwd: req.cwd.clone(),
                    setup: req.setup.clone(),
                    loaded_in: conn.generation,
                    file_changes: HashMap::new(),
                    commands: Vec::new(),
                },
            );
            notes.extend(setup::unsupported_notices(&req.setup));
            if users {
                notes.extend(never_asks(&resp));
            }
            inner.notices(&mut st, &req.radar_session_id, notes);
            if let Some(model) = map::str_at(&resp, "model") {
                inner.emit(
                    &mut st,
                    &req.radar_session_id,
                    Mapped::ephemeral(
                        None,
                        EventKind::ModelChanged {
                            model: model.into(),
                        },
                    ),
                );
            }
        }
        inner.report_commands(&req.radar_session_id, &conn, &req.cwd);
        Ok(provider)
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl Inner {
    /// Tell the part what `/` offers in this session: the built-ins now, and with the skills
    /// as soon as Codex lists them (not awaited: listing scans the disk).
    fn report_commands(self: &Arc<Self>, radar: &RadarSessionId, conn: &Arc<Conn>, cwd: &std::path::Path) {
        self.set_commands(radar, commands::all(vec![]));
        let (inner, radar, conn, cwd) = (Arc::clone(self), radar.clone(), Arc::clone(conn), cwd.to_path_buf());
        tokio::spawn(async move {
            let asked = conn
                .request("skills/list", json!({ "cwds": [cwd], "forceReload": false }), inner.cfg.request_timeout)
                .await;
            let Ok(listed) = asked else { return };
            let skills = commands::from_skills(&listed, &cwd);
            if !skills.is_empty() {
                inner.set_commands(&radar, commands::all(skills));
            }
        });
    }

    /// What `text` runs when it is `/name text` for one of the session's commands; `None` when
    /// it is anything else (a path, a sentence, a name Codex did not list), sent as written.
    fn command(&self, radar: &RadarSessionId, text: &str) -> Option<(Run, Option<String>)> {
        let (word, rest) = radar_sessions::parse_slash(text)?;
        let st = self.state.lock().unwrap();
        let cmd = commands::find(&st.sessions.get(radar)?.commands, word)?;
        Some((cmd.run.clone(), rest.map(str::to_string)))
    }

    /// Keep a session's commands and report them.
    fn set_commands(&self, radar: &RadarSessionId, cmds: Vec<Cmd>) {
        let offered: Vec<SlashCommand> = cmds.iter().map(|c| c.command.clone()).collect();
        {
            let mut st = self.state.lock().unwrap();
            let Some(s) = st.sessions.get_mut(radar) else { return };
            s.commands = cmds;
        }
        let _ = self.updates_tx.send(Update::Commands { session: radar.clone(), commands: offered });
    }

    /// Stamp a mapped event with its session, order and time, and send it.
    fn emit(&self, st: &mut State, radar: &RadarSessionId, m: Mapped) {
        let Some(s) = st.sessions.get_mut(radar) else {
            return;
        };
        s.seq += 1;
        let event = Event {
            radar_session_id: radar.clone(),
            provider: ProviderId::new(PROVIDER),
            provider_session_id: s.provider.clone(),
            provider_event_id: m.event_id.map(ProviderEventId::new),
            sequence: s.seq,
            observed_at: (self.cfg.clock)(),
            kind: m.kind,
            durability: m.durability,
            agent: None,
            parent: m.parent.map(ProviderEventId::new),
            payload: m.payload,
        };
        let _ = self.updates_tx.send(Update::Event(event));
    }

    /// The live connection, started on first use. While a lost process is
    /// being brought back, calls fail rather than quietly starting another.
    async fn ensure_conn(self: &Arc<Self>) -> anyhow::Result<Arc<Conn>> {
        {
            let mut st = self.state.lock().unwrap();
            // A new executable or CODEX_HOME: start the app-server again, but
            // only between turns. The sessions resume themselves in it.
            if st.restart_pending
                && st.perms.is_empty()
                && st.sessions.values().all(|s| s.turn.is_none())
            {
                st.restart_pending = false;
                if let Some(old) = st.conn.take() {
                    old.close();
                }
            }
            if let Some(c) = &st.conn {
                return Ok(Arc::clone(c));
            }
            if st.lost {
                anyhow::bail!("Codex is not connected; radar is reconnecting");
            }
        }
        self.connect().await
    }

    /// Start `codex app-server` and shake hands.
    async fn connect(self: &Arc<Self>) -> anyhow::Result<Arc<Conn>> {
        let _one_at_a_time = self.connecting.lock().await;
        if let Some(c) = self.state.lock().unwrap().conn.clone() {
            return Ok(c);
        }
        let wire = self.spawner.spawn().await?;
        let generation = {
            let mut st = self.state.lock().unwrap();
            st.generation += 1;
            st.generation
        };
        let conn = Arc::new(Conn::new(generation, wire.tx));
        let mut rx = wire.rx;
        // The reader must not keep the adapter or its pipe alive: dropping the
        // adapter drops the connection, which ends the child.
        let inner = Arc::downgrade(self);
        let weak_conn = Arc::downgrade(&conn);
        tokio::spawn(async move {
            while let Some(line) = rx.recv().await {
                let (Some(inner), Some(conn)) = (inner.upgrade(), weak_conn.upgrade()) else {
                    return;
                };
                inner.on_line(&conn, &line);
            }
            if let (Some(inner), Some(conn)) = (inner.upgrade(), weak_conn.upgrade()) {
                inner.on_closed(&conn);
            }
        });
        let hello = json!({ "clientInfo": { "name": "radar", "title": "radar", "version": env!("CARGO_PKG_VERSION") } });
        if let Err(e) = conn
            .request("initialize", hello, self.cfg.request_timeout)
            .await
        {
            conn.close();
            return Err(e);
        }
        conn.notify("initialized");
        self.state.lock().unwrap().conn = Some(Arc::clone(&conn));
        Ok(conn)
    }

    /// After a loss: try again with growing pauses until the process is back,
    /// then say so. Ends when the adapter is dropped.
    fn reconnect_in_background(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut pause = Duration::from_secs(1);
            loop {
                tokio::time::sleep(pause).await;
                let Some(inner) = weak.upgrade() else { return };
                if inner.connect().await.is_ok() {
                    inner.state.lock().unwrap().lost = false;
                    let _ = inner.updates_tx.send(Update::Reconnected);
                    return;
                }
                pause = (pause * 2).min(Duration::from_secs(30));
            }
        });
    }

    fn on_line(&self, conn: &Arc<Conn>, line: &str) {
        match rpc::parse(line) {
            Some(Incoming::Response { id, reply }) => conn.resolve(id, reply),
            Some(Incoming::Notification { method, params }) => {
                self.on_notification(&method, &params)
            }
            Some(Incoming::Request { id, method, params }) => {
                self.on_request(conn, id, &method, &params)
            }
            None => {}
        }
    }

    fn session_of(st: &State, params: &Value) -> Option<RadarSessionId> {
        map::str_at(params, "threadId")
            .and_then(|t| st.by_thread.get(t))
            .cloned()
    }

    fn on_notification(&self, method: &str, params: &Value) {
        let mut st = self.state.lock().unwrap();
        let Some(radar) = Self::session_of(&st, params) else {
            return;
        };
        match method {
            "turn/started" => {
                let turn = params
                    .get("turn")
                    .and_then(|t| map::str_at(t, "id"))
                    .map(str::to_string);
                if let Some(s) = st.sessions.get_mut(&radar) {
                    s.turn = turn;
                }
            }
            "turn/completed" => {
                if let Some(s) = st.sessions.get_mut(&radar) {
                    s.turn = None;
                }
            }
            "item/started" => {
                let item = params.get("item");
                if item.and_then(|i| map::str_at(i, "type")) == Some("fileChange")
                    && let (Some(id), Some(changes)) = (
                        item.and_then(|i| map::str_at(i, "id")),
                        item.and_then(|i| i.get("changes")),
                    )
                    && let Some(s) = st.sessions.get_mut(&radar)
                {
                    s.file_changes.insert(id.to_string(), changes.clone());
                }
            }
            "serverRequest/resolved" => {
                let rid = params.get("requestId").cloned().unwrap_or(Value::Null);
                let asked = st
                    .asks
                    .iter()
                    .find(|(_, a)| a.session == radar && a.rpc_ids.contains(&rid))
                    .map(|(k, _)| k.clone());
                if let Some(key) = asked
                    && let Some(a) = st.asks.remove(&key)
                {
                    st.asks_done.insert(key.clone());
                    // Codex does not say what was answered; the store shows the operator's own replies.
                    let replies = a.answered.then(Vec::new);
                    self.emit(
                        &mut st,
                        &radar,
                        Mapped::ephemeral(
                            Some(format!("ask:{key}:resolved")),
                            EventKind::QuestionResolved { request_id: key, replies },
                        ),
                    );
                    return;
                }
                let found = st
                    .perms
                    .iter()
                    .find(|(_, p)| p.session == radar && p.rpc_ids.contains(&rid))
                    .map(|(k, _)| k.clone());
                if let Some(key) = found
                    && let Some(p) = st.perms.remove(&key)
                {
                    st.answered.insert(key.clone());
                    self.emit(
                        &mut st,
                        &radar,
                        Mapped::ephemeral(
                            Some(format!("perm:{key}:resolved")),
                            EventKind::PermissionResolved {
                                request_id: key,
                                approved: p.decided.is_some_and(map::Answer::approved),
                            },
                        ),
                    );
                }
                return;
            }
            _ => {}
        }
        for m in map::notification(method, params) {
            self.emit(&mut st, &radar, m);
        }
    }

    /// Codex asks the operator questions. One entry per request, whatever number of
    /// times it is delivered; it waits until the dock answers or declines it.
    fn on_question(&self, st: &mut State, conn: &Arc<Conn>, id: Value, params: &Value) {
        let session = Self::session_of(st, params);
        let (Some(radar), Some(request)) = (session, map::question_request(params)) else {
            conn.respond_error(&id, "radar does not know this thread or question");
            return;
        };
        let key = request.request_id.clone();
        if let Some(a) = st.asks.get_mut(&key) {
            match &a.sent {
                Some(result) => {
                    conn.respond(&id, result.clone());
                }
                None => a.rpc_ids.push(id),
            }
            return;
        }
        if st.asks_done.contains(&key) {
            // Settled already; a late repeat hears nothing was answered.
            conn.respond(&id, map::question_result(None));
            return;
        }
        st.asks.insert(
            key.clone(),
            Ask {
                session: radar.clone(),
                rpc_ids: vec![id],
                generation: conn.generation,
                sent: None,
                answered: false,
            },
        );
        self.emit(
            st,
            &radar,
            Mapped::ephemeral(Some(format!("ask:{key}")), EventKind::QuestionAsked { request }),
        );
    }

    fn on_request(&self, conn: &Arc<Conn>, id: Value, method: &str, params: &Value) {
        let mut st = self.state.lock().unwrap();
        if method == map::ASK_METHOD {
            return self.on_question(&mut st, conn, id, params);
        }
        let session = Self::session_of(&st, params);
        let changes = session.as_ref().and_then(|g| {
            let item = map::str_at(params, "itemId")?;
            st.sessions.get(g)?.file_changes.get(item).cloned()
        });
        let Some((kind, request, available)) =
            map::permission_request(method, params, changes.as_ref())
        else {
            // A question radar cannot show (typed input, an elicitation, a dynamic
            // tool): refuse it, say so, never invent an answer.
            conn.respond_error(&id, "radar does not handle this request");
            if let Some(g) = session {
                self.emit(
                    &mut st,
                    &g,
                    Mapped::ephemeral(None, EventKind::Notice {
                        text: format!("Codex asked for `{method}`, which radar does not support; it was refused."),
                    }),
                );
            }
            return;
        };
        let Some(radar) = session else {
            conn.respond_error(&id, "radar does not know this thread");
            return;
        };
        let key = request.request_id.clone();
        // The same request delivered twice: one entry, answered once, both
        // ids replied to.
        if let Some(p) = st.perms.get_mut(&key) {
            match p.decided {
                Some(answer) => {
                    let result = map::decision_result(p.kind, answer, &p.available, &p.requested);
                    conn.respond(&id, result);
                }
                None => p.rpc_ids.push(id),
            }
            return;
        }
        if st.answered.contains(&key) {
            // Answered and resolved already; a late repeat gets a refusal.
            conn.respond(
                &id,
                map::decision_result(kind, map::Answer::Refuse, &available, &Value::Null),
            );
            return;
        }
        let requested = params.get("permissions").cloned().unwrap_or(Value::Null);
        st.perms.insert(
            key.clone(),
            Perm {
                session: radar.clone(),
                kind,
                available,
                requested,
                rpc_ids: vec![id],
                generation: conn.generation,
                decided: None,
            },
        );
        let event_id = format!("perm:{key}");
        self.emit(
            &mut st,
            &radar,
            Mapped::ephemeral(Some(event_id), EventKind::PermissionRequested { request }),
        );
    }

    /// Say something in a session, once, to the operator.
    fn notices(&self, st: &mut State, radar: &RadarSessionId, texts: Vec<String>) {
        for text in texts {
            self.emit(st, radar, Mapped::ephemeral(None, EventKind::Notice { text }));
        }
    }

    /// Make the app-server's extra skill folders those every session asked
    /// for, plus `extra`. Holds nothing; the caller holds `roots_lock`.
    async fn sync_skill_roots(&self, conn: &Conn, extra: &[PathBuf]) -> anyhow::Result<()> {
        let want: BTreeSet<PathBuf> = {
            let st = self.state.lock().unwrap();
            st.sessions
                .values()
                .flat_map(|s| s.setup.skill_dirs.iter())
                .chain(extra)
                .cloned()
                .collect()
        };
        let have = {
            let st = self.state.lock().unwrap();
            st.roots_applied
                .as_ref()
                .filter(|(generation, _)| *generation == conn.generation)
                .map(|(_, roots)| roots.clone())
                .unwrap_or_default()
        };
        if have == want {
            return Ok(());
        }
        conn.request(
            "skills/extraRoots/set",
            json!({ "extraRoots": want }),
            self.cfg.request_timeout,
        )
        .await?;
        self.state.lock().unwrap().roots_applied = Some((conn.generation, want));
        Ok(())
    }

    /// Before a thread starts: load the setup's skill folders, then make sure
    /// each skill it names is there and switched on. What cannot be done comes
    /// back as notices for the session.
    async fn prepare_skills(&self, conn: &Conn, cwd: &Path, setup: &SessionSetup) -> Vec<String> {
        let mut notes = Vec::new();
        if let Err(e) = self.sync_skill_roots(conn, &setup.skill_dirs).await {
            notes.push(format!("The skill folders of this setup were not loaded: {e}"));
        }
        if setup.skills.is_empty() {
            return notes;
        }
        let listed = conn
            .request(
                "skills/list",
                json!({ "cwds": [cwd], "forceReload": true }),
                self.cfg.request_timeout,
            )
            .await;
        let rows = match listed {
            Ok(r) => setup::skill_rows(&r),
            Err(e) => {
                notes.push(format!("Could not check the skills this session names: {e}"));
                return notes;
            }
        };
        for step in setup::skill_steps(&setup.skills, &rows) {
            match step {
                SkillStep::Missing(name) => notes.push(format!(
                    "Codex has no skill named `{name}`; the brief names it, but Codex cannot load it."
                )),
                // Switching it on would write the operator's own Codex config
                // for good; radar never changes a CLI's config, it says so.
                SkillStep::Enable { name, path } => notes.push(format!(
                    "Skill `{name}` is switched off in Codex's own config ({}); switch it on there to use it.",
                    path.display()
                )),
            }
        }
        notes
    }

    /// The process ended: everything pending on it is lost.
    fn on_closed(self: &Arc<Self>, conn: &Arc<Conn>) {
        conn.close();
        let mut st = self.state.lock().unwrap();
        if !st
            .conn
            .as_ref()
            .is_some_and(|c| c.generation == conn.generation)
        {
            return;
        }
        st.conn = None;
        st.lost = true;
        let lost: Vec<(String, Perm)> = st.perms.drain().collect();
        for (key, p) in lost {
            self.emit(
                &mut st,
                &p.session,
                Mapped::ephemeral(
                    Some(format!("perm:{key}:lost")),
                    EventKind::PermissionResolved {
                        request_id: key,
                        approved: false,
                    },
                ),
            );
        }
        let lost: Vec<(String, Ask)> = st.asks.drain().collect();
        for (key, a) in lost {
            st.asks_done.insert(key.clone());
            self.emit(
                &mut st,
                &a.session,
                Mapped::ephemeral(
                    Some(format!("ask:{key}:lost")),
                    EventKind::QuestionResolved { request_id: key, replies: None },
                ),
            );
        }
        for s in st.sessions.values_mut() {
            s.turn = None;
        }
        let _ = self.updates_tx.send(Update::Disconnected {
            reason: "Codex app-server exited".into(),
        });
        drop(st);
        self.reconnect_in_background();
    }

    /// The thread's connection and ids, resuming it first when a new process
    /// has not loaded it yet.
    async fn loaded(self: &Arc<Self>, radar: &RadarSessionId) -> anyhow::Result<(Arc<Conn>, String)> {
        let conn = self.ensure_conn().await?;
        let (thread, stale, policy, cwd, setup) = {
            let st = self.state.lock().unwrap();
            let s = st
                .sessions
                .get(radar)
                .ok_or_else(|| anyhow::anyhow!("unknown session {radar}"))?;
            (
                s.thread.clone(),
                s.loaded_in != conn.generation,
                s.policy.clone(),
                s.cwd.clone(),
                s.setup.clone(),
            )
        };
        if stale {
            let params = resume_params(&thread, &cwd, policy.as_ref(), &setup);
            let _roots = self.roots_lock.lock().await;
            if let Err(e) = self.sync_skill_roots(&conn, &[]).await {
                let mut st = self.state.lock().unwrap();
                let text = format!("The skill folders of this setup were not loaded: {e}");
                self.notices(&mut st, radar, vec![text]);
            }
            conn.request("thread/resume", params, self.cfg.request_timeout)
                .await?;
            if let Some(s) = self.state.lock().unwrap().sessions.get_mut(radar) {
                s.loaded_in = conn.generation;
            }
        }
        Ok((conn, thread))
    }
}

impl CodexConfig {
    /// radar's policy for a session with this setup: a review keeps radar's own;
    /// `None` leaves an ordinary session to the operator's Codex config.
    pub fn policy_for(&self, setup: &SessionSetup) -> Option<SessionPolicy> {
        (setup.review || !self.user_policy).then(|| self.policy.clone())
    }
}

/// radar's policy on a `thread/start` or `thread/resume`; nothing for the
/// operator's own config.
fn put_policy(params: &mut Value, policy: Option<&SessionPolicy>) {
    if let Some(p) = policy {
        params["approvalPolicy"] = json!(p.approval_policy);
        params["sandbox"] = json!(p.sandbox);
    }
}

/// A notice when the operator's own config runs the thread without asking
/// (`approvalPolicy` `never` in the `thread/start` answer): the dock will see
/// no permission request from it.
fn never_asks(resp: &Value) -> Option<String> {
    (map::str_at(resp, "approvalPolicy") == Some("never")).then(|| {
        let sandbox = resp
            .pointer("/sandbox/type")
            .and_then(Value::as_str)
            .unwrap_or("its configured sandbox");
        format!(
            "Codex runs with your own settings: approval policy `never`, sandbox `{sandbox}`. It will not ask before it runs a command or writes a file."
        )
    })
}

fn resume_params(
    thread: &str,
    cwd: &std::path::Path,
    policy: Option<&SessionPolicy>,
    setup: &SessionSetup,
) -> Value {
    let mut params = json!({
        "threadId": thread,
        "cwd": cwd,
    });
    put_policy(&mut params, policy);
    if let Some(config) = setup::thread_config(policy, setup) {
        params["config"] = config;
    }
    params
}

impl Adapter for CodexAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::new(PROVIDER)
    }

    fn capabilities(&self) -> Capabilities {
        capabilities()
    }

    fn status(&self) -> BoxFuture<'_, AdapterStatus> {
        Box::pin(async move {
            let inner = &self.inner;
            let mut status = AdapterStatus {
                installed: false,
                version: None,
                protocol: None,
                auth: AuthStatus::Unknown,
                error: None,
                capabilities: capabilities(),
            };
            let Some(version) = inner.spawner.version().await else {
                status.error = Some("`codex` was not found on PATH; install the Codex CLI".into());
                return status;
            };
            status.installed = true;
            status.version = Some(version);
            status.protocol = Some("app-server v2 (JSON-RPC)".into());
            match inner.ensure_conn().await {
                Err(e) => status.error = Some(e.to_string()),
                Ok(conn) => {
                    match conn
                        .request(
                            "account/read",
                            json!({ "refreshToken": false }),
                            inner.cfg.request_timeout,
                        )
                        .await
                    {
                        Ok(r) => status.auth = auth_from_account(&r),
                        Err(e) => status.error = Some(e.to_string()),
                    }
                }
            }
            status
        })
    }

    fn list_models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let inner = &self.inner;
            let conn = inner.ensure_conn().await?;
            let mut out = Vec::new();
            let mut cursor: Option<String> = None;
            loop {
                let r = conn
                    .request(
                        "model/list",
                        json!({ "limit": 100, "cursor": cursor }),
                        inner.cfg.request_timeout,
                    )
                    .await?;
                for m in r
                    .get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if m.get("hidden").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    let Some(id) = map::str_at(m, "id") else {
                        continue;
                    };
                    let name = map::str_at(m, "displayName").unwrap_or(id);
                    out.push(ModelInfo {
                        id: id.into(),
                        name: name.into(),
                    });
                }
                cursor = map::str_at(&r, "nextCursor").map(str::to_string);
                if cursor.is_none() {
                    return Ok(out);
                }
            }
        })
    }

    fn subscribe(&self) -> BoxFuture<'_, anyhow::Result<Updates>> {
        Box::pin(async move {
            self.inner
                .updates_rx
                .lock()
                .unwrap()
                .take()
                .ok_or_else(|| anyhow::anyhow!("the Codex adapter has one subscriber already"))
        })
    }

    fn create(&self, req: CreateSession) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
        Box::pin(self.create_with(req, None))
    }

    fn send(&self, session: &RadarSessionId, input: SendInput) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let inner = &self.inner;
            let (conn, thread) = inner.loaded(&session).await?;
            let turn = inner
                .state
                .lock()
                .unwrap()
                .sessions
                .get(&session)
                .and_then(|s| s.turn.clone());
            // The app-server reads no slash in a message: a command is the call it stands for.
            let command = inner.command(&session, &input.text);
            let content = match &command {
                Some((Run::Skill { name, path }, text)) => commands::skill_input(name, path, text.as_deref()),
                _ => json!([{ "type": "text", "text": input.text }]),
            };
            let resp = match (command, turn) {
                (Some((Run::Compact, _)), _) => {
                    conn.request("thread/compact/start", json!({ "threadId": thread }), inner.cfg.request_timeout)
                        .await?;
                    return Ok(());
                }
                (Some((Run::Review, instructions)), _) => {
                    conn.request(
                        "review/start",
                        json!({ "threadId": thread, "target": commands::review_target(instructions.as_deref()), "delivery": "inline" }),
                        inner.cfg.request_timeout,
                    )
                    .await?
                }
                // A turn is running: add to it.
                (_, Some(expected)) => {
                    conn.request(
                        "turn/steer",
                        json!({ "threadId": thread, "expectedTurnId": expected, "input": content }),
                        inner.cfg.request_timeout,
                    )
                    .await?
                }
                (_, None) => {
                    conn.request(
                        "turn/start",
                        json!({ "threadId": thread, "input": content }),
                        inner.cfg.request_timeout,
                    )
                    .await?
                }
            };
            let turn_id = resp
                .get("turn")
                .and_then(|t| map::str_at(t, "id"))
                .or_else(|| map::str_at(&resp, "turnId"))
                .map(str::to_string);
            // Only while that process is still the live one: a loss that
            // arrived first already ended the turn.
            let mut st = inner.state.lock().unwrap();
            let live = st
                .conn
                .as_ref()
                .is_some_and(|c| c.generation == conn.generation);
            if let (true, Some(id), Some(s)) = (live, turn_id, st.sessions.get_mut(&session)) {
                s.turn = Some(id);
            }
            Ok(())
        })
    }

    fn stop(&self, session: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let inner = &self.inner;
            let (conn, turn, thread) = {
                let st = inner.state.lock().unwrap();
                let s = st
                    .sessions
                    .get(&session)
                    .ok_or_else(|| anyhow::anyhow!("unknown session {session}"))?;
                (st.conn.clone(), s.turn.clone(), s.thread.clone())
            };
            // No running turn, or no process: nothing to stop.
            let (Some(conn), Some(turn)) = (conn, turn) else {
                return Ok(());
            };
            conn.request(
                "turn/interrupt",
                json!({ "threadId": thread, "turnId": turn }),
                inner.cfg.request_timeout,
            )
            .await?;
            Ok(())
        })
    }

    fn resume(&self, req: ResumeSession) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move {
            let inner = &self.inner;
            let conn = inner.ensure_conn().await?;
            let thread_id = req.provider_session_id.as_str().to_string();
            let _roots = inner.roots_lock.lock().await;
            let _ = inner.prepare_skills(&conn, &req.cwd, &req.setup).await;
            let policy = {
                let mut st = inner.state.lock().unwrap();
                st.by_thread
                    .insert(thread_id.clone(), req.radar_session_id.clone());
                let policy = inner.cfg.policy_for(&req.setup);
                let entry = st
                    .sessions
                    .entry(req.radar_session_id.clone())
                    .or_insert_with(|| Sess {
                        thread: thread_id.clone(),
                        provider: req.provider_session_id.clone(),
                        seq: 0,
                        turn: None,
                        policy,
                        cwd: req.cwd.clone(),
                        setup: req.setup.clone(),
                        loaded_in: 0,
                        file_changes: HashMap::new(),
                        commands: Vec::new(),
                    });
                entry.setup = req.setup.clone();
                entry.policy.clone()
            };
            let resp = conn
                .request(
                    "thread/resume",
                    resume_params(&thread_id, &req.cwd, policy.as_ref(), &req.setup),
                    inner.cfg.request_timeout,
                )
                .await?;
            let thread = resp
                .get("thread")
                .ok_or_else(|| anyhow::anyhow!("Codex answered thread/resume without a thread"))?;
            let mut st = inner.state.lock().unwrap();
            if let Some(s) = st.sessions.get_mut(&req.radar_session_id) {
                s.loaded_in = conn.generation;
                s.turn = map::running_turn(thread);
            }
            // The whole history again, with the ids the live events had: the
            // store keeps what it lacks and drops the rest.
            let mut replay = map::history(thread);
            // Only what comes after the last event radar has; a cursor the
            // provider no longer knows replays everything.
            if let Some(cursor) = &req.cursor
                && let Some(at) = replay
                    .iter()
                    .position(|m| m.event_id.as_deref() == Some(cursor.as_str()))
            {
                replay.drain(..=at);
            }
            for m in replay {
                inner.emit(&mut st, &req.radar_session_id, m);
            }
            // No notices here: a resume replays persisted events only (the
            // shared contract), and the session said what it could not take
            // when it was created.
            drop(st);
            inner.report_commands(&req.radar_session_id, &conn, &req.cwd);
            Ok(())
        })
    }

    fn list_sessions(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async move {
            let inner = &self.inner;
            let conn = inner.ensure_conn().await?;
            let r = conn
                .request(
                    "thread/list",
                    json!({ "limit": 50 }),
                    inner.cfg.request_timeout,
                )
                .await?;
            Ok(r.get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|t| {
                    Some(ProviderSessionInfo {
                        provider_session_id: ProviderSessionId::new(map::str_at(t, "id")?),
                        title: map::str_at(t, "name")
                            .or_else(|| map::str_at(t, "preview"))
                            .map(|s| map::bounded(s, 120)),
                        cwd: map::str_at(t, "cwd").map(str::to_string),
                        updated_at: t.get("updatedAt").and_then(Value::as_i64),
                    })
                })
                .collect())
        })
    }

    fn delete_session(&self, session: &ProviderSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let id = session.as_str().to_string();
        Box::pin(async move {
            let inner = &self.inner;
            let conn = inner.ensure_conn().await?;
            conn.request(
                "thread/delete",
                json!({ "threadId": id }),
                inner.cfg.request_timeout,
            )
            .await?;
            let mut st = inner.state.lock().unwrap();
            if let Some(radar) = st.by_thread.remove(&id) {
                st.sessions.remove(&radar);
            }
            Ok(())
        })
    }

    /// Starts `codex app-server` and shakes hands.
    fn warm(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move { self.inner.ensure_conn().await.map(|_| ()) })
    }

    /// A thread Codex starts and nobody writes to has no rollout and is not in `thread/list`
    /// (it leaves a shell snapshot and a lock file in its state folder, which Codex sweeps), so one
    /// made ahead is invisible in Codex's own session list.
    fn idle_session_is_free(&self) -> bool {
        true
    }

    /// Lets the app-server unload the thread and forgets it here.
    fn discard(&self, session: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let inner = &self.inner;
            let thread = {
                let mut st = inner.state.lock().unwrap();
                let thread = st.sessions.remove(&session).map(|s| s.thread);
                if let Some(t) = &thread {
                    st.by_thread.remove(t);
                }
                thread
            };
            if let Some(thread) = thread {
                let conn = inner.ensure_conn().await?;
                conn.request("thread/unsubscribe", json!({ "threadId": thread }), inner.cfg.request_timeout).await?;
            }
            Ok(())
        })
    }

    fn configure(&self, setup: ProviderSetup) {
        let mut st = self.inner.state.lock().unwrap();
        if st.provider_setup == setup {
            return;
        }
        self.inner.spawner.configure(&setup);
        st.provider_setup = setup;
        st.restart_pending = true;
    }

    fn catalog(
        &self,
        cwd: PathBuf,
        setup: SessionSetup,
    ) -> BoxFuture<'_, anyhow::Result<Catalog>> {
        Box::pin(async move {
            let inner = &self.inner;
            let conn = inner.ensure_conn().await?;
            let _roots = inner.roots_lock.lock().await;
            inner.sync_skill_roots(&conn, &setup.skill_dirs).await?;
            let listed = conn
                .request(
                    "skills/list",
                    json!({ "cwds": [cwd], "forceReload": true }),
                    inner.cfg.request_timeout,
                )
                .await;
            // The list is the app-server's own for every thread: put it back
            // to what the running sessions asked for.
            let _ = inner.sync_skill_roots(&conn, &[]).await;
            let rows = setup::skill_rows(&listed?);
            let others: Vec<PathBuf> = {
                let st = inner.state.lock().unwrap();
                st.sessions
                    .values()
                    .flat_map(|s| s.setup.skill_dirs.iter())
                    .filter(|d| !setup.skill_dirs.contains(d))
                    .cloned()
                    .collect()
            };
            Ok(Catalog {
                agents: vec![],
                skills: setup::catalog_skills(&rows, &setup.skill_dirs, &others),
            })
        })
    }

    fn answer_permission(
        &self,
        session: &RadarSessionId,
        answer: PermissionAnswer,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let mut st = self.inner.state.lock().unwrap();
            let key = answer.request_id;
            let conn = st.conn.clone();
            let Some(p) = st.perms.get_mut(&key) else {
                // Answered before, or resolved by Codex itself: a repeat is a no-op.
                if st.answered.contains(&key) {
                    return Ok(());
                }
                anyhow::bail!("no pending permission request {key}");
            };
            if p.session != session {
                anyhow::bail!("permission request {key} belongs to another session");
            }
            if p.decided.is_some() {
                return Ok(());
            }
            let Some(conn) = conn.filter(|c| c.generation == p.generation) else {
                anyhow::bail!("Codex restarted; the request {key} is gone");
            };
            let decided = match answer.decision {
                PermissionDecision::ApproveOnce => map::Answer::Accept,
                PermissionDecision::ApproveForSession => map::Answer::AcceptForSession,
                PermissionDecision::Deny { .. } => map::Answer::Refuse,
            };
            let approve = decided.approved();
            let result = map::decision_result(p.kind, decided, &p.available, &p.requested);
            p.decided = Some(decided);
            for id in &p.rpc_ids {
                conn.respond(id, result.clone());
            }
            // Codex offered no "decline" here, so a deny is a cancel and ends the
            // turn; say so, or the model just goes quiet.
            if !approve && result.get("decision").and_then(|d| d.as_str()) == Some("cancel") {
                self.inner.emit(
                    &mut st,
                    &session,
                    Mapped::ephemeral(
                        None,
                        EventKind::Notice {
                            text: "Codex offers no decline for this request; deny ended the turn"
                                .into(),
                        },
                    ),
                );
            }
            Ok(())
        })
    }

    fn answer_question(
        &self,
        session: &RadarSessionId,
        answer: QuestionAnswer,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let mut st = self.inner.state.lock().unwrap();
            let key = answer.request_id;
            let conn = st.conn.clone();
            let Some(a) = st.asks.get_mut(&key) else {
                // Answered before, or resolved by Codex itself: a repeat is a no-op.
                if st.asks_done.contains(&key) {
                    return Ok(());
                }
                anyhow::bail!("no pending question {key}");
            };
            if a.session != session {
                anyhow::bail!("question {key} belongs to another session");
            }
            if a.sent.is_some() {
                return Ok(());
            }
            let Some(conn) = conn.filter(|c| c.generation == a.generation) else {
                anyhow::bail!("Codex restarted; the question {key} is gone");
            };
            let (result, answered) = match &answer.decision {
                QuestionDecision::Answer { replies } => (map::question_result(Some(replies)), true),
                QuestionDecision::Decline { .. } => (map::question_result(None), false),
            };
            for id in &a.rpc_ids {
                conn.respond(id, result.clone());
            }
            a.sent = Some(result);
            a.answered = answered;
            Ok(())
        })
    }
}
