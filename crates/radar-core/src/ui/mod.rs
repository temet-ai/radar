//! The kit: the elements every part draws its screens with, so a tree, a
//! table, a page or a detail pane looks and behaves the same in every part.
//!
//! Each element is built on a library where one does the job (ratatui's
//! `Table`, `Tabs`, `List`, `LineGauge`, `Sparkline` and `Scrollbar`,
//! `tui-tree-widget`, `tui-scrollview`, `throbber-widgets-tui`), takes the
//! [`Theme`](crate::Theme) and draws only with its slots. Where a part lets
//! the user choose a slot for a word (its look words), the element takes the
//! resolved style, so the choice stays the part's.
//!
//! | Element | For | Module |
//! |---|---|---|
//! | banded tree | a hierarchy with tree lines, a stripe per group, bands, `⋯` when folded | [`tree`] |
//! | columns | named columns at the right of rows, dropped by width | [`columns`] |
//! | table | a flat list of rows with headings and a cursor | [`table`] |
//! | list | selectable rows with a word at the right edge (settings, nav panes) | [`rows`] |
//! | entries | rows with lines under them, and headings that are never chosen | [`entries`] |
//! | log | a long output with a cursor, folding sections, search, its colours | [`log`] |
//! | highlight | code coloured by language, as spans per line (theme slots; plain when unknown) | [`highlight`] |
//! | diff | a file's changes: hunks, both sides' line numbers, a cursor, blocks under lines | [`diff`] |
//! | rich text | one document model (blocks, runs with marks, links, mentions, emoji, badges, media) laid out as lines, with its links | [`rich`] |
//! | markdown | text written in markdown (a description, a comment) parsed into rich text | [`markdown`] |
//! | page frame | breadcrumb, title and details, a line under it, tabs, the body | [`page`] |
//! | card | a titled panel of text that scrolls, with a scrollbar | [`card`], [`scroll`] |
//! | chips | short labels set apart from text | [`chips`] |
//! | status strip, checks | counts in a row; a check's verdict | [`status`] |
//! | stage bar, gauge, sparkline | steps of a run, a ratio, numbers over time | [`meter`] |
//! | key-value rows, add row | facts beside their labels; the `+ add …` row | [`rows`] |
//! | composer | text typed for a write: `⏎` a new line, `ctrl-s` review, `esc` stop | [`composer`] |
//! | connection | a part's connection section: its rows, words, detail panels, and what was found in the environment as a question | [`connection`] |
//! | empty, set-up, loading, spinner | what a panel shows when it has nothing yet | [`states`] |
//! | step rail | a run's steps along a line: done, skipped, current, ahead, and how each went | [`rail`] |
//!
//! See every element live with `cargo run -p radar-core --example gallery`.
//! Parts use these instead of drawing their own (docs/GUIDE.md 1.15).

pub mod card;
pub mod chips;
pub mod columns;
pub mod composer;
pub mod connection;
pub mod diff;
pub mod entries;
pub mod gallery;
pub mod highlight;
pub mod hover;
pub mod icon;
pub mod image;
pub mod link;
pub mod log;
pub mod markdown;
pub mod meter;
pub mod motion_demo;
pub mod page;
pub mod pointer;
pub mod rail;
pub mod rich;
pub mod rows;
pub mod scrim;
pub mod scroll;
pub mod states;
pub mod status;
pub mod stories;
pub mod table;
pub mod tree;

#[cfg(test)]
mod tests;

pub use columns::{Column, Columns};
pub use scroll::Scroll;
pub use tree::{BandedTree, Node, TreeLook};

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::plugin::Tone;

/// How many cells `text` takes on screen: an emoji or a CJK character two,
/// a combining mark none. Every fit, cut and pad in the kit counts this,
/// never characters, so wide text never moves a column.
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// The longest start of `text` that fits in `w` cells, and the cells it
/// takes. A wide character that would cross the edge is left out whole,
/// never drawn half.
pub fn take(text: &str, w: usize) -> (&str, usize) {
    let mut used = 0;
    for (i, c) in text.char_indices() {
        let cw = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > w {
            return (&text[..i], used);
        }
        used += cw;
    }
    (text, used)
}

/// `text` with spaces after it to `w` cells (as it is when wider).
pub fn pad(text: &str, w: usize) -> String {
    let n = width(text);
    format!("{text}{}", " ".repeat(w.saturating_sub(n)))
}

/// `text` cut to `w` cells with an ellipsis, or padded to them; always
/// exactly `w` cells.
pub fn fit(text: &str, w: usize) -> String {
    if width(text) <= w {
        pad(text, w)
    } else if w == 0 {
        String::new()
    } else {
        let (head, used) = take(text, w - 1);
        format!("{head}…{}", " ".repeat(w - 1 - used))
    }
}

