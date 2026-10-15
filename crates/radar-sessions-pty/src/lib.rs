//! Any CLI's own TUI in a terminal tab of the agent dock.
//!
//! The host runs the program in a PTY ([`term`]), keeps its recent output and
//! exit status, and serves both on a Unix socket ([`server`], wire in
//! [`frame`]). The Ink renderer (`ink/src/terminal/`) attaches, parses the
//! bytes into a screen, and sends keys, mouse reports and resizes back. The
//! program therefore survives a renderer restart, and what the renderer shows
//! after one is the screen as it was.
//!
//! As an [`radar_sessions::Adapter`] it is honest: no structured events and no
//! permission requests, because the CLI asks for those on its own screen.

pub mod adapter;
pub mod frame;
pub mod server;
pub mod term;

pub use adapter::{for_cli, socket_dir, standard_adapters, PtyAdapter, PtyCommand, TerminalInfo};
pub use frame::StateFrame;
pub use server::ServedTerminal;
pub use term::{TermState, Terminal, TerminalSpec};
