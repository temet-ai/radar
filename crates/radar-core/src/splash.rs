//! The startup splash: the `radar` wordmark in half blocks, the parts in this
//! build under it, and a short motion that hands off to the first screen.
//!
//! Everything both renderers draw is worked out here, as cells: [`frame`]
//! turns a [`Scene`] (what to show, how far into it) into rows of runs with
//! their colours, which the Ratatui runner paints over its buffer
//! ([`draw`]) and the bridge sends to Ink ([`view`]), so the two draw the
//! same cells. The timing lives in [`Splash`], which each runner keeps:
//! when it starts, when it hands off, and that any input ends it at once.
//!
//! - `[theme] splash = "auto" | "off"`: `off` never shows it.
//! - Motion `full`: the mark's columns light left to right with a soft
//!   leading edge while its colour ramp slides in, then the tagline reads
//!   in and each part's name lights as the part is ready; then the first
//!   screen fades in from the base colour (the screen cue).
//! - `subtle`: the mark at once, the names lighting, the fade.
//! - `off` (or motion held back: over SSH, below full colour, a slow
//!   terminal): nothing moves; the mark shows only while the first screen
//!   is not ready after [`STATIC_AFTER_MS`].
//! - A key, click or paste ends it at once and acts on the screen under it.

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

/// The wordmark, 8 pixels tall: `#` a lit pixel, `C` the block cursor.
/// Two pixel rows make one cell row (`▀` `▄` `█`), so it is 4 rows tall.
pub const BITMAP: [&str; 8] = [
    ".####.#..####.#..CC",
    "##..###.##..###..CC",
    "##...##.##...##..CC",
    "##..###.##..###..CC",
    ".###.##..###.##..CC",
    ".....##......##....",
    "##..##..##..##.....",
    ".####....####......",
];

/// The mark's size in cells.
pub const MARK_W: u16 = 19;
pub const MARK_H: u16 = 4;
/// The letters' last column: the ramp runs from column 0 to here.
const LETTERS_LAST: u16 = 14;
/// The tagline under the parts' names.
pub const TAGLINE: &str = "one screen, one set of keys";
/// Between the parts' names.
pub const SEPARATOR: &str = " · ";
/// Narrower than this, no splash at all.
pub const MIN_WIDTH: u16 = 20;
/// The mark, a blank row and the two lines of text.
const FULL_HEIGHT: u16 = MARK_H + 3;

/// The mark's columns light up, left to right.
pub const ASSEMBLE_MS: u32 = 450;
/// Then the two lines of text read in.
pub const TAGLINE_MS: u32 = 120;
/// The soft leading edge, in columns.
const EDGE: f32 = 3.0;
/// When the first part's name starts to light at `full`, and at `subtle`.
const LIGHT_FULL_MS: u32 = 470;
const LIGHT_SUBTLE_MS: u32 = 0;
/// Each next name lights this much later.
const STAGGER_FULL_MS: u32 = 30;
const STAGGER_SUBTLE_MS: u32 = 50;
/// A name goes from muted to its colour in this long.
const LIGHT_MS: u32 = 80;
/// The splash hands off once the first screen is ready and this long has
/// passed: at `full` the assemble and the names; at `subtle` the names.
pub const HOLD_FULL_MS: u32 = 680;
pub const HOLD_SUBTLE_MS: u32 = 400;
/// With no motion, the mark shows only when the first screen is not ready
/// after this long.
pub const STATIC_AFTER_MS: u32 = 150;
/// The cursor is on, then off, this long each.
pub const BLINK_MS: u32 = 500;
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
            Self::Auto => "the radar mark while radar starts, moving as motion allows",
            Self::Off => "straight to the first screen",
        }
    }
}

/// How the splash moves, from the motion level that runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    /// The assemble, the tagline reading in, the names lighting, the fade.
    Full,
    /// The mark at once, the names lighting, the fade.
    Subtle,
    /// Nothing moves; shown only when startup takes time.
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

/// A part's name on the splash, and since when it is ready (ms into the
/// splash), if it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    pub ready_at: Option<u32>,
}

/// What one frame of the splash shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    pub style: Style,
    /// Ms since the splash started.
    pub at: u32,
    pub parts: Vec<Part>,
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
#[derive(Debug, Clone)]
pub struct Splash {
    style: Style,
    /// When it started (the runner's first frame), on the runner's clock.
    started: Option<Duration>,
    /// It ended: handed off, skipped, or never to show.
    done: bool,
    /// Each part, by name, and since when it is ready.
    ready: Vec<(String, Option<u32>)>,
}

impl Splash {
    pub fn new(setting: Setting, level: Level) -> Self {
        Self {
            style: Style::of(level),
            started: None,
            done: setting == Setting::Off,
            ready: Vec::new(),
        }
    }

    pub fn style(&self) -> Style {
        self.style
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
        self.done = true;
        was
    }

    /// The runner's frame tick: `Some` while it shows or may.
    pub fn next_frame(&self) -> Option<Duration> {
        (!self.done).then_some(FRAME)
    }

