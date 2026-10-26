//! The start line: one line under the mark at every start, picked by rules.
//!
//! [`pick`] is pure. It takes the dictionary ([`Dictionary`], the shipped
//! `assets/start-lines.toml` with the user's own file on top), the user's
//! `[start_lines]` config ([`UserConfig`]), what is remembered between starts
//! ([`State`]), the facts radar holds ([`Facts`]), the moment and the terminal
//! ([`Context`]) and a source of chance ([`Rng`]), and answers with the line
//! ([`Picked`]) or nothing. Reading and writing the files is in the thin
//! functions at the end ([`load_state`], [`save_state`], [`load_dictionary`]);
//! none of them can fail a start.
//!
//! The order, the first that gives a line wins:
//!
//! 1. `plain`: something failed. The first eligible line, no chance, no joke.
//! 2. `first_run`: the first start ever ([`Context::first_run`]).
//! 3. `day`: the first start of a local day only.
//! 4. echo: when the sweep rolled a rare echo ([`Context::echo_rare`]) the
//!    rare line that asks for it shows, and there is no event roll.
//! 5. `animated` (an event): chance, a month apart, motion full, never while
//!    a pipeline fails. Only the animations in [`BUILT_ANIMS`] exist; a line
//!    with another is ineligible.
//! 6. `rare`: chance, a week apart.
//! 7. the draw: weights over the groups that have an eligible line, each
//!    group a shuffle bag, never more than `same_group_max` starts running
//!    from one group.
//!
//! A line is eligible when its emoji can draw, its animation exists and can
//! run, every `when` holds and every placeholder can be filled. A fact radar
//! does not hold, or holds from more than a day ago, makes a condition false.
//!
//! What a placeholder means:
//!
//! - `{name}` is the fact of that name (a string, cut to 16 cells with an
//!   ellipsis). `{time}` is the clock, `HH:MM`, when no fact is called that.
//! - `{n}`, `{m}`, `{days}` are counts (capped at 999) of a fact the line's
//!   own `when` names: the first fact in it that is not the clock, but `n`
//!   leaves out a fact whose name has `days` in it when another is there, and
//!   `days` prefers one. `{m}` is the right side of a `fact < fact`. A fact
//!   that is a bare number (`install.anniversary`) counts as itself.
//!   `{starts}` is the start count with thousands separators; `{today}` the
//!   starts today.
//! - `{n:noun}` and `{n:one|many}` are a count and its noun: `1 branch`,
//!   `3 branches` (`s` is added when only one form is given; one is "one").
//! - `{key:x}` is a key, a span of its own for the renderer to colour.
//!
//! Facts whose names are held by [`pick`] itself, from the state and the
//! clock, and which a caller's [`Facts`] of the same name override:
//! `start.count`, `start.first_today`, `start.today`, `start.gap_days`,
//! `install.days`, `install.anniversary` (the years), `hour`, `minute`,
//! `weekday` (`mon`..`sun`), `date` (`MM-DD`), `month_day`, `day_of_year`,
//! `quarter_days_left`, `splash.echo_rare`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ui::{take, width};

/// The dictionary that ships inside the binary.
pub const SHIPPED: &str = include_str!("../assets/start-lines.toml");
/// A start line is never wider than this, in cells.
pub const MAX_CELLS: usize = 56;
/// The animations that exist. A line with any other is never eligible.
pub const BUILT_ANIMS: [&str; 7] = ["fire", "snow", "confetti", "tumbleweed", "duck", "ufo", "steam"];

const NAME_CELLS: usize = 16;
const COUNT_CAP: i64 = 999;
const STARTS_CAP: i64 = 99_999;
const FACT_MAX_AGE_SECS: i64 = 86_400;
const TIP_RETIRES_AFTER: u32 = 3;
const EARLY_STARTS: i64 = 30;
const RECENT_KEPT: usize = 8;
const MINE_WEIGHT: u32 = 20;
const ECHO_FACT: &str = "splash.echo_rare";
/// Groups that are steps of the order, not draws.
const STEPS: [&str; 5] = ["plain", "first_run", "day", "animated", "rare"];
const COUNT_NAMES: [&str; 5] = ["n", "m", "days", "today", "starts"];
const CLOCK_FACTS: [&str; 7] = [
    "hour",
    "minute",
    "weekday",
    "date",
    "month_day",
    "day_of_year",
    "quarter_days_left",
];
/// The draw's groups in the order the weights are walked.
const GROUP_ORDER: [&str; 8] = [
    "scope", "work", "facts", "tips", "agents", "time", "glyph", "mine",
];
const WEEKDAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

// ------------------------------------------------------------------- chance

/// A source of chance. Tests script it; radar uses [`SplitMix64`].
pub trait Rng {
    fn next_u64(&mut self) -> u64;

    /// A number in `0..n`; 0 when `n` is 0 or 1, without using chance.
    fn below(&mut self, n: usize) -> usize {
        if n <= 1 {
            return 0;
        }
        ((u128::from(self.next_u64()) * n as u128) >> 64) as usize
    }

    /// A number in `0.0..1.0`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// A small, fast generator (SplitMix64) for choices that need no secrecy.
#[derive(Debug, Clone)]
pub struct SplitMix64(u64);

impl SplitMix64 {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// Seeded from the system's randomness (through the v4 id).
    pub fn from_entropy() -> Self {
        let bits = uuid::Uuid::new_v4().as_u128();
        Self::new((bits ^ (bits >> 64)) as u64)
    }
}

impl Rng for SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn shuffle<T>(items: &mut [T], rng: &mut impl Rng) {
    for i in (1..items.len()).rev() {
        items.swap(i, rng.below(i + 1));
    }
}

// -------------------------------------------------------------------- dates

/// A calendar date in the user's time zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// Days since 1970-01-01.
    pub fn days(self) -> i64 {
        let (y, m, d) = (
            i64::from(self.year),
            i64::from(self.month),
            i64::from(self.day),
        );
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Self { year, month, day }
    }

