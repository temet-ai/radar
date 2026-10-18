//! Unit fixtures only: caller-owned tiny buffers and explicit themes. No Host,
//! renderer Frame, environment, filesystem, wall clock, timers or global marks.

use ratatui::buffer::CellDiffOption;
use ratatui::style::{Modifier, Style};

use super::*;
use crate::theme::ThemeConfig;
use crate::ui::frame::{TitleEdge, focus_border, title_region};

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

fn color(n: u8) -> Color {
    Color::Rgb(n, n.saturating_add(1), n.saturating_add(2))
}

fn styled(symbol: &str, n: u8) -> Cell {
    let mut cell = Cell::default();
    cell.set_symbol(symbol)
        .set_fg(color(n))
        .set_bg(color(n.saturating_add(10)))
        .set_style(Style::new().add_modifier(Modifier::BOLD | Modifier::ITALIC))
        .set_diff_option(CellDiffOption::AlwaysUpdate);
    cell
}

fn clean(area: Rect) -> Buffer {
    let mut buf = Buffer::empty(area);
    for pos in area.positions() {
        buf[pos] = styled(".", 9);
    }
    buf
}

fn row_cells(rows: &str, width: u16) -> Vec<Cell> {
    rows.chars()
        .flat_map(|ch| (0..width).map(move |_| styled(&ch.to_string(), ch as u8)))
        .collect()
}

fn put_rows(buf: &mut Buffer, area: Rect, cells: &[Cell]) {
    for (pos, cell) in area.positions().zip(cells) {
        buf[pos] = cell.clone();
    }
}

fn row_labels(buf: &Buffer, area: Rect) -> String {
    (area.y..area.bottom())
        .map(|y| buf[(area.x, y)].symbol())
        .collect()
}

fn assert_outside_unchanged(before: &Buffer, after: &Buffer, area: Rect) {
    for pos in before.area.positions() {
        if !area.contains(pos) {
            assert_eq!(before[pos], after[pos], "outside cell {pos:?}");
        }
    }
}

fn border(area: Rect) -> Border<'static> {
    Border {
        area,
        top_title: None,
        keep: &[],
    }
}

fn test_band() -> FocusBand {
    FocusBand([
        (1, 3, 5),
        (11, 13, 15),
        (21, 23, 25),
        (31, 33, 35),
        (41, 43, 45),
        (51, 53, 55),
        (61, 63, 65),
    ])
}

fn assert_foreground_only(before: &Buffer, after: &Buffer, geometry: Border<'_>) {
    let points: Vec<_> = perimeter(geometry.area).collect();
    for pos in before.area.positions() {
        if points.contains(&pos) && !geometry.protected(pos) {
            let mut restored = after[pos].clone();
            restored.fg = before[pos].fg;
            assert_eq!(restored, before[pos], "non-foreground change at {pos:?}");
        } else {
            assert_eq!(after[pos], before[pos], "protected/non-border cell {pos:?}");
        }
    }
}

#[test]
fn fold_integer_steps_and_reverse_are_exact_at_quarters_and_endpoint() {
    for (ms, open, close) in [
        (0, 0, 4),
        (40, 1, 3),
        (80, 2, 2),
        (120, 3, 1),
        (159, 3, 1),
        (160, 4, 0),
        (u32::MAX, 4, 0),
    ] {
        assert_eq!(fold_visible(4, ms, FoldDirection::Open), open);
        assert_eq!(fold_visible(4, ms, FoldDirection::Close), close);
    }
    assert_eq!(fold_visible(0, 80, FoldDirection::Open), 0);
    assert_eq!(fold_visible(0, 80, FoldDirection::Close), 0);
    assert_eq!(
        fold_visible(usize::MAX, 160, FoldDirection::Open),
        usize::MAX
    );
    assert_eq!(fold_visible(usize::MAX, 160, FoldDirection::Close), 0);
}

