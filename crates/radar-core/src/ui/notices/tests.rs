use super::*;
use crate::theme::ThemeConfig;

fn theme(tier: Tier, icons: &str) -> Theme {
    Theme::build(
        &ThemeConfig {
            icons: icons.into(),
            ..Default::default()
        },
        tier,
        &[],
    )
}
fn notice(id: u64, tone: NoticeTone, text: &str, created_ms: u64) -> NoticeView {
    NoticeView {
        id,
        tone,
        icon: None,
        text: text.into(),
        created_ms,
        lifetime_ms: if tone == NoticeTone::Error {
            10_000
        } else {
            5_000
        },
    }
}
fn text(buf: &Buffer) -> String {
    (buf.area.y..buf.area.bottom())
        .map(|y| crate::shell::row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}
fn history_view(entries: Vec<NoticeView>, detail: Option<NoticeView>) -> NoticeHistoryView {
    NoticeHistoryView {
        entries,
        detail,
        omitted: 0,
        selected: 0,
        scroll: 0,
        detail_scroll: 0,
    }
}

#[test]
fn stack_is_inside_content_newest_bottom_and_expiry_is_exact() {
    let entries = vec![
        notice(1, NoticeTone::Info, "Copied the Watch link", 0),
        notice(2, NoticeTone::Error, "Watch request failed", 1000),
    ];
    let content = Rect::new(1, 2, 78, 20);
    let stack = layout(&entries, 0, content, 2500, true);
    assert_eq!(
        stack
            .toasts
            .iter()
            .map(|n| (n.id, n.area.y, n.fill, n.older))
            .collect::<Vec<_>>(),
        [(1, 20, 32, true), (2, 21, 55, false)]
    );
    assert!(
        stack
            .toasts
            .iter()
            .all(|n| n.area.x == 15 && n.area.right() == content.right())
    );
    assert_eq!(layout(&entries, 0, content, 5000, true).toasts.len(), 1);
    assert!(layout(&entries, 0, content, 11_000, true).toasts.is_empty());
    assert!(
        layout(&entries, 0, content, 2500, false)
            .toasts
            .iter()
            .all(|n| n.fill == 64)
    );
}

#[test]
fn bounded_stack_exposes_overflow_and_narrow_rows_stay_safe() {
    let entries: Vec<_> = (1..=4)
        .map(|i| notice(i, NoticeTone::Info, "Watch note", 0))
        .collect();
    let roomy = layout(&entries, 8, Rect::new(1, 2, 78, 20), 0, true);
    assert_eq!(roomy.toasts.len(), 4);
    assert_eq!(roomy.summary.unwrap().hidden, 8);
    let two = layout(&entries, 8, Rect::new(0, 0, 8, 2), 0, true);
    assert_eq!(two.toasts.len(), 1);
    assert_eq!(two.summary.unwrap().hidden, 11);
    let one = layout(&entries, 8, Rect::new(0, 0, 1, 1), 0, true);
    assert_eq!(one.toasts[0].hidden, 11);
    assert!(one.summary.is_none());
    let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
    assert_eq!(
        paint(&one, &theme(Tier::TrueColor, "emoji"), &mut buf).len(),
        1
    );
    assert_eq!(text(&buf), "");
}

#[test]
fn complete_graphemes_are_clipped_and_wrapped_without_spilling() {
    assert_eq!(fit("界界", 3), "界…");
    assert_eq!(fit("e\u{301}界界", 4), "e\u{301}界…");
    assert_eq!(fit("👩‍💻 notebook", 3), "👩‍💻…");
    assert_eq!(
        wrap("a 界界界\nsecond", 4),
        ["a", "界界", "界", "seco", "nd"]
    );
    assert_eq!(wrap("界", 1), ["…"]);
    assert_eq!(one_line("Watch\n  note\t\u{1b} here"), "Watch note here");
}

#[test]
fn tint_is_host_theme_derived_dimmed_and_static_below_truecolor() {
    let entries = vec![
        notice(1, NoticeTone::Info, "Watch", 0),
        notice(2, NoticeTone::Error, "Watch", 1000),
    ];
    for tier in [Tier::TrueColor, Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        let t = theme(tier, "off");
        let area = Rect::new(0, 0, 20, 2);
        let stack = layout(&entries, 0, area, 2500, true);
        let mut buf = Buffer::empty(area);
        paint(&stack, &t, &mut buf);
        assert!(buf[(0, 0)].modifier.contains(Modifier::DIM));
        assert!(buf[(0, 1)].modifier.contains(Modifier::BOLD));
        if tier == Tier::TrueColor {
            assert_eq!(buf[(0, 0)].bg, tint(&t, NoticeTone::Info).unwrap());
            assert_eq!(buf[(19, 0)].bg, t.s(Slot::BgOverlay).fg.unwrap());
        } else {
            assert_eq!(buf[(0, 0)].bg, buf[(19, 0)].bg);
            assert_eq!(buf[(0, 1)].bg, buf[(19, 1)].bg);
        }
        if tier == Tier::Mono {
            assert!(buf[(0, 0)].modifier.contains(Modifier::REVERSED));
        }
    }
}

#[test]
fn overlay_clears_split_underlying_wide_glyph_and_never_splits_its_drain() {
    let t = theme(Tier::TrueColor, "off");
    let area = Rect::new(0, 0, 68, 1);
    let mut buf = Buffer::empty(area);
    buf.set_string(2, 0, "界", t.s(Slot::TextSecondary));
    let outside = buf[(2, 0)].style();
    let stack = layout(
        &[notice(1, NoticeTone::Info, "界界界", 0)],
        0,
        Rect::new(3, 0, 64, 1),
        4300,
        true,
    );
    paint(&stack, &t, &mut buf);
    assert_eq!(buf[(2, 0)].symbol(), " ");
    assert_eq!(buf[(2, 0)].style(), outside);
    assert_eq!(buf[(11, 0)].bg, buf[(12, 0)].bg);
}

#[test]
fn history_is_inert_and_selected_full_detail_is_independent_of_summary() {
    let mut summary = notice(91, NoticeTone::Error, "Bounded summary", 0);
    summary.icon = Some(Icon::Error);
    let mut full = summary.clone();
    full.text =
        "Full recovery guidance stays readable.\nPress x to choose the current Watch action."
            .into();
    let mut view = history_view(vec![summary], Some(full));
    view.omitted = 12;
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 22));
    let area = buf.area;
    let hits = history(&theme(Tier::TrueColor, "unicode"), &view, area, &mut buf);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].1, 91);
    let text = text(&buf);
    assert!(text.contains("Bounded summary"));
    assert!(text.contains("Full recovery guidance stays readable."));
    assert!(text.contains("Press x to choose the current Watch action."));
    assert!(text.contains("12 older omitted"));
}

#[test]
fn overflow_summary_clips_to_buffer_and_ignores_empty_or_outside_geometry() {
    let t = theme(Tier::TrueColor, "off");
    let area = Rect::new(2, 3, 4, 2);
    let before = Buffer::empty(area);
    for summary in [
        Rect::new(99, 99, 1, 1),
        Rect::new(2, 3, 0, 1),
        Rect::new(2, 3, 1, 0),
    ] {
        let mut buf = before.clone();
        paint(
            &Stack {
                toasts: vec![],
                summary: Some(Summary {
                    area: summary,
                    hidden: 9,
                }),
            },
            &t,
            &mut buf,
        );
        assert_eq!(buf, before);
    }
    let mut buf = before.clone();
    paint(
        &Stack {
            toasts: vec![],
            summary: Some(Summary {
                area: Rect::new(0, 4, 10, 1),
                hidden: 9,
            }),
        },
        &t,
        &mut buf,
    );
    assert_eq!(buf[(2, 4)].symbol(), "+");
    assert_eq!(buf[(5, 4)].symbol(), "…");
    for x in area.x..area.right() {
        assert_eq!(buf[(x, 3)], before[(x, 3)]);
    }
}

#[path = "captures.rs"]
mod captures;
