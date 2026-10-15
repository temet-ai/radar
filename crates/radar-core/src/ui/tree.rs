//! The banded tree: a hierarchy drawn by `tui-tree-widget` with no fold
//! markers (no chevrons, no `[+]`). Tree lines join each row to its parent
//! (`├─`, `└─`, and `│` past a parent that is not the last), each top-level
//! group has a colour stripe `▌` down its rows, every other block at the
//! band level sits on a band, top-level rows sit on the heading shade, a
//! folded node shows a dim `⋯`, and named columns line up at the right
//! ([`Columns`]). The selected row keeps the selection's background.
//!
//! The tree indents each level by 2 cells; a row's own tree line starts its
//! text, so its room is taken before the name is fitted and the columns stay
//! aligned at every depth.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, StatefulWidget};
use tui_tree_widget::{Tree, TreeItem};
/// The tree's state (what is open and selected), so a part needs no
/// tree crate of its own.
pub use tui_tree_widget::TreeState;

use super::columns::Columns;
use crate::design::theme::Slot;
use crate::input::TextInput;
use crate::theme::Theme;
use crate::widgets;

/// One row and what is under it.
#[derive(Debug, Clone)]
pub struct Node {
    /// Unique among its siblings; a row is known by the ids down to it.
    pub id: String,
    /// The name part, left of the columns; cut to fit.
    pub name: Vec<Span<'static>>,
    /// Values by column heading; a value may be several spans (an icon in
    /// its own colour, then the words in the cell's).
    pub cells: Vec<(String, Line<'static>)>,
    pub children: Vec<Node>,
    /// A top-level node's colour, for the stripe down all its rows.
    pub stripe: Option<Style>,
    /// Chips after the name, and which name span gives way to them
    /// ([`super::chips::after_name`]).
    pub chips: Option<(usize, Vec<Span<'static>>)>,
    /// After the name, shown whole or not at all (a count the detail panel
    /// also has).
    pub detail: Option<Span<'static>>,
    /// Folds even with no children given (its fold is the part's): `⋯`
    /// while it is closed.
    pub foldable: bool,
    /// A text input is drawn on this row, this many cells into its name
    /// (past the stripe or tree line the kit puts before it); see
    /// [`BandedTree::input`].
    pub input_at: Option<u16>,
}

impl Node {
    pub fn new(id: impl Into<String>, name: Vec<Span<'static>>) -> Self {
        Self {
            id: id.into(),
            name,
            cells: Vec::new(),
            children: Vec::new(),
            stripe: None,
            chips: None,
            detail: None,
            foldable: false,
            input_at: None,
        }
    }

    /// The text input of [`BandedTree::input`] goes on this row, starting
    /// `x` cells into the name, over the words that start there.
    pub fn input_at(mut self, x: u16) -> Self {
        self.input_at = Some(x);
        self
    }

    /// Shows `⋯` while closed even when it has no children to hide.
    pub fn foldable(mut self) -> Self {
        self.foldable = true;
        self
    }

    /// Detail after the name, left out when it does not fit whole.
    pub fn detail(mut self, detail: Span<'static>) -> Self {
        self.detail = Some(detail);
        self
    }

    /// Chips after the name (another part's labels): the name span at
    /// `gives` (a title) gives way to them while it keeps 12 cells.
    pub fn chips(mut self, gives: usize, chips: Vec<Span<'static>>) -> Self {
        if !chips.is_empty() {
            self.chips = Some((gives, chips));
        }
        self
    }

    /// The value under the column `heading`: a span, or several (a `Line`
    /// or `Vec<Span>`) so an icon keeps its own colour beside the words.
    pub fn cell(mut self, heading: impl Into<String>, value: impl Into<Line<'static>>) -> Self {
        self.cells.push((heading.into(), value.into()));
        self
    }

    pub fn child(mut self, node: Node) -> Self {
        self.children.push(node);
        self
    }

    pub fn children(mut self, nodes: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(nodes);
        self
    }

    pub fn stripe(mut self, style: Style) -> Self {
        self.stripe = Some(style);
        self
    }
}

/// How the tree's own marks look. A part fills it from its look words
/// ([`TreeLook::from_specs`]) so the user can change them.
#[derive(Debug, Clone)]
pub struct TreeLook {
    /// The tree lines.
    pub lines: Style,
    /// The shade behind every other block at `band_level`.
    pub band: Option<Color>,
    /// The shade behind top-level rows.
    pub heading: Option<Color>,
    /// The `⋯` of a folded node.
    pub folded: Style,
    /// The depth whose blocks alternate bands: 1 bands every other child of
    /// a top-level node with everything under it.
    pub band_level: usize,
}

impl TreeLook {
    /// Lines muted, band and heading on the overlay shade.
    pub fn new(theme: &Theme) -> Self {
        Self::from_specs(theme, "text.muted", "bg.overlay", "bg.overlay")
    }

    /// From style specs (`"text.muted"`, a part's look words). A band and a
    /// heading are backgrounds: only their slot's colour is used.
    pub fn from_specs(theme: &Theme, lines: &str, band: &str, heading: &str) -> Self {
        Self {
            lines: theme.spec(lines),
            band: theme.spec(band).fg,
            heading: theme.spec(heading).fg,
            folded: theme.s(Slot::TextMuted),
            band_level: 1,
        }
    }

    pub fn band_level(mut self, level: usize) -> Self {
        self.band_level = level;
        self
    }
}

/// A row's own tree line: none at the top, `├─ ` under it, and deeper one
/// more space per level (` ├─ `, `  ├─ `), so the glyph sits under the
/// first letter of its parent's name.
pub fn joint(depth: usize, last: bool) -> String {
    if depth == 0 {
        return String::new();
    }
    let glyph = if last { "└─ " } else { "├─ " };
    format!("{}{glyph}", " ".repeat(depth - 1))
}

/// The column of the glyph of a row at `depth` (1 or more), from the
/// tree's left edge: 2 cells of indent a level, plus the joint's spaces.
fn glyph_x(depth: usize) -> u16 {
    (3 * depth - 1) as u16
}

/// What the overlay needs about a row.
struct Look {
    depth: usize,
    stripe: Option<Style>,
    /// For each level above this row (1..depth): whether the ancestor
    /// there has siblings after it, so a line runs down past it.
    continues: Vec<bool>,
    band: bool,
    /// Where a text input starts on this row, from the tree's left edge.
    input_x: Option<u16>,
    /// It has children or is [`Node::foldable`].
    folds: bool,
}

/// A text input and where the terminal cursor goes while it is drawn.
type Input<'a> = (&'a TextInput, &'a Cell<Option<Position>>);

/// The tree, ready to draw: its nodes, its columns, its look, and an
/// optional heading row (what is over the names, then the column headings).
pub struct BandedTree<'a> {
    nodes: &'a [Node],
    columns: &'a Columns,
    look: TreeLook,
    header: Option<Vec<Span<'static>>>,
    focused: bool,
    input: Option<Input<'a>>,
}

impl<'a> BandedTree<'a> {
    pub fn new(nodes: &'a [Node], columns: &'a Columns, look: TreeLook) -> Self {
        Self {
            nodes,
            columns,
            look,
            header: None,
            focused: true,
            input: None,
        }
    }

