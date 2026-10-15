//! Row selection styling.
//!
//! "If a row is active, it is highlighted. The highlight is the row treatment
//! itself, across the row — selection adds no geometry: no marker column, no
//! leading symbol, no indent shift" (LAYOUT.md), and TUI-G1 bans `▸` as a
//! selection marker outright.
//!
//! So this module hands back **styles**, never glyphs. There is no marker
//! parameter, no marker constant, and no function that returns a prefix — a
//! caller cannot add a marker column through this API. (Icons that say what
//! a row *is* come from one place, [`crate::ui::icon`]; selection still adds
//! none.)
//!
//! The two selected treatments:
//!
//! | where | background |
//! |---|---|
//! | selected row, focused panel | `bg.selection` |
//! | selected row, unfocused panel | `bg.overlay` |
//!
//! Both survive the monochrome tier: focus keeps `BOLD` and selection keeps
//! `UNDERLINED`, so which row is active and which panel owns it are still
//! readable with no color at all (TUI-P4, TUI-P7 tier 1).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};

use crate::design::theme::{DesignTheme, Slot, Tier};

/// The style for a row, given whether its panel has focus and whether the row
/// is the selected one.
#[must_use]
pub fn row(theme: &DesignTheme, panel_focused: bool, selected: bool) -> Style {
    if selected {
        selected_row(theme, panel_focused)
    } else {
        unselected_row(theme)
    }
}

/// The selected row's style: `bg.selection` in the focused panel,
/// `bg.overlay` in an unfocused one.
#[must_use]
pub fn selected_row(theme: &DesignTheme, panel_focused: bool) -> Style {
    let (fg, bg) = if panel_focused {
        (Slot::TextEmphasis, Slot::BgSelection)
    } else {
        (Slot::TextPrimary, Slot::BgOverlay)
    };

    let mut style = theme.on(fg, bg);
    if panel_focused {
        style = style.add_modifier(Modifier::BOLD);
    }
    if theme.tier() == Tier::Mono {
        // With no backgrounds to carry it, the selection needs a modifier of
        // its own or the active row vanishes.
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

/// An unselected row: plain body text on the panel's own surface.
#[must_use]
pub fn unselected_row(theme: &DesignTheme) -> Style {
    theme.style(Slot::TextPrimary)
}

/// Paint `style` across every cell of `row_area`.
///
/// This is the "across the row" half of the rule: the highlight covers the
/// panel's full inner width, not just the text. Pass the row's rectangle
/// inside the panel border.
pub fn fill_row(buf: &mut Buffer, row_area: Rect, style: Style) {
    if row_area.width == 0 || row_area.height == 0 {
        return;
    }
    buf.set_style(row_area, style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn focused_selection_uses_bg_selection() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let style = selected_row(&theme, true);
        assert_eq!(style.bg, theme.color(Slot::BgSelection));
        assert_eq!(style.fg, theme.color(Slot::TextEmphasis));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn unfocused_selection_uses_bg_overlay() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let style = selected_row(&theme, false);
        assert_eq!(style.bg, theme.color(Slot::BgOverlay));
        assert_eq!(style.fg, theme.color(Slot::TextPrimary));
        assert!(!style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn the_two_selected_treatments_are_distinguishable() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        assert_ne!(selected_row(&theme, true), selected_row(&theme, false));
        assert_ne!(selected_row(&theme, true), unselected_row(&theme));
        assert_ne!(selected_row(&theme, false), unselected_row(&theme));
    }

    #[test]
    fn row_dispatches_on_selection() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        assert_eq!(row(&theme, true, true), selected_row(&theme, true));
        assert_eq!(row(&theme, false, true), selected_row(&theme, false));
        assert_eq!(row(&theme, true, false), unselected_row(&theme));
        assert_eq!(row(&theme, false, false), unselected_row(&theme));
    }

    #[test]
    fn selection_stays_visible_and_colorless_in_monochrome() {
        let theme = DesignTheme::with_tier(Tier::Mono);
        for focused in [true, false] {
            let style = selected_row(&theme, focused);
            assert_eq!(style.fg, None, "focused={focused}");
            assert_eq!(style.bg, None, "focused={focused}");
            assert!(
                style.add_modifier.contains(Modifier::UNDERLINED),
                "focused={focused}"
            );
        }
        assert_ne!(selected_row(&theme, true), selected_row(&theme, false));
        assert_ne!(selected_row(&theme, true), unselected_row(&theme));
        assert_ne!(selected_row(&theme, false), unselected_row(&theme));
    }

    #[test]
    fn ansi256_selection_uses_the_fallback_indices() {
        let theme = DesignTheme::with_tier(Tier::Ansi256);
        assert_eq!(selected_row(&theme, true).bg, Some(Color::Indexed(239)));
        assert_eq!(selected_row(&theme, false).bg, Some(Color::Indexed(235)));
    }

    #[test]
    fn fill_row_covers_the_whole_row_and_adds_no_glyph() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let area = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(area);
        buf.set_string(0, 1, "entries", Style::new());

        let row_area = Rect::new(0, 1, 12, 1);
        fill_row(&mut buf, row_area, selected_row(&theme, true));

        for x in 0..12 {
            assert_eq!(
                buf[(x, 1)].bg,
                theme.color(Slot::BgSelection).unwrap(),
                "the highlight spans the full row at {x}"
            );
        }
        let text: String = (0..12).map(|x| buf[(x, 1)].symbol()).collect();
        assert_eq!(text, "entries     ", "no marker glyph, no indent shift");

        // Neighbouring rows are untouched.
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
        assert_eq!(buf[(0, 2)].bg, Color::Reset);
    }

    #[test]
    fn fill_row_ignores_a_zero_sized_row() {
        let theme = DesignTheme::with_tier(Tier::TrueColor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 1));
        fill_row(&mut buf, Rect::new(0, 0, 0, 0), selected_row(&theme, true));
        assert_eq!(buf[(0, 0)].bg, Color::Reset);
    }
}
