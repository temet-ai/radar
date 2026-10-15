//! What is under the mouse pointer: the area a click there would act on,
//! lit with `bg.hover`, quieter than the selection so it is never taken for
//! the cursor. The host finds the area from the hits of the last frame and
//! the shell paints it over what was drawn, so every element hovers the
//! same way and a part does nothing for it beyond marking what it can
//! click (`Cx::hit_row`, `Cx::hit_label`, …).
//!
//! The cursor row keeps its own look under the pointer. With 16 colours or
//! none, where `bg.hover` cannot be told from the selection, the area is
//! underlined instead.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

use crate::design::theme::{Slot, Tier};
use crate::theme::Theme;

/// Light `area` as hovered.
pub fn paint(buf: &mut Buffer, area: Rect, theme: &Theme) {
    let area = area.intersection(buf.area);
    let selection = theme.bg(Slot::BgSelection).bg;
    let accent = theme.s(Slot::AccentPrimary).fg;
    // A chip on a state's ground (the status line's attention chip) keeps it:
    // its bold base text would not read on the hover shade.
    let states = [Slot::StatusWarning, Slot::StatusSuccess, Slot::StatusError].map(|s| theme.s(s).fg);
    let hover = theme.bg(Slot::BgHover).bg;
    let underline = matches!(theme.tier(), Tier::Mono | Tier::Ansi16) || hover.is_none();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            // The cursor row, the top bar's focused tab and a state's chip stay as drawn.
            let bg = Some(cell.bg).filter(|c| *c != Color::Reset);
            if bg.is_some() && (bg == selection || bg == accent || states.contains(&bg)) {
                continue;
            }
            if underline {
                cell.modifier.insert(Modifier::UNDERLINED);
            } else if let Some(c) = hover {
                cell.bg = c;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::ThemeConfig;
    use ratatui::style::Style;

    fn theme(tier: Tier) -> Theme {
        Theme::build(&ThemeConfig::default(), tier, &[])
    }

    #[test]
    fn the_area_takes_the_hover_background_and_the_cursor_row_keeps_its_own() {
        let t = theme(Tier::TrueColor);
        let mut buf = Buffer::empty(Rect::new(0, 0, 10, 2));
        buf.set_style(Rect::new(0, 1, 10, 1), t.bg(Slot::BgSelection));
        paint(&mut buf, Rect::new(2, 0, 4, 2), &t);
        let hover = t.bg(Slot::BgHover).bg.unwrap();
        assert_eq!(buf[(2, 0)].bg, hover);
        assert_eq!(buf[(5, 0)].bg, hover);
        assert_ne!(buf[(6, 0)].bg, hover, "only the area");
        assert_eq!(Some(buf[(3, 1)].bg), t.bg(Slot::BgSelection).bg, "the cursor row stays");
        assert_ne!(Some(hover), t.bg(Slot::BgSelection).bg, "never the selection's colour");
    }

    #[test]
    fn a_chip_on_a_states_ground_keeps_it() {
        let t = theme(Tier::TrueColor);
        for slot in [Slot::StatusWarning, Slot::StatusSuccess, Slot::StatusError] {
            let ground = t.s(slot).fg.unwrap();
            let mut buf = Buffer::empty(Rect::new(0, 0, 10, 1));
            buf.set_style(Rect::new(3, 0, 5, 1), Style::new().bg(ground));
            paint(&mut buf, Rect::new(0, 0, 10, 1), &t);
            assert_eq!(buf[(4, 0)].bg, ground, "{slot:?}");
            assert_eq!(buf[(1, 0)].bg, t.bg(Slot::BgHover).bg.unwrap(), "the key before it is lit");
        }
    }

    #[test]
    fn with_few_colours_or_none_it_is_underlined() {
        for tier in [Tier::Ansi16, Tier::Mono] {
            let t = theme(tier);
            let mut buf = Buffer::empty(Rect::new(0, 0, 6, 1));
            buf.set_string(0, 0, "repo", Style::new());
            paint(&mut buf, Rect::new(0, 0, 4, 1), &t);
            assert!(buf[(1, 0)].modifier.contains(Modifier::UNDERLINED), "{tier:?}");
            assert!(!buf[(4, 0)].modifier.contains(Modifier::UNDERLINED));
        }
    }
}
