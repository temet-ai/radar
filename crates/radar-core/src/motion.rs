//! Motion: short effects over what was drawn, tied to what happened.
//!
//! The host says what happened, as a [`Cue`]: a screen switched, an item's
//! page opened, a menu or confirm appeared, a message came or went, a part's
//! rows changed in a sync. While a frame is drawn, the shell and the parts
//! mark where things are ([`region`], [`row`], [`loading`]). The runner's
//! [`Engine`] turns cues into effects (built with `tachyonfx`), binds each
//! to its place in the frame, and applies them over the frame just drawn.
//!
//! Parts never use `tachyonfx` themselves. A part:
//! - reports what a sync changed with `Effect::Changed(Changes)` (see
//!   [`Changes::between`]), keyed by the row's subject (`kind:id`);
//! - marks where those rows are while it draws, with [`row`];
//! - marks a panel that waits for its first data, with [`loading`].
//!
//! What the engine keeps to:
//! - Every effect is short (100 to 450 ms) and time-based; the clock is
//!   injectable ([`Clock`], [`ManualClock`]) so tests are exact.
//! - Input wins: a key, click or paste drops what is running
//!   ([`Engine::input`]), and the key acts at once.
//! - Drawing is untouched: effects run in the runner over a frame already
//!   drawn, so tests and snapshots draw without them; `off` draws exactly
//!   what radar draws without motion, and so does the frame after an effect.
//! - The runner redraws on a frame tick only while [`Engine::running`].

pub mod transition;
pub mod frames;
mod interaction;

use std::cell::{Cell as StdCell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::Rc;
use std::time::Duration;

use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::Color;
use tachyonfx::{Effect as Fx, Interpolatable, Interpolation, Motion as Way, fx};

use crate::design::theme::{Slot, Tier};
use crate::theme::Theme;

/// How much motion: `[theme] motion = "full" | "subtle" | "off"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Every effect.
    Full,
    /// Screen switches, menus and confirms, and the glow of synced rows.
    #[default]
    Subtle,
    /// None at all: radar draws as it does without motion.
    Off,
}

impl Level {
    pub const ALL: [Level; 3] = [Level::Full, Level::Subtle, Level::Off];

    /// The config word; anything unknown is the default, `subtle`.
    pub fn parse(word: &str) -> Self {
        match word.trim() {
            "full" => Self::Full,
            "off" | "none" => Self::Off,
            _ => Self::Subtle,
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Subtle => "subtle",
            Self::Off => "off",
        }
    }

    /// What it shows, in a few words.
    pub fn about(self) -> &'static str {
        match self {
            Self::Full => "everything: also pages, new rows, messages and loading",
            Self::Subtle => "switching screens, menus, and rows a sync changed",
            Self::Off => "no motion at all",
        }
    }
}

/// Why motion is off here whatever the setting says: over SSH (every frame
/// crosses the network), or a terminal not drawing in full colour (the
/// effects blend colours, which only full colour shows as meant).
pub fn held_back(tier: Tier, ssh: bool) -> Option<&'static str> {
    if ssh {
        Some("over SSH")
    } else if tier != Tier::TrueColor {
        Some("the terminal is not drawing in full colour")
    } else {
        None
    }
}

/// The level that runs: the setting, unless [`held_back`].
pub fn effective(level: Level, tier: Tier, ssh: bool) -> Level {
    if held_back(tier, ssh).is_some() {
        Level::Off
    } else {
        level
    }
}

/// The rows a sync changed, by key: ones that were there and differ, and
/// ones that were not there. Keys are the row's subject, `kind:id`, so two
/// parts' keys never meet.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    pub changed: BTreeSet<String>,
    pub added: BTreeSet<String>,
}

impl Changes {
    /// Compare rows before and after a sync. With nothing before (the first
    /// data), nothing is new: a list that fills is not a list that changed.
    pub fn between<T: PartialEq>(
        before: impl IntoIterator<Item = (String, T)>,
        after: impl IntoIterator<Item = (String, T)>,
    ) -> Self {
        let before: HashMap<String, T> = before.into_iter().collect();
        let mut changes = Self::default();
        if before.is_empty() {
            return changes;
        }
        for (key, now) in after {
            match before.get(&key) {
                Some(was) if *was == now => {}
                Some(_) => {
                    changes.changed.insert(key);
                }
                None => {
                    changes.added.insert(key);
                }
            }
        }
        changes
    }

    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.added.is_empty()
    }
}

/// What happened, for the engine to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cue {
    /// Another screen (a tab's key, settings, a show across parts).
    Screen,
    /// An item's page opened on the same screen.
    Page,
    /// A menu or a confirm appeared.
    Popup,
    /// A message appeared on the status line.
    Message,
    /// The message left the status line.
    MessageGone,
    /// A part's sync changed these rows.
    Changed(Changes),
}

