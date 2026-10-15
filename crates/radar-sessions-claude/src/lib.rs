//! Claude Code for radar's agent dock, through the Agent SDK.
//!
//! The SDK is TypeScript, so a small Node package (`sidecars/claude-agent`) hosts it and
//! speaks newline-delimited JSON over stdio. [`link`] supervises that process; [`map`] turns
//! its event lines into radar's envelope; [`ClaudeAdapter`] implements the provider-neutral
//! `Adapter` on top.
//!
//! Credentials: radar never reads or stores them. The sidecar drives the operator's own installed
//! `claude`, which uses the login that CLI already has. Anthropic does not allow third-party
//! products to offer claude.ai login or rate limits unless it approved them (Agent SDK
//! overview), so radar offers no sign-in of its own: Settings → Agents shows Claude Code's
//! `claude auth status` and runs `claude auth login` in a PTY tab. An API key is the supported
//! route for a product.

pub mod adapter;
pub mod link;
pub mod map;

pub use adapter::{ClaudeAdapter, ClaudeConfig, MIN_CLAUDE_CODE};
pub use link::{Link, LinkMsg, ProcessSpawner, Spawned, Spawner};
