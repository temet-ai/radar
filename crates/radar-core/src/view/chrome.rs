//! The host's own chrome as view data: the top bar, the source tabs, the
//! status line, the open menu or question, the message and the settings
//! navigation. The host produces a [`ChromeView`] for a frame
//! (`Host::chrome_view`); the Ratatui shell paints from it, a headless
//! frontend returns it, and the Ink renderer receives it over IPC.
//!
//! Everything here is bounded data: words, icon names, chords and
//! [`Target`]s. A frontend that activates a target sends it back as an
//! [`super::Intent`]; it never names a cell. Nothing here names a terminal
//! library (the view guard test reads this file too).

use serde::{Deserialize, Serialize};

use super::{Change, Chord, Ink, Target, icon_name};
use crate::ui::icon::Icon;

/// The chrome contract's version, carried with every [`ChromeView`]. It
/// moves with [`super::VERSION`]; additive fields default.
pub const CHROME_VERSION: u32 = super::VERSION;

/// At most this many tabs, hints, menu rows or settings rows are carried;
/// the rest are cut (a frontend shows no more than a screen holds anyway).
pub const MOST: usize = 256;

/// Where the keys are in the chrome, so a frontend lights the right row and
/// sends movement where the host expects it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum KeysAt {
    /// The body (a screen's panes, or settings) has the keys.
    #[default]
    Body,
    /// The top bar has the keys: its active tab stands out.
    Bar,
    /// The source tabs under the bar have the keys.
    Sources,
    /// An item page's tab row has the keys.
    PageTabs,
}

/// The host's chrome for one frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ChromeView {
    pub version: u32,
    /// The room it was made for, in cells.
    pub width: u16,
    pub height: u16,
    pub top_bar: TopBar,
    /// The sources of the screen showing (each part's name), when it has
    /// more than one; empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<SourceTab>,
    pub status: StatusLine,
    /// The menu or question open, if one is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub menu: Option<MenuView>,
    /// Settings navigation, while settings show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<SettingsNav>,
    #[serde(default)]
    pub keys_at: KeysAt,
    /// The history overlay, when it is open over the body: its rows, detail,
    /// help and keys, each pointable by a `chrome:history:*` target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history: Option<crate::history::HistoryView>,
    /// The capital letters the host takes as screen or source keys, which
    /// a renderer must not use for itself (an unclaimed `G` scrolls a
    /// document pane to the end, as Ratatui's `Scroll::key` does).
    /// Filled by the host; empty until it is.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub screen_keys: Vec<char>,
    /// Setup, while it shows (instead of the tabs and the settings nav).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub setup: Option<SetupChrome>,
    /// The tour's card, while the tour shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tour: Option<TourCard>,
}

impl ChromeView {
    /// An empty chrome of this size (nothing showing yet).
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            version: CHROME_VERSION,
            width,
            height,
            top_bar: TopBar::default(),
            sources: vec![],
            status: StatusLine::default(),
            menu: None,
            settings: None,
            keys_at: KeysAt::Body,
            history: None,
            screen_keys: vec![],
            setup: None,
            tour: None,
        }
    }
}

/// The tour's card over the body, while the tour shows
/// ([`crate::tour`]): `tour · 2 of 5`, a title, its lines, what to try,
/// and its keys. Drawn at the body's lower right, over the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TourCard {
    /// This stop, from 1.
    pub stop: usize,
    pub of: usize,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lines: Vec<crate::view::Text>,
    /// `try it: press J`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub try_it: Option<String>,
    /// `→ next   ← back   esc end the tour`.
    pub keys: crate::view::Text,
}

/// ` radar   Books 3   history   settings          ⠷ synced 12s   read-only `
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct TopBar {
    /// The brand at its left (`radar`).
    pub brand: String,
    pub tabs: Vec<BarTab>,
    /// The part's words on the right before the mode (`synced 12s`, or the
    /// host's `paused`). They give way before the tabs do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub words: Option<Badge>,
    /// The embedded terminal's state, after the part's words; kept the
    /// longest of the words on the right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<Badge>,
    /// The part is working (the throbber).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub busy: bool,
    /// The mode at the right end, always shown; activating it opens the
    /// mode's menu.
    pub mode: ModeChip,
}

/// One tab of the top bar: a screen, history or settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct BarTab {
    pub label: String,
    /// The key that goes there, underlined in the label where it is in it.
    pub mnemonic: char,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// A count or a state after the label, in its tone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Badge>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub active: bool,
    pub target: Target,
}

/// A count or state after a tab's label (`3`, `unsaved`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Badge {
    pub text: String,
    #[serde(default)]
    pub ink: Ink,
}

/// The mode at the right of the top bar (`read-only`, `edit`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct ModeChip {
    pub text: String,
    #[serde(default)]
    pub ink: Ink,
    /// Stands out (read only does).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strong: bool,
    pub target: Target,
}

/// A source tab under the top bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct SourceTab {
    pub label: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub active: bool,
    pub target: Target,
}

