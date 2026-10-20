//! Ratatui's motion effects as cells, at the start, the middle and the end
//! of each effect, as fixtures for Ink's motion engine (T29).
//!
//! RADAR_MOTION_CELLS_DIR=$PWD/ink/test/fixtures/motion CARGO_TARGET_DIR=target/fleet cargo nextest run -p radar --test motion_cells
//!
//! Without RADAR_MOTION_CELLS_DIR the committed files in ink/test/fixtures/motion
//! are compared and drift fails the test.
//!
//! The demo host or gallery is drawn at 80x24 (true colour,
//! radar's default theme), then `radar_core::motion::Engine` runs over the frame
//! with a `ManualClock`: the cue is raised, and the same frame is drawn at
//! the start (t=0), the middle and the end of the effect. One file per
//! effect, level and moment: `<effect>-<level>-<t>.json`, `<t>` being
//! `start`, `mid` or `end`; `off.json` shows that nothing changes.
//!
//! File format (`format: 1`):
//!
//! ```text
//! { "format": 1, "effect": "screen", "level": "full", "moment": "mid",
//!   "t_ms": 100, "duration_ms": 200, "width": 80, "height": 24,
//!   "cue": { "type": "screen", "region": "body" } | null,   // as the bridge sends it
//!   "motion": { "level": ..., "timings": {...}, "slots": {...} },
//!   "marks": { "body": [x,y,w,h], "popup": ..., "message": ..., "rows": {key: rect}, "loading": [rect] },
//!   "before": ROWS,   // the frame before the effect: what radar drew, no engine
//!   "after": ROWS,    // the scene of the cue drawn with no effect: what stays once it is done
//!   "settle": ROWS,   // the frame the runner draws one FRAME after the last (plain: == after)
//!   "settle_follows": bool, // the engine said running() (a plain frame is still to come) after the end
//!   "rows": ROWS }    // the frame after the engine ran at t_ms
//! ```
//!
//! ROWS is one array per screen row; a row is runs of cells with the same
//! style: `[text, fg, bg, mods]`, colours `#rrggbb` (or `reset`), `mods` a
//! `+`-joined list (`bold+dim`, empty for none). A run's text is the
//! cells' symbols left to right; the cell right of a two-column character
//! has an empty symbol, so a run's text can be shorter than its cells.
//! Marks are the places the shell marked in the frame (`motion::take`), which
//! the engine finds its targets by.

use std::path::PathBuf;

use radar_core::design::theme::Tier;
use radar_core::headless::{Headless, offered};
use radar_core::host::View;
use radar_core::ipc::{MotionCue, MotionLevel, MotionSlots, MotionTimings};
use radar_core::motion::{self, Changes, Cue, Engine, Level, ManualClock};
use radar_core::ratatui::Terminal;
use radar_core::ratatui::backend::TestBackend;
use radar_core::ratatui::buffer::Buffer;
use radar_core::ratatui::layout::Rect;
use radar_core::ratatui::style::{Color, Modifier};
use radar_core::shell::render;
use radar_core::theme::Theme;
use radar_core::ui::motion_demo::MotionDemo;
use radar_core::view::Intent;
use serde_json::{Value, json};

const W: u16 = 80;
const H: u16 = 24;

fn demo() -> Headless {
    let mut host = radar_core::shot::demo_host(radar::parts()).unwrap();
    host.view = View::Screen(0);
    Headless::new(host, W, H)
}

fn theme_of(head: &Headless) -> Theme {
    head.host().build_theme(Tier::TrueColor)
}

/// The host or gallery drawn once; the marks it made come with it.
fn paint(scene: &Scene, theme: &Theme) -> (Buffer, motion::Marks) {
    let mut term = Terminal::new(TestBackend::new(W, H)).expect("test backend");
    term.draw(|f| match scene {
        Scene::Host(head) => render(head.host(), theme, f),
        Scene::Gallery(demo) => {
            motion::begin();
            let area = f.area();
            demo.render(theme, area, f.buffer_mut());
        }
    })
    .expect("draw");
    let marks = motion::take();
    let buf = term.backend().buffer().clone();
    // The engine takes the marks itself when it draws: put them back.
    restore(&marks);
    (buf, marks)
}

/// The marks of a frame made again, so the engine finds them.
fn restore(marks: &motion::Marks) {
    motion::begin();
    for (region, rect) in [
        (motion::Region::Body, marks.body),
        (motion::Region::Popup, marks.popup),
        (motion::Region::Message, marks.message),
    ] {
        if let Some(rect) = rect {
            motion::region(region, rect);
        }
    }
    for (key, rect) in &marks.rows {
        motion::row(key.clone(), *rect);
    }
    for rect in &marks.loading {
        motion::loading(*rect);
    }
}