#[test]
fn opening_unrolls_prefix_and_displaces_compact_tail_without_duplicate_rows() {
    let viewport = Rect::new(2, 2, 3, 7);
    let suffix = Rect::new(2, 3, 3, 6);
    let expanded = row_cells("ABCDuv", 3);
    let compact = row_cells("uvwxyz", 3);
    let mut base = clean(Rect::new(0, 0, 8, 11));
    put_rows(&mut base, suffix, &expanded);
    for (ms, expected) in [
        (0, "uvwxyz"),
        (40, "Auvwxy"),
        (80, "ABuvwx"),
        (120, "ABCuvw"),
        (159, "ABCuvw"),
        (160, "ABCDuv"),
    ] {
        let mut got = base.clone();
        let active = compose_fold(
            &mut got,
            viewport,
            FoldRows {
                area: suffix,
                expanded: &expanded,
                compact: &compact,
                descendants: 4,
                direction: FoldDirection::Open,
            },
            ms,
        );
        assert_eq!(active, ms < FOLD_MS);
        assert_eq!(row_labels(&got, suffix), expected);
        assert_outside_unchanged(&base, &got, suffix);
        if ms == FOLD_MS {
            assert_eq!(got, base);
        }
    }
}

#[test]
fn closing_preserves_old_descendant_cells_then_removes_bottommost_first() {
    let viewport = Rect::new(1, 1, 2, 7);
    let suffix = Rect::new(1, 2, 2, 6);
    let expanded = row_cells("ABCDuv", 2);
    let compact = row_cells("uvwxyz", 2);
    let mut base = clean(Rect::new(0, 0, 5, 9));
    put_rows(&mut base, suffix, &compact);
    for (ms, expected) in [
        (0, "ABCDuv"),
        (40, "ABCuvw"),
        (80, "ABuvwx"),
        (120, "Auvwxy"),
        (159, "Auvwxy"),
        (160, "uvwxyz"),
    ] {
        let mut got = base.clone();
        compose_fold(
            &mut got,
            viewport,
            FoldRows {
                area: suffix,
                expanded: &expanded,
                compact: &compact,
                descendants: 4,
                direction: FoldDirection::Close,
            },
            ms,
        );
        assert_eq!(row_labels(&got, suffix), expected);
        assert_outside_unchanged(&base, &got, suffix);
        if ms < FOLD_MS {
            assert_eq!(got[(1, 2)], expanded[0], "old glyph/style must survive");
        } else {
            assert_eq!(got, base);
        }
    }
}

#[test]
fn single_row_fold_stays_discrete_until_its_last_step() {
    let area = Rect::new(0, 0, 1, 1);
    let expanded = row_cells("A", 1);
    let compact = row_cells("u", 1);
    for (direction, ms, expected) in [
        (FoldDirection::Open, 0, "u"),
        (FoldDirection::Open, 80, "u"),
        (FoldDirection::Open, 159, "u"),
        (FoldDirection::Close, 0, "A"),
        (FoldDirection::Close, 80, "A"),
        (FoldDirection::Close, 159, "A"),
    ] {
        let mut buf = clean(area);
        assert!(compose_fold(
            &mut buf,
            area,
            FoldRows {
                area,
                expanded: &expanded,
                compact: &compact,
                descendants: 1,
                direction,
            },
            ms,
        ));
        assert_eq!(row_labels(&buf, area), expected);
    }
}

#[test]
fn logical_descendants_are_capped_to_the_retained_viewport() {
    let area = Rect::new(1, 1, 1, 4);
    let expanded = row_cells("ABCD", 1);
    let compact = row_cells("uvwx", 1);
    for direction in [FoldDirection::Open, FoldDirection::Close] {
        let mut buf = clean(Rect::new(0, 0, 3, 6));
        assert!(compose_fold(
            &mut buf,
            area,
            FoldRows {
                area,
                expanded: &expanded,
                compact: &compact,
                descendants: usize::MAX,
                direction,
            },
            80,
        ));
        assert_eq!(row_labels(&buf, area), "ABuv");
    }
}

