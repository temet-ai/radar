//! Pinned numbers for what a frame measures: wrapped rows, the menu's
//! viewport (a long command palette, a confirmation, a terminal too small),
//! settings' panes and how far the detail scrolls, setup's card and sheet.
//! They replace the shell's `measure` tests, which read the same numbers
//! from a drawn buffer.

use super::*;
use crate::design::theme::Tier;
use crate::view::chrome::PaletteView;

fn theme() -> Theme {
    Theme::new(Tier::Mono)
}

fn row(label: &str, keys: &str) -> Choice {
    Choice {
        mnemonic_at: None,
        mnemonic_shift: false,
        value: label.into(),
        label: label.into(),
        hint: String::new(),
        key: None,
        keys: keys.into(),
        icon: None,
        heading: false,
        note: false,
        sample: None,
        shown_only: false,
        muted: false,
        target: None,
    }
}

fn palette(n: usize, total: usize, items: Vec<Choice>) -> MenuView {
    let mut view = MenuView::new("palette", "commands", items);
    view.palette = Some(Box::new(PaletteView {
        matched: n,
        total,
        highlights: vec![vec![]; n],
        hint_highlights: vec![],
    }));
    view
}

#[test]
fn rows_wrap_by_word_and_break_a_word_longer_than_the_room() {
    assert_eq!(rows("", 10), 1);
    assert_eq!(rows("a\nb\n", 10), 3);
    assert_eq!(rows("alpha beta gamma", 11), 2);
    assert_eq!(rows("alpha beta gamma", 10), 2);
    assert_eq!(rows("alpha beta gamma", 16), 1);
    assert_eq!(rows("abcdefghij", 4), 3);
    // The spaces at the end of a line, and where it wraps, take no row.
    assert_eq!(rows("abc   ", 4), 1);
    assert_eq!(rows("abc    def", 4), 2);
    // An indent counts while the line has room for it.
    assert_eq!(rows("    abc    def", 8), 2);
    // Wide characters count two cells and are never cut in half.
    assert_eq!(rows("日本語日本語", 6), 2);
    assert_eq!(rows("日本語", 5), 2);
    // Five 9-cell words in 43 cells: four fit a row.
    assert_eq!(rows(&["wordwordw"; 5].join(" "), 43), 2);
}

#[test]
fn a_paragraph_wider_than_the_offsets_is_counted_in_chunks() {
    // 40 000 cells with no break: one chunk of 32 768 and one of 7 232, each
    // a row in a room wider than both.
    let text = "x".repeat(40_000);
    assert_eq!(rows(&text, 100_000), 2);
}

#[test]
fn a_long_palette_shows_a_screenful_and_reads_from_the_selected_row() {
    let items: Vec<Choice> = (0..40).map(|n| row(&format!("command {n:02}"), "")).collect();
    let mut view = palette(40, 120, items);
    view.selected = Some(25);
    // A body 60 cells wide and 12 high: the frame takes a border each side.
    let m = menu(&view, &theme(), Size::new(60, 12));
    // The title "commands · 40 of 120" and a cell either side set the width.
    assert_eq!(m.width, 22);
    assert_eq!((m.total, m.shown, m.usable), (40, 10, true));
    assert_eq!((m.footer, m.field), (0, 0));
    assert_eq!(m.parts.len(), 40);
    assert_eq!(m.parts[25], (Some(25), 25..26));
    // Selecting row 25 from the top scrolls just far enough to show it last.
    assert_eq!(m.scroll_for(0, 25, true, false), 16);
    // Row 0 comes back into sight at the top.
    assert_eq!(m.scroll_for(16, 0, true, false), 0);
    // Not moved, not resized: the reading position stays, kept in range.
    assert_eq!(m.scroll_for(12, 25, false, false), 12);
    assert_eq!(m.scroll_for(99, 25, false, false), 30);
}

