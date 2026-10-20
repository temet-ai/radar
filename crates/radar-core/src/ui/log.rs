//! A log: the lines of a long text (a build's output, a program's) with a
//! cursor, sections that fold under their titles, and a search whose
//! matches stand out. Colours come from the text's own escape codes, mapped
//! to the theme's slots ([`LogLine::ansi`]). Only the rows in view are
//! drawn, each wrapped to the width.
//!
//! Keys ([`LogState::key`]): `↑↓ j k` move the cursor, `PgUp PgDn` a page,
//! `g G` (Home End) the start and the end, `space` folds the section the
//! cursor is in, `n N` go to the next and previous match.

use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;

use ratatui::buffer::Buffer;
use crate::view::{Chord, Key};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use super::scroll::scrollbar;
use crate::design::theme::Slot;
use crate::theme::Theme;
use crate::widgets;

/// What a piece of a line is drawn in: a slot, chosen from the colour the
/// text asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ink {
    #[default]
    Plain,
    Muted,
    Error,
    Good,
    Warn,
    Info,
    Accent,
    Other,
}

impl Ink {
    pub const fn slot(self) -> Slot {
        match self {
            Self::Plain => Slot::TextPrimary,
            Self::Muted => Slot::TextMuted,
            Self::Error => Slot::StatusError,
            Self::Good => Slot::StatusSuccess,
            Self::Warn => Slot::StatusWarning,
            Self::Info => Slot::StatusInfo,
            Self::Accent => Slot::AccentSecondary,
            Self::Other => Slot::AccentTertiary,
        }
    }
}

/// A run of text in one ink.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Piece {
    pub text: String,
    pub ink: Ink,
    pub bold: bool,
}

/// One line of the log, as pieces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogLine {
    pub pieces: Vec<Piece>,
}

impl LogLine {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            pieces: vec![Piece {
                text: text.into(),
                ..Piece::default()
            }],
        }
    }

    /// The text without its inks.
    pub fn text(&self) -> String {
        self.pieces.iter().map(|p| p.text.as_str()).collect()
    }

    fn width(&self) -> usize {
        self.pieces.iter().map(|p| super::width(&p.text)).sum()
    }

    /// A line of a terminal's output: SGR colours become inks (red is
    /// `Error`, green `Good`, and so on; bold is kept), every other escape
    /// sequence and control character is dropped, a tab is four spaces.
    pub fn ansi(raw: &str) -> Self {
        let mut pieces = Vec::new();
        let mut cur = Piece::default();
        let mut chars = raw.chars().peekable();
        let flush = |cur: &mut Piece, pieces: &mut Vec<Piece>| {
            if !cur.text.is_empty() {
                pieces.push(Piece {
                    text: std::mem::take(&mut cur.text),
                    ..cur.clone()
                });
            }
        };
        while let Some(c) = chars.next() {
            match c {
                '\u{1b}' => match chars.next() {
                    Some('[') => {
                        let mut params = String::new();
                        let mut last = None;
                        for d in chars.by_ref() {
                            if ('@'..='~').contains(&d) {
                                last = Some(d);
                                break;
                            }
                            params.push(d);
                        }
                        if last == Some('m') {
                            flush(&mut cur, &mut pieces);
                            sgr(&mut cur, &params);
                        }
                    }
                    // An operating system command runs to BEL or ESC \.
                    Some(']') => {
                        while let Some(d) = chars.next() {
                            if d == '\u{7}' {
                                break;
                            }
                            if d == '\u{1b}' {
                                if chars.peek() == Some(&'\\') {
                                    chars.next();
                                }
                                break;
                            }
                        }
                    }
                    _ => {}
                },
                '\t' => cur.text.push_str("    "),
                c if c.is_control() => {}
                c => cur.text.push(c),
            }
        }
        flush(&mut cur, &mut pieces);
        Self { pieces }
    }
}

