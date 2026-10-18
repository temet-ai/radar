use std::collections::HashSet;

use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Widget};

use super::*;
use crate::theme::ThemeConfig;

fn theme(tier: Tier) -> Theme {
    Theme::build(
        &ThemeConfig {
            icons: "off".into(),
            ..Default::default()
        },
        tier,
        &[],
    )
}

fn test_ramp() -> Ramp {
    Ramp([(0, 0, 0), (255, 100, 1), (0, 200, 255)])
}

#[test]
fn samples_include_all_three_stops_and_use_integer_theme_mixing() {
    let ramp = test_ramp();
    assert_eq!(ramp.at(0, 5), Color::Rgb(0, 0, 0));
    assert_eq!(ramp.at(1, 5), Color::Rgb(128, 50, 1));
    assert_eq!(ramp.at(2, 5), Color::Rgb(255, 100, 1));
    assert_eq!(ramp.at(3, 5), Color::Rgb(128, 150, 128));
    assert_eq!(ramp.at(4, 5), Color::Rgb(0, 200, 255));
    assert_eq!(ramp.at(1, 4), mix(ramp.0[0], ramp.0[1], 666));
    assert_eq!(ramp.at(2, 4), mix(ramp.0[1], ramp.0[2], 333));
    assert_eq!(ramp.at(0, 0), ramp.at(0, 1));
    assert_eq!(ramp.at(usize::MAX, 1), Color::Rgb(0, 0, 0));
    assert_eq!(ramp.at(usize::MAX, 5), Color::Rgb(0, 200, 255));
    assert_eq!(ramp.at(usize::MAX, usize::MAX), Color::Rgb(0, 200, 255));
}

#[test]
fn ramp_uses_resolved_user_overrides() {
    let mut cfg = ThemeConfig {
        icons: "off".into(),
        colors: "truecolor".into(),
        ..Default::default()
    };
    cfg.slots.insert("glow.purple".into(), "#102030".into());
    cfg.slots.insert("accent.tertiary".into(), "#405060".into());
    cfg.slots.insert("glow.cyan".into(), "#708090".into());
    let t = Theme::build(&cfg, Tier::Mono, &[]);
    let ramp = Ramp::from_theme(&t).unwrap();
    assert_eq!(ramp.at(0, 3), Color::Rgb(0x10, 0x20, 0x30));
    assert_eq!(ramp.at(1, 3), Color::Rgb(0x40, 0x50, 0x60));
    assert_eq!(ramp.at(2, 3), Color::Rgb(0x70, 0x80, 0x90));
}

#[test]
fn perimeter_walks_clockwise_from_top_left_with_each_corner_once() {
    let points: Vec<_> = perimeter(Rect::new(4, 7, 4, 3))
        .map(|p| (p.x, p.y))
        .collect();
    assert_eq!(
        points,
        vec![
            (4, 7),
            (5, 7),
            (6, 7),
            (7, 7),
            (7, 8),
            (7, 9),
            (6, 9),
            (5, 9),
            (4, 9),
            (4, 8),
        ]
    );
    assert_eq!(points.len(), points.iter().collect::<HashSet<_>>().len());
}

#[test]
fn tiny_perimeters_are_unique_and_have_no_interior_cells() {
    for w in 0..=5 {
        for h in 0..=5 {
            let area = Rect::new(3, 8, w, h);
            let points: Vec<_> = perimeter(area).collect();
            let expected = match (w, h) {
                (0, _) | (_, 0) => 0,
                (_, 1) => w,
                (1, _) => h,
                _ => 2 * w + 2 * h - 4,
            };
            assert_eq!(points.len(), usize::from(expected));
            assert_eq!(points.len(), points.iter().collect::<HashSet<_>>().len());
            for p in &points {
                assert!(area.contains(*p));
                assert!(
                    p.x == area.x
                        || p.x + 1 == area.right()
                        || p.y == area.y
                        || p.y + 1 == area.bottom()
                );
            }
        }
    }
    assert_eq!(
        perimeter(Rect::new(1, 2, 1, 3)).collect::<Vec<_>>(),
        vec![
            Position::new(1, 2),
            Position::new(1, 3),
            Position::new(1, 4)
        ],
    );
    assert_eq!(
        perimeter(Rect::new(1, 2, 3, 1)).collect::<Vec<_>>(),
        vec![
            Position::new(1, 2),
            Position::new(2, 2),
            Position::new(3, 2)
        ],
    );
}