    /// `mon`..`sun`.
    pub fn weekday(self) -> &'static str {
        // 1970-01-01 was a Thursday.
        WEEKDAYS[(self.days() + 3).rem_euclid(7) as usize]
    }

    fn day_of_year(self) -> i64 {
        self.days() - Self::new(self.year, 1, 1).days() + 1
    }

    /// Days from this one to the last day of its quarter.
    fn quarter_days_left(self) -> i64 {
        let end_month = (self.month - 1) / 3 * 3 + 3;
        let next = if end_month == 12 {
            Self::new(self.year + 1, 1, 1)
        } else {
            Self::new(self.year, end_month + 1, 1)
        };
        next.days() - 1 - self.days()
    }

    /// `YYYY-MM-DD`.
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.splitn(3, '-');
        let year = parts.next()?.parse().ok()?;
        let month: u32 = parts.next()?.parse().ok()?;
        let day: u32 = parts.next()?.parse().ok()?;
        ((1..=12).contains(&month) && (1..=31).contains(&day)).then(|| Self::new(year, month, day))
    }
}

// ------------------------------------------------------------ facts, context

/// A fact's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Num(i64),
    Bool(bool),
    Str(String),
}

impl Value {
    fn num(&self) -> Option<i64> {
        match self {
            Self::Num(n) => Some(*n),
            Self::Bool(b) => Some(i64::from(*b)),
            Self::Str(s) => s.trim().parse().ok(),
        }
    }

    fn text(&self) -> String {
        match self {
            Self::Num(n) => n.to_string(),
            Self::Bool(b) => b.to_string(),
            Self::Str(s) => s.clone(),
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Self::Num(n) => *n != 0,
            Self::Bool(b) => *b,
            Self::Str(s) => !s.is_empty(),
        }
    }
}

macro_rules! value_from_num {
    ($($t:ty),*) => {$(
        impl From<$t> for Value {
            fn from(n: $t) -> Self {
                Self::Num(i64::from(n))
            }
        }
    )*};
}
value_from_num!(i32, i64, u32, u8);

impl From<usize> for Value {
    fn from(n: usize) -> Self {
        Self::Num(i64::try_from(n).unwrap_or(i64::MAX))
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Self::Bool(b)
    }
}
impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Self::Str(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Self::Str(s)
    }
}

/// What radar holds, by name, and when it was read. A fact held from more
/// than a day before [`Context::epoch`] is not held.
#[derive(Debug, Clone)]
pub struct Facts {
    /// Unix seconds the facts are as of, for every fact without its own time.
    pub as_of: i64,
    values: BTreeMap<String, (Value, Option<i64>)>,
}

impl Facts {
    pub fn new(as_of: i64) -> Self {
        Self {
            as_of,
            values: BTreeMap::new(),
        }
    }

    pub fn set(&mut self, name: &str, value: impl Into<Value>) -> &mut Self {
        self.values.insert(name.to_string(), (value.into(), None));
        self
    }

    /// A fact read at its own time.
    pub fn set_at(&mut self, name: &str, value: impl Into<Value>, as_of: i64) -> &mut Self {
        self.values
            .insert(name.to_string(), (value.into(), Some(as_of)));
        self
    }

    pub fn with(mut self, name: &str, value: impl Into<Value>) -> Self {
        self.set(name, value);
        self
    }
}

/// The moment and the terminal.
#[derive(Debug, Clone)]
pub struct Context {
    /// Unix seconds, UTC.
    pub epoch: i64,
    /// The local date and time of day.
    pub date: Date,
    pub hour: u32,
    pub minute: u32,
    /// Motion is `full` (events need it).
    pub motion_full: bool,
    /// The terminal can draw emoji.
    pub emoji_ok: bool,
    /// The sweep rolled a rare echo.
    pub echo_rare: bool,
    /// This is the first start ever.
    pub first_run: bool,
    /// How many times each action a tip teaches has been used.
    pub tip_uses: BTreeMap<String, u32>,
}

impl Context {
    /// `epoch` seen from a zone `offset` seconds ahead of UTC. Motion, emoji,
    /// echo and first run are off: set them.
    pub fn at(epoch: i64, offset: i64) -> Self {
        let local = epoch + offset;
        let secs = local.rem_euclid(86_400);
        Self {
            epoch,
            date: Date::from_days(local.div_euclid(86_400)),
            hour: (secs / 3600) as u32,
            minute: (secs % 3600 / 60) as u32,
            motion_full: false,
            emoji_ok: false,
            echo_rare: false,
            first_run: false,
            tip_uses: BTreeMap::new(),
        }
    }

    /// Now, in the local zone.
    pub fn now() -> Self {
        let epoch = crate::time::now_secs();
        Self::at(epoch, crate::time::local_offset(epoch))
    }

    fn clock(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }
}

// --------------------------------------------------------------- user config

/// The user's `[start_lines]` table in radar's config.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UserConfig {
    /// `false`: no line, no tip, no event.
    pub show: bool,
    /// `false`: no tips.
    pub tips: bool,
    /// Groups never drawn. `plain` cannot be turned off here (`show` does).
    pub off: Vec<String>,
    /// The user's own lines: group `mine`.
    pub mine: Vec<String>,
    /// Weights that replace the dictionary's, `mine` too.
    pub weights: BTreeMap<String, u32>,
}

impl Default for UserConfig {
    fn default() -> Self {
        Self {
            show: true,
            tips: true,
            off: Vec::new(),
            mine: Vec::new(),
            weights: BTreeMap::new(),
        }
    }
}

impl UserConfig {
    /// The user's own lines as dictionary lines. One that cannot show (a
    /// placeholder that does not exist, wider than [`MAX_CELLS`]) is left out
    /// and logged.
    fn mine_lines(&self, dict: &Dictionary) -> Vec<Line> {
        let mut lines = Vec::new();
        for text in &self.mine {
            let raw = RawLine {
                id: Some(format!("mine-{:08x}", fnv1a(text))),
                text: Some(text.clone()),
                ..RawLine::default()
            };
            match Line::compile(raw, "mine", &dict.widest) {
                Ok(line) => lines.push(line),
                Err(why) => crate::log::warn(format!("start lines: your line `{text}`: {why}")),
            }
        }
        lines
    }

    fn is_off(&self, dict: &Dictionary, group: &str) -> bool {
        group != "plain"
            && (self.off.iter().any(|g| g == group)
                || dict.off.contains(group)
                || (group == "tips" && !self.tips))
    }
}

fn fnv1a(text: &str) -> u32 {
    text.bytes().fold(0x811C_9DC5, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    })
}

