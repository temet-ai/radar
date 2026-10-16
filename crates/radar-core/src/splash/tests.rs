use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use serde_json::{Value, json};

use super::*;
use crate::theme::ThemeConfig;

fn theme(colors: &str, tier: Tier) -> Theme {
    let cfg = ThemeConfig { colors: colors.into(), ..Default::default() };
    Theme::build(&cfg, tier, &[])
}

fn truecolour() -> Theme {
    theme("truecolor", Tier::TrueColor)
}

fn contact(name: &str, mark: char, tail: &str, count: u32) -> Contact {
    Contact { name: name.into(), mark: mark.to_string(), tail: Some(tail.into()), count, live: true }
}

/// The board's contacts at an ordinary start.
fn contacts() -> Vec<Contact> {
    vec![
        contact("Ledger", '\u{e7eb}', "3 to review", 3),
        contact("Desk", '\u{e75c}', "4 yours", 4),
        contact("Chats", '\u{f198}', "5 mentions", 5),
        contact("Forges", '\u{f408}', "2 failing", 2),
        contact("cog", '\u{e702}', "2 dirty", 2),
        contact("agents", '\u{f51b}', "1 waiting", 1),
    ]
}

fn words(text: &str) -> Vec<LineSpan> {
    vec![LineSpan { text: text.into(), key: false }]
}

fn start(echo: Echo, event: Option<Event>, line: &str) -> Start {
    Start { echo, event, line: words(line) }
}