/// Select Graphic Rendition: the codes between `ESC [` and `m`.
fn sgr(cur: &mut Piece, params: &str) {
    let mut codes = params.split(';').map(|c| c.parse::<u16>().unwrap_or(0));
    while let Some(code) = codes.next() {
        match code {
            0 => {
                cur.ink = Ink::Plain;
                cur.bold = false;
            }
            1 => cur.bold = true,
            2 => cur.ink = Ink::Muted,
            22 => cur.bold = false,
            30 | 90 => cur.ink = Ink::Muted,
            31 | 91 => cur.ink = Ink::Error,
            32 | 92 => cur.ink = Ink::Good,
            33 | 93 => cur.ink = Ink::Warn,
            34 | 94 => cur.ink = Ink::Info,
            35 | 95 => cur.ink = Ink::Other,
            36 | 96 => cur.ink = Ink::Accent,
            37 | 97 | 39 => cur.ink = Ink::Plain,
            // 256 colours and true colour: their numbers follow.
            38 | 48 => {
                let skip = match codes.next() {
                    Some(5) => 1,
                    Some(2) => 3,
                    _ => 0,
                };
                for _ in 0..skip {
                    codes.next();
                }
            }
            _ => {}
        }
    }
}

/// A part of the log under a title, folded or open. `start..end` are the
/// lines in it. Sections nest; give them ordered by `start`, an outer one
/// before the ones inside it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Section {
    pub title: String,
    /// After the title, muted: how long it took, say.
    pub detail: String,
    pub start: usize,
    pub end: usize,
    /// How it starts.
    pub folded: bool,
}

/// What a log shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LogText {
    pub lines: Vec<LogLine>,
    pub sections: Vec<Section>,
}

/// A row of the log as shown: a section's title or a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Title(usize),
    Line(usize),
}

/// Where a log is: the cursor, the first row shown, which sections were
/// folded or opened by hand, and the search. Kept in cells so it can be
/// moved while drawing through `&self`.
#[derive(Debug, Default)]
pub struct LogState {
    /// `None`: at the end.
    cursor: Cell<Option<Row>>,
    top: Cell<usize>,
    /// Rows shown in the last frame.
    shown: Cell<usize>,
    /// Sections whose fold differs from how they start, by start and title.
    toggled: RefCell<BTreeSet<(usize, String)>>,
    query: RefCell<String>,
    /// The lines that match the query, in order.
    matches: RefCell<Vec<usize>>,
}

impl LogState {
    pub fn new() -> Self {
        Self::default()
    }

    fn is_folded(&self, text: &LogText, s: usize) -> bool {
        let sec = &text.sections[s];
        sec.folded
            != self
                .toggled
                .borrow()
                .contains(&(sec.start, sec.title.clone()))
    }

    fn toggle(&self, text: &LogText, s: usize) {
        let sec = &text.sections[s];
        let key = (sec.start, sec.title.clone());
        let mut t = self.toggled.borrow_mut();
        if !t.remove(&key) {
            t.insert(key);
        }
    }

    /// The rows shown, each with how many sections it is in.
    pub fn rows(&self, text: &LogText) -> Vec<(Row, usize)> {
        let secs = &text.sections;
        let n = text.lines.len();
        let mut out = Vec::with_capacity(n + secs.len());
        let mut open: Vec<usize> = Vec::new();
        let (mut s, mut i) = (0, 0);
        loop {
            open.retain(|&o| secs[o].end > i);
            while s < secs.len() && secs[s].start <= i {
                if secs[s].start < i {
                    s += 1;
                    continue;
                }
                open.retain(|&o| secs[o].end > i);
                out.push((Row::Title(s), open.len()));
                let end = secs[s].end;
                if self.is_folded(text, s) {
                    s += 1;
                    while s < secs.len() && secs[s].start < end {
                        s += 1;
                    }
                    i = i.max(end);
                    open.retain(|&o| secs[o].end > i);
                } else {
                    open.push(s);
                    s += 1;
                }
            }
            if i >= n {
                break;
            }
            out.push((Row::Line(i), open.len()));
            i += 1;
        }
        out
    }

