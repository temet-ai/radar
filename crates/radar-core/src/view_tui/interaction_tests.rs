//! Renderer-boundary contracts for interaction motion geometry.
use super::*;
use crate::design::theme::Tier;
use crate::motion::{
    self,
    frames::FrameMotion,
    transition::{MotionContext, MotionFold},
};
use crate::theme::ThemeConfig;
use crate::view::PaneView;

fn context() -> MotionContext {
    MotionContext::Screen {
        plugin: "fixture".into(),
        screen: "tree".into(),
        page: None,
        depth: 0,
        tab: None,
    }
}
fn draw(open: bool) -> (Buffer, motion::Marks, Theme) {
    let theme = Theme::build(
        &ThemeConfig {
            icons: "off".into(),
            ..Default::default()
        },
        Tier::TrueColor,
        &[],
    );
    let mut root = TreeRow::new("root", "PARENT");
    root.children = vec![TreeRow::new("one", "CHILD1"), TreeRow::new("two", "CHILD2")];
    let tree = Tree {
        rows: vec![root, TreeRow::new("tail", "AFTER")],
        selected: vec!["root".into()],
        open: if open {
            vec![vec!["root".into()]]
        } else {
            vec![]
        },
        ..Default::default()
    };
    let view = ScreenView::new(
        "fixture",
        vec![PaneView::new(PaneId::Middle, "tree", Node::Tree(tree))],
    );
    let cursor = StdCell::new(None);
    let hits = RefCell::new(vec![]);
    let plugins = vec![];
    let cx = Cx {
        theme: &theme,
        now: 0,
        focused: false,
        focus: Pane::Middle,
        bar: false,
        tabs: false,
        cursor: &cursor,
        hits: &hits,
        parts: crate::compose::Parts::new(&plugins, None),
    };
    let mut buf = Buffer::empty(Rect::new(0, 0, 32, 10));
    motion::begin();
    motion::frames::context(Some(context()), None);
    render_screen_with_ramp(&Frontend::default(), &view, buf.area, &mut buf, &cx, true);
    (buf, motion::take(), theme)
}
fn line(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}
#[test]
fn small_tree_retains_full_blank_viewport_and_keeps_scrollbar_outside_fold_cells() {
    let (closed, before, theme) = draw(false);
    let (opened, after, _) = draw(true);
    assert_eq!(before.trees.len(), 1);
    assert_eq!(after.trees.len(), 1);
    let area = before.trees[0].area;
    assert_eq!(area, after.trees[0].area);
    assert!(usize::from(area.height) > after.trees[0].rows.len());
    assert_eq!(
        area.right(),
        30,
        "tree reserves the final inner column for its scrollbar"
    );
    assert_eq!(before.trees[0].rows.len(), 2);
    assert_eq!(after.trees[0].rows.len(), 4);
    for opening in [true, false] {
        let (old, old_marks, final_buf, final_marks) = if opening {
            (&closed, &before, &opened, &after)
        } else {
            (&opened, &after, &closed, &before)
        };
        let mut engine = FrameMotion::default();
        engine.draw(&mut old.clone(), &theme, old_marks, 0);
        engine.input();
        let mut marks = final_marks.clone();
        marks.fold = Some(MotionFold {
            context: context(),
            pane: PaneId::Middle,
            slot: "body".into(),
            path: vec!["root".into()],
            before_open: !opening,
            after_open: opening,
            origin_seq: None,
            origin_cursor: None,
        });
        for time in [0, 80, 160] {
            let mut animated = final_buf.clone();
            engine.draw(&mut animated, &theme, &marks, time);
            if let Some(directory) = std::env::var_os("RADAR_INTERACTION_SHOTS") {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                let name = format!("fold-{}-{time}.svg", if opening { "open" } else { "close" });
                std::fs::write(directory.join(name), crate::visual::buffer_to_svg(&animated, &crate::visual::Look::of(&theme))).unwrap();
            }
            for y in 0..10 {
                assert_eq!(
                    animated[(30, y)],
                    final_buf[(30, y)],
                    "scrollbar must not move with rows"
                );
            }
            let visible: String = (area.y + 1..area.bottom())
                .map(|y| line(&animated, y))
                .collect();
            assert_eq!(
                visible.matches("CHILD").count(),
                if time == 80 {
                    1
                } else if (time == 0) == opening {
                    0
                } else {
                    2
                }
            );
            if time == 160 {
                assert_eq!(animated, *final_buf);
            }
        }
        assert!(
            engine.running(),
            "one final clean settle frame is scheduled"
        );
        let mut settled = final_buf.clone();
        engine.draw(&mut settled, &theme, &marks, 190);
        assert_eq!(settled, *final_buf);
        assert!(!engine.running());
    }
}
