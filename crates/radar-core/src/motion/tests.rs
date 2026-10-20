use super::*;
use crate::theme::ThemeConfig;

const AREA: Rect = Rect {
    x: 0,
    y: 0,
    width: 30,
    height: 8,
};
const POPUP: Rect = Rect {
    x: 5,
    y: 2,
    width: 12,
    height: 4,
};

fn theme() -> Theme {
    Theme::build(&ThemeConfig::default(), Tier::TrueColor, &[])
}

/// A frame as radar would draw it: text on the surface.
fn plain(theme: &Theme) -> Buffer {
    let mut b = Buffer::empty(AREA);
    b.set_style(AREA, theme.bg(Slot::BgSurface));
    for y in 0..AREA.height {
        b.set_string(0, y, format!("row {y} with some text"), theme.s(Slot::TextPrimary));
    }
    b
}

/// Two rounded panels side by side, rows of text inside: what the assemble moment builds.
fn panels_frame(theme: &Theme) -> Buffer {
    use ratatui::widgets::{Block, BorderType, Widget};
    let mut b = Buffer::empty(AREA);
    b.set_style(AREA, theme.bg(Slot::BgSurface));
    for area in [Rect::new(0, 0, 12, 8), Rect::new(12, 0, 18, 8)] {
        Block::bordered().border_type(BorderType::Rounded).render(area, &mut b);
        for y in 1..7 {
            b.set_string(area.x + 1, y, format!("row {y} text"), theme.s(Slot::TextPrimary));
        }
    }
    b
}

#[test]
fn assemble_builds_panels_left_to_right_and_rows_after_their_frame() {
    let t = theme();
    let frame = panels_frame(&t);
    let cells = frame.content.clone();
    assert_eq!(
        assemble::panels(&cells, AREA),
        [Rect::new(0, 0, 12, 8), Rect::new(12, 0, 18, 8)]
    );
    let at = |ms: u32| {
        let mut c = cells.clone();
        assemble::apply(&mut c, AREA, ms);
        let w = AREA.width as usize;
        move |x: usize, y: usize| c[y * w + x].symbol().to_string()
    };
    let start = at(0);
    assert!((0..30).all(|x| start(x, 0) == " "), "nothing before the first frame row");
    let first = at(40);
    assert_eq!(first(0, 0), "╭", "the left panel's top edge at 40 ms");
    assert_eq!(first(12, 0), " ", "the right panel starts 30 ms later");
    assert_eq!(first(0, 1), " ", "its next frame row 8 ms on");
    let rows = at(88 + 30);
    // Row 1 of the left panel started at 40 + 8 + 40 = 88 ms; half its 10 cells after 30 ms.
    assert_eq!((1..=5).map(|x| rows(x, 1)).collect::<String>(), "row 1");
    assert_eq!(rows(6, 1), " ");
    let end = at(assemble::ASSEMBLE_MS);
    assert!((0..8).all(|y| (0..30).all(|x| end(x, y) == frame.content[y * 30 + x].symbol())));
}

#[test]
fn assemble_runs_at_full_and_subtle_keeps_the_fade() {
    let t = theme();
    let (mut full, clock) = engine(Level::Full);
    full.cue(Cue::Screen, &t);
    clock.advance(40);
    begin();
    region(Region::Body, AREA);
    let mut b = panels_frame(&t);
    full.draw(&mut b, &t);
    assert_eq!(b[(12, 0)].symbol(), " ", "full: the right panel is still to come");
    let (mut subtle, clock) = engine(Level::Subtle);
    subtle.cue(Cue::Screen, &t);
    clock.advance(40);
    begin();
    region(Region::Body, AREA);
    let mut b = panels_frame(&t);
    subtle.draw(&mut b, &t);
    assert_eq!(b[(12, 0)].symbol(), "╭", "subtle: every glyph is there, fading in");
    full.input();
    begin();
    region(Region::Body, AREA);
    let mut b = panels_frame(&t);
    full.draw(&mut b, &t);
    assert_eq!(b, panels_frame(&t), "a key finishes the build at once");
}

fn row_rect(y: u16) -> Rect {
    Rect {
        y,
        height: 1,
        ..AREA
    }
}

/// Draw a frame with these marks and run the engine over it.
fn draw(engine: &mut Engine, theme: &Theme, marks: impl Fn()) -> Buffer {
    begin();
    marks();
    let mut b = plain(theme);
    engine.draw(&mut b, theme);
    b
}

