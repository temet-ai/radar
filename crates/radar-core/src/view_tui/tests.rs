use super::*;

mod mnemonic_tests {
use super::*;

#[test]
fn rich_glyph_named_and_modifier_keys_precede_words_and_keep_wrapped_hits() {
    use crate::view::Block;
    use ratatui::widgets::{Paragraph, Widget, Wrap};
    let theme = Theme::new(crate::design::theme::Tier::TrueColor);
    for (key, words, expected) in [("⏎", "open", "⏎ open"), ("esc", "close", "esc close"), ("ctrl-r", "refresh", "ctrl-r refresh"), ("1", "choose", "1 choose")] {
        // Stale mnemonic fields cannot turn a real chord/glyph into a letter.
        let line = keyed_line(&theme, &Text::plain(words), Some(key), Some(0), true, 30);
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 1));
        buf.set_line(0, 0, &line, 30);
        assert_eq!(crate::shell::row_text(&buf, 0), expected);
        assert_eq!(Some(buf[(0, 0)].fg), theme.bold(Slot::AccentSecondary).fg);
        assert!(buf[(0, 0)].modifier.contains(ratatui::style::Modifier::BOLD));
    }
    let block = Block::keyed("act:move", Text::plain("move a task"), ">");
    let (lines, _) = rich_lines(&theme, std::slice::from_ref(&block), 5, false);
    let area = Rect::new(0, 0, 6, 2);
    let mut buf = Buffer::empty(area);
    Paragraph::new(lines).wrap(Wrap { trim: false }).render(area, &mut buf);
    assert_eq!([crate::shell::row_text(&buf, 0), crate::shell::row_text(&buf, 1)], ["> move", "a task"]);
    let mut hits = Vec::new();
    rich_hits(&theme, &[block], 5, 6, false, 0, 0, &mut hits);
    assert_eq!(hits, [(0, 0, 6, "act:move".into()), (1, 0, 6, "act:move".into())]);
    for stale in [None, Some(99)] {
        let fallback = keyed_line(&theme, &Text::plain("move a task"), Some("x"), stale, true, 12);
        assert_eq!(fallback.to_string(), "move a task [x]");
    }
}

#[test]
fn padded_rich_mnemonics_match_paragraph_cells_and_continuation_hits() {
    use crate::view::Block;
    use ratatui::widgets::{Paragraph, Widget, Wrap};
    let theme = Theme::new(crate::design::theme::Tier::TrueColor);
    for (words, expected) in [(" yes ", vec![" ye", "s "]), ("👩‍💻 yes", vec!["👩‍💻", "yes"])] {
        let block = Block::keyed("act:yes", Text::plain(words), "y");
        let (lines, _) = rich_lines(&theme, std::slice::from_ref(&block), 2, false);
        let wrapped = wrap_line(lines[0].clone(), 3);
        assert_eq!(wrapped.iter().map(Line::to_string).collect::<Vec<_>>(), expected);
        let area = Rect::new(0, 0, 3, 2);
        let mut actual = Buffer::empty(area);
        Paragraph::new(lines).wrap(Wrap { trim: false }).render(area, &mut actual);
        let mut prewrapped = Buffer::empty(area);
        Paragraph::new(wrapped).render(area, &mut prewrapped);
        assert_eq!(prewrapped, actual, "painted cells and underlines: {words:?}");
        let mut hits = Vec::new();
        rich_hits(&theme, &[block], 2, 3, false, 0, 0, &mut hits);
        assert_eq!(hits, [(0, 0, 3, "act:yes".into()), (1, 0, 3, "act:yes".into())]);
    }
}