fn scene(style: Style, at: u32) -> Scene {
    Scene { style, at, contacts: contacts(), start: start(Echo::Plane, None, "Sweeping the horizon for anything new.") }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Each row's characters.
fn text(frame: &Frame) -> Vec<String> {
    frame.rows.iter().map(|runs| runs.iter().map(|r| r.text.as_str()).collect()).collect()
}

/// The column `needle` starts at in a row.
fn col(row: &str, needle: &str) -> usize {
    crate::ui::width(&row[..row.find(needle).unwrap_or_else(|| panic!("{needle:?} in {row:?}"))])
}

/// The run holding column `x` of row `y`.
fn run_at(frame: &Frame, x: usize, y: usize) -> &Run {
    let mut at = 0;
    for r in &frame.rows[y] {
        let w = crate::ui::width(&r.text);
        if x < at + w {
            return r;
        }
        at += w;
    }
    panic!("no run at {x},{y}");
}

fn fg(theme: &Theme, slot: Slot) -> Option<Color> {
    theme.s(slot).fg
}

/// The rows of the scope in a frame laid out at 64 wide.
fn scope_rows(f: &Frame) -> Vec<String> {
    text(f)[..SCOPE_H as usize].to_vec()
}

#[test]
fn the_mark_is_four_rows_of_half_blocks() {
    let rows: Vec<String> =
        mark_pixels().iter().map(|r| r.iter().map(|&(t, b)| half_block(t, b)).collect()).collect();
    assert_eq!(
        rows,
        [
            "                █            ",
            "█▄▀▀▀  ▀▀▀▄ ▄▀▀▀█  ▀▀▀▄ █▄▀▀▀",
            "█     ▄▀▀▀█ █   █ ▄▀▀▀█ █    ",
            "▀      ▀▀▀▀  ▀▀▀▀  ▀▀▀▀ ▀    ",
        ]
    );
    assert_eq!((rows.len(), crate::ui::width(&rows[0])), (MARK_H as usize, MARK_W as usize));
}

/// The board's own frames ([`frame`] ported from the canvas's sweep board), character for character: the
/// scope's braille, the labels, the readout, the mark and the line. Colours come from theme slots here, so only
/// the characters are compared; a few braille cells may differ where the board's noise rounds the other way.
#[test]
fn the_scope_draws_the_boards_frames() {
    let doc: Value = serde_json::from_str(include_str!("board-frames.json")).unwrap();
    let t = truecolour();
    for f in doc["frames"].as_array().unwrap() {
        let style = match f["motion"].as_str().unwrap() {
            "full" => Style::Full,
            "subtle" => Style::Subtle,
            _ => Style::Static,
        };
        let echo = match f["echo"].as_str().unwrap() {
            "starship" => Echo::Starship,
            "battlestar" => Echo::Battlestar,
            "mech" => Echo::Mech,
            _ => Echo::Plane,
        };
        let fire = f["fire"].as_bool().unwrap();
        let at = (f["t"].as_f64().unwrap() * 1000.0).round() as u32;
        let line = f["line"].as_str().unwrap();
        let said = if fire {
            "This is fine."
        } else if echo.rare() {
            "That wasn’t a plane."
        } else {
            "Sweeping the horizon for anything new."
        };
        let scene = Scene { style, at, contacts: contacts(), start: start(echo, fire.then_some(Event::Fire), said) };
        let name = format!("{} {:?} fire={fire} t={at}", f["motion"], echo);
        let frame = frame(&t, &scene, SCOPE_W, FULL_HEIGHT).unwrap();
        let got = text(&frame);
        let want: Vec<&str> = f["scope"].as_array().unwrap().iter().map(|r| r.as_str().unwrap()).collect();
        let mut off = 0;
        for (y, (g, w)) in scope_rows(&frame).iter().zip(&want).enumerate() {
            let (g, w): (Vec<char>, Vec<char>) = (g.chars().collect(), w.chars().collect());
            assert_eq!(g.len(), w.len(), "{name}: row {y} is as wide");
            off += g.iter().zip(&w).filter(|(a, b)| a != b).count();
        }
        assert!(off <= 6, "{name}: {off} scope cells differ from the board");
        // The board's fixture drops the blank run between the bearing and the count.
        assert_eq!(got[SCOPE_H as usize].trim().replace("°   ", "°"), f["readout"].as_str().unwrap(), "{name}: the readout");
        assert_eq!(got[FULL_HEIGHT as usize - 1].trim(), line.trim(), "{name}: the line as typed so far");
        if !fire {
            let mark: Vec<String> = got[26..30].iter().map(|r| r.trim_end().to_string()).collect();
            let want: Vec<String> = f["mark"]
                .as_array()
                .unwrap()
                .iter()
                .map(|r| format!("{:>w$}", r.as_str().unwrap(), w = 17 + 29).trim_end().to_string())
                .collect();
            assert_eq!(mark.iter().map(|r| r.trim()).collect::<Vec<_>>(), want.iter().map(|r| r.trim()).collect::<Vec<_>>(), "{name}: the mark");
        }
    }
}

#[test]
fn the_screen_is_the_scope_the_mark_the_names_and_the_line_centred_on_the_base() {
    let t = truecolour();
    let f = frame(&t, &scene(Style::Static, 0), 160, 40).unwrap();
    let rows = text(&f);
    assert_eq!(rows.len(), 40);
    assert!(rows.iter().all(|r| crate::ui::width(r) == 160), "every row runs the full width");
    // 34 rows used, (40 - 34) / 2 = 3 above; the scope is 64 wide from (160 - 64) / 2 = 48.
    assert!(rows[..3].iter().all(|r| r.trim().is_empty()));
    assert!(rows[3..27].iter().any(|r| r.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))), "braille");
    assert_eq!(rows[27].trim(), "6 contacts · 17 for you");
    assert!(rows[29].contains('▀') && rows[32].contains('▀'), "the mark, four rows");
    assert_eq!(rows[34].trim(), "Ledger · Desk · Chats · Forges · cog · agents");
    assert_eq!(rows[36].trim(), "Sweeping the horizon for anything new.");
    let line = &rows[36];
    assert_eq!(col(line, "Sweeping"), (160 - crate::ui::width(line.trim())) / 2);
    let base = fg(&t, Slot::BgBase);
    assert!(f.rows.iter().flatten().filter(|r| !r.text.contains('▀')).all(|r| r.bg == base), "on bg.base");
    assert_eq!(run_at(&f, col(line, "Sweeping"), 36).fg, fg(&t, Slot::TextSecondary));
    let names = &rows[34];
    assert_eq!(run_at(&f, col(names, "Ledger"), 34).fg, fg(&t, Slot::AccentPrimary));
    assert_eq!(run_at(&f, col(names, "·"), 34).fg, fg(&t, Slot::TextMuted));
}

