//! radar-sessions adapter for Codex, over `codex app-server` (JSON-RPC over
//! stdio). `CodexAdapter` implements `radar_sessions::Adapter`; the binary
//! registers it with the sessions part.

mod adapter;
pub mod commands;
pub mod map;
mod rpc;
pub mod setup;
pub mod wire;

#[cfg(test)]
mod contract_tests;
#[cfg(test)]
mod replay;
#[cfg(test)]
mod tests;

pub use adapter::{
    CodexAdapter, CodexConfig, SessionPolicy, auth_from_account, capabilities, login_command,
};
pub use wire::{ChildSpawner, Spawner, Wire};