#[test]
fn skipped_controls_keep_the_final_mnemonic_source_position() {
    use ratatui::style::Modifier;
    let theme = Theme::new(crate::design::theme::Tier::TrueColor);
    for (words, key, at, symbol) in [("\tr", "r", 1, "r"), ("a\r\nr", "r", 3, "r"), ("\te\u{301}", "e", 1, "e\u{301}")] {
        let line = keyed_line(&theme, &Text::plain(words), Some(key), Some(at), false, 20);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 1));
        buffer.set_line(0, 0, &line, 20);
        assert!(buffer.content.iter().any(|cell| cell.symbol() == symbol && cell.modifier.contains(Modifier::UNDERLINED)), "{words:?}: {line:?}");
    }
}

#[test]
fn long_multi_style_captions_coalesce_linearly_and_shift_keeps_named_key_style() {
    use ratatui::style::{Modifier, Style};
    let mut spans = Vec::new();
    for _ in 0..10_000 {
        spans.push(Span::styled("e", Style::new().add_modifier(Modifier::BOLD)));
        spans.push(Span::styled("\u{301}", Style::new().add_modifier(Modifier::UNDERLINED)));
    }
    let line = coalesced_line(Line::from(spans));
    assert_eq!(line.to_string(), "e\u{301}".repeat(10_000));
    assert_eq!(line.width(), 10_000);
    assert_eq!(line.spans.len(), 1, "adjacent equal styles remain one run");
    assert!(line.spans[0].style.add_modifier.contains(Modifier::BOLD | Modifier::UNDERLINED));
    let theme = Theme::new(crate::design::theme::Tier::TrueColor);
    let shifted = keyed_line(&theme, &Text::plain("ask an agent"), Some("A"), Some(0), true, 40);
    assert_eq!(shifted.spans[0].content, "shift ");
    assert_eq!(shifted.spans[0].style, theme.bold(Slot::AccentSecondary));
}

#[test]
fn rich_mnemonics_keep_source_offsets_hidden_icons_chips_and_segment_styles() {
    use ratatui::style::Modifier;
    use crate::ui::icon::Icon;
    let text = Text::new().push(Seg::icon_or(Icon::Link, "open ", Ink::Muted))
        .push(Seg::new("e", Ink::Accent).strong()).push(Seg::new("\u{301}dit", Ink::Secondary).linked("https://example.test/watch"));
    for pack in ["off", "unicode"] {
        let theme = Theme::build(&crate::theme::ThemeConfig { icons: pack.into(), ..Default::default() }, crate::design::theme::Tier::TrueColor, &[]);
        let line = keyed_line(&theme, &text, Some("e"), Some(5), false, 80);
        assert!(!line.to_string().contains("[e]"));
        let lit: String = line.spans.iter().filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED)).map(|s| s.content.as_ref()).collect();
        assert_eq!(lit, "e\u{301}");
        assert!(line.spans.iter().find(|s| s.content == "e\u{301}").unwrap().style.add_modifier.contains(Modifier::BOLD));
        let area = Rect::new(0, 0, 30, 1);
        let mut buffer = Buffer::empty(area);
        buffer.set_line(0, 0, &line, 30);
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "e\u{301}" && cell.modifier.contains(Modifier::UNDERLINED)));
        let mut chip = Seg::new("copy", Ink::Accent);
        chip.icon = Some(Icon::Copy);
        chip.chip = true;
        let chip = Text::from(vec![chip]);
        let keyed = keyed_line(&theme, &chip, Some("y"), Some(3), false, 80);
        assert_eq!(keyed.to_string(), line_for_chip(&theme, &chip));
        assert_eq!(keyed.spans.iter().filter(|s| s.style.add_modifier.contains(Modifier::UNDERLINED)).map(|s| s.content.as_ref()).collect::<String>(), "y");
    }
    fn line_for_chip(theme: &Theme, text: &Text) -> String { super::line(theme, text).to_string() }
}

