//! The real [`Backend`]: the official Rust SDK driving the user's installed `copilot`.
//!
//! No build-time download (`CliProgram::Path`), `COPILOT_AUTO_UPDATE=false` for the
//! child, remote sessions off (the SDK default, set explicitly), and a permission
//! handler and a question (`ask_user`) handler that only wait for radar's operator: no
//! `ApproveAllHandler`.
//!
//! The operator's own approvals apply: radar passes the CLI no permission flags and its own
//! `COPILOT_HOME` (unless Settings names another), so the CLI decides from its config
//! (`trustedFolders`, `allowedUrls`, `deniedUrls`, the approvals it remembers per location
//! and the repository's `.github/copilot/settings*.json`) and its read-only defaults before
//! it asks. A request a hook resolved (`resolvedByHook`) never reaches the handler (the SDK
//! skips it) and is not shown as waiting. Approve for session is Copilot's own
//! `approve-for-session` decision ([`crate::map::session_decision`]), offered where the
//! request has `canOfferSessionApproval`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use radar_sessions::adapter::{Catalog, ProviderSetup, SessionSetup};
use radar_sessions::{
    BoxFuture, RadarSessionId, ModelInfo, PermissionDecision, ProviderSessionId, ProviderSessionInfo,
    QuestionDecision,
};
use github_copilot_sdk::handler::{
    PermissionHandler, PermissionResult, UserInputHandler, UserInputResponse,
};
use github_copilot_sdk::rpc::{AgentSelectRequest, SkillsEnableRequest};
use github_copilot_sdk::session::Session;
use github_copilot_sdk::rpc::CommandsInvokeRequest;
use github_copilot_sdk::types::{AgentMode, MessageOptions, PermissionRequestData, RequestId, SessionId};
use github_copilot_sdk::{CliProgram, Client, ClientOptions, ResumeSessionConfig, SessionConfig};

use crate::backend::{Backend, BackendStatus, Sink, StatusError};
use crate::commands::{self, CommandInfo, Invoked};
use crate::map::RawEvent;
use crate::setup::{self, Followup, SkillRow};

/// How the `copilot` child is started: what Settings and tests can change.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Launch {
    /// The configured `copilot`; `None` looks on `PATH` each time it is needed, so an
    /// install made while radar runs is found.
    pub exe: Option<PathBuf>,
    /// `COPILOT_HOME` of the child (Settings, or a scratch one for tests and recordings).
    pub home: Option<PathBuf>,
}

/// What to do with the running client when the launch it was started with is not the one
/// now wanted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Same launch (or nothing running yet).
    Keep,
    /// Different and nothing is open: drop it, the next call starts the new one.
    Restart,
    /// Different but sessions are open: never end a client under a turn; it is replaced
    /// once the last of them closes.
    Defer,
}

pub fn plan(running: Option<&Launch>, wanted: &Launch, open: usize) -> Plan {
    match running {
        Some(r) if r != wanted && open == 0 => Plan::Restart,
        Some(r) if r != wanted => Plan::Defer,
        _ => Plan::Keep,
    }
}

pub struct SdkBackend {
    /// The launch wanted for the next start of the client.
    wanted: Mutex<Launch>,
    /// The running client and the launch it was started with.
    client: tokio::sync::Mutex<Option<(Arc<Client>, Launch)>>,
    sessions: Mutex<HashMap<RadarSessionId, Open>>,
    /// Probe sessions in flight (catalog): the client is not replaced under them.
    probes: AtomicUsize,
    /// Set when a session's event stream ended without radar closing it: the client is dead.
    broken: Arc<AtomicBool>,
}

/// A session attached to the current client.
struct Open {
    session: Arc<Session>,
    /// radar is closing it itself; the end of its event stream is not a lost connection.
    closing: Arc<AtomicBool>,
}

