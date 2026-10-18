//! The gallery's stories: one page per group of elements, each element in
//! each of its states side by side (selected, without focus, empty, loading,
//! failed, too long), like Storybook. They are what `radar shot` draws for
//! `gallery.<page>`, and what the colour snapshots (`ui/tests.rs`) dump.
//!
//! To add one: write `fn my_story(theme, area, buf, frame) -> u16` that
//! lays its panels out on a [`Board`] and returns the rows it used, and add
//! a line to [`STORIES`]. Say in the panel's title which state it shows.
//! Everything is drawn from the kit's own functions on neutral sample data
//! (shelves, boxes, items); nothing here is a copy of an element.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tui_tree_widget::TreeState;

use super::card::panel;
use super::chips::{after_name, chips};
use super::columns::{Column, Columns};
use super::composer::Composer;
use super::entries::Entries;
use super::gallery::{NOTE, sample_entries, sample_stages, sample_tree, tree_columns};
use super::log::{LogState, LogText, LogView};
use super::markdown;
use super::meter::stages;
use super::rows::{ListRow, RowList, add_row};
use super::scroll::Scroll;
use super::states::{busy, empty, failed, fetching, loading, set_up};
use super::status::{Check, as_you_type, as_you_type_unsure, strip};
use super::table::Table;
use super::tree::{self, BandedTree, Node, TreeLook};
use crate::input::TextInput;
use super::{Mark, clip, fit, wrapped};
use crate::design::theme::Slot;
use crate::plugin::Tone;
use crate::theme::{SLOT_GROUPS, Theme, slot_info};
use crate::widgets;

/// A page of the gallery.
pub struct Story {
    /// The tab's label (short: the tab row has to fit).
    pub name: &'static str,
    /// Draw into `area` (as tall as it needs: the gallery scrolls it);
    /// returns the rows used. `frame` is the spinner's frame.
    pub render: fn(&Theme, Rect, &mut Buffer, usize) -> u16,
}

/// Every story, in tab order. Adding a page is a function and a line here.
pub const STORIES: &[Story] = &[
    Story { name: "Colours", render: colours },
    Story { name: "Tones", render: tones },
    Story { name: "Row states", render: row_states },
    Story { name: "Grid states", render: grid_states },
    Story { name: "Waiting", render: waiting },
    Story { name: "Long text", render: long_text },
    Story { name: "Icons", render: icons },
    Story { name: "Notices", render: notices },
    Story { name: "Detail", render: super::gallery::detail_story },
];

/// Each built-in pack: every icon with its name, in its colour, and a tree
/// row drawn with the pack, so a column after icons is seen lining up.
fn icons(theme: &Theme, area: Rect, buf: &mut Buffer, _frame: usize) -> u16 {
    use super::icon::{self, Icon};
    let mut board = Board::new(theme, buf, area);
    let cell_w = 22usize;
    for pack in ["nerd", "unicode", "emoji"] {
        let cfg = crate::theme::ThemeConfig { icons: pack.into(), ..Default::default() };
        let t = Theme::build(&cfg, theme.tier(), &[]);
        let per = ((area.width as usize).saturating_sub(4) / cell_w).max(1);
        let rows = Icon::ALL.len().div_ceil(per);
        let title = format!("{pack}: {}", icon::built_in_about(pack));
        let cells = board.band(&[(title.as_str(), false)], rows as u16 + 2);
        let lines: Vec<Line<'static>> = Icon::ALL
            .chunks(per)
            .map(|chunk| {
                let mut spans = Vec::new();
                for &i in chunk {
                    let lead = icon::lead(&t, i);
                    let room = cell_w.saturating_sub(lead.width());
                    spans.push(lead);
                    spans.push(Span::styled(fit(i.name(), room), t.s(Slot::TextSecondary)));
                }
                Line::from(spans)
            })
            .collect();
        board.text(&cells[0], lines);
    }
    let mut items: Vec<(String, bool)> = Vec::new();
    for pack in ["off", "unicode", "nerd", "emoji"] {
        items.push((format!("a tree with {pack}"), false));
    }
    let titles: Vec<(&str, bool)> = items.iter().map(|(s, f)| (s.as_str(), *f)).collect();
    let cells = board.band(&titles, 7);
    for (cell, pack) in cells.iter().zip(["off", "unicode", "nerd", "emoji"]) {
        let cfg = crate::theme::ThemeConfig { icons: pack.into(), ..Default::default() };
        let t = Theme::build(&cfg, theme.tier(), &[]);
        let row = |id: &str, kind: Icon, name: &str, state: Icon, word: &str| {
            Node::new(id, vec![icon::lead(&t, kind), Span::raw(name.to_string())])
                .cell("state", Span::from(vec![icon::lead(&t, state), Span::raw(word.to_string())].into_iter().map(|s| s.content.into_owned()).collect::<String>()))
        };
        let nodes = vec![
            Node::new("g", vec![icon::lead(&t, Icon::Group), Span::raw("north-wing")]).children([
                row("1", Icon::Change, "sort the atlas box", Icon::RunPassed, "ready"),
                row("2", Icon::Draft, "label the drawer", Icon::RunFailed, "fix"),
                row("3", Icon::Bug, "a loose map", Icon::RunRunning, "busy"),
            ]),
        ];
        let cols = Columns::new([Column::new("state", 10).icon(Icon::Pipeline)]);
        let mut state = TreeState::default();
        tree::open_new(&mut state, &nodes, &mut HashSet::new());
        let inner = widgets::padded(cell.inner);
        for _ in 0..2 {
            BandedTree::new(&nodes, &cols, TreeLook::new(&t))
                .header(vec![Span::raw("ITEMS")])
                .render(&t, inner, board.buf, &mut state);
        }
    }
    board.used()
}

