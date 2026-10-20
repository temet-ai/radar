//! The host: owns the frame, focus, screens, settings and config file, and
//! routes everything else to plugins. `update(event) → effects` is pure
//! (TUI-A2): the runner performs the effects.

#[path = "host/detail_actions.rs"]
mod detail_actions;

#[path = "host_palette.rs"]
mod palette;

#[path = "host_notices.rs"]
mod notice_support;

#[path = "host_motion.rs"]
pub(crate) mod interaction_motion;

#[path = "host_tree.rs"]
mod shared_tree;

#[cfg(test)]
#[path = "host_palette_tests.rs"]
mod palette_tests;

use std::cell::Cell;
use std::collections::HashMap;

use anyhow::{Result, bail};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

use crate::colours::{Colours, Reply};
use crate::compose::{self, Contribution, Point, Showing, Subject};
pub use crate::icons::ICONS_SECTION;
use crate::journal::{ActionDescriptor, ActionId, Origin, Outcome as JournalOutcome, Reason};
pub use crate::journal_runtime::JournalRecord;
use crate::journal_runtime::{MenuTrace, Trace};
use crate::motion::Cue;
use crate::plugin::{
    Binding, Effect, Menu, MenuItem, Msg, Outcome, Pane, Plugin, ScreenSpec, SectionSpec,
    SlotValue, Task,
};
use crate::design::help_bar::KeyHint;
use crate::theme::{Theme, ThemeConfig};
use crate::widgets::{horizontal, step, vertical};

#[cfg(test)]
#[path = "host_history_discovery_tests.rs"]
mod history_discovery_tests;

#[cfg(test)]
#[path = "host_history_tests.rs"]
mod history_tests;

#[cfg(test)]
#[path = "host_menu_shortcut_tests.rs"]
mod menu_shortcut_tests;

#[cfg(test)]
#[path = "host_menu_context_tests.rs"]
mod menu_context_tests;

#[cfg(test)]
#[path = "host_contributed_context_tests.rs"]
mod contributed_context_tests;

#[cfg(test)]
#[path = "host_menu_overflow_tests.rs"]
mod menu_overflow_tests;

#[cfg(test)]
#[path = "host_work_tests.rs"]
mod work_tests;

#[cfg(test)]
#[path = "host_ink_intent_tests.rs"]
mod ink_intent_tests;

#[cfg(test)]
#[path = "host_dock_tests.rs"]
mod dock_tests;

#[cfg(test)]
#[path = "host_agent_tests.rs"]
mod agent_tests;

/// The next item down (or up) that can be chosen, skipping headings; stays
/// put at either end.
fn next_item(items: &[MenuItem], from: usize, down: bool) -> usize {
    let mut row = from;
    loop {
        let next = step(row, items.len(), down);
        if next == row {
            return from;
        }
        row = next;
        if !items[row].heading {
            return row;
        }
    }
}

/// Keys no contributed action may take: the host's own, moving about
/// (lists, tabs, pages), and every capital (screens).
pub(crate) fn host_key(c: char) -> bool {
    "qx?bwzhljkgG[]/: +=-".contains(c) || c.is_ascii_digit() || c.is_ascii_uppercase()
}

/// The plain keys a part's own `x` rows press.
fn own_keys(rows: &[Binding]) -> Vec<char> {
    rows.iter()
        .filter_map(|b| match crate::plugin::parse_key(&b.keys) {
            Some(KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            }) if modifiers.difference(KeyModifiers::SHIFT).is_empty() => Some(c),
            _ => None,
        })
        .collect()
}

/// Who owns a settings section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    /// The host's colours (the theme).
    Host,
    /// The host's list of the parts compiled in.
    Parts,
    /// The host's mode: which writes radar may send.
    Mode,
    /// The look: icons, theme and motion, as setup's look step shows them.
    Look,
    Plugin(usize),
}

/// The host's own settings section: icons, theme and motion, the same card
/// as setup's look step ([`Host::look_settings_view`]).
pub const LOOK_SECTION: SectionSpec = SectionSpec {
    id: "appearance",
    name: "appearance",
    group: "general",
};

/// The host's own settings section: the theme.
pub const COLOURS_SECTION: SectionSpec = SectionSpec {
    id: "colours",
    name: "colours",
    group: "general",
};

/// The host's own settings section: the parts in this build, whether each
/// is set up, and what each adds to the others.
pub const PARTS_SECTION: SectionSpec = SectionSpec {
    id: "parts",
    name: "parts",
    group: "general",
};

/// The config table of icon overrides, `[icons]`: glyphs over the pack's.
pub const ICONS_TABLE: &str = "icons";

/// Where a show came from, so `esc` can come back to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Return {
    pub view: View,
    pub focus: Pane,
    /// The screen the show went to.
    pub to: usize,
    /// That screen's depth once the shown page was open; backing out below
    /// it returns.
    pub depth: usize,
}

/// What reached the focused part: a terminal key, or an intent from a
/// frontend that speaks the view contract.
enum Input {
    Key(KeyEvent),
    Intent(crate::view::Intent),
}

/// What choosing a row of the keys menu does.
#[derive(Clone)]
enum Press {
    /// Press this key.
    Key(KeyEvent, Option<MenuKeyContext>),
    /// A shared tree operation retains the exact held tree and selection.
    TreeKey(KeyEvent, shared_tree::MenuContext),
    /// A contributed action: the plugin, the subject, the action's id.
    Act(usize, Subject, String, Option<MenuKeyContext>),
    /// Captured, validated subject scope; reading never follows a later selection.
    History(Option<crate::journal::SubjectKey>),
    RecentHistory,
}

impl std::fmt::Debug for Press {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Key(key, context) => f.debug_tuple("Key").field(key).field(context).finish(),
            Self::TreeKey(key, _) => f.debug_tuple("TreeKey").field(key).finish(),
            Self::Act(..) => f.write_str("Act(..)"),
            Self::History(..) => f.write_str("History(..)"),
            Self::RecentHistory => f.write_str("RecentHistory"),
        }
    }
}

/// An opted-in key belongs to both its original route and the part's target.
/// The token may contain private target identifiers: never format or persist it.
#[derive(Clone)]
struct MenuKeyContext {
    plugin: usize,
    scope: &'static str,
    view: View,
    focus: Pane,
    bar: bool,
    tabs: bool,
    action: String,
    token: String,
}

impl std::fmt::Debug for MenuKeyContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MenuKeyContext(..)")
    }
}

/// A menu on screen: who opened it, the menu, the highlighted item.
pub struct OpenMenu {
    pub owner: Owner,
    pub menu: Menu,
    pub row: usize,
    /// The field of a menu that asks for text ([`Menu::typed`]).
    pub input: Option<crate::input::TextInput>,
    /// Why the part refuses the text in the field now, if it does.
    pub refused: Option<String>,
    /// Reading position and geometry of this menu, independent of its choice.
    pub viewport: MenuViewport,
    /// Pointer position when a context click opened the menu.
    pub anchor: Option<ratatui::layout::Position>,
}

/// Geometry learned while drawing. Scrolling only changes the reading
/// position; it never changes a confirmation's chosen value.
#[derive(Debug)]
pub struct MenuViewport {
    pub(crate) at: Cell<usize>,
    pub(crate) height: Cell<usize>,
    pub(crate) total: Cell<usize>,
    pub(crate) width: Cell<u16>,
    pub(crate) selected: Cell<Option<usize>>,
    pub(crate) usable: Cell<bool>,
}

impl Default for MenuViewport {
    fn default() -> Self {
        Self {
            at: Cell::new(0),
            height: Cell::new(0),
            total: Cell::new(0),
            width: Cell::new(0),
            selected: Cell::new(None),
            usable: Cell::new(true),
        }
    }
}

impl MenuViewport {
    pub(crate) fn max(&self) -> usize {
        self.total.get().saturating_sub(self.height.get())
    }

    pub(crate) fn more(&self) -> bool {
        self.usable.get() && self.max() > 0
    }

    pub(crate) fn scroll(&self, down: bool, page: bool) {
        let step = if page {
            self.height.get().saturating_sub(1).max(1)
        } else {
            1
        };
        self.at.set(if down {
            self.at.get().saturating_add(step).min(self.max())
        } else {
            self.at.get().saturating_sub(step)
        });
    }
}

/// No offer highlighted under a typed menu's field.
pub const NO_ROW: usize = usize::MAX;

impl OpenMenu {
    /// `menu` as it opens: its first item that can be chosen is
    /// highlighted (a question's `y`), so `⏎` chooses it; a typed menu
    /// opens with nothing highlighted, the field taking the keys.
    pub fn open(owner: Owner, menu: Menu) -> Self {
        let first = menu.items.iter().position(|i| !i.heading).unwrap_or(0);
        let row = menu
            .at
            .as_ref()
            .and_then(|v| {
                menu.items
                    .iter()
                    .position(|i| !i.heading && !i.note && &i.value == v)
            })
            .unwrap_or(first);
        let input = menu.typed.as_deref().map(crate::input::TextInput::new);
        let row = if input.is_some() { NO_ROW } else { row };
        Self {
            owner,
            menu,
            row,
            input,
            refused: None,
            viewport: MenuViewport::default(),
            anchor: None,
        }
    }

    /// The items that can be picked under a typed menu's field.
    fn picks(&self) -> Vec<usize> {
        let items = &self.menu.items;
        (0..items.len())
            .filter(|i| !items[*i].note && !items[*i].heading)
            .collect()
    }
}

/// Which screen is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum View {
    /// Index into the host's screen list.
    Screen(usize),
    Settings,
}

/// The settings screen's switch key; reserved for the host. A comma, as
/// many terminal apps use for preferences. Shift+H is history; other
/// Shift+letters remain available for the parts' screens.
pub const SETTINGS_KEY: char = ',';
/// Read-only activity history; text editors retain this key while capturing.
pub const HISTORY_KEY: char = 'H';
/// The id of the host's keys menu (`x`).
pub const KEYS_MENU: &str = "keys";
/// The id a plugin's bulk commands (`b`) come back under in `on_menu`.
pub const BULK_MENU: &str = "bulk";
/// The id of the question `q` asks when settings are unsaved.
const QUIT_MENU: &str = "quit";
/// Keys the host keeps for itself on every screen.
const RESERVED: [char; 2] = ['q', '?'];
/// How deep bus events may chain before the host stops delivering them.
const BUS_DEPTH: usize = 8;

pub enum Event {
    History {
        id: u64,
        result: Result<Box<crate::journal::work_map::EventPage>, crate::history::ReadError>,
    },
    /// Runner envelope preserving the exact originating action for an
    /// asynchronous result. No pairing by plugin or completion order.
    Tracked {
        action: ActionId,
        semantic: Option<ActionId>,
        event: Box<Event>,
    },
    Key(KeyEvent),
    Tick(i64),
    /// A part woke the host ([`crate::wake`]), with the current unix time: the
    /// woken parts take in what their background work brought.
    Wake(i64),
    /// A task's result, for the plugin at this index.
    Msg {
        plugin: usize,
        msg: Msg,
    },
    Saved(Result<(), String>),
    /// How something the runner did went, for the status line (a link
    /// opened, or why it could not be).
    Notice(String),
    /// A runner failure, explicitly typed.
    ErrorNotice(String),
    /// Result of the one-pane Herdr right-click routing request.
    TerminalRoute(Result<(), String>),
    /// Put `text` on the user's clipboard through their terminal, then show
    /// `note` (if any). The runner's own: it writes to the terminal between
    /// frames, and the host only sees the note.
    ToClipboard {
        text: String,
        note: String,
    },
    /// Text pasted into the terminal.
    Paste(String),
    /// What setup's look for keys found ([`HostEffect::Scan`]): names only.
    Scanned(Vec<crate::setup::Found>),
    /// What the user did, from a frontend that speaks the view contract
    /// ([`crate::view::Intent`]) instead of terminal events. Routed, checked
    /// and journalled as the key or click it stands for.
    Intent(crate::view::Intent),
    /// A mouse press or wheel turn at a cell, with when it happened (ms),
    /// so two presses on the same thing close together are a double click.
    Mouse {
        kind: MouseKind,
        column: u16,
        row: u16,
        at_ms: u64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseKind {
    Press,
    Context,
    ScrollUp,
    ScrollDown,
}

/// A clickable area, as the last frame drew it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hit {
    Notice(u64),
    NoticeHistory(u64),
    NoticeHistoryOpen,
    NoticeHistoryKey(KeyEvent),
    /// A history row or key; interpreted only by the active overlay.
    History,
    /// The History tab in the top bar.
    HistoryTab,
    Tab(View),
    SourceTab(View),
    Section(usize),
    /// A group's heading in the settings nav.
    Group(&'static str),
    ColourRow(usize),
    PartsRow(usize),
    /// The mode at the right of the top bar, or its settings row.
    Mode,
    /// A part's health chip at the right of the status line.
    Health(usize),
    /// The status line's chip for an agent session that wants the user.
    Attention,
    /// The `pause if idle` row of the General section.
    PauseIdle,
    Pane(Pane),
    MenuBox,
    MenuItem(usize),
    /// A key shown on the status line: a click presses it. The number is
    /// the hint it is part of (`PgUp/PgDn scroll` is one hint, two keys).
    Key(KeyEvent, usize),
    /// A row of one of setup's own steps, by its target.
    Setup(String),
    /// A stop of setup's rail.
    SetupStop(usize),
    /// An area a plugin marked: on its screen, or in one of its sections.
    Plugin {
        plugin: usize,
        scope: &'static str,
        pane: Pane,
        id: String,
    },
}

/// What the runner must do.
pub enum HostEffect {
    Work {
        plugin: usize,
        id: u64,
        request: Box<crate::work_api::Request>,
        mode: crate::writes::Mode,
    },
    /// Read one bounded, snapshot-stable page off the UI thread.
    History(Box<crate::history::Request>),
    /// In-memory records produced by the reducer; the runner writes them.
    Journal(JournalRecord),
    /// An effect's unique causal context, retained by its eventual answer.
    Tracked {
        action: ActionId,
        semantic: Option<ActionId>,
        effect: Box<HostEffect>,
    },
    Spawn {
        plugin: usize,
        task: Task,
    },
    Open(String),
    /// An app's own link, else `web` in the browser ([`Effect::OpenApp`]).
    OpenApp {
        link: String,
        web: String,
    },
    Copy(String),
    /// Copy text the user selected on the screen ([`crate::view::Intent::Copy`]):
    /// as [`HostEffect::Copy`], and the status line says how many characters
    /// and by which way.
    CopySelection(String),
    /// Write the config file with this text.
    Save(String),
    /// Write a plugin's cache.
    WriteCache {
        plugin: &'static str,
        value: Value,
    },
    /// Setup's look for keys, which the user said yes to: the runner reads
    /// the variable names and checks which files exist
    /// ([`crate::setup::scan`]), and answers with [`Event::Scanned`].
    Scan(crate::setup::Asks),
}

impl std::fmt::Debug for HostEffect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Work { .. } => f.write_str("Work(..)"),
            Self::History(_) => f.write_str("History(..)"),
            Self::Journal(_) => f.write_str("Journal(..)"),
            Self::Tracked { .. } => f.write_str("Tracked(..)"),
            Self::Spawn { plugin, .. } => write!(f, "Spawn({plugin})"),
            Self::Open(u) => write!(f, "Open({u})"),
            Self::OpenApp { link, web } => write!(f, "OpenApp({link}, else {web})"),
            Self::Copy(t) => write!(f, "Copy({t})"),
            Self::CopySelection(t) => write!(f, "CopySelection({t})"),
            Self::Save(_) => f.write_str("Save(..)"),
            Self::WriteCache { plugin, .. } => write!(f, "WriteCache({plugin})"),
            Self::Scan(asks) => write!(f, "Scan({} parts)", asks.len()),
        }
    }
}

pub struct Host {
    context_service: crate::context::Service,
    review_context_started: std::collections::BTreeMap<u64, i64>,
    pub(crate) history: Option<crate::history::History>,
    history_request: u64,
    trace: Trace,
    pub plugins: Vec<Box<dyn Plugin>>,
    /// Which parts woke the host since it last looked ([`crate::wake`]).
    wake: crate::wake::WakeHub,
    pub screens: Vec<(usize, ScreenSpec)>,
    pub sections: Vec<(Owner, SectionSpec)>,
    pub view: View,
    pub focus: Pane,
    /// The agent dock's host state: shown, focused, height ([`crate::dock`]).
    pub dock: crate::dock::DockState,
    /// The top bar (the screens' tabs) has the keys: `← →` switch screens,
    /// `↓` goes back in. `↑` at the top of any pane of a screen gets here.
    pub bar: bool,
    /// The source row beneath a grouped screen's top tab has the keys.
    pub sources: bool,
    /// An item page's tab row has the keys: `← →` show the tab beside,
    /// `↓` goes back into the body. `↑` at the top of the body gets here.
    pub tabs: bool,
    /// The pane each screen had focused last, for `↓` out of the bar.
    last_pane: HashMap<View, Pane>,
    pub section: usize,
    /// Wrapped settings explanation; selection events reset it, focus and
    /// changing check timestamps do not.
    pub(crate) settings_detail: crate::ui::Scroll,
    /// The settings nav's cursor is on the heading of `section`'s group.
    pub on_heading: bool,
    /// Settings groups folded in the nav.
    pub folded: std::collections::HashSet<&'static str>,
    /// The status line's message (its words, and an icon drawn before them).
    pub message: Option<crate::plugin::Message>,
    pub notices: crate::notices::Notices,
    pub(crate) notice_history: Option<crate::notices::HistorySelection>,
    notice_input_supported: bool,
    notice_motion: bool,
    /// A drag is selecting text (the Ratatui runner's): the key that makes
    /// the terminal select its own text instead ([`crate::select::bypass_key`]),
    /// which the status line names while it lasts.
    pub selecting: Option<&'static str>,
    /// Detected terminal layers and the current right-click routing state.
    pub(crate) terminal: Option<crate::terminal_host::State>,
    pub now: i64,
    pub quit: bool,
    /// `[theme]` and the colours section that edits it.
    pub colours: Colours,
    /// The context menu on screen, if any.
    pub menu: Option<OpenMenu>,
    /// While the keys menu is open: what each of its items does.
    menu_keys: Vec<Option<Press>>,
    palette: Option<palette::State>,
    palette_generation: u64,
    palette_visits: HashMap<palette::Place, u64>,
    /// Where shows across parts came from, the latest last.
    pub back: Vec<Return>,
    /// The selected row of the parts section.
    pub parts_row: usize,
    /// The cursor in Settings' look section.
    pub look_row: usize,
    /// "save and quit" was chosen: quit once the save succeeds.
    quit_after_save: bool,
    /// An event changed the config while something was being typed: save
    /// it once the typing ends.
    changed: bool,
    /// Saves sent to the runner and not answered yet, oldest first: what
    /// each writes, and whether `w` asked for it (only then is "saved" said).
    saving: std::collections::VecDeque<(toml::Table, bool)>,
    /// Why the last save failed, until one succeeds.
    pub save_failed: Option<String>,
    /// Advances every tick; drives spinners.
    pub frame: usize,
    /// Lifetime shell-render invocations on this host, independent of ticks.
    /// Kept on the owned host so parallel or interleaved fixtures cannot mix.
    #[cfg(feature = "test-support")]
    pub(crate) scenario_shell_frames: Cell<u64>,
    /// Clickable areas of the last frame, in drawing order (later on top).
    pub hits: std::cell::RefCell<Vec<(ratatui::layout::Rect, Hit)>>,
    /// Where the mouse pointer is, while it is over the window.
    pub pointer: Option<ratatui::layout::Position>,
    /// What the pointer is over as the last frame drew it: the area that
    /// is lit and what a click there does ([`Host::hover_at`]).
    pub hovered: std::cell::RefCell<Option<(ratatui::layout::Rect, Hit)>>,
    /// The Ratatui frontend's own state for parts drawn from views
    /// ([`crate::view_tui`]): scroll offsets and click targets, never a
    /// part's state.
    pub frontend: crate::view_tui::Frontend,
    /// The last press: what it hit and when, for double clicks.
    last_press: Option<(Hit, u64)>,
    detail_lease: std::cell::RefCell<detail_actions::State>,
    saved: toml::Table,
    /// The config file as last read or written: a save edits it, so what radar
    /// does not own (a part this build lacks, comments) stays.
    file: String,
    /// What happened that motion shows, for the runner ([`Host::take_cues`]).
    cues: Vec<Cue>,
    /// Bounded semantic interaction evidence; never retained renderer cells.
    interaction_motion: interaction_motion::State,
    /// `[general] mode`: which writes radar may send ([`crate::mode`]).
    pub mode_cfg: crate::writes::Mode,
    /// The mode the environment sets for this run, and the variable.
    pub mode_env: Option<(crate::writes::Mode, String)>,
    /// The pause every part shares ([`crate::mode::PauseState`]).
    pub pause: crate::mode::PauseState,
    /// Setup, while it shows ([`crate::setup`]).
    pub setup: Option<crate::setup::Setup>,
    /// The tour, while it shows ([`crate::tour`]).
    pub tour: Option<crate::tour::Tour>,
    /// The start line on the status line, and when it goes (unix seconds).
    start_line: Option<(String, i64)>,
    /// Where the config file is, as the user would write it
    /// (`~/.config/radar/config.toml`); the runner says.
    pub config_place: Option<String>,
}

/// What the screen showed before an event, to tell what it changed.
struct Seen {
    view: View,
    depth: Option<usize>,
    menu: Option<(String, String)>,
    notice_id: u64,
    notices_active: bool,
    /// Setup's screen, while it shows.
    setup: Option<crate::setup::At>,
}

/// Cues kept for a runner that does not take them (tests).
const CUES_KEPT: usize = 32;

impl Host {
    /// Register plugins. Fails when two plugins claim the same id or screen
    /// key, two sections share a name under one group, or a screen key is one
    /// the host reserves.
    pub fn new(plugins: Vec<Box<dyn Plugin>>, now: i64) -> Result<Self> {
        let mut screens = Vec::new();
        // General first, then each part's.
        let mut sections = vec![
            (Owner::Parts, PARTS_SECTION),
            (Owner::Mode, crate::mode::MODE_SECTION),
            (Owner::Look, LOOK_SECTION),
            (Owner::Host, COLOURS_SECTION),
            (Owner::Host, ICONS_SECTION),
        ];
        let mut slots: Vec<(String, SlotValue)> = Vec::new();
        let mut ids = Vec::new();
        for (i, p) in plugins.iter().enumerate() {
            if ids.contains(&p.id()) {
                bail!("two plugins are called {}", p.id());
            }
            if [
                COLOURS_SECTION.id,
                LOOK_SECTION.id,
                PARTS_SECTION.id,
                ICONS_SECTION.id,
                ICONS_TABLE,
                "theme",
                crate::mode::MODE_SECTION.id,
                crate::mode::GENERAL_TABLE,
                "terminal",
                "setup",
            ]
            .contains(&p.id())
            {
                bail!("{}: that name is the host's", p.id());
            }
            ids.push(p.id());
            for s in p.screens() {
                if s.key == SETTINGS_KEY
                    || s.key == HISTORY_KEY
                    || RESERVED.contains(&s.key)
                    || !s.key.is_ascii_uppercase()
                {
                    bail!(
                        "{}: screen key {} is reserved or not Shift+letter",
                        p.id(),
                        s.key
                    );
                }
                if screens
                    .iter()
                    .any(|(_, o): &(usize, ScreenSpec)| o.key == s.key)
                {
                    bail!("{}: screen key {} is already taken", p.id(), s.key);
                }
                screens.push((i, s));
            }
            for s in p.sections() {
                // A name is unique under its group's heading, so every part
                // can have its own `connection`.
                if sections
                    .iter()
                    .any(|(_, o): &(Owner, SectionSpec)| o.group == s.group && o.name == s.name)
                {
                    bail!(
                        "{}: settings section {} is already taken under {}",
                        p.id(),
                        s.name,
                        s.group
                    );
                }
                sections.push((Owner::Plugin(i), s));
            }
            for (name, value) in p.slots() {
                if slots.iter().any(|(n, _)| *n == name) {
                    bail!("{}: theme slot {name} is already declared", p.id());
                }
                slots.push((name, value));
            }
        }
        // Settings opens on the first part's settings, where setting up starts.
        let section = sections
            .iter()
            .position(|(o, _)| matches!(o, Owner::Plugin(_)))
            .unwrap_or(0);
        let mut colours = Colours {
            declared: slots,
            ..Default::default()
        };
        colours.first_row();
        colours.icons_first_row();
        let wake = crate::wake::WakeHub::default();
        let mut plugins = plugins;
        for (i, p) in plugins.iter_mut().enumerate() {
            p.set_waker(wake.waker(i));
        }
        Ok(Self {
            context_service: crate::context::Service::default(),
            review_context_started: std::collections::BTreeMap::new(),
            wake,
            history: None,
            history_request: 0,
            trace: Trace::default(),
            view: if screens.is_empty() {
                View::Settings
            } else {
                View::Screen(0)
            },
            plugins,
            screens,
            sections,
            focus: Pane::Middle,
            dock: Default::default(),
            bar: false,
            sources: false,
            tabs: false,
            last_pane: HashMap::new(),
            section,
            settings_detail: crate::ui::Scroll::new(),
            on_heading: false,
            folded: Default::default(),
            message: None,
            notices: Default::default(),
            notice_history: None,
            notice_input_supported: false,
            // The runner and the bridge set it from the motion level each frame; a still render shows notices whole.
            notice_motion: false,
            selecting: None,
            terminal: None,
            now,
            quit: false,
            colours,
            menu: None,
            menu_keys: Vec::new(),
            palette: None,
            palette_generation: 0,
            palette_visits: HashMap::new(),
            back: Vec::new(),
            parts_row: 0,
            look_row: 0,
            quit_after_save: false,
            changed: false,
            saving: Default::default(),
            save_failed: None,
            frame: 0,
            #[cfg(feature = "test-support")]
            scenario_shell_frames: Cell::new(0),
            hits: std::cell::RefCell::new(Vec::new()),
            pointer: None,
            hovered: std::cell::RefCell::new(None),
            frontend: crate::view_tui::Frontend::default(),
            last_press: None,
            detail_lease: Default::default(),
            saved: toml::Table::new(),
            file: String::new(),
            cues: Vec::new(),
            interaction_motion: Default::default(),
            mode_cfg: crate::writes::Mode::default(),
            mode_env: None,
            pause: crate::mode::PauseState {
                last_input: now,
                ..Default::default()
            },
            setup: None,
            tour: None,
            start_line: None,
            config_place: None,
        })
        .inspect(|host| host.apply_mode())
    }

    pub fn build_theme(&self, detected: crate::design::theme::Tier) -> Theme {
        let theme = Theme::build(&self.colours.cfg, detected, &self.colours.declared);
        crate::ui::icon::set_current(theme.icons());
        theme
    }

    /// The settings section showing is the host's icons.
    pub fn on_icons(&self) -> bool {
        self.view == View::Settings
            && self
                .current_section()
                .is_some_and(|(o, s)| o == Owner::Host && s.id == ICONS_SECTION.id)
    }

    fn colours_key(&mut self, key: KeyEvent) -> Vec<HostEffect> {
        if !self.colours_take(key) && key.code == KeyCode::Esc {
            self.focus = Pane::Nav;
        }
        vec![]
    }

    /// A key for the colours (or icons) section; false when it did not
    /// take it.
    pub(crate) fn colours_take(&mut self, key: KeyEvent) -> bool {
        let reply = if self.on_icons() {
            self.colours.icons_key(key)
        } else {
            self.colours.key(key)
        };
        match reply {
            Reply::Ignored => return false,
            Reply::Done => {}
            Reply::Say(m) => self.notify(m.into()),
            Reply::Error(m) => self.notify_error(m),
            Reply::Menu(menu) => self.menu = Some(OpenMenu::open(Owner::Host, menu)),
        }
        true
    }

    /// Keys while a menu is open: move, choose by `⏎` or the item's key, close.
    /// Ask the part what it makes of the text in the open menu's field:
    /// its verdict as the first line, then, when `offers`, its choices
    /// (else the choices stay as they were, as when an arrow filled the
    /// field from them).
    fn typed_changed(&mut self, offers: bool) {
        if self.palette.is_some() && self.menu.as_ref().is_some_and(|open| open.owner == Owner::Host && open.menu.id == palette::ID) {
            self.palette_changed();
            return;
        }
        let Some(open) = self.menu.as_mut() else {
            return;
        };
        let (Some(input), Owner::Plugin(p)) = (&open.input, open.owner) else {
            return;
        };
        let typing = self.plugins[p].typing(&open.menu.id, &input.value());
        open.refused = match &typing.verdict {
            Some(crate::Verdict::Bad(why)) => Some(why.clone()),
            _ => None,
        };
        let verdict = typing.verdict.map(|v| {
            use crate::ui::icon::Icon;
            let (icon, mark, words) = match v {
                crate::Verdict::Good(w) => (Icon::RunPassed, "✓", w),
                crate::Verdict::Unsure(w) => (Icon::Warning, "⚠", w),
                crate::Verdict::Bad(w) => (Icon::Error, "✗", w),
            };
            let (icon, mark) = crate::ui::icon::plain_or(icon, mark);
            MenuItem::note(format!("{mark}{words}")).icon(icon)
        });
        let kept: Vec<MenuItem> = if offers {
            typing.offers
        } else {
            open.menu
                .items
                .iter()
                .filter(|i| !i.note)
                .cloned()
                .collect()
        };
        open.menu.items = verdict.into_iter().chain(kept).collect();
        if offers {
            open.row = NO_ROW;
            open.viewport.at.set(0);
            open.viewport.selected.set(None);
        }
    }

    /// A key in a typed menu's field.
    fn typed_key(&mut self, key: KeyEvent) -> Vec<HostEffect> {
        let Some(open) = self.menu.as_mut() else {
            return vec![];
        };
        let picks = open.picks();
        let at = picks.iter().position(|i| *i == open.row);
        let fill = |open: &mut OpenMenu, i: usize| {
            open.row = i;
            open.input = Some(crate::input::TextInput::new(&open.menu.items[i].value));
        };
        match key.code {
            KeyCode::Esc => {
                self.cancel_menu_trace(true);
                self.menu = None;
                return vec![];
            }
            KeyCode::Up | KeyCode::Down if !picks.is_empty() => {
                let next = match (at, key.code == KeyCode::Down) {
                    (None, true) => 0,
                    (None, false) => picks.len() - 1,
                    (Some(a), true) => (a + 1) % picks.len(),
                    (Some(a), false) => (a + picks.len() - 1) % picks.len(),
                };
                fill(open, picks[next]);
                self.typed_changed(false);
                return vec![];
            }
            KeyCode::Tab => {
                if let Some(&i) = at.map(|a| &picks[a]).or(picks.first()) {
                    fill(open, i);
                    self.typed_changed(true);
                }
                return vec![];
            }
            KeyCode::Enter => {
                if !open.viewport.usable.get() {
                    return vec![];
                }
                // A click (or ⏎) on an offer not yet in the field puts it there.
                if let Some(i) = at.map(|a| picks[a])
                    && open
                        .input
                        .as_ref()
                        .is_some_and(|f| f.value() != open.menu.items[i].value)
                {
                    fill(open, i);
                    self.typed_changed(false);
                    return vec![];
                }
                let text = open.input.as_ref().map(|f| f.value()).unwrap_or_default();
                if let Some(why) = open.refused.clone() {
                    self.notify_error(format!("not yet: {why}"));
                    return vec![];
                }
                let open = self.menu.take().expect("menu is open");
                let Owner::Plugin(p) = open.owner else {
                    return vec![];
                };
                self.complete_menu_trace(false);
                if self.view == View::Settings {
                    self.settings_detail.top();
                }
                let fx = self.plugins[p].on_menu(&open.menu.id, &text);
                return self.route(p, fx, 0);
            }
            _ => {}
        }
        if let Some(field) = open.input.as_mut() {
            match field.key(key) {
                crate::input::Edit::Cancel => {
                    self.cancel_menu_trace(true);
                    self.menu = None;
                    return vec![];
                }
                crate::input::Edit::Typing | crate::input::Edit::Submit(_) => {}
            }
        }
        self.typed_changed(true);
        vec![]
    }

