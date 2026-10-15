//! Selecting text with the mouse. radar captures the mouse for clicks and the
//! wheel, so the terminal cannot select text itself; the runner does it
//! instead: a drag marks cells as the terminal would (from where it started
//! to where it is, row by row), and letting go copies what was marked.

use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

/// A drag from `anchor` to `head`, in screen cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Position,
    pub head: Position,
}

impl Selection {
    /// Start and end in reading order.
    fn ends(&self) -> (Position, Position) {
        let (a, h) = (self.anchor, self.head);
        if (h.y, h.x) < (a.y, a.x) { (h, a) } else { (a, h) }
    }

    /// Whether the pointer moved far enough to select: onto another row, or
    /// two cells along. A hand that slips one cell while clicking still
    /// clicks (seen live: a click on a tab arrived as a drag of one cell).
    pub fn moved(&self) -> bool {
        self.anchor.y != self.head.y || self.anchor.x.abs_diff(self.head.x) >= 2
    }

    /// The cells it covers on row `y`, as `x` from..=to, within `width`.
    fn span(&self, y: u16, width: u16) -> Option<(u16, u16)> {
        let (s, e) = self.ends();
        if y < s.y || y > e.y || width == 0 {
            return None;
        }
        let from = if y == s.y { s.x } else { 0 };
        let to = if y == e.y { e.x } else { width - 1 };
        Some((from, to.min(width - 1)))
    }
}

/// Show the selection: its cells reversed.
pub fn paint(buf: &mut Buffer, sel: &Selection) {
    let area = buf.area;
    for y in area.top()..area.bottom() {
        if let Some((from, to)) = sel.span(y, area.right()) {
            for x in from.max(area.left())..=to {
                buf[(x, y)].set_style(Style::new().add_modifier(Modifier::REVERSED));
            }
        }
    }
}

/// The text the selection covers, one line per row, without the spaces
/// that pad a row out to the edge.
pub fn text(buf: &Buffer, sel: &Selection) -> String {
    let area = buf.area;
    let mut lines = Vec::new();
    for y in area.top()..area.bottom() {
        if let Some((from, to)) = sel.span(y, area.right()) {
            let mut line = String::new();
            let mut x = from.max(area.left());
            while x <= to {
                let symbol = buf[(x, y)].symbol();
                line.push_str(symbol);
                // A wide character covers the cells after it.
                x += (Span::raw(symbol).width() as u16).max(1);
            }
            lines.push(line.trim_end().to_string());
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    fn screen() -> Buffer {
        Buffer::with_lines(["!263 fix the build   ", "  stage test · 1m    ", "last line            "])
    }

    fn sel(ax: u16, ay: u16, hx: u16, hy: u16) -> Selection {
        Selection {
            anchor: Position::new(ax, ay),
            head: Position::new(hx, hy),
        }
    }

    #[test]
    fn a_drag_on_one_row_copies_the_cells_between() {
        assert_eq!(text(&screen(), &sel(5, 0, 7, 0)), "fix");
    }

    #[test]
    fn across_rows_it_runs_to_the_edge_and_from_the_start_like_a_terminal() {
        let t = text(&screen(), &sel(5, 0, 6, 1));
        assert_eq!(t, "fix the build\n  stage");
    }

    #[test]
    fn dragging_backwards_selects_the_same() {
        let b = screen();
        assert_eq!(text(&b, &sel(6, 1, 5, 0)), text(&b, &sel(5, 0, 6, 1)));
    }

    #[test]
    fn painting_reverses_only_the_selected_cells() {
        let mut b = screen();
        paint(&mut b, &sel(5, 0, 7, 0));
        let reversed = |x, y| {
            let c: &ratatui::buffer::Cell = &b[(x, y)];
            c.modifier.contains(Modifier::REVERSED)
        };
        assert!(reversed(5, 0) && reversed(7, 0));
        assert!(!reversed(4, 0) && !reversed(8, 0) && !reversed(5, 1));
        assert_eq!(b.area, Rect::new(0, 0, 21, 3));
    }

    #[test]
    fn a_wide_character_is_copied_once() {
        let mut b = Buffer::empty(Rect::new(0, 0, 10, 1));
        b.set_string(0, 0, "✅ ok", Style::new());
        assert_eq!(text(&b, &sel(0, 0, 9, 0)), "✅ ok");
    }

    #[test]
    fn a_press_that_does_not_move_is_a_click() {
        assert!(!sel(3, 1, 3, 1).moved());
        assert!(!sel(3, 1, 4, 1).moved(), "a slip of one cell is still a click");
        assert!(!sel(4, 1, 3, 1).moved());
        assert!(sel(3, 1, 5, 1).moved());
        assert!(sel(3, 1, 3, 2).moved(), "another row selects");
    }
}