/// One panel of a [`Board`]: its border and what is inside.
#[derive(Clone, Copy)]
pub struct Cell {
    pub frame: Rect,
    pub inner: Rect,
}

/// Panels in bands down a page: side by side where the page is wide,
/// stacked where it is not, each titled with the state it shows.
pub struct Board<'a> {
    pub theme: &'a Theme,
    pub buf: &'a mut Buffer,
    area: Rect,
    y: u16,
}

impl<'a> Board<'a> {
    pub fn new(theme: &'a Theme, buf: &'a mut Buffer, area: Rect) -> Self {
        Self { theme, buf, area, y: 0 }
    }

    pub fn wide(&self) -> bool {
        self.area.width >= 100
    }

    /// The rows used so far.
    pub fn used(&self) -> u16 {
        self.y
    }

    /// A heading line over what follows.
    pub fn heading(&mut self, text: &str) {
        let at = Rect { y: self.area.y + self.y, height: 1, ..self.area };
        if at.bottom() <= self.area.bottom() {
            self.buf.set_line(at.x, at.y, &widgets::heading(self.theme, text), at.width);
        }
        self.y += 1;
    }

    /// A band of panels `h` rows tall (borders included), one per item
    /// `(title, focused)`: side by side when wide, else one under another.
    pub fn band(&mut self, items: &[(&str, bool)], h: u16) -> Vec<Cell> {
        let n = items.len() as u16;
        let rects: Vec<Rect> = if self.wide() {
            let top = Rect { y: self.area.y + self.y, height: h, ..self.area };
            self.y += h;
            Layout::horizontal(vec![Constraint::Fill(1); items.len()]).split(top).to_vec()
        } else {
            let rects = (0..n)
                .map(|i| Rect { y: self.area.y + self.y + i * h, height: h, ..self.area })
                .collect();
            self.y += n * h;
            rects
        };
        items
            .iter()
            .zip(rects)
            .map(|((title, focused), r)| {
                let frame = r.intersection(self.area);
                let inner = panel(self.theme, frame, self.buf, title, *focused);
                Cell { frame, inner }
            })
            .collect()
    }

    /// Lines in a panel's padded inside, one to a row: a line too long is
    /// cut at the edge, never wrapped (a story's height is its rows).
    pub fn text(&mut self, cell: &Cell, lines: Vec<Line<'_>>) {
        lines_in(self.buf, widgets::padded(cell.inner), &lines);
    }
}

