//! Rows: facts beside their labels, a heading with its detail, the `+ add …`
//! row that starts adding, and the selectable list (ratatui's `List`) with a
//! word at each row's right edge, as settings sections and nav panes use.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState, StatefulWidget};

use super::scroll::{Scroll, scrollbar};
use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::widgets;

/// `label   value` rows, the labels muted and padded to the longest.
pub fn kv(theme: &Theme, rows: Vec<(&str, Vec<Span<'static>>)>) -> Vec<Line<'static>> {
    let w = rows
        .iter()
        .map(|(l, _)| super::width(l))
        .max()
        .unwrap_or(0)
        + 2;
    rows.into_iter()
        .map(|(label, value)| widgets::kv(theme, label, w, value))
        .collect()
}

/// `PIPELINE  passed · 6m 12s`: a heading and its detail in plain case.
pub fn labelled(theme: &Theme, heading: &str, detail: &str) -> Line<'static> {
    let mut line = widgets::heading(theme, heading);
    line.spans.push(Span::styled(
        format!("  {detail}"),
        theme.s(Slot::TextSecondary),
    ));
    line
}

/// The `+ …` row that starts adding: an accent `+`, the action, and what it
/// takes when that fits in `width`. Always the first row of its list, so it
/// is visible that more can be added.
pub fn add_row(theme: &Theme, action: &str, takes: &str, width: u16) -> Line<'static> {
    let mut line = Line::from(vec![
        Span::styled(" + ", theme.bold(Slot::AccentPrimary)),
        Span::styled(action.to_string(), theme.bold(Slot::AccentSecondary)),
    ]);
    let hint = Span::styled(format!("   {takes}"), theme.s(Slot::TextMuted));
    if line.width() + hint.width() + 2 <= width as usize {
        line.spans.push(hint);
    }
    line
}

/// One row of a [`list`]: the left part, optional detail after it, and a
/// word at the right edge.
#[derive(Debug, Clone)]
pub struct ListRow {
    pub line: Line<'static>,
    /// Shown whole, with air before the word, or not at all: the detail
    /// panel has it.
    pub detail: Option<Span<'static>>,
    pub right: Option<Span<'static>>,
    /// False for a heading: the cursor never rests on it and it is not
    /// clicked ([`ListRow::heading`], [`select`]).
    pub selectable: bool,
}

impl ListRow {
    pub fn new(line: impl Into<Line<'static>>) -> Self {
        Self {
            line: line.into(),
            detail: None,
            right: None,
            selectable: true,
        }
    }

    /// A heading: drawn in its place among the rows, never selected. A
    /// list that moves with [`select`] (given `selectable` of its rows)
    /// steps over it, and [`RowList::render`] returns no area for it.
    pub fn heading(line: impl Into<Line<'static>>) -> Self {
        Self {
            selectable: false,
            ..Self::new(line)
        }
    }

    pub fn detail(mut self, detail: Span<'static>) -> Self {
        self.detail = Some(detail);
        self
    }

    pub fn right(mut self, word: Span<'static>) -> Self {
        self.right = Some(word);
        self
    }
}

/// A way the cursor moves in a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Move {
    Down,
    Up,
    First,
    Last,
    /// By this many rows (a page).
    PageDown(usize),
    PageUp(usize),
}

impl Move {
    /// The move a key asks for: `↓ j`, `↑ k`, `Home g`, `End G`, and
    /// `PgDn`/`PgUp` by `page` rows (`Scroll::height`). `None` for any
    /// other key.
    pub fn from_key(key: &KeyEvent, page: u16) -> Option<Move> {
        let page = usize::from(page.saturating_sub(1).max(1));
        Some(match key.code {
            KeyCode::Down | KeyCode::Char('j') => Move::Down,
            KeyCode::Up | KeyCode::Char('k') => Move::Up,
            KeyCode::Home | KeyCode::Char('g') => Move::First,
            KeyCode::End | KeyCode::Char('G') => Move::Last,
            KeyCode::PageDown => Move::PageDown(page),
            KeyCode::PageUp => Move::PageUp(page),
            _ => return None,
        })
    }
}

/// The first row at or after `from` (going down, or up) that is
/// selectable, below `n`.
fn seek(n: usize, from: usize, down: bool, selectable: &impl Fn(usize) -> bool) -> Option<usize> {
    if down {
        (from..n).find(|&i| selectable(i))
    } else {
        (0..=from.min(n.saturating_sub(1))).rev().find(|&i| selectable(i))
    }
}

