//! The renderer-neutral plugin contract (M11 A0): what a part shows and what
//! the user can do there, described without a terminal.
//!
//! A part publishes a [`ScreenView`] for each of its screens and a
//! [`SectionView`] for each settings section: semantic nodes with stable ids
//! and bounded data, never painted cells. A frontend lays them out and paints
//! them; the Ink renderer does so over IPC ([`crate::ipc`]), and a
//! headless consumer ([`crate::headless`]) reads the same descriptions and
//! drives the same part with [`Intent`]s. What the user does reaches the part
//! as an [`Intent`] aimed at a [`Target`], never as a terminal event or a
//! cell position, and the host keeps permission, mode, confirmation and
//! journal causality whichever frontend sent it.
//!
//! Nothing in this module names a terminal library (a test reads the source
//! to keep it so). Every description carries [`VERSION`]; a frontend that
//! meets an [`Node::Extension`] it does not know draws its fallback, and an
//! unknown node degrades visibly without dropping the part's actions.

use serde::{Deserialize, Serialize};

use crate::ui::icon::Icon;

/// The contract's version. Bumped on any change a frontend could misread;
/// additive fields default so older descriptions still parse.
pub const VERSION: u32 = 1;

// --- text ----------------------------------------------------------------------

/// What a run of text means, for the frontend to colour: the foreground
/// slots of the design system, never a colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Ink {
    /// As the surrounding text.
    #[default]
    Plain,
    Primary,
    Secondary,
    Muted,
    Emphasis,
    Accent,
    AccentMuted,
    AccentSecondary,
    AccentTertiary,
    Good,
    Warn,
    Bad,
    Info,
    Number,
    Added,
    Removed,
}

impl Ink {
    /// The tone an ink stands for, where it is one (a chip's).
    pub fn tone(self) -> crate::plugin::Tone {
        use crate::plugin::Tone;
        match self {
            Ink::Muted => Tone::Muted,
            Ink::Accent => Tone::Accent,
            Ink::Info => Tone::Info,
            Ink::Good => Tone::Good,
            Ink::Warn => Tone::Warn,
            Ink::Bad => Tone::Bad,
            _ => Tone::Plain,
        }
    }
}

impl From<crate::plugin::Tone> for Ink {
    fn from(t: crate::plugin::Tone) -> Self {
        use crate::plugin::Tone;
        match t {
            Tone::Plain => Ink::Plain,
            Tone::Muted => Ink::Muted,
            Tone::Accent => Ink::Accent,
            Tone::Info => Ink::Info,
            Tone::Good => Ink::Good,
            Tone::Warn => Ink::Warn,
            Tone::Bad => Ink::Bad,
        }
    }
}

/// A run of text, or an icon (its name; the frontend draws it from the pack
/// in use, in the icon's own colour). An icon with no words is a lead (the
/// pack's gap after it; nothing with icons off); an icon with words is drawn
/// bare, the words standing in for it with icons off ([`Seg::icon_or`]).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct Seg {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    #[serde(default, skip_serializing_if = "is_plain")]
    pub ink: Ink,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub strong: bool,
    /// A search match or other highlighted run.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub marked: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// Leave the icon's normal gap before following words. The fallback
    /// already includes its own gap when icons are hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub icon_lead: bool,
    /// Where the words go when they are a link.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// An inline target, used for actions embedded in a line under a title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
    /// Presentation-only shortcut after this phrase in a plain-text reader.
    /// Visual drawers use its supplied underline styling and ignore this field;
    /// this does not name an action or grant any input-routing authority.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hint: Option<String>,
    /// A look the part names for its own domain instead of an ink: a style
    /// spec in the theme's words (`<part>.blue bold`, a look the user set),
    /// which the frontend resolves in its theme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<String>,
    /// Drawn as a chip (a label on its own shade) in its ink's tone, its
    /// icon on the chip ([`Seg::chip`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub chip: bool,
}

fn is_plain(ink: &Ink) -> bool {
    *ink == Ink::Plain
}

impl Seg {
    pub fn new(text: impl Into<String>, ink: Ink) -> Self {
        Self {
            text: text.into(),
            ink,
            ..Self::default()
        }
    }

    /// An icon, drawn before what follows it with the pack's own gap.
    pub fn icon(icon: Icon) -> Self {
        Self {
            icon: Some(icon),
            ..Self::default()
        }
    }

    /// An icon drawn bare, or `words` in `ink` where icons are off (`✗`).
    pub fn icon_or(icon: Icon, words: impl Into<String>, ink: Ink) -> Self {
        Self {
            icon: Some(icon),
            text: words.into(),
            ink,
            ..Self::default()
        }
    }

    pub fn icon_or_lead(icon: Icon, words: impl Into<String>, ink: Ink) -> Self {
        let mut seg = Self::icon_or(icon, words, ink);
        seg.icon_lead = true;
        seg
    }

    /// Words in a look the part names ([`Seg::look`]).
    pub fn styled(text: impl Into<String>, look: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            look: Some(look.into()),
            ..Self::default()
        }
    }

    pub fn strong(mut self) -> Self {
        self.strong = true;
        self
    }

    pub fn marked(mut self) -> Self {
        self.marked = true;
        self
    }

    /// The words are a link to `url`.
    pub fn linked(mut self, url: impl Into<String>) -> Self {
        self.link = Some(url.into());
        self
    }

    pub fn targeted(mut self, target: impl Into<String>) -> Self {
        self.target = Some(Target::new(target));
        self
    }

    /// Another part's chip ([`crate::compose::Chip`]): its words in its
    /// tone on the chip's shade, its icon and link.
    pub fn chip(c: &crate::compose::Chip) -> Self {
        Self {
            text: c.text.clone(),
            ink: Ink::from(c.tone),
            icon: c.icon,
            link: c.url.clone(),
            chip: true,
            ..Self::default()
        }
    }

    /// The width it takes in cells, icons left out (they depend on the pack).
    pub fn width(&self) -> usize {
        crate::ui::width(&self.text)
    }
}

/// One line of rich text: its runs in order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct Text(pub Vec<Seg>);

impl Text {
    pub fn new() -> Self {
        Self::default()
    }

    /// Words in one ink.
    pub fn of(text: impl Into<String>, ink: Ink) -> Self {
        Self(vec![Seg::new(text, ink)])
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self::of(text, Ink::Plain)
    }

    pub fn push(mut self, seg: Seg) -> Self {
        self.0.push(seg);
        self
    }

    pub fn add(&mut self, seg: Seg) {
        self.0.push(seg);
    }

    pub fn extend(&mut self, other: Text) {
        self.0.extend(other.0);
    }

    /// The words, icons left out.
    pub fn words(&self) -> String {
        self.0.iter().map(|s| s.text.as_str()).collect()
    }

    /// Plain readers retain the exact original shortcut even without underline.
    /// Unlike words(), this is presentation text and never a scalar-index source.
    pub fn plain_words(&self) -> String {
        self.0.iter().map(|s| match &s.key_hint {
            Some(key) => format!("{} [{key}]", s.text),
            None => s.text.clone(),
        }).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.0.iter().all(|s| s.text.is_empty() && s.icon.is_none())
    }
}

/// Its words (icons are drawn, not written).
impl std::fmt::Display for Text {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.words())
    }
}

impl From<&str> for Text {
    fn from(s: &str) -> Self {
        Self::plain(s)
    }
}

impl From<String> for Text {
    fn from(s: String) -> Self {
        Self::plain(s)
    }
}

impl From<Vec<Seg>> for Text {
    fn from(segs: Vec<Seg>) -> Self {
        Self(segs)
    }
}

