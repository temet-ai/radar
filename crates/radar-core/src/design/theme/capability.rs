//! Terminal color-capability detection — TUI-C2 / TUI-C3.
//!
//! Four tiers, detected once from the environment at construction and then
//! carried as data. Detection is split in two so tests never touch process
//! state: [`Tier::from_env`] is a pure function over an injected
//! [`TerminalEnv`], and [`Tier::detect`] is the thin wrapper that reads the
//! three real variables and hands them to it.
//!
//! `NO_COLOR` wins unconditionally (TUI-C3): when it is present and non-empty
//! the tier is [`Tier::Mono`] no matter what `COLORTERM` or `TERM` claim.

use std::env;

/// A terminal's color capability, ordered worst to best.
///
/// The ordering is meaningful: `Mono < Ansi16 < Ansi256 < TrueColor`, so a
/// caller can write `if tier >= Tier::Ansi256 { … }`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Tier {
    /// No color at all. Hierarchy must survive on modifiers and words alone
    /// (TUI-P4 / TUI-P7 tier 1).
    Mono,
    /// The 16 named ANSI colors, inheriting the user's terminal theme.
    Ansi16,
    /// The fixed xterm-256 palette. Values come from the TUI-C4a fallback
    /// column, never from a runtime approximation.
    Ansi256,
    /// 24-bit color. Slot values pass through unchanged.
    TrueColor,
}

impl Tier {
    /// Whether this tier renders any color at all.
    #[must_use]
    pub const fn is_color(self) -> bool {
        !matches!(self, Self::Mono)
    }

    /// Read `NO_COLOR`, `COLORTERM` and `TERM` from the process environment
    /// **once** and classify them.
    ///
    /// Call this at startup and store the result; do not call it per frame.
    /// Tests must use [`Tier::from_env`] with explicit values instead — this
    /// crate never mutates the environment.
    #[must_use]
    pub fn detect() -> Self {
        let no_color = env::var("NO_COLOR").ok();
        let colorterm = env::var("COLORTERM").ok();
        let term = env::var("TERM").ok();
        Self::from_env(&TerminalEnv {
            no_color: no_color.as_deref(),
            colorterm: colorterm.as_deref(),
            term: term.as_deref(),
        })
    }

    /// Classify an explicit environment. Pure — this is the injectable half of
    /// detection and the only half tests should exercise.
    ///
    /// Precedence, highest first:
    /// 1. `NO_COLOR` present and non-empty → [`Tier::Mono`] (TUI-C3,
    ///    unconditional — nothing below can raise it again).
    /// 2. `COLORTERM` containing `truecolor` or `24bit` → [`Tier::TrueColor`].
    /// 3. `TERM` containing `direct` (e.g. `xterm-direct`) →
    ///    [`Tier::TrueColor`].
    /// 4. `TERM` containing `256color` → [`Tier::Ansi256`].
    /// 5. `TERM` absent, empty, or `dumb` → [`Tier::Mono`] (no terminal
    ///    information is not a licence to paint).
    /// 6. Anything else → [`Tier::Ansi16`], the TUI-C2 foundation.
    #[must_use]
    pub fn from_env(env: &TerminalEnv<'_>) -> Self {
        if env.no_color.is_some_and(|v| !v.is_empty()) {
            return Self::Mono;
        }

        if let Some(colorterm) = env.colorterm {
            let lower = colorterm.to_ascii_lowercase();
            if lower.contains("truecolor") || lower.contains("24bit") {
                return Self::TrueColor;
            }
        }

        let Some(term) = env.term else {
            return Self::Mono;
        };
        let term = term.to_ascii_lowercase();
        if term.is_empty() || term == "dumb" {
            return Self::Mono;
        }
        if term.contains("direct") {
            return Self::TrueColor;
        }
        if term.contains("256color") {
            return Self::Ansi256;
        }
        Self::Ansi16
    }
}

/// The three environment variables TUI-C3 names, as injectable data.
///
/// `None` means "unset"; `Some("")` means "set to the empty string" — the two
/// differ for `NO_COLOR`, whose published contract only triggers on a
/// non-empty value.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TerminalEnv<'a> {
    /// `$NO_COLOR`.
    pub no_color: Option<&'a str>,
    /// `$COLORTERM`.
    pub colorterm: Option<&'a str>,
    /// `$TERM`.
    pub term: Option<&'a str>,
}

impl<'a> TerminalEnv<'a> {
    /// Build an environment snapshot from explicit values.
    #[must_use]
    pub const fn new(
        no_color: Option<&'a str>,
        colorterm: Option<&'a str>,
        term: Option<&'a str>,
    ) -> Self {
        Self {
            no_color,
            colorterm,
            term,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_color_wins_over_truecolor() {
        let env = TerminalEnv::new(Some("1"), Some("truecolor"), Some("xterm-256color"));
        assert_eq!(Tier::from_env(&env), Tier::Mono);
    }

    #[test]
    fn empty_no_color_does_not_strip() {
        let env = TerminalEnv::new(Some(""), Some("truecolor"), Some("xterm-256color"));
        assert_eq!(Tier::from_env(&env), Tier::TrueColor);
    }

    #[test]
    fn colorterm_truecolor_and_24bit() {
        for value in ["truecolor", "24bit", "TrueColor"] {
            let env = TerminalEnv::new(None, Some(value), Some("xterm"));
            assert_eq!(Tier::from_env(&env), Tier::TrueColor, "COLORTERM={value}");
        }
    }

    #[test]
    fn term_256color_is_ansi256() {
        let env = TerminalEnv::new(None, None, Some("screen-256color"));
        assert_eq!(Tier::from_env(&env), Tier::Ansi256);
    }

    #[test]
    fn term_direct_is_truecolor() {
        let env = TerminalEnv::new(None, None, Some("xterm-direct"));
        assert_eq!(Tier::from_env(&env), Tier::TrueColor);
    }

    #[test]
    fn plain_term_is_ansi16() {
        let env = TerminalEnv::new(None, None, Some("xterm"));
        assert_eq!(Tier::from_env(&env), Tier::Ansi16);
    }

    #[test]
    fn dumb_or_absent_term_is_mono() {
        assert_eq!(
            Tier::from_env(&TerminalEnv::new(None, None, Some("dumb"))),
            Tier::Mono
        );
        assert_eq!(
            Tier::from_env(&TerminalEnv::new(None, None, Some(""))),
            Tier::Mono
        );
        assert_eq!(Tier::from_env(&TerminalEnv::default()), Tier::Mono);
    }

    #[test]
    fn tiers_are_ordered_worst_to_best() {
        assert!(Tier::Mono < Tier::Ansi16);
        assert!(Tier::Ansi16 < Tier::Ansi256);
        assert!(Tier::Ansi256 < Tier::TrueColor);
        assert!(!Tier::Mono.is_color());
        assert!(Tier::Ansi16.is_color());
    }
}
