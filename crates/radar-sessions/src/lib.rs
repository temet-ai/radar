//! Agent sessions: the part that owns them and feeds the agent dock.
//!
//! Provider-neutral. Each CLI (Copilot, Codex, Claude Code, any command in a
//! PTY) is an [`Adapter`] in its own crate; they all speak [`Event`]s and
//! [`PermissionRequest`]s defined here, and all run the shared tests in
//! [`contract`]. The host owns the state; this part holds session identity,
//! cursors and bounded metadata only.

pub mod adapter;
pub mod brief;
pub mod contract;
pub mod dock;
pub mod event;
pub mod fake;
pub mod login;
pub mod plugin;
pub mod review;
pub mod settings;
pub mod store;
pub mod tools;
pub mod warm;

pub use adapter::parse_slash;
pub use adapter::{
    Adapter, AdapterStatus, AuthStatus, BoxFuture, Capabilities, Catalog, CatalogEntry, CreateSession, LoginEvent,
    LoginHandle, ModelInfo, ProviderSessionInfo, ProviderSetup, ResumeSession, SendInput, SessionSetup, SlashCommand, TerminalStatus,
    TerminalSurface, ToolServer, Unsupported, Update,
};
pub use event::{
    AgentRef, Durability, Event, EventKind, RadarSessionId, PermissionAnswer, PermissionDecision,
    PermissionKind, PermissionRequest, PermissionTarget, ProviderEventId, ProviderId,
    ProviderSessionId, Question, QuestionAnswer, QuestionChoice, QuestionDecision, QuestionReply,
    QuestionRequest,
};
pub use plugin::SessionsPlugin;