fn popup_marks() {
    region(Region::Body, AREA);
    region(Region::Popup, POPUP);
    for y in 0..AREA.height {
        row(format!("row:{y}"), row_rect(y));
    }
}

fn engine(level: Level) -> (Engine, ManualClock) {
    let clock = ManualClock::default();
    (Engine::new(level, Box::new(clock.clone())), clock)
}

fn differs_in(a: &Buffer, b: &Buffer, area: Rect) -> bool {
    area.positions().any(|p| a[p] != b[p])
}

fn every_cue() -> Vec<Cue> {
    let mut changes = Changes::default();
    changes.changed.insert("row:1".into());
    changes.added.insert("row:2".into());
    vec![
        Cue::Screen,
        Cue::Page,
        Cue::Popup,
        Cue::Message,
        Cue::MessageGone,
        Cue::Changed(changes),
    ]
}

#[test]
fn off_draws_exactly_what_was_drawn_whatever_happens() {
    let t = theme();
    let (mut e, clock) = engine(Level::Off);
    for cue in every_cue() {
        e.cue(cue, &t);
    }
    assert!(!e.running());
    assert_eq!(e.next_frame(), None, "no frame tick when nothing runs");
    for _ in 0..3 {
        let b = draw(&mut e, &t, || {
            popup_marks();
            loading(AREA);
        });
        assert_eq!(b, plain(&t));
        clock.advance(50);
    }
    assert!(!e.running());
}

#[test]
fn a_popup_sweeps_in_over_its_own_area_and_ends_exactly_as_drawn() {
    let t = theme();
    let (mut e, clock) = engine(Level::Subtle);
    e.cue(Cue::Popup, &t);
    assert_eq!(e.next_frame(), Some(FRAME));
    let first = draw(&mut e, &t, popup_marks);
    assert!(differs_in(&first, &plain(&t), POPUP), "the popup starts hidden");
    for p in AREA.positions().filter(|p| !POPUP.contains(*p)) {
        assert_eq!(first[p], plain(&t)[p], "only the popup moves, not {p:?}");
    }
    clock.advance(u64::from(POPUP_MS) / 2);
    let half = draw(&mut e, &t, popup_marks);
    assert!(e.running(), "still running half way");
    assert_ne!(half, first, "it moved on with the clock");
    clock.advance(u64::from(POPUP_MS));
    let _ = draw(&mut e, &t, popup_marks);
    assert!(e.running(), "one plain frame follows the last");
    let last = draw(&mut e, &t, popup_marks);
    assert_eq!(last, plain(&t), "it ends exactly as drawn");
    assert!(!e.running());
    assert_eq!(e.next_frame(), None, "and the frame tick stops");
}

#[test]
fn each_effect_runs_for_its_time_and_no_longer() {
    let t = theme();
    for (cue, ms) in [
        (Cue::Screen, assemble::ASSEMBLE_MS),
        (Cue::Page, PAGE_MS),
        (Cue::Popup, POPUP_MS),
        (Cue::Message, MESSAGE_MS),
    ] {
        let (mut e, clock) = engine(Level::Full);
        e.cue(cue.clone(), &t);
        let marks = || {
            popup_marks();
            region(Region::Message, row_rect(7));
        };
        let _ = draw(&mut e, &t, marks);
        clock.advance(u64::from(ms) - 20);
        let _ = draw(&mut e, &t, marks);
        assert!(e.running(), "{cue:?} still runs before {ms} ms");
        clock.advance(20);
        let _ = draw(&mut e, &t, marks);
        let _ = draw(&mut e, &t, marks);
        assert!(!e.running(), "{cue:?} is over at {ms} ms");
        assert!((100..=450).contains(&ms), "{cue:?} is short");
    }
}

#[test]
fn subtle_has_switches_popups_and_the_glow_only() {
    let t = theme();
    for cue in every_cue() {
        let (mut e, _) = engine(Level::Subtle);
        let shows = matches!(cue, Cue::Screen | Cue::Popup | Cue::Changed(_));
        e.cue(cue.clone(), &t);
        assert_eq!(e.running(), shows, "{cue:?} at subtle");
        let (mut e, _) = engine(Level::Full);
        e.cue(cue.clone(), &t);
        let runs = !matches!(cue, Cue::MessageGone);
        assert_eq!(e.running(), runs, "{cue:?} at full");
    }
}