/// `lines` one to a row of `area`, cut at its right edge.
fn lines_in(buf: &mut Buffer, area: Rect, lines: &[Line<'_>]) {
    for (i, line) in lines.iter().enumerate().take(area.height as usize) {
        buf.set_line(area.x, area.y + i as u16, line, area.width);
    }
}

/// A line of `label` in muted text, then `rest`.
fn labelled<'a>(theme: &Theme, label: &str, width: usize, rest: Vec<Span<'a>>) -> Line<'a> {
    widgets::kv(theme, label, width, rest)
}

// --- Colours -------------------------------------------------------------

fn colours(theme: &Theme, area: Rect, buf: &mut Buffer, _frame: usize) -> u16 {
    let mut board = Board::new(theme, buf, area);
    // Every slot, by group, with the colour it has and what it is for.
    let mut lines: Vec<Line> = Vec::new();
    let names = theme.slot_names();
    for group in SLOT_GROUPS {
        let in_group: Vec<&String> = names.iter().filter(|n| slot_info(n).group == group).collect();
        if in_group.is_empty() {
            continue;
        }
        lines.push(widgets::heading(theme, group));
        for name in in_group {
            let swatch = theme.swatch(name).map_or(Style::new(), |c| Style::new().fg(c));
            lines.push(Line::from(vec![
                Span::styled("██████ ", swatch),
                Span::styled(format!("{name:<17}"), theme.s(Slot::TextPrimary)),
                Span::styled(format!("{}  ", theme.hex(name)), theme.s(Slot::DataNumber)),
                Span::styled(slot_info(name).meaning, theme.s(Slot::TextMuted)),
            ]));
        }
    }
    let cols = if board.wide() { 2 } else { 1 };
    let per = lines.len().div_ceil(cols);
    let cell = board.band(&[("slots", false)], per as u16 + 2).remove(0);
    for (i, chunk) in lines.chunks(per).enumerate() {
        let width = cell.inner.width / cols as u16;
        let part = Rect { x: cell.inner.x + 1 + i as u16 * width, width: width.saturating_sub(2), ..cell.inner };
        lines_in(board.buf, part, chunk);
    }

    // Each text colour on each background it is drawn on.
    let on = [Slot::BgBase, Slot::BgSurface, Slot::BgOverlay, Slot::BgSelection];
    let texts = [
        Slot::TextPrimary,
        Slot::TextSecondary,
        Slot::TextMuted,
        Slot::TextEmphasis,
        Slot::AccentPrimary,
        Slot::AccentMuted,
        Slot::AccentSecondary,
        Slot::AccentTertiary,
        Slot::StatusSuccess,
        Slot::StatusWarning,
        Slot::StatusError,
        Slot::StatusInfo,
        Slot::DataNumber,
    ];
    let mut lines = vec![Line::from(
        std::iter::once(Span::styled(format!("{:<18}", ""), theme.s(Slot::TextMuted)))
            .chain(on.iter().map(|b| {
                Span::styled(format!("{:<10}", b.token_name().trim_start_matches("bg.")), theme.s(Slot::TextMuted))
            }))
            .collect::<Vec<_>>(),
    )];
    for t in texts {
        let mut spans = vec![Span::styled(format!("{:<18}", t.token_name()), theme.s(Slot::TextMuted))];
        for b in on {
            spans.push(Span::styled(" Sample   ", theme.s(t).patch(theme.bg(b))));
        }
        lines.push(Line::from(spans));
    }
    let cell = board.band(&[("text on each background", false)], lines.len() as u16 + 2).remove(0);
    board.text(&cell, lines);
    board.used()
}

// --- Tones and marks -----------------------------------------------------

const TONES: [(&str, Tone); 7] = [
    ("plain", Tone::Plain),
    ("muted", Tone::Muted),
    ("accent", Tone::Accent),
    ("info", Tone::Info),
    ("good", Tone::Good),
    ("warn", Tone::Warn),
    ("bad", Tone::Bad),
];

const MARKS: [(&str, Mark); 7] = [
    ("done", Mark::Done),
    ("failed", Mark::Failed),
    ("running", Mark::Running),
    ("waiting", Mark::Waiting),
    ("manual", Mark::Manual),
    ("stopped", Mark::Stopped),
    ("skipped", Mark::Skipped),
];

