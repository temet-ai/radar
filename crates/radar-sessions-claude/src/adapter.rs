//! [`ClaudeAdapter`]: radar-sessions' [`Adapter`] over the Node sidecar.
//!
//! Credentials stay with Claude Code. The adapter never reads a token; sign-in status is what
//! `claude auth status` says (via the sidecar), and signing in is `claude auth login` in a PTY
//! tab (T8), so [`Adapter::start_login`] keeps its default.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{Result, anyhow};
use radar_sessions::{
    Adapter, AdapterStatus, AuthStatus, BoxFuture, Capabilities, CreateSession, RadarSessionId,
    ModelInfo, PermissionAnswer, PermissionDecision, ProviderId, ProviderSessionId, ProviderSessionInfo,
    QuestionAnswer, QuestionDecision, ResumeSession, SendInput, Unsupported, Update,
};
use radar_sessions::adapter::{Catalog, CatalogEntry, ProviderSetup, SessionSetup};
use radar_sessions::adapter::Updates;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::link::{Link, LinkMsg, ProcessSpawner, Spawned, Spawner};
use crate::map::{Mapper, SessionState, commands, provider};

/// Oldest Claude Code the sidecar's permission callbacks work with (`requestId` on `canUseTool`,
/// v2.1.199; the SDK is 0.3.296 and bundles 2.1.296).
pub const MIN_CLAUDE_CODE: (u64, u64, u64) = (2, 1, 199);

/// Standing choices from Settings → Agents, in Claude Code's rule syntax (`Bash(git diff:*)`).
#[derive(Clone, Debug, Default)]
pub struct ClaudeConfig {
    /// A `claude` to use instead of the one on `PATH`.
    pub claude_path: Option<String>,
    /// Tools approved without asking.
    pub allow: Vec<String>,
    /// Tools refused without asking.
    pub deny: Vec<String>,
    /// Tools that always ask, over every allow rule. `None`: the sidecar's own (`askRules`):
    /// none with project configuration on, so the operator's own Claude Code rules apply;
    /// `Bash` with it off; commands, writes and the network in a review session.
    pub ask: Option<Vec<String>>,
}

/// Stands in for a sidecar that cannot be started, so the adapter reports why.
struct Missing(String);

impl Spawner for Missing {
    fn spawn(&self) -> BoxFuture<'_, Result<Spawned>> {
        Box::pin(async move { Err(anyhow!("{}", self.0)) })
    }
}

type Sessions =Arc<StdMutex<HashMap<RadarSessionId, SessionState>>>;

pub struct ClaudeAdapter {
    link: Arc<Link>,
    msgs: StdMutex<Option<mpsc::UnboundedReceiver<LinkMsg>>>,
    out: StdMutex<Option<mpsc::UnboundedSender<Update>>>,
    sessions: Sessions,
    config: ClaudeConfig,
    /// Where `claude` is and keeps its state, from Settings; read at each request, so a change
    /// applies from the next query.
    provider: StdMutex<ProviderSetup>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
}

impl ClaudeAdapter {
    pub fn new(spawner: Box<dyn Spawner>, config: ClaudeConfig, now: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        let (link, msgs) = Link::new(spawner, Arc::clone(&now));
        Self {
            link,
            msgs: StdMutex::new(Some(msgs)),
            out: StdMutex::new(None),
            sessions: Arc::default(),
            config,
            provider: StdMutex::default(),
            now,
        }
    }

    /// The adapter as the binary builds it: `node` from `PATH`, the sidecar next to the
    /// install or in the checkout, the system clock. When node or the sidecar is missing the
    /// adapter still exists and says so in [`Adapter::status`].
    pub fn from_env(config: ClaudeConfig) -> Self {
        let exe = std::env::current_exe().ok();
        let spawner: Box<dyn Spawner> = match ProcessSpawner::locate(std::env::var("PATH").ok().as_deref(), &|k| std::env::var(k).ok(), exe.as_deref()) {
            Ok(p) => Box::new(p),
            Err(e) => Box::new(Missing(e.to_string())),
        };
        let now = Arc::new(|| {
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
        });
        Self::new(spawner, config, now)
    }

