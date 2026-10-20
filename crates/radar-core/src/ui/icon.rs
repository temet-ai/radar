//! Icons as words: the glyph the theme's resolved pack gives, as text. The
//! set, the packs and how a pack resolves are in [`crate::design::icon`] and
//! re-exported here, so a part writes `icon::text(theme, Icon::Repo)` and
//! `icon::Icon` from one place. Drawing an icon in its colour is the
//! renderer's.

use crate::theme::Theme;

pub use crate::design::icon::*;

/// The icon and a space as plain text, for words that are a string (a tab,
/// a menu row, a chip): empty when there is no icon.
pub fn text(theme: &Theme, icon: Icon) -> String {
    let r = theme.icons().get(icon);
    if r.glyph.is_empty() { String::new() } else { format!("{} ", r.glyph) }
}

/// `words` with the icon before them, as plain text.
pub fn with(theme: &Theme, icon: Icon, words: &str) -> String {
    format!("{}{words}", text(theme, icon))
}

/// The one-cell attention mark is meaningful even when decorative icons are off.
pub const ATTENTION_DOT: &str = "●";

/// Whether icons are drawn at all.
pub fn on(theme: &Theme) -> bool {
    theme.icons().on()
}

/// The cells an icon and its space take: the pack's width and a space, or none.
pub fn lead_width(theme: &Theme) -> usize {
    let w = theme.icons().width;
    if w == 0 { 0 } else { w + 1 }
}

#[cfg(test)]
mod tests;
