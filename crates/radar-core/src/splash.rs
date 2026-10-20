//! The startup splash: a radar scope that sweeps once over the parts in this
//! build, the `radar` wordmark painted in under it, the parts' names, and the
//! start line; then it hands off to the first screen.
//!
//! Everything both renderers draw is worked out here, as cells: [`frame`]
//! turns a [`Scene`] (what to show, how far into it) into rows of runs with
//! their colours, which the Ratatui runner paints over its buffer
//! ([`draw`]) and the bridge sends to Ink ([`view`]), so the two draw the
//! same cells. The timing lives in [`Splash`], which each runner keeps:
//! when it starts, when it hands off, and that any input ends it at once.
//!
//! The design is the first-run canvas's sweep boards (the live sweep and its
//! key frames). The scope is 64 × 24 cells of braille, 2 × 4 dots a cell and
//! one colour a cell, laid out as if each cell were 10 × 25 pixels: what the
//! beam crosses lights on one phosphor ramp and decays by the time since the
//! beam passed its bearing, so a dropped frame costs nothing.
//!
//! - `[theme] splash = "auto" | "off"`: `off` never shows it.
//! - Motion `full`, in ms: the scope opens 0–300; the beam makes one turn
//!   200–1400, and each part's contact locks as the beam reaches its bearing
//!   (brackets close in 200, its label types at 14 a letter); the name paints
//!   in 1300–1750; the parts' names 1620–1920; the start line types from
//!   1800 at 12 a cell; the first screen fades in from 2300 (the screen cue).
//! - `subtle`: no beam; the scope, contacts, mark and line fade in.
//! - `off` (or motion held back: over SSH, below full colour, a slow
//!   terminal): nothing moves; the last frame shows only while the first
//!   screen is not ready after [`STATIC_AFTER_MS`].
//! - A rare echo on the scope ([`Echo`]) or a special event ([`Event`], motion
//!   full only) comes with the start ([`Start`]); an event takes the mark's
//!   and the names' place and holds the splash [`EVENT_HOLD_MS`] longer.
//! - Too small for the scope: the mark, the names and the line, from where
//!   the beam would have finished; too small for the mark, the two lines.
//! - A key, click or paste ends it at once and acts on the screen under it.

use std::f64::consts::{PI, TAU};
use std::time::Duration;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style as Paint};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use unicode_width::UnicodeWidthChar;

use crate::design::theme::{Slot, Tier};
use crate::motion::Level;
use crate::theme::Theme;

/// The wordmark, 8 pixels tall: `#` a lit pixel. Two pixel rows make one
/// cell row (`▀` `▄`), so it is 4 rows tall.
pub const BITMAP: [&str; 8] = [
    "................#............",
    "................#............",
    "#.###..###...####..###..#.###",
    "##........#.#...#.....#.##...",
    "#......####.#...#..####.#....",
    "#.....#...#.#...#.#...#.#....",
    "#......####..####..####.#....",
    ".............................",
];

/// The mark's size in cells.
pub const MARK_W: u16 = 29;
pub const MARK_H: u16 = 4;
/// The scope's size in cells.
pub const SCOPE_W: u16 = 64;
pub const SCOPE_H: u16 = 24;
/// Everything with the scope: the scope, its readout, a blank row, the mark,
/// a blank row, the names, a blank row, the line.
pub const FULL_HEIGHT: u16 = 34;
/// Without the scope: the mark, a blank row, the names, a blank row, the line.
const COMPACT_HEIGHT: u16 = MARK_H + 4;
/// Between the parts' names.
pub const SEPARATOR: &str = " · ";
/// Narrower than this, no splash at all.
pub const MIN_WIDTH: u16 = 20;
/// The start line's most cells.
pub const LINE_MAX: usize = 56;

/// The scope opens (its rings draw out from the centre).
pub const OPEN_MS: u32 = 300;
/// The beam's one turn starts, and how long it takes.
pub const BEAM_FROM_MS: u32 = 200;
pub const TURN_MS: u32 = 1200;
/// The name paints in from here, a scan line left to right.
pub const PAINT_FROM_MS: u32 = 1300;
const PAINT_MS: u32 = 450;
/// The parts' names fade in from here.
pub const NAMES_FROM_MS: u32 = 1620;
/// The start line types from here, done by [`LINE_FROM_MS`] + 500.
pub const LINE_FROM_MS: u32 = 1800;
const TYPE_MS: u32 = 500;
/// The fire starts burning here.
const FIRE_FROM_MS: u32 = 1000;
/// The splash hands off once the first screen is ready and this long has
/// passed.
pub const HOLD_FULL_MS: u32 = 2300;
pub const HOLD_SUBTLE_MS: u32 = 1600;
/// A special event holds the splash this much longer.
pub const EVENT_HOLD_MS: u32 = 1500;
/// With no motion, the last frame shows only when the first screen is not
/// ready after this long.
pub const STATIC_AFTER_MS: u32 = 150;
/// How often to draw while the splash shows.
pub const FRAME: Duration = crate::motion::FRAME;

/// `[theme] splash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Setting {
    /// Shown at start, as motion allows.
    #[default]
    Auto,
    /// Never shown.
    Off,
}

impl Setting {
    pub const ALL: [Setting; 2] = [Setting::Auto, Setting::Off];

    /// The config word; anything unknown is `auto`.
    pub fn parse(word: &str) -> Self {
        match word.trim() {
            "off" | "none" | "no" => Self::Off,
            _ => Self::Auto,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
        }
    }

    /// What it does, in a few words.
    pub fn about(self) -> &'static str {
        match self {
            Self::Auto => "the radar sweep while radar starts, moving as motion allows",
            Self::Off => "straight to the first screen",
        }
    }
}

/// How the splash moves, from the motion level that runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The scope opens, the beam turns, the name paints, the line types.
    Full,
    /// No beam: everything fades in.
    Subtle,
    /// Nothing moves: the last frame, shown only when startup takes time.
    Static,
}

impl Style {
    pub fn of(level: Level) -> Self {
        match level {
            Level::Full => Self::Full,
            Level::Subtle => Self::Subtle,
            Level::Off => Self::Static,
        }
    }
}

/// A part on the scope: a contact at its own bearing, and its name under the
/// mark.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub name: String,
    /// The service's mark as the icon pack draws it; empty with icons off.
    pub mark: String,
    /// What it holds for you, after its name (`3 for you`).
    pub tail: Option<String>,
    /// How many things that is, for the readout's total.
    pub count: u32,
    /// It answers: a part that is failing or signed out shows `no signal`.
    pub live: bool,
}

/// The big return the beam paints this start: a plane most starts, now and
/// then something else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Echo {
    #[default]
    Plane,
    Starship,
    Battlestar,
    Mech,
}

impl Echo {
    /// From a roll in `0..1`: each rare echo is 1 in 50.
    pub fn roll(r: f64) -> Self {
        match r {
            r if r < 0.02 => Self::Starship,
            r if r < 0.04 => Self::Battlestar,
            r if r < 0.06 => Self::Mech,
            _ => Self::Plane,
        }
    }

    /// Not a plane: the readout says `1 unidentified`, and the start line
    /// says so too.
    pub fn rare(self) -> bool {
        self != Self::Plane
    }

