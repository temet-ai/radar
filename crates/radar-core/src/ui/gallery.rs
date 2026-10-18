//! The gallery: every element of the kit on sample data, a tab per group,
//! so elements are built and looked at without any part. Run it with
//! `cargo run -p radar-core --example gallery`; its snapshots are in
//! `ui/tests.rs`. The samples are neutral (shelves, boxes, items): no tool.

use std::cell::RefCell;
use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tui_tree_widget::TreeState;

use super::card::{card, panel};
use super::chips::chips;
use super::columns::{Column, Columns};
use super::connection;
use super::diff::{self, DiffLine, DiffState, DiffView};
use super::entries::{Entries, Entry};
use super::markdown;
use super::rich;
use super::log::{LogLine, LogState, LogText, LogView, Section};
use super::meter::{Stage, ratio, spark, sparkline, stages};
use super::motion_demo::MotionDemo;
use super::page::PageFrame;
use super::rows::{ListRow, Move, RowList, add_row, kv, labelled, select};
use super::scroll::Scroll;
use super::states::{busy, empty, failed, loading, set_up};
use super::status::{Check, as_you_type, strip};
use super::stories::{self, STORIES};
use super::table::Table;
use super::tree::{self, BandedTree, Node, TreeLook};
use super::{Mark, wrapped};
use crate::design::theme::Slot;
use crate::plugin::Tone;
use crate::theme::Theme;
use crate::widgets;

/// The motion tab, the last of [`TABS`]: its keys go to [`MotionDemo`].
pub const MOTION_TAB: usize = 8;

/// The first tabs, each a live element. After them come the stories
/// ([`STORIES`]): every element in each of its states.
pub const TABS: [&str; 9] = [
    "Tree",
    "Table",
    "Card",
    "Meters",
    "Rows & states",
    "Entries & log",
    "Markdown",
    "Diff",
    "Motion",
];

/// Every tab's label: [`TABS`], then the stories.
pub fn tab_names() -> Vec<&'static str> {
    TABS.iter().copied().chain(STORIES.iter().map(|s| s.name)).collect()
}

/// The labels that fit in `width` with the tab `on` showing: tabs before it
/// drop off the left, as many as it takes. Returns them and its place.
fn tabs_in(names: &[&str], on: usize, width: u16) -> (Vec<String>, usize) {
    let width_of = |from: usize| -> usize {
        // Each shows as `[label]` or `label` and two spaces between.
        names[from..=on].iter().map(|n| super::width(n) + 4).sum()
    };
    let mut from = 0;
    while from < on && width_of(from) > width as usize {
        from += 1;
    }
    (names[from..].iter().map(|n| n.to_string()).collect(), on - from)
}

/// The gallery's state: the tab showing, and each element's own.
#[derive(Default)]
pub struct Gallery {
    pub tab: usize,
    tree: RefCell<TreeState<String>>,
    seen: RefCell<HashSet<Vec<String>>>,
    table_row: usize,
    table: Scroll,
    card: Scroll,
    markdown: Scroll,
    /// The story showing, scrolled.
    story: Scroll,
    list_row: usize,
    list: Scroll,
    entries: Scroll,
    log: LogState,
    log_text: LogText,
    diff: DiffState,
    diff_lines: Vec<DiffLine>,
    /// The motion tab: its sample rows and the cues its keys made.
    pub motion: MotionDemo,
    /// Spinner frames, one per tick.
    pub frame: usize,
}