#[test]
fn a_key_during_an_effect_drops_it_and_the_next_frame_is_plain() {
    let t = theme();
    let (mut e, clock) = engine(Level::Full);
    e.cue(Cue::Page, &t);
    let first = draw(&mut e, &t, popup_marks);
    assert_ne!(first, plain(&t));
    clock.advance(30);
    e.input();
    assert!(e.running(), "one frame to put it right");
    assert_eq!(draw(&mut e, &t, popup_marks), plain(&t));
    assert!(!e.running());
}

#[test]
fn rows_a_sync_changed_glow_where_they_are_drawn_and_fade() {
    let t = theme();
    let (mut e, clock) = engine(Level::Subtle);
    let mut changes = Changes::default();
    changes.changed.insert("row:3".into());
    e.cue(Cue::Changed(changes), &t);
    let first = draw(&mut e, &t, popup_marks);
    assert!(differs_in(&first, &plain(&t), row_rect(3)), "row 3 glows");
    for y in (0..AREA.height).filter(|y| *y != 3) {
        assert!(!differs_in(&first, &plain(&t), row_rect(y)), "row {y} does not");
    }
    let bg = |b: &Buffer| b[(20, 3)].bg;
    assert_ne!(bg(&first), bg(&plain(&t)), "the background, not the text");
    assert_eq!(first[(0, 3)].fg, plain(&t)[(0, 3)].fg);
    clock.advance(u64::from(GLOW_MS));
    let _ = draw(&mut e, &t, popup_marks);
    assert_eq!(draw(&mut e, &t, popup_marks), plain(&t));
}

#[test]
fn a_row_moved_by_the_sync_glows_where_it_now_is_and_one_not_shown_is_let_go() {
    let t = theme();
    let (mut e, clock) = engine(Level::Subtle);
    let mut changes = Changes::default();
    changes.changed.insert("row:x".into());
    changes.changed.insert("row:gone".into());
    e.cue(Cue::Changed(changes), &t);
    let at = |y| move || row("row:x", row_rect(y));
    let first = draw(&mut e, &t, at(1));
    assert!(differs_in(&first, &plain(&t), row_rect(1)));
    clock.advance(40);
    let moved = draw(&mut e, &t, at(5));
    assert!(differs_in(&moved, &plain(&t), row_rect(5)));
    assert!(!differs_in(&moved, &plain(&t), row_rect(1)));
    clock.advance(40);
    let _ = draw(&mut e, &t, || {});
    let _ = draw(&mut e, &t, || {});
    assert!(!e.running(), "rows no longer drawn are let go");
}

#[test]
fn new_rows_sweep_in_at_full_and_glow_at_subtle() {
    let t = theme();
    let mut changes = Changes::default();
    changes.added.insert("row:4".into());
    let (mut full, _) = engine(Level::Full);
    full.cue(Cue::Changed(changes.clone()), &t);
    let swept = draw(&mut full, &t, popup_marks);
    assert_ne!(swept[(25, 4)].fg, plain(&t)[(25, 4)].fg, "text comes in");
    let (mut subtle, _) = engine(Level::Subtle);
    subtle.cue(Cue::Changed(changes), &t);
    let glowed = draw(&mut subtle, &t, popup_marks);
    assert_eq!(glowed[(0, 4)].fg, plain(&t)[(0, 4)].fg);
    assert_ne!(glowed[(0, 4)].bg, plain(&t)[(0, 4)].bg);
}

#[test]
fn a_waiting_panel_shimmers_at_full_while_it_is_marked_and_input_does_not_stop_it() {
    let t = theme();
    let (mut e, clock) = engine(Level::Full);
    // The blank columns right of each row's words: outlines are drawn on blank rows only.
    let wait = || loading(Rect::new(22, 0, 8, AREA.height));
    let _ = draw(&mut e, &t, wait);
    assert!(e.running(), "frames keep coming while it waits");
    let mut changed = false;
    for _ in 0..10 {
        clock.advance(100);
        changed |= draw(&mut e, &t, wait) != plain(&t);
    }
    assert!(changed, "the rows to come are outlined");
    e.input();
    let _ = draw(&mut e, &t, wait);
    assert!(e.running(), "a key does not stop the wait showing");
    let _ = draw(&mut e, &t, || {});
    let _ = draw(&mut e, &t, || {});
    assert!(!e.running(), "the data came: it stops");

    let (mut subtle, _) = engine(Level::Subtle);
    assert_eq!(draw(&mut subtle, &t, wait), plain(&t), "not at subtle");
    assert!(!subtle.running());
}