fn tones(theme: &Theme, area: Rect, buf: &mut Buffer, _frame: usize) -> u16 {
    let mut board = Board::new(theme, buf, area);
    let cells = board.band(&[("tones: text, chip", false), ("marks: symbol, tone", false)], 9);
    let lines = TONES
        .iter()
        .map(|(name, tone)| {
            labelled(
                theme,
                name,
                8,
                vec![
                    Span::styled("sorting 12 items  ", widgets::tone(theme, *tone)),
                    widgets::chip(theme, name, *tone),
                    Span::raw("  "),
                    Span::styled("bold", widgets::tone(theme, *tone).add_modifier(Modifier::BOLD)),
                ],
            )
        })
        .collect();
    board.text(&cells[0], lines);
    let lines = MARKS
        .iter()
        .map(|(name, mark)| {
            labelled(
                theme,
                name,
                9,
                vec![
                    Span::styled(format!("{} ", mark.symbol()), widgets::tone(theme, mark.tone())),
                    Span::styled(format!("box 17 {name}"), widgets::tone(theme, mark.tone())),
                ],
            )
        })
        .collect();
    board.text(&cells[1], lines);

    let cells = board.band(&[("checks", false), ("as you type, status strip", false)], 9);
    let checks = [
        ("not yet", Check::NotYet),
        ("running", Check::Running),
        ("passed", Check::Passed { said: "signed in as alice".into(), ago: Some("2m".into()) }),
        ("failed", Check::Failed { said: "the server said 401".into(), ago: None }),
    ];
    let mut lines = Vec::new();
    for (name, check) in &checks {
        let mut line = check.line(theme, "not checked yet · c checks it", "asking the server…");
        line.spans.insert(0, Span::styled(format!("{name:<8}"), theme.s(Slot::TextMuted)));
        lines.push(line);
    }
    lines.push(Line::default());
    let mark = |c: &Check| c.mark(theme).unwrap_or_else(|| Span::raw("(no mark)"));
    lines.push(Line::from(vec![
        Span::styled("marks   ", theme.s(Slot::TextMuted)),
        mark(&checks[1].1),
        Span::raw("  "),
        mark(&checks[2].1),
        Span::raw("  "),
        mark(&checks[3].1),
        Span::raw("  "),
        mark(&checks[0].1),
    ]));
    board.text(&cells[0], lines);
    let lines = vec![
        as_you_type(theme, Ok("a link to a shelf")),
        as_you_type(theme, Err("not a link or a path")),
        as_you_type_unsure(theme, "shelf-9 not found here"),
        Line::default(),
        strip(
            theme,
            &[("4 open".into(), Tone::Plain), ("1 needs you".into(), Tone::Warn), ("1 ready".into(), Tone::Good)],
        ),
        strip(theme, &[("nothing waiting".into(), Tone::Muted)]),
        Line::default(),
        Line::from(chips(theme, &[("needs you", Tone::Warn), ("ready to file", Tone::Good), ("lost", Tone::Bad)])),
    ];
    board.text(&cells[1], lines);

    let cells = board.band(&[("stage bar: each mark in a run", false), ("key hints", false)], 7);
    let lines = stages(&sample_stages(theme), cells[0].inner.width.saturating_sub(2));
    board.text(&cells[0], lines);
    let hints: Vec<Span> = [("↑↓", "move"), ("space", "fold"), ("⏎", "open"), ("x", "extra actions"), ("q", "quit")]
        .iter()
        .flat_map(|(k, l)| {
            let mut h = widgets::key_hint(theme, k, l, crate::mnemonic::at(k, l), crate::mnemonic::at(k, l).is_some() && crate::mnemonic::shifted(k));
            h.push(Span::raw("   "));
            h
        })
        .collect();
    board.text(&cells[1], vec![Line::from(hints)]);
    board.used()
}

// --- Rows ----------------------------------------------------------------

