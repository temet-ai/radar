//! Panels and the detail card: a titled panel whose text scrolls by the
//! caller's [`Scroll`] (`↑↓` when it has focus, `PgUp`/`PgDn` from anywhere),
//! with a scrollbar on its edge when there is more. What can be done to the
//! thing shown is in the `x` menu, not on the card.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};

use super::icon::{self, Icon};
use super::scroll::{Scroll, scrolled};
use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::widgets;

/// A detail card's heading and quiet rule, on the card's normal surface.
pub fn section(
    theme: &Theme,
    mark: Option<Icon>,
    title: &str,
    detail: Option<&str>,
    width: u16,
) -> Line<'static> {
    let mut line = widgets::heading(theme, title);
    if let Some(mark) = mark {
        line.spans.insert(0, icon::lead(theme, mark));
    }
    if let Some(detail) = detail.filter(|detail| line.width() + 2 + super::width(detail) <= width as usize) {
        line.spans.push(Span::styled(format!("  {detail}"), theme.s(Slot::TextSecondary)));
    }
    let rest = (width as usize).saturating_sub(line.width() + 1);
    if rest > 0 {
        line.spans.push(Span::styled(format!(" {}", "─".repeat(rest)), theme.s(Slot::TextMuted)));
    }
    line
}

/// Inset section content by two cells, preserving its own styles.
pub fn section_body(mut line: Line<'static>) -> Line<'static> {
    line.spans.insert(0, Span::raw("  "));
    line
}

/// A rounded, titled panel (accent border when focused); returns the area
/// inside its border.
pub fn panel(theme: &Theme, area: Rect, buf: &mut Buffer, title: &str, focused: bool) -> Rect {
    let block = widgets::panel(theme, title, focused);
    let inner = block.inner(area);
    block.render(area, buf);
    inner
}

/// The title of the foot of a list's panel in every part: it says what the
/// panel shows. Write its words plain and lower case.
pub const SHOWS: &str = "shows";

/// A fixed area at the foot of a panel's `inner` area: a rule with `title`
/// in it, then `height` lines for `lines`, wrapped, the last cut with `…`
/// when there is more. It is drawn only while the panel keeps at least
/// three rows above it, and shrinks to fit before it goes. Returns the area
/// left above it, for the panel's list. A list's panel says here what it
/// shows ([`shows`]), so the words never mix with the rows and never
/// crowd the nav pane. A list drawn in the area left takes [`above`] as its
/// frame, so its scrollbar stops here.
pub fn footer(
    theme: &Theme,
    inner: Rect,
    buf: &mut Buffer,
    title: &str,
    lines: Vec<Line<'static>>,
    height: u16,
) -> Rect {
    const KEEP: u16 = 3;
    let height = height.min(inner.height.saturating_sub(KEEP + 1));
    if height == 0 || inner.width < 4 {
        return inner;
    }
    let rule_y = inner.y + inner.height - height - 1;
    let muted = theme.s(Slot::TextMuted);
    let label = format!("─ {title} ");
    let mut rule = vec![Span::styled(label.clone(), muted)];
    let rest = (inner.width as usize).saturating_sub(super::width(&label));
    rule.push(Span::styled("─".repeat(rest), muted));
    buf.set_line(inner.x, rule_y, &Line::from(rule), inner.width);
    let text = Rect {
        x: inner.x + 1,
        y: rule_y + 1,
        width: inner.width.saturating_sub(2),
        height,
    };
    let para = Paragraph::new(lines).wrap(Wrap { trim: true });
    let more = para.line_count(text.width) > height as usize;
    para.render(text, buf);
    if more {
        // The last line says there is more: `…` right after its words, or
        // over its last cell when it is full.
        let y = text.y + height - 1;
        let end = (text.x..text.x + text.width)
            .rev()
            .find(|x| !buf[(*x, y)].symbol().trim().is_empty())
            .map_or(text.x, |x| x + 1);
        let x = end.min(text.x + text.width.saturating_sub(1));
        buf[(x, y)].set_symbol("…");
    }
    Rect {
        height: rule_y - inner.y,
        ..inner
    }
}

/// The foot of a list's panel: [`footer`] titled [`SHOWS`], as tall as
/// `lines` wrap to in the panel's width, at most `most` lines (the last
/// cut with `…`). A sync that failed goes first, then the view in words.
/// Nothing is drawn for no lines. Returns the area left above it.
pub fn shows(theme: &Theme, inner: Rect, buf: &mut Buffer, lines: Vec<Line<'static>>, most: u16) -> Rect {
    if lines.is_empty() {
        return inner;
    }
    let width = inner.width.saturating_sub(2);
    let height = Paragraph::new(lines.clone()).wrap(Wrap { trim: true }).line_count(width) as u16;
    footer(theme, inner, buf, SHOWS, lines, height.clamp(1, most.max(1)))
}

/// The part of a panel's `frame` beside `list` (what [`footer`] left): the
/// frame a list's scrollbar runs down, so it stops at the footer's rule.
pub fn above(frame: Rect, list: Rect) -> Rect {
    Rect {
        height: (list.y + list.height + 1).saturating_sub(frame.y).min(frame.height),
        ..frame
    }
}

/// A card: `lines` in a titled panel, one cell of padding each side,
/// scrolled. Returns the text's area.
pub fn card(
    theme: &Theme,
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    focused: bool,
    scroll: &Scroll,
    lines: Vec<Line<'static>>,
) -> Rect {
    let inner = widgets::padded(panel(theme, area, buf, title, focused));
    scrolled(theme, scroll, area, inner, buf, lines);
    inner
}