/// Loading policy composes the real mark ledger and engine over small owned buffers,
/// with a manual clock and no host, terminal, I/O or full application frame.
mod loading_policy {
    use super::*;

    #[test]
    fn leaving_full_drops_every_shimmer_and_subtle_never_rediscovers_loading() {
        let t = theme();
        let (mut e, clock) = engine(Level::Full);
        let wait = || {
            loading(Rect::new(22, 1, 8, 1));
            loading(Rect::new(22, 5, 8, 1));
        };
        let _ = draw(&mut e, &t, wait);
        clock.advance(300);
        assert_ne!(draw(&mut e, &t, wait), plain(&t));

        e.set_level(Level::Subtle);
        e.set_level(Level::Subtle);
        assert_eq!(e.next_frame(), Some(FRAME), "one clean frame is still owed");
        assert_eq!(draw(&mut e, &t, wait), plain(&t));
        assert!(!e.running());
        assert_eq!(e.next_frame(), None);
        for areas in [vec![], vec![row_rect(2)], vec![row_rect(1), row_rect(5)]] {
            clock.advance(300);
            e.set_level(Level::Subtle);
            let b = draw(&mut e, &t, || {
                for &area in &areas {
                    loading(area);
                }
            });
            assert_eq!(b, plain(&t));
            assert_eq!(e.next_frame(), None, "neither old nor new loading restarts subtle");
        }
    }

    #[test]
    fn leaving_full_preserves_compatible_cues_and_their_elapsed_time() {
        let t = theme();
        let mut changes = Changes::default();
        changes.changed.insert("row:3".into());
        for cue in [Cue::Screen, Cue::Popup, Cue::Changed(changes)] {
            let (mut actual, actual_clock) = engine(Level::Full);
            let (mut expected, expected_clock) = engine(Level::Full);
            let wait = || {
                popup_marks();
                loading(row_rect(7));
            };
            actual.cue(cue.clone(), &t);
            expected.cue(cue.clone(), &t);
            let _ = draw(&mut actual, &t, wait);
            let _ = draw(&mut expected, &t, popup_marks);
            actual_clock.advance(30);
            expected_clock.advance(30);
            let _ = draw(&mut actual, &t, wait);
            let _ = draw(&mut expected, &t, popup_marks);

            actual.set_level(Level::Subtle);
            actual.set_level(Level::Subtle);
            for elapsed in [30, 30, 360, 30] {
                actual_clock.advance(elapsed);
                expected_clock.advance(elapsed);
                assert_eq!(
                    draw(&mut actual, &t, wait),
                    draw(&mut expected, &t, popup_marks),
                    "{cue:?} keeps its original elapsed time"
                );
                assert_eq!(actual.running(), expected.running());
            }
            assert_eq!(actual.next_frame(), None);
        }
    }

    #[test]
    fn repeated_full_preserves_shimmer_phase_and_off_clears_pending_settle() {
        let t = theme();
        let (mut actual, actual_clock) = engine(Level::Full);
        let (mut expected, expected_clock) = engine(Level::Full);
        let wait = || loading(AREA);
        let _ = draw(&mut actual, &t, wait);
        let _ = draw(&mut expected, &t, wait);
        for elapsed in [100, 200, 300] {
            actual_clock.advance(elapsed);
            expected_clock.advance(elapsed);
            actual.set_level(Level::Full);
            assert_eq!(draw(&mut actual, &t, wait), draw(&mut expected, &t, wait));
        }
        actual.set_level(Level::Subtle);
        assert!(actual.running(), "the removed shimmer owes a clean frame");
        actual_clock.advance(400);
        actual.set_level(Level::Full);
        let (mut fresh, _) = engine(Level::Full);
        assert_eq!(
            draw(&mut actual, &t, wait),
            draw(&mut fresh, &t, wait),
            "a downgrade discards the old shimmer clock"
        );
        actual.set_level(Level::Subtle);
        actual.set_level(Level::Off);
        assert_eq!(actual.next_frame(), None, "off drops even a pending settling frame");
        assert_eq!(draw(&mut actual, &t, wait), plain(&t));

        actual_clock.advance(1000);
        actual.set_level(Level::Full);
        assert!(!actual.running(), "a level change cannot revive discarded loading");
        let (mut fresh, _) = engine(Level::Full);
        assert_eq!(
            draw(&mut actual, &t, wait),
            draw(&mut fresh, &t, wait),
            "fresh discovery starts at zero elapsed time"
        );
        assert!(actual.running());
    }

