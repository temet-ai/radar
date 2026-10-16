//! Selecting text with the mouse. radar captures the mouse for clicks and the
//! wheel, so the terminal cannot select text itself; the runner does it
//! instead: a drag marks cells as the terminal would (from where it started
//! to where it is, row by row), and letting go copies what was marked.
//!
//! A selection belongs to the pane its drag started in: it runs along the
//! pane's rows between the pane's own edges, never across its border, its
//! scrollbar or into the pane beside it. The panes (and the rest of what a
//! frame draws that can be selected) are the [`Area`]s the frame marked.
//! Everything here is pure: the runner keeps the state and the clock.

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use crate::theme::Theme;
use crate::design::theme::Slot;

/// Two presses on the same cell within this many milliseconds are a double
/// click, three a triple (as the host counts a double click on a row).
pub const MULTI_CLICK_MS: u64 = 400;

/// A part of the screen a drag can select in: the inside of one pane (its
/// border, title and scrollbar are not in it), or a menu's, or a whole
/// body where the frame has no panes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Area {
    pub rect: Rect,
    /// Text to read ([`crate::view::Node::is_text`]): a double click selects
    /// a word. In rows to choose from, a double click is the host's.
    pub text: bool,
}

/// The area under `at`: the one marked last (what is drawn over another is
/// marked after it).
pub fn area_at(areas: &[Area], at: Position) -> Option<Area> {
    areas.iter().rev().find(|a| a.rect.contains(at)).copied()
}

/// `p` moved into `rect`: onto its nearest edge cell when it is outside.
pub fn clamp(p: Position, rect: Rect) -> Position {
    if rect.is_empty() {
        return p;
    }
    Position::new(
        p.x.clamp(rect.left(), rect.right() - 1),
        p.y.clamp(rect.top(), rect.bottom() - 1),
    )
}

/// A drag from `anchor` to `head`, in screen cells, inside `bound`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub anchor: Position,
    pub head: Position,
    /// The pane's inside: the selection never leaves it.
    pub bound: Rect,
}

impl Selection {
    /// A selection in `bound`: both ends are moved onto it.
    pub fn new(anchor: Position, head: Position, bound: Rect) -> Self {
        Self {
            anchor: clamp(anchor, bound),
            head: clamp(head, bound),
            bound,
        }
    }

    /// Start and end in reading order.
    fn ends(&self) -> (Position, Position) {
        let (a, h) = (self.anchor, self.head);
        if (h.y, h.x) < (a.y, a.x) { (h, a) } else { (a, h) }
    }

    /// Whether the pointer moved far enough to select: onto another row, or
    /// two cells along. A hand that slips one cell while clicking still
    /// clicks (seen live: a click on a tab arrived as a drag of one cell).
    pub fn moved(&self) -> bool {
        slid(self.anchor, self.head)
    }

    /// The cells it covers on row `y` of `buf`: `x` from..=to, between the
    /// pane's edges (the first row starts where the drag did, the last ends
    /// where it did, rows between run edge to edge).
    fn span(&self, y: u16, area: Rect) -> Option<(u16, u16)> {
        let bound = self.bound.intersection(area);
        if bound.is_empty() || y < bound.top() || y >= bound.bottom() {
            return None;
        }
        let (s, e) = self.ends();
        if y < s.y || y > e.y {
            return None;
        }
        let from = if y == s.y { s.x } else { bound.left() };
        let to = if y == e.y { e.x } else { bound.right() - 1 };
        Some((from.max(bound.left()), to.min(bound.right() - 1)))
    }

    /// Move the head by `dx` cells and `dy` rows, inside the pane.
    pub fn extend(&mut self, dx: i32, dy: i32) {
        let x = (i32::from(self.head.x) + dx).clamp(i32::from(self.bound.left()), i32::from(self.bound.right()) - 1);
        let y = (i32::from(self.head.y) + dy).clamp(i32::from(self.bound.top()), i32::from(self.bound.bottom()) - 1);
        self.head = Position::new(x as u16, y as u16);
    }

    /// Move the head to the pane's left (`end` false) or right edge of its row.
    pub fn extend_to_edge(&mut self, end: bool) {
        self.head.x = if end { self.bound.right() - 1 } else { self.bound.left() };
    }
}

