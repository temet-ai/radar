use super::*;
use crate::design::theme::Tier;
use crate::host::{Event, MouseKind};
use crate::motion::{self, Engine, Level, ManualClock};
use crate::theme::ThemeConfig;
use ratatui::{
    Terminal,
    backend::TestBackend,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

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
fn text(buf: &Buffer) -> String {
    (buf.area.y..buf.area.bottom())
        .map(|y| crate::shell::row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn notices_keep_panel_border_and_status_visible() {
    assert_eq!(
        content(Rect::new(0, 0, 80, 24), false),
        Rect::new(1, 2, 78, 20)
    );
    assert!(content(Rect::new(0, 0, 1, 1), false).is_empty());
    assert!(content(Rect::default(), false).is_empty());
    assert!(content(Rect::new(0, 0, 3, 5), true).is_empty());
    assert_eq!(content(Rect::new(0, 0, 3, 6), true), Rect::new(1, 3, 1, 1));
}

#[test]
fn notices_stand_inside_the_bottom_right_panel() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 40, 10));
    let fallback = content(buf.area, false);
    assert_eq!(anchor(&buf, fallback), fallback, "no panel drawn: the content");
    // Two panels side by side above the status line, the right one 1..=8 tall.
    for area in [Rect::new(0, 1, 15, 8), Rect::new(15, 1, 25, 8)] {
        use ratatui::widgets::Widget;
        ratatui::widgets::Block::bordered()
            .border_type(ratatui::widgets::BorderType::Rounded)
            .render(area, &mut buf);
    }
    assert_eq!(anchor(&buf, fallback), Rect::new(16, 2, 23, 6));
    buf[(15, 4)].set_symbol("x");
    assert_eq!(anchor(&buf, fallback), fallback, "a broken edge is not a panel");
}

#[test]
fn late_composite_keeps_retained_frames_clean_and_clicks_consume_the_occurrence() {
    let mut host = Host::new(vec![], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host.set_motion_enabled(true);
    // Without notice motion the row is whole at once (it reads in from the right otherwise).
    host.set_notice_motion_enabled(false);
    host.update(Event::Notice("The Watch notice is transient".into()));
    let theme = theme();
    let clock = ManualClock::default();
    let mut engine = Engine::new(Level::Full, Box::new(clock.clone()));
    let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
    let mut clean = String::new();
    term.draw(|f| {
        crate::shell::render(&host, &theme, f);
        clean = text(f.buffer_mut());
        host.hits.borrow_mut().push((
            Rect::new(15, 21, 64, 1),
            Hit::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE), 0),
        ));
        engine.draw(f.buffer_mut(), &theme);
        crate::shell::render_notices(&host, &theme, f.buffer_mut());
    })
    .unwrap();
    assert!(!clean.contains("The Watch notice is transient"));
    assert!(text(term.backend().buffer()).contains("The Watch notice is transient"));
    assert!(matches!(host.hit_at(78, 21), Some(Hit::Notice(_))));
    host.update(Event::Mouse {
        kind: MouseKind::Press,
        column: 78,
        row: 21,
        at_ms: 1,
    });
    assert!(!host.quit, "the covered q target must never run");
    assert!(host.notices.active().next().is_none());
    engine.cue(motion::Cue::Screen, &theme);
    clock.advance(50);
    term.draw(|f| {
        crate::shell::render(&host, &theme, f);
        engine.draw(f.buffer_mut(), &theme);
        crate::shell::render_notices(&host, &theme, f.buffer_mut());
    })
    .unwrap();
    assert!(
        !text(term.backend().buffer()).contains("The Watch notice is transient"),
        "retained motion cannot resurrect a dismissed occurrence"
    );
}

#[test]
fn explicit_history_preserves_status_and_blocks_transient_notices() {
    let mut host = Host::new(vec![], 0).unwrap();
    host.load_config(Some("")).unwrap();
    host.set_motion_enabled(true);
    host.update(Event::Notice("Watch detail stays in this session".into()));
    host.update(Event::Key(KeyEvent::from(KeyCode::Char('~'))));
    let text = crate::shell::to_text(&host, 80, 24);
    assert!(text.contains("Recent notices"));
    assert!(text.contains("This session only"));
    assert!(text.contains("Watch detail stays in this session"));
    assert!(text.lines().last().unwrap().contains("esc"));
    assert!(
        host.hits
            .borrow()
            .iter()
            .any(|(_, hit)| matches!(hit, Hit::NoticeHistory(_)))
    );
    assert!(
        host.hits
            .borrow()
            .iter()
            .all(|(_, hit)| !matches!(hit, Hit::Notice(_)))
    );
    assert!(motion::take().context.is_none());
}