impl SdkBackend {
    pub fn new(configured: Option<PathBuf>) -> Self {
        Self {
            wanted: Mutex::new(Launch {
                exe: configured,
                home: None,
            }),
            client: tokio::sync::Mutex::new(None),
            sessions: Mutex::new(HashMap::new()),
            probes: AtomicUsize::new(0),
            broken: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Detaches a session radar opened, without reporting a lost connection.
    async fn close(&self, id: &RadarSessionId) {
        let open = self.sessions.lock().unwrap().remove(id);
        if let Some(o) = open {
            o.closing.store(true, Ordering::SeqCst);
            let _ = o.session.disconnect().await;
        }
    }

    /// Reads a session's events into the sink until it ends; an end radar did not cause is
    /// a lost connection.
    fn watch(
        &self,
        mut sub: github_copilot_sdk::subscription::EventSubscription,
        sink: Arc<dyn Sink>,
    ) -> Arc<AtomicBool> {
        let closing = Arc::new(AtomicBool::new(false));
        let (flag, broken) = (closing.clone(), self.broken.clone());
        tokio::spawn(async move {
            loop {
                match sub.recv().await {
                    Ok(ev) => sink.event(raw(ev)),
                    // Fell behind: the oldest live events were dropped, the stream goes on. Ending here
                    // would leave the session showing a turn that never ends.
                    Err(e) if matches!(e.kind(), github_copilot_sdk::subscription::RecvErrorKind::Lagged(_)) => {
                        sink.missed(e.to_string());
                    }
                    Err(e) => {
                        if !flag.load(Ordering::SeqCst) {
                            broken.store(true, Ordering::SeqCst);
                            sink.lost(format!("{e:?}"));
                        }
                        break;
                    }
                }
            }
        });
        closing
    }

    pub fn with_home(self, home: PathBuf) -> Self {
        self.wanted.lock().unwrap().home = Some(home);
        self
    }

    /// The launch the next client start uses.
    pub fn launch(&self) -> Launch {
        self.wanted.lock().unwrap().clone()
    }

    /// Sessions and probes that would end with the client.
    fn open(&self) -> usize {
        self.sessions.lock().unwrap().len() + self.probes.load(Ordering::SeqCst)
    }

    async fn client(&self) -> anyhow::Result<Arc<Client>> {
        let wanted = self.launch();
        let mut guard = self.client.lock().await;
        if let Some((c, started)) = guard.as_ref() {
            match plan(Some(started), &wanted, self.open()) {
                Plan::Keep | Plan::Defer => return Ok(c.clone()),
                Plan::Restart => {}
            }
        }
        // Dropping the old client ends its child; nothing was open on it.
        guard.take();
        let mut o = ClientOptions::default();
        let program = crate::login::resolve_program(wanted.exe.as_deref())
            .ok_or_else(|| anyhow::anyhow!("copilot is not installed"))?;
        o.program = CliProgram::Path(program);
        o.working_directory = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        o.enable_remote_sessions = false;
        o.env.push(("COPILOT_AUTO_UPDATE".into(), "false".into()));
        if let Some(home) = &wanted.home {
            o.env
                .push(("COPILOT_HOME".into(), home.as_os_str().to_os_string()));
        }
        let c = Arc::new(
            Client::start(o)
                .await
                .map_err(|e| anyhow::anyhow!("copilot did not start: {e}"))?,
        );
        *guard = Some((c.clone(), wanted));
        Ok(c)
    }

    async fn forget_client(&self) {
        self.client.lock().await.take();
    }

    fn session(&self, id: &RadarSessionId) -> anyhow::Result<Arc<Session>> {
        self.sessions
            .lock()
            .unwrap()
            .get(id)
            .map(|o| o.session.clone())
            .ok_or_else(|| anyhow::anyhow!("session {id} is not open; resume it first"))
    }
}

fn raw(ev: github_copilot_sdk::types::SessionEvent) -> RawEvent {
    RawEvent {
        ephemeral: ev.ephemeral.unwrap_or(false),
        id: ev.id,
        timestamp: ev.timestamp,
        parent_id: ev.parent_id,
        agent_id: ev.agent_id,
        event_type: ev.event_type,
        data: ev.data,
    }
}

/// Waits for radar's operator; never answers by itself.
struct Gate(Arc<dyn Sink>);

#[async_trait::async_trait]
impl PermissionHandler for Gate {
    async fn handle(
        &self,
        _s: SessionId,
        id: RequestId,
        data: PermissionRequestData,
    ) -> PermissionResult {
        let value = serde_json::to_value(&data).unwrap_or_default();
        match self.0.permission(id.to_string(), value.clone()).await {
            PermissionDecision::ApproveOnce => PermissionResult::approve_once(),
            // Copilot's own session approval; a kind it has none for is approved once (radar
            // remembers it for the session itself).
            PermissionDecision::ApproveForSession => crate::map::session_decision(&value)
                .and_then(|d| serde_json::from_value(d).ok())
                .map(|decision| PermissionResult::Decision { decision, context: None })
                .unwrap_or_else(PermissionResult::approve_once),
            PermissionDecision::Deny { reason } => {
                PermissionResult::reject(Some(reason.unwrap_or_else(|| "denied from radar".into())))
            }
        }
    }
}

/// Copilot's `ask_user` (the legacy question-and-answer variant, the SDK default): waits
/// for radar's operator; never answers by itself. A decline is "no response".
struct Ask(Arc<dyn Sink>);

#[async_trait::async_trait]
impl UserInputHandler for Ask {
    async fn handle(
        &self,
        _s: SessionId,
        question: String,
        choices: Option<Vec<String>>,
        allow_freeform: Option<bool>,
    ) -> Option<UserInputResponse> {
        let choices = choices.unwrap_or_default();
        let free = choices.is_empty() || allow_freeform.unwrap_or(true);
        match self.0.question(question, choices, free).await {
            QuestionDecision::Answer { replies } => {
                replies.into_iter().next().map(|r| UserInputResponse {
                    answer: r.values.join(", "),
                    was_freeform: r.own_words,
                })
            }
            QuestionDecision::Decline { .. } => None,
        }
    }
}

impl Backend for SdkBackend {
    fn warm(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async move { self.client().await.map(|_| ()) })
    }

    fn status(&self) -> BoxFuture<'_, Result<BackendStatus, StatusError>> {
        Box::pin(async move {
            if crate::login::resolve_program(self.launch().exe.as_deref()).is_none() {
                return Err(StatusError::Missing);
            }
            let c = self
                .client()
                .await
                .map_err(|e| StatusError::Connection(e.to_string()))?;
            let st = c
                .get_status()
                .await
                .map_err(|e| StatusError::Connection(e.to_string()))?;
            let au = c
                .get_auth_status()
                .await
                .map_err(|e| StatusError::Connection(e.to_string()))?;
            Ok(BackendStatus {
                version: st.version,
                protocol: st.protocol_version,
                authenticated: au.is_authenticated,
                login: au.login,
                host: au.host,
                message: au.status_message,
            })
        })
    }

