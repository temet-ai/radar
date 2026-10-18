//! The plugin contract. Every ability radar has is a [`Plugin`] (a part); the
//! host knows no tool or service.
//!
//! A plugin can contribute, all optional:
//! - **screens** in the universal layout (nav │ middle │ detail), switched to
//!   with a Shift+letter;
//! - **settings sections** shown on the host's settings screen;
//! - a **config table** `[<id>]` in `config.toml`;
//! - **theme slots** for its own domain colours, with defaults the user can
//!   override (TUI-C4, TUI-C5);
//! - **background work**, as [`Effect::Spawn`] tasks whose results come back
//!   through [`Plugin::on_msg`];
//! - a **cache** restored at start-up so the first frame is never empty;
//! - **bus topics** other plugins react to, without depending on each other;
//! - **CLI commands** under `radar <id> …` (TUI-A3).
//!
//! The reducer rule holds for plugins too (TUI-A2): handlers change the
//! plugin's own state and return [`Effect`]s; they never do I/O themselves.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;

use crate::compose::{Adds, Contribution, Offer, Owned, Point, Setup, Showing, Subject};
use crate::design::help_bar::KeyHint;
pub use crate::design::theme::SlotValue;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

use crate::ui::icon::Icon;

/// A message a plugin sends itself: the result of a spawned task.
pub type Msg = Box<dyn Any + Send>;

/// Work done off the event loop. Its output returns to the plugin that
/// spawned it.
pub type Task = Pin<Box<dyn Future<Output = Msg> + Send>>;

/// A CLI command's work; the string is printed.
pub type CliTask = Pin<Box<dyn Future<Output = anyhow::Result<String>> + Send>>;

#[path = "plugin_cli.rs"]
mod cli;
pub use cli::{CliExecution, CliObservation, CliObservations, MAX_CLI_OBSERVATIONS, ObservedCliTask};

/// What a plugin asks the host to do.
pub enum Effect {
    /// Bounded local work-map operation. The runner owns storage and mode.
    Work { id: u64, request: Box<crate::work_api::Request> },
    /// Resolve explicit candidates through their ready owners, without I/O.
    ResolveSources { id: u64, subjects: Vec<Subject> },
    /// Open an exact persisted source through its owning part. No guessed URL.
    ShowSource(crate::journal::SubjectKey),
    /// Start a semantic action beneath the current causal action. Use a
    /// stable code, never user text. Later effects in this batch follow it;
    /// another Journal starts a sibling beneath the batch's original parent.
    Journal(crate::journal::ActionDescriptor),
    /// Record a known domain outcome for the current correlated action.
    /// An opaque task completing alone never implies domain success.
    Report(crate::journal::Outcome),
    /// Run a task; deliver its output to this plugin's `on_msg`.
    Spawn(Task),
    /// Start a semantic action for this task and its result's continuation
    /// only. The action is a child of the current causal action; following
    /// effects keep their prior causal and semantic context. Report a known
    /// outcome from `on_msg`, as with [`Effect::Journal`].
    SpawnAction {
        action: crate::journal::ActionDescriptor,
        task: Task,
    },
    /// Record a known terminal outcome for an independent semantic child
    /// without changing the context of following effects or completing the
    /// current task. Use for synchronous observations, such as a skipped read.
    Observe {
        action: crate::journal::ActionDescriptor,
        outcome: crate::journal::Outcome,
    },
    /// Ask the host to start a local agent review of these change requests
    /// ([`crate::agent_context::ReviewRequest`]). The host runs the journey;
    /// the part names no provider and posts nothing.
    StartReview(crate::agent_context::ReviewRequest),
    /// Ask the host to send one subject to an agent: it asks the owning part
    /// for its [`crate::agent_context::AgentContext`] and hands it to the
    /// dock's part, which starts a session with it as the first message.
    SendToAgent(crate::agent_context::AgentContextRequest),
    /// Show the agent dock and give it the keys: its owner opened a tab the
    /// operator must see now (a CLI's own sign-in).
    ShowDock,
    /// Hide the agent dock and give its keys back to the page: its owner closed the last tab, so the dock
    /// has nothing to show.
    HideDock,
    /// Open a URL in the browser.
    Open(String),
    /// Open `link`, a desktop app's own link (`app://…`), with the program
    /// the desktop keeps for its scheme, never a browser; where that cannot
    /// be done (no desktop, over SSH, nothing takes the scheme), open `web`
    /// as [`Effect::Open`] does.
    OpenApp { link: String, web: String },
    /// Put text on the clipboard.
    Copy(String),
    /// Show a one-line message to the user.
    Notify(String),
    /// [`Effect::Notify`] led by an icon, which the host draws from the
    /// pack in use in the icon's own colour (nothing with icons off): for
    /// a part that builds the words where no theme is in reach.
    NotifyWith(Icon, String),
    /// One notice occurrence with an explicit producer-selected tone.
    Notice(Message),
    /// Tell every other plugin something happened.
    Emit { topic: &'static str, payload: Value },
    /// Write this plugin's [`Plugin::snapshot`] to its cache file.
    PersistCache,
    /// Move the focus to a pane, e.g. into the detail pane on `⏎`.
    Focus(Pane),
    /// Open a context menu over the screen. The choice comes back through
    /// [`Plugin::on_menu`].
    Menu(Menu),
    /// Show a subject: the part that owns its kind opens it (its page, its
    /// screen) and `esc` comes back here; with no owner, its link opens.
    Show(Subject),
    /// A sync changed these rows (by key, [`crate::motion::Changes`]): the
    /// host shows it, a glow where each row is drawn. Mark your rows with
    /// [`crate::motion::row`] while drawing.
    Changed(crate::motion::Changes),
    /// Press this key, as if typed where the focus is: a row the part drew
    /// for another part's action (`Parts::actions`) does what its key does.
    Press(char),
    /// Do the action `id` another part added to `subject` (a chip with an
    /// action that the owner drew, `Chip::acts`): the part named `part`
    /// (its `name()`, as `Parts::lines` gives it) is asked, as from `x`.
    Act {
        part: &'static str,
        subject: Subject,
        id: String,
    },
}

/// A context menu (after lazydocker's): a titled list of actions, each with
/// a key, a label and a line saying what it does. `j/k` and `⏎`, or the
/// item's key, choose; `esc` closes. A question ([`Menu::confirm`], the
/// write confirms) is drawn as a dialog instead: what it says, then its
/// choices as buttons in a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Menu {
    /// Returned to the plugin with the chosen item's value.
    pub id: String,
    pub title: String,
    pub items: Vec<MenuItem>,
    /// An icon before the title (what the menu is about), drawn by the
    /// host in its own colour; after the host's own (a confirm's warning).
    pub icon: Option<Icon>,
    /// The question before a write to a service: what the write does. The
    /// host adds the mode's note before `y` when the mode refuses it
    /// ([`crate::writes`]).
    pub change: Option<crate::writes::Change>,
    /// A question: drawn as a dialog, its notes then its choices as
    /// buttons in a row at the bottom right (each its key and its hint),
    /// `←`/`→` moving along them; not as a list. Set by [`Menu::confirm`],
    /// [`Menu::write_confirm`] and [`Menu::write_plain`].
    pub question: bool,
    /// The value of the item the highlight opens on, when not the first
    /// (a pick menu a part re-opens after each choice keeps its place).
    pub at: Option<String>,
    /// A field to type into, with this text to start ([`Menu::typed`]).
    pub typed: Option<String>,
}

