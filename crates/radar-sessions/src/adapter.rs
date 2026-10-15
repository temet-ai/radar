//! What every provider adapter implements. An adapter declares what it
//! supports in [`Capabilities`]; a control it lacks is absent or explained,
//! never faked.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::event::{
    Event, RadarSessionId, PermissionAnswer, ProviderEventId, ProviderId, ProviderSessionId,
    QuestionAnswer,
};

/// A boxed future, so the trait can be used as `Box<dyn Adapter>` without a
/// macro crate.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "auth", rename_all = "snake_case")]
pub enum AuthStatus {
    SignedIn { as_user: String },
    SignedOut,
    /// An organisation policy forbids it.
    PolicyBlocked { message: String },
    Unknown,
}

/// A control the provider lacks, and why.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unsupported {
    pub control: String,
    pub reason: String,
}

/// A command the composer offers after a `/`, and what it does. A command is sent as the
/// text `/name args`; the provider runs it (Claude Code, Copilot) or the adapter turns
/// the text into the call that does (Codex).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    /// Typed after the slash, without it.
    pub name: String,
    pub description: String,
    /// What follows the name (`<file>`), when it takes anything.
    pub argument_hint: Option<String>,
}

/// `/name rest` as typed: the word after the slash and what follows it (trimmed, `None` when
/// empty). `None` unless the text starts with a slash and a word: a path (`/etc/hosts`) or a
/// sentence is not a command.
pub fn parse_slash(text: &str) -> Option<(&str, Option<&str>)> {
    let body = text.trim().strip_prefix('/')?;
    let (word, rest) = match body.split_once(char::is_whitespace) {
        Some((w, r)) => (w, Some(r.trim()).filter(|r| !r.is_empty())),
        None => (body, None),
    };
    (!word.is_empty() && !word.contains('/')).then_some((word, rest))
}

impl SlashCommand {
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self { name: name.into().trim_start_matches('/').to_string(), description: description.into(), argument_hint: None }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        let hint = hint.into();
        self.argument_hint = (!hint.trim().is_empty()).then_some(hint);
        self
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    /// Sessions can be resumed with their history.
    pub resume: bool,
    pub stop: bool,
    /// Text sent while a turn runs reaches the agent in that turn.
    pub steer: bool,
    /// Text sent while a turn runs is kept by the provider and run after the
    /// turn (Copilot's queue). With neither this nor `steer`, radar keeps it and
    /// sends it when the turn ends.
    #[serde(default)]
    pub queue: bool,
    pub list_sessions: bool,
    pub delete_session: bool,
    pub list_models: bool,
    /// Structured permission requests (false for a PTY tab).
    pub permissions: bool,
    /// The agent can ask the operator a question and wait for the answer
    /// ([`crate::EventKind::QuestionAsked`], [`Adapter::answer_question`]).
    #[serde(default)]
    pub questions: bool,
    /// Structured events (false for a PTY tab).
    pub structured_events: bool,
    /// Slash commands every session of this provider has. A provider whose list depends on
    /// the session (skills, project commands) leaves this empty and reports each session's
    /// list with [`Update::Commands`].
    pub slash_commands: Vec<SlashCommand>,
    /// [`Adapter::catalog`] lists the agents, and a session can be started as one.
    #[serde(default)]
    pub agents: bool,
    /// [`Adapter::catalog`] lists the skills, and the skill folders of a setup are loaded.
    #[serde(default)]
    pub skills: bool,
    /// Why a control above is false, for the UI to explain.
    pub unsupported: Vec<Unsupported>,
}

/// Everything Settings → Agents shows about a CLI.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub protocol: Option<String>,
    pub auth: AuthStatus,
    /// A connection or compatibility error, in words.
    pub error: Option<String>,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
}

