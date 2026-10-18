use super::*;
use crate::{design::theme::Tier, theme::ThemeConfig};
fn theme() -> Theme {
    Theme::build(
        &ThemeConfig {
            icons: "off".into(),
            ..Default::default()
        },
        Tier::TrueColor,
        &[],
    )
}
fn context() -> MotionContext {
    MotionContext::Screen {
        plugin: "watch".into(),
        screen: "cases".into(),
        page: None,
        depth: 0,
        tab: None,
    }
}
fn frames(owner: PaneId) -> (Buffer, super::super::Marks) {
    let mut buf = Buffer::empty(Rect::new(0, 0, 24, 8));
    let marks: Vec<_> = [
        (PaneId::Nav, Rect::new(0, 0, 12, 8)),
        (PaneId::Middle, Rect::new(12, 0, 12, 8)),
    ]
    .into_iter()
    .map(|(pane, area)| {
        for p in crate::ui::frame::perimeter(area) {
            buf[(p.x, p.y)].set_symbol("─").set_fg(if pane == owner {
                Color::Rgb(100, 50, 200)
            } else {
                Color::Rgb(20, 30, 40)
            });
        }
        FrameMark {
            key: FrameKey {
                pane,
                slot: "body".into(),
            },
            area,
            owns_keys: pane == owner,
            top_title: Some(Rect::new(area.x + 1, 0, 4, 1)),
            keep: vec![
                Rect::new(area.x + 1, 0, 4, 1),
                Rect::new(area.right() - 1, 1, 1, 6),
            ],
        }
    })
    .collect();
    (
        buf,
        super::super::Marks {
            context: Some(context()),
            frames: marks,
            ..Default::default()
        },
    )
}
fn tree(open: bool) -> (Buffer, super::super::Marks) {
    let area = Rect::new(1, 1, 6, 5);
    let mut buf = Buffer::empty(Rect::new(0, 0, 8, 7));
    let paths: Vec<Vec<String>> = if open {
        vec![
            vec!["root"],
            vec!["root", "one"],
            vec!["root", "two"],
            vec!["after"],
            vec!["last"],
        ]
    } else {
        vec![vec!["root"], vec!["after"], vec!["last"]]
    }
    .into_iter()
    .map(|path| path.into_iter().map(str::to_string).collect())
    .collect();
    let words = if open {
        vec!["PARENT", "CHILD1", "CHILD2", "AFTER1", "AFTER2"]
    } else {
        vec!["PARENT", "AFTER1", "AFTER2"]
    };
    for (y, word) in words.iter().enumerate() {
        buf.set_string(1, 1 + y as u16, word, ratatui::style::Style::new());
    }
    let mark = TreeMark {
        key: FrameKey {
            pane: PaneId::Middle,
            slot: "body".into(),
        },
        area,
        rows: paths
            .into_iter()
            .enumerate()
            .map(|(i, path)| (path, 1 + i as u16))
            .collect(),
        open: if open {
            vec![vec!["root".into()]]
        } else {
            vec![]
        },
    };
    (
        buf,
        super::super::Marks {
            context: Some(context()),
            trees: vec![mark],
            ..Default::default()
        },
    )
}
fn fold(open: bool) -> MotionFold {
    MotionFold {
        context: context(),
        pane: PaneId::Middle,
        slot: "body".into(),
        path: vec!["root".into()],
        before_open: !open,
        after_open: open,
        origin_seq: None,
        origin_cursor: None,
    }
}
fn row(buf: &Buffer, y: u16) -> String {
    (1..7).map(|x| buf[(x, y)].symbol()).collect()
}