#[test]
fn rich_wrapping_keeps_emoji_graphemes_and_legacy_shortcuts_visible() {
    let theme = Theme::new(crate::design::theme::Tier::Mono);
    let text = Text::plain("👩‍💻 edit");
    let line = keyed_line(&theme, &text, Some("e"), Some(4), false, 5);
    let wrapped = wrap_line(line, 5);
    assert_eq!(wrapped.iter().map(Line::to_string).collect::<Vec<_>>(), ["👩‍💻", "edit"]);
    assert_eq!(wrapped[0].width(), 2);
    assert_eq!(wrap_line(Line::raw("👩‍💻e"), 1).iter().map(Line::to_string).collect::<Vec<_>>(), ["e"]);
    assert!(wrapped[1].spans[0].style.add_modifier.contains(ratatui::style::Modifier::UNDERLINED));
    let fallback = keyed_line(&theme, &Text::plain("long action caption"), Some("x"), Some(90), false, 5);
    let wrapped = wrap_line(fallback, 5);
    assert!(wrapped.iter().any(|line| line.to_string().contains("[x]")));
}
}

use crate::view::{PaneView, TreeRow};

fn screen() -> ScreenView {
    let nav = Node::List {
        rows: vec![ListRow::new("view:0", "all"), ListRow::new("view:1", "dirty")],
        selected: Some(0),
        editing: None,
    };
    let tree = Tree {
        rows: vec![TreeRow::new("a", "a").child(TreeRow::new("b", "b"))],
        open: vec![vec!["a".into()]],
        selected: vec!["a".into()],
        ..Tree::default()
    };
    ScreenView::new(
        "s",
        vec![
            PaneView::new(PaneId::Nav, "views", nav),
            PaneView::new(PaneId::Middle, "all", Node::Tree(tree)),
            PaneView::new(PaneId::Detail, "a", Node::Document { lines: vec!["x".into()], cut: false }),
        ],
    )
}

#[test]
fn movement_selects_through_the_part_and_enter_activates() {
    let fe = Frontend::default();
    let v = screen();
    let sel = |id: &str| Routed::Intent(Intent::Select { target: Target::new(id) });
    assert_eq!(key(&fe, &v, PaneId::Nav, Chord::char('j')), sel("view:1"));
    assert_eq!(
        key(&fe, &v, PaneId::Middle, Chord::new(Key::Down)),
        Routed::Intent(Intent::Select { target: TreeRow::target_of(&["a".into(), "b".into()]) })
    );
    assert_eq!(
        key(&fe, &v, PaneId::Middle, Chord::new(Key::Enter)),
        Routed::Intent(Intent::Activate { target: Target::new("a") })
    );
    assert_eq!(
        key(&fe, &v, PaneId::Middle, Chord::char('d')),
        Routed::Intent(Intent::Key { chord: Chord::char('d') }),
        "anything else is the part's"
    );
}

/// `/` typed from the views rail opens the tree's field: ↓ is the field's,
/// not a move along the rail.
fn typing_in_the_tree(mut v: ScreenView) -> ScreenView {
    v.panes[1].fields.push(crate::view::FieldBar {
        mark: " / ".into(),
        text: "ti".into(),
        caret: Some(2),
        hint: crate::view::Text::default(),
    });
    v
}

#[test]
fn a_field_being_typed_into_takes_every_key_whichever_pane_has_the_focus() {
    let fe = Frontend::default();
    let v = typing_in_the_tree(screen());
    for chord in [Chord::new(Key::Down), Chord::char('j'), Chord::new(Key::PageDown)] {
        assert_eq!(key(&fe, &v, PaneId::Nav, chord), Routed::Intent(Intent::Key { chord }), "{chord:?}");
    }
}

#[test]
fn clicks_select_their_target_and_the_pane_only_focuses() {
    let fe = Frontend::default();
    assert_eq!(click(&fe, PANE_HIT, false), Routed::Done);
    assert_eq!(
        click(&fe, "view:1", true),
        Routed::Intent(Intent::Context { target: Target::new("view:1") })
    );
    assert_eq!(click(&fe, "\u{1e}cell:9", false), Routed::Ignored, "a stale cell does nothing");
}

