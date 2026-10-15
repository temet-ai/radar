//! The mouse pointer's shape (OSC 22): a hand over what a click acts on,
//! a text cursor while a drag selects, the terminal's own arrow elsewhere.
//!
//! Only where the terminal is known to take it (`[theme] pointer =
//! auto|on|off`, [`wanted`]): Ghostty, kitty, foot and WezTerm. Never
//! inside tmux or screen unless `on`, since neither passes it on by
//! default. The runner writes the sequence between frames ([`sequence`])
//! and puts the arrow back when radar stops.

use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the runner changes the pointer.
static ON: AtomicBool = AtomicBool::new(false);

/// Whether radar changes the pointer here.
pub fn on() -> bool {
    ON.load(Ordering::Relaxed)
}

/// Change the pointer (the runner, once it knows the terminal), or not.
pub fn set(on: bool) {
    ON.store(on, Ordering::Relaxed);
}

/// A pointer shape, by its CSS name (what OSC 22 takes).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Shape {
    /// The terminal's own.
    #[default]
    Default,
    /// A hand: a click here acts.
    Hand,
    /// A text cursor: a drag is selecting.
    Text,
}

impl Shape {
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Hand => "pointer",
            Self::Text => "text",
        }
    }
}

/// The sequence that sets the pointer to `shape`.
pub fn sequence(shape: Shape) -> String {
    format!("\x1b]22;{}\x1b\\", shape.name())
}

/// `[theme] pointer`: `on`, `off`, or `auto` ([`detect`]).
pub fn wanted(setting: &str, var: &dyn Fn(&str) -> Option<String>) -> bool {
    match setting {
        "on" => true,
        "off" => false,
        _ => detect(var),
    }
}

/// Whether this terminal is one known to change the pointer on OSC 22,
/// and nothing between (tmux, screen) would swallow it.
pub fn detect(var: &dyn Fn(&str) -> Option<String>) -> bool {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    let term = var("TERM").unwrap_or_default();
    if term == "dumb" || term.starts_with("screen") || term.starts_with("tmux") || set("TMUX") || set("STY") {
        return false;
    }
    let program = var("TERM_PROGRAM").unwrap_or_default();
    if ["ghostty", "WezTerm"].contains(&program.as_str()) {
        return true;
    }
    ["kitty", "ghostty", "wezterm", "foot"].iter().any(|t| term.contains(t)) || set("KITTY_WINDOW_ID")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
    }

    #[test]
    fn known_terminals_change_it_and_tmux_never_unless_asked() {
        assert!(detect(&env(&[("TERM_PROGRAM", "ghostty"), ("TERM", "xterm-256color")])));
        assert!(detect(&env(&[("TERM", "xterm-kitty")])));
        assert!(detect(&env(&[("TERM", "foot")])));
        assert!(detect(&env(&[("TERM_PROGRAM", "WezTerm")])));
        assert!(!detect(&env(&[("TERM", "xterm-256color")])), "an unknown terminal: no");
        assert!(!detect(&env(&[("TERM_PROGRAM", "ghostty"), ("TMUX", "/tmp/tmux-1000/default,1,0")])));
        assert!(!detect(&env(&[("TERM", "tmux-256color"), ("TERM_PROGRAM", "ghostty")])));
        assert!(wanted("on", &env(&[("TMUX", "x")])), "on means on, even in tmux");
        assert!(!wanted("off", &env(&[("TERM_PROGRAM", "ghostty")])));
    }

    #[test]
    fn the_sequence_names_the_shape() {
        assert_eq!(sequence(Shape::Hand), "\x1b]22;pointer\x1b\\");
        assert_eq!(sequence(Shape::Text), "\x1b]22;text\x1b\\");
        assert_eq!(sequence(Shape::Default), "\x1b]22;default\x1b\\");
    }
}