    /// Its plan-view silhouette, in scope pixels before [`ECHO_SCALE`], nose
    /// toward +x.
    fn shape(self) -> Shape {
        use Piece::{E, P, X, Y};
        match self {
            Self::Plane => Shape {
                rot: -25.0,
                dy: 0.0,
                parts: &[
                    E(0.0, 0.0, 80.0, 9.0),
                    E(66.0, 0.0, 16.0, 6.0),
                    Y(&[(16.0, -5.0), (-20.0, -54.0), (-34.0, -54.0), (-10.0, -5.0)]),
                    Y(&[(-60.0, -3.0), (-72.0, -24.0), (-82.0, -24.0), (-74.0, -3.0)]),
                    E(-2.0, -27.0, 12.0, 5.0),
                    E(-2.0, 27.0, 12.0, 5.0),
                ],
            },
            Self::Starship => Shape {
                rot: -18.0,
                dy: 0.0,
                parts: &[
                    E(40.0, 0.0, 40.0, 40.0),
                    P(&[(14.0, -6.0), (-24.0, -7.0), (-24.0, 7.0), (14.0, 6.0)]),
                    E(-36.0, 0.0, 28.0, 11.0),
                    Y(&[(-28.0, -8.0), (-46.0, -32.0), (-56.0, -32.0), (-40.0, -8.0)]),
                    E(-62.0, -36.0, 36.0, 7.0),
                    E(-62.0, 36.0, 36.0, 7.0),
                ],
            },
            Self::Battlestar => Shape {
                rot: -12.0,
                dy: 0.0,
                parts: &[
                    P(&[
                        (86.0, -12.0),
                        (46.0, -20.0),
                        (-60.0, -15.0),
                        (-92.0, -22.0),
                        (-102.0, -10.0),
                        (-102.0, 10.0),
                        (-92.0, 22.0),
                        (-60.0, 15.0),
                        (46.0, 20.0),
                        (86.0, 12.0),
                    ]),
                    Y(&[(60.0, -28.0), (-46.0, -28.0), (-60.0, -42.0), (50.0, -42.0)]),
                    Y(&[(-8.0, -14.0), (12.0, -14.0), (12.0, -29.0), (-8.0, -29.0)]),
                ],
            },
            Self::Mech => Shape {
                rot: 0.0,
                dy: 20.0,
                parts: &[
                    P(&[(-10.0, -76.0), (10.0, -76.0), (10.0, -58.0), (-10.0, -58.0)]),
                    X(&[(3.0, -76.0), (8.0, -76.0), (20.0, -98.0), (15.0, -98.0)]),
                    P(&[(-30.0, -56.0), (30.0, -56.0), (22.0, -12.0), (-22.0, -12.0)]),
                    X(&[(30.0, -60.0), (50.0, -55.0), (52.0, -36.0), (32.0, -38.0)]),
                    X(&[(36.0, -38.0), (50.0, -38.0), (52.0, 2.0), (38.0, 4.0)]),
                    P(&[(-22.0, -12.0), (22.0, -12.0), (22.0, 0.0), (-22.0, 0.0)]),
                    X(&[(6.0, 0.0), (22.0, 0.0), (24.0, 46.0), (8.0, 46.0)]),
                    X(&[(3.0, 46.0), (28.0, 46.0), (32.0, 58.0), (0.0, 58.0)]),
                ],
            },
        }
    }
}

/// An echo's silhouette: pieces turned to a heading (`rot`, degrees) and
/// moved down by `dy`.
struct Shape {
    rot: f64,
    dy: f64,
    parts: &'static [Piece],
}

/// A piece of a silhouette.
enum Piece {
    /// An ellipse: centre and radii.
    E(f64, f64, f64, f64),
    /// A polygon.
    P(&'static [(f64, f64)]),
    /// A polygon and its mirror across the x axis.
    Y(&'static [(f64, f64)]),
    /// A polygon and its mirror across the y axis.
    X(&'static [(f64, f64)]),
}

/// Echo silhouettes are drawn at this scale, within this many pixels of
/// their centre.
const ECHO_SCALE: f64 = 0.72;
const ECHO_REACH: f64 = 80.0;

/// Even-odd ray casting.
fn in_polygon(pts: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let ((xi, yi), (xj, yj)) = (pts[i], pts[j]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl Shape {
    /// Whether the point `dx, dy` from the echo's centre is inside it.
    fn inside(&self, dx: f64, dy: f64) -> bool {
        let a = -self.rot * PI / 180.0;
        let x = (dx * a.cos() - dy * a.sin()) / ECHO_SCALE;
        let y = (dx * a.sin() + dy * a.cos()) / ECHO_SCALE - self.dy;
        self.parts.iter().any(|p| match p {
            Piece::E(cx, cy, rx, ry) => ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0,
            Piece::P(pts) => in_polygon(pts, x, y),
            Piece::Y(pts) => in_polygon(pts, x, y) || in_polygon(pts, x, -y),
            Piece::X(pts) => in_polygon(pts, x, y) || in_polygon(pts, -x, y),
        })
    }
}

/// A special event: its own animation in the mark's place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// "This is fine.": the PSX Doom fire.
    Fire,
}

/// A piece of the start line: words, or a key drawn in the key colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineSpan {
    pub text: String,
    pub key: bool,
}

/// What this start brings to the splash, rolled once when radar starts.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Start {
    pub echo: Echo,
    pub event: Option<Event>,
    /// The start line; empty for none.
    pub line: Vec<LineSpan>,
}

/// This start's echo, event and line: the echo rolled first, then the start
/// line ([`crate::start_lines`]) from the shipped and the user's dictionary,
/// `[start_lines]` in the config and the state file, which it saves. Facts
/// come from nothing fetched; a line that needs one the host has not handed
/// over cannot show yet.
pub fn roll_start(motion_full: bool, emoji_ok: bool, held: Vec<(String, crate::start_lines::Value)>) -> Start {
    use crate::start_lines::{self as lines, Rng as _};
    let mut rng = lines::SplitMix64::from_entropy();
    let echo = Echo::roll(rng.unit());
    let path = crate::runner::config_path();
    let first_run = !path.exists();
    let cfg: lines::UserConfig = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|table| table.get("start_lines").cloned())
        .and_then(|v| {
            v.try_into()
                .inspect_err(|why| crate::log::warn(format!("[start_lines] ignored: {why}")))
                .ok()
        })
        .unwrap_or_default();
    let dict = lines::load_dictionary(&lines::user_dictionary_path());
    let state_path = lines::state_path();
    let mut state = state_path.as_deref().map(lines::load_state).unwrap_or_default();
    let mut ctx = lines::Context::now();
    ctx.motion_full = motion_full;
    ctx.emoji_ok = emoji_ok;
    ctx.echo_rare = echo.rare();
    ctx.first_run = first_run || state.is_fresh();
    let mut facts = lines::Facts::new(ctx.epoch);
    for (name, value) in held {
        facts.set(&name, value);
    }
    let picked = lines::pick(&dict, &cfg, &mut state, &facts, &ctx, &mut rng);
    if let Some(p) = &state_path
        && let Err(why) = lines::save_state(p, &state)
    {
        crate::log::warn(format!("start lines: cannot save {}: {why}", p.display()));
    }
    let Some(picked) = picked else {
        return Start { echo, ..Start::default() };
    };
    Start {
        echo,
        event: (motion_full && picked.anim.as_deref() == Some("fire")).then_some(Event::Fire),
        line: picked
            .spans
            .into_iter()
            .map(|s| LineSpan { text: s.text, key: s.kind == lines::SpanKind::Key })
            .collect(),
    }
}

/// What one frame of the splash shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    pub style: Style,
    /// Ms since the splash started.
    pub at: u32,
    pub contacts: Vec<Contact>,
    pub start: Start,
}

/// What the runner does this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Draw the splash over the frame.
    Show(Scene),
    /// The splash ended on its own just now: draw the first screen and cue
    /// its fade in (`full` and `subtle`).
    HandOff,
    /// Nothing to draw (not yet, or no more).
    Hidden,
}

/// The splash's timing, kept by each runner.
#[derive(Debug)]
pub struct Splash {
    style: Style,
    /// When it started (the runner's first frame), on the runner's clock.
    started: Option<Duration>,
    /// It ended: handed off, skipped, or never to show.
    done: bool,
    start: Start,
    /// This start's roll, worked out on its own thread ([`Splash::roll`]).
    rolling: Option<std::sync::mpsc::Receiver<Start>>,
    /// How long it had shown at its last frame (ms).
    shown_ms: u32,
    /// It was skipped before [`EARLY_MS`]: its line goes to the status line
    /// ([`Splash::take_early_line`]).
    early: bool,
}

/// A splash skipped sooner than this hands its line to the status line.
pub const EARLY_MS: u32 = 1500;