#[test]
fn chords_and_terminal_keys_round_trip() {
    for c in [Chord::char('x'), Chord::ctrl('r'), Chord::new(Key::PageDown), Chord::new(Key::BackTab)] {
        assert_eq!(chord_of(&key_of(c)), Some(c));
    }
}

#[test]
fn actions_become_the_menu_rows_with_their_keys() {
    let rows = bindings(&[
        ActionSpec::named("", &Binding::heading("main")),
        ActionSpec::named("git.branch.delete", &Binding::new("d", "delete the branch…").edits()),
    ]);
    assert!(rows[0].is_heading());
    assert_eq!(crate::plugin::parse_key(&rows[1].keys), Some(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE)));
    assert_eq!(rows[1].change, Some(crate::writes::Change::Edit));
}

#[test]
fn every_view_action_chord_remains_pressable_in_a_menu() {
    for key in [
        Key::Backspace,
        Key::Delete,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
    ] {
        let chord = Chord::new(key);
        let mut action = ActionSpec::named("t.s.action", &Binding::new("a", "action"));
        action.key = Some(chord);
        let row = bindings(&[action]).remove(0);
        assert_eq!(crate::plugin::parse_key(&row.keys), Some(key_of(chord)));
    }
}

#[test]
fn editing_a_tree_row_sends_keys_to_the_part() {
    let view = SectionView {
        version: crate::view::VERSION,
        scope: "projects".into(),
        body: Node::Tree(Tree {
            rows: vec![TreeRow::new("site", "site")],
            selected: vec!["site".into()],
            editing: Some(Box::new(crate::view::TreeEditing {
                path: vec!["site".into()],
                at: 2,
                text: "acme".into(),
                caret: 4,
            })),
            ..Tree::default()
        }),
        detail_title: String::new(),
        detail: vec![],
        detail_blocks: vec![],
        actions: vec![],
    };
    let chord = Chord::new(Key::Down);
    assert_eq!(section_key(&view, chord), Intent::Key { chord });
}

// --- the Ink renderer's input fixture ---------------------------------------------