/// ` BOOKS   PgUp/PgDn scroll   q quit   x menu          11 open · 3 need you `
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct StatusLine {
    /// The chip at its left: the screen's name (`BOOKS`, `SETTINGS`).
    pub scope: String,
    /// The keys, in the order they are shown; the first to give way when
    /// there is no room are listed in [`StatusLine::least_needed`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<Hint>,
    /// Hint actions to drop first, in order, when the line does not fit.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub least_needed: Vec<String>,
    /// The counts at the right, each in its tone, shown ` · ` apart; they
    /// give way before the keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub counts: Vec<Badge>,
    /// A message shown in place of the keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    /// One chip per service a part syncs with, at the far right after the
    /// counts, a space apart; the last to give way, after the keys.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health: Vec<HealthChip>,
    /// An agent session wants the user and its tab is out of sight: a chip
    /// after the scope, which never gives way. Its key and a click open the
    /// dock on that tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<AttentionChip>,
}

/// ` review dock needs you ` on its state's ground, after the key that goes
/// there (`` ` ``).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct AttentionChip {
    /// The session's name and what it wants (`review dock needs you`,
    /// `codex done`), with `+n` when other sessions want the user too.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// The ground: warn for needs you, bad for failed, good for done.
    pub ink: Ink,
    /// The key that goes there, as written before the chip.
    pub keys: String,
    pub target: Target,
}

/// A service's health at the right of the status line: its mark (or its
/// part's name with icons off) in the colour of its state, and how many of
/// its sources fail when some do (`1/4`). Activating it opens the
/// service's health menu.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct HealthChip {
    /// The part's id.
    pub part: String,
    /// What is drawn when there is no glyph (icons off).
    pub label: String,
    /// The service's own mark in the icon pack in use: a Nerd Font glyph,
    /// a plain symbol or an emoji; empty with icons off.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub glyph: String,
    pub state: HealthState,
    #[serde(default)]
    pub ink: Ink,
    /// After the glyph, a space apart: failing sources of all (`1/4`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<String>,
    pub target: Target,
}

impl HealthChip {
    /// The chip as drawn: the glyph, else the label, then the count.
    pub fn text(&self) -> String {
        let head = if self.glyph.is_empty() { self.label.as_str() } else { self.glyph.as_str() };
        match &self.count {
            Some(c) => format!("{head} {c}"),
            None => head.to_string(),
        }
    }
}

/// Where a service stands, worst first among its sources (`crate::health`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    /// Every source synced within three refresh intervals.
    Ok,
    /// A sync is under way and nothing fails.
    Syncing,
    /// Nothing fails, but some source has had no good sync for three
    /// refresh intervals (or none yet).
    Stale,
    /// Some sources fail, the others are fine.
    Partial,
    /// Every source fails.
    Failing,
    /// The service turned every source's credentials down, or there are none.
    SignedOut,
    /// Held back without a failure: a rate limit nearly spent.
    Waiting,
}

/// One key on the status line: `x ≡ menu`, `PgUp/PgDn scroll`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Hint {
    /// The key labels as written (`PgUp/PgDn`, `←→↑↓`, `x`).
    pub keys: String,
    /// What they do (`scroll`, `expand/collapse`).
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// Each key the labels stand for, in order, each with its target; a
    /// hint whose labels are not keys has none and cannot be clicked.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys_of: Vec<HintKey>,
}

/// One pressable key of a [`Hint`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct HintKey {
    /// Its label inside [`Hint::keys`].
    pub label: String,
    pub chord: Chord,
    /// The action it runs, when it is one with an id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    pub target: Target,
}

/// A line the host says (a notification, what an action did).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Message {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    #[serde(default)]
    pub ink: Ink,
}

/// The open menu or question (a dialog with buttons), as data: what it
/// asks and the choices, each by the value [`super::Intent::Invoke`]
/// answers it with, and each with a target a pointer chooses it by.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct MenuView {
    pub id: String,
    /// What it is, or what a question asks; may run to several lines.
    pub title: String,
    /// An icon before the title (what the menu is about).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// A question (a dialog with buttons), not a list.
    pub question: bool,
    /// The write it is before, when it is one; the host has already added
    /// the mode's note when the mode refuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<Change>,
    pub items: Vec<Choice>,
    /// The highlighted row (an index into `items`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<usize>,
    /// The text in its field, when it asks for text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub typed: Option<String>,
    /// Where the caret is in the field, in characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caret: Option<usize>,
    /// Where a context click opened it (the frame's top-left corner goes
    /// there, kept inside the body); centred when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<CellAt>,
    /// How far its rows are scrolled: the first row shown, counted in lines
    /// as laid out at the width it was last shown at.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub scroll: usize,
    /// How many lines its rows take at the width it was measured at, and
    /// how many of them show; more than show means it scrolls (`PgUp/PgDn`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub lines: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shown: usize,
    /// Its controls fit in the room it has (else a frontend says to make the
    /// terminal larger and `⏎` chooses nothing).
    #[serde(default = "yes", skip_serializing_if = "is_yes")]
    pub usable: bool,
    /// Why the part refuses the typed text now, if it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused: Option<String>,
}