/// Whether a press at `a` and the pointer at `b` are apart enough to be a
/// drag and not a click.
pub fn slid(a: Position, b: Position) -> bool {
    a.y != b.y || a.x.abs_diff(b.x) >= 2
}

/// How selected cells look: `bg.selection`, reversed where there are no
/// colours to carry it (the mono tier).
pub fn style(theme: &Theme) -> Style {
    if theme.tier() == crate::design::theme::Tier::Mono {
        Style::new().add_modifier(Modifier::REVERSED)
    } else {
        theme.bg(Slot::BgSelection)
    }
}

/// Show the selection: its cells in `style`.
pub fn paint(buf: &mut Buffer, sel: &Selection, style: Style) {
    let area = buf.area;
    for y in area.top()..area.bottom() {
        if let Some((from, to)) = sel.span(y, area) {
            for x in from..=to {
                buf[(x, y)].set_style(style);
            }
        }
    }
}

/// The text the selection covers, one line per row, without the spaces
/// that pad a row out to the edge.
pub fn text(buf: &Buffer, sel: &Selection) -> String {
    let area = buf.area;
    let mut lines = Vec::new();
    for y in area.top()..area.bottom() {
        if let Some((from, to)) = sel.span(y, area) {
            let mut line = String::new();
            let mut x = from;
            while x <= to {
                let symbol = buf[(x, y)].symbol();
                line.push_str(symbol);
                // A wide character covers the cells after it.
                x += (Span::raw(symbol).width() as u16).max(1);
            }
            lines.push(line.trim_end().to_string());
        }
    }
    lines.join("\n")
}

// --- words and lines ---------------------------------------------------------------

/// One character of a row and the cells it covers.
struct Glyph {
    x: u16,
    to: u16,
    ch: char,
}

/// The row `y` inside `bound`, a glyph at a time.
fn glyphs(buf: &Buffer, y: u16, bound: Rect) -> Vec<Glyph> {
    let bound = bound.intersection(buf.area);
    let mut out = Vec::new();
    if y < bound.top() || y >= bound.bottom() {
        return out;
    }
    let mut x = bound.left();
    while x < bound.right() {
        let symbol = buf[(x, y)].symbol();
        let width = (Span::raw(symbol).width() as u16).max(1);
        out.push(Glyph {
            x,
            to: (x + width - 1).min(bound.right() - 1),
            ch: symbol.chars().next().unwrap_or(' '),
        });
        x += width;
    }
    out
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Space,
    /// Letters, digits and what joins them in a path or a name, as iTerm2's
    /// word characters do (`/-+\~_.`): a file path selects whole.
    Word,
    Other,
}

fn class(ch: char) -> Class {
    if ch.is_whitespace() || ch == '\0' {
        Class::Space
    } else if ch.is_alphanumeric() || "/-+\\~_.".contains(ch) {
        Class::Word
    } else {
        Class::Other
    }
}

/// The glyph run around `i` of one class.
fn run(glyphs: &[Glyph], i: usize, of: impl Fn(char) -> bool) -> (usize, usize) {
    let mut s = i;
    while s > 0 && of(glyphs[s - 1].ch) {
        s -= 1;
    }
    let mut e = i;
    while e + 1 < glyphs.len() && of(glyphs[e + 1].ch) {
        e += 1;
    }
    (s, e)
}

/// The glyphs of the word under glyph `i`: a run of word characters, or of
/// spaces, or of the same other character; a link or a path selects whole
/// (everything up to a space, less the brackets, quotes and stops around it).
fn word_range(glyphs: &[Glyph], i: usize) -> (usize, usize) {
    let here = class(glyphs[i].ch);
    if here == Class::Space {
        return run(glyphs, i, |c| class(c) == Class::Space);
    }
    // A link: its characters run to the next space, except a closing bracket
    // or quote and the stops that end a sentence.
    let (ts, te) = run(glyphs, i, |c| class(c) != Class::Space);
    let token: String = glyphs[ts..=te].iter().map(|g| g.ch).collect();
    if token.contains("://") {
        let (mut s, mut e) = (ts, te);
        while s < e && "([{<\"'`".contains(glyphs[s].ch) {
            s += 1;
        }
        while e > s && ")]}>\"'`.,;:!?".contains(glyphs[e].ch) {
            e -= 1;
        }
        if (s..=e).contains(&i) {
            return (s, e);
        }
    }
    if here == Class::Word {
        let (s, mut e) = run(glyphs, i, |c| class(c) == Class::Word);
        // A stop or a dash after a word ends it, as in `fix the build.`
        while e > s.max(i) && matches!(glyphs[e].ch, '.' | '-') {
            e -= 1;
        }
        return (s, e);
    }
    let same = glyphs[i].ch;
    run(glyphs, i, |c| c == same)
}