impl Splash {
    pub fn new(setting: Setting, level: Level) -> Self {
        Self {
            style: Style::of(level),
            started: None,
            done: setting == Setting::Off,
            start: Start::default(),
            rolling: None,
            shown_ms: 0,
            early: false,
        }
    }

    /// The start line, once, when the splash was skipped before
    /// [`EARLY_MS`]: the words for the status line.
    pub fn take_early_line(&mut self) -> Option<String> {
        if !std::mem::take(&mut self.early) {
            return None;
        }
        self.take_roll();
        let words: String = self.start.line.iter().map(|s| s.text.as_str()).collect();
        (!words.trim().is_empty()).then_some(words)
    }

    /// Roll this start ([`roll_start`]) on a thread of its own, so the files
    /// it reads and writes never hold up the first frame or anything else;
    /// the frames take it up when it is there (the line types from
    /// [`LINE_FROM_MS`]). Nothing when the splash will not show.
    pub fn roll(&mut self, motion_full: bool, emoji_ok: bool, facts: Vec<(String, crate::start_lines::Value)>) {
        if self.done {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("start-line".into())
            .spawn(move || {
                // The scope's fixed geometry too, so no frame pays for it.
                let _ = geometry();
                let _ = tx.send(roll_start(motion_full, emoji_ok, facts));
            });
        if spawned.is_ok() {
            self.rolling = Some(rx);
        }
    }

    /// Take up the roll if it has come.
    fn take_roll(&mut self) {
        if let Some(rx) = &self.rolling {
            match rx.try_recv() {
                Ok(start) => {
                    self.start = start;
                    self.rolling = None;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.rolling = None,
            }
        }
    }

    pub fn style(&self) -> Style {
        self.style
    }

    /// What this start brings: the echo, an event, the line.
    pub fn set_start(&mut self, start: Start) {
        self.start = start;
    }

    pub fn start(&self) -> &Start {
        &self.start
    }

    /// Whether it still shows, or may.
    pub fn active(&self) -> bool {
        !self.done
    }

    /// The motion level changed before the splash ended (settings, a slow
    /// terminal): it follows.
    pub fn set_level(&mut self, level: Level) {
        self.style = Style::of(level);
    }

    /// The setting changed: `off` ends it.
    pub fn set_setting(&mut self, setting: Setting) {
        if setting == Setting::Off {
            self.done = true;
        }
    }

    /// A key, click or paste: it ends at once, with no fade. True when it
    /// was showing (or about to), so the caller draws again.
    pub fn input(&mut self) -> bool {
        let was = !self.done;
        if was && self.started.is_some() && self.style != Style::Static && self.shown_ms < EARLY_MS {
            self.early = true;
        }
        self.done = true;
        was
    }

    /// The runner's frame tick: `Some` while it shows or may.
    pub fn next_frame(&self) -> Option<Duration> {
        (!self.done).then_some(FRAME)
    }

    /// How long it holds before it hands off, at its style.
    pub fn hold(&self) -> u32 {
        match self.style {
            Style::Full => HOLD_FULL_MS + if self.start.event.is_some() { EVENT_HOLD_MS } else { 0 },
            Style::Subtle => HOLD_SUBTLE_MS,
            Style::Static => 0,
        }
    }

    /// The frame at `now` on the runner's clock: `screen_ready` when the
    /// first screen can be drawn, `contacts` the parts on the scope, in the
    /// order shown.
    pub fn step(&mut self, now: Duration, screen_ready: bool, contacts: &[Contact]) -> Step {
        if self.done {
            return Step::Hidden;
        }
        self.take_roll();
        let started = *self.started.get_or_insert(now);
        let at = now.saturating_sub(started).as_millis().min(u32::MAX as u128) as u32;
        self.shown_ms = at;
        let scene =|| Scene { style: self.style, at, contacts: contacts.to_vec(), start: self.start.clone() };
        match self.style {
            Style::Static => {
                if screen_ready {
                    self.done = true;
                    Step::Hidden
                } else if at >= STATIC_AFTER_MS {
                    Step::Show(scene())
                } else {
                    Step::Hidden
                }
            }
            Style::Full | Style::Subtle => {
                if screen_ready && at >= self.hold() {
                    self.done = true;
                    Step::HandOff
                } else {
                    Step::Show(scene())
                }
            }
        }
    }
}

/// One run of cells of one look.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub fg: Option<Color>,
    pub bg: Option<Color>,
    pub bold: bool,
}

/// A whole screen of the splash: every row, runs across its full width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: u16,
    pub height: u16,
    pub rows: Vec<Vec<Run>>,
    /// The scope's dots, each in its own colour, when the scope was drawn:
    /// what the pixel tier paints ([`picture`]).
    pub scope: Option<ScopeDots>,
}

/// The scope's dots as the frame lit them: its top-left cell, and every
/// dot's colour ([`DW`] × [`DH`], `None` where it is dark).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeDots {
    pub x: u16,
    pub y: u16,
    dots: Vec<Ink>,
}

/// The scope painted in pixels, for a terminal that shows pictures (kitty's
/// graphics): a PNG to lay over the scope's cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picture {
    /// Its top-left cell and its size in cells.
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
    pub png: Vec<u8>,
}

/// The kitty image id the splash's picture takes, replaced each frame.
pub const PICTURE_ID: u32 = 7370;

/// The scope of `frame` in pixels, a cell being `cell` pixels (wide, high):
/// each dot a soft round light in its own colour, with a faint glow, on the
/// base. `None` without a scope, or without true colour (the dots' colours
/// would not be its own).
pub fn picture(frame: &Frame, cell: (u16, u16)) -> Option<Picture> {
    let scope = frame.scope.as_ref()?;
    let (cw, ch) = (cell.0.clamp(4, 40) as f64, cell.1.clamp(8, 80) as f64);
    let rgb = |c: Ink| match c {
        Some(Color::Rgb(r, g, b)) => Some([r as f64, g as f64, b as f64]),
        _ => None,
    };
    let base = rgb(frame.rows.first().and_then(|r| r.first()).and_then(|r| r.bg)).unwrap_or([0.0; 3]);
    let (w, h) = ((SCOPE_W as f64 * cw) as usize, (SCOPE_H as f64 * ch) as usize);
    let mut light = vec![[0.0f64; 3]; w * h];
    // A dot's pitch: two across a cell, four down.
    let (px, py) = (cw / 2.0, ch / 4.0);
    let core = px.min(py) * 0.5;
    let glow = core * 2.6;
    for (i, ink) in scope.dots.iter().enumerate() {
        let Some(ink) = ink else { continue };
        let c = rgb(Some(*ink))?;
        let (dx, dy) = ((i % DW) as f64, (i / DW) as f64);
        let (cx, cy) = ((dx + 0.5) * px, (dy + 0.5) * py);
        let (xa, xb) = (((cx - glow).floor().max(0.0)) as usize, ((cx + glow).ceil() as usize).min(w - 1));
        let (ya, yb) = (((cy - glow).floor().max(0.0)) as usize, ((cy + glow).ceil() as usize).min(h - 1));
        for y in ya..=yb {
            for x in xa..=xb {
                let d = (x as f64 + 0.5 - cx).hypot(y as f64 + 0.5 - cy);
                let k = if d <= core {
                    1.0
                } else if d <= core + 1.0 {
                    1.0 - (d - core)
                } else {
                    0.0
                }
                .max(0.22 * (1.0 - d / glow).max(0.0).powi(2));
                if k <= 0.0 {
                    continue;
                }
                let p = &mut light[y * w + x];
                for n in 0..3 {
                    p[n] = (p[n] + (c[n] - base[n]).max(0.0) * k).min(255.0);
                }
            }
        }
    }
    let mut bytes = Vec::with_capacity(w * h * 3);
    for p in &light {
        for n in 0..3 {
            bytes.push((base[n] + p[n]).min(255.0) as u8);
        }
    }
    let mut png = Vec::new();
    use image::ImageEncoder as _;
    image::codecs::png::PngEncoder::new_with_quality(
        &mut png,
        image::codecs::png::CompressionType::Fast,
        image::codecs::png::FilterType::Sub,
    )
    .write_image(&bytes, w as u32, h as u32, image::ExtendedColorType::Rgb8)
    .ok()?;
    Some(Picture { x: scope.x, y: scope.y, cols: SCOPE_W, rows: SCOPE_H, png })
}

