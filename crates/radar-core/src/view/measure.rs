//! What a frame measures before anything is drawn, from view data and cell
//! widths alone: how many lines the open menu has, how many show, whether
//! its controls fit and where it reads from; and how far settings' detail
//! (and setup's sheet) can scroll. The host runs it while it builds the
//! chrome, so every frontend is sent the same numbers. No renderer type is
//! named here: sizes are [`Size`], text is counted with [`crate::design::width`].
//!
//! Ink lays the same rows out itself; these are the numbers the host keeps
//! for paging and scroll limits. Wrapping ([`rows`]) is by words, breaking a
//! word longer than the line by grapheme, the way a wrapping paragraph does,
//! with the differences listed on [`rows`].

use std::ops::Range;

use crate::design::width::{clusters, width as cells};
use crate::host::MenuViewport;
use crate::mnemonic;
use crate::theme::Theme;
use crate::ui::icon::{self, Icon};
use crate::view::chrome::{Choice, MenuView};
use crate::view::{Block, SectionView, Text};

/// A room in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

impl Size {
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }
}

// --- wrapping ---------------------------------------------------------

/// A paragraph longer than this many cells is counted in chunks, so a row
/// count never overflows the 16-bit offsets a frontend scrolls by.
const MOST_CELLS: usize = 32_768;

/// How many rows `text` takes in `width` cells: each `\n` ends a line, a
/// line wraps at word boundaries, a word wider than the room is broken by
/// grapheme, and an empty line is one row. Leading spaces stay (indent);
/// spaces at a wrap and at the end of a line are dropped.
///
/// Where this differs from the paragraph the removed terminal shell counted with:
/// whitespace runs that overflow are dropped rather than carried, a tab is
/// one cell, and a line wider than 32 768 cells is counted in chunks cut on
/// grapheme boundaries (as the shell did). `design::width::wrap_words` is
/// not used for the count: it joins words with single spaces, keeps no
/// indent, never breaks a long word and raises the room to at least eight.
pub fn rows(text: &str, width: usize) -> usize {
    let width = width.max(1);
    text.split('\n')
        .map(|line| {
            chunks(line.trim_end_matches('\r'))
                .into_iter()
                .map(|chunk| line_rows(chunk, width))
                .sum::<usize>()
                .max(1)
        })
        .sum()
}

fn chunks(line: &str) -> Vec<&str> {
    if cells(line) <= MOST_CELLS {
        return vec![line];
    }
    let mut out = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (at, _, w) in clusters(line) {
        if used + w > MOST_CELLS {
            out.push(&line[start..at]);
            start = at;
            used = 0;
        }
        used += w;
    }
    out.push(&line[start..]);
    out
}

fn line_rows(line: &str, width: usize) -> usize {
    let mut rows = 1;
    let mut used = 0;
    let mut pending = 0;
    let mut word = String::new();
    let flush = |word: &mut String, pending: &mut usize, used: &mut usize, rows: &mut usize| {
        if word.is_empty() {
            return;
        }
        let w = cells(word);
        if *used + *pending + w <= width {
            *used += *pending + w;
        } else {
            if *used > 0 {
                *rows += 1;
                *used = 0;
            }
            if w <= width {
                *used = w;
            } else {
                for (_, _, cw) in clusters(word) {
                    if *used > 0 && *used + cw > width {
                        *rows += 1;
                        *used = 0;
                    }
                    *used += cw;
                }
            }
        }
        *pending = 0;
        word.clear();
    };
    for c in line.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut pending, &mut used, &mut rows);
            pending += 1;
        } else {
            word.push(c);
        }
    }
    flush(&mut word, &mut pending, &mut used, &mut rows);
    rows
}

/// Rows of `lines` wrapped by grapheme at `width` (at least two), the way a
/// palette's rows wrap: a continuation stays part of its command and no
/// text is clipped.
fn grapheme_rows(lines: &[String], width: usize) -> usize {
    let width = width.max(2);
    lines
        .iter()
        .map(|line| {
            let mut rows = 1;
            let mut used = 0;
            for (_, _, w) in clusters(line) {
                if used > 0 && used + w > width {
                    rows += 1;
                    used = 0;
                }
                used += w;
            }
            rows
        })
        .sum()
}