#[test]
fn title_regions_use_cell_width_and_leave_corners_free() {
    let area = Rect::new(4, 7, 9, 3);
    assert_eq!(
        title_region(area, super::super::width(" 界 "), TitleEdge::Top),
        Rect::new(5, 7, 4, 1)
    );
    assert_eq!(
        title_region(area, usize::MAX, TitleEdge::Bottom),
        Rect::new(5, 9, 7, 1)
    );
    assert_eq!(
        title_region(Rect::new(0, 0, 1, 1), 7, TitleEdge::Top).width,
        0
    );
    assert!(title_region(Rect::new(0, 0, 9, 0), 7, TitleEdge::Bottom).is_empty());
}

#[test]
fn border_changes_only_foreground_and_preserves_explicit_top_and_bottom_titles() {
    let t = theme(Tier::TrueColor);
    let area = Rect::new(0, 0, 14, 5);
    let base = Style::new()
        .fg(Color::White)
        .bg(Color::Black)
        .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
    let mut buf = Buffer::empty(area);
    buf.set_style(area, base);
    for y in 0..area.height {
        for x in 0..area.width {
            // The overlay cannot classify a border by its symbol or style.
            buf[(x, y)].set_symbol("x");
        }
    }
    let before = buf.clone();
    let keep = [
        title_region(area, 4, TitleEdge::Top),
        title_region(area, 7, TitleEdge::Bottom),
    ];
    focus_border(&t, area, &mut buf, true, &keep);
    let ramp = Ramp::from_theme(&t).unwrap();
    let border: Vec<_> = perimeter(area).collect();
    for y in 0..area.height {
        for x in 0..area.width {
            let pos = Position::new(x, y);
            let old = &before[(x, y)];
            let cell = &buf[(x, y)];
            if let Some(index) = border.iter().position(|p| *p == pos)
                && !keep.iter().any(|r| r.contains(pos))
            {
                let mut expected = old.clone();
                expected.set_fg(ramp.at(index, border.len()));
                assert_eq!(cell, &expected);
            } else {
                assert_eq!(cell, old);
            }
        }
    }
}

#[test]
fn lower_tiers_are_unchanged_and_truecolor_non_owner_borders_are_quiet() {
    for tier in [Tier::TrueColor, Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        let t = theme(tier);
        for owns_keys in [false, true] {
            let area = Rect::new(0, 0, 12, 4);
            let mut buf = Buffer::empty(area);
            crate::widgets::panel(&t, "frame", true).render(area, &mut buf);
            let before = buf.clone();
            focus_border(&t, area, &mut buf, owns_keys, &[]);
            if tier != Tier::TrueColor {
                assert_eq!(buf, before);
            } else {
                assert_ne!(buf, before);
                if !owns_keys {
                    for pos in perimeter(area) {
                        let mut expected = before[pos].clone();
                        expected.set_fg(t.s(Slot::AccentMuted).fg.unwrap());
                        assert_eq!(buf[pos], expected);
                    }
                }
            }
        }
    }
}

#[test]
fn real_block_title_cells_survive_and_later_scrollbars_keep_their_styles() {
    let t = theme(Tier::TrueColor);
    let area = Rect::new(0, 0, 16, 5);
    let top = " 界a\u{301} ";
    let bottom = " unsaved ";
    let mut actual = Buffer::empty(area);
    let block = Block::new()
        .borders(Borders::ALL)
        .title(Line::styled(top, t.bold(Slot::TextEmphasis)))
        .title_bottom(Line::styled(bottom, t.s(Slot::StatusWarning)));
    block.render(area, &mut actual);
    let before = actual.clone();
    let keep = [
        title_region(area, super::super::width(top), TitleEdge::Top),
        title_region(area, super::super::width(bottom), TitleEdge::Bottom),
    ];
    focus_border(&t, area, &mut actual, true, &keep);
    for region in keep {
        for x in region.x..region.right() {
            assert_eq!(actual[(x, region.y)], before[(x, region.y)]);
        }
    }
    let mut baseline = before;
    super::super::scroll::scrollbar(&t, area, &mut baseline, 8, 3);
    super::super::scroll::scrollbar(&t, area, &mut actual, 8, 3);
    for y in area.y + 1..area.bottom() - 1 {
        assert_eq!(
            actual[(area.right() - 1, y)],
            baseline[(area.right() - 1, y)]
        );
    }
}

