//! Theme-resolved focus and wordmark ramps. The caller still owns every
//! glyph, title style, base focus style, and the order of its scrollbars.

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::design::theme::{Slot, Tier};
use crate::theme::{Theme, mix};

/// Legacy focus styling and actual keyboard ownership are independent.
#[derive(Clone, Copy)]
pub struct Focus {
    pub focused: bool,
    pub owns_keys: bool,
}

type Rgb = (u8, u8, u8);

#[derive(Clone, Copy)]
struct Ramp([Rgb; 3]);

impl Ramp {
    fn from_theme(theme: &Theme) -> Option<Self> {
        if theme.tier() != Tier::TrueColor {
            return None;
        }
        let rgb = |slot| match theme.s(slot).fg {
            Some(Color::Rgb(r, g, b)) => Some((r, g, b)),
            _ => None,
        };
        Some(Self([
            rgb(Slot::GlowPurple)?,
            rgb(Slot::AccentTertiary)?,
            rgb(Slot::GlowCyan)?,
        ]))
    }

    /// Endpoints are inclusive. One or zero cells use the first stop.
    /// Quantize the two-segment position down to integer per-mille, then
    /// let the shared theme mixer round each channel half up.
    fn at(self, index: usize, cells: usize) -> Color {
        let last = cells.saturating_sub(1);
        let phase = if last == 0 {
            0
        } else {
            ((index.min(last) as u128 * 2000) / last as u128) as u32
        };
        if phase <= 1000 {
            mix(self.0[0], self.0[1], phase)
        } else {
            mix(self.0[1], self.0[2], phase - 1000)
        }
    }
}

/// Clockwise from top-left, with each corner once. Degenerate rectangles
/// run left-to-right (one row) or top-to-bottom (one column).
pub(crate) fn perimeter(area: Rect) -> impl ExactSizeIterator<Item = Position> {
    // Match Rect's saturating coordinate bounds even for a struct literal.
    let w = usize::from(area.right() - area.x);
    let h = usize::from(area.bottom() - area.y);
    let cells = match (w, h) {
        (0, _) | (_, 0) => 0,
        (_, 1) => w,
        (1, _) => h,
        _ => 2 * w + 2 * h - 4,
    };
    (0..cells).map(move |i| {
        let (x, y) = if h == 1 {
            (i, 0)
        } else if w == 1 {
            (0, i)
        } else if i < w {
            (i, 0)
        } else if i < w + h - 1 {
            (w - 1, i - w + 1)
        } else if i < 2 * w + h - 2 {
            (2 * w + h - 3 - i, h - 1)
        } else {
            (0, cells - i)
        };
        Position::new(area.x + x as u16, area.y + y as u16)
    })
}

/// Which of an all-sided panel's horizontal edges holds a left-aligned title.
#[derive(Clone, Copy)]
pub enum TitleEdge {
    Top,
    Bottom,
}

/// Reserve the explicit display-cell width of one left-aligned title,
/// including any caller-provided padding. This is the geometry used by
/// `widgets::panel`: one title per edge, with a border at both sides.
/// Pass the padded title's width, not its byte or character count.
pub fn title_region(area: Rect, width: usize, edge: TitleEdge) -> Rect {
    if area.is_empty() {
        return Rect::default();
    }
    Rect::new(
        area.x.saturating_add(1),
        match edge {
            TitleEdge::Top => area.y,
            TitleEdge::Bottom => area.bottom().saturating_sub(1),
        },
        width.min(usize::from(area.width.saturating_sub(2))) as u16,
        1,
    )
}

/// Recolour truecolor panel borders with the owner ramp or quiet accent.
/// `owns_keys` is independent of the existing
/// base Block's focused flag, which must keep its lower-tier behaviour.
///
/// Call immediately after the Block and before content or scrollbars.
/// `keep` explicitly names title regions (or other edge regions) to leave
/// alone. Those regions still count toward the full perimeter's position.
/// No glyph or existing style is inspected to guess what is a border.
/// Only foreground changes; symbols, backgrounds and modifiers survive.
///
/// Pass the actual rendered panel rectangle. If clipping a Block to the
/// buffer, compute that intersection before both rendering and this call.
pub fn focus_border(theme: &Theme, area: Rect, buf: &mut Buffer, owns_keys: bool, keep: &[Rect]) {
    crate::motion::frames::frame(area, owns_keys, keep);
    let Some(ramp) = Ramp::from_theme(theme) else {
        return;
    };
    let cells = perimeter(area);
    let count = cells.len();
    for (i, pos) in cells.enumerate() {
        if keep.iter().any(|region| region.contains(pos)) {
            continue;
        }
        if let Some(cell) = buf.cell_mut(pos) {
            cell.set_fg(if owns_keys { ramp.at(i, count) } else {
                theme.s(Slot::AccentMuted).fg.unwrap_or(Color::Reset)
            });
        }
    }
}

/// A single-line wordmark, coloured left-to-right by display cells.
/// Each grapheme gets the colour at its starting cell and stays whole;
/// a wide grapheme advances the next sample by its full display width.
/// Pass just the wordmark: keep surrounding padding outside the ramp.
/// `base` is the existing caller style, preserved except for foreground.
/// Below truecolor this returns the original content and style unchanged.
pub fn wordmark(theme: &Theme, text: &str, base: Style) -> Vec<Span<'static>> {
    let source = Span::styled(text.to_owned(), base);
    let Some(ramp) = Ramp::from_theme(theme) else {
        return vec![source];
    };
    let width = source.width();
    let mut x = 0;
    source
        .styled_graphemes(Style::default())
        .map(|grapheme| {
            let style = grapheme.style.fg(ramp.at(x, width));
            x += super::width(grapheme.symbol);
            Span::styled(grapheme.symbol.to_owned(), style)
        })
        .collect()
}

#[cfg(test)]
#[path = "frame_tests.rs"]
mod tests;