// ---------------------------------------------------------------- conditions

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Eq,
    Ge,
    Le,
    Lt,
    Gt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Rhs {
    Num(i64),
    Str(String),
    Fact(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Set {
    Range(String, String),
    List(Vec<String>),
}

/// One condition of a line's `when`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Cond {
    /// The pseudo-fact: the terminal cannot draw emoji.
    NoEmoji,
    Bare(String),
    Cmp(String, Op, Rhs),
    In(String, Set),
}

fn is_fact_name(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
}

impl Cond {
    fn parse(src: &str) -> Result<Self, String> {
        let s = src.trim();
        let bad = |why: &str| format!("condition `{s}`: {why}");
        if s == "no_emoji" {
            return Ok(Self::NoEmoji);
        }
        if is_fact_name(s) {
            return Ok(Self::Bare(s.to_string()));
        }
        if let Some((lhs, rhs)) = s.split_once(" in ") {
            let lhs = lhs.trim();
            if !is_fact_name(lhs) {
                return Err(bad("the left side is not a fact name"));
            }
            let rhs = rhs.trim();
            let set = match rhs.split_once("..") {
                Some((a, b)) if !a.trim().is_empty() && !b.trim().is_empty() => {
                    Set::Range(a.trim().to_lowercase(), b.trim().to_lowercase())
                }
                Some(_) => return Err(bad("a range needs both ends")),
                None => Set::List(rhs.split(',').map(|i| i.trim().to_lowercase()).collect()),
            };
            return Ok(Self::In(lhs.to_string(), set));
        }
        for (token, op) in [
            (">=", Op::Ge),
            ("<=", Op::Le),
            ("==", Op::Eq),
            ("<", Op::Lt),
            (">", Op::Gt),
        ] {
            let Some((lhs, rhs)) = s.split_once(token) else {
                continue;
            };
            let (lhs, rhs) = (lhs.trim(), rhs.trim());
            if !is_fact_name(lhs) {
                return Err(bad("the left side is not a fact name"));
            }
            if rhs.is_empty() {
                return Err(bad("nothing on the right side"));
            }
            if rhs.contains(['<', '>', '=']) {
                return Err(bad("more than one comparison"));
            }
            let rhs = if let Ok(n) = rhs.parse() {
                Rhs::Num(n)
            } else if is_fact_name(rhs) && rhs.contains('.') {
                Rhs::Fact(rhs.to_string())
            } else {
                Rhs::Str(rhs.to_lowercase())
            };
            return Ok(Self::Cmp(lhs.to_string(), op, rhs));
        }
        Err(bad(
            "expected a fact, `fact OP value`, `fact in a..b` or `fact in a,b`",
        ))
    }

    fn holds(&self, held: &Held, ctx: &Context) -> bool {
        match self {
            Self::NoEmoji => !ctx.emoji_ok,
            Self::Bare(name) => held.get(name).is_some_and(Value::truthy),
            Self::Cmp(name, op, rhs) => {
                let Some(left) = held.get(name) else {
                    return false;
                };
                let right = match rhs {
                    Rhs::Num(n) => Value::Num(*n),
                    Rhs::Str(s) => Value::Str(s.clone()),
                    Rhs::Fact(other) => match held.get(other) {
                        Some(v) => v.clone(),
                        None => return false,
                    },
                };
                compare(left, *op, &right)
            }
            Self::In(name, set) => {
                let Some(value) = held.get(name) else {
                    return false;
                };
                let text = value.text().to_lowercase();
                match set {
                    Set::Range(a, b) => match (value.num(), a.parse::<i64>(), b.parse::<i64>()) {
                        (Some(v), Ok(a), Ok(b)) => a <= v && v <= b,
                        _ => *a <= text && text <= *b,
                    },
                    Set::List(items) => items.contains(&text),
                }
            }
        }
    }

    /// The facts the condition reads.
    fn facts(&self) -> Vec<&str> {
        match self {
            Self::NoEmoji => Vec::new(),
            Self::Bare(name) | Self::In(name, _) => vec![name],
            Self::Cmp(name, _, Rhs::Fact(other)) => vec![name, other],
            Self::Cmp(name, _, _) => vec![name],
        }
    }
}

fn compare(left: &Value, op: Op, right: &Value) -> bool {
    use std::cmp::Ordering;
    let order = match (left.num(), right.num()) {
        (Some(a), Some(b)) => a.cmp(&b),
        _ => left.text().to_lowercase().cmp(&right.text().to_lowercase()),
    };
    match op {
        Op::Eq => order == Ordering::Equal,
        Op::Ge => order != Ordering::Less,
        Op::Le => order != Ordering::Greater,
        Op::Lt => order == Ordering::Less,
        Op::Gt => order == Ordering::Greater,
    }
}

// ----------------------------------------------------------------- held facts

struct HeldFact {
    value: Value,
    /// From the caller's [`Facts`], not worked out here.
    caller: bool,
}

/// The facts of this start: worked out here, then the caller's over them.
struct Held {
    facts: BTreeMap<String, HeldFact>,
}

/// What the state said about today before this start was counted.
#[derive(Debug, Clone, Copy)]
struct Tick {
    first_today: bool,
    gap_days: i64,
}

impl Held {
    fn new(state: &State, tick: Tick, facts: &Facts, ctx: &Context) -> Self {
        let mut map = BTreeMap::new();
        let mut set = |name: &str, value: Value| {
            map.insert(
                name.to_string(),
                HeldFact {
                    value,
                    caller: false,
                },
            );
        };
        let today = ctx.date;
        set("start.count", Value::Num(state.starts as i64));
        set("start.first_today", Value::Bool(tick.first_today));
        set("start.today", Value::Num(i64::from(state.starts_today)));
        set("start.gap_days", Value::Num(tick.gap_days));
        if let Some(first) = state.first_day.as_deref().and_then(Date::parse) {
            set(
                "install.days",
                Value::Num((today.days() - first.days()).max(0)),
            );
            let years = i64::from(today.year - first.year);
            if tick.first_today
                && years >= 1
                && (first.month, first.day) == (today.month, today.day)
            {
                set("install.anniversary", Value::Num(years));
            }
        }
        set("hour", Value::Num(i64::from(ctx.hour)));
        set("minute", Value::Num(i64::from(ctx.minute)));
        set("weekday", Value::Str(today.weekday().to_string()));
        set(
            "date",
            Value::Str(format!("{:02}-{:02}", today.month, today.day)),
        );
        set("month_day", Value::Num(i64::from(today.day)));
        set("day_of_year", Value::Num(today.day_of_year()));
        set("quarter_days_left", Value::Num(today.quarter_days_left()));
        set(ECHO_FACT, Value::Bool(ctx.echo_rare));
        for (name, (value, at)) in &facts.values {
            if ctx.epoch - at.unwrap_or(facts.as_of) <= FACT_MAX_AGE_SECS {
                map.insert(
                    name.clone(),
                    HeldFact {
                        value: value.clone(),
                        caller: true,
                    },
                );
            }
        }
        Self { facts: map }
    }

    fn get(&self, name: &str) -> Option<&Value> {
        self.facts.get(name).map(|f| &f.value)
    }

    /// What a repeat of the same line is compared on: the caller's own facts,
    /// not the clock's or the state's.
    fn signature(&self, name: &str) -> Option<(String, String)> {
        let fact = self.facts.get(name)?;
        (fact.caller && !CLOCK_FACTS.contains(&name)).then(|| (name.to_string(), fact.value.text()))
    }
}

// ------------------------------------------------------------------- templates

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Key(String),
    /// `{name}` or `{name:one|many}`.
    Value {
        name: String,
        noun: Option<(String, String)>,
    },
}