impl Gallery {
    pub fn new() -> Self {
        let g = Self {
            log_text: sample_log(),
            diff_lines: diff::parse(SAMPLE_DIFF),
            ..Self::default()
        };
        g.diff.set(DIFF_NOTED);
        g.log.search(&g.log_text, "label");
        g
    }

    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// A key; false when it asks to quit (`q`, `esc`).
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let n = TABS.len() + STORIES.len();
        let tab = self.tab;
        let moved = match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return false,
            KeyCode::Char(']') | KeyCode::Tab => {
                self.tab = (self.tab + 1) % n;
                true
            }
            KeyCode::Char('[') | KeyCode::BackTab => {
                self.tab = (self.tab + n - 1) % n;
                true
            }
            KeyCode::Char(c @ '1'..='9') if (c as usize - '1' as usize) < n => {
                self.tab = c as usize - '1' as usize;
                true
            }
            _ => false,
        };
        if moved {
            if self.tab != tab {
                self.story.top();
            }
        } else if self.tab == MOTION_TAB {
            self.motion.key(&key);
        } else {
            match self.tab {
                t if t >= TABS.len() => {
                    self.story.key(&key);
                }
                0 => {
                    tree::key(&mut self.tree.borrow_mut(), &key);
                }
                1 => {
                    if let Some(down) = widgets::vertical(&key) {
                        self.table_row = widgets::step(self.table_row, TABLE_ROWS, down);
                    }
                }
                2 => {
                    self.card.key(&key);
                }
                4 => {
                    if let Some(mv) = Move::from_key(&key, self.list.height()) {
                        self.list_row = select(SETTINGS_ROWS, self.list_row, mv, |i| i != HEADING_AT);
                    }
                }
                5 => {
                    self.log.key(&self.log_text, &key);
                }
                6 => {
                    self.markdown.key(&key);
                }
                7 => match key.code {
                    KeyCode::Char('}') => {
                        self.diff.next_hunk(&self.diff_lines);
                    }
                    KeyCode::Char('{') => {
                        self.diff.previous_hunk(&self.diff_lines);
                    }
                    _ => {
                        self.diff.key(&key, self.diff_lines.len());
                    }
                },
                _ => {}
            }
        }
        if self.tab != tab {
            self.motion.cue(crate::motion::Cue::Screen);
        }
        true
    }

    /// What motion should show since the last call (a tab switch, the
    /// motion tab's keys), for the gallery's engine.
    pub fn take_cues(&mut self) -> Vec<crate::motion::Cue> {
        self.motion.take_cues()
    }

    /// Draw the whole gallery; `note` goes at the right of the title (the
    /// colour level, say).
    pub fn render(&self, theme: &Theme, area: Rect, buf: &mut Buffer, note: &str) {
        crate::motion::begin();
        buf.set_style(area, theme.bg(Slot::BgBase));
        let [main, keys] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
        let names = tab_names();
        let (tabs, tab) = tabs_in(&names, self.tab, area.width.saturating_sub(2));
        let frame = PageFrame {
            crumbs: vec![
                Span::styled("Gallery", theme.s(Slot::TextSecondary)),
                Span::styled(names[self.tab].to_string(), theme.s(Slot::TextSecondary)),
            ],
            title: Span::styled("The kit: every element on sample data", theme.bold(Slot::TextEmphasis)),
            details: vec![Span::styled(note.to_string(), theme.s(Slot::TextMuted))],
            under: None,
            tabs: tabs.into_iter().map(Into::into).collect(),
            tab,
            focused: false,
        };
        let body = frame.render(theme, main, buf).body;
        crate::motion::region(crate::motion::Region::Body, body);
        match self.tab {
            0 => self.tree_tab(theme, body, buf),
            1 => self.table_tab(theme, body, buf),
            2 => self.card_tab(theme, body, buf),
            3 => self.meters_tab(theme, body, buf),
            4 => self.rows_tab(theme, body, buf),
            5 => self.log_tab(theme, body, buf),
            6 => self.markdown_tab(theme, body, buf),
            7 => self.diff_tab(theme, body, buf),
            MOTION_TAB => self.motion.render(theme, body, buf),
            n => self.story_tab(&STORIES[n - TABS.len()], theme, body, buf),
        }
        let hints: &[(&str, &str)] = if self.tab == MOTION_TAB {
            &[
                ("[ ]", "tabs"),
                ("s m g p n t l", "effects"),
                ("f", "level"),
                ("c", "colours"),
                ("q", "quit"),
            ]
        } else {
            &[
                ("[ ]", "tabs"),
                ("j k", "move"),
                ("space", "fold"),
                ("PgUp/PgDn", "scroll"),
                ("c", "colours"),
                ("q", "quit"),
            ]
        };
        let hints: Vec<Span> = hints
            .iter()
        .flat_map(|(k, l)| {
            let mut h = widgets::key_hint(theme, k, l);
            h.push(Span::raw("   "));
            h
        })
        .collect();
        buf.set_line(keys.x + 1, keys.y, &Line::from(hints), keys.width.saturating_sub(1));
    }

    fn tree_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let inner = panel(theme, area, buf, "shelves", true);
        let nodes = sample_tree(theme);
        let width = inner.width.saturating_sub(1) as usize;
        let cols = tree_columns().fit(width, 40);
        let mut state = self.tree.borrow_mut();
        tree::open_new(&mut state, &nodes, &mut self.seen.borrow_mut());
        BandedTree::new(&nodes, &cols, TreeLook::new(theme))
            .header(vec![
                Span::styled(" HELD BY ", theme.bold(Slot::TextMuted)),
                Span::styled("everyone", theme.s(Slot::TextSecondary)),
            ])
            .render(theme, inner, buf, &mut state);
    }

    fn table_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let inner = panel(theme, area, buf, "items", true);
        let cols = Columns::new([
            Column::flex("name", 20),
            Column::new("state", 10).keep(5),
            Column::new("owner", 12).keep(2),
            Column::new("size", 8).keep(1),
            Column::new("updated", 9).keep(4),
        ])
        .fit(inner.width.saturating_sub(2) as usize, 0);
        let rows = (0..TABLE_ROWS)
            .map(|i| {
                let (word, tone) = STATES[i % STATES.len()];
                vec![
                    ("name".into(), Span::raw(format!("#{} {}", 100 + i, TITLES[i % TITLES.len()]))),
                    ("state".into(), Span::styled(word, widgets::tone(theme, tone))),
                    ("owner".into(), Span::styled(OWNERS[i % OWNERS.len()], theme.s(Slot::TextSecondary))),
                    ("size".into(), Span::styled(format!("{} kB", 3 + i * 7 % 40), theme.s(Slot::DataNumber))),
                    ("updated".into(), Span::styled(format!("{}h", 1 + i % 9), theme.s(Slot::TextMuted))),
                ]
            })
            .collect();
        Table::new(&cols, rows)
            .cursor(self.table_row)
            .render(theme, area, inner, buf, &self.table);
    }

    fn card_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let width = area.width.saturating_sub(4);
        let mut lines = vec![
            Line::from(vec![
                Span::styled("A ", theme.spec("glow.purple bold")),
                Span::styled("north-wing", theme.s(Slot::TextSecondary)),
                Span::styled("  #104", theme.s(Slot::AccentSecondary)),
            ]),
            Line::styled("Sort the atlas box by decade", theme.bold(Slot::TextEmphasis)),
            Line::from(vec![
                Span::styled("Alice E.", theme.s(Slot::TextSecondary)),
                Span::styled(" · ", theme.s(Slot::TextMuted)),
                Span::styled("shelf-4", theme.s(Slot::AccentSecondary)),
                Span::styled(" · 16m ago", theme.s(Slot::TextMuted)),
            ]),
            Line::default(),
            Line::from(chips(theme, &[("needs you", Tone::Warn), ("ready to file", Tone::Good)])),
            Line::default(),
        ];
        lines.extend(kv(
            theme,
            vec![
                ("owner", vec![Span::styled("Alice Example", theme.s(Slot::TextPrimary))]),
                ("where", vec![Span::styled("north-wing / shelf-4", theme.s(Slot::TextPrimary))]),
                ("size", vec![Span::styled("42 kB", theme.s(Slot::DataNumber))]),
            ],
        ));
        lines.push(Line::default());
        lines.push(labelled(theme, "steps", "4 of 5 done · 6m 12s"));
        lines.extend(stages(&sample_stages(theme), width));
        lines.push(Line::default());
        lines.push(Line::from(vec![
            Span::styled("RECENT RUNS  ", theme.s(Slot::TextMuted).add_modifier(Modifier::BOLD)),
            spark(theme, &RUNS, 24, Tone::Info),
        ]));
        lines.push(Line::default());
        lines.push(widgets::heading(theme, "notes"));
        let bar = theme.s(Slot::AccentMuted);
        lines.push(Line::from(vec![
            Span::styled("│ ", bar),
            Span::styled("bob", theme.s(Slot::TextSecondary)),
            Span::styled(" · 2h ago", theme.s(Slot::TextMuted)),
        ]));
        lines.extend(wrapped(NOTE, width, Span::styled("│ ", bar), theme.s(Slot::TextPrimary)));
        lines.push(Line::default());
        lines.extend(wrapped(NOTE, width, Span::raw(""), theme.s(Slot::TextSecondary)));
        super::card::card_owned(
            theme, area, buf, "north-wing #104",
            super::frame::Focus { focused: true, owns_keys: true }, &self.card, lines,
        );
    }

    fn meters_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let inner = widgets::padded(panel(theme, area, buf, "meters", true));
        let mut y = inner.y;
        let line = |buf: &mut Buffer, l: &Line<'_>, y: &mut u16| {
            if *y < inner.y + inner.height {
                buf.set_line(inner.x, *y, l, inner.width);
            }
            *y += 1;
        };
        line(buf, &widgets::heading(theme, "stage bar"), &mut y);
        for l in stages(&sample_stages(theme), inner.width) {
            line(buf, &l, &mut y);
        }
        y += 1;
        line(buf, &widgets::heading(theme, "ratios"), &mut y);
        for (label, r, tone) in [("approved", 0.5, Tone::Warn), ("filed", 0.8, Tone::Good), ("failed", 0.1, Tone::Bad)] {
            if y < inner.y + inner.height {
                let w = inner.width.min(60);
                ratio(theme, Rect { y, height: 1, width: w, ..inner }, buf, &format!("{label:<8}"), r, tone);
            }
            y += 1;
        }
        y += 1;
        line(buf, &widgets::heading(theme, "status strip"), &mut y);
        line(
            buf,
            &strip(theme, &[("4 open".into(), Tone::Plain), ("1 needs you".into(), Tone::Warn), ("1 ready".into(), Tone::Good)]),
            &mut y,
        );
        y += 1;
        line(buf, &widgets::heading(theme, "spinner"), &mut y);
        line(buf, &busy(theme, self.frame, "asking the server…"), &mut y);
        y += 1;
        line(buf, &widgets::heading(theme, "sparkline"), &mut y);
        let h = 3.min((inner.y + inner.height).saturating_sub(y));
        sparkline(theme, Rect { y, height: h, width: inner.width.min(60), ..inner }, buf, &RUNS, Tone::Info);
    }

    fn rows_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let wide = area.width >= 100;
        let [left, right] = if wide {
            Layout::horizontal([Constraint::Percentage(50), Constraint::Fill(1)]).areas(area)
        } else {
            Layout::vertical([Constraint::Length(8), Constraint::Fill(1)]).areas(area)
        };
        let inner = panel(theme, left, buf, "settings rows", true);
        // A footer: what the selected row is, below the rows, never among them.
        let about = Line::styled(
            "The selected row, in words: what it holds and what it accepts.",
            theme.s(Slot::TextSecondary),
        );
        let inner = super::card::footer(theme, inner, buf, "about", vec![about], 2);
        // A heading among the rows: the cursor steps over it.
        let mut rows = settings_rows(theme, inner.width);
        rows.insert(HEADING_AT, ListRow::heading(widgets::heading(theme, " more shelves")));
        RowList::new(rows)
            .cursor(self.list_row)
            .render(theme, super::card::above(left, inner), inner, buf, &self.list);

        let [detail, states] = Layout::vertical([Constraint::Fill(1), Constraint::Fill(1)]).areas(right);
        let inner = panel(theme, detail, buf, "server", false);
        let checked = Check::Passed { said: "signed in as alice".into(), ago: Some("2m".into()) };
        let mut lines = vec![
            Line::styled("Where the shelves are kept.", theme.s(Slot::TextPrimary)),
            Line::default(),
            widgets::heading(theme, "checked"),
            checked.line(theme, "not checked yet · c checks it", "asking the server…"),
            Check::Failed { said: "the server said 401".into(), ago: None }.line(theme, "", ""),
            Check::NotYet.line(theme, "not checked yet · c checks it", ""),
            Line::default(),
            widgets::heading(theme, "as you type"),
            as_you_type(theme, Ok("a link to a shelf")),
            as_you_type(theme, Err("not a link or a path")),
        ];
        lines.push(loading(theme));
        lines.push(failed(theme, "the server did not answer"));
        widgets::paragraph(buf, inner, lines);

        let [a, b] = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(states);
        let inner = panel(theme, a, buf, "empty", false);
        empty(theme, inner, buf, "No shelves yet. Press , for settings, then add one under shelves.");
        let inner = panel(theme, b, buf, "set up", false);
        set_up(
            theme,
            inner,
            buf,
            "Shelves",
            "It needs a server and a token.",
            Some("Press ⏎ to set it up in settings."),
        );
    }
}