fn lines_of(text: &str) -> Vec<String> {
    text.split('\n').map(|s| s.trim_end_matches('\r').to_string()).collect()
}

// --- the menu ---------------------------------------------------------

fn mnemonic_at(item: &Choice) -> Option<usize> {
    item.mnemonic_at.filter(|_| mnemonic::valid_at(&item.keys, &item.label, item.mnemonic_at))
}

fn separate_key(item: &Choice) -> &str {
    if mnemonic_at(item).is_some() { "" } else { &item.keys }
}

fn shift_width(item: &Choice) -> usize {
    if mnemonic_at(item).is_some() && item.mnemonic_shift { 6 } else { 0 }
}

fn lead(theme: &Theme, icon: Option<Icon>) -> String {
    icon.map(|i| icon::text(theme, i)).unwrap_or_default()
}

/// The lines of a menu's rows, how many of them each row takes, and what
/// they add up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuMeasure {
    /// The body's width in cells (the frame's inside).
    pub width: u16,
    /// Rows of the body that show.
    pub shown: usize,
    /// Rows the content takes at `width`.
    pub total: usize,
    /// The controls fit in the room; else a frontend says to make the
    /// terminal larger.
    pub usable: bool,
    /// Each part's rows, with the item it belongs to (`None` for a title or
    /// heading).
    pub parts: Vec<(Option<usize>, Range<usize>)>,
    pub question: bool,
    /// The height of the footer (a question's buttons) and of the typed
    /// field above the body.
    pub footer: usize,
    pub field: usize,
}

struct Content {
    width: u16,
    parts: Vec<(Option<usize>, Range<usize>)>,
    total: usize,
}

impl Content {
    fn push(&mut self, lines: &[String], inset: u16, item: Option<usize>) {
        let inset = inset.min(self.width.saturating_sub(2));
        let room = usize::from(self.width.saturating_sub(inset));
        for line in lines {
            let height = rows(line, room);
            self.parts.push((item, self.total..self.total + height));
            self.total += height;
        }
    }

    /// One row already known to fit.
    fn push_rows(&mut self, height: usize, item: Option<usize>) {
        self.parts.push((item, self.total..self.total + height));
        self.total += height;
    }
}

/// A question's button: only what its width and caption need.
struct Button {
    keys: String,
    icon: String,
    words: String,
    at: Option<usize>,
    shift: bool,
}

impl Button {
    fn prefix(&self) -> &str {
        mnemonic::prefix(&self.keys, &self.words, self.at, self.shift)
    }

    fn inline_shift(&self) -> bool {
        mnemonic::valid_at(&self.keys, &self.words, self.at) && self.shift
    }

    /// A cell either side of the key and the words.
    fn width(&self) -> usize {
        let prefix = self.prefix();
        2 + if prefix.is_empty() { 0 } else { cells(prefix) + 1 } + cells(&self.icon) + cells(&self.words)
    }

    /// The face: ` key ` or the modifier, the icon and the words.
    fn caption(&self) -> String {
        let prefix = self.prefix();
        let head = if prefix.is_empty() || self.inline_shift() { " ".to_string() } else { format!(" {prefix} ") };
        format!("{head}{}{}{} ", self.icon, if self.inline_shift() { "shift " } else { "" }, self.words)
    }
}

fn button_row_width(buttons: &[(usize, Button)], width: u16) -> usize {
    let (mut current, mut widest) = (0, 0);
    for (_, button) in buttons {
        let w = button.width().min(usize::from(width));
        if current > 0 && current + 2 + w > usize::from(width) {
            widest = widest.max(current);
            current = 0;
        }
        current += usize::from(current > 0) * 2 + w;
    }
    widest.max(current)
}