#[test]
fn fold_clones_wide_graphemes_continuations_and_every_cell_attribute() {
    let area = Rect::new(1, 2, 4, 2);
    let mut expanded = row_cells("AB", 4);
    expanded[0] = styled("界", 20);
    expanded[1] = styled("", 21);
    expanded[1].set_diff_option(CellDiffOption::Skip);
    expanded[2] = styled("e\u{301}", 22);
    expanded[3] = styled("x", 23);
    let compact = row_cells("uv", 4);
    let mut buf = clean(Rect::new(0, 0, 7, 5));
    let before = buf.clone();
    assert!(compose_fold(
        &mut buf,
        area,
        FoldRows {
            area,
            expanded: &expanded,
            compact: &compact,
            descendants: 2,
            direction: FoldDirection::Close,
        },
        80,
    ));
    for x in 0..4_u16 {
        assert_eq!(buf[(1 + x, 2)], expanded[usize::from(x)]);
        assert_eq!(buf[(1 + x, 3)], compact[usize::from(x)]);
    }
    assert_outside_unchanged(&before, &buf, area);
}

#[test]
fn fold_rejects_incomplete_rows_bad_geometry_and_wide_right_edge_atomically() {
    let area = Rect::new(1, 1, 2, 2);
    let expanded = row_cells("AB", 2);
    let compact = row_cells("uv", 2);
    let original = clean(Rect::new(0, 0, 4, 4));
    let rows = FoldRows {
        area,
        expanded: &expanded,
        compact: &compact,
        descendants: 2,
        direction: FoldDirection::Close,
    };
    for invalid in [
        FoldRows {
            expanded: &expanded[..3],
            ..rows
        },
        FoldRows {
            compact: &compact[..3],
            ..rows
        },
        FoldRows {
            descendants: 0,
            ..rows
        },
        FoldRows {
            area: Rect::new(1, 1, 0, 2),
            ..rows
        },
        FoldRows {
            area: Rect::new(3, 3, 2, 2),
            ..rows
        },
    ] {
        let mut buf = original.clone();
        assert!(!compose_fold(&mut buf, original.area, invalid, 80));
        assert_eq!(buf, original);
    }
    let mut bad = expanded.clone();
    bad[1].set_symbol("界");
    let mut buf = original.clone();
    assert!(!compose_fold(
        &mut buf,
        original.area,
        FoldRows {
            expanded: &bad,
            ..rows
        },
        80
    ));
    assert_eq!(buf, original);
    bad[1]
        .set_symbol("x")
        .set_diff_option(CellDiffOption::ForcedWidth(
            std::num::NonZeroU16::new(2).unwrap(),
        ));
    assert!(!compose_fold(
        &mut buf,
        original.area,
        FoldRows {
            expanded: &bad,
            ..rows
        },
        80
    ));
    assert_eq!(buf, original);
}

#[test]
fn fold_accepts_already_clipped_viewports_but_never_reclips_a_retained_anchor() {
    let viewport = Rect::new(4, 7, 3, 3);
    let area = Rect::new(4, 8, 3, 2);
    let expanded = row_cells("AB", 3);
    let compact = row_cells("uv", 3);
    let rows = FoldRows {
        area,
        expanded: &expanded,
        compact: &compact,
        descendants: 2,
        direction: FoldDirection::Open,
    };
    let mut buf = clean(viewport);
    assert!(compose_fold(&mut buf, viewport, rows, 80));
    assert_eq!(row_labels(&buf, area), "Au");
    let before = buf.clone();
    assert!(!compose_fold(&mut buf, Rect::new(4, 7, 2, 3), rows, 80));
    assert_eq!(buf, before);
}