/// The intents the Ratatui adapter sends for chords and clicks over small
/// recorded views, written to `ink/test/input/ratatui-intents.json`. The Ink
/// renderer's input test sends the same chords over the same views and must
/// get the same intents. This test fails when the adapter drifts from the
/// committed file; `RADAR_WRITE_INK_FIXTURES=1` rewrites it.
#[test]
fn the_ink_input_fixture_matches_what_the_ratatui_adapter_sends() {
    use crate::view::{ColumnSpec, EntryView, State, TableRow};
    use serde_json::{Value, json};

    fn middle(scope: &str, body: Node) -> ScreenView {
        ScreenView::new(scope, vec![PaneView::new(PaneId::Middle, "rows", body)])
    }
    fn list(selected: Option<usize>) -> Node {
        Node::List {
            rows: vec![
                ListRow::heading("h", "group"),
                ListRow::new("row:a", "a"),
                ListRow::new("row:b", "b"),
                ListRow::new("row:c", "c"),
            ],
            selected,
            editing: None,
        }
    }
    fn tree(selected: &[&str]) -> Node {
        Node::Tree(Tree {
            rows: vec![
                TreeRow::new("g1", "closed").child(TreeRow::new("a", "a")).child(TreeRow::new("b", "b")),
                TreeRow::new("g2", "open").child(TreeRow::new("c", "c")).child(TreeRow::new("d", "d")),
            ],
            open: vec![vec!["g2".into()]],
            selected: selected.iter().map(|s| s.to_string()).collect(),
            ..Tree::default()
        })
    }
    let k = Chord::new;
    let c = Chord::char;
    let list_keys = vec![
        k(Key::Down), k(Key::Up), c('j'), c('k'), k(Key::Home), k(Key::End), c('g'), c('G'),
        k(Key::Enter), c('x'), k(Key::Tab), k(Key::PageDown), Chord::ctrl('r'),
    ];
    let tree_keys = vec![
        k(Key::Down), k(Key::Up), k(Key::Right), c('+'), c('='), c('-'), c(' '), k(Key::Enter), c('x'),
    ];
    let table = Node::Table {
        columns: vec![ColumnSpec::new("name", 10, 4)],
        rows: vec![TableRow::new("t:0").cell("name", "one"), TableRow::new("t:1").cell("name", "two")],
        selected: Some(0),
        empty: None,
    };
    let entries = Node::Entries {
        rows: vec![EntryView::new("e:0", "first"), EntryView::new("e:1", "second")],
        selected: Some(0),
        empty: "nothing".into(),
    };
    let mut overlay = middle("overlay", list(Some(1)));
    let editor = crate::view::Editor { title: "reply".into(), lines: vec![], line: "hi".into(), caret: 2, selection: None };
    overlay.overlay = Some(Box::new(PaneView::new(PaneId::Middle, "reply", editor.node())));
    let document = ScreenView::new(
        "document",
        vec![
            PaneView::new(PaneId::Middle, "rows", list(Some(1))),
            PaneView::new(PaneId::Detail, "about", Node::Document { lines: vec!["x".into()], cut: false }),
        ],
    );
    let screens: Vec<(&str, PaneId, ScreenView, Vec<Chord>)> = vec![
        ("list", PaneId::Middle, middle("list", list(Some(1))), list_keys.clone()),
        ("list-unselected", PaneId::Middle, middle("list", list(None)), list_keys.clone()),
        ("table", PaneId::Middle, middle("table", table), vec![k(Key::Down), k(Key::Enter)]),
        ("entries", PaneId::Middle, middle("entries", entries), vec![k(Key::Down), k(Key::Enter)]),
        ("tree-on-group", PaneId::Middle, middle("tree", tree(&["g2"])), tree_keys.clone()),
        ("tree-on-leaf", PaneId::Middle, middle("tree", tree(&["g2", "c"])), tree_keys.clone()),
        ("overlay", PaneId::Middle, overlay, vec![k(Key::Down), k(Key::Enter), c('x')]),
        (
            "state",
            PaneId::Middle,
            middle("state", Node::State { state: State::Empty, text: "nothing yet".into() }),
            vec![k(Key::Enter), k(Key::Down)],
        ),
        ("document-detail", PaneId::Detail, document, vec![k(Key::Down), k(Key::PageDown), k(Key::Enter)]),
        (
            "field-on-another-pane",
            PaneId::Nav,
            typing_in_the_tree(screen()),
            vec![k(Key::Down), c('j'), k(Key::PageDown), k(Key::Enter)],
        ),
    ];
    let screens: Vec<Value> = screens
        .into_iter()
        .map(|(name, focus, view, chords)| {
            let fe = Frontend::default();
            let cases: Vec<Value> = chords
                .into_iter()
                .filter_map(|chord| match key(&fe, &view, focus, chord) {
                    Routed::Intent(intent) => Some(json!({ "chord": chord, "intent": intent })),
                    _ => None,
                })
                .collect();
            json!({ "name": name, "focus": focus, "view": view, "cases": cases })
        })
        .collect();
    let section = |scope: &str, body: Node| SectionView {
        version: crate::view::VERSION,
        scope: scope.into(),
        body,
        detail_title: String::new(),
        detail: vec![],
        detail_blocks: vec![],
        actions: vec![],
    };
    let rows = |selected| Node::List {
        rows: vec![ListRow::new("s:0", "first"), ListRow::new("s:1", "second"), ListRow::new("s:2", "third")],
        selected: Some(selected),
        editing: None,
    };
    let sections: Vec<(&str, SectionView, Vec<Chord>)> = vec![
        ("list-first", section("list", rows(0)), vec![k(Key::Up), k(Key::Down), k(Key::Enter), c('x')]),
        ("list", section("list", rows(1)), vec![k(Key::Up), k(Key::Down), k(Key::Enter), c('x')]),
        ("tree", section("tree", tree(&["g2"])), vec![k(Key::Down), k(Key::Enter), c('x')]),
    ];
    let sections: Vec<Value> = sections
        .into_iter()
        .map(|(name, view, chords)| {
            let cases: Vec<Value> = chords
                .into_iter()
                .map(|chord| json!({ "chord": chord, "intent": section_key(&view, chord) }))
                .collect();
            json!({ "name": name, "view": view, "cases": cases })
        })
        .collect();
    let fe = Frontend::default();
    let clicks: Vec<Value> = [("view:1", false), ("view:1", true), ("row:b", false), ("row:b", true)]
        .into_iter()
        .filter_map(|(id, context)| match click(&fe, id, context) {
            Routed::Intent(intent) => Some(json!({ "id": id, "context": context, "intent": intent })),
            _ => None,
        })
        .collect();
    let fixture = json!({ "screens": screens, "sections": sections, "clicks": clicks });
    let text = serde_json::to_string_pretty(&fixture).unwrap() + "\n";
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../ink/test/input/ratatui-intents.json");
    if std::env::var_os("RADAR_WRITE_INK_FIXTURES").is_some_and(|v| v == "1") {
        std::fs::write(&path, &text).unwrap();
        return;
    }
    let committed = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        committed == text,
        "{} differs from what the Ratatui adapter sends; rerun with RADAR_WRITE_INK_FIXTURES=1 and commit it",
        path.display()
    );
}