fn sample_rows(theme: &Theme) -> Vec<ListRow> {
    let say = |text: &str, slot: Slot| Line::from(Span::styled(format!(" {text}"), theme.s(slot)));
    vec![
        ListRow::heading(widgets::heading(theme, "a heading: the cursor skips it")),
        ListRow::new(say("a plain row", Slot::TextPrimary)),
        ListRow::new(say("a row with a word at the right", Slot::TextPrimary))
            .right(Span::styled("hidden", theme.s(Slot::TextMuted))),
        ListRow::new(say("the row the cursor is on", Slot::TextPrimary))
            .right(Span::styled("✓", theme.s(Slot::StatusSuccess))),
        ListRow::new(say(
            "a row whose text is far too long for the room its word leaves, so it is cut",
            Slot::TextPrimary,
        ))
        .right(Span::styled("checking", theme.s(Slot::StatusInfo))),
        ListRow::new(add_row(theme, "add a shelf", "paste a link to a shelf, or its path", 60)),
        ListRow::new(say("日本語の長い行 with wide characters in it, cut at the edge", Slot::TextSecondary)),
    ]
}

fn row_states(theme: &Theme, area: Rect, buf: &mut Buffer, _frame: usize) -> u16 {
    let mut board = Board::new(theme, buf, area);
    let scroll = Scroll::new();
    let cells = board.band(&[("list, focused (cursor on the fourth row)", true), ("list, without focus", false)], 11);
    for (cell, focused) in cells.iter().zip([true, false]) {
        RowList::new(sample_rows(theme)).cursor(3).focused(focused).render(
            theme,
            cell.frame,
            cell.inner,
            board.buf,
            &scroll,
        );
    }

    let cells = board.band(&[("row styles: the four a row can have", false), ("list, empty", false)], 7);
    let inner = cells[0].inner;
    for (i, (label, focused, selected)) in [
        ("selected, pane focused", true, true),
        ("selected, pane without focus", false, true),
        ("not selected, pane focused", true, false),
        ("not selected, no focus", false, false),
    ]
    .iter()
    .enumerate()
    {
        widgets::row(
            board.buf,
            inner,
            inner.y + i as u16,
            widgets::row_style(theme, *focused, *selected),
            Line::from(Span::raw(*label)),
            Some(Span::raw("word")),
        );
    }
    empty(theme, cells[1].inner, board.buf, "No shelves yet. Press , for settings, then add one under shelves.");

    let cells = board.band(&[("entries, focused (cursor on the first)", true), ("entries, without focus", false)], 12);
    for (cell, focused) in cells.iter().zip([true, false]) {
        Entries::new(sample_entries(theme)).cursor(0).focused(focused).render(
            theme,
            cell.frame,
            cell.inner,
            board.buf,
            &Scroll::new(),
        );
    }
    let cells = board.band(&[("entries, empty", false), ("log, empty", false)], 5);
    Entries::new(vec![]).empty("Nothing failed.").render(theme, cells[0].frame, cells[0].inner, board.buf, &Scroll::new());
    LogView {
        text: &LogText::default(),
        state: &LogState::default(),
        focused: false,
        empty: "Nothing written yet.",
    }
    .render(theme, cells[1].frame, cells[1].inner, board.buf);
    board.used()
}

// --- Table and tree --------------------------------------------------------

fn grid_columns(width: u16) -> Columns {
    Columns::new([
        Column::flex("name", 14),
        Column::new("state", 10).keep(5),
        Column::new("owner", 12).keep(2),
        Column::new("size", 8).keep(1),
    ])
    .fit(width.saturating_sub(2) as usize, 0)
}

fn grid_rows(theme: &Theme) -> Vec<Vec<(String, Span<'static>)>> {
    let row = |name: &str, state: (&str, Tone), owner: &str, size: &str| {
        vec![
            ("name".to_string(), Span::raw(name.to_string())),
            ("state".to_string(), Span::styled(state.0.to_string(), widgets::tone(theme, state.1))),
            ("owner".to_string(), Span::styled(owner.to_string(), theme.s(Slot::TextSecondary))),
            ("size".to_string(), Span::styled(size.to_string(), theme.s(Slot::DataNumber))),
        ]
    };
    vec![
        row("sort the atlas box", ("● sorting", Tone::Info), "Alice E.", "42 kB"),
        row("label the map drawer", ("✓ filed", Tone::Good), "you", "3 kB"),
        row(
            "mend the spine of the old ledger that has been on the shelf since before the move",
            ("✗ lost", Tone::Bad),
            "Bob",
            "17 kB",
        ),
        row("count the postcards", ("◷ waiting", Tone::Muted), "Carol D.", "9 kB"),
    ]
}