#[test]
fn the_host_keeps_the_numbers_a_menu_measures() {
    let items: Vec<Choice> = (0..40).map(|n| row(&format!("command {n:02}"), "")).collect();
    let mut view = palette(40, 120, items);
    view.selected = Some(25);
    let viewport = MenuViewport::default();
    apply_menu(&view, &viewport, &theme(), Size::new(60, 12));
    assert_eq!(
        (viewport.at.get(), viewport.height.get(), viewport.total.get(), viewport.width.get(), viewport.usable.get()),
        (16, 10, 40, 22, true)
    );
    assert_eq!(viewport.max(), 30);
    assert!(viewport.more());
    // The selection moving up past the top of what shows scrolls to it.
    view.selected = Some(3);
    apply_menu(&view, &viewport, &theme(), Size::new(60, 12));
    assert_eq!(viewport.at.get(), 3);
    // Paging past the end stays inside.
    viewport.scroll(true, true);
    viewport.scroll(true, true);
    viewport.scroll(true, true);
    viewport.scroll(true, true);
    assert_eq!(viewport.at.get(), 30);
}

#[test]
fn a_palette_row_longer_than_the_room_wraps_onto_a_second_row() {
    let mut view = palette(1, 1, vec![row(&"x".repeat(60), "")]);
    view.selected = Some(0);
    let m = menu(&view, &theme(), Size::new(60, 12));
    // 60 cells and the palette's padding do not fit 58: the width is the
    // most the body allows, and the word breaks inside 56.
    assert_eq!(m.width, 58);
    assert_eq!((m.total, m.shown), (2, 2));
    assert_eq!(m.parts, vec![(Some(0), 0..2)]);
}

fn question() -> MenuView {
    let mut note = row("Reply to Sam Vimes on WATCH-12", "");
    note.note = true;
    let mut view = MenuView::new("post", "Post this comment?", vec![note, row("post it", "y"), row("cancel", "n")]);
    view.question = true;
    view.selected = Some(1);
    view
}

#[test]
fn a_confirmation_is_a_note_over_a_row_of_buttons() {
    let t = theme();
    let m = menu(&question(), &t, Size::new(80, 24));
    // The buttons "y post it" and "n cancel" are 11 and 10 cells and sit two
    // apart: 23. The title, a warning icon and a cell either side, is
    // narrower unless the icon pack is wide.
    let title = 1 + crate::design::width::width(&icon::text(&t, Icon::Warning)) + "Post this comment?".len() + 1;
    assert_eq!(usize::from(m.width), title.max(23));
    // The note wraps at the width less its one-cell inset: two rows.
    assert_eq!(m.parts, vec![(None, 0..2)]);
    // Raised buttons are three rows (a bevel either side), on one row.
    assert_eq!((m.footer, m.total, m.shown, m.usable), (3, 2, 2, true));
    assert!(m.question);
    // A question begins at its first words whatever is selected.
    assert_eq!(m.scroll_for(0, 1, true, true), 0);
}

#[test]
fn a_confirmation_in_a_short_terminal_flattens_its_buttons_then_scrolls() {
    let t = theme();
    // Three rows inside the frame: the raised footer (3) leaves no body, so
    // the buttons go flat (1) and the note keeps one row of its two.
    let m = menu(&question(), &t, Size::new(30, 5));
    assert_eq!((m.footer, m.total, m.shown, m.usable), (1, 2, 1, true));
    // One row inside: no room for a body row over a button row.
    let m = menu(&question(), &t, Size::new(30, 3));
    assert!(!m.usable);
}

#[test]
fn settings_split_into_panes_by_width() {
    // Below 120 cells the detail sits under the section.
    let [nav, middle, detail] = settings_panes(Size::new(100, 30));
    assert_eq!((nav, middle, detail), (Size::new(24, 30), Size::new(76, 15), Size::new(76, 15)));
    // From 120 they sit side by side, three parts to two.
    let [nav, middle, detail] = settings_panes(Size::new(140, 30));
    assert_eq!((nav, middle, detail), (Size::new(24, 30), Size::new(69, 30), Size::new(47, 30)));
    // The sections shrink to a third of a narrow body.
    assert_eq!(settings_panes(Size::new(60, 20))[0].width, 20);
    // The text room: a border and a cell of padding either side, a border
    // above and below.
    assert_eq!(settings_detail_text(Size::new(100, 30)), Size::new(72, 13));
    assert_eq!(settings_detail_text(Size::new(140, 30)), Size::new(43, 28));
    assert_eq!(settings_middle_inside(Size::new(100, 30)), Size::new(74, 13));
}