/// From #26: the detail scrolls back to the top when the selected row is
/// another, even at the same place on another page.
#[test]
fn table_selection_uses_identity_not_position_to_reset_detail() {
    use crate::view::TableRow;
    let mut view = ScreenView::new(
        "observations",
        vec![PaneView::new(
            PaneId::Middle,
            "events",
            Node::Table { columns: vec![], rows: vec![TableRow::new("page-one-event")], selected: Some(0), empty: None },
        )],
    );
    let original = selection(&view);
    if let Node::Table { rows, .. } = &mut view.panes[0].body {
        rows[0].target = Target::new("page-two-event");
    }
    assert_ne!(selection(&view), original, "a same-index event on another page resets detail");
}

/// The sizes the mix screens are recorded at, as the panes take them: at
/// 80 cells one pane, the one with the keys; from 120 the three side by
/// side; between, the nav pane and one pane (or the middle and the detail with
/// no nav pane), from the width the screen asks for.
#[test]
fn a_screen_is_one_pane_narrow_and_three_wide() {
    let at = |w: u16, view: &ScreenView| fit(Rect::new(0, 0, w, 38), view);
    let v = screen();
    assert!(matches!(at(80, &v), Fit::Narrow));
    assert!(matches!(at(83, &v), Fit::Narrow));
    assert!(matches!(at(84, &v), Fit::Medium));
    assert!(matches!(at(119, &v), Fit::Medium));
    assert!(matches!(at(120, &v), Fit::Wide));
    assert!(matches!(at(160, &v), Fit::Wide));
    let mut later = screen();
    later.medium_from = Some(100);
    assert!(matches!(at(99, &later), Fit::Narrow));
    assert!(matches!(at(100, &later), Fit::Medium));
    assert!(matches!(at(120, &later), Fit::Wide));
}

fn words_of(lines: &[Line<'_>]) -> Vec<String> {
    lines.iter().map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect()).collect()
}