#[test]
fn fold_reversal_never_retains_or_overpaints_a_late_notice() {
    use crate::motion::transition::{MotionContext, MotionFold};
    use crate::view::{
        PaneId,
        chrome::{NoticeTone, NoticeView},
    };
    let theme = theme();
    let clock = ManualClock::default();
    let mut engine = Engine::new(Level::Full, Box::new(clock.clone()));
    let context = MotionContext::Screen {
        plugin: "watch".into(),
        screen: "cases".into(),
        page: None,
        depth: 0,
        tab: None,
    };
    let frame = |open: bool, fold: bool| {
        let mut buf = Buffer::empty(Rect::new(0, 0, 24, 10));
        buf.set_style(
            buf.area,
            theme
                .s(crate::design::theme::Slot::TextPrimary)
                .patch(theme.bg(crate::design::theme::Slot::BgBase)),
        );
        let rows = if open {
            vec![
                (vec!["group"], "WATCH GROUP"),
                (vec!["group", "one"], "WATCH ONE"),
                (vec!["group", "two"], "WATCH TWO"),
                (vec!["next"], "NEXT WATCH"),
                (vec!["last"], "LAST WATCH"),
            ]
        } else {
            vec![
                (vec!["group"], "WATCH GROUP"),
                (vec!["next"], "NEXT WATCH"),
                (vec!["last"], "LAST WATCH"),
            ]
        };
        for (i, (_, word)) in rows.iter().enumerate() {
            buf.set_string(
                1,
                1 + i as u16,
                word,
                theme.s(crate::design::theme::Slot::TextPrimary),
            );
        }
        motion::begin();
        motion::frames::context(
            Some(context.clone()),
            fold.then(|| MotionFold {
                context: context.clone(),
                pane: PaneId::Middle,
                slot: "body".into(),
                path: vec!["group".into()],
                before_open: !open,
                after_open: open,
                origin_seq: None,
                origin_cursor: None,
            }),
        );
        motion::frames::tree(
            PaneId::Middle,
            "body",
            Rect::new(1, 1, 22, 8),
            rows.into_iter()
                .enumerate()
                .map(|(i, (path, _))| {
                    (path.into_iter().map(str::to_string).collect(), 1 + i as u16)
                })
                .collect(),
            if open {
                vec![vec!["group".into()]]
            } else {
                vec![]
            },
        );
        buf
    };
    let stack = ui::notices::layout(
        &theme,
        &[NoticeView {
            id: 1,
            tone: NoticeTone::Info,
            icon: None,
            text: "NOTICE-LAYER".into(),
            created_ms: 0,
            lifetime_ms: 5000,
        }],
        0,
        Rect::new(1, 1, 22, 3),
        1000,
        true,
    );
    let mut closed = frame(false, false);
    engine.draw(&mut closed, &theme);
    ui::notices::paint(&stack, &theme, &mut closed);
    assert!(text(&closed).contains("NOTICE-LAYER"));
    engine.input();
    let mut opened = frame(true, true);
    engine.draw(&mut opened, &theme);
    assert!(engine.running());
    assert!(!text(&opened).contains("NOTICE-LAYER"));
    ui::notices::paint(&stack, &theme, &mut opened);
    assert!(text(&opened).contains("NOTICE-LAYER"));
    clock.advance(80);
    let mut midway = frame(true, false);
    engine.draw(&mut midway, &theme);
    assert!(!text(&midway).contains("NOTICE-LAYER"));
    ui::notices::paint(&stack, &theme, &mut midway);
    assert!(text(&midway).contains("NOTICE-LAYER"));
    engine.input();
    let mut reversed = frame(false, true);
    engine.draw(&mut reversed, &theme);
    assert!(engine.running());
    assert!(!text(&reversed).contains("NOTICE-LAYER"));
    clock.advance(160);
    let mut settled = frame(false, false);
    let expected = settled.clone();
    engine.draw(&mut settled, &theme);
    assert_eq!(
        settled, expected,
        "fold reversal settles to the clean tree, without toast ghosts"
    );
}