fn grid_states(theme: &Theme, area: Rect, buf: &mut Buffer, _frame: usize) -> u16 {
    let mut board = Board::new(theme, buf, area);
    let cells = board.band(&[("table, focused (cursor on row 3)", true), ("table, without focus", false)], 8);
    for (cell, focused) in cells.iter().zip([true, false]) {
        let cols = grid_columns(cell.inner.width);
        Table::new(&cols, grid_rows(theme)).cursor(2).focused(focused).render(
            theme,
            cell.frame,
            cell.inner,
            board.buf,
            &Scroll::new(),
        );
    }
    let cells = board.band(&[("table, empty", false), ("table, narrow: columns dropped", false)], 6);
    let cols = grid_columns(cells[0].inner.width);
    Table::new(&cols, vec![]).empty("No items match this view.").render(
        theme,
        cells[0].frame,
        cells[0].inner,
        board.buf,
        &Scroll::new(),
    );
    let narrow = Columns::new([
        Column::flex("name", 14),
        Column::new("state", 10).keep(5),
        Column::new("owner", 12).keep(2),
        Column::new("size", 8).keep(1),
    ])
    .fit(34, 0);
    let inner = Rect { width: 36.min(cells[1].inner.width), ..cells[1].inner };
    Table::new(&narrow, grid_rows(theme)).cursor(0).render(theme, cells[1].frame, inner, board.buf, &Scroll::new());

    // A tree row that takes text: the add row being typed into.
    let cells = board.band(&[("tree, typing into a row", true), ("tree, groups folded (⋯)", false)], 8);
    let plain = theme.s(Slot::TextPrimary);
    let nodes = vec![
        Node::new("add", vec![Span::styled(" + add a group", plain)]).input_at(3),
        Node::new("g", vec![Span::styled("● ab ", plain), Span::styled("north-wing", theme.bold(Slot::TextEmphasis))])
            .stripe(theme.spec("glow.purple"))
            .children([
                Node::new("g/add", vec![Span::styled("+ add a box", plain)]),
                Node::new("g/b", vec![Span::styled("a box", plain)]),
            ]),
    ];
    let mut state = TreeState::default();
    state.open(vec!["g".to_string()]);
    state.select(vec!["add".to_string()]);
    let input = TextInput::new("atlases");
    let cursor = std::cell::Cell::new(None);
    BandedTree::new(&nodes, &Columns::default(), TreeLook::new(theme))
        .input(&input, &cursor)
        .render(theme, cells[0].inner, board.buf, &mut state);
    let nodes = sample_tree(theme);
    let mut state = TreeState::default();
    state.select(vec!["north-wing".into()]);
    let cols = tree_columns().fit(cells[1].inner.width.saturating_sub(1) as usize, 24);
    BandedTree::new(&nodes, &cols, TreeLook::new(theme)).focused(false).render(theme, cells[1].inner, board.buf, &mut state);

    let cells = board.band(&[("tree, focused (cursor on 198)", true), ("tree, without focus", false)], 14);
    for (cell, focused) in cells.iter().zip([true, false]) {
        let nodes = sample_tree(theme);
        let mut state = TreeState::default();
        tree::open_new(&mut state, &nodes, &mut HashSet::new());
        state.select(vec!["north-wing".into(), "maps".into(), "198".into()]);
        let cols = tree_columns().fit(cell.inner.width.saturating_sub(1) as usize, 24);
        BandedTree::new(&nodes, &cols, TreeLook::new(theme))
            .header(vec![Span::styled(" HELD BY ", theme.bold(Slot::TextMuted))])
            .focused(focused)
            .render(theme, cell.inner, board.buf, &mut state);
    }
    board.used()
}

// --- Waiting, empty and failing ---------------------------------------------