/// A section is `ui::card::section`'s heading and rule across the room,
/// then its blocks two cells in; an action's key sits at the right edge.
#[test]
fn a_section_draws_its_heading_and_rule_and_sets_its_lines_in() {
    use crate::view::Block;
    let t = Theme::new(crate::design::theme::Tier::Mono);
    let blocks = vec![
        Block::section(
            None,
            "pipeline",
            Some("passed · 6m 12s".into()),
            vec![Block::line(Text::plain("3 jobs"))],
        ),
        Block::section(None, "actions", None, vec![Block::keyed("act:m", Text::plain("merge into main"), "m")]),
    ];
    // The pane lays rich blocks out at the room less one (`rich_lines`'s width).
    let (lines, _) = rich_lines(&t, &blocks, 29, false);
    assert_eq!(
        words_of(&lines),
        [
            "PIPELINE  passed · 6m 12s ────",
            "  3 jobs",
            "ACTIONS ──────────────────────",
            "  merge into main",
        ]
    );
    assert_eq!(lines[0], ui::card::section(&t, None, "PIPELINE", Some("passed · 6m 12s"), 30));
    assert_eq!(lines[3].width(), 17, "the mnemonic occupies its word, not extra cells");
}

/// A long line in a section breaks between words at the room inside it,
/// every row set in, as Ink's wrapCells breaks it.
#[test]
fn a_long_line_in_a_section_wraps_inside_it() {
    use crate::view::Block;
    let t = Theme::new(crate::design::theme::Tier::Mono);
    let blocks = vec![Block::section_of(None, "notes", None, vec![Text::plain("one two three four five six")])];
    let (lines, _) = rich_lines(&t, &blocks, 13, false);
    assert_eq!(words_of(&lines), ["NOTES ────────", "  one two", "  three four", "  five six"]);
}

/// A legacy key that does not fit beside the words remains available to wrap.
#[test]
fn a_legacy_key_that_does_not_fit_is_retained() {
    let t = Theme::new(crate::design::theme::Tier::Mono);
    let line = keyed_line(&t, &Text::plain("rebase onto main"), Some("u"), None, false, 18);
    assert_eq!(words_of(&[line]), ["rebase onto main [u]"]);
}

/// A click on an action line in a section reaches its target: the row
/// under the heading, set in two cells.
#[test]
fn a_line_in_a_section_is_hit_where_it_is_drawn() {
    use crate::view::Block;
    let t = Theme::new(crate::design::theme::Tier::Mono);
    let blocks = vec![
        Block::line(Text::plain("title")),
        Block::line(Text::new()),
        Block::section(
            None,
            "actions",
            None,
            vec![
                Block::keyed("act:u", Text::plain("rebase"), "u"),
                Block::keyed("act:m", Text::plain("merge"), "m"),
            ],
        ),
    ];
    let mut hits = Vec::new();
    rich_hits(&t, &blocks, 29, 30, false, 0, 0, &mut hits);
    assert_eq!(hits, [(3, 2, 28, "act:u".to_string()), (4, 2, 28, "act:m".to_string())]);
    let targets = crate::headless::node_targets(&Node::Rich { blocks, pictures: None });
    assert_eq!(targets, [Target::new("act:u"), Target::new("act:m")], "the headless explorer offers them too");
}

#[test]
fn modified_arrows_round_trip_without_changing_printable_shift_shortcuts() {
    for key in [Key::Left, Key::Right, Key::Home, Key::End] {
        let chord = Chord { shift: true, ..Chord::new(key) };
        assert_eq!(chord_of(&key_of(chord)), Some(chord));
    }
    for c in ['W', '+', 'G'] {
        let physical = KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT);
        assert_eq!(chord_of(&physical), Some(Chord::char(c)));
    }
}

#[test]
fn an_empty_settings_tree_still_translates_open_all_without_a_fake_selection() {
    let view = SectionView {
        version: crate::view::VERSION,
        scope: "watch".into(),
        body: Node::Tree(Tree::default()),
        detail_title: String::new(),
        detail: vec![],
        detail_blocks: vec![],
        actions: vec![],
    };
    assert_eq!(section_key(&view, Chord::char('=')), Intent::OpenAll);
    assert_eq!(section_key(&view, Chord::new(Key::Up)), Intent::Key { chord: Chord::new(Key::Up) });
}