#[test]
fn safety_caps_and_saturated_rectangles_fail_without_allocating_snapshots() {
    let buf = clean(Rect::new(0, 0, 2, 2));
    for area in [
        Rect {
            x: 0,
            y: 0,
            width: 257,
            height: 256,
        },
        Rect {
            x: u16::MAX - 1,
            y: 0,
            width: 3,
            height: 1,
        },
        Rect::new(0, 0, 0, 1),
    ] {
        assert!(!bounded(area));
        assert!(border_foregrounds(&buf, border(area)).is_none());
        assert!(lap_origin(border(area)).is_none());
    }
    assert!(bounded(Rect {
        x: 0,
        y: 0,
        width: 256,
        height: 256
    }));
    assert_eq!(MAX_VIEWPORT_CELLS, 65_536);
}

#[test]
fn lap_head_pins_quarters_and_last_millisecond() {
    assert_eq!(lap_head(40, 0), 0);
    assert_eq!(lap_head(40, 75), 10);
    assert_eq!(lap_head(40, 150), 20);
    assert_eq!(lap_head(40, 225), 30);
    assert_eq!(lap_head(40, 299), 39);
    assert_eq!(lap_head(40, 300), 40);
    assert_eq!(lap_head(40, u32::MAX), 40);
    assert_eq!(lap_head(usize::MAX, 300), usize::MAX);
    assert_eq!(lap_head(0, 150), 0);
    for (count, expected) in [
        (0, 0),
        (1, 1),
        (6, 6),
        (7, 7),
        (40, 7),
        (56, 7),
        (57, 8),
        (80, 10),
    ] {
        assert_eq!(lap_width(count), expected);
    }
}

#[test]
fn status_band_uses_seven_stops_in_temporal_order_and_half_up_interpolation() {
    let band = test_band();
    for (i, expected) in [
        Color::Rgb(1, 3, 5),
        Color::Rgb(11, 13, 15),
        Color::Rgb(21, 23, 25),
        Color::Rgb(31, 33, 35),
        Color::Rgb(41, 43, 45),
        Color::Rgb(51, 53, 55),
        Color::Rgb(61, 63, 65),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(band.at(i, 7), expected);
    }
    assert_eq!(band.at(usize::MAX, 7), Color::Rgb(61, 63, 65));
    assert_eq!(band.at(usize::MAX, 1), Color::Rgb(1, 3, 5));
    assert_eq!(band.at(0, 0), Color::Rgb(1, 3, 5));
    let odd = FocusBand([
        (0, 0, 0),
        (255, 100, 1),
        (0, 0, 0),
        (0, 0, 0),
        (0, 0, 0),
        (0, 0, 0),
        (0, 0, 0),
    ]);
    assert_eq!(odd.at(1, 13), Color::Rgb(128, 50, 1));
    assert_eq!(odd.at(1, 19), Color::Rgb(85, 33, 0));
}

#[test]
fn focus_band_resolves_custom_slots_and_has_no_lower_tier_fallback() {
    let mut config = ThemeConfig {
        icons: "off".into(),
        ..Default::default()
    };
    for (name, value) in [
        ("status.error", "#010305"),
        ("data.number", "#0b0d0f"),
        ("status.warning", "#151719"),
        ("status.success", "#1f2123"),
        ("glow.cyan", "#292b2d"),
        ("status.info", "#333537"),
        ("glow.purple", "#3d3f41"),
    ] {
        config.slots.insert(name.into(), value.into());
    }
    let band = FocusBand::from_theme(&Theme::build(&config, Tier::TrueColor, &[])).unwrap();
    for i in 0..7 {
        assert_eq!(band.at(i, 7), test_band().at(i, 7));
    }
    for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        assert!(FocusBand::from_theme(&theme(tier)).is_none());
    }
}

