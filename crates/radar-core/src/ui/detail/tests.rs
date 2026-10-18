use super::*;
use crate::design::theme::Tier;
use crate::theme::ThemeConfig;
use crate::view::detail::{DetailFact, DetailHeader};
use crate::view::{ActionSpec, Change, Target};

fn theme() -> Theme { Theme::build(&ThemeConfig { icons: "off".into(), ..Default::default() }, Tier::TrueColor, &[]) }
fn content() -> DetailContent {
    let mut header = DetailHeader::new("watch / supplies", "Check lantern stock");
    header.branch = Some(Text::plain("lantern-stock"));
    header.author = Some(Text::plain("Sam Vimes"));
    header.age = Some(Text::plain("2d"));
    DetailContent::new(header, vec![DetailFact::new("Count", "0"), DetailFact::new("Location", "Ankh Morpork watch house")], vec![
        Block::section(None, "notes", None, vec![Block::line("A steady light at every gate")]),
    ])
}
fn control(key: &str, label: &str, target: &str) -> FootAction {
    let mut control = FootAction::new(ActionSpec::named(format!("watch.{target}"), &crate::Binding::new(key, label)));
    control.target = Some(Target::new(format!("detail-foot:1:{target}")));
    control
}
fn foot() -> FootActions {
    let mut foot = FootActions::new(vec![control("e", "edit", "0"), control("r", "reply", "1"), control("d", "drop", "2").disabled("waiting")]);
    foot.menu.target = Some(Target::new("detail-foot:1:menu"));
    foot
}

#[test]
fn header_fills_all_three_rows_and_keeps_identity_separate_from_metadata() {
    let t = theme();
    let rows = layout(&t, &content(), 40, false);
    assert_eq!(rows.lines[0].width(), 40);
    assert_eq!(rows.lines[1].width(), 40);
    assert_eq!(rows.lines[2].width(), 40);
    assert!(words(&rows.lines[2]).contains("lantern-stock"));
    assert!(words(&rows.lines[2]).ends_with("Sam Vimes · 2d "));
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 40, 3));
    for (y, row) in rows.lines.iter().take(3).enumerate() { buffer.set_line(0, y as u16, row, 40); }
    for cell in buffer.content { assert_eq!(cell.bg, t.bg(Slot::BgOverlay).bg.unwrap()); }
}

#[test]
fn header_age_wins_when_branch_and_author_do_not_fit() {
    let t = theme();
    let mut c = content();
    c.header.age = Some("123456d".into());
    let rows = layout(&t, &c, 8, false);
    assert_eq!(words(&rows.lines[2]), " 12345… ");
    c.header.branch = None;
    c.header.age = Some("2d".into());
    let rows = layout(&t, &c, 12, false);
    assert_eq!(words(&rows.lines[2]), " Sam … · 2d ");
}

#[test]
fn facts_align_wrapped_values_and_stack_below_twelve_content_cells() {
    let t = theme();
    let rows = layout(&t, &content(), 24, false);
    assert_eq!(words(&rows.lines[4]), " Count    0");
    assert_eq!(words(&rows.lines[5]), " Locati…  Ankh Morpork");
    assert_eq!(words(&rows.lines[6]), "          watch house");
    let narrow = layout(&t, &content(), 13, false);
    assert_eq!(words(&narrow.lines[4]), " Count      ");
    assert_eq!(words(&narrow.lines[5]), " 0");
}

#[test]
fn empty_and_zero_width_detail_does_not_invent_facts_or_trailing_rows() {
    let t = theme();
    let c = DetailContent::new(DetailHeader::new("", ""), vec![], vec![]);
    assert!(layout(&t, &c, 0, false).lines.is_empty());
    assert_eq!(layout(&t, &c, 40, false).lines.len(), 3);
}

#[test]
fn unicode_labels_values_and_header_clip_without_splitting_graphemes() {
    let t = theme();
    let mut c = content();
    c.header.title = Text::new().push(Seg::new("e", Ink::Accent)).push(Seg::new("\u{301}中文🙂 inventory", Ink::Secondary));
    c.facts = vec![DetailFact::new("場所", "中文 e\u{301} 🙂"), DetailFact::new("e\u{301}tiquette", "0")];
    for width in [1, 2, 3, 7, 11, 14, 24, 40] {
        let rows = layout(&t, &c, width, false);
        assert!(rows.lines.iter().all(|line| line.width() <= usize::from(width)), "width {width}");
        if width >= 4 { assert!(words(&rows.lines[1]).contains("e\u{301}")); }
    }
}