impl Gallery {
    fn log_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let wide = area.width >= 100;
        let [left, right] = if wide {
            Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(area)
        } else {
            Layout::vertical([Constraint::Length(9), Constraint::Fill(1)]).areas(area)
        };
        let inner = panel(theme, left, buf, "why the sort stopped", false);
        Entries::new(sample_entries(theme))
            .cursor(0)
            .focused(false)
            .render(theme, left, inner, buf, &self.entries);
        let (on, of) = self.log.found(&self.log_text);
        let title = format!("log · /{} {on} of {of}", self.log.query());
        let inner = panel(theme, right, buf, &title, true);
        LogView {
            text: &self.log_text,
            state: &self.log,
            focused: true,
            empty: "Nothing written yet.",
        }
        .render(theme, right, inner, buf);
    }
}

impl Gallery {
    /// A story: drawn whole into a tall scratch buffer (it says how many
    /// rows it used), then the rows from the scroll's offset are copied in,
    /// with the scrollbar on the right edge.
    fn story_tab(&self, story: &stories::Story, theme: &Theme, area: Rect, buf: &mut Buffer) {
        const CANVAS: u16 = 300;
        let width = area.width.saturating_sub(1);
        let canvas = Rect::new(0, 0, width, CANVAS);
        let mut scratch = Buffer::empty(canvas);
        scratch.set_style(canvas, theme.bg(Slot::BgBase));
        let used = (story.render)(theme, canvas, &mut scratch, self.frame).min(CANVAS);
        let at = self.story.fit(area.height, used);
        for y in 0..area.height.min(used.saturating_sub(at)) {
            for x in 0..width {
                buf[(area.x + x, area.y + y)] = scratch[(x, at + y)].clone();
            }
        }
        super::scroll::scrollbar(theme, area, buf, self.story.max() as usize, at as usize);
    }
}