/// What a part says of the text typed into a menu's field
/// ([`Menu::typed`], [`Plugin::typing`]): a verdict, and what to offer
/// under the field (picks whose value fills the field).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Typing {
    pub verdict: Option<Verdict>,
    pub offers: Vec<MenuItem>,
}

/// Typed text, as you type: taken, taken with a warning, or refused (`⏎`
/// then says why and keeps the field open).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Good(String),
    Unsure(String),
    Bad(String),
}

/// A notice's explicitly chosen tone, words and optional icon. Reads as its words
/// (`message.as_deref() == Some("saved")`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub tone: crate::view::chrome::NoticeTone,
    pub icon: Option<Icon>,
    pub text: String,
}

impl Message {
    pub fn new(icon: Option<Icon>, text: impl Into<String>) -> Self {
        Self {
            tone: crate::view::chrome::NoticeTone::Info,
            icon,
            text: text.into(),
        }
    }

    pub fn error(icon: Option<Icon>, text: impl Into<String>) -> Self {
        Self { tone: crate::view::chrome::NoticeTone::Error, icon, text: text.into() }
    }
}

/// Its words (the icon is drawn, not written).
impl std::fmt::Display for Message {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

impl std::ops::Deref for Message {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

impl From<String> for Message {
    fn from(text: String) -> Self {
        Self::new(None, text)
    }
}

impl From<&str> for Message {
    fn from(text: &str) -> Self {
        text.to_string().into()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuItem {
    /// Single key that chooses the item; `None` for list-style menus.
    pub key: Option<char>,
    /// What the key column shows: the key, or a name like `space` or `⏎`.
    /// Empty on a heading, which cannot be chosen.
    pub keys: String,
    pub heading: bool,
    /// A heading drawn as written, not as a heading: a line of what an
    /// action will send, shown in its confirm.
    pub note: bool,
    pub label: String,
    /// What choosing it does, in a few words: a menu's line after the
    /// label, a question's button.
    pub hint: String,
    /// A style spec to draw a sample of the item in (colour pickers).
    pub sample: Option<String>,
    /// Passed back to [`Plugin::on_menu`].
    pub value: String,
    /// An icon before the label (a heading's or an item's), drawn by the
    /// host in its own colour; nothing with icons off.
    pub icon: Option<Icon>,
    /// Drawn muted: it can be chosen, but will not do all it says now
    /// (a write the mode refuses).
    pub muted: bool,
}

impl MenuItem {
    pub fn new(
        key: char,
        label: impl Into<String>,
        hint: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            key: Some(key),
            keys: key_text(key),
            heading: false,
            note: false,
            label: label.into(),
            hint: hint.into(),
            sample: None,
            value: value.into(),
            icon: None,
            muted: false,
        }
    }

    /// An item chosen by moving to it, not by a key.
    pub fn pick(
        label: impl Into<String>,
        hint: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            key: None,
            keys: String::new(),
            heading: false,
            note: false,
            label: label.into(),
            hint: hint.into(),
            sample: None,
            value: value.into(),
            icon: None,
            muted: false,
        }
    }

    /// A heading over the items that follow; moving skips it.
    pub fn heading(text: impl Into<String>) -> Self {
        Self {
            heading: true,
            ..Self::pick(text, "", "")
        }
    }

    /// A line of text shown as written (case, spaces and all), never
    /// chosen: what a confirm is about to send.
    pub fn note(text: impl Into<String>) -> Self {
        Self {
            note: true,
            ..Self::heading(text)
        }
    }

    /// Show `keys` in the key column, e.g. `⏎` for an item `⏎` chooses
    /// from outside the menu.
    pub fn shown_as(mut self, keys: impl Into<String>) -> Self {
        self.keys = keys.into();
        self
    }

    pub fn sample(mut self, spec: impl Into<String>) -> Self {
        self.sample = Some(spec.into());
        self
    }

    /// An icon before the label, in its own colour (`None` for none, as
    /// [`crate::ui::icon::plain_or`] gives with icons off).
    pub fn icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.icon = icon.into();
        self
    }
}

/// Keep meaningful confirmation wording while making its yes/no mnemonic visible.
fn answer_words(answer: &str, words: impl Into<String>) -> String {
    let words = words.into();
    if words.is_empty() { answer.into() }
    else if words == answer || words.starts_with(&format!("{answer},")) || words.starts_with(&format!("{answer} ")) { words }
    else { format!("{answer}, {words}") }
}

impl Menu {
    pub fn new(id: impl Into<String>, title: impl Into<String>, items: Vec<MenuItem>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            items,
            icon: None,
            change: None,
            question: false,
            at: None,
            typed: None,
        }
    }

    /// A field to type an answer into (a folder, a name), starting with
    /// `value`: the part's [`Plugin::typing`] says what it makes of the text
    /// as it changes and offers choices under it (`↑`/`↓` put one in the
    /// field, `tab` takes the first and asks again, a click puts one in);
    /// `⏎` gives the text to [`Plugin::on_menu`] unless the verdict refuses
    /// it, `esc` closes. Paste works; every key goes to the field.
    pub fn typed(
        id: impl Into<String>,
        title: impl Into<String>,
        value: impl Into<String>,
    ) -> Self {
        Self {
            typed: Some(value.into()),
            ..Self::new(id, title, vec![])
        }
    }

    /// Open with the highlight on the item whose value is `value`.
    pub fn at(mut self, value: impl Into<String>) -> Self {
        self.at = Some(value.into());
        self
    }

