//! [`Adapter`] for a command run in a terminal tab.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Result, bail};
use radar_sessions::{
    Adapter, AdapterStatus, AuthStatus, BoxFuture, Capabilities, CreateSession, Durability, Event,
    EventKind, RadarSessionId, ModelInfo, TerminalStatus, TerminalSurface, PermissionAnswer, ProviderId, ProviderSessionId,
    ProviderSessionInfo, ProviderSetup, ResumeSession, SendInput, Unsupported, Update,
};
use radar_sessions::adapter::Updates;
use tokio::sync::mpsc;

use crate::frame::StateFrame;
use crate::server::ServedTerminal;
use crate::term::{Msg, TerminalSpec};

/// What to run for a provider id, from settings: `codex`, `claude`,
/// `copilot`, or any command. `login_args` is the CLI's own sign-in command
/// (Settings → Agents runs it in a tab, then re-reads status).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyCommand {
    pub id: ProviderId,
    pub command: String,
    pub args: Vec<String>,
}

/// What the dock needs to draw a tab's terminal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalInfo {
    pub socket: PathBuf,
    pub command: String,
    pub cwd: PathBuf,
    pub frame: StateFrame,
}

pub struct PtyAdapter {
    cmd: PtyCommand,
    /// From Settings → Agents: the executable in place of `cmd.command`, and
    /// the CLI's config folder (`COPILOT_HOME`, `CODEX_HOME`, `CLAUDE_CONFIG_DIR`).
    setup: Mutex<ProviderSetup>,
    socket_dir: PathBuf,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    terminals: Arc<Mutex<HashMap<RadarSessionId, ServedTerminal>>>,
    tx: mpsc::UnboundedSender<Update>,
    rx: Mutex<Option<Updates>>,
}

impl PtyAdapter {
    /// `socket_dir` is radar's state folder for sockets (made 0700 when missing).
    pub fn new(cmd: PtyCommand, socket_dir: impl Into<PathBuf>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            cmd,
            setup: Mutex::new(ProviderSetup::default()),
            socket_dir: socket_dir.into(),
            now: Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0)
            }),
            terminals: Arc::default(),
            tx,
            rx: Mutex::new(Some(rx)),
        }
    }

    /// Replaces the clock (tests).
    pub fn with_clock(mut self, now: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.now = Arc::new(now);
        self
    }

    /// The tab's terminal: where to attach and how it stands.
    pub fn info(&self, session: &RadarSessionId) -> Option<TerminalInfo> {
        let map = self.terminals.lock().ok()?;
        let t = map.get(session)?;
        Some(TerminalInfo {
            socket: t.socket().to_path_buf(),
            command: t.terminal().spec().command.clone(),
            cwd: t.terminal().spec().cwd.clone(),
            frame: t.terminal().frame(),
        })
    }

    /// Opens a tab running `cmd` with `extra_args` in `cwd` (for example the
    /// CLI's login command). The tab is a session like any other.
    pub fn open_with(
        &self,
        session: RadarSessionId,
        cwd: &Path,
        args: Vec<String>,
        size: (u16, u16),
    ) -> Result<ProviderSessionId> {
        let cmd = PtyCommand { args, ..self.cmd.clone() };
        self.open_as(cmd, session, cwd, size)
    }

    /// The program and environment settings give this CLI: its executable,
    /// and its config folder under the variable the CLI reads.
    fn launch(&self, command: &str) -> (String, Vec<(String, String)>) {
        let setup = self.setup.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let name = Path::new(command).file_name().and_then(|n| n.to_str()).unwrap_or(command);
        let var = match name {
            "copilot" => Some("COPILOT_HOME"),
            "codex" => Some("CODEX_HOME"),
            "claude" => Some("CLAUDE_CONFIG_DIR"),
            _ => None,
        };
        let env = var.zip(setup.config_dir).map(|(v, d)| vec![(v.to_string(), d.display().to_string())]).unwrap_or_default();
        // The executable stands in for the CLI's own command, never for another program (a shell).
        let program = match (var, setup.executable) {
            (Some(_), Some(exe)) if name == Path::new(&self.cmd.command).file_name().and_then(|n| n.to_str()).unwrap_or("") => exe.display().to_string(),
            _ => command.to_string(),
        };
        (program, env)
    }

    fn open_as(&self, cmd: PtyCommand, session: RadarSessionId, cwd: &Path, size: (u16, u16)) -> Result<ProviderSessionId> {
        let (program, env) = self.launch(&cmd.command);
        let mut spec = TerminalSpec::new(program, cwd).args(cmd.args.clone()).size(size.0, size.1);
        spec.env.extend(env);
        let served = ServedTerminal::start(spec, &self.socket_dir, session.as_str())?;
        self.watch(&session, &served);
        let psid = ProviderSessionId::new(session.as_str());
        self.emit(&session, &psid, 1, EventKind::Other { name: "terminal_started".into() });
        self.terminals.lock().unwrap_or_else(|e| e.into_inner()).insert(session, served);
        Ok(psid)
    }

    fn emit(&self, session: &RadarSessionId, psid: &ProviderSessionId, sequence: u64, kind: EventKind) {
        let _ = self.tx.send(Update::Event(Event {
            radar_session_id: session.clone(),
            provider: self.cmd.id.clone(),
            provider_session_id: psid.clone(),
            provider_event_id: None,
            sequence,
            observed_at: (self.now)(),
            kind,
            durability: Durability::Ephemeral,
            agent: None,
            parent: None,
            payload: serde_json::Value::Null,
        }));
    }

    /// Tells the part when the program ends, so the tab can show it.
    fn watch(&self, session: &RadarSessionId, served: &ServedTerminal) {
        let (_, _, rx) = served.terminal().shared.attach();
        let (tx, now, id, session) = (self.tx.clone(), Arc::clone(&self.now), self.cmd.id.clone(), session.clone());
        let psid = ProviderSessionId::new(session.as_str());
        let _ = std::thread::Builder::new().name("pty-watch".into()).spawn(move || {
            let mut sequence = 1;
            while let Ok(msg) = rx.recv() {
                let Msg::State(f) = msg else { continue };
                let kind = match f.state.as_str() {
                    "exited" => EventKind::Notice {
                        text: match (f.code, &f.signal) {
                            (_, Some(sig)) => format!("ended by signal {sig}"),
                            (Some(c), _) => format!("exited with status {c}"),
                            _ => "exited".into(),
                        },
                    },
                    "failed" => EventKind::Error {
                        message: f.message.clone().unwrap_or_else(|| "could not start".into()),
                        retryable: true,
                    },
                    _ => continue,
                };
                sequence += 1;
                let ev = Event {
                    radar_session_id: session.clone(),
                    provider: id.clone(),
                    provider_session_id: psid.clone(),
                    provider_event_id: None,
                    sequence,
                    observed_at: now(),
                    kind,
                    durability: Durability::Ephemeral,
                    agent: None,
                    parent: None,
                    payload: serde_json::Value::Null,
                };
                if tx.send(Update::Event(ev)).is_err() {
                    break;
                }
            }
        });
    }
}