/// Kitty's graphics command that shows `pic` at the cursor, sized to its
/// cells, replacing the splash's last picture (the cursor stays put).
pub fn kitty_show(pic: &Picture) -> String {
    use base64::Engine as _;
    let data = base64::engine::general_purpose::STANDARD.encode(&pic.png);
    let mut out = String::new();
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
    for (i, c) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let head = if i == 0 {
            format!("a=T,f=100,i={PICTURE_ID},p=1,C=1,c={},r={},q=2,", pic.cols, pic.rows)
        } else {
            "q=2,".to_string()
        };
        out.push_str(&format!("\x1b_G{head}m={more};{}\x1b\\", std::str::from_utf8(c).unwrap_or("")));
    }
    out
}

/// Kitty's graphics command that takes the splash's picture away.
pub fn kitty_hide() -> String {
    format!("\x1b_Ga=d,d=I,i={PICTURE_ID},q=2\x1b\\")
}

/// Blank the cells `pic` lies over, so no braille shows round its dots.
pub fn clear_under(pic: &Picture, buf: &mut Buffer) {
    let area = Rect::new(pic.x, pic.y, pic.cols, pic.rows).intersection(buf.area);
    for p in area.positions() {
        buf[p].set_symbol(" ");
    }
}

/// [`kitty_show`] at the picture's cell, the cursor saved and put back: what
/// the runner writes after a frame.
pub fn kitty_at(pic: &Picture) -> String {
    format!("\x1b7\x1b[{};{}H{}\x1b8", pic.y + 1, pic.x + 1, kitty_show(pic))
}

/// The splash as Ink draws it: every row of the screen as runs across its
/// width, colours written as Ink's `color` takes them (`#rrggbb`,
/// `ansi256(n)`, a terminal colour's name, `""` for the terminal's own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SplashView {
    pub width: u16,
    pub height: u16,
    pub rows: Vec<Vec<SplashRun>>,
    /// The scope in pixels, for a renderer whose terminal shows kitty's
    /// pictures: lay it over the rows at its cell, replacing the last one
    /// (kitty image [`PICTURE_ID`], placement 1); take it away when the
    /// splash goes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub picture: Option<SplashPicture>,
}

/// [`Picture`] as Ink takes it: the PNG in base64.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SplashPicture {
    pub x: u16,
    pub y: u16,
    pub cols: u16,
    pub rows: u16,
    pub id: u32,
    pub png: String,
}

impl SplashPicture {
    pub fn of(pic: &Picture) -> Self {
        use base64::Engine as _;
        Self {
            x: pic.x,
            y: pic.y,
            cols: pic.cols,
            rows: pic.rows,
            id: PICTURE_ID,
            png: base64::engine::general_purpose::STANDARD.encode(&pic.png),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SplashRun {
    pub text: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fg: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub bg: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bold: bool,
}

type Ink = Option<Color>;

/// Where each contact sits on the scope: bearing in degrees, distance out
/// as a share of the radius, and whether its label goes to the left.
const SPOTS: [(f64, f64, bool); 6] =
    [(35.0, 0.70, false), (100.0, 0.45, false), (160.0, 0.75, false), (215.0, 0.55, true), (280.0, 0.70, false), (325.0, 0.40, false)];

/// The colours the splash draws with, from the theme.
#[derive(Debug, Clone)]
struct Inks {
    tier: Tier,
    base: Ink,
    muted: Ink,
    /// The graticule's quieter lines.
    dim: Ink,
    secondary: Ink,
    number: Ink,
    key: Ink,
    white: Ink,
    cyan: Ink,
    /// The phosphor, hottest first, at [`PHOSPHOR_AT`].
    phosphor: [Ink; 8],
    /// The mark's ramp, left to right; one colour below full colour.
    ramp: [Ink; 3],
    /// Each contact's colour on the scope, and each name's under the mark.
    contact: [Ink; 6],
    names: [Ink; 6],
    /// The fire's heat 0 to [`FIRE_MAX`].
    fire: Vec<Ink>,
}

/// Where the phosphor ramp steps: the energy at or above which each colour
/// shows.
const PHOSPHOR_AT: [f64; 8] = [0.9, 0.72, 0.56, 0.42, 0.3, 0.2, 0.12, 0.0];
const FIRE_MAX: usize = 36;

impl Inks {
    fn of(theme: &Theme) -> Self {
        let fg = |s| theme.s(s).fg;
        let tier = theme.tier();
        let true_colour = tier == Tier::TrueColor;
        let base = fg(Slot::BgBase);
        let muted = fg(Slot::TextMuted);
        let cyan = fg(Slot::GlowCyan);
        let white = fg(Slot::TextEmphasis);
        let violet = fg(Slot::AccentMuted);
        let purple = fg(Slot::GlowPurple);
        let phosphor = if true_colour {
            [
                mix(cyan, white, 0.9),
                cyan,
                mix(cyan, violet, 0.5),
                violet,
                mix(violet, purple, 0.6),
                mix(purple, base, 0.4),
                mix(purple, base, 0.6),
                mix(purple, base, 0.78),
            ]
        } else {
            [white, cyan, cyan, violet, purple, purple, muted, muted]
        };
        let ramp = if true_colour {
            [purple, fg(Slot::AccentTertiary), cyan]
        } else {
            [fg(Slot::AccentPrimary); 3]
        };
        let (red, amber, yellow) = (fg(Slot::StatusError), fg(Slot::DataNumber), fg(Slot::StatusWarning));
        let stops = [
            base,
            mix(base, red, 0.25),
            mix(base, red, 0.5),
            mix(base, red, 0.8),
            red,
            mix(red, amber, 0.5),
            amber,
            mix(amber, yellow, 0.5),
            yellow,
            mix(yellow, white, 0.6),
            white,
        ];
        let fire = (0..=FIRE_MAX)
            .map(|h| {
                let u = h as f64 / FIRE_MAX as f64 * (stops.len() - 1) as f64;
                let i = (u.floor() as usize).min(stops.len() - 2);
                mix(stops[i], stops[i + 1], (u - i as f64) as f32)
            })
            .collect();
        Self {
            tier,
            base,
            muted,
            dim: if true_colour { mix(muted, base, 0.45) } else { muted },
            secondary: fg(Slot::TextSecondary),
            number: amber,
            key: fg(Slot::AccentPrimary),
            white,
            cyan,
            phosphor,
            ramp,
            contact: [amber, fg(Slot::StatusInfo), fg(Slot::AccentTertiary), fg(Slot::TextPrimary), yellow, fg(Slot::AccentPrimary)],
            names: [
                fg(Slot::AccentPrimary),
                fg(Slot::AccentTertiary),
                fg(Slot::StatusInfo),
                fg(Slot::TextSecondary),
                yellow,
                fg(Slot::AccentSecondary),
            ],
            fire,
        }
    }

    /// No colour at all: the mark and the labels are bold instead.
    fn mono(&self) -> bool {
        self.tier == Tier::Mono
    }

    /// The ramp at `u` (0 at the left, 1 at the right).
    fn ramp_at(&self, u: f32) -> Ink {
        let u = u.clamp(0.0, 1.0) * 2.0;
        if u <= 1.0 { mix(self.ramp[0], self.ramp[1], u) } else { mix(self.ramp[1], self.ramp[2], u - 1.0) }
    }

    /// The phosphor at energy `e`.
    fn glow(&self, e: f64) -> Ink {
        PHOSPHOR_AT.iter().position(|&k| e >= k).map_or(self.phosphor[7], |i| self.phosphor[i])
    }
}

/// `a` to `b` by `k` (0 to 1): channel by channel between two full
/// colours; otherwise the nearer of the two.
fn mix(a: Ink, b: Ink, k: f32) -> Ink {
    let k = k.clamp(0.0, 1.0);
    match (a, b) {
        (Some(Color::Rgb(r1, g1, b1)), Some(Color::Rgb(r2, g2, b2))) => {
            let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * k).round() as u8;
            Some(Color::Rgb(ch(r1, r2), ch(g1, g2), ch(b1, b2)))
        }
        _ if k >= 0.5 => b,
        _ => a,
    }
}

fn mixf(a: Ink, b: Ink, k: f64) -> Ink {
    mix(a, b, k as f32)
}

fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

fn ease_out(v: f64) -> f64 {
    1.0 - (1.0 - clamp01(v)).powi(3)
}

/// A fixed noise in `0..1` for a dot.
fn hash(x: f64, y: f64) -> f64 {
    let s = (x * 12.9898 + y * 78.233).sin() * 43758.5453;
    s - s.floor()
}

/// The half-block glyph for a cell's top and bottom pixels.
pub fn half_block(top: bool, bottom: bool) -> char {
    match (top, bottom) {
        (true, true) => '█',
        (true, false) => '▀',
        (false, true) => '▄',
        (false, false) => ' ',
    }
}

/// The mark as cells, `MARK_H` rows of `MARK_W`: two pixel rows a cell,
/// each `(top lit, bottom lit)`.
pub fn mark_pixels() -> Vec<Vec<(bool, bool)>> {
    let px: Vec<Vec<bool>> = BITMAP.iter().map(|r| r.chars().map(|c| c == '#').collect()).collect();
    (0..MARK_H as usize)
        .map(|row| (0..MARK_W as usize).map(|col| (px[row * 2][col], px[row * 2 + 1][col])).collect())
        .collect()
}

/// One cell of the screen being laid out; `bg` `None` is the base.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    ch: String,
    fg: Ink,
    bg: Ink,
    bold: bool,
}

struct Grid {
    width: u16,
    height: u16,
    cells: Vec<Vec<Cell>>,
}

impl Grid {
    fn new(width: u16, height: u16) -> Self {
        let blank = Cell { ch: " ".into(), fg: None, bg: None, bold: false };
        Self { width, height, cells: vec![vec![blank; width as usize]; height as usize] }
    }