    /// An icon before the title, in its own colour.
    pub fn icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    /// The write this question is before adds something
    /// ([`crate::writes::Change::Add`]).
    pub fn adds(mut self) -> Self {
        self.change = Some(crate::writes::Change::Add);
        self
    }

    /// The write this question is before changes something there
    /// ([`crate::writes::Change::Edit`]).
    pub fn edits(mut self) -> Self {
        self.change = Some(crate::writes::Change::Edit);
        self
    }

    /// Ask a yes/no question, drawn as a dialog: `y` answers `yes`, `n`
    /// answers `no`. `yes` and `no` are the buttons' words: what each
    /// choice does, in a few words ("delete it", "keep it"); what will
    /// happen goes before them with [`Menu::saying`].
    pub fn confirm(
        id: impl Into<String>,
        question: impl Into<String>,
        yes: impl Into<String>,
        no: impl Into<String>,
    ) -> Self {
        Self::new(
            id,
            question,
            vec![
                MenuItem::new('y', "yes", answer_words("yes", yes), "yes"),
                MenuItem::new('n', "no", answer_words("no", no), "no"),
            ],
        )
        .asking()
    }

    /// The question before a write that sends text (`{verb} this on
    /// {item}?`): the whole text as notes, as it will be sent, cut into
    /// lines of 72 cells; then the buttons `y` (`yes`, the hint `{verb}
    /// it`), `e` keep writing (`edit`) and `n` (`no`, usually `drop it`).
    /// See [`crate::writes`].
    pub fn write_confirm(
        id: impl Into<String>,
        question: impl Into<String>,
        text: &str,
        yes: impl Into<String>,
        no: impl Into<String>,
    ) -> Self {
        let mut items: Vec<MenuItem> = note_lines(text, 72)
            .into_iter()
            .map(MenuItem::note)
            .collect();
        items.push(MenuItem::new('y', "yes", answer_words("yes", yes), "yes"));
        items.push(MenuItem::new('e', "edit", "keep writing", "edit"));
        items.push(MenuItem::new('n', "no", answer_words("no", no), "no"));
        Self::new(id, question, items).adds().asking()
    }

    /// The question before a write that sends no text (approve, move,
    /// react): what it does as notes, then the buttons `y` (`yes`) and `n`
    /// (`no`). It edits unless it says it adds (`.adds()`, a reaction).
    pub fn write_plain(
        id: impl Into<String>,
        question: impl Into<String>,
        notes: Vec<String>,
        yes: impl Into<String>,
        no: impl Into<String>,
    ) -> Self {
        let mut items: Vec<MenuItem> = notes.into_iter().map(MenuItem::note).collect();
        items.push(MenuItem::new('y', "yes", answer_words("yes", yes), "yes"));
        items.push(MenuItem::new('n', "no", answer_words("no", no), "no"));
        Self::new(id, question, items).edits().asking()
    }

    /// Drawn as a dialog: a question ([`Menu::question`]).
    fn asking(mut self) -> Self {
        self.question = true;
        self
    }

    /// `line` as the last line before the choices: what will happen ("it
    /// leaves the nav pane"), under the question.
    pub fn saying(self, line: impl Into<String>) -> Self {
        self.noting(Some(line.into()))
    }

    /// `note`, when there is one, as the last line before the choices:
    /// what will happen ([`Menu::saying`]), or a write's question the mode
    /// refuses ([`crate::writes::confirm_note`]).
    pub fn noting(mut self, note: Option<String>) -> Self {
        if let Some(note) = note {
            let at = self
                .items
                .iter()
                .position(|i| !i.note && !i.heading)
                .unwrap_or(self.items.len());
            self.items.insert(at, MenuItem::note(note));
        }
        self
    }
}

/// `text`'s lines cut into lines of at most `w` cells, exactly as written
/// otherwise (spaces kept, an empty line kept).
pub fn note_lines(text: &str, w: usize) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.split('\n') {
        let mut rest = line.trim_end_matches('\r');
        if rest.is_empty() {
            out.push(String::new());
        }
        while !rest.is_empty() {
            let (head, _) = crate::ui::take(rest, w.max(1));
            // A character wider than the line still goes on one.
            let head = if head.is_empty() {
                rest.char_indices().nth(1).map_or(rest, |(i, _)| &rest[..i])
            } else {
                head
            };
            out.push(head.to_string());
            rest = &rest[head.len()..];
        }
    }
    out
}

impl Effect {
    /// Semantic failure, never inferred from the words or chosen icon.
    pub fn error(icon: Option<Icon>, words: impl Into<String>) -> Self {
        Self::Notice(Message::error(icon, words))
    }
    /// [`Effect::NotifyWith`] when there is an icon, else [`Effect::Notify`]
    /// (with [`crate::ui::icon::plain_or`]'s symbol in the words).
    pub fn notify(icon: Option<Icon>, words: impl Into<String>) -> Self {
        match icon {
            Some(i) => Self::NotifyWith(i, words.into()),
            None => Self::Notify(words.into()),
        }
    }
}

impl std::fmt::Debug for Effect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Work { .. } => f.write_str("Work(..)"),
            Self::ResolveSources { .. } => f.write_str("ResolveSources(..)"),
            Self::ShowSource(_) => f.write_str("ShowSource(..)"),
            Self::Journal(_) => f.write_str("Journal(..)"),
            Self::Report(outcome) => write!(f, "Report({outcome:?})"),
            Self::Spawn(_) => f.write_str("Spawn(..)"),
            Self::SpawnAction { .. } => f.write_str("SpawnAction(..)"),
            Self::Observe { outcome, .. } => write!(f, "Observe({outcome:?})"),
            Self::SendToAgent(_) => f.write_str("SendToAgent(..)"),
            Self::ShowDock => f.write_str("ShowDock"),
            Self::HideDock => f.write_str("HideDock"),
            Self::StartReview(r) => write!(f, "StartReview({} changes)", r.changes.len()),
            Self::Open(u) => write!(f, "Open({u})"),
            Self::OpenApp { link, web } => write!(f, "OpenApp({link}, else {web})"),
            Self::Copy(t) => write!(f, "Copy({t})"),
            Self::Notify(m) => write!(f, "Notify({m})"),
            Self::NotifyWith(i, m) => write!(f, "Notify({}: {m})", i.name()),
            Self::Notice(m) => write!(f, "Notice({:?}: {m})", m.tone),
            Self::Emit { topic, payload } => write!(f, "Emit({topic}, {payload})"),
            Self::PersistCache => f.write_str("PersistCache"),
            Self::Menu(m) => write!(f, "Menu({})", m.id),
            Self::Focus(p) => write!(f, "Focus({p:?})"),
            Self::Show(s) => write!(f, "Show({s:?})"),
            Self::Changed(c) => write!(f, "Changed({:?}, added {:?})", c.changed, c.added),
            Self::Press(c) => write!(f, "Press({c})"),
            Self::Act { part, subject, id } => write!(f, "Act({part}, {}, {id})", subject.id),
        }
    }
}