    fn menu_key(&mut self, key: KeyEvent) -> Vec<HostEffect> {
        if self.palette.is_some() {
            if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                if let Some(open) = &mut self.menu { open.viewport.scroll(key.code == KeyCode::PageDown, true); }
                return vec![];
            }
            return self.palette_key(key);
        }
        // Reading never submits, selects an offer, or reaches the input's
        // editing shortcuts. Modified keys retain their complete binding.
        if key.modifiers.is_empty()
            && matches!(key.code, KeyCode::PageUp | KeyCode::PageDown)
            && let Some(open) = &self.menu
        {
            open.viewport.scroll(key.code == KeyCode::PageDown, true);
            return vec![];
        }
        if self.menu.as_ref().is_some_and(|m| m.input.is_some()) {
            return self.typed_key(key);
        }
        let Some(open) = self.menu.as_mut() else {
            return vec![];
        };
        let keys_menu = open.owner == Owner::Host && open.menu.id == KEYS_MENU;
        // A modified binding keeps its complete key. In particular Ctrl-R
        // must never take an ordinary menu's `r` action (or vice versa).
        let modified = !key.modifiers.difference(KeyModifiers::SHIFT).is_empty();
        let chosen = if modified {
            if keys_menu {
                self.menu_keys.iter().position(|press| {
                    matches!(press, Some(Press::Key(bound, _) | Press::TreeKey(bound, _))
                        if bound.code == key.code && bound.modifiers == key.modifiers)
                })
            } else {
                None
            }
        } else {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.cancel_menu_trace(true);
                    self.menu = None;
                    return vec![];
                }
                // The keys that open the keys menu close it again.
                KeyCode::Char('x' | '?') if keys_menu => {
                    self.cancel_menu_trace(true);
                    self.menu = None;
                    return vec![];
                }
                KeyCode::Enter => Some(open.row),
                KeyCode::Char(c) if open.menu.items.iter().any(|i| i.key == Some(c)) => {
                    open.menu.items.iter().position(|i| i.key == Some(c))
                }
                _ => {
                    // A question's buttons sit in a row: ←/→ (h/l, tab) move
                    // along it too.
                    let along = open.menu.question.then(|| horizontal(&key)).flatten();
                    if let Some(down) = vertical(&key).or(along) {
                        open.row = next_item(&open.menu.items, open.row, down);
                    }
                    None
                }
            }
        };
        let Some(i) = chosen else { return vec![] };
        if !open.viewport.usable.get() {
            return vec![];
        }
        if open.menu.items.get(i).is_none_or(|item| item.heading) {
            return vec![];
        }
        let open = self.menu.take().expect("menu is open");
        let item = &open.menu.items[i];
        let press = keys_menu
            .then(|| {
                item.value
                    .parse::<usize>()
                    .ok()
                    .and_then(|k| self.menu_keys.get(k).cloned().flatten())
            })
            .flatten();
        let stale = match &press {
            Some(Press::Key(_, Some(context))) => !self.menu_key_is_current(context),
            Some(Press::TreeKey(key, context)) => !self.tree_menu_is_current(*key, context),
            Some(Press::Act(plugin, subject, id, Some(context))) => {
                !self.menu_subject_is_current(context, *plugin, subject, id)
            }
            _ => false,
        };
        if stale {
            self.trace
                .record(self.trace.current.clone(), JournalOutcome::Blocked, None);
            if let Some(menu) = self.trace.menu.take() {
                self.trace
                    .record(Some(menu.action), JournalOutcome::Blocked, None);
            }
            self.menu_keys.clear();
            self.notify_error("that target changed; press x to choose again");
            return vec![];
        }
        self.complete_menu_trace(open.menu.question && item.value == "no");
        if self.view == View::Settings && !keys_menu && item.value != "no" {
            self.settings_detail.top();
        }
        match open.owner {
            Owner::Host if keys_menu => match press {
                Some(Press::History(subject)) => self.open_history(subject),
                Some(Press::RecentHistory) => self.open_recent_history(),
                Some(Press::Key(k, _) | Press::TreeKey(k, _)) => self.on_key(k),
                Some(Press::Act(p, subject, id, _)) => self.contributed_act(p, &subject, &id, 0),
                None => vec![],
            },
            Owner::Host if open.menu.id == crate::mode::MODE_MENU => {
                self.on_mode_menu(&item.value.clone());
                vec![]
            }
            Owner::Host if open.menu.id == crate::mode::IDLE_MENU => {
                self.on_idle_menu(&item.value.clone());
                vec![]
            }
            Owner::Host if open.menu.id.starts_with(crate::health::MENU) => {
                let part = open.menu.id.split_once(':').map(|(_, id)| id);
                match part.and_then(|id| self.plugins.iter().position(|p| p.id() == id)) {
                    Some(p) if item.value == "refresh" => {
                        let fx = self.plugins[p].refresh_now();
                        self.route(p, fx, 0)
                    }
                    _ => vec![],
                }
            }
            Owner::Host if open.menu.id == QUIT_MENU => match item.value.as_str() {
                "save" => {
                    self.quit_after_save = true;
                    vec![self.save(true)]
                }
                "discard" => {
                    self.quit = true;
                    vec![]
                }
                _ => vec![],
            },
            Owner::Host => {
                if let Some(m) = self.colours.on_menu(&open.menu.id, &item.value) {
                    self.notify(m.into());
                }
                vec![]
            }
            Owner::Parts | Owner::Mode | Owner::Look => vec![],
            Owner::Plugin(p) => {
                let fx = self.plugins[p].on_menu(&open.menu.id, &item.value);
                self.route(p, fx, 0)
            }
        }
    }

    fn in_colours_section(&self) -> bool {
        self.view == View::Settings
            && self.focus == Pane::Middle
            && self
                .current_section()
                .is_some_and(|(o, _)| o == Owner::Host)
    }

    /// Hand each plugin its table from the config file. With no file, plugins
    /// run their first-run setup; the file is written with the first change.
    pub fn load_config(&mut self, text: Option<&str>) -> Result<()> {
        self.context_service.clear();
        let table: toml::Table = match text {
            Some(t) => toml::from_str(t)?,
            None => toml::Table::new(),
        };
        self.file = text.unwrap_or_default().to_string();
        for p in &mut self.plugins {
            p.configure(table.get(p.id()))?;
        }
        self.colours.cfg = match table.get("theme") {
            Some(v) => v.clone().try_into()?,
            None => ThemeConfig::default(),
        };
        self.colours.cfg.icon_overrides = crate::ui::icon::overrides(table.get(ICONS_TABLE));
        self.mode_cfg = match table
            .get(crate::mode::GENERAL_TABLE)
            .and_then(|g| g.get("mode"))
        {
            None => crate::writes::Mode::default(),
            Some(v) => v
                .as_str()
                .and_then(crate::writes::Mode::parse)
                .ok_or_else(|| {
                    anyhow::anyhow!("[general] mode = {v}: it is read-only, write or edit")
                })?,
        };
        self.apply_mode();
        self.read_idle(&table);
        let unfinished = table
            .get(crate::mode::GENERAL_TABLE)
            .and_then(|g| g.get("setup"))
            .and_then(|v| v.as_str())
            == Some("unfinished");
        if text.is_some() && unfinished {
            // Quit during setup: it opens again, from its start.
            let _ = self.open_setup(None);
            self.saved = self.config_table();
        } else if text.is_some() {
            self.saved = self.config_table();
        } else {
            // First run: setup, from its start. The parts' first-run notes
            // are what setup's steps cover.
            for p in &mut self.plugins {
                let _ = p.first_run();
            }
            let _ = self.open_setup(None);
        }
        self.changed = false;
        Ok(())
    }

    /// Something is being typed (a part's field, or a colour): a change is
    /// not complete until it is kept.
    fn typing(&self) -> bool {
        self.colours.capturing() || self.plugins.iter().any(|p| p.capturing())
    }

    /// Write the config as it is now. `asked`: `w` asked for it, so say
    /// when it is written.
    pub(crate) fn save(&mut self, asked: bool) -> HostEffect {
        let table = self.config_table();
        let mut owned: Vec<&str> = self.plugins.iter().map(|p| p.id()).collect();
        owned.extend(["theme", ICONS_TABLE, crate::mode::GENERAL_TABLE]);
        let text = merged_config(&self.file, &table, &owned);
        self.file = text.clone();
        self.saving.push_back((table, asked));
        HostEffect::Save(text)
    }

    /// Save after a completed change: the event changed the config (`before`
    /// is how it found it), or one did while typing, and nothing is being
    /// typed now. Never mid-typing: such a change is saved once the typing
    /// ends. Nothing is written that is already written or on its way.
    fn autosave(&mut self, before: &toml::Table) -> Option<HostEffect> {
        let now = self.config_table();
        self.changed |= now != *before;
        if !self.changed || self.typing() {
            return None;
        }
        self.changed = false;
        let pending = self.saving.back().map_or(&self.saved, |(t, _)| t);
        (now != *pending).then(|| self.save(false))
    }

    /// Settings that are not in the file because writing it failed. (With
    /// saving after each change, nothing else can be unsaved.)
    pub fn unsaved(&self) -> bool {
        self.save_failed.is_some() && self.dirty()
    }

    pub fn config_table(&self) -> toml::Table {
        let mut table: toml::Table = self
            .plugins
            .iter()
            .filter_map(|p| p.config().map(|v| (p.id().to_string(), v)))
            .collect();
        if let Ok(theme) = toml::Value::try_from(&self.colours.cfg) {
            table.insert("theme".into(), theme);
        }
        if !self.colours.cfg.icon_overrides.is_empty()
            && let Ok(icons) = toml::Value::try_from(&self.colours.cfg.icon_overrides)
        {
            table.insert(ICONS_TABLE.into(), icons);
        }
        // Edit, the default, is not written.
        let mut general = toml::Table::new();
        if self.mode_cfg != crate::writes::Mode::default() {
            general.insert("mode".into(), self.mode_cfg.key().into());
        }
        if self.pause.idle_minutes != crate::mode::DEFAULT_IDLE_MINUTES {
            general.insert(
                "idle_minutes".into(),
                (self.pause.idle_minutes as i64).into(),
            );
        }
        // Setup walked from its start and not yet left: the next start
        // opens it again (quitting is not finishing).
        if self.setup.as_ref().is_some_and(|s| !s.single) {
            general.insert("setup".into(), "unfinished".into());
        }
        if !general.is_empty() {
            table.insert(crate::mode::GENERAL_TABLE.into(), general.into());
        }
        table
    }

    pub fn config_text(&self) -> String {
        toml::to_string_pretty(&self.config_table()).unwrap_or_default()
    }

    pub fn dirty(&self) -> bool {
        self.config_table() != self.saved
    }

    pub fn screen(&self) -> Option<(usize, ScreenSpec)> {
        match self.view {
            View::Screen(i) => self.screens.get(i).copied(),
            View::Settings => None,
        }
    }

    pub fn screen_group(&self, index: usize) -> Option<crate::plugin::ScreenGroup> {
        let (plugin, screen) = self.screens.get(index)?;
        self.plugins[*plugin].screen_group(screen.id)
    }

    /// Screen `i` is offered (a tab, a source): its part is set up. A part
    /// that is not shows nowhere until setup or Settings sets it up.
    pub fn screen_shown(&self, i: usize) -> bool {
        self.screens.get(i).is_some_and(|(p, _)| self.ready(*p))
    }

    /// At start: open on a screen that is offered, as a part not set up
    /// has none (the runner, after the config is read).
    pub fn open_on_a_shown_screen(&mut self) {
        if self.setup.is_none()
            && let View::Screen(i) = self.view
            && !self.screen_shown(i)
        {
            self.view = self.first_screen().map_or(View::Settings, View::Screen);
        }
    }

    /// The first screen offered, where radar opens.
    pub fn first_screen(&self) -> Option<usize> {
        (0..self.screens.len()).find(|&i| self.screen_shown(i))
    }

    pub fn top_screens(&self) -> Vec<usize> {
        let mut seen = std::collections::HashSet::new();
        self.screens
            .iter()
            .enumerate()
            .filter(|(i, _)| self.screen_shown(*i))
            .filter_map(|(i, _)| match self.screen_group(i) {
                Some(group) if !seen.insert(group.id) => None,
                _ => Some(i),
            })
            .collect()
    }

    pub fn group_screens(&self, index: usize) -> Vec<usize> {
        let Some(group) = self.screen_group(index) else {
            return vec![];
        };
        self.screens
            .iter()
            .enumerate()
            .filter_map(|(i, _)| {
                (self.screen_shown(i) && self.screen_group(i).is_some_and(|other| other.id == group.id))
                    .then_some(i)
            })
            .collect()
    }

    pub fn current_section(&self) -> Option<(Owner, SectionSpec)> {
        self.sections.get(self.section).copied()
    }

    /// Whether plugin `p` is set up (see [`compose::ready`]).
    /// The splash's start line, on the status line for five seconds: the
    /// splash went before it could show it.
    pub fn show_start_line(&mut self, words: String) {
        self.notify(words.clone().into());
        self.start_line = Some((words, self.now + 5));
    }

    pub fn ready(&self, p: usize) -> bool {
        compose::ready(self.plugins[p].as_ref())
    }

    /// The screen showing belongs to a part that is not set up yet: the
    /// host draws its set-up page and takes its keys.
    pub fn setting_up(&self) -> Option<usize> {
        self.screen().map(|(p, _)| p).filter(|&p| !self.ready(p))
    }

    /// Plugin `p`'s first settings section, where setting it up starts.
    pub(crate) fn first_section_of(&self, p: usize) -> Option<usize> {
        self.sections
            .iter()
            .position(|(o, _)| *o == Owner::Plugin(p))
    }

    /// Go to plugin `p`'s settings.
    pub(crate) fn go_to_settings_of(&mut self, p: usize) {
        if let Some(i) = self.first_section_of(p) {
            self.view = View::Settings;
            self.select_section(i);
            self.focus = Pane::Middle;
        }
    }

    /// Select section `i` in the settings nav, unfolding its group.
    pub(crate) fn select_section(&mut self, i: usize) {
        self.settings_detail.top();
        self.section = i;
        self.on_heading = false;
        if let Some((_, s)) = self.sections.get(i) {
            self.folded.remove(s.group);
        }
        if let Some((Owner::Plugin(p), _)) = self.sections.get(i) {
            let span = self.plugin_dispatch(*p, "plugin.settings.show");
            self.finish_dispatch(span, true);
        }
    }

    /// The settings nav's groups in order, each with its sections.
    pub fn nav_groups(&self) -> Vec<(&'static str, Vec<usize>)> {
        let mut groups: Vec<(&'static str, Vec<usize>)> = Vec::new();
        for (i, (_, s)) in self.sections.iter().enumerate() {
            match groups.iter_mut().find(|(g, _)| *g == s.group) {
                Some((_, items)) => items.push(i),
                None => groups.push((s.group, vec![i])),
            }
        }
        groups
    }

    /// A group's name for people: the part's own name when the group is a
    /// part's id (its `name()`), else the word with a capital ("General").
    pub fn group_name(&self, group: &str) -> String {
        if let Some(p) = self.plugins.iter().find(|p| p.id() == group) {
            return p.name().to_string();
        }
        let mut c = group.chars();
        c.next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default()
    }

    /// A section's panel title: its name, with its group's in front when
    /// another group has a section of that name ("Shelves connection").
    pub fn section_title(&self, i: usize) -> String {
        let Some((_, s)) = self.sections.get(i) else {
            return String::new();
        };
        let shared = self
            .sections
            .iter()
            .any(|(_, o)| o.name == s.name && o.group != s.group);
        if shared {
            format!("{} {}", self.group_name(s.group), s.name)
        } else {
            s.name.to_string()
        }
    }

    /// The settings nav's rows as shown: each group's heading (`None`),
    /// then its sections unless it is folded.
    pub fn nav_rows(&self) -> Vec<(&'static str, Option<usize>)> {
        let mut rows = Vec::new();
        for (g, items) in self.nav_groups() {
            rows.push((g, None));
            if !self.folded.contains(g) {
                rows.extend(items.into_iter().map(|i| (g, Some(i))));
            }
        }
        rows
    }

    /// The nav row the cursor is on.
    pub fn nav_at(&self) -> (&'static str, Option<usize>) {
        let g = self.current_section().map_or("", |(_, s)| s.group);
        if self.on_heading || self.folded.contains(g) {
            (g, None)
        } else {
            (g, Some(self.section))
        }
    }

    /// Put the nav's cursor on a row: a heading shows its group's first
    /// section.
    fn nav_go(&mut self, (g, i): (&'static str, Option<usize>)) {
        self.settings_detail.top();
        match i {
            Some(i) => self.select_section(i),
            None => {
                if let Some(first) = self.sections.iter().position(|(_, s)| s.group == g) {
                    self.section = first;
                }
                self.on_heading = true;
            }
        }
    }

    /// Fold or unfold a group in the settings nav; the cursor goes to its
    /// heading.
    fn toggle_group(&mut self, g: &'static str) {
        let before = self.settings_fold_before(g);
        if !self.folded.remove(g) {
            self.folded.insert(g);
        }
        self.nav_go((g, None));
        self.settings_fold_after(g, before);
    }

    /// `+` or `-` found nothing to open or close on the row.
    fn nothing_to_fold(&mut self, c: char) {
        self.notify(
            if c == '-' {
                "nothing to collapse here"
            } else {
                "nothing to expand here: it is open, or has nothing under it"
            }
            .into(),
        );
    }

    /// Whether the focused panel is a tree whose groups fold with `+` and
    /// `-`: the settings nav, or a part's ([`Plugin::folds`]).
    pub fn folds_here(&self) -> bool {
        if let Some(tree) = self.focused_tree() {
            return !tree.groups().is_empty();
        }
        if self.menu.is_some() || self.bar || self.tabs {
            return false;
        }
        match self.view {
            View::Settings if self.focus == Pane::Nav => true,
            View::Settings => match self.current_section() {
                Some((Owner::Plugin(p), s)) if self.focus == Pane::Middle => {
                    self.plugins[p].section_folds(s.id)
                }
                _ => false,
            },
            View::Screen(_) => match self.screen() {
                Some((p, s)) if self.ready(p) && self.setting_up().is_none() => {
                    self.plugins[p].folds(s.id, self.focus)
                }
                _ => false,
            },
        }
    }

    /// The plugin that receives keys right now, if any.
    /// Something is being typed: a colour, or into a part's field.
    pub fn capturing(&self) -> bool {
        self.colours.capturing() || self.target().is_some_and(|p| self.plugins[p].capturing())
    }

    fn target(&self) -> Option<usize> {
        match self.view {
            View::Screen(_) => self.screen().map(|(p, _)| p).filter(|&p| self.ready(p)),
            // Setup's own steps have no section under them.
            View::Settings if self.setup_host_step() => None,
            View::Settings => match self.current_section() {
                Some((Owner::Plugin(p), _)) => Some(p),
                _ => None,
            },
        }
    }

    /// Handle one event; then, unless it was a save's answer, save a change
    /// it completed (see [`Host::autosave`]).
    pub fn update(&mut self, event: Event) -> Vec<HostEffect> {
        self.guard_notice_history_owner();
        if let Some(out) = self.update_notices(&event) {
            self.reconcile_detail_lease();
            return out;
        }
        // Every event supersedes unconsumed evidence. In particular background
        // sync/tick changes cannot inherit the last input's fold causality.
        self.interaction_motion.fold = None;
        let before = self.view;
        let out = self.update_inner(event);
        self.guard_notice_history_owner();
        if self.view != before {
            self.visit_palette_place(match self.view {
                View::Screen(i) => palette::Place::Screen(i),
                View::Settings => palette::Place::Settings,
            });
        }
        // An outside click or a background question may dismiss/replace the
        // palette. Restore suspended History only when no new menu owns input.
        if self.palette.is_some() && self.menu.as_ref().is_none_or(|open| open.owner != Owner::Host || open.menu.id != palette::ID) {
            if self.menu.is_none() { self.cancel_palette(); } else { self.clear_palette_hits(); self.palette = None; }
        }
        self.tell_dock();
        self.finish_motion_event();
        self.reconcile_detail_lease();
        out
    }

    /// Tell the part that owns the dock when the dock is shown or hidden,
    /// whatever showed or hid it (a key, a click, an effect), so what ends
    /// out of sight is marked unread and what is shown is seen.
    fn tell_dock(&mut self) {
        let visible = self.dock.visible;
        if self.dock.told_visible == Some(visible) {
            return;
        }
        if let Some(p) = self.dock_owner() {
            self.dock.told_visible = Some(visible);
            self.plugins[p].dock_visible(visible);
        }
    }

    /// The tabs that want the user out of sight, worst first, with the
    /// active tab's id.
    pub fn dock_wanting(&self) -> Vec<(crate::dock::Want, crate::dock::DockTab)> {
        let Some((tabs, active)) = self.plugins.iter().find_map(|p| p.dock_tabs()) else {
            return vec![];
        };
        crate::dock::wanting(&tabs, active.as_deref(), self.dock.visible)
            .into_iter()
            .map(|(w, t)| (w, t.clone()))
            .collect()
    }

    /// The dock key, or a click on the status line's attention chip, while a
    /// tab wants the user out of sight: the dock shows on that tab, with the
    /// keys. `None` when nothing wants the user.
    fn go_to_attention(&mut self) -> Option<Vec<HostEffect>> {
        let (_, tab) = self.dock_wanting().into_iter().next()?;
        let mut out = self.on_dock_intent(crate::dock::DockIntent::SwitchTab { id: tab.id });
        out.extend(self.on_dock_intent(crate::dock::DockIntent::Show));
        Some(out)
    }

    fn update_inner(&mut self, event: Event) -> Vec<HostEffect> {
        // Reading history is intentionally not a source of new history. Background
        // provider completions still follow the ordinary trace and reducer route.
        if let Event::History { id, result } = event {
            if let Some(history) = self.history.as_mut().or_else(|| self.palette.as_mut().and_then(|state| state.history.as_mut())) {
                history.receive(id, result.map(|page| *page));
            }
            return vec![];
        }
        let mut input = &event;
        while let Event::Tracked { event, .. } = input { input = event; }
        let opens_palette = match input {
            Event::Key(key) => palette::shortcut(*key),
            Event::Intent(crate::view::Intent::Key { chord }) => palette::shortcut(crate::view_tui::key_of(*chord)),
            _ => false,
        };
        if opens_palette && self.menu.is_none() && !self.capturing() {
            return self.handle_outside_history(event);
        }
        if self.history.is_some() {
            // Tracked input is the overlay's as untracked input is: read
            // through the wrappers, and not audited (browsing history is
            // not activity).
            let mut inner = &event;
            while let Event::Tracked { event, .. } = inner {
                inner = event;
            }
            let input = matches!(
                inner,
                Event::Key(_) | Event::Intent(_) | Event::Paste(_) | Event::Mouse { .. }
            );
            let mut event = event;
            while input && let Event::Tracked { event: inner, .. } = event {
                event = *inner;
            }
            match event {
                Event::Key(key) => {
                    self.someone_is_there();
                    return self.history_key(key);
                }
                // Ink and Headless send keys as intents: the overlay takes
                // them as it takes the terminal's.
                Event::Intent(crate::view::Intent::Key { chord }) => {
                    self.someone_is_there();
                    return self.history_key(crate::view_tui::key_of(chord));
                }
                // A chrome target (a tab, a row or key of the overlay) acts;
                // anything meant for the screen, section or menu under the
                // overlay is dropped, never delivered behind it.
                Event::Intent(intent) => {
                    self.someone_is_there();
                    return match &intent {
                        // Text selected on the overlay copies like any other.
                        crate::view::Intent::Copy { .. } => self.on_intent(intent),
                        crate::view::Intent::Activate { target } | crate::view::Intent::Select { target }
                            if target.as_str().starts_with(crate::view::chrome::targets::PREFIX) =>
                        {
                            self.on_intent(intent)
                        }
                        _ => vec![],
                    };
                }
                Event::Paste(_) => return vec![],
                Event::Mouse {
                    kind, column, row, ..
                } => {
                    self.someone_is_there();
                    if kind == MouseKind::Press {
                        // A tab, or a row or key of the overlay: pressed by
                        // its target, as every frontend presses it.
                        let at = ratatui::layout::Position::new(column, row);
                        let target = match self.hit_at(column, row) {
                            Some(hit @ (Hit::Tab(_) | Hit::HistoryTab)) => self.chrome_target_of(&hit),
                            _ => self.history.as_ref().and_then(|h| h.target_at(at)),
                        };
                        if let Some(target) = target {
                            return self.on_intent(crate::view::Intent::Activate { target });
                        }
                    }
                    let key = self
                        .history
                        .as_mut()
                        .and_then(|history| history.mouse(kind, column, row));
                    return key.map_or_else(Vec::new, |key| self.history_key(key));
                }
                other => return self.handle_outside_history(other),
            }
        }
        self.handle_outside_history(event)
    }

    /// [`Host::update`] for what the History overlay does not take.
    fn handle_outside_history(&mut self, event: Event) -> Vec<HostEffect> {

        let (event, inherited, semantic) = match event {
            Event::Tracked {
                action,
                semantic,
                event,
            } => (*event, Some(action), semantic),
            event => (event, None, None),
        };
        self.trace.mode = Some(match self.mode() {
            crate::writes::Mode::ReadOnly => crate::journal::WriteMode::ReadOnly,
            crate::writes::Mode::Write => crate::journal::WriteMode::Write,
            crate::writes::Mode::Edit => crate::journal::WriteMode::Edit,
        });
        let completed_task = matches!(event, Event::Msg { .. })
            .then(|| inherited.clone())
            .flatten();
        self.trace.delivering_task = completed_task.clone();
        self.trace.current = inherited;
        self.trace.semantic = semantic;
        let menu_before = self.menu.is_some();
        if menu_before && matches!(event, Event::Key(_) | Event::Mouse { .. } | Event::Intent(_)) {
            self.trace.current = self
                .trace
                .menu
                .as_ref()
                .and_then(|menu| menu.parent.clone());
            self.trace.semantic = self
                .trace
                .menu
                .as_ref()
                .and_then(|menu| menu.semantic.clone());
        }
        let kind = match &event {
            Event::Key(k)
                if !menu_before
                    && !self.capturing()
                    && k.code == KeyCode::Char(HISTORY_KEY)
                    && k.modifiers.difference(KeyModifiers::SHIFT).is_empty() =>
            {
                None
            }
            // The History key as a renderer's intent: not journalled either.
            Event::Intent(crate::view::Intent::Key { chord })
                if !menu_before
                    && !self.capturing()
                    && chord.key == crate::view::Key::Char(HISTORY_KEY)
                    && !chord.ctrl
                    && !chord.alt =>
            {
                None
            }
            Event::Key(k)
                if self.menu.as_ref().is_some_and(|m| m.input.is_some())
                    || (self.capturing() && self.menu.is_none()) =>
            {
                match k.code {
                    KeyCode::Enter => Some("input.commit"),
                    KeyCode::Esc => Some("input.cancel"),
                    _ => None,
                }
            }
            // The same typing from a renderer (Headless, Ink): journalled as
            // the terminal's keys are.
            Event::Intent(crate::view::Intent::Key { chord })
                if self.menu.as_ref().is_some_and(|m| m.input.is_some())
                    || (self.capturing() && self.menu.is_none()) =>
            {
                match chord.key {
                    crate::view::Key::Enter => Some("input.commit"),
                    crate::view::Key::Esc => Some("input.cancel"),
                    _ => None,
                }
            }
            // Copying what is selected acts on no screen: its own span is
            // `system.copy.selection`, not a `ui.action` over it.
            Event::Intent(crate::view::Intent::Copy { .. }) => None,
            Event::Key(_) | Event::Intent(_) if menu_before => Some("ui.menu.action"),
            Event::Key(_) | Event::Intent(_) => Some("ui.action"),
            Event::Mouse { .. } if menu_before => Some("ui.menu.action"),
            Event::Mouse { .. } if !menu_before && !self.capturing() => Some("ui.pointer.action"),
            _ => None,
        };
        let dispatch = kind.and_then(|kind| self.trace.host(kind, self.trace.current.clone()));
        if dispatch.is_some() {
            self.trace.current = dispatch.clone();
        }
        let seen = self.seen();
        let mut out = if matches!(event, Event::Saved(_)) {
            self.handle(event)
        } else {
            let before = self.config_table();
            let mut out = self.handle(event);
            if let Some(save) = self.autosave(&before) {
                out.push(save);
            }
            out
        };
        // Host-owned menus are not routed plugin effects, so capture their
        // creation here too. Plugin menu spans are installed by route().
        if self.menu.is_some() && self.trace.menu.is_none() {
            let id = self.trace.host("ui.menu", self.trace.current.clone());
            self.trace.menu = id.map(|action| MenuTrace {
                action,
                parent: self.trace.current.clone(),
                semantic: self.trace.semantic.clone(),
            });
        } else if self.menu.is_none() && self.trace.menu.is_some() {
            self.trace.menu = None;
        }
        // Host-originated effects (autosave, fallback links) also get spans.
        out = out
            .into_iter()
            .map(|effect| self.track_host_effect(effect))
            .collect();
        // This says the input dispatch completed, not that spawned work did.
        if self.menu.is_none() {
            self.trace.record(dispatch, JournalOutcome::Succeeded, None);
        }
        if let Some(action) = completed_task {
            let awaiting_menu = self.trace.menu.as_ref().is_some_and(|menu| {
                menu.semantic.is_none() && menu.parent.as_ref() == Some(&action)
            });
            if !self.trace.terminal(&action) && !awaiting_menu {
                self.trace.record(
                    Some(action),
                    JournalOutcome::Unknown,
                    Some(Reason::TaskCompleted),
                );
            }
        }
        self.cue_since(seen);
        self.trace.current = None;
        self.trace.semantic = None;
        self.trace.delivering_task = None;
        let mut records = self
            .trace
            .drain()
            .into_iter()
            .map(HostEffect::Journal)
            .collect::<Vec<_>>();
        records.extend(out);
        records
    }

    /// Enable pure action collection. Only the runner opens a journal.
    pub fn enable_journal(&mut self) {
        self.trace.seed = Some(ActionId::new());
        self.trace.enabled = true;
    }

    /// Isolated fixture namespace; each scenario owns its own journal.
    #[cfg(feature = "test-support")]
    pub(crate) fn enable_scenario_journal(&mut self) {
        self.trace.seed = Some(ActionId::parse("10000000-0000-4000-8000-000000000000").expect("fixture action namespace"));
        self.trace.enabled = true;
    }

    fn track_host_effect(&mut self, effect: HostEffect) -> HostEffect {
        let kind = match &effect {
            HostEffect::Spawn { .. } => "runtime.task",
            HostEffect::Open(_) => "system.open",
            HostEffect::OpenApp { .. } => "system.open.app",
            HostEffect::Copy(_) => "system.copy",
            HostEffect::CopySelection(_) => "system.copy.selection",
            HostEffect::Save(_) => "config.save",
            HostEffect::WriteCache { .. } => "cache.save",
            HostEffect::Tracked { .. }
            | HostEffect::Journal(_)
            | HostEffect::History(_)
            | HostEffect::Scan(_)
            | HostEffect::Work { .. } => {
                return effect;
            }
        };
        let mut descriptor = ActionDescriptor::new(kind, Origin::Host);
        match &effect {
            HostEffect::Open(url) | HostEffect::OpenApp { web: url, .. } => {
                Self::journal_url(&mut descriptor, url)
            }
            _ => {}
        }
        match self.trace.begin(descriptor, self.trace.current.clone()) {
            Some(action) => HostEffect::Tracked {
                action,
                semantic: self.trace.semantic.clone(),
                effect: Box::new(effect),
            },
            None => effect,
        }
    }

    fn complete_menu_trace(&mut self, cancelled: bool) {
        if let Some(menu) = self.trace.menu.take() {
            self.trace.current = menu.parent;
            self.trace.semantic = menu.semantic;
            if cancelled {
                self.trace
                    .record(Some(menu.action), JournalOutcome::Cancelled, None);
            } else {
                self.trace
                    .record(Some(menu.action.clone()), JournalOutcome::Confirmed, None);
                self.trace
                    .record(Some(menu.action), JournalOutcome::Succeeded, None);
            }
        }
    }

    fn cancel_menu_trace(&mut self, user_cancelled: bool) {
        if let Some(menu) = self.trace.menu.take() {
            self.trace
                .record(Some(menu.action), JournalOutcome::Cancelled, None);
            if user_cancelled {
                self.trace
                    .record(menu.semantic, JournalOutcome::Cancelled, None);
            }
        }
    }

    /// What motion should show since the last time this was called.
    pub fn take_cues(&mut self) -> Vec<Cue> {
        std::mem::take(&mut self.cues)
    }

    fn cue(&mut self, cue: Cue) {
        if self.cues.len() >= CUES_KEPT {
            self.cues.remove(0);
        }
        self.cues.push(cue);
    }

    fn seen(&self) -> Seen {
        Seen {
            view: self.view,
            depth: self
                .screen()
                .filter(|(p, _)| self.ready(*p))
                .map(|(p, s)| self.plugins[p].depth(s.id)),
            menu: self
                .menu
                .as_ref()
                .map(|m| (m.menu.id.clone(), m.menu.title.clone())),
            notice_id: self.notices.through_id(),
            notices_active: self.notices.has_active(),
            setup: self.setup.as_ref().map(|s| s.at),
        }
    }

    /// Cue what an event changed: the screen, a page opened, a menu or
    /// confirm shown, the message.
    fn cue_since(&mut self, before: Seen) {
        let now = self.seen();
        if now.view != before.view {
            self.cue(Cue::Screen);
        } else if now.setup.is_some() && before.setup.is_some() && now.setup != before.setup {
            // Another step of setup: a fade (subtle), the sweep over it (full).
            self.cue(Cue::Screen);
            self.cue(Cue::Page);
        } else if let (Some(was), Some(is)) = (before.depth, now.depth)
            && is > was
        {
            self.cue(Cue::Page);
        }
        if now.menu.is_some() && now.menu != before.menu {
            self.cue(Cue::Popup);
        }
        if now.notice_id != before.notice_id { self.cue(Cue::Message); }
        else if before.notices_active && !now.notices_active { self.cue(Cue::MessageGone); }
    }

    fn handle(&mut self, event: Event) -> Vec<HostEffect> {
        match event {
            Event::History { .. } => vec![],
            Event::Tracked { event, .. } => self.handle(*event),
            Event::Tick(now) => {
                self.now = now;
                self.frame = self.frame.wrapping_add(1);
                if let Some((words, until)) = &self.start_line
                    && now >= *until
                {
                    if self.message.as_ref().is_some_and(|m| m.text == *words) {
                        self.clear_message_compat();
                    }
                    self.start_line = None;
                }
                self.check_idle(now);
                let mut out = Vec::new();
                // Paused: no part asks its service.
                let ticking = if self.paused().is_some() {
                    0
                } else {
                    self.plugins.len()
                };
                for i in 0..ticking {
                    let fx = self.plugins[i].tick(now);
                    out.extend(self.route(i, fx, 0));
                }
                if ticking > 0 {
                    out.extend(self.pump_reviews());
                }
                self.pump_context();
                out
            }
            // Paused: the wakes wait, and the tick after the pause takes them in.
            Event::Wake(now) if self.paused().is_none() => {
                self.now = now;
                let mut out = Vec::new();
                for i in self.wake.take() {
                    if i < self.plugins.len() {
                        let fx = self.plugins[i].woken(now);
                        out.extend(self.route(i, fx, 0));
                    }
                }
                self.pump_context();
                out
            }
            Event::Wake(_) => vec![],
            Event::Msg { plugin, msg } => {
                let fx = self.plugins[plugin].on_msg(msg);
                self.route(plugin, fx, 0)
            }
            Event::Mouse {
                kind,
                column,
                row,
                at_ms,
            } => {
                let fx = self.on_mouse(kind, column, row, at_ms);
                self.someone_is_there();
                fx
            }
            Event::Intent(intent) => {
                self.someone_is_there();
                self.on_intent(intent)
            }
            Event::Scanned(found) => {
                self.setup_scanned(found);
                vec![]
            }
            Event::Paste(text) => {
                self.someone_is_there();
                if let Some(field) = self.menu.as_mut().and_then(|m| m.input.as_mut()) {
                    field.paste(&text);
                    self.typed_changed(true);
                    return vec![];
                }
                if self.menu.is_some() {
                    return vec![];
                }
                if let Some(input) = self.colours.edit.as_mut() {
                    input.paste(&text);
                    return vec![];
                }
                match self.target() {
                    Some(p) if self.plugins[p].capturing() => {
                        let fx = self.plugins[p].paste(&text);
                        self.route(p, fx, 0)
                    }
                    _ => vec![],
                }
            }
            Event::Notice(text) | Event::ToClipboard { note: text, .. } => {
                if !text.is_empty() {
                    self.notify(text.into());
                }
                vec![]
            }
            Event::ErrorNotice(text) => { self.notify_error(text); vec![] }
            Event::TerminalRoute(result) => {
                if self.terminal.is_some() {
                    let route = match result {
                        Ok(()) => crate::terminal_host::Route::Ready,
                        Err(error) => {
                            self.notify_error(format!("{error}; use x for actions"));
                            crate::terminal_host::Route::Failed(error)
                        }
                    };
                    self.terminal.as_mut().expect("checked").route = route;
                }
                vec![]
            }
            Event::Saved(result) => {
                let (table, asked) = self
                    .saving
                    .pop_front()
                    .unwrap_or_else(|| (self.config_table(), true));
                match result {
                    Ok(()) => {
                        self.saved = table;
                        self.save_failed = None;
                        // "save and quit" quits only once the file is written.
                        if self.saving.is_empty() && std::mem::take(&mut self.quit_after_save) {
                            self.quit = true;
                        }
                        // A save after a change says nothing: the part's
                        // own word about the change stays on the line.
                        if asked {
                            self.notify("saved".into());
                        }
                    }
                    Err(e) => {
                        self.quit_after_save = false;
                        self.notify_error(format!("✗ settings not saved: {e} · w tries again"));
                        self.save_failed = Some(e);
                    }
                }
                vec![]
            }
            Event::Key(key) => {
                let fx = self.on_key(key);
                self.someone_is_there();
                fx
            }
        }
    }

    /// Tell every plugin the user did something, after it was handled.
    fn someone_is_there(&mut self) {
        self.input_seen(self.now);
        for p in &mut self.plugins {
            p.input(self.now);
        }
    }

    /// Turn a plugin's effects into host effects, delivering bus events to
    /// every other plugin on the way.
    pub(crate) fn route(
        &mut self,
        from: usize,
        effects: Vec<Effect>,
        depth: usize,
    ) -> Vec<HostEffect> {
        let mut out = Vec::new();
        let parent = self.trace.current.clone();
        let parent_semantic = self.trace.semantic.clone();
        for effect in effects {
            match effect {
                Effect::Work { id, request } => {
                    out.push(HostEffect::Work {
                        plugin: from,
                        id,
                        request,
                        mode: self.mode(),
                    });
                    continue;
                }
                Effect::ResolveSources { id, subjects } => {
                    if depth >= BUS_DEPTH {
                        continue;
                    }
                    let mut sources = Vec::new();
                    let mut omitted = subjects
                        .len()
                        .saturating_sub(crate::source_context::MAX_SOURCES);
                    for subject in subjects
                        .into_iter()
                        .take(crate::source_context::MAX_SOURCES)
                    {
                        if let Some(snapshot) =
                            self.journal_snapshot(&subject, crate::journal::SubjectRole::Context)
                        {
                            if !sources
                                .iter()
                                .any(|s: &crate::source_context::ResolvedSource| {
                                    s.key == snapshot.key
                                })
                            {
                                sources.push(crate::source_context::ResolvedSource {
                                    subject,
                                    key: snapshot.key,
                                });
                            }
                        } else {
                            omitted += 1;
                        }
                    }
                    let fx = self.plugins[from].on_msg(Box::new(
                        crate::source_context::ResolvedSources {
                            id,
                            action_id: self.trace.current.clone(),
                            sources,
                            omitted,
                        },
                    ));
                    out.extend(self.route(from, fx, depth + 1));
                    continue;
                }
                Effect::ShowSource(key) => {
                    if depth >= BUS_DEPTH {
                        continue;
                    }
                    // Only the ready owner of this exact kind may reconstruct
                    // it, and the normal owner-validated identity must roundtrip.
                    let subject = compose::owner_of(&self.plugins, &key.kind)
                        .and_then(|(owner, _)| {
                            let subject = self.plugins[owner].subject_from_journal(&key)?;
                            if subject.kind != key.kind {
                                return None;
                            }
                            let check = self.journal_snapshot(
                                &subject,
                                crate::journal::SubjectRole::Context,
                            )?;
                            (check.key == key).then_some(subject)
                        });
                    if let Some(subject) = subject {
                        out.extend(self.route(from, vec![Effect::Show(subject)], depth + 1));
                    } else {
                        self.notify_error("This source is unavailable here. Its exact locator is still available to copy.");
                    }
                    continue;
                }
                Effect::Journal(descriptor) => {
                    self.trace.current = self.trace.begin(descriptor, parent.clone());
                    self.trace.semantic = self.trace.current.clone();
                    continue;
                }
                Effect::SpawnAction { action, task } => {
                    let previous = self.trace.current.clone();
                    let previous_semantic = self.trace.semantic.clone();
                    self.trace.current = self.trace.begin(action, previous.clone());
                    self.trace.semantic = self.trace.current.clone();
                    out.extend(self.route(from, vec![Effect::Spawn(task)], depth));
                    self.trace.current = previous;
                    self.trace.semantic = previous_semantic;
                    continue;
                }
                Effect::Observe { action, outcome } => {
                    let observation = self.trace.begin(action, self.trace.current.clone());
                    self.trace.record(observation, outcome, None);
                    continue;
                }
                Effect::Report(outcome) => {
                    let semantic = self
                        .trace
                        .semantic
                        .clone()
                        .or_else(|| self.trace.current.clone());
                    self.trace.record(semantic.clone(), outcome, None);
                    // A terminal domain report is an explicit observation for
                    // the exact delivering task too, never an inferred ancestor.
                    if outcome.is_terminal()
                        && self.trace.current != semantic
                        && self.trace.current == self.trace.delivering_task
                    {
                        self.trace.record(self.trace.current.clone(), outcome, None);
                    }
                    if outcome.is_terminal() {
                        self.trace.semantic = None;
                    }
                    continue;
                }
                _ => {}
            }
            let kind = match &effect {
                Effect::Spawn(_) => "runtime.task",
                Effect::Open(_) => "system.open",
                Effect::StartReview(_) => "agent.review.request",
                Effect::SendToAgent(_) => "agent.send.request",
                Effect::ShowDock => "ui.dock.show",
                Effect::HideDock => "ui.dock.hide",
                Effect::OpenApp { .. } => "system.open.app",
                Effect::Copy(_) => "system.copy",
                Effect::Notify(_) | Effect::NotifyWith(..) | Effect::Notice(_) => "ui.notice",
                Effect::Focus(_) => "ui.focus",
                Effect::PersistCache => "cache.save",
                Effect::Emit { .. } => "bus.emit",
                Effect::Menu(_) => "ui.menu",
                Effect::Show(_) => "ui.show",
                Effect::Changed(_) => "ui.changed",
                Effect::Press(_) => "ui.press",
                Effect::Act { .. } => "plugin.action.dispatch",
                Effect::Work { .. }
                | Effect::ResolveSources { .. }
                | Effect::ShowSource(_)
                | Effect::Journal(_)
                | Effect::Report(_)
                | Effect::SpawnAction { .. }
                | Effect::Observe { .. } => unreachable!(),
            };
            let previous = self.trace.current.clone();
            let previous_semantic = self.trace.semantic.clone();
            let mut descriptor =
                ActionDescriptor::new(kind, Origin::Plugin(self.plugins[from].id().into()));
            match &effect {
                Effect::Show(subject) | Effect::Act { subject, .. } => {
                    descriptor = self.journal_subject(descriptor, subject)
                }
                Effect::Open(url) | Effect::OpenApp { web: url, .. } => {
                    Self::journal_url(&mut descriptor, url)
                }
                _ => {}
            }
            if let Effect::Emit { topic, .. } = &effect {
                descriptor.metadata.command = Some(crate::journal::StableCode::new(topic));
            }
            let action = self.trace.begin(descriptor, previous.clone());
            self.trace.current = action.clone().or(previous.clone());
            let mut pending = false;
            let mut blocked = false;
            let mut external = None;
            match effect {
                Effect::Spawn(task) => external = Some(HostEffect::Spawn { plugin: from, task }),
                Effect::Open(u) => external = Some(HostEffect::Open(u)),
                Effect::StartReview(request) => {
                    let fx = self.start_reviews(request, depth);
                    out.extend(fx);
                }
                Effect::ShowDock => {
                    self.apply_dock(&crate::dock::DockIntent::Show);
                    self.apply_dock(&crate::dock::DockIntent::Focus);
                }
                Effect::HideDock => {
                    self.apply_dock(&crate::dock::DockIntent::Hide);
                }
                Effect::SendToAgent(request) => {
                    // Ask the parts (the one that owns the subject answers),
                    // then hand the contexts to the part that owns the dock.
                    let contexts: Vec<_> = self.plugins.iter_mut().filter_map(|p| p.agent_context(&request)).collect();
                    match (contexts.is_empty(), self.dock_owner()) {
                        (true, _) => self.notify_error("nothing to send for this"),
                        (false, None) => self.notify_error("no agent dock here"),
                        (false, Some(d)) => {
                            if let Outcome::Handled(fx) = self.plugins[d].agent_brief(contexts) {
                                out.extend(self.route(d, fx, depth + 1));
                            }
                            self.apply_dock(&crate::dock::DockIntent::Show);
                        }
                    }
                }
                Effect::OpenApp { link, web } => external = Some(HostEffect::OpenApp { link, web }),
                Effect::Copy(t) => external = Some(HostEffect::Copy(t)),
                Effect::Notice(message) => self.notify(message),
                Effect::Notify(m) => self.notify(m.into()),
                Effect::NotifyWith(icon, m) => {
                    self.notify(crate::plugin::Message::new(Some(icon), m))
                }
                Effect::Focus(p) => self.focus = p,
                Effect::PersistCache => {
                    if let Some(value) = self.plugins[from].snapshot() {
                        external = Some(HostEffect::WriteCache {
                            plugin: self.plugins[from].id(),
                            value,
                        });
                    }
                }
                Effect::Emit { topic, payload } if depth < BUS_DEPTH => {
                    for j in 0..self.plugins.len() {
                        if j != from {
                            let delivery = self.trace.begin(
                                ActionDescriptor::new(
                                    "bus.deliver",
                                    Origin::Plugin(self.plugins[j].id().into()),
                                ),
                                action.clone(),
                            );
                            self.trace.current = delivery.clone();
                            // Fanout is causal context, not permission for
                            // independent receivers to finish the same intent.
                            self.trace.semantic = None;
                            let fx = self.plugins[j].on_bus(topic, &payload);
                            out.extend(self.route(j, fx, depth + 1));
                            self.trace.record(delivery, JournalOutcome::Succeeded, None);
                        }
                    }
                }
                Effect::Emit { .. } => blocked = true,
                Effect::Menu(menu) => {
                    self.cancel_menu_trace(false);
                    let menu = match menu.change {
                        Some(c) => menu.noting(crate::writes::confirm_note(c)),
                        None => menu,
                    };
                    self.menu = Some(OpenMenu::open(Owner::Plugin(from), menu));
                    self.typed_changed(true);
                    self.trace.menu = action.clone().map(|action| MenuTrace {
                        action,
                        parent: previous.clone(),
                        semantic: self.trace.semantic.clone(),
                    });
                    pending = true;
                }
                Effect::Show(subject) if depth < BUS_DEPTH => {
                    out.extend(
                        self.show(subject, depth + 1)
                            .into_iter()
                            .map(|effect| self.track_host_effect(effect))
                            .collect::<Vec<_>>(),
                    );
                }
                Effect::Show(_) => blocked = true,
                Effect::Changed(changes) => {
                    if !changes.is_empty() {
                        self.cue(Cue::Changed(changes));
                    }
                }
                Effect::Press(c) if depth < BUS_DEPTH => {
                    out.extend(self.on_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
                }
                Effect::Press(_) => blocked = true,
                Effect::Act { part, subject, id } if depth < BUS_DEPTH => {
                    let at = (0..self.plugins.len())
                        .find(|&i| self.plugins[i].name() == part && self.ready(i));
                    if let Some(i) = at {
                        out.extend(self.contributed_act(i, &subject, &id, depth + 1));
                    } else {
                        blocked = true;
                    }
                }
                Effect::Act { .. } => blocked = true,
                Effect::Work { .. }
                | Effect::ResolveSources { .. }
                | Effect::ShowSource(_)
                | Effect::Journal(_)
                | Effect::Report(_)
                | Effect::SpawnAction { .. }
                | Effect::Observe { .. } => unreachable!(),
            }
            if let Some(effect) = external {
                out.push(match action.clone() {
                    Some(action) => HostEffect::Tracked {
                        action,
                        semantic: self.trace.semantic.clone(),
                        effect: Box::new(effect),
                    },
                    None => effect,
                });
                pending = true;
            }
            if !pending {
                self.trace.record(
                    action,
                    if blocked {
                        JournalOutcome::Blocked
                    } else {
                        JournalOutcome::Succeeded
                    },
                    blocked.then_some(Reason::Unsupported),
                );
            }
            self.trace.current = previous;
            self.trace.semantic = previous_semantic;
        }
        self.trace.current = parent;
        self.trace.semantic = parent_semantic;
        out
    }

    /// Show a subject: to the set-up part that owns its kind, else its link.
    /// A link one of the parts recognises is shown as what it is.
    fn show(&mut self, subject: Subject, depth: usize) -> Vec<HostEffect> {
        let subject = if subject.is(compose::LINK) {
            let link = subject.id.clone();
            compose::recognise(&self.plugins, &link)
                .into_iter()
                .find(|(r, _)| r.start == 0 && r.end == link.len())
                .map_or(subject, |(_, s)| s)
        } else {
            subject
        };
        let Some((owner, _)) = compose::owner_of(&self.plugins, &subject.kind) else {
            return self.show_link(&subject);
        };
        self.show_owned(owner, subject, depth, true)
    }

    fn show_owned(&mut self, owner: usize, subject: Subject, depth: usize, fallback_link: bool) -> Vec<HostEffect> {
        let (view, focus) = (self.view, self.focus);
        match self.plugins[owner].show(&subject) {
            Showing::Page { screen, effects } => {
                let to = self
                    .screens
                    .iter()
                    .position(|(p, s)| *p == owner && s.id == screen);
                if let Some(to) = to {
                    if View::Screen(to) != view {
                        self.back.push(Return {
                            view,
                            focus,
                            to,
                            depth: self.plugins[owner].depth(screen),
                        });
                        self.view = View::Screen(to);
                    }
                    self.focus = Pane::Middle;
                }
                let current = self.plugins[owner].subject(screen, Pane::Middle).unwrap_or(subject);
                self.visit_palette_subject(owner, &current);
                self.route(owner, effects, depth)
            }
            Showing::Done(effects) => {
                self.visit_palette_subject(owner, &subject);
                self.route(owner, effects, depth)
            }
            Showing::Cannot if fallback_link => self.show_link(&subject),
            Showing::Cannot => {
                self.notify_error("that destination is no longer available");
                vec![]
            },
        }
    }

    /// No part here opens it: open its link, the radar way, and say so.
    fn show_link(&mut self, subject: &Subject) -> Vec<HostEffect> {
        match &subject.url {
            Some(url) => {
                self.notify(format!("opening {}", subject.label).into());
                vec![HostEffect::Open(url.clone())]
            }
            None => {
                self.notify_error(format!("cannot open {} here", subject.label));
                vec![]
            }
        }
    }

    /// After `esc` on a screen a show went to: once it backed out past the
    /// shown page (or had nothing to back out of), return to where the show
    /// came from.
    fn come_back(&mut self, ignored: bool) -> bool {
        let Some(r) = self.back.last().copied() else {
            return false;
        };
        let Some((p, spec)) = self.screen() else {
            return false;
        };
        if self.view != View::Screen(r.to) {
            return false;
        }
        if ignored || self.plugins[p].depth(spec.id) < r.depth {
            self.back.pop();
            self.view = r.view;
            self.focus = r.focus;
            self.tabs = false;
            return true;
        }
        false
    }

    /// Where `esc` would return to now, if it would leave this screen.
    fn returns_to(&self) -> Option<String> {
        let r = self.back.last()?;
        let (p, spec) = self.screen()?;
        if self.view != View::Screen(r.to) || self.plugins[p].depth(spec.id) > r.depth {
            return None;
        }
        Some(match r.view {
            View::Screen(i) => self
                .screens
                .get(i)
                .map_or("where you were", |(_, s)| s.name)
                .to_string(),
            View::Settings => "settings".into(),
        })
    }

    /// What the focused pane is about, if its part says.
    fn subject_here(&self) -> Option<(usize, Subject)> {
        let (p, spec) = self.screen()?;
        if !self.ready(p) {
            return None;
        }
        self.plugins[p].subject(spec.id, self.focus).map(|s| (p, s))
    }

    /// The actions other parts add to `subject`'s menu, owned by `owner`,
    /// in plugin order, each part's under its name. A key the owner's rows
    /// (`taken`), the host or an earlier part already use is dropped from
    /// the row, which is then chosen from the menu only.
    fn contributed_actions(
        &self,
        owner: usize,
        subject: &Subject,
        mut taken: Vec<char>,
    ) -> Vec<(usize, &'static str, Vec<compose::Action>)> {
        let mut out = Vec::new();
        for (i, p) in self.plugins.iter().enumerate() {
            if i == owner || !self.ready(i) {
                continue;
            }
            let Some(Contribution::Actions(mut actions)) = p.contribute(Point::Actions, subject)
            else {
                continue;
            };
            for a in &mut actions {
                if let Some(c) = a.key {
                    if taken.contains(&c) || host_key(c) {
                        a.key = None;
                    } else {
                        taken.push(c);
                    }
                }
            }
            if !actions.is_empty() {
                out.push((i, p.name(), actions));
            }
        }
        out
    }

    /// Run a contributed action with what the other parts know now.
    fn contributed_act(
        &mut self,
        p: usize,
        subject: &Subject,
        id: &str,
        depth: usize,
    ) -> Vec<HostEffect> {
        let prepared = compose::prepare_action(&self.plugins, p, subject);
        let descriptor = self.journal_action_context(
            ActionDescriptor::new("plugin.action", Origin::Plugin(self.plugins[p].id().into())),
            prepared.subject(),
        );
        let parent = self.trace.current.clone();
        let action = self.trace.begin(descriptor, parent.clone());
        self.trace.current = action.clone().or(parent.clone());
        let span = (action, parent);
        let fx = prepared.dispatch(&mut self.plugins, id);
        let out = self.route(p, fx, depth);
        self.finish_dispatch(span, true);
        out
    }

    /// A key the focused pane's part did not use: a contributed action's?
    fn contributed_key(&mut self, c: char) -> Option<Vec<HostEffect>> {
        let (owner, subject) = self.subject_here()?;
        let (_, spec) = self.screen()?;
        let taken = own_keys(&self.screen_keys(owner, spec.id, self.focus));
        let (p, id) = self
            .contributed_actions(owner, &subject, taken)
            .into_iter()
            .find_map(|(p, _, actions)| {
                actions
                    .into_iter()
                    .find(|a| a.key == Some(c))
                    .map(|a| (p, a.id))
            })?;
        Some(self.contributed_act(p, &subject, &id, 0))
    }

    fn journal_snapshot(
        &self,
        subject: &Subject,
        role: crate::journal::SubjectRole,
    ) -> Option<crate::journal::SubjectSnapshot> {
        let (owner, _) = compose::owner_of(&self.plugins, &subject.kind)?;
        let mut snapshot = self.plugins[owner].journal_subject(subject)?;
        if snapshot.key.kind != subject.kind || snapshot.validate().is_err() {
            return None;
        }
        snapshot.role = role;
        Some(snapshot)
    }

    fn journal_subject(
        &self,
        mut descriptor: ActionDescriptor,
        subject: &Subject,
    ) -> ActionDescriptor {
        self.journal_candidate(
            &mut descriptor,
            subject,
            crate::journal::SubjectRole::Primary,
        );
        descriptor
    }

    /// Capture only the references in the prepared input, never their children.
    /// These roles describe supplied context, not use, existence or causality.
    fn journal_action_context(
        &self,
        descriptor: ActionDescriptor,
        subject: &Subject,
    ) -> ActionDescriptor {
        let mut descriptor = self.journal_subject(descriptor, subject);
        for related in &subject.related {
            self.journal_candidate(
                &mut descriptor,
                related,
                crate::journal::SubjectRole::Context,
            );
        }
        descriptor
    }

    fn journal_candidate(
        &self,
        descriptor: &mut ActionDescriptor,
        subject: &Subject,
        role: crate::journal::SubjectRole,
    ) {
        if let Some(snapshot) = self.journal_snapshot(subject, role) {
            // Deduplication here is scoped identity, independent of the input's
            // kind/id matching. Primary wins; the delivered input is untouched.
            if descriptor.subjects.iter().any(|s| s.key == snapshot.key) {
                return;
            }
            if descriptor.subjects.len() < crate::journal::MAX_SUBJECTS {
                descriptor.subjects.push(snapshot);
                return;
            }
            descriptor.metadata.truncated = true;
        } else {
            descriptor.metadata.redacted = true;
        }
        // Count omitted candidate occurrences, not distinct missing identities.
        // The retained-key scan stays bounded even when the input is large.
        let count = &mut descriptor.metadata.omitted_subjects;
        if *count == u32::MAX {
            descriptor.metadata.truncated = true;
        }
        *count = count.saturating_add(1);
    }

    fn journal_url(descriptor: &mut ActionDescriptor, url: &str) {
        // Arbitrary open targets can carry reset/invite credentials in the
        // path itself. Only owning plugins may approve subject URL paths.
        descriptor.metadata.url = url::Url::parse(url).ok().and_then(|parsed| {
            crate::journal::SafeUrl::parse(&parsed.origin().ascii_serialization()).ok()
        });
        descriptor.metadata.redacted |= descriptor
            .metadata
            .url
            .as_ref()
            .is_none_or(|safe| safe.as_str() != url);
    }

    fn plugin_dispatch(
        &mut self,
        plugin: usize,
        kind: &'static str,
    ) -> (Option<ActionId>, Option<ActionId>) {
        let parent = self.trace.current.clone();
        let action = self.trace.begin(
            ActionDescriptor::new(kind, Origin::Plugin(self.plugins[plugin].id().into())),
            parent.clone(),
        );
        self.trace.current = action.clone().or(parent.clone());
        (action, parent)
    }

    fn finish_dispatch(
        &mut self,
        (action, parent): (Option<ActionId>, Option<ActionId>),
        handled: bool,
    ) {
        let awaiting_menu = self
            .trace
            .menu
            .as_ref()
            .is_some_and(|menu| menu.parent == action);
        if !awaiting_menu {
            self.trace.record(
                action,
                if handled {
                    JournalOutcome::Succeeded
                } else {
                    JournalOutcome::Unknown
                },
                (!handled).then_some(Reason::Unsupported),
            );
        }
        self.trace.current = parent;
    }

    pub(crate) fn deliver(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        self.deliver_input(Input::Key(key))
    }

    /// An intent: a key's goes where the key would (the host's own keys
    /// included); a menu's choice answers the open menu; the rest go to the
    /// focused part.
    fn pump_context(&mut self) {
        for i in 0..self.plugins.len() {
            for call in self.plugins[i].context_calls() {
                let answer = self.context_service.execute(&mut self.plugins, &call.tool, call.arguments, self.now);
                self.plugins[i].context_answer(call.id, answer);
            }
        }
    }

    /// A part asked for a local review (`Effect::StartReview`): each link
    /// is mapped to the change request the part that recognises it owns,
    /// and the dock owner starts one review per change request. The row it
    /// was asked on gives the brief its origin. Nothing is posted.
    fn start_reviews(&mut self, request: crate::agent_context::ReviewRequest, depth: usize) -> Vec<HostEffect> {
        let Some(d) = self.dock_owner() else {
            self.notify_error("no agent dock here");
            return vec![];
        };
        let mut changes = request.changes.clone();
        for link in &request.links {
            for p in &self.plugins {
                for (_, subject) in p.recognise(link) {
                    if let Some(snapshot) = self.plugins.iter().find_map(|q| q.journal_subject(&subject))
                        && !changes.contains(&snapshot.key)
                    {
                        changes.push(snapshot.key);
                    }
                }
            }
        }
        if changes.is_empty() {
            self.notify_error("no change request to review here");
            return vec![];
        }
        let ask = crate::agent_context::AgentContextRequest { subject: request.origin };
        let origin = self.plugins.iter_mut().find_map(|p| p.agent_context(&ask));
        let mut out = Vec::new();
        for change in changes {
            // A change request asked for on its own row is its own origin:
            // the brief quotes it once, as the change.
            let origin = origin.clone().filter(|o| o.subject != change);
            let outcome = if request.automatic { self.plugins[d].review_enqueue(origin, change) } else { self.plugins[d].review_start(origin, change) };
            if let Outcome::Handled(fx) = outcome {
                out.extend(self.route(d, fx, depth + 1));
            }
        }
        if !request.automatic { self.apply_dock(&crate::dock::DockIntent::Show); }
        out
    }

    /// Answer what running reviews wait for: the part that owns the change
    /// request, the part that owns repositories, the part that keeps notes.
    /// One round a tick: a read that is still under way is asked again next
    /// tick, and the journey's own clock ends one that never answers.
    fn pump_reviews(&mut self) -> Vec<HostEffect> {
        use crate::plugin::{ReviewAnswer, ReviewAsk};
        let Some(d) = self.dock_owner() else { return vec![] };
        let asks = self.plugins[d].review_asks();
        let mut out = Vec::new();
        for (run, ask) in asks {
            let answer = match ask {
                ReviewAsk::Resolve(request) => {
                    let mut answer = None;
                    for i in 0..self.plugins.len() {
                        if let Some((resolve, fx)) = self.plugins[i].resolve_change(&request) {
                            out.extend(self.route(i, fx, 1));
                            answer = Some(resolve);
                            break;
                        }
                    }
                    if matches!(answer, Some(crate::agent_context::Resolve::Ready(_))) {
                        let args = serde_json::json!({"roots":[crate::context::reference(&request.subject)],"depth":2});
                        if let Ok(packet) = self.context_service.execute(&mut self.plugins, "context", args, self.now)
                            && let Some(id) = packet["snapshot_id"].as_str()
                        {
                            let mut bundle = self.context_service.brief(id);
                            let mut pending = false;
                            for context in &bundle.contexts {
                                for i in 0..self.plugins.len() {
                                    let (loading, fx) = self.plugins[i].prepare_agent_context(&context.subject);
                                    pending |= loading;
                                    out.extend(self.route(i, fx, 1));
                                }
                            }
                            let started = *self.review_context_started.entry(run).or_insert(self.now);
                            if pending && self.now - started < 10 { answer = Some(crate::agent_context::Resolve::Pending); }
                            else {
                                self.review_context_started.remove(&run);
                                if pending { bundle.missing.push("some linked sources are still loading; use context again to read their results".into()); }
                                self.plugins[d].review_context(run, bundle);
                            }
                        }
                    }
                    ReviewAnswer::Resolved(answer.unwrap_or(crate::agent_context::Resolve::Failed(
                        "no part here can read this change request".into(),
                    )))
                }
                ReviewAsk::Worktree(request) => {
                    let mut answer = None;
                    for i in 0..self.plugins.len() {
                        if let Some((result, fx)) = self.plugins[i].ensure_worktree(&request) {
                            out.extend(self.route(i, fx, 1));
                            answer = Some(result);
                            break;
                        }
                    }
                    ReviewAnswer::Worktree(answer.unwrap_or(Err(crate::agent_context::WorktreeError::Failed(
                        "no part here makes worktrees".into(),
                    ))))
                }
                ReviewAsk::SaveNote(request) => {
                    let saved = self.plugins.iter_mut().find_map(|p| p.save_note(&request));
                    ReviewAnswer::NoteSaved(saved.and_then(Result::ok))
                }
            };
            if let Outcome::Handled(fx) = self.plugins[d].review_answer(run, answer) {
                out.extend(self.route(d, fx, 1));
            }
        }
        out
    }

    /// The part that supplies the agent dock, if any is loaded.
    pub fn dock_owner(&self) -> Option<usize> {
        self.plugins.iter().position(|p| p.dock().is_some())
    }

    /// The dock for the screens: the owner's content with the host's state
    /// (shown, focus, height) laid over it.
    /// Where the parts' wakes gather; a runner pumps them into [`Event::Wake`].
    pub fn wake_hub(&self) -> crate::wake::WakeHub {
        self.wake.clone()
    }

    pub fn dock_view(&self) -> Option<Box<crate::dock::DockView>> {
        self.plugins
            .iter()
            .find_map(|p| p.dock())
            .map(|content| Box::new(self.dock.over(content)))
    }

    /// Whether the dock is drawn where the user is now: beside a settings
    /// section, or under the middle pane of a screen of panes (a part's
    /// set-up page is one). An item page and the work area have no place
    /// for it ([`crate::view::ScreenView::has_dock_place`]).
    pub fn dock_has_place(&self) -> bool {
        if self.view == View::Settings {
            return true;
        }
        let Some((p, spec)) = self.screen() else {
            return false;
        };
        self.setting_up().is_some() || self.screen_view(p, spec.id).is_some_and(|v| v.has_dock_place())
    }

    /// Apply a host-owned dock intent (`false` for one the part takes). The
    /// dock takes the keys only where it is drawn: shown where it has no
    /// place, it waits for the screen of panes without them, and the keys
    /// stay with the page.
    fn apply_dock(&mut self, intent: &crate::dock::DockIntent) -> bool {
        let hosts = self.dock.apply(intent);
        if hosts && self.dock.focused && !self.dock_has_place() {
            self.dock.focused = false;
        }
        hosts
    }

    /// A dock intent: show, hide, focus and height are the host's; the rest
    /// go to the owner under a dispatch span, so the journal reads the same
    /// whichever frontend sent them and effects keep mode checks and
    /// causality.
    fn on_dock_intent(&mut self, intent: crate::dock::DockIntent) -> Vec<HostEffect> {
        if self.apply_dock(&intent) {
            return vec![];
        }
        let Some(p) = self.dock_owner() else {
            return vec![];
        };
        let span = self.plugin_dispatch(p, "plugin.dock");
        let outcome = self.plugins[p].dock_intent(&intent);
        let out = match outcome {
            Outcome::Handled(fx) => Some(self.route(p, fx, 0)),
            Outcome::Ignored => None,
        };
        self.finish_dispatch(span, out.is_some());
        out.unwrap_or_default()
    }

    fn on_intent(&mut self, intent: crate::view::Intent) -> Vec<HostEffect> {
        use crate::view::Intent;
        if let Intent::Select { target } | Intent::Activate { target } | Intent::Context { target }
            | Intent::Fold { target, .. } | Intent::Show { target, .. } = &intent
            && target.as_str().starts_with(crate::view::detail::TARGET_PREFIX)
        {
            return if matches!(intent, Intent::Activate { .. }) {
                self.activate_detail_control(target)
            } else { vec![] };
        }
        if let Intent::Key { chord } = intent {
            return self.on_key(crate::view_tui::key_of(chord));
        }
        if let Intent::Dock(dock) = intent {
            return self.on_dock_intent(dock);
        }
        if let Intent::Copy { text } = intent {
            return self.copy_selection(text);
        }
        if let Intent::Invoke { action } = &intent && action.starts_with("palette:") {
            let Some(row) = self.palette_row(action) else { return vec![]; };
            self.menu.as_mut().expect("active palette").row = row;
            return self.palette_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        // The chrome's own targets (a tab, a menu row, a key on the status
        // line, a settings row): what a click on them does.
        if let Intent::Activate { target } | Intent::Select { target } = &intent
            && target.as_str().starts_with(crate::view::chrome::targets::PREFIX)
        {
            let activate = matches!(intent, Intent::Activate { .. });
            return self.chrome_intent(&target.clone(), activate);
        }
        // A row of one of setup's own steps.
        if self.setup.is_some()
            && let Intent::Activate { target } | Intent::Select { target } = &intent
            && target.as_str().starts_with(crate::setup::PREFIX)
        {
            if self.menu.is_some() || self.capturing() {
                return vec![];
            }
            let activate = matches!(intent, Intent::Activate { .. });
            return self.setup_target(&target.clone(), activate);
        }
        // A row of Settings' look section (the same card as setup's look).
        if self.setup.is_none()
            && self.view == View::Settings
            && self.current_section().is_some_and(|(o, _)| o == Owner::Look)
            && let Intent::Activate { target } | Intent::Select { target } = &intent
            && target.as_str().starts_with(crate::setup::PREFIX)
        {
            if self.menu.is_some() {
                return vec![];
            }
            self.focus = Pane::Middle;
            self.look_settings_target(&target.clone(), matches!(intent, Intent::Activate { .. }));
            return vec![];
        }
        if self.palette.is_some()
            && let Intent::Invoke { action } = &intent
            && matches!(action.as_str(), "chrome.palette.scroll_up" | "chrome.palette.scroll_down")
        {
            if let Some(open) = &mut self.menu
                && open.menu.id == palette::ID
                && open.viewport.more()
            {
                open.viewport.scroll(action == "chrome.palette.scroll_down", false);
            }
            return vec![];
        }
        if let Some(open) = &mut self.menu {
            let Intent::Invoke { action } = &intent else {
                return vec![];
            };
            let Some(i) = open.menu.items.iter().position(|it| {
                !it.heading && !it.note && it.value == *action
            }) else {
                return vec![];
            };
            open.row = i;
            return self.menu_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        }
        // `pane:<id>` from a renderer that lays the panes out itself (Ink): a
        // press on a pane's empty space, or the wheel over it, focuses that
        // pane and nothing else, as `Hit::Pane` does in `on_mouse`.
        if let Intent::Select { target } = &intent
            && let Some(pane) = pane_of_own_target(target.as_str())
        {
            if !self.capturing() && self.panes().contains(&pane) {
                self.focus = pane;
                self.bar = false;
                self.sources = false;
                self.tabs = false;
            }
            return vec![];
        }
        // While a field has the keys (an edit, a filter, a reply), a press
        // on one of the part's rows waits for it (⏎ or esc), as a click
        // does in the terminal (`on_mouse`): moving the selection under an
        // open edit would commit the edit into the row pressed. The field's
        // own intents (keys, actions, its text selection) still reach it.
        if self.capturing()
            && matches!(
                intent,
                Intent::Select { .. }
                    | Intent::Activate { .. }
                    | Intent::Context { .. }
                    | Intent::Fold { .. }
                    | Intent::OpenAll
                    | Intent::Show { .. }
            )
        {
            return vec![];
        }
        if let Intent::Fold { target, open } = &intent
            && target.as_str().starts_with(crate::view::chrome::targets::PREFIX)
        {
            return self.settings_nav_fold(target, *open);
        }
        if matches!(intent, Intent::OpenAll) {
            return self.open_all_intent();
        }
        self.clear_message_compat();
        let context = matches!(intent, Intent::Context { .. });
        let invoked = match &intent {
            Intent::Invoke { action } => Some(action.clone()),
            _ => None,
        };
        let mut out = self.deliver_input(Input::Intent(intent));
        // An action invoked by its id that no part took is the key it lists
        // here, pressed: the host's own keys (⏎ on a settings navigation
        // row goes into the section) are no part's.
        if out.is_none()
            && let Some(chord) = invoked.and_then(|action| self.invoked_key(&action))
        {
            out = Some(self.on_key(crate::view_tui::key_of(chord)));
        }
        // A context intent that the part took opens the menu of the keys
        // for what it selected, as a right-click does in the terminal.
        if context && out.is_some() {
            self.bar = false;
            self.tabs = false;
            self.open_keys_menu();
        }
        out.unwrap_or_default()
    }

    /// Selected screen text a renderer sent ([`crate::view::Intent::Copy`]):
    /// the runner copies it through [`crate::system::copy_ways`] and says how.
    /// Text over [`crate::ipc::MAX_CLIPBOARD`] bytes is refused, blank text
    /// is nothing to copy.
    fn copy_selection(&mut self, text: String) -> Vec<HostEffect> {
        if text.trim().is_empty() {
            return vec![];
        }
        if text.len() > crate::ipc::MAX_CLIPBOARD {
            self.notify_error("too much text to copy");
            return vec![];
        }
        vec![HostEffect::CopySelection(text)]
    }

    /// Route a key or an intent from any frontend to the focused part, under
    /// the same dispatch span, so the journal reads the same whichever
    /// frontend sent it.
    fn deliver_input(&mut self, input: Input) -> Option<Vec<HostEffect>> {
        let p = self.target()?;
        // A press on a section's row from a renderer that lays the section
        // out itself (Ink) focuses the section first, as a click on it does
        // in the terminal (`on_mouse`, `Hit::Plugin`). The nav's rows are
        // chrome targets and never come here.
        if self.view == View::Settings
            && self.focus != Pane::Middle
            && matches!(
                &input,
                Input::Intent(
                    crate::view::Intent::Select { .. }
                        | crate::view::Intent::Activate { .. }
                        | crate::view::Intent::Context { .. }
                )
            )
        {
            self.focus = Pane::Middle;
        }
        let (scope, section) = match (self.view, self.focus) {
            (View::Screen(_), _) => (self.screen()?.1.id, false),
            (View::Settings, Pane::Middle) => (self.current_section()?.1.id, true),
            (View::Settings, _) => return None,
        };
        let commit = match &input {
            Input::Key(key) => matches!(key.code, KeyCode::Enter | KeyCode::Esc),
            Input::Intent(i) => matches!(
                i.chord().map(|c| c.key),
                Some(crate::view::Key::Enter | crate::view::Key::Esc)
            ),
        };
        // Never turn typing/editing keys into a keystroke log. A field's
        // explicit commit or cancel may create an action without its text.
        let trace = !self.plugins[p].capturing() || commit;
        let span = trace.then(|| {
            self.plugin_dispatch(
                p,
                if section {
                    "plugin.settings.action"
                } else {
                    "plugin.input"
                },
            )
        });
        let capturing = self.plugins[p].capturing();
        let outcome = match &input {
            Input::Key(key) if section => self.section_input(p, scope, *key),
            Input::Key(key) => self.screen_input(p, scope, *key),
            Input::Intent(intent) => self.intent_input(p, scope, section, intent),
        };
        let moved = match &input {
            Input::Key(key) => matches!(
                key.code,
                KeyCode::Up
                    | KeyCode::Down
                    | KeyCode::Home
                    | KeyCode::End
                    | KeyCode::Char('j' | 'k' | 'g' | 'G' | '+' | '-' | '=' | ' ')
            ),
            Input::Intent(i) => matches!(
                i,
                crate::view::Intent::Select { .. } | crate::view::Intent::Fold { .. }
            ),
        };
        if section
            && matches!(outcome, Outcome::Handled(_))
            && (capturing || self.plugins[p].capturing() || moved)
        {
            self.settings_detail.top();
        }
        let out = match outcome {
            Outcome::Handled(fx) => Some(self.route(p, fx, 0)),
            Outcome::Ignored => None,
        };
        if let Some(span) = span {
            self.finish_dispatch(span, out.is_some());
        }
        out
    }

    /// The room views are asked for in between frames (a frontend without
    /// frames, [`crate::headless`], says it once).
    pub fn set_view_room(&self, width: u16, height: u16) {
        self.frontend.set_room(width, height);
    }

    /// The view context for part `p`, in `width` × `height` cells.
    fn view_cx(&self, p: usize, width: u16, height: u16) -> crate::view::ViewCx<'_> {
        crate::view::ViewCx {
            now: self.now,
            focus: self.focus.into(),
            width,
            height,
            parts: compose::Parts::new(&self.plugins, Some(p)),
            icon_lead: self.frontend.lead(),
            setup: self.setup_on_section(),
        }
    }

    /// Part `p`'s screen `scope` as a view, in the room the last frame had;
    /// `None` while the part draws it itself (legacy).
    pub fn screen_view(&self, p: usize, scope: &str) -> Option<crate::view::ScreenView> {
        let (w, h) = self.frontend.room();
        self.screen_view_in(p, scope, w, h)
    }

    pub fn screen_view_in(
        &self,
        p: usize,
        scope: &str,
        width: u16,
        height: u16,
    ) -> Option<crate::view::ScreenView> {
        self.plugins[p].view(scope, &self.view_cx(p, width, height))
    }

    /// Part `p`'s settings section `scope` as a view; `None` while legacy.
    pub fn section_view(&self, p: usize, scope: &str) -> Option<crate::view::SectionView> {
        let (w, h) = self.frontend.room();
        self.section_view_in(p, scope, w, h)
    }

    pub fn section_view_in(
        &self,
        p: usize,
        scope: &str,
        width: u16,
        height: u16,
    ) -> Option<crate::view::SectionView> {
        let mut view = self.plugins[p].section_view(scope, &self.view_cx(p, width, height))?;
        // On a setup step ⏎ goes on and space is the section's ⏎: its help
        // says so.
        if self.setup_on_section() {
            crate::setup::help_only(&mut view);
            crate::setup::space_for_enter(&mut view);
        }
        Some(view)
    }

    fn has_view(&self, p: usize, scope: &str) -> bool {
        if self.plugins[p].screens().iter().any(|s| s.id == scope) {
            self.screen_view(p, scope).is_some()
        } else {
            self.section_view(p, scope).is_some()
        }
    }

    /// The capital letters the host takes as screen keys (every screen's
    /// key) or as the current screen's action keys, in order, once each: a
    /// renderer leaves these to the host and uses any other capital itself.
    pub(crate) fn claimed_capitals(&self) -> Vec<char> {
        let mut keys: Vec<char> = self
            .plugins
            .iter()
            .flat_map(|plugin| plugin.screens())
            .map(|spec| spec.key)
            .collect();
        if let View::Screen(_) = self.view
            && let Some(p) = self.target()
            && let Some((_, spec)) = self.screen()
        {
            keys.extend(own_keys(&self.screen_keys(p, spec.id, self.focus)));
        }
        let mut out = vec![];
        for c in keys {
            if c.is_uppercase() && !out.contains(&c) {
                out.push(c);
            }
        }
        out
    }

    /// The actions menu's rows for a screen, from its view.
    fn screen_keys(&self, p: usize, scope: &str, pane: Pane) -> Vec<Binding> {
        let (w, h) = self.frontend.room();
        let mut cx = self.view_cx(p, w, h);
        cx.focus = pane.into();
        match self.plugins[p].view(scope, &cx) {
            Some(v) => crate::view_tui::bindings(&v.actions),
            None => vec![],
        }
    }

    fn section_keys(&self, p: usize, scope: &str) -> Vec<Binding> {
        match self.section_view(p, scope) {
            Some(v) => crate::view_tui::bindings(&v.actions),
            None => vec![],
        }
    }

    /// A terminal key on a screen, translated by the renderer adapter.
    fn screen_input(&mut self, p: usize, scope: &'static str, key: KeyEvent) -> Outcome {
        let Some(view) = self.screen_view(p, scope) else {
            return Outcome::Ignored;
        };
        let Some(chord) = crate::view_tui::chord_of(&key) else {
            return Outcome::Ignored;
        };
        if self.plugins[p].capturing() {
            return self.plugins[p].intent(scope, self.focus, &crate::view::Intent::Key { chord });
        }
        let routed = crate::view_tui::key(&self.frontend, &view, self.focus.into(), chord);
        let node = interaction_motion::screen_node(&view, self.focus);
        self.routed(p, scope, self.focus, routed, node)
    }

    fn section_input(&mut self, p: usize, scope: &'static str, key: KeyEvent) -> Outcome {
        let Some(view) = self.section_view(p, scope) else {
            return Outcome::Ignored;
        };
        let Some(chord) = crate::view_tui::chord_of(&key) else {
            return Outcome::Ignored;
        };
        if self.plugins[p].capturing() {
            return self.plugins[p].intent(scope, Pane::Middle, &crate::view::Intent::Key { chord });
        }
        let intent = crate::view_tui::section_key(&view, chord);
        self.shared_part_input(p, scope, Pane::Middle, &intent, Some(&view.body))
    }

    /// An intent from a frontend with no terminal (or the Ratatui adapter's,
    /// already translated). A missing view cannot accept plugin input.
    fn intent_input(
        &mut self,
        p: usize,
        scope: &'static str,
        section: bool,
        intent: &crate::view::Intent,
    ) -> Outcome {
        if !self.has_view(p, scope) {
            return Outcome::Ignored;
        }
        // A press on a row of another pane focuses that pane first, and
        // takes the keys from the top bar or a tab row, as a click does in
        // the terminal (`pointer_input`'s caller).
        if !section
            && let crate::view::Intent::Select { target }
            | crate::view::Intent::Activate { target }
            | crate::view::Intent::Context { target } = intent
        {
            if let Some(pane) = self.pane_of_target(p, scope, target) {
                self.focus = pane;
            }
            if !matches!(intent, crate::view::Intent::Context { .. }) {
                self.bar = false;
                self.sources = false;
                self.tabs = false;
            }
        }
        let pane = if section { Pane::Middle } else { self.focus };
        let outcome = self.shared_part_input(p, scope, pane, intent, None);
        // Transitional (M11 A0b): an action invoked by its stable id that the
        // part does not match itself runs by the key its own view lists for
        // that id, under the same dispatch span. An action with no key must
        // be matched by id in the part.
        if let (Outcome::Ignored, crate::view::Intent::Invoke { action }) = (&outcome, intent)
            && let Some(chord) = self.action_chord(p, scope, action)
        {
            // Through the router the terminal's key goes through (a tree's
            // space folds the selected row there).
            let key = crate::view_tui::key_of(chord);
            return if section {
                self.section_input(p, scope, key)
            } else {
                self.screen_input(p, scope, key)
            };
        }
        outcome
    }

    /// The pane of screen `scope` whose rows hold `target`, from the
    /// screen's view; `None` when no pane's rows name it (a tab, a chrome
    /// target, a cell).
    fn pane_of_target(&self, p: usize, scope: &str, target: &crate::view::Target) -> Option<Pane> {
        use crate::view::{Node, TreeRow};
        fn in_tree(rows: &[TreeRow], path: &mut Vec<String>, want: &str) -> bool {
            rows.iter().any(|row| {
                path.push(row.id.clone());
                let hit = TreeRow::target_of(path).as_str() == want || in_tree(&row.children, path, want);
                path.pop();
                hit
            })
        }
        if !self.plugins[p].screens().iter().any(|s| s.id == scope) {
            return None;
        }
        let view = self.screen_view(p, scope)?;
        let want = target.as_str();
        // An actual page owns its panes even when that list is empty. Hidden
        // root panes cannot take the page's pointer intent or keyboard focus.
        let panes = view.page.as_ref().map_or(view.panes.as_slice(), |page| &page.panes);
        panes.iter()
            .find(|pane| match &pane.body {
                Node::List { rows, .. } => rows.iter().any(|r| r.target.as_str() == want),
                Node::Table { rows, .. } => rows.iter().any(|r| r.target.as_str() == want),
                Node::Entries { rows, .. } => rows.iter().any(|r| r.target.as_str() == want),
                Node::Tree(tree) => in_tree(&tree.rows, &mut vec![], want),
                _ => false,
            })
            .map(|pane| pane.pane.into())
    }

    /// The key of the action `action` in the view of the surface the host
    /// is on, when a part's view lists it with one.
    fn invoked_key(&self, action: &str) -> Option<crate::view::Chord> {
        let p = self.target()?;
        let scope = match self.view {
            View::Screen(_) => self.screen()?.1.id,
            View::Settings => self.current_section()?.1.id,
        };
        self.action_chord(p, scope, action)
    }

    /// The key of the action `id` in part `p`'s view of `scope`, when the
    /// view lists it with one.
    fn action_chord(&self, p: usize, scope: &str, id: &str) -> Option<crate::view::Chord> {
        let actions = if self.plugins[p].screens().iter().any(|s| s.id == scope) {
            self.screen_view(p, scope)?.actions
        } else {
            self.section_view(p, scope)?.actions
        };
        actions
            .into_iter()
            .find(|a| !a.heading && !a.id.is_empty() && a.id == id)?
            .key
    }

    /// A pointer press on a target drawn by the renderer adapter.
    fn pointer_input(
        &mut self,
        p: usize,
        scope: &'static str,
        pane: Pane,
        id: &str,
        _at: ratatui::layout::Position,
        context: bool,
    ) -> Outcome {
        if id.starts_with(crate::view::detail::TARGET_PREFIX) {
            return Outcome::done();
        }
        if !self.has_view(p, scope) {
            return Outcome::Ignored;
        }
        let routed = crate::view_tui::click(&self.frontend, id, context);
        self.routed(p, scope, pane, routed, None)
    }

    fn routed(
        &mut self,
        p: usize,
        scope: &'static str,
        pane: Pane,
        routed: crate::view_tui::Routed,
        known: Option<&crate::view::Node>,
    ) -> Outcome {
        use crate::view_tui::Routed;
        match routed {
            Routed::Intent(i) => self.shared_part_input(p, scope, pane, &i, known),
            Routed::Done => Outcome::done(),
            Routed::Ignored => Outcome::Ignored,
        }
    }

    /// What the last frame drew at a cell (the topmost area).
    pub fn hit_at(&self, column: u16, row: u16) -> Option<Hit> {
        let at = ratatui::layout::Position { x: column, y: row };
        self.hits
            .borrow()
            .iter()
            .rev()
            .find(|(r, _)| r.contains(at))
            .map(|(_, h)| h.clone())
    }

    /// What a click at `at` would act on, and the area of it to light: the
    /// topmost hit there, on the pointer's line. `None` over nothing a
    /// click acts on: empty space, a pane's background (a click there only
    /// focuses it), a menu's frame or one of its headings.
    pub fn hover_at(&self, at: ratatui::layout::Position) -> Option<(ratatui::layout::Rect, Hit)> {
        let hits = self.hits.borrow();
        let (rect, hit) = hits.iter().rev().find(|(r, _)| r.contains(at))?;
        // While something is typed, clicks wait for it (⏎ or esc), except
        // on the status line's keys: only those are lit.
        let typing = self.menu.is_none() && self.capturing();
        let target = match hit {
            Hit::Key(..) => true,
            _ if typing => false,
            Hit::Pane(_) | Hit::MenuBox => false,
            Hit::Plugin { id, .. } => id != crate::view_tui::PANE_HIT,
            Hit::MenuItem(i) => self
                .menu
                .as_ref()
                .and_then(|m| m.menu.items.get(*i))
                .is_some_and(|item| !item.heading && !item.note),
            _ => true,
        };
        target.then(|| {
            (
                ratatui::layout::Rect {
                    y: at.y,
                    height: 1,
                    ..*rect
                },
                hit.clone(),
            )
        })
    }

    /// The pointer moved to `at` (`None`: it left the window). Over a
    /// menu's item it moves the menu's highlight there, as a desktop menu
    /// does. Whether what is lit changed, so the screen needs drawing.
    pub fn hover(&mut self, at: Option<ratatui::layout::Position>) -> bool {
        self.pointer = at;
        let now = at.and_then(|p| self.hover_at(p));
        let mut changed = *self.hovered.borrow() != now;
        if let (Some((_, Hit::MenuItem(i))), Some(open)) = (&now, self.menu.as_mut())
            && open.row != *i
        {
            open.row = *i;
            changed = true;
        }
        *self.hovered.borrow_mut() = now;
        changed
    }

    /// The pointer's shape for what it is over: a hand where a click acts.
    pub fn pointer_shape(&self) -> crate::ui::pointer::Shape {
        match self.hovered.borrow().as_ref() {
            Some(_) => crate::ui::pointer::Shape::Hand,
            None => crate::ui::pointer::Shape::Default,
        }
    }

    fn on_mouse(&mut self, kind: MouseKind, column: u16, row: u16, at_ms: u64) -> Vec<HostEffect> {
        let replacing_context = kind == MouseKind::Context
            && self
                .menu
                .as_ref()
                .is_some_and(|open| open.owner == Owner::Host && open.menu.id == KEYS_MENU);
        // The menu is drawn last, so its hits cover the screen's hits. A
        // second right-click addresses the screen beneath that menu.
        let hit = if replacing_context {
            let at = ratatui::layout::Position { x: column, y: row };
            self.hits
                .borrow()
                .iter()
                .rev()
                .find(|(rect, hit)| {
                    rect.contains(at) && !matches!(hit, Hit::MenuBox | Hit::MenuItem(_))
                })
                .map(|(_, hit)| hit.clone())
        } else {
            self.hit_at(column, row)
        };
        if matches!(hit, Some(Hit::NoticeHistoryOpen)) {
            if kind == MouseKind::Press && self.notice_history_can_open() { self.open_notice_history(); }
            return vec![];
        }
        if let Some(Hit::Notice(id)) = hit {
            if kind == MouseKind::Press && self.notices_visible() { self.notices.dismiss(id); self.sync_message_compat(); }
            return vec![];
        }
        let detail_target = match &hit {
            Some(Hit::Plugin { id, .. }) if id.starts_with(crate::view::detail::TARGET_PREFIX) => Some(id.clone()),
            _ => None,
        };
        if detail_target.is_some() && kind == MouseKind::Context {
            self.last_press = None;
            return vec![];
        }
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        if kind == MouseKind::Context {
            if self.menu.is_some() && !replacing_context || self.capturing() {
                return vec![];
            }
            if replacing_context {
                self.cancel_menu_trace(true);
                self.menu = None;
            }
            self.last_press = None;
            let out = match hit {
                Some(Hit::Plugin { plugin, scope, pane, id }) => {
                    let outcome = self.pointer_input(
                        plugin,
                        scope,
                        pane,
                        &id,
                        ratatui::layout::Position { x: column, y: row },
                        true,
                    );
                    let Outcome::Handled(effects) = outcome else {
                        return vec![];
                    };
                    self.focus = pane;
                    self.route(plugin, effects, 0)
                }
                Some(Hit::ColourRow(i)) if self.view == View::Settings => {
                    self.focus = Pane::Middle;
                    if self.on_icons() {
                        self.colours.icon_row = i;
                    } else {
                        self.colours.row = i;
                    }
                    vec![]
                }
                Some(Hit::PartsRow(i)) if self.view == View::Settings => {
                    self.focus = Pane::Middle;
                    self.parts_row = i;
                    vec![]
                }
                Some(Hit::Section(i)) if self.view == View::Settings => {
                    self.focus = Pane::Nav;
                    self.select_section(i);
                    vec![]
                }
                _ => return vec![],
            };
            self.settings_detail.top();
            self.bar = false;
            self.tabs = false;
            self.open_keys_menu();
            if let Some(open) = self.menu.as_mut() {
                open.anchor = Some(ratatui::layout::Position { x: column, y: row });
            }
            return out;
        }
        // A key on the status line is pressed, whatever has the keys: the
        // menu's `⏎ choose`, a field's `esc cancel`.
        // Chrome targets are pressed the way every frontend presses them: an
        // Intent::Activate on the target the chrome view gave.
        if let (MouseKind::Press, Some(h @ Hit::Key(..))) = (kind, &hit)
            && let Some(target) = self.chrome_target_of(h)
        {
            self.last_press = None;
            return self.on_intent(crate::view::Intent::Activate { target });
        }
        // With a menu open, the mouse is the menu's.
        if let Some(open) = self.menu.as_mut() {
            return match (kind, hit) {
                (MouseKind::Press, Some(h @ Hit::MenuItem(_))) => match self.chrome_target_of(&h) {
                    Some(target) => self.on_intent(crate::view::Intent::Activate { target }),
                    None => vec![],
                },
                (MouseKind::Press, Some(Hit::MenuBox)) => vec![],
                (MouseKind::Press, _) => {
                    self.cancel_menu_trace(true);
                    self.menu = None;
                    vec![]
                }
                (MouseKind::ScrollUp | MouseKind::ScrollDown, _)
                    if open.menu.question || open.viewport.more() =>
                {
                    open.viewport.scroll(kind == MouseKind::ScrollDown, false);
                    vec![]
                }
                (MouseKind::ScrollUp, _) => self.menu_key(key(KeyCode::Up)),
                (MouseKind::ScrollDown, _) => self.menu_key(key(KeyCode::Down)),
                (MouseKind::Context, _) => unreachable!("context clicks are handled above"),
            };
        }
        // Typing goes on until ⏎ or esc; a click elsewhere does not drop it.
        if self.colours.capturing() || self.target().is_some_and(|p| self.plugins[p].capturing()) {
            return vec![];
        }
        // Controls bypass row focus and double-press synthesis entirely.
        if let Some(target) = detail_target {
            self.last_press = None;
            return if kind == MouseKind::Press {
                self.activate_detail_control(&crate::view::Target::new(target))
            } else { vec![] };
        }
        // A click anywhere takes the keys from the top bar, or from a
        // page's tab row.
        if kind == MouseKind::Press {
            self.bar = false;
            self.sources = false;
            self.tabs = false;
        }
        let Some(hit) = hit else { return vec![] };
        let pane = match &hit {
            Hit::Section(_) | Hit::Group(_) => Some(Pane::Nav),
            Hit::ColourRow(_) | Hit::PartsRow(_) | Hit::Mode | Hit::PauseIdle | Hit::Setup(_) => {
                Some(Pane::Middle)
            }
            Hit::Pane(p) | Hit::Plugin { pane: p, .. } => Some(*p),
            _ => None,
        };
        if kind != MouseKind::Press {
            // The wheel moves the selection of the pane under the pointer.
            if let Some(p) = pane {
                self.focus = p;
            }
            let k = if kind == MouseKind::ScrollUp {
                'k'
            } else {
                'j'
            };
            let fx = self.on_key(key(KeyCode::Char(k)));
            // The wheel past the top stays in the pane; only keys go up
            // into the top bar or a page's tab row.
            self.bar = false;
            self.sources = false;
            self.tabs = false;
            return fx;
        }
        // A click on a row focuses its pane and selects the row at once. A
        // click on a pane's empty space or border only focuses it: its cursor
        // stays where you left it.
        let focuses_only = matches!(hit, Hit::Pane(_)) && pane.is_some_and(|p| p != self.focus);
        if focuses_only {
            self.focus = pane.expect("checked");
            self.last_press = None;
            return vec![];
        }
        // Page tabs act on the first click. Repeating one before a redraw
        // must not press Enter on the new tab's first row.
        // A chooser's row ticks on each click.
        let ticking = matches!(&hit, Hit::Setup(id) if id.starts_with("setup:choose:"));
        let single_click = matches!(&hit, Hit::SourceTab(_) | Hit::SetupStop(_))
            || ticking
            || matches!(&hit, Hit::Plugin { id, .. } if id.starts_with("tab:"));
        let double = !single_click
            && self
                .last_press
                .as_ref()
                .is_some_and(|(h, t)| *h == hit && at_ms.saturating_sub(*t) <= 400);
        self.last_press = if double {
            None
        } else {
            Some((hit.clone(), at_ms))
        };
        if let Some(p) = pane {
            self.focus = p;
        }
        if self.view == View::Settings
            && self.focus == Pane::Middle
            && !matches!(&hit, Hit::Pane(_))
            && !matches!(&hit, Hit::Plugin { id, .. } if id == crate::view_tui::PANE_HIT)
        {
            self.settings_detail.top();
        }
        let mut out = match hit {
            hit @ (Hit::HistoryTab
            | Hit::Tab(_)
            | Hit::SourceTab(_)
            | Hit::Section(_)
            | Hit::Group(_)
            | Hit::Mode
            | Hit::Health(_)
            | Hit::SetupStop(_)
            | Hit::Attention) => {
                return match self.chrome_target_of(&hit) {
                    Some(target) => self.on_intent(crate::view::Intent::Activate { target }),
                    None => vec![],
                };
            }
            Hit::PartsRow(i) => {
                self.parts_row = i;
                vec![]
            }
            // Selected (a chooser's row ticked); a double click chooses.
            Hit::Setup(id) => {
                if self.setup.is_none() {
                    self.look_settings_target(&crate::view::Target::new(id), false);
                } else {
                    self.setup_target(&crate::view::Target::new(id), ticking);
                }
                vec![]
            }
            Hit::PauseIdle => {
                self.pause.row = 1;
                self.open_idle_menu();
                return vec![];
            }
            Hit::ColourRow(i) => {
                if self.on_icons() {
                    self.colours.icon_row = i;
                } else {
                    self.colours.row = i;
                }
                vec![]
            }
            Hit::Plugin {
                plugin, scope, id, pane,
            } => {
                let span = self.plugin_dispatch(plugin, "plugin.pointer.action");
                let outcome = self.pointer_input(
                    plugin,
                    scope,
                    pane,
                    &id,
                    ratatui::layout::Position { x: column, y: row },
                    false,
                );
                let (out, handled) = match outcome {
                    Outcome::Handled(fx) => (self.route(plugin, fx, 0), true),
                    Outcome::Ignored => (vec![], false),
                };
                self.finish_dispatch(span, handled);
                out
            }
            Hit::Notice(_) | Hit::NoticeHistory(_) | Hit::NoticeHistoryOpen | Hit::NoticeHistoryKey(_) | Hit::History | Hit::Pane(_) | Hit::MenuBox | Hit::MenuItem(_) | Hit::Key(..) => vec![],
        };
        if double {
            // On a setup step that is a section, a double click is the
            // row's own ⏎, not setup's "continue".
            let enter = key(KeyCode::Enter);
            if self.setup_on_section() {
                out.extend(self.setup_section_takes(enter).unwrap_or_default());
            } else {
                out.extend(self.on_key(enter));
            }
        }
        out
    }

    fn on_key(&mut self, key: KeyEvent) -> Vec<HostEffect> {
        self.clear_message_compat();
        // ctrl-c always gets out, even mid-typing or with a menu open.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return vec![];
        }
        if self.menu.is_some() {
            return self.menu_key(key);
        }
        if palette::shortcut(key) && !self.capturing() {
            self.open_palette();
            return vec![];
        }
        if self.colours.capturing() {
            return self.colours_key(key);
        }
        if self.target().is_some_and(|p| self.plugins[p].capturing()) {
            return self.deliver(key).unwrap_or_default();
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            // Other ctrl keys are the plugin's.
            return self.deliver(key).unwrap_or_default();
        }
        // Setup's keys ([`crate::setup`]); what it leaves goes on below, to
        // the host's quit and menu, and to the section a step shows.
        if self.setup.is_some()
            && let Some(fx) = self.setup_key(key)
        {
            return fx;
        }
        if self.tour.is_some()
            && let Some(fx) = self.tour_key(key)
        {
            return fx;
        }
        if key.code == KeyCode::Char('~') && self.notice_history_can_open() {
            self.open_notice_history();
            return vec![];
        }
        if key.code == KeyCode::Char(HISTORY_KEY) {
            return self.open_recent_history();
        }
        // The page whose tab row had the keys is gone: so is the row.
        if self.tabs && self.page_tabs().is_none() {
            self.tabs = false;
        }
        if self.sources && !self.has_source_row() {
            self.sources = false;
        }
        if self.tabs {
            if let Some(fx) = self.tabs_key(key) {
                return fx;
            }
        } else if self.bar {
            if let Some(fx) = self.bar_key(key) {
                return fx;
            }
        } else if self.sources {
            if let Some(fx) = self.sources_key(key) {
                return fx;
            }
        } else {
            self.last_pane.insert(self.view, self.focus);
        }
        if let Some(out) = self.shared_tree_key(key) {
            return out;
        }
        match key.code {
            // Settings are saved as they change; only a failed save leaves
            // something to lose to a stray q: ask first.
            KeyCode::Char('q') if self.unsaved() => {
                self.menu = Some(OpenMenu::open(
                    Owner::Host,
                    Menu::new(
                        QUIT_MENU,
                        "your settings are not saved",
                        vec![
                            MenuItem::new(
                                'w',
                                "write settings and quit",
                                "write config.toml first",
                                "save",
                            ),
                            MenuItem::new(
                                'd',
                                "discard changes and quit",
                                "lose the changes",
                                "discard",
                            ),
                            MenuItem::new('s', "stay", "keep working", "stay"),
                        ],
                    ),
                ));
                return vec![];
            }
            KeyCode::Char('q') => {
                self.quit = true;
                return vec![];
            }
            KeyCode::Char('x' | '?') => {
                self.open_keys_menu();
                return vec![];
            }
            // The agent dock: shows and takes the keys, then hides.
            KeyCode::Char(crate::dock::KEY) if key.modifiers.is_empty() && self.dock_owner().is_some() => {
                // Where the dock is not drawn (an item page, the work area)
                // the key shows or hides it for the lists, and the keys stay
                // here.
                let placed = self.dock_has_place();
                // A session that wants the user out of sight comes first.
                let out = match self.go_to_attention() {
                    Some(out) => out,
                    None => {
                        let intent = if !self.dock.visible {
                            crate::dock::DockIntent::Show
                        } else if !self.dock.focused && placed {
                            crate::dock::DockIntent::Focus
                        } else {
                            crate::dock::DockIntent::Hide
                        };
                        self.on_dock_intent(intent)
                    }
                };
                if !placed && self.dock.visible {
                    self.notify("the agent dock shows beside the lists, not on this page".into());
                }
                return out;
            }
            // One pause for every part.
            KeyCode::Char('z') if key.modifiers.is_empty() => {
                self.toggle_pause();
                return vec![];
            }
            KeyCode::Char('b') => {
                match self.bulk() {
                    Some((p, items)) => {
                        self.menu = Some(OpenMenu::open(
                            Owner::Plugin(p),
                            Menu::new(BULK_MENU, "bulk commands", items),
                        ))
                    }
                    None => self.notify("no bulk commands here".into()),
                }
                return vec![];
            }
            // Saving happens by itself; w still writes the file now.
            KeyCode::Char('w') => return vec![self.save(true)],
            KeyCode::Char(SETTINGS_KEY) => {
                self.view = View::Settings;
                self.back.clear();
                self.tabs = false;
                self.sources = false;
                return vec![];
            }
            KeyCode::Char(c) if c.is_ascii_uppercase() => {
                let group = self
                    .top_screens()
                    .into_iter()
                    .find(|&i| self.screen_group(i).is_some_and(|group| group.key == c));
                let target = group
                    .map(|i| match self.view {
                        View::Screen(current)
                            if self
                                .screen_group(current)
                                .zip(self.screen_group(i))
                                .is_some_and(|(a, b)| a.id == b.id) =>
                        {
                            current
                        }
                        _ => i,
                    })
                    .or_else(|| self.screens.iter().position(|(_, s)| s.key == c));
                if let Some(i) = target {
                    self.view = View::Screen(i);
                    self.back.clear();
                    self.tabs = false;
                    self.sources = false;
                    return vec![];
                }
            }
            // Panes, lazydocker's way: tab and shift-tab go round, h l ← →
            // go left and right, a number goes straight to that pane.
            KeyCode::Tab | KeyCode::BackTab => {
                let panes = self.panes();
                let at = panes.iter().position(|p| *p == self.focus).unwrap_or(0);
                let n = panes.len();
                // An item page with one pane and a row of tabs: round the
                // tabs instead, so the key is never dead on a page.
                if n == 1 && self.page_tabs().is_some_and(|t| t.names.len() > 1) {
                    return self.show_tab_beside(key.code == KeyCode::Tab);
                }
                let next = if key.code == KeyCode::Tab {
                    (at + 1) % n
                } else {
                    (at + n - 1) % n
                };
                self.focus = panes[next];
                return vec![];
            }
            KeyCode::Char('h' | 'l') | KeyCode::Left | KeyCode::Right => {
                let panes = self.panes();
                let at = panes.iter().position(|p| *p == self.focus).unwrap_or(0);
                let left = matches!(key.code, KeyCode::Char('h') | KeyCode::Left);
                // Past the outer pane: the screen beside, as the top bar
                // has them (not from inside an item page).
                let edge = if left { at == 0 } else { at + 1 >= panes.len() };
                if edge && self.on_top_level() {
                    self.next_screen(!left);
                    return vec![];
                }
                if edge {
                    // Past the outer pane of an item page: the tab beside,
                    // as the top bar's screens lie beside each other.
                    if self.page_tabs().is_some_and(|t| t.names.len() > 1) {
                        return self.show_tab_beside(!left);
                    }
                    self.nothing_to_move_to();
                    return vec![];
                }
                let next = if left {
                    at.saturating_sub(1)
                } else {
                    (at + 1).min(panes.len() - 1)
                };
                self.focus = panes[next];
                return vec![];
            }
            KeyCode::Char(c @ '1'..='9') => {
                if let Some(jump) = self.pane_jumps().into_iter().find(|jump| jump.key == c) {
                    self.focus = jump.pane.into();
                    return vec![];
                }
            }
            _ => {}
        }
        if self.view == View::Settings {
            if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
                if !self.settings_detail.page(key.code == KeyCode::PageDown) {
                    self.notify("nothing to scroll here".into());
                }
                return vec![];
            }
            if self.focus == Pane::Detail && self.settings_detail.key(&key) {
                return vec![];
            }
        }
        if self.view == View::Settings && self.focus == Pane::Nav
            && let Some(down) = vertical(&key)
        {
            let rows = self.nav_rows();
            let at = rows.iter().position(|r| *r == self.nav_at()).unwrap_or(0);
            if !down && at == 0 {
                self.enter_bar();
            } else {
                self.nav_go(rows[step(at, rows.len(), down)]);
            }
            return vec![];
        }
        if self.in_colours_section() {
            let before = (self.colours.row, self.colours.icon_row);
            let out = self.colours_key(key);
            if before != (self.colours.row, self.colours.icon_row) || self.colours.capturing() {
                self.settings_detail.top();
            }
            return out;
        }
        if self.view == View::Settings
            && self.focus == Pane::Middle
            && self
                .current_section()
                .is_some_and(|(o, _)| o == Owner::Parts)
        {
            let before = self.parts_row;
            let out = self.parts_key(key);
            if before != self.parts_row {
                self.settings_detail.top();
            }
            return out;
        }
        if self.view == View::Settings
            && self.focus == Pane::Middle
            && self.setup.is_none()
            && self.current_section().is_some_and(|(o, _)| o == Owner::Look)
        {
            if self.look_settings_key(key) {
                return vec![];
            }
            if key.code == KeyCode::Esc {
                self.focus = Pane::Nav;
                return vec![];
            }
        }
        if self.view == View::Settings
            && self.focus == Pane::Middle
            && self
                .current_section()
                .is_some_and(|(o, _)| o == Owner::Mode)
        {
            let before = self.pause.row;
            let out = self.mode_key(key);
            if before != self.pause.row {
                self.settings_detail.top();
            }
            return out;
        }
        // A part not set up yet: its screen is the set-up page; ⏎ goes to
        // its settings.
        if let Some(p) = self.setting_up() {
            if key.code == KeyCode::Enter {
                self.go_to_settings_of(p);
            } else if key.code == KeyCode::Char('s')
                && let Some(step) = self.plugins[p].setup_step()
            {
                let _ = self.open_setup(Some(step.id));
            } else if key.code == KeyCode::Esc {
                self.come_back(true);
            } else if matches!(key.code, KeyCode::Up | KeyCode::Char('k')) {
                self.above_body();
            }
            return vec![];
        }
        if let Some(fx) = self.deliver(key) {
            if key.code == KeyCode::Esc {
                self.come_back(false);
            }
            return fx;
        }
        if key.code == KeyCode::Esc && self.come_back(true) {
            return vec![];
        }
        if let KeyCode::Char(c) = key.code
            && let Some(fx) = self.contributed_key(c)
        {
            return fx;
        }
        // `+` or `-` in a tree where the row has nothing to open or close:
        // the tree stays as it is, and the status line says why.
        if let KeyCode::Char(c @ ('+' | '=' | '-')) = key.code
            && self.folds_here()
        {
            self.nothing_to_fold(c);
            return vec![];
        }
        // The plugin did not want it: ⏎ goes into the next pane, esc comes
        // back out.
        let panes = self.panes();
        let at = panes.iter().position(|p| *p == self.focus).unwrap_or(0);
        match key.code {
            KeyCode::Enter if at + 1 < panes.len() && self.focus == Pane::Nav => {
                self.focus = panes[at + 1]
            }
            KeyCode::Esc if at > 0 => self.focus = panes[at - 1],
            // Up past the top of a pane (it could not move): the top bar,
            // on a screen's own panes; inside an item page, nothing.
            KeyCode::Up | KeyCode::Char('k') if self.on_top_level() => self.above_body(),
            // Up past the top of an item page's body: its row of tabs.
            KeyCode::Up | KeyCode::Char('k') if self.page_tabs().is_some() => self.tabs = true,
            // Inside an item page, an arrow nothing took says so (the
            // status line offers them everywhere).
            KeyCode::Up | KeyCode::Down => self.nothing_to_move_to(),
            // The status line offers PgUp/PgDn everywhere: where nothing
            // took them, it says so rather than doing nothing.
            KeyCode::PageUp | KeyCode::PageDown => {
                self.notify("nothing to scroll here".into())
            }
            _ => {}
        }
        vec![]
    }

    /// An arrow with nowhere to go, said on the status line.
    fn nothing_to_move_to(&mut self) {
        let back = if self.on_top_level() {
            ""
        } else {
            " · esc goes back"
        };
        self.notify(format!("nothing to move to here{back}").into());
    }

    /// The bulk commands for what is on screen, and the plugin they belong
    /// to; `None` when there are none.
    pub fn bulk(&self) -> Option<(usize, Vec<MenuItem>)> {
        let (p, scope) = match self.view {
            View::Screen(_) => self.screen().map(|(p, s)| (p, s.id))?,
            View::Settings => match self.current_section()? {
                (Owner::Plugin(p), s) => (p, s.id),
                (Owner::Host | Owner::Parts | Owner::Mode | Owner::Look, _) => return None,
            },
        };
        if !self.ready(p) && self.view != View::Settings {
            return None;
        }
        let items = self.plugins[p].bulk(scope);
        (!items.is_empty()).then_some((p, items))
    }

    /// The part and scope whose own keys are offered here. Host navigation
    /// rows and setup pages have no plugin target context.
    fn menu_key_route(&self) -> Option<(usize, &'static str)> {
        if self.bar || self.sources || self.tabs {
            return None;
        }
        match (self.view, self.focus) {
            (View::Screen(_), _) => self
                .screen()
                .filter(|(p, _)| self.ready(*p))
                .map(|(p, s)| (p, s.id)),
            (View::Settings, Pane::Middle) => match self.current_section()? {
                (Owner::Plugin(p), s) => Some((p, s.id)),
                _ => None,
            },
            _ => None,
        }
    }

    fn menu_key_context(&self, action: &str) -> Option<MenuKeyContext> {
        let (plugin, scope) = self.menu_key_route()?;
        let token = self.plugins[plugin].menu_key_context(scope, self.focus, action)?;
        Some(MenuKeyContext {
            plugin,
            scope,
            view: self.view,
            focus: self.focus,
            bar: self.bar,
            tabs: self.tabs,
            action: action.to_owned(),
            token,
        })
    }

    fn menu_key_is_current(&self, context: &MenuKeyContext) -> bool {
        self.view == context.view
            && self.focus == context.focus
            && self.bar == context.bar
            && self.tabs == context.tabs
            && !self.capturing()
            && self.menu_key_route() == Some((context.plugin, context.scope))
            && self.plugins[context.plugin]
                .menu_key_context(context.scope, context.focus, &context.action)
                .as_ref()
                == Some(&context.token)
    }

    fn menu_subject_context(&self) -> Option<MenuKeyContext> {
        let (plugin, scope) = self.menu_key_route()?;
        let token = self.plugins[plugin].subject_menu_context(scope, self.focus)?;
        Some(MenuKeyContext {
            plugin,
            scope,
            view: self.view,
            focus: self.focus,
            bar: self.bar,
            tabs: self.tabs,
            action: String::new(),
            token,
        })
    }

    fn menu_subject_is_current(
        &self,
        context: &MenuKeyContext,
        contributor: usize,
        subject: &Subject,
        action: &str,
    ) -> bool {
        if self.view != context.view
            || self.focus != context.focus
            || self.bar != context.bar
            || self.tabs != context.tabs
            || self.capturing()
            || self.menu_key_route() != Some((context.plugin, context.scope))
            || self.plugins[context.plugin]
                .subject_menu_context(context.scope, context.focus)
                .as_ref()
                != Some(&context.token)
            || !self.ready(contributor)
        {
            return false;
        }
        let Some((owner, current)) = self.subject_here() else {
            return false;
        };
        if owner != context.plugin
            || current != *subject
            || current.url != subject.url
            || current.fields != subject.fields
        {
            return false;
        }
        matches!(self.plugins[contributor].contribute(Point::Actions, &current),
            Some(Contribution::Actions(actions)) if actions.iter().any(|a| a.id == action))
    }

    /// The focused pane's keys, then the host's, as a menu: choosing a row
    /// presses its key.
    fn open_keys_menu(&mut self) {
        // Only what can be done here, to what is selected; moving about is
        // on the status line. The first heading, if any, names the menu.
        let setting_up = self.setting_up();
        let mut rows = match (self.view, self.focus) {
            (View::Screen(_), _) if setting_up.is_some() => {
                let p = setting_up.expect("checked");
                let name = self.plugins[p].name();
                let mut rows = vec![Binding::heading(name)];
                if self.first_section_of(p).is_some() {
                    rows.push(Binding::new("⏎", format!("set up {name} in settings")));
                }
                if self.returns_to().is_some() {
                    rows.push(Binding::new("esc", "back"));
                }
                rows
            }
            (View::Screen(_), pane) => match self.screen() {
                Some((p, s)) => self.screen_keys(p, s.id, pane),
                None => vec![],
            },
            (View::Settings, Pane::Middle) => match self.current_section() {
                Some((Owner::Parts, _)) => crate::parts::keys(self),
                Some((Owner::Mode, _)) => crate::mode::keys(self),
                Some((Owner::Look, _)) => vec![
                    crate::Binding::new("space", "choose"),
                    crate::Binding::new("↑↓", "move"),
                ],
                Some((Owner::Plugin(p), s)) => self.section_keys(p, s.id),
                Some((Owner::Host, s)) if s.id == ICONS_SECTION.id => self.colours.icons_keys(),
                Some((Owner::Host, s)) => {
                    let what = match self.colours.selected() {
                        crate::colours::Row::Palette => "pick a palette",
                        crate::colours::Row::Depth => "pick how many colours",
                        crate::colours::Row::Motion => "pick how much motion",
                        crate::colours::Row::Splash => "choose whether the startup splash shows",
                        crate::colours::Row::Icons => "pick an icon pack",
                        crate::colours::Row::Slot(_) => "change this colour",
                        crate::colours::Row::Links => "choose when links open on a click",
                        crate::colours::Row::Images => "choose when pictures are drawn",
                        crate::colours::Row::Pointer => "choose when the mouse pointer changes",
                        crate::colours::Row::Header(_) => "",
                    };
                    let mut rows = vec![Binding::heading(s.name)];
                    if !what.is_empty() {
                        rows.push(Binding::new("⏎", what));
                    }
                    rows
                }
                None => vec![],
            },
            (View::Settings, Pane::Detail) => vec![
                Binding::heading("setting details"),
                Binding::new("esc", "back to the settings fields"),
            ],
            (View::Settings, Pane::Nav) => {
                let mut rows = vec![Binding::heading("settings")];
                if let Some((_, s)) = self.current_section() {
                    rows.push(Binding::new("⏎", format!("go into {}", s.name)));
                }
                rows
            }
        };
        // The top bar's own keys.
        if self.bar {
            rows = vec![
                Binding::heading("screens"),
                Binding::new("←", "the screen to the left"),
                Binding::new("→", "the screen to the right"),
                Binding::new("↓", "into the source row or screen"),
            ];
        }
        if self.sources {
            rows = vec![
                Binding::heading("repository sources"),
                Binding::new("←", "the source to the left"),
                Binding::new("→", "the source to the right"),
                Binding::new("↑", "back to the top bar"),
                Binding::new("↓", "into this source's panes"),
            ];
        }
        // A page's tab row's own keys: the tabs either side, by name.
        if self.tabs
            && let Some(t) = self.page_tabs()
        {
            let n = t.names.len();
            let word = |i: usize| {
                t.names[i % n]
                    .split(" · ")
                    .next()
                    .unwrap_or_default()
                    .to_string()
            };
            rows = vec![
                Binding::heading("tabs"),
                Binding::new("←", format!("the tab to the left: {}", word(t.on + n - 1))),
                Binding::new("→", format!("the tab to the right: {}", word(t.on + 1))),
                Binding::new("↓", "into the page"),
            ];
        }
        // A tree with groups: `+` and `-` (the status line names them too).
        // Before a trailing "back" row, so that stays last.
        let fold_keys = self.shared_fold_keys().unwrap_or_else(|| match self.view {
            View::Screen(_) => self
                .screen()
                .filter(|(p, _)| self.ready(*p))
                .map(|(p, s)| self.plugins[p].fold_keys(s.id, self.focus))
                .unwrap_or_default(),
            View::Settings if self.folds_here() => vec![
                Binding::new("+", "expand the group"),
                Binding::new("-", "collapse the group, or the one this row is in"),
            ],
            View::Settings => Vec::new(),
        });
        if !fold_keys.is_empty() && !self.bar && !self.sources && !self.tabs {
            let at = rows
                .iter()
                .rposition(|r| r.keys == "esc")
                .filter(|i| i + 1 == rows.len())
                .unwrap_or(rows.len());
            let fold_keys = fold_keys.into_iter().filter(|fold| !rows.iter().any(|row| row.keys == fold.keys)).collect::<Vec<_>>();
            rows.splice(at..at, fold_keys);
        }
        if self.view == View::Settings && self.unsaved() {
            rows.push(Binding::new("w", "try writing config.toml again"));
        }
        // Shown from another part: esc goes back there.
        if let Some(back) = self.returns_to() {
            for b in rows.iter_mut().filter(|b| b.keys == "esc") {
                b.what = format!("back to {back}");
            }
        }
        let (title, title_icon) = match rows.first() {
            Some(b) if b.is_heading() => {
                let b = rows.remove(0);
                (b.what, b.icon)
            }
            _ => ("actions".into(), None),
        };
        // A row that cannot be pressed is not shown.
        rows.retain(|b| b.is_heading() || crate::plugin::parse_key(&b.keys).is_some());
        // What the other parts add, under their names.
        let contributed = match self.subject_here().filter(|_| !self.bar && !self.sources && !self.tabs) {
            Some((owner, subject)) => self
                .contributed_actions(owner, &subject, own_keys(&rows))
                .into_iter()
                .map(|(p, name, actions)| (p, name, actions, subject.clone()))
                .collect(),
            None => vec![],
        };
        let mut items = Vec::new();
        let mut presses = Vec::new();
        for b in &rows {
            if b.is_heading() {
                let mut heading = MenuItem::heading(&b.what);
                heading.icon = b.icon;
                items.push(heading);
                presses.push(None);
                continue;
            }
            let mut item = MenuItem::pick(&b.what, "", items.len().to_string()).shown_as(&b.keys);
            item.icon = b.icon;
            // A write the mode refuses: listed, muted, saying why; its key
            // still opens the question, which says so too.
            if let Some(why) = b.change.and_then(crate::writes::row_note) {
                item.label = format!("{} · {why}", b.what);
                item.muted = true;
            }
            // Typing the key in the menu runs it too, except the keys
            // that move in the menu or close it.
            if let Some(KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            }) = crate::plugin::parse_key(&b.keys)
                && modifiers.difference(KeyModifiers::SHIFT).is_empty()
                && !matches!(c, 'j' | 'k' | 'q' | 'x' | '?')
            {
                item.key = Some(c);
            }
            items.push(item);
            presses.push(
                crate::plugin::parse_key(&b.keys)
                    .map(|key| match self.tree_menu_context(key) {
                        Some(context) => Press::TreeKey(key, context),
                        None => Press::Key(key, self.menu_key_context(&b.keys)),
                    }),
            );
        }
        for (p, name, actions, subject) in contributed {
            items.push(MenuItem::heading(name));
            presses.push(None);
            for a in actions {
                let keys = a.key.map(crate::plugin::key_text).unwrap_or_default();
                let mut item = MenuItem::pick(&a.what, "", items.len().to_string())
                    .shown_as(keys)
                    .icon(Some(a.icon.unwrap_or(crate::ui::icon::Icon::Action)));
                item.key = a.key;
                if let Some(why) = a.change.and_then(crate::writes::row_note) {
                    item.label = format!("{} · {why}", a.what);
                    item.muted = true;
                }
                items.push(item);
                presses.push(Some(Press::Act(
                    p,
                    subject.clone(),
                    a.id,
                    self.menu_subject_context(),
                )));
            }
        }
        items.push(MenuItem::new('~', "recent notices (this session)", "", items.len().to_string()));
        presses.push(Some(Press::Key(KeyEvent::new(KeyCode::Char('~'), KeyModifiers::NONE), self.menu_key_context("~"))));
        items.push(MenuItem::heading("History"));
        presses.push(None);
        items.push(MenuItem::new(
            HISTORY_KEY,
            "recent activity (last 24 hours)",
            "",
            items.len().to_string(),
        ));
        presses.push(Some(Press::RecentHistory));
        items.push(MenuItem::pick(
            "all activity history",
            "",
            items.len().to_string(),
        ));
        presses.push(Some(Press::History(None)));
        if let Some((_, subject)) = self.subject_here()
            && let Some(snapshot) =
                self.journal_snapshot(&subject, crate::journal::SubjectRole::Primary)
        {
            items.push(MenuItem::pick(
                "history for selected item",
                "",
                items.len().to_string(),
            ));
            presses.push(Some(Press::History(Some(snapshot.key))));
        }
        self.menu_keys = presses;
        self.menu = Some(OpenMenu::open(
            Owner::Host,
            Menu {
                icon: title_icon,
                ..Menu::new(KEYS_MENU, title, items)
            },
        ));
    }

    fn open_recent_history(&mut self) -> Vec<HostEffect> {
        self.notice_history = None;
        self.history_request = self.history_request.wrapping_add(1);
        let (history, request) =
            crate::history::History::recent(self.now.saturating_mul(1000), self.history_request);
        self.history = Some(history);
        self.hits.borrow_mut().clear();
        *self.hovered.borrow_mut() = None;
        vec![HostEffect::History(Box::new(request))]
    }

    fn open_history(&mut self, subject: Option<crate::journal::SubjectKey>) -> Vec<HostEffect> {
        self.history_request = self.history_request.wrapping_add(1);
        let filter = crate::journal::work_map::EventFilter {
            subject,
            ..Default::default()
        };
        let (history, request) = crate::history::History::new(filter, self.history_request);
        self.history = Some(history);
        self.hits.borrow_mut().clear();
        *self.hovered.borrow_mut() = None;
        vec![HostEffect::History(Box::new(request))]
    }

    fn history_key(&mut self, key: KeyEvent) -> Vec<HostEffect> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.quit = true;
            return vec![];
        }
        if key.code == KeyCode::Esc {
            if self
                .history
                .as_mut()
                .is_some_and(crate::history::History::dismiss_help)
            {
                return vec![];
            }
            self.history = None;
            // Previous hit targets must not remain active until the next draw.
            self.hits.borrow_mut().clear();
            *self.hovered.borrow_mut() = None;
            return vec![];
        }
        self.history_request = self.history_request.wrapping_add(1);
        if let Some(history) = &mut self.history {
            history.set_now(self.now.saturating_mul(1000));
        }
        self.history
            .as_mut()
            .and_then(|history| history.key(key, self.history_request))
            .map(|request| HostEffect::History(Box::new(request)))
            .into_iter()
            .collect()
    }

    /// The panes `tab`, `h` and `l` move through on the current screen.
    /// On a screen's own panes or in settings, not inside an item page.
    fn on_top_level(&self) -> bool {
        match self.screen() {
            Some((p, s)) => !self.ready(p) || self.plugins[p].depth(s.id) == 0,
            None => true,
        }
    }

    /// Whether this screen has a selectable source row beneath its top tab.
    fn has_source_row(&self) -> bool {
        matches!(self.view, View::Screen(i) if self.group_screens(i).len() > 1)
    }

    fn above_body(&mut self) {
        if self.has_source_row() {
            self.enter_sources();
        } else {
            self.enter_bar();
        }
    }

    /// `↑` past a pane enters the source row when present, then the top bar.
    fn enter_bar(&mut self) {
        self.last_pane.insert(self.view, self.focus);
        self.sources = false;
        self.bar = true;
    }

    fn enter_sources(&mut self) {
        self.last_pane.insert(self.view, self.focus);
        self.bar = false;
        self.sources = true;
    }

    /// Down from the top bar visits the source row when one is shown.
    fn leave_bar(&mut self) {
        self.bar = false;
        if self.has_source_row() {
            self.sources = true;
            return;
        }
        self.leave_sources();
    }

    /// Back into the body, to the pane it had focused last.
    fn leave_sources(&mut self) {
        self.sources = false;
        let panes = self.panes();
        self.focus = self
            .last_pane
            .get(&self.view)
            .copied()
            .filter(|p| panes.contains(p))
            .unwrap_or(panes[0]);
    }

    fn sources_key(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => self.next_source(false),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => self.next_source(true),
            KeyCode::Up | KeyCode::Char('k') => self.enter_bar(),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter | KeyCode::Esc => {
                self.leave_sources()
            }
            KeyCode::Char('x' | '?') => self.open_keys_menu(),
            KeyCode::Char(c)
                if c.is_ascii_uppercase()
                    || c.is_ascii_digit()
                    || matches!(c, SETTINGS_KEY | 'q' | 'w') =>
            {
                self.sources = false;
                return None;
            }
            _ => {}
        }
        Some(vec![])
    }

    fn next_source(&mut self, forward: bool) {
        let View::Screen(current) = self.view else {
            return;
        };
        let sources = self.group_screens(current);
        let Some(at) = sources.iter().position(|i| *i == current) else {
            return;
        };
        let to = if forward {
            (at + 1) % sources.len()
        } else {
            (at + sources.len() - 1) % sources.len()
        };
        self.view = View::Screen(sources[to]);
        self.back.clear();
        self.tabs = false;
    }

    /// A key while the top bar has focus; `None` to go on as usual (the
    /// bar let go of the keys).
    fn bar_key(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        match key.code {
            KeyCode::Left | KeyCode::Char('h') => self.next_screen(false),
            KeyCode::Right | KeyCode::Char('l') => self.next_screen(true),
            KeyCode::Down
            | KeyCode::Char('j')
            | KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Tab
            | KeyCode::BackTab => self.leave_bar(),
            KeyCode::Char('x' | '?') => self.open_keys_menu(),
            // The host's own keys work as anywhere (a screen's letter,
            // settings, quit, a pane by number); they leave the bar.
            KeyCode::Char(c)
                if c.is_ascii_uppercase()
                    || c.is_ascii_digit()
                    || matches!(c, SETTINGS_KEY | 'q' | 'w') =>
            {
                self.bar = false;
                return None;
            }
            _ => {}
        }
        Some(vec![])
    }

    /// Move through the top bar's screens and settings.
    fn next_screen(&mut self, forward: bool) {
        let top = self.top_screens();
        let n = top.len() + 1;
        let at = match self.view {
            View::Screen(i) => top
                .iter()
                .position(|candidate| {
                    *candidate == i
                        || self
                            .screen_group(*candidate)
                            .zip(self.screen_group(i))
                            .is_some_and(|(a, b)| a.id == b.id)
                })
                .unwrap_or(0),
            View::Settings => n - 1,
        };
        let to = if forward {
            (at + 1) % n
        } else {
            (at + n - 1) % n
        };
        self.view = if to == n - 1 {
            View::Settings
        } else {
            View::Screen(top[to])
        };
        self.back.clear();
        self.tabs = false;
        self.sources = false;
        let panes = self.panes();
        self.focus = if forward {
            panes[0]
        } else {
            panes[panes.len() - 1]
        };
    }

    /// The tabs of the item page showing, when one with a tab row is open
    /// on the screen.
    pub(crate) fn page_tabs(&self) -> Option<crate::plugin::PageTabs> {
        let (p, s) = self.screen()?;
        if !self.ready(p) || self.plugins[p].depth(s.id) == 0 {
            return None;
        }
        self.plugins[p].tabs(s.id).filter(|t| !t.names.is_empty())
    }

    /// Show the page's tab beside the one showing, wrapping: `forward` the
    /// one to the right, landing on the page's first pane; else the one to
    /// the left, landing on its last.
    fn show_tab_beside(&mut self, forward: bool) -> Vec<HostEffect> {
        let Some(t) = self.page_tabs() else {
            return vec![];
        };
        let n = t.names.len();
        let to = if forward {
            (t.on + 1) % n
        } else {
            (t.on + n - 1) % n
        };
        let Some((p, spec)) = self.screen() else {
            return vec![];
        };
        let fx = self.plugins[p].show_tab(spec.id, to);
        let out = self.route(p, fx, 0);
        let panes = self.panes();
        self.focus = if forward {
            panes[0]
        } else {
            panes[panes.len() - 1]
        };
        out
    }

    /// A key while a page's tab row has them; `None` to go on as usual
    /// (the row let go of the keys).
    fn tabs_key(&mut self, key: KeyEvent) -> Option<Vec<HostEffect>> {
        match key.code {
            KeyCode::Left | KeyCode::Char('h' | '[') | KeyCode::BackTab => {
                return Some(self.show_tab_beside(false));
            }
            KeyCode::Right | KeyCode::Char('l' | ']') | KeyCode::Tab => {
                return Some(self.show_tab_beside(true));
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Enter | KeyCode::Esc => self.tabs = false,
            KeyCode::Char('x' | '?') => self.open_keys_menu(),
            // The host's own keys work as anywhere (a screen's letter,
            // settings, quit, a pane by number); they leave the row.
            KeyCode::Char(c)
                if c.is_ascii_uppercase()
                    || c.is_ascii_digit()
                    || matches!(c, SETTINGS_KEY | 'q' | 'w') =>
            {
                self.tabs = false;
                return None;
            }
            _ => {}
        }
        Some(vec![])
    }

    /// Digits follow the active surface's logical routing, including hidden panes.
    pub fn pane_jumps(&self) -> Vec<crate::view::chrome::PaneJump> {
        if self.setup.is_some() { return vec![]; }
        // Some owners measure responsive page routing while producing their view.
        // An unrelated raw view enumeration must not decide the displayed jumps.
        if let Some((owner, spec)) = self.screen().filter(|(owner, _)| self.ready(*owner)) {
            let (width, height) = self.frontend.room();
            let _ = self.screen_view_in(owner, spec.id, width, height);
        }
        self.panes().into_iter().zip('1'..='9')
            .map(|(pane, key)| crate::view::chrome::PaneJump { pane: pane.into(), key }).collect()
    }

    /// The panes of what is showing, left to right.
    pub fn panes(&self) -> Vec<Pane> {
        // Setup has one place for the keys: the card.
        if self.setup.is_some() && self.view == View::Settings {
            return vec![Pane::Middle];
        }
        let panes = match self.screen() {
            Some((p, _)) if !self.ready(p) => vec![Pane::Middle],
            Some((p, s)) => self.plugins[p].panes(s.id),
            None => vec![Pane::Nav, Pane::Middle, Pane::Detail],
        };
        if panes.is_empty() {
            vec![Pane::Middle]
        } else {
            panes
        }
    }
}