    /// Where `row` sorts: rows are in this order.
    fn order(text: &LogText, row: Row) -> (usize, u8, usize) {
        match row {
            Row::Title(s) => (text.sections[s].start, 0, s),
            Row::Line(i) => (i, 1, 0),
        }
    }

    /// The cursor's place among `rows`.
    fn at(&self, text: &LogText, rows: &[(Row, usize)]) -> usize {
        let last = rows.len().saturating_sub(1);
        match self.cursor.get() {
            None => last,
            Some(c) => {
                let key = Self::order(text, c);
                rows.partition_point(|(r, _)| Self::order(text, *r) < key)
                    .min(last)
            }
        }
    }

    fn set_at(&self, rows: &[(Row, usize)], at: usize) {
        if let Some((r, _)) = rows.get(at) {
            self.cursor.set(Some(*r));
        }
    }

    /// The line the cursor is on (a title: the section's first line).
    pub fn line(&self, text: &LogText) -> Option<usize> {
        let rows = self.rows(text);
        match rows.get(self.at(text, &rows))?.0 {
            Row::Line(i) => Some(i),
            Row::Title(s) => Some(text.sections[s].start),
        }
    }

    /// The cursor is on the first row.
    pub fn at_top(&self, text: &LogText) -> bool {
        let rows = self.rows(text);
        !rows.is_empty() && self.at(text, &rows) == 0
    }

    /// Put the cursor on `line`, opening the sections around it, a few rows
    /// below the top.
    pub fn goto(&self, text: &LogText, line: usize) {
        if line >= text.lines.len() {
            return;
        }
        self.open_around(text, line);
        self.cursor.set(Some(Row::Line(line)));
        let rows = self.rows(text);
        self.top.set(self.at(text, &rows).saturating_sub(3));
    }

    fn open_around(&self, text: &LogText, line: usize) {
        for (s, sec) in text.sections.iter().enumerate() {
            if sec.start <= line && line < sec.end && self.is_folded(text, s) {
                self.toggle(text, s);
            }
        }
    }

    /// Lines were put before the ones held (an earlier part arrived): keep
    /// the cursor, the view and the folds on the same lines.
    pub fn shift(&self, by: usize) {
        if let Some(Row::Line(i)) = self.cursor.get() {
            self.cursor.set(Some(Row::Line(i + by)));
        } else if self.cursor.get().is_some() {
            self.cursor.set(Some(Row::Line(by)));
        }
        self.top.set(self.top.get() + by);
        let moved = self
            .toggled
            .borrow()
            .iter()
            .map(|(start, title)| (start + by, title.clone()))
            .collect();
        *self.toggled.borrow_mut() = moved;
        self.matches.borrow_mut().iter_mut().for_each(|m| *m += by);
    }

    /// Search for `query` (letters in either case) and go to the first
    /// match from the cursor on. How many lines match.
    pub fn search(&self, text: &LogText, query: &str) -> usize {
        *self.query.borrow_mut() = query.to_string();
        self.matches.replace(find(text, query));
        if !self.matches.borrow().is_empty() {
            let from = self.line(text).unwrap_or(0);
            self.jump(text, from.checked_sub(1), true);
        }
        self.matches.borrow().len()
    }

    /// Look for the query again in a text that changed.
    pub fn research(&self, text: &LogText) {
        let q = self.query.borrow().clone();
        self.matches.replace(find(text, &q));
    }

    pub fn clear_search(&self) {
        self.query.borrow_mut().clear();
        self.matches.borrow_mut().clear();
    }

    pub fn query(&self) -> String {
        self.query.borrow().clone()
    }

    /// The match the cursor is on (from 1, 0 when on none) and how many there are.
    pub fn found(&self, text: &LogText) -> (usize, usize) {
        let m = self.matches.borrow();
        let on = self
            .line(text)
            .and_then(|l| m.iter().position(|&x| x == l))
            .map_or(0, |i| i + 1);
        (on, m.len())
    }