/// Whether a plugin used a key. Unused keys fall back to the host's own
/// navigation (moving between panels, leaving the detail panel).
#[derive(Debug)]
pub enum Outcome {
    Ignored,
    Handled(Vec<Effect>),
}

impl Outcome {
    pub fn done() -> Self {
        Self::Handled(vec![])
    }
}

/// What a running local review needs another part to do, one step at a
/// time. The part that owns the dock runs the journey and asks; the host
/// finds the part that can answer ([`Plugin::resolve_change`],
/// [`Plugin::ensure_worktree`], [`Plugin::save_note`]), so no part names
/// another.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewAsk {
    Resolve(crate::agent_context::ChangeResolveRequest),
    Worktree(crate::agent_context::WorktreeRequest),
    SaveNote(NoteRequest),
}

/// The answer to a [`ReviewAsk`].
#[derive(Debug, Clone, PartialEq, Eq)]
// One answer per step of a review, moved once.
#[allow(clippy::large_enum_variant)]
pub enum ReviewAnswer {
    Resolved(crate::agent_context::Resolve),
    Worktree(Result<crate::agent_context::WorktreeReady, crate::agent_context::WorktreeError>),
    /// Where the note was written; None when no part keeps notes here.
    NoteSaved(Option<std::path::PathBuf>),
}

/// A note to keep with the work item it is about, in the place the
/// workspace keeps notes (`.okf.yaml`'s create location).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteRequest {
    /// The work item the note is about.
    pub subject: crate::journal::SubjectKey,
    /// A file name, with no folder.
    pub file_name: String,
    pub text: String,
}

/// The three panels of the universal layout, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Nav,
    Middle,
    Detail,
}

/// The held subject and configured-connection incarnation of a preview.
/// Private to host admission: never serialize, journal, or include secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetailIdentity {
    pub connection: u64,
    pub subject: Vec<String>,
}

/// A screen a plugin contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenSpec {
    /// Stable id, unique within the plugin.
    pub id: &'static str,
    /// Shown in the app bar path: `radar / <name> / …`.
    pub name: &'static str,
    /// Shift+letter that switches to it (TUI-K3). Unique across all plugins.
    pub key: char,
}

/// Several provider screens shown under one place in the top bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenGroup {
    pub id: &'static str,
    pub name: &'static str,
    pub key: char,
    /// Label for this provider in the source row inside the group.
    pub source: &'static str,
}

/// A settings section a plugin contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionSpec {
    pub id: &'static str,
    /// The nav row and the middle pane's title. Unique under its group (so
    /// each part can have a `connection`); a name two groups share is
    /// titled with the group's name in front.
    pub name: &'static str,
    /// The heading it is listed under in the settings nav, e.g. the part's id.
    pub group: &'static str,
}

/// Where a part's step is listed in setup's chooser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SetupGroup {
    /// This machine: what radar reads here (checkouts, the terminal).
    Machine,
    /// A service the part syncs with.
    #[default]
    Service,
    /// An agent the part runs.
    Agent,
}

/// A part's step in setup ([`Plugin::setup_step`]): one stop on the rail,
/// one row in the chooser, and one of its settings sections as the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupStep {
    /// What `radar setup <id>` names it by; unique among the steps.
    pub id: &'static str,
    /// The rail's and the chooser's word for it.
    pub label: &'static str,
    pub icon: Option<crate::ui::icon::Icon>,
    /// Which of its settings sections ([`Plugin::sections`]) is the card.
    pub section: &'static str,
    /// The chooser's muted words after the label (`rows, checks`).
    pub about: &'static str,
    /// What it holds now, in one line, for the rail and the ready screen
    /// (`@vimes · 2 groups`); `None` while there is nothing to say.
    pub summary: Option<String>,
    pub group: SetupGroup,
    /// The card's first line (`Connect Shelves.`) and the muted line under it.
    pub heading: &'static str,
    pub lead: &'static str,
    /// Rows of its own in setup's chooser, one per thing it sets up (an
    /// agent's provider), instead of one row for the step: ticking any of
    /// them takes the step.
    pub options: Vec<SetupOption>,
}

/// One of a step's rows in setup's chooser ([`SetupStep::options`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetupOption {
    pub id: String,
    pub label: String,
    pub icon: Option<crate::ui::icon::Icon>,
    /// Its state at the right (`signed in`, `not installed`).
    pub note: Option<String>,
    /// Ready to use: the look for keys ticks it.
    pub ready: bool,
}

impl SetupStep {
    pub fn new(id: &'static str, label: &'static str, section: &'static str) -> Self {
        Self {
            id,
            label,
            icon: None,
            section,
            about: "",
            summary: None,
            group: SetupGroup::default(),
            heading: "",
            lead: "",
            options: vec![],
        }
    }

    pub fn option(mut self, option: SetupOption) -> Self {
        self.options.push(option);
        self
    }

    pub fn heading(mut self, heading: &'static str, lead: &'static str) -> Self {
        self.heading = heading;
        self.lead = lead;
        self
    }

    pub fn icon(mut self, icon: crate::ui::icon::Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn about(mut self, about: &'static str) -> Self {
        self.about = about;
        self
    }

    pub fn summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    pub fn group(mut self, group: SetupGroup) -> Self {
        self.group = group;
        self
    }
}

/// Where setup may look for a part's keys, once the user said yes
/// ([`Plugin::setup_finds`]). Only names are ever read or kept: a
/// variable's name, whether a file exists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SetupFinds {
    /// Environment variable names: exact (`SHELF_TOKEN`), or a prefix
    /// ending in `_` (`SHELF_` matches `SHELF_URL`).
    pub env: Vec<&'static str>,
    /// A program's sign-in: the program, and its file's path under the
    /// home folder (`.config/shelf/hosts.yml`). Only its existence counts.
    pub files: Vec<(&'static str, &'static str)>,
}