/// The footer's height: buttons side by side while they fit, wrapped onto
/// rows when not; raised ones carry a bevel row either side.
fn footer(buttons: &[(usize, Button)], width: u16, raised: bool) -> usize {
    let mut rows: Vec<usize> = Vec::new();
    let mut used = 0usize;
    for (_, button) in buttons {
        let w = button.width().min(usize::from(width));
        let height = rows_of_caption(button, w) + if raised { 2 } else { 0 };
        if rows.is_empty() || used + 2 + w > usize::from(width) {
            rows.push(0);
            used = 0;
        }
        if let Some(last) = rows.last_mut() {
            *last = (*last).max(height);
        }
        used += if used == 0 { 0 } else { 2 } + w;
    }
    rows.iter().sum()
}

fn rows_of_caption(button: &Button, w: usize) -> usize {
    rows(&button.caption(), w).max(1)
}

/// Lay out the menu `view` in a body `area` big and count it. Pure.
pub fn menu(view: &MenuView, theme: &Theme, area: Size) -> MenuMeasure {
    let max_w = area.width.saturating_sub(2);
    let max_h = area.height.saturating_sub(2);
    let items = &view.items;
    let question = view.question;
    let title_text = view.palette.as_ref().map_or_else(
        || view.title.clone(),
        |palette| format!("{} · {} of {}", view.title, palette.matched, palette.total),
    );
    let title_icon = match view.id.as_str() {
        _ if question => Some(Icon::Warning),
        crate::host::BULK_MENU => Some(Icon::Bulk),
        "keys" => Some(Icon::Menu),
        "quit" => Some(Icon::Quit),
        _ => None,
    };
    let title_icons: Vec<String> = title_icon.into_iter().chain(view.icon).map(|i| icon::text(theme, i)).collect();
    let title_w = 1 + title_icons.iter().map(|s| cells(s)).sum::<usize>() + cells(&title_text) + 1;
    let key_w = items.iter().map(|i| cells(separate_key(i))).max().unwrap_or(0);
    let label_w = items
        .iter()
        .filter(|i| !i.heading)
        .map(|i| cells(&i.label) + shift_width(i) + cells(&lead(theme, i.icon)))
        .max()
        .unwrap_or(0);
    // Measured before padding: one exceptionally long label must not make
    // every other row allocate that label's natural width.
    let compact_widths: Vec<usize> = items
        .iter()
        .map(|item| {
            let line_w = |s: &str| s.split('\n').map(cells).max().unwrap_or(0);
            if view.palette.is_some() {
                return line_w(&item.label)
                    + shift_width(item)
                    + cells(&lead(theme, item.icon))
                    + if item.sample.is_some() { 3 } else { 0 }
                    + if item.hint.is_empty() { 0 } else { 3 + line_w(&item.hint) }
                    + if key_w == 0 { 2 } else { key_w + 4 };
            }
            (if key_w == 0 { 1 } else { key_w.saturating_add(2) })
                .saturating_add(if item.sample.is_some() { 3 } else { 0 })
                .saturating_add(1)
                .saturating_add(label_w)
                .saturating_add(4)
                .saturating_add(cells(&item.hint))
        })
        .collect();
    let buttons: Vec<(usize, Button)> = if question {
        items
            .iter()
            .enumerate()
            .filter(|(_, i)| !i.heading && !i.note)
            .map(|(i, item)| {
                (
                    i,
                    Button {
                        keys: item.keys.clone(),
                        icon: lead(theme, item.icon),
                        words: mnemonic::choice_words(true, &item.label, &item.hint).into(),
                        at: item.mnemonic_at,
                        shift: item.mnemonic_shift,
                    },
                )
            })
            .collect()
    } else {
        Vec::new()
    };
    let natural = compact_widths
        .iter()
        .enumerate()
        .filter(|(i, _)| !items[*i].heading && !question)
        .map(|(_, w)| *w)
        .chain(
            items
                .iter()
                .filter(|i| i.heading)
                .map(|i| 1 + cells(&i.label) + cells(&lead(theme, i.icon))),
        )
        .chain([title_w, if view.typed.is_some() { 58 } else { 0 }])
        .max()
        .unwrap_or(1);
    let button_width = button_row_width(&buttons, max_w);
    let width = natural.max(button_width).min(usize::from(max_w)) as u16;
    let columns_fit = compact_widths.iter().enumerate().all(|(i, w)| {
        items[i].heading
            || (*w <= usize::from(width) && !items[i].label.contains('\n') && !items[i].hint.contains('\n'))
    });
    let long_title = title_w > usize::from(max_w) || title_text.contains('\n');
    let narrow_count = view
        .palette
        .as_ref()
        .filter(|_| long_title)
        .map(|palette| format!(" {} of {} ", palette.matched, palette.total))
        .filter(|line| cells(line) <= usize::from(width));
    let mut content = Content { width, parts: Vec::new(), total: 0 };
    let palette_room = usize::from(width.saturating_sub(1));
    if long_title {
        let words = if narrow_count.is_some() { &view.title } else { &title_text };
        let mut lines = lines_of(words);
        lines[0] = format!("{}{}", title_icons.concat(), lines[0]);
        if view.palette.is_some() {
            content.push_rows(grapheme_rows(&lines, palette_room), None);
        } else {
            content.push(&lines, 1, None);
        }
    }
    for (i, item) in items.iter().enumerate() {
        if item.heading || item.note {
            let mut lines = if item.note {
                lines_of(&item.label)
            } else {
                item.label.split('\n').map(str::to_uppercase).collect()
            };
            lines[0] = format!("{}{}", lead(theme, item.icon), lines[0]);
            if view.palette.is_some() {
                content.push_rows(grapheme_rows(&lines, palette_room), None);
            } else {
                content.push(&lines, 1, None);
            }
        } else if !question {
            if view.palette.is_some() {
                content.push_rows(palette_rows(theme, item, width, key_w), Some(i));
            } else if columns_fit {
                content.push_rows(1, Some(i));
            } else {
                // Leave room for at least one two-cell grapheme. At tiny
                // widths the key becomes its own wrapped block.
                let prefix_w = if key_w == 0 { 2 } else { key_w.saturating_add(3) };
                let inset = if prefix_w + 2 <= usize::from(width) { prefix_w as u16 } else { 0 };
                if inset == 0 && !separate_key(item).is_empty() {
                    content.push(&[separate_key(item).to_owned()], 0, Some(i));
                }
                let mut lines = lines_of(&item.label);
                let before = format!(
                    "{}{}{}",
                    if item.sample.is_some() { "██ " } else { "" },
                    lead(theme, item.icon),
                    if shift_width(item) > 0 { "shift " } else { "" }
                );
                lines[0] = format!("{before}{}", lines[0]);
                content.push(&lines, inset, Some(i));
                if !item.hint.is_empty() {
                    content.push(&lines_of(&item.hint), inset, Some(i));
                }
            }
        }
    }
    let field = usize::from(view.typed.is_some());
    let min_body = usize::from(content.total > 0);
    let mut footer_h = footer(&buttons, width, true);
    if footer_h + field + min_body > usize::from(max_h) {
        footer_h = footer(&buttons, width, false);
    }
    let min_width = if view.typed.is_some() { 4 } else { 2 };
    let usable = width >= min_width && usize::from(max_h) >= footer_h + field + min_body && max_h > 0;
    let gap = usize::from(content.total > 0 && footer_h > 0 && usize::from(max_h) > footer_h + field + min_body);
    let shown = content.total.min(usize::from(max_h).saturating_sub(footer_h + field + gap));
    MenuMeasure {
        width,
        shown,
        total: content.total,
        usable,
        parts: content.parts,
        question,
        footer: footer_h,
        field,
    }
}