/// A cell of a row: `text` fitted to `w - 1` and one space after.
pub fn cell(text: &str, w: usize, style: Style) -> Span<'static> {
    Span::styled(format!("{} ", fit(text, w.saturating_sub(1))), style)
}

/// `Alice Example` → `Alice E.`; one name stays as it is. A surname with a
/// particle (a lower-case word between the first name and the last) stays
/// whole, since an initial would drop what makes it theirs: `Maria de Souza`
/// stays `Maria de Souza`, not `Maria S.`.
pub fn short_name(name: &str) -> String {
    let parts: Vec<&str> = name.split_whitespace().collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => (*one).to_string(),
        [first, middle @ .., last] => {
            let particle = middle
                .iter()
                .any(|w| w.chars().next().is_some_and(char::is_lowercase));
            if particle {
                parts.join(" ")
            } else {
                format!("{first} {}.", last.chars().next().unwrap_or(' '))
            }
        }
    }
}

/// The detail pane's width beside a list in `total` cells: 30%, never under 38
/// nor over 60. One fit for every screen of every part (the nav pane's width is
/// its own).
pub fn detail_width(total: u16) -> u16 {
    (total * 30 / 100).clamp(38, 60)
}

/// `45s`, `6m 12s`, `1h 05m`.
pub fn duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h {:02}m", secs / 3600, secs % 3600 / 60)
    } else if secs >= 60 {
        format!("{}m {:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// `text` broken between words to fit `width` after `prefix`, each line
/// behind the prefix (a quote bar `│ `, an indent).
pub fn wrapped(text: &str, width: u16, prefix: Span<'static>, style: Style) -> Vec<Line<'static>> {
    wrap_words(text, width, self::width(&prefix.content))
        .into_iter()
        .map(|line| Line::from(vec![prefix.clone(), Span::styled(line, style)]))
        .collect()
}

/// Word wrapping before any frontend chooses colors or cell styles.
pub fn wrap_words(text: &str, width: u16, prefix_width: usize) -> Vec<String> {
    let cells = self::width;
    let room = (width as usize).saturating_sub(prefix_width).max(8);
    let mut out = Vec::new();
    for para in text.lines() {
        let mut line = String::new();
        for word in para.split_whitespace() {
            if !line.is_empty() && cells(&line) + 1 + cells(word) > room {
                out.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        out.push(line);
    }
    out
}

/// Spans cut to `w` cells, keeping their styles.
pub fn clip(spans: Vec<Span<'static>>, w: usize) -> Vec<Span<'static>> {
    let mut used = 0;
    let mut out = Vec::new();
    for s in spans {
        if used >= w {
            break;
        }
        let (text, n) = take(&s.content, w - used);
        used += n;
        let cut = text.len() < s.content.len();
        out.push(Span::styled(text.to_string(), s.style));
        if cut {
            // Cut short (a wide character at the edge): the line ends here.
            break;
        }
    }
    out
}

/// The state of a run, a step or a check, as a symbol that reads without
/// colour. A part maps its own states onto these and styles them with its
/// own look words; [`Mark::tone`] is the default feeling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Mark {
    Done,
    Failed,
    Running,
    Waiting,
    Manual,
    Stopped,
    Skipped,
}

impl Mark {
    pub fn symbol(self) -> &'static str {
        match self {
            Mark::Done => "✓",
            Mark::Failed => "✗",
            Mark::Running => "●",
            Mark::Waiting => "◷",
            Mark::Manual => "▷",
            Mark::Stopped => "⊘",
            Mark::Skipped => "»",
        }
    }

    /// Its icon ([`icon`]): a run's state.
    pub fn icon(self) -> icon::Icon {
        use icon::Icon;
        match self {
            Mark::Done => Icon::RunPassed,
            Mark::Failed => Icon::RunFailed,
            Mark::Running => Icon::RunRunning,
            Mark::Waiting => Icon::RunPending,
            Mark::Manual => Icon::RunManual,
            Mark::Stopped => Icon::RunCanceled,
            Mark::Skipped => Icon::RunSkipped,
        }
    }

    /// Drawn: its icon when icons are on, else its symbol in `style`.
    pub fn span(self, theme: &crate::Theme, style: Style) -> Span<'static> {
        icon::or(theme, self.icon(), self.symbol(), style)
    }

    pub fn tone(self) -> Tone {
        match self {
            Mark::Done => Tone::Good,
            Mark::Failed => Tone::Bad,
            Mark::Running => Tone::Info,
            Mark::Manual => Tone::Accent,
            Mark::Waiting | Mark::Stopped | Mark::Skipped => Tone::Muted,
        }
    }
}