#[test]
fn title_origin_uses_display_geometry_without_rotating_the_static_ramp() {
    let area = Rect::new(2, 3, 10, 4);
    assert_eq!(lap_origin(border(area)), Some(0));
    let title = title_region(area, crate::ui::width(" 界e\u{301} "), TitleEdge::Top);
    assert_eq!(title.width, 5);
    let titled = Border {
        top_title: Some(title),
        ..border(area)
    };
    assert_eq!(lap_origin(titled), Some(6));
    assert_eq!(
        lap_origin(Border {
            top_title: Some(Rect::new(3, 3, 0, 1)),
            ..border(area)
        }),
        Some(0)
    );
    // Protecting the entire top edge advances to the next right-edge cell.
    assert_eq!(
        lap_origin(Border {
            top_title: Some(Rect::new(2, 3, 10, 1)),
            ..border(area)
        }),
        Some(10)
    );
    let keep = [Rect::new(11, 3, 1, 3)];
    let full_usable_title = title_region(area, usize::MAX, TitleEdge::Top);
    assert_eq!(
        lap_origin(Border {
            top_title: Some(full_usable_title),
            keep: &keep,
            area
        }),
        Some(12)
    );
    assert_eq!(
        lap_origin(Border {
            keep: &[area],
            ..border(area)
        }),
        None
    );
}

#[test]
fn lap_walks_clockwise_at_all_quarters_and_leaves_exact_clean_ramp_behind() {
    let theme = theme(Tier::TrueColor);
    let area = Rect::new(2, 3, 15, 7); // 40 physical positions.
    let geometry = border(area);
    let mut base = clean(Rect::new(0, 0, 19, 12));
    focus_border(&theme, area, &mut base, true, &[]);
    let quiet: Vec<_> = (0..40).map(|i| color(100 + i)).collect();
    for (ms, head, pos) in [
        (0, 0, Position::new(2, 3)),
        (75, 10, Position::new(12, 3)),
        (150, 20, Position::new(16, 9)),
        (225, 30, Position::new(6, 9)),
        (299, 39, Position::new(2, 4)),
    ] {
        let mut got = base.clone();
        assert!(focus_lap(
            &mut got,
            geometry,
            geometry,
            &quiet,
            test_band(),
            ms
        ));
        assert_eq!(got[pos].fg, Color::Rgb(1, 3, 5));
        for (i, point) in perimeter(area).enumerate() {
            if i > head {
                assert_eq!(got[point].fg, quiet[i], "unvisited cell {i}");
            } else if head - i >= 7 {
                assert_eq!(
                    got[point], base[point],
                    "passed cell must be exact #31 at {i}"
                );
            }
        }
        assert_foreground_only(&base, &got, geometry);
    }
    for ms in [300, u32::MAX] {
        let mut got = base.clone();
        assert!(!focus_lap(
            &mut got,
            geometry,
            geometry,
            &quiet,
            test_band(),
            ms
        ));
        assert_eq!(got, base);
    }
}

#[test]
fn lap_trail_never_wraps_and_all_seven_stops_are_visible_after_six_steps() {
    let area = Rect::new(0, 0, 10, 2); // 20 physical positions.
    let geometry = border(area);
    let quiet = vec![color(100); 20];
    let base = clean(area);
    let mut start = base.clone();
    focus_lap(&mut start, geometry, geometry, &quiet, test_band(), 0);
    assert_eq!(start[(0, 0)].fg, Color::Rgb(1, 3, 5));
    assert_eq!(
        start[(0, 1)].fg,
        color(100),
        "tail must not wrap to the last cell"
    );
    let mut got = base.clone();
    focus_lap(&mut got, geometry, geometry, &quiet, test_band(), 90); // head 6.
    for (x, expected) in [
        Color::Rgb(61, 63, 65),
        Color::Rgb(51, 53, 55),
        Color::Rgb(41, 43, 45),
        Color::Rgb(31, 33, 35),
        Color::Rgb(21, 23, 25),
        Color::Rgb(11, 13, 15),
        Color::Rgb(1, 3, 5),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(got[(x as u16, 0)].fg, expected);
    }
}