    fn capabilities_static() -> Capabilities {
        Capabilities {
            resume: true,
            stop: true,
            // The session's streaming input takes a message mid-turn: Claude reads it in the same turn once the
            // tool calls running finish (the SDK's default priority, `next`), or it starts the next turn.
            steer: true,
            queue: false,
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
                    control: "local_only".into(),
                    reason: "Claude Code has no setting for it; a radar session starts no remote run and its subagents run locally".into(),
                },
            ],
        }
    }

    fn emit(&self, events: Vec<radar_sessions::Event>) {
        if let Some(tx) = self.out.lock().unwrap().as_ref() {
            for e in events {
                let _ = tx.send(Update::Event(e));
            }
        }
    }

    /// The `claude` to run and its config folder, as every request that starts or asks
    /// `claude` something carries them: Settings' executable beats the config's.
    fn base_args(&self) -> Value {
        let p = self.provider.lock().unwrap();
        let mut a = json!({});
        if let Some(x) = p.executable.as_ref().map(|x| x.to_string_lossy().into_owned()).or_else(|| self.config.claude_path.clone()) {
            a["claude_path"] = json!(x);
        }
        if let Some(d) = &p.config_dir {
            a["config_dir"] = json!(d.to_string_lossy());
        }
        a
    }

    fn session_args(&self, radar: &RadarSessionId, cwd: &std::path::Path, model: &Option<String>, setup: &SessionSetup) -> Value {
        let mut a = self.base_args();
        a["session"] = json!(radar.as_str());
        a["cwd"] = json!(cwd.to_string_lossy());
        a["allow"] = json!(self.config.allow);
        a["deny"] = json!(self.config.deny);
        a["setup"] = serde_json::to_value(setup).unwrap_or_default();
        if let Some(m) = model {
            a["model"] = json!(m);
        }
        if let Some(ask) = &self.config.ask {
            a["ask"] = json!(ask);
        }
        a
    }

    fn known(&self, radar: &RadarSessionId) -> Result<()> {
        self.sessions.lock().unwrap().contains_key(radar).then_some(()).ok_or_else(|| anyhow!("unknown session {radar}"))
    }
}

fn version_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let mut it = v.split('.').map(|p| p.parse::<u64>().ok());
    Some((it.next()??, it.next()??, it.next()??))
}

impl Adapter for ClaudeAdapter {
    fn id(&self) -> ProviderId {
        provider()
    }

    fn capabilities(&self) -> Capabilities {
        Self::capabilities_static()
    }