#[test]
fn the_body_is_the_frame_between_the_bar_and_the_status_line() {
    assert_eq!(body_of(Size::new(100, 32), false), Size::new(100, 30));
    assert_eq!(body_of(Size::new(100, 32), true), Size::new(100, 29));
    assert_eq!(body_of(Size::new(100, 1), true), Size::new(100, 0));
}

#[test]
fn settings_detail_scrolls_by_the_rows_past_its_room() {
    let mut detail: Vec<Text> = (0..19).map(|n| Text::plain(format!("row {n}"))).collect();
    detail.push(Text::plain(["wordwordw"; 5].join(" ")));
    // 19 lines of one row and one that wraps at 43 cells: 21 rows.
    let room = settings_detail_text(Size::new(140, 20));
    assert_eq!(room, Size::new(43, 18));
    assert_eq!(texts_rows(&detail, room.width), 21);
    let scroll = crate::ui::Scroll::new();
    scroll.fit(room.height, texts_rows(&detail, room.width) as u16);
    assert_eq!(scroll.max(), 3);
    // Taller than its content: nothing to scroll.
    let room = settings_detail_text(Size::new(140, 40));
    scroll.fit(room.height, texts_rows(&detail, room.width) as u16);
    assert_eq!(scroll.max(), 0);
    // Stacked under the section it has 72 cells, so the long line fits.
    assert_eq!(texts_rows(&detail, settings_detail_text(Size::new(100, 30)).width), 20);
}

#[test]
fn setup_gives_the_sheet_its_text_and_the_card_the_rest() {
    // The frame is a cell in from each side, at most 140 × 40.
    assert_eq!(setup_frame(Size::new(150, 40)), Size::new(140, 40));
    assert_eq!(setup_frame(Size::new(80, 24)), Size::new(78, 24));
    assert_eq!(setup_about_width(Size::new(150, 40)), 136);
    // A sheet of six rows is seven high with its tab; the card has what is
    // left under a row, the tab rows and the gap.
    let (card, sheet) = setup_panes(Size::new(150, 40), 6);
    assert_eq!((card, sheet), (Size::new(140, 30), Size::new(136, 6)));
    // A long sheet is held to a third of the frame or what the card's 22
    // rows leave, whichever is more: 18 rows and its tab.
    let (card, sheet) = setup_panes(Size::new(150, 40), 40);
    assert_eq!((card, sheet), (Size::new(140, 19), Size::new(136, 17)));
}

fn line(words: &str) -> Block {
    Block::Line { text: Text::plain(words), mnemonic_at: None, mnemonic_shift: false, target: None, key: None }
}

#[test]
fn a_section_with_rich_blocks_taller_than_the_pane_scrolls_to_its_last_row_and_stops() {
    use crate::view::{Node, State};
    let mut blocks: Vec<Block> = (0..30).map(|n| line(&format!("line {n}"))).collect();
    // A line of ten 9-cell words wraps to two rows in 72 cells.
    blocks.push(line(&["wordwordw"; 10].join(" ")));
    // A section is its heading row and its blocks.
    blocks.push(Block::Section { mark: None, title: "NOTES".into(), detail: None, blocks: (0..3).map(|n| line(&format!("note {n}"))).collect() });
    let view = SectionView {
        version: crate::view::VERSION,
        scope: "probe".into(),
        body: Node::State { state: State::Empty, text: String::new() },
        detail_title: "probe".into(),
        detail: vec![],
        detail_blocks: blocks,
        actions: vec![],
    };
    let room = settings_detail_text(Size::new(100, 20));
    assert_eq!(room, Size::new(72, 8));
    let rows = detail_rows(&view, room.width, SETTINGS_INSET);
    assert_eq!(rows, 30 + 2 + 1 + 3);
    let scroll = crate::ui::Scroll::new();
    scroll.fit(room.height, rows as u16);
    // The last row is reachable: the furthest the pane scrolls ends on it.
    assert_eq!(usize::from(scroll.max()) + usize::from(room.height), rows);
    // Scrolling far past it stops there.
    scroll.laid_out(room.height, rows as u16, u16::MAX);
    assert_eq!(scroll.at(), scroll.max());
    scroll.fit(room.height, rows as u16);
    assert_eq!(scroll.at(), scroll.max());
}