fn parse_template(text: &str) -> Result<Vec<Piece>, String> {
    let mut pieces = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        if open > 0 {
            pieces.push(Piece::Text(rest[..open].to_string()));
        }
        let body_start = open + 1;
        // A key may be `}` itself no more than any other single character.
        let search_from = match rest[body_start..].strip_prefix("key:") {
            Some(key) => body_start + 4 + key.chars().next().map_or(0, char::len_utf8),
            None => body_start,
        };
        let Some(close) = rest[search_from..].find('}') else {
            return Err(format!("`{{` without a `}}` in `{text}`"));
        };
        let body = &rest[body_start..search_from + close];
        rest = &rest[search_from + close + 1..];
        if let Some(key) = body.strip_prefix("key:") {
            if key.is_empty() {
                return Err("`{key:}` names no key".into());
            }
            pieces.push(Piece::Key(key.to_string()));
            continue;
        }
        let (name, noun) = match body.split_once(':') {
            Some((name, noun)) => {
                let (one, many) = match noun.split_once('|') {
                    Some((one, many)) => (one.to_string(), many.to_string()),
                    None => (noun.to_string(), format!("{noun}s")),
                };
                if one.is_empty() || many.is_empty() {
                    return Err(format!("`{{{body}}}` has an empty noun"));
                }
                (name, Some((one, many)))
            }
            None => (body, None),
        };
        if !is_fact_name(name) {
            return Err(format!("`{{{body}}}` is not a placeholder"));
        }
        pieces.push(Piece::Value {
            name: name.to_string(),
            noun,
        });
    }
    if !rest.is_empty() {
        pieces.push(Piece::Text(rest.to_string()));
    }
    Ok(pieces)
}

/// Which fact each count placeholder of a line reads (see the module docs).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Binds {
    n: Option<String>,
    m: Option<String>,
    days: Option<String>,
}

impl Binds {
    fn of(when: &[Cond]) -> Self {
        let mut named: Vec<(&str, Option<&str>)> = Vec::new();
        for cond in when {
            let (name, other) = match cond {
                Cond::Bare(name) => (name.as_str(), None),
                Cond::Cmp(name, _, Rhs::Fact(other)) => (name.as_str(), Some(other.as_str())),
                Cond::Cmp(name, ..) | Cond::In(name, _) => (name.as_str(), None),
                Cond::NoEmoji => continue,
            };
            if !CLOCK_FACTS.contains(&name) && !named.iter().any(|(n, _)| *n == name) {
                named.push((name, other));
            }
        }
        let has_days = |name: &str| name.contains("days");
        let n = named
            .iter()
            .find(|(name, _)| !has_days(name))
            .or(named.first());
        let days = named
            .iter()
            .find(|(name, _)| has_days(name))
            .or(named.first());
        Self {
            n: n.map(|(name, _)| name.to_string()),
            m: n.and_then(|(_, other)| other.map(str::to_string)),
            days: days.map(|(name, _)| name.to_string()),
        }
    }

    /// The fact a count placeholder reads.
    fn fact(&self, name: &str) -> Option<&str> {
        match name {
            "starts" => Some("start.count"),
            "today" => Some("start.today"),
            "n" => self.n.as_deref(),
            "m" => self.m.as_deref(),
            "days" => self.days.as_deref(),
            _ => None,
        }
    }
}

// ----------------------------------------------------------------------- lines

/// One start line of the dictionary.
#[derive(Debug, Clone)]
pub struct Line {
    pub id: String,
    pub group: String,
    pub text: String,
    pub emoji: bool,
    pub anim: Option<String>,
    /// The action a tip teaches: the tip retires after it was used three times.
    pub used: Option<String>,
    /// Cells at its widest, every placeholder as wide as it gets.
    pub cells: usize,
    when: Vec<Cond>,
    pieces: Vec<Piece>,
    binds: Binds,
}

impl Line {
    fn compile(
        raw: RawLine,
        group: &str,
        widest: &BTreeMap<String, String>,
    ) -> Result<Self, String> {
        let id = raw.id.filter(|i| !i.is_empty()).ok_or("has no id")?;
        let text = raw.text.ok_or_else(|| format!("`{id}` has no text"))?;
        let at = |why: String| format!("`{id}`: {why}");
        let when = raw
            .when
            .iter()
            .map(|c| Cond::parse(c))
            .collect::<Result<Vec<_>, _>>()
            .map_err(at)?;
        let pieces = parse_template(&text).map_err(at)?;
        let binds = Binds::of(&when);
        let mut cells = 0;
        for piece in &pieces {
            cells += match piece {
                Piece::Text(t) | Piece::Key(t) => width(t),
                Piece::Value { name, noun } => {
                    let count = COUNT_NAMES.contains(&name.as_str());
                    let sample = match (name.as_str(), widest.get(name)) {
                        ("n" | "m" | "days", _) => "999".to_string(),
                        ("today", _) => "99".to_string(),
                        ("starts", _) => "99,999".to_string(),
                        (_, Some(sample)) => sample.clone(),
                        _ => {
                            return Err(at(format!(
                                "`{{{name}}}` is neither a count nor listed in [widest]"
                            )));
                        }
                    };
                    if noun.is_some() && !count {
                        return Err(at(format!("`{{{name}}}` is not a count and takes no noun")));
                    }
                    if count && binds.fact(name).is_none() {
                        return Err(at(format!("`{{{name}}}` needs a fact in `when` to count")));
                    }
                    width(&sample) + noun.as_ref().map_or(0, |(_, many)| 1 + width(many))
                }
            };
        }
        if cells > MAX_CELLS {
            return Err(at(format!(
                "is {cells} cells at its widest; the most is {MAX_CELLS}"
            )));
        }
        Ok(Self {
            id,
            group: group.to_string(),
            text,
            emoji: raw.emoji,
            anim: raw.anim,
            used: raw.used,
            cells,
            when,
            pieces,
            binds,
        })
    }
}

