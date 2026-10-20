//! A table: rows of cells under their headings, drawn by ratatui's `Table`.
//! Columns come from [`Columns`] and drop by width with [`Columns::fit`];
//! one [`Column::flex`](super::Column::flex) column takes what is left. The
//! cursor row is highlighted across the panel and kept in view; a scrollbar
//! shows on the panel's edge when there are more rows than fit.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Row, StatefulWidget, Table as RTable, TableState};

use super::columns::Columns;
use super::scroll::{Scroll, scrollbar};
use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::widgets;

/// A table to draw: columns, rows (cells by heading), an optional cursor.
/// A value too long for its column is cut with `…`.
pub struct Table<'a> {
    columns: &'a Columns,
    rows: Vec<Vec<(String, Line<'static>)>>,
    cursor: Option<usize>,
    focused: bool,
    empty: &'a str,
}

impl<'a> Table<'a> {
    pub fn new(columns: &'a Columns, rows: Vec<Vec<(String, Span<'static>)>>) -> Self {
        Self::lines(
            columns,
            rows.into_iter()
                .map(|cells| cells.into_iter().map(|(h, s)| (h, Line::from(s))).collect())
                .collect(),
        )
    }

    /// Cells of several spans (a title and its chips).
    pub fn lines(columns: &'a Columns, rows: Vec<Vec<(String, Line<'static>)>>) -> Self {
        Self {
            columns,
            rows,
            cursor: None,
            focused: true,
            empty: "Nothing here.",
        }
    }

    /// The width the flex column gets in a panel `width` wide (inside its
    /// border): what is left of its padding, the other columns and the
    /// spaces between them.
    pub fn flex_width(columns: &Columns, width: u16) -> usize {
        let fixed: usize = columns.iter().filter(|c| !c.flex).map(|c| c.width).sum();
        (width as usize).saturating_sub(2 + fixed)
    }

    /// The row with the cursor; it is kept in view.
    pub fn cursor(mut self, row: usize) -> Self {
        self.cursor = Some(row);
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// What it says when there are no rows.
    pub fn empty(mut self, text: &'a str) -> Self {
        self.empty = text;
        self
    }

    /// Draw into `area` (inside a panel `frame`, whose edge takes the
    /// scrollbar). The scroll keeps the first row shown. Returns each shown
    /// row's area and index, for clicks.
    pub fn render(
        self,
        theme: &Theme,
        frame: Rect,
        area: Rect,
        buf: &mut Buffer,
        scroll: &Scroll,
    ) -> Vec<(Rect, usize)> {
        let inner = widgets::padded(area);
        let flex = Self::flex_width(self.columns, area.width);
        let header = Row::new(
            self.columns
                .iter()
                .map(|c| {
                    let w = if c.flex { flex } else { c.width.saturating_sub(1) };
                    let spans = c.heading_spans(theme, w + 1, theme.bold(Slot::TextMuted));
                    Cell::from(Line::from(spans))
                })
                .collect::<Vec<_>>(),
        )
        .style(theme.bold(Slot::TextMuted));
        if self.rows.is_empty() {
            widgets::empty(buf, area, theme, self.empty);
            scroll.fit(area.height, 0);
            return vec![];
        }
        let widths: Vec<Constraint> = self
            .columns
            .iter()
            .map(|c| {
                if c.flex {
                    Constraint::Min(c.width.saturating_sub(1) as u16)
                } else {
                    Constraint::Length(c.width.saturating_sub(1) as u16)
                }
            })
            .collect();
        let n = self.rows.len();
        let h = area.height.saturating_sub(1).max(1) as usize;
        let first = match self.cursor {
            Some(row) => {
                let row = row.min(n - 1);
                let mut first = scroll.at() as usize;
                if row < first {
                    first = row;
                }
                if row >= first + h {
                    first = row + 1 - h;
                }
                first.min(n.saturating_sub(h))
            }
            None => (scroll.at() as usize).min(n.saturating_sub(h)),
        };
        scroll.set(first as u16);
        scroll.fit(h as u16, n as u16);
        let style = theme.s(Slot::TextPrimary);
        let rows: Vec<Row> = self
            .rows
            .iter()
            .map(|cells| {
                Row::new(
                    self.columns
                        .iter()
                        .map(|c| match cells.iter().find(|(k, _)| *k == c.heading) {
                            Some((_, line)) => {
                                let w = if c.flex { flex } else { c.width.saturating_sub(1) };
                                Cell::from(cut(line.clone(), w))
                            }
                            None => Cell::from(""),
                        })
                        .collect::<Vec<_>>(),
                )
                .style(style)
            })
            .collect();
        let highlight = widgets::row_style(theme, self.focused, true);
        // The highlight covers the panel's padding too.
        if let Some(row) = self.cursor.map(|r| r.min(n - 1))
            && row >= first
            && row < first + h
        {
            let y = area.y + 1 + (row - first) as u16;
            widgets::fill_row(buf, Rect { y, height: 1, ..area }, highlight);
        }
        let table = RTable::new(rows, widths)
            .header(header)
            .column_spacing(1)
            .row_highlight_style(highlight);
        let mut state = TableState::new()
            .with_offset(first)
            .with_selected(self.cursor.map(|r| r.min(n - 1)));
        StatefulWidget::render(table, inner, buf, &mut state);
        scrollbar(theme, frame, buf, n.saturating_sub(h), first);
        (first..n.min(first + h))
            .map(|i| {
                let y = area.y + 1 + (i - first) as u16;
                (Rect { y, height: 1, ..area }, i)
            })
            .collect()
    }
}

/// `line` cut to `w` cells, the last of them `…` when it did not fit.
fn cut(line: Line<'static>, w: usize) -> Line<'static> {
    if line.width() <= w {
        return line;
    }
    let style = line.spans.last().map(|s| s.style).unwrap_or_default();
    let mut spans = super::clip(line.spans, w.saturating_sub(1));
    if w > 0 {
        spans.push(Span::styled("…", style));
    }
    Line::from(spans)
}