/// The pane a `pane:<id>` target names (`pane:nav`, `pane:middle`,
/// `pane:detail`): the host's own target for a pane's area, sent by a
/// renderer that lays the panes out itself.
fn pane_of_own_target(target: &str) -> Option<Pane> {
    match target.strip_prefix("pane:")? {
        "nav" => Some(Pane::Nav),
        "middle" => Some(Pane::Middle),
        "detail" => Some(Pane::Detail),
        _ => None,
    }
}

/// The config file `file` with radar's own tables (`owned`) set from `table`. A
/// table radar does not own (a part this build lacks, one added by hand) is
/// kept, and so are the comments and layout of every table that did not
/// change. A file that does not parse is written fresh, as before.
fn merged_config(file: &str, table: &toml::Table, owned: &[&str]) -> String {
    let fresh = || toml::to_string_pretty(table).unwrap_or_default();
    let (Ok(mut doc), Ok(old)) = (
        file.parse::<toml_edit::DocumentMut>(),
        toml::from_str::<toml::Table>(file),
    ) else {
        return fresh();
    };
    for &key in owned {
        match table.get(key) {
            None => {
                doc.remove(key);
            }
            Some(v) if old.get(key) == Some(v) => {}
            Some(v) => {
                let one = toml::Table::from_iter([(key.to_string(), v.clone())]);
                let Ok(part) = toml::to_string_pretty(&one)
                    .unwrap_or_default()
                    .parse::<toml_edit::DocumentMut>()
                else {
                    return fresh();
                };
                let Some(mut item) = part.get(key).cloned() else {
                    doc.remove(key);
                    continue;
                };
                // The comment above a changed table stays above it, and the
                // table stays where it was in the file.
                if let (Some(new), Some(was)) =
                    (item.as_table_mut(), doc.get(key).and_then(|i| i.as_table()))
                {
                    *new.decor_mut() = was.decor().clone();
                    if let Some(at) = was.position() {
                        new.set_position(at);
                    }
                }
                doc.insert(key, item);
            }
        }
    }
    doc.to_string()
}