impl Gallery {
    /// Text written in markdown, laid out for the card it sits in.
    fn markdown_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let width = area.width.saturating_sub(4);
        let mut lines = markdown::render(theme, MARKDOWN, width).lines;
        lines.push(Line::default());
        lines.extend(rich::layout(&rich_sample(), &rich::Look::new(theme), width).lines);
        card(theme, area, buf, "a note, written in markdown, and what other formats add", true, &self.markdown, lines);
    }

    /// A file's changes, with what was said under one of its lines.
    fn diff_tab(&self, theme: &Theme, area: Rect, buf: &mut Buffer) {
        let inner = panel(theme, area, buf, "shelf/Shelves.java · 6 added, 2 removed", true);
        let view = DiffView::new(&self.diff_lines).language(Some("java".into()));
        let width = inner.width.saturating_sub(view.text_column() as u16 + 1);
        let bar = Span::styled("│ ", theme.s(Slot::AccentMuted));
        let mut note = vec![Line::from(vec![
            Span::styled("● open", theme.bold(Slot::StatusWarning)),
            Span::styled(" · bob · 2h ago", theme.s(Slot::TextMuted)),
        ])];
        note.extend(wrapped(NOTE, width, bar, theme.s(Slot::TextPrimary)));
        view.under(DIFF_NOTED, note).render(theme, area, inner, buf, &self.diff);
    }
}