#[test]
fn section_rows_and_inline_hit_offsets_share_wrapping_and_insets() {
    let t = theme();
    let c = DetailContent::new(DetailHeader::new("watch", "Inventory"), vec![], vec![
        Block::section(None, "notes", None, vec![Block::line(Text::new().push(Seg::new("open ", Ink::Plain)).push(Seg::new("watch-house records", Ink::Accent).targeted("records")))]),
        Block::section(None, "next", None, vec![Block::action("continue", "Continue checking lanterns")]),
    ]);
    let rows = layout(&t, &c, 20, false);
    assert_eq!(words(&rows.lines[4]).split_whitespace().next(), Some("NOTES"));
    assert_eq!(words(&rows.lines[5]), "   open watch-house");
    assert_eq!(words(&rows.lines[6]), "   records");
    let first = rows.hits.iter().find(|hit| hit.target == "records").unwrap();
    assert_eq!((first.row, first.x, first.width), (5, 8, 11));
    assert!(rows.hits.iter().any(|hit| hit.target == "records" && hit.row == 6 && hit.x == 3 && hit.width == 7));
    for hit in &rows.hits { assert!(hit.row < rows.lines.len()); assert!(hit.x + hit.width <= 20); }
}

#[test]
fn footer_reserves_only_two_rows_after_a_body_row_exists() {
    for height in 0..6 {
        let area = Rect::new(2, 3, 20, height);
        let (body, footer) = reserve(area, true);
        if height < 3 { assert_eq!(body, area); assert_eq!(footer, None); }
        else { assert_eq!(body.height, height - 2); assert_eq!(footer.unwrap(), Rect::new(2, body.bottom(), 20, 2)); }
    }
    assert_eq!(reserve(Rect::new(0, 0, 0, 8), true).1, None);
}

#[test]
fn footer_menu_and_whole_prefix_fit_without_separator_or_disabled_hits() {
    let t = theme();
    let foot = foot();
    let controls = controls(&t, &foot, 40, Mode::Edit);
    assert_eq!(controls.len(), 4);
    assert_eq!(controls[0].x, 0);
    assert_eq!(controls[1].x, 6);
    assert!(controls[2].target.is_none());
    let menu = controls.last().unwrap();
    assert_eq!(menu.x + menu.line.width() as u16, 40);
    let narrow = super::controls(&t, &foot, 19, Mode::Edit);
    assert_eq!(narrow.len(), 2, "only edit and menu fit");
    let tiny = super::controls(&t, &foot, 1, Mode::Edit);
    assert_eq!(tiny.len(), 1);
    assert_eq!(words(&tiny[0].line), "x");
    assert_eq!(tiny[0].target.as_deref(), Some("detail-foot:1:menu"));
    assert!(super::controls(&t, &foot, 0, Mode::Edit).is_empty());
}

#[test]
fn write_mode_mutes_but_keeps_the_original_enabled_control_and_mnemonic() {
    let t = theme();
    let mut foot = foot();
    foot.actions[0].action.change = Some(Change::Edit);
    let read = controls(&t, &foot, 40, Mode::ReadOnly);
    assert!(read[0].target.is_some());
    assert!(read[0].line.spans.iter().all(|span| span.style.fg == t.s(Slot::TextMuted).fg));
    assert!(read[0].line.spans.iter().any(|span| span.style.add_modifier.contains(Modifier::UNDERLINED)));
    let edit = controls(&t, &foot, 40, Mode::Edit);
    assert!(edit[0].line.spans.iter().any(|span| span.style.fg == t.s(Slot::TextSecondary).fg));
}

#[test]
fn footer_glyph_shift_and_invalid_letter_keys_remain_discoverable() {
    let t = theme();
    for (key, caption, expected) in [("⏎", "open", "⏎ open"), ("E", "edit", "shift edit"), ("r", "open", "open [r]")] {
        let action = control(key, caption, "0");
        assert_eq!(words(&action_line(&t, &action, Mode::Edit)), expected);
    }
}