fn hex(c: Color) -> String {
    match c {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        Color::Reset => "reset".into(),
        other => format!("{other:?}").to_lowercase(),
    }
}

const MODS: [(Modifier, &str); 9] = [
    (Modifier::BOLD, "bold"),
    (Modifier::DIM, "dim"),
    (Modifier::ITALIC, "italic"),
    (Modifier::UNDERLINED, "underline"),
    (Modifier::SLOW_BLINK, "slow_blink"),
    (Modifier::RAPID_BLINK, "rapid_blink"),
    (Modifier::REVERSED, "reversed"),
    (Modifier::HIDDEN, "hidden"),
    (Modifier::CROSSED_OUT, "strike"),
];

fn rows_of(buf: &Buffer) -> Value {
    let a = buf.area;
    let mut rows = Vec::new();
    for y in a.y..a.y + a.height {
        let mut runs: Vec<(String, String, String, String)> = Vec::new();
        let mut covered = false;
        for x in a.x..a.x + a.width {
            let cell = &buf[(x, y)];
            let ch = if covered { "" } else { cell.symbol() };
            covered = !covered && radar_core::ui::width(cell.symbol()) > 1;
            let mods: Vec<&str> =
                MODS.iter().filter(|(m, _)| cell.modifier.contains(*m)).map(|(_, w)| *w).collect();
            let style = (hex(cell.fg), hex(cell.bg), mods.join("+"));
            match runs.last_mut() {
                Some(run) if (&run.1, &run.2, &run.3) == (&style.0, &style.1, &style.2) => run.0.push_str(ch),
                _ => runs.push((ch.to_string(), style.0, style.1, style.2)),
            }
        }
        rows.push(Value::Array(runs.into_iter().map(|(t, f, b, m)| json!([t, f, b, m])).collect()));
    }
    Value::Array(rows)
}

fn rect(r: Rect) -> Value {
    json!([r.x, r.y, r.width, r.height])
}

fn marks_json(m: &motion::Marks) -> Value {
    let rows: serde_json::Map<String, Value> = m.rows.iter().map(|(k, r)| (k.clone(), rect(*r))).collect();
    json!({
        "body": m.body.map(rect),
        "popup": m.popup.map(rect),
        "message": m.message.map(rect),
        "rows": rows,
        "loading": m.loading.iter().map(|r| rect(*r)).collect::<Vec<_>>(),
    })
}

/// The cue as the bridge sends it (`Bridge::gather_cues`).
fn bridge_cue(cue: &Cue) -> MotionCue {
    use radar_core::ipc::MotionRegion as R;
    match cue {
        Cue::Screen => MotionCue::Screen { region: R::Body },
        Cue::Page => MotionCue::Page { region: R::Body },
        Cue::Popup => MotionCue::Popup { region: R::Popup },
        Cue::Message => MotionCue::Message { region: R::Message },
        Cue::MessageGone => MotionCue::MessageGone { region: R::Message },
        Cue::Changed(c) => MotionCue::Changed {
            changed: c.changed.iter().cloned().collect(),
            added: c.added.iter().cloned().collect(),
        },
    }
}

/// What is drawn: the demo host, or the gallery's sample messages and waiting
/// panels. Host notices are composited after motion, outside its retained marks.
enum Scene {
    Host(Box<Headless>),
    Gallery(MotionDemo),
}

impl From<Headless> for Scene {
    fn from(h: Headless) -> Self {
        Scene::Host(Box::new(h))
    }
}

/// What one effect needs: the host before it, the host after the cue, and
/// whether the engine sees the frame before first (the message that leaves
/// is remembered from the frame it was drawn in).
struct Case {
    effect: &'static str,
    level: Level,
    cue: Option<Cue>,
    duration_ms: u64,
    before: Scene,
    after: Scene,
    prime: bool,
}