/// The line of [`SAMPLE_DIFF`] the gallery's note is under.
const DIFF_NOTED: usize = 7;

/// A file's changes in two hunks, as a unified diff.
pub const SAMPLE_DIFF: &str = "@@ -18,6 +18,12 @@ public class Shelves {\n \
@RestController\n \
public class Shelves {\n\
-    // TODO: the dashboard\n\
+    private final Atlas atlas;\n\
+\n\
+    @GetMapping(\"/shelf/{id}\")\n\
+    public Shelf shelf(@PathVariable UUID id) {\n\
+        return atlas.find(id);\n\
+    }\n\
 \n\
 }\n\
\\ No newline at end of file\n\
@@ -40,3 +46,3 @@\n \
 void sort() {\n\
-    by(decade);\n\
+    by(decade, title);\n";

/// What a tracker's or a chat's text adds to markdown: a call-out, a
/// state as a badge, mentions, emoji, a file.
fn rich_sample() -> rich::Doc {
    use rich::{Block, Doc, Inline, Media};
    Doc::new(vec![
        Block::Callout {
            tone: Tone::Info,
            symbol: "ℹ ".into(),
            blocks: vec![Block::para(vec![
                Inline::text("The binder is "),
                Inline::badge("in progress", Tone::Info),
                Inline::text(", asked by "),
                Inline::mention("@alice", false),
                Inline::text(" for "),
                Inline::mention("@you", true),
                Inline::text(" "),
                Inline::emoji(rich::emoji("tada").unwrap_or_else(|| ":tada:".into())),
                Inline::text(" "),
                Inline::emoji(":shelf-bot:"),
            ])],
        },
        Block::Media(Media {
            image: false,
            name: "inventory.csv".into(),
            url: "https://example.com/inventory.csv".into(),
            ..Media::default()
        }),
    ])
}