/// A terminal tab's program (an adapter that runs a CLI's own screen): where the
/// renderer attaches to it and how it stands.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSurface {
    /// The per-terminal socket the renderer connects to.
    pub socket: PathBuf,
    pub command: String,
    pub cwd: String,
    pub columns: u16,
    pub rows: u16,
    pub state: TerminalStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerminalStatus {
    Running,
    Exited(i32),
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateSession {
    pub radar_session_id: RadarSessionId,
    pub cwd: PathBuf,
    pub model: Option<String>,
    pub setup: SessionSetup,
}

/// What a session is set up with, from Settings → Agents: the same for a new
/// session and a resumed one (a provider forgets some of it on resume). Every
/// path is absolute. An adapter maps each field onto its CLI's own option and
/// says in [`Capabilities::unsupported`] what its CLI cannot take.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSetup {
    /// Folders the session may read and write besides its working directory.
    pub extra_dirs: Vec<PathBuf>,
    /// The agent to run as: a name in [`Catalog::agents`]. None: the CLI's own.
    pub agent: Option<String>,
    /// Skill folders (`<name>/SKILL.md`) to load besides the CLI's own.
    pub skill_dirs: Vec<PathBuf>,
    /// Skills the session is asked to use, by name (a recipe's `skills:`).
    /// Empty: whichever it finds. An adapter makes sure each is available
    /// (enabled, not filtered out); the brief also names them to the agent.
    pub skills: Vec<String>,
    /// Folders of instruction files (AGENTS.md and the like) besides the CLI's own.
    pub instruction_dirs: Vec<PathBuf>,
    /// Plugin folders to load, in the CLI's own plugin format.
    pub plugin_dirs: Vec<PathBuf>,
    /// Load the CLI's own configuration of the user and the project: its
    /// instruction files (AGENTS.md, CLAUDE.md), agents, skills, MCP servers
    /// and hooks. Off: only what this setup names.
    pub project_config: bool,
    /// No remote or delegated runs.
    pub local_only: bool,
    /// A review session: the adapter keeps radar's own strict policy (ask before
    /// anything not known to be safe) over the approvals in the user's CLI
    /// settings, which an ordinary session takes.
    #[serde(default)]
    pub review: bool,
    /// radar's own MCP server for this session ([`crate::tools`]): a stdio command the CLI
    /// starts, which relays to the session's socket. None: no radar tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_server: Option<ToolServer>,
}

/// A stdio MCP server a session is given: `command args…` speaks MCP on its stdin and
/// stdout. `tools` are the tools it offers, by their bare names (`echo`), so an adapter can
/// name each in its CLI's allow list without a wildcard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolServer {
    /// The server's name as the CLI shows it (`radar`): Claude Code calls its tools
    /// `mcp__radar__<tool>`.
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub tools: Vec<String>,
}

/// Where a provider's CLI is and keeps its own state, from Settings → Agents.
/// Applies from the next start of the CLI.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProviderSetup {
    /// The executable; None: found on PATH.
    pub executable: Option<PathBuf>,
    /// The CLI's own config and state folder (`COPILOT_HOME`, `CODEX_HOME`,
    /// `CLAUDE_CONFIG_DIR`); None: the CLI's default.
    pub config_dir: Option<PathBuf>,
}

