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

fn parts() -> Vec<Part> {
    [("Shelf", Some(0)), ("Ledger", Some(0)), ("Board", None), ("Chat", Some(100)), ("Code", Some(0))]
        .into_iter()
        .map(|(name, ready_at)| Part { name: name.into(), ready_at })
        .collect()
}

fn scene(style: Style, at: u32) -> Scene {
    Scene { style, at, parts: parts() }
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Each row's characters.
fn text(frame: &Frame) -> Vec<String> {
    frame.rows.iter().map(|runs| runs.iter().map(|r| r.text.as_str()).collect()).collect()
}

/// Columns `a..b` of a row (every character here is one cell).
fn cols(row: &str, a: usize, b: usize) -> String {
    row.chars().skip(a).take(b - a).collect()
}

/// The column `needle` starts at in a row.
fn col(row: &str, needle: &str) -> usize {
    row[..row.find(needle).unwrap_or_else(|| panic!("{needle:?} in {row:?}"))].chars().count()
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

#[test]
fn the_bitmap_is_four_rows_of_half_blocks_with_the_cursor_its_own() {
    let cells = mark_cells();
    let rows: Vec<String> = cells
        .iter()
        .map(|r| {
            r.iter()
                .map(|c| match c {
                    MarkCell::Blank => ' ',
                    MarkCell::Letter(g) | MarkCell::Cursor(g) => *g,
                })
                .collect()
        })
        .collect();
    assert_eq!(
        rows,
        ["▄█▀▀█▄█ ▄█▀▀█▄█  ██", "██  ▄██ ██  ▄██  ██", " ▀▀▀ ██  ▀▀▀ ██  ▀▀", "▀█▄▄█▀  ▀█▄▄█▀     "]
    );
    assert_eq!((cells.len(), cells[0].len()), (MARK_H as usize, MARK_W as usize));
    // The cursor is 2 wide and 5 pixels tall: two full rows and a top half.
    assert_eq!(cells[0][17], MarkCell::Cursor('█'));
    assert_eq!(cells[2][18], MarkCell::Cursor('▀'));
    assert_eq!(cells[3][17], MarkCell::Blank);
    assert_eq!(cells[1][0], MarkCell::Letter('█'));
    assert_eq!(half_block(false, true), '▄');
}

#[test]
fn the_screen_is_the_mark_a_blank_row_the_parts_and_the_tagline_centred_on_the_base() {
    let t = truecolour();
    let f = frame(&t, &scene(Style::Subtle, 700), 80, 24).unwrap();
    let rows = text(&f);
    assert_eq!(rows.len(), 24);
    assert!(rows.iter().all(|r| crate::ui::width(r) == 80), "every row runs the full width");
    // 7 rows used, (24 - 7) / 2 = 8 above.
    assert_eq!(cols(&rows[8], 30, 49), "▄█▀▀█▄█ ▄█▀▀█▄█    ", "the cursor is off at 700 ms (second half of its blink)");
    assert_eq!(rows[12].trim(), "");
    assert_eq!(rows[13].trim(), "Shelf · Ledger · Board · Chat · Code");
    assert_eq!(rows[14].trim(), TAGLINE);
    let names = &rows[13];
    assert_eq!(col(names, "Shelf"), (80 - crate::ui::width(rows[13].trim())) / 2);
    let base = fg(&t, Slot::BgBase);
    assert!(f.rows.iter().flatten().all(|r| r.bg == base), "everything on bg.base");
    assert_eq!(run_at(&f, 2, 14).fg, None, "a blank cell has no ink");
    let tag_x = col(&rows[14], "one");
    assert_eq!(run_at(&f, tag_x, 14).fg, fg(&t, Slot::TextSecondary));
    let sep = col(names, "·");
    assert_eq!(run_at(&f, sep, 13).fg, fg(&t, Slot::TextMuted));
}

#[test]
fn on_truecolour_the_letters_ramp_purple_to_tertiary_to_cyan_from_the_theme_slots() {
    let t = truecolour();
    let f = frame(&t, &scene(Style::Subtle, 0), 80, 24).unwrap();
    // The mark starts at column 30, 8 rows down. The ramp runs over the letters, mark columns 0 to 14.
    assert_eq!(run_at(&f, 30, 9).fg, fg(&t, Slot::GlowPurple), "the first letter column");
    assert_eq!(run_at(&f, 44, 8).fg, fg(&t, Slot::GlowCyan), "the last letter column");
    assert_eq!(
        run_at(&f, 39, 9).fg,
        mix(fg(&t, Slot::AccentTertiary), fg(&t, Slot::GlowCyan), (9.0 / 14.0) * 2.0 - 1.0),
        "between, through accent.tertiary"
    );
    assert_eq!(run_at(&f, 47, 8).fg, fg(&t, Slot::TextPrimary), "the cursor in text.primary, on at 0 ms");
}

#[test]
fn below_truecolour_the_mark_is_accent_primary_and_with_no_colour_it_is_bold() {
    let t = theme("256", Tier::Ansi256);
    let f = frame(&t, &scene(Style::Static, 0), 80, 24).unwrap();
    let primary = fg(&t, Slot::AccentPrimary);
    for x in [30, 39, 44] {
        assert_eq!(run_at(&f, x, 9).fg, primary, "column {x}");
    }
    let t = theme("none", Tier::Mono);
    let f = frame(&t, &scene(Style::Static, 0), 80, 24).unwrap();
    let r = run_at(&f, 30, 9);
    assert_eq!((r.fg, r.bold), (None, true));
    // Lit names are bold too; one not ready is not.
    let names = text(&f)[13].clone();
    assert!(run_at(&f, col(&names, "Shelf"), 13).bold);
    assert!(!run_at(&f, col(&names, "Board"), 13).bold);
}

#[test]
fn the_assemble_lights_columns_left_to_right_then_the_tagline_reads_in() {
    let t = truecolour();
    let base = fg(&t, Slot::BgBase);
    let at = |ms| frame(&t, &scene(Style::Full, ms), 80, 24).unwrap();
    let start = at(0);
    assert!(text(&start).iter().all(|r| r.trim().is_empty()), "nothing lit at 0 ms");
    let mid = at(120);
    let row = &text(&mid)[9];
    assert_eq!(cols(row, 30, 32), "██", "the left columns are lit");
    assert_eq!(cols(row, 44, 80).trim(), "", "the right ones not yet");
    // The leading edge is soft: the last lit column is nearer the base than the first.
    let lit: Vec<usize> = (30..49).filter(|&x| run_at(&mid, x, 9).fg.is_some()).collect();
    let edge = run_at(&mid, *lit.last().unwrap(), 9).fg;
    assert_ne!(edge, fg(&t, Slot::GlowPurple));
    assert_ne!(edge, base);
    assert_eq!(text(&mid)[13].trim(), "", "the tagline waits for the mark");
    let done = at(ASSEMBLE_MS);
    assert_eq!(cols(&text(&done)[8], 30, 49), "▄█▀▀█▄█ ▄█▀▀█▄█  ██");
    let reading = at(ASSEMBLE_MS + TAGLINE_MS / 2);
    let names = text(&reading)[13].clone();
    assert!(names.contains("Shelf") && !names.contains("Code "), "half read in: {names:?}");
    let read = at(ASSEMBLE_MS + TAGLINE_MS);
    assert_eq!(text(&read)[14].trim(), TAGLINE);
}

#[test]
fn each_name_lights_to_its_colour_when_its_part_is_ready_and_its_turn_comes() {
    let t = truecolour();
    let muted = fg(&t, Slot::TextMuted);
    let ink = |f: &Frame, name: &str| run_at(f, col(&text(f)[13], name), 13).fg;
    let early = frame(&t, &scene(Style::Subtle, 0), 80, 24).unwrap();
    assert_eq!(ink(&early, "Shelf"), mix(muted, fg(&t, Slot::GlowPurple), 0.0), "lighting starts at 0 ms");
    assert_eq!(ink(&early, "Ledger"), muted, "its turn has not come");
    let late = frame(&t, &scene(Style::Subtle, 1000), 80, 24).unwrap();
    assert_eq!(ink(&late, "Shelf"), fg(&t, Slot::GlowPurple), "the first name takes the ramp's start");
    assert_eq!(ink(&late, "Code "), fg(&t, Slot::GlowCyan), "the last its end");
    assert_eq!(ink(&late, "Board"), muted, "not ready: stays muted");
    let mid = frame(&t, &scene(Style::Subtle, 140), 80, 24).unwrap();
    assert_eq!(ink(&mid, "Chat"), muted, "its turn is at 150 ms");
}

#[test]
fn full_holds_until_the_assemble_is_done_then_hands_off_once() {
    let mut s = Splash::new(Setting::Auto, Level::Full);
    let parts = vec![("Shelf".to_string(), true)];
    assert!(matches!(s.step(ms(1000), true, &parts), Step::Show(Scene { at: 0, .. })));
    assert!(matches!(s.step(ms(1300), true, &parts), Step::Show(Scene { at: 300, .. })));
    assert_eq!(s.next_frame(), Some(FRAME));
    assert_eq!(s.step(ms(1000 + HOLD_FULL_MS as u64), true, &parts), Step::HandOff);
    assert_eq!(s.step(ms(2000), true, &parts), Step::Hidden);
    assert_eq!(s.next_frame(), None, "no frame tick once it is gone");
}

const _: () = assert!(HOLD_FULL_MS <= 700, "the splash hands off within ~700 ms");

#[test]
fn a_slow_start_keeps_the_splash_until_the_first_screen_is_ready() {
    let mut s = Splash::new(Setting::Auto, Level::Full);
    assert!(matches!(s.step(ms(0), false, &[]), Step::Show(_)));
    assert!(matches!(s.step(ms(2000), false, &[]), Step::Show(_)));
    assert_eq!(s.step(ms(2030), true, &[]), Step::HandOff);
}

#[test]
fn subtle_shows_the_mark_at_once_and_hands_off_sooner() {
    let mut s = Splash::new(Setting::Auto, Level::Subtle);
    let Step::Show(scene) = s.step(ms(0), true, &[]) else { panic!("shown") };
    assert_eq!(scene.style, Style::Subtle);
    let f = frame(&truecolour(), &scene, 80, 24).unwrap();
    assert_eq!(cols(&text(&f)[8], 30, 49), "▄█▀▀█▄█ ▄█▀▀█▄█  ██", "no sweep: whole at once");
    assert!(matches!(s.step(ms(HOLD_SUBTLE_MS as u64 - 1), true, &[]), Step::Show(_)));
    assert_eq!(s.step(ms(HOLD_SUBTLE_MS as u64), true, &[]), Step::HandOff);
}

#[test]
fn with_motion_off_it_shows_only_a_static_mark_when_startup_is_slow() {
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
fn a_part_ready_later_is_lit_from_when_it_became_ready() {
    let mut s = Splash::new(Setting::Auto, Level::Subtle);
    let not = vec![("Board".to_string(), false)];
    let yes = vec![("Board".to_string(), true)];
    s.step(ms(0), true, &not);
    let Step::Show(scene) = s.step(ms(90), true, &yes) else { panic!() };
    assert_eq!(scene.parts[0].ready_at, Some(90));
    let Step::Show(scene) = s.step(ms(200), true, &yes) else { panic!() };
    assert_eq!(scene.parts[0].ready_at, Some(90), "kept from the first time");
}

#[test]
fn narrow_terminals_show_only_the_text_and_below_twenty_columns_nothing() {
    let t = truecolour();
    assert!(frame(&t, &scene(Style::Subtle, 1000), MIN_WIDTH - 1, 24).is_none());
    assert!(frame(&t, &scene(Style::Subtle, 1000), 80, 1).is_none());
    let short = frame(&t, &scene(Style::Subtle, 1000), 80, 6).unwrap();
    let rows = text(&short);
    assert!(!rows.iter().any(|r| r.contains('█')), "no room for the mark: text only");
    assert_eq!(rows[2].trim(), "Shelf · Ledger · Board · Chat · Code");
    assert_eq!(rows[3].trim(), TAGLINE);
    let narrow = frame(&t, &scene(Style::Subtle, 1000), 20, 7).unwrap();
    assert!(text(&narrow)[0].contains("▄█▀▀█▄█"), "19 columns of mark fit in 20");
    assert!(text(&narrow).iter().all(|r| crate::ui::width(r) == 20), "the names are cut at the edge");
}

#[test]
fn ratatui_paints_the_frame_and_ink_is_sent_the_same_cells() {
    let t = truecolour();
    let f = frame(&t, &scene(Style::Full, 300), 80, 24).unwrap();
    let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
    draw(&f, &mut buf);
    let v = view(&f);
    assert_eq!((v.width, v.height, v.rows.len()), (80, 24, 24));
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
    for (name, style, at, w, h) in [
        ("full-200-80x24", Style::Full, 200, 80, 24),
        ("full-530-160x40", Style::Full, 530, 160, 40),
        ("subtle-300-80x24", Style::Subtle, 300, 80, 24),
        ("subtle-300-160x40", Style::Subtle, 300, 160, 40),
    ] {
        let f = frame(&t, &scene(style, at), w, h).unwrap();
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