/// One row of the keys menu (`x` or `?`, after lazydocker's): what to press
/// and what it does. Choosing the row presses the key for you.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// As shown: `a`, `space`, `⏎`, `pgdn`. Empty for a heading.
    pub keys: String,
    pub what: String,
    /// An icon before the words (a heading's: what the actions are for),
    /// drawn by the host in its own colour.
    pub icon: Option<Icon>,
    /// A write to a service: what it does. While the mode refuses it the
    /// host mutes the row and says why (`· needs edit mode`); its key
    /// still opens the question.
    pub change: Option<crate::writes::Change>,
}

impl Binding {
    pub fn new(keys: impl Into<String>, what: impl Into<String>) -> Self {
        let keys = keys.into();
        Self {
            keys,
            what: what.into(),
            icon: None,
            change: None,
        }
    }

    /// A heading over the rows that follow. A heading first in the list
    /// names the menu instead: what the actions are for (`!263`).
    pub fn heading(text: impl Into<String>) -> Self {
        Self {
            keys: String::new(),
            what: text.into(),
            icon: None,
            change: None,
        }
    }

    /// The row's key writes something new to a service.
    pub fn adds(mut self) -> Self {
        self.change = Some(crate::writes::Change::Add);
        self
    }

    /// The row's key changes something there on a service.
    pub fn edits(mut self) -> Self {
        self.change = Some(crate::writes::Change::Edit);
        self
    }

    /// An icon before the words, in its own colour: a heading's names what
    /// the menu is about (`Binding::heading("!263").icon(Icon::Change)`).
    pub fn icon(mut self, icon: impl Into<Option<Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn is_heading(&self) -> bool {
        self.keys.is_empty()
    }
}

/// One key, optionally prefixed by `ctrl-`, as shown in an actions menu.
pub fn parse_key(keys: &str) -> Option<KeyEvent> {
    let (keys, modifiers) = keys
        .strip_prefix("ctrl-")
        .map_or((keys, KeyModifiers::NONE), |key| {
            (key, KeyModifiers::CONTROL)
        });
    let code = match keys {
        "⏎" | "enter" => KeyCode::Enter,
        "esc" => KeyCode::Esc,
        "space" => KeyCode::Char(' '),
        "tab" => KeyCode::Tab,
        "shift-tab" => KeyCode::BackTab,
        "backspace" => KeyCode::Backspace,
        "delete" => KeyCode::Delete,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "←" => KeyCode::Left,
        "→" => KeyCode::Right,
        "↑" => KeyCode::Up,
        "↓" => KeyCode::Down,
        "pgup" => KeyCode::PageUp,
        "pgdn" => KeyCode::PageDown,
        _ => {
            let mut chars = keys.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => KeyCode::Char(c),
                _ => return None,
            }
        }
    };
    Some(KeyEvent::new(code, modifiers))
}

/// How a key is shown in the keys menu: `' '` is `space`.
pub fn key_text(c: char) -> String {
    if c == ' ' {
        "space".into()
    } else {
        c.to_string()
    }
}

/// An item page's row of tabs, for the host to move across
/// ([`Plugin::tabs`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTabs {
    /// Names only, without icons or counts, shared by tab navigation and menus.
    pub names: Vec<String>,
    /// The tab showing.
    pub on: usize,
}

/// The feeling of a count or label, mapped to a theme slot by the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Plain,
    Muted,
    Accent,
    Info,
    Good,
    Warn,
    Bad,
}

/// The moment demo data is relative to (2026-10-06 10:16:40 UTC): the host
/// for `radar shot` is built with it, and a part's [`Plugin::demo`] sets its
/// own clock to it, so a picture is the same every day.
pub const DEMO_NOW: i64 = 1_791_271_000;

