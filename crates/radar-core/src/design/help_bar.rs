//! The help bar: the last row, navigation keys, nothing else.
//!
//! "**No statuses allowed.** The help bar contains navigation help only.
//! System state, counts, connection, and progress go elsewhere" (LAYOUT.md);
//! TUI-P3 caps it at the three-to-five most important keys, with the rest
//! behind `?`.
//!
//! Both rules are enforced by the type rather than by review: [`HelpBar`]
//! takes a slice of [`KeyHint`] and a theme, and there is **no** message,
//! badge, mode, count, or status parameter anywhere in its API — a caller
//! cannot express a status here even by accident. The five-hint cap is
//! applied on render, so passing more silently drops the tail rather than
//! producing a wall of shortcuts.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::design::theme::{DesignTheme, Slot};

/// The documented inline separator (TUI-G1), the only glyph this widget
/// contributes on its own.
const SEPARATOR: &str = " \u{b7} ";

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

/// One row of navigation key hints.
#[derive(Debug, Clone, Copy)]
pub struct HelpBar<'a> {
    hints: &'a [KeyHint<'a>],
    theme: &'a DesignTheme,
}

impl<'a> HelpBar<'a> {
    /// Rows the help bar occupies. Always one.
    pub const HEIGHT: u16 = 1;

    /// The most hints that will ever be rendered (TUI-P3: 3–5 keys).
    pub const MAX_HINTS: usize = 5;

    /// Build a help bar for the focused panel's navigation keys.
    ///
    /// Hints beyond [`HelpBar::MAX_HINTS`] are dropped.
    #[must_use]
    pub const fn new(hints: &'a [KeyHint<'a>], theme: &'a DesignTheme) -> Self {
        Self { hints, theme }
    }

    /// The hints that will actually be shown — the first five, at most.
    #[must_use]
    pub fn visible_hints(&self) -> &'a [KeyHint<'a>] {
        let end = self.hints.len().min(Self::MAX_HINTS);
        &self.hints[..end]
    }

    /// The styled line, capped at five hints and at `width` columns.
    ///
    /// A hint that does not fit is dropped whole rather than clipped, so the
    /// bar never shows half a key name.
    #[must_use]
    pub fn line_for_width(&self, width: u16) -> Line<'a> {
        let keys_style = self.theme.style(Slot::AccentSecondary);
        let action_style = self.theme.style(Slot::TextMuted);
        let separator_style = action_style;

        let width = width as usize;
        let mut spans: Vec<Span<'a>> = Vec::with_capacity(Self::MAX_HINTS * 4);
        let mut used = 0usize;

        for (i, hint) in self.visible_hints().iter().enumerate() {
            let separator = if i > 0 { SEPARATOR.chars().count() } else { 0 };
            let cost = separator + hint.keys.chars().count() + 1 + hint.action.chars().count();
            if used + cost > width {
                break;
            }
            if i > 0 {
                spans.push(Span::styled(SEPARATOR, separator_style));
            }
            spans.push(Span::styled(hint.keys, keys_style));
            spans.push(Span::styled(" ", action_style));
            spans.push(Span::styled(hint.action, action_style));
            used += cost;
        }

        Line::from(spans)
    }
}

impl Widget for HelpBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let row = Rect {
            y: area.y + area.height - Self::HEIGHT,
            height: Self::HEIGHT,
            ..area
        };

        let surface = self.theme.bg_style(Slot::BgSurface);
        if surface != Style::new() {
            buf.set_style(row, surface);
        }

        let line = self.line_for_width(row.width);
        buf.set_line(row.x, row.y, &line, row.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::design::theme::Tier;
    use ratatui::style::Color;

    const FIVE: [KeyHint<'static>; 5] = [
        KeyHint::new("\u{2191}\u{2193}/jk", "move"),
        KeyHint::new("\u{21e5}", "panel"),
        KeyHint::new("\u{23ce}", "open"),
        KeyHint::new("\u{238b}", "back"),
        KeyHint::new("?", "help"),
    ];

    fn render(hints: &[KeyHint<'_>], theme: &DesignTheme, width: u16) -> String {
        let area = Rect::new(0, 0, width, 1);
        let mut buf = Buffer::empty(area);
        HelpBar::new(hints, theme).render(area, &mut buf);
        (0..width)
            .map(|x| buf[(x, 0)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    #[test]
    fn renders_the_navigation_pattern() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        assert_eq!(
            render(&FIVE, &theme, 80),
            "\u{2191}\u{2193}/jk move \u{b7} \u{21e5} panel \u{b7} \u{23ce} open \u{b7} \u{238b} back \u{b7} ? help"
        );
    }

    #[test]
    fn never_shows_more_than_five_hints() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let many: Vec<KeyHint<'static>> = vec![
            KeyHint::new("a", "one"),
            KeyHint::new("b", "two"),
            KeyHint::new("c", "three"),
            KeyHint::new("d", "four"),
            KeyHint::new("e", "five"),
            KeyHint::new("f", "six"),
            KeyHint::new("g", "seven"),
        ];
        let bar = HelpBar::new(&many, &theme);
        assert_eq!(bar.visible_hints().len(), HelpBar::MAX_HINTS);

        let text = render(&many, &theme, 120);
        assert!(text.contains("e five"), "the fifth hint is shown: {text}");
        assert!(!text.contains("f six"), "the sixth is dropped: {text}");
        assert!(!text.contains("g seven"), "the seventh is dropped: {text}");
        assert_eq!(
            text.matches('\u{b7}').count(),
            4,
            "five hints, four separators"
        );
    }

    #[test]
    fn fewer_than_five_hints_render_as_given() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let three = &FIVE[..3];
        assert_eq!(HelpBar::new(three, &theme).visible_hints().len(), 3);
        assert_eq!(render(three, &theme, 80).matches('\u{b7}').count(), 2);
    }

    #[test]
    fn a_hint_that_does_not_fit_is_dropped_whole() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let text = render(&FIVE, &theme, 20);
        assert!(text.chars().count() <= 20, "overflowed: {text}");
        assert!(text.starts_with("\u{2191}\u{2193}/jk move"));
        assert!(!text.contains("hel"), "no half-rendered hint: {text}");
    }

    #[test]
    fn an_empty_hint_list_renders_an_empty_row() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        assert_eq!(render(&[], &theme, 40), "");
    }

    #[test]
    fn keys_and_actions_take_different_slots() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let line = HelpBar::new(&FIVE, &theme).line_for_width(80);
        assert_eq!(line.spans[0].style, theme.style(Slot::AccentSecondary));
        assert_eq!(line.spans[2].style, theme.style(Slot::TextMuted));
    }

    #[test]
    fn monochrome_help_bar_carries_no_color() {
        let theme = DesignTheme::with_tier(Tier::Mono);
        let area = Rect::new(0, 0, 60, 1);
        let mut buf = Buffer::empty(area);
        HelpBar::new(&FIVE, &theme).render(area, &mut buf);
        for x in 0..area.width {
            assert_eq!(buf[(x, 0)].fg, Color::Reset, "fg at {x}");
            assert_eq!(buf[(x, 0)].bg, Color::Reset, "bg at {x}");
        }
        // The keys themselves are still readable text.
        let text: String = (0..area.width).map(|x| buf[(x, 0)].symbol()).collect();
        assert!(text.contains("panel"), "{text}");
    }

    #[test]
    fn zero_sized_area_renders_nothing() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
        HelpBar::new(&FIVE, &theme).render(Rect::new(0, 0, 0, 0), &mut buf);
        assert_eq!(buf[(0, 0)].symbol(), " ");
    }
}