#[test]
fn each_contact_is_labelled_with_its_mark_name_and_tail_and_a_lost_one_says_no_signal() {
    let t = truecolour();
    let mut s = scene(Style::Static, 0);
    s.contacts[2].live = false;
    let f = frame(&t, &s, 64, 34).unwrap();
    let all = text(&f).join("\n");
    assert!(all.contains("\u{e7eb} Ledger  3 to review"), "{all}");
    assert!(all.contains("\u{f198} Chats  no signal"));
    assert!(all.contains("2 failing  Forges \u{f408}") || all.contains("\u{f408} Forges  2 failing"));
    let readout = &text(&f)[SCOPE_H as usize];
    assert_eq!(readout.trim(), "5 contacts · 12 for you · Chats no signal");
    let lost = &text(&f).iter().position(|r| r.contains("Chats  no signal")).unwrap();
    let row = &text(&f)[*lost];
    assert_eq!(run_at(&f, col(row, "Chats"), *lost).fg, fg(&t, Slot::TextMuted), "a lost contact is muted");
    let row_g = text(&f).iter().position(|r| r.contains("Ledger  3")).unwrap();
    let r = &text(&f)[row_g];
    assert_eq!(run_at(&f, col(r, "3 to"), row_g).fg, fg(&t, Slot::DataNumber), "the tail is a number");
}

#[test]
fn the_beam_turns_once_and_the_readout_tracks_its_bearing() {
    let t = truecolour();
    let at = |ms| frame(&t, &scene(Style::Full, ms), 64, 34).unwrap();
    assert!(text(&at(0)).iter().all(|r| r.trim().is_empty() || r.chars().all(|c| c == ' ' || ('\u{2800}'..='\u{28ff}').contains(&c))));
    assert_eq!(text(&at(100))[SCOPE_H as usize].trim(), "", "no readout before the beam");
    assert_eq!(text(&at(800))[SCOPE_H as usize].trim(), "brg 180°   3 contacts");
    assert_eq!(text(&at(1500))[SCOPE_H as usize].trim(), "6 contacts · 17 for you");
    // The name paints in from 1300 ms; before it the mark's rows are empty.
    assert!(text(&at(1200))[27].trim().is_empty());
    assert!(text(&at(1800))[27].contains('▀'));
    let hot = run_at(&at(1320), (64 - 29) / 2, 27).fg;
    assert_eq!(hot, at(1320).rows.iter().flatten().find(|r| r.text.contains('▀')).unwrap().fg, "the scan line is hot");
}

#[test]
fn the_line_types_from_1800_ms_and_keys_are_in_the_key_colour() {
    let t = truecolour();
    let mut s = scene(Style::Full, 1799);
    s.start.line = vec![
        LineSpan { text: "Press ".into(), key: false },
        LineSpan { text: "x".into(), key: true },
        LineSpan { text: " for every key here.".into(), key: false },
    ];
    let line = |s: &Scene| text(&frame(&t, s, 64, 34).unwrap())[33].trim().to_string();
    assert_eq!(line(&s), "");
    s.at = 1800;
    assert_eq!(line(&s), "P");
    s.at = 1800 + 12 * 6;
    assert_eq!(line(&s), "Press x");
    s.at = 2300;
    assert_eq!(line(&s), "Press x for every key here.");
    let f = frame(&t, &s, 64, 34).unwrap();
    let x = col(&text(&f)[33], "x ");
    let key = run_at(&f, x, 33);
    assert_eq!((key.fg, key.bold), (fg(&t, Slot::AccentPrimary), true));
}

#[test]
fn a_rare_echo_says_one_unidentified() {
    let t = truecolour();
    let mut s = scene(Style::Static, 0);
    s.start.echo = Echo::Mech;
    let f = frame(&t, &s, 64, 34).unwrap();
    assert!(text(&f)[SCOPE_H as usize].ends_with("· 1 unidentified") || text(&f)[SCOPE_H as usize].trim().ends_with("1 unidentified"));
    assert_eq!(Echo::roll(0.01), Echo::Starship);
    assert_eq!(Echo::roll(0.03), Echo::Battlestar);
    assert_eq!(Echo::roll(0.05), Echo::Mech);
    assert_eq!(Echo::roll(0.06), Echo::Plane);
    assert!(!Echo::Plane.rare());
}

#[test]
fn the_fire_takes_the_marks_and_names_place_at_full_only_and_holds_longer() {
    let t = truecolour();
    let mut s = scene(Style::Full, 2000);
    s.start = start(Echo::Plane, Some(Event::Fire), "This is fine.");
    let f = frame(&t, &s, 64, 34).unwrap();
    let rows = text(&f);
    assert!(rows[25..32].iter().any(|r| r.contains('▀')), "it burns");
    assert!(!rows[31].contains("Ledger"), "no names");
    assert_eq!(rows[33].trim(), "This is fine.");
    let warm = f.rows[25..32].iter().flatten().filter(|r| r.text.contains('▀')).count();
    assert!(warm > 10, "many fire cells: {warm}");
    // At subtle it is the ordinary splash.
    s.style = Style::Subtle;
    let f = frame(&t, &s, 64, 34).unwrap();
    assert_eq!(text(&f)[31].trim(), "Ledger · Desk · Chats · Forges · cog · agents");
    let mut sp = Splash::new(Setting::Auto, Level::Full);
    sp.set_start(start(Echo::Plane, Some(Event::Fire), "This is fine."));
    assert_eq!(sp.hold(), HOLD_FULL_MS + EVENT_HOLD_MS);
}