/// Icons cross the boundary by their stable names ([`Icon::name`]); an
/// unknown name reads as none rather than failing the whole description.
pub(crate) mod icon_name {
    use super::Icon;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(icon: &Option<Icon>, s: S) -> Result<S::Ok, S::Error> {
        match icon {
            Some(i) => s.serialize_some(i.name()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Icon>, D::Error> {
        Ok(Option::<String>::deserialize(d)?.and_then(|n| Icon::from_name(&n)))
    }
}

// --- targets, intents and keys ---------------------------------------------------

/// A stable id for something the user can point at (a row, a cell that
/// shows a subject, a tab), unique within its view. The part gets it back in
/// an [`Intent`]; frontends never invent or parse it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct Target(pub String);

impl Target {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A key, by what it is rather than how a terminal reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Tab,
    BackTab,
    Backspace,
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
}

/// A key with its modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
pub struct Chord {
    pub key: Key,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ctrl: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub alt: bool,
    // Distinguish modified navigation keys on the wire. Printable shifted
    // characters keep their existing uppercase/symbol encoding.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub shift: bool,
}

impl Chord {
    pub const fn new(key: Key) -> Self {
        Self {
            key,
            ctrl: false,
            alt: false,
            shift: false,
        }
    }

    pub const fn char(c: char) -> Self {
        Self::new(Key::Char(c))
    }

    pub const fn ctrl(c: char) -> Self {
        Self {
            key: Key::Char(c),
            ctrl: true,
            alt: false,
            shift: false,
        }
    }

    /// No control or alt held; shift is ignored (a shifted character is
    /// already its own character).
    pub const fn plain(&self) -> bool {
        !self.ctrl && !self.alt
    }

    /// The vim/arrow direction of the key, if it is one: `Some(true)` down.
    pub const fn vertical(&self) -> Option<bool> {
        match self.key {
            Key::Char('j') | Key::Down => Some(true),
            Key::Char('k') | Key::Up => Some(false),
            _ => None,
        }
    }

    /// The direction along a row: `Some(true)` right (`l`, `→`, `tab`),
    /// `Some(false)` left (`h`, `←`, `shift-tab`).
    pub const fn horizontal(&self) -> Option<bool> {
        match self.key {
            Key::Char('l') | Key::Right | Key::Tab => Some(true),
            Key::Char('h') | Key::Left | Key::BackTab => Some(false),
            _ => None,
        }
    }

    /// No modifier at all.
    pub const fn bare(&self) -> bool {
        !self.ctrl && !self.alt && !self.shift
    }

    /// As an actions menu writes it: `d`, `⏎`, `space`, `ctrl-r`, `pgdn`.
    pub fn parse(keys: &str) -> Option<Self> {
        let (keys, ctrl) = keys
            .strip_prefix("ctrl-")
            .map_or((keys, false), |k| (k, true));
        let key = match keys {
            "⏎" | "enter" => Key::Enter,
            "esc" => Key::Esc,
            "space" => Key::Char(' '),
            "tab" => Key::Tab,
            "shift-tab" => Key::BackTab,
            "backspace" => Key::Backspace,
            "delete" => Key::Delete,
            "home" => Key::Home,
            "end" => Key::End,
            "←" => Key::Left,
            "→" => Key::Right,
            "↑" => Key::Up,
            "↓" => Key::Down,
            "pgup" => Key::PageUp,
            "pgdn" => Key::PageDown,
            _ => {
                let mut chars = keys.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Key::Char(c),
                    _ => return None,
                }
            }
        };
        Some(Self {
            key,
            ctrl,
            alt: false,
            shift: false,
        })
    }

    /// The name [`Chord::parse`] reads back.
    pub fn name(&self) -> String {
        let key = match self.key {
            Key::Char(' ') => "space".to_string(),
            Key::Char(c) => c.to_string(),
            Key::Enter => "⏎".into(),
            Key::Esc => "esc".into(),
            Key::Tab => "tab".into(),
            Key::BackTab => "shift-tab".into(),
            Key::Backspace => "backspace".into(),
            Key::Delete => "delete".into(),
            Key::Up => "↑".into(),
            Key::Down => "↓".into(),
            Key::Left => "←".into(),
            Key::Right => "→".into(),
            Key::Home => "home".into(),
            Key::End => "end".into(),
            Key::PageUp => "pgup".into(),
            Key::PageDown => "pgdn".into(),
        };
        if self.ctrl {
            format!("ctrl-{key}")
        } else {
            key
        }
    }
}

/// What the user did, aimed at the part. A frontend turns its own input
/// (keys, pointer, touch) into these; the host routes them with the same
/// mode, permission and journal handling whichever frontend sent them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Intent {
    /// Presentation-only acknowledgement for one physical key.
    NoticeInput { escape: bool, #[ts(type = "number")] through_id: u64 },
    /// Dismiss one currently visible occurrence without activating anything beneath it.
    DismissNotice { #[ts(type = "number")] id: u64 },
    /// Move the selection to a target (a row, a view in the nav pane).
    Select { target: Target },
    /// Do what the target does when chosen (`⏎`, a double click).
    Activate { target: Target },
    /// Select a target for its context menu without activating it.
    Context { target: Target },
    /// Open or close a group in a tree.
    Fold { target: Target, open: bool },
    /// Open every group held by the focused tree. The host expands this
    /// into validated folds; it never asks an owner to fetch more rows.
    OpenAll,
    /// Choose a cell another part added to a row ([`CellView::show`]): the
    /// host selects the row through its owner, then shows the subject. The
    /// owner gets [`Intent::Select`]; it never sees this one.
    Show { target: Target, subject: SubjectRef },
    /// Run one of the actions the view lists ([`ActionSpec::id`]).
    Invoke { action: String },
    /// A key the frontend did not use for movement. Keys are a fallback for
    /// what has no action id yet; prefer [`Intent::Invoke`].
    Key { chord: Chord },
    /// Select text in the editor open: `anchor` to `extent`, as the
    /// editor's `caret` counts (either may be the smaller), within its
    /// `line`. Moves the caret to `extent`.
    EditSelect { anchor: usize, extent: usize },
    /// Clear the editor's selection, leaving the caret where it is.
    EditUnselect,
    /// Copy screen text the user selected (a drag, a double click, ctrl+c on
    /// a selection): the host puts `text` on the clipboard through the
    /// ways it knows (a program, else the terminal's OSC 52), and says how.
    /// The renderer sends
    /// the text it drew in the selected cells; more than
    /// [`crate::ipc::MAX_CLIPBOARD`] bytes is refused.
    Copy { text: String },
    /// The user scrolled a pane to the end of what it shows (more can be
    /// fetched: older comments).
    End { pane: PaneId },
    /// Something done in the agent dock. The host takes its own state
    /// (show, hide, focus, resize) and hands the rest to the part that
    /// supplies the dock; a screen never sees this one.
    Dock(crate::dock::DockIntent),
}

// --- actions ---------------------------------------------------------------------

/// What a write does to a service, as the confirm and the mode see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Add,
    Edit,
}

impl From<crate::writes::Change> for Change {
    fn from(c: crate::writes::Change) -> Self {
        match c {
            crate::writes::Change::Add => Change::Add,
            crate::writes::Change::Edit => Change::Edit,
        }
    }
}

/// One thing that can be done where the view is focused: the rows of the
/// actions menu, as data. The id is stable for the scope and is what
/// [`Intent::Invoke`] names; the journal records what running it does.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ActionSpec {
    /// Zero-based Unicode scalar position in label; underline its complete grapheme.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mnemonic_at: Option<usize>,
    /// Show the named Shift modifier before a valid keyed word.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mnemonic_shift: bool,
    pub id: String,
    pub label: String,
    /// The key that runs it, when it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<Chord>,
    /// A heading over the actions that follow (not runnable).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// A write to a service: the host mutes it while the mode refuses it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change: Option<Change>,
    /// It acts on the pane or the session, not on the selected row: the
    /// pane's menu offers it, the row's does not.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pane: bool,
    /// A fact about the row it acts on (`running 4m`), drawn at the right
    /// edge of its menu row, never in the label.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub hint: String,
    /// What doing it is for, which places it under a heading.
    #[serde(default, skip_serializing_if = "ActionGroup::is_open")]
    pub group: ActionGroup,
    /// The row's state asks for it: it comes straight after the default action.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub lead: bool,
}

/// What an action is for: the row menu groups by this, not by which part
/// offers it. `Open` rows come first, under no heading.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ActionGroup {
    #[default]
    Open,
    /// Changes it where it lives: merge, approve, rebase.
    Change,
    /// Works on it on this machine: check out, an editor, a terminal.
    Local,
    /// Brings someone in: an agent, a review ask, a work item.
    Ask,
}