impl MenuView {
    /// A menu with these rows and nothing else said: not scrolled, not
    /// typed into, opened in the middle. Build the rest with `..`.
    pub fn new(id: impl Into<String>, title: impl Into<String>, items: Vec<Choice>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            icon: None,
            question: false,
            change: None,
            items,
            selected: None,
            typed: None,
            caret: None,
            at: None,
            scroll: 0,
            lines: 0,
            shown: 0,
            usable: true,
            refused: None,
        }
    }
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

fn yes() -> bool {
    true
}

fn is_yes(b: &bool) -> bool {
    *b
}

/// A cell on the screen, where the user pointed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct CellAt {
    pub column: u16,
    pub row: u16,
}

/// One row of a menu, or one button of a question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Choice {
    pub value: String,
    pub label: String,
    pub hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<char>,
    /// What its key column shows (`d`, `space`, `⏎`); empty on a heading.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub keys: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// A heading over the rows that follow.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
    /// A line drawn as written (what a write will send, in its question).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub note: bool,
    /// A look to show a sample of the row in (a colour picker's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<String>,
    /// A heading or a note: shown, never chosen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shown_only: bool,
    /// It can be chosen but will not do all it says now (the mode).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muted: bool,
    /// What a pointer chooses it by (`chrome:menu:<index>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
}

/// Settings: the sections as a tree grouped by part, the one selected, and
/// the titles of the section and its detail.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct SettingsNav {
    pub groups: Vec<NavGroup>,
    /// The selected row's target (a group's or a section's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<Target>,
    /// The section's title, over the middle pane.
    pub section_title: String,
    /// The detail pane's title.
    pub detail_title: String,
    /// Settings could not be saved (`unsaved · w retries`).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unsaved: bool,
}

/// A group of settings sections (a part, the host's own).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct NavGroup {
    pub id: String,
    pub name: String,
    pub open: bool,
    pub target: Target,
    pub sections: Vec<NavSection>,
}

/// A settings section in the navigation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct NavSection {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    pub target: Target,
}

/// The targets the chrome names, so the host and every frontend agree on
/// them without parsing anything else.
pub mod targets {
    use super::Target;

    pub const PREFIX: &str = "chrome:";

    /// The top bar tab for screen `index`.
    pub fn screen_tab(index: usize) -> Target {
        Target(format!("chrome:tab:{index}"))
    }
    pub fn history_tab() -> Target {
        Target::new("chrome:history")
    }
    pub fn settings_tab() -> Target {
        Target::new("chrome:settings")
    }
    pub fn mode() -> Target {
        Target::new("chrome:mode")
    }
    /// The health chip of the part with this id.
    pub fn health(part: &str) -> Target {
        Target(format!("chrome:health:{part}"))
    }
    /// The status line's chip for the agent session that wants the user.
    pub fn attention() -> Target {
        Target::new("chrome:attention")
    }
    /// The source tab for screen `index`.
    pub fn source_tab(index: usize) -> Target {
        Target(format!("chrome:source:{index}"))
    }
    /// Key `k` of status-line hint `n`.
    pub fn hint_key(n: usize, k: usize) -> Target {
        Target(format!("chrome:key:{n}:{k}"))
    }
    /// Row `index` of the open menu.
    pub fn menu_item(index: usize) -> Target {
        Target(format!("chrome:menu:{index}"))
    }
    pub fn nav_group(id: &str) -> Target {
        Target(format!("chrome:group:{id}"))
    }
    /// Settings section `index` (into the host's sections).
    pub fn nav_section(index: usize) -> Target {
        Target(format!("chrome:section:{index}"))
    }
    /// Stop `index` of setup's rail.
    pub fn setup_stop(index: usize) -> Target {
        Target(format!("chrome:setup:{index}"))
    }
}

/// Setup's chrome, while setup shows: the screen's title for the top bar
/// (`setup / <title>`), `step n of m`, and the rail. The card and the
/// sheet are the snapshot's section (its body and its detail); the key bar
/// is the status line, scoped `SETUP`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct SetupChrome {
    /// The screen's name, after `setup / ` in the top bar and over the card.
    pub title: String,
    /// `step 2 of 5`, on a step's screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<String>,
    /// One stop per chosen step, then `tour?`; empty before the choice.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rail: Vec<RailStop>,
    /// The stop of the step showing, an index into `rail`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<usize>,
    /// The card's first line, bold (`What would you like to set up?`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub heading: String,
    /// The muted line under it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lead: String,
}

/// A stop of setup's rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct RailStop {
    /// The step's id (`look`, a part's step id, `tour`).
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    pub state: StopState,
    /// The line under the stop: a done step's result, `skipped`, the
    /// current one's `step n of m`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Activating a stop reached before goes back to it.
    pub target: Target,
}

/// Where a rail's stop is in the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum StopState {
    #[default]
    Upcoming,
    Current,
    Done,
    Skipped,
}