    #[test]
    fn slow_terminal_off_policy_cannot_be_raised_while_loading_is_marked() {
        let t = theme();
        let (mut e, _) = engine(Level::Full);
        let wait = || loading(AREA);
        let _ = draw(&mut e, &t, wait);
        let slow = Duration::from_millis(100);
        assert_eq!(e.drew_in(slow), None);
        assert_eq!(e.drew_in(slow), None);
        assert!(e.drew_in(slow).is_some());
        for level in [Level::Subtle, Level::Full, Level::Off, Level::Full] {
            e.set_level(level);
            assert_eq!(e.level(), Level::Off);
            assert_eq!(draw(&mut e, &t, wait), plain(&t));
            assert_eq!(e.next_frame(), None);
        }
    }
}

#[test]
fn a_message_that_leaves_fades_out_from_what_was_shown() {
    let t = theme();
    let (mut e, clock) = engine(Level::Full);
    let line = row_rect(7);
    let shown = draw(&mut e, &t, || region(Region::Message, line));
    e.cue(Cue::MessageGone, &t);
    // The message is gone from the frame: the row now has other text.
    begin();
    let mut b = Buffer::empty(AREA);
    b.set_string(0, 7, "keys keys keys", t.s(Slot::TextMuted));
    e.draw(&mut b, &t);
    assert_eq!(b[(0, 7)].symbol(), shown[(0, 7)].symbol(), "the old text, fading");
    clock.advance(u64::from(MESSAGE_GONE_MS));
    let _ = draw(&mut e, &t, || {});
    let _ = draw(&mut e, &t, || {});
    assert!(!e.running());
}

#[test]
fn a_slow_terminal_turns_motion_off_for_the_session() {
    let t = theme();
    let (mut e, _) = engine(Level::Full);
    e.cue(Cue::Screen, &t);
    let slow = Duration::from_millis(100);
    assert_eq!(e.drew_in(slow), None);
    assert_eq!(e.drew_in(Duration::from_millis(5)), None, "a fast frame starts again");
    assert_eq!(e.drew_in(slow), None);
    assert_eq!(e.drew_in(slow), None);
    assert!(e.drew_in(slow).is_some(), "the third slow one in a row");
    assert_eq!(e.level(), Level::Off);
    assert!(!e.running());
    e.set_level(Level::Full);
    assert_eq!(e.level(), Level::Off, "it stays off");
}

#[test]
fn changes_compare_rows_and_the_first_data_is_not_a_change() {
    let rows = |r: &[(&str, i32)]| -> Vec<(String, i32)> {
        r.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    };
    assert!(Changes::between(rows(&[]), rows(&[("a", 1)])).is_empty());
    let c = Changes::between(
        rows(&[("a", 1), ("b", 2), ("gone", 0)]),
        rows(&[("a", 1), ("b", 3), ("c", 4)]),
    );
    assert_eq!(c.changed.iter().collect::<Vec<_>>(), ["b"]);
    assert_eq!(c.added.iter().collect::<Vec<_>>(), ["c"]);
}

#[test]
fn motion_is_held_back_over_ssh_and_below_full_colour() {
    assert_eq!(effective(Level::Full, Tier::TrueColor, false), Level::Full);
    assert_eq!(effective(Level::Full, Tier::TrueColor, true), Level::Off);
    for tier in [Tier::Ansi256, Tier::Ansi16, Tier::Mono] {
        assert_eq!(effective(Level::Subtle, tier, false), Level::Off, "{tier:?}");
    }
    assert_eq!(Level::parse("full"), Level::Full);
    assert_eq!(Level::parse("off"), Level::Off);
    assert_eq!(Level::parse("whatever"), Level::Subtle);
    for l in Level::ALL {
        assert_eq!(Level::parse(l.word()), l);
    }
}