fn waiting(theme: &Theme, area: Rect, buf: &mut Buffer, frame: usize) -> u16 {
    let mut board = Board::new(theme, buf, area);
    let cells = board.band(&[("loading, and what fetching says", false), ("failed", false)], 9);
    let said = |label: &str, line: Option<Line<'static>>| {
        let mut spans = vec![Span::styled(format!("{label:<22}"), theme.s(Slot::TextMuted))];
        // A line's own style (`Line::styled`) goes onto its spans.
        spans.extend(match line {
            Some(l) => l.spans.into_iter().map(|s| Span::styled(s.content, l.style.patch(s.style))).collect(),
            None => vec![Span::styled("(nothing)", theme.s(Slot::TextMuted))],
        });
        Line::from(spans)
    };
    let lines = vec![
        said("loading", Some(loading(theme))),
        said("busy", Some(busy(theme, frame, "asking the server…"))),
        said("fetching, nothing yet", fetching(theme, true, false, None)),
        said("fetching, has rows", fetching(theme, true, true, None)),
        said("fetching, error", fetching(theme, false, false, Some("the server did not answer"))),
        said("fetched", fetching(theme, false, true, None)),
        Line::from(vec![
            Span::styled(format!("{:<22}", "spinner frames"), theme.s(Slot::TextMuted)),
            Span::raw((0..6).map(|i| format!("{} ", super::states::spinner(theme, i).content)).collect::<String>()),
        ]),
    ];
    board.text(&cells[0], lines);
    let lines = vec![
        failed(theme, "the server did not answer"),
        failed(theme, "401: the token was refused, so nothing could be listed and this line is cut"),
        Line::default(),
        Check::Failed { said: "the server said 401".into(), ago: Some("2m".into()) }.line(theme, "", ""),
        as_you_type(theme, Err("not a link or a path")),
    ];
    board.text(&cells[1], lines);

    let cells = board.band(&[("empty", false), ("set up (a part that is not)", false)], 9);
    empty(theme, cells[0].inner, board.buf, "No shelves yet. Press , for settings, then add one under shelves.");
    set_up(
        theme,
        cells[1].inner,
        board.buf,
        "Shelves",
        "It needs a server and a token.",
        Some("Press ⏎ to set it up in settings."),
    );
    board.used()
}

// --- Long text ----------------------------------------------------------------