#[test]
fn title_and_scrollbar_are_protected_but_still_consume_lap_distance() {
    let area = Rect::new(1, 1, 10, 4); // 24 physical positions.
    let keep = [Rect::new(10, 2, 1, 2), Rect::new(3, 4, 3, 1)];
    let geometry = Border {
        area,
        top_title: Some(Rect::new(2, 1, 4, 1)),
        keep: &keep,
    };
    let theme = theme(Tier::TrueColor);
    let mut base = clean(Rect::new(0, 0, 13, 7));
    focus_border(
        &theme,
        area,
        &mut base,
        true,
        &[geometry.top_title.unwrap()],
    );
    let quiet = vec![color(100); 24];
    assert_eq!(lap_origin(geometry), Some(5));
    let mut got = base.clone();
    focus_lap(&mut got, geometry, geometry, &quiet, test_band(), 75); // head 6, physical 11: protected scrollbar.
    assert_eq!(got[(10, 3)], base[(10, 3)]);
    assert_eq!(
        got[(10, 4)].fg,
        color(100),
        "next physical cell is still ahead"
    );
    assert_foreground_only(&base, &got, geometry);
    let mut late = base.clone();
    focus_lap(&mut late, geometry, geometry, &quiet, test_band(), 299);
    assert_eq!(
        late[(6, 1)],
        base[(6, 1)],
        "passed origin keeps original stationary ramp sample"
    );
}

#[test]
fn tiny_perimeters_and_completely_protected_frames_are_safe() {
    for (width, height) in [(1, 1), (1, 4), (4, 1)] {
        let area = Rect::new(2, 2, width, height);
        let geometry = border(area);
        let mut base = clean(Rect::new(1, 1, 6, 6));
        let quiet = border_foregrounds(&base, geometry).unwrap();
        let original = base.clone();
        assert!(focus_lap(
            &mut base,
            geometry,
            geometry,
            &quiet,
            test_band(),
            150
        ));
        assert_foreground_only(&original, &base, geometry);
        let protected = Border {
            keep: &[area],
            ..geometry
        };
        let before = base.clone();
        assert!(!focus_lap(
            &mut base,
            protected,
            protected,
            &quiet,
            test_band(),
            150
        ));
        assert!(!focus_drain(&mut base, protected, protected, &quiet, 150));
        assert_eq!(base, before);
    }
}

#[test]
fn border_changes_cancel_before_painting_and_clipped_geometry_is_explicit() {
    let buffer_area = Rect::new(3, 4, 8, 5);
    let base = clean(buffer_area);
    let geometry = border(Rect::new(4, 5, 6, 3));
    let prior = border_foregrounds(&base, geometry).unwrap();
    for changed in [
        Border {
            area: Rect::new(4, 5, 5, 3),
            ..geometry
        },
        Border {
            top_title: Some(Rect::new(5, 5, 2, 1)),
            ..geometry
        },
        Border {
            keep: &[Rect::new(9, 6, 1, 1)],
            ..geometry
        },
    ] {
        let mut got = base.clone();
        assert!(!focus_lap(
            &mut got,
            geometry,
            changed,
            &prior,
            test_band(),
            150
        ));
        assert!(!focus_drain(&mut got, geometry, changed, &prior, 150));
        assert_eq!(got, base);
    }
    let offscreen = border(Rect::new(1, 2, 8, 5));
    let mut got = base.clone();
    assert!(border_foregrounds(&got, offscreen).is_none());
    assert!(!focus_lap(
        &mut got,
        offscreen,
        offscreen,
        &prior,
        test_band(),
        150
    ));
    assert_eq!(got, base);
    let clipped = border(offscreen.area.intersection(buffer_area));
    let clipped_prior = border_foregrounds(&got, clipped).unwrap();
    assert!(focus_lap(
        &mut got,
        clipped,
        clipped,
        &clipped_prior,
        test_band(),
        150
    ));
    assert_foreground_only(&base, &got, clipped);
    let mut got = base.clone();
    assert!(!focus_lap(
        &mut got,
        geometry,
        geometry,
        &prior[..prior.len() - 1],
        test_band(),
        150
    ));
    assert!(!focus_drain(
        &mut got,
        geometry,
        geometry,
        &prior[..prior.len() - 1],
        150
    ));
    assert_eq!(got, base);
}