/// The files of one case: `(file name, text)`.
fn run(case: Case) -> Vec<(String, String)> {
    let Case { effect, level, cue, duration_ms, before, after, prime } = case;
    let theme = theme_of(&demo());
    let clock = ManualClock::default();
    let mut engine = Engine::new(level, Box::new(clock.clone()));
    let (before_buf, before_marks) = paint(&before, &theme);
    if matches!(cue.as_ref(), Some(Cue::MessageGone)) {
        assert!(before_marks.message.is_some_and(|area| !area.is_empty()),
            "the departing message must be drawn and marked before priming");
    }
    if prime {
        engine.draw(&mut before_buf.clone(), &theme);
    } else {
        motion::take();
    }
    let sent = cue.as_ref().filter(|c| c.runs_at(level)).map(bridge_cue);
    if let Some(cue) = cue {
        engine.cue(cue, &theme);
    }
    let level_word = level.word();
    let moments: Vec<(&str, u64)> = if level == Level::Off {
        vec![("", 0)]
    } else {
        vec![("start", 0), ("mid", duration_ms / 2), ("end", duration_ms)]
    };
    let motion_view = json!({
        "level": serde_json::to_value(MotionLevel::from(level)).unwrap(),
        "timings": serde_json::to_value(MotionTimings::default()).unwrap(),
        "slots": serde_json::to_value(MotionSlots::default()).unwrap(),
    });
    let mut at = 0;
    let mut frames = Vec::new();
    for (moment, t) in moments {
        clock.advance(t - at);
        at = t;
        let (mut buf, marks) = paint(&after, &theme);
        if effect == "message" {
            assert!(marks.message.is_some_and(|area| !area.is_empty()),
                "the arriving message must be drawn and marked for motion");
        }
        let plain = (effect == "message_gone").then(|| buf.clone());
        engine.draw(&mut buf, &theme);
        if let Some(plain) = plain {
            let message = before_marks.message.expect("primed message mark");
            for position in buf.area.positions().filter(|position| !message.contains(*position)) {
                assert_eq!(buf[position], plain[position],
                    "the departing message cannot replace fresh content outside its mark");
            }
        }
        frames.push((moment, t, rows_of(&buf), marks_json(&marks)));
    }
    // `after`: the scene drawn with no effect, what radar shows once the effect is
    // done. `settle`: the frame the runner draws next, one FRAME after the
    // last (the engine reports `running()` for one plain frame after the last
    // effect ends, motion.rs `settle`, set at the end of `Engine::draw`).
    let (after_buf, _) = paint(&after, &theme);
    motion::take();
    let after_rows = rows_of(&after_buf);
    let (settle_rows, settle_running) = {
        clock.advance(motion::FRAME.as_millis() as u64);
        let was_running = engine.running();
        let (mut buf, _) = paint(&after, &theme);
        engine.draw(&mut buf, &theme);
        (rows_of(&buf), was_running)
    };
    let mut files = Vec::new();
    for (moment, t, rows, marks) in frames {
        let doc = json!({
            "format": 1,
            "effect": effect,
            "level": level_word,
            "moment": if moment.is_empty() { "none" } else { moment },
            "t_ms": t,
            "duration_ms": duration_ms,
            "width": W,
            "height": H,
            "cue": sent,
            "motion": motion_view,
            "marks": marks,
            "before": rows_of(&before_buf),
            "after": after_rows,
            "settle": settle_rows,
            "settle_follows": settle_running,
            "rows": rows,
        });
        let name = if moment.is_empty() {
            format!("{effect}.json")
        } else {
            format!("{effect}-{level_word}-{moment}.json")
        };
        files.push((name, format!("{}\n", serde_json::to_string(&doc).unwrap())));
    }
    files
}

/// A host with a menu or confirm open, found as `ratatui_cells` finds one.
fn with_menu() -> Headless {
    let mut head = demo();
    for i in 0..head.screen_count() {
        head.show_screen(i);
        let Some(view) = head.screen() else { continue };
        for o in offered(&view) {
            head.focus(o.pane);
            head.send(Intent::Select { target: o.target.clone() });
            head.send(Intent::Context { target: o.target.clone() });
            if head.menu().is_some() {
                return head;
            }
            head.send(Intent::Activate { target: o.target });
            if head.menu().is_some() {
                return head;
            }
            head.close_overlays();
        }
    }
    panic!("no screen of the demo host opens a menu");
}

fn with_message() -> MotionDemo {
    // Exercise the live gallery's message primitive. Host notices no longer
    // enter the retained motion frame, so setting host.message has no target.
    let mut demo = MotionDemo::default();
    let key = radar_core::ratatui::crossterm::event::KeyEvent::from(radar_core::ratatui::crossterm::event::KeyCode::Char('t'));
    assert!(demo.key(&key));
    assert_eq!(demo.take_cues(), [Cue::Message]);
    demo
}

fn after_message() -> MotionDemo {
    let mut demo = MotionDemo::default();
    // The old host scene revealed longer status hints when its message left.
    // Use a real new gallery row to keep fresh content outside the fading mark;
    // the whole before frame must never be restored over this replacement.
    let key = radar_core::ratatui::crossterm::event::KeyEvent::from(radar_core::ratatui::crossterm::event::KeyCode::Char('n'));
    assert!(demo.key(&key));
    demo
}