/// What a span of a start line is, for the renderer's colours.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    Text,
    /// A fact's value, a count and its noun.
    Value,
    /// A key.
    Key,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub text: String,
    pub kind: SpanKind,
}

/// The line that was picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Picked {
    pub id: String,
    pub group: String,
    pub spans: Vec<Span>,
    /// The animation that goes with the line (an event).
    pub anim: Option<String>,
}

impl Picked {
    /// The line as plain text.
    pub fn text(&self) -> String {
        self.spans.iter().map(|s| s.text.as_str()).collect()
    }

    /// Its width in cells.
    pub fn cells(&self) -> usize {
        width(&self.text())
    }
}

fn cut_name(s: &str) -> String {
    if width(s) <= NAME_CELLS {
        s.to_string()
    } else {
        format!("{}…", take(s, NAME_CELLS - 1).0)
    }
}

fn thousands(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A line's spans with every placeholder filled, and the values it was filled
/// with; none when a fact is missing or the line came out too wide.
/// The caller's facts a line read, with their values.
type Seen = Vec<(String, String)>;

fn fill(line: &Line, held: &Held, ctx: &Context) -> Option<(Vec<Span>, Seen)> {
    let mut spans = Vec::new();
    let mut seen = Vec::new();
    for piece in &line.pieces {
        match piece {
            Piece::Text(t) => spans.push(Span {
                text: t.clone(),
                kind: SpanKind::Text,
            }),
            Piece::Key(k) => spans.push(Span {
                text: k.clone(),
                kind: SpanKind::Key,
            }),
            Piece::Value { name, noun } => {
                let text = if COUNT_NAMES.contains(&name.as_str()) {
                    let fact = line.binds.fact(name)?;
                    let n = held.get(fact)?.num()?.max(0);
                    seen.extend(held.signature(fact));
                    let shown = if name == "starts" {
                        thousands(n.min(STARTS_CAP))
                    } else {
                        n.min(COUNT_CAP).to_string()
                    };
                    match noun {
                        Some((one, many)) => format!("{shown} {}", if n == 1 { one } else { many }),
                        None => shown,
                    }
                } else if name == "time" && held.get("time").is_none() {
                    ctx.clock()
                } else {
                    seen.extend(held.signature(name));
                    cut_name(&held.get(name)?.text())
                };
                spans.push(Span {
                    text,
                    kind: SpanKind::Value,
                });
            }
        }
    }
    let cells: usize = spans.iter().map(|s| width(&s.text)).sum();
    (cells <= MAX_CELLS).then_some((spans, seen))
}

// ------------------------------------------------------------------ dictionary

/// A dictionary that cannot be used, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DictError(pub String);

impl fmt::Display for DictError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DictError {}

#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Odds {
    pub chance: f64,
    pub min_days_between: i64,
}