    /// A text input drawn on the row of the node marked [`Node::input_at`],
    /// when that row is on screen, with the terminal cursor in it. The
    /// input starts where the words it replaces started, so the stripe and
    /// the tree line stay.
    pub fn input(mut self, input: &'a TextInput, cursor: &'a Cell<Option<Position>>) -> Self {
        self.input = Some((input, cursor));
        self
    }

    /// A heading row: `left` over the names, the column headings over theirs.
    pub fn header(mut self, left: Vec<Span<'static>>) -> Self {
        self.header = Some(left);
        self
    }

    /// Whether its pane has focus (the cursor is bold), default true.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Draw into `area`. Returns the rows' area (under the heading, the
    /// panel's width, only the lines that hold a row), for clicks: mark it
    /// with `Cx::hit` and [`row_at`] finds the row.
    pub fn render(
        self,
        theme: &Theme,
        area: Rect,
        buf: &mut Buffer,
        state: &mut TreeState<String>,
    ) -> Rect {
        let mut body = area;
        if let Some(left) = self.header.clone() {
            let w = area.width.saturating_sub(1) as usize;
            buf.set_line(area.x, area.y, &self.columns.header(theme, left, w), area.width);
            body = Rect {
                y: area.y + 1,
                height: area.height.saturating_sub(1),
                ..area
            };
        }
        // The last column holds the scrollbar.
        let width = body.width.saturating_sub(1) as usize;
        let opened = state.opened().clone();
        let mut looks = HashMap::new();
        let n = self.nodes.len();
        let items: Vec<TreeItem<'static, String>> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| {
                let mut path = Vec::new();
                self.item(
                    node,
                    Place {
                        depth: 0,
                        last: i + 1 == n,
                        index: i,
                        band: false,
                        stripe: node.stripe,
                    },
                    width,
                    &opened,
                    &mut path,
                    &mut Vec::new(),
                    &mut looks,
                )
            })
            .collect();
        let folding = looks.iter().filter(|(_, l)| l.folds).map(|(p, _)| p.clone()).collect();
        FOLDS.with(|f| f.borrow_mut().insert(folds_of(state), folding));
        let Ok(tree) = Tree::new(&items) else {
            return body;
        };
        let tree = tree
            .highlight_style(widgets::row_style(theme, self.focused, true))
            .node_closed_symbol("")
            .node_open_symbol("")
            .node_no_children_symbol("")
            .experimental_scrollbar(Some(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None)
                    .track_style(theme.s(Slot::BgOverlay))
                    .thumb_style(theme.s(Slot::AccentMuted)),
            ));
        StatefulWidget::render(tree, body, buf, state);
        self.guides(&items, &looks, body, buf, state);
        self.draw_input(theme, &items, &looks, body, buf, state);
        // Only the lines that hold a row: the space under the last one is
        // the pane's, not a row's (a click there selects nothing, and
        // nothing is lit under the pointer).
        let drawn = (body.y..body.bottom())
            .take_while(|&y| row_at(state, Position { x: body.x, y }).is_some())
            .count() as u16;
        Rect { height: drawn, ..body }
    }

    #[allow(clippy::too_many_arguments)]
    fn item(
        &self,
        node: &Node,
        at: Place,
        width: usize,
        opened: &HashSet<Vec<String>>,
        path: &mut Vec<String>,
        continues: &mut Vec<bool>,
        looks: &mut HashMap<Vec<String>, Look>,
    ) -> TreeItem<'static, String> {
        path.push(node.id.clone());
        let band = if at.depth == self.look.band_level {
            at.index % 2 == 1
        } else {
            at.band
        };
        let (lead, lead_style) = match (at.depth, at.stripe) {
            (0, Some(s)) => ("▌ ".to_string(), s),
            _ => (joint(at.depth, at.last), self.look.lines),
        };
        looks.insert(
            path.clone(),
            Look {
                depth: at.depth,
                stripe: at.stripe,
                continues: continues.clone(),
                band: at.depth >= self.look.band_level && at.depth > 0 && band,
                input_x: node
                    .input_at
                    .map(|x| (at.depth * 2 + super::width(&lead)) as u16 + x),
                folds: node.foldable || !node.children.is_empty(),
            },
        );
        let room = width
            .saturating_sub(at.depth * 2)
            .saturating_sub(super::width(&lead));
        let folded = (node.foldable || !node.children.is_empty()) && !opened.contains(path);
        // A row with no cells (a group's heading) runs across the columns.
        let none = Columns::default();
        let columns = if node.cells.is_empty() {
            &none
        } else {
            self.columns
        };
        let mut name = node.name.clone();
        // What the name has before the columns, a cell of air kept.
        let name_w = room.saturating_sub(columns.width() + 1);
        if let Some(d) = &node.detail {
            let used: usize = name.iter().map(Span::width).sum();
            if used + d.width() <= name_w {
                name.push(d.clone());
            }
        }
        if let Some((gives, chips)) = &node.chips {
            name = super::chips::after_name(name, *gives, chips.clone(), name_w);
        }
        if folded {
            name.push(Span::styled(" ⋯", self.look.folded));
        }
        let cells: Vec<(&str, Line<'static>)> = node
            .cells
            .iter()
            .map(|(h, s)| (h.as_str(), s.clone()))
            .collect();
        let line = columns.line_of(name, room, &cells);
        let mut spans = Vec::new();
        if !lead.is_empty() {
            spans.push(Span::styled(lead, lead_style));
        }
        spans.extend(line.spans);
        let line = Line::from(spans);
        let item = if node.children.is_empty() {
            TreeItem::new_leaf(node.id.clone(), line)
        } else {
            if at.depth >= 1 {
                continues.push(!at.last);
            }
            let m = node.children.len();
            let children = node
                .children
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    self.item(
                        c,
                        Place {
                            depth: at.depth + 1,
                            last: i + 1 == m,
                            index: i,
                            band,
                            stripe: at.stripe,
                        },
                        width,
                        opened,
                        path,
                        continues,
                        looks,
                    )
                })
                .collect();
            if at.depth >= 1 {
                continues.pop();
            }
            // Ids are unique among siblings; a clash shows the node bare.
            TreeItem::new(node.id.clone(), line.clone(), children)
                .unwrap_or_else(|_| TreeItem::new_leaf(node.id.clone(), line))
        };
        path.pop();
        item
    }

    /// The text input, on its row if that is shown.
    fn draw_input(
        &self,
        theme: &Theme,
        items: &[TreeItem<'static, String>],
        looks: &HashMap<Vec<String>, Look>,
        body: Rect,
        buf: &mut Buffer,
        state: &TreeState<String>,
    ) {
        let Some((input, cursor)) = self.input else {
            return;
        };
        let flat = state.flatten(items);
        for (row, f) in flat
            .iter()
            .skip(state.get_offset())
            .take(body.height as usize)
            .enumerate()
        {
            let Some(x) = looks.get(&f.identifier).and_then(|l| l.input_x) else {
                continue;
            };
            let rect = Rect {
                x: body.x + x,
                y: body.y + row as u16,
                // The last column holds the scrollbar.
                width: body.width.saturating_sub(x + 1),
                height: 1,
            };
            input.render(theme, rect, buf, cursor);
            return;
        }
    }

    /// Paint what the tree widget's indent leaves empty: the stripe, the
    /// lines past parents that go on, and the bands.
    fn guides(
        &self,
        items: &[TreeItem<'static, String>],
        looks: &HashMap<Vec<String>, Look>,
        body: Rect,
        buf: &mut Buffer,
        state: &TreeState<String>,
    ) {
        let flat = state.flatten(items);
        let selected = state.selected();
        let width = body.width.saturating_sub(1);
        for (row, f) in flat
            .iter()
            .skip(state.get_offset())
            .take(body.height as usize)
            .enumerate()
        {
            let Some(look) = looks.get(&f.identifier) else {
                continue;
            };
            let y = body.y + row as u16;
            if f.identifier != selected {
                let bg = match look.depth {
                    0 => self.look.heading,
                    _ if look.band => self.look.band,
                    _ => None,
                };
                if let Some(bg) = bg {
                    buf.set_style(
                        Rect {
                            x: body.x,
                            y,
                            width,
                            height: 1,
                        },
                        Style::new().bg(bg),
                    );
                }
            }
            if look.depth >= 1
                && let Some(stripe) = look.stripe
            {
                buf.set_string(body.x, y, "▌", stripe);
            }
            for (k, goes_on) in look.continues.iter().enumerate() {
                let level = k + 1;
                if *goes_on && level < look.depth {
                    buf.set_string(body.x + glyph_x(level), y, "│", self.look.lines);
                }
            }
        }
    }
}

#[derive(Clone, Copy)]
struct Place {
    depth: usize,
    last: bool,
    index: usize,
    band: bool,
    stripe: Option<Style>,
}

/// Open every node with children the first time it is seen (`seen` keeps
/// the ids down to each); after that, open and closed are the user's. With
/// nothing selected, the first row is.
pub fn open_new(state: &mut TreeState<String>, nodes: &[Node], seen: &mut HashSet<Vec<String>>) {
    fn walk(
        state: &mut TreeState<String>,
        nodes: &[Node],
        seen: &mut HashSet<Vec<String>>,
        path: &mut Vec<String>,
    ) {
        for n in nodes {
            path.push(n.id.clone());
            if !n.children.is_empty() {
                if seen.insert(path.clone()) {
                    state.open(path.clone());
                }
                walk(state, &n.children, seen, path);
            }
            path.pop();
        }
    }
    walk(state, nodes, seen, &mut Vec::new());
    if state.selected().is_empty()
        && let Some(first) = nodes.first()
    {
        state.select(vec![first.id.clone()]);
    }
}

/// The row drawn on the line at `at`, from the last frame, or none on the
/// empty space below the last row. (`TreeState::rendered_at` answers the
/// last row there too: it finds the row at or above the line. Rows are one
/// line high, so a line holds a row only when the line above holds another.)
pub fn row_at(state: &TreeState<String>, at: Position) -> Option<Vec<String>> {
    let here = state.rendered_at(at)?;
    let above = at
        .y
        .checked_sub(1)
        .and_then(|y| state.rendered_at(Position { x: at.x, y }));
    (above != Some(here)).then(|| here.to_vec())
}

thread_local! {
    /// The rows that fold (with children, or [`Node::foldable`]) of each
    /// tree as last drawn, by its state's address: what `+` and `-` look at.
    static FOLDS: std::cell::RefCell<HashMap<usize, HashSet<Vec<String>>>> = Default::default();
}

fn folds_of(state: &TreeState<String>) -> usize {
    std::ptr::from_ref(state) as usize
}

/// Whether the row at `path` folds, as the tree was last drawn.
fn folds(state: &TreeState<String>, path: &[String]) -> bool {
    FOLDS.with(|f| f.borrow().get(&folds_of(state)).is_some_and(|s| s.contains(path)))
}

/// `+`: open the selected row when it folds. True when it was closed.
pub fn expand(state: &mut TreeState<String>) -> bool {
    let at = state.selected().to_vec();
    folds(state, &at) && state.open(at)
}

/// `-`: close the selected row when it is open; on a row inside a group
/// (a leaf, or a group already closed), close that group and select it,
/// as file managers do. False when there was nothing to close.
pub fn collapse(state: &mut TreeState<String>) -> bool {
    let at = state.selected().to_vec();
    if folds(state, &at) && state.close(&at) {
        return true;
    }
    if at.len() < 2 {
        return false;
    }
    let parent = at[..at.len() - 1].to_vec();
    state.close(&parent);
    state.select(parent);
    true
}

/// Moving and folding: `↑↓ j k`, `g`/`Home`, `G`/`End`, `space` folds,
/// `+` (or `=`, no shift) opens the selected group, `-` closes it (or the
/// group the row is in). True when the key was one of them and did
/// something; `↑` on the first row is false, so the host can take it to
/// the top bar. Draw a frame first: the tree knows its rows only after it
/// was rendered.
pub fn key(state: &mut TreeState<String>, key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('+' | '=') => return expand(state),
        KeyCode::Char('-') => return collapse(state),
        _ => {}
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            state.key_down();
        }
        // On the first row `↑` is not the tree's: the host takes it up to
        // the top bar.
        KeyCode::Up | KeyCode::Char('k') => return state.key_up(),
        KeyCode::Home | KeyCode::Char('g') => {
            state.select_first();
        }
        KeyCode::End | KeyCode::Char('G') => {
            state.select_last();
        }
        KeyCode::Char(' ') => {
            state.toggle_selected();
        }
        _ => return false,
    }
    true
}