fn from_range(glyphs: &[Glyph], (s, e): (usize, usize), y: u16, bound: Rect) -> Selection {
    Selection::new(
        Position::new(glyphs[s].x, y),
        Position::new(glyphs[e].to, y),
        bound,
    )
}

/// The word under `at` in `bound`, as a double click selects it; `None` when
/// the cell is not in the row's text.
pub fn word(buf: &Buffer, at: Position, bound: Rect) -> Option<Selection> {
    let g = glyphs(buf, at.y, bound);
    let i = g.iter().position(|g| g.x <= at.x && at.x <= g.to)?;
    Some(from_range(&g, word_range(&g, i), at.y, bound))
}

/// The line under `at` in `bound`, as a triple click selects it: from the
/// pane's left edge to the end of what the row says.
pub fn line(buf: &Buffer, at: Position, bound: Rect) -> Option<Selection> {
    let g = glyphs(buf, at.y, bound);
    g.first()?;
    let last = g.iter().rposition(|g| class(g.ch) != Class::Space);
    let head = last.map_or(g[0].x, |i| g[i].to);
    Some(Selection::new(Position::new(g[0].x, at.y), Position::new(head, at.y), bound))
}

/// Presses counted: one, two (a word), three (a line), and round again.
#[derive(Debug, Default)]
pub struct Clicks {
    last: Option<(Position, u64, u8)>,
}

impl Clicks {
    /// A press at `at`, `ms` on the clock: how many presses in a row it is.
    pub fn press(&mut self, at: Position, ms: u64) -> u8 {
        let n = match self.last {
            Some((p, t, n)) if p == at && ms.saturating_sub(t) <= MULTI_CLICK_MS && n < 3 => n + 1,
            _ => 1,
        };
        self.last = Some((at, ms, n));
        n
    }
}

// --- keys with a selection showing --------------------------------------------------

/// What a key does while a selection shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Move the selection's head by cells and rows.
    Extend(i32, i32),
    /// Move the head to the left or the right edge of its row.
    Edge { end: bool },
    /// Copy it again (ctrl+c).
    Copy,
    /// Anything else: the selection goes, and the key does what it does.
    Clear,
}

/// What `key` does with a selection showing. `editing`: text is being typed
/// (the editor, a field), where shift+arrows are the editor's own.
pub fn act(key: &KeyEvent, editing: bool) -> Act {
    let m = key.modifiers;
    if m == KeyModifiers::CONTROL && key.code == KeyCode::Char('c') {
        return Act::Copy;
    }
    if !editing && m == KeyModifiers::SHIFT {
        match key.code {
            KeyCode::Left => return Act::Extend(-1, 0),
            KeyCode::Right => return Act::Extend(1, 0),
            KeyCode::Up => return Act::Extend(0, -1),
            KeyCode::Down => return Act::Extend(0, 1),
            KeyCode::Home => return Act::Edge { end: false },
            KeyCode::End => return Act::Edge { end: true },
            _ => {}
        }
    }
    Act::Clear
}

// --- the gesture ---------------------------------------------------------------------

/// The left button, where it went down.
#[derive(Debug, Clone, Copy)]
struct Press {
    at: Position,
    ms: u64,
    /// Where it can select: `None` on a border, a title or the chrome.
    area: Option<Area>,
    /// It slid far enough to be a drag, not a click.
    moved: bool,
}

/// What letting go of the button did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Released {
    /// Nothing: a drag that started where nothing selects, or no press.
    Nothing,
    /// Text to copy: a drag, a double click or a triple click selected it.
    Copy(String),
    /// A click (a press that slid at most a cell): the host gets it.
    Click(Position, u64),
}

