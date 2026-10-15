//! Entries: a list whose rows carry lines under them (a run's failures with
//! the lines that say why, a thread's replies), and lines before a row that
//! are never chosen (a heading over a group). The cursor's row is
//! highlighted; it and as much of what is under it as fits are kept in view.
//! Lines are drawn as given, cut at the width: wrap them before.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use super::scroll::{Scroll, scrollbar};
use crate::theme::Theme;
use crate::widgets;

/// One row that can be chosen, with what is drawn before and under it.
#[derive(Debug, Clone, Default)]
pub struct Entry {
    pub head: Line<'static>,
    /// Under the head, part of the entry (a click on them chooses it).
    pub body: Vec<Line<'static>>,
    /// Before the head, not part of it (a heading, a gap).
    pub before: Vec<Line<'static>>,
}

impl Entry {
    pub fn new(head: impl Into<Line<'static>>) -> Self {
        Self {
            head: head.into(),
            ..Self::default()
        }
    }

    pub fn body(mut self, lines: Vec<Line<'static>>) -> Self {
        self.body = lines;
        self
    }

    pub fn before(mut self, lines: Vec<Line<'static>>) -> Self {
        self.before = lines;
        self
    }

    /// The lines it takes, before and body included.
    pub fn height(&self) -> usize {
        self.before.len() + 1 + self.body.len()
    }
}

/// Entries to draw, with a cursor.
pub struct Entries {
    entries: Vec<Entry>,
    cursor: Option<usize>,
    focused: bool,
    empty: String,
}

impl Entries {
    pub fn new(entries: Vec<Entry>) -> Self {
        Self {
            entries,
            cursor: None,
            focused: true,
            empty: "Nothing here.".into(),
        }
    }

    /// The entry with the cursor; it is kept in view.
    pub fn cursor(mut self, entry: usize) -> Self {
        self.cursor = Some(entry);
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// What it says when there are none.
    pub fn empty(mut self, text: impl Into<String>) -> Self {
        self.empty = text.into();
        self
    }

    /// Every line, before and body included.
    pub fn height(&self) -> usize {
        self.entries.iter().map(Entry::height).sum()
    }

    /// Draw into `area` (inside a panel `frame`, whose edge takes the
    /// scrollbar). The scroll keeps the first line shown. Returns the area
    /// of each shown line of an entry (head and body) with its index, for
    /// clicks.
    pub fn render(
        self,
        theme: &Theme,
        frame: Rect,
        area: Rect,
        buf: &mut Buffer,
        scroll: &Scroll,
    ) -> Vec<(Rect, usize)> {
        if self.entries.is_empty() {
            widgets::empty(buf, area, theme, &self.empty);
            scroll.fit(area.height, 0);
            return vec![];
        }
        // Every line, with the entry it belongs to and whether it is a head.
        let mut lines: Vec<(Line<'static>, Option<usize>, bool)> = Vec::new();
        let mut spans: Vec<(usize, usize)> = Vec::new();
        for (i, e) in self.entries.into_iter().enumerate() {
            lines.extend(e.before.into_iter().map(|l| (l, None, false)));
            let start = lines.len();
            lines.push((e.head, Some(i), true));
            lines.extend(e.body.into_iter().map(|l| (l, Some(i), false)));
            spans.push((start, lines.len()));
        }
        let n = lines.len();
        let h = area.height.max(1) as usize;
        let mut first = scroll.at() as usize;
        let cursor = self.cursor.map(|c| c.min(spans.len() - 1));
        if let Some(c) = cursor {
            let (start, end) = spans[c];
            // Its body as far as it fits, but never its head out of view.
            if end > first + h {
                first = end - h;
            }
            if start < first {
                first = start;
            }
        }
        first = first.min(n.saturating_sub(h));
        scroll.set(first.min(u16::MAX as usize) as u16);
        scroll.fit(h as u16, n.min(u16::MAX as usize) as u16);
        let width = area.width as usize;
        let highlight = widgets::row_style(theme, self.focused, true);
        let mut hits = Vec::new();
        for (row, (line, entry, head)) in lines.into_iter().enumerate().skip(first).take(h) {
            let y = area.y + (row - first) as u16;
            let rect = Rect { y, height: 1, ..area };
            let mut shown = vec![Span::raw(" ")];
            shown.extend(super::clip(line.spans, width.saturating_sub(2)));
            buf.set_line(area.x, y, &Line::from(shown).style(line.style), area.width);
            if let Some(i) = entry {
                if head && cursor == Some(i) {
                    buf.set_style(rect, highlight);
                }
                hits.push((rect, i));
            }
        }
        scrollbar(theme, frame, buf, n.saturating_sub(h), first);
        hits
    }
}
