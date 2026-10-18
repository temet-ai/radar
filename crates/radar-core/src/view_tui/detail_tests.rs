use super::*;
use crate::design::theme::Tier;
use crate::theme::ThemeConfig;
use crate::view::detail::{DetailContent, DetailHeader, FootAction, FootActions, pane};
use crate::view::{Block, PaneView};

fn view() -> ScreenView {
    let content = DetailContent::new(DetailHeader::new("watch / supplies", "Count every lantern"), vec![], vec![
        Block::section(None, "notes", None, (0..30).map(|i| Block::action(format!("note:{i}"), format!("Note {i}"))).collect()),
    ]);
    let mut foot = FootActions::new(vec![FootAction::new(ActionSpec::named("watch.open", &Binding::new("o", "open")))]);
    foot.actions[0].target = Some(Target::new("detail-foot:1:0"));
    foot.menu.target = Some(Target::new("detail-foot:1:menu"));
    ScreenView::new("watch", vec![pane("detail", content, foot)])
}

fn draw(fe: &Frontend, view: &ScreenView, width: u16, height: u16, jumps: &[crate::view::chrome::PaneJump]) -> (Buffer, Vec<(Rect, Pane, String)>) {
    let theme = Theme::build(&ThemeConfig { icons: "off".into(), ..Default::default() }, Tier::TrueColor, &[]);
    let cursor = StdCell::new(None);
    let hits = RefCell::new(vec![]);
    let plugins = vec![];
    let cx = Cx { theme: &theme, now: 0, focused: false, focus: Pane::Detail, bar: false, tabs: false, cursor: &cursor, hits: &hits, parts: crate::compose::Parts::new(&plugins, None) };
    let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
    fe.begin_selectable();
    render_screen_with_ramp_and_jumps(fe, view, buffer.area, &mut buffer, &cx, true, jumps);
    (buffer, hits.into_inner())
}

#[test]
fn native_footer_stays_pinned_while_detail_scrolls_and_never_belongs_to_text_selection() {
    let fe = Frontend::default();
    let view = view();
    let (before, hits) = draw(&fe, &view, 40, 12, &[]);
    let footer: Vec<_> = hits.iter().filter(|(_, _, id)| id.starts_with(crate::view::detail::TARGET_PREFIX)).collect();
    assert_eq!(footer.len(), 2);
    assert!(footer.iter().all(|(rect, _, _)| rect.y == 10 && rect.height == 1));
    assert!(fe.selectable().iter().all(|area| area.rect.bottom() <= 9));
    assert!(matches!(key(&fe, &view, PaneId::Detail, Chord::new(Key::PageDown)), Routed::Done | Routed::Intent(Intent::End { .. })));
    let (after, next_hits) = draw(&fe, &view, 40, 12, &[]);
    assert_ne!(crate::shell::row_text(&before, 1), crate::shell::row_text(&after, 1));
    assert_eq!(crate::shell::row_text(&before, 10), crate::shell::row_text(&after, 10));
    assert!(next_hits.iter().filter(|(_, _, id)| id.starts_with("note:")).all(|(rect, _, _)| rect.bottom() <= 9));
}

#[test]
fn tiny_native_panes_omit_footer_hits_instead_of_covering_the_body() {
    let fe = Frontend::default();
    let view = view();
    for (width, height) in [(0, 0), (1, 1), (3, 3), (8, 4)] {
        let (_, hits) = draw(&fe, &view, width, height, &[]);
        assert!(hits.iter().all(|(_, _, id)| !id.starts_with(crate::view::detail::TARGET_PREFIX)));
    }
}

#[test]
fn explicit_jump_map_keeps_hidden_logical_numbers_and_missing_jumps_unlabelled() {
    let fe = Frontend::default();
    let view = view();
    let jumps = [crate::view::chrome::PaneJump { pane: PaneId::Detail, key: '3' }];
    let (numbered, _) = draw(&fe, &view, 40, 12, &jumps);
    assert!(crate::shell::row_text(&numbered, 0).contains("3 detail"));
    let (plain, _) = draw(&fe, &view, 40, 12, &[]);
    assert!(!crate::shell::row_text(&plain, 0).contains("3 detail"));
}

#[test]
fn extension_picture_discovery_is_recursive_and_deduplicated() {
    let mut content = DetailContent::new(DetailHeader::new("watch", "Inventory"), vec![], vec![]);
    content.pictures = Some("watch-images".into());
    let typed = content.node(&FootActions::new(vec![]));
    let wrap = |fallback| Node::Extension { capability: "unknown".into(), version: 99, data: serde_json::Value::Null, fallback: Box::new(fallback) };
    let view = ScreenView::new("watch", vec![
        PaneView::new(PaneId::Detail, "one", typed.clone()),
        PaneView::new(PaneId::Middle, "two", wrap(wrap(typed))),
    ]);
    assert_eq!(picture_sources(&view), ["watch-images"]);
}