    /// To the next match after `from` (or the previous before it),
    /// around the end.
    fn jump(&self, text: &LogText, from: Option<usize>, next: bool) {
        let m = self.matches.borrow().clone();
        let to = if next {
            m.iter()
                .find(|&&x| from.is_none_or(|f| x > f))
                .or(m.first())
        } else {
            m.iter()
                .rev()
                .find(|&&x| from.is_some_and(|f| x < f))
                .or(m.last())
        };
        if let Some(&to) = to {
            self.goto(text, to);
        }
    }

    /// Fold or open the section at the cursor (on a line: the innermost
    /// one around it; folding puts the cursor on its title).
    pub fn fold(&self, text: &LogText) -> bool {
        self.fold_to(text, None)
    }

    /// `+` and `-`: open (`Some(true)`) or fold (`Some(false)`) the section
    /// the cursor is in, the cursor to its title; `None` toggles. False when
    /// the cursor is in no section or it is already so.
    pub fn fold_to(&self, text: &LogText, open: Option<bool>) -> bool {
        let rows = self.rows(text);
        let s = match rows.get(self.at(text, &rows)).map(|r| r.0) {
            Some(Row::Title(s)) => s,
            Some(Row::Line(i)) => match text
                .sections
                .iter()
                .rposition(|sec| sec.start <= i && i < sec.end)
            {
                Some(s) => s,
                None => return false,
            },
            None => return false,
        };
        if open.is_some_and(|open| open != self.is_folded(text, s)) {
            return false;
        }
        self.toggle(text, s);
        self.cursor.set(Some(Row::Title(s)));
        true
    }

    /// A key that moves through the log; false for any other.
    pub fn key(&self, text: &LogText, key: Chord) -> bool {
        let rows = self.rows(text);
        if rows.is_empty() {
            return false;
        }
        let at = self.at(text, &rows);
        let last = rows.len() - 1;
        let page = self.shown.get().saturating_sub(1).max(1);
        let to = match key.key {
            Key::Down | Key::Char('j') => (at + 1).min(last),
            Key::Up | Key::Char('k') => at.saturating_sub(1),
            Key::PageDown => (at + page).min(last),
            Key::PageUp => at.saturating_sub(page),
            Key::Home | Key::Char('g') => 0,
            Key::End | Key::Char('G') => {
                self.cursor.set(None);
                return true;
            }
            Key::Char(' ') => return self.fold(text),
            Key::Char('+' | '=') => return self.fold_to(text, Some(true)),
            Key::Char('-') => return self.fold_to(text, Some(false)),
            Key::Char(c @ ('n' | 'N')) => {
                if self.matches.borrow().is_empty() {
                    return false;
                }
                let from = self.line(text);
                self.jump(text, from, c == 'n');
                return true;
            }
            _ => return false,
        };
        self.set_at(&rows, to);
        true
    }

    /// A click on the `n`th row shown (as [`LogView::render`] returned it).
    pub fn click(&self, text: &LogText, row: usize) {
        let rows = self.rows(text);
        self.set_at(&rows, row);
    }