/// Where the cursor goes from `from` in a list of `n` rows for `mv`,
/// skipping rows that are not `selectable`. It stays at `from` when there
/// is nowhere to go that way, and a page that ends on a heading goes on to
/// the next row that can be selected (or back to the last one before it).
pub fn select(n: usize, from: usize, mv: Move, selectable: impl Fn(usize) -> bool) -> usize {
    if n == 0 {
        return 0;
    }
    let last = n - 1;
    let from = from.min(last);
    let (target, down) = match mv {
        Move::Down => (from + 1, true),
        Move::Up => (from.wrapping_sub(1), false),
        Move::First => (0, true),
        Move::Last => (last, false),
        Move::PageDown(k) => ((from + k).min(last), true),
        Move::PageUp(k) => (from.saturating_sub(k), false),
    };
    if target > last {
        // Up from the first row wraps to a huge number: nowhere to go.
        return from;
    }
    let found = seek(n, target, down, &selectable).or_else(|| match mv {
        // A page that ran out: the last row the other way that moves on.
        Move::PageDown(_) => seek(n, last, false, &selectable).filter(|&i| i > from),
        Move::PageUp(_) => seek(n, 0, true, &selectable).filter(|&i| i < from),
        _ => None,
    });
    found.unwrap_or(from)
}

/// `cursor` made one a list can rest on: itself when selectable, else the
/// next selectable row, else the nearest one before it.
pub fn settle(n: usize, cursor: usize, selectable: impl Fn(usize) -> bool) -> usize {
    if n == 0 {
        return 0;
    }
    let cursor = cursor.min(n - 1);
    if selectable(cursor) {
        return cursor;
    }
    seek(n, cursor, true, &selectable)
        .or_else(|| seek(n, cursor, false, &selectable))
        .unwrap_or(cursor)
}

/// Selectable rows: the cursor row is highlighted across the row and kept
/// in view; a pane without focus keeps its cursor visible. Each row's left
/// part is cut to leave room for its word.
pub struct RowList {
    rows: Vec<ListRow>,
    cursor: usize,
    focused: bool,
}

impl RowList {
    pub fn new(rows: Vec<ListRow>) -> Self {
        Self {
            rows,
            cursor: 0,
            focused: true,
        }
    }

    pub fn cursor(mut self, row: usize) -> Self {
        self.cursor = row;
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Draw into `area` (inside the panel `frame`, whose edge takes the
    /// scrollbar). Returns each shown selectable row's area and index, for
    /// clicks: a heading has none.
    pub fn render(
        self,
        theme: &Theme,
        frame: Rect,
        area: Rect,
        buf: &mut Buffer,
        scroll: &Scroll,
    ) -> Vec<(Rect, usize)> {
        list(theme, frame, area, buf, self, scroll)
    }

    /// Draw into `inner`, the area inside a panel somebody else drew (the
    /// host's, for a settings section): the scrollbar runs down the panel's
    /// edge around it. Returns what [`RowList::render`] does.
    pub fn render_in(
        self,
        theme: &Theme,
        inner: Rect,
        buf: &mut Buffer,
        scroll: &Scroll,
    ) -> Vec<(Rect, usize)> {
        list(theme, super::scroll::around(inner), inner, buf, self, scroll)
    }
}

fn list(
    theme: &Theme,
    frame: Rect,
    area: Rect,
    buf: &mut Buffer,
    RowList {
        rows,
        cursor,
        focused,
    }: RowList,
    scroll: &Scroll,
) -> Vec<(Rect, usize)> {
    let n = rows.len();
    let h = area.height.max(1) as usize;
    if n == 0 {
        scroll.fit(area.height, 0);
        return vec![];
    }
    let can: Vec<bool> = rows.iter().map(|r| r.selectable).collect();
    let cursor = settle(n, cursor, |i| can[i]);
    let mut first = scroll.at() as usize;
    if cursor < first {
        first = cursor;
    }
    if cursor >= first + h {
        first = cursor + 1 - h;
    }
    first = first.min(n.saturating_sub(h));
    scroll.set(first as u16);
    scroll.fit(h as u16, n as u16);
    let width = area.width as usize;
    let items: Vec<ListItem> = rows
        .into_iter()
        .map(|r| {
            let rw = r.right.as_ref().map_or(0, Span::width);
            let room = width.saturating_sub(if rw > 0 { rw + 3 } else { 2 });
            let mut line = r.line;
            if let Some(d) = r.detail {
                let air = if rw > 0 { 2 } else { 0 };
                if line.width() + d.width() + air <= room {
                    line.spans.push(d);
                }
            }
            let mut spans = vec![Span::raw(" ")];
            let left = super::clip(line.spans, room);
            let used: usize = left.iter().map(Span::width).sum();
            spans.extend(left);
            if let Some(word) = r.right {
                spans.push(Span::raw(
                    " ".repeat(width.saturating_sub(1 + used + rw + 1)),
                ));
                spans.push(word);
            }
            ListItem::new(Line::from(spans)).style(line.style)
        })
        .collect();
    let list = List::new(items)
        .style(theme.s(Slot::TextPrimary))
        .highlight_style(widgets::row_style(theme, focused, true));
    let mut state = ListState::default()
        .with_offset(first)
        .with_selected(Some(cursor));
    StatefulWidget::render(list, area, buf, &mut state);
    scrollbar(theme, frame, buf, n.saturating_sub(h), first);
    (first..n.min(first + h))
        .filter(|&i| can[i])
        .map(|i| {
            let y = area.y + (i - first) as u16;
            (Rect { y, height: 1, ..area }, i)
        })
        .collect()
}
