//! A diff: one file's changes as unified lines (hunk headings, lines kept,
//! added and removed) with the old and new line numbers beside them, a
//! cursor on a line, and blocks of lines under any line (what was said
//! about it, a comment being written). Only the rows in view are drawn, so
//! a long file costs what fits on the screen.
//!
//! The cursor row is marked `▎` at its left edge as well as highlighted, so
//! it reads without colour. Added and removed lines take the theme's
//! `diff.added` and `diff.removed`; a hunk's heading `status.info`.

use std::cell::{Cell, RefCell};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use super::scroll::scrollbar;
use crate::design::theme::Slot;
use crate::widgets::blend;
use crate::theme::Theme;
use crate::widgets;
use serde::{Deserialize, Serialize};

/// What a line of a diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[ts(rename = "DiffKind")]
pub enum Kind {
    /// `@@ -12,7 +12,9 @@ …`: where a hunk starts.
    Hunk,
    /// The same on both sides.
    Context,
    Added,
    Removed,
    /// `\ No newline at end of file` and the like.
    Note,
}

/// One line of a diff, with its numbers on each side.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
pub struct DiffLine {
    pub kind: Kind,
    /// Its number in the old file (kept and removed lines).
    pub old: Option<u32>,
    /// Its number in the new file (kept and added lines).
    pub new: Option<u32>,
    /// The text without its `+`, `-` or space; a hunk's whole heading.
    pub text: String,
}

/// The numbers in `@@ -12,7 +12,9 @@`: where each side starts.
fn hunk_starts(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("@@ ")?;
    let (ranges, _) = rest.split_once(" @@")?;
    let mut parts = ranges.split_whitespace();
    let start = |p: &str, sign: char| -> Option<u32> {
        p.strip_prefix(sign)?.split(',').next()?.parse().ok()
    };
    Some((start(parts.next()?, '-')?, start(parts.next()?, '+')?))
}

/// A unified diff's text as lines with their numbers. File headings
/// (`diff --git`, `---`, `+++`, `index`) are left out; text before the first
/// hunk is ignored.
pub fn parse(diff: &str) -> Vec<DiffLine> {
    let mut out = Vec::new();
    let (mut old, mut new) = (0u32, 0u32);
    let mut in_hunk = false;
    for raw in diff.lines() {
        if let Some((o, n)) = hunk_starts(raw) {
            (old, new, in_hunk) = (o, n, true);
            out.push(DiffLine {
                kind: Kind::Hunk,
                old: None,
                new: None,
                text: raw.to_string(),
            });
            continue;
        }
        if !in_hunk {
            continue;
        }
        let (kind, text) = match raw.chars().next() {
            Some('+') => (Kind::Added, &raw[1..]),
            Some('-') => (Kind::Removed, &raw[1..]),
            Some('\\') => (Kind::Note, raw),
            Some(' ') => (Kind::Context, &raw[1..]),
            // An empty kept line whose space was trimmed on the way.
            None => (Kind::Context, ""),
            Some(_) => (Kind::Context, raw),
        };
        let (o, n) = match kind {
            Kind::Added => (None, Some(new)),
            Kind::Removed => (Some(old), None),
            Kind::Context => (Some(old), Some(new)),
            Kind::Hunk | Kind::Note => (None, None),
        };
        match kind {
            Kind::Added => new += 1,
            Kind::Removed => old += 1,
            Kind::Context => {
                old += 1;
                new += 1;
            }
            Kind::Hunk | Kind::Note => {}
        }
        out.push(DiffLine {
            kind,
            old: o,
            new: n,
            text: text.to_string(),
        });
    }
    out
}

/// The lines side by side: each row's line on the left (the old file) and
/// on the right (the new). A run of removed lines faces the added lines
/// after it, in order; a side with nothing to face is `None`. A kept line,
/// a hunk's heading or a note is the same index on both sides.
pub fn split_rows(lines: &[DiffLine]) -> Vec<(Option<usize>, Option<usize>)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if matches!(lines[i].kind, Kind::Removed | Kind::Added) {
            let r0 = i;
            while i < lines.len() && lines[i].kind == Kind::Removed {
                i += 1;
            }
            let a0 = i;
            while i < lines.len() && lines[i].kind == Kind::Added {
                i += 1;
            }
            let (nr, na) = (a0 - r0, i - a0);
            for k in 0..nr.max(na) {
                out.push(((k < nr).then_some(r0 + k), (k < na).then_some(a0 + k)));
            }
        } else {
            out.push((Some(i), Some(i)));
            i += 1;
        }
    }
    out
}

/// Where each hunk starts, as indexes into the lines.
pub fn hunks(lines: &[DiffLine]) -> Vec<usize> {
    lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.kind == Kind::Hunk)
        .map(|(i, _)| i)
        .collect()
}

