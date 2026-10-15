//! The seam between the adapter and the Copilot SDK. The adapter owns everything radar
//! decides (envelope, sequence, permission waiting, local-only); a [`Backend`] only
//! talks to Copilot and pushes raw events and permission requests into a [`Sink`].
//! [`crate::sdk::SdkBackend`] is the real one; tests use a replay of a recorded stream.

use std::path::Path;
use std::sync::Arc;

use radar_sessions::adapter::{Catalog, ProviderSetup, SessionSetup};
use radar_sessions::{
    BoxFuture, RadarSessionId, ModelInfo, PermissionDecision, ProviderSessionInfo, QuestionDecision,
};

use crate::commands::{CommandInfo, Invoked};
use crate::map::RawEvent;

/// What `get_status` and `get_auth_status` said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackendStatus {
    pub version: String,
    pub protocol: u32,
    pub authenticated: bool,
    pub login: Option<String>,
    pub host: Option<String>,
    pub message: Option<String>,
}

/// Why a status could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusError {
    /// No `copilot` executable.
    Missing,
    /// It is there but would not talk.
    Connection(String),
}

/// Where a backend delivers what Copilot says about one session.
pub trait Sink: Send + Sync {
    /// One session event, in the order Copilot gave it.
    fn event(&self, event: RawEvent);
    /// Copilot asks permission. Resolves when the operator has answered; it waits as
    /// long as it takes and never decides. `data` is the request as Copilot sent it.
    fn permission(
        &self,
        request_id: String,
        data: serde_json::Value,
    ) -> BoxFuture<'_, PermissionDecision>;
    /// Copilot's `ask_user` asks the operator a question. Resolves when the operator has
    /// answered or declined; it waits as long as it takes and never answers by itself.
    /// The SDK hands the handler no request id, so the adapter finds it in the
    /// `user_input.requested` event that comes with the question.
    fn question(
        &self,
        question: String,
        choices: Vec<String>,
        allow_freeform: bool,
    ) -> BoxFuture<'_, QuestionDecision>;
    /// The connection to Copilot is gone.
    fn lost(&self, reason: String);
    /// Some live events were dropped before they were read (the reader fell behind); the
    /// stream goes on. `what` says how many.
    fn missed(&self, _what: String) {}
    /// The connection to Copilot is back (the part then resumes from its cursor).
    fn restored(&self);
}

pub trait Backend: Send + Sync + 'static {
    /// Start the client (the `copilot` child) so the first call does not wait for it. A backend
    /// with nothing to start keeps this default.
    fn warm(&self) -> BoxFuture<'_, anyhow::Result<()>> {
        Box::pin(async { Ok(()) })
    }
    fn status(&self) -> BoxFuture<'_, Result<BackendStatus, StatusError>>;
    fn models(&self) -> BoxFuture<'_, anyhow::Result<Vec<ModelInfo>>>;
    /// A new session with radar's id (Copilot keeps it as its own session id).
    fn create<'a>(
        &'a self,
        id: &'a RadarSessionId,
        cwd: &'a Path,
        model: Option<&'a str>,
        setup: &'a SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>>;
    /// Reattach and push the persisted history through the sink before live events.
    fn resume<'a>(
        &'a self,
        id: &'a RadarSessionId,
        cwd: &'a Path,
        setup: &'a SessionSetup,
        sink: Arc<dyn Sink>,
    ) -> BoxFuture<'a, anyhow::Result<()>>;
    fn send<'a>(&'a self, id: &'a RadarSessionId, text: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;
    fn abort<'a>(&'a self, id: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<()>>;
    /// The slash commands the open session has (`session.commands.list`). A backend that
    /// cannot say has none.
    fn commands<'a>(&'a self, _id: &'a RadarSessionId) -> BoxFuture<'a, anyhow::Result<Vec<CommandInfo>>> {
        Box::pin(async { Ok(vec![]) })
    }
    /// Run one of them (`session.commands.invoke`); `input` is what follows the name.
    fn invoke<'a>(&'a self, _id: &'a RadarSessionId, _name: &'a str, _input: Option<&'a str>) -> BoxFuture<'a, anyhow::Result<Invoked>> {
        Box::pin(async { Err(anyhow::anyhow!("this backend does not run slash commands")) })
    }
    /// Submit the prompt a command produced, shown in the transcript as `display`; `plan` asks it
    /// in Copilot's plan mode.
    fn send_prompt<'a>(&'a self, _id: &'a RadarSessionId, _prompt: &'a str, _display: &'a str, _plan: bool) -> BoxFuture<'a, anyhow::Result<()>> {
        Box::pin(async { Err(anyhow::anyhow!("this backend does not run slash commands")) })
    }
    fn delete<'a>(&'a self, provider_session_id: &'a str) -> BoxFuture<'a, anyhow::Result<()>>;
    fn list(&self) -> BoxFuture<'_, anyhow::Result<Vec<ProviderSessionInfo>>>;
    /// Where `copilot` is and keeps its state, from the next start of the CLI on.
    /// Returns a note for the log when it cannot apply yet (sessions are open).
    fn configure(&self, _setup: ProviderSetup) -> Option<String> {
        None
    }
    /// The agents and skills a session in `cwd` with `setup` would have.
    fn catalog<'a>(
        &'a self,
        _cwd: &'a Path,
        _setup: &'a SessionSetup,
    ) -> BoxFuture<'a, anyhow::Result<Catalog>> {
        Box::pin(async { Ok(Catalog::default()) })
    }
}