#[test]
fn below_truecolour_it_draws_from_the_slots_and_with_no_colour_the_mark_is_bold() {
    let t = theme("256", Tier::Ansi256);
    let f = frame(&t, &scene(Style::Static, 0), 64, 34).unwrap();
    let x = (64 - 29) / 2;
    assert_eq!(run_at(&f, x, 27).fg, fg(&t, Slot::AccentPrimary));
    let t = theme("none", Tier::Mono);
    let f = frame(&t, &scene(Style::Static, 0), 64, 34).unwrap();
    let r = run_at(&f, x, 27);
    assert_eq!((r.fg, r.bold), (None, true));
}

#[test]
fn full_holds_until_the_line_is_typed_then_hands_off_once() {
    let mut s = Splash::new(Setting::Auto, Level::Full);
    let c = contacts();
    assert!(matches!(s.step(ms(1000), true, &c), Step::Show(Scene { at: 0, .. })));
    assert!(matches!(s.step(ms(1300), true, &c), Step::Show(Scene { at: 300, .. })));
    assert_eq!(s.next_frame(), Some(FRAME));
    assert!(matches!(s.step(ms(1000 + HOLD_FULL_MS as u64 - 1), true, &c), Step::Show(_)));
    assert_eq!(s.step(ms(1000 + HOLD_FULL_MS as u64), true, &c), Step::HandOff);
    assert_eq!(s.step(ms(5000), true, &c), Step::Hidden);
    assert_eq!(s.next_frame(), None, "no frame tick once it is gone");
}

const _: () = assert!(HOLD_FULL_MS >= LINE_FROM_MS + TYPE_MS, "the line is typed before it hands off");

#[test]
fn a_slow_start_keeps_the_splash_until_the_first_screen_is_ready() {
    let mut s = Splash::new(Setting::Auto, Level::Full);
    assert!(matches!(s.step(ms(0), false, &[]), Step::Show(_)));
    assert!(matches!(s.step(ms(4000), false, &[]), Step::Show(_)));
    assert_eq!(s.step(ms(4030), true, &[]), Step::HandOff);
}

#[test]
fn subtle_fades_in_with_no_beam_and_hands_off_sooner() {
    let mut s = Splash::new(Setting::Auto, Level::Subtle);
    s.set_start(start(Echo::Plane, None, "Sweeping the horizon for anything new."));
    let Step::Show(scene) = s.step(ms(0), true, &contacts()) else { panic!("shown") };
    assert_eq!(scene.style, Style::Subtle);
    assert!(matches!(s.step(ms(HOLD_SUBTLE_MS as u64 - 1), true, &[]), Step::Show(_)));
    assert_eq!(s.step(ms(HOLD_SUBTLE_MS as u64), true, &[]), Step::HandOff);
    let t = truecolour();
    let early = frame(&t, &Scene { style: Style::Subtle, ..scene.clone() }, 64, 34).unwrap();
    assert!(text(&early)[33].trim().is_empty(), "the line waits");
    let late = frame(&t, &Scene { style: Style::Subtle, at: 1400, ..scene }, 64, 34).unwrap();
    assert_eq!(text(&late)[33].trim(), "Sweeping the horizon for anything new.", "then shows whole");
}

#[test]
fn with_motion_off_it_shows_only_the_last_frame_when_startup_is_slow() {
    let mut fast = Splash::new(Setting::Auto, Level::Off);
    assert_eq!(fast.step(ms(0), true, &[]), Step::Hidden, "ready at once: skipped, no fade");
    assert!(!fast.active());
    let mut slow = Splash::new(Setting::Auto, Level::Off);
    assert_eq!(slow.step(ms(0), false, &[]), Step::Hidden);
    assert_eq!(slow.step(ms(STATIC_AFTER_MS as u64 - 1), false, &[]), Step::Hidden);
    let Step::Show(scene) = slow.step(ms(STATIC_AFTER_MS as u64), false, &[]) else { panic!("shown when slow") };
    assert_eq!(scene.style, Style::Static);
    assert_eq!(slow.step(ms(900), true, &[]), Step::Hidden, "gone when ready, with no hand-off fade");
}