    /// Project the visible log into renderer-neutral, wrapped lines. This
    /// updates the same viewport used by PageUp/PageDown and later reads.
    pub fn paint(
        &self,
        text: &LogText,
        width: u16,
        height: u16,
    ) -> (Vec<crate::view::LogPaintLine>, usize, usize) {
        use crate::view::{Ink as ViewInk, LogPaintLine, Seg, Text};
        let rows = self.rows(text);
        if rows.is_empty() || height == 0 || width < 4 {
            self.shown.set(0);
            return (vec![], 0, rows.len());
        }
        let h = height as usize;
        let width = width as usize - 2;
        let at = self.at(text, &rows);
        let row_height = |(row, depth): &(Row, usize)| match row {
            Row::Title(_) => 1,
            Row::Line(i) => {
                let room = width.saturating_sub(2 * depth).max(1);
                text.lines[*i].width().div_ceil(room).max(1)
            }
        };
        let mut top = self.top.get().min(rows.len() - 1);
        if at < top {
            top = at;
        }
        while top < at && rows[top..=at].iter().map(row_height).sum::<usize>() > h {
            top += 1;
        }
        let below: usize = rows[top..].iter().map(row_height).sum();
        if below < h {
            let mut room = h - below;
            while top > 0 && row_height(&rows[top - 1]) <= room {
                top -= 1;
                room -= row_height(&rows[top]);
            }
        }
        self.top.set(top);
        let query = self.query.borrow().to_ascii_lowercase();
        let mut lines = Vec::new();
        let mut shown = 0;
        for (n, (row, depth)) in rows.iter().enumerate().skip(top) {
            if lines.len() >= h {
                break;
            }
            let mut bar = "│ ".repeat(*depth);
            let chunks: Vec<Text> = match row {
                Row::Title(s) => {
                    let sec = &text.sections[*s];
                    let folded = self.is_folded(text, *s);
                    bar.push_str(if folded { "─ " } else { "┌ " });
                    let mut line = Text::new()
                        .push(Seg::new(sec.title.clone(), ViewInk::AccentSecondary).strong());
                    if !sec.detail.is_empty() {
                        line.add(Seg::new(format!(" · {}", sec.detail), ViewInk::Muted));
                    }
                    if folded {
                        let count = sec.end - sec.start;
                        line.add(Seg::new(
                            format!(" ⋯ {count} line{}", if count == 1 { "" } else { "s" }),
                            ViewInk::Muted,
                        ));
                    }
                    vec![line]
                }
                Row::Line(i) => paint_wrap(
                    &text.lines[*i],
                    width.saturating_sub(2 * depth).max(1),
                    &query,
                ),
            };
            for (k, chunk) in chunks.into_iter().enumerate() {
                if lines.len() >= h {
                    break;
                }
                let lead = if k == 0 || matches!(row, Row::Title(_)) {
                    bar.clone()
                } else {
                    "│ ".repeat(*depth)
                };
                let mut line = Text::from(vec![
                    Seg::new(" ", ViewInk::Plain),
                    Seg::new(lead, ViewInk::AccentMuted),
                ]);
                line.extend(chunk);
                lines.push(LogPaintLine {
                    text: line,
                    row: n,
                    selected: n == at,
                });
            }
            shown += 1;
        }
        self.shown.set(shown);
        (lines, top, rows.len())
    }
}

fn paint_wrap(line: &LogLine, room: usize, query: &str) -> Vec<crate::view::Text> {
    use crate::view::{Ink as ViewInk, Seg, Text};
    let mut rows = vec![Text::new()];
    let mut used = 0;
    let lower = line.text().to_ascii_lowercase();
    let hits: Vec<(usize, usize)> = if query.is_empty() {
        vec![]
    } else {
        lower
            .match_indices(query)
            .map(|(at, matched)| (at, at + matched.len()))
            .collect()
    };
    let mut offset = 0;
    for piece in &line.pieces {
        let ink = match piece.ink {
            Ink::Plain => ViewInk::Primary,
            Ink::Muted => ViewInk::Muted,
            Ink::Error => ViewInk::Bad,
            Ink::Good => ViewInk::Good,
            Ink::Warn => ViewInk::Warn,
            Ink::Info => ViewInk::Info,
            Ink::Accent => ViewInk::AccentSecondary,
            Ink::Other => ViewInk::AccentTertiary,
        };
        let (start, end) = (offset, offset + piece.text.len());
        offset = end;
        let mut cuts = vec![start, end];
        for &(a, b) in &hits {
            cuts.extend([a, b].into_iter().filter(|&cut| cut > start && cut < end));
        }
        cuts.sort_unstable();
        cuts.dedup();
        for range in cuts.windows(2) {
            let marked = hits.iter().any(|&(a, b)| range[0] >= a && range[1] <= b);
            let mut current = String::new();
            for ch in piece.text[range[0] - start..range[1] - start].chars() {
                if used == room {
                    if !current.is_empty() {
                        let mut seg = Seg::new(std::mem::take(&mut current), ink);
                        if piece.bold {
                            seg = seg.strong();
                        }
                        if marked {
                            seg = seg.marked();
                        }
                        rows.last_mut().expect("a row").add(seg);
                    }
                    rows.push(Text::new());
                    used = 0;
                }
                current.push(ch);
                used += 1;
            }
            if !current.is_empty() {
                let mut seg = Seg::new(current, ink);
                if piece.bold {
                    seg = seg.strong();
                }
                if marked {
                    seg = seg.marked();
                }
                rows.last_mut().expect("a row").add(seg);
            }
        }
    }
    rows
}