    fn put(&mut self, x: i32, y: i32, ch: char, fg: Ink, bg: Ink, bold: bool) {
        let w = ch.width().unwrap_or(0) as i32;
        if w == 0 || x < 0 || y < 0 || y >= self.height as i32 || x + w > self.width as i32 {
            return;
        }
        let row = &mut self.cells[y as usize];
        row[x as usize] = Cell { ch: ch.to_string(), fg, bg, bold };
        if w == 2 {
            row[x as usize + 1] = Cell { ch: String::new(), fg: None, bg, bold: false };
        }
    }

    /// `parts` centred on row `y` in columns `x0..x0 + room`, the first
    /// `shown` characters only; centred by the whole width so typing does
    /// not move it.
    fn centred(&mut self, y: i32, x0: i32, room: i32, parts: &[(String, Ink, bool)], shown: usize) {
        let total: i32 = parts.iter().map(|(t, _, _)| crate::ui::width(t) as i32).sum();
        let mut x = x0 + (room - total).max(0) / 2;
        let mut n = 0;
        for (text, ink, bold) in parts {
            for c in text.chars() {
                if n >= shown || x >= x0 + room {
                    return;
                }
                self.put(x, y, c, *ink, None, *bold);
                x += c.width().unwrap_or(0) as i32;
                n += 1;
            }
        }
    }

    fn rows(self, base: Ink) -> Vec<Vec<Run>> {
        self.cells
            .into_iter()
            .map(|cells| {
                let mut runs: Vec<Run> = Vec::new();
                for c in cells {
                    let fg = if c.ch == " " { None } else { c.fg };
                    let bold = c.ch != " " && c.bold;
                    let bg = c.bg.or(base);
                    match runs.last_mut() {
                        Some(r) if r.fg == fg && r.bg == bg && r.bold == bold => r.text.push_str(&c.ch),
                        _ => runs.push(Run { text: c.ch, fg, bg, bold }),
                    }
                }
                runs
            })
            .collect()
    }
}

/// The splash at `scene`, `width` × `height` cells; `None` where it is not
/// drawn at all (narrower than [`MIN_WIDTH`], or under two rows).
pub fn frame(theme: &Theme, scene: &Scene, width: u16, height: u16) -> Option<Frame> {
    if width < MIN_WIDTH || height < 2 {
        return None;
    }
    let inks = Inks::of(theme);
    let mut grid = Grid::new(width, height);
    let full = scene.style == Style::Full;
    let fire = full && scene.start.event == Some(Event::Fire);
    // Seconds into the splash; with no motion, its last frame.
    let mut t = match scene.style {
        Style::Static => 9.0,
        _ => scene.at as f64 / 1000.0,
    };
    let w = width as i32;
    let mut scope = None;
    if width >= SCOPE_W && height >= FULL_HEIGHT {
        let top = ((height - FULL_HEIGHT) / 2) as i32;
        let x0 = ((width - SCOPE_W) / 2) as i32;
        let dots = draw_scope(&mut grid, &inks, scene, t, x0, top);
        scope = Some(ScopeDots { x: x0 as u16, y: top as u16, dots });
        if fire {
            draw_fire(&mut grid, &inks, t, top + SCOPE_H as i32 + 1);
        } else {
            draw_mark(&mut grid, &inks, scene.style, t, top + SCOPE_H as i32 + 2);
            draw_names(&mut grid, &inks, scene, t, top + SCOPE_H as i32 + 7);
        }
        draw_line(&mut grid, &inks, scene, t, top + FULL_HEIGHT as i32 - 1, w);
    } else {
        // No scope: start from where the beam would have finished.
        if full {
            t += (BEAM_FROM_MS + TURN_MS) as f64 / 1000.0 - 0.2;
        }
        if height >= COMPACT_HEIGHT && width >= MARK_W {
            let top = ((height - COMPACT_HEIGHT) / 2) as i32;
            draw_mark(&mut grid, &inks, scene.style, t, top);
            draw_names(&mut grid, &inks, scene, t, top + MARK_H as i32 + 1);
            draw_line(&mut grid, &inks, scene, t, top + COMPACT_HEIGHT as i32 - 1, w);
        } else {
            let top = ((height - 2) / 2) as i32;
            draw_names(&mut grid, &inks, scene, t, top);
            draw_line(&mut grid, &inks, scene, t, top + 1, w);
        }
    }
    Some(Frame { width, height, rows: grid.rows(inks.base), scope })
}

/// The scope's dots: which are lit in each cell and the colour of its
/// brightest.
struct Dots {
    bits: Vec<u8>,
    best: Vec<f64>,
    ink: Vec<Ink>,
    /// Each dot's own best score and colour, for the pixel tier.
    dot_best: Vec<f64>,
    dot_ink: Vec<Ink>,
}

impl Dots {
    const W: usize = SCOPE_W as usize;
    const BIT: [[u8; 4]; 2] = [[0x1, 0x2, 0x4, 0x40], [0x8, 0x10, 0x20, 0x80]];

    fn new() -> Self {
        let n = Self::W * SCOPE_H as usize;
        Self {
            bits: vec![0; n],
            best: vec![-1.0; n],
            ink: vec![None; n],
            dot_best: vec![-1.0; DW * DH],
            dot_ink: vec![None; DW * DH],
        }
    }