// --- the chrome as view data -----------------------------------------------------

/// The icon before a key's words on the status line, for the host's own
/// keys.
fn hint_icon(action: &str) -> Option<crate::ui::icon::Icon> {
    use crate::ui::icon::Icon;
    Some(match action {
        "extra actions" => Icon::Menu,
        "quit" => Icon::Quit,
        "scroll" => Icon::Scroll,
        "bulk" => Icon::Bulk,
        "expand/collapse" => Icon::Fold,
        _ => return None,
    })
}

/// The labels of a status-line hint that each press a key: `PgUp/PgDn`
/// is two, `←→↑↓` four, `x` one.
pub(crate) fn hint_labels(keys: &str) -> Vec<&str> {
    if keys.chars().count() > 1 && keys.chars().all(|c| "←→↑↓".contains(c)) {
        keys.char_indices()
            .map(|(i, c)| &keys[i..i + c.len_utf8()])
            .collect()
    } else if keys.len() > 1 && keys.contains('/') {
        keys.split('/').collect()
    } else {
        vec![keys]
    }
}

/// The key a status line's key label stands for: `PgUp`, `⏎`, `esc`, `+`.
fn hint_key(label: &str) -> Option<KeyEvent> {
    if label.chars().count() == 1 {
        crate::plugin::parse_key(label)
    } else {
        crate::plugin::parse_key(&label.to_lowercase())
    }
}