impl Cue {
    /// Whether it shows at `level`: `subtle` has screen switches, popups
    /// and the sync glow; `full` everything.
    pub fn runs_at(&self, level: Level) -> bool {
        match level {
            Level::Off => false,
            Level::Full => true,
            Level::Subtle => matches!(self, Self::Screen | Self::Popup | Self::Changed(_)),
        }
    }
}

/// A place the shell marks while drawing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Between the top bar and the status line.
    Body,
    /// The menu or confirm on screen.
    Popup,
    /// The message on the status line.
    Message,
}

/// Where things were drawn in the frame being drawn.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Marks {
    pub context: Option<transition::MotionContext>,
    pub fold: Option<transition::MotionFold>,
    pub frames: Vec<frames::FrameMark>,
    pub trees: Vec<frames::TreeMark>,
    pub body: Option<Rect>,
    pub popup: Option<Rect>,
    pub message: Option<Rect>,
    pub rows: Vec<(String, Rect)>,
    pub loading: Vec<Rect>,
}

thread_local! {
    static MARKS: RefCell<Marks> = RefCell::default();
}

/// Start a frame's marks afresh (the shell does, before drawing).
pub fn begin() {
    MARKS.with(|m| *m.borrow_mut() = Marks::default());
}

/// The marks of the frame just drawn.
pub fn take() -> Marks {
    MARKS.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Mark one of the shell's places.
pub fn region(region: Region, rect: Rect) {
    MARKS.with(|m| {
        let mut m = m.borrow_mut();
        let slot = match region {
            Region::Body => &mut m.body,
            Region::Popup => &mut m.popup,
            Region::Message => &mut m.message,
        };
        *slot = Some(rect);
    });
}

/// The key a row goes by in [`Changes`] and [`row`]: its subject's kind and
/// id, `kind:id` (`shelf:A-12`), so two parts' keys never meet.
pub fn key(kind: &str, id: &str) -> String {
    format!("{kind}:{id}")
}

/// Mark where a row was drawn, under the key a sync reports it by
/// ([`Changes`]). Mark every visible row; one a sync changed glows there.
pub fn row(key: impl Into<String>, rect: Rect) {
    MARKS.with(|m| m.borrow_mut().rows.push((key.into(), rect)));
}

/// Mark a panel that waits for its first data: at `full` it shimmers
/// while it is marked.
pub fn loading(rect: Rect) {
    MARKS.with(|m| m.borrow_mut().loading.push(rect));
}

/// Where effects get the time from.
pub trait Clock {
    /// Time since some fixed start.
    fn now(&self) -> Duration;
}

/// The real clock.
pub struct SystemClock(std::time::Instant);

impl Default for SystemClock {
    fn default() -> Self {
        Self(std::time::Instant::now())
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// A clock that moves only when told, for tests. Clones share the time.
#[derive(Debug, Clone, Default)]
pub struct ManualClock(Rc<StdCell<u64>>);

impl ManualClock {
    pub fn advance(&self, ms: u64) {
        self.0.set(self.0.get() + ms);
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.get())
    }
}

/// How long each effect runs, in ms. A screen switch is the shortest: it
/// happens most, and the new screen's words read from its first frame.
pub const SCREEN_MS: u32 = 100;
/// How far from the background toward the text a screen switch's words
/// start: dimmed, never invisible.
pub const SCREEN_FROM: f32 = 0.5;
pub const PAGE_MS: u32 = 300;
pub const POPUP_MS: u32 = 180;
pub const MESSAGE_MS: u32 = 250;
pub const MESSAGE_GONE_MS: u32 = 200;
pub const GLOW_MS: u32 = 450;
pub const ADDED_MS: u32 = 320;
/// One pass of the loading shimmer; it repeats while the panel waits.
pub const SHIMMER_MS: u32 = 1400;
/// How often the runner redraws while an effect runs.
pub const FRAME: Duration = Duration::from_millis(30);
/// A frame slower than this to draw, three times in a row while effects
/// run, turns motion off for the session.
const SLOW: Duration = Duration::from_millis(60);

/// The theme's colours the effects blend with.
#[derive(Debug, Clone, Copy)]
struct Ink {
    base: Color,
    surface: Color,
    overlay: Color,
    glow: Color,
    text: Color,
}

impl Ink {
    fn of(theme: &Theme) -> Self {
        let bg = |s| theme.bg(s).bg.unwrap_or(Color::Reset);
        let fg = |s| theme.s(s).fg.unwrap_or(Color::Reset);
        Self {
            base: bg(Slot::BgBase),
            surface: bg(Slot::BgSurface),
            overlay: bg(Slot::BgOverlay),
            glow: fg(Slot::AccentPrimary),
            text: fg(Slot::TextPrimary),
        }
    }
}

/// Where a running effect applies, found again in every frame's marks.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Body,
    Popup,
    Message,
    Row(String),
    Area(Rect),
}

impl Target {
    fn find(&self, marks: &Marks) -> Option<Rect> {
        match self {
            Self::Body => marks.body,
            Self::Popup => marks.popup,
            Self::Message => marks.message,
            Self::Row(key) => marks.rows.iter().find(|(k, _)| k == key).map(|(_, r)| *r),
            Self::Area(r) => Some(*r),
        }
    }
}

struct Running {
    target: Target,
    fx: Fx,
    /// A loading shimmer: it runs while its panel is marked, and input
    /// does not stop it.
    shimmer: bool,
}

/// Runs the effects: cues start them, each frame applies them.
pub struct Engine {
    frames: frames::FrameMotion,
    level: Level,
    clock: Box<dyn Clock>,
    /// When the last frame with effects was drawn.
    last: Option<Duration>,
    running: Vec<Running>,
    /// The last effect ended in the frame just drawn: one more plain frame
    /// follows, so what stays on screen is exactly what radar drew.
    settle: bool,
    /// The message as last drawn (where, and its cells), for its fade out.
    message: Option<(Rect, Vec<Cell>)>,
    /// Slow frames in a row; three turn motion off.
    slow: u8,
    /// The terminal was too slow: off for the session.
    too_slow: bool,
}

impl Engine {
    pub fn new(level: Level, clock: Box<dyn Clock>) -> Self {
        Self {
            frames: frames::FrameMotion::default(),
            level,
            clock,
            last: None,
            running: Vec::new(),
            settle: false,
            message: None,
            slow: 0,
            too_slow: false,
        }
    }

    pub fn level(&self) -> Level {
        self.level
    }

    /// A new setting. Off drops everything at once.
    pub fn set_level(&mut self, level: Level) {
        self.frames.clear();
        self.level = if self.too_slow { Level::Off } else { level };
        if self.level == Level::Off {
            self.running.clear();
            self.last = None;
            self.message = None;
        }
    }

    /// Whether the runner should draw again soon: an effect runs, or the
    /// plain frame after one is still to come.
    pub fn running(&self) -> bool {
        !self.running.is_empty() || self.settle || self.frames.running()
    }

    /// The runner's frame tick: `Some` only while [`Engine::running`].
    pub fn next_frame(&self) -> Option<Duration> {
        self.running().then_some(FRAME)
    }

    /// Someone pressed a key, clicked or pasted: what runs is dropped and
    /// the next frame is plain, so nothing stands in the way. A loading
    /// shimmer goes on (it says the panel still waits).
    pub fn input(&mut self) {
        self.frames.input();
        let before = self.running.len();
        self.running.retain(|r| r.shimmer);
        if self.running.len() != before {
            self.settle = true;
        }
    }

    /// Background work cannot borrow a locally consumed input's pending start.
    pub fn background(&mut self) { self.frames.background(); }

    /// Start what `cue` shows at this level.
    pub fn cue(&mut self, cue: Cue, theme: &Theme) {
        if !cue.runs_at(self.level) {
            return;
        }
        let ink = Ink::of(theme);
        let ease = Interpolation::QuadOut;
        match cue {
            Cue::Screen => {
                // The terminal's own text colour is read as tachyonfx reads a
                // Reset foreground, as the fade does for each cell.
                let text = match ink.text {
                    Color::Reset => Color::Rgb(192, 192, 192),
                    c => c,
                };
                let from = ink.base.lerp(&text, SCREEN_FROM);
                self.replace(Target::Body, fx::fade_from_fg(from, (SCREEN_MS, ease)));
            }
            Cue::Page => self.replace(
                Target::Body,
                fx::sweep_in(Way::LeftToRight, 24, 0, ink.base, (PAGE_MS, ease)),
            ),
            Cue::Popup => self.replace(
                Target::Popup,
                fx::sweep_in(Way::UpToDown, 3, 0, ink.base, (POPUP_MS, ease)),
            ),
            Cue::Message => self.replace(
                Target::Message,
                fx::sweep_in(Way::LeftToRight, 8, 0, ink.overlay, (MESSAGE_MS, ease)),
            ),
            Cue::MessageGone => {
                if let Some((rect, cells)) = self.message.take() {
                    self.replace(Target::Area(rect), fade_out(cells, rect.width, ink.overlay));
                }
            }
            Cue::Changed(changes) => {
                for key in changes.changed {
                    self.replace(Target::Row(key), glow(ink));
                }
                for key in changes.added {
                    let fx = if self.level == Level::Full {
                        fx::sweep_in(Way::LeftToRight, 12, 0, ink.surface, (ADDED_MS, ease))
                    } else {
                        glow(ink)
                    };
                    self.replace(Target::Row(key), fx);
                }
            }
        }
    }

    fn replace(&mut self, target: Target, fx: Fx) {
        self.running.retain(|r| r.target != target);
        self.running.push(Running {
            target,
            fx,
            shimmer: false,
        });
    }

    /// Apply what runs to the frame just drawn into `buf`, using the marks
    /// made while drawing it.
    pub fn draw(&mut self, buf: &mut Buffer, theme: &Theme) {
        let marks = take();
        self.settle = false;
        if self.level == Level::Off {
            return;
        }
        if self.level == Level::Full {
            self.frames.draw(buf, theme, &marks, self.clock.now().as_millis());
            self.running.retain(|r| {
                !r.shimmer || matches!(r.target, Target::Area(a) if marks.loading.contains(&a))
            });
            for &area in &marks.loading {
                let known = self
                    .running
                    .iter()
                    .any(|r| r.shimmer && r.target == Target::Area(area));
                if !known {
                    self.running.push(Running {
                        target: Target::Area(area),
                        fx: shimmer(Ink::of(theme)),
                        shimmer: true,
                    });
                }
            }
            self.message = marks.message.map(|r| {
                let r = r.intersection(buf.area);
                (r, cells(buf, r))
            });
        }
        if self.running.is_empty() {
            self.last = None;
            return;
        }
        let now = self.clock.now();
        let dt = self.last.map_or(Duration::ZERO, |l| now.saturating_sub(l));
        let mut kept = Vec::new();
        for mut r in self.running.drain(..) {
            let Some(area) = r.target.find(&marks).map(|a| a.intersection(buf.area)) else {
                continue;
            };
            if area.is_empty() {
                continue;
            }
            r.fx.process(dt.into(), buf, area);
            if !r.fx.done() {
                kept.push(r);
            }
        }
        self.running = kept;
        self.settle = self.running.is_empty();
        self.last = (!self.running.is_empty()).then_some(now);
    }

    /// How long the frame took to reach the terminal. Three slow ones in a
    /// row while effects run turn motion off for the session; the reason is
    /// returned once, for the status line.
    pub fn drew_in(&mut self, took: Duration) -> Option<&'static str> {
        if !self.running() || self.too_slow {
            return None;
        }
        if took < SLOW {
            self.slow = 0;
            return None;
        }
        self.slow += 1;
        if self.slow < 3 {
            return None;
        }
        self.too_slow = true;
        self.set_level(Level::Off);
        Some("motion is off: this terminal is slow to draw")
    }
}