fn which(command: &str) -> Option<PathBuf> {
    let p = Path::new(command);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(command))
        .find(|c| c.is_file())
}

fn version_of(path: &Path) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let out = std::process::Command::new(path).arg("--version").stdin(std::process::Stdio::null()).output();
        let _ = tx.send(out);
    });
    let out = rx.recv_timeout(Duration::from_secs(3)).ok()?.ok()?;
    let text = String::from_utf8_lossy(if out.stdout.is_empty() { &out.stderr } else { &out.stdout });
    text.lines().next().map(|l| l.trim().chars().take(80).collect())
}

fn capabilities() -> Capabilities {
    let no = |control: &str, reason: &str| Unsupported { control: control.into(), reason: reason.into() };
    Capabilities {
        resume: false,
        stop: false,
        steer: false,
        queue: false,
        list_sessions: true,
        delete_session: true,
        list_models: false,
        permissions: false,
        questions: false,
        structured_events: false,
        slash_commands: Vec::new(),
        agents: false,
        skills: false,
        unsupported: vec![
            no("agents", "The CLI's own screen: choose an agent inside it."),
            no("permissions", "This is the CLI's own screen: it asks for permission there, not in the dock."),
            no("questions", "The CLI asks its questions on its own screen; answer them in the terminal."),
            no("structured_events", "No transcript: the dock shows the CLI's screen, not structured events."),
            no("stop", "Press Ctrl+C in the terminal; the CLI decides what it stops."),
            no("resume", "A terminal lasts as long as radar runs; it survives a renderer restart."),
            no("steer", "Text typed into a terminal goes to whatever the CLI is doing."),
            no("list_models", "Choose the model inside the CLI."),
        ],
    }
}