/// Status-line hint `n` as view data: its labels, words, icon and each key
/// it presses with its target. A hint whose labels are not all keys
/// cannot be pressed.
pub(crate) fn chrome_hint(n: usize, h: &KeyHint<'_>) -> crate::view::chrome::Hint {
    use crate::view::chrome::{self as c, targets};
    let labels = hint_labels(h.keys);
    let keys: Vec<_> = labels
        .iter()
        .map(|l| hint_key(l).and_then(|k| crate::view_tui::chord_of(&k)))
        .collect();
    let keys_of = if keys.iter().all(Option::is_some) {
        labels
            .iter()
            .zip(keys.into_iter().flatten())
            .enumerate()
            .map(|(k, (label, chord))| c::HintKey {
                label: (*label).to_string(),
                chord,
                action: None,
                target: targets::hint_key(n, k),
            })
            .collect()
    } else {
        vec![]
    };
    c::Hint {
        mnemonic_at: crate::mnemonic::at(h.keys, h.action),
        mnemonic_shift: crate::mnemonic::at(h.keys, h.action).is_some() && crate::mnemonic::shifted(h.keys),
        keys: h.keys.to_string(),
        label: h.action.to_string(),
        icon: hint_icon(h.action),
        keys_of,
    }
}

impl Host {
    /// The chrome for a frame `width` × `height` cells big, as view data:
    /// what the Ratatui shell paints, a headless frontend returns and the
    /// Ink renderer receives. Bounded: at most [`crate::view::chrome::MOST`]
    /// of each list.
    pub fn chrome_view(&self, width: u16, height: u16) -> crate::view::ChromeView {
        // Measured in the user's icons and look (colour does not change a
        // width).
        let theme = Theme::build(
            &self.colours.cfg,
            crate::design::theme::Tier::Mono,
            &self.colours.declared,
        );
        self.chrome_view_with(width, height, &theme)
    }

    /// [`Host::chrome_view`] measured in `theme`: first what scrolls at this
    /// size (settings' detail, the open menu), so the status line's keys and
    /// the menu's reading position are the same for every frontend.
    pub fn chrome_view_with(&self, width: u16, height: u16, theme: &Theme) -> crate::view::ChromeView {
        use crate::view::chrome::{self as c, targets};
        let body = crate::shell::body_of(self, width, height);
        self.frontend.set_room(body.width, body.height);
        crate::shell::measure(self, theme, width, height);
        let mut out = c::ChromeView::new(width, height);
        out.notices_visible = self.notices_visible();
        out.notices = self.notices.view();
        if !out.notices_visible { out.notices.now_ms = 0; }
        out.notice_history = self.notice_history.as_ref().map(|s| self.notices.history_view(s.selected, s.scroll, s.detail_scroll));
        // The overlay takes the body: everything under the top bar.
        out.history = self
            .history
            .as_ref()
            .map(|h| h.view(width, height.saturating_sub(1)));
        out.keys_at = if self.tabs && self.page_tabs().is_some() {
            c::KeysAt::PageTabs
        } else if self.bar {
            c::KeysAt::Bar
        } else if self.sources {
            c::KeysAt::Sources
        } else {
            c::KeysAt::Body
        };
        out.top_bar = self.chrome_top_bar();
        if let View::Screen(index) = self.view {
            let sources = self.group_screens(index);
            if sources.len() > 1 {
                out.sources = sources
                    .into_iter()
                    .filter_map(|i| {
                        Some(c::SourceTab {
                            label: self.screen_group(i)?.source.to_string(),
                            active: self.view == View::Screen(i),
                            target: targets::source_tab(i),
                        })
                    })
                    .take(c::MOST)
                    .collect();
            }
        }
        out.screen_keys = self.claimed_capitals();
        out.status = self.chrome_status();
        out.pane_jumps = self.pane_jumps();
        out.write_mode = self.mode().into();
        // The services' marks as this theme's icons draw them.
        out.status.health = self.health_chips(theme.icons());
        out.menu = self.menu_view();
        if self.history.is_none() {
            out.setup = self.setup_chrome();
            out.tour = self.tour_card();
        }
        if self.view == View::Settings && self.history.is_none() && out.setup.is_none() {
            out.settings = Some(self.settings_nav());
        }
        out
    }

    pub(crate) fn chrome_top_bar(&self) -> crate::view::chrome::TopBar {
        use crate::view::Ink;
        use crate::view::chrome::{self as c, targets};
        use crate::ui::icon::Icon;
        let mut bar = c::TopBar {
            brand: "radar".into(),
            ..c::TopBar::default()
        };
        if let Some((p, s)) = self.screen().filter(|(p, _)| self.ready(*p)) {
            let plugin = &self.plugins[p];
            bar.busy = plugin.busy(s.id);
            // Paused is the host's, said once for every part.
            bar.words = match self.paused_words() {
                Some(text) => Some(c::Badge { text: text.into(), ink: Ink::Warn }),
                None => plugin.top_right(s.id).map(|text| c::Badge { text, ink: Ink::Muted }),
            };
        }
        bar.terminal = self.terminal.as_ref().map(|terminal| c::Badge {
            text: terminal.bar_words(),
            ink: if matches!(terminal.route, crate::terminal_host::Route::Failed(_)) {
                Ink::Warn
            } else {
                Ink::Muted
            },
        });
        let mode = self.mode();
        bar.mode = c::ModeChip {
            text: crate::mode::bar_words(self),
            ink: match mode {
                crate::writes::Mode::ReadOnly => Ink::Info,
                crate::writes::Mode::Write => Ink::AccentSecondary,
                crate::writes::Mode::Edit => Ink::Secondary,
            },
            strong: mode == crate::writes::Mode::ReadOnly,
            target: targets::mode(),
        };
        // Setup's top bar is `setup / <step>` (its chrome), not the tabs.
        if self.setup.is_some() {
            return bar;
        }
        for i in self.top_screens().into_iter().take(c::MOST) {
            let (p, s) = &self.screens[i];
            let group = self.screen_group(i);
            let active = self.history.is_none()
                && match self.view {
                    View::Screen(active) => {
                        active == i
                            || group
                                .zip(self.screen_group(active))
                                .is_some_and(|(a, b)| a.id == b.id)
                    }
                    View::Settings => false,
                };
            bar.tabs.push(c::BarTab {
                label: group.map_or(s.name, |g| g.name).to_string(),
                mnemonic: group.map_or(s.key, |g| g.key),
                icon: self.plugins[*p].icon(s.id),
                // Counts stay in status; only a warning or failure needs a dot.
                attention: group.map_or_else(|| vec![i], |_| self.group_screens(i)).into_iter().any(|index| {
                    let (owner, screen) = &self.screens[index];
                    self.ready(*owner)
                        && self.plugins[*owner].badge(screen.id)
                            .is_some_and(|(_, tone)| matches!(tone, crate::Tone::Warn | crate::Tone::Bad))
                }),
                active,
                target: targets::screen_tab(i),
            });
        }
        bar.tabs.push(c::BarTab {
            label: "history".into(),
            mnemonic: HISTORY_KEY,
            icon: Some(Icon::Log),
            attention: false,
            active: self.history.is_some(),
            target: targets::history_tab(),
        });
        bar.tabs.push(c::BarTab {
            label: "settings".into(),
            mnemonic: SETTINGS_KEY,
            icon: Some(Icon::Settings),
            attention: self.unsaved(),
            active: self.history.is_none() && self.view == View::Settings,
            target: targets::settings_tab(),
        });
        bar
    }