pub trait Plugin: Send {
    /// Stable id: the config table name, the cache file name and the CLI
    /// subcommand.
    fn id(&self) -> &'static str;
    /// The part's name for people: the parts section, and the heading over
    /// the actions it adds to others' menus. Defaults to the id.
    fn name(&self) -> &'static str {
        self.id()
    }

    // --- joining the other parts (see `compose`) -------------------------

    /// Whether this part can work yet. Until it can, its screen shows a
    /// short set-up page and it adds nothing to other parts.
    fn set_up(&self) -> Setup {
        Setup::Ready
    }
    /// Safe, provider-scoped identity for a subject this part owns. Never
    /// include labels, text, credentials or query parameters. The host asks
    /// only the ready owning part; None explicitly marks omitted linkage.
    /// The host assigns invocation roles: the selected subject is Primary,
    /// supplied references are Context. A Context snapshot establishes neither
    /// remote existence nor actual use, identity aliasing or a relation claim.
    fn journal_subject(&self, _subject: &Subject) -> Option<crate::journal::SubjectSnapshot> {
        None
    }

    /// Cached destinations for the host's cross-part command palette. Pure local
    /// inspection only: no I/O or credential discovery. Return only subjects this
    /// part can currently show, with public search words and stable identities.
    fn palette_targets(&self) -> Vec<crate::palette::PaletteTarget> {
        Vec::new()
    }

    /// Reconstruct a routable source from this owner's exact persisted locator.
    /// The host validates the roundtrip; returning None never guesses a link.
    fn subject_from_journal(&self, _key: &crate::journal::SubjectKey) -> Option<Subject> {
        None
    }

    /// The kinds of subject this part owns (shows and recognises), each
    /// with what people call them and whether it has a page for one.
    fn owns(&self) -> Vec<Owned> {
        vec![]
    }
    /// Subjects of the kinds it owns found in `text` (a link, a key), as
    /// byte ranges. Only kinds in [`Plugin::owns`] count.
    fn recognise(&self, _text: &str) -> Vec<(std::ops::Range<usize>, Subject)> {
        vec![]
    }
    /// Show a subject of a kind it owns.
    fn show(&mut self, _subject: &Subject) -> Showing {
        Showing::Cannot
    }
    /// How many pages deep the screen is (0: its list). The host uses it to
    /// come back across parts: after a [`Showing::Page`], `esc` back past
    /// that page returns to where the show came from.
    fn depth(&self, _screen: &str) -> usize {
        0
    }
    /// Opaque identity of the actual item page open on `screen`, read from
    /// the same page state as [`Plugin::view`]. Keep it stable across title,
    /// tab, selection and refresh changes; distinct items must differ even
    /// at the same stack depth. The host supplies the plugin/screen/tab scope.
    /// Never substitute a label, stack depth or a selected child subject.
    /// `None` means no explicit identity: an open page still draws, but the
    /// host must not infer its identity for page-scoped motion. Never fetch.
    fn page_identity(&self, _screen: &str) -> Option<String> {
        None
    }
    /// The item page open on `screen`, when it has a row of tabs: their
    /// names and the one showing. The host moves across them (GUIDE 1.5):
    /// `← →` `h l` past the edge of the page's panes, `tab` where the page
    /// has one pane, and from the row itself once `↑` past the top of the
    /// body gave it the keys (the page tab row). `None` on the screen's own
    /// panes.
    fn tabs(&self, _screen: &str) -> Option<PageTabs> {
        None
    }
    /// Show the page's tab `i` (see [`Plugin::tabs`]): the host asks.
    fn show_tab(&mut self, _screen: &str, _i: usize) -> Vec<Effect> {
        vec![]
    }
    /// The agent dock's content, from the one part that owns agent sessions
    /// (tabs, the active transcript, composer, status line, pickers). The
    /// host lays its own state (shown, focus, height) over it and sends it
    /// on every screen. Read only what the part holds; never fetch here.
    fn dock(&self) -> Option<crate::dock::DockView> {
        None
    }
    /// The dock's tabs and the active tab's id, without the transcript: what
    /// the status line's attention chip reads on every frame. A part that
    /// builds its dock at some cost answers this more cheaply.
    fn dock_tabs(&self) -> Option<(Vec<crate::dock::DockTab>, Option<String>)> {
        self.dock().map(|d| (d.tabs, d.active))
    }
    /// The host shows or hides the dock: a turn that ends in the tab shown
    /// while the dock is visible has been seen; one that ends out of sight
    /// is unread ([`crate::dock::DockTab::unread`]).
    fn dock_visible(&mut self, _visible: bool) {}
    /// What the user did in the dock, other than the host's own state
    /// (show, hide, focus, resize). Only the part that returns a dock gets
    /// these. Nothing is answered for the user: a permission request waits
    /// until a renderer sends [`crate::dock::DockIntent::Approve`] or
    /// `Deny`.
    fn dock_intent(&mut self, _intent: &crate::dock::DockIntent) -> Outcome {
        Outcome::Ignored
    }
    /// Items sent to an agent: the contexts the owning parts gave, for the
    /// part that supplies the dock to write the opening prompt from and
    /// start a session with, sending it at once.
    fn agent_brief(&mut self, _contexts: Vec<crate::agent_context::AgentContext>) -> Outcome {
        Outcome::Ignored
    }
    /// Start a local review of one change request (dock owner only).
    /// `origin` is the context of the row it was asked on.
    fn review_start(
        &mut self,
        _origin: Option<crate::agent_context::AgentContext>,
        _change: crate::journal::SubjectKey,
    ) -> Outcome {
        Outcome::Ignored
    }
    /// What the running reviews wait for, by run; each is asked once.
    fn review_asks(&mut self) -> Vec<(u64, ReviewAsk)> {
        vec![]
    }
    /// The answer to an ask of `run`.
    fn review_answer(&mut self, _run: u64, _answer: ReviewAnswer) -> Outcome {
        Outcome::Ignored
    }
    /// Keep a note with its work item, where this part keeps notes. None
    /// when it does not.
    fn save_note(&mut self, _request: &NoteRequest) -> Option<Result<std::path::PathBuf, String>> {
        None
    }
    /// What the focused pane is about, for the actions other parts add to
    /// its `x` menu (and direct keys for them).
    fn subject(&self, _screen: &str, _pane: Pane) -> Option<Subject> {
        None
    }
    /// The places it offers others, for the parts section.
    fn offers(&self) -> Vec<Offer> {
        vec![]
    }
    /// What it adds to others, for the parts section.
    fn adds(&self) -> Vec<Adds> {
        vec![]
    }
    /// Fill `point` on `subject`, drawn by another part. Read only what this
    /// part already holds (its syncs, the bus); never fetch here: it is
    /// called while drawing.
    fn contribute(&self, _point: Point, _subject: &Subject) -> Option<Contribution> {
        None
    }
    /// Subjects related to `subject`, from what this part already holds.
    /// Never fetch or block here. The host gathers these from the other
    /// set-up parts before a contributed action runs.
    fn related(&self, _subject: &Subject) -> Vec<Subject> {
        vec![]
    }
    /// One of the actions it contributed was chosen, on `subject`. The
    /// default [`Plugin::act_with_related`] calls this.
    fn act(&mut self, _subject: &Subject, _action: &str) -> Vec<Effect> {
        vec![]
    }
    /// A contributed action, with the related subjects the other set-up
    /// parts know, once per kind and id in plugin order. By default, keeps
    /// the existing [`Plugin::act`] behavior.
    fn act_with_related(
        &mut self,
        subject: &Subject,
        action: &str,
        _related: &[Subject],
    ) -> Vec<Effect> {
        self.act(subject, action)
    }

    // --- config ---------------------------------------------------------

    /// Take this plugin's `[<id>]` table; `None` when the file has none.
    fn configure(&mut self, table: Option<&toml::Value>) -> anyhow::Result<()>;
    /// The `[<id>]` table to write, or `None` for nothing.
    fn config(&self) -> Option<toml::Value>;
    /// Called when there is no config file yet. Returns a note for the user,
    /// e.g. what was imported.
    fn first_run(&mut self) -> Option<String> {
        None
    }

    // --- start-up -------------------------------------------------------

    /// Domain slots this plugin draws with, and their defaults. The user can
    /// override any of them in `[theme.slots]`; plugins never hold colours.
    fn slots(&self) -> Vec<(String, SlotValue)> {
        vec![]
    }
    fn restore(&mut self, _cache: Value) {}
    fn snapshot(&self) -> Option<Value> {
        None
    }

    // --- events ---------------------------------------------------------

    /// Every 200 ms (and on resize), with the current unix time. Throttle
    /// real work yourself.
    fn tick(&mut self, _now: i64) -> Vec<Effect> {
        vec![]
    }
    /// The host's [`crate::wake::Waker`] for this part, given once at start. A part whose
    /// background work has something to show (an agent's streamed text) calls
    /// [`crate::wake::Waker::wake`] from any thread, and the host calls
    /// [`Plugin::woken`] soon after, instead of waiting for the next tick.
    fn set_waker(&mut self, _waker: crate::wake::Waker) {}
    /// This part woke the host ([`Plugin::set_waker`]): take in what its work brought.
    /// Wakes close together come as one call (at most one per [`crate::wake::COALESCE`]).
    fn woken(&mut self, _now: i64) -> Vec<Effect> {
        vec![]
    }
    /// Someone is there: a key, a click or a paste reached radar, on any
    /// screen, after it was handled. For pausing background work while
    /// nobody is looking, and resuming it when they are back.
    fn input(&mut self, _now: i64) {}
    fn on_msg(&mut self, _msg: Msg) -> Vec<Effect> {
        vec![]
    }
    /// Another plugin emitted `topic`.
    fn on_bus(&mut self, _topic: &str, _payload: &Value) -> Vec<Effect> {
        vec![]
    }
    /// Text was pasted while this plugin is [`capturing`](Plugin::capturing):
    /// insert it into the input being edited.
    fn paste(&mut self, _text: &str) -> Vec<Effect> {
        vec![]
    }
    /// True while the plugin is taking text input; every key then goes to it,
    /// including the host's own (`q`, `?`, screen keys).
    fn capturing(&self) -> bool {
        false
    }

    // --- screens --------------------------------------------------------

    fn screens(&self) -> Vec<ScreenSpec> {
        vec![]
    }

    fn screen_group(&self, _screen: &str) -> Option<ScreenGroup> {
        None
    }
    /// The icon on the screen's tab (drawn from the pack in use; none when
    /// icons are off).
    fn icon(&self, _screen: &str) -> Option<crate::ui::icon::Icon> {
        None
    }
    /// A summary for the host, including the splash. Warn/Bad also mark the
    /// screen tab as needing attention; tabs show a dot, never this count.
    fn badge(&self, _screen: &str) -> Option<(String, Tone)> {
        None
    }
    /// Right-hand side of the top bar, e.g. when the screen last synced.
    fn top_right(&self, _screen: &str) -> Option<String> {
        None
    }
    /// True while the screen is loading; the top bar shows a spinner.
    fn busy(&self, _screen: &str) -> bool {
        false
    }
    /// Counts for the right of the status line.
    fn status(&self, _screen: &str) -> Vec<(String, Tone)> {
        vec![]
    }
    /// How its syncing goes, source by source, for its chip at the far
    /// right of the status line ([`crate::health`]); `None` for a part that
    /// syncs with no service. Read only what the part holds.
    fn health(&self) -> Option<crate::health::Health> {
        None
    }
    /// Whether its name is on the startup splash, among the services radar
    /// brings together ([`crate::splash`]): by default a part that syncs
    /// with a service ([`Plugin::health`]).
    fn on_splash(&self) -> bool {
        self.health().is_some()
    }
    /// Sync every source now, a waiting one too (its chip's `refresh now`).
    fn refresh_now(&mut self) -> Vec<Effect> {
        vec![]
    }
    /// What an agent should be told about one of this part's subjects
    /// ([`crate::agent_context`]). Answer from what the part already holds;
    /// never fetch. None when it is not this part's subject or holds nothing.
    fn agent_context(
        &mut self,
        _request: &crate::agent_context::AgentContextRequest,
    ) -> Option<crate::agent_context::AgentContext> {
        None
    }
    /// Resolve a change request this part owns for a review: branches, head
    /// sha, diff stats, description, open threads. Never blocks: the first
    /// ask may answer `Pending` and spawn the fetch (journaled as a read);
    /// the host asks again until `Ready` or `Failed`. None: not this part's.
    fn resolve_change(
        &mut self,
        _request: &crate::agent_context::ChangeResolveRequest,
    ) -> Option<(crate::agent_context::Resolve, Vec<Effect>)> {
        None
    }
    /// Make, or reuse, a worktree of a branch at a sha in the configured
    /// worktree place. None when this part does not own repositories.
    fn ensure_worktree(
        &mut self,
        _request: &crate::agent_context::WorktreeRequest,
    ) -> Option<(
        Result<crate::agent_context::WorktreeReady, crate::agent_context::WorktreeError>,
        Vec<Effect>,
    )> {
        None
    }
    /// The screen as renderer-neutral data ([`crate::view`]). A part that
    /// gives one is drawn and driven through it, whatever the frontend;
    /// what the user does arrives at [`Plugin::intent`].
    /// Read only what the part holds; never fetch here.
    fn view(
        &self,
        _screen: &str,
        _cx: &crate::view::ViewCx<'_>,
    ) -> Option<crate::view::ScreenView> {
        None
    }
    /// Exact current preview subject for host-owned foot-control admission.
    /// Read held state only; return None on pages or without an eligible subject.
    fn detail_identity(&self, _scope: &str, _pane: Pane) -> Option<DetailIdentity> {
        None
    }
    /// As [`Plugin::view`], for a settings section: its body, detail and
    /// actions are renderer-neutral, and interaction arrives as an intent.
    fn section_view(
        &self,
        _section: &str,
        _cx: &crate::view::ViewCx<'_>,
    ) -> Option<crate::view::SectionView> {
        None
    }
    /// What the user did on a screen or section the part describes with a
    /// view (`scope` is its id; `pane` has the focus, `Middle` in a
    /// section). The same intent arrives from any frontend.
    fn intent(&mut self, _scope: &str, _pane: Pane, _intent: &crate::view::Intent) -> Outcome {
        Outcome::Ignored
    }
    /// How the pictures in a view's rich text are fetched (the `pictures`
    /// a [`crate::view::Node::Rich`] names): with the part's own account,
    /// sent only where it belongs. Asked while drawing; never fetch here.
    fn pictures(&self, _source: &str) -> Option<crate::ui::image::Fetch> {
        None
    }

    /// The panes this screen can focus, left to right (`tab` cycles them).
    fn panes(&self, _screen: &str) -> Vec<Pane> {
        vec![Pane::Nav, Pane::Middle, Pane::Detail]
    }
    /// The status line's keys while the plugin is [`capturing`](Plugin::capturing),
    /// e.g. `⏎ keep  esc cancel`. Otherwise the status line is lazydocker's
    /// fixed bar (scroll, bulk, quit, menu, navigate) and every other key is
    /// in the `x` menu.
    fn hints(&self, _screen: &str, _pane: Pane) -> Vec<KeyHint<'static>> {
        vec![]
    }
    /// Whether `pane` of the screen shows a tree with groups that fold
    /// (`+` opens, `-` closes, as [`crate::ui::tree::key`] does): the
    /// status line then shows `+/- expand/collapse` and the `x` menu lists
    /// both keys.
    fn folds(&self, _screen: &str, _pane: Pane) -> bool {
        false
    }
    /// Fold actions that apply to the selected row in the actions menu.
    /// Trees with leaf rows can narrow this beyond the pane-wide [`Plugin::folds`].
    fn fold_keys(&self, screen: &str, pane: Pane) -> Vec<Binding> {
        if self.folds(screen, pane) {
            vec![
                Binding::new("+", "expand the group"),
                Binding::new("-", "collapse the group, or the one this row is in"),
                Binding::new("=", "expand every held group"),
            ]
        } else {
            Vec::new()
        }
    }
    /// As [`Plugin::folds`], for a settings section.
    fn section_folds(&self, _section: &str) -> bool {
        false
    }
    /// Keys for what is selected, put first on the fixed status line while
    /// the plugin is not capturing: only those that apply to the selection
    /// now (`u rebase`, `m merge`). Few, as the line is short; the rest are
    /// in the `x` menu.
    fn row_hints(&self, _screen: &str, _pane: Pane) -> Vec<KeyHint<'static>> {
        vec![]
    }
    /// Bulk commands (`b`, after lazydocker's) for a screen or a settings
    /// section, by its id: things done to everything it shows at once. The
    /// chosen item's value comes back through [`Plugin::on_menu`] as menu
    /// `bulk`.
    fn bulk(&self, _scope: &str) -> Vec<MenuItem> {
        vec![]
    }
    /// The actions menu (`x`) for the focused pane: what can be done here,
    /// to what is selected, and nothing else. Start with a heading naming
    /// the selection. Each row's key must work when pressed (the host drops
    /// rows it cannot press); moving about is not listed, it is on the
    /// status line.
    fn keys(&self, _screen: &str, _pane: Pane) -> Vec<Binding> {
        vec![]
    }

    /// Optional ownership of one key offered in the host's `x` menu. `scope`
    /// is the screen or settings section id. The host captures this token when
    /// opening the menu and requires the same token before replaying the key.
    /// A changed or missing token refuses that choice and asks to reopen `x`.
    /// Direct shortcuts and plugin-owned menus are unaffected.
    ///
    /// Read only cheap, local state. Include the exact target and connection
    /// revision where relevant, not loading status or a general refresh counter:
    /// an unchanged target should survive background reads. Tokens are transient,
    /// opaque to the host, and never displayed or journalled; never include secrets.
    fn menu_key_context(&self, _scope: &str, _pane: Pane, _action: &str) -> Option<String> {
        None
    }

    /// Opt in to fresh ownership of contributed actions in the host's `x` menu.
    /// The host checks this token, route and the current subject's identity, URL
    /// and fields before replaying a captured contribution. An unchanged target
    /// may survive background reads; include connection revisions, not refresh
    /// counters. Tokens remain ephemeral and are never displayed or journalled.
    /// Returning `None` preserves existing captured-subject semantics.
    fn subject_menu_context(&self, _scope: &str, _pane: Pane) -> Option<String> {
        None
    }

    // --- settings -------------------------------------------------------

    fn sections(&self) -> Vec<SectionSpec> {
        vec![]
    }
    /// The icon beside a section in the settings nav.
    fn section_icon(&self, _section: &str) -> Option<crate::ui::icon::Icon> {
        None
    }
    /// As [`Plugin::hints`], for a section being typed into.
    fn section_hints(&self, _section: &str) -> Vec<KeyHint<'static>> {
        vec![]
    }
    /// As [`Plugin::keys`], for the selected row of a settings section.
    fn section_keys(&self, _section: &str) -> Vec<Binding> {
        vec![]
    }

    // --- setup ----------------------------------------------------------

    /// Its step in setup, if it has one: the card is one of its sections.
    fn setup_step(&self) -> Option<SetupStep> {
        None
    }
    /// Where setup may look for its keys after asking (names only).
    fn setup_finds(&self) -> SetupFinds {
        SetupFinds::default()
    }
    /// The user said yes to setup's look for keys: it may now look on this
    /// machine for what it sets up (checkouts under the home folder), off
    /// the screen thread, and offer what it finds in its step.
    fn setup_looked(&mut self) {}

    /// What it already holds, for the splash's start line
    /// (`crates/radar-core/assets/start-lines.toml`, FACTS): each fact by
    /// its name (`mine.open`, `reviews.count`), from its cache only; never
    /// fetched for a line.
    fn start_facts(&self, _now: i64) -> Vec<(&'static str, crate::start_lines::Value)> {
        vec![]
    }
    // --- menus ----------------------------------------------------------

    /// What it makes of `text` typed into the field of the menu it opened as
    /// `menu` ([`Menu::typed`]), asked after each change: read only what is
    /// cheap to read (a folder listing, a file at the top of it).
    fn typing(&self, _menu: &str, _text: &str) -> Typing {
        Typing::default()
    }

    /// The user chose `value` in the menu this plugin opened as `menu`.
    fn on_menu(&mut self, _menu: &str, _value: &str) -> Vec<Effect> {
        vec![]
    }

    // --- CLI ------------------------------------------------------------

    /// Replace what the part holds with its sample data: the same stand-ins
    /// its tests use (neutral names, no live service), so `radar shot` can draw
    /// its screens and settings without a network or a config. Times in it are
    /// relative to [`DEMO_NOW`]. The default does nothing. A part implements
    /// it behind a `demo` cargo feature (`#[cfg(any(test, feature = "demo"))]`
    /// on its sample), so a build without the feature carries none of it.
    fn demo(&mut self) {}

    /// Subcommands under `radar <id>`.
    fn cli(&self) -> Option<clap::Command> {
        None
    }
    fn run_cli(&self, _matches: &clap::ArgMatches) -> CliTask {
        Box::pin(async { Ok(String::new()) })
    }

    /// A completed command plus bounded, explicit provider observations. The
    /// default preserves `run_cli` output and errors exactly, with no observations.
    /// Providers keep observations outside their fallible work so a later error
    /// does not erase earlier facts. This is completion-only: cancellation or a
    /// crash before return cannot deliver intermediate facts. The host timestamps
    /// receipt of the envelope, not provider requests, responses or elapsed time.
    fn run_cli_observed(&self, matches: &clap::ArgMatches) -> ObservedCliTask {
        let task = self.run_cli(matches);
        Box::pin(async move { CliExecution::new(task.await, CliObservations::default()) })
    }
}