/// How the draw is weighted and limited.
#[derive(Debug, Clone, PartialEq)]
pub struct Draw {
    pub weights: BTreeMap<String, u32>,
    /// The `tips` weight for the first 30 starts.
    pub tips_early: Option<u32>,
    /// Starts in a row that may come from one group; 0: no limit.
    pub same_group_max: usize,
    /// A facts line whose values are those of its last showing waits.
    pub facts_unchanged_waits: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RawDraw {
    weights: BTreeMap<String, u32>,
    tips_early: Option<u32>,
    same_group_max: Option<usize>,
    facts_unchanged: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawOdds {
    rare: Option<Odds>,
    animated: Option<Odds>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLine {
    id: Option<String>,
    text: Option<String>,
    group: Option<String>,
    #[serde(default)]
    when: Vec<String>,
    #[serde(default)]
    emoji: bool,
    anim: Option<String>,
    used: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    version: Option<u32>,
    #[serde(default)]
    off: Vec<String>,
    #[serde(default)]
    draw: RawDraw,
    #[serde(default)]
    odds: RawOdds,
    #[serde(default)]
    gates: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    widest: BTreeMap<String, String>,
    #[serde(default)]
    plain: Vec<RawLine>,
    #[serde(default)]
    first_run: Vec<RawLine>,
    #[serde(default)]
    day: Vec<RawLine>,
    #[serde(default)]
    animated: Vec<RawLine>,
    #[serde(default)]
    rare: Vec<RawLine>,
    #[serde(default)]
    line: Vec<RawLine>,
}

/// The lines, the weights and the odds.
#[derive(Debug, Clone)]
pub struct Dictionary {
    pub draw: Draw,
    pub rare: Odds,
    pub animated: Odds,
    widest: BTreeMap<String, String>,
    gates: BTreeMap<String, Vec<Cond>>,
    off: BTreeSet<String>,
    lines: Vec<Line>,
}

impl Dictionary {
    fn empty() -> Self {
        Self {
            draw: Draw {
                weights: BTreeMap::from([("mine".to_string(), MINE_WEIGHT)]),
                tips_early: None,
                same_group_max: 2,
                facts_unchanged_waits: true,
            },
            rare: Odds {
                chance: 0.01,
                min_days_between: 7,
            },
            animated: Odds {
                chance: 0.0033,
                min_days_between: 30,
            },
            widest: BTreeMap::new(),
            gates: BTreeMap::new(),
            off: BTreeSet::new(),
            lines: Vec::new(),
        }
    }

    /// The dictionary inside the binary. If it ever did not parse (a test
    /// stops that) radar logs it and has no start lines.
    pub fn shipped() -> Self {
        Self::parse(SHIPPED).unwrap_or_else(|why| {
            crate::log::error(format!("start lines: the shipped dictionary: {why}"));
            Self::empty()
        })
    }

    /// A dictionary from its TOML.
    pub fn parse(text: &str) -> Result<Self, DictError> {
        Self::empty().merged(text)
    }

    /// This dictionary with a user's file on top: its lines are added (one
    /// with the id of an earlier line replaces it), its `[draw]`, `[odds]`,
    /// `[gates]` and `[widest]` change the earlier ones, its `off` turns
    /// groups off. Nothing is changed when the file is not good.
    pub fn with_user(&self, text: &str) -> Result<Self, DictError> {
        self.clone().merged(text)
    }

    fn merged(mut self, text: &str) -> Result<Self, DictError> {
        let raw: RawFile =
            toml::from_str(text).map_err(|e| DictError(format!("not valid TOML: {e}")))?;
        if let Some(version) = raw.version.filter(|v| *v != 1) {
            return Err(DictError(format!(
                "version {version}: this radar reads version 1"
            )));
        }
        self.draw.weights.extend(raw.draw.weights);
        self.draw.tips_early = raw.draw.tips_early.or(self.draw.tips_early);
        self.draw.same_group_max = raw.draw.same_group_max.unwrap_or(self.draw.same_group_max);
        match raw.draw.facts_unchanged.as_deref() {
            None => {}
            Some("skip") => self.draw.facts_unchanged_waits = true,
            Some("show") => self.draw.facts_unchanged_waits = false,
            Some(other) => {
                return Err(DictError(format!(
                    "[draw] facts_unchanged is `{other}`; it is `skip` or `show`"
                )));
            }
        }
        for (name, odds) in [("rare", raw.odds.rare), ("animated", raw.odds.animated)] {
            if let Some(odds) = odds {
                if !(0.0..=1.0).contains(&odds.chance) || odds.min_days_between < 0 {
                    return Err(DictError(format!(
                        "[odds] {name}: chance is 0 to 1 and min_days_between is not negative"
                    )));
                }
                if name == "rare" {
                    self.rare = odds;
                } else {
                    self.animated = odds;
                }
            }
        }
        self.widest.extend(raw.widest);
        for (group, conds) in raw.gates {
            let conds = conds
                .iter()
                .map(|c| Cond::parse(c))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| DictError(format!("[gates] {group}: {e}")))?;
            self.gates.insert(group, conds);
        }
        self.off.extend(raw.off);

        let mut seen = BTreeSet::new();
        let sections = [
            ("plain", raw.plain),
            ("first_run", raw.first_run),
            ("day", raw.day),
            ("animated", raw.animated),
            ("rare", raw.rare),
            ("", raw.line),
        ];
        for (section, lines) in sections {
            for mut raw_line in lines {
                let named = raw_line.id.clone().unwrap_or_else(|| "(no id)".into());
                let group = match (raw_line.group.take(), section) {
                    (Some(group), _) => group,
                    (None, "") => return Err(DictError(format!("line `{named}` has no group"))),
                    (None, section) => section.to_string(),
                };
                if !STEPS.contains(&group.as_str()) && !self.draw.weights.contains_key(&group) {
                    return Err(DictError(format!(
                        "line `{named}`: group `{group}` has no weight; give it one in [draw] weights"
                    )));
                }
                let line = Line::compile(raw_line, &group, &self.widest)
                    .map_err(|why| DictError(format!("line {why}")))?;
                if !seen.insert(line.id.clone()) {
                    return Err(DictError(format!(
                        "line `{}`: the id is used twice in the file",
                        line.id
                    )));
                }
                match self.lines.iter_mut().find(|l| l.id == line.id) {
                    Some(old) => *old = line,
                    None => self.lines.push(line),
                }
            }
        }
        Ok(self)
    }

    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    pub fn line(&self, id: &str) -> Option<&Line> {
        self.lines.iter().find(|l| l.id == id)
    }
}

// ----------------------------------------------------------------------- state

/// When a line last showed, and the values it showed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shown {
    pub day: String,
    pub values: BTreeMap<String, String>,
}

/// What radar remembers between starts: `~/.local/state/radar/start-lines.toml`.
/// Nothing in it is needed; a missing or corrupt file is a fresh state.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub starts: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_day: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_day: Option<String>,
    pub starts_today: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_event: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_rare: Option<String>,
    /// The groups of the latest starts, oldest first.
    pub recent: Vec<String>,
    /// Every line id the dictionary had at the last start: one not here is new.
    pub known: Vec<String>,
    /// Per group the ids not yet shown in this round, next first.
    pub bags: BTreeMap<String, Vec<String>>,
    /// Per group the id shown last.
    pub last_line: BTreeMap<String, String>,
    /// Per line id when it last showed, and the facts it showed.
    pub shown: BTreeMap<String, Shown>,
}

impl State {
    /// No start has been counted: the caller may take this for the first run.
    pub fn is_fresh(&self) -> bool {
        self.starts == 0
    }

    /// Counts this start.
    fn tick(&mut self, ctx: &Context) -> Tick {
        let today = ctx.date;
        let before = self.last_day.as_deref().and_then(Date::parse);
        let first_today = before != Some(today);
        let gap_days = match before {
            Some(day) if first_today => (today.days() - day.days()).max(0),
            _ => 0,
        };
        self.starts += 1;
        self.starts_today = if first_today {
            1
        } else {
            self.starts_today + 1
        };
        self.last_day = Some(today.iso());
        self.first_day.get_or_insert_with(|| today.iso());
        Tick {
            first_today,
            gap_days,
        }
    }

    /// Lines that were not in the dictionary at the last start go to the
    /// front of their bags.
    fn welcome_new_lines(&mut self, lines: &[&Line]) {
        let known: BTreeSet<&str> = self.known.iter().map(String::as_str).collect();
        let fresh: Vec<&&Line> = if known.is_empty() {
            Vec::new()
        } else {
            lines
                .iter()
                .filter(|l| !known.contains(l.id.as_str()))
                .collect()
        };
        let ids: Vec<String> = lines.iter().map(|l| l.id.clone()).collect();
        for line in fresh.into_iter().rev() {
            let bag = self.bags.entry(line.group.clone()).or_default();
            if !bag.contains(&line.id) {
                bag.insert(0, line.id.clone());
            }
        }
        self.known = ids;
    }