    fn models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>> {
        Box::pin(async move {
            let c = self.client().await?;
            let models = c.list_models().await.map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(models
                .into_iter()
                .map(|m| ModelInfo {
                    name: m.name.clone(),
                    id: m.id,
                })
                .collect())
        })
    }

    fn create<'a>(
        &'a self,
        id: &'a RadarSessionId,
        cwd: &'a Path,
        model: Option<&'a str>,
        setup: &'a SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let c = self.client().await?;
            let mut cfg = SessionConfig::default()
                .with_permission_handler(Arc::new(Gate(sink.clone())))
                .with_user_input_handler(Arc::new(Ask(sink.clone())));
            cfg.session_id = Some(SessionId::new(id.as_str()));
            cfg.working_directory = Some(cwd.to_path_buf());
            cfg.model = model.map(str::to_string);
            cfg.streaming = Some(true);
            let cfg = setup::apply_create(cfg, setup);
            // The SDK subscribes after create_session returns; events of the create call
            // itself (session.start) are in get_events and not needed for a new session.
            let session = Arc::new(
                c.create_session(cfg)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            );
            if let Err(e) = follow(&session, &Followup::of(setup)).await {
                // A session that is not the one asked for must not be left behind.
                let _ = session.disconnect().await;
                let _ = c.delete_session(session.id()).await;
                return Err(e);
            }
            let closing = self.watch(session.subscribe(), sink);
            self.sessions
                .lock()
                .unwrap()
                .insert(id.clone(), Open { session, closing });
            Ok(())
        })
    }

    fn resume<'a>(
        &'a self,
        id: &'a RadarSessionId,
        cwd: &'a Path,
        setup: &'a SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            // After a lost connection the client is dead: drop every session on it and
            // start a fresh client. Otherwise only this session is detached first.
            if self.broken.swap(false, Ordering::SeqCst) {
                let all: Vec<RadarSessionId> = self.sessions.lock().unwrap().keys().cloned().collect();
                for s in &all {
                    if let Some(o) = self.sessions.lock().unwrap().remove(s) {
                        o.closing.store(true, Ordering::SeqCst);
                    }
                }
                self.forget_client().await;
            } else {
                self.close(id).await;
            }
            let c = self.client().await?;
            let mut cfg = ResumeSessionConfig::new(SessionId::new(id.as_str()))
                .with_permission_handler(Arc::new(Gate(sink.clone())))
                .with_user_input_handler(Arc::new(Ask(sink.clone())));
            cfg.streaming = Some(true);
            cfg.working_directory = Some(cwd.to_path_buf());
            let cfg = setup::apply_resume(cfg, setup);
            let session = Arc::new(
                c.resume_session(cfg)
                    .await
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            );
            if let Err(e) = follow(&session, &Followup::of(setup)).await {
                let _ = session.disconnect().await;
                return Err(e);
            }
            let sub = session.subscribe();
            for ev in session
                .get_events()
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?
            {
                sink.event(raw(ev));
            }
            let closing = self.watch(sub, sink);
            self.sessions
                .lock()
                .unwrap()
                .insert(id.clone(), Open { session, closing });
            Ok(())
        })
    }

    fn send<'a>(&'a self, id: &'a RadarSessionId, text: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let s = self.session(id)?;
            s.send(text)
                .await
                .map(|_| ())
                .map_err(|e| anyhow::anyhow!("{e}"))
        })
    }

    fn commands<'a>(&'a self, id: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<Vec<CommandInfo>>> {
        Box::pin(async move {
            let list = self.session(id)?.rpc().commands().list().await.map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(commands::parse_list(&serde_json::to_value(list)?))
        })
    }

    fn invoke<'a>(&'a self, id: &'a RadarSessionId, name: &'a str, input: Option<&'a str>) -> BoxFuture<'a, anyhow::Result<Invoked>> {
        Box::pin(async move {
            let req = CommandsInvokeRequest { name: name.to_string(), input: input.map(str::to_string), origin: None };
            let r = self.session(id)?.rpc().commands().invoke(req).await.map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(commands::parse_invoked(&serde_json::to_value(r)?))
        })
    }

    fn send_prompt<'a>(&'a self, id: &'a RadarSessionId, prompt: &'a str, display: &'a str, plan: bool) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let mut m = MessageOptions::new(prompt);
            m.display_prompt = Some(display.to_string());
            if plan {
                m.agent_mode = Some(AgentMode::Plan);
            }
            self.session(id)?.send(m).await.map(|_| ()).map_err(|e| anyhow::anyhow!("{e}"))
        })
    }

    fn abort<'a>(&'a self, id: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            self.session(id)?
                .abort()
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
        })
    }

    fn delete<'a>(&'a self, provider_session_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async move {
            let radar = RadarSessionId::new(provider_session_id);
            self.close(&radar).await;
            let c = self.client().await?;
            c.delete_session(&SessionId::new(provider_session_id))
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))
        })
    }

    fn list(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async move {
            let c = self.client().await?;
            let all = c
                .list_sessions(None)
                .await
                .map_err(|e| anyhow::anyhow!("{e}"))?;
            Ok(all
                .into_iter()
                .filter(|m| !m.is_remote)
                .map(|m| ProviderSessionInfo {
                    provider_session_id: ProviderSessionId::new(m.session_id.to_string()),
                    title: m.summary,
                    cwd: None,
                    updated_at: None,
                })
                .collect())
        })
    }

    fn configure(&self, setup: ProviderSetup) -> Option<String> {
        let new = Launch {
            exe: setup.executable,
            home: setup.config_dir,
        };
        let changed = std::mem::replace(&mut *self.wanted.lock().unwrap(), new.clone()) != new;
        (changed && self.open() > 0).then(|| {
            "copilot: the new executable or config folder applies once the open sessions end, \
             not under a running turn"
                .to_string()
        })
    }

    fn catalog<'a>(
        &'a self,
        cwd: &'a Path,
        setup: &'a SessionSetup,
    ) -> BoxFuture<'a, anyhow::Result<Catalog>> {
        Box::pin(async move {
            let c = self.client().await?;
            if setup::discovery_is_faithful(setup) {
                discover(&c, cwd, setup).await
            } else {
                self.probe(&c, cwd, setup).await
            }
        })
    }
}