impl ActionGroup {
    pub fn is_open(&self) -> bool {
        *self == Self::Open
    }

    /// The heading over the group in a row menu.
    pub fn heading(self) -> Option<&'static str> {
        match self {
            Self::Open => None,
            Self::Change => Some("change it"),
            Self::Local => Some("work on it here"),
            Self::Ask => Some("ask someone"),
        }
    }
}

impl ActionSpec {
    /// An actions-menu row ([`crate::Binding`]) as data under the part's own
    /// stable id (`<part>.<surface>.<verb>`). A heading keeps an empty id.
    pub fn named(id: impl Into<String>, b: &crate::Binding) -> Self {
        Self {
            id: if b.is_heading() {
                String::new()
            } else {
                id.into()
            },
            label: b.what.clone(),
            mnemonic_at: (!b.is_heading()).then(|| crate::mnemonic::at(&b.keys, &b.what)).flatten(),
            mnemonic_shift: !b.is_heading() && crate::mnemonic::at(&b.keys, &b.what).is_some() && crate::mnemonic::shifted(&b.keys),
            key: Chord::parse(&b.keys),
            heading: b.is_heading(),
            icon: b.icon,
            change: b.change.map(Change::from),
            pane: b.pane,
            hint: b.hint.clone(),
            group: b.group,
            lead: b.lead,
        }
    }
}

// --- nodes -----------------------------------------------------------------------

/// The panes of the universal layout, left to right.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum PaneId {
    Nav,
    Middle,
    Detail,
}

impl From<crate::plugin::Pane> for PaneId {
    fn from(p: crate::plugin::Pane) -> Self {
        match p {
            crate::plugin::Pane::Nav => PaneId::Nav,
            crate::plugin::Pane::Middle => PaneId::Middle,
            crate::plugin::Pane::Detail => PaneId::Detail,
        }
    }
}

impl From<PaneId> for crate::plugin::Pane {
    fn from(p: PaneId) -> Self {
        match p {
            PaneId::Nav => crate::plugin::Pane::Nav,
            PaneId::Middle => crate::plugin::Pane::Middle,
            PaneId::Detail => crate::plugin::Pane::Detail,
        }
    }
}

/// A row of a flat list (the nav pane's views, a settings section).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ListRow {
    pub target: Target,
    pub text: Text,
    /// Kept on the right edge: a count, a state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<Text>,
    /// The row is about something with a link: the frontend makes the row
    /// open it the way links open there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    /// After the text, shown whole or not at all (the detail panel has it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Text>,
    /// A heading over the rows that follow: never selected or chosen.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
}

impl ListRow {
    pub fn new(target: impl Into<String>, text: impl Into<Text>) -> Self {
        Self {
            target: Target::new(target),
            text: text.into(),
            right: None,
            link: None,
            detail: None,
            heading: false,
        }
    }

    /// See [`ListRow::heading`].
    pub fn heading(target: impl Into<String>, text: impl Into<Text>) -> Self {
        Self {
            heading: true,
            ..Self::new(target, text)
        }
    }

    pub fn detail(mut self, detail: impl Into<Text>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn right(mut self, right: impl Into<Text>) -> Self {
        self.right = Some(right.into());
        self
    }

    pub fn linked(mut self, url: Option<String>) -> Self {
        self.link = url;
        self
    }
}

/// Text being typed on a row: where the field starts (cells into the row),
/// what is in it and where the caret is (in characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Editing {
    pub row: usize,
    pub at: u16,
    pub text: String,
    pub caret: usize,
}

/// A column of a tree or table: its heading, its preferred width in cells,
/// and how narrow it may get before it is dropped whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ColumnSpec {
    pub heading: String,
    pub width: u16,
    pub keep: u16,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    /// Takes the room the others leave, at least `width` (a table's title).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flex: bool,
    /// Another part's column: shown before the last of the part's own while
    /// the names keep their room, never pushing one of those out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub extra: bool,
}

impl ColumnSpec {
    pub fn new(heading: impl Into<String>, width: u16, keep: u16) -> Self {
        Self {
            heading: heading.into(),
            width,
            keep,
            icon: None,
            flex: false,
            extra: false,
        }
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// See [`ColumnSpec::flex`].
    pub fn flex(heading: impl Into<String>, width: u16) -> Self {
        Self {
            flex: true,
            ..Self::new(heading, width, 0)
        }
    }

    /// See [`ColumnSpec::extra`].
    pub fn extra(heading: impl Into<String>, width: u16) -> Self {
        Self {
            extra: true,
            ..Self::new(heading, width, 0)
        }
    }
}

/// A subject as view data: the shared vocabulary's kind and id, its label
/// and link, and the fields other parts match on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct SubjectRef {
    pub kind: String,
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub fields: std::collections::BTreeMap<String, String>,
}

impl From<&crate::compose::Subject> for SubjectRef {
    fn from(s: &crate::compose::Subject) -> Self {
        Self {
            kind: s.kind.clone(),
            id: s.id.clone(),
            label: s.label.clone(),
            url: s.url.clone(),
            fields: s.fields.clone(),
        }
    }
}

impl From<SubjectRef> for crate::compose::Subject {
    fn from(s: SubjectRef) -> Self {
        let mut out = crate::compose::Subject::new(s.kind, s.id, s.label);
        out.url = s.url;
        out.fields = s.fields;
        out
    }
}

/// One cell of a row, under the column with the same heading. A cell that
/// shows a subject (another part's cell) can be chosen on its own: the row
/// is selected and the subject shown, by the host, whichever frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct CellView {
    pub heading: String,
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<SubjectRef>,
    /// A cell action owned by its row's part, distinct from a contributed
    /// subject to show. The target can encode the row's stable path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
}

/// A row of a tree; its id is unique among its siblings, and its path (the
/// ids from the root) is its [`Target`] (`TreeRow::target_of`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TreeRow {
    pub id: String,
    pub name: Text,
    /// A group label that cannot be selected; its children remain reachable.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub heading: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cells: Vec<CellView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeRow>,
    /// A top-level row's look for the stripe down all its rows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stripe: Option<String>,
    /// Chips after the name (another part's labels).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chips: Option<Chips>,
    /// The key a sync names this row by ([`crate::Effect::Changed`]): the
    /// frontend shows the change where the row is drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glow: Option<String>,
    /// After the name, shown whole or not at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Text>,
}

/// Text being typed on a tree's row: the row's path, how far into its name
/// the field starts, what is in it and where the caret is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TreeEditing {
    pub path: Vec<String>,
    pub at: u16,
    pub text: String,
    pub caret: usize,
}

/// Chips after a row's name, and which run of the name gives way to them
/// while it keeps a dozen cells (a title).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Chips {
    pub gives: usize,
    pub text: Text,
}