    fn days_since(&self, day: &Option<String>, ctx: &Context) -> Option<i64> {
        day.as_deref()
            .and_then(Date::parse)
            .map(|d| ctx.date.days() - d.days())
    }

    fn remember(&mut self, cand: &Cand, ctx: &Context) {
        self.shown.insert(
            cand.line.id.clone(),
            Shown {
                day: ctx.date.iso(),
                values: cand.seen.iter().cloned().collect(),
            },
        );
        self.last_line
            .insert(cand.line.group.clone(), cand.line.id.clone());
        self.recent.push(cand.line.group.clone());
        if self.recent.len() > RECENT_KEPT {
            self.recent.drain(..self.recent.len() - RECENT_KEPT);
        }
    }

    /// The next id of the group's bag that `eligible` allows, taken out of
    /// it; when the bag has none the group is shuffled into a new bag.
    fn take_from_bag(
        &mut self,
        group: &str,
        eligible: &[&str],
        all: &[&str],
        rng: &mut impl Rng,
    ) -> Option<String> {
        let bag = self.bags.entry(group.to_string()).or_default();
        if let Some(at) = bag.iter().position(|id| eligible.contains(&id.as_str())) {
            return Some(bag.remove(at));
        }
        let mut fresh: Vec<String> = all.iter().map(|id| id.to_string()).collect();
        shuffle(&mut fresh, rng);
        let last = self.last_line.get(group);
        let allowed = |id: &String| eligible.contains(&id.as_str());
        let at = fresh
            .iter()
            .position(|id| allowed(id) && Some(id) != last)
            .or_else(|| fresh.iter().position(allowed))?;
        let id = fresh.remove(at);
        self.bags.insert(group.to_string(), fresh);
        Some(id)
    }
}

// ------------------------------------------------------------------------ pick

/// A line that may show now.
struct Cand<'a> {
    line: &'a Line,
    spans: Vec<Span>,
    /// The caller's facts the line read, with their values.
    seen: Seen,
}

fn candidate<'a>(
    dict: &Dictionary,
    line: &'a Line,
    held: &Held,
    ctx: &Context,
) -> Option<Cand<'a>> {
    if line.emoji && !ctx.emoji_ok {
        return None;
    }
    if let Some(anim) = &line.anim
        && (!ctx.motion_full || !BUILT_ANIMS.contains(&anim.as_str()))
    {
        return None;
    }
    let gate = dict
        .gates
        .get(&line.group)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if !gate.iter().chain(&line.when).all(|c| c.holds(held, ctx)) {
        return None;
    }
    let (spans, mut seen) = fill(line, held, ctx)?;
    for name in line.when.iter().flat_map(Cond::facts) {
        seen.extend(held.signature(name));
    }
    seen.sort();
    seen.dedup();
    Some(Cand { line, spans, seen })
}

fn picked(cand: &Cand) -> Picked {
    Picked {
        id: cand.line.id.clone(),
        group: cand.line.group.clone(),
        spans: cand.spans.clone(),
        anim: cand.line.anim.clone(),
    }
}

/// Picks the start line for this start, or none. Counts the start in `state`
/// and remembers what showed; the caller saves the state ([`save_state`]).
///
/// Chance is used in this order, and only when it is needed: the event roll,
/// the rare roll, the weighted draw, then a shuffle when a bag is new.
pub fn pick(
    dict: &Dictionary,
    cfg: &UserConfig,
    state: &mut State,
    facts: &Facts,
    ctx: &Context,
    rng: &mut impl Rng,
) -> Option<Picked> {
    let tick = state.tick(ctx);
    let mine = cfg.mine_lines(dict);
    let all: Vec<&Line> = dict.lines.iter().chain(&mine).collect();
    state.welcome_new_lines(&all);
    if !cfg.show {
        return None;
    }
    let held = Held::new(state, tick, facts, ctx);
    let cands: Vec<Cand> = all
        .iter()
        .filter(|l| !cfg.is_off(dict, &l.group))
        .filter_map(|l| candidate(dict, l, &held, ctx))
        .collect();
    let of_group = |group: &'static str| of_group(&cands, group);

    // 1. Something failed.
    if let Some(cand) = of_group("plain").next() {
        state.remember(cand, ctx);
        return Some(picked(cand));
    }
    // 2. The first start ever.
    if ctx.first_run
        && let Some(cand) = of_group("first_run").next()
    {
        state.remember(cand, ctx);
        return Some(picked(cand));
    }
    // 3. The first start of the day.
    if tick.first_today
        && let Some(cand) = of_group("day").next()
    {
        state.remember(cand, ctx);
        return Some(picked(cand));
    }
    // 4. The sweep's rare echo takes the start's rare thing, and no event.
    if ctx.echo_rare {
        let echoes: Vec<&Cand> = of_group("rare")
            .filter(|c| {
                c.line
                    .when
                    .iter()
                    .any(|w| matches!(w, Cond::Bare(n) if n == ECHO_FACT))
            })
            .collect();
        if !echoes.is_empty() {
            let cand = echoes[rng.below(echoes.len())];
            state.remember(cand, ctx);
            return Some(picked(cand));
        }
    }
    // 5. An event. Plain lines have won already, so none is held; a failing
    // pipeline still stops it.
    let failing = held
        .get("pipelines.failing")
        .and_then(Value::num)
        .is_some_and(|n| n > 0);
    if !ctx.echo_rare
        && ctx.motion_full
        && !failing
        && state
            .days_since(&state.last_event, ctx)
            .is_none_or(|d| d >= dict.animated.min_days_between)
    {
        let events: Vec<&Cand> = of_group("animated").collect();
        if !events.is_empty() && rng.unit() < dict.animated.chance {
            let cand = events[rng.below(events.len())];
            state.last_event = Some(ctx.date.iso());
            state.remember(cand, ctx);
            return Some(picked(cand));
        }
    }
    // 6. A rare line.
    if state
        .days_since(&state.last_rare, ctx)
        .is_none_or(|d| d >= dict.rare.min_days_between)
    {
        let rares: Vec<&Cand> = of_group("rare")
            .filter(|c| {
                !c.line
                    .when
                    .iter()
                    .any(|w| matches!(w, Cond::Bare(n) if n == ECHO_FACT))
            })
            .collect();
        if !rares.is_empty() && rng.unit() < dict.rare.chance {
            let eligible: Vec<&str> = rares.iter().map(|c| c.line.id.as_str()).collect();
            let everyone: Vec<&str> = group_ids(&all, "rare");
            let id = state.take_from_bag("rare", &eligible, &everyone, rng)?;
            let cand = rares.iter().find(|c| c.line.id == id)?;
            state.last_rare = Some(ctx.date.iso());
            state.remember(cand, ctx);
            return Some(picked(cand));
        }
    }
    // 7. The draw.
    let count = held
        .get("start.count")
        .and_then(Value::num)
        .unwrap_or(state.starts as i64);
    let mut groups: Vec<(&str, u32, Vec<&Cand>)> = Vec::new();
    for cand in &cands {
        let group = cand.line.group.as_str();
        if STEPS.contains(&group) || draw_waits(dict, state, cand, ctx) {
            continue;
        }
        match groups.iter_mut().find(|(g, ..)| *g == group) {
            Some((.., list)) => list.push(cand),
            None => {
                let weight = weight_of(dict, cfg, group, count);
                if weight > 0 {
                    groups.push((group, weight, vec![cand]));
                }
            }
        }
    }
    groups.sort_by_key(|(g, ..)| {
        (
            GROUP_ORDER
                .iter()
                .position(|o| o == g)
                .unwrap_or(GROUP_ORDER.len()),
            *g,
        )
    });
    let max = dict.draw.same_group_max;
    let limited = |group: &str| {
        max > 0
            && state.recent.len() >= max
            && state.recent[state.recent.len() - max..]
                .iter()
                .all(|g| g == group)
    };
    // Never more than `same_group_max` in a row, unless it would leave
    // nothing to show.
    let open: Vec<usize> = (0..groups.len())
        .filter(|&i| !limited(groups[i].0))
        .collect();
    let open = if open.is_empty() {
        (0..groups.len()).collect()
    } else {
        open
    };
    let total: u32 = open.iter().map(|&i| groups[i].1).sum();
    if total == 0 {
        return None;
    }
    let mut roll = rng.below(total as usize) as u32;
    let chosen = open.iter().copied().find(|&i| {
        if roll < groups[i].1 {
            true
        } else {
            roll -= groups[i].1;
            false
        }
    })?;
    let (group, _, list) = &groups[chosen];
    let eligible: Vec<&str> = list.iter().map(|c| c.line.id.as_str()).collect();
    let everyone = group_ids(&all, group);
    let id = state.take_from_bag(group, &eligible, &everyone, rng)?;
    let cand = list.iter().find(|c| c.line.id == id)?;
    state.remember(cand, ctx);
    Some(picked(cand))
}