/// The catalog from the client's own discovery: no session, so nothing in the user's list.
async fn discover(c: &Client, cwd: &Path, setup: &SessionSetup) -> anyhow::Result<Catalog> {
    let agents = c
        .rpc()
        .agents()
        .discover(setup::agents_request(cwd))
        .await
        .map_err(|e| anyhow::anyhow!("agents: {e}"))?;
    let skills = c
        .rpc()
        .skills()
        .discover(setup::skills_request(cwd, setup))
        .await
        .map_err(|e| anyhow::anyhow!("skills: {e}"))?;
    Ok(Catalog {
        agents: setup::agent_entries(&agents.agents),
        skills: setup::skill_entries(skills.skills.iter().map(SkillRow::from)),
    })
}

/// Counts a probe while it lives, so the client is not replaced under it.
struct ProbeGuard<'a>(&'a AtomicUsize);

impl<'a> ProbeGuard<'a> {
    fn new(n: &'a AtomicUsize) -> Self {
        n.fetch_add(1, Ordering::SeqCst);
        Self(n)
    }
}

impl Drop for ProbeGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl SdkBackend {
    /// The catalog from a short-lived session that has the setup, for the cases the
    /// client's discovery cannot answer. The session denies every request (it runs no
    /// turn), and is disconnected and deleted whatever happened, so it never stays in
    /// the user's session list.
    async fn probe(
        &self,
        c: &Arc<Client>,
        cwd: &Path,
        setup: &SessionSetup,
    ) -> anyhow::Result<Catalog> {
        let _guard = ProbeGuard::new(&self.probes);
        let id = SessionId::new(uuid::Uuid::new_v4().to_string());
        let mut cfg = SessionConfig::default().deny_all_permissions();
        cfg.session_id = Some(id.clone());
        cfg.working_directory = Some(cwd.to_path_buf());
        let mut cfg = setup::apply_create(cfg, setup);
        cfg.enable_skills = Some(true);
        let session = c
            .create_session(cfg)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let listed = async {
            let agents = session.rpc().agent().list().await?;
            let skills = session.rpc().skills().list().await?;
            Ok::<_, github_copilot_sdk::Error>(Catalog {
                agents: setup::agent_entries(&agents.agents),
                skills: setup::skill_entries(skills.skills.iter().map(SkillRow::from)),
            })
        }
        .await;
        let _ = session.disconnect().await;
        let _ = c.delete_session(&id).await;
        listed.map_err(|e| anyhow::anyhow!("{e}"))
    }
}

