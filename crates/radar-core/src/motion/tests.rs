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
        (Cue::Screen, SCREEN_MS),
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
    e.cue(Cue::Screen, &t);
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
    let wait = || loading(AREA);
    let _ = draw(&mut e, &t, wait);
    assert!(e.running(), "frames keep coming while it waits");
    let mut changed = false;
    for _ in 0..10 {
        clock.advance(100);
        changed |= draw(&mut e, &t, wait) != plain(&t);
    }
    assert!(changed, "a band of light crosses it");
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
            let mut b = if gone { after_message() } else { plain(&t) };
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