/// Ink's motion engine is checked against this engine cell for cell
/// (`ink/test/motion/parity.test.ts`), from the frames written here: each
/// cue over [`plain`] on a [`ManualClock`], at fixed times. The committed
/// file must be what this engine draws; `RADAR_MOTION_REF=1` rewrites it.
#[test]
fn the_ink_motion_reference_is_what_this_engine_draws() {
    use serde_json::{Value, json};
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ink/test/motion/fixtures/ratatui-motion.json");
    let t = theme();
    let message = Rect {
        x: 0,
        y: 7,
        width: 14,
        height: 1,
    };
    let hex = |c: Color| match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => "reset".into(),
        other => format!("{other:?}"),
    };
    let ink = Ink::of(&t);
    let muted = t.s(Slot::TextMuted).fg.unwrap_or(Color::Reset);
    let marks = |shimmer: bool| {
        move || {
            popup_marks();
            region(Region::Message, message);
            if shimmer {
                loading(AREA);
            }
        }
    };
    let after_message = || {
        let mut b = Buffer::empty(AREA);
        b.set_style(AREA, t.bg(Slot::BgSurface));
        b.set_string(0, 7, "keys keys keys", t.s(Slot::TextMuted));
        b
    };
    let mut changes = Changes::default();
    changes.changed.insert("row:3".into());
    changes.added.insert("row:5".into());
    let effect_times: Vec<u64> = vec![0, 30, 60, 100, 140, 180, 200, 230, 260, 300, 340, 400, 450, 500];
    let scenarios: Vec<(&str, Level, Option<Cue>, Vec<u64>)> = vec![
        ("plain", Level::Full, None, vec![0]),
        ("screen", Level::Subtle, Some(Cue::Screen), effect_times.clone()),
        ("assemble", Level::Full, Some(Cue::Screen), effect_times.clone()),
        ("page", Level::Full, Some(Cue::Page), effect_times.clone()),
        ("popup", Level::Subtle, Some(Cue::Popup), effect_times.clone()),
        ("message", Level::Full, Some(Cue::Message), effect_times.clone()),
        ("message_gone", Level::Full, Some(Cue::MessageGone), effect_times.clone()),
        ("changed_subtle", Level::Subtle, Some(Cue::Changed(changes.clone())), effect_times.clone()),
        ("changed_full", Level::Full, Some(Cue::Changed(changes)), effect_times),
        ("shimmer", Level::Full, None, (0..16).map(|i| i * 100).collect()),
    ];
    let mut out = Vec::new();
    for (name, level, cue, times) in scenarios {
        let (mut e, clock) = engine(level);
        let gone = name == "message_gone";
        let mk = marks(name == "shimmer");
        if gone {
            let _ = draw(&mut e, &t, mk);
        }
        if let Some(cue) = cue {
            e.cue(cue, &t);
        }
        let mut now = 0;
        let mut frames = Vec::new();
        for at in times {
            clock.advance(at - now);
            now = at;
            begin();
            mk();
            let mut b = if gone {
                after_message()
            } else if name == "assemble" {
                panels_frame(&t)
            } else {
                plain(&t)
            };
            e.draw(&mut b, &t);
            let cells: Vec<Value> = b
                .content
                .iter()
                .map(|c| json!([c.symbol(), hex(c.fg), hex(c.bg), format!("{:?}", c.modifier)]))
                .collect();
            frames.push(json!({ "t": at, "running": e.running(), "cells": cells }));
        }
        out.push(json!({ "name": name, "level": level.word(), "frames": frames }));
    }
    let written = json!({
        "colours": {
            "base": hex(ink.base), "surface": hex(ink.surface), "overlay": hex(ink.overlay),
            "glow": hex(ink.glow), "text": hex(ink.text), "muted": hex(muted),
        },
        "width": AREA.width, "height": AREA.height,
        "popup": [POPUP.x, POPUP.y, POPUP.width, POPUP.height],
        "message": [message.x, message.y, message.width, message.height],
        "scenarios": out,
    });
    if std::env::var_os("RADAR_MOTION_REF").is_some() {
        std::fs::write(&path, serde_json::to_string(&written).unwrap()).unwrap();
        return;
    }
    let committed: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for (a, b) in written["scenarios"].as_array().unwrap().iter().zip(committed["scenarios"].as_array().unwrap()) {
        assert_eq!(a, b, "{}: run with RADAR_MOTION_REF=1", a["name"]);
    }
    assert_eq!(written, committed, "run with RADAR_MOTION_REF=1");
}
