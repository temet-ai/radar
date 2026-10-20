//! The help bar's key hints: navigation keys, nothing else.
//!
//! "**No statuses allowed.** The help bar contains navigation help only.
//! System state, counts, connection, and progress go elsewhere" (LAYOUT.md);
//! TUI-P3 caps it at the three-to-five most important keys, with the rest
//! behind `?`. A hint has a key label and an action verb and no other field,
//! so a caller cannot express a status here even by accident.

/// One navigation hint: the key or keys, and what they do.
///
/// The action is a navigation verb — `move`, `panel`, `open`, `back`, `help`.
/// An application action (`restart`, `delete`) belongs in the `?` overlay, not
/// here (TUI-K2 layer L2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyHint<'a> {
    /// The key label, e.g. `"j/k"` or `"⏎"`.
    pub keys: &'a str,
    /// What the key does, e.g. `"move"`.
    pub action: &'a str,
}

impl<'a> KeyHint<'a> {
    /// Build a hint.
    #[must_use]
    pub const fn new(keys: &'a str, action: &'a str) -> Self {
        Self { keys, action }
    }
}
