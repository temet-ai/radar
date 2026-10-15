//! The review journey (task T11): a change request asked for in chat,
//! reviewed locally by an agent in a worktree at its head sha, saved as a
//! local note. Nothing is posted anywhere.
//!
//! - [`journey`]: the steps as a state machine with no I/O;
//! - [`policy`]: what the reviewer may do without asking;
//! - [`note`]: the saved review, and "reviewed" / "stale" for the row.

pub mod driver;
pub mod journey;
pub mod note;
pub mod policy;

pub use journey::{Action, Input, Journey, SessionSpec, Step, recipe};
pub use note::{ReviewNote, ReviewRecord, Reviews, Standing};
pub use policy::{Verdict, decide};

#[cfg(test)]
mod plugin_tests;
#[cfg(test)]
mod tests;