impl TreeRow {
    pub fn new(id: impl Into<String>, name: impl Into<Text>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            heading: false,
            cells: vec![],
            children: vec![],
            stripe: None,
            chips: None,
            glow: None,
            detail: None,
        }
    }

    pub fn heading(id: impl Into<String>, name: impl Into<Text>) -> Self {
        Self {
            heading: true,
            ..Self::new(id, name)
        }
    }

    /// See [`TreeRow::detail`].
    pub fn detail(mut self, detail: impl Into<Text>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// A cell that is text already laid out ([`cell_of`] for another
    /// part's).
    pub fn cell_view(mut self, cell: CellView) -> Self {
        self.cells.push(cell);
        self
    }

    pub fn children(mut self, rows: impl IntoIterator<Item = TreeRow>) -> Self {
        self.children.extend(rows);
        self
    }

    pub fn stripe(mut self, look: impl Into<String>) -> Self {
        self.stripe = Some(look.into());
        self
    }

    /// See [`Chips`]; none when `text` is empty.
    pub fn chips(mut self, gives: usize, text: Text) -> Self {
        if !text.0.is_empty() {
            self.chips = Some(Chips { gives, text });
        }
        self
    }

    pub fn glow(mut self, key: impl Into<String>) -> Self {
        self.glow = Some(key.into());
        self
    }

    pub fn cell(mut self, heading: impl Into<String>, text: impl Into<Text>) -> Self {
        self.cells.push(CellView {
            heading: heading.into(),
            text: text.into(),
            show: None,
            target: None,
        });
        self
    }

    pub fn child(mut self, row: TreeRow) -> Self {
        self.children.push(row);
        self
    }

    /// A row's target: its path, ids joined by a separator no id carries.
    pub fn target_of(path: &[String]) -> Target {
        Target(path.join(PATH_SEP))
    }

    /// The path a row's target names.
    pub fn path_of(target: &Target) -> Vec<String> {
        target.0.split(PATH_SEP).map(str::to_string).collect()
    }
}

/// Separates the ids of a tree path in its target (a unit separator: never
/// in a folder name or a branch).
pub const PATH_SEP: &str = "\u{1f}";

/// A tree of rows with columns (clones, their worktrees and branches).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, ts_rs::TS)]
pub struct Tree {
    /// The line over the rows (what they are, how many).
    #[serde(default, skip_serializing_if = "Text::is_empty")]
    pub header: Text,
    pub columns: Vec<ColumnSpec>,
    /// Fit every column in declared order, including contributed columns.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ordered_columns: bool,
    pub rows: Vec<TreeRow>,
    /// The selected row's path; empty for none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected: Vec<String>,
    /// The paths of the groups shown open.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub open: Vec<Vec<String>>,
    /// The width the rows' names may take before the columns.
    #[serde(default)]
    pub name_width: u16,
    /// The tree's own marks in looks the part names (the user's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<TreeLook>,
    /// The header's words choose this target (a menu of who to show).
    /// An empty header and no target omit the header line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_target: Option<Target>,
    /// Why there are no rows yet, drawn over where they go; rows there
    /// still show under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Note>,
    /// Names too long for the room give way run by run, each cut ending
    /// in `…`, so no word is cut bare and the columns keep their gap.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub cut_names: bool,
    /// A row being typed into; while it is, every key is the part's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editing: Option<Box<TreeEditing>>,
    /// A pointer press sends `node:<id>` so it can choose a file while
    /// keyboard selection only moves the tree cursor.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pointer_action: bool,
}

/// A tree's marks: its lines, the band behind every other block at
/// `band_level`, the shade behind top-level rows; each a look.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TreeLook {
    pub lines: String,
    pub band: String,
    pub heading: String,
    pub band_level: usize,
}

/// An empty, loading or failed state in words.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Note {
    pub state: State,
    pub text: String,
}

impl Tree {
    /// Resolve one unambiguous held path, including rows under closed groups.
    pub fn row(&self, path: &[String]) -> Option<&TreeRow> {
        let mut rows = self.rows.as_slice();
        let mut found = None;
        for id in path {
            let mut matches = rows.iter().filter(|row| row.id == *id);
            let row = matches.next()?;
            if matches.next().is_some() { return None; }
            found = Some(row);
            rows = &row.children;
        }
        found
    }

    /// Every unambiguous nonempty held group, parent before child. Counts and
    /// open-all use only these rows, never a remote total or missing children.
    pub fn groups(&self) -> Vec<Vec<String>> {
        fn walk(tree: &Tree, rows: &[TreeRow], prefix: &[String], out: &mut Vec<Vec<String>>) {
            for row in rows {
                let mut path = prefix.to_vec();
                path.push(row.id.clone());
                if !row.children.is_empty() && tree.row(&path).is_some() {
                    out.push(path.clone());
                    walk(tree, &row.children, &path, out);
                }
            }
        }
        let mut groups = vec![];
        walk(self, &self.rows, &[], &mut groups);
        groups
    }

    /// A row can receive a selection only while it is shown and selectable.
    pub fn selectable(&self, path: &[String]) -> bool {
        self.row(path).is_some_and(|row| !row.heading)
            && (1..path.len()).all(|end| self.open.iter().any(|p| p == &path[..end]))
    }

