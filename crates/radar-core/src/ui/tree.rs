//! The banded tree: a hierarchy drawn by `tui-tree-widget` with no fold
//! markers (no chevrons, no `[+]`). Tree lines join each row to its parent
//! (`├─`, `└─`, and `│` past a parent that is not the last), each top-level
//! group sits on the heading shade, every other block at the band level
//! sits on a band, a folded node shows `⋯ n` for its held descendants, and
//! named columns line up at the right ([`Columns`]). Selection changes the
//! ground without replacing semantic foreground colours.
//!
//! The tree indents each level by 2 cells; a row's own tree line starts its
//! text, so its room is taken before the name is fitted and the columns stay
//! aligned at every depth.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, StatefulWidget};
/// The tree's state (what is open and selected), so a part needs no
/// tree crate of its own.
pub use tui_tree_widget::TreeState;
use tui_tree_widget::{Tree, TreeItem};

use super::columns::Columns;
use crate::design::theme::{Slot, Tier};
use crate::input::TextInput;
use crate::theme::Theme;

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
    /// Legacy group colour, retained for source compatibility. Tree rails
    /// are no longer drawn; semantic colours belong on name/cell spans.
    pub stripe: Option<Style>,
    /// Chips after the name, and which name span gives way to them
    /// ([`super::chips::after_name`]).
    pub chips: Option<(usize, Vec<Span<'static>>)>,
    /// After the name, shown whole or not at all (a count the detail panel
    /// also has).
    pub detail: Option<Span<'static>>,
    /// Folds even with no children given (its fold is the part's). No
    /// hidden-descendant marker is shown until children are actually held.
    pub foldable: bool,
    /// A text input is drawn on this row, this many cells into its name
    /// (past the tree line the kit puts before it); see
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

    /// Can be opened even before children are supplied. It does not claim
    /// a hidden count when no children are held.
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
    /// The `⋯ n` of a folded node.
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

/// Descendants actually held by the view, including children of folded groups.
pub fn held_descendants(node: &Node) -> usize {
    node.children
        .iter()
        .map(|child| 1 + held_descendants(child))
        .sum()
}

/// Whether a row keeps its columns at this width (excluding the scrollbar).
/// A folded row sacrifices columns only when they would hide its signal.
pub fn row_columns_visible(
    depth: usize,
    width: usize,
    columns: usize,
    has_cells: bool,
    folded: bool,
) -> bool {
    let lead = if depth == 0 { 0 } else { depth + 2 };
    let room = width.saturating_sub(depth * 2).saturating_sub(lead);
    has_cells && !(folded && room <= columns.saturating_add(1))
}

/// A folded signal is either an exact count or just dots. Never clip a count
/// into a different number, or imply unfetched children for an empty group.
fn folded_marker(count: usize, room: usize) -> String {
    if count == 0 || room == 0 {
        return String::new();
    }
    let full = format!(" ⋯ {count}");
    if super::width(&full) <= room {
        full
    } else {
        "⋯".to_string()
    }
}

/// Tree selection changes only the ground. Semantic foregrounds and modifiers
/// survive; monochrome adds the non-colour focus and selection cues.
fn selected_style(theme: &Theme, focused: bool) -> Style {
    let mut style = theme.bg(if focused {
        Slot::BgSelection
    } else {
        Slot::BgOverlay
    });
    if theme.tier() == Tier::Mono {
        style = style.add_modifier(Modifier::UNDERLINED);
        if focused {
            style = style.add_modifier(Modifier::BOLD);
        }
    }
    style
}

/// Fit the widest spans first so a short semantic suffix can stay visible.
fn shrink(spans: &mut Vec<Span<'static>>, budget: usize) {
    const LEAST: usize = 4;
    loop {
        let total: usize = spans.iter().map(Span::width).sum();
        if total <= budget {
            return;
        }
        let excess = total - budget;
        let Some((i, w)) = spans
            .iter()
            .map(Span::width)
            .enumerate()
            .max_by_key(|(_, w)| *w)
            .filter(|(_, w)| *w > LEAST)
        else {
            if spans.len() > 1 {
                spans.pop();
                continue;
            }
            return;
        };
        let to = w.saturating_sub(excess).max(LEAST);
        let text = super::fit(spans[i].content.trim_end(), to);
        let style = spans[i].style;
        spans[i] = Span::styled(text.trim_end().to_string(), style);
        if spans[i].width() >= w {
            spans.truncate(i + 1);
            return;
        }
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
    cut_names: bool,
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
            cut_names: false,
            input: None,
        }
    }

    /// A text input drawn on the row of the node marked [`Node::input_at`],
    /// when that row is on screen, with the terminal cursor in it. The
    /// input starts where the words it replaces started, so the tree line stays.
    pub fn input(mut self, input: &'a TextInput, cursor: &'a Cell<Option<Position>>) -> Self {
        self.input = Some((input, cursor));
        self
    }

    /// A heading row: `left` over the names, the column headings over theirs.
    pub fn header(mut self, left: Vec<Span<'static>>) -> Self {
        self.header = Some(left);
        self
    }

    /// Whether its pane has focus, default true.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Shorten the widest name spans with an ellipsis before clipping. The
    /// folded marker, chips and columns keep their own reserved space.
    pub fn cut_names(mut self, cut: bool) -> Self {
        self.cut_names = cut;
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
        let area = area.intersection(buf.area);
        let mut body = area;
        if area.height > 0
            && let Some(left) = self.header.clone()
        {
            let w = area.width.saturating_sub(1) as usize;
            buf.set_line(
                area.x,
                area.y,
                &self.columns.header(theme, left, w),
                area.width,
            );
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
                        visible: true,
                    },
                    width,
                    &opened,
                    &mut path,
                    &mut Vec::new(),
                    &mut looks,
                )
            })
            .collect();
        let folding = looks
            .iter()
            .filter(|(_, l)| l.folds)
            .map(|(p, _)| p.clone())
            .collect();
        let mut paths = Vec::new();
        held_paths(self.nodes, &mut Vec::new(), &mut paths);
        KEYS.with(|k| {
            k.borrow_mut()
                .insert(folds_of(state), KeyRows { folding, paths })
        });
        let Ok(tree) = Tree::new(&items) else {
            return body;
        };
        let tree = tree
            .highlight_style(selected_style(theme, self.focused))
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
        // The widget's depth indent is not bounded by its area. A viewport
        // buffer keeps very deep rows from painting into a neighbouring pane.
        let mut viewport = Buffer::empty(body);
        for y in body.y..body.bottom() {
            for x in body.x..body.right() {
                viewport[(x, y)] = buf[(x, y)].clone();
            }
        }
        StatefulWidget::render(tree, body, &mut viewport, state);
        self.guides(&items, &looks, body, &mut viewport, state);
        self.draw_input(theme, &items, &looks, body, &mut viewport, state);
        for y in body.y..body.bottom() {
            for x in body.x..body.right() {
                buf[(x, y)] = viewport[(x, y)].clone();
            }
        }
        // Only the lines that hold a row: the space under the last one is
        // the pane's, not a row's (a click there selects nothing, and
        // nothing is lit under the pointer).
        let drawn = (body.y..body.bottom())
            .take_while(|&y| row_at(state, Position { x: body.x, y }).is_some())
            .count() as u16;
        Rect {
            height: drawn,
            ..body
        }
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
        let lead = joint(at.depth, at.last);
        looks.insert(
            path.clone(),
            Look {
                depth: at.depth,
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
        // Count only visible closed groups. Their subtrees are disjoint, so
        // all count traversals together visit at most the held nodes once.
        let count = if at.visible && !opened.contains(path) {
            held_descendants(node)
        } else {
            0
        };
        // A group's heading runs across the columns. At tiny widths a folded
        // signal takes precedence over columns that would leave no name room.
        let none = Columns::default();
        let columns = if row_columns_visible(
            at.depth,
            width,
            self.columns.width(),
            !node.cells.is_empty(),
            count > 0,
        ) {
            self.columns
        } else {
            &none
        };
        let names_w = room.saturating_sub(columns.width());
        let capacity = if count > 0 && room == 1 {
            1
        } else {
            names_w.saturating_sub(1)
        };
        let marker = folded_marker(count, capacity);
        let name_w = capacity.saturating_sub(super::width(&marker));
        let mut name = node.name.clone();
        if self.cut_names {
            shrink(&mut name, name_w);
        }
        if let Some(d) = &node.detail {
            let used: usize = name.iter().map(Span::width).sum();
            if used + d.width() <= name_w {
                name.push(d.clone());
            }
        }
        if let Some((gives, chips)) = &node.chips {
            name = super::chips::after_name(name, *gives, chips.clone(), name_w);
        }
        let cells: Vec<(&str, Line<'static>)> = node
            .cells
            .iter()
            .map(|(h, s)| (h.as_str(), s.clone()))
            .collect();
        let mut used = 0;
        let mut name: Vec<Span<'static>> = name
            .into_iter()
            .map(|span| {
                let (text, width) = super::take(&span.content, name_w.saturating_sub(used));
                used += width;
                Span::styled(text.to_string(), span.style)
            })
            .collect();
        if !marker.is_empty() {
            name.push(Span::styled(marker, self.look.folded));
        }
        let used: usize = name.iter().map(Span::width).sum();
        name.push(Span::raw(" ".repeat(names_w.saturating_sub(used))));
        name.extend(columns.cell_lines(&cells));
        let mut spans = Vec::new();
        if !lead.is_empty() {
            spans.push(Span::styled(lead, self.look.lines));
        }
        spans.extend(super::clip(name, room));
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
                            visible: at.visible && opened.contains(path),
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
            if x >= body.width.saturating_sub(1) {
                return;
            }
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

    /// Paint what the tree widget's indent leaves empty: the lines past
    /// parents that go on, and the bands.
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
            for (k, goes_on) in look.continues.iter().enumerate() {
                let level = k + 1;
                if *goes_on && level < look.depth && glyph_x(level) < width {
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
    visible: bool,
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
    let above =
        at.y.checked_sub(1)
            .and_then(|y| state.rendered_at(Position { x: at.x, y }));
    (above != Some(here)).then(|| here.to_vec())
}

#[derive(Default)]
struct KeyRows {
    folding: HashSet<Vec<String>>,
    /// Held rows in preorder, including rows behind a closed ancestor.
    paths: Vec<Vec<String>>,
}

thread_local! {
    /// The held rows of each tree as last drawn, by its state's address.
    static KEYS: std::cell::RefCell<HashMap<usize, KeyRows>> = Default::default();
}

fn held_paths(nodes: &[Node], path: &mut Vec<String>, out: &mut Vec<Vec<String>>) {
    for node in nodes {
        path.push(node.id.clone());
        out.push(path.clone());
        held_paths(&node.children, path, out);
        path.pop();
    }
}

fn folds_of(state: &TreeState<String>) -> usize {
    std::ptr::from_ref(state) as usize
}

/// Whether the row at `path` folds, as the tree was last drawn.
fn folds(state: &TreeState<String>, path: &[String]) -> bool {
    KEYS.with(|k| {
        k.borrow()
            .get(&folds_of(state))
            .is_some_and(|rows| rows.folding.contains(path))
    })
}

/// Reconcile buffered keys against current folds, not the widget's stale
/// list of rows from before the previous key closed or opened a group.
fn visible_paths(state: &TreeState<String>) -> Vec<Vec<String>> {
    KEYS.with(|k| {
        k.borrow()
            .get(&folds_of(state))
            .map(|rows| {
                rows.paths
                    .iter()
                    .filter(|path| {
                        (1..path.len()).all(|depth| state.opened().contains(&path[..depth]))
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    })
}

/// `+`: open the selected row when it folds. True when it was closed.
pub fn expand(state: &mut TreeState<String>) -> bool {
    let at = state.selected().to_vec();
    folds(state, &at) && state.open(at)
}

/// `=`: open every held group, including groups below a closed ancestor.
pub fn expand_all(state: &mut TreeState<String>) -> bool {
    let paths: Vec<Vec<String>> = KEYS.with(|k| {
        k.borrow()
            .get(&folds_of(state))
            .map(|rows| {
                rows.paths
                    .iter()
                    .filter(|path| rows.folding.contains(*path))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    });
    let mut changed = false;
    for path in paths {
        changed |= state.open(path);
    }
    changed
}

/// `-`: close the selected row when it is open; on a row inside a group
/// (a leaf, or a group already closed), close that group and select it.
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

/// Shared tree keys. Left/right stay in the tree at a boundary; Up/k retain
/// the top-bar escape. Draw once to supply the held rows.
pub fn key(state: &mut TreeState<String>, key: &KeyEvent) -> bool {
    if !key.modifiers.is_empty()
        && !(key.modifiers == KeyModifiers::SHIFT && matches!(key.code, KeyCode::Char(_)))
    {
        return false;
    }
    match key.code {
        KeyCode::Char('+') => return expand(state),
        KeyCode::Char('=') => return expand_all(state),
        KeyCode::Char('-') => return collapse(state),
        KeyCode::Char(' ') => {
            return folds(state, state.selected()) && state.toggle_selected();
        }
        KeyCode::Left => {
            let at = state.selected().to_vec();
            if !(folds(state, &at) && state.close(&at)) && at.len() > 1 {
                state.select(at[..at.len() - 1].to_vec());
            }
            return true;
        }
        KeyCode::Right => {
            if !expand(state) && state.opened().contains(state.selected()) {
                let at = state.selected();
                if let Some(child) = visible_paths(state)
                    .into_iter()
                    .find(|path| path.len() == at.len() + 1 && path.starts_with(at))
                {
                    state.select(child);
                }
            }
            return true;
        }
        _ => {}
    }
    let rows = visible_paths(state);
    let index = rows.iter().position(|path| path == state.selected());
    let next = match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            index.map_or(0, |i| (i + 1).min(rows.len().saturating_sub(1)))
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if index == Some(0) {
                return false;
            }
            index.map_or(rows.len().saturating_sub(1), |i| i.saturating_sub(1))
        }
        KeyCode::Home | KeyCode::Char('g') => 0,
        KeyCode::End | KeyCode::Char('G') => rows.len().saturating_sub(1),
        _ => return false,
    };
    if let Some(path) = rows.get(next) {
        state.select(path.clone());
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::Column;

    fn node(id: &str) -> Node {
        Node::new(id, vec![Span::raw(id.to_string())])
    }

    fn path(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    fn render(
        nodes: &[Node],
        columns: &Columns,
        state: &mut TreeState<String>,
        width: u16,
        focused: bool,
    ) -> Buffer {
        let theme = Theme::new(Tier::TrueColor);
        let area = Rect::new(0, 0, width, 6);
        let mut buf = Buffer::empty(area);
        BandedTree::new(nodes, columns, TreeLook::new(&theme))
            .focused(focused)
            .cut_names(true)
            .render(&theme, area, &mut buf, state);
        buf
    }

    fn press(state: &mut TreeState<String>, code: KeyCode) -> bool {
        key(state, &KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn folded_counts_include_held_grandchildren_and_never_guess_unloaded_rows() {
        let group = node("watch").children([
            node("streets").child(node("alleys").child(node("lanes"))),
            node("river"),
        ]);
        assert_eq!(held_descendants(&group), 4);
        assert_eq!(held_descendants(&node("unseen").foldable()), 0);
        assert_eq!(folded_marker(4, 4), " ⋯ 4");
        assert_eq!(folded_marker(12, 5), " ⋯ 12");
        for width in 1..5 {
            assert_eq!(folded_marker(12, width), "⋯", "never a partial count");
        }
        assert_eq!(folded_marker(12, 0), "");
        assert_eq!(folded_marker(0, 12), "");
        let mut state = TreeState::default();
        let buf = render(&[group], &Columns::default(), &mut state, 24, true);
        assert!(crate::shell::row_text(&buf, 0).contains("watch ⋯ 4"));
    }

    #[test]
    fn reserved_fold_count_survives_names_chips_columns_and_unicode() {
        let group = Node::new(
            "watch",
            vec![Span::raw("街路 e\u{301} 🎉 a very long name")],
        )
        .stripe(Style::new().fg(Color::Red))
        .detail(Span::raw(" long optional detail"))
        .chips(0, vec![Span::raw(" [WATCH-12]")])
        .cell("state", Span::raw("ready"))
        .children((0..12).map(|n| node(&format!("held-{n}"))));
        let columns = Columns::new([Column::new("state", 6)]);
        for width in [0, 1, 2, 3, 4, 5, 8, 9, 13, 20, 40, 60] {
            let mut state = TreeState::default();
            let buf = render(
                std::slice::from_ref(&group),
                &columns,
                &mut state,
                width,
                true,
            );
            if width == 0 {
                continue;
            }
            let text = crate::shell::row_text(&buf, 0);
            assert!(!text.contains('▌'), "{width}: {text}");
            assert!(
                crate::ui::width(&text) <= width.saturating_sub(1) as usize,
                "{width}: {text}"
            );
            if width >= 2 {
                assert!(
                    text.contains('⋯'),
                    "folded signal must survive: {width}: {text}"
                );
            }
            let room = width.saturating_sub(1) as usize;
            let cols = if row_columns_visible(0, room, 6, true, true) {
                6
            } else {
                0
            };
            let capacity = if room == 1 {
                1
            } else {
                room.saturating_sub(cols + 1)
            };
            if capacity >= 5 {
                assert!(text.contains("⋯ 12"), "exact held count: {width}: {text}");
            } else {
                assert!(!text.contains("⋯ 1"), "no truncated count: {width}: {text}");
            }
        }
    }

    #[test]
    fn a_deep_narrow_tree_never_paints_outside_its_pane() {
        let theme = Theme::new(Tier::TrueColor);
        let nodes = [node("watch").child(node("streets").children([
            node("alleys").child(node("lanes").child(node("doors"))),
            node("square"),
        ]))];
        let mut state = TreeState::default();
        open_new(&mut state, &nodes, &mut HashSet::new());
        let area = Rect::new(2, 1, 3, 6);
        let mut buf = Buffer::empty(Rect::new(0, 0, 14, 8));
        for y in 0..8 {
            buf.set_string(0, y, "~~~~~~~~~~~~~~", Style::new());
        }
        BandedTree::new(&nodes, &Columns::default(), TreeLook::new(&theme))
            .render(&theme, area, &mut buf, &mut state);
        for y in 0..8 {
            for x in 0..14 {
                if !area.contains(Position { x, y }) {
                    assert_eq!(buf[(x, y)].symbol(), "~", "outside pane at {x},{y}");
                }
            }
        }
    }

    #[test]
    fn tree_selection_keeps_semantic_colours_and_heading_hover_precedence() {
        let theme = Theme::new(Tier::TrueColor);
        let name = theme.s(Slot::StatusError).add_modifier(Modifier::ITALIC);
        let cell = theme
            .s(Slot::StatusWarning)
            .add_modifier(Modifier::UNDERLINED);
        let nodes = [node("watch").stripe(theme.s(Slot::GlowPurple)).children([
            Node::new("streets", vec![Span::styled("N", name)])
                .cell("state", Span::styled("C", cell)),
            node("river"),
        ])];
        let columns = Columns::new([Column::new("state", 6)]);
        for focused in [true, false] {
            let mut state = TreeState::default();
            state.open(path(&["watch"]));
            state.select(path(&["watch", "streets"]));
            let mut buf = render(&nodes, &columns, &mut state, 24, focused);
            let selected = theme
                .bg(if focused {
                    Slot::BgSelection
                } else {
                    Slot::BgOverlay
                })
                .bg
                .unwrap();
            assert_eq!(
                buf[(0, 0)].bg,
                theme.bg(Slot::BgOverlay).bg.unwrap(),
                "heading shade"
            );
            assert_eq!(
                buf[(5, 1)].fg,
                name.fg.unwrap(),
                "selected semantic name colour"
            );
            assert_eq!(buf[(5, 1)].bg, selected);
            assert!(buf[(5, 1)].modifier.contains(Modifier::ITALIC));
            assert!(
                !buf[(5, 1)].modifier.contains(Modifier::BOLD),
                "selection adds no colour-tier emphasis"
            );
            assert_eq!(
                buf[(17, 1)].fg,
                cell.fg.unwrap(),
                "selected semantic cell colour"
            );
            assert!(buf[(17, 1)].modifier.contains(Modifier::UNDERLINED));
            crate::ui::hover::paint(&mut buf, Rect::new(0, 0, 23, 2), &theme);
            assert_eq!(
                buf[(0, 0)].bg,
                theme.bg(Slot::BgHover).bg.unwrap(),
                "hover covers heading"
            );
            if focused {
                assert_eq!(
                    buf[(5, 1)].bg,
                    selected,
                    "focused selection wins over hover"
                );
            } else {
                assert_eq!(
                    buf[(5, 1)].bg,
                    theme.bg(Slot::BgHover).bg.unwrap(),
                    "hover identifies the unfocused row"
                );
            }
            assert_eq!(
                buf[(5, 1)].fg,
                name.fg.unwrap(),
                "hover preserves semantic colour"
            );
        }
        let mono = Theme::new(Tier::Mono);
        assert!(
            selected_style(&mono, true)
                .add_modifier
                .contains(Modifier::BOLD | Modifier::UNDERLINED)
        );
        assert_eq!(
            selected_style(&mono, false).add_modifier,
            Modifier::UNDERLINED
        );
    }

    #[test]
    fn legacy_tree_keys_reconcile_buffered_folds_and_open_all_held_groups() {
        let nodes = [
            node("watch").children([
                node("streets").child(node("alleys").child(node("lanes"))),
                node("river"),
            ]),
            node("unseen").foldable(),
        ];
        let mut state = TreeState::default();
        state.select(path(&["watch"]));
        let _ = render(&nodes, &Columns::default(), &mut state, 30, true);
        assert!(press(&mut state, KeyCode::Right));
        assert!(state.opened().contains(&path(&["watch"])));
        assert!(press(&mut state, KeyCode::Right));
        assert_eq!(
            state.selected(),
            path(&["watch", "streets"]),
            "no redraw between keys"
        );
        assert!(press(&mut state, KeyCode::Char('=')));
        for group in [
            path(&["watch"]),
            path(&["watch", "streets"]),
            path(&["watch", "streets", "alleys"]),
            path(&["unseen"]),
        ] {
            assert!(state.opened().contains(&group), "{group:?}");
        }
        assert!(
            !state.opened().contains(&path(&["watch", "river"])),
            "leaves never open"
        );
        assert!(press(&mut state, KeyCode::Left));
        assert_eq!(state.selected(), path(&["watch", "streets"]));
        assert!(press(&mut state, KeyCode::Down));
        assert_eq!(
            state.selected(),
            path(&["watch", "river"]),
            "collapsed descendants disappear immediately"
        );
        assert!(press(&mut state, KeyCode::Left));
        assert_eq!(state.selected(), path(&["watch"]));
        assert!(
            state.opened().contains(&path(&["watch"])),
            "left selects parent without closing it"
        );
        assert!(press(&mut state, KeyCode::Left));
        assert!(
            press(&mut state, KeyCode::Left),
            "left boundary remains tree-owned"
        );
        assert_eq!(state.selected(), path(&["watch"]));
        assert!(
            !press(&mut state, KeyCode::Up),
            "up at first row reaches the top bar"
        );
        assert!(!key(
            &mut state,
            &KeyEvent::new(KeyCode::Right, KeyModifiers::ALT)
        ));
        assert!(!key(
            &mut state,
            &KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT)
        ));
    }
}