    /// Light dot `dx, dy`; the cell takes the colour of its highest `score`.
    fn light(&mut self, dx: usize, dy: usize, score: f64, ink: Ink) {
        let c = (dy >> 2) * Self::W + (dx >> 1);
        self.bits[c] |= Self::BIT[dx & 1][dy & 3];
        if score > self.best[c] {
            self.best[c] = score;
            self.ink[c] = ink;
        }
        let d = dy * DW + dx;
        if score > self.dot_best[d] {
            self.dot_best[d] = score;
            self.dot_ink[d] = ink;
        }
    }
}

const DW: usize = SCOPE_W as usize * 2;
const DH: usize = SCOPE_H as usize * 4;
const CX: f64 = 320.0;
const CY: f64 = 300.0;
const RAD: f64 = 290.0;

/// What never changes about a dot of the scope, worked out once: where it
/// is from the centre, its bearing, which graticule lines it is on, and its
/// fixed noise. A frame then costs a few comparisons and an `exp` a dot.
struct Geo {
    r: f64,
    phi: f64,
    /// On the dotted cross.
    cross: bool,
    /// On a tick, and a long one (every 90 degrees).
    tick: bool,
    long: bool,
    /// The wake's, the returns' and their threshold's noise.
    wake: f64,
    clutter: f64,
    floor: f64,
}

fn geometry() -> &'static [Geo] {
    static GEO: std::sync::OnceLock<Vec<Geo>> = std::sync::OnceLock::new();
    GEO.get_or_init(|| {
        let mut out = Vec::with_capacity(DW * DH);
        for dy in 0..DH {
            for dx in 0..DW {
                let (ex, ey) = (dx as f64 * 5.0 + 2.5 - CX, dy as f64 * 6.25 + 3.125 - CY);
                let r = ex.hypot(ey);
                let mut phi = ex.atan2(-ey);
                if phi < 0.0 {
                    phi += TAU;
                }
                let deg = phi * 180.0 / PI;
                out.push(Geo {
                    r,
                    phi,
                    cross: (ex.abs() < 2.6 || ey.abs() < 3.2) && (dx + dy) % 3 == 0,
                    tick: (((deg + 7.5) % 15.0) - 7.5).abs() * PI / 180.0 * r < 2.6,
                    long: (((deg + 45.0) % 90.0) - 45.0).abs() < 1.0,
                    wake: hash(dx as f64, dy as f64),
                    clutter: hash(dx as f64 * 3.0 + 1.0, dy as f64 * 7.0 + 2.0),
                    floor: hash(dx as f64 + 9.0, dy as f64 + 4.0),
                });
            }
        }
        out
    })
}

