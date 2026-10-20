//! Icons, drawn: the icon in its colour as a span or as words, from the
//! theme's resolved pack. The set, the packs and how a pack resolves are in
//! [`crate::design::icon`] and re-exported here, so a part writes
//! `icon::lead(theme, Icon::Repo)` and `icon::Icon` from one place.
//!
//! ```ignore
//! let mut name = vec![icon::lead(theme, Icon::Repo), Span::raw("api")];
//! ```

use ratatui::text::Span;

use crate::theme::Theme;

pub use crate::design::icon::*;

// ---------------------------------------------------------------------
// Drawing.

/// The icon in its colour, padded to the pack's width; empty when icons
/// are off or the pack has none for it.
pub fn span(theme: &Theme, icon: Icon) -> Span<'static> {
    let r = theme.icons().get(icon);
    if r.glyph.is_empty() {
        return Span::raw("");
    }
    Span::styled(r.glyph.clone(), theme.spec(&r.style))
}

/// The icon and a space, to put before a name; empty (no gap) when there
/// is no icon.
pub fn lead(theme: &Theme, icon: Icon) -> Span<'static> {
    let mut s = span(theme, icon);
    if !s.content.is_empty() {
        s.content = format!("{} ", s.content).into();
    }
    s
}

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

/// The icon in its colour when the pack has one, else `fallback` in
/// `style`: for a state a screen showed as a symbol before icons (`✓`,
/// `●`), so it reads the same with icons off.
pub fn or(theme: &Theme, icon: Icon, fallback: &str, style: ratatui::style::Style) -> Span<'static> {
    let s = span(theme, icon);
    if s.content.is_empty() {
        Span::styled(fallback.to_string(), style)
    } else {
        s
    }
}

/// The one-cell attention mark is meaningful even when decorative icons are off.
pub const ATTENTION_DOT: &str = "●";

/// A tab's attention mark: always warning ink, independent of the icon pack.
pub fn attention(theme: &Theme) -> Span<'static> {
    Span::styled(ATTENTION_DOT, theme.s(crate::design::theme::Slot::StatusWarning))
}

/// Whether icons are drawn at all.
pub fn on(theme: &Theme) -> bool {
    theme.icons().on()
}

/// The cells [`lead`] takes: the pack's width and a space, or none.
pub fn lead_width(theme: &Theme) -> usize {
    let w = theme.icons().width;
    if w == 0 { 0 } else { w + 1 }
}

#[cfg(test)]
mod tests;