/// What a CLI offers a session in a working directory with a setup: the
/// agents it can run as and the skills it has loaded.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    pub agents: Vec<CatalogEntry>,
    pub skills: Vec<CatalogEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogEntry {
    pub name: String,
    pub description: Option<String>,
    /// Where it comes from (`project`, `user`, a folder, a plugin), when the CLI says.
    pub source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeSession {
    pub radar_session_id: RadarSessionId,
    pub provider_session_id: ProviderSessionId,
    pub cwd: PathBuf,
    /// The last persisted provider event radar already has. The adapter replays
    /// the persisted events after it (all of them when `None`); events radar has
    /// seen are dropped by the store, so replaying more is harmless.
    pub cursor: Option<ProviderEventId>,
    pub setup: SessionSetup,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendInput {
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderSessionInfo {
    pub provider_session_id: ProviderSessionId,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub updated_at: Option<i64>,
}

/// What an adapter pushes to the part.
#[derive(Clone, Debug, PartialEq)]
// Events are the common case and each is moved once, into the store.
#[allow(clippy::large_enum_variant)]
pub enum Update {
    Event(Event),
    /// The connection to the provider is gone; sessions are unreliable
    /// until [`Update::Reconnected`].
    Disconnected { reason: String },
    /// The connection is back; the part resumes sessions from their cursors.
    Reconnected,
    /// The slash commands one session has now: the whole list, replacing what was reported
    /// before. Sent when the provider has told the adapter, which may be after the session
    /// starts. Not history: it is never stored or replayed, so an adapter reports it again
    /// when it resumes a session.
    Commands { session: RadarSessionId, commands: Vec<SlashCommand> },
}

pub type Updates = mpsc::UnboundedReceiver<Update>;

/// A step of a sign-in the adapter runs itself (Copilot's device code).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginEvent {
    /// Show the operator this code and link; the provider waits.
    DeviceCode { url: String, code: String },
    Completed,
    Failed(String),
    Cancelled,
}

/// A sign-in in progress. The token never passes through radar.
pub trait LoginHandle: Send {
    fn next_event(&mut self) -> BoxFuture<'_, LoginEvent>;
    fn cancel(&self);
}

/// One provider. Methods take `&self`: an adapter is shared and does its own
/// locking.
pub trait Adapter: Send + Sync {
    fn id(&self) -> ProviderId;
    fn capabilities(&self) -> Capabilities;
    /// Installed, version, protocol, auth, errors. Never prints a token.
    fn status(&self) -> BoxFuture<'_, AdapterStatus>;
    fn list_models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>>;
    /// The stream of updates for every session of this adapter. Called once;
    /// a second call is an error.
    fn subscribe(&self) -> BoxFuture<'_, anyhow::Result<Updates>>;
    fn create(&self, req: CreateSession) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>>;
    fn send(
        &self,
        session: &RadarSessionId,
        input: SendInput,
    ) -> BoxFuture<'_, anyhow::Result<()>>;
    /// Abort the running turn. The session stays usable.
    fn stop(&self, session: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>>;
    /// Reattach to a session the provider still holds and replay its
    /// persisted events after the cursor.
    fn resume(&self, req: ResumeSession) -> BoxFuture<'_, anyhow::Result<()>>;
    fn list_sessions(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>>;
    fn delete_session(&self, session: &ProviderSessionId) -> BoxFuture<'_, anyhow::Result<()>>;
    /// Start a sign-in radar can show. Adapters whose CLI signs in by its own
    /// command (Codex, Claude Code) keep this default: T8 runs that command
    /// in a PTY tab.
    fn start_login(&self, _host: Option<String>) -> BoxFuture<'_, anyhow::Result<Box<dyn LoginHandle>>> {
        Box::pin(async { Err(anyhow::anyhow!("sign-in is not run by this adapter")) })
    }
    /// The screen of a session that is a terminal tab; `None` for every other
    /// adapter.
    fn terminal(&self, _session: &RadarSessionId) -> Option<TerminalSurface> {
        None
    }
    /// Open a terminal tab running `argv` instead of the adapter's own command (a CLI's login command:
    /// `codex login`). Only a terminal adapter does; the others refuse.
    fn open_command(&self, _req: CreateSession, _argv: Vec<String>) -> BoxFuture<'_, anyhow::Result<ProviderSessionId>> {
        Box::pin(async { Err(anyhow::anyhow!("this provider does not run commands in a terminal tab")) })
    }
    /// Start a terminal tab's program again.
    fn restart_terminal(&self, _session: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Err(anyhow::anyhow!("this session is not a terminal")) })
    }
    /// Where the CLI is and keeps its state. Takes effect from the next start
    /// of the CLI; an adapter that cannot take it keeps this default.
    fn configure(&self, _setup: ProviderSetup) {}
    /// Start what the first session would otherwise wait for: the CLI's client, its sidecar or
    /// its app-server. Starts no session, so nothing appears in the CLI's own list. Idempotent,
    /// and an error is not news (the next call that needs the CLI says why it cannot run);
    /// the part calls it in the background for the default provider and never waits on it.
    fn warm(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    /// A session created and then left alone leaves nothing in the CLI's own session list
    /// (no record of it until a message is sent), so radar may keep one ready for the next new
    /// tab ([`Adapter::discard`] ends it if it is not wanted). The default says no.
    fn idle_session_is_free(&self) -> bool {
        false
    }
    /// Forget a session radar created and never showed or used (a spare that did not fit the
    /// next new tab), and end what the adapter holds for it. The provider keeps no record of
    /// it. Failing is harmless: the process ends with radar.
    fn discard(&self, _session: &RadarSessionId) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    /// The agents and skills the CLI offers a session in `cwd` with `setup`.
    /// An adapter whose CLI cannot list them keeps this default (empty) and
    /// says so in its capabilities.
    fn catalog(&self, _cwd: PathBuf, _setup: SessionSetup) -> BoxFuture<'_, anyhow::Result<Catalog>> {
        Box::pin(async { Ok(Catalog::default()) })
    }
    /// Answer a permission request. Idempotent by request id: a second
    /// answer to the same request is a no-op, not an error.
    fn answer_permission(
        &self,
        session: &RadarSessionId,
        answer: PermissionAnswer,
    ) -> BoxFuture<'_, anyhow::Result<()>>;
    /// Answer (or decline) a question the agent asked. Idempotent by request
    /// id, as permission answers are. Only an adapter whose capabilities say
    /// `questions` is ever asked.
    fn answer_question(
        &self,
        _session: &RadarSessionId,
        _answer: QuestionAnswer,
    ) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Err(anyhow::anyhow!("this provider does not ask questions")) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_is_a_command_only_when_it_starts_with_a_slash_and_a_word() {
        assert_eq!(parse_slash("/env"), Some(("env", None)));
        assert_eq!(parse_slash("  /plan add a readme  "), Some(("plan", Some("add a readme"))));
        assert_eq!(parse_slash("/model\nauto"), Some(("model", Some("auto"))));
        assert_eq!(parse_slash("/etc/hosts has a typo"), None);
        assert_eq!(parse_slash("/ alone"), None);
        assert_eq!(parse_slash("explain /env"), None);
    }
}