    fn status(&self) -> BoxFuture<'_, AdapterStatus> {
        Box::pin(async move {
            let caps = Self::capabilities_static();
            let r = match self.link.request("status", self.base_args()).await {
                Ok(r) => r,
                // The sidecar (or node) is missing or keeps dying: nothing to run Claude Code with.
                Err(e) => {
                    return AdapterStatus { installed: false, version: None, protocol: None, auth: AuthStatus::Unknown, error: Some(e.to_string()), capabilities: caps };
                }
            };
            let sdk = self.link.sdk_version().map(|s| format!("Agent SDK {s}"));
            if r["installed"] != true {
                return AdapterStatus { installed: false, version: None, protocol: sdk, auth: AuthStatus::Unknown, error: None, capabilities: caps };
            }
            let version = r["version"].as_str().map(str::to_string);
            let mut error = r["error"].as_str().map(str::to_string);
            if let Some(v) = version.as_deref().and_then(version_tuple)
                && v < MIN_CLAUDE_CODE
            {
                let (a, b, c) = MIN_CLAUDE_CODE;
                error = Some(format!("Claude Code {} is older than {a}.{b}.{c}, which radar's permission handling needs", version.clone().unwrap_or_default()));
            }
            let acct = &r["account"];
            let auth = if acct["logged_in"] == true {
                let who = acct["email"].as_str().unwrap_or("signed in");
                AuthStatus::SignedIn { as_user: who.to_string() }
            } else if acct["error"].is_string() {
                AuthStatus::Unknown
            } else {
                AuthStatus::SignedOut
            };
            AdapterStatus { installed: true, version, protocol: sdk, auth, error, capabilities: caps }
        })
    }

    fn list_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let r = self.link.request("models", self.base_args()).await?;
            Ok(r.as_array()
                .map(|a| a.iter().map(|m| ModelInfo { id: m["id"].as_str().unwrap_or_default().into(), name: m["name"].as_str().unwrap_or_default().into() }).collect())
                .unwrap_or_default())
        })
    }

    fn configure(&self, setup: ProviderSetup) {
        *self.provider.lock().unwrap() = setup;
    }

    /// Starts the sidecar (node and the Agent SDK loaded); a session's `claude` starts with
    /// the session.
    fn warm(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.link.request("hello", json!({})).await?;
            Ok(())
        })
    }

    /// Claude Code writes nothing for a session until its first message (no transcript, so it is
    /// not in `claude --resume` or the session list), so one made ahead is invisible there.
    fn idle_session_is_free(&self) -> bool {
        true
    }

    /// Closes the session's query in the sidecar, which ends its `claude` process.
    fn discard(&self, session: &RadarSessionId) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            self.sessions.lock().unwrap().remove(&session);
            self.link.request("close", json!({ "session": session.as_str() })).await?;
            Ok(())
        })
    }

    fn catalog(&self, cwd: PathBuf, setup: SessionSetup) -> BoxFuture<'_, Result<Catalog>> {
        Box::pin(async move {
            let mut args = self.base_args();
            args["cwd"] = json!(cwd.to_string_lossy());
            args["setup"] = serde_json::to_value(&setup)?;
            let r = self.link.request("catalog", args).await?;
            let entries = |key: &str| -> Vec<CatalogEntry> {
                r[key]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|e| {
                                Some(CatalogEntry {
                                    name: e["name"].as_str()?.to_string(),
                                    description: e["description"].as_str().map(str::to_string),
                                    source: e["source"].as_str().map(str::to_string),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            Ok(Catalog { agents: entries("agents"), skills: entries("skills") })
        })
    }

    fn subscribe(&self) -> BoxFuture<'_, Result<Updates>> {
        Box::pin(async move {
            let mut rx = self.msgs.lock().unwrap().take().ok_or_else(|| anyhow!("subscribe was already called"))?;
            let (tx, updates) = mpsc::unbounded_channel();
            *self.out.lock().unwrap() = Some(tx.clone());
            let sessions = Arc::clone(&self.sessions);
            let now = Arc::clone(&self.now);
            tokio::spawn(async move {
                while let Some(m) = rx.recv().await {
                    match m {
                        LinkMsg::Event(v) => {
                            let Some(radar) = v["session"].as_str().map(RadarSessionId::new) else { continue };
                            let events = {
                                let mut map = sessions.lock().unwrap();
                                let Some(st) = map.get_mut(&radar) else { continue };
                                Mapper::new(&radar, st, now()).map(&v)
                            };
                            for e in events {
                                if tx.send(Update::Event(e)).is_err() {
                                    return;
                                }
                            }
                        }
                        LinkMsg::Commands(v) => {
                            let Some(radar) = v["session"].as_str().map(RadarSessionId::new) else { continue };
                            if !sessions.lock().unwrap().contains_key(&radar) {
                                continue;
                            }
                            if tx.send(Update::Commands { session: radar, commands: commands(&v) }).is_err() {
                                return;
                            }
                        }
                        LinkMsg::Disconnected(reason) => {
                            for st in sessions.lock().unwrap().values_mut() {
                                st.live = false;
                            }
                            let _ = tx.send(Update::Disconnected { reason });
                        }
                        LinkMsg::Reconnected => {
                            let _ = tx.send(Update::Reconnected);
                        }
                    }
                }
            });
            Ok(updates)
        })
    }

    fn create(&self, req: CreateSession) -> BoxFuture<'_, Result<ProviderSessionId>> {
        Box::pin(async move {
            // Claude Code lets radar choose the session id; radar's own id is never reused as it.
            let provider_id = uuid::Uuid::new_v4().to_string();
            let mut args = self.session_args(&req.radar_session_id, &req.cwd, &req.model, &req.setup);
            args["session_id"] = json!(provider_id);
            self.sessions.lock().unwrap().insert(
                req.radar_session_id.clone(),
                SessionState { provider_session_id: Some(ProviderSessionId::new(&provider_id)), cwd: Some(req.cwd.to_string_lossy().into()), live: true, ..Default::default() },
            );
            match self.link.request("create", args).await {
                Ok(r) => Ok(ProviderSessionId::new(r["provider_session_id"].as_str().unwrap_or(&provider_id))),
                Err(e) => {
                    self.sessions.lock().unwrap().remove(&req.radar_session_id);
                    Err(e)
                }
            }
        })
    }

    fn send(&self, session: &RadarSessionId, input: SendInput) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            self.known(&session)?;
            if let Some(st) = self.sessions.lock().unwrap().get_mut(&session) {
                st.stopping = false;
            }
            self.link.request("send", json!({ "session": session.as_str(), "text": input.text })).await?;
            Ok(())
        })
    }

    fn stop(&self, session: &RadarSessionId) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            self.known(&session)?;
            if let Some(st) = self.sessions.lock().unwrap().get_mut(&session) {
                st.stopping = true;
            }
            self.link.request("interrupt", json!({ "session": session.as_str() })).await?;
            Ok(())
        })
    }

    fn resume(&self, req: ResumeSession) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            let pid = req.provider_session_id.clone();
            // A session this adapter already holds keeps its numbering; the sidecar may or may
            // not still have it (it does not after a restart).
            let live = {
                let mut map = self.sessions.lock().unwrap();
                let st = map.entry(req.radar_session_id.clone()).or_default();
                st.provider_session_id = Some(pid.clone());
                st.cwd = Some(req.cwd.to_string_lossy().into());
                st.live
            };
            if !live {
                let mut args = self.session_args(&req.radar_session_id, &req.cwd, &None, &req.setup);
                args["resume"] = json!(pid.as_str());
                if let Some(st) = self.sessions.lock().unwrap().get_mut(&req.radar_session_id) {
                    st.restart_numbering();
                }
                self.link.request("create", args).await?;
                if let Some(st) = self.sessions.lock().unwrap().get_mut(&req.radar_session_id) {
                    st.live = true;
                }
            }
            // The provider owns the transcript: read it back and replay it through the same
            // stream. Its event ids match the live ones, so the store drops what it has.
            let mut hargs = self.base_args();
            hargs["session"] = json!(req.radar_session_id.as_str());
            hargs["provider_session_id"] = json!(pid.as_str());
            hargs["dir"] = json!(req.cwd.to_string_lossy());
            let h = self.link.request("history", hargs).await?;
            let events: Vec<radar_sessions::Event> = {
                let mut map = self.sessions.lock().unwrap();
                let st = map.get_mut(&req.radar_session_id).ok_or_else(|| anyhow!("session vanished"))?;
                let mut m = Mapper::new(&req.radar_session_id, st, (self.now)());
                h["events"].as_array().map(|a| a.iter().flat_map(|v| m.map(v)).filter(|e| e.durability == radar_sessions::Durability::Persisted).collect()).unwrap_or_default()
            };
            self.emit(events);
            Ok(())
        })
    }

    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async move {
            let mut args = self.base_args();
            args["limit"] = json!(200);
            let r = self.link.request("list", args).await?;
            Ok(r.as_array()
                .map(|a| {
                    a.iter()
                        .map(|s| ProviderSessionInfo {
                            provider_session_id: ProviderSessionId::new(s["provider_session_id"].as_str().unwrap_or_default()),
                            title: s["title"].as_str().map(str::to_string),
                            cwd: s["cwd"].as_str().map(str::to_string),
                            updated_at: s["last_modified"].as_i64().map(|ms| ms / 1000),
                        })
                        .collect()
                })
                .unwrap_or_default())
        })
    }

    fn delete_session(&self, session: &ProviderSessionId) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let dir = self
                .sessions
                .lock()
                .unwrap()
                .values()
                .find(|s| s.provider_session_id.as_ref() == Some(&session))
                .and_then(|s| s.cwd.clone());
            let mut args = self.base_args();
            args["provider_session_id"] = json!(session.as_str());
            args["dir"] = json!(dir);
            self.link.request("delete", args).await?;
            Ok(())
        })
    }

    fn answer_permission(&self, session: &RadarSessionId, answer: PermissionAnswer) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            self.known(&session)?;
            let (behavior, message, scope) = match answer.decision {
                PermissionDecision::ApproveOnce => ("allow", None, None),
                // Claude Code's suggested rules for the request, kept for this session (sidecar `sessionRules`).
                PermissionDecision::ApproveForSession => ("allow", None, Some("session")),
                PermissionDecision::Deny { reason } => ("deny", reason, None),
            };
            let mut args = json!({ "session": session.as_str(), "request_id": answer.request_id, "behavior": behavior, "message": message });
            if let Some(scope) = scope {
                args["scope"] = json!(scope);
            }
            // The sidecar answers a request it already answered with "duplicate": a no-op.
            self.link.request("answer", args).await?;
            Ok(())
        })
    }

    fn answer_question(&self, session: &RadarSessionId, answer: QuestionAnswer) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            self.known(&session)?;
            // AskUserQuestion takes its answers keyed by question text (the questions' ids
            // here); several picks of one question are joined with ", ", as the SDK documents.
            let (answers, message) = match answer.decision {
                QuestionDecision::Answer { replies } => {
                    let map: serde_json::Map<String, Value> = replies.into_iter().map(|r| (r.question_id, json!(r.values.join(", ")))).collect();
                    (Value::Object(map), None)
                }
                QuestionDecision::Decline { reason } => (Value::Null, reason),
            };
            // The sidecar answers a question it already answered with "duplicate": a no-op.
            self.link
                .request("answer_question", json!({ "session": session.as_str(), "request_id": answer.request_id, "answers": answers, "message": message }))
                .await?;
            Ok(())
        })
    }
}
