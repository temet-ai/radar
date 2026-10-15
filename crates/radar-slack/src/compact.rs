//! The compact density's one rule the page view shares: messages by the same
//! person this close together leave the author blank. The page itself is
//! laid out from view data (`page/neutral.rs`); there is no drawing here.

/// Messages by the same person this close (seconds) leave the author blank.
pub(crate) const WITHIN: i64 = 5 * 60;