/// What a setup asks of an open session: the agent it runs as and the skills it must
/// have. Fails when either is not there, so a session never runs as something else.
async fn follow(session: &Session, f: &Followup) -> anyhow::Result<()> {
    if let Some(agent) = &f.agent {
        session
            .rpc()
            .agent()
            .select(AgentSelectRequest {
                name: agent.clone(),
            })
            .await
            .map_err(|e| anyhow::anyhow!("copilot has no agent {agent:?} here: {e}"))?;
    }
    if !f.skills.is_empty() {
        for name in &f.skills {
            session
                .rpc()
                .skills()
                .enable(SkillsEnableRequest { name: name.clone() })
                .await
                .map_err(|e| anyhow::anyhow!("copilot could not enable skill {name:?}: {e}"))?;
        }
        let listed = session
            .rpc()
            .skills()
            .list()
            .await
            .map_err(|e| anyhow::anyhow!("copilot did not list its skills: {e}"))?;
        let missing = setup::missing_skills(
            &f.skills,
            listed.skills.iter().map(|s| (s.name.clone(), s.enabled)),
        );
        if !missing.is_empty() {
            anyhow::bail!(
                "copilot does not have these skills enabled: {}",
                missing.join(", ")
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn launch(exe: &str) -> Launch {
        Launch {
            exe: Some(PathBuf::from(exe)),
            home: None,
        }
    }

    #[test]
    fn a_changed_launch_replaces_an_idle_client_and_waits_for_a_busy_one() {
        let (a, b) = (launch("/a/copilot"), launch("/b/copilot"));
        assert_eq!(plan(None, &b, 0), Plan::Keep, "nothing runs yet");
        assert_eq!(plan(Some(&a), &a, 3), Plan::Keep);
        assert_eq!(plan(Some(&a), &b, 0), Plan::Restart);
        assert_eq!(plan(Some(&a), &b, 1), Plan::Defer);
        let home = Launch {
            home: Some(PathBuf::from("/h")),
            ..a.clone()
        };
        assert_eq!(plan(Some(&a), &home, 0), Plan::Restart, "COPILOT_HOME counts");
    }

    #[test]
    fn configure_stores_the_executable_and_the_home() {
        let b = SdkBackend::new(Some(PathBuf::from("/old/copilot")));
        let note = b.configure(ProviderSetup {
            executable: Some(PathBuf::from("/new/copilot")),
            config_dir: Some(PathBuf::from("/new/home")),
        });
        assert_eq!(note, None, "nothing is open, so nothing to wait for");
        assert_eq!(
            b.launch(),
            Launch {
                exe: Some(PathBuf::from("/new/copilot")),
                home: Some(PathBuf::from("/new/home")),
            }
        );
        // Back to the defaults: PATH and the CLI's own home.
        b.configure(ProviderSetup::default());
        assert_eq!(b.launch(), Launch::default());
    }

    #[test]
    fn configure_while_something_is_open_keeps_the_client_and_says_so() {
        let b = SdkBackend::new(None);
        let _probe = ProbeGuard::new(&b.probes);
        let setup = ProviderSetup {
            executable: Some(PathBuf::from("/new/copilot")),
            config_dir: None,
        };
        let note = b.configure(setup.clone()).expect("a note");
        assert!(note.contains("once the open sessions end"), "{note}");
        // The same setting again is not news.
        assert_eq!(b.configure(setup), None);
        assert_eq!(b.launch().exe, Some(PathBuf::from("/new/copilot")));
    }
}