impl Adapter for PtyAdapter {
    fn id(&self) -> ProviderId {
        self.cmd.id.clone()
    }
    fn capabilities(&self) -> Capabilities {
        capabilities()
    }
    /// A tab opened from now on runs the configured executable with the
    /// CLI's config folder; a running tab keeps what it started with.
    fn configure(&self, setup: ProviderSetup) {
        *self.setup.lock().unwrap_or_else(|e| e.into_inner()) = setup;
    }
    fn status(&self) -> BoxFuture<'_, AdapterStatus> {
        Box::pin(async move {
            let command = self.cmd.command.clone();
            let found = tokio::task::spawn_blocking(move || {
                let path = which(&command)?;
                Some(version_of(&path))
            })
            .await
            .ok()
            .flatten();
            AdapterStatus {
                installed: found.is_some(),
                version: found.flatten(),
                protocol: None,
                // A terminal cannot tell: the CLI shows its own sign-in.
                auth: AuthStatus::Unknown,
                error: None,
                capabilities: capabilities(),
            }
        })
    }
    fn list_models(&self) -> BoxFuture<'_, Result<Vec<ModelInfo>>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    fn subscribe(&self) -> BoxFuture<'_, Result<Updates>> {
        Box::pin(async move {
            match self.rx.lock().unwrap_or_else(|e| e.into_inner()).take() {
                Some(rx) => Ok(rx),
                None => bail!("subscribe was already called"),
            }
        })
    }
    fn create(&self, req: CreateSession) -> BoxFuture<'_, Result<ProviderSessionId>> {
        Box::pin(async move {
            let args = self.cmd.args.clone();
            self.open_with(req.radar_session_id, &req.cwd, args, (80, 24))
        })
    }
    /// Types the text into the program without pressing Enter: a brief sent
    /// to a terminal is reviewed there, and the operator submits it.
    fn send(&self, session: &RadarSessionId, input: SendInput) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let map = self.terminals.lock().unwrap_or_else(|e| e.into_inner());
            let Some(t) = map.get(&session) else { bail!("no terminal for session {session}") };
            t.terminal().write(input.text.as_bytes())
        })
    }
    fn stop(&self, _: &RadarSessionId) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { bail!("a terminal has no turn to stop: press Ctrl+C inside it") })
    }
    fn resume(&self, req: ResumeSession) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if self.terminals.lock().unwrap_or_else(|e| e.into_inner()).contains_key(&req.radar_session_id) {
                Ok(())
            } else {
                bail!("the terminal ended with radar: open a new one")
            }
        })
    }
    fn list_sessions(&self) -> BoxFuture<'_, Result<Vec<ProviderSessionInfo>>> {
        Box::pin(async move {
            let map = self.terminals.lock().unwrap_or_else(|e| e.into_inner());
            Ok(map
                .iter()
                .map(|(id, t)| ProviderSessionInfo {
                    provider_session_id: ProviderSessionId::new(id.as_str()),
                    title: Some(t.terminal().spec().command.clone()),
                    cwd: Some(t.terminal().spec().cwd.display().to_string()),
                    updated_at: None,
                })
                .collect())
        })
    }
    fn delete_session(&self, session: &ProviderSessionId) -> BoxFuture<'_, Result<()>> {
        let key = RadarSessionId::new(session.as_str());
        Box::pin(async move {
            self.terminals.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
            Ok(())
        })
    }
    fn open_command(&self, req: CreateSession, argv: Vec<String>) -> BoxFuture<'_, Result<ProviderSessionId>> {
        Box::pin(async move {
            let Some((program, args)) = argv.split_first() else { bail!("no command to run") };
            let cmd = PtyCommand { id: self.cmd.id.clone(), command: program.clone(), args: args.to_vec() };
            self.open_as(cmd, req.radar_session_id, &req.cwd, (80, 24))
        })
    }
    fn terminal(&self, session: &RadarSessionId) -> Option<TerminalSurface> {
        let info = self.info(session)?;
        let state = match info.frame.state.as_str() {
            "running" => TerminalStatus::Running,
            "exited" => TerminalStatus::Exited(info.frame.code.unwrap_or(-1)),
            _ => TerminalStatus::Failed,
        };
        Some(TerminalSurface {
            socket: info.socket,
            command: info.command,
            cwd: info.cwd.display().to_string(),
            columns: info.frame.columns,
            rows: info.frame.rows,
            state,
        })
    }
    fn restart_terminal(&self, session: &RadarSessionId) -> BoxFuture<'_, Result<()>> {
        let session = session.clone();
        Box::pin(async move {
            let map = self.terminals.lock().unwrap_or_else(|e| e.into_inner());
            let Some(t) = map.get(&session) else { bail!("no terminal for session {session}") };
            t.terminal().restart()
        })
    }
    fn answer_permission(&self, _: &RadarSessionId, _: PermissionAnswer) -> BoxFuture<'_, Result<()>> {
        Box::pin(async { bail!("a terminal asks for permission on its own screen") })
    }
}


/// The adapter of a CLI configured in `[[sessions.pty]]`: its own id, command and arguments.
pub fn for_cli(cli: &radar_sessions::settings::config::PtyCli) -> PtyAdapter {
    PtyAdapter::new(PtyCommand { id: ProviderId::new(cli.id.clone()), command: cli.command.clone(), args: cli.args.clone() }, socket_dir())
}

/// Where the terminals' sockets are made: radar's state folder (mode 0700), the temporary folder without one.
pub fn socket_dir() -> PathBuf {
    std::env::var_os("XDG_STATE_HOME")
        .map(std::path::PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local").join("state")))
        .unwrap_or_else(std::env::temp_dir)
        .join("radar")
        .join("terminals")
}

/// The CLIs a terminal tab offers out of the box, one provider each (`pty:codex`, ...): the three agent CLIs
/// and the user's shell. Settings → Agents adds configured ones.
pub fn standard_adapters() -> Vec<PtyAdapter> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".into());
    [("pty:codex", "codex"), ("pty:claude", "claude"), ("pty:copilot", "copilot"), ("pty:shell", shell.as_str())]
        .into_iter()
        .map(|(id, command)| PtyAdapter::new(PtyCommand { id: ProviderId::new(id), command: command.into(), args: vec![] }, socket_dir()))
        .collect()
}