    /// The status line as view data. A painter asks for it after the body
    /// and any menu are measured, so what scrolls says so.
    pub(crate) fn chrome_status(&self) -> crate::view::chrome::StatusLine {
        use crate::view::Ink;
        use crate::view::chrome as c;
        // A part may make its selection valid only while it builds its view
        // (a tree synced to its rows there), and the chrome is built before the body is
        // painted, and Headless never paints: build the screen's view first
        // so the counts and row hints are for the selection this frame draws.
        if let Some((p, spec)) = self.screen()
            && self.ready(p)
        {
            let _ = self.screen_view(p, spec.id);
        }
        let counts = match self.screen() {
            Some((p, s)) if self.ready(p) => self.plugins[p].status(s.id),
            _ => vec![],
        };
        let least_needed: &[&str] =
            if self.menu.is_none() && self.view == View::Settings && self.settings_detail.more() {
                // Compact settings need their scrollable help discoverable.
                &["quit", "bulk", "expand/collapse", "navigate"]
            } else {
                &["navigate", "scroll", "quit", "expand/collapse"]
            };
        let hints = self
            .status_hints()
            .iter()
            .take(c::MOST)
            .enumerate()
            .map(|(n, h)| {
                let mut hint = chrome_hint(n, h);
                if self.notice_history.is_some() {
                    for key in &mut hint.keys_of {
                        if let Some(target) = Self::notice_history_key_target(crate::view_tui::key_of(key.chord)) { key.target = target; }
                    }
                } else if self.palette.is_some() {
                    for (k, key) in hint.keys_of.iter_mut().enumerate() {
                        key.target = crate::view::Target::new(format!("chrome:palette-key:{}:{n}:{k}", self.palette_generation));
                    }
                }
                hint
            })
            .collect();
        let scope = if self.setup.is_some() { "setup" } else { self.screen().map_or("settings", |(_, s)| s.name) };
        c::StatusLine {
            scope: scope.to_uppercase(),
            hints,
            least_needed: least_needed.iter().map(|s| s.to_string()).collect(),
            counts: counts
                .into_iter()
                .take(c::MOST)
                .map(|(text, tone)| c::Badge { text, ink: Ink::from(tone) })
                .collect(),
            message: None,
            health: self.health_chips(&crate::ui::icon::current()),
            attention: self.attention_chip(),
        }
    }

    /// The chip for the agent session that wants the user most and is out of
    /// sight: its name and what it wants, on that state's ground, after the
    /// dock key, which goes there; `+n` when others want the user too.
    fn attention_chip(&self) -> Option<crate::view::chrome::AttentionChip> {
        use crate::dock::Want;
        use crate::view::Ink;
        let wanting = self.dock_wanting();
        let (want, tab) = wanting.first()?;
        let more = if wanting.len() > 1 { format!(" +{}", wanting.len() - 1) } else { String::new() };
        let (icon, ink) = match want {
            Want::NeedsYou => (crate::ui::icon::Icon::Warning, Ink::Warn),
            Want::Failed => (crate::ui::icon::Icon::RunFailed, Ink::Bad),
            Want::Done => (crate::ui::icon::Icon::Done, Ink::Good),
        };
        Some(crate::view::chrome::AttentionChip {
            // A long name is cut, so the chip never pushes the keys off the line.
            text: format!("{} {}{more}", crate::ui::fit(&tab.title, crate::ui::width(&tab.title).min(24)), want.words()),
            icon: Some(icon),
            ink,
            keys: crate::dock::KEY.to_string(),
            target: crate::view::chrome::targets::attention(),
        })
    }

    /// The parts named on the startup splash ([`Plugin::on_splash`]), in
    /// the parts' order, and whether each is ready: set up, and, for one
    /// that syncs with a service, holding something to show (a cache
    /// restored, a sync behind it) or its first sync begun.
    /// What radar holds now, for the start line: the parts' own facts, then
    /// the host's (agents set up, a service signed out or not answering).
    pub fn start_facts(&self) -> Vec<(String, crate::start_lines::Value)> {
        use crate::start_lines::Value;
        use crate::view::chrome::HealthState;
        let mut out: Vec<(String, Value)> = Vec::new();
        for (p, plugin) in self.plugins.iter().enumerate() {
            if !self.ready(p) {
                continue;
            }
            out.extend(plugin.start_facts(self.now).into_iter().map(|(k, v)| (k.to_string(), v)));
        }
        let agents = (0..self.plugins.len())
            .filter(|&p| self.ready(p) && self.plugins[p].dock_tabs().is_some())
            .count();
        out.push(("agents.setup".into(), Value::Num(agents as i64)));
        for plugin in &self.plugins {
            let Some(health) = plugin.health() else { continue };
            let fact = match health.state(self.now) {
                HealthState::SignedOut => "auth.expired",
                HealthState::Failing => "sync.last_failed",
                _ => continue,
            };
            if !out.iter().any(|(k, _)| k == fact) {
                out.push((fact.into(), Value::Bool(true)));
                out.push(("service".into(), Value::Str(plugin.name().into())));
            }
        }
        out
    }

    pub fn splash_parts(&self, icons: &crate::ui::icon::Icons) -> Vec<crate::splash::Contact> {
        use crate::view::chrome::HealthState;
        (0..self.plugins.len())
            .filter(|&p| self.plugins[p].on_splash())
            .map(|p| {
                let plugin = &self.plugins[p];
                let health = plugin.health();
                let live = self.ready(p)
                    && health.as_ref().is_none_or(|h| {
                        !matches!(h.state(self.now), HealthState::Failing | HealthState::SignedOut)
                    });
                // What its first screen's tab counts: things that need you.
                let count = plugin
                    .screens()
                    .first()
                    .and_then(|s| plugin.badge(s.id))
                    .and_then(|(n, _)| n.trim().parse::<u32>().ok())
                    .unwrap_or(0);
                crate::splash::Contact {
                    name: plugin.name().to_string(),
                    mark: health.map(|h| h.mark.in_pack(icons)).unwrap_or_default(),
                    tail: (count > 0).then(|| format!("{} for you", count.min(999))),
                    count,
                    live,
                }
            })
            .collect()
    }

    /// One chip per ready part that syncs with a service, in the parts'
    /// order, each its service's mark as `icons` draws marks, in the colour
    /// of its state at the host's clock.
    fn health_chips(&self, icons: &crate::ui::icon::Icons) -> Vec<crate::view::chrome::HealthChip> {
        use crate::view::chrome::{self as c, targets};
        (0..self.plugins.len())
            .filter(|p| self.ready(*p))
            .filter_map(|p| {
                let plugin = &self.plugins[p];
                let health = plugin.health()?;
                let state = health.state(self.now);
                Some(c::HealthChip {
                    part: plugin.id().into(),
                    label: plugin.id().into(),
                    glyph: health.mark.in_pack(icons),
                    state,
                    ink: crate::health::ink(state),
                    count: health.count(self.now),
                    target: targets::health(plugin.id()),
                })
            })
            .take(c::MOST)
            .collect()
    }

    /// A part's health menu: `refresh now`, then each source, its state
    /// and when.
    pub(crate) fn open_health_menu(&mut self, p: usize) {
        let Some(health) = self.plugins[p].health() else { return };
        let plugin = &self.plugins[p];
        let title = format!("{} · {}", plugin.name(), crate::health::words(&health, self.now));
        let id = format!("{}:{}", crate::health::MENU, plugin.id());
        let items = crate::health::menu_items(&health, self.now);
        self.menu = Some(OpenMenu {
            owner: Owner::Host,
            menu: Menu::new(id, title, items).icon(crate::ui::icon::Icon::Connection),
            row: 0,
            input: None,
            refused: None,
            viewport: Default::default(),
            anchor: None,
        });
    }

    /// The status line's keys, in the order shown.
    fn status_hints(&self) -> Vec<KeyHint<'static>> {
        if self.notice_history.is_some() { return vec![KeyHint::new("↑↓", "move"), KeyHint::new("PgUp/PgDn", "details"), KeyHint::new("esc", "back")]; }
        // While a drag selects: how to select with the terminal instead.
        if let Some(key) = self.selecting {
            return vec![KeyHint::new(key, "terminal's own selection")];
        }
        if let Some(open) = &self.menu {
            // Moving only where there is somewhere to move to.
            let choices = open
                .menu
                .items
                .iter()
                .filter(|i| !i.heading && !i.note)
                .count();
            let typed = open.input.is_some();
            let mut h = Vec::new();
            if choices > 1 {
                // A question's buttons sit in a row; in a field j and k are
                // letters.
                h.push(KeyHint::new(
                    if open.menu.question {
                        "←→"
                    } else {
                        "↑↓"
                    },
                    "move",
                ));
            }
            if open.viewport.more() {
                h.push(KeyHint::new("PgUp/PgDn", "scroll"));
            }
            // A field: how to finish it.
            if typed {
                h.push(KeyHint::new("⏎", if self.palette.is_some() { "choose" } else { "save" }));
                h.push(KeyHint::new("esc", "cancel"));
                return h;
            }
            if choices > 0 && open.viewport.usable.get() {
                h.push(KeyHint::new("⏎", "choose"));
            }
            h.push(KeyHint::new("esc", "close"));
            return h;
        }
        // While typing: how to finish.
        if self.colours.capturing() {
            return vec![KeyHint::new("⏎", "keep"), KeyHint::new("esc", "cancel")];
        }
        match self.screen() {
            // A hidden editor on a set-up page is not a Host key target.
            Some((p, spec)) if self.ready(p) && self.plugins[p].capturing() => {
                return self.plugins[p].hints(spec.id, self.focus);
            }
            None => {
                if let Some((Owner::Plugin(p), s)) = self.current_section()
                    && self.plugins[p].capturing()
                {
                    return self.plugins[p].section_hints(s.id);
                }
            }
            _ => {}
        }
        if let Some(h) = self.setup_hints() {
            return h;
        }
        // A page's tab row has the keys: moving between its tabs and back in.
        if self.tabs && self.page_tabs().is_some() {
            return vec![
                KeyHint::new("←→", "tabs"),
                KeyHint::new("↓", "enter"),
                KeyHint::new("q", "quit"),
                KeyHint::new("x", "extra actions"),
            ];
        }
        // The top bar has the keys: moving between screens and back in.
        if self.bar {
            return vec![
                KeyHint::new("←→", "screens"),
                KeyHint::new("↓", "enter"),
                KeyHint::new("q", "quit"),
                KeyHint::new("x", "extra actions"),
            ];
        }
        if self.sources {
            return vec![
                KeyHint::new("←→", "sources"),
                KeyHint::new("↑", "screens"),
                KeyHint::new("↓", "enter"),
                KeyHint::new("q", "quit"),
                KeyHint::new("x", "extra actions"),
            ];
        }
        // Otherwise lazydocker's bar: the same few keys everywhere; the rest
        // are in the x menu.
        let mut h = Vec::new();
        if let Some((p, spec)) = self.screen()
            && self.ready(p)
        {
            h.extend(self.plugins[p].row_hints(spec.id, self.focus));
        }
        if (self.screen().is_some() && self.setting_up().is_none())
            || (self.view == View::Settings && self.settings_detail.more())
        {
            h.push(KeyHint::new("PgUp/PgDn", "scroll"));
        }
        if self.bulk().is_some() {
            h.push(KeyHint::new("b", "bulk"));
        }
        if self.folds_here() {
            h.push(KeyHint::new("+/-", "expand/collapse"));
        }
        h.extend([
            KeyHint::new("q", "quit"),
            KeyHint::new("x", "extra actions"),
            KeyHint::new("~", "notices"),
            KeyHint::new("←→↑↓", "navigate"),
        ]);
        h
    }

    /// The menu or question open, if one is, as view data.
    pub fn menu_view(&self) -> Option<crate::view::MenuView> {
        self.menu.as_ref().map(|open| {
            let mut view = menu_view_of(open);
            view.palette = self.palette_view().map(Box::new);
            if view.palette.is_some() {
                for item in &mut view.items {
                    item.target = Some(crate::view::Target::new(format!("chrome:menu:{}", item.value)));
                }
            }
            view
        })
    }
}

/// An open menu as view data.
pub(crate) fn menu_view_of(open: &OpenMenu) -> crate::view::MenuView {
    use crate::view::chrome::{self as c, targets};
    let m = &open.menu;
    {
        c::MenuView {
            palette: None,
            id: m.id.clone(),
            title: m.title.clone(),
            icon: m.icon,
            question: m.question,
            change: m.change.map(crate::view::Change::from),
            items: m
                .items
                .iter()
                .take(c::MOST)
                .enumerate()
                .map(|(i, it)| c::Choice {
                    mnemonic_at: (!it.heading && !it.note).then(|| crate::mnemonic::at(&it.keys, crate::mnemonic::choice_words(m.question, &it.label, &it.hint))).flatten(),
                    mnemonic_shift: !it.heading && !it.note && crate::mnemonic::at(&it.keys, crate::mnemonic::choice_words(m.question, &it.label, &it.hint)).is_some() && crate::mnemonic::shifted(&it.keys),
                    value: it.value.clone(),
                    label: it.label.clone(),
                    hint: it.hint.clone(),
                    key: it.key,
                    keys: it.keys.clone(),
                    icon: it.icon,
                    heading: it.heading,
                    note: it.note,
                    shown_only: it.heading || it.note,
                    muted: it.muted,
                    sample: it.sample.clone(),
                    target: (!it.heading && !it.note).then(|| targets::menu_item(i)),
                })
                .collect(),
            selected: Some(open.row),
            typed: open.input.as_ref().map(|i| i.value()),
            caret: open.input.as_ref().map(|i| i.cursor()),
            at: open.anchor.map(|p| c::CellAt { column: p.x, row: p.y }),
            scroll: open.viewport.at.get(),
            lines: open.viewport.total.get(),
            shown: open.viewport.height.get(),
            usable: open.viewport.usable.get(),
            refused: open.refused.clone(),
        }
    }
}

#[cfg(test)]
mod mnemonic_projection_tests {
    use super::*;

    #[test]
    fn host_projects_effective_words_display_only_keys_and_shift_without_changing_routes() {
        let hint = chrome_hint(3, &KeyHint::new("A", "ask an agent"));
        assert_eq!(hint.mnemonic_at, Some(0));
        assert!(hint.mnemonic_shift);
        assert_eq!(hint.keys_of[0].chord, crate::view::Chord::char('A'));
        let rows = vec![
            MenuItem::pick("refresh", "later", "refresh-id").shown_as("r"),
            MenuItem::new('Y', "copy item key", "", "copy-id"),
            MenuItem::heading("read"),
            MenuItem::note("recorded text"),
        ];
        let menu = menu_view_of(&OpenMenu::open(Owner::Host, Menu::new("watch", "actions", rows)));
        assert_eq!(menu.items[0].key, None);
        assert_eq!(menu.items[0].mnemonic_at, Some(0));
        assert_eq!(menu.items[0].value, "refresh-id");
        assert_eq!(menu.items[1].mnemonic_at, Some(3));
        assert!(menu.items[1].mnemonic_shift);
        assert!(menu.items[2..].iter().all(|i| i.mnemonic_at.is_none()));
        let question = menu_view_of(&OpenMenu::open(Owner::Host, Menu::confirm("confirm", "post?", "post it", "keep it")));
        assert_eq!(question.items[0].hint, "yes, post it");
        assert_eq!(question.items[1].hint, "no, keep it");
        assert_eq!(question.items[0].mnemonic_at, Some(0));
        assert_eq!(question.items[1].mnemonic_at, Some(0));
        assert_eq!(question.items[0].value, "yes");
        assert_eq!(question.items[1].value, "no");
    }
}

impl Host {
    /// Settings navigation as view data: the groups, their sections, the
    /// row the cursor is on and the panes' titles.
    fn settings_nav(&self) -> crate::view::chrome::SettingsNav {
        use crate::ui::icon::Icon;
        use crate::view::chrome::{self as c, targets};
        let groups = self
            .nav_groups()
            .into_iter()
            .take(c::MOST)
            .map(|(g, sections)| c::NavGroup {
                id: g.to_string(),
                name: self.group_name(g),
                open: !self.folded.contains(g),
                target: targets::nav_group(g),
                sections: sections
                    .into_iter()
                    .take(c::MOST)
                    .map(|i| {
                        let (owner, spec) = self.sections[i];
                        let icon = match owner {
                            Owner::Parts => Some(Icon::Parts),
                            Owner::Mode => Some(self.mode().icon()),
                            Owner::Look => Some(Icon::Colours),
                            Owner::Host if spec.id == crate::icons::ICONS_SECTION.id => {
                                Some(Icon::Icons)
                            }
                            Owner::Host => Some(Icon::Colours),
                            Owner::Plugin(p) => self.plugins[p].section_icon(spec.id),
                        };
                        c::NavSection {
                            id: spec.id.to_string(),
                            name: spec.name.to_string(),
                            icon,
                            target: targets::nav_section(i),
                        }
                    })
                    .collect(),
            })
            .collect();
        let selected = match self.nav_at() {
            (_, Some(i)) => targets::nav_section(i),
            (g, None) => targets::nav_group(g),
        };
        let (section_title, detail_title) = match self.current_section() {
            Some((Owner::Plugin(p), s)) => (
                self.section_title(self.section),
                self.section_view(p, s.id)
                    .map_or_else(|| "about".into(), |v| v.detail_title),
            ),
            Some((Owner::Host, s)) if self.on_icons() => {
                (s.name.to_string(), self.colours.icons_detail_title())
            }
            Some((Owner::Host, s)) => (s.name.to_string(), self.colours.detail_title()),
            Some((Owner::Parts, s)) => (s.name.to_string(), crate::parts::detail_title(self)),
            Some((Owner::Mode, s)) => (s.name.to_string(), "the modes".into()),
            Some((Owner::Look, s)) => (s.name.to_string(), self.look_settings_view().detail_title),
            None => ("nothing to set".into(), "about".into()),
        };
        c::SettingsNav {
            groups,
            selected: Some(selected),
            section_title,
            detail_title,
            unsaved: self.unsaved(),
        }
    }

    /// What a chrome target stands for, as the pointer map names it.
    pub(crate) fn chrome_hit(&self, target: &crate::view::Target) -> Option<Hit> {
        let rest = target.as_str().strip_prefix(crate::view::chrome::targets::PREFIX)?;
        let (kind, arg) = rest.split_once(':').unwrap_or((rest, ""));
        let index = || arg.parse::<usize>().ok();
        Some(match kind {
            "notices" if self.notice_history.is_some() => Hit::NoticeHistoryKey(Self::notice_history_target_key(arg)?),
            "history" => Hit::HistoryTab,
            "settings" => Hit::Tab(View::Settings),
            "mode" => Hit::Mode,
            "health" => Hit::Health(self.plugins.iter().position(|p| p.id() == arg)?),
            "attention" => Hit::Attention,
            "tab" => Hit::Tab(View::Screen(index().filter(|i| *i < self.screens.len())?)),
            "source" => Hit::SourceTab(View::Screen(index().filter(|i| *i < self.screens.len())?)),
            "menu" if arg.starts_with("palette:") => Hit::MenuItem(self.palette_row(arg)?),
            "menu" if self.palette.is_none() => Hit::MenuItem(index()?),
            "section" => Hit::Section(index().filter(|i| *i < self.sections.len())?),
            "group" => Hit::Group(self.nav_groups().into_iter().find(|(g, _)| *g == arg)?.0),
            "setup" if self.setup.is_some() => Hit::SetupStop(index()?),
            "palette-key" | "key" => {
                let arg = if kind == "palette-key" {
                    let (generation, arg) = arg.split_once(':')?;
                    if self.palette.is_none() || generation.parse::<u64>().ok()? != self.palette_generation {
                        return None;
                    }
                    arg
                } else {
                    if self.palette.is_some() { return None; }
                    arg
                };
                let (n, k) = arg.split_once(':')?;
                let (n, k) = (n.parse::<usize>().ok()?, k.parse::<usize>().ok()?);
                let hints = self.status_hints();
                let label = *hint_labels(hints.get(n)?.keys).get(k)?;
                Hit::Key(hint_key(label)?, n)
            }
            _ => return None,
        })
    }

    /// The chrome target a pointer hit stands for: the inverse of
    /// [`Host::chrome_hit`], so the Ratatui shell's clicks reach the host as
    /// the same [`crate::view::Intent::Activate`] Headless and Ink send.
    pub(crate) fn chrome_target_of(&self, hit: &Hit) -> Option<crate::view::Target> {
        use crate::view::chrome::targets;
        Some(match hit {
            Hit::HistoryTab => targets::history_tab(),
            Hit::NoticeHistoryOpen => crate::view::Target::new("chrome:notices:open"),
            Hit::NoticeHistory(id) => crate::view::Target::new(format!("chrome:notices:id:{id}")),
            Hit::NoticeHistoryKey(key) => Self::notice_history_key_target(*key)?,
            Hit::Tab(View::Settings) => targets::settings_tab(),
            Hit::Tab(View::Screen(i)) => targets::screen_tab(*i),
            Hit::SourceTab(View::Screen(i)) => targets::source_tab(*i),
            Hit::Section(i) => targets::nav_section(*i),
            Hit::Group(g) => targets::nav_group(g),
            Hit::Mode => targets::mode(),
            Hit::Health(p) => targets::health(self.plugins.get(*p)?.id()),
            Hit::Attention => targets::attention(),
            Hit::MenuItem(i) if self.palette.is_some() => {
                let item = self.menu.as_ref()?.menu.items.get(*i)?;
                crate::view::Target::new(format!("chrome:menu:{}", item.value))
            }
            Hit::MenuItem(i) => targets::menu_item(*i),
            Hit::SetupStop(i) => targets::setup_stop(*i),
            Hit::Key(key, n) => {
                let hints = self.status_hints();
                let k = hint_labels(hints.get(*n)?.keys)
                    .iter()
                    .position(|l| hint_key(l) == Some(*key))?;
                if self.palette.is_some() {
                    crate::view::Target::new(format!("chrome:palette-key:{}:{n}:{k}", self.palette_generation))
                } else {
                    targets::hint_key(*n, k)
                }
            }
            _ => return None,
        })
    }

    /// A chrome target chosen without a pointer (a headless frontend, the
    /// Ink renderer) or with one (the Ratatui shell, through
    /// [`Host::chrome_target_of`]): what a click on it does. `activate`
    /// false only moves a menu's highlight.
    fn chrome_intent(&mut self, target: &crate::view::Target, activate: bool) -> Vec<HostEffect> {
        if target.as_str() == "chrome:notices:open" {
            if activate && self.notice_history_can_open() { self.open_notice_history(); }
            return vec![];
        }
        // The history overlay's rows and keys are its own: a row selects, a
        // key does what pressing it does.
        if target.as_str().starts_with(crate::history::targets::PREFIX) {
            let key = self.history.as_mut().and_then(|h| h.target(target));
            return match key {
                Some(key) if activate => self.history_key(key),
                _ => vec![],
            };
        }
        let Some(hit) = self.chrome_hit(target) else {
            return vec![];
        };
        // Over the history overlay a tab leaves it for its view; the
        // history tab is the one showing.
        if self.history.is_some() && matches!(hit, Hit::Tab(_) | Hit::HistoryTab) {
            if let (Hit::Tab(view), true) = (hit, activate) {
                self.history = None;
                self.view = view;
                self.bar = false;
                self.sources = false;
                self.back.clear();
                self.hits.borrow_mut().clear();
            }
            return vec![];
        }
        if let Hit::Key(k, _) = hit {
            return if activate { self.on_key(k) } else { vec![] };
        }
        if let Some(open) = self.menu.as_mut() {
            let Hit::MenuItem(i) = hit else { return vec![] };
            if open.menu.items.get(i).is_none_or(|it| it.heading || it.note) {
                return vec![];
            }
            open.row = i;
            return if activate {
                self.menu_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            } else {
                vec![]
            };
        }
        if !activate || self.capturing() {
            return vec![];
        }
        // Setup draws no tabs and no settings nav: none of theirs act.
        if self.setup.is_some()
            && matches!(hit, Hit::Tab(_) | Hit::HistoryTab | Hit::SourceTab(_) | Hit::Section(_) | Hit::Group(_))
        {
            return vec![];
        }
        self.bar = false;
        self.sources = false;
        self.tabs = false;
        match hit {
            Hit::Section(_) | Hit::Group(_) => self.focus = Pane::Nav,
            Hit::Mode => self.focus = Pane::Middle,
            _ => {}
        }
        if self.view == View::Settings && self.focus == Pane::Middle {
            self.settings_detail.top();
        }
        self.chrome_press(hit).unwrap_or_default()
    }

    /// A press on one of the chrome's own targets (a tab, the mode, a
    /// settings row), from the pointer or a [`Host::chrome_intent`].
    fn chrome_press(&mut self, hit: Hit) -> Option<Vec<HostEffect>> {
        Some(match hit {
            Hit::HistoryTab => self.open_recent_history(),
            // The screen showing: its tab takes the keys, as `↑` from the
            // top of a pane gives them to it.
            Hit::Tab(v)
                if v == self.view
                    || match (v, self.view) {
                        (View::Screen(a), View::Screen(b)) => self
                            .screen_group(a)
                            .zip(self.screen_group(b))
                            .is_some_and(|(a, b)| a.id == b.id),
                        _ => false,
                    } =>
            {
                self.enter_bar();
                vec![]
            }
            Hit::Tab(v) => {
                self.view = v;
                self.back.clear();
                self.tabs = false;
                vec![]
            }
            Hit::SourceTab(v) => {
                self.view = v;
                self.back.clear();
                self.tabs = false;
                self.bar = false;
                self.sources = true;
                self.focus = Pane::Nav;
                vec![]
            }
            Hit::Section(i) => {
                self.view = View::Settings;
                self.select_section(i);
                vec![]
            }
            Hit::Group(g) => {
                self.view = View::Settings;
                self.toggle_group(g);
                vec![]
            }
            Hit::Mode => {
                self.pause.row = 0;
                self.open_mode_menu();
                vec![]
            }
            Hit::Health(p) => {
                self.open_health_menu(p);
                vec![]
            }
            Hit::Attention => self.go_to_attention().unwrap_or_default(),
            Hit::SetupStop(i) => {
                self.setup_stop(i);
                vec![]
            }
            _ => return None,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::design::theme::Slot;
    use crate::plugin::{Outcome, Pane};
    use crate::ui::icon::Icon;
    use ratatui::style::Style;
    use serde_json::json;

    use std::sync::{Arc, Mutex};

    mod tab_attention {
        use super::*;
        use crate::Tone;

        struct Part {
            id: &'static str,
            key: char,
            tone: Tone,
            ready: bool,
            grouped: bool,
        }

        impl Plugin for Part {
            fn id(&self) -> &'static str { self.id }
            fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> { Ok(()) }
            fn config(&self) -> Option<toml::Value> { None }
            fn screens(&self) -> Vec<ScreenSpec> {
                vec![ScreenSpec { id: "main", name: self.id, key: self.key }]
            }
            fn screen_group(&self, _: &str) -> Option<crate::plugin::ScreenGroup> {
                self.grouped.then_some(crate::plugin::ScreenGroup {
                    id: "watch", name: "Watch", key: 'W', source: self.id,
                })
            }
            fn set_up(&self) -> crate::compose::Setup {
                if self.ready { crate::compose::Setup::Ready } else { crate::compose::Setup::Needed("connect".into()) }
            }
            fn badge(&self, _: &str) -> Option<(String, Tone)> { Some(("42".into(), self.tone)) }
            fn status(&self, _: &str) -> Vec<(String, Tone)> { vec![("42 total".into(), Tone::Info)] }
        }

        fn part(id: &'static str, key: char, tone: Tone, ready: bool, grouped: bool) -> Box<dyn Plugin> {
            Box::new(Part { id, key, tone, ready, grouped })
        }

        #[test]
        fn host_owns_warning_dots_without_rewriting_status() {
            let tones = [Tone::Plain, Tone::Muted, Tone::Accent, Tone::Info, Tone::Good, Tone::Warn, Tone::Bad];
            let names = ["ankh", "borogravia", "chirm", "djelibeybi", "ephebe", "fourecks", "genua"];
            let mut host = Host::new(tones.into_iter().zip(names).zip('A'..='G')
                .map(|((tone, name), key)| part(name, key, tone, true, false)).collect(), 0).unwrap();
            let bar = host.chrome_top_bar();
            assert_eq!(bar.tabs.iter().map(|tab| tab.attention).collect::<Vec<_>>(),
                [false, false, false, false, false, true, true, false, false]);
            assert_eq!(bar.tabs[..7].iter().map(|tab| tab.label.as_str()).collect::<Vec<_>>(), names);
            assert!(host.chrome_status().counts.iter().any(|count| count.text == "42 total"));
            let wire = serde_json::to_value(&bar).unwrap();
            assert!(wire["tabs"].as_array().unwrap().iter().all(|tab| tab.get("badge").is_none()));
            host.save_failed = Some("storage unavailable".into());
            assert!(host.chrome_top_bar().tabs.last().unwrap().attention);
            host.saved = host.config_table();
            assert!(!host.chrome_top_bar().tabs.last().unwrap().attention);
        }

        #[test]
        fn grouped_tabs_include_ready_siblings_only() {
            let mut host = Host::new(vec![
                part("ankh", 'A', Tone::Info, true, true),
                part("lancre", 'L', Tone::Warn, true, true),
                part("sto_lat", 'S', Tone::Bad, false, true),
            ], 0).unwrap();
            let bar = host.chrome_top_bar();
            assert_eq!(bar.tabs.len(), 3, "one group plus history and settings");
            assert_eq!(bar.tabs[0].label, "Watch");
            assert!(bar.tabs[0].attention, "a ready sibling needs attention");
            host.plugins[1] = part("lancre", 'L', Tone::Good, true, true);
            assert!(!host.chrome_top_bar().tabs[0].attention, "an unconfigured sibling has no dot");
        }
    }

    /// What a probe plugin was given.
    #[derive(Default, Debug)]
    pub struct Log {
        pub value: i64,
        pub bus: Vec<String>,
        pub keys: Vec<char>,
        /// When the user was last seen, as `input` was told.
        pub inputs: Vec<i64>,
        /// Typing into a field of its section (`t` starts, `⏎` keeps).
        pub typing: bool,
        pub overlay: bool,
        /// Pages open on its screen (`p` opens one), and the page's tab
        /// showing (of three: the first has one pane, the others two).
        pub depth: usize,
        pub tab: usize,
        /// Its keys menu names a thing with an icon, and `i` opens a menu
        /// with icons and says so with one.
        pub icons: bool,
        /// It writes to a service: `v` adds (a note), `w` edits (approves);
        /// both are in its keys menu.
        pub writes: bool,
    }

    /// A plugin that records what it is given into a shared [`Log`].
    pub struct Probe {
        pub id: &'static str,
        pub key: char,
        pub section: &'static str,
        pub log: Arc<Mutex<Log>>,
    }

    impl Probe {
        fn with<T>(&self, f: impl FnOnce(&mut Log) -> T) -> T {
            f(&mut self.log.lock().unwrap())
        }
    }

    impl Plugin for Probe {
        fn id(&self) -> &'static str {
            self.id
        }
        fn configure(&mut self, t: Option<&toml::Value>) -> Result<()> {
            let v = t
                .and_then(|t| t.get("value"))
                .and_then(toml::Value::as_integer)
                .unwrap_or(0);
            self.with(|l| l.value = v);
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            let mut t = toml::Table::new();
            t.insert("value".into(), toml::Value::Integer(self.with(|l| l.value)));
            Some(toml::Value::Table(t))
        }
        fn screens(&self) -> Vec<ScreenSpec> {
            vec![ScreenSpec {
                id: "main",
                name: self.id,
                key: self.key,
            }]
        }
        fn sections(&self) -> Vec<SectionSpec> {
            vec![SectionSpec {
                id: "s",
                name: self.section,
                group: self.id,
            }]
        }
        fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
            self.with(|l| l.bus.push(format!("menu {menu} {value}")));
            vec![]
        }
        fn on_msg(&mut self, msg: Msg) -> Vec<Effect> {
            if msg.downcast_ref::<&str>() == Some(&"question") {
                vec![Effect::Menu(Menu::confirm("background-question", "proceed?", "yes", "no"))]
            } else { vec![] }
        }
        fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
            use crate::view as v;
            let panes = self.panes(scope).into_iter().map(|pane| v::PaneView::new(
                pane.into(), self.id, v::Node::List { rows: vec![], selected: None, editing: None },
            )).collect();
            let mut view = v::ScreenView::new(scope, panes);
            if self.with(|log| log.overlay) {
                view.overlay = Some(Box::new(v::PaneView::new(v::PaneId::Middle, "dialog", v::Node::Document { lines: vec![], cut: false })));
            }
            view.actions = self.keys(scope, Pane::Middle).iter()
                .map(|b| v::ActionSpec::named(format!("{}.{scope}.{}", self.id, b.keys), b))
                .collect();
            Some(view)
        }
        fn intent(&mut self, scope: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
            // Keys only: its actions invoked by id reach it through the
            // host's transitional id-to-key fallback.
            let crate::view::Intent::Key { chord } = intent else { return Outcome::Ignored };
            if scope == "s" {
                match chord.key {
                    crate::view::Key::Char('+') => self.with(|l| l.value += 1),
                    crate::view::Key::Char('t') => self.with(|l| l.typing = true),
                    crate::view::Key::Enter if self.with(|l| l.typing) => self.with(|l| l.typing = false),
                    _ => return Outcome::Ignored,
                }
                return Outcome::done();
            }
            let key = crate::view_tui::key_of(*chord);
            match key.code {
                KeyCode::Char('m') => Outcome::Handled(vec![Effect::Menu(Menu::confirm(
                    "ask", "sure?", "do it", "leave it",
                ))]),
                KeyCode::Char('n') => Outcome::Handled(vec![Effect::Menu(Menu::new(
                    "long",
                    "long",
                    (0..40)
                        .map(|i| MenuItem::pick(format!("item {i}"), "", i.to_string()))
                        .collect(),
                ))]),
                KeyCode::Char('i') if self.with(|l| l.icons) => Outcome::Handled(vec![
                    Effect::NotifyWith(Icon::RunFailed, "it broke".into()),
                    Effect::Menu(
                        Menu::new(
                            "icons",
                            "crash",
                            vec![
                                MenuItem::heading("where").icon(Icon::Repo),
                                MenuItem::new('o', "open", "opens it", "o").icon(Icon::Link),
                                MenuItem::new('c', "close", "closes it", "c"),
                            ],
                        )
                        .icon(Icon::Bug),
                    ),
                ]),
                KeyCode::Char('g') if self.with(|l| l.writes) => {
                    Outcome::Handled(vec![Effect::Menu(Menu::write_plain(
                        "approve",
                        "approve #12?",
                        vec!["as you".into()],
                        "approve it",
                        "leave it",
                    ))])
                }
                KeyCode::Char('v') if self.with(|l| l.writes) => {
                    Outcome::Handled(vec![Effect::Menu(Menu::write_confirm(
                        "note",
                        "post this on #12?",
                        "hello",
                        "post it",
                        "drop it",
                    ))])
                }
                KeyCode::Char('e') => Outcome::Handled(vec![Effect::Emit {
                    topic: "probe.x",
                    payload: json!({"from": self.id}),
                }]),
                KeyCode::Char(c @ ('a' | 'y')) => {
                    self.with(|l| l.keys.push(c));
                    Outcome::done()
                }
                KeyCode::Char('p') => {
                    self.with(|l| l.depth += 1);
                    Outcome::done()
                }
                KeyCode::Char('c') => {
                    let mut changes = crate::motion::Changes::default();
                    changes.changed.insert("probe:1".into());
                    Outcome::Handled(vec![
                        Effect::Changed(changes),
                        Effect::Changed(Default::default()),
                    ])
                }
                _ => Outcome::Ignored,
            }
        }
        fn depth(&self, _: &str) -> usize {
            self.with(|l| l.depth)
        }
        fn tabs(&self, _: &str) -> Option<crate::plugin::PageTabs> {
            (self.with(|l| l.depth) > 0).then(|| crate::plugin::PageTabs {
                names: vec![
                    "Overview".into(),
                    "Changes · 2 files".into(),
                    "Threads".into(),
                ],
                on: self.with(|l| l.tab),
            })
        }
        fn show_tab(&mut self, _: &str, i: usize) -> Vec<Effect> {
            self.with(|l| l.tab = i);
            vec![]
        }
        fn panes(&self, _: &str) -> Vec<Pane> {
            match self.with(|l| (l.depth, l.tab)) {
                (0, _) => vec![Pane::Nav, Pane::Middle, Pane::Detail],
                (_, 0) => vec![Pane::Middle],
                _ => vec![Pane::Nav, Pane::Middle],
            }
        }
        fn keys(&self, _: &str, _: Pane) -> Vec<Binding> {
            if self.with(|l| l.writes) {
                return vec![
                    Binding::heading("#12"),
                    Binding::new("v", "post a note on #12…").adds(),
                    Binding::new("g", "approve #12…").edits(),
                ];
            }
            if !self.with(|l| l.icons) {
                return vec![];
            }
            vec![
                Binding::heading("crash").icon(Icon::Bug),
                Binding::new("a", "act on it").icon(Icon::Link),
            ]
        }
        fn section_view(&self, section: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::SectionView> {
            Some(crate::view::SectionView {
                version: crate::view::VERSION,
                scope: section.into(),
                body: crate::view::Node::Document { lines: vec![], cut: false },
                detail_title: "about".into(),
                detail: vec![],
                detail_blocks: vec![],
                actions: vec![],
            })
        }
        fn capturing(&self) -> bool {
            self.with(|l| l.typing)
        }
        fn on_bus(&mut self, topic: &str, payload: &Value) -> Vec<Effect> {
            self.with(|l| l.bus.push(format!("{topic} {payload}")));
            vec![]
        }
        fn input(&mut self, now: i64) {
            self.with(|l| l.inputs.push(now));
        }
    }

    pub fn probe(
        id: &'static str,
        key: char,
        section: &'static str,
    ) -> (Box<dyn Plugin>, Arc<Mutex<Log>>) {
        let log = Arc::new(Mutex::new(Log::default()));
        (
            Box::new(Probe {
                id,
                key,
                section,
                log: log.clone(),
            }),
            log,
        )
    }

    fn host(specs: &[(&'static str, char, &'static str)]) -> (Host, Vec<Arc<Mutex<Log>>>) {
        let (plugins, logs) = specs.iter().map(|&(i, k, s)| probe(i, k, s)).unzip();
        (Host::new(plugins, 0).unwrap(), logs)
    }

    #[test]
    fn headless_audits_every_registered_legacy_surface() {
        struct MissingScreen;
        impl Plugin for MissingScreen {
            fn id(&self) -> &'static str { "legacy-screen" }
            fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> { Ok(()) }
            fn config(&self) -> Option<toml::Value> { None }
            fn screens(&self) -> Vec<ScreenSpec> {
                vec![ScreenSpec { id: "legacy-main", name: "legacy screen", key: 'L' }]
            }
        }
        struct MissingSection;
        impl Plugin for MissingSection {
            fn id(&self) -> &'static str { "legacy" }
            fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> { Ok(()) }
            fn config(&self) -> Option<toml::Value> { None }
            fn sections(&self) -> Vec<SectionSpec> {
                vec![SectionSpec { id: "legacy-settings", name: "legacy settings", group: "legacy" }]
            }
        }
        let legacy_host = || {
            Host::new(vec![probe("probe", 'P', "settings").0, Box::new(MissingScreen), Box::new(MissingSection)], 0).unwrap()
        };
        let gaps = crate::headless::Headless::new(legacy_host(), 80, 24).view_gaps();
        assert_eq!(gaps.len(), 2);
        assert_eq!((&gaps[0].plugin, &gaps[0].scope, gaps[0].section, &gaps[0].reason),
            (&"legacy-screen".to_string(), &"legacy-main".to_string(), false, &"missing".to_string()));
        assert_eq!((&gaps[1].plugin, &gaps[1].scope, gaps[1].section, &gaps[1].reason),
            (&"legacy".to_string(), &"legacy-settings".to_string(), true, &"missing".to_string()));
        assert_eq!(crate::headless::Headless::requiring_views(legacy_host(), 80, 24).err(), Some(gaps));
        let missing = Host::new(vec![Box::new(MissingScreen)], 0).unwrap();
        assert!(crate::shell::to_text(&missing, 80, 24).contains("no view for this screen"));
    }

    fn press(host: &mut Host, code: KeyCode) -> Vec<HostEffect> {
        host.update(Event::Key(KeyEvent::new(code, KeyModifiers::NONE)))
    }

    /// A part that asks for a shelf's name: two offers, `x` refused.
    #[derive(Default)]
    struct Asker {
        got: Arc<Mutex<Vec<String>>>,
    }

    impl Plugin for Asker {
        fn id(&self) -> &'static str {
            "asker"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> anyhow::Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn screens(&self) -> Vec<ScreenSpec> {
            vec![ScreenSpec {
                id: "s",
                name: "shelves",
                key: 'A',
            }]
        }
        fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
            use crate::view as v;
            Some(v::ScreenView::new(scope, vec![v::PaneView::new(
                v::PaneId::Middle, "shelves", v::Node::Document { lines: vec![], cut: false },
            )]))
        }
        fn intent(&mut self, _: &str, _: Pane, intent: &crate::view::Intent) -> Outcome {
            match intent {
                crate::view::Intent::Key { chord } if chord.key == crate::view::Key::Char('n') => Outcome::Handled(vec![Effect::Menu(Menu::typed(
                    "name",
                    "name the shelf",
                    "",
                ))]),
                _ => Outcome::Ignored,
            }
        }
        fn typing(&self, _: &str, text: &str) -> crate::Typing {
            let verdict = match text {
                "" => None,
                "x" => Some(crate::Verdict::Bad("x is taken".into())),
                t => Some(crate::Verdict::Good(format!("a shelf called {t}"))),
            };
            let offers = ["books", "boxes"]
                .into_iter()
                .filter(|o| o.starts_with(text))
                .map(|o| MenuItem::pick(o, "", o))
                .collect();
            crate::Typing { verdict, offers }
        }
        fn on_menu(&mut self, menu: &str, value: &str) -> Vec<Effect> {
            self.got.lock().unwrap().push(format!("{menu}={value}"));
            vec![]
        }
    }