/// The cells of `area`, row by row.
fn cells(buf: &Buffer, area: Rect) -> Vec<Cell> {
    area.positions().map(|p| buf[p].clone()).collect()
}

/// A background pulse in the accent colour that fades to the row's own.
fn glow(ink: Ink) -> Fx {
    fx::effect_fn(ink.glow, (GLOW_MS, Interpolation::SineOut), |glow, ctx, cells| {
        let k = (1.0 - ctx.alpha()) * 0.45;
        for (_, cell) in cells {
            let bg = cell.bg;
            cell.set_bg(bg.lerp(glow, k));
        }
    })
}

/// The cells that were there fade into `to`; then what is there now shows.
fn fade_out(was: Vec<Cell>, width: u16, to: Color) -> Fx {
    fx::effect_fn(
        (was, width),
        (MESSAGE_GONE_MS, Interpolation::QuadIn),
        move |(was, width), ctx, cells| {
            let a = ctx.alpha();
            if a >= 1.0 {
                return;
            }
            let area = ctx.area;
            for (pos, cell) in cells {
                let i = (pos.y - area.y) as usize * *width as usize + (pos.x - area.x) as usize;
                if let Some(old) = was.get(i) {
                    *cell = old.clone();
                    cell.set_fg(old.fg.lerp(&to, a));
                }
            }
        },
    )
}

/// A soft band of light that crosses a waiting panel, again and again.
fn shimmer(ink: Ink) -> Fx {
    fx::repeating(fx::effect_fn(
        (ink.overlay, ink.text),
        (SHIMMER_MS, Interpolation::Linear),
        |(light, text), ctx, cells| {
            let area = ctx.area;
            let band = (area.width as f32 / 4.0).clamp(6.0, 24.0);
            let span = area.width as f32 + area.height as f32 * 2.0 + band * 2.0;
            let centre = ctx.alpha() * span - band;
            for (pos, cell) in cells {
                let x = (pos.x - area.x) as f32 + (pos.y - area.y) as f32 * 2.0;
                let d = (x - centre).abs();
                if d >= band {
                    continue;
                }
                let k = (1.0 - d / band) * 0.5;
                let bg = cell.bg;
                cell.set_bg(bg.lerp(light, k));
                if cell.symbol() != " " {
                    let fg = cell.fg;
                    cell.set_fg(fg.lerp(text, k));
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests;