/// What a key did with a selection showing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Keyed {
    /// The selection took it (shift+arrows moved its head).
    Used,
    /// Copy this again (ctrl+c).
    Copy(String),
    /// Not a selection's key: the selection (if any) is gone, and the key
    /// does what it does.
    Pass,
}

/// The mouse's selecting, as the runner keeps it between events: the press,
/// the selection showing, and the presses counted. Pure: it gets the
/// clock and the frame as arguments.
#[derive(Debug, Default)]
pub struct Gesture {
    pressed: Option<Press>,
    /// The selection showing: while dragging, and after, until a key or a
    /// click ends it.
    pub selection: Option<Selection>,
    clicks: Clicks,
}

impl Gesture {
    /// The button went down at `at`: a selection showing is gone, and a new
    /// one may start in the area under it.
    pub fn down(&mut self, at: Position, ms: u64, areas: &[Area]) {
        self.selection = None;
        self.pressed = Some(Press {
            at,
            ms,
            area: area_at(areas, at),
            moved: false,
        });
    }

    /// The pointer moved with the button down: the selection runs from the
    /// press to `head`, kept inside the area the press was in.
    pub fn drag(&mut self, head: Position) {
        let Some(p) = self.pressed.as_mut() else { return };
        p.moved |= slid(p.at, head);
        if let (true, Some(area)) = (p.moved, p.area) {
            self.selection = Some(Selection::new(p.at, head, area.rect));
        }
    }

    /// A drag is selecting now (the button is down and has moved).
    pub fn selecting(&self) -> bool {
        self.pressed.is_some_and(|p| p.moved && p.area.is_some())
    }

    /// The button came up; `shown` is what the frame drew under the selection.
    pub fn up(&mut self, shown: &Buffer) -> Released {
        let Some(p) = self.pressed.take() else {
            return Released::Nothing;
        };
        if p.moved {
            return self.copy(shown);
        }
        let n = self.clicks.press(p.at, p.ms);
        if let (2 | 3, Some(area)) = (n, p.area)
            && area.text
        {
            self.selection = if n == 2 { word(shown, p.at, area.rect) } else { line(shown, p.at, area.rect) };
            return self.copy(shown);
        }
        Released::Click(p.at, p.ms)
    }

    /// The text of the selection, to copy; a blank one is dropped.
    fn copy(&mut self, shown: &Buffer) -> Released {
        match self.selection.as_ref().map(|s| text(shown, s)) {
            Some(t) if !t.trim().is_empty() => Released::Copy(t),
            _ => {
                self.selection = None;
                Released::Nothing
            }
        }
    }

    /// A key, with a selection showing or not. `editing`: text is being typed.
    pub fn key(&mut self, key: &KeyEvent, editing: bool, shown: &Buffer) -> Keyed {
        let Some(sel) = self.selection.as_mut() else {
            return Keyed::Pass;
        };
        match act(key, editing) {
            Act::Extend(dx, dy) => {
                sel.extend(dx, dy);
                Keyed::Used
            }
            Act::Edge { end } => {
                sel.extend_to_edge(end);
                Keyed::Used
            }
            Act::Copy => match self.copy(shown) {
                Released::Copy(t) => Keyed::Copy(t),
                _ => Keyed::Pass,
            },
            Act::Clear => {
                self.selection = None;
                Keyed::Pass
            }
        }
    }

    /// The selection goes (the wheel moved what is under it, or the screen
    /// changed size).
    pub fn clear(&mut self) {
        self.selection = None;
    }
}