#[test]
fn multiline_facts_keep_blank_lines_and_aligned_continuations() {
    let t = theme();
    let c = DetailContent::new(DetailHeader::new("watch", "Inventory"), vec![
        DetailFact::new("Owner", Text::new().push(Seg::new("Sam\n", Ink::Accent)).push(Seg::new("\nVimes", Ink::Secondary))),
    ], vec![]);
    let rows = layout(&t, &c, 30, false);
    assert_eq!(words(&rows.lines[4]), " Owner  Sam");
    assert_eq!(words(&rows.lines[5]), "        ");
    assert_eq!(words(&rows.lines[6]), "        Vimes");
    let narrow = layout(&t, &c, 8, false);
    assert_eq!(words(&narrow.lines[5]), " Sam");
    assert_eq!(words(&narrow.lines[6]), " ");
    assert_eq!(words(&narrow.lines[7]), " Vimes");
}

#[test]
#[allow(deprecated)]
fn media_and_later_action_hits_use_the_same_exact_laid_out_row_offsets() {
    use ratatui_image::{FontSize, picker::{Picker, ProtocolType}};
    let mut picker = Picker::from_fontsize(FontSize { width: 10, height: 20 });
    picker.set_protocol_type(ProtocolType::Kitty);
    super::super::image::start(picker, None);
    struct Stop;
    impl Drop for Stop { fn drop(&mut self) { crate::ui::image::stop(); } }
    let _stop = Stop;
    let t = theme();
    let doc = super::super::rich::Doc::new(vec![super::super::rich::Block::Media(super::super::rich::Media {
        image: true, src: "detail-layout-image".into(), name: "lantern.png".into(), size: Some((100, 60)), ..Default::default()
    })]);
    let c = DetailContent::new(DetailHeader::new("watch", "Inventory"), vec![DetailFact::new("Count", "0")], vec![
        Block::section(None, "notes", None, vec![Block::line("Before image"), Block::doc(doc), Block::action("after", "After image")]),
    ]);
    let rows = layout(&t, &c, 40, true);
    let image = &rows.media[0];
    assert_eq!((image.line, image.x, image.rows), (8, 3, 3));
    let after = rows.hits.iter().find(|hit| hit.target == "after").unwrap();
    assert_eq!(words(&rows.lines[after.row]).trim(), "After image");
    assert!(after.row >= image.line + usize::from(image.rows));
    assert_eq!(after.x, image.x);
}

#[test]
fn fact_labels_do_not_mute_plain_first_values_or_their_continuations() {
    let t = theme();
    let c = DetailContent::new(DetailHeader::new("watch", "Inventory"), vec![
        DetailFact::new("Owner", "Night Watch\nNorth gate"),
        DetailFact::new("Count", "0"),
        DetailFact::new("State", Text::of("Ready", Ink::Good)),
    ], vec![]);
    let rows = layout(&t, &c, 24, false);
    let area = Rect::new(0, 0, 24, rows.lines.len() as u16);
    let mut buffer = ratatui::buffer::Buffer::empty(area);
    buffer.set_style(area, t.s(Slot::TextPrimary).patch(t.bg(Slot::BgSurface)));
    for (y, row) in rows.lines.iter().enumerate() { buffer.set_line(0, y as u16, row, 24); }
    assert_eq!(buffer[(1, 4)].fg, t.s(Slot::TextMuted).fg.unwrap(), "label alone is muted");
    for y in [4, 5, 6] {
        assert_eq!(buffer[(8, y)].fg, t.s(Slot::TextPrimary).fg.unwrap(), "plain value row {y}");
    }
    assert_eq!(buffer[(8, 7)].fg, t.s(Slot::StatusSuccess).fg.unwrap(), "explicit value ink survives");
}

#[test]
fn actual_gallery_initial_view_shows_footer_without_claiming_digit_jumps() {
    let t = theme();
    for (width, height) in [(80, 24), (160, 40)] {
        let mut gallery = crate::ui::gallery::Gallery::new();
        gallery.tab = crate::ui::gallery::tab_names().iter().position(|name| *name == "Detail").unwrap();
        let area = Rect::new(0, 0, width, height);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        gallery.render(&t, area, &mut buffer, "colours: full colour");
        let rows: Vec<_> = (0..height).map(|y| crate::shell::row_text(&buffer, y)).collect();
        assert!(rows.iter().any(|row| row.contains("extra actions")), "{width}x{height} initial footer visible");
        assert!(!rows.iter().any(|row| row.contains("3 shared identity") || row.contains("3 missing facts")));
    }
}
