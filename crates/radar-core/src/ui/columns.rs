//! Named columns at the right of rows, lined up under their headings at
//! every depth, and dropped one by one as the width runs out. A tree's rows
//! ([`super::tree`]) and a table ([`super::table`]) share this model.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::icon::{self, Icon};

use crate::design::theme::Slot;
use crate::theme::Theme;

/// One column: its heading, its width in cells (a space after the value
/// included), and how long it is kept as room runs out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub heading: String,
    pub width: usize,
    /// Higher is kept longer; the lowest goes first, the later of equals first.
    pub keep: u8,
    /// Takes what is left (a table's name column); `width` is its least.
    pub flex: bool,
    /// An icon before the heading, when icons are on.
    pub icon: Option<Icon>,
}

impl Column {
    pub fn new(heading: impl Into<String>, width: usize) -> Self {
        Self {
            heading: heading.into(),
            width,
            keep: 0,
            flex: false,
            icon: None,
        }
    }

    /// An icon before the heading (the heading still names the column).
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The heading as drawn in `width` cells: its icon (when icons are on)
    /// and the heading in capitals, in `style`, padded.
    pub fn heading_spans(&self, theme: &Theme, width: usize, style: Style) -> Vec<Span<'static>> {
        let text = self.heading.to_uppercase();
        let lead = self.icon.map(|i| icon::lead(theme, i)).filter(|s| !s.content.is_empty());
        let Some(lead) = lead else {
            return vec![Span::styled(super::pad(&text, width), style)];
        };
        let mut spans = super::clip(vec![lead, Span::styled(text, style)], width.saturating_sub(1));
        let used: usize = spans.iter().map(Span::width).sum();
        spans.push(Span::styled(" ".repeat(width - used), style));
        spans
    }

    /// A column that takes the room the others leave, at least `min` cells.
    pub fn flex(heading: impl Into<String>, min: usize) -> Self {
        Self {
            flex: true,
            keep: u8::MAX,
            ..Self::new(heading, min)
        }
    }

    pub fn keep(mut self, keep: u8) -> Self {
        self.keep = keep;
        self
    }
}

/// Columns in the order they show.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Columns(pub Vec<Column>);

impl Columns {
    pub fn new(cols: impl IntoIterator<Item = Column>) -> Self {
        Self(cols.into_iter().collect())
    }

    /// Their total width.
    pub fn width(&self) -> usize {
        self.0.iter().map(|c| c.width).sum()
    }

    pub fn iter(&self) -> impl Iterator<Item = &Column> {
        self.0.iter()
    }

    /// The columns that fit in `width` beside a name of at least `name`
    /// cells: the lowest `keep` goes first, and the order stays. The last
    /// one left is kept whatever the width.
    pub fn fit(&self, width: usize, name: usize) -> Columns {
        let mut cols = self.0.clone();
        while cols.len() > 1 && cols.iter().map(|c| c.width).sum::<usize>() + name > width {
            let (drop, _) = cols
                .iter()
                .enumerate()
                .min_by_key(|(i, c)| (c.keep, std::cmp::Reverse(*i)))
                .expect("more than one column");
            cols.remove(drop);
        }
        Columns(cols)
    }

    /// `extra` columns (another part's, say) put before the last one, each
    /// while the names keep `name` cells in `width`: they never push out
    /// these columns.
    pub fn with_extra(&self, extra: &[Column], width: usize, name: usize) -> Columns {
        let mut cols = self.0.clone();
        let mut used = self.width() + name;
        let at = cols.len().saturating_sub(1);
        let mut add = Vec::new();
        for c in extra {
            if used + c.width <= width {
                add.push(c.clone());
                used += c.width;
            }
        }
        cols.splice(at..at, add);
        Columns(cols)
    }

    /// Where the column `heading` is in a row whose columns end at `end`
    /// (the x after the last one): its first x and its width without the
    /// cell of air after it. For marking a cell clickable.
    pub fn at(&self, heading: &str, end: u16) -> Option<(u16, u16)> {
        let mut x = end.checked_sub(self.width() as u16)?;
        for c in &self.0 {
            if c.heading == heading {
                return Some((x, c.width.saturating_sub(1) as u16));
            }
            x += c.width as u16;
        }
        None
    }