#[test]
fn focus_transfer_laps_and_settles_to_exact_clean_frame() {
    let t = theme();
    let mut motion = FrameMotion::default();
    let (mut old, marks) = frames(PaneId::Nav);
    motion.draw(&mut old, &t, &marks, 0);
    assert!(!motion.running());
    motion.input();
    let (clean, marks) = frames(PaneId::Middle);
    let mut drawn = clean.clone();
    motion.draw(&mut drawn, &t, &marks, 10);
    assert!(motion.running());
    assert_ne!(drawn, clean);
    for frame in &marks.frames {
        for rect in &frame.keep {
            for y in rect.y..rect.bottom() {
                for x in rect.x..rect.right() {
                    assert_eq!(drawn[(x, y)], clean[(x, y)]);
                }
            }
        }
    }
    let mut drawn = clean.clone();
    motion.draw(&mut drawn, &t, &marks, 310);
    assert_eq!(drawn, clean);
    assert!(motion.running());
    motion.draw(&mut drawn, &t, &marks, 340);
    assert!(!motion.running());
}
#[test]
fn ownership_without_input_is_instant_and_first_frame_never_laps() {
    let t = theme();
    let mut motion = FrameMotion::default();
    let (mut a, am) = frames(PaneId::Nav);
    motion.draw(&mut a, &t, &am, 0);
    let (mut b, bm) = frames(PaneId::Middle);
    let clean = b.clone();
    motion.draw(&mut b, &t, &bm, 5);
    assert_eq!(b, clean);
    assert!(!motion.running());
}
#[test]
fn modal_or_context_replacement_discards_active_motion() {
    let t = theme();
    for modal in [false, true] {
        let mut motion = FrameMotion::default();
        let (mut a, am) = frames(PaneId::Nav);
        motion.draw(&mut a, &t, &am, 0);
        motion.input();
        let (clean, mut bm) = frames(PaneId::Middle);
        let mut b = clean.clone();
        motion.draw(&mut b, &t, &bm, 1);
        assert!(motion.running());
        bm.context = if modal {
            None
        } else {
            Some(MotionContext::History)
        };
        let mut b = clean.clone();
        motion.draw(&mut b, &t, &bm, 2);
        assert_eq!(b, clean);
        assert!(!motion.running());
    }
}
#[test]
fn opening_unrolls_rows_and_closing_retains_disappeared_children() {
    let t = theme();
    for open in [true, false] {
        let mut motion = FrameMotion::default();
        let (mut old, old_marks) = tree(!open);
        motion.draw(&mut old, &t, &old_marks, 0);
        motion.input();
        let (clean, mut marks) = tree(open);
        marks.fold = Some(fold(open));
        let mut drawn = clean.clone();
        motion.draw(&mut drawn, &t, &marks, 10);
        assert_eq!(row(&drawn, 2), if open { "AFTER1" } else { "CHILD1" });
        let mut drawn = clean.clone();
        motion.draw(&mut drawn, &t, &marks, 90);
        assert_eq!(row(&drawn, 2), "CHILD1");
        assert_eq!(row(&drawn, 3), "AFTER1");
        let mut drawn = clean.clone();
        motion.draw(&mut drawn, &t, &marks, 170);
        assert_eq!(drawn, clean);
        motion.draw(&mut drawn, &t, &marks, 200);
        assert!(!motion.running());
    }
}
#[test]
fn refresh_without_verified_fold_does_not_animate_row_changes() {
    let t = theme();
    let mut motion = FrameMotion::default();
    let (mut old, marks) = tree(false);
    motion.draw(&mut old, &t, &marks, 0);
    motion.input();
    let (mut now, marks) = tree(true);
    let clean = now.clone();
    motion.draw(&mut now, &t, &marks, 1);
    assert_eq!(now, clean);
    assert!(!motion.running());
}
#[test]
fn input_cancels_but_keeps_clean_baseline_for_immediate_reversal() {
    let t = theme();
    let mut motion = FrameMotion::default();
    let (mut old, marks) = tree(false);
    motion.draw(&mut old, &t, &marks, 0);
    motion.input();
    let (mut now, mut marks) = tree(true);
    marks.fold = Some(fold(true));
    motion.draw(&mut now, &t, &marks, 1);
    assert!(motion.running());
    motion.input();
    let (mut close, mut marks) = tree(false);
    marks.fold = Some(fold(false));
    motion.draw(&mut close, &t, &marks, 2);
    assert_eq!(row(&close, 2), "CHILD1");
    assert!(motion.running());
    motion.input();
    let (mut clean, marks) = tree(false);
    let expected = clean.clone();
    motion.draw(&mut clean, &t, &marks, 3);
    assert_eq!(clean, expected);
    assert!(!motion.running());
}
#[test]
fn changed_tree_content_and_moved_parent_cancel_retained_rows() {
    let t = theme();
    for moved in [false, true] {
        let mut motion = FrameMotion::default();
        let (mut old, marks) = tree(false);
        motion.draw(&mut old, &t, &marks, 0);
        motion.input();
        let (mut now, mut marks) = tree(true);
        marks.fold = Some(fold(true));
        motion.draw(&mut now, &t, &marks, 1);
        let (mut changed, mut next) = tree(true);
        if moved {
            next.trees[0].rows[0].1 += 1;
        } else {
            changed[(1, 2)].set_symbol("N");
        }
        let expected = changed.clone();
        motion.draw(&mut changed, &t, &next, 2);
        assert_eq!(changed, expected);
        assert!(motion.fold.is_none());
    }
}
#[test]
fn absent_owner_and_resize_never_create_a_transfer() {
    let t = theme();
    let mut motion = FrameMotion::default();
    let (mut old, marks) = frames(PaneId::Nav);
    motion.draw(&mut old, &t, &marks, 0);
    motion.input();
    let (mut now, mut marks) = frames(PaneId::Middle);
    marks.frames.iter_mut().for_each(|m| m.owns_keys = false);
    let clean = now.clone();
    motion.draw(&mut now, &t, &marks, 1);
    assert_eq!(now, clean);
    assert!(!motion.running());
    motion.input();
    let mut resized = Buffer::empty(Rect::new(0, 0, 25, 8));
    motion.draw(&mut resized, &t, &marks, 2);
    assert!(!motion.running());
}
#[test]
fn restored_geometry_does_not_resume_an_invalidated_lap() {
    let t = theme();
    let mut motion = FrameMotion::default();
    let (mut a, am) = frames(PaneId::Nav);
    motion.draw(&mut a, &t, &am, 0);
    motion.input();
    let (clean, marks) = frames(PaneId::Middle);
    let mut b = clean.clone();
    motion.draw(&mut b, &t, &marks, 1);
    let mut moved = marks.clone();
    moved.frames[1].keep.push(Rect::new(13, 7, 1, 1));
    let mut b = clean.clone();
    motion.draw(&mut b, &t, &moved, 2);
    assert!(motion.focus.is_none());
    let mut b = clean.clone();
    motion.draw(&mut b, &t, &marks, 3);
    assert_eq!(b, clean);
    assert!(!motion.running());
}
#[test]
fn fold_rejects_unrelated_added_siblings_and_discontiguous_descendants() {
    let t = theme();
    for unrelated in [false, true] {
        let mut motion = FrameMotion::default();
        let (mut old, mut old_marks) = tree(false);
        if unrelated {
            old_marks.trees[0].rows.truncate(1);
        }
        motion.draw(&mut old, &t, &old_marks, 0);
        motion.input();
        let (mut clean, mut marks) = tree(true);
        marks.fold = Some(fold(true));
        if !unrelated {
            marks.trees[0].rows.swap(2, 3);
        }
        let expected = clean.clone();
        motion.draw(&mut clean, &t, &marks, 1);
        assert_eq!(clean, expected);
        assert!(!motion.running());
    }
}