fn of_group<'b, 'a>(cands: &'b [Cand<'a>], group: &'b str) -> impl Iterator<Item = &'b Cand<'a>> {
    cands.iter().filter(move |c| c.line.group == group)
}

fn group_ids<'a>(all: &[&'a Line], group: &str) -> Vec<&'a str> {
    all.iter()
        .filter(|l| l.group == group)
        .map(|l| l.id.as_str())
        .collect()
}

fn weight_of(dict: &Dictionary, cfg: &UserConfig, group: &str, count: i64) -> u32 {
    if let Some(weight) = cfg.weights.get(group) {
        return *weight;
    }
    if group == "tips"
        && count <= EARLY_STARTS
        && let Some(early) = dict.draw.tips_early
    {
        return early;
    }
    dict.draw.weights.get(group).copied().unwrap_or(0)
}

/// A line that eligible lines of its group must wait out: a time line that
/// showed today, a facts line whose values are those of its last showing, a
/// tip whose action was used enough.
fn draw_waits(dict: &Dictionary, state: &State, cand: &Cand, ctx: &Context) -> bool {
    let line = cand.line;
    if let Some(action) = &line.used
        && ctx
            .tip_uses
            .get(action)
            .is_some_and(|n| *n >= TIP_RETIRES_AFTER)
    {
        return true;
    }
    let Some(last) = state.shown.get(&line.id) else {
        return false;
    };
    match line.group.as_str() {
        "time" => last.day == ctx.date.iso(),
        "facts" => {
            dict.draw.facts_unchanged_waits
                && !cand.seen.is_empty()
                && last.values.len() == cand.seen.len()
                && cand.seen.iter().all(|(k, v)| last.values.get(k) == Some(v))
        }
        _ => false,
    }
}

// ------------------------------------------------------------------------ files

/// Where the state is kept: radar's state folder. None under `RADAR_SANDBOX`
/// unless `XDG_STATE_HOME` is set, so a test never writes to the real one.
pub fn state_path() -> Option<PathBuf> {
    if crate::system::sandboxed() && std::env::var_os("XDG_STATE_HOME").is_none() {
        return None;
    }
    crate::system::state_home().map(|h| h.join("radar").join("start-lines.toml"))
}

/// The user's dictionary: `start-lines.toml` in radar's config folder.
pub fn user_dictionary_path() -> PathBuf {
    crate::runner::config_home().join("start-lines.toml")
}

/// The state in `path`; a fresh one when the file is missing, unreadable or
/// not a state (the last two are logged).
pub fn load_state(path: &Path) -> State {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).unwrap_or_else(|why| {
            crate::log::warn(format!(
                "start lines: {} is not a state, starting afresh: {why}",
                path.display()
            ));
            State::default()
        }),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => State::default(),
        Err(why) => {
            crate::log::warn(format!(
                "start lines: cannot read {}: {why}",
                path.display()
            ));
            State::default()
        }
    }
}

/// Writes the state: to a file beside it, then moved over it.
pub fn save_state(path: &Path, state: &State) -> std::io::Result<()> {
    let text = toml::to_string(state).map_err(std::io::Error::other)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// The shipped dictionary with the user's file at `user` on top. A missing
/// file adds nothing; one that is not good is logged and left out.
pub fn load_dictionary(user: &Path) -> Dictionary {
    let shipped = Dictionary::shipped();
    let text = match std::fs::read_to_string(user) {
        Ok(text) => text,
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return shipped,
        Err(why) => {
            crate::log::warn(format!(
                "start lines: cannot read {}: {why}",
                user.display()
            ));
            return shipped;
        }
    };
    shipped.with_user(&text).unwrap_or_else(|why| {
        crate::log::warn(format!(
            "start lines: leaving out {}: {why}",
            user.display()
        ));
        shipped
    })
}

#[cfg(test)]
mod tests;