/// A palette row: its label lines with the hint joined to the last, wrapped
/// by grapheme beside the key column (or under it when the key will not fit).
fn palette_rows(theme: &Theme, item: &Choice, width: u16, key_w: usize) -> usize {
    let mut labels = lines_of(&item.label);
    labels[0] = format!(
        "{}{}{}{}",
        if item.sample.is_some() { "██ " } else { "" },
        lead(theme, item.icon),
        if shift_width(item) > 0 { "shift " } else { "" },
        labels[0]
    );
    if !item.hint.is_empty() {
        let mut hints = lines_of(&item.hint).into_iter();
        if let (Some(first), Some(last)) = (hints.next(), labels.last_mut()) {
            last.push_str("   ");
            last.push_str(&first);
        }
        labels.extend(hints);
    }
    let inner = usize::from(width).saturating_sub(2).max(2);
    let side_keys = key_w > 0 && key_w.saturating_add(4) <= inner;
    let text_w = if side_keys { inner - key_w - 2 } else { inner };
    let mut total = grapheme_rows(&labels, text_w);
    if !side_keys && !separate_key(item).is_empty() {
        total += grapheme_rows(&lines_of(separate_key(item)), inner);
    }
    total
}

impl MenuMeasure {
    /// Where the menu reads from after this layout: `at` (the reading
    /// position so far) kept so the selected row `sel` is in sight when the
    /// selection or the size changed. Questions begin at their first words:
    /// their choice stays visible in the fixed footer.
    pub fn scroll_for(&self, at: usize, sel: usize, selection_changed: bool, resized: bool) -> usize {
        let body_h = self.shown;
        let max = self.total.saturating_sub(body_h);
        let mut at = at.min(max);
        if !self.question && (selection_changed || resized) {
            let range = self
                .parts
                .iter()
                .filter(|(item, _)| *item == Some(sel))
                .fold(None, |range: Option<Range<usize>>, (_, rows)| {
                    Some(range.map_or_else(|| rows.clone(), |r| r.start..rows.end))
                });
            if let Some(range) = range {
                if range.len() > body_h {
                    if selection_changed || range.end <= at || range.start >= at + body_h {
                        at = range.start;
                    }
                } else if range.start < at {
                    at = range.start;
                } else if range.end > at + body_h {
                    at = range.end.saturating_sub(body_h);
                }
            }
        }
        at.min(max)
    }
}