#[test]
fn unknown_or_malformed_detail_uses_readable_fallback_and_scrolls() {
    for malformed in [false, true] {
        let fe = Frontend::default();
        let mut view = view();
        let Node::Extension { version, data, .. } = &mut view.panes[0].body else { unreachable!() };
        if malformed { *data = serde_json::json!({"header": null}); } else { *version = 99; }
        let (buffer, _) = draw(&fe, &view, 40, 12, &[]);
        assert!((0..12).any(|y| crate::shell::row_text(&buffer, y).contains("Count every lantern")));
        assert!((0..12).all(|y| !crate::shell::row_text(&buffer, y).contains("shown simply")));
        assert!(matches!(key(&fe, &view, PaneId::Detail, Chord::new(Key::End)), Routed::Done | Routed::Intent(Intent::End { .. })));
        let (last, _) = draw(&fe, &view, 40, 12, &[]);
        assert!(crate::shell::row_text(&last, 8).contains("extra actions"), "last fallback row remains reachable above its pinned foot");
    }
}

#[test]
fn reserved_footer_targets_never_become_owner_select_or_context() {
    let fe = Frontend::default();
    for target in ["detail-foot:1:0", "detail-foot:bad", "detail-foot:"] {
        for context in [false, true] { assert!(matches!(click(&fe, target, context), Routed::Done)); }
    }
}

#[test]
fn unrelated_extensions_keep_their_existing_owner_movement_keys() {
    let fe = Frontend::default();
    let view = ScreenView::new("watch", vec![PaneView::new(PaneId::Detail, "work", Node::Extension {
        capability: "work.watch".into(), version: 1, data: serde_json::json!({}),
        fallback: Box::new(Node::Rich { blocks: (0..30).map(|i| Block::line(format!("Row {i}"))).collect(), pictures: None }),
    })]);
    let (buffer, _) = draw(&fe, &view, 40, 12, &[]);
    assert!((0..12).any(|y| crate::shell::row_text(&buffer, y).contains("shown simply")));
    let chord = Chord::new(Key::Char('j'));
    assert!(matches!(key(&fe, &view, PaneId::Detail, chord), Routed::Intent(Intent::Key { chord: routed }) if routed == chord));
}

#[test]
fn ordinary_rich_panes_keep_their_existing_full_frame_scrollbar_with_notes() {
    let fe = Frontend::default();
    let mut pane = PaneView::new(PaneId::Detail, "notes", Node::Rich {
        blocks: (0..30).map(|i| Block::line(format!("Row {i}"))).collect(), pictures: None,
    });
    pane.notes.push(Text::plain("Held context"));
    let view = ScreenView::new("watch", vec![pane]);
    let (buffer, _) = draw(&fe, &view, 40, 12, &[]);
    let theme = Theme::build(&ThemeConfig { icons: "off".into(), ..Default::default() }, Tier::TrueColor, &[]);
    assert_eq!(buffer[(39, 10)].fg, theme.s(Slot::BgOverlay).fg.unwrap());
}

#[test]
fn overflowing_detail_keeps_complete_age_cells_clear_of_the_scrollbar() {
    let fe = Frontend::default();
    let mut view = view();
    let mut content = DetailContent::of(&view.panes[0].body).unwrap();
    content.header.branch = Some("lantern-stock".into());
    content.header.author = Some("Sam Vimes".into());
    content.header.age = Some("2d".into());
    view.panes[0].body = content.node(view.panes[0].foot_actions.as_ref().unwrap());
    let theme = Theme::build(&ThemeConfig { icons: "off".into(), ..Default::default() }, Tier::TrueColor, &[]);
    for width in [16, 80] {
        let (buffer, _) = draw(&fe, &view, width, 12, &[]);
        assert!(fe.with_scroll(&view.scope, PaneId::Detail, |scroll| scroll.more()));
        assert_eq!(buffer[(width - 4, 3)].symbol(), "2");
        assert_eq!(buffer[(width - 3, 3)].symbol(), "d");
        assert_eq!(buffer[(width - 2, 3)].symbol(), " ", "header right inset stays clear");
        assert!(matches!(buffer[(width - 1, 3)].symbol(), "█" | "║"), "overflow bar occupies only the border");
        assert_eq!(buffer[(width - 3, 3)].fg, theme.s(Slot::TextMuted).fg.unwrap());
        assert_eq!(buffer[(width - 3, 3)].bg, theme.bg(Slot::BgOverlay).bg.unwrap());
    }
}