    /// One span per column, by heading, padded to the column; a missing
    /// cell is blank.
    pub fn cells(&self, cells: &[(&str, Span<'static>)]) -> Vec<Span<'static>> {
        self.0
            .iter()
            .map(|c| match cells.iter().find(|(k, _)| *k == c.heading) {
                Some((_, s)) => {
                    let (text, _) = super::take(&s.content, c.width.saturating_sub(1));
                    Span::styled(super::pad(text, c.width), s.style)
                }
                None => Span::raw(" ".repeat(c.width)),
            })
            .collect()
    }

    /// [`Columns::cells`] for cells of several spans (an icon in its own
    /// colour before the words): each cut to its column, its spans keeping
    /// their styles, and padded in the last span's style.
    pub fn cell_lines(&self, cells: &[(&str, Line<'static>)]) -> Vec<Span<'static>> {
        let mut out = Vec::new();
        for c in &self.0 {
            match cells.iter().find(|(k, _)| *k == c.heading) {
                Some((_, line)) => {
                    let spans: Vec<Span<'static>> = line
                        .spans
                        .iter()
                        .map(|s| Span::styled(s.content.clone(), line.style.patch(s.style)))
                        .collect();
                    let pad = spans.last().map_or(line.style, |s| s.style);
                    let mut spans = super::clip(spans, c.width.saturating_sub(1));
                    let used: usize = spans.iter().map(Span::width).sum();
                    match spans.last_mut() {
                        Some(last) if last.style == pad => {
                            last.content = super::pad(&last.content, last.width() + c.width - used).into();
                        }
                        _ => spans.push(Span::styled(" ".repeat(c.width - used), pad)),
                    }
                    out.extend(spans);
                }
                None => out.push(Span::raw(" ".repeat(c.width))),
            }
        }
        out
    }

    /// A row: `name` cut and padded to the room the columns leave in
    /// `room` cells, then the cells, so every row's columns end at the same
    /// edge.
    pub fn line(
        &self,
        name: Vec<Span<'static>>,
        room: usize,
        cells: &[(&str, Span<'static>)],
    ) -> Line<'static> {
        let cells: Vec<(&str, Line<'static>)> = cells.iter().map(|(h, s)| (*h, Line::from(s.clone()))).collect();
        self.line_of(name, room, &cells)
    }

    /// [`Columns::line`] with cells of several spans ([`Columns::cell_lines`]).
    pub fn line_of(
        &self,
        name: Vec<Span<'static>>,
        room: usize,
        cells: &[(&str, Line<'static>)],
    ) -> Line<'static> {
        let name_w = room.saturating_sub(self.width());
        let mut used = 0;
        let mut spans = Vec::new();
        for s in name {
            let left = name_w.saturating_sub(used + 1);
            let (text, n) = super::take(&s.content, left);
            used += n;
            spans.push(Span::styled(text.to_string(), s.style));
        }
        spans.push(Span::raw(" ".repeat(name_w.saturating_sub(used))));
        spans.extend(self.cell_lines(cells));
        Line::from(spans)
    }

    /// The heading row in `width` cells: `left` over the names (what the
    /// rows are, who they are for), then each heading over its column.
    pub fn header(&self, theme: &Theme, left: Vec<Span<'static>>, width: usize) -> Line<'static> {
        let room = width.saturating_sub(self.width());
        let mut spans = super::clip(left, room.saturating_sub(1));
        let used: usize = spans.iter().map(Span::width).sum();
        spans.push(Span::raw(" ".repeat(room.saturating_sub(used))));
        let style = theme.s(Slot::TextMuted).add_modifier(Modifier::BOLD);
        for c in &self.0 {
            spans.extend(c.heading_spans(theme, c.width, style));
        }
        Line::from(spans)
    }
}