    /// Every row a frontend shows, in order, with its path: the roots and
    /// the children of the open groups.
    pub fn visible(&self) -> Vec<(Vec<String>, &TreeRow)> {
        fn walk<'a>(
            rows: &'a [TreeRow],
            prefix: &[String],
            open: &[Vec<String>],
            out: &mut Vec<(Vec<String>, &'a TreeRow)>,
        ) {
            for r in rows {
                let mut path = prefix.to_vec();
                path.push(r.id.clone());
                out.push((path.clone(), r));
                if !r.children.is_empty() && open.contains(&path) {
                    walk(&r.children, &path, open, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.rows, &[], &self.open, &mut out);
        out
    }
}

/// What a pane holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum Node {
    /// Rows to move along and choose from.
    List {
        rows: Vec<ListRow>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        editing: Option<Editing>,
    },
    Tree(Tree),
    /// Lines of rich text to read and scroll.
    Document {
        lines: Vec<Text>,
        /// Lines too long are cut at the edge, not wrapped (a column of
        /// facts).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        cut: bool,
    },
    /// Blocks to read and scroll: lines, and documents the frontend lays
    /// out at its width (a description, a comment). Pictures in them are
    /// fetched through the part ([`crate::Plugin::pictures`], by
    /// `pictures`); without it they are named, not shown.
    Rich {
        blocks: Vec<Block>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pictures: Option<String>,
    },
    /// Rows of cells under named columns, one selected.
    Table {
        columns: Vec<ColumnSpec>,
        rows: Vec<TableRow>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        empty: Option<String>,
    },
    /// Selectable rows with grouped lines before and under each row.
    Entries {
        rows: Vec<EntryView>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<usize>,
        empty: String,
    },
    /// A log's visible, wrapped rows. The owner keeps fold, search and
    /// cursor state; each frontend paints these same ordered rows.
    Log {
        lines: Vec<LogPaintLine>,
        top: usize,
        total: usize,
        empty: String,
    },
    /// A file diff with line numbers, cursor, and blocks beneath lines.
    /// The frontend may choose syntax colours and side-by-side layout.
    Diff {
        lines: Vec<crate::ui::diff::DiffLine>,
        under: Vec<DiffUnder>,
        selected: usize,
        top: usize,
        split: bool,
        language: Option<String>,
        empty: String,
        /// Each line's text (tabs as four spaces) cut into runs by what
        /// the code is, for a renderer that has no highlighter of its own.
        /// The bridge fills it (see [`ScreenView::paint_diffs`]); empty
        /// when `language` is unknown, and for hunk and note lines.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        painted: Vec<Vec<PaintRun>>,
        /// The words that changed within each line of a replaced pair, as
        /// `[from, to)` in characters of the line's text (tabs as four
        /// spaces), for the renderer to emphasise; empty when no line has any.
        /// The bridge fills it with `painted`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        words: Vec<Vec<(u32, u32)>>,
    },
    /// Nothing to show, and why: in words, with what kind of nothing.
    State {
        state: State,
        text: String,
    },
    /// A richer view a part declares (an editor, a terminal, media): the
    /// frontend that has the capability draws `data`; any other draws
    /// `fallback`, and says it did.
    Extension {
        capability: String,
        version: u32,
        data: serde_json::Value,
        fallback: Box<Node>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct LogPaintLine {
    pub text: Text,
    pub row: usize,
    pub selected: bool,
}

/// A stretch of a diff line and what the code in it is (`token` is one of
/// `plain`, `comment`, `text`, `number`, `keyword`, `type`, `function`,
/// `constant`, `mark`: [`crate::ui::highlight::Token::name`]). The
/// renderer styles it as `Token::style` does; the changed-word emphasis
/// is its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct PaintRun {
    pub text: String,
    pub token: String,
}

impl Node {
    /// Text to read, not rows to choose from: a double click in it selects a
    /// word and a triple click a line ([`crate::select`]). In the rows of a
    /// list, tree, table, entries or diff a double click chooses the row.
    /// A renderer tells the two apart by the `node` tag of the pane's body:
    /// `document`, `rich`, `log`, `state` and `extension` (an editor, a
    /// terminal) are text.
    pub fn is_text(&self) -> bool {
        matches!(
            self,
            Node::Document { .. } | Node::Rich { .. } | Node::Log { .. } | Node::State { .. } | Node::Extension { .. }
        )
    }

    /// Fill a [`Node::Diff`]'s `painted` and `words`. A diff is painted once
    /// and kept while it shows: the host builds a snapshot for every change,
    /// and highlighting and comparing the words of a long diff each time
    /// cost more than the rest of the snapshot.
    pub fn paint_diff(&mut self) {
        let Node::Diff { lines, language, painted, words, .. } = self else {
            return;
        };
        let done = paint_cached(lines, language.as_deref());
        painted.clone_from(&done.0);
        words.clone_from(&done.1);
    }

    /// [`Node::paint_diff`] on this node and the nodes inside it.
    pub fn paint_diffs(&mut self) {
        match self {
            Node::Diff { .. } => self.paint_diff(),
            Node::Extension { fallback, .. } => fallback.paint_diffs(),
            _ => {}
        }
    }
}

/// A diff's `painted` and `words`, as [`Node::paint_diff`] sends them.
type PaintedDiff = std::rc::Rc<(Vec<Vec<PaintRun>>, Vec<Vec<(u32, u32)>>)>;

/// The diffs painted lately, by their lines and language: a screen shows
/// a few diffs at most (a page, its overlay), and each is painted once.
const PAINTED_KEPT: usize = 4;

thread_local! {
    static PAINTED: std::cell::RefCell<Vec<(u64, PaintedDiff)>> = const { std::cell::RefCell::new(Vec::new()) };
}

fn paint_cached(lines: &[crate::ui::diff::DiffLine], language: Option<&str>) -> PaintedDiff {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (lines, language).hash(&mut h);
    let key = h.finish();
    if let Some(done) = PAINTED.with_borrow(|kept| kept.iter().find(|(k, _)| *k == key).map(|(_, d)| d.clone())) {
        return done;
    }
    let done: PaintedDiff = std::rc::Rc::new((paint_runs(lines, language), changed_chars(lines)));
    PAINTED.with_borrow_mut(|kept| {
        if kept.len() == PAINTED_KEPT {
            kept.remove(0);
        }
        kept.push((key, done.clone()));
    });
    done
}

/// Each line cut into runs by what the code is, as the renderer
/// takes them: context lines to both the old and the new side,
/// added lines to the new, removed lines to the old. Nothing when the
/// language is unknown.
fn paint_runs(lines: &[crate::ui::diff::DiffLine], language: Option<&str>) -> Vec<Vec<PaintRun>> {
    use crate::ui::diff::Kind;
    use crate::ui::highlight::Highlighter;
    let Some(language) = language else {
        return Vec::new();
    };
    let (Some(mut new_hl), Some(mut old_hl)) = (Highlighter::new(language), Highlighter::new(language)) else {
        return Vec::new();
    };
    let runs = |hl: &mut Highlighter, text: &str| -> Vec<PaintRun> {
        match hl.runs(text) {
            Some(runs) => {
                let mut out: Vec<PaintRun> = Vec::new();
                for (piece, token) in runs {
                    match out.last_mut() {
                        Some(last) if last.token == token.name() => last.text.push_str(piece),
                        _ => out.push(PaintRun { text: piece.to_string(), token: token.name().into() }),
                    }
                }
                out
            }
            None => vec![PaintRun { text: text.into(), token: "plain".into() }],
        }
    };
    lines
        .iter()
        .map(|l| {
            let text = l.text.replace('\t', "    ");
            match l.kind {
                Kind::Hunk | Kind::Note => vec![],
                Kind::Context => {
                    runs(&mut old_hl, &text);
                    runs(&mut new_hl, &text)
                }
                Kind::Added => runs(&mut new_hl, &text),
                Kind::Removed => runs(&mut old_hl, &text),
            }
        })
        .collect()
}

/// [`crate::ui::diff::changed_lines`] in characters instead of bytes, and
/// nothing at all when no line has a changed word.
fn changed_chars(lines: &[crate::ui::diff::DiffLine]) -> Vec<Vec<(u32, u32)>> {
    let ranges = crate::ui::diff::changed_lines(lines);
    if ranges.iter().all(Vec::is_empty) {
        return Vec::new();
    }
    ranges
        .iter()
        .zip(lines)
        .map(|(ranges, l)| {
            if ranges.is_empty() {
                return Vec::new();
            }
            let text = l.text.replace('\t', "    ");
            let chars = |byte: usize| text[..byte].chars().count() as u32;
            ranges.iter().map(|&(f, t)| (chars(f), chars(t))).collect()
        })
        .collect()
}

impl PaneView {
    pub fn paint_diffs(&mut self) {
        self.body.paint_diffs();
    }
}

impl ScreenView {
    /// Paint every diff showing on the screen (panes, page, overlay).
    pub fn paint_diffs(&mut self) {
        let page = self.page.iter_mut().flat_map(|p| p.panes.iter_mut());
        for pane in self.panes.iter_mut().chain(page).chain(self.overlay.iter_mut().map(|b| &mut **b)) {
            pane.paint_diffs();
        }
    }
}

impl SectionView {
    pub fn paint_diffs(&mut self) {
        self.body.paint_diffs();
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct DiffUnder {
    pub line: usize,
    pub lines: Vec<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<InlineInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct InlineInput {
    pub text: String,
    pub caret: usize,
    /// The selected text as `(anchor, extent)`, counted as `caret` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "[number, number] | null")]
    pub selection: Option<(usize, usize)>,
}

/// Why a pane shows no content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Empty,
    Loading,
    Failed,
}

/// The screen the host shows now, as every renderer is given it: the
/// part's own screen, or the host's set-up page while the part is not set
/// up yet.
pub fn screen_showing(host: &crate::host::Host, width: u16, height: u16) -> Option<ScreenView> {
    let (p, spec) = host.screen()?;
    let view = match host.setting_up() {
        Some(p) => Some(setup_screen(host, p, spec.id)),
        None => host.screen_view_in(p, spec.id, width, height),
    };
    view.map(|mut view| {
        view.dock = host.dock_view();
        view
    })
}

/// The set-up page of a part that is not set up yet, as a screen of one pane.
fn setup_screen(host: &crate::host::Host, p: usize, scope: &str) -> crate::view::ScreenView {
    use crate::compose::Setup;
    use crate::view::{Ink, Node, PaneId, PaneView, Seg, Text};
    let plugin = host.plugins[p].as_ref();
    let name = plugin.name();
    let what = match plugin.set_up() {
        Setup::Needed(what) => what,
        Setup::Ready => String::new(),
    };
    let mut head = Seg::icon(Icon::Settings);
    head.icon_lead = true;
    let mut lines = vec![
        Text::default().push(head).push(Seg::new(format!("{name} is not set up yet."), Ink::Emphasis)),
        Text::default(),
        Text::of(what, Ink::Primary),
        Text::default(),
    ];
    if host.first_section_of(p).is_some() {
        lines.push(Text::of(
            format!("Press ⏎ to set it up in settings, under {}.", plugin.sections()[0].group),
            Ink::Secondary,
        ));
    }
    if plugin.setup_step().is_some() {
        lines.push(Text::of(crate::setup::RESUME, Ink::Secondary));
    }
    crate::view::ScreenView::new(
        scope,
        vec![PaneView::new(PaneId::Middle, format!("set up {name}"), Node::Document { lines, cut: false })],
    )
}

/// A block of a [`Node::Rich`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "block", rename_all = "snake_case")]
pub enum Block {
    Line {
        text: Text,
        /// Unicode scalar position in Text::words(), including icon fallback text
        /// in offsets. Only non-icon text and chip captions supply mnemonics.
        /// Draw the complete containing grapheme, preserving segment styles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mnemonic_at: Option<usize>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        mnemonic_shift: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<Target>,
        /// The unchanged shortcut for this line. Valid mnemonic metadata puts
        /// its letter inside the words; glyph/named/modifier keys precede them.
        /// Legacy or invalid letter metadata and plain readers keep `[k]`,
        /// right-aligned when it fits and wrapped otherwise.
        /// See [`Block::keyed`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// A section of a detail pane: its heading
    /// (`mark`, the title in muted capitals, `detail` in secondary words
    /// where it fits) and a quiet rule to the edge of the room, then its
    /// blocks two cells in, laid out in the room left. A line in it wraps
    /// at that room and each of its rows is set in.
    Section {
        #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
        #[ts(as = "Option<String>")]
        mark: Option<Icon>,
        /// As drawn: in capitals ([`Block::section`]).
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
        blocks: Vec<Block>,
    },
    /// A document laid out at the frontend's width, `bar` before each of
    /// its lines (a comment's), at most `most` lines and then `…`.
    Doc {
        #[ts(as = "Vec<crate::ui::rich::Block>")]
        doc: crate::ui::rich::Doc,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bar: Option<Seg>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        most: Option<usize>,
        /// Show link addresses after their words when the frontend cannot
        /// open the words themselves. `None` uses the frontend default.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        urls: Option<bool>,
    },
    /// A run's stages, laid out as a bar and labels at the frontend's width.
    Meter { stages: Vec<MeterStage> },
    /// Plain words wrapped behind a prefix on every line.
    Wrapped { text: String, prefix: Seg, ink: Ink },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct MeterStage {
    pub name: String,
    pub mark: crate::ui::Mark,
    pub look: String,
}

impl Block {
    pub fn line(text: impl Into<Text>) -> Self {
        Self::Line {
            text: text.into(),
            mnemonic_at: None,
            mnemonic_shift: false,
            target: None,
            key: None,
        }
    }

    /// A line that can be selected like an action in the detail pane.
    pub fn action(target: impl Into<String>, text: impl Into<Text>) -> Self {
        Self::Line {
            text: text.into(),
            mnemonic_at: None,
            mnemonic_shift: false,
            target: Some(Target::new(target)),
            key: None,
        }
    }

    /// An action's line with its letter inside the word, or a named/symbol key.
    pub fn keyed(target: impl Into<String>, text: impl Into<Text>, key: impl Into<String>) -> Self {
        let text = text.into();
        let key = key.into();
        let mnemonic_at = crate::mnemonic::at_text(&key, &text);
        let mnemonic_shift = mnemonic_at.is_some() && crate::mnemonic::shifted(&key);
        Self::Line { text, mnemonic_at, mnemonic_shift, target: Some(Target::new(target)), key: Some(key) }
    }

    /// A section titled `title` (drawn in capitals) holding `blocks`.
    pub fn section(mark: Option<Icon>, title: &str, detail: Option<String>, blocks: Vec<Block>) -> Self {
        Self::Section {
            mark,
            title: title.to_uppercase(),
            detail,
            blocks,
        }
    }

    /// A section of lines ([`Block::section`]).
    pub fn section_of(mark: Option<Icon>, title: &str, detail: Option<String>, lines: Vec<Text>) -> Self {
        Self::section(mark, title, detail, lines.into_iter().map(Block::line).collect())
    }

    pub fn doc(doc: crate::ui::rich::Doc) -> Self {
        Self::Doc {
            doc,
            bar: None,
            most: None,
            urls: None,
        }
    }
}

/// A row of a [`Node::Table`]: its cells by column heading.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TableRow {
    pub target: Target,
    pub cells: Vec<CellView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct EntryView {
    pub target: Target,
    pub head: Text,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub before: Vec<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub body: Vec<Text>,
}

impl EntryView {
    pub fn new(target: impl Into<String>, head: impl Into<Text>) -> Self {
        Self {
            target: Target::new(target),
            head: head.into(),
            before: vec![],
            body: vec![],
        }
    }
}

impl TableRow {
    pub fn new(target: impl Into<String>) -> Self {
        Self {
            target: Target::new(target),
            cells: vec![],
        }
    }

    pub fn cell(mut self, heading: impl Into<String>, text: impl Into<Text>) -> Self {
        self.cells.push(CellView {
            heading: heading.into(),
            text: text.into(),
            show: None,
            target: None,
        });
        self
    }
}

/// A line at the top of a pane to type into (go to, a filter): its mark,
/// what is in it, the caret while it has the keys (else the words are
/// shown as set), and a line under it saying what it does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct FieldBar {
    pub mark: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caret: Option<usize>,
    pub hint: Text,
}

/// A tab of an item page: its name and, when needed, one warning dot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct TabView {
    /// The name drawn after the icon (`Changes`), without counts.
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub icon: Option<Icon>,
    // Something here needs attention, rather than a count.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub attention: bool,
}

impl TabView {
    pub fn new(label: impl Into<String>, icon: Option<Icon>) -> Self {
        Self {
            label: label.into(),
            icon,
            attention: false,
        }
    }

    /// Mark this tab as needing attention without putting state in its name.
    pub fn attention(mut self, attention: bool) -> Self {
        self.attention = attention;
        self
    }

    /// The target choosing tab `i` sends ([`Intent::Select`]).
    pub fn target(i: usize) -> Target {
        Target(format!("tab:{i}"))
    }

    /// The tab a target chooses.
    pub fn index(target: &Target) -> Option<usize> {
        target.0.strip_prefix("tab:")?.parse().ok()
    }
}

/// An item's own page: what is above its body (the way down to it, its
/// title with details at the right, a line under it, the tabs) and its
/// body's panes side by side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct PageView {
    /// From the screen down to this page; the last stands out.
    pub crumbs: Vec<Seg>,
    pub title: Text,
    /// At the right of the title; the first runs give way first.
    #[serde(default, skip_serializing_if = "Text::is_empty")]
    pub details: Text,
    /// A line under the title; kept (blank) while what fills it loads, so
    /// the tabs do not move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub under: Option<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tabs: Vec<TabView>,
    #[serde(default)]
    pub tab: usize,
    /// A page-wide notice between the tabs and the body, outside row indices.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notice: Option<Text>,
    /// Left to right; a pane with a width keeps it, the others share
    /// what is left. A pane with [`PaneView::from`] shows only on a page
    /// at least that wide.
    pub panes: Vec<PaneView>,
    /// Optional responsive arrangement of the panes. Without it, panes
    /// share a horizontal row as in the original page contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<PageLayout>,
}

/// A page's pane arrangement, independent of terminal coordinates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PageLayout {
    Pane {
        index: usize,
    },
    Split {
        axis: Axis,
        parts: Vec<LayoutPart>,
    },
    Responsive {
        min_width: u16,
        wide: Box<PageLayout>,
        narrow: Box<PageLayout>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct LayoutPart {
    pub size: Size,
    pub layout: PageLayout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum Size {
    Fill(u16),
    Length(u16),
}

impl PageLayout {
    pub fn pane(index: usize) -> Self {
        Self::Pane { index }
    }
    pub fn split(axis: Axis, parts: impl IntoIterator<Item = (Size, PageLayout)>) -> Self {
        Self::Split {
            axis,
            parts: parts
                .into_iter()
                .map(|(size, layout)| LayoutPart { size, layout })
                .collect(),
        }
    }
}

/// What a renderer laid out in a pane whose body it scrolls itself: the
/// body's rows that show, the rows its content took at the pane's width,
/// and the first row shown. The renderer reports it
/// ([`crate::ipc::ToHost::Rows`]); the host gives it to the part's scroll
/// for that pane ([`crate::Plugin::scroll`], [`crate::Host::laid_out`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct PaneRows {
    pub pane: PaneId,
    pub rows: u16,
    pub content: u16,
    pub at: u16,
}

/// A titled pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct PaneView {
    pub pane: PaneId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "icon_name")]
    #[ts(as = "Option<String>")]
    pub title_icon: Option<Icon>,
    pub body: Node,
    /// Lines kept at the pane's foot (what the view shows, a warning).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Text>,
    /// A plain line kept at the pane's foot, without a section rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<Text>,
    /// The width the pane prefers when there is room for all panes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
    /// Lines to type into at its top.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldBar>,
    /// A one-line input kept at the bottom of the pane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_field: Option<FieldBar>,
    /// A line kept at its top, over the body (what went wrong loading it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<Text>,
    /// On a page: shown only when the page is at least this wide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<u16>,
    /// How many lines the notes may take at most (2 when not said).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_most: Option<u16>,
    /// Its rows sit a cell in from each side (a page's list of rows).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inset: bool,
    // Pinned, host-admitted controls for the shared detail surface.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foot_actions: Option<detail::FootActions>,
    /// The first row of the body to show, when the part moved its scroll
    /// away from where the renderer last said it was (a page down from
    /// another pane): the renderer scrolls there. Absent while the
    /// renderer's own position stands. The host fills it
    /// ([`crate::Plugin::scroll`]); a part never sets it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scroll: Option<u16>,
}

impl PaneView {
    pub fn new(pane: PaneId, title: impl Into<String>, body: Node) -> Self {
        Self {
            pane,
            title: title.into(),
            title_icon: None,
            body,
            foot_actions: None,
            notes: vec![],
            footer: None,
            width: None,
            fields: vec![],
            bottom_field: None,
            head: None,
            from: None,
            notes_most: None,
            inset: false,
            scroll: None,
        }
    }
}

/// Text being written for a write (a comment, a reply): the lines kept and
/// the one being typed, with its caret. A frontend with the `editor`
/// capability edits it in place; any other shows its fallback, the lines
/// as written ([`Editor::node`]). Keys reach the part as intents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct Editor {
    pub title: String,
    pub lines: Vec<String>,
    pub line: String,
    pub caret: usize,
    /// The selected text of `line` as `(anchor, extent)`, counted as
    /// `caret` is; none when nothing is selected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(type = "[number, number] | null")]
    pub selection: Option<(usize, usize)>,
}

impl Editor {
    pub const CAPABILITY: &'static str = "editor";