/// A sorting run's failed steps, each with the lines that say why, under a
/// heading; then the steps that went well.
pub(super) fn sample_entries(theme: &Theme) -> Vec<Entry> {
    let bar = Span::styled("  │ ", theme.s(Slot::AccentMuted));
    let said = |text: &str| Line::from(vec![bar.clone(), Span::styled(text.to_string(), theme.s(Slot::TextSecondary))]);
    let step = |mark: Mark, name: &str, how: &str| {
        Line::from(vec![
            Span::styled(format!("{} ", mark.symbol()), widgets::tone(theme, mark.tone())),
            Span::styled(format!("{name:<10}"), theme.s(Slot::TextPrimary)),
            Span::styled(how.to_string(), theme.s(Slot::TextMuted)),
        ])
    };
    vec![
        Entry::new(step(Mark::Failed, "label", "2m 04s · ran out of labels"))
            .before(vec![widgets::heading(theme, "why it stopped")])
            .body(vec![said("no label left for box 17"), said("the drawer holds 0 of 40")]),
        Entry::new(step(Mark::Failed, "file", "12s · timed out")).body(vec![said("shelf 4 did not answer")]),
        Entry::new(step(Mark::Done, "fetch", "1m 10s"))
            .before(vec![Line::default(), widgets::heading(theme, "every step")]),
        Entry::new(step(Mark::Done, "sort", "3m 02s")),
    ]
}