/// Measure the menu `view` in a body `area` big and keep the host's reading
/// state for it: how many lines it has, how many show, whether its controls
/// are usable, and where it reads from (the selected row kept in sight).
pub fn apply_menu(view: &MenuView, viewport: &MenuViewport, theme: &Theme, area: Size) {
    let m = menu(view, theme, area);
    let sel = view.selected.unwrap_or(0);
    let resized = viewport.width.replace(m.width) != m.width || viewport.height.get() != m.shown;
    let selection_changed = viewport.selected.replace(Some(sel)) != Some(sel);
    viewport.height.set(m.shown);
    viewport.total.set(m.total);
    viewport.usable.set(m.usable);
    viewport.at.set(m.scroll_for(viewport.at.get(), sel, selection_changed, resized));
}

// --- settings and setup -----------------------------------------------

/// The body a frame leaves the screen or settings: under the top bar (and a
/// source row, when `sources`) and over the status line.
pub fn body_of(frame: Size, sources: bool) -> Size {
    let height = frame.height.saturating_sub(2);
    let height = if sources && height > 1 { height - 1 } else { height };
    Size::new(frame.width, height)
}

/// Settings' panes in `body`: the sections, the section, its detail (side by
/// side from 120 cells, the detail under the section below that).
pub fn settings_panes(body: Size) -> [Size; 3] {
    if body.width >= 120 {
        let rest = body.width - 24;
        // Fill(3) and Fill(2) share what the sections leave.
        let middle = (u32::from(rest) * 3 / 5) as u16;
        [Size::new(24, body.height), Size::new(middle, body.height), Size::new(rest - middle, body.height)]
    } else {
        let nav = 24.min(body.width / 3);
        let content = body.width - nav;
        let middle = body.height / 2;
        [Size::new(nav, body.height), Size::new(content, middle), Size::new(content, body.height - middle)]
    }
}

/// The section pane's inside (a border each side): what a part is asked to
/// fit its section to.
pub fn settings_middle_inside(body: Size) -> Size {
    let [_, middle, _] = settings_panes(body);
    Size::new(middle.width.saturating_sub(2), middle.height.saturating_sub(2))
}