    /// As an extension node, with the lines as its fallback.
    pub fn node(&self) -> Node {
        let mut lines: Vec<Text> = self
            .lines
            .iter()
            .map(|l| Text::of(l.clone(), Ink::Primary))
            .collect();
        lines.push(Text::of(self.line.clone(), Ink::Primary));
        Node::Extension {
            capability: Self::CAPABILITY.into(),
            version: 1,
            data: serde_json::to_value(self).unwrap_or_default(),
            fallback: Box::new(Node::Document { lines, cut: false }),
        }
    }

    /// The editor an extension node holds, when it is one.
    pub fn of(node: &Node) -> Option<Self> {
        match node {
            Node::Extension {
                capability,
                version: 1,
                data,
                ..
            } if capability == Self::CAPABILITY => serde_json::from_value(data.clone()).ok(),
            _ => None,
        }
    }
}

/// A screen: its panes in the universal layout, and the actions where the
/// focus is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct ScreenView {
    pub version: u32,
    /// The screen's id ([`crate::ScreenSpec::id`]).
    pub scope: String,
    pub panes: Vec<PaneView>,
    /// The smallest room for the nav pane and one content pane side by side.
    /// The frontend's default is 84 cells when this is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium_from: Option<u16>,
    /// What can be done in the focused pane, the actions menu's rows.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionSpec>,
    /// An item's page open over the screen: drawn in the whole body
    /// instead of the panes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<Box<PageView>>,
    /// Over the foot of the body (an [`Editor`] being written in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overlay: Option<Box<PaneView>>,
    /// The agent dock below the middle pane, on every screen. Filled by the
    /// host ([`crate::dock`]), never by the part whose screen this is; Ink
    /// draws it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock: Option<Box<crate::dock::DockView>>,
}