    #[test]
    fn a_typed_menu_takes_every_key_says_what_it_makes_of_the_text_and_offers_choices() {
        let asker = Asker::default();
        let got = asker.got.clone();
        let mut host = Host::new(vec![Box::new(asker)], 0).unwrap();
        host.focus = Pane::Middle;
        let typed = |host: &mut Host, s: &str| {
            for c in s.chars() {
                press(host, KeyCode::Char(c));
            }
        };
        let open = |host: &mut Host| {
            press(host, KeyCode::Char('n'));
            assert!(host.menu.as_ref().is_some_and(|m| m.input.is_some()));
        };
        let labels = |host: &Host| -> Vec<String> {
            host.menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .map(|i| i.label.clone())
                .collect()
        };
        open(&mut host);
        assert_eq!(labels(&host), ["books", "boxes"]);
        // q and x are text here, not close and the menu key.
        typed(&mut host, "x");
        assert_eq!(labels(&host), ["✗ x is taken"]);
        press(&mut host, KeyCode::Enter);
        assert_eq!(host.message.as_deref(), Some("not yet: x is taken"));
        assert!(host.menu.is_some(), "a refusal keeps the field open");
        press(&mut host, KeyCode::Backspace);
        typed(&mut host, "bo");
        assert_eq!(labels(&host), ["✓ a shelf called bo", "books", "boxes"]);
        // ↓ puts the first offer in the field; ⏎ gives that text.
        press(&mut host, KeyCode::Down);
        assert_eq!(
            host.menu.as_ref().unwrap().input.as_ref().unwrap().value(),
            "books"
        );
        assert_eq!(labels(&host)[0], "✓ a shelf called books");
        press(&mut host, KeyCode::Enter);
        assert!(host.menu.is_none());
        // Pasted, then kept as typed.
        open(&mut host);
        host.update(Event::Paste("my shelf".into()));
        press(&mut host, KeyCode::Enter);
        // esc closes and gives nothing.
        open(&mut host);
        typed(&mut host, "q");
        press(&mut host, KeyCode::Esc);
        assert!(host.menu.is_none());
        assert_eq!(*got.lock().unwrap(), ["name=books", "name=my shelf"]);
        // The field is drawn where the dialog starts, the cursor in it.
        open(&mut host);
        typed(&mut host, "bo");
        let text = crate::shell::to_text(&host, 80, 24);
        assert!(
            text.contains("name the shelf") && text.contains("│ bo"),
            "{text}"
        );
        assert!(text.contains("✓ a shelf called bo"), "{text}");
    }