fn long_text(theme: &Theme, area: Rect, buf: &mut Buffer, _frame: usize) -> u16 {
    let mut board = Board::new(theme, buf, area);
    let cells = board.band(&[("cut with … (fit)", false), ("wrapped behind a prefix", false)], 11);
    let text = "move the globes to the top shelf once the new brackets are up";
    let mut lines = Vec::new();
    for w in [60, 40, 24, 12, 4] {
        lines.push(Line::from(vec![
            Span::styled(format!("{w:>2} "), theme.s(Slot::TextMuted)),
            Span::styled(fit(text, w), theme.s(Slot::TextPrimary)),
            Span::styled("|", theme.s(Slot::TextMuted)),
        ]));
    }
    // Styled spans cut keep their styles (clip).
    let spans = vec![
        Span::styled("#104 ", theme.s(Slot::AccentSecondary)),
        Span::styled("sort the atlas box by decade", theme.bold(Slot::TextPrimary)),
        Span::styled("  Alice E.", theme.s(Slot::TextMuted)),
    ];
    for w in [40, 20] {
        let mut line = vec![Span::styled(format!("{w:>2} "), theme.s(Slot::TextMuted))];
        line.extend(clip(spans.clone(), w));
        lines.push(Line::from(line));
    }
    board.text(&cells[0], lines);
    let width = cells[1].inner.width.saturating_sub(2).min(46);
    let bar = Span::styled("│ ", theme.s(Slot::AccentMuted));
    let mut lines = wrapped(NOTE, width, bar, theme.s(Slot::TextPrimary));
    lines.truncate(9);
    board.text(&cells[1], lines);

    let cells = board.band(&[("chips give way to the title", false), ("columns dropped as the room shrinks", false)], 9);
    let name = |title: &str| {
        vec![
            Span::styled("#104 ", theme.s(Slot::AccentSecondary)),
            Span::styled(title.to_string(), theme.s(Slot::TextPrimary)),
        ]
    };
    let chip_set = chips(theme, &[("needs you", Tone::Warn), ("ready", Tone::Good)]);
    let mut lines = Vec::new();
    for w in [60, 40, 28, 20] {
        let mut line = vec![Span::styled(format!("{w:>2} "), theme.s(Slot::TextMuted))];
        line.extend(after_name(name("sort the atlas box by decade, then label every drawer"), 1, chip_set.clone(), w));
        lines.push(Line::from(line));
    }
    board.text(&cells[0], lines);
    let cols = Columns::new([
        Column::flex("name", 14),
        Column::new("state", 10).keep(5),
        Column::new("owner", 12).keep(2),
        Column::new("size", 8).keep(1),
        Column::new("updated", 9).keep(4),
    ]);
    let lines = [70, 50, 40, 30, 20]
        .iter()
        .map(|w| {
            let fitted = cols.fit(*w, 0);
            let kept: Vec<String> = fitted.iter().map(|c| c.heading.clone()).collect();
            Line::from(vec![
                Span::styled(format!("{w:>2} "), theme.s(Slot::TextMuted)),
                Span::styled(kept.join(" · "), theme.s(Slot::TextPrimary)),
            ])
        })
        .collect();
    board.text(&cells[1], lines);

    let cells = board.band(&[("markdown in a narrow card (30 wide)", false), ("markdown in a wide card (60 wide)", false)], 18);
    let sample = "A note with **bold**, a [link](https://example.com), `code`, and a long unbroken run of words that has to wrap.\n\n- one\n- two with a good deal more text than fits\n\n> a quote that is long enough to wrap at thirty\n\n```\na code line that is longer than the card is wide\n```";
    for (cell, w) in cells.iter().zip([30, 60]) {
        // The card's text is its width less the padding on each side.
        let lines = markdown::render(theme, sample, w - 2).lines;
        let at = Rect { width: w.min(cell.inner.width), ..widgets::padded(cell.inner) };
        lines_in(board.buf, at, &lines);
    }

    // Text typed for a write: lines kept above the one being typed; a
    // title on one line.
    let cells = board.band(&[("composer, lines kept above the one typed", false), ("composer, one line (a title)", false)], 9);
    let cursor = std::cell::Cell::new(None);
    let many = Composer::new("Label every drawer\nby decade,\nthen the boxes");
    many.draw_foot(theme, "✎ note on box 104", widgets::padded(cells[0].inner), board.buf, &cursor);
    let one = Composer::one_line("sort the atlas box");
    one.draw_foot(theme, "new item on shelf 3 · its title", widgets::padded(cells[1].inner), board.buf, &cursor);
    board.used()
}

/// The same bounded occurrence list with moving and static ground, then the
/// inert reader that keeps recovery guidance after its transient row expires.
fn notices(theme: &Theme, area: Rect, buf: &mut Buffer, _frame: usize) -> u16 {
    use crate::view::chrome::{NoticeHistoryView, NoticeTone, NoticeView};
    let entries = vec![
        NoticeView {
            id: 1,
            tone: NoticeTone::Info,
            icon: Some(super::icon::Icon::Copy),
            text: "Copied the Watch link".into(),
            created_ms: 0,
            lifetime_ms: 5000,
        },
        NoticeView {
            id: 2,
            tone: NoticeTone::Error,
            icon: None,
            text: "The Watch request failed. Try again when the connection returns.".into(),
            created_ms: 1000,
            lifetime_ms: 10_000,
        },
    ];
    let mut board = Board::new(theme, buf, area);
    let cells = board.band(
        &[
            ("timed ground · older notice dimmed", false),
            ("motion off · same expiry", false),
        ],
        7,
    );
    for (cell, moving) in cells.into_iter().zip([true, false]) {
        let stack = super::notices::layout(&entries, 0, cell.inner, 2500, moving);
        super::notices::paint(&stack, theme, board.buf);
    }
    let cells = board.band(&[("recent notices · session only", false)], 18);
    let history = NoticeHistoryView {
        entries: entries.clone(),
        detail: entries.last().cloned(),
        omitted: 3,
        selected: 0,
        scroll: 0,
        detail_scroll: 0,
    };
    super::notices::history(theme, &history, cells[0].inner, board.buf);
    board.used()
}