#[test]
fn off_never_shows_and_a_key_skips_it_at_once() {
    let mut off = Splash::new(Setting::Off, Level::Full);
    assert_eq!(off.step(ms(0), false, &[]), Step::Hidden);
    assert_eq!(off.next_frame(), None);
    let mut s = Splash::new(Setting::Auto, Level::Full);
    assert!(matches!(s.step(ms(0), true, &[]), Step::Show(_)));
    assert!(s.input(), "it was showing");
    assert_eq!(s.step(ms(30), true, &[]), Step::Hidden, "skipped: no hand-off, no fade");
    assert!(!s.input());
    let mut later = Splash::new(Setting::Auto, Level::Full);
    later.step(ms(0), true, &[]);
    later.set_setting(Setting::Off);
    assert_eq!(later.step(ms(30), true, &[]), Step::Hidden, "turned off in settings");
    assert_eq!(Setting::parse("off"), Setting::Off);
    assert_eq!(Setting::parse("whatever"), Setting::Auto);
}

#[test]
fn small_terminals_drop_the_scope_then_the_mark_and_below_twenty_columns_show_nothing() {
    let t = truecolour();
    assert!(frame(&t, &scene(Style::Subtle, 1000), MIN_WIDTH - 1, 24).is_none());
    assert!(frame(&t, &scene(Style::Subtle, 1000), 80, 1).is_none());
    // 80 x 24 has no room for the scope: the mark, the names and the line.
    let f = frame(&t, &scene(Style::Full, 1100), 80, 24).unwrap();
    let rows = text(&f);
    assert!(!rows.iter().any(|r| r.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c))), "no scope");
    assert!(rows[8].contains('▀'), "the mark from row (24 - 8) / 2");
    assert_eq!(rows[13].trim(), "Ledger · Desk · Chats · Forges · cog · agents");
    assert_eq!(rows[15].trim(), "Sweeping the horizon for anything new.", "already typed: the beam is skipped");
    let short = frame(&t, &scene(Style::Static, 0), 80, 6).unwrap();
    let rows = text(&short);
    assert!(!rows.iter().any(|r| r.contains('▀')), "no room for the mark: text only");
    assert_eq!(rows[2].trim(), "Ledger · Desk · Chats · Forges · cog · agents");
    assert_eq!(rows[3].trim(), "Sweeping the horizon for anything new.");
    let narrow = frame(&t, &scene(Style::Static, 0), 20, 10).unwrap();
    assert!(text(&narrow).iter().all(|r| crate::ui::width(r) == 20), "cut at the edge");
}

#[test]
fn ratatui_paints_the_frame_and_ink_is_sent_the_same_cells() {
    let t = truecolour();
    let f = frame(&t, &scene(Style::Full, 1500), 80, 40).unwrap();
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 40));
    draw(&f, &mut buf);
    let v = view(&f);
    assert_eq!((v.width, v.height, v.rows.len()), (80, 40, 40));
    let painted = cells(&buf);
    let sent = view_cells(&v);
    assert_eq!(painted, sent);
}

/// Each cell of `buf` as `[symbol, fg, bg, bold]`, colours as Ink's colour words.
fn cells(buf: &Buffer) -> Vec<Vec<Value>> {
    use crate::ipc::look::colour_word;
    let a = buf.area;
    (0..a.height)
        .map(|y| {
            (0..a.width)
                .map(|x| {
                    let c = &buf[(x, y)];
                    let word = |c: Color| colour_word(Some(c));
                    json!([c.symbol(), word(c.fg), word(c.bg), c.modifier.contains(Modifier::BOLD)])
                })
                .collect()
        })
        .collect()
}

fn view_cells(v: &SplashView) -> Vec<Vec<Value>> {
    v.rows
        .iter()
        .map(|runs| {
            runs.iter()
                .flat_map(|r| r.text.chars().map(move |c| json!([c.to_string(), r.fg, r.bg, r.bold])))
                .collect()
        })
        .collect()
}