impl ScreenView {
    pub fn new(scope: impl Into<String>, panes: Vec<PaneView>) -> Self {
        Self {
            version: VERSION,
            scope: scope.into(),
            panes,
            medium_from: None,
            actions: vec![],
            page: None,
            overlay: None,
            dock: None,
        }
    }

    /// A screen showing an item's page.
    pub fn page(scope: impl Into<String>, page: PageView) -> Self {
        Self {
            page: Some(Box::new(page)),
            ..Self::new(scope, vec![])
        }
    }

    /// Whether the agent dock has a place on this screen: under the middle
    /// pane of a screen of panes. An item's page and the work area (a pane
    /// that is a `work.*` extension) have none; Ink draws no dock there
    /// (`dockShown`).
    pub fn has_dock_place(&self) -> bool {
        self.page.is_none()
            && !self.panes.iter().any(|p| matches!(&p.body, Node::Extension { capability, .. } if capability.starts_with("work.")))
    }

    pub fn pane(&self, pane: PaneId) -> Option<&PaneView> {
        self.panes.iter().find(|p| p.pane == pane)
    }
}

/// A settings section: its rows (a list, maybe with a field being typed
/// into), the detail beside them, and the actions on the selected row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct SectionView {
    pub version: u32,
    /// The section's id ([`crate::SectionSpec::id`]).
    pub scope: String,
    pub body: Node,
    pub detail_title: String,
    pub detail: Vec<Text>,
    /// Rich detail content, used when a section includes documents or media.
    /// When present, this replaces `detail` in the frontend.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail_blocks: Vec<Block>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionSpec>,
}

/// What a part is told when asked for a view: the clock, the focus, the
/// room the frontend has (a part may fit its words to it; it never places
/// cells), and the other parts, for their contributions.
pub struct ViewCx<'a> {
    pub now: i64,
    pub focus: PaneId,
    pub width: u16,
    pub height: u16,
    pub parts: crate::compose::Parts<'a>,
    /// Whether the frontend draws icons, and how wide their lead is.
    pub icon_lead: usize,
    /// The section is shown as a setup step: rows only Settings needs
    /// (refresh rates, read-back windows, session defaults) are left out.
    pub setup: bool,
}

impl ViewCx<'_> {
    pub fn has_focus(&self, pane: PaneId) -> bool {
        self.focus == pane
    }
}

/// A contributed column cell as view data: its words in its tone (strong
/// where the tone is), after its icon, and the subject it shows.
pub fn cell_of(cell: &crate::compose::Cell) -> CellView {
    use crate::plugin::Tone;
    let mut text = Text::new();
    if let Some(i) = cell.icon {
        text.add(Seg::icon(i));
    }
    let mut words = Seg::new(cell.text.clone(), Ink::from(cell.tone));
    words.strong = matches!(cell.tone, Tone::Accent | Tone::Warn | Tone::Bad);
    text.add(words);
    CellView {
        heading: cell.heading.clone(),
        text,
        show: cell.show.as_ref().map(SubjectRef::from),
        target: None,
    }
}

impl Intent {
    /// The key an intent stands for: a key's own, or an action invoked by a
    /// key's name (`Invoke { action: "d" }`). Stable action ids never parse
    /// as a key; the host delivers such an action by its key when the part
    /// does not match the id (transitional, M11 A0b).
    pub fn chord(&self) -> Option<Chord> {
        match self {
            Intent::Key { chord } => Some(*chord),
            Intent::Invoke { action } => Chord::parse(action),
            _ => None,
        }
    }
}

// --- navigation a frontend does without asking the part ----------------------------

