//! A snapshot per element at 80 and 160 wide, rendered without colour (a
//! snapshot proves layout, not colour), and the behaviour that layout
//! cannot show.

use std::collections::HashSet;

use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use tui_tree_widget::TreeState;

use super::card::{card, panel};
use super::chips::{after_name, chips};
use super::columns::{Column, Columns};
use super::entries::{Entries, Entry};
use super::gallery::Gallery;
use super::stories;
use super::log::{self, LogLine, LogState, LogText, LogView};
use super::meter::{Stage, ratio, spark, sparkline, stages};
use super::page::PageFrame;
use super::rows::{ListRow, Move, RowList, add_row, kv, labelled, select, settle};
use super::scroll::{self, Scroll};
use super::states::{busy, fetching, set_up};
use super::status::{Check, strip};
use super::table::Table;
use super::tree::{self, BandedTree, Node, TreeLook};
use super::*;
use crate::design::theme::{Slot, Tier};
use crate::plugin::Tone;
use crate::theme::Theme;

fn mono() -> Theme {
    Theme::new(Tier::Mono)
}

fn text(buf: &Buffer) -> String {
    let a = buf.area;
    (a.y..a.y + a.height)
        .map(|y| crate::shell::row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Draw with `f` into a `w` x `h` buffer and return it as text.
fn draw(w: u16, h: u16, f: impl FnOnce(&Theme, Rect, &mut Buffer)) -> String {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    f(&mono(), area, &mut buf);
    text(&buf)
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn sample_nodes(t: &Theme) -> Vec<Node> {
    let leaf = |id: &str, name: &str| {
        Node::new(id, vec![Span::raw(name.to_string())])
            .cell("state", Span::raw("open"))
            .cell("owner", Span::raw("alice"))
            .cell("size", Span::raw("12"))
            .cell("updated", Span::raw("3d"))
    };
    vec![
        Node::new("a", vec![Span::raw("A north-wing")])
            .stripe(t.s(Slot::GlowPurple))
            .children([
                Node::new("atlases", vec![Span::raw("atlases")])
                    .cell("state", Span::raw("1 item"))
                    .child(leaf("104", "#104 sort the atlas box by decade")),
                Node::new("maps", vec![Span::raw("maps")]).children([
                    leaf("187", "#187 label the map drawer").child(leaf("1", "#1 a loose map")),
                    leaf("198", "#198 count the postcards"),
                ]),
                Node::new("ledgers", vec![Span::raw("ledgers")]).cell("state", Span::raw("—")),
            ]),
        Node::new("b", vec![Span::raw("B south-wing")])
            .stripe(t.s(Slot::GlowCyan))
            .child(Node::new("globes", vec![Span::raw("globes")]).child(leaf("342", "#342 move the globes"))),
    ]
}

fn columns() -> Columns {
    Columns::new([
        Column::new("state", 10).keep(5),
        Column::new("owner", 12).keep(2),
        Column::new("size", 8).keep(1),
        Column::new("updated", 9).keep(4),
    ])
}

fn tree_text(w: u16, h: u16, keys: &[KeyCode]) -> String {
    let t = mono();
    let nodes = sample_nodes(&t);
    let mut state = TreeState::default();
    let mut seen = HashSet::new();
    let render = |state: &mut TreeState<String>| {
        let area = Rect::new(0, 0, w, h);
        let mut buf = Buffer::empty(area);
        let cols = columns().fit(w as usize - 1, 24);
        BandedTree::new(&nodes, &cols, TreeLook::new(&t))
            .header(vec![Span::raw(" HELD BY everyone")])
            .render(&t, area, &mut buf, state);
        text(&buf)
    };
    tree::open_new(&mut state, &nodes, &mut seen);
    let _ = render(&mut state);
    for k in keys {
        tree::key(&mut state, &press(*k));
        let _ = render(&mut state);
    }
    render(&mut state)
}

/// A list's panel: its rows above, its footer at the bottom.
fn foot_text(w: u16, h: u16, words: &str) -> String {
    let t = mono();
    let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
    let inner = panel(&t, buf.area, &mut buf, "views", true);
    let footer = Line::styled(words.to_string(), t.s(Slot::TextSecondary));
    let list = super::card::footer(&t, inner, &mut buf, "shows", vec![footer], 3);
    let rows = ["Everything", "Mine", "Waiting"].map(|r| ListRow::new(Line::from(r)));
    RowList::new(rows.to_vec())
        .cursor(1)
        .render(&t, buf.area, list, &mut buf, &Scroll::default());
    text(&buf)
}

#[test]
fn list_footer_24() {
    insta::assert_snapshot!(foot_text(24, 14, "every shelf you opened, newest first"));
}

#[test]
fn a_shows_foot_takes_the_lines_its_words_need_and_nothing_for_none() {
    let t = mono();
    let draw = |lines: Vec<Line<'static>>| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
        let inner = panel(&t, buf.area, &mut buf, "list", true);
        let left = super::card::shows(&t, inner, &mut buf, lines, 3);
        (text(&buf), left.height)
    };
    // Words that fit: the rule, one line, the border.
    let (out, left) = draw(vec![Line::from("every shelf you opened")]);
    let rows: Vec<&str> = out.lines().collect();
    assert!(rows[7].starts_with("│─ shows ─"), "{out}");
    assert!(rows[8].contains("every shelf you opened"), "{out}");
    assert_eq!(left, 6);
    // Words that wrap take two lines.
    let (out, left) = draw(vec![Line::from(
        "every shelf you opened, newest first · and the ones others lent you",
    )]);
    let rows: Vec<&str> = out.lines().collect();
    assert!(rows[6].starts_with("│─ shows ─"), "{out}");
    assert_eq!(left, 5);
    // Nothing to say: the rows keep the panel.
    let (out, left) = draw(vec![]);
    assert!(!out.contains("shows"), "{out}");
    assert_eq!(left, 8);
}

#[test]
fn a_list_footer_stays_at_the_foot_cuts_with_dots_and_gives_way_when_short() {
    let long = "every shelf you opened, newest first, and the ones others lent you this week";
    let out = foot_text(24, 14, long);
    let rows: Vec<&str> = out.lines().collect();
    // The rule sits a fixed distance from the bottom, whatever the rows.
    assert!(rows[9].starts_with("│─ shows ─"), "{out}");
    assert!(rows[12].trim_end_matches('│').trim_end().ends_with('…'), "{out}");
    // The rows stay above it.
    assert!(rows[2].contains("Mine"), "{out}");
    // Too short for rows and a footer: the rows keep the panel.
    let short = foot_text(24, 6, long);
    assert!(!short.contains("shows"), "{short}");
}

#[test]
fn a_cut_footer_puts_its_dots_right_after_the_last_words() {
    let t = mono();
    let mut buf = Buffer::empty(Rect::new(0, 0, 30, 12));
    let inner = Rect::new(0, 0, 30, 12);
    let lines = ["one", "two", "three", "four"].map(Line::from).to_vec();
    super::card::footer(&t, inner, &mut buf, super::card::SHOWS, lines, 2);
    let last = crate::shell::row_text(&buf, 11);
    assert_eq!(last.trim_end(), " two…", "{last:?}");
}

#[test]
fn the_part_wide_words_read_alike() {
    use super::connection::{every, signed_in};
    use super::status::sync_words;
    assert_eq!(every(60), "every 1m");
    assert_eq!(every(120), "every 2m");
    assert_eq!(every(90), "every 90s");
    assert_eq!(every(7200), "every 2h");
    assert_eq!(sync_words("Notes", true, Some(0), 10, Some(60)), "syncing Notes · every 1m");
    assert_eq!(sync_words("Notes", false, Some(0), 4, None), "synced 4s ago");
    assert_eq!(sync_words("Notes", false, None, 4, Some(120)), "not synced yet · every 2m");
    let t = mono();
    let text = |s: Vec<Span<'static>>| s.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert_eq!(text(signed_in(&t, "Alice Example", "alice")), "Alice Example  alice");
    assert_eq!(text(signed_in(&t, "", "alice")), "alice");
    assert_eq!(text(signed_in(&t, "alice", "alice")), "alice");
    let line = super::states::sync_failed(&t, "acme", "401 unauthorized");
    assert!(text(line.spans).ends_with("acme could not sync: 401 unauthorized"));
}

#[test]
fn plus_and_equals_open_a_group_minus_closes_it_or_the_one_a_row_is_in() {
    let t = mono();
    let nodes = sample_nodes(&t);
    let mut state = TreeState::default();
    tree::open_new(&mut state, &nodes, &mut HashSet::new());
    let draw = |state: &mut TreeState<String>| {
        let area = Rect::new(0, 0, 80, 14);
        let mut buf = Buffer::empty(area);
        BandedTree::new(&nodes, &columns(), TreeLook::new(&t)).render(&t, area, &mut buf, state);
    };
    let path = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let key = |state: &mut TreeState<String>, c: char| {
        draw(state);
        tree::key(state, &press(KeyCode::Char(c)))
    };
    // On an open group: `-` closes it, `+` and `=` open it.
    state.select(path(&["a", "maps"]));
    assert!(key(&mut state, '-'));
    assert!(!state.opened().contains(&path(&["a", "maps"])));
    assert!(!key(&mut state, '-') || state.selected() == path(&["a"]), "closed: `-` goes on to its group");
    state.select(path(&["a", "maps"]));
    state.open(path(&["a"]));
    assert!(key(&mut state, '+'));
    assert!(state.opened().contains(&path(&["a", "maps"])));
    assert!(!key(&mut state, '+'), "open already");
    assert!(key(&mut state, '-'));
    assert!(key(&mut state, '='), "`=` is `+` without shift");
    assert!(state.opened().contains(&path(&["a", "maps"])));
    // On a row inside a group: `-` closes the group and selects it.
    state.select(path(&["a", "maps", "198"]));
    assert!(key(&mut state, '-'));
    assert_eq!(state.selected(), path(&["a", "maps"]));
    assert!(!state.opened().contains(&path(&["a", "maps"])));
    // On a row with nothing to fold, `+` does nothing; a top-level row
    // that does not fold has no group to close either.
    state.open(path(&["a", "maps"]));
    state.select(path(&["a", "maps", "198"]));
    assert!(!key(&mut state, '+'));
    assert!(!state.opened().contains(&path(&["a", "maps", "198"])), "a leaf is never opened");
    let flat = vec![Node::new("x", vec![Span::raw("x")])];
    let mut lone = TreeState::default();
    lone.select(path(&["x"]));
    let area = Rect::new(0, 0, 40, 4);
    BandedTree::new(&flat, &columns(), TreeLook::new(&t)).render(&t, area, &mut Buffer::empty(area), &mut lone);
    assert!(!tree::key(&mut lone, &press(KeyCode::Char('-'))));
    assert!(!tree::key(&mut lone, &press(KeyCode::Char('+'))));
    // Space still toggles.
    state.select(path(&["a", "maps"]));
    assert!(key(&mut state, ' '));
    assert!(!state.opened().contains(&path(&["a", "maps"])));
}

#[test]
fn tree_hit_area_ends_at_the_last_actual_row() {
    use ratatui::layout::Position;

    let t = mono();
    let cols = Columns::default();
    let nodes = [
        Node::new("a", vec![Span::raw("first")])
            .child(Node::new("child", vec![Span::raw("child")])),
        Node::new("b", vec![Span::raw("last")]),
    ];
    let mut state = TreeState::default();
    tree::open_new(&mut state, &nodes, &mut HashSet::new());
    let area = Rect::new(5, 3, 20, 8);
    let mut buf = Buffer::empty(area);
    let rows = BandedTree::new(&nodes, &cols, TreeLook::new(&t))
        .header(vec![Span::raw("heading")])
        .render(&t, area, &mut buf, &mut state);
    assert_eq!(rows, Rect::new(5, 4, 20, 3));
    for (y, path) in [(4, vec!["a"]), (5, vec!["a", "child"]), (6, vec!["b"])] {
        for x in [rows.x, rows.right() - 1] {
            assert_eq!(
                tree::row_at(&state, Position { x, y }),
                Some(path.iter().map(|s| s.to_string()).collect())
            );
        }
    }
    for at in [
        Position {
            x: rows.x,
            y: area.y,
        },
        Position {
            x: rows.x - 1,
            y: rows.y,
        },
        Position {
            x: rows.right(),
            y: rows.y,
        },
        Position {
            x: rows.x,
            y: rows.bottom(),
        },
        Position {
            x: rows.x,
            y: area.bottom() - 1,
        },
        Position {
            x: rows.x,
            y: area.bottom(),
        },
    ] {
        assert!(!rows.contains(at), "not a clickable row: {at:?}");
        assert!(
            tree::row_at(&state, at).is_none(),
            "not an actual row: {at:?}"
        );
    }

    // Closing a group shortens the next frame's hit area too.
    state.close(&["a".to_string()]);
    let rows =
        BandedTree::new(&nodes, &cols, TreeLook::new(&t)).render(&t, area, &mut buf, &mut state);
    assert_eq!(rows, Rect::new(5, 3, 20, 2));
    assert!(
        tree::row_at(
            &state,
            Position {
                x: rows.x,
                y: rows.bottom()
            }
        )
        .is_none()
    );
}

#[test]
fn tree_hit_area_tracks_empty_scrolled_and_tiny_frames() {
    use ratatui::layout::Position;

    let t = mono();
    let cols = Columns::default();
    let nodes = [
        Node::new("a", vec![Span::raw("first")]),
        Node::new("b", vec![Span::raw("last")]),
    ];
    let mut state = TreeState::default();
    let old = Rect::new(5, 3, 20, 8);
    BandedTree::new(&nodes, &cols, TreeLook::new(&t)).render(
        &t,
        old,
        &mut Buffer::empty(old),
        &mut state,
    );
    state.select(vec!["b".to_string()]);
    let small = Rect::new(1, 1, 10, 1);
    let rows = BandedTree::new(&nodes, &cols, TreeLook::new(&t)).render(
        &t,
        small,
        &mut Buffer::empty(small),
        &mut state,
    );
    assert_eq!(rows, small);
    assert_eq!(
        tree::row_at(&state, Position { x: 1, y: 1 }),
        Some(vec!["b".to_string()])
    );
    assert!(
        tree::row_at(&state, Position { x: old.x, y: old.y }).is_none(),
        "old coordinates are stale"
    );

    let rows = BandedTree::new(&[], &cols, TreeLook::new(&t)).render(
        &t,
        old,
        &mut Buffer::empty(old),
        &mut state,
    );
    assert_eq!(rows.height, 0);
    assert!(tree::row_at(&state, Position { x: old.x, y: old.y }).is_none());

    for area in [
        Rect::new(0, 0, 1, 1),
        Rect::new(0, 0, 0, 1),
        Rect::new(0, 0, 1, 0),
    ] {
        let rows = BandedTree::new(&nodes, &cols, TreeLook::new(&t)).render(
            &t,
            area,
            &mut Buffer::empty(area),
            &mut state,
        );
        assert_eq!(rows.height, u16::from(!area.is_empty()), "{area:?}");
        assert_eq!(
            tree::row_at(&state, Position { x: 0, y: 0 }).is_some(),
            !area.is_empty(),
            "{area:?}"
        );
    }
    let area = Rect::new(0, 0, 10, 1);
    let rows = BandedTree::new(&nodes, &cols, TreeLook::new(&t))
        .header(vec![Span::raw("heading")])
        .render(&t, area, &mut Buffer::empty(area), &mut state);
    assert_eq!(rows.height, 0, "a heading alone has no row targets");
    assert!(tree::row_at(&state, Position { x: 0, y: 0 }).is_none());
}

#[test]
fn tree_80() {
    insta::assert_snapshot!(tree_text(80, 14, &[]));
}

#[test]
fn tree_160() {
    insta::assert_snapshot!(tree_text(160, 14, &[]));
}

#[test]
fn tree_lines_join_rows_to_their_parents_and_columns_line_up() {
    let out = tree_text(100, 14, &[]);
    let rows: Vec<&str> = out.lines().collect();
    // Stripe on every row of a group, its heading included.
    assert!(rows[1].starts_with("▌ A north-wing"), "{out}");
    assert!(rows[2].starts_with("▌ ├─ atlases"), "{out}");
    // Under a box that is not the last, the line runs down past it.
    assert!(rows[3].starts_with("▌ │  └─ #104"), "{out}");
    // Three deep: a line past the box, and one past the item above, each
    // under its parent's name.
    assert!(rows[5].starts_with("▌ │  ├─ #187"), "{out}");
    assert!(rows[6].starts_with("▌ │  │  └─ #1 a loose map"), "{out}");
    assert!(rows[7].starts_with("▌ │  └─ #198"), "{out}");
    // The last box: no line past it.
    assert!(rows[8].starts_with("▌ └─ ledgers"), "{out}");
    // Every row's state column starts under its heading.
    let col = rows[0].find("STATE").expect("a heading");
    for r in &rows[1..10] {
        let cells: String = r.chars().skip(col).take(4).collect();
        assert!(
            cells.trim().is_empty() || !cells.starts_with(' '),
            "column out of line: {r}"
        );
    }
}

#[test]
fn a_folded_node_shows_dots_and_space_folds() {
    // Down onto atlases, fold it.
    let out = tree_text(100, 14, &[KeyCode::Down, KeyCode::Char(' ')]);
    assert!(out.contains("├─ atlases ⋯"), "{out}");
    assert!(!out.contains("#104"), "{out}");
}

/// One group over one row, `w` wide, with the state column.
fn one_row(w: u16, group: Node, row: Node, open: bool) -> String {
    draw(w, 3, |t, area, buf| {
        let nodes = vec![group.child(row)];
        let cols = Columns::new([Column::new("state", 10)]);
        let mut state = TreeState::default();
        if open {
            state.open(vec![nodes[0].id.clone()]);
        }
        BandedTree::new(&nodes, &cols, TreeLook::new(t)).render(t, area, buf, &mut state);
    })
}

/// A tree of an add row, a striped group and the group's own add row, with
/// a text input `abc` on the node `on`, `w` wide, `h` tall; the text and
/// where the terminal cursor went.
fn input_tree(on: &str, h: u16, w: u16) -> (String, Option<ratatui::layout::Position>) {
    let t = mono();
    let at = |id: &str, x: u16, node: Node| if id == on { node.input_at(x) } else { node };
    let nodes = vec![
        at("add", 3, Node::new("add", vec![Span::raw(" + add a group")])),
        at("g", 2, Node::new("g", vec![Span::raw("● ab north-wing")]))
            .stripe(t.s(Slot::GlowPurple))
            .children([
                at("g/add", 2, Node::new("add", vec![Span::raw("+ add a box")])),
                Node::new("b", vec![Span::raw("a box")]),
            ]),
    ];
    let cols = Columns::default();
    let mut state = TreeState::default();
    state.open(vec!["g".to_string()]);
    let mut input = crate::input::TextInput::new("abc");
    input.key(press(KeyCode::End));
    let cursor = std::cell::Cell::new(None);
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    BandedTree::new(&nodes, &cols, TreeLook::new(&t))
        .input(&input, &cursor)
        .render(&t, area, &mut buf, &mut state);
    (text(&buf), cursor.get())
}

#[test]
fn a_text_input_goes_on_a_tree_row_past_its_stripe_and_tree_line() {
    // On the top add row: after ` + `.
    let (out, cursor) = input_tree("add", 5, 30);
    let row = out.lines().next().unwrap();
    assert_eq!(row, " + abc", "{out}");
    assert_eq!(cursor.map(|p| (p.x, p.y)), Some((6, 0)));
    // On a striped group: after the stripe and `● `, over its name.
    let (out, cursor) = input_tree("g", 5, 30);
    assert_eq!(out.lines().nth(1).unwrap(), "▌ ● abc", "{out}");
    assert_eq!(cursor.map(|p| (p.x, p.y)), Some((7, 1)));
    // On the group's add row, one level down: past the stripe's lead and `├─ + `.
    let (out, cursor) = input_tree("g/add", 5, 30);
    let row = out.lines().nth(2).unwrap();
    assert_eq!(row, "▌ ├─ + abc", "{out}");
    assert_eq!(cursor.map(|p| (p.x, p.y)), Some((10, 2)), "{out}");
    // A row not shown draws no input and sets no cursor.
    let (out, cursor) = input_tree("g/add", 2, 30);
    assert!(!out.contains("abc") && cursor.is_none(), "{out}");
    // Nothing marked: the tree is as it was.
    let (out, cursor) = input_tree("none", 5, 30);
    assert!(!out.contains("abc") && cursor.is_none(), "{out}");
}

#[test]
fn a_heading_with_no_cells_runs_across_the_columns() {
    let group = Node::new("g", vec![Span::raw("G north-wing   3 boxes · 4 items · 1 lost")]);
    let row = Node::new("r", vec![Span::raw("atlases")]).cell("state", Span::raw("open"));
    // 40 wide: the scrollbar's column and a cell of air are all it leaves.
    let out = one_row(40, group, row, true);
    assert!(out.contains("G north-wing   3 boxes · 4 items · 1 l\n"), "{out}");
    assert!(out.contains("atlases"), "{out}");
}

#[test]
fn a_rows_detail_shows_whole_or_not_at_all_and_chips_follow_the_name() {
    let row = |name: &str| {
        Node::new("r", vec![Span::raw(name.to_string())])
            .detail(Span::raw("   12 items"))
            .chips(0, vec![Span::raw(" "), Span::raw("[ABC-1]")])
            .cell("state", Span::raw("open"))
    };
    let out = one_row(60, Node::new("g", vec![Span::raw("G")]), row("atlases"), true);
    assert!(out.contains("atlases   12 items [ABC-1]"), "{out}");
    // Too narrow for the detail: it goes, the chips stay while the name keeps 12.
    let out = one_row(48, Node::new("g", vec![Span::raw("G")]), row("sort the atlas box by decade"), true);
    assert!(!out.contains("items"), "{out}");
    assert!(out.contains("[ABC-1]"), "{out}");
}

#[test]
fn a_foldable_node_shows_dots_while_closed_with_no_children_given() {
    let leaf = Node::new("r", vec![Span::raw("atlases")]).cell("state", Span::raw("open"));
    let out = draw(40, 2, |t, area, buf| {
        let nodes = vec![Node::new("g", vec![Span::raw("G north-wing")]).foldable(), leaf];
        let cols = Columns::new([Column::new("state", 10)]);
        BandedTree::new(&nodes, &cols, TreeLook::new(t)).render(t, area, buf, &mut TreeState::default());
    });
    assert!(out.contains("G north-wing ⋯"), "{out}");
    assert!(!out.contains("atlases ⋯"), "a plain leaf has nothing folded: {out}");
}

#[test]
fn a_table_cuts_a_long_value_with_dots_and_takes_lines_of_spans() {
    let cols = Columns::new([Column::flex("name", 8), Column::new("owner", 8)]);
    let rows = vec![vec![
        ("name".to_string(), Line::from(vec![Span::raw("sort the atlas box by decade"), Span::raw(" [A-1]")])),
        ("owner".to_string(), Line::raw("alexandra")),
    ]];
    let out = draw(30, 4, |t, area, buf| {
        assert_eq!(Table::flex_width(&cols, area.width), 20);
        Table::lines(&cols, rows).render(t, area, area, buf, &Scroll::new());
    });
    assert!(out.contains("sort the atlas box … alexan…"), "{out}");
}

#[test]
fn a_list_rows_detail_is_dropped_whole_when_it_does_not_fit() {
    let rows = |w: u16| {
        draw(w, 1, |t, area, buf| {
            let row = ListRow::new(Line::raw("lost"))
                .detail(Span::raw("  an item nobody can find on its shelf"))
                .right(Span::raw("warning"));
            RowList::new(vec![row]).render(t, area, area, buf, &Scroll::new());
        })
    };
    assert!(rows(60).contains("lost  an item nobody can find on its shelf"), "{}", rows(60));
    let narrow = rows(40);
    assert!(!narrow.contains("shelf"), "{narrow}");
    assert!(narrow.contains("lost") && narrow.ends_with("warning"), "{narrow}");
}

#[test]
fn a_column_is_found_where_its_row_draws_it() {
    let cols = Columns::new([Column::new("state", 10), Column::new("note", 6)]);
    // Columns ending at x 40: state from 24 (9 wide, then air), note from 34.
    assert_eq!(cols.at("state", 40), Some((24, 9)));
    assert_eq!(cols.at("note", 40), Some((34, 5)));
    assert_eq!(cols.at("missing", 40), None);
    let line = cols.line(vec![Span::raw("name")], 40, &[("note", Span::raw("N-1"))]);
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(&text[34..37], "N-1");
}

#[test]
fn columns_drop_the_least_kept_first_and_keep_their_order() {
    let all = columns();
    let names = |c: &Columns| c.iter().map(|c| c.heading.clone()).collect::<Vec<_>>();
    assert_eq!(names(&all.fit(200, 24)), ["state", "owner", "size", "updated"]);
    assert_eq!(names(&all.fit(60, 24)), ["state", "owner", "updated"]);
    assert_eq!(names(&all.fit(50, 24)), ["state", "updated"]);
    assert_eq!(names(&all.fit(10, 24)), ["state"]);
    let extra = [Column::new("tracker", 10)];
    assert_eq!(
        names(&all.fit(200, 24).with_extra(&extra, 200, 28)),
        ["state", "owner", "size", "tracker", "updated"]
    );
    assert_eq!(names(&all.with_extra(&extra, 60, 28)), names(&all));
}

fn table_text(w: u16, row: usize) -> String {
    draw(w, 10, |t, area, buf| {
        let inner = panel(t, area, buf, "items", true);
        let cols = Columns::new([
            Column::flex("name", 20),
            Column::new("state", 10).keep(5),
            Column::new("owner", 12).keep(2),
            Column::new("updated", 9).keep(4),
        ])
        .fit(inner.width as usize - 2, 0);
        let rows = (0..12)
            .map(|i| {
                vec![
                    ("name".to_string(), Span::raw(format!("#{i} item number {i} with a long name"))),
                    ("state".to_string(), Span::raw("open")),
                    ("owner".to_string(), Span::raw("alice")),
                    ("updated".to_string(), Span::raw("3d")),
                ]
            })
            .collect();
        Table::new(&cols, rows)
            .cursor(row)
            .render(t, area, inner, buf, &Scroll::new());
    })
}

#[test]
fn table_80() {
    insta::assert_snapshot!(table_text(80, 9));
}

#[test]
fn table_160() {
    insta::assert_snapshot!(table_text(160, 0));
}

#[test]
fn a_table_keeps_its_cursor_in_view_and_says_when_it_is_empty() {
    let out = table_text(80, 11);
    assert!(out.contains("#11 item"), "{out}");
    assert!(!out.contains("#0 item"), "{out}");
    let cols = Columns::new([Column::flex("name", 10)]);
    let out = draw(40, 5, |t, area, buf| {
        Table::new(&cols, vec![]).empty("No items yet.").render(t, area, area, buf, &Scroll::new());
    });
    assert!(out.contains("No items yet."), "{out}");
}

fn page_text(w: u16) -> String {
    draw(w, 8, |t, area, buf| {
        let frame = PageFrame {
            crumbs: vec![Span::raw("Shelves"), Span::raw("a"), Span::raw("maps"), Span::raw("#187")],
            title: Span::raw("Label the map drawer so the maps can be found again"),
            details: vec![Span::raw("Alice E."), Span::raw(" · "), Span::raw("shelf-4 → archive")],
            under: Some(Line::raw("kept at  /work/north/maps · on shelf-4")),
            tabs: vec!["Overview".into(), "Changes".into(), super::page::Tab { label: "Notes".into(), attention: true }, "Steps".into()],
            tab: 1,
            focused: false,
        };
        let areas = frame.render(t, area, buf);
        let inner = panel(t, areas.body, buf, "overview", true);
        buf.set_line(inner.x + 1, inner.y, &Line::raw(format!("tabs at {:?}", areas.tabs.iter().map(|r| r.x).collect::<Vec<_>>())), inner.width);
    })
}

#[test]
fn page_frame_80() {
    insta::assert_snapshot!(page_text(80));
}

#[test]
fn page_frame_160() {
    insta::assert_snapshot!(page_text(160));
}

fn card_lines(t: &Theme, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![Line::raw("Sort the atlas box by decade"), Line::from(chips(t, &[("needs you", Tone::Warn), ("ready", Tone::Good)]))];
    lines.extend(kv(t, vec![("owner", vec![Span::raw("alice")]), ("where", vec![Span::raw("north / shelf-4")])]));
    lines.push(labelled(t, "notes", "2, 1 open"));
    lines.extend(wrapped(
        "The decades from 1900 to 1950 are done. The rest wait for the new dividers, which come on Thursday. Two atlases have loose maps.",
        width,
        Span::raw("│ "),
        t.s(Slot::TextPrimary),
    ));
    for i in 0..10 {
        lines.push(Line::raw(format!("line {i}")));
    }
    lines
}

fn card_text(w: u16, scroll: &Scroll) -> String {
    draw(w, 10, |t, area, buf| {
        card(t, area, buf, "north-wing #104", true, scroll, card_lines(t, w - 4));
    })
}

/// A run of six steps in every state, and a stop with a long name.
fn rail_text(w: u16) -> String {
    use crate::view::chrome::StopState as S;
    use super::rail::{Stop, render};
    let stop = |label, state, note| Stop { label, icon: Some(icon::Icon::Board), state, note };
    let stops = [
        stop("look & terminal", S::Done, Some("neon · nerd icons")),
        stop("local code", S::Skipped, Some("skipped")),
        stop("Shelves", S::Current, Some("step 3 of 5")),
        stop("the Unseen University archive", S::Upcoming, None),
        stop("Ledger", S::Upcoming, None),
        stop("tour?", S::Upcoming, Some("asked at the end")),
    ];
    draw(w, 2, |t, area, buf| {
        let hits = render(t, &stops, area, buf);
        assert_eq!(hits.len(), stops.len(), "every stop can be clicked");
        assert!(hits.iter().all(|(r, _)| r.right() <= area.right()));
    })
}

#[test]
fn rail_80() {
    insta::assert_snapshot!(rail_text(80));
}

#[test]
fn rail_160() {
    insta::assert_snapshot!(rail_text(160));
}

#[test]
fn card_80() {
    insta::assert_snapshot!(card_text(80, &Scroll::new()));
}

#[test]
fn card_160() {
    insta::assert_snapshot!(card_text(160, &Scroll::new()));
}

#[test]
fn a_card_scrolls_within_what_it_holds() {
    let s = Scroll::new();
    let _ = card_text(80, &s);
    assert!(s.more());
    s.page(true);
    let out = card_text(80, &s);
    assert!(!out.contains("Sort the atlas"), "{out}");
    s.key(&press(KeyCode::Char('G')));
    let out = card_text(80, &s);
    assert!(out.contains("line 9"), "{out}");
    s.by(100);
    assert_eq!(s.at(), s.max());
    s.top();
    assert!(card_text(80, &s).contains("Sort the atlas"));
}

fn meters_text(w: u16) -> String {
    draw(w, 9, |t, _, buf| {
        let st = [
            Stage::new("fetch", Mark::Done, t.s(Slot::StatusSuccess)),
            Stage::new("sort", Mark::Failed, t.s(Slot::StatusError)),
            Stage::new("check", Mark::Running, t.s(Slot::StatusInfo)),
            Stage::new("file", Mark::Waiting, t.s(Slot::TextMuted)),
        ];
        for (i, l) in stages(&st, w).iter().enumerate() {
            buf.set_line(0, i as u16, l, w);
        }
        ratio(t, Rect::new(0, 3, w.min(40), 1), buf, "approved", 0.5, Tone::Warn);
        sparkline(t, Rect::new(0, 4, 24, 2), buf, &[1, 2, 3, 4, 8, 6, 5, 3, 2, 7, 9, 4], Tone::Info);
        buf.set_line(0, 6, &Line::from(vec![Span::raw("runs "), spark(t, &[1, 5, 9, 3, 0], 10, Tone::Info)]), w);
        buf.set_line(0, 7, &strip(t, &[("4 open".into(), Tone::Plain), ("1 needs you".into(), Tone::Warn)]), w);
        buf.set_line(0, 8, &busy(t, 0, "asking the server…"), w);
    })
}

#[test]
fn meters_80() {
    insta::assert_snapshot!(meters_text(80));
}

#[test]
fn meters_160() {
    insta::assert_snapshot!(meters_text(160));
}

fn rows_text(w: u16, cursor: usize, focused: bool) -> String {
    draw(w, 8, |t, area, buf| {
        let inner = panel(t, area, buf, "settings", focused);
        let rows = vec![
            ListRow::new(add_row(t, "add a shelf", "paste a link to a shelf, or its path", inner.width)),
            ListRow::new(widgets_kv(t, " server", "https://shelves.example")).right(Check::Passed { said: "ok".into(), ago: None }.mark(t).unwrap()),
            ListRow::new(widgets_kv(t, " token", "env:SHELF_TOKEN")).right(Check::Running.mark(t).unwrap()),
            ListRow::new(widgets_kv(t, " refresh", "every 1m")),
        ];
        RowList::new(rows)
            .cursor(cursor)
            .focused(focused)
            .render(t, area, inner, buf, &Scroll::new());
    })
}

#[test]
fn a_detail_shows_whole_or_not_at_all() {
    let list = |w: u16| {
        draw(w, 3, |t, area, buf| {
            let inner = panel(t, area, buf, "rows", true);
            let row = ListRow::new(Line::raw("server"))
                .detail(Span::raw("  shelves.example · 2 shelves"))
                .right(Span::raw("✓"));
            RowList::new(vec![row]).render(t, area, inner, buf, &Scroll::new());
        })
    };
    assert!(list(60).contains("server  shelves.example · 2 shelves"), "{}", list(60));
    let narrow = list(30);
    assert!(narrow.contains("server") && !narrow.contains("shelves") && narrow.contains('✓'), "{narrow}");

    let tree = |w: u16| {
        let nodes = vec![Node::new("a", vec![Span::raw("A")]).child(
            Node::new("x", vec![Span::raw("XYZ")]).detail(Span::raw("  read from field 10042")).cell("state", Span::raw("✓")),
        )];
        let cols = Columns::new([Column::new("state", 4)]);
        let mut state = TreeState::default();
        tree::open_new(&mut state, &nodes, &mut HashSet::new());
        draw(w, 3, |t, area, buf| {
            BandedTree::new(&nodes, &cols, TreeLook::new(t)).render(t, area, buf, &mut state);
        })
    };
    assert!(tree(60).contains("XYZ  read from field 10042"), "{}", tree(60));
    let narrow = tree(24);
    assert!(narrow.contains("XYZ") && !narrow.contains("read") && narrow.contains('✓'), "{narrow}");
}

/// Rows 0 and 3 are headings, as in a list grouped under words.
fn grouped(t: &Theme) -> Vec<ListRow> {
    let row = |s: &str| ListRow::new(Line::raw(s.to_string()));
    let head = |s: &str| ListRow::heading(crate::widgets::heading(t, s));
    vec![head("first"), row("one"), row("two"), head("second"), row("three"), row("four")]
}

#[test]
fn the_cursor_steps_over_headings_in_every_move() {
    let rows = grouped(&mono());
    let can = |i: usize| rows[i].selectable;
    let go = |from, mv| select(rows.len(), from, mv, can);
    assert_eq!(go(1, Move::Down), 2);
    assert_eq!(go(2, Move::Down), 4, "down over the heading");
    assert_eq!(go(4, Move::Up), 2, "up over the heading");
    assert_eq!(go(1, Move::Up), 1, "nowhere above the first, a heading does not count");
    assert_eq!(go(5, Move::Down), 5, "the end stays");
    assert_eq!(go(4, Move::First), 1, "home is the first row that can be selected");
    assert_eq!(go(1, Move::Last), 5);
    assert_eq!(go(1, Move::PageDown(2)), 4, "a page that ends on a heading goes on");
    assert_eq!(go(1, Move::PageDown(20)), 5);
    assert_eq!(go(5, Move::PageUp(2)), 2, "up by a page, off the heading");
    assert_eq!(go(5, Move::PageUp(20)), 1);
    assert_eq!(go(4, Move::PageUp(1)), 2, "a page up from just under a heading");
    // Nothing to select: it stays where it was.
    assert_eq!(select(2, 0, Move::Down, |_| false), 0);
    assert_eq!(select(0, 0, Move::Down, |_| true), 0);
    assert_eq!(settle(rows.len(), 0, can), 1, "a cursor on a heading rests below it");
    assert_eq!(settle(rows.len(), 3, can), 4);
    assert_eq!(settle(2, 1, |i| i == 0), 0, "or above it when nothing follows");
}

#[test]
fn keys_ask_for_moves_and_a_page_is_one_short_of_the_height() {
    assert_eq!(Move::from_key(&press(KeyCode::Char('j')), 10), Some(Move::Down));
    assert_eq!(Move::from_key(&press(KeyCode::Up), 10), Some(Move::Up));
    assert_eq!(Move::from_key(&press(KeyCode::Home), 10), Some(Move::First));
    assert_eq!(Move::from_key(&press(KeyCode::Char('G')), 10), Some(Move::Last));
    assert_eq!(Move::from_key(&press(KeyCode::PageDown), 10), Some(Move::PageDown(9)));
    assert_eq!(Move::from_key(&press(KeyCode::PageUp), 0), Some(Move::PageUp(1)));
    assert_eq!(Move::from_key(&press(KeyCode::Char('x')), 10), None);
}

#[test]
fn a_heading_row_is_drawn_but_has_no_click_area_and_the_cursor_is_not_put_on_it() {
    let t = mono();
    let area = Rect::new(0, 0, 40, 8);
    let mut buf = Buffer::empty(area);
    let inner = panel(&t, area, &mut buf, "grouped", true);
    let shown = RowList::new(grouped(&t)).cursor(3).render(&t, area, inner, &mut buf, &Scroll::new());
    let at: Vec<usize> = shown.iter().map(|(_, i)| *i).collect();
    assert_eq!(at, vec![1, 2, 4, 5], "headings are not clickable");
    let drawn = text(&buf);
    assert!(drawn.contains("FIRST") && drawn.contains("SECOND"), "{drawn}");
    // The row under the heading holds the cursor: it is underlined in mono.
    let y = shown.iter().find(|(_, i)| *i == 4).unwrap().0.y;
    let cursor_row = |y: u16| (inner.x..inner.x + inner.width).any(|x| buf[(x, y)].modifier.contains(ratatui::style::Modifier::UNDERLINED));
    assert!(cursor_row(y), "the cursor settled on the next row");
    assert!(!cursor_row(y - 1), "the heading has no cursor");
}

#[test]
fn a_list_draws_its_scrollbar_on_the_edge_around_the_inner_area_it_is_given() {
    let many = || (0..20).map(|i| ListRow::new(Line::raw(format!("row {i}")))).collect::<Vec<_>>();
    let area = Rect::new(0, 0, 30, 8);
    // The panel drawn by the list's caller, with its frame known.
    let by_frame = draw(30, 8, |t, area, buf| {
        let inner = panel(t, area, buf, "rows", true);
        RowList::new(many()).cursor(10).render(t, area, inner, buf, &Scroll::new());
    });
    // The panel drawn by somebody else: only its inner area is known.
    let by_inner = draw(30, 8, |t, area, buf| {
        let inner = panel(t, area, buf, "rows", true);
        RowList::new(many()).cursor(10).render_in(t, inner, buf, &Scroll::new());
    });
    assert_eq!(by_inner, by_frame);
    let edge = by_inner.lines().filter(|l| l.ends_with('█') || l.ends_with('║')).count();
    assert!(edge > 0, "a thumb and its track sit on the border:\n{by_inner}");
    assert_eq!(scroll::around(Rect::new(1, 1, 28, 6)), area);
    // Everything fits: no bar.
    let fits = draw(30, 8, |t, area, buf| {
        let inner = panel(t, area, buf, "rows", true);
        RowList::new(many().into_iter().take(3).collect()).render_in(t, inner, buf, &Scroll::new());
    });
    assert!(!fits.contains('█') && !fits.contains('║'), "{fits}");
}

fn widgets_kv(t: &Theme, label: &str, value: &str) -> Line<'static> {
    crate::widgets::kv(t, label, 12, vec![Span::raw(value.to_string())])
}

#[test]
fn rows_80() {
    insta::assert_snapshot!(rows_text(80, 1, true));
}

#[test]
fn rows_160() {
    insta::assert_snapshot!(rows_text(160, 1, false));
}

fn connection_text(w: u16) -> String {
    draw(w, 24, |t, area, buf| {
        let top = Rect { height: 11, ..area };
        let inner = panel(t, top, buf, "connection", true);
        RowList::new(super::gallery::settings_rows(t, inner.width))
            .cursor(4)
            .focused(true)
            .render(t, top, inner, buf, &Scroll::new());
        let lines = super::gallery::sample_found().detail(t);
        let bottom = Rect { y: 11, height: 13, ..area };
        let inner = panel(t, bottom, buf, &super::gallery::sample_found().title(), false);
        use ratatui::widgets::{Paragraph, Widget, Wrap};
        Paragraph::new(lines).wrap(Wrap { trim: false }).render(inner, buf);
    })
}

#[test]
fn connection_80() {
    insta::assert_snapshot!(connection_text(80));
}

#[test]
fn connection_160() {
    insta::assert_snapshot!(connection_text(160));
}

#[test]
fn a_found_variable_is_asked_about_by_name_and_its_names_give_way_when_narrow() {
    let found = connection::Found {
        what: "a shelf site".into(),
        names: vec![
            ("SHELF_X_BASE_URL".into(), "the site's address".into()),
            ("SHELF_X_USERNAME".into(), "the account".into()),
            ("SHELF_X_API_TOKEN".into(), "the token".into()),
        ],
        file: None,
        does: "adds the site x".into(),
        note: None,
    };
    assert_eq!(
        found.question(),
        "Found in your environment: a shelf site (SHELF_X_BASE_URL, SHELF_X_USERNAME, SHELF_X_API_TOKEN). Use it?"
    );
    let t = mono();
    let wide = found.row(&t, 120).to_string();
    assert_eq!(wide, format!(" ? {}", found.question()));
    let rows = |w: u16| found.row(&t, w).to_string();
    assert_eq!(rows(80), " ? Found in your environment: a shelf site (SHELF_X_BASE_URL, …). Use it?");
    assert_eq!(rows(70), " ? Found: a shelf site (SHELF_X_BASE_URL, …). Use it?");
    assert_eq!(rows(54), " ? Found in your environment: a shelf site. Use it?");
    assert_eq!(rows(40), " ? Found: a shelf site. Use it?");
    for w in [40, 54, 70, 80, 120] {
        assert!(rows(w).chars().count() + 2 <= w as usize || w == 40, "{w}: {}", rows(w));
    }
    let detail: Vec<String> = found.detail(&t).iter().map(|l| l.to_string()).collect();
    assert!(detail.iter().any(|l| l.contains("SHELF_X_API_TOKEN") && l.contains("the token")), "{detail:?}");
    assert!(detail.iter().any(|l| l.contains("never shows the values or saves them")), "{detail:?}");
    assert!(detail.iter().any(|l| l == "radar adds the site x."), "{detail:?}");
}

#[test]
fn a_token_is_shown_by_name_or_masked_never_whole() {
    use connection::masked;
    assert_eq!(masked("", &[]), "—");
    assert_eq!(masked("env:SHELF_TOKEN", &[]), "env:SHELF_TOKEN");
    assert_eq!(masked("shelfcli", &["shelfcli"]), "shelfcli");
    assert_eq!(masked("shelfcli", &[]), "typed in, not shown");
    assert_eq!(masked("shpat-abcdefghijklmnop", &[]), "shpa…mnop (typed in)");
}

fn states_text(w: u16) -> String {
    draw(w, 11, |t, area, buf| {
        let inner = panel(t, Rect { height: 7, ..area }, buf, "set up shelves", true);
        set_up(t, inner, buf, "Shelves", "It needs a server and a token.", Some("Press ⏎ to set it up in settings, under shelves."));
        let checks = [
            Check::NotYet.line(t, "not checked yet · c checks it", ""),
            Check::Running.line(t, "", "asking the server…"),
            Check::Failed { said: "the server said 401".into(), ago: Some("2m".into()) }.line(t, "", ""),
            fetching(t, true, false, None).unwrap(),
        ];
        for (i, l) in checks.iter().enumerate() {
            buf.set_line(1, 7 + i as u16, l, w - 2);
        }
    })
}

#[test]
fn states_80() {
    insta::assert_snapshot!(states_text(80));
}

#[test]
fn states_160() {
    insta::assert_snapshot!(states_text(160));
}

#[test]
fn chips_give_way_only_while_the_title_keeps_twelve_cells() {
    let t = mono();
    let name = vec![Span::raw("#12 "), Span::raw("a title that is quite long indeed"), Span::raw("  alice")];
    let c = chips(&t, &[("ABC-1 · doing", Tone::Warn)]);
    let out = after_name(name.clone(), 1, c.clone(), 50);
    assert_eq!(out.len(), 4);
    assert!(out[1].content.ends_with('…'));
    assert_eq!(after_name(name.clone(), 1, c, 30), name);
}

#[test]
fn text_helpers() {
    assert_eq!(fit("abcdef", 4), "abc…");
    assert_eq!(fit("ab", 4), "ab  ");
    assert_eq!(short_name("Alice Bea Example"), "Alice E.");
    assert_eq!(short_name("Maria de Souza"), "Maria de Souza");
    assert_eq!(short_name("Jan van der Berg"), "Jan van der Berg");
    assert_eq!(short_name("Alice"), "Alice");
    assert_eq!(detail_width(160), 48);
    assert_eq!(detail_width(80), 38);
    assert_eq!(detail_width(400), 60);
    assert_eq!(duration(3725), "1h 02m");
    assert_eq!(duration(372), "6m 12s");
    assert_eq!(clip(vec![Span::raw("abc"), Span::raw("def")], 4).len(), 2);
}

fn entry_list(t: &Theme) -> Vec<Entry> {
    let under = |s: &str| Line::raw(format!("  │ {s}"));
    vec![
        Entry::new("✗ label   ran out of labels")
            .before(vec![Line::raw("WHY IT STOPPED")])
            .body(vec![under("no label left for box 17"), under("the drawer holds 0 of 40")]),
        Entry::new("✗ file    timed out").body(vec![under("shelf 4 did not answer")]),
        Entry::new(Line::styled("✓ fetch   1m 10s", t.s(Slot::TextPrimary)))
            .before(vec![Line::default(), Line::raw("EVERY STEP")]),
        Entry::new("✓ sort    3m 02s"),
    ]
}

fn entries_text(w: u16, h: u16, cursor: usize, scroll: &Scroll) -> String {
    draw(w, h, |t, area, buf| {
        let inner = panel(t, area, buf, "why the sort stopped", true);
        Entries::new(entry_list(t)).cursor(cursor).render(t, area, inner, buf, scroll);
    })
}

fn diff_text(w: u16, h: u16, cursor: usize) -> String {
    let lines = diff::parse(gallery::SAMPLE_DIFF);
    let state = diff::DiffState::new();
    state.set(cursor);
    draw(w, h, |t, area, buf| {
        let inner = panel(t, area, buf, "shelf/Shelves.java", true);
        let under = vec![
            Line::raw("● open · bob · 2h ago"),
            Line::raw(format!("│ {}", "Should the atlas be cached? ".repeat(4))),
        ];
        diff::DiffView::new(&lines).under(7, under).render(t, area, inner, buf, &state);
    })
}

#[test]
fn diff_80() {
    insta::assert_snapshot!(diff_text(80, 20, 7));
}

#[test]
fn diff_160() {
    insta::assert_snapshot!(diff_text(160, 22, 15));
}

#[test]
fn entries_80() {
    insta::assert_snapshot!(entries_text(80, 12, 0, &Scroll::new()));
}

#[test]
fn entries_160() {
    insta::assert_snapshot!(entries_text(160, 12, 1, &Scroll::new()));
}

#[test]
fn entries_keep_the_cursors_lines_in_view_and_click_on_any_of_them() {
    let scroll = Scroll::new();
    // Six rows inside the border: the last entry pulls the view down.
    let text = entries_text(40, 8, 3, &scroll);
    assert!(text.contains("✓ sort") && !text.contains("WHY IT STOPPED"), "{text}");
    // Back to the first: its heading comes with it only when scrolled there.
    let text = entries_text(40, 8, 1, &scroll);
    assert!(text.contains("✗ file") && text.contains("shelf 4 did not answer"), "{text}");
    let hits = {
        let area = Rect::new(0, 0, 40, 12);
        let mut buf = Buffer::empty(area);
        let t = mono();
        let inner = panel(&t, area, &mut buf, "x", true);
        Entries::new(entry_list(&t)).render(&t, area, inner, &mut buf, &Scroll::new())
    };
    // Heading lines are not entries; the body lines click their entry.
    assert_eq!(hits.iter().map(|(_, i)| *i).collect::<Vec<_>>(), [0, 0, 0, 1, 1, 2, 3]);
    let empty = draw(40, 4, |t, area, buf| {
        let inner = panel(t, area, buf, "x", true);
        Entries::new(vec![]).empty("No failures.").render(t, area, inner, buf, &Scroll::new());
    });
    assert!(empty.contains("No failures."), "{empty}");
}

fn log_sample() -> LogText {
    let mut lines: Vec<LogLine> = vec![
        LogLine::ansi("\u{1b}[36;1mStarting\u{1b}[0m"),
        LogLine::plain("fetching box 1"),
        LogLine::plain("fetching box 2"),
        LogLine::ansi("\u{1b}[32;1m$ sort\u{1b}[0m"),
    ];
    for i in 1..=20 {
        lines.push(LogLine::plain(format!("box {i}: sorted")));
    }
    lines.push(LogLine::ansi("\u{1b}[31;1merror: no label left for box 17\u{1b}[0m"));
    let n = lines.len();
    LogText {
        sections: vec![
            log::Section { title: "Getting the boxes".into(), detail: "4s".into(), start: 1, end: 3, folded: true },
            log::Section { title: "Sorting".into(), detail: "2m 04s".into(), start: 3, end: n, folded: false },
        ],
        lines,
    }
}

fn log_text(w: u16, h: u16, text: &LogText, state: &LogState) -> String {
    draw(w, h, |t, area, buf| {
        let inner = panel(t, area, buf, "log", true);
        LogView { text, state, focused: true, empty: "Nothing yet." }.render(t, area, inner, buf);
    })
}

#[test]
fn log_80() {
    let (text, state) = (log_sample(), LogState::new());
    insta::assert_snapshot!(log_text(80, 12, &text, &state));
}

#[test]
fn log_160() {
    let (text, state) = (log_sample(), LogState::new());
    state.key(&text, &press(KeyCode::Char('g')));
    state.search(&text, "box 1");
    insta::assert_snapshot!(log_text(160, 12, &text, &state));
}

#[test]
fn a_log_starts_at_its_end_folds_sections_and_goes_from_match_to_match() {
    let (text, state) = (log_sample(), LogState::new());
    let shown = log_text(60, 10, &text, &state);
    assert!(shown.contains("error: no label left for box 17"), "at the end:\n{shown}");
    // The first section starts folded: its title, the dots, no lines.
    state.key(&text, &press(KeyCode::Char('g')));
    let shown = log_text(60, 10, &text, &state);
    assert!(shown.contains("─ Getting the boxes · 4s ⋯ 2 lines"), "{shown}");
    assert!(!shown.contains("fetching box 1"), "{shown}");
    // space on its title opens it; its lines show under a gutter.
    state.key(&text, &press(KeyCode::Down));
    state.key(&text, &press(KeyCode::Char(' ')));
    let shown = log_text(60, 10, &text, &state);
    assert!(shown.contains("┌ Getting the boxes · 4s") && shown.contains("│ fetching box 1"), "{shown}");
    // On a line, space folds the section around it and lands on its title.
    state.key(&text, &press(KeyCode::Down));
    state.key(&text, &press(KeyCode::Char(' ')));
    assert_eq!(state.line(&text), Some(1));
    assert!(log_text(60, 10, &text, &state).contains("⋯ 2 lines"));
    // A search lands on the first match from the cursor; n and N go round.
    assert_eq!(state.search(&text, "BOX 1"), 13, "box 1 and 10 to 19, and box 17's error, any case");
    assert_eq!(state.line(&text), Some(1), "inside the folded section: it opens");
    assert_eq!(state.found(&text), (1, 13));
    state.key(&text, &press(KeyCode::Char('N')));
    assert_eq!(state.line(&text), Some(24), "previous from the first is the last");
    state.key(&text, &press(KeyCode::Char('n')));
    assert_eq!(state.line(&text), Some(1));
    state.clear_search();
    assert!(!state.key(&text, &press(KeyCode::Char('n'))), "no search, n is not the log's");
    // G is the end again; the cursor there sees the top was not reached.
    state.key(&text, &press(KeyCode::Char('G')));
    assert!(!state.at_top(&text));
    state.key(&text, &press(KeyCode::Home));
    assert!(state.at_top(&text));
}

#[test]
fn plus_opens_a_logs_section_and_minus_folds_the_one_the_cursor_is_in() {
    let (text, state) = (log_sample(), LogState::new());
    state.key(&text, &press(KeyCode::Char('g')));
    state.key(&text, &press(KeyCode::Down));
    assert!(!state.key(&text, &press(KeyCode::Char('-'))), "folded already");
    assert!(state.key(&text, &press(KeyCode::Char('+'))));
    assert!(log_text(60, 10, &text, &state).contains("│ fetching box 1"));
    assert!(!state.key(&text, &press(KeyCode::Char('='))), "open already");
    state.key(&text, &press(KeyCode::Down));
    assert!(state.key(&text, &press(KeyCode::Char('-'))), "from a line inside it");
    assert_eq!(state.line(&text), Some(1), "on its title");
    assert!(log_text(60, 10, &text, &state).contains("⋯ 2 lines"));
    assert!(state.key(&text, &press(KeyCode::Char('='))));
}

#[test]
fn a_log_keeps_its_place_when_earlier_lines_arrive_and_wraps_long_lines() {
    let text = log_sample();
    let state = LogState::new();
    state.goto(&text, 10);
    let mut more = LogText {
        lines: vec![LogLine::plain("an earlier line"), LogLine::plain("another")],
        ..LogText::default()
    };
    more.lines.extend(text.lines.clone());
    more.sections = text
        .sections
        .iter()
        .map(|s| log::Section { start: s.start + 2, end: s.end + 2, ..s.clone() })
        .collect();
    state.shift(2);
    assert_eq!(state.line(&more), Some(12));
    let long = LogText { lines: vec![LogLine::plain("x".repeat(70))], sections: vec![] };
    let shown = log_text(40, 6, &long, &LogState::new());
    assert_eq!(shown.matches('x').count(), 70, "every character, over two rows:\n{shown}");
}

#[test]
fn terminal_colours_become_inks_and_other_escapes_go() {
    let l = LogLine::ansi("\u{1b}[0K\u{1b}[31;1mERROR:\u{1b}[0m done\u{1b}]8;;http://x\u{7}\tok\r");
    assert_eq!(l.text(), "ERROR: done    ok");
    assert_eq!(l.pieces[0], log::Piece { text: "ERROR:".into(), ink: log::Ink::Error, bold: true });
    assert_eq!(l.pieces[1].ink, log::Ink::Plain);
    assert!(!l.pieces[1].bold);
    let c = LogLine::ansi("\u{1b}[38;5;196mred\u{1b}[90m grey\u{1b}[39m plain");
    assert_eq!(c.text(), "red grey plain");
    assert_eq!(c.pieces[1].ink, log::Ink::Muted);
    assert_eq!(c.pieces[2].ink, log::Ink::Plain);
}

fn gallery_text(w: u16, h: u16, tab: usize) -> String {
    let mut g = Gallery::new();
    for _ in 0..tab {
        g.key(press(KeyCode::Char(']')));
    }
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    g.render(&mono(), area, &mut buf, "colours: none");
    // Twice: the tree opens and selects on its first frame.
    g.render(&mono(), area, &mut buf, "colours: none");
    text(&buf)
}

/// The gallery page `tab` as each line's runs and their styles, at the
/// full-colour tier: colours by slot name (`visual::style_dump`).
fn gallery_styles(w: u16, h: u16, tab: usize) -> String {
    let theme = Theme::new(Tier::TrueColor);
    let mut g = Gallery::new();
    for _ in 0..tab {
        g.key(press(KeyCode::Char(']')));
    }
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    // Twice: the tree opens and selects on its first frame.
    g.render(&theme, area, &mut buf, "colours: full colour");
    g.render(&theme, area, &mut buf, "colours: full colour");
    crate::visual::style_dump(&buf, &theme)
}

#[test]
fn gallery_every_tab_at_80x24_and_160x40() {
    for tab in 0..gallery::tab_names().len() {
        insta::assert_snapshot!(format!("gallery_{tab}_80x24"), gallery_text(80, 24, tab));
        insta::assert_snapshot!(format!("gallery_{tab}_160x40"), gallery_text(160, 40, tab));
    }
}

/// Colours and styles of every gallery page: a wrong colour, a lost
/// highlight or a missing bold fails here, which the text snapshots above
/// cannot show. Read the diff as slot names: `fg:accent.primary` became
/// `fg:text.primary` is the focus colour lost. To look at a page instead of
/// reading it: `radar shot --place gallery.<page>`.
#[test]
fn gallery_colours_every_tab_at_80x24_and_160x40() {
    for tab in 0..gallery::tab_names().len() {
        insta::assert_snapshot!(format!("gallery_styles_{tab}_80x24"), gallery_styles(80, 24, tab));
        insta::assert_snapshot!(format!("gallery_styles_{tab}_160x40"), gallery_styles(160, 40, tab));
    }
}

#[test]
fn every_story_fits_its_canvas_and_uses_rows_at_both_widths() {
    let theme = Theme::new(Tier::TrueColor);
    for story in stories::STORIES {
        for w in [79, 159] {
            let area = Rect::new(0, 0, w, 300);
            let mut buf = Buffer::empty(area);
            let used = (story.render)(&theme, area, &mut buf, 0);
            assert!(used > 10 && used < 300, "{} at {w}: {used} rows", story.name);
            // Nothing is drawn below what it says it used.
            let below = (used..300).any(|y| (0..w).any(|x| buf[(x, y)].symbol() != " "));
            assert!(!below, "{} at {w} draws below its {used} rows", story.name);
        }
    }
}

#[test]
fn a_story_scrolls_to_its_end_and_back_to_the_top_on_a_new_tab() {
    let mut g = Gallery::new();
    // To the first story, whatever tabs come before it.
    for _ in 0..gallery::TABS.len() {
        g.key(press(KeyCode::Char(']')));
    }
    assert_eq!(g.tab, gallery::TABS.len());
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    g.render(&mono(), area, &mut buf, "");
    let top = text(&buf);
    g.key(press(KeyCode::End));
    g.render(&mono(), area, &mut buf, "");
    assert_ne!(text(&buf), top, "the story scrolled");
    g.key(press(KeyCode::Char(']')));
    g.key(press(KeyCode::Char('[')));
    g.render(&mono(), area, &mut buf, "");
    assert_eq!(text(&buf), top, "back on the page, at its top");
}

#[test]
fn the_gallerys_motion_tab_starts_each_effect_from_a_key() {
    use crate::motion::{Cue, Level};
    let mut g = Gallery::new();
    g.key(press(KeyCode::Char('9')));
    assert_eq!(g.tab, gallery::MOTION_TAB);
    assert_eq!(g.take_cues(), [Cue::Screen], "switching tabs is a screen switch");
    for (k, want) in [('s', "Screen"), ('p', "Page"), ('m', "Popup"), ('t', "Message")] {
        g.key(press(KeyCode::Char(k)));
        assert_eq!(format!("{:?}", g.take_cues()), format!("[{want}]"), "{k}");
    }
    g.key(press(KeyCode::Char('g')));
    let Some(Cue::Changed(c)) = g.take_cues().pop() else { panic!("g glows rows") };
    assert_eq!(c.changed.into_iter().collect::<Vec<_>>(), ["item:1", "item:3"]);
    g.key(press(KeyCode::Char('n')));
    let Some(Cue::Changed(c)) = g.take_cues().pop() else { panic!("n adds a row") };
    assert_eq!(c.added.into_iter().collect::<Vec<_>>(), ["item:new1"]);
    g.key(press(KeyCode::Char('l')));
    let area = Rect::new(0, 0, 80, 24);
    let mut buf = Buffer::empty(area);
    g.render(&mono(), area, &mut buf, "colours: none");
    let marks = crate::motion::take();
    assert!(marks.popup.is_some() && marks.message.is_some() && marks.body.is_some());
    assert_eq!(marks.loading.len(), 1, "the waiting panel is marked");
    assert_eq!(marks.rows[0].0, "item:new1", "the new row is on top, marked");
    insta::assert_snapshot!("gallery_motion_open_80x24", text(&buf));
    g.key(press(KeyCode::Char('t')));
    assert_eq!(g.take_cues(), [Cue::MessageGone]);
    g.key(press(KeyCode::Char('f')));
    assert_eq!(g.motion.level, Level::Subtle);
}

#[test]
fn the_gallery_quits_on_q_and_moves_through_its_tabs() {
    let mut g = Gallery::new();
    assert!(g.key(press(KeyCode::Char(']'))));
    assert_eq!(g.tab, 1);
    assert!(g.key(press(KeyCode::Char('['))));
    assert!(g.key(press(KeyCode::Char('['))));
    assert_eq!(g.tab, gallery::tab_names().len() - 1);
    assert!(g.key(press(KeyCode::Char('3'))));
    assert_eq!(g.tab, 2);
    assert!(!g.key(press(KeyCode::Char('q'))));
}

// ---------------------------------------------------------------------
// Wide text: emoji and CJK take two cells, and nothing moves a column.

/// The x where `needle` starts on row `y`, reading each cell's symbol (a
/// wide character's second cell is skipped, whatever it holds).
fn x_of(buf: &Buffer, y: u16, needle: &str) -> Option<u16> {
    let a = buf.area;
    let mut row = Vec::new();
    let mut x = a.x;
    while x < a.x + a.width {
        let s = buf[(x, y)].symbol().to_string();
        let w = super::width(&s).max(1) as u16;
        row.push((x, s));
        x += w;
    }
    (0..row.len()).find_map(|i| {
        let rest: String = row[i..].iter().map(|(_, s)| s.as_str()).collect();
        rest.starts_with(needle).then_some(row[i].0)
    })
}

/// The cells a row's text takes, up to its last non-space.
fn row_cells(buf: &Buffer, y: u16) -> usize {
    let a = buf.area;
    let mut last = 0;
    let mut x = a.x;
    while x < a.x + a.width {
        let s = buf[(x, y)].symbol();
        let w = super::width(s).max(1) as u16;
        if !s.trim().is_empty() {
            last = (x + w - a.x) as usize;
        }
        x += w;
    }
    last
}

#[test]
fn fit_take_and_pad_count_cells_not_characters() {
    assert_eq!(width("日本語"), 6);
    assert_eq!(width("🎉 done"), 7);
    assert_eq!(fit("日本語のテキスト", 7), "日本語…");
    assert_eq!(width(&fit("日本語のテキスト", 7)), 7);
    // A wide character never splits: the cell it would cross is padded.
    assert_eq!(fit("日本語のテキスト", 6), "日本… ");
    assert_eq!(width(&fit("日本語のテキスト", 6)), 6);
    assert_eq!(fit("🎉 party", 10), "🎉 party  ");
    assert_eq!(take("🎉🎉", 3), ("🎉", 2));
    assert_eq!(pad("中", 4), "中  ");
    let clipped = clip(vec![Span::raw("ab"), Span::raw("中文"), Span::raw("z")], 5);
    assert_eq!(clipped.iter().map(Span::width).sum::<usize>(), 4);
    let lines = wrapped("🎉 日本語 テキスト 🎉 party time 中文 字符", 12, Span::raw("│ "), Style::new());
    assert!(lines.iter().all(|l| l.width() <= 12), "{lines:?}");
}

fn wide_nodes() -> Vec<Node> {
    let leaf = |id: &str, name: &str| {
        Node::new(id, vec![Span::raw(name.to_string())])
            .cell("state", Span::raw("open"))
            .cell("owner", Span::raw("alice"))
            .cell("size", Span::raw("12"))
            .cell("updated", Span::raw("3d"))
    };
    vec![Node::new("g", vec![Span::raw("north-wing")]).children([
        leaf("1", "#1 plain ascii name"),
        leaf("2", "#2 🎉 ship it 🚀 with a long tail of words that goes on"),
        leaf("3", "#3 日本語のタイトルはとても長いのでここで切れます"),
        leaf("4", "#4 中文 mixed with ascii"),
        Node::new("5", vec![Span::raw("#5 🐛 ")])
            .cell("state", Span::raw("🔥 hot"))
            .cell("owner", Span::raw("山田太郎さん"))
            .cell("size", Span::raw("12"))
            .cell("updated", Span::raw("3d")),
    ])]
}

fn wide_tree(w: u16) -> Buffer {
    let t = mono();
    let nodes = wide_nodes();
    let mut state = TreeState::default();
    tree::open_new(&mut state, &nodes, &mut HashSet::new());
    let area = Rect::new(0, 0, w, 8);
    let mut buf = Buffer::empty(area);
    let cols = columns().fit(w as usize - 1, 24);
    for _ in 0..2 {
        buf = Buffer::empty(area);
        BandedTree::new(&nodes, &cols, TreeLook::new(&t))
            .header(vec![Span::raw(" ITEMS")])
            .render(&t, area, &mut buf, &mut state);
    }
    buf
}

#[test]
fn tree_rows_with_emoji_and_cjk_keep_their_columns_aligned() {
    for w in [80, 160] {
        let buf = wide_tree(w);
        let size = x_of(&buf, 0, "SIZE").expect("size heading");
        let updated = x_of(&buf, 0, "UPDATED").expect("updated heading");
        for y in 2..7 {
            assert_eq!(x_of(&buf, y, "12"), Some(size), "{w} row {y}:\n{}", text(&buf));
            assert_eq!(x_of(&buf, y, "3d"), Some(updated), "{w} row {y}");
            assert!(row_cells(&buf, y) < w as usize, "{w} row {y} runs into the scrollbar");
        }
        // A cell wider than its column is cut by cells: `山田太郎さん` is 12
        // cells in an owner column of 11.
        assert!(x_of(&buf, 6, "山田太郎さ").is_some(), "{}", text(&buf));
    }
}

#[test]
fn tree_wide_80() {
    insta::assert_snapshot!(text(&wide_tree(80)));
}

#[test]
fn a_name_with_wide_text_is_cut_to_its_room_by_cells() {
    let cols = columns();
    for name in ["🎉 ship it 🚀 now and then some", "日本語のタイトルはとても長い", "plain"] {
        let line = cols.line(vec![Span::raw(name)], 60, &[("state", Span::raw("open"))]);
        assert_eq!(line.width(), 60, "{name}: {line:?}");
    }
    let cells = cols.cells(&[("owner", Span::raw("山田太郎さん")), ("state", Span::raw("🔥🔥🔥🔥🔥"))]);
    for (c, s) in cols.iter().zip(&cells) {
        assert_eq!(s.width(), c.width, "{}: {s:?}", c.heading);
    }
}

#[test]
fn a_tree_cell_of_several_spans_keeps_each_style_and_its_column() {
    use ratatui::style::{Color, Style};
    let t = mono();
    let icon = Style::new().fg(Color::Red);
    let words = Style::new().fg(Color::Blue);
    let nodes = vec![
        Node::new("a", vec![Span::raw("one")])
            .cell("state", vec![Span::styled("✗ ", icon), Span::styled("failed", words)])
            .cell("owner", Span::styled("bo", words)),
        Node::new("b", vec![Span::raw("two")])
            .cell("state", Line::from(vec![Span::styled("✓ ", icon), Span::styled("passed and much more", words)]))
            .cell("owner", Span::raw("al")),
    ];
    let cols = columns();
    let mut state = TreeState::default();
    let area = Rect::new(0, 0, 80, 3);
    let mut buf = Buffer::empty(area);
    for _ in 0..2 {
        buf = Buffer::empty(area);
        BandedTree::new(&nodes, &cols, TreeLook::new(&t))
            .header(vec![Span::raw(" ITEMS")])
            .render(&t, area, &mut buf, &mut state);
    }
    let state_x = x_of(&buf, 0, "STATE").expect("state heading");
    let owner_x = x_of(&buf, 0, "OWNER").expect("owner heading");
    for y in 1..3 {
        assert_eq!(buf[(state_x, y)].fg, Color::Red, "the icon keeps its colour:\n{}", text(&buf));
        assert_eq!(buf[(state_x + 2, y)].fg, Color::Blue, "the words keep theirs");
    }
    assert_eq!(x_of(&buf, 1, "bo"), Some(owner_x), "{}", text(&buf));
    assert_eq!(x_of(&buf, 2, "al"), Some(owner_x), "a long cell is cut to its column:\n{}", text(&buf));
    // Cut and padded to the column, as a single span is.
    let spans = cols.cell_lines(&[("state", Line::from(vec![Span::styled("✓ ", icon), Span::styled("passed and much more", words)]))]);
    let one = cols.cells(&[("state", Span::raw("✓ passed and much more"))]);
    let joined = |s: &[Span<'_>]| s.iter().map(|s| s.content.as_ref()).collect::<String>();
    assert_eq!(joined(&spans), joined(&one));
}

#[test]
fn a_table_with_emoji_and_cjk_keeps_its_columns() {
    let cols = Columns::new([Column::flex("name", 8), Column::new("owner", 9), Column::new("age", 4)]);
    let rows = vec![
        vec![("name".into(), Span::raw("plain row")), ("owner".into(), Span::raw("alice")), ("age".into(), Span::raw("3d"))],
        vec![("name".into(), Span::raw("🎉 a party 🚀 launch day")), ("owner".into(), Span::raw("山田太郎さん")), ("age".into(), Span::raw("3d"))],
        vec![("name".into(), Span::raw("日本語のタイトルはとても長い")), ("owner".into(), Span::raw("🐛")), ("age".into(), Span::raw("3d"))],
    ];
    let area = Rect::new(0, 0, 40, 6);
    let mut buf = Buffer::empty(area);
    Table::new(&cols, rows).cursor(1).render(&mono(), area, area, &mut buf, &Scroll::new());
    let age = x_of(&buf, 0, "AGE").expect("heading");
    for y in 1..4 {
        assert_eq!(x_of(&buf, y, "3d"), Some(age), "row {y}:\n{}", text(&buf));
    }
}

#[test]
fn a_card_with_emoji_and_cjk_wraps_inside_its_panel() {
    let t = mono();
    let lines = wrapped(
        "🎉 リリースしました! 日本語のテキストと中文字符 mixed with words 🚀🚀 and more 🐛 to wrap",
        26,
        Span::raw("│ "),
        t.s(Slot::TextPrimary),
    );
    let area = Rect::new(0, 0, 30, 10);
    let mut buf = Buffer::empty(area);
    card(&t, area, &mut buf, "🎉 notes", true, &Scroll::new(), lines);
    for y in 1..9 {
        assert!(row_cells(&buf, y) <= 30, "row {y}:\n{}", text(&buf));
        assert_eq!(buf[(29, y)].symbol(), "│", "the border stays put on row {y}:\n{}", text(&buf));
    }
}

#[test]
fn card_section_keeps_its_title_when_detail_does_not_fit() {
    let t = mono();
    let wide = card::section(&t, None, "replies", Some("2 replies · last 2d ago"), 40);
    let narrow = card::section(&t, None, "replies", Some("2 replies · last 2d ago"), 12);
    let words = |line: Line<'_>| line.spans.into_iter().map(|span| span.content.to_string()).collect::<String>();
    assert!(words(wide).contains("last 2d ago"));
    let narrow = words(narrow);
    assert!(narrow.starts_with("REPLIES"));
    assert!(!narrow.contains("last"));
    assert_eq!(width(&narrow), 12);
}

#[test]
fn chips_after_a_cjk_title_fit_by_cells() {
    let t = mono();
    let name = vec![Span::raw("日本語のタイトルはとても長いのでここで切れます")];
    let out = after_name(name, 0, chips(&t, &[("ready", Tone::Good)]), 30);
    assert!(out.iter().map(Span::width).sum::<usize>() <= 30, "{out:?}");
}