/// The key that makes the terminal select its own text through radar's mouse
/// capture, from `$TERM_PROGRAM`: shift in most, option in iTerm2, fn in
/// Terminal.app.
pub fn bypass_key(term_program: Option<&str>) -> &'static str {
    match term_program {
        Some("iTerm.app") => "option",
        Some("Apple_Terminal") => "fn",
        _ => "shift",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Buffer {
        Buffer::with_lines(["!263 fix the build   ", "  stage test · 1m    ", "last line            "])
    }

    fn all() -> Rect {
        Rect::new(0, 0, 21, 3)
    }

    fn sel(ax: u16, ay: u16, hx: u16, hy: u16) -> Selection {
        Selection::new(Position::new(ax, ay), Position::new(hx, hy), all())
    }

    #[test]
    fn a_drag_on_one_row_copies_the_cells_between() {
        assert_eq!(text(&screen(), &sel(5, 0, 7, 0)), "fix");
    }

    #[test]
    fn across_rows_it_runs_to_the_edge_and_from_the_start_like_a_terminal() {
        let t = text(&screen(), &sel(5, 0, 6, 1));
        assert_eq!(t, "fix the build\n  stage");
    }

    #[test]
    fn dragging_backwards_selects_the_same() {
        let b = screen();
        assert_eq!(text(&b, &sel(6, 1, 5, 0)), text(&b, &sel(5, 0, 6, 1)));
    }

    #[test]
    fn painting_marks_only_the_selected_cells() {
        let mut b = screen();
        let mark = Style::new().bg(ratatui::style::Color::Red);
        paint(&mut b, &sel(5, 0, 7, 0), mark);
        let marked = |x, y| b[(x, y)].bg == ratatui::style::Color::Red;
        assert!(marked(5, 0) && marked(7, 0));
        assert!(!marked(4, 0) && !marked(8, 0) && !marked(5, 1));
        assert_eq!(b.area, Rect::new(0, 0, 21, 3));
    }

    #[test]
    fn a_wide_character_is_copied_once() {
        let mut b = Buffer::empty(Rect::new(0, 0, 10, 1));
        b.set_string(0, 0, "✅ ok", Style::new());
        let s = Selection::new(Position::new(0, 0), Position::new(9, 0), b.area);
        assert_eq!(text(&b, &s), "✅ ok");
    }

    #[test]
    fn a_press_that_does_not_move_is_a_click() {
        assert!(!sel(3, 1, 3, 1).moved());
        assert!(!sel(3, 1, 4, 1).moved(), "a slip of one cell is still a click");
        assert!(!sel(4, 1, 3, 1).moved());
        assert!(sel(3, 1, 5, 1).moved());
        assert!(sel(3, 1, 3, 2).moved(), "another row selects");
    }

    // Two panes side by side on one screen: a list at the left, a detail at
    // the right, each in a border; the selection stays in the pane it started.
    fn two_panes() -> (Buffer, Rect, Rect) {
        let b = Buffer::with_lines([
            "╭ list ────╮╭ detail ──╮",
            "│ alpha one││ the fix  │",
            "│ beta two ││ is here  │",
            "│ gamma    ││ for you  │",
            "╰──────────╯╰──────────╯",
        ]);
        (b, Rect::new(1, 1, 10, 3), Rect::new(13, 1, 10, 3))
    }

    #[test]
    fn a_drag_past_a_panes_edge_copies_only_that_panes_text() {
        let (b, list, detail) = two_panes();
        // Started in the detail, dragged left across the border and the list.
        let s = Selection::new(Position::new(16, 1), Position::new(2, 1), detail);
        assert_eq!(s.head, Position::new(13, 1), "the head stops at the pane's left edge");
        assert_eq!(text(&b, &s), " the");
        // Started in the list, dragged right into the detail.
        let s = Selection::new(Position::new(3, 1), Position::new(22, 1), list);
        assert_eq!(text(&b, &s), "lpha one");
    }

    #[test]
    fn several_rows_stop_at_the_panes_right_edge_and_resume_at_its_left() {
        let (b, _, detail) = two_panes();
        let s = Selection::new(Position::new(18, 1), Position::new(15, 3), detail);
        assert_eq!(text(&b, &s), "fix\n is here\n fo");
        assert!(!text(&b, &s).contains('│') && !text(&b, &s).contains("beta"));
    }

    #[test]
    fn a_head_dragged_above_or_below_clamps_to_the_nearest_row() {
        let (_, list, _) = two_panes();
        let s = Selection::new(Position::new(3, 2), Position::new(5, 40), list);
        assert_eq!(s.head, Position::new(5, 3));
        let s = Selection::new(Position::new(3, 2), Position::new(5, 0), list);
        assert_eq!(s.head, Position::new(5, 1));
    }

    #[test]
    fn painting_stays_inside_the_pane_too() {
        let (mut b, _, detail) = two_panes();
        let mark = Style::new().bg(ratatui::style::Color::Red);
        let s = Selection::new(Position::new(14, 1), Position::new(14, 3), detail);
        paint(&mut b, &s, mark);
        for y in 0..5 {
            for x in 0..24u16 {
                let inside = detail.contains(Position::new(x, y)) && (y > 1 || x >= 14) && (y < 3 || x <= 14);
                assert_eq!(b[(x, y)].bg == ratatui::style::Color::Red, inside, "cell {x},{y}");
            }
        }
    }

    #[test]
    fn the_area_under_a_press_is_the_one_marked_last() {
        let pane = Area { rect: Rect::new(0, 0, 20, 10), text: false };
        let menu = Area { rect: Rect::new(5, 2, 8, 4), text: false };
        let areas = [pane, menu];
        assert_eq!(area_at(&areas, Position::new(6, 3)), Some(menu));
        assert_eq!(area_at(&areas, Position::new(1, 1)), Some(pane));
        assert_eq!(area_at(&areas, Position::new(25, 1)), None, "chrome and borders are in no area");
    }

    #[test]
    fn shift_arrows_move_the_head_inside_the_pane() {
        let (_, list, _) = two_panes();
        let mut s = Selection::new(Position::new(3, 1), Position::new(4, 1), list);
        s.extend(1, 0);
        assert_eq!(s.head, Position::new(5, 1));
        s.extend(0, 1);
        assert_eq!(s.head, Position::new(5, 2));
        s.extend(0, 9);
        assert_eq!(s.head, Position::new(5, 3), "not past the pane's last row");
        s.extend(-99, 0);
        assert_eq!(s.head, Position::new(1, 3), "not onto the border");
        s.extend_to_edge(true);
        assert_eq!(s.head, Position::new(10, 3));
        assert_eq!(s.anchor, Position::new(3, 1), "the anchor stays");
    }

    fn row(s: &str) -> Buffer {
        let mut b = Buffer::empty(Rect::new(0, 0, 60, 1));
        b.set_string(0, 0, s, Style::new());
        b
    }

    fn word_at(s: &str, x: u16) -> String {
        let b = row(s);
        text(&b, &word(&b, Position::new(x, 0), b.area).unwrap())
    }

    #[test]
    fn a_double_click_selects_a_word() {
        assert_eq!(word_at("fix the build", 5), "the");
        assert_eq!(word_at("fix the build", 0), "fix");
        assert_eq!(word_at("fix the build.", 10), "build", "a stop is not part of it");
        assert_eq!(word_at("snake_case, next", 3), "snake_case");
    }

    #[test]
    fn a_file_path_selects_whole() {
        let line = "see crates/radar-core/src/select.rs: line 12";
        assert_eq!(word_at(line, 4), "crates/radar-core/src/select.rs");
        assert_eq!(word_at(line, 20), "crates/radar-core/src/select.rs");
        assert_eq!(word_at(line, 42), "12");
    }

    #[test]
    fn a_url_selects_whole() {
        let line = "open (https://example.com/a/b?x=1&y=2#top). then";
        assert_eq!(word_at(line, 10), "https://example.com/a/b?x=1&y=2#top");
        assert_eq!(word_at(line, 30), "https://example.com/a/b?x=1&y=2#top");
        assert_eq!(word_at("go to https://x.io.", 8), "https://x.io");
    }

    #[test]
    fn a_double_click_on_spaces_or_a_mark_selects_that_run() {
        assert_eq!(word_at("a    b", 2), "");
        let b = row("a == b");
        let s = word(&b, Position::new(3, 0), b.area).unwrap();
        assert_eq!((s.anchor.x, s.head.x), (2, 3), "the two marks");
    }

    #[test]
    fn a_word_stays_in_its_pane() {
        let (b, _, detail) = two_panes();
        let s = word(&b, Position::new(14, 2), detail).unwrap();
        assert_eq!(text(&b, &s), "is");
        let s = word(&b, Position::new(19, 2), detail).unwrap();
        assert_eq!(text(&b, &s), "here");
    }

    #[test]
    fn a_wide_character_is_a_word_of_its_own_cells() {
        let mut b = Buffer::empty(Rect::new(0, 0, 12, 1));
        b.set_string(0, 0, "✅ ok", Style::new());
        let s = word(&b, Position::new(1, 0), b.area).unwrap();
        assert_eq!(text(&b, &s), "✅");
    }

    #[test]
    fn a_triple_click_selects_the_line_without_its_padding() {
        let b = screen();
        let s = line(&b, Position::new(7, 1), all()).unwrap();
        assert_eq!(text(&b, &s), "  stage test · 1m");
        let (b, _, detail) = two_panes();
        let s = line(&b, Position::new(15, 2), detail).unwrap();
        assert_eq!(text(&b, &s), " is here");
        let blank = Buffer::empty(Rect::new(0, 0, 5, 1));
        let s = line(&blank, Position::new(2, 0), blank.area).unwrap();
        assert_eq!(text(&blank, &s), "");
    }

    #[test]
    fn presses_are_counted_up_to_three_then_start_again() {
        let mut c = Clicks::default();
        let at = Position::new(4, 4);
        assert_eq!(c.press(at, 1000), 1);
        assert_eq!(c.press(at, 1200), 2);
        assert_eq!(c.press(at, 1300), 3);
        assert_eq!(c.press(at, 1400), 1, "a fourth starts over");
        assert_eq!(c.press(at, 2000), 1, "too late");
        assert_eq!(c.press(Position::new(5, 4), 2100), 1, "another cell");
        assert_eq!(c.press(Position::new(5, 4), 2200), 2);
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn keys_with_a_selection_showing() {
        let none = KeyModifiers::NONE;
        let shift = KeyModifiers::SHIFT;
        assert_eq!(act(&key(KeyCode::Right, shift), false), Act::Extend(1, 0));
        assert_eq!(act(&key(KeyCode::Left, shift), false), Act::Extend(-1, 0));
        assert_eq!(act(&key(KeyCode::Up, shift), false), Act::Extend(0, -1));
        assert_eq!(act(&key(KeyCode::Down, shift), false), Act::Extend(0, 1));
        assert_eq!(act(&key(KeyCode::Home, shift), false), Act::Edge { end: false });
        assert_eq!(act(&key(KeyCode::End, shift), false), Act::Edge { end: true });
        assert_eq!(act(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), false), Act::Copy);
        assert_eq!(act(&key(KeyCode::Down, none), false), Act::Clear, "any other key clears it");
        assert_eq!(act(&key(KeyCode::Char('j'), none), false), Act::Clear);
        assert_eq!(act(&key(KeyCode::Char('C'), shift), false), Act::Clear);
        assert_eq!(
            act(&key(KeyCode::Right, shift), true),
            Act::Clear,
            "with the editor open shift+arrows are the editor's"
        );
        assert_eq!(act(&key(KeyCode::Char('c'), KeyModifiers::CONTROL), true), Act::Copy);
    }

    #[test]
    fn the_terminals_own_selection_key() {
        assert_eq!(bypass_key(None), "shift");
        assert_eq!(bypass_key(Some("ghostty")), "shift");
        assert_eq!(bypass_key(Some("iTerm.app")), "option");
        assert_eq!(bypass_key(Some("Apple_Terminal")), "fn");
    }

    #[test]
    fn selected_cells_are_drawn_on_bg_selection_and_reversed_in_the_mono_tier() {
        use crate::design::theme::Tier;
        let colour = Theme::new(Tier::TrueColor);
        assert_eq!(style(&colour), colour.bg(Slot::BgSelection));
        assert!(style(&colour).bg.is_some());
        let mono = Theme::new(Tier::Mono);
        assert_eq!(style(&mono), Style::new().add_modifier(Modifier::REVERSED));
    }

    // --- the gesture on two panes ---------------------------------------------------

    fn panes() -> (Buffer, [Area; 2]) {
        let (b, list, detail) = two_panes();
        (b, [Area { rect: list, text: false }, Area { rect: detail, text: true }])
    }

    fn at(x: u16, y: u16) -> Position {
        Position::new(x, y)
    }

    #[test]
    fn a_drag_in_the_detail_that_runs_past_its_edges_copies_only_the_detail() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        // Past the left edge, across the list's border and into the list.
        g.down(at(15, 2), 0, &areas);
        g.drag(at(14, 2));
        g.drag(at(3, 2));
        assert!(g.selecting());
        assert_eq!(g.up(&b), Released::Copy(" is".into()));
        // Past the right edge, off the screen and down below the pane.
        g.down(at(18, 2), 0, &areas);
        g.drag(at(40, 8));
        assert_eq!(g.up(&b), Released::Copy("ere\n for you".into()));
    }

    #[test]
    fn a_drag_that_starts_on_a_border_or_the_chrome_is_not_a_selection() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        for start in [at(0, 2), at(11, 2), at(5, 0), at(12, 2), at(23, 2), at(5, 4), at(5, 9)] {
            g.down(start, 0, &areas);
            g.drag(at(start.x + 5, start.y));
            assert!(!g.selecting());
            assert!(g.selection.is_none());
            assert_eq!(g.up(&b), Released::Nothing, "a drag from {start:?} did something");
        }
    }

    #[test]
    fn a_press_that_does_not_slide_is_a_click_for_the_host() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        g.down(at(3, 2), 500, &areas);
        g.drag(at(4, 2));
        assert_eq!(g.up(&b), Released::Click(at(3, 2), 500));
        assert!(g.selection.is_none());
    }

    #[test]
    fn double_and_triple_click_in_text_give_a_word_then_the_line() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        let click = |g: &mut Gesture, x, ms| {
            g.down(at(x, 2), ms, &areas);
            g.up(&b)
        };
        assert_eq!(click(&mut g, 18, 1000), Released::Click(at(18, 2), 1000), "the first press is the host's");
        assert_eq!(click(&mut g, 18, 1100), Released::Copy("here".into()));
        assert!(g.selection.is_some(), "the word stays selected");
        assert_eq!(click(&mut g, 18, 1200), Released::Copy(" is here".into()));
    }

    #[test]
    fn a_double_click_on_rows_is_left_to_the_host() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        g.down(at(3, 2), 1000, &areas);
        assert_eq!(g.up(&b), Released::Click(at(3, 2), 1000));
        g.down(at(3, 2), 1100, &areas);
        assert_eq!(g.up(&b), Released::Click(at(3, 2), 1100), "the host turns two presses into a double click");
        assert!(g.selection.is_none());
    }

    #[test]
    fn shift_arrows_extend_a_showing_selection_inside_its_pane() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        g.down(at(14, 1), 0, &areas);
        g.drag(at(16, 1));
        assert_eq!(g.up(&b), Released::Copy("the".into()));
        let shift = |code| KeyEvent::new(code, KeyModifiers::SHIFT);
        assert_eq!(g.key(&shift(KeyCode::Right), false, &b), Keyed::Used);
        assert_eq!(g.key(&shift(KeyCode::Down), false, &b), Keyed::Used);
        assert_eq!(g.selection.unwrap().head, at(17, 2));
        assert_eq!(g.key(&shift(KeyCode::End), false, &b), Keyed::Used);
        assert_eq!(g.selection.unwrap().head, at(22, 2), "the pane's edge, not the border");
        assert_eq!(g.key(&shift(KeyCode::Home), false, &b), Keyed::Used);
        assert_eq!(g.selection.unwrap().head, at(13, 2));
        // With the editor open they are the editor's: the selection goes.
        assert_eq!(g.key(&shift(KeyCode::Right), true, &b), Keyed::Pass);
        assert!(g.selection.is_none());
    }

    #[test]
    fn ctrl_c_copies_a_showing_selection_again_and_any_other_key_clears_it() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        g.down(at(14, 1), 0, &areas);
        g.drag(at(16, 1));
        g.up(&b);
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(g.key(&ctrl_c, false, &b), Keyed::Copy("the".into()));
        assert!(g.selection.is_some(), "it stays showing");
        assert_eq!(g.key(&KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE), false, &b), Keyed::Pass);
        assert!(g.selection.is_none());
        assert_eq!(g.key(&ctrl_c, false, &b), Keyed::Pass, "with nothing selected ctrl+c is the app's");
    }

    #[test]
    fn a_click_elsewhere_clears_a_showing_selection() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        g.down(at(14, 1), 0, &areas);
        g.drag(at(16, 1));
        g.up(&b);
        assert!(g.selection.is_some());
        g.down(at(3, 3), 900, &areas);
        assert!(g.selection.is_none());
    }

    #[test]
    fn a_selection_of_only_spaces_copies_nothing() {
        let (b, areas) = panes();
        let mut g = Gesture::default();
        g.down(at(21, 3), 0, &areas);
        g.drag(at(23, 3));
        assert_eq!(g.up(&b), Released::Nothing);
        assert!(g.selection.is_none());
    }
}