#[test]
fn border_is_safe_for_empty_and_partly_outside_buffers() {
    let t = theme(Tier::TrueColor);
    let mut buf = Buffer::empty(Rect::new(2, 2, 3, 3));
    let before = buf.clone();
    focus_border(&t, Rect::new(0, 0, 0, 3), &mut buf, true, &[]);
    assert_eq!(buf, before);
    focus_border(&t, Rect::new(0, 0, 5, 5), &mut buf, true, &[]);
    let ramp = Ramp::from_theme(&t).unwrap();
    let perimeter: Vec<_> = perimeter(Rect::new(0, 0, 5, 5)).collect();
    let index = perimeter
        .iter()
        .position(|p| *p == Position::new(4, 2))
        .unwrap();
    assert_eq!(buf[(4, 2)].fg, ramp.at(index, perimeter.len()));
    assert_eq!(buf[(2, 2)], before[(2, 2)]);
}

#[test]
fn wordmark_keeps_graphemes_and_advances_by_display_cells() {
    let t = theme(Tier::TrueColor);
    let base = t
        .bold(Slot::AccentPrimary)
        .bg(Color::Black)
        .add_modifier(Modifier::UNDERLINED);
    let spans = wordmark(&t, "a\u{301}界z", base);
    let symbols: Vec<_> = spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(symbols, vec!["a\u{301}", "界", "z"]);
    assert_eq!(spans.iter().map(Span::width).sum::<usize>(), 4);
    let ramp = Ramp::from_theme(&t).unwrap();
    for (span, x) in spans.iter().zip([0, 1, 3]) {
        assert_eq!(span.style, base.fg(ramp.at(x, 4)));
    }
    assert_eq!(spans[2].style.fg, t.s(Slot::GlowCyan).fg);
    let emoji = wordmark(&t, "👩‍💻z", base);
    assert_eq!(emoji[0].content, "👩‍💻");
    assert_eq!(emoji[1].content, "z");
    assert_eq!(
        emoji.iter().map(Span::width).sum::<usize>(),
        super::super::width("👩‍💻z")
    );
}

#[test]
fn wordmark_keeps_low_tier_content_styles_and_empty_or_single_cells() {
    for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        let t = theme(tier);
        let base = t.bold(Slot::AccentPrimary);
        assert_eq!(
            wordmark(&t, "radar", base),
            vec![Span::styled("radar", base)]
        );
        assert_eq!(wordmark(&t, "", base), vec![Span::styled("", base)]);
    }
    let t = theme(Tier::TrueColor);
    let base = t.bold(Slot::AccentPrimary);
    assert!(wordmark(&t, "", base).is_empty());
    assert_eq!(
        wordmark(&t, "a\u{301}", base),
        vec![Span::styled(
            "a\u{301}",
            base.fg(t.s(Slot::GlowPurple).fg.unwrap())
        )]
    );
}

#[test]
fn owned_panel_uses_the_blocks_clipped_frame_but_keeps_legacy_inner() {
    let t = theme(Tier::TrueColor);
    let requested = Rect::new(0, 0, 18, 8);
    let actual = Rect::new(3, 2, 12, 5);
    let mut flat = Buffer::empty(actual);
    let inner = super::super::card::panel(&t, requested, &mut flat, "界", true);
    let mut ramped = Buffer::empty(actual);
    let got = super::super::card::panel_owned(
        &t,
        requested,
        &mut ramped,
        "界",
        Focus {
            focused: true,
            owns_keys: true,
        },
    );
    assert_eq!(got, inner);
    let ramp = Ramp::from_theme(&t).unwrap();
    let points: Vec<_> = perimeter(actual).collect();
    let title = title_region(actual, 4, TitleEdge::Top);
    for (index, pos) in points.iter().enumerate() {
        let mut expected = flat[(pos.x, pos.y)].clone();
        if !title.contains(*pos) {
            expected.set_fg(ramp.at(index, points.len()));
        }
        assert_eq!(ramped[(pos.x, pos.y)], expected);
    }
}

#[test]
fn owned_card_keeps_the_scrollbar_and_lower_tier_buffers_exactly() {
    let area = Rect::new(0, 0, 18, 6);
    for tier in [Tier::TrueColor, Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        let t = theme(tier);
        let lines = || (0..20).map(|n| Line::raw(format!("row {n}"))).collect();
        let mut old = Buffer::empty(area);
        let mut new = Buffer::empty(area);
        let scroll = super::super::Scroll::new();
        super::super::card::card(&t, area, &mut old, "scroll", true, &scroll, lines());
        super::super::card::card_owned(
            &t,
            area,
            &mut new,
            "scroll",
            Focus {
                focused: true,
                owns_keys: true,
            },
            &scroll,
            lines(),
        );
        if tier != Tier::TrueColor {
            assert_eq!(new, old);
        } else {
            assert_ne!(new, old);
            for y in 1..area.height - 1 {
                assert_eq!(new[(area.width - 1, y)], old[(area.width - 1, y)]);
            }
        }
    }
}