#[test]
fn background_delivery_cannot_borrow_a_locally_consumed_input() {
    let mut engine = FrameMotion::default();
    let (mut before, marks) = frames(PaneId::Nav);
    engine.draw(&mut before, &theme(), &marks, 0);
    engine.input();
    engine.background();
    let (mut after, marks) = frames(PaneId::Middle);
    let clean = after.clone();
    engine.draw(&mut after, &theme(), &marks, 10);
    assert_eq!(after, clean);
    assert!(!engine.running());
}

#[test]
fn registration_rejects_duplicate_identity_and_unbounded_metadata() {
    super::super::begin();
    let mut huge = context();
    if let MotionContext::Screen { plugin, .. } = &mut huge {
        *plugin = "x".repeat(16_385);
    }
    super::context(Some(huge), None);
    assert!(!enabled());
    super::context(Some(context()), None);
    with_frame(PaneId::Middle, "body", || {
        frame(Rect::new(0, 0, 10, 10), true, &[]);
        frame(Rect::new(0, 0, 10, 10), true, &[]);
    });
    assert!(!enabled());
    super::super::begin();
    super::context(Some(context()), None);
    super::tree(
        PaneId::Middle,
        "body",
        Rect::new(0, 0, 10, 2),
        vec![(vec!["root".into()], 0)],
        vec![vec!["a".into()], vec!["b".into()]],
    );
    assert!(!enabled(), "open metadata must be bounded by visible rows");
    super::super::begin();
}