/// Where the selection goes on a movement key in a list of `n` rows from
/// `at`; `None` when the key is not movement or does not move (the first
/// row's `↑` belongs to the host: it goes to the top bar).
pub fn list_step(n: usize, at: usize, chord: Chord) -> Option<usize> {
    if n == 0 || chord.ctrl || chord.alt {
        return None;
    }
    let last = n - 1;
    let to = match chord.key {
        Key::Down | Key::Char('j') => (at + 1).min(last),
        Key::Up | Key::Char('k') if at == 0 => return None,
        Key::Up | Key::Char('k') => at - 1,
        Key::Home | Key::Char('g') => 0,
        Key::End | Key::Char('G') => last,
        _ => return None,
    };
    Some(to)
}

/// [`list_step`] over rows some of which are headings: the selection goes
/// past a heading to the next row that is not one (`↑` from the first such
/// row is the host's, as from the first row).
pub fn rows_step(rows: &[ListRow], at: usize, chord: Chord) -> Option<usize> {
    rows_step_by(rows.len(), at, chord, |i| rows[i].heading)
}

/// [`rows_step`] over `n` rows, `heading` saying which are headings.
pub fn rows_step_by(
    n: usize,
    at: usize,
    chord: Chord,
    heading: impl Fn(usize) -> bool,
) -> Option<usize> {
    let can = |i: usize| i < n && !heading(i);
    let first = (0..n).find(|i| can(*i))?;
    let last = (0..n).rev().find(|i| can(*i))?;
    if chord.ctrl || chord.alt {
        return None;
    }
    let to = match chord.key {
        Key::Down | Key::Char('j') => ((at + 1)..n)
            .find(|i| can(*i))
            .unwrap_or(at.max(first).min(last)),
        Key::Up | Key::Char('k') if at <= first => return None,
        Key::Up | Key::Char('k') => (0..at).rev().find(|i| can(*i)).unwrap_or(first),
        Key::Home | Key::Char('g') => first,
        Key::End | Key::Char('G') => last,
        _ => return None,
    };
    Some(to)
}

/// Whether the shared tree owns a key, including no-op boundaries. Shifted
/// arrows and modified keys retain their existing outer dispatch ownership;
/// printable `+` and `G` may arrive with Shift from a terminal.
pub fn tree_key(chord: Chord) -> bool {
    !chord.ctrl && !chord.alt && match chord.key {
        Key::Up | Key::Down | Key::Left | Key::Right | Key::Home | Key::End => !chord.shift,
        Key::Char('j' | 'k' | 'g' | 'G' | ' ' | '+' | '-' | '=') => true,
        _ => false,
    }
}

/// What a movement key does in a tree: select another row, open or close a
/// group, or open every held group. `None` also means an owned no-op; callers
/// use [`tree_key`] to keep boundary left/right from moving into another pane.
/// A stale selection never retargets a row-local fold to the first row.
pub fn tree_step(tree: &Tree, chord: Chord) -> Option<Intent> {
    if !tree_key(chord) {
        return None;
    }
    if chord.key == Key::Char('=') {
        return Some(Intent::OpenAll);
    }
    let shown = tree.visible();
    let target = |i: usize| TreeRow::target_of(&shown[i].0);
    let select = |i: usize| Intent::Select { target: target(i) };
    let at = shown.iter().position(|(path, row)| *path == tree.selected && !row.heading);
    let Some(at) = at else {
        return matches!(chord.key, Key::Up | Key::Down | Key::Home | Key::End | Key::Char('j' | 'k' | 'g' | 'G'))
            .then(|| shown.iter().position(|(_, row)| !row.heading).map(select))
            .flatten();
    };
    let (path, row) = &shown[at];
    let group = !row.children.is_empty();
    let open = tree.open.contains(path);
    let parent = || (0..at).rev().find(|&i| {
        !shown[i].1.heading && shown[i].0.len() < path.len() && path.starts_with(&shown[i].0)
    });
    let fold = |i: usize, open| Intent::Fold { target: target(i), open };
    match chord.key {
        Key::Char('+') if group && !open => Some(fold(at, true)),
        Key::Right if group && !open => Some(fold(at, true)),
        Key::Right if group => shown.iter().enumerate().skip(at + 1)
            .take_while(|(_, (p, _))| p.starts_with(path))
            .find(|(_, (_, row))| !row.heading).map(|(i, _)| select(i)),
        Key::Left if group && open => Some(fold(at, false)),
        Key::Left => parent().map(select),
        Key::Char('-') if group && open => Some(fold(at, false)),
        Key::Char('-') => parent().map(|i| fold(i, false)),
        Key::Char(' ') if group => Some(fold(at, !open)),
        _ => rows_step_by(shown.len(), at, chord, |i| shown[i].1.heading).map(select),
    }
}

/// A tree's selection and open groups, kept by the part that owns the rows
/// (what is selected decides what its actions do). Groups open the first
/// time they appear, as the legacy tree did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TreeNav {
    pub selected: Vec<String>,
    pub open: Vec<Vec<String>>,
    seen: std::collections::BTreeSet<Vec<String>>,
}

impl TreeNav {
    pub fn selected(&self) -> &[String] {
        &self.selected
    }

    pub fn select(&mut self, path: Vec<String>) {
        self.selected = path;
    }

    pub fn is_open(&self, path: &[String]) -> bool {
        self.open.iter().any(|p| p == path)
    }

    /// Open a group (once).
    pub fn open(&mut self, path: Vec<String>) {
        if !self.is_open(&path) {
            self.open.push(path);
        }
    }

    pub fn close_all(&mut self) {
        self.open.clear();
    }

    /// Open the selected group, or close it when it is open.
    pub fn toggle_selected(&mut self) {
        let path = self.selected.clone();
        if self.is_open(&path) {
            self.open.retain(|p| *p != path);
        } else {
            self.open.push(path);
        }
    }

    /// Open groups seen for the first time; keep the selection on a row
    /// that is there (else the first row).
    pub fn sync(&mut self, rows: &[TreeRow]) {
        fn groups(rows: &[TreeRow], prefix: &[String], out: &mut Vec<Vec<String>>) {
            for r in rows {
                let mut p = prefix.to_vec();
                p.push(r.id.clone());
                if !r.children.is_empty() {
                    out.push(p.clone());
                    groups(&r.children, &p, out);
                }
            }
        }
        let mut all = Vec::new();
        groups(rows, &[], &mut all);
        self.open.retain(|path| all.contains(path));
        for g in all {
            if self.seen.insert(g.clone()) && !self.open.contains(&g) {
                self.open.push(g);
            }
        }
        let probe = Tree {
            rows: rows.to_vec(),
            open: self.open.clone(),
            ..Tree::default()
        };
        let shown = probe.visible();
        if !shown.iter().any(|(p, row)| *p == self.selected && !row.heading) {
            // A row whose group closed keeps the selection on the group.
            let mut up = self.selected.clone();
            while !up.is_empty() && !shown.iter().any(|(p, row)| *p == up && !row.heading) {
                up.pop();
            }
            self.selected = if up.is_empty() {
                shown.iter().find(|(_, row)| !row.heading).map(|(p, _)| p.clone()).unwrap_or_default()
            } else {
                up
            };
        }
    }

    /// Apply a selection or fold intent; true when it was one.
    pub fn apply(&mut self, intent: &Intent) -> bool {
        match intent {
            Intent::Select { target } | Intent::Context { target } => {
                self.selected = TreeRow::path_of(target);
                true
            }
            Intent::Fold { target, open } => {
                let path = TreeRow::path_of(target);
                self.open.retain(|p| *p != path);
                if *open {
                    self.open.push(path);
                } else if self.selected.starts_with(&path) && self.selected.len() > path.len() {
                    self.selected = path;
                }
                true
            }
            _ => false,
        }
    }
}

pub mod chrome;
pub mod clean;
pub mod detail;
pub mod diff;
pub mod doc;
pub mod kit;
pub mod measure;
pub mod scroll;

pub use chrome::{Choice, ChromeView, MenuView};

#[cfg(test)]
mod tests;