fn find(text: &LogText, query: &str) -> Vec<usize> {
    if query.is_empty() {
        return vec![];
    }
    let q = query.to_ascii_lowercase();
    text.lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.text().to_ascii_lowercase().contains(&q))
        .map(|(i, _)| i)
        .collect()
}

/// A log to draw.
pub struct LogView<'a> {
    pub text: &'a LogText,
    pub state: &'a LogState,
    pub focused: bool,
    /// What it says when there are no lines.
    pub empty: &'a str,
}

impl LogView<'_> {
    /// Draw into `area` (inside a panel `frame`, whose edge takes the
    /// scrollbar). Returns each shown row's area and its place among the
    /// rows, for [`LogState::click`].
    pub fn render(
        &self,
        theme: &Theme,
        frame: Rect,
        area: Rect,
        buf: &mut Buffer,
    ) -> Vec<(Rect, usize)> {
        let (text, state) = (self.text, self.state);
        let rows = state.rows(text);
        if rows.is_empty() || area.height == 0 || area.width < 4 {
            widgets::empty(buf, area, theme, self.empty);
            return vec![];
        }
        let h = area.height as usize;
        let width = area.width as usize - 2;
        let at = state.at(text, &rows);
        let height = |(row, depth): &(Row, usize)| match row {
            Row::Title(_) => 1,
            Row::Line(i) => {
                let room = width.saturating_sub(2 * depth).max(1);
                text.lines[*i].width().div_ceil(room).max(1)
            }
        };
        // Keep the cursor's row in view, all of it when it fits.
        let mut top = state.top.get().min(rows.len() - 1);
        if at < top {
            top = at;
        }
        while top < at && rows[top..=at].iter().map(height).sum::<usize>() > h {
            top += 1;
        }
        // At the end, fill the panel rather than leave it half empty.
        let below: usize = rows[top..].iter().map(height).sum();
        if below < h {
            let mut room = h - below;
            while top > 0 && height(&rows[top - 1]) <= room {
                top -= 1;
                room -= height(&rows[top]);
            }
        }
        state.top.set(top);
        let query = state.query.borrow().to_ascii_lowercase();
        let gutter = theme.s(Slot::AccentMuted);
        let highlight = widgets::row_style(theme, self.focused, true);
        let mut y = area.y;
        let mut hits = Vec::new();
        let mut shown = 0;
        for (n, (row, depth)) in rows.iter().enumerate().skip(top) {
            if y >= area.y + area.height {
                break;
            }
            let mut bar = "│ ".repeat(*depth);
            let lines: Vec<Vec<Span<'static>>> = match row {
                Row::Title(s) => {
                    let sec = &text.sections[*s];
                    let folded = state.is_folded(text, *s);
                    bar.push_str(if folded { "─ " } else { "┌ " });
                    let mut spans = vec![Span::styled(
                        sec.title.clone(),
                        theme.bold(Slot::AccentSecondary),
                    )];
                    if !sec.detail.is_empty() {
                        spans.push(Span::styled(
                            format!(" · {}", sec.detail),
                            theme.s(Slot::TextMuted),
                        ));
                    }
                    if folded {
                        let n = sec.end - sec.start;
                        spans.push(Span::styled(
                            format!(" ⋯ {n} line{}", if n == 1 { "" } else { "s" }),
                            theme.s(Slot::TextMuted).add_modifier(Modifier::DIM),
                        ));
                    }
                    vec![super::clip(spans, width.saturating_sub(super::width(&bar)))]
                }
                Row::Line(i) => wrap(
                    styled(theme, &text.lines[*i], &query),
                    width.saturating_sub(2 * depth).max(1),
                ),
            };
            for (k, spans) in lines.into_iter().enumerate() {
                if y >= area.y + area.height {
                    break;
                }
                let lead = if k == 0 || matches!(row, Row::Title(_)) {
                    bar.clone()
                } else {
                    "│ ".repeat(*depth)
                };
                let mut all = vec![Span::raw(" "), Span::styled(lead, gutter)];
                all.extend(spans);
                buf.set_line(area.x, y, &Line::from(all), area.width);
                let rect = Rect {
                    y,
                    height: 1,
                    ..area
                };
                if n == at {
                    buf.set_style(rect, highlight);
                }
                hits.push((rect, n));
                y += 1;
            }
            shown += 1;
        }
        state.shown.set(shown);
        let max = rows.len().saturating_sub(shown);
        scrollbar(theme, frame, buf, max, top.min(max));
        hits
    }
}