/// The scope, its contacts and their labels, and the readout under it, with
/// its top-left cell at `x0, y0`.
fn draw_scope(grid: &mut Grid, inks: &Inks, scene: &Scene, t: f64, x0: i32, y0: i32) -> Vec<Ink> {
    let (cx, cy, rad) = (CX, CY, RAD);
    let open = OPEN_MS as f64 / 1000.0;
    let t0 = BEAM_FROM_MS as f64 / 1000.0;
    let turn = TURN_MS as f64 / 1000.0;
    let full = scene.style == Style::Full;
    let subtle = scene.style == Style::Subtle;
    let grow = if full { ease_out(t / open) } else { 1.0 };
    let theta = if full && t >= t0 && t <= t0 + turn { (t - t0) / turn * TAU } else { -1.0 };
    let fade_in = if subtle { clamp01(t / 0.3) } else { 1.0 };
    let hot = inks.phosphor[0];

    struct Blip<'a> {
        c: &'a Contact,
        x: f64,
        y: f64,
        left: bool,
        ink: Ink,
        age: f64,
    }
    let blips: Vec<Blip> = scene
        .contacts
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let (ang, rr, left) = SPOTS.get(i).copied().unwrap_or_else(|| {
                let a = (35.0 + 61.0 * i as f64) % 360.0;
                (a, 0.6, (180.0..270.0).contains(&a))
            });
            let a = ang * PI / 180.0;
            let ta = if full { t0 + ang / 360.0 * turn } else { 0.35 + i as f64 * 0.1 };
            Blip {
                c,
                x: cx + a.sin() * rr * rad,
                y: cy - a.cos() * rr * rad,
                left,
                ink: inks.contact[i % inks.contact.len()],
                age: t - ta,
            }
        })
        .collect();
    let found = blips.iter().filter(|b| b.age >= 0.0 && b.c.live).count();

    let mut dots = Dots::new();
    let geo = geometry();
    for dy in 0..DH {
        for dx in 0..DW {
            let d = &geo[dy * DW + dx];
            let (r, phi) = (d.r, d.phi);
            if r > rad * grow + 2.5 {
                continue;
            }
            // Seconds since the beam crossed this bearing (one turn only).
            let te = t0 + phi / TAU * turn;
            let age = if full && t >= te { t - te } else { f64::INFINITY };
            // The graticule: three rings, a dotted cross, a tick every 15
            // degrees (longer every 90), the centre.
            let mut g: f64 = 0.0;
            for f in [1.0, 0.66, 0.33] {
                if (r - rad * f * grow).abs() < 2.4 {
                    g = g.max(if f == 1.0 { 0.5 } else { 0.3 });
                }
            }
            if d.cross {
                g = g.max(0.25);
            }
            if d.tick && r > rad * grow - if d.long { 26.0 } else { 13.0 } {
                g = g.max(0.45);
            }
            if r < 4.0 {
                g = 0.5;
            }
            // The beam: one bright line, a dot or two wide.
            if theta >= 0.0 {
                let mut behind = theta - phi;
                if behind < 0.0 {
                    behind += TAU;
                }
                if behind < 0.6 && r * behind.sin() < 2.6 && r < rad * grow {
                    dots.light(dx, dy, 3.0, hot);
                    continue;
                }
            }
            let mut e: f64 = 0.0;
            let mut ink: Option<Ink> = None;
            // Its glow: a short soft wake that thins out as it decays.
            if age < 0.5 && r < rad * grow - 1.0 {
                let w = 0.82 * (-age / 0.07).exp();
                if d.wake < w * 0.6 {
                    e = w;
                    ink = Some(inks.glow(w));
                }
            }
            // Returns: sparse clutter, thicker near the centre, that lights
            // as the beam crosses and fades.
            let clutter = if r < 0.2 * rad { 0.045 } else { 0.006 };
            if age < 3.0 && d.clutter < clutter {
                let c = 0.92 * (-age / 0.5).exp();
                if c > 0.12 + 0.55 * d.floor && c > e {
                    e = c;
                    ink = Some(inks.glow(c));
                }
            }
            // The graticule is phosphor too: flashed as the scope opens and
            // as the beam crosses, settling to its rest.
            if g > 0.0 {
                let tr = open * (1.0 - (1.0 - r / rad).max(0.0).cbrt());
                let opening = if full && t >= tr { 0.95 * (-(t - tr) / 0.12).exp() } else { 0.0 };
                let crossed = if age < 3.0 { 0.8 * (-age / 0.28).exp() } else { 0.0 };
                let flash = opening.max(crossed);
                let rest = mixf(inks.base, if g >= 0.45 { inks.muted } else { inks.dim }, fade_in);
                if flash > 0.3 && flash > e {
                    e = flash;
                    ink = Some(inks.glow(flash));
                } else if e < 0.25 {
                    e = e.max(0.01);
                    if ink.is_none() || e <= 0.2 {
                        ink = Some(rest);
                    }
                }
            }
            if let Some(ink) = ink {
                dots.light(dx, dy, 1.0 + e, ink);
            }
        }
    }

    // The echo: a big return the beam paints, at bearing 128, 0.6 out.
    let shape = scene.start.echo.shape();
    let a = 128.0 * PI / 180.0;
    let (ex0, ey0) = (cx + a.sin() * 0.6 * rad, cy - a.cos() * 0.6 * rad);
    let ya = ((ey0 - ECHO_REACH) / 6.25).floor().max(0.0) as usize;
    let yb = (((ey0 + ECHO_REACH) / 6.25).ceil() as usize).min(DH - 1);
    let xa = ((ex0 - ECHO_REACH) / 5.0).floor().max(0.0) as usize;
    let xb = (((ex0 + ECHO_REACH) / 5.0).ceil() as usize).min(DW - 1);
    for dy in ya..=yb {
        for dx in xa..=xb {
            if !shape.inside(dx as f64 * 5.0 + 2.5 - ex0, dy as f64 * 6.25 + 3.125 - ey0) {
                continue;
            }
            if full {
                let mut phi = (dx as f64 * 5.0 + 2.5 - cx).atan2(-(dy as f64 * 6.25 + 3.125 - cy));
                if phi < 0.0 {
                    phi += TAU;
                }
                let te = t0 + phi / TAU * turn;
                if t < te {
                    continue;
                }
                let e = (0.95 * (-(t - te) / 0.45).exp()).max(0.22);
                dots.light(dx, dy, 2.0 + e, inks.glow(e));
            } else {
                let k = if subtle { clamp01((t - 0.6) / 0.4) } else { 1.0 };
                dots.light(dx, dy, 2.2, mixf(inks.base, inks.phosphor[5], k));
            }
        }
    }

    // Contacts: a blip, brackets that close on it (lock-on), then its label.
    for b in blips.iter().filter(|b| b.age >= 0.0) {
        let live = b.c.live;
        let tone = if live { b.ink } else { inks.muted };
        let blip = if full {
            let e = if live { (-b.age / 0.6).exp().max(0.5) } else { (0.7 * (-b.age / 0.25).exp()).max(0.25) };
            if b.age < 0.08 {
                inks.white
            } else if live {
                tone
            } else {
                mixf(inks.base, tone, 0.5 + e)
            }
        } else {
            mixf(inks.base, tone, clamp01(b.age / 0.25))
        };
        let close = if full { ease_out(b.age / 0.2) } else { 1.0 };
        let s = 17.0 + 30.0 * (1.0 - close);
        let sy = s + 2.0;
        let bracket = match live {
            false => None,
            true if full && b.age < 0.2 => Some(mixf(b.ink, inks.white, 0.4)),
            true => {
                let k = if full { 1.0 - 0.45 * clamp01((b.age - 0.4) / 0.5) } else { 0.55 * clamp01(b.age / 0.25) };
                Some(mixf(inks.base, b.ink, k))
            }
        };
        let xa = ((b.x - s - 6.0) / 5.0).floor().max(0.0) as usize;
        let xb = (((b.x + s + 6.0) / 5.0).ceil() as usize).min(DW - 1);
        let ya = ((b.y - sy - 8.0) / 6.25).floor().max(0.0) as usize;
        let yb = (((b.y + sy + 8.0) / 6.25).ceil() as usize).min(DH - 1);
        for dy in ya..=yb {
            for dx in xa..=xb {
                let ax = (dx as f64 * 5.0 + 2.5 - b.x).abs();
                let ay = (dy as f64 * 6.25 + 3.125 - b.y).abs();
                if ax.hypot(ay * 0.9) < 7.0 && hash(dx as f64 + 11.0, dy as f64 + 3.0) < 0.85 {
                    dots.light(dx, dy, 5.0, blip);
                    continue;
                }
                let Some(k) = bracket else { continue };
                let on_x = (ax - s).abs() < 2.5 && ay <= sy + 3.2 && ay > sy - 9.0;
                let on_y = (ay - sy).abs() < 3.2 && ax <= s + 2.5 && ax > s - 8.0;
                if on_x || on_y {
                    dots.light(dx, dy, 4.0, k);
                }
            }
        }
    }

    let w = SCOPE_W as usize;
    for (c, &bits) in dots.bits.iter().enumerate() {
        if bits == 0 {
            continue;
        }
        let ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
        grid.put(x0 + (c % w) as i32, y0 + (c / w) as i32, ch, dots.ink[c], None, false);
    }

    // Each contact's label types in beside it: its mark, its name, its tail.
    #[derive(Clone, Copy, PartialEq)]
    enum Role {
        Mark,
        Name,
        Tail,
    }
    for b in &blips {
        if b.age < 0.0 || (full && b.age < 0.06) {
            continue;
        }
        let mut text: Vec<(char, Role)> = Vec::new();
        if !b.c.mark.is_empty() {
            text.extend(b.c.mark.chars().map(|c| (c, Role::Mark)));
            text.push((' ', Role::Name));
        }
        text.extend(b.c.name.chars().map(|c| (c, Role::Name)));
        let tail = if b.c.live { b.c.tail.clone() } else { Some("no signal".into()) };
        if let Some(tail) = tail {
            text.extend("  ".chars().chain(tail.chars()).map(|c| (c, Role::Tail)));
        }
        let cells: i32 = text.iter().map(|(c, _)| c.width().unwrap_or(0) as i32).sum();
        let shown = if full { (((b.age - 0.06) / 0.014).floor().max(0.0) as usize).min(text.len()) } else { text.len() };
        let k = if full { 1.0 } else { clamp01(b.age / 0.25) };
        let row = (b.y / 25.0).floor() as i32;
        let at = (b.x / 10.0).floor() as i32;
        let mut x = at + 3;
        if b.left || x + cells > SCOPE_W as i32 - 1 {
            x = at - 3 - cells;
        }
        for &(ch, role) in text.iter().take(shown) {
            let want = match role {
                Role::Mark if b.c.live => b.ink,
                _ if !b.c.live => inks.muted,
                Role::Mark | Role::Name if full && b.age < 0.5 => inks.white,
                Role::Mark | Role::Name => inks.secondary,
                Role::Tail => inks.number,
            };
            if x >= 0 && x < SCOPE_W as i32 {
                grid.put(x0 + x, y0 + row, ch, mixf(inks.base, want, k), None, inks.mono());
            }
            x += ch.width().unwrap_or(0) as i32;
        }
    }

    // The readout under the scope: the beam's bearing and the contacts so
    // far; then what the sweep found.
    let n = |v: usize, one: &str, many: &str| if v == 1 { one.to_string() } else { many.to_string() };
    let done = !full || t > t0 + turn;
    let mut parts: Vec<(String, Ink, bool)> = Vec::new();
    if !done && t >= t0 {
        let brg = ((t - t0) / turn * 360.0).floor() as u32;
        parts.push(("brg ".into(), inks.muted, false));
        parts.push((format!("{brg:03}°"), inks.cyan, false));
        parts.push(("   ".into(), None, false));
        parts.push((found.to_string(), inks.number, false));
        parts.push((n(found, " contact", " contacts"), inks.muted, false));
    } else if done && found > 0 {
        parts.push((found.to_string(), inks.number, false));
        parts.push((n(found, " contact", " contacts"), inks.muted, false));
        let total: u32 = blips.iter().filter(|b| b.age >= 0.0 && b.c.live).map(|b| b.c.count).sum();
        if total > 0 {
            parts.push((SEPARATOR.into(), inks.muted, false));
            parts.push((total.min(999).to_string(), inks.number, false));
            parts.push((" for you".into(), inks.muted, false));
        }
        if let Some(lost) = blips.iter().find(|b| b.age >= 0.0 && !b.c.live) {
            parts.push((SEPARATOR.into(), inks.muted, false));
            parts.push((lost.c.name.clone(), inks.secondary, false));
            parts.push((" no signal".into(), inks.muted, false));
        }
        if scene.start.echo.rare() {
            parts.push((SEPARATOR.into(), inks.muted, false));
            parts.push(("1 unidentified".into(), inks.phosphor[3], false));
        }
    }
    let k = if subtle { clamp01((t - 0.35) / 0.3) } else { 1.0 };
    for p in &mut parts {
        p.1 = mixf(inks.base, p.1, k);
    }
    grid.centred(y0 + SCOPE_H as i32, x0, SCOPE_W as i32, &parts, usize::MAX);
    dots.dot_ink
}

