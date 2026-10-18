//! radar-core: the host every radar ability plugs into.
//!
//! Write a plugin by implementing [`Plugin`]; add it to the list in the `radar`
//! binary. The host gives it the universal layout (TUI Design), a settings
//! section, a `[<id>]` config table, a cache, a bus to other plugins and a
//! `radar <id>` CLI.

pub mod agent_context;
pub mod check;
pub mod colours;
pub mod compose;
pub mod context;
pub mod agent_text;
pub mod design;
pub mod dock;
pub mod health;
#[cfg(test)]
mod health_host_tests;
pub mod host;
pub mod headless;
pub mod history;
pub mod icons;
pub mod input;
pub mod ipc;
pub mod journal;
mod journal_runtime;
pub mod log;
pub mod mode;
pub mod mnemonic;
pub mod motion;
pub mod notices;
pub mod palette;
mod parts;
pub mod plugin;
mod plugin_cache;
pub mod runner;
pub mod runtime;
#[cfg(feature = "test-support")]
pub mod scenario;
pub mod select;
pub mod settings_view;
pub mod setup;
pub mod shell;
pub mod shot;
pub mod source_context;
pub mod splash;
pub mod start_lines;
pub mod system;
mod task_completion;
mod terminal_host;
pub mod tour;
pub mod theme;
pub mod time;
pub mod ui;
pub mod view;
pub mod view_tui;
pub mod visual;
pub mod wake;
pub mod widgets;
pub mod work_api;
mod work_cli;
pub mod writes;

#[cfg(test)]
mod settings_navigation_tests;

pub use compose::{
    Action, Adds, Cell, Chip, Contribution, Offer, Owned, Parts, Point, Row, Setup, Showing,
    Subject,
};
pub use host::Host;
pub use plugin::{
    Binding, CliExecution, CliObservation, CliObservations, CliTask, Effect, MAX_CLI_OBSERVATIONS,
    Menu, MenuItem, Msg, ObservedCliTask, Outcome, Pane, Plugin, ScreenGroup, ScreenSpec,
    SectionSpec, SetupFinds, SetupGroup, SetupStep, Task, Tone, Typing, Verdict,
};
pub use theme::Theme;

// Re-exported so plugins build against the host's exact versions.
pub use opaline;
pub use ratatui;