/// How a match stands out: its own background, or reversed where there are
/// no colours.
pub(crate) fn found_style(theme: &Theme) -> Style {
    let bg = theme.bg(Slot::StatusWarning);
    if bg == Style::new() {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        bg.patch(theme.s(Slot::BgBase)).add_modifier(Modifier::BOLD)
    }
}

/// A line's pieces as spans, with `query`'s matches standing out.
fn styled(theme: &Theme, line: &LogLine, query: &str) -> Vec<Span<'static>> {
    let style = |p: &Piece| {
        if p.bold {
            theme.bold(p.ink.slot())
        } else {
            theme.s(p.ink.slot())
        }
    };
    if query.is_empty() {
        return line
            .pieces
            .iter()
            .map(|p| Span::styled(p.text.clone(), style(p)))
            .collect();
    }
    let lower = line.text().to_ascii_lowercase();
    let hits: Vec<(usize, usize)> = lower
        .match_indices(query)
        .map(|(at, m)| (at, at + m.len()))
        .collect();
    let found = found_style(theme);
    let mut out = Vec::new();
    let mut at = 0;
    for p in &line.pieces {
        let (start, end) = (at, at + p.text.len());
        at = end;
        // Cut the piece where matches start and end.
        let mut cuts = vec![start, end];
        for &(a, b) in &hits {
            cuts.extend([a, b].into_iter().filter(|&c| c > start && c < end));
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let piece = &p.text[w[0] - start..w[1] - start];
            let inside = hits.iter().any(|&(a, b)| w[0] >= a && w[1] <= b);
            let s = if inside {
                style(p).patch(found)
            } else {
                style(p)
            };
            out.push(Span::styled(piece.to_string(), s));
        }
    }
    out
}

/// Spans cut into rows of `w` cells.
fn wrap(spans: Vec<Span<'static>>, w: usize) -> Vec<Vec<Span<'static>>> {
    let mut rows = vec![Vec::new()];
    let mut used = 0;
    for s in spans {
        let mut text = String::new();
        for c in s.content.chars() {
            if used == w {
                rows.last_mut()
                    .expect("a row")
                    .push(Span::styled(std::mem::take(&mut text), s.style));
                rows.push(Vec::new());
                used = 0;
            }
            text.push(c);
            used += 1;
        }
        if !text.is_empty() {
            rows.last_mut()
                .expect("a row")
                .push(Span::styled(text, s.style));
        }
    }
    rows
}