/// A sorting robot's output: two sections, one folded, colours as a
/// terminal writes them.
fn sample_log() -> LogText {
    let mut lines: Vec<LogLine> = [
        "\u{1b}[36;1mStarting the sort\u{1b}[0m",
        "using the east room",
        "fetching box 1",
        "fetching box 2",
        "\u{1b}[32;1m$ sort --by decade\u{1b}[0m",
        "box 1: 12 atlases, 1900-1950",
        "box 2: 9 atlases, 1950-1990",
        "\u{1b}[33mwarning: box 2 has a loose map\u{1b}[0m",
        "\u{1b}[32;1m$ label --all\u{1b}[0m",
    ]
    .iter()
    .map(|l| LogLine::ansi(l))
    .collect();
    for i in 1..=16 {
        lines.push(LogLine::plain(format!("box {i}: label printed")));
    }
    lines.push(LogLine::ansi("\u{1b}[31;1merror: no label left for box 17\u{1b}[0m"));
    lines.push(LogLine::plain("the drawer holds 0 of 40 labels; the long line runs on and on so that it wraps at the narrow width of the panel"));
    let n = lines.len();
    LogText {
        sections: vec![
            Section { title: "Getting the boxes".into(), detail: "4s".into(), start: 1, end: 4, folded: true },
            Section { title: "Sorting and labelling".into(), detail: "2m 04s".into(), start: 4, end: n, folded: false },
        ],
        lines,
    }
}

const TABLE_ROWS: usize = 30;
/// The settings rows and the heading put among them.
const SETTINGS_ROWS: usize = 10;
const HEADING_AT: usize = 7;
const STATES: [(&str, Tone); 4] = [
    ("✓ filed", Tone::Good),
    ("● sorting", Tone::Info),
    ("✗ lost", Tone::Bad),
    ("◷ waiting", Tone::Muted),
];
const TITLES: [&str; 5] = [
    "sort the atlas box by decade",
    "label the map drawer",
    "mend the spine of the old ledger",
    "count the postcards",
    "move the globes to the top shelf",
];
const OWNERS: [&str; 3] = ["Alice E.", "Bob", "Carol D."];
const RUNS: [u64; 24] = [3, 4, 4, 6, 5, 9, 7, 6, 5, 5, 4, 8, 12, 9, 7, 6, 6, 5, 7, 8, 6, 5, 4, 6];
const MARKDOWN: &str = "\
<!-- a comment: never shown -->
# Sorting the atlas box