#[test]
fn outgoing_drain_uses_integer_half_up_and_exact_current_clean_destination() {
    let area = Rect::new(1, 1, 5, 3);
    let geometry = border(area);
    let mut base = clean(Rect::new(0, 0, 7, 5));
    for pos in perimeter(area) {
        base[pos].set_fg(Color::Rgb(255, 100, 1));
    }
    let prior = vec![Color::Rgb(0, 0, 0); 12];
    for (ms, expected) in [
        (0, Color::Rgb(0, 0, 0)),
        (75, Color::Rgb(64, 25, 0)),
        (100, Color::Rgb(85, 33, 0)),
        (150, Color::Rgb(128, 50, 1)),
        (225, Color::Rgb(191, 75, 1)),
        (299, Color::Rgb(254, 100, 1)),
        (300, Color::Rgb(255, 100, 1)),
    ] {
        let mut got = base.clone();
        assert_eq!(
            focus_drain(&mut got, geometry, geometry, &prior, ms),
            ms < 300
        );
        for pos in perimeter(area) {
            assert_eq!(got[pos].fg, expected, "at {ms} ms, {pos:?}");
        }
        assert_foreground_only(&base, &got, geometry);
        if ms == 300 {
            assert_eq!(got, base);
        }
    }
}

#[test]
fn drain_preserves_titles_scrollbars_and_nonuniform_resolved_targets() {
    let area = Rect::new(1, 1, 6, 4);
    let keep = [Rect::new(6, 2, 1, 2)];
    let geometry = Border {
        area,
        top_title: Some(Rect::new(2, 1, 2, 1)),
        keep: &keep,
    };
    let mut base = clean(Rect::new(0, 0, 9, 7));
    let mut prior = Vec::new();
    for (i, pos) in perimeter(area).enumerate() {
        base[pos].set_fg(Color::Rgb(20 + i as u8, 40, 60));
        prior.push(Color::Rgb(0, 0, 0));
        if geometry.protected(pos) {
            base[pos].set_fg(Color::Reset);
            *prior.last_mut().unwrap() = Color::Indexed(2);
        }
    }
    let mut got = base.clone();
    assert!(focus_drain(&mut got, geometry, geometry, &prior, 150));
    for (i, pos) in perimeter(area).enumerate() {
        if !geometry.protected(pos) {
            assert_eq!(got[pos].fg, Color::Rgb((20 + i as u8).div_ceil(2), 20, 30));
        }
    }
    assert_foreground_only(&base, &got, geometry);
}

#[test]
fn non_rgb_drain_source_or_destination_cancels_entire_overlay() {
    let area = Rect::new(0, 0, 4, 3);
    let geometry = border(area);
    let base = clean(area);
    let mut prior = vec![color(50); 10];
    prior[9] = Color::Reset;
    let mut got = base.clone();
    assert!(!focus_drain(&mut got, geometry, geometry, &prior, 150));
    assert_eq!(
        got, base,
        "even positions before the bad one must stay clean"
    );
    prior[9] = color(50);
    got[(0, 1)].set_fg(Color::Indexed(5));
    let before = got.clone();
    assert!(!focus_drain(&mut got, geometry, geometry, &prior, 150));
    assert_eq!(got, before);
}

#[test]
fn non_rgb_lap_prior_cancels_atomically_before_any_border_write() {
    let area = Rect::new(0, 0, 12, 6);
    let mut buf = clean(area);
    let border = Border {
        area,
        top_title: None,
        keep: &[],
    };
    let mut prior = border_foregrounds(&buf, border).unwrap();
    prior[5] = Color::Reset;
    let before = buf.clone();
    assert!(!focus_lap(
        &mut buf,
        border,
        border,
        &prior,
        FocusBand::from_theme(&theme(Tier::TrueColor)).unwrap(),
        75
    ));
    assert_eq!(buf, before);
}