    #[test]
    fn folders_like_completes_a_typed_path_in_the_form_typed() {
        let dir = std::env::temp_dir().join(format!("radar-folders-{}", std::process::id()));
        for d in ["code", "Config", ".hidden", "other"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        std::fs::write(dir.join("cfile"), "").unwrap();
        let base = format!("{}/", dir.display());
        let got = crate::system::folders_like(&format!("{base}c"), 8);
        assert_eq!(got, [format!("{base}code/"), format!("{base}Config/")]);
        assert_eq!(
            crate::system::folders_like(&format!("{base}."), 8),
            [format!("{base}.hidden/")]
        );
        assert_eq!(crate::system::folders_like(&base, 2).len(), 2);
        assert!(crate::system::folders_like("relative", 8).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clashing_ids_keys_and_sections_are_refused() {
        let new = |specs: &[(&'static str, char, &'static str)]| {
            Host::new(specs.iter().map(|&(i, k, s)| probe(i, k, s).0).collect(), 0).is_ok()
        };
        assert!(!new(&[("a", 'A', "a"), ("a", 'B', "b")]), "same id");
        assert!(!new(&[("a", 'A', "a"), ("b", 'A', "b")]), "same screen key");
        assert!(
            new(&[("a", 'A', "x"), ("b", 'B', "x")]),
            "a name under each part's own heading"
        );
        assert!(
            !new(&[("general", 'A', "colours")]),
            "the same name under one heading"
        );
        assert!(!new(&[("a", SETTINGS_KEY, "a")]), "settings key");
        assert!(new(&[("a", 'S', "a")]), "S is a part's to take");
        assert!(!new(&[("a", 'a', "a")]), "not shift+letter");
        assert!(new(&[("a", 'A', "a"), ("b", 'B', "b")]));
    }

    #[test]
    fn keys_reach_the_screen_plugin_and_shift_letters_switch_screens() {
        let (mut host, logs) = host(&[("a", 'A', "a"), ("b", 'B', "b")]);
        press(&mut host, KeyCode::Char('a'));
        press(&mut host, KeyCode::Char('B'));
        press(&mut host, KeyCode::Char('y'));
        assert_eq!(logs[0].lock().unwrap().keys, vec!['a']);
        assert_eq!(logs[1].lock().unwrap().keys, vec!['y']);
    }

    #[test]
    fn bus_events_reach_every_other_plugin_only() {
        let (mut host, logs) = host(&[("a", 'A', "a"), ("b", 'B', "b")]);
        press(&mut host, KeyCode::Char('e'));
        assert!(logs[0].lock().unwrap().bus.is_empty());
        assert_eq!(logs[1].lock().unwrap().bus, vec![r#"probe.x {"from":"a"}"#]);
    }

    #[test]
    fn a_save_keeps_what_radar_does_not_own_and_the_comments_of_unchanged_tables() {
        let (mut host, _logs) = host(&[("a", 'A', "a"), ("b", 'B', "b")]);
        let file = "# my radar\n\n[a]\nvalue = 3\n\n# b stays as I wrote it\n[b]\nvalue = 0 # zero\n\n\
                    [gone]\nkept = true\n\n[gone.deep]\nx = 1\n";
        host.load_config(Some(file)).unwrap();
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        let fx = press(&mut host, KeyCode::Char('+'));
        let [HostEffect::Save(text)] = &fx[..] else { panic!("{fx:?}") };
        assert!(text.contains("value = 4"), "{text}");
        for kept in ["# my radar", "# b stays as I wrote it", "value = 0 # zero", "[gone]", "kept = true", "[gone.deep]"] {
            assert!(text.contains(kept), "lost {kept:?} in:\n{text}");
        }
        let back: toml::Table = toml::from_str(text).unwrap();
        assert_eq!(back["gone"]["deep"]["x"].as_integer(), Some(1));
        assert_eq!(back["a"]["value"].as_integer(), Some(4));
    }

    #[test]
    fn a_config_that_will_not_edit_is_written_fresh() {
        let table = toml::from_str::<toml::Table>("[a]\nvalue = 1\n").unwrap();
        assert_eq!(merged_config("", &table, &["a"]), "[a]\nvalue = 1\n");
        let text = merged_config("[a]\nvalue = 1\n[b]\nv = 2\n", &toml::Table::new(), &["a"]);
        assert_eq!(toml::from_str::<toml::Table>(&text).unwrap().len(), 1, "{text}");
    }

    #[test]
    fn config_is_one_table_per_plugin_and_edits_mark_it_dirty() {
        let (mut host, logs) = host(&[("a", 'A', "a"), ("b", 'B', "b")]);
        host.load_config(Some("[a]\nvalue = 3\n\n[b]\nvalue = 0\n"))
            .unwrap();
        assert_eq!(logs[0].lock().unwrap().value, 3);
        assert!(!host.dirty());
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        let fx = press(&mut host, KeyCode::Char('+'));
        assert_eq!(logs[0].lock().unwrap().value, 4);
        assert!(host.dirty(), "until the file is written");
        assert!(matches!(&fx[..], [HostEffect::Save(text)] if text.contains("value = 4")));
        host.update(Event::Saved(Ok(())));
        assert!(!host.dirty());
        // w still writes, harmlessly, and says so.
        let fx = press(&mut host, KeyCode::Char('w'));
        assert!(matches!(&fx[..], [HostEffect::Save(text)] if text.contains("value = 4")));
        host.update(Event::Saved(Ok(())));
        assert_eq!(host.message.as_deref(), Some("saved"));
    }

    #[test]
    fn two_parts_each_with_a_connection_title_it_with_their_names() {
        let (two, _) = host(&[("a", 'A', "connection"), ("b", 'B', "connection")]);
        let i = two
            .sections
            .iter()
            .position(|(o, _)| *o == Owner::Plugin(1))
            .unwrap();
        assert_eq!(
            two.section_title(i),
            "b connection",
            "the part's name, its id here"
        );
        let (one, _) = host(&[("a", 'A', "connection")]);
        let i = one
            .sections
            .iter()
            .position(|(o, _)| *o == Owner::Plugin(0))
            .unwrap();
        assert_eq!(one.section_title(i), "connection");
    }

    #[test]
    fn a_completed_change_is_saved_at_once_and_never_mid_typing() {
        let (mut host, logs) = host(&[("a", 'A', "a")]);
        host.load_config(Some("[a]\nvalue = 0\n")).unwrap();
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        assert!(
            host.update(Event::Tick(1)).is_empty(),
            "nothing changed, nothing written"
        );
        let fx = press(&mut host, KeyCode::Char('+'));
        assert!(matches!(&fx[..], [HostEffect::Save(t)] if t.contains("value = 1")));
        assert!(!host.unsaved(), "a save on its way is not a failure");
        host.message = Some("the part's word".into());
        host.update(Event::Saved(Ok(())));
        assert!(!host.dirty());
        assert_eq!(
            host.message.as_deref(),
            Some("the part's word"),
            "a save after a change says nothing"
        );

        // Typing: a change made while typing waits for ⏎.
        press(&mut host, KeyCode::Char('t'));
        assert!(
            press(&mut host, KeyCode::Char('+')).is_empty(),
            "not mid-typing"
        );
        assert!(
            host.update(Event::Tick(2)).is_empty(),
            "not on a tick mid-typing either"
        );
        let fx = press(&mut host, KeyCode::Enter);
        assert!(
            matches!(&fx[..], [HostEffect::Save(t)] if t.contains("value = 2")),
            "{fx:?}"
        );
        assert_eq!(logs[0].lock().unwrap().value, 2);
        assert!(
            press(&mut host, KeyCode::Char('j')).is_empty(),
            "moving changes nothing"
        );
    }

    #[test]
    fn a_failed_save_says_so_keeps_the_marker_and_q_asks_before_losing_it() {
        let fresh = || {
            let (mut h, _) = host(&[("a", 'A', "a")]);
            h.load_config(Some("[a]\nvalue = 0\n")).unwrap();
            h
        };
        let mut host = fresh();
        press(&mut host, KeyCode::Char('q'));
        assert!(host.quit, "nothing unsaved: q quits");

        // A change saved as it was made: q quits without asking.
        let mut host = fresh();
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        press(&mut host, KeyCode::Char('+'));
        host.update(Event::Saved(Ok(())));
        press(&mut host, KeyCode::Char('q'));
        assert!(host.quit && host.menu.is_none(), "saved: no question");

        let mut host = fresh();
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        press(&mut host, KeyCode::Char('+'));
        host.update(Event::Saved(Err("disk full".into())));
        assert!(
            host.message
                .as_deref()
                .is_some_and(|m| m.contains("disk full"))
        );
        assert!(host.unsaved());
        let screen = crate::shell::to_text(&host, 120, 20);
        assert!(
            screen.contains(&format!("Settings (,) {}", crate::ui::icon::ATTENTION_DOT)) && screen.contains("unsaved · w retries"),
            "{screen}"
        );
        press(&mut host, KeyCode::Char('q'));
        assert!(!host.quit && host.menu.is_some(), "unsaved: q asks");
        press(&mut host, KeyCode::Char('s'));
        assert!(!host.quit && host.menu.is_none(), "stay");

        press(&mut host, KeyCode::Char('q'));
        let fx = press(&mut host, KeyCode::Char('w'));
        assert!(matches!(&fx[..], [HostEffect::Save(_)]));
        assert!(!host.quit, "not before the file is written");
        host.update(Event::Saved(Err("disk full".into())));
        assert!(!host.quit, "a failed save keeps you in");
        press(&mut host, KeyCode::Char('q'));
        press(&mut host, KeyCode::Char('w'));
        host.update(Event::Saved(Ok(())));
        assert!(host.quit);
        assert!(!host.unsaved());

        let mut host = fresh();
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        press(&mut host, KeyCode::Char('+'));
        host.update(Event::Saved(Err("disk full".into())));
        press(&mut host, KeyCode::Char('q'));
        press(&mut host, KeyCode::Char('d'));
        assert!(host.quit, "discard changes and quit");
    }

    #[test]
    fn no_config_file_means_first_run_on_settings_and_unsaved() {
        let (mut host, _) = host(&[("a", 'A', "a")]);
        host.load_config(None).unwrap();
        assert_eq!(host.view, View::Settings);
        assert!(host.dirty());
    }

    #[test]
    fn colours_are_changed_through_menus_and_saved_under_theme() {
        let (mut host, _) = host(&[("a", 'A', "a")]);
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        host.section = host
            .sections
            .iter()
            .position(|(o, _)| *o == Owner::Host)
            .unwrap();
        host.focus = Pane::Middle;
        // The motion and the icon pack are chosen in the look section,
        // on its card: space or ⏎ on a row, no menu.
        let colours_section = host.section;
        host.section = host
            .sections
            .iter()
            .position(|(o, _)| *o == Owner::Look)
            .unwrap();
        press(&mut host, KeyCode::End); // off
        press(&mut host, KeyCode::Up); // full
        press(&mut host, KeyCode::Char(' '));
        assert!(host.menu.is_none(), "the card is the choice");
        assert_eq!(host.colours.cfg.motion, "full");
        assert!(host.config_text().contains("motion = \"full\""));
        press(&mut host, KeyCode::Home); // the first pack: nerd
        press(&mut host, KeyCode::Down); // plain: unicode
        press(&mut host, KeyCode::Enter);
        assert_eq!(host.colours.cfg.icons, "unicode");
        assert!(host.config_text().contains("icons = \"unicode\""));
        host.section = colours_section;
        // The colours section starts at the depth.
        press(&mut host, KeyCode::Enter);
        assert!(host.menu.is_some(), "⏎ opens a menu");
        press(&mut host, KeyCode::Char('j'));
        press(&mut host, KeyCode::Enter); // second item: full colour
        assert!(host.menu.is_none());
        assert_eq!(host.colours.cfg.colors, "truecolor");
        press(&mut host, KeyCode::Char('j')); // splash
        assert!(!host.config_text().contains("splash"), "auto is not written");
        press(&mut host, KeyCode::Enter);
        press(&mut host, KeyCode::Char('j'));
        press(&mut host, KeyCode::Enter); // second item: off
        assert_eq!(host.colours.cfg.splash, "off");
        assert!(host.config_text().contains("splash = \"off\""));
        press(&mut host, KeyCode::Char('j')); // first slot: canvas
        press(&mut host, KeyCode::Enter);
        press(&mut host, KeyCode::Char('e'));
        for c in "010203".chars() {
            press(&mut host, KeyCode::Char(c));
        }
        press(&mut host, KeyCode::Enter);
        assert_eq!(
            host.colours.cfg.slots.get("bg.base").map(String::as_str),
            Some("#010203")
        );
        assert!(host.config_text().contains("bg.base"));
    }

    #[test]
    fn a_plugin_menu_routes_the_choice_back_to_that_plugin() {
        let (mut host, logs) = host(&[("a", 'A', "a"), ("b", 'B', "b")]);
        press(&mut host, KeyCode::Char('m'));
        assert_eq!(host.menu.as_ref().unwrap().menu.items.len(), 2);
        press(&mut host, KeyCode::Char('q'));
        assert!(
            host.menu.is_none() && !host.quit,
            "q closes the menu, not the app"
        );
        press(&mut host, KeyCode::Char('m'));
        press(&mut host, KeyCode::Char('y'));
        assert_eq!(logs[0].lock().unwrap().bus, vec!["menu ask yes"]);
        assert!(logs[1].lock().unwrap().bus.is_empty());
    }

    #[test]
    fn a_menus_notes_show_as_written_and_are_never_chosen() {
        let (mut host, logs) = host(&[("a", 'A', "a")]);
        host.menu = Some(OpenMenu {
            owner: Owner::Plugin(0),
            menu: Menu::new(
                "ask",
                "send this?",
                vec![
                    MenuItem::note("Fixed in **!263**, see"),
                    MenuItem::note("  - the Tests"),
                    MenuItem::new('y', "yes", "send it", "yes"),
                ],
            ),
            row: 2,
            input: None,
            refused: None,
            viewport: Default::default(),
            anchor: None,
        });
        let text = crate::shell::to_text(&host, 80, 24);
        assert!(text.contains("Fixed in **!263**, see"), "{text}");
        assert!(text.contains("  - the Tests"), "{text}");
        press(&mut host, KeyCode::Up);
        press(&mut host, KeyCode::Enter);
        assert_eq!(
            logs[0].lock().unwrap().bus,
            vec!["menu ask yes"],
            "the notes are skipped"
        );
    }

    fn writer() -> (Host, Arc<Mutex<Log>>) {
        let (mut host, logs) = host(&[("a", 'A', "a")]);
        host.load_config(Some("")).unwrap();
        logs[0].lock().unwrap().writes = true;
        (host, logs[0].clone())
    }

    fn labels(host: &Host) -> Vec<String> {
        host.menu
            .as_ref()
            .map(|m| m.menu.items.iter().map(|i| i.label.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_chrome_is_view_data_that_survives_the_wire() {
        use crate::view::chrome::{ChromeView, KeysAt, targets};
        let (mut host, _) = host(&[("books", 'B', "shelves"), ("films", 'F', "reels")]);
        host.load_config(Some("")).unwrap();
        let chrome = host.chrome_view(80, 24);
        let wire = serde_json::to_string_pretty(&chrome).unwrap();
        // The sample a reader of the slice sees (cargo nextest … --no-capture).
        println!("{wire}");
        let back: ChromeView = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, chrome);
        assert_eq!(chrome.version, crate::view::VERSION);
        let labels: Vec<_> = chrome.top_bar.tabs.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["books", "films", "history", "settings"]);
        assert!(chrome.top_bar.tabs[0].active);
        assert_eq!(chrome.top_bar.tabs[0].target, targets::screen_tab(0));
        assert_eq!(chrome.top_bar.mode.target, targets::mode());
        assert_eq!(chrome.status.scope, "BOOKS");
        let menu = chrome.status.hints.iter().find(|h| h.label == "extra actions").expect("extra actions");
        assert_eq!(menu.keys_of[0].chord, crate::view::Chord::char('x'));
        assert_eq!(chrome.keys_at, KeysAt::Body);
        assert!(chrome.menu.is_none() && chrome.settings.is_none());
        assert!(wire.contains("\"icon\": \"settings\""), "icons go by name: {wire}");
    }

    /// A part that makes its selection valid only while it
    /// builds its view; until then it has no row and offers no row keys.
    #[derive(Default)]
    struct SettlesInView {
        settled: std::cell::Cell<bool>,
    }

    impl Plugin for SettlesInView {
        fn id(&self) -> &'static str {
            "settles"
        }
        fn configure(&mut self, _: Option<&toml::Value>) -> Result<()> {
            Ok(())
        }
        fn config(&self) -> Option<toml::Value> {
            None
        }
        fn screens(&self) -> Vec<ScreenSpec> {
            vec![ScreenSpec {
                id: "s",
                name: "branches",
                key: 'B',
            }]
        }
        fn view(&self, scope: &str, _: &crate::view::ViewCx<'_>) -> Option<crate::view::ScreenView> {
            use crate::view as v;
            self.settled.set(true);
            Some(v::ScreenView::new(scope, vec![v::PaneView::new(
                v::PaneId::Middle, "branches", v::Node::Document { lines: vec![], cut: false },
            )]))
        }
        fn row_hints(&self, _: &str, _: Pane) -> Vec<KeyHint<'static>> {
            if self.settled.get() {
                vec![KeyHint::new("⏎", "use it")]
            } else {
                vec![]
            }
        }
    }

    #[test]
    fn chrome_status_has_the_row_keys_of_a_selection_settled_while_the_view_is_built() {
        // The chrome is built before the body paints (and Headless never
        // paints), so the status line must not read a selection the part has
        // not settled yet: a list screen lost its '⏎ use it' that way.
        let mut host = Host::new(vec![Box::new(SettlesInView::default())], 0).unwrap();
        host.load_config(Some("")).unwrap();
        host.focus = Pane::Middle;
        let status = host.chrome_status();
        let labels: Vec<_> = status.hints.iter().map(|h| h.label.as_str()).collect();
        assert!(labels.contains(&"use it"), "the row's key is on the status line: {labels:?}");
    }

    #[test]
    fn chrome_targets_do_what_a_click_on_them_does() {
        use crate::view::Intent;
        use crate::view::chrome::targets;
        let (mut host, _) = host(&[("books", 'B', "shelves"), ("films", 'F', "reels")]);
        host.load_config(Some("")).unwrap();
        // A tab: the screen it names shows.
        host.update(Event::Intent(Intent::Activate { target: targets::screen_tab(1) }));
        assert_eq!(host.view, View::Screen(1));
        // Settings, and a section in its navigation.
        host.update(Event::Intent(Intent::Activate { target: targets::settings_tab() }));
        assert_eq!(host.view, View::Settings);
        let nav = host.chrome_view(80, 24).settings.expect("settings navigation");
        let reels = nav.groups.iter().flat_map(|g| &g.sections).find(|s| s.name == "reels").unwrap();
        host.update(Event::Intent(Intent::Activate { target: reels.target.clone() }));
        assert_eq!(host.chrome_view(80, 24).settings.unwrap().selected.as_ref(), Some(&reels.target));
        // The mode opens its menu, whose rows are chosen by their targets.
        host.update(Event::Intent(Intent::Activate { target: targets::mode() }));
        let menu = host.chrome_view(80, 24).menu.expect("the mode's menu");
        let row = menu.items.iter().position(|c| c.target.is_some()).unwrap();
        host.update(Event::Intent(Intent::Select { target: targets::menu_item(row) }));
        assert_eq!(host.menu_view().unwrap().selected, Some(row));
        // A key on the status line is pressed: esc closes the menu.
        let status = host.chrome_status();
        let esc = status.hints.iter().find(|h| h.label == "close").unwrap();
        host.update(Event::Intent(Intent::Activate { target: esc.keys_of[0].target.clone() }));
        assert!(host.menu.is_none());
        // An unknown chrome target does nothing.
        assert!(host.update(Event::Intent(Intent::Activate { target: crate::view::Target::new("chrome:nope") })).is_empty());
    }

    #[test]
    fn the_history_overlay_is_chrome_view_data_pressed_by_its_targets() {
        use crate::view::Intent;
        let (mut host, _) = host(&[("books", 'B', "shelves")]);
        host.load_config(Some("")).unwrap();
        assert!(host.chrome_view(80, 24).history.is_none());
        host.update(Event::Intent(Intent::Activate { target: crate::view::chrome::targets::history_tab() }));
        let view = host.chrome_view(80, 24).history.expect("the overlay as view data");
        assert_eq!((view.width, view.height), (80, 23), "it takes the body under the top bar");
        // Its esc key, pressed by the target the view gave, closes it.
        let esc = view.footer.iter().flat_map(|h| &h.keys_of).find(|k| k.label == "esc").unwrap();
        host.update(Event::Intent(Intent::Activate { target: esc.target.clone() }));
        assert!(host.history.is_none());
    }

    #[test]
    fn the_history_overlay_takes_its_keys_sent_as_intents_as_it_takes_them_from_the_terminal() {
        // Ink and Headless send keys as Intent::Key; the overlay must take
        // them exactly as the Ratatui runner's Event::Key, not pass them to
        // the part under it.
        use crate::view::{Chord, Intent, Key};
        let open = || {
            let (mut host, logs) = host(&[("books", 'B', "shelves")]);
            host.load_config(Some("")).unwrap();
            host.update(Event::Intent(Intent::Activate { target: crate::view::chrome::targets::history_tab() }));
            assert!(host.history.is_some());
            (host, logs)
        };
        // Every key its help lists (and the capitals a user may type).
        let mut chords: Vec<Chord> = "jkhlnpratcixJR?".chars().map(Chord::char).collect();
        chords.extend(
            [Key::Up, Key::Down, Key::Left, Key::Right, Key::Home, Key::End, Key::Tab, Key::Enter, Key::PageUp, Key::PageDown]
                .map(Chord::new),
        );
        for chord in chords {
            let c = format!("{chord:?}");
            let (mut by_key, _) = open();
            let (mut by_intent, logs) = open();
            let a = by_key.update(Event::Key(crate::view_tui::key_of(chord)));
            let b = by_intent.update(Event::Intent(Intent::Key { chord }));
            assert_eq!(a.len(), b.len(), "{c}: the same effects");
            assert_eq!(
                by_key.chrome_view(80, 24).history,
                by_intent.chrome_view(80, 24).history,
                "{c}: the overlay is in the same state"
            );
            assert!(logs[0].lock().unwrap().keys.is_empty(), "{c}: the part under it got nothing");
        }
        // Esc closes it (after closing its help when that is open).
        let (mut host, logs) = open();
        host.update(Event::Intent(Intent::Key { chord: Chord::new(crate::view::Key::Esc) }));
        assert!(host.history.is_none(), "esc as an intent closes the overlay");
        assert!(logs[0].lock().unwrap().keys.is_empty());
    }

    #[test]
    fn an_action_invoked_by_its_stable_id_runs_by_its_key_under_one_dispatch() {
        let (mut host, _log) = writer();
        host.enable_journal();
        host.set_env_mode(Some((crate::writes::Mode::Edit, "RADAR_MODE".into())));
        let out = host.update(Event::Intent(crate::view::Intent::Invoke { action: "a.main.g".into() }));
        assert_eq!(host.menu.as_ref().map(|m| m.menu.id.as_str()), Some("approve"));
        // An id the view does not list reaches nobody.
        host.menu = None;
        host.update(Event::Intent(crate::view::Intent::Invoke { action: "a.main.nothing".into() }));
        assert!(host.menu.is_none());
        let records: Vec<_> = out.iter().filter_map(|e| match e {
            HostEffect::Journal(r) => Some(r),
            _ => None,
        }).collect();
        assert!(!records.is_empty(), "the invoke is journalled");
    }

    #[test]
    fn the_mode_decides_each_write_and_its_question_and_row_say_so() {
        use crate::writes::Mode;
        for mode in Mode::ALL {
            let (mut host, log) = writer();
            host.set_env_mode(Some((mode, "RADAR_MODE".into())));
            // An edit: approve.
            press(&mut host, KeyCode::Char('g'));
            let shown = labels(&host);
            match mode {
                Mode::ReadOnly => assert_eq!(
                    shown,
                    [
                        "as you",
                        "radar is read only (set by RADAR_MODE): nothing will be sent",
                        "yes",
                        "no"
                    ]
                ),
                Mode::Write => assert_eq!(
                    shown,
                    [
                        "as you",
                        "radar is in write mode (set by RADAR_MODE): this needs edit mode, nothing will be sent",
                        "yes",
                        "no"
                    ]
                ),
                Mode::Edit => assert_eq!(shown, ["as you", "yes", "no"]),
            }
            // The answer still reaches the part, which asks the mode before sending.
            press(&mut host, KeyCode::Char('y'));
            assert_eq!(
                log.lock().unwrap().bus.last().map(String::as_str),
                Some("menu approve yes")
            );
            assert_eq!(
                crate::writes::not_sent(crate::writes::Change::Edit).is_none(),
                mode == Mode::Edit
            );
            // An add: a note.
            press(&mut host, KeyCode::Char('v'));
            assert_eq!(
                labels(&host)
                    .iter()
                    .any(|l| l.contains("nothing will be sent")),
                mode == Mode::ReadOnly,
                "{mode:?}"
            );
            press(&mut host, KeyCode::Esc);
            // The x menu: each action is listed, muted with why while refused.
            press(&mut host, KeyCode::Char('x'));
            let rows = labels(&host);
            let muted: Vec<bool> = host
                .menu
                .as_ref()
                .unwrap()
                .menu
                .items
                .iter()
                .map(|i| i.muted)
                .collect();
            match mode {
                Mode::ReadOnly => {
                    assert_eq!(
                        rows[..2],
                        [
                            "post a note on #12… · radar is read only",
                            "approve #12… · radar is read only"
                        ]
                    );
                    assert_eq!(muted[..2], [true, true]);
                }
                Mode::Write => {
                    assert_eq!(
                        rows[..2],
                        ["post a note on #12…", "approve #12… · needs edit mode"]
                    );
                    assert_eq!(muted[..2], [false, true]);
                }
                Mode::Edit => {
                    assert_eq!(rows[..2], ["post a note on #12…", "approve #12…"]);
                    assert_eq!(muted[..2], [false, false]);
                }
            }
            assert!(rows.iter().all(|row| !row.starts_with("mode:")));
            // Its key still opens the question.
            press(&mut host, KeyCode::Char('g'));
            assert_eq!(host.menu.as_ref().unwrap().menu.id, "approve", "{mode:?}");
        }
    }

    #[test]
    fn the_mode_is_kept_in_general_and_the_environment_sets_it_for_one_run() {
        use crate::writes::Mode;
        let (mut host, _) = host(&[("a", 'A', "a")]);
        host.load_config(Some("")).unwrap();
        assert_eq!(host.mode(), Mode::Edit, "edit by default");
        assert!(
            !host.config_text().contains("general"),
            "the default is not written"
        );
        host.load_config(Some("[general]\nmode = \"write\"\n"))
            .unwrap();
        assert_eq!(host.mode(), Mode::Write);
        assert_eq!(crate::writes::mode().0, Mode::Write, "told to this thread");
        assert!(
            host.config_text().contains("[general]\nmode = \"write\""),
            "{}",
            host.config_text()
        );
        assert!(
            host.load_config(Some("[general]\nmode = \"sometimes\"\n"))
                .is_err(),
            "not a mode"
        );
        // RADAR_MODE, or an old switch, for one run: the setting is kept underneath.
        host.load_config(Some("[general]\nmode = \"write\"\n"))
            .unwrap();
        host.set_env_mode(crate::writes::env_mode([(
            "RADAR_ACME_NO_WRITES".to_string(),
            "1".to_string(),
        )]));
        assert_eq!(host.mode(), Mode::ReadOnly);
        assert_eq!(host.mode_cfg, Mode::Write);
        assert_eq!(
            crate::writes::mode(),
            (Mode::ReadOnly, Some("RADAR_ACME_NO_WRITES".into()))
        );
        host.set_env_mode(crate::writes::env_mode([(
            "RADAR_MODE".to_string(),
            "edit".to_string(),
        )]));
        assert_eq!(host.mode(), Mode::Edit);
        // Choosing in the menu changes the setting, saved; the run keeps the environment's.
        host.open_mode_menu();
        assert_eq!(
            host.menu.as_ref().unwrap().menu.title,
            "what radar may change · RADAR_MODE sets it for this run"
        );
        let fx = press(&mut host, KeyCode::Char('r'));
        assert!(
            fx.iter()
                .any(|e| matches!(e, HostEffect::Save(t) if t.contains("mode = \"read-only\""))),
            "{fx:?}"
        );
        assert_eq!((host.mode_cfg, host.mode()), (Mode::ReadOnly, Mode::Edit));
        assert_eq!(
            host.message.as_deref(),
            Some("mode read only saved; RADAR_MODE keeps radar edit for this run")
        );
        host.set_env_mode(None);
        assert_eq!(host.mode(), Mode::ReadOnly);
    }

    #[test]
    fn the_top_bar_shows_the_mode_at_every_width_and_a_click_opens_its_menu() {
        use crate::writes::Mode;
        let (mut host, _) = host(&[("a", 'A', "a"), ("b", 'B', "b")]);
        host.load_config(Some("")).unwrap();
        for (mode, words) in [
            (Mode::ReadOnly, "◉ read only"),
            (Mode::Write, "✎ write"),
            (Mode::Edit, "✎ edit"),
        ] {
            host.set_env_mode(Some((mode, "RADAR_MODE".into())));
            for w in [80, 160] {
                let text = crate::shell::to_text(&host, w, 24);
                let top = text.lines().next().unwrap();
                assert!(top.trim_end().ends_with(words), "{w}: {top}");
            }
        }
        host.set_env_mode(None);
        let _ = crate::shell::to_text(&host, 80, 24);
        let at = host
            .hits
            .borrow()
            .iter()
            .find(|(_, h)| *h == Hit::Mode)
            .map(|(r, _)| (r.x, r.y))
            .expect("the mode is clickable");
        host.update(Event::Mouse {
            kind: crate::host::MouseKind::Press,
            column: at.0,
            row: at.1,
            at_ms: 0,
        });
        assert_eq!(host.menu.as_ref().unwrap().menu.id, crate::mode::MODE_MENU);
        assert_eq!(labels(&host), ["read only", "write", "edit · now"]);
    }

    #[test]
    fn settings_general_mode_is_a_row_with_each_modes_meaning() {
        use crate::writes::Mode;
        let (mut host, _) = host(&[("a", 'A', "a")]);
        host.load_config(Some("")).unwrap();
        host.set_env_mode(Some((Mode::ReadOnly, "RADAR_MODE".into())));
        host.view = View::Settings;
        let i = host
            .sections
            .iter()
            .position(|(o, _)| *o == Owner::Mode)
            .unwrap();
        host.select_section(i);
        host.focus = Pane::Middle;
        let text = crate::shell::to_text(&host, 160, 40);
        assert!(text.contains("mode    read only"), "{text}");
        assert!(text.contains("        set by RADAR_MODE "), "{text}");
        assert!(
            text.contains("Now RADAR_MODE keeps radar read only for this run"),
            "{text}"
        );
        assert!(text.contains("may add things: comments, replies"), "{text}");
        insta::assert_snapshot!("settings_mode_160x40", text);
        insta::assert_snapshot!("settings_mode_80x24", crate::shell::to_text(&host, 80, 24));
        press(&mut host, KeyCode::Enter);
        assert_eq!(host.menu.as_ref().unwrap().menu.id, crate::mode::MODE_MENU);
        press(&mut host, KeyCode::Char('w'));
        assert_eq!(host.mode_cfg, Mode::Write);
    }

    #[test]
    fn a_menus_question_is_never_cut_while_it_fits() {
        let (mut host, _) = host(&[("a", 'A', "a")]);
        let question = "post this in #general, in Somebody Longname's thread?";
        host.menu = Some(OpenMenu {
            owner: Owner::Plugin(0),
            menu: Menu::write_confirm("ask", question, "ok", "post it", "drop it"),
            row: 1,
            input: None,
            refused: None,
            viewport: Default::default(),
            anchor: None,
        });
        let text = crate::shell::to_text(&host, 80, 24);
        assert!(text.contains(question), "{text}");
    }

    #[test]
    fn a_question_opens_on_yes_and_arrows_tab_and_hl_move_along_its_buttons() {
        let (mut host, log) = writer();
        press(&mut host, KeyCode::Char('v'));
        let row = |host: &Host| host.menu.as_ref().expect("the question is open").row;
        assert_eq!(row(&host), 1, "the note is skipped: y is highlighted");
        press(&mut host, KeyCode::Right);
        assert_eq!(row(&host), 2);
        press(&mut host, KeyCode::Char('l'));
        assert_eq!(row(&host), 3);
        press(&mut host, KeyCode::Right);
        assert_eq!(row(&host), 3, "the last button");
        press(&mut host, KeyCode::Left);
        assert_eq!(row(&host), 2);
        press(&mut host, KeyCode::BackTab);
        assert_eq!(row(&host), 1);
        press(&mut host, KeyCode::Tab);
        assert_eq!(row(&host), 2);
        press(&mut host, KeyCode::Char('j'));
        assert_eq!(row(&host), 3, "j/k move too");
        press(&mut host, KeyCode::Char('h'));
        press(&mut host, KeyCode::Enter);
        assert_eq!(log.lock().unwrap().bus, vec!["menu note edit"]);
        // A list's items are not along a row: ←/→ leave its highlight alone.
        press(&mut host, KeyCode::Char('n'));
        press(&mut host, KeyCode::Right);
        assert_eq!(row(&host), 0);
    }

    #[test]
    fn a_menu_opens_on_the_item_it_names_and_the_arrows_move_from_there() {
        let (mut host, _) = host(&[("a", 'A', "a")]);
        let items = ["go", "one", "two", "three"]
            .map(|v| MenuItem::pick(v, "", v))
            .to_vec();
        host.menu = Some(OpenMenu::open(
            Owner::Plugin(0),
            Menu::new("m", "pick", items.clone()).at("two"),
        ));
        let row = |host: &Host| host.menu.as_ref().expect("open").row;
        assert_eq!(row(&host), 2);
        press(&mut host, KeyCode::Down);
        assert_eq!(row(&host), 3, "the arrow moves from the named item");
        press(&mut host, KeyCode::Up);
        press(&mut host, KeyCode::Up);
        assert_eq!(row(&host), 1);
        let none = OpenMenu::open(Owner::Plugin(0), Menu::new("m", "pick", items.clone()));
        assert_eq!(none.row, 0, "no `at`: the first, as ever");
        let unknown = OpenMenu::open(Owner::Plugin(0), Menu::new("m", "pick", items).at("gone"));
        assert_eq!(
            unknown.row, 0,
            "a value that is not there opens on the first"
        );
    }

    #[test]
    fn a_menu_taller_than_the_screen_scrolls_to_the_highlighted_row() {
        let (mut host, logs) = host(&[("a", 'A', "a")]);
        press(&mut host, KeyCode::Char('n'));
        assert!(!crate::shell::to_text(&host, 80, 24).contains("item 39"));
        for _ in 0..40 {
            press(&mut host, KeyCode::Down);
        }
        assert!(crate::shell::to_text(&host, 80, 24).contains("item 39"));
        press(&mut host, KeyCode::Enter);
        assert_eq!(logs[0].lock().unwrap().bus, vec!["menu long 39"]);
    }

    #[test]
    fn every_plugin_hears_that_someone_pressed_clicked_or_pasted() {
        let (mut host, logs) = host(&[("a", 'A', "a"), ("b", 'B', "b")]);
        host.update(Event::Tick(100));
        assert!(
            logs[0].lock().unwrap().inputs.is_empty(),
            "a tick is not input"
        );
        press(&mut host, KeyCode::Char('y'));
        host.update(Event::Tick(200));
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: 0,
            row: 0,
            at_ms: 0,
        });
        host.update(Event::Paste("x".into()));
        for log in &logs {
            assert_eq!(log.lock().unwrap().inputs, vec![100, 200, 200]);
        }
    }

    #[test]
    fn z_pauses_every_part_once_and_idle_pauses_itself_until_a_key() {
        let (mut host, _logs) = host(&[("a", 'A', "alpha"), ("b", 'B', "beta")]);
        let t0 = host.now;
        let z = |host: &mut Host| press(host, KeyCode::Char('z'));
        z(&mut host);
        assert_eq!(host.paused(), Some(crate::mode::Pause::Asked));
        assert_eq!(host.paused_words(), Some("paused · z resumes"));
        // Another key does not undo a pause that was asked for.
        press(&mut host, KeyCode::Char('j'));
        assert_eq!(host.paused(), Some(crate::mode::Pause::Asked));
        z(&mut host);
        assert_eq!(host.paused(), None);
        // Idle: after the minutes, then any key resumes.
        host.update(Event::Tick(t0 + 29 * 60));
        assert_eq!(host.paused(), None);
        host.update(Event::Tick(t0 + 30 * 60));
        assert_eq!(host.paused(), Some(crate::mode::Pause::Idle));
        press(&mut host, KeyCode::Char('j'));
        assert_eq!(host.paused(), None);
        // 0 never pauses.
        host.pause.idle_minutes = 0;
        host.update(Event::Tick(t0 + 10 * 3600));
        assert_eq!(host.paused(), None);
    }

    #[test]
    fn idle_minutes_come_from_general_else_a_parts_old_key_and_are_kept_only_when_changed() {
        let (mut host, _logs) = host(&[("a", 'A', "alpha")]);
        host.load_config(Some("[a]\nidle_minutes = 5\n")).unwrap();
        assert_eq!(host.pause.idle_minutes, 5);
        host.load_config(Some(
            "[a]\nidle_minutes = 5\n[general]\nidle_minutes = 15\n",
        ))
        .unwrap();
        assert_eq!(host.pause.idle_minutes, 15);
        assert!(host.config_text().contains("idle_minutes = 15"));
        host.load_config(Some("")).unwrap();
        assert_eq!(host.pause.idle_minutes, 30);
        assert!(!host.config_text().contains("idle_minutes"));
    }

    #[test]
    fn ctrl_c_quits_from_anywhere() {
        let (mut host, _) = host(&[("a", 'A', "a")]);
        press(&mut host, KeyCode::Char('m'));
        assert!(host.menu.is_some());
        host.update(Event::Key(KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
        )));
        assert!(host.quit);
    }

    /// A host with icons from `pack` and a probe whose menus and message
    /// carry icons.
    fn icon_host(pack: &str) -> Host {
        let (mut host, logs) = host(&[("a", 'A', "alpha")]);
        host.load_config(Some(&format!("[theme]\nicons = \"{pack}\"\n")))
            .unwrap();
        logs[0].lock().unwrap().icons = true;
        host
    }

    /// The styles of the cells on row `y` from `x`, `n` of them.
    fn styles_at(host: &Host, needle: &str) -> (u16, u16, Vec<(String, Style)>) {
        let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        term.draw(|f| { crate::shell::render(host, &theme, f); crate::shell::render_notices(host, &theme, f.buffer_mut()); })
            .unwrap();
        let buf = term.backend().buffer().clone();
        for y in 0..24 {
            let row = crate::shell::row_text(&buf, y);
            if let Some(i) = row.find(needle) {
                let x = crate::ui::width(&row[..i]) as u16;
                let cells = (0..80u16.saturating_sub(x))
                    .map(|k| {
                        (
                            buf[(x + k, y)].symbol().to_string(),
                            buf[(x + k, y)].style(),
                        )
                    })
                    .take(needle.chars().count())
                    .collect();
                return (x, y, cells);
            }
        }
        panic!(
            "{needle:?} not on screen:\n{}",
            crate::shell::to_text(host, 80, 24)
        );
    }

    #[test]
    fn menus_and_messages_draw_a_parts_icons_in_their_own_colour() {
        let mut host = icon_host("unicode");
        press(&mut host, KeyCode::Char('i'));
        let theme = host.build_theme(crate::design::theme::Tier::TrueColor);
        let own = |i: Icon| theme.spec(&theme.icons().get(i).style).fg;
        let bug = theme.icons().get(Icon::Bug).glyph.clone();
        let link = theme.icons().get(Icon::Link).glyph.clone();
        let repo = theme.icons().get(Icon::Repo).glyph.clone();
        let failed = theme.icons().get(Icon::RunFailed).glyph.clone();
        // The title: the part's icon, then its words.
        let (_, _, cells) = styles_at(&host, &format!("{bug} crash"));
        assert_eq!(cells[0].1.fg, own(Icon::Bug), "the title's icon");
        assert_eq!(
            cells[2].1.fg,
            theme.s(Slot::TextEmphasis).fg,
            "the title's words"
        );
        // A heading and an item.
        let (_, _, cells) = styles_at(&host, &format!("{repo} WHERE"));
        assert_eq!(cells[0].1.fg, own(Icon::Repo));
        let (_, _, cells) = styles_at(&host, &format!("{link} open"));
        assert_eq!(cells[0].1.fg, own(Icon::Link));
        assert_ne!(
            cells[2].1.fg,
            own(Icon::Link),
            "the label in the row's colour"
        );
        // Items line up with an item that has no icon.
        let (x_open, _, _) = styles_at(&host, &format!("{link} open"));
        let (x_close, _, _) = styles_at(&host, "close");
        assert_eq!(
            x_close, x_open,
            "an item without an icon starts where one with an icon does"
        );
        assert!(crate::shell::to_text(&host, 80, 24).contains("closes it"));
        // The message.
        assert_eq!(host.message.as_deref(), Some("it broke"));
        assert_eq!(host.message.as_ref().unwrap().icon, Some(Icon::RunFailed));
        host.menu = None;
        let (_, y, cells) = styles_at(&host, &format!("{failed} it broke"));
        assert_eq!(y, 21, "notice stays above the panel border and status hints");
        assert_eq!(cells[0].1.fg, own(Icon::RunFailed));
        assert_eq!(cells[2].1.fg, theme.s(Slot::TextPrimary).fg);
    }

    #[test]
    fn a_keys_menu_heading_icon_leads_the_menus_title_and_items_keep_theirs() {
        let mut host = icon_host("unicode");
        press(&mut host, KeyCode::Char('x'));
        let open = host.menu.as_ref().expect("the keys menu");
        assert_eq!(
            (open.menu.title.as_str(), open.menu.icon),
            ("crash", Some(Icon::Bug))
        );
        assert_eq!(open.menu.items[0].icon, Some(Icon::Link));
        let text = crate::shell::to_text(&host, 80, 24);
        let theme = host.build_theme(crate::design::theme::Tier::Mono);
        let menu = theme.icons().get(Icon::Menu).glyph.clone();
        let bug = theme.icons().get(Icon::Bug).glyph.clone();
        assert!(
            text.contains(&format!("{menu} {bug} crash")),
            "the host's icon, then the part's:\n{text}"
        );
    }

    #[test]
    fn with_icons_off_a_parts_icons_take_no_room() {
        let mut host = icon_host("off");
        press(&mut host, KeyCode::Char('i'));
        let text = crate::shell::to_text(&host, 80, 24);
        assert!(text.contains(" crash "), "{text}");
        assert!(text.contains(" WHERE"), "{text}");
        assert!(text.contains(" open    opens it"), "{text}");
        assert!(!text.contains(" o  open"), "no separate letter or icon column: {text}");
        host.menu = None;
        let text = crate::shell::to_text(&host, 80, 24);
        assert!(
            text.lines().nth(21).unwrap().contains("info it broke"),
            "{text}"
        );
    }

    #[test]
    fn the_actions_menu_is_for_where_you_are_and_names_it() {
        let (mut host, _) = host(&[("a", 'A', "alpha")]);
        host.load_config(Some("[a]\nvalue = 0\n")).unwrap();
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        host.focus = Pane::Nav;
        press(&mut host, KeyCode::Char('x'));
        let open = host.menu.as_ref().expect("a menu");
        assert_eq!(open.menu.title, "settings");
        let labels: Vec<&str> = open.menu.items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(
            labels,
            vec![
                "go into alpha",
                "toggle the group",
                "expand the group",
                "collapse the group, or the one this row is in",
                "expand every held group",
                "recent notices (this session)",
                "History",
                "recent activity (last 24 hours)",
                "all activity history"
            ],
            "nothing from elsewhere; the nav's groups fold"
        );
    }

    #[test]
    fn settings_nav_moves_between_sections_of_different_plugins() {
        let (mut host, logs) = host(&[("a", 'A', "alpha"), ("b", 'B', "beta")]);
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        host.focus = Pane::Nav;
        press(&mut host, KeyCode::Char('j'));
        assert_eq!(host.current_section().unwrap().1.name, "beta");
        press(&mut host, KeyCode::Enter);
        press(&mut host, KeyCode::Char('+'));
        assert_eq!(logs[1].lock().unwrap().value, 1);
    }

    #[test]
    fn up_past_the_top_reaches_the_bar_which_switches_screens_and_down_goes_back_to_the_pane_left()
    {
        let (mut host, _) = host(&[("a", 'A', "alpha"), ("b", 'B', "beta")]);
        let bottom = |host: &Host| {
            crate::shell::to_text(host, 120, 24)
                .lines()
                .last()
                .unwrap()
                .to_string()
        };
        // On alpha's detail pane; the probe takes no ↑, so it is past the top.
        host.focus = Pane::Detail;
        press(&mut host, KeyCode::Up);
        assert!(host.bar, "↑ at the top: the bar");
        assert!(
            bottom(&host).contains("←→ screens") && bottom(&host).contains("↓ enter"),
            "{}",
            bottom(&host)
        );
        // j and k do not move in the bar; ← → switch the screen as they go.
        press(&mut host, KeyCode::Char('k'));
        assert!(host.bar);
        press(&mut host, KeyCode::Right);
        assert_eq!((host.view, host.bar), (View::Screen(1), true));
        // Beta was never visited: ↓ goes to its first pane.
        press(&mut host, KeyCode::Down);
        assert!(!host.bar);
        assert_eq!(host.focus, Pane::Nav);
        // Back up, left to alpha, ↓: the pane it had.
        host.focus = Pane::Middle;
        press(&mut host, KeyCode::Char('k'));
        assert!(host.bar);
        press(&mut host, KeyCode::Char('h'));
        assert_eq!(host.view, View::Screen(0));
        press(&mut host, KeyCode::Enter);
        assert_eq!((host.bar, host.focus), (false, Pane::Detail));
        // Settings is on the bar too; up from the nav's first row gets back.
        press(&mut host, KeyCode::Up);
        press(&mut host, KeyCode::Right);
        press(&mut host, KeyCode::Right);
        assert_eq!(host.view, View::Settings);
        press(&mut host, KeyCode::Esc);
        assert_eq!((host.bar, host.focus), (false, Pane::Nav));
        for _ in 0..12 {
            press(&mut host, KeyCode::Up);
        }
        assert!(host.bar, "up from the nav's first row");
        // The x menu says what the bar's keys do; a screen's letter leaves it.
        press(&mut host, KeyCode::Char('x'));
        let labels: Vec<String> = host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .map(|i| format!("{} {}", i.keys, i.label))
            .collect();
        assert!(
            labels.iter().any(|l| l.starts_with("← "))
                && labels.iter().any(|l| l.starts_with("↓ ")),
            "{labels:?}"
        );
        host.menu = None;
        press(&mut host, KeyCode::Char('A'));
        assert_eq!((host.view, host.bar), (View::Screen(0), false));
    }

    #[test]
    fn inside_an_item_page_up_at_the_top_reaches_its_tab_row_not_the_bar() {
        let (mut host, logs) = host(&[("a", 'A', "alpha")]);
        press(&mut host, KeyCode::Char('p'));
        host.focus = Pane::Middle;
        let bottom = |host: &Host| {
            crate::shell::to_text(host, 120, 24)
                .lines()
                .last()
                .unwrap()
                .to_string()
        };
        let tab = || logs[0].lock().unwrap().tab;
        press(&mut host, KeyCode::Up);
        assert_eq!(
            (host.bar, host.tabs),
            (false, true),
            "↑ at the top of the body: the tab row"
        );
        assert!(
            bottom(&host).contains("←→ tabs") && bottom(&host).contains("↓ enter"),
            "{}",
            bottom(&host)
        );
        // ← → (h l, [ ], tab) show the tab beside, wrapping; the row keeps the keys.
        press(&mut host, KeyCode::Right);
        assert_eq!((tab(), host.tabs), (1, true));
        press(&mut host, KeyCode::Char('h'));
        assert_eq!(tab(), 0);
        press(&mut host, KeyCode::Left);
        assert_eq!(tab(), 2, "wraps");
        press(&mut host, KeyCode::Char(']'));
        assert_eq!(tab(), 0);
        press(&mut host, KeyCode::Tab);
        assert_eq!((tab(), host.tabs), (1, true));
        // The x menu names the tabs either side; ↓ goes back into the page.
        press(&mut host, KeyCode::Char('x'));
        let labels: Vec<String> = host
            .menu
            .as_ref()
            .unwrap()
            .menu
            .items
            .iter()
            .map(|i| format!("{} {}", i.keys, i.label))
            .collect();
        assert!(
            labels.contains(&"← the tab to the left: Overview".to_string()),
            "{labels:?}"
        );
        assert!(
            labels.contains(&"→ the tab to the right: Threads".to_string()),
            "{labels:?}"
        );
        host.menu = None;
        press(&mut host, KeyCode::Down);
        assert!(!host.tabs);
        assert_eq!(host.focus, Pane::Nav, "→ lands on the tab's first pane");
        // esc on the row leaves the row, not the page.
        press(&mut host, KeyCode::Up);
        press(&mut host, KeyCode::Esc);
        assert_eq!((host.tabs, logs[0].lock().unwrap().depth), (false, 1));
    }

    #[test]
    fn on_an_item_page_h_l_past_the_panes_and_tab_on_one_pane_move_across_the_tabs() {
        let (mut host, logs) = host(&[("a", 'A', "alpha")]);
        press(&mut host, KeyCode::Char('p'));
        host.focus = Pane::Middle;
        let tab = || logs[0].lock().unwrap().tab;
        // The first tab has one pane: l goes to the next tab, h back.
        press(&mut host, KeyCode::Char('l'));
        assert_eq!((tab(), host.focus, host.tabs), (1, Pane::Nav, false));
        press(&mut host, KeyCode::Left);
        assert_eq!(
            (tab(), host.focus),
            (0, Pane::Middle),
            "back, on the tab's last pane"
        );
        press(&mut host, KeyCode::Tab);
        assert_eq!(
            (tab(), host.focus),
            (1, Pane::Nav),
            "tab with one pane: the next tab"
        );
        // A tab with two panes: tab goes round them, h and l move between
        // them and past the edge to the tabs beside.
        press(&mut host, KeyCode::Tab);
        assert_eq!((tab(), host.focus), (1, Pane::Middle));
        press(&mut host, KeyCode::BackTab);
        assert_eq!((tab(), host.focus), (1, Pane::Nav));
        press(&mut host, KeyCode::Char('h'));
        assert_eq!(
            (tab(), host.focus),
            (0, Pane::Middle),
            "past the first pane: the tab before, on its last pane"
        );
        press(&mut host, KeyCode::Char('h'));
        assert_eq!(
            (tab(), host.focus),
            (2, Pane::Middle),
            "wraps to the last tab"
        );
        press(&mut host, KeyCode::Tab);
        assert_eq!((tab(), host.focus), (2, Pane::Nav));
        press(&mut host, KeyCode::Char('l'));
        assert_eq!((tab(), host.focus), (2, Pane::Middle));
        press(&mut host, KeyCode::Right);
        assert_eq!(
            (tab(), host.focus),
            (0, Pane::Middle),
            "past the last pane: the next tab"
        );
        assert!(
            host.message.is_none(),
            "nothing is dead on a page with tabs"
        );
    }

    #[test]
    fn a_click_takes_the_keys_back_from_the_bar() {
        let (mut host, _) = host(&[("a", 'A', "alpha")]);
        press(&mut host, KeyCode::Up);
        assert!(host.bar);
        let _ = crate::shell::to_text(&host, 80, 24);
        host.update(Event::Mouse {
            kind: MouseKind::Press,
            column: 40,
            row: 10,
            at_ms: 0,
        });
        assert!(!host.bar);
    }

    #[test]
    fn plus_and_minus_fold_the_settings_navs_groups_and_the_status_line_says_so() {
        let (mut host, _) = host(&[("a", 'A', "alpha"), ("b", 'B', "beta")]);
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        host.focus = Pane::Nav;
        let bottom = |host: &Host| {
            crate::shell::to_text(host, 120, 24)
                .lines()
                .last()
                .unwrap()
                .to_string()
        };
        assert!(
            bottom(&host).contains("+/- expand/collapse"),
            "{}",
            bottom(&host)
        );
        // `-` on a section folds its group, the cursor to the heading.
        let g = host.nav_at().0;
        press(&mut host, KeyCode::Char('-'));
        assert!(host.folded.contains(g));
        assert_eq!(host.nav_at(), (g, None));
        press(&mut host, KeyCode::Char('-'));
        assert!(host.folded.contains(g), "folded already: nothing");
        press(&mut host, KeyCode::Char('+'));
        assert!(!host.folded.contains(g));
        press(&mut host, KeyCode::Char('+'));
        assert!(!host.folded.contains(g), "open already: nothing");
        press(&mut host, KeyCode::Char('-'));
        press(&mut host, KeyCode::Char('='));
        assert!(!host.folded.contains(g), "`=` is `+` without shift");
        // Elsewhere, no tree with groups: no hint.
        host.focus = Pane::Detail;
        assert!(
            !bottom(&host).contains("expand/collapse"),
            "{}",
            bottom(&host)
        );
    }

    #[test]
    fn settings_nav_lists_general_first_and_folds_a_group() {
        let (mut host, _) = host(&[("a", 'A', "alpha"), ("b", 'B', "beta")]);
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        host.focus = Pane::Nav;
        let groups: Vec<&str> = host.nav_groups().iter().map(|(g, _)| *g).collect();
        assert_eq!(groups, ["general", "a", "b"]);
        assert_eq!(host.group_name("general"), "General");
        // Opens on the first part's section, not on general's.
        assert_eq!(host.current_section().unwrap().1.name, "alpha");
        // Up stops on the group's heading; space folds it.
        press(&mut host, KeyCode::Char('k'));
        assert_eq!(host.nav_at(), ("a", None));
        press(&mut host, KeyCode::Char(' '));
        assert!(!host.nav_rows().contains(&("a", Some(host.section))));
        // Down from the folded heading skips its sections.
        press(&mut host, KeyCode::Char('j'));
        assert_eq!(host.nav_at(), ("b", None));
        press(&mut host, KeyCode::Char('j'));
        assert_eq!(host.current_section().unwrap().1.name, "beta");
        // Going to a part's settings unfolds its group.
        host.go_to_settings_of(0);
        assert!(!host.folded.contains("a"));
    }

    #[test]
    fn left_and_right_past_the_outer_panes_go_to_the_screen_beside_unless_tree_owned() {
        let (mut host, _) = host(&[("a", 'A', "alpha"), ("b", 'B', "beta")]);
        assert_eq!(host.view, View::Screen(0));
        host.focus = Pane::Detail;
        press(&mut host, KeyCode::Right);
        assert_eq!((host.view, host.focus), (View::Screen(1), Pane::Nav));
        press(&mut host, KeyCode::Char('l'));
        assert_eq!(host.focus, Pane::Middle, "inside a screen it moves panes");
        host.focus = Pane::Detail;
        press(&mut host, KeyCode::Right);
        assert_eq!((host.view, host.focus), (View::Settings, Pane::Nav));
        // Settings navigation is an actual tree: left selects the parent,
        // then closes it, then stays at its boundary. h still changes panes.
        let group = host.nav_at().0;
        press(&mut host, KeyCode::Left);
        assert_eq!((host.view, host.focus), (View::Settings, Pane::Nav));
        assert!(host.on_heading);
        assert!(!host.folded.contains(group));
        press(&mut host, KeyCode::Left);
        assert!(host.folded.contains(group));
        press(&mut host, KeyCode::Left);
        assert_eq!((host.view, host.focus), (View::Settings, Pane::Nav));
        press(&mut host, KeyCode::Char('h'));
        assert_eq!((host.view, host.focus), (View::Screen(1), Pane::Detail));
        host.view = View::Settings;
        host.focus = Pane::Middle;
        press(&mut host, KeyCode::Right);
        assert_eq!((host.view, host.focus), (View::Settings, Pane::Detail));
        press(&mut host, KeyCode::Right);
        assert_eq!(host.view, View::Screen(0), "it wraps round");
    }

    #[test]
    fn what_changed_on_screen_is_cued_for_motion() {
        use crate::motion::{Changes, Cue};
        let (mut host, _) = host(&[("alpha", 'A', "a"), ("beta", 'B', "b")]);
        assert!(host.take_cues().is_empty());
        press(&mut host, KeyCode::Char('B'));
        assert_eq!(host.take_cues(), [Cue::Screen]);
        press(&mut host, KeyCode::Char(SETTINGS_KEY));
        assert_eq!(host.take_cues(), [Cue::Screen], "settings is a screen too");
        press(&mut host, KeyCode::Char('A'));
        host.take_cues();
        press(&mut host, KeyCode::Char('p'));
        assert_eq!(
            host.take_cues(),
            [Cue::Page],
            "a page opened on the same screen"
        );
        press(&mut host, KeyCode::Char('m'));
        assert_eq!(host.take_cues(), [Cue::Popup]);
        press(&mut host, KeyCode::Down);
        assert!(host.take_cues().is_empty(), "moving in it shows nothing");
        press(&mut host, KeyCode::Esc);
        assert!(host.take_cues().is_empty(), "closing it is instant");
        host.update(Event::Notice("copied 12 characters".into()));
        assert_eq!(host.take_cues(), [Cue::Message]);
        host.update(Event::Notice("copied 12 characters".into()));
        assert_eq!(host.take_cues(), [Cue::Message], "identical words are a new occurrence");
        press(&mut host, KeyCode::Char('a'));
        assert!(host.take_cues().is_empty(), "information remains until expiry or Escape");
        press(&mut host, KeyCode::Esc);
        assert_eq!(host.take_cues(), [Cue::MessageGone]);
        press(&mut host, KeyCode::Char('c'));
        let mut changes = Changes::default();
        changes.changed.insert("probe:1".into());
        assert_eq!(
            host.take_cues(),
            [Cue::Changed(changes)],
            "a part's changed rows; an empty change is nothing"
        );
        for _ in 0..100 {
            press(&mut host, KeyCode::Char('m'));
            press(&mut host, KeyCode::Esc);
        }
        assert_eq!(
            host.take_cues().len(),
            CUES_KEPT,
            "kept short when nobody takes them"
        );
    }
}