/// The name in half blocks, centred on row `y`, painted by a scan line on
/// the same phosphor (full) or faded in.
fn draw_mark(grid: &mut Grid, inks: &Inks, style: Style, t: f64, y: i32) {
    let paint = PAINT_FROM_MS as f64 / 1000.0;
    let last = (MARK_W - 1) as f64;
    let pix = |x: usize, lit: bool| -> Option<Ink> {
        if !lit {
            return None;
        }
        let own = inks.ramp_at(x as f32 / last as f32);
        match style {
            Style::Full => {
                let age = t - (paint + x as f64 / last * PAINT_MS as f64 / 1000.0 * 0.7);
                if age < 0.0 {
                    return None;
                }
                let e = (-age / 0.1).exp();
                Some(if e > 0.7 {
                    inks.phosphor[0]
                } else if e > 0.4 {
                    inks.cyan
                } else {
                    mixf(inks.cyan, own, clamp01((0.4 - e) / 0.25))
                })
            }
            Style::Subtle => Some(mixf(inks.base, own, clamp01((t - 1.0) / 0.3))),
            Style::Static => Some(own),
        }
    };
    let x0 = (grid.width as i32 - MARK_W as i32) / 2;
    for (row, cells) in mark_pixels().iter().enumerate() {
        for (col, &(top, bottom)) in cells.iter().enumerate() {
            let (a, b) = (pix(col, top), pix(col, bottom));
            let (ch, fg, bg) = match (a, b) {
                (Some(a), Some(b)) => ('▀', a, b),
                (Some(a), None) => ('▀', a, None),
                (None, Some(b)) => ('▄', b, None),
                (None, None) => continue,
            };
            grid.put(x0 + col as i32, y + row as i32, ch, fg, bg, inks.mono());
        }
    }
}

/// The parts' names on row `y`, each in its colour, fading in.
fn draw_names(grid: &mut Grid, inks: &Inks, scene: &Scene, t: f64, y: i32) {
    let k = match scene.style {
        Style::Full => clamp01((t - NAMES_FROM_MS as f64 / 1000.0) / 0.3),
        Style::Subtle => clamp01((t - 1.2) / 0.3),
        Style::Static => 1.0,
    };
    if k <= 0.0 {
        return;
    }
    let mut parts: Vec<(String, Ink, bool)> = Vec::new();
    for (i, c) in scene.contacts.iter().enumerate() {
        if i > 0 {
            parts.push((SEPARATOR.into(), mixf(inks.base, inks.muted, k), false));
        }
        parts.push((c.name.clone(), mixf(inks.base, inks.names[i % inks.names.len()], k), inks.mono()));
    }
    grid.centred(y, 0, grid.width as i32, &parts, usize::MAX);
}

/// The start line on row `y`, typed in (full) or shown whole.
fn draw_line(grid: &mut Grid, inks: &Inks, scene: &Scene, t: f64, y: i32, width: i32) {
    let parts: Vec<(String, Ink, bool)> = scene
        .start
        .line
        .iter()
        .map(|s| if s.key { (s.text.clone(), inks.key, true) } else { (s.text.clone(), inks.secondary, false) })
        .collect();
    let n: usize = parts.iter().map(|(t, _, _)| t.chars().count()).sum();
    if n == 0 {
        return;
    }
    let from = LINE_FROM_MS as f64 / 1000.0;
    let shown = match scene.style {
        Style::Full if t < from => 0,
        Style::Full => {
            let each = 0.012_f64.min(TYPE_MS as f64 / 1000.0 / n as f64);
            n.min(1 + ((t - from) / each).floor() as usize)
        }
        Style::Subtle if t <= 1.3 => 0,
        _ => n,
    };
    grid.centred(y, 0, width, &parts, shown);
}

/// The fire for "This is fine.": the PSX Doom fire (heat spreads up a row at
/// a time; one roll decides both the sideways lick and whether it cools,
/// which makes ragged tongues), 51 × 16 pixels in half blocks, stepped at 14
/// a second from [`FIRE_FROM_MS`] and replayed from its seed each frame, so
/// any moment draws the same picture. Seven rows from `y`; the top pixel row
/// pair is mostly tips and goes.
fn draw_fire(grid: &mut Grid, inks: &Inks, t: f64, y: i32) {
    const W: usize = 51;
    const H: usize = 16;
    const T: usize = H + 2;
    let max = FIRE_MAX as i32;
    let mut heat = vec![0_i32; W * T];
    let mut s: u32 = 7;
    let mut rnd = move || {
        s ^= s << 13;
        s ^= ((s as i32) >> 17) as u32;
        s ^= s << 5;
        s as f64 / 4_294_967_296.0
    };
    let steps = ((t - FIRE_FROM_MS as f64 / 1000.0) * 14.0).floor().max(0.0) as u32;
    for n in 1..=steps {
        let n = n as f64;
        for x in 0..W {
            let u = (x as f64 + 0.5) / W as f64 - 0.5;
            let bed = (1.0 - (u / 0.46).powi(4)).max(0.0);
            let wave = 0.75 + 0.25 * (x as f64 * 0.7 + n * 0.23).sin() * (x as f64 * 0.31 - n * 0.11).sin();
            let v = (max as f64 * bed * wave).round() as i32;
            heat[(T - 1) * W + x] = v;
            heat[(T - 2) * W + x] = v;
        }
        for yy in 0..T - 2 {
            for x in 0..W {
                let r = (rnd() * 4.0).floor() as usize;
                let sx = (x as i32 + [-1, 0, 1, 0][r]).clamp(0, W as i32 - 1) as usize;
                let edge = ((x as f64 + 0.5) / W as f64 - 0.5).abs() * 2.0;
                let side = if rnd() < edge * edge * 0.9 { 2 } else { 0 };
                let tip = if (yy as f64) < H as f64 * 0.35 && rnd() < 0.5 { 1 } else { 0 };
                heat[yy * W + x] = (heat[(yy + 1) * W + sx] - (r as i32 & 1) * 3 - side - tip).max(0);
            }
        }
    }
    let at = |x: usize, yy: usize| {
        let h = heat[yy * W + x];
        (h > 2).then(|| inks.fire[h.min(max) as usize])
    };
    let x0 = (grid.width as i32 - W as i32) / 2;
    for r in 1..H / 2 {
        for x in 0..W {
            let (top, bottom) = (at(x, 2 * r), at(x, 2 * r + 1));
            if top.is_none() && bottom.is_none() {
                continue;
            }
            grid.put(x0 + x as i32, y + r as i32 - 1, '▀', top.unwrap_or(inks.base), bottom.flatten(), false);
        }
    }
}

/// Paint `frame` over `buf` from its top-left corner.
pub fn draw(frame: &Frame, buf: &mut Buffer) {
    let area = buf.area;
    for (y, runs) in frame.rows.iter().enumerate() {
        let y = area.y + y as u16;
        if y >= area.bottom() {
            break;
        }
        let mut x = area.x;
        for run in runs {
            let mut paint = Paint::new().fg(run.fg.unwrap_or(Color::Reset)).bg(run.bg.unwrap_or(Color::Reset));
            if run.bold {
                paint = paint.add_modifier(Modifier::BOLD);
            }
            for c in run.text.chars() {
                let w = c.width().unwrap_or(0) as u16;
                if w == 0 || x + w > area.right() {
                    continue;
                }
                let cell = &mut buf[(x, y)];
                cell.reset();
                cell.set_char(c).set_style(paint);
                if w == 2 {
                    buf[(x + 1, y)].reset();
                    buf[(x + 1, y)].set_symbol("").set_style(paint);
                }
                x += w;
            }
        }
        // Whatever the runs did not reach (a resize between frames) is the base too.
        let rest = Rect::new(x, y, area.right().saturating_sub(x), 1);
        for p in rest.positions() {
            buf[p].reset();
            if let Some(bg) = frame.rows[0].first().and_then(|r| r.bg) {
                buf[p].set_bg(bg);
            }
        }
    }
}

/// `frame` as Ink draws it.
pub fn view(frame: &Frame) -> SplashView {
    use crate::ipc::look::colour_word;
    SplashView {
        width: frame.width,
        height: frame.height,
        rows: frame
            .rows
            .iter()
            .map(|runs| {
                runs.iter()
                    .map(|r| SplashRun {
                        text: r.text.clone(),
                        fg: colour_word(r.fg.map(Into::into)),
                        bg: colour_word(r.bg.map(Into::into)),
                        bold: r.bold,
                    })
                    .collect()
            })
            .collect(),
        picture: None,
    }
}

#[cfg(test)]
mod tests;