/// The frames Ink's cell-parity test draws (ink/test/splash/parity.test.tsx): the splash view sent and the
/// cells Ratatui painted, per frame. `RADAR_SPLASH_CELLS_DIR=$PWD/ink/test/fixtures/splash` writes them; without
/// it the committed files are compared and drift fails.
#[test]
fn the_cells_ink_compares_against_are_the_committed_fixtures() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ink/test/fixtures/splash");
    let write = std::env::var_os("RADAR_SPLASH_CELLS_DIR").map(std::path::PathBuf::from);
    let t = truecolour();
    let fire = |at| Scene { start: start(Echo::Plane, Some(Event::Fire), "This is fine."), ..scene(Style::Full, at) };
    for (name, s, w, h) in [
        ("full-600-80x24", scene(Style::Full, 600), 80, 24),
        ("full-1500-160x40", scene(Style::Full, 1500), 160, 40),
        ("subtle-1400-80x24", scene(Style::Subtle, 1400), 80, 24),
        ("subtle-1400-160x40", scene(Style::Subtle, 1400), 160, 40),
        ("fire-2600-160x40", fire(2600), 160, 40),
    ] {
        let f = frame(&t, &s, w, h).unwrap();
        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
        draw(&f, &mut buf);
        let doc = json!({ "format": 1, "name": name, "width": w, "height": h, "view": view(&f), "cells": cells(&buf) });
        let text = serde_json::to_string(&doc).unwrap() + "\n";
        let file = format!("{name}.json");
        if let Some(out) = &write {
            std::fs::create_dir_all(out).unwrap();
            std::fs::write(out.join(&file), &text).unwrap();
        } else {
            let have = std::fs::read_to_string(dir.join(&file))
                .unwrap_or_else(|_| panic!("{file} missing: run with RADAR_SPLASH_CELLS_DIR=$PWD/ink/test/fixtures/splash"));
            assert!(have == text, "{file} differs from the splash Ratatui paints: regenerate with RADAR_SPLASH_CELLS_DIR");
        }
    }
}

#[test]
fn a_splash_skipped_early_hands_its_line_on_once_and_a_late_one_does_not() {
    let line = vec![LineSpan { text: "Sweeping the horizon for anything new.".into(), key: false }];
    let start = Start { line, ..Start::default() };
    let mut early = Splash::new(Setting::Auto, Level::Full);
    early.set_start(start.clone());
    let _ = early.step(Duration::from_millis(0), true, &[]);
    let _ = early.step(Duration::from_millis(400), true, &[]);
    early.input();
    assert_eq!(early.take_early_line().as_deref(), Some("Sweeping the horizon for anything new."));
    assert_eq!(early.take_early_line(), None, "once");
    let mut late = Splash::new(Setting::Auto, Level::Full);
    late.set_start(start);
    let _ = late.step(Duration::from_millis(0), true, &[]);
    let _ = late.step(Duration::from_millis(1600), true, &[]);
    late.input();
    assert_eq!(late.take_early_line(), None);
}

#[test]
fn the_pixel_tier_paints_the_scope_as_a_png_over_its_cells_and_kitty_replaces_it() {
    let theme = Theme::build(&ThemeConfig::default(), crate::design::theme::Tier::TrueColor, &[]);
    let scene = Scene { style: Style::Full, at: 900, contacts: vec![], start: Start::default() };
    let frame = frame(&theme, &scene, 160, 40).unwrap();
    let pic = picture(&frame, (10, 20)).expect("a scope in true colour");
    assert_eq!((pic.cols, pic.rows), (SCOPE_W, SCOPE_H));
    assert_eq!((pic.x, pic.y), ((160 - SCOPE_W) / 2, (40 - FULL_HEIGHT) / 2));
    // RADAR_SPLASH_PNG_DIR=<dir> keeps the picture, to look at.
    if let Ok(dir) = std::env::var("RADAR_SPLASH_PNG_DIR") {
        std::fs::write(std::path::Path::new(&dir).join("scope-900.png"), &pic.png).unwrap();
    }
    let img = image::load_from_memory(&pic.png).unwrap();
    assert_eq!((img.width(), img.height()), (640, 480));
    let shown = kitty_at(&pic);
    assert!(shown.contains(&format!("i={PICTURE_ID},p=1")), "one picture, replaced each frame");
    assert!(kitty_hide().contains(&format!("a=d,d=I,i={PICTURE_ID}")));
    // Too small for the scope, or without true colour: no picture.
    assert!(picture(&frame_of(&theme, &scene, 80, 24), (10, 20)).is_none());
    let mono = Theme::build(&ThemeConfig::default(), crate::design::theme::Tier::Ansi256, &[]);
    assert!(picture(&frame_of(&mono, &scene, 160, 40), (10, 20)).is_none());
}

fn frame_of(theme: &Theme, scene: &Scene, w: u16, h: u16) -> Frame {
    frame(theme, scene, w, h).unwrap()
}