The box holds **two decades** of maps, ~~three~~ four shelves of *loose* sheets, and a
[catalogue](https://example.com/catalogue) kept in `shelf-4`. See the
![shelf plan](https://example.com/plan.svg) for where they go.

## What was done

- Sorted 1900 to 1950
  - labelled every drawer
  - counted the postcards
- [x] Boxed the atlases
- [ ] Filed the loose maps

1. Dust the lid
2. Open the box

> Keep the maps flat.
> The old ones tear.

```
sort --by decade --keep-flat atlases/ maps/ postcards/ loose-sheets/ drawers/
```

| Shelf | Items | Done |
|:--|--:|:-:|
| north-wing / shelf-4 | 42 | yes |
| east-wing / shelf-1 | 7 | no |

---

<details><summary>Who helped</summary>

Alice, Bob and Carol, with a <a href=\"https://example.com/team\">kind word</a> from the whole team.

</details>
";
pub(super) const NOTE: &str ="The decades from 1900 to 1950 are done. The rest wait for the new \
     dividers, which come on Thursday. Two atlases have loose maps: they are in the \
     tray on the left until the binder has a look. Nothing here needs anyone else.";

pub(super) fn sample_stages(theme: &Theme) -> Vec<Stage> {
    [
        ("fetch", Mark::Done),
        ("sort", Mark::Done),
        ("label", Mark::Done),
        ("check", Mark::Running),
        ("file", Mark::Waiting),
    ]
    .into_iter()
    .map(|(n, m)| Stage::new(n, m, widgets::tone(theme, m.tone())))
    .collect()
}

pub(super) fn tree_columns() -> Columns {
    Columns::new([
        Column::new("state", 10).keep(6),
        Column::new("label", 10).keep(4),
        Column::new("owner", 12).keep(2),
        Column::new("size", 8).keep(1),
        Column::new("updated", 9).keep(5),
    ])
}

/// Two groups of shelves, each with boxes, some holding items.
pub(super) fn sample_tree(theme: &Theme) -> Vec<Node> {
    let item = |id: &str, title: &str, who: &str, (word, tone): (&str, Tone)| {
        Node::new(
            id,
            vec![
                Span::styled(format!("#{id} "), theme.s(Slot::AccentSecondary)),
                Span::styled(title.to_string(), theme.s(Slot::TextPrimary)),
                Span::styled(format!("  {who}"), theme.s(Slot::TextMuted)),
            ],
        )
        .cell("state", Span::styled(word.to_string(), widgets::tone(theme, tone)))
        .cell("owner", Span::styled(who.to_string(), theme.s(Slot::TextSecondary)))
        .cell("updated", Span::styled("16m", theme.s(Slot::TextMuted)))
        .cell("label", Span::styled("#maps", theme.s(Slot::AccentTertiary)))
    };
    let shelf = |id: &str, n: usize| {
        Node::new(
            id,
            vec![
                Span::styled(id.to_string(), theme.bold(Slot::TextPrimary)),
                Span::styled("  in the east room", theme.s(Slot::TextMuted)),
            ],
        )
        .cell("size", Span::styled(format!("{} kB", n * 12), theme.s(Slot::DataNumber)))
        .cell("updated", Span::styled("4d", theme.s(Slot::TextMuted)))
        .cell(
            "state",
            Span::styled(
                if n == 0 { "—".to_string() } else { format!("{n} item{}", if n == 1 { "" } else { "s" }) },
                theme.s(Slot::DataNumber),
            ),
        )
    };
    let group = |id: &str, code: &str, slot: &str| {
        Node::new(
            id,
            vec![
                Span::styled(format!("{code} "), theme.spec(&format!("{slot} bold"))),
                Span::styled(id.to_string(), theme.bold(Slot::TextEmphasis)),
            ],
        )
        .stripe(theme.spec(slot))
    };
    vec![
        group("north-wing", "A", "glow.purple").children([
            shelf("atlases", 1).child(item(
                "104",
                "sort the atlas box by decade",
                "Alice E.",
                ("● sorting", Tone::Info),
            )),
            shelf("maps", 2).children([
                item("187", "label the map drawer", "you", ("✓ filed", Tone::Good)),
                item("198", "count the postcards", "Bob", ("✗ lost", Tone::Bad)),
            ]),
            shelf("ledgers", 0),
        ]),
        group("south-wing", "B", "glow.cyan").child(shelf("globes", 1).child(item(
            "342",
            "move the globes to the top shelf",
            "you",
            ("◷ waiting", Tone::Muted),
        ))),
    ]
}

/// A sample of what a part's environment holds, offered as a question.
pub(crate) fn sample_found() -> connection::Found {
    connection::Found {
        what: "a shelf token".into(),
        names: vec![("SHELF_TOKEN".into(), "the token".into())],
        file: None,
        does: "sets the token to env:SHELF_TOKEN and checks it with the shelves".into(),
        note: None,
    }
}

/// A part's connection section (`connection::field` and friends), then
/// the other kinds of settings rows.
pub(crate) fn settings_rows(theme: &Theme, width: u16) -> Vec<ListRow> {
    let ok = Check::Passed { said: "signed in as alice".into(), ago: Some("2m".into()) };
    let lead = || vec![Span::raw(" ")];
    let field = |label: &str, value: &str| {
        connection::field(theme, lead(), label, vec![Span::styled(value.to_string(), theme.s(Slot::TextPrimary))])
    };
    vec![
        ListRow::new(sample_found().row(theme, width)),
        ListRow::new(field("server", "https://shelves.example")),
        ListRow::new(field(connection::TOKEN, &connection::masked("env:SHELF_TOKEN", &[]))),
        ListRow::new(field(connection::SIGNED_IN, "alice")),
        ListRow::new(connection::check_field(theme, lead(), &ok, "the shelves")),
        ListRow::new(field(connection::REFRESH, &connection::every(60))),
        ListRow::new(field(connection::IDLE, &connection::idle(30))),
        ListRow::new(add_row(theme, "add a shelf", "paste a link to a shelf, or its path", width)),
        ListRow::new(Line::from(Span::styled(" a hidden shelf", Style::default()))).right(Span::styled("hidden", theme.s(Slot::TextMuted))),
    ]
}