/// The cursor and how far the view is scrolled, kept in `Cell`s so drawing
/// through `&self` can keep the cursor in view.
#[derive(Debug, Default)]
pub struct DiffState {
    cursor: Cell<usize>,
    /// The first row shown (rows count the blocks under lines too).
    top: Cell<usize>,
    /// Rows visible in the last frame.
    height: Cell<usize>,
    /// The lines' text as painted, with what it was painted from.
    painted: RefCell<Option<(u64, Painted)>>,
}

impl DiffState {
    /// The lines painted, from the last frame's when nothing changed.
    fn painted(&self, theme: &Theme, look: &Look, lines: &[DiffLine], language: Option<&str>) -> Option<Painted> {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (lines, language, look, theme.tier() as u8, theme.s(Slot::AccentPrimary), theme.s(Slot::TextPrimary)).hash(&mut h);
        let key = h.finish();
        if let Some((k, p)) = self.painted.borrow().as_ref()
            && *k == key
        {
            return Some(p.clone());
        }
        let p = Arc::new(paint(theme, look, lines, language));
        *self.painted.borrow_mut() = Some((key, p.clone()));
        Some(p)
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// The line with the cursor.
    pub fn cursor(&self) -> usize {
        self.cursor.get()
    }

    /// Put the cursor on line `i` (kept in view at the next frame).
    pub fn set(&self, i: usize) {
        self.cursor.set(i);
    }

    /// The first row shown.
    pub fn top(&self) -> usize {
        self.top.get()
    }

    /// The viewport size a renderer reported, for page movement.
    pub fn set_height(&self, height: usize) {
        self.height.set(height);
    }

    /// A renderer's local projection starts at this row.
    pub fn set_top(&self, top: usize) {
        self.top.set(top);
    }

    /// Back to the first line, scrolled to the top.
    pub fn reset(&self) {
        self.cursor.set(0);
        self.top.set(0);
    }

    /// `↑↓ j k PgUp PgDn g G Home End` over `n` lines; false for any other
    /// key, and for `↑` on the first line (the host takes it up).
    pub fn key(&self, key: &KeyEvent, n: usize) -> bool {
        let last = n.saturating_sub(1);
        let at = self.cursor.get().min(last);
        let page = self.height.get().saturating_sub(1).max(1);
        let to = match key.code {
            KeyCode::Down | KeyCode::Char('j') => (at + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') if at == 0 => return false,
            KeyCode::Up | KeyCode::Char('k') => at - 1,
            KeyCode::PageDown => (at + page).min(last),
            KeyCode::PageUp => at.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => 0,
            KeyCode::End | KeyCode::Char('G') => last,
            _ => return false,
        };
        self.cursor.set(to);
        true
    }

    /// The same keys side by side ([`split_rows`]): the cursor moves by
    /// rows, staying on the side it is on where the row has a line there
    /// (the new side first).
    pub fn key_split(&self, key: &KeyEvent, lines: &[DiffLine]) -> bool {
        let rows = split_rows(lines);
        if rows.is_empty() {
            return false;
        }
        let at = self.cursor.get();
        let r = rows.iter().position(|(a, b)| *a == Some(at) || *b == Some(at)).unwrap_or(0);
        let last = rows.len() - 1;
        let page = self.height.get().saturating_sub(1).max(1);
        let to = match key.code {
            KeyCode::Down | KeyCode::Char('j') => (r + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') if r == 0 => return false,
            KeyCode::Up | KeyCode::Char('k') => r - 1,
            KeyCode::PageDown => (r + page).min(last),
            KeyCode::PageUp => r.saturating_sub(page),
            KeyCode::Home | KeyCode::Char('g') => 0,
            KeyCode::End | KeyCode::Char('G') => last,
            _ => return false,
        };
        let left = rows[r].0 == Some(at) && rows[r].1 != Some(at);
        let (a, b) = rows[to];
        self.cursor.set(if left { a.or(b) } else { b.or(a) }.unwrap_or(at));
        true
    }

    /// The cursor to the next hunk's heading; false when there is none after it.
    pub fn next_hunk(&self, lines: &[DiffLine]) -> bool {
        let at = self.cursor.get();
        match hunks(lines).into_iter().find(|&h| h > at) {
            Some(h) => {
                self.cursor.set(h);
                true
            }
            None => false,
        }
    }

    /// The cursor to the hunk before (or to the start of its own hunk);
    /// false when there is none before it.
    pub fn previous_hunk(&self, lines: &[DiffLine]) -> bool {
        let at = self.cursor.get();
        match hunks(lines).into_iter().rev().find(|&h| h < at) {
            Some(h) => {
                self.cursor.set(h);
                true
            }
            None => false,
        }
    }
}

/// What a frame drew: each shown diff line's row with its index (for
/// clicks; side by side, each side's half), each line's numbers (a click
/// there can start a comment on it), and each block shown under a line:
/// the line's index and the rows of the block in view (a part draws an
/// input into one).
#[derive(Debug, Default)]
pub struct Drawn {
    pub lines: Vec<(Rect, usize)>,
    pub numbers: Vec<(Rect, usize)>,
    pub under: Vec<(usize, Rect)>,
}

/// The tints added and removed lines sit on, and the stronger ones for the
/// words that changed: the diff slots blended into the panel's background.
/// Only with full colour; otherwise none, and changed words are bold and
/// underlined instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Look {
    added: Option<Color>,
    removed: Option<Color>,
    added_word: Style,
    removed_word: Style,
}

impl Look {
    fn of(theme: &Theme) -> Self {
        let surface = theme.bg(Slot::BgSurface).bg.unwrap_or(Color::Reset);
        let tint = |slot: Slot, amount: f32| theme.s(slot).fg.and_then(|c| blend(c, surface, amount));
        let word = |slot: Slot| match tint(slot, 0.42) {
            Some(bg) => Style::default().bg(bg).add_modifier(Modifier::BOLD),
            None => Style::default().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        };
        Self {
            added: tint(Slot::DiffAdded, 0.16),
            removed: tint(Slot::DiffRemoved, 0.16),
            added_word: word(Slot::DiffAdded),
            removed_word: word(Slot::DiffRemoved),
        }
    }
}

/// Spans cut at byte offsets `from..to` of their text, the part inside
/// patched with `style`.
fn emphasise(spans: Vec<Span<'static>>, ranges: &[(usize, usize)], style: Style) -> Vec<Span<'static>> {
    if ranges.is_empty() {
        return spans;
    }
    let mut out = Vec::new();
    let mut at = 0;
    for s in spans {
        let text = s.content.as_ref();
        let end = at + text.len();
        // Cut points inside this span.
        let mut cuts: Vec<usize> = vec![at, end];
        for &(f, t) in ranges {
            for c in [f, t] {
                if c > at && c < end && text.is_char_boundary(c - at) {
                    cuts.push(c);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for w in cuts.windows(2) {
            let (a, b) = (w[0], w[1]);
            let piece = text[a - at..b - at].to_string();
            let inside = ranges.iter().any(|&(f, t)| a >= f && b <= t);
            let st = if inside { s.style.patch(style) } else { s.style };
            out.push(Span::styled(piece, st));
        }
        at = end;
    }
    out
}

/// Where a removed line and the line that replaced it differ, word by
/// word, as byte ranges of each; nothing when they share too little for
/// the difference to help.
fn changed_words(old: &str, new: &str) -> (Ranges, Ranges) {
    use similar::{Algorithm, DiffTag, capture_diff_slices};
    // Words, runs of spaces, and each other character on its own.
    fn tokens(s: &str) -> Vec<&str> {
        let mut out = Vec::new();
        let mut start = 0;
        let class = |c: char| if c.is_alphanumeric() || c == '_' { 0 } else if c.is_whitespace() { 1 } else { 2 };
        let mut prev: Option<u8> = None;
        for (i, c) in s.char_indices() {
            let k = class(c);
            if prev.is_some_and(|p| p != k || k == 2) {
                out.push(&s[start..i]);
                start = i;
            }
            prev = Some(k);
        }
        if start < s.len() {
            out.push(&s[start..]);
        }
        out
    }
    let (a, b) = (tokens(old), tokens(new));
    let offsets = |t: &[&str]| {
        let mut at = vec![0];
        for x in t {
            at.push(at.last().copied().unwrap_or(0) + x.len());
        }
        at
    };
    let (ao, bo) = (offsets(&a), offsets(&b));
    let (mut o, mut n) = (Vec::new(), Vec::new());
    let mut same = 0usize;
    for op in capture_diff_slices(Algorithm::Myers, &a, &b) {
        let (or, nr) = (op.old_range(), op.new_range());
        match op.tag() {
            DiffTag::Equal => {
                same += a[or].iter().filter(|t| !t.trim().is_empty()).map(|t| t.len()).sum::<usize>();
            }
            DiffTag::Delete => o.push((ao[or.start], ao[or.end])),
            DiffTag::Insert => n.push((bo[nr.start], bo[nr.end])),
            DiffTag::Replace => {
                o.push((ao[or.start], ao[or.end]));
                n.push((bo[nr.start], bo[nr.end]));
            }
        }
    }
    let longest = old.trim().len().max(new.trim().len()).max(1);
    if same * 4 < longest {
        return (vec![], vec![]);
    }
    (o, n)
}

/// Each line's text as spans: highlighted by `language` when one is
/// given and known, the words that changed within a replaced line
/// emphasised. Hunk headings and notes get none.
fn paint(theme: &Theme, look: &Look, lines: &[DiffLine], language: Option<&str>) -> Vec<Vec<Span<'static>>> {
    use super::highlight::{Highlighter, plain};
    let new_hl = language.and_then(Highlighter::new);
    let old_hl = language.and_then(Highlighter::new);
    let (mut new_hl, mut old_hl) = (new_hl, old_hl);
    let text_of = |l: &DiffLine| l.text.replace('\t', "    ");
    let fallback = |l: &DiffLine| -> Vec<Span<'static>> {
        let style = match l.kind {
            Kind::Added => theme.s(Slot::DiffAdded),
            Kind::Removed => theme.s(Slot::DiffRemoved),
            _ => theme.s(Slot::TextPrimary),
        };
        vec![Span::styled(text_of(l), style)]
    };
    let mut out: Vec<Vec<Span<'static>>> = lines
        .iter()
        .map(|l| {
            let text = text_of(l);
            match l.kind {
                Kind::Hunk | Kind::Note => vec![],
                Kind::Context => {
                    if let Some(h) = old_hl.as_mut() {
                        h.line(theme, &text);
                    }
                    match new_hl.as_mut() {
                        Some(h) => h.line(theme, &text),
                        None => plain(theme, &text),
                    }
                }
                Kind::Added => match new_hl.as_mut() {
                    Some(h) => h.line(theme, &text),
                    None => fallback(l),
                },
                Kind::Removed => match old_hl.as_mut() {
                    Some(h) => h.line(theme, &text),
                    None => fallback(l),
                },
            }
        })
        .collect();
    // Removed lines followed by added ones: pair them up in order and
    // emphasise the words that changed.
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != Kind::Removed {
            i += 1;
            continue;
        }
        let r0 = i;
        while i < lines.len() && lines[i].kind == Kind::Removed {
            i += 1;
        }
        let a0 = i;
        while i < lines.len() && lines[i].kind == Kind::Added {
            i += 1;
        }
        for k in 0..(a0 - r0).min(i - a0) {
            let (r, a) = (r0 + k, a0 + k);
            let (o, n) = changed_words(&text_of(&lines[r]), &text_of(&lines[a]));
            out[r] = emphasise(std::mem::take(&mut out[r]), &o, look.removed_word);
            out[a] = emphasise(std::mem::take(&mut out[a]), &n, look.added_word);
        }
    }
    out
}

/// Byte ranges of a line.
type Ranges = Vec<(usize, usize)>;

/// Side by side, where blocks under a line start: they span both sides.
const SPLIT_INDENT: usize = 2;

/// Each line's text as spans, shared between frames.
type Painted = Arc<Vec<Vec<Span<'static>>>>;

/// A diff to draw: its lines, the blocks under some of them, a cursor.
pub struct DiffView<'a> {
    lines: &'a [DiffLine],
    under: Vec<(usize, Vec<Line<'static>>)>,
    focused: bool,
    empty: String,
    language: Option<String>,
    split: bool,
}

impl<'a> DiffView<'a> {
    pub fn new(lines: &'a [DiffLine]) -> Self {
        Self {
            lines,
            under: Vec::new(),
            focused: true,
            empty: "No changes to show.".into(),
            language: None,
            split: false,
        }
    }

    /// Side by side: the old file on the left, the new on the right, a
    /// removed line facing the line that replaced it ([`split_rows`]).
    /// Move the cursor with [`DiffState::key_split`].
    pub fn split(mut self, split: bool) -> Self {
        self.split = split;
        self
    }

    /// Highlight the code as `language` ([`super::highlight::language_of`]
    /// a file's name); unknown, it stays plain. Lines are highlighted once,
    /// when first drawn, and kept on the state until they change.
    pub fn language(mut self, language: Option<String>) -> Self {
        self.language = language;
        self
    }

    /// Lines drawn under line `i`, lined up with its text; drawn as given,
    /// cut at the width (wrap them before). Several under one line stack.
    pub fn under(mut self, i: usize, lines: Vec<Line<'static>>) -> Self {
        self.under.push((i, lines));
        self
    }

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// What it says when there are no lines.
    pub fn empty(mut self, text: impl Into<String>) -> Self {
        self.empty = text.into();
        self
    }

    /// How wide the numbers' column of each side is.
    fn widths(&self) -> (usize, usize) {
        let w = |f: fn(&DiffLine) -> Option<u32>| {
            self.lines
                .iter()
                .filter_map(f)
                .max()
                .map_or(1, |n| n.to_string().len())
        };
        (w(|l| l.old), w(|l| l.new))
    }

    /// Where the text starts, from the left edge: what a part lines up
    /// with when it wraps the lines it puts under one.
    pub fn text_column(&self) -> usize {
        if self.split {
            return SPLIT_INDENT;
        }
        let (o, n) = self.widths();
        1 + o + 1 + n + 1 + 2
    }

    /// One side of a row side by side: the cursor's mark, the number, the
    /// sign and the text, on its tint; the cursor's side lit. Returns the
    /// side's area and its numbers' (for clicks).
    #[allow(clippy::too_many_arguments)]
    fn side(
        &self,
        theme: &Theme,
        look: &Look,
        painted: Option<&Painted>,
        buf: &mut Buffer,
        at: Rect,
        i: Option<usize>,
        old: bool,
        numw: usize,
        cursor: bool,
    ) -> Option<(Rect, Rect)> {
        let i = i?;
        let l = &self.lines[i];
        let (sign, style): (&str, Style) = match l.kind {
            Kind::Added => ("+ ", theme.s(Slot::DiffAdded)),
            Kind::Removed => ("- ", theme.s(Slot::DiffRemoved)),
            _ => ("  ", theme.s(Slot::TextPrimary)),
        };
        let n = if old { l.old } else { l.new };
        let mut spans = vec![
            Span::styled(if cursor { "▎" } else { " " }, theme.s(Slot::AccentPrimary)),
            Span::styled(n.map_or_else(|| " ".repeat(numw + 1), |v| format!("{v:>numw$} ")), theme.s(Slot::TextMuted)),
            Span::styled(sign, style),
        ];
        match painted.and_then(|p| p.get(i)) {
            Some(text) if !text.is_empty() => spans.extend(text.iter().cloned()),
            _ => spans.push(Span::styled(l.text.replace('\t', "    "), style)),
        }
        let numbers = Rect { width: (1 + numw + 1).min(at.width as usize) as u16, ..at };
        let tint = match l.kind {
            Kind::Added => look.added,
            Kind::Removed => look.removed,
            _ => None,
        };
        if let Some(bg) = tint {
            let x = at.x + numbers.width;
            buf.set_style(Rect { x, width: at.width.saturating_sub(numbers.width), ..at }, Style::default().bg(bg));
        }
        buf.set_line(at.x, at.y, &Line::from(super::clip(spans, at.width as usize)), at.width);
        if cursor {
            let lit = widgets::row_style(theme, self.focused, true);
            buf.set_style(Rect { x: at.x + 1, width: at.width.saturating_sub(1), ..at }, lit);
        }
        Some((at, numbers))
    }

    /// Side by side ([`DiffView::split`]).
    fn render_split(self, theme: &Theme, frame: Rect, area: Rect, buf: &mut Buffer, state: &DiffState) -> Drawn {
        let mut drawn = Drawn::default();
        let (ow, nw) = self.widths();
        let pairs = split_rows(self.lines);
        enum Row {
            Pair(usize),
            Under(usize, Line<'static>),
        }
        let mut under = self.under.clone();
        under.sort_by_key(|(i, _)| *i);
        let mut rows: Vec<Row> = Vec::with_capacity(pairs.len());
        let mut span = vec![(0usize, 0usize); self.lines.len()];
        for (k, &(a, b)) in pairs.iter().enumerate() {
            let start = rows.len();
            rows.push(Row::Pair(k));
            let held: Vec<usize> = if a == b { a.into_iter().collect() } else { [a, b].into_iter().flatten().collect() };
            for &i in &held {
                let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut under).into_iter().partition(|(at, _)| *at == i);
                under = rest;
                for (_, block) in mine {
                    rows.extend(block.into_iter().map(|l| Row::Under(i, l)));
                }
            }
            for &i in &held {
                span[i] = (start, rows.len());
            }
        }
        let n = rows.len();
        let h = area.height as usize;
        state.height.set(h);
        let cursor = state.cursor.get().min(self.lines.len() - 1);
        state.cursor.set(cursor);
        let (start, end) = span[cursor];
        let mut first = state.top.get();
        if end > first + h {
            first = end - h;
        }
        if start < first {
            first = start;
        }
        first = first.min(n.saturating_sub(h));
        state.top.set(first);

        // The last column is the scrollbar's; a rule between the sides.
        let usable = area.width.saturating_sub(1);
        let lw = usable / 2;
        let rw = usable.saturating_sub(lw + 1);
        let look = Look::of(theme);
        let painted = state.painted(theme, &look, self.lines, self.language.as_deref());
        let rule = theme.s(Slot::AccentMuted);
        let mut block_at: Option<(usize, Rect)> = None;
        for (row, r) in rows.into_iter().enumerate().skip(first).take(h) {
            let y = area.y + (row - first) as u16;
            match r {
                Row::Pair(k) => {
                    if let Some(b) = block_at.take() {
                        drawn.under.push(b);
                    }
                    let (a, b) = pairs[k];
                    let whole = a == b && a.is_some_and(|i| matches!(self.lines[i].kind, Kind::Hunk | Kind::Note));
                    if whole {
                        let i = a.expect("checked");
                        let style = if self.lines[i].kind == Kind::Hunk { theme.s(Slot::StatusInfo) } else { theme.s(Slot::TextMuted) };
                        let mark = Span::styled(if i == cursor { "▎" } else { " " }, theme.s(Slot::AccentPrimary));
                        let line = Line::from(vec![mark, Span::styled(self.lines[i].text.clone(), style)]);
                        let rect = Rect { y, height: 1, width: usable, ..area };
                        buf.set_line(area.x, y, &line, usable);
                        if i == cursor {
                            let lit = widgets::row_style(theme, self.focused, true);
                            buf.set_style(Rect { x: area.x + 1, width: usable.saturating_sub(1), ..rect }, lit);
                        }
                        drawn.lines.push((rect, i));
                        continue;
                    }
                    let left = Rect { x: area.x, y, width: lw, height: 1 };
                    let right = Rect { x: area.x + lw + 1, y, width: rw, height: 1 };
                    buf.set_string(area.x + lw, y, "│", rule);
                    for (at, i, old, w) in [(left, a, true, ow), (right, b, false, nw)] {
                        let here = i == Some(cursor);
                        if let Some((r, num)) = self.side(theme, &look, painted.as_ref(), buf, at, i, old, w, here) {
                            let i = i.expect("drawn");
                            drawn.lines.push((r, i));
                            drawn.numbers.push((num, i));
                        }
                    }
                }
                Row::Under(i, line) => {
                    let x = area.x + SPLIT_INDENT as u16;
                    let room = (area.width as usize).saturating_sub(SPLIT_INDENT + 1);
                    buf.set_line(x, y, &Line::from(super::clip(line.spans, room)).style(line.style), room as u16);
                    block_at = match block_at {
                        Some((at, r)) if at == i => Some((at, Rect { height: r.height + 1, ..r })),
                        other => {
                            if let Some(b) = other {
                                drawn.under.push(b);
                            }
                            Some((i, Rect { x, y, width: room as u16, height: 1 }))
                        }
                    };
                }
            }
        }
        if let Some(b) = block_at {
            drawn.under.push(b);
        }
        scrollbar(theme, frame, buf, n.saturating_sub(h), first);
        drawn
    }

    /// Draw into `area` (inside a panel `frame`, whose edge takes the
    /// scrollbar), keeping the cursor's line, and as much as fits of what
    /// is under it, in view.
    pub fn render(self, theme: &Theme, frame: Rect, area: Rect, buf: &mut Buffer, state: &DiffState) -> Drawn {
        let mut drawn = Drawn::default();
        if self.lines.is_empty() || area.height == 0 {
            widgets::empty(buf, area, theme, &self.empty);
            state.height.set(area.height as usize);
            return drawn;
        }
        if self.split {
            return self.render_split(theme, frame, area, buf, state);
        }
        let (ow, nw) = self.widths();
        let indent = self.text_column();
        // Rows: a line (its index) or a line of a block under one.
        enum Row {
            Line(usize),
            Under(usize, Line<'static>),
        }
        let mut under = self.under;
        under.sort_by_key(|(i, _)| *i);
        let mut rows: Vec<Row> = Vec::with_capacity(self.lines.len());
        // Each line's first row and the last row of what is under it.
        let mut span: Vec<(usize, usize)> = Vec::with_capacity(self.lines.len());
        let mut blocks = under.into_iter().peekable();
        for i in 0..self.lines.len() {
            let start = rows.len();
            rows.push(Row::Line(i));
            while let Some((_, block)) = blocks.next_if(|(at, _)| *at == i) {
                rows.extend(block.into_iter().map(|l| Row::Under(i, l)));
            }
            span.push((start, rows.len()));
        }
        let n = rows.len();
        let h = area.height as usize;
        state.height.set(h);
        let cursor = state.cursor.get().min(self.lines.len() - 1);
        state.cursor.set(cursor);
        let (start, end) = span[cursor];
        let mut first = state.top.get();
        if end > first + h {
            first = end - h;
        }
        if start < first {
            first = start;
        }
        first = first.min(n.saturating_sub(h));
        state.top.set(first);

        let width = area.width as usize;
        let muted = theme.s(Slot::TextMuted);
        let highlight = widgets::row_style(theme, self.focused, true);
        let look = Look::of(theme);
        let painted = state.painted(theme, &look, self.lines, self.language.as_deref());
        let mut block_at: Option<(usize, Rect)> = None;
        for (row, r) in rows.into_iter().enumerate().skip(first).take(h) {
            let y = area.y + (row - first) as u16;
            let rect = Rect { y, height: 1, ..area };
            match r {
                Row::Line(i) => {
                    if let Some(b) = block_at.take() {
                        drawn.under.push(b);
                    }
                    let l = &self.lines[i];
                    let num = |v: Option<u32>, w: usize| match v {
                        Some(v) => format!("{v:>w$} "),
                        None => " ".repeat(w + 1),
                    };
                    let mut spans = vec![Span::styled(
                        if i == cursor { "▎" } else { " " },
                        theme.s(Slot::AccentPrimary),
                    )];
                    let (sign, style): (&str, Style) = match l.kind {
                        Kind::Hunk => ("", theme.s(Slot::StatusInfo)),
                        Kind::Added => ("+ ", theme.s(Slot::DiffAdded)),
                        Kind::Removed => ("- ", theme.s(Slot::DiffRemoved)),
                        Kind::Context => ("  ", theme.s(Slot::TextPrimary)),
                        Kind::Note => ("  ", muted),
                    };
                    if l.kind == Kind::Hunk {
                        spans.push(Span::styled(l.text.clone(), style));
                    } else {
                        spans.push(Span::styled(num(l.old, ow), muted));
                        spans.push(Span::styled(num(l.new, nw), muted));
                        spans.push(Span::styled(sign, style));
                        match painted.as_ref().and_then(|p| p.get(i)) {
                            Some(text) if !text.is_empty() => spans.extend(text.iter().cloned()),
                            _ => spans.push(Span::styled(l.text.replace('\t', "    "), style)),
                        }
                    }
                    // Added and removed lines on their tint, across the row.
                    let tint = match l.kind {
                        Kind::Added => look.added,
                        Kind::Removed => look.removed,
                        _ => None,
                    };
                    if let Some(bg) = tint {
                        let x = area.x + (1 + ow + 1 + nw + 1) as u16;
                        let w = area.width.saturating_sub(x - area.x + 1);
                        buf.set_style(Rect { x, width: w, ..rect }, Style::default().bg(bg));
                    }
                    let shown = super::clip(spans, width.saturating_sub(1));
                    buf.set_line(area.x, y, &Line::from(shown), area.width);
                    if i == cursor {
                        buf.set_style(Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..rect }, highlight);
                    }
                    drawn.lines.push((rect, i));
                    if l.kind != Kind::Hunk {
                        drawn.numbers.push((Rect { width: (1 + ow + 1 + nw + 1).min(width) as u16, ..rect }, i));
                    }
                }
                Row::Under(i, line) => {
                    let x = area.x + indent.min(width) as u16;
                    let room = width.saturating_sub(indent + 1);
                    let shown = super::clip(line.spans, room);
                    buf.set_line(x, y, &Line::from(shown).style(line.style), room as u16);
                    block_at = match block_at {
                        Some((at, r)) if at == i => Some((at, Rect { height: r.height + 1, ..r })),
                        other => {
                            if let Some(b) = other {
                                drawn.under.push(b);
                            }
                            Some((i, Rect { x, y, width: room as u16, height: 1 }))
                        }
                    };
                }
            }
        }
        if let Some(b) = block_at {
            drawn.under.push(b);
        }
        scrollbar(theme, frame, buf, n.saturating_sub(h), first);
        drawn
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::gallery::SAMPLE_DIFF as SAMPLE;
    use ratatui::crossterm::event::KeyModifiers;

    #[test]
    fn a_unified_diff_is_read_with_both_sides_numbers() {
        let lines = parse(SAMPLE);
        assert_eq!(lines[0].kind, Kind::Hunk);
        assert_eq!((lines[1].old, lines[1].new), (Some(18), Some(18)));
        let removed = &lines[3];
        assert_eq!((removed.kind, removed.old, removed.new), (Kind::Removed, Some(20), None));
        assert_eq!(removed.text, "    // TODO: the dashboard");
        let added = &lines[4];
        assert_eq!((added.kind, added.old, added.new), (Kind::Added, None, Some(20)));
        assert_eq!(lines[5].text, "", "an added empty line");
        assert_eq!(lines[12].kind, Kind::Note);
        assert_eq!((lines[14].old, lines[14].new), (Some(40), Some(46)));
        assert_eq!(lines[14].text, "void sort() {");
        assert_eq!(hunks(&lines), [0, 13]);
        // File headings before the first hunk are not lines.
        let git = format!("diff --git a/x b/x\nindex 1..2\n--- a/x\n+++ b/x\n{SAMPLE}");
        assert_eq!(parse(&git), lines);
        assert!(parse("").is_empty());
    }

    #[test]
    fn the_cursor_moves_by_line_page_and_hunk() {
        let lines = parse(SAMPLE);
        let s = DiffState::new();
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        assert!(s.key(&key(KeyCode::Down), lines.len()));
        assert_eq!(s.cursor(), 1);
        assert!(s.next_hunk(&lines));
        assert_eq!(s.cursor(), 13);
        assert!(!s.next_hunk(&lines), "none after the last");
        assert!(s.previous_hunk(&lines));
        assert_eq!(s.cursor(), 0);
        assert!(s.key(&key(KeyCode::Char('G')), lines.len()));
        assert_eq!(s.cursor(), lines.len() - 1);
        assert!(!s.key(&key(KeyCode::Char('x')), lines.len()));
    }

    #[test]
    fn the_cursor_and_what_is_under_it_stay_in_view() {
        let lines = parse(SAMPLE);
        let t = Theme::new(crate::design::theme::Tier::Mono);
        let area = Rect::new(0, 0, 60, 6);
        let mut buf = Buffer::empty(area);
        let s = DiffState::new();
        s.set(9);
        let under = vec![Line::raw("a note"), Line::raw("a reply")];
        let drawn = DiffView::new(&lines).under(9, under).render(&t, area, area, &mut buf, &s);
        let shown: Vec<usize> = drawn.lines.iter().map(|(_, i)| *i).collect();
        assert!(shown.contains(&9), "{shown:?}");
        assert_eq!(drawn.under.len(), 1);
        assert_eq!(drawn.under[0].0, 9);
        assert_eq!(drawn.under[0].1.height, 2, "both lines of the block in view");
        assert!(s.top() > 0, "scrolled to keep it in view");
    }

    #[test]
    fn side_by_side_a_removed_line_faces_the_line_that_replaced_it() {
        let lines = parse(SAMPLE);
        let rows = split_rows(&lines);
        // The hunk heading and kept lines are one index on both sides.
        assert_eq!(rows[0], (Some(0), Some(0)));
        assert_eq!(rows[1], (Some(1), Some(1)));
        // Line 3 (removed) faces 4 (added); 5 (added) faces nothing.
        assert_eq!(rows[3], (Some(3), Some(4)));
        assert_eq!(rows[4], (None, Some(5)));
        let every: usize = rows.iter().map(|(a, b)| if a == b { 1 } else { a.is_some() as usize + b.is_some() as usize }).sum();
        assert_eq!(every, lines.len(), "every line once");
        // The cursor moves by rows and keeps its side.
        let s = DiffState::new();
        let key = |c| KeyEvent::new(c, KeyModifiers::NONE);
        s.set(2);
        assert!(s.key_split(&key(KeyCode::Down), &lines));
        assert_eq!(s.cursor(), 4, "the new side of the paired row");
        assert!(s.key_split(&key(KeyCode::Down), &lines));
        assert_eq!(s.cursor(), 5);
        s.set(3);
        assert!(s.key_split(&key(KeyCode::Up), &lines));
        assert_eq!(s.cursor(), 2);
        s.set(0);
        assert!(!s.key_split(&key(KeyCode::Up), &lines), "↑ on the first row is the host's");
        assert!(!s.key(&key(KeyCode::Up), lines.len()), "and on the first line");
        // Drawn: both sides of a paired row, a click target each, and the
        // numbers of each.
        let t = Theme::new(crate::design::theme::Tier::Mono);
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        s.set(4);
        let drawn = DiffView::new(&lines).split(true).render(&t, area, area, &mut buf, &s);
        let row3: Vec<usize> = drawn.lines.iter().filter(|(r, _)| r.y == 3).map(|(_, i)| *i).collect();
        assert_eq!(row3, [3, 4]);
        assert!(drawn.numbers.iter().any(|(_, i)| *i == 3) && drawn.numbers.iter().any(|(_, i)| *i == 4));
        let text: String = (0..80).map(|x| buf[(x, 3)].symbol().to_string()).collect();
        assert!(text.contains("// TODO: the dashboard") && text.contains('│'), "{text}");
    }

    #[test]
    fn the_words_that_changed_in_a_replaced_line_stand_out() {
        let (o, n) = changed_words("    by(decade);", "    by(decade, title);");
        assert!(o.is_empty() || o.iter().all(|&(f, t)| f < t));
        let new = "    by(decade, title);";
        let added: String = n.iter().map(|&(f, t)| &new[f..t]).collect();
        assert!(added.contains("title"), "{added:?}");
        assert!(!added.contains("by"), "{added:?}");
        // Wholly different lines: nothing singled out.
        assert_eq!(changed_words("return a;", "throw new Error(x);"), (vec![], vec![]));
        let t = Theme::new(crate::design::theme::Tier::TrueColor);
        let look = Look::of(&t);
        assert!(look.added.is_some(), "a tint with full colour");
        let lines = parse(SAMPLE);
        let painted = paint(&t, &look, &lines, Some("java"));
        let last: Vec<&Span> = painted[16].iter().collect();
        let emphasised: String = last
            .iter()
            .filter(|s| s.style.add_modifier.contains(Modifier::BOLD) && s.style.bg.is_some())
            .map(|s| s.content.as_ref())
            .collect();
        assert!(emphasised.contains("title"), "{last:?}");
        // The text is all there, highlighted.
        let text: String = painted[7].iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "    public Shelf shelf(@PathVariable UUID id) {");
        assert!(painted[7].len() > 1, "split by what each part is");
        // Without colour, the changed words are bold and underlined.
        let mono = Look::of(&Theme::new(crate::design::theme::Tier::Mono));
        assert_eq!((mono.added, mono.added_word.add_modifier.contains(Modifier::UNDERLINED)), (None, true));
    }
}