#[test]
fn retained_fold_generations_share_cells_across_clean_redraws() {
    let (mut closed, closed_marks) = tree(false);
    let (mut opened, mut marks) = tree(true);
    let mut engine = FrameMotion::default();
    engine.draw(&mut closed, &theme(), &closed_marks, 0);
    let before = Rc::clone(&engine.previous.as_ref().unwrap().trees[0].cells);
    engine.input();
    marks.fold = Some(fold(true));
    let clean = opened.clone();
    engine.draw(&mut opened, &theme(), &marks, 0);
    let active = engine.fold.as_ref().unwrap();
    assert!(Rc::ptr_eq(&active.compact, &before));
    assert!(Rc::ptr_eq(&active.expanded, &active.final_tree.cells));
    let after = Rc::clone(&engine.previous.as_ref().unwrap().trees[0].cells);
    assert!(Rc::ptr_eq(&active.expanded, &after));
    engine.draw(&mut clean.clone(), &theme(), &marks, 80);
    assert!(Rc::ptr_eq(
        &engine.previous.as_ref().unwrap().trees[0].cells,
        &after
    ));
    assert!(Rc::ptr_eq(&engine.fold.as_ref().unwrap().compact, &before));
}

#[test]
fn ambiguous_owners_and_oversized_viewports_use_clean_frames() {
    let mut engine = FrameMotion::default();
    let (mut old, mut marks) = frames(PaneId::Nav);
    marks.frames[1].owns_keys = true;
    engine.draw(&mut old, &theme(), &marks, 0);
    engine.input();
    let (mut next, marks) = frames(PaneId::Middle);
    let clean = next.clone();
    engine.draw(&mut next, &theme(), &marks, 1);
    assert_eq!(next, clean);
    assert!(!engine.running());
    let mut oversized = Buffer::empty(Rect::new(0, 0, 257, 256));
    engine.draw(&mut oversized, &theme(), &marks, 2);
    assert!(engine.previous.is_none());
    assert!(!engine.running());
}

#[test]
fn aggregate_border_and_late_protection_metadata_are_bounded() {
    super::super::begin();
    super::context(Some(context()), None);
    with_frame(PaneId::Middle, "body", || {
        frame(Rect::new(0, 0, 10, 10), true, &[])
    });
    for _ in 0..64 {
        protect(Rect::new(9, 1, 1, 1));
    }
    assert!(enabled());
    protect(Rect::new(9, 1, 1, 1));
    assert!(!enabled());
    super::super::begin();
    super::context(Some(context()), None);
    for i in 0..3 {
        with_frame(PaneId::Middle, &i.to_string(), || {
            frame(Rect::new(0, 0, 30_000, 1), i == 0, &[])
        });
    }
    assert!(!enabled(), "sum of retained border colors has a global cap");
    super::super::begin();
}