    /// The frame at `now` on the runner's clock: `screen_ready` when the
    /// first screen can be drawn, `parts` each part's name and whether it
    /// is ready now, in the order shown.
    pub fn step(&mut self, now: Duration, screen_ready: bool, parts: &[(String, bool)]) -> Step {
        if self.done {
            return Step::Hidden;
        }
        let started = *self.started.get_or_insert(now);
        let at = now.saturating_sub(started).as_millis().min(u32::MAX as u128) as u32;
        for (name, ready) in parts {
            match self.ready.iter_mut().find(|(n, _)| n == name) {
                Some((_, since)) => {
                    if *ready && since.is_none() {
                        *since = Some(at);
                    }
                }
                None => self.ready.push((name.clone(), ready.then_some(at))),
            }
        }
        let scene = || Scene {
            style: self.style,
            at,
            parts: parts
                .iter()
                .map(|(name, _)| Part {
                    name: name.clone(),
                    ready_at: self.ready.iter().find(|(n, _)| n == name).and_then(|(_, s)| *s),
                })
                .collect(),
        };
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
                let hold = if self.style == Style::Full { HOLD_FULL_MS } else { HOLD_SUBTLE_MS };
                if screen_ready && at >= hold {
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
}

/// The splash as Ink draws it: every row of the screen as runs across its
/// width, colours written as Ink's `color` takes them (`#rrggbb`,
/// `ansi256(n)`, a terminal colour's name, `""` for the terminal's own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SplashView {
    pub width: u16,
    pub height: u16,
    pub rows: Vec<Vec<SplashRun>>,
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

/// The colours the splash draws with, from the theme.
#[derive(Debug, Clone, Copy)]
struct Inks {
    tier: Tier,
    base: Option<Color>,
    /// The mark's ramp, left to right; one colour below full colour.
    ramp: [Option<Color>; 3],
    cursor: Option<Color>,
    muted: Option<Color>,
    secondary: Option<Color>,
}

impl Inks {
    fn of(theme: &Theme) -> Self {
        let fg = |s| theme.s(s).fg;
        let tier = theme.tier();
        let ramp = if tier == Tier::TrueColor {
            [fg(Slot::GlowPurple), fg(Slot::AccentTertiary), fg(Slot::GlowCyan)]
        } else {
            [fg(Slot::AccentPrimary); 3]
        };
        Self {
            tier,
            base: fg(Slot::BgBase),
            ramp,
            cursor: fg(Slot::TextPrimary),
            muted: fg(Slot::TextMuted),
            secondary: fg(Slot::TextSecondary),
        }
    }

    /// No colour at all: the mark and lit names are bold instead.
    fn mono(&self) -> bool {
        self.tier == Tier::Mono
    }

    /// The ramp at `u` (0 at the left, 1 at the right).
    fn ramp_at(&self, u: f32) -> Option<Color> {
        let u = u.clamp(0.0, 1.0) * 2.0;
        if u <= 1.0 { mix(self.ramp[0], self.ramp[1], u) } else { mix(self.ramp[1], self.ramp[2], u - 1.0) }
    }
}

/// `a` to `b` by `k` (0 to 1): channel by channel between two full
/// colours; otherwise the nearer of the two.
fn mix(a: Option<Color>, b: Option<Color>, k: f32) -> Option<Color> {
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

fn ease_out(p: f32) -> f32 {
    let p = p.clamp(0.0, 1.0);
    1.0 - (1.0 - p) * (1.0 - p)
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

/// What a cell of the mark holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkCell {
    Blank,
    /// Part of the letters.
    Letter(char),
    /// Part of the cursor.
    Cursor(char),
}

/// The mark as cells, `MARK_H` rows of `MARK_W`: two pixel rows a cell.
pub fn mark_cells() -> Vec<Vec<MarkCell>> {
    let px: Vec<Vec<char>> = BITMAP.iter().map(|r| r.chars().collect()).collect();
    (0..MARK_H as usize)
        .map(|row| {
            (0..MARK_W as usize)
                .map(|col| {
                    let top = px[row * 2][col];
                    let bottom = px[row * 2 + 1][col];
                    let glyph = half_block(top != '.', bottom != '.');
                    if glyph == ' ' {
                        MarkCell::Blank
                    } else if top == 'C' || bottom == 'C' {
                        MarkCell::Cursor(glyph)
                    } else {
                        MarkCell::Letter(glyph)
                    }
                })
                .collect()
        })
        .collect()
}

/// One cell of the screen being laid out.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Cell {
    ch: String,
    fg: Option<Color>,
    bold: bool,
}

/// The splash at `scene`, `width` × `height` cells; `None` where it is not
/// drawn at all (narrower than [`MIN_WIDTH`], or under two rows).
pub fn frame(theme: &Theme, scene: &Scene, width: u16, height: u16) -> Option<Frame> {
    if width < MIN_WIDTH || height < 2 {
        return None;
    }
    let inks = Inks::of(theme);
    let blank = Cell { ch: " ".into(), fg: None, bold: false };
    let mut grid = vec![vec![blank; width as usize]; height as usize];
    let with_mark = height >= FULL_HEIGHT;
    let used = if with_mark { FULL_HEIGHT } else { 2 };
    let top = (height - used) / 2;
    let at = scene.at as f32;
    let full = scene.style == Style::Full;

    if with_mark {
        let x0 = (width - MARK_W) / 2;
        // How far the assemble is: 1 when done, and always at subtle and static.
        let p = if full { ease_out(at / ASSEMBLE_MS as f32) } else { 1.0 };
        let front = p * (MARK_W as f32 + EDGE);
        // The ramp slides in from the left while the columns light.
        let slide = (1.0 - p) * 0.5;
        let blink_on = scene.style == Style::Static || (scene.at / BLINK_MS).is_multiple_of(2);
        for (row, cells) in mark_cells().iter().enumerate() {
            for (col, cell) in cells.iter().enumerate() {
                let k = ((front - col as f32) / EDGE).clamp(0.0, 1.0);
                let (glyph, ink) = match *cell {
                    MarkCell::Blank => continue,
                    MarkCell::Letter(g) => {
                        // Across the letters (the cursor is not on the ramp), so the last one is cyan.
                        let u = col as f32 / LETTERS_LAST as f32 - slide;
                        (g, inks.ramp_at(u))
                    }
                    MarkCell::Cursor(_) if !blink_on => continue,
                    MarkCell::Cursor(g) => (g, inks.cursor),
                };
                if k <= 0.0 {
                    continue;
                }
                grid[(top + row as u16) as usize][(x0 + col as u16) as usize] = Cell {
                    ch: glyph.to_string(),
                    fg: mix(inks.base, ink, k),
                    bold: inks.mono(),
                };
            }
        }
    }

    // The two lines of text: the parts' names, then the tagline.
    let names_y = top + if with_mark { MARK_H + 1 } else { 0 };
    let reveal = if full { (at - ASSEMBLE_MS as f32) / TAGLINE_MS as f32 } else { 1.0 };
    let (light_from, stagger) = match scene.style {
        Style::Full => (LIGHT_FULL_MS, STAGGER_FULL_MS),
        _ => (LIGHT_SUBTLE_MS, STAGGER_SUBTLE_MS),
    };
    let mut names: Vec<(String, Option<Color>, bool)> = Vec::new();
    let n = scene.parts.len();
    for (i, part) in scene.parts.iter().enumerate() {
        if i > 0 {
            names.push((SEPARATOR.into(), inks.muted, false));
        }
        // Lit: its place in the ramp, once it is ready and its turn came.
        let lit = match part.ready_at {
            None => 0.0,
            Some(_) if scene.style == Style::Static => 1.0,
            Some(ready) => {
                let from = ready.max(light_from + stagger * i as u32);
                (scene.at.saturating_sub(from) as f32 / LIGHT_MS as f32).min(1.0)
                    * (scene.at >= from) as u8 as f32
            }
        };
        let own = inks.ramp_at(if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 });
        names.push((part.name.clone(), mix(inks.muted, own, lit), inks.mono() && lit >= 1.0));
    }
    let lines = [names, vec![(TAGLINE.to_string(), inks.secondary, false)]];
    for (dy, line) in lines.iter().enumerate() {
        let y = names_y + dy as u16;
        if y >= height {
            break;
        }
        let total: usize = line.iter().map(|(t, _, _)| crate::ui::width(t)).sum();
        let room = width as usize;
        let start = room.saturating_sub(total) / 2;
        let span = total.min(room) as f32;
        let mut x = start;
        for (text, ink, bold) in line {
            for c in text.chars() {
                let w = c.width().unwrap_or(0);
                if w == 0 {
                    continue;
                }
                if x + w > room {
                    break;
                }
                // The read-in: a soft edge crossing the line.
                let k = ((reveal * (span + EDGE) - (x - start) as f32) / EDGE).clamp(0.0, 1.0);
                if k > 0.0 {
                    grid[y as usize][x] = Cell { ch: c.to_string(), fg: mix(inks.base, *ink, k), bold: *bold };
                    if w == 2 && x + 1 < room {
                        grid[y as usize][x + 1] = Cell { ch: String::new(), fg: None, bold: false };
                    }
                }
                x += w;
            }
        }
    }

    let rows = grid
        .into_iter()
        .map(|cells| {
            let mut runs: Vec<Run> = Vec::new();
            for c in cells {
                let fg = if c.ch == " " { None } else { c.fg };
                let bold = c.ch != " " && c.bold;
                match runs.last_mut() {
                    Some(r) if r.fg == fg && r.bold == bold => r.text.push_str(&c.ch),
                    _ => runs.push(Run { text: c.ch, fg, bg: inks.base, bold }),
                }
            }
            runs
        })
        .collect();
    Some(Frame { width, height, rows })
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
                        fg: colour_word(r.fg),
                        bg: colour_word(r.bg),
                        bold: r.bold,
                    })
                    .collect()
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests;