/// Where settings' detail text goes: the pane inside its border and one
/// cell of padding either side.
pub fn settings_detail_text(body: Size) -> Size {
    let [_, _, detail] = settings_panes(body);
    Size::new(detail.width.saturating_sub(4), detail.height.saturating_sub(2))
}

/// How far a block of the detail inside the text room is inset (blocks fill
/// one cell past the width they are given, so they get one less).
pub const SETTINGS_INSET: u16 = 1;
/// The same in setup's sheet, which is not padded again.
pub const SETUP_INSET: u16 = 3;

/// The rows a section's detail takes in a text room `text_width` wide:
/// its rich blocks when it has them (laid out `inset` cells narrower), else
/// its lines.
pub fn detail_rows(view: &SectionView, text_width: u16, inset: u16) -> usize {
    if view.detail_blocks.is_empty() {
        texts_rows(&view.detail, text_width)
    } else {
        blocks_rows(&view.detail_blocks, text_width.saturating_sub(inset).saturating_add(1))
    }
}

/// The rows of lines of rich text, each wrapped at `width`.
pub fn texts_rows(texts: &[Text], width: u16) -> usize {
    texts.iter().map(|t| rows(&t.words(), usize::from(width))).sum()
}

/// Rows of rich blocks in `width` cells. A section is its heading and its
/// blocks two cells in; a document is its laid-out lines (at most `most`,
/// then one for the `…`); a keyed line carries its `[key]` after its words.
fn blocks_rows(blocks: &[Block], width: u16) -> usize {
    let room = usize::from(width);
    blocks
        .iter()
        .map(|block| match block {
            Block::Line { text, key, mnemonic_at, .. } => {
                let words = text.words();
                match key.as_deref().filter(|k| !(mnemonic_at.is_some() && k.chars().count() == 1)) {
                    Some(key) => rows(&format!("{words} [{key}]"), room),
                    None => rows(&words, room),
                }
            }
            Block::Section { blocks, .. } => 1 + blocks_rows(blocks, width.saturating_sub(2)),
            Block::Doc { doc, most, .. } => {
                let lines = crate::ui::rich::plain(doc, width).lines().count();
                most.map_or(lines, |m| if lines > m { m + 1 } else { lines })
            }
            Block::Meter { .. } => 2,
            Block::Wrapped { text, .. } => rows(text, room.saturating_sub(2)),
        })
        .sum()
}

/// Setup's frame inside a body: [`SETUP_MOST_WIDE`] × [`SETUP_MOST_HIGH`] at
/// most, a cell in from each side.
pub const SETUP_MOST_WIDE: u16 = 140;
pub const SETUP_MOST_HIGH: u16 = 40;
/// The rows the card keeps, below the about sheet, where the frame has them.
const CARD_LEAST: u16 = 22;

pub fn setup_frame(body: Size) -> Size {
    let inside = body.width.saturating_sub(2);
    Size::new(inside.min(SETUP_MOST_WIDE), body.height.min(SETUP_MOST_HIGH))
}

/// The text width of setup's about sheet, which measures its own rows first.
pub fn setup_about_width(body: Size) -> u16 {
    setup_frame(body).width.saturating_sub(4)
}

/// Setup's card and the sheet's text room, once the sheet's text takes
/// `about_rows`: the sheet on top (as high as its text, leaving the card at
/// least [`CARD_LEAST`] rows or two thirds), then the card, each under its
/// tab row.
pub fn setup_panes(body: Size, about_rows: u16) -> (Size, Size) {
    let f = setup_frame(body);
    let about = about_rows
        .saturating_add(1)
        .min((f.height / 3).max(f.height.saturating_sub(CARD_LEAST)))
        .max(2);
    let after_first = f.height.saturating_sub(1);
    let sheet = about.min(after_first);
    let card = after_first.saturating_sub(sheet).saturating_sub(2);
    (
        Size::new(f.width, card),
        Size::new(f.width.saturating_sub(4), sheet.saturating_sub(1)),
    )
}

#[cfg(test)]
mod tests;