/// The gallery's sample panels with the "waiting" panel on.
fn waiting() -> MotionDemo {
    let mut demo = MotionDemo::default();
    let key = radar_core::ratatui::crossterm::event::KeyEvent::from(radar_core::ratatui::crossterm::event::KeyCode::Char('l'));
    assert!(demo.key(&key));
    demo
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();
    let plain = |effect, level, cue: Cue, ms| Case {
        effect,
        level,
        cue: Some(cue),
        duration_ms: ms,
        before: demo().into(),
        after: demo().into(),
        prime: false,
    };
    // At full a screen assembles from its panels; subtle keeps the fade.
    out.push(plain("screen", Level::Full, Cue::Screen, motion::assemble::ASSEMBLE_MS as u64));
    out.push(plain("screen", Level::Subtle, Cue::Screen, motion::SCREEN_MS as u64));
    out.push(plain("page", Level::Full, Cue::Page, motion::PAGE_MS as u64));
    for level in [Level::Full, Level::Subtle] {
        let mut before = with_menu();
        before.close_overlays();
        out.push(Case {
            effect: "popup",
            level,
            cue: Some(Cue::Popup),
            duration_ms: motion::POPUP_MS as u64,
            before: before.into(),
            after: with_menu().into(),
            prime: false,
        });
    }
    out.push(Case {
        effect: "message",
        level: Level::Full,
        cue: Some(Cue::Message),
        duration_ms: motion::MESSAGE_MS as u64,
        before: Scene::Gallery(MotionDemo::default()),
        after: Scene::Gallery(with_message()),
        prime: false,
    });
    out.push(Case {
        effect: "message_gone",
        level: Level::Full,
        cue: Some(Cue::MessageGone),
        duration_ms: motion::MESSAGE_GONE_MS as u64,
        before: Scene::Gallery(with_message()),
        after: Scene::Gallery(after_message()),
        prime: true,
    });
    // Changed rows: the first row the frame marks is changed, the second added.
    let head: Scene = demo().into();
    let theme = theme_of(&demo());
    let (_, marks) = paint(&head, &theme);
    motion::take();
    assert!(marks.rows.len() >= 2, "the demo screen marks rows for motion");
    let mut changes = Changes::default();
    changes.changed.insert(marks.rows[0].0.clone());
    changes.added.insert(marks.rows[1].0.clone());
    for level in [Level::Full, Level::Subtle] {
        out.push(plain("changed", level, Cue::Changed(changes.clone()), motion::GLOW_MS as u64));
    }
    // Loading: no cue; the shimmer runs while a panel is marked as waiting.
    out.push(Case {
        effect: "loading",
        level: Level::Full,
        cue: None,
        duration_ms: motion::SHIMMER_MS as u64,
        before: Scene::Gallery(waiting()),
        after: Scene::Gallery(waiting()),
        prime: false,
    });
    // Off: nothing changes.
    out.push(plain("off", Level::Off, Cue::Screen, 0));
    out
}

fn committed() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ink/test/fixtures/motion")
}

/// Each effect changes the frame between its start and its end; off changes
/// nothing. Checked on the files about to be written or compared.
fn check_moves(files: &[(String, String)]) {
    let doc = |n: &str| -> Value {
        let text = &files.iter().find(|(name, _)| name == n).unwrap_or_else(|| panic!("{n}")).1;
        serde_json::from_str(text).unwrap()
    };
    let off = doc("off.json");
    assert_eq!(off["rows"], off["before"], "off changes nothing");
    assert_eq!(off["cue"], Value::Null);
    for (name, _) in files.iter().filter(|(n, _)| n.ends_with("-start.json")) {
        let stem = name.trim_end_matches("-start.json");
        let (start, mid, end) = (doc(name), doc(&format!("{stem}-mid.json")), doc(&format!("{stem}-end.json")));
        if stem.starts_with("loading") {
            assert_ne!(start["rows"], mid["rows"], "the shimmer moves");
            continue;
        }
        assert_eq!(end["settle"], end["after"], "{stem}: the frame after the end is plain, as radar drew it");
        assert_eq!(end["settle_follows"], true, "{stem}: the runner is told one more frame follows");
        assert_ne!(start["rows"], start["before"], "{stem} shows something at its start");
        assert_ne!(start["rows"], end["rows"], "{stem} moves");
    }
}

#[test]
fn motion_cells_match_the_committed_fixtures() {
    let files: Vec<(String, String)> = cases().into_iter().flat_map(run).collect();
    check_moves(&files);
    if let Ok(dir) = std::env::var("RADAR_MOTION_CELLS_DIR") {
        let dir = PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in &files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        return;
    }
    let mut drift = Vec::new();
    for (name, text) in &files {
        match std::fs::read_to_string(committed().join(name)) {
            Ok(old) if &old == text => {}
            Ok(_) => drift.push(format!("{name} differs")),
            Err(_) => drift.push(format!("{name} is missing")),
        }
    }
    assert!(drift.is_empty(), "rewrite with RADAR_MOTION_CELLS_DIR=$PWD/ink/test/fixtures/motion: {drift:?}");
}
