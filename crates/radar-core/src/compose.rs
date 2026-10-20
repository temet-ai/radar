//! How the parts fit together. radar-core is a clean interface that knows no
//! tool; the parts integrate the tools, and never name each other. They meet
//! here, through three things:
//!
//! - **subjects**: references to things (a kind the parts agree on, an id,
//!   a label, a link, and fields others can match on). Core never looks
//!   inside a kind: the names and fields are the parts' shared vocabulary,
//!   written down in GUIDE.md;
//! - **showing**: any part asks the host to show a subject
//!   ([`Effect::Show`](crate::Effect::Show)); the part that owns its kind
//!   opens it, and `esc` comes back to where you were. With no owner, its
//!   link opens instead;
//! - **contributions**: a part that draws a subject offers places for it
//!   ([`Point`]s: chips on its row, a column, a section, a tab, actions);
//!   every other part may fill them ([`Plugin::contribute`]), from its own
//!   cache, never fetching. The owner draws what it is given without knowing
//!   who gave it.
//!
//! Absent means absent: a part that is not compiled in, or not set up, adds
//! nothing anywhere.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

use crate::plugin::{Effect, Plugin, Tone};

/// The one kind core knows: a bare link. Showing one asks the parts whether
/// they recognise it, and opens it in the browser when none does.
pub const LINK: &str = "link";

/// A thing radar knows about, whoever owns it. Two subjects are the same thing
/// when their kind and id are.
#[derive(Debug, Clone)]
pub struct Subject {
    /// What sort of thing, by the name the parts agree on (`"book"`).
    pub kind: String,
    /// Which one, unique within its kind.
    pub id: String,
    /// A short name for menus and the status line.
    pub label: String,
    /// Where it lives on the web: opened when no part here shows it.
    pub url: Option<String>,
    /// What other parts may match on (a title, a branch), by name.
    pub fields: BTreeMap<String, String>,
    /// References supplied by the other parts before an action runs.
    pub related: Vec<Subject>,
}

impl PartialEq for Subject {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind && self.id == other.id
    }
}

impl Eq for Subject {}

impl std::hash::Hash for Subject {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.id.hash(state);
    }
}

impl Subject {
    pub fn new(kind: impl Into<String>, id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
            label: label.into(),
            url: None,
            fields: BTreeMap::new(),
            related: Vec::new(),
        }
    }

    /// A bare link.
    pub fn link(url: impl Into<String>) -> Self {
        let url = url.into();
        Self::new(LINK, url.clone(), url.clone()).with_url(url)
    }

    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }

    /// Add a field others can match on.
    pub fn with(mut self, field: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(field.into(), value.into());
        self
    }

    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }

    pub fn is(&self, kind: &str) -> bool {
        self.kind == kind
    }
}

/// A kind a part owns: its name in the vocabulary, how people call such
/// things (`books`), and whether the part opens a page for one in radar (or
/// shows it by its link).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Owned {
    pub kind: &'static str,
    pub name: &'static str,
    pub page: bool,
}

impl Owned {
    /// Shown on a page of the part's own.
    pub const fn page(kind: &'static str, name: &'static str) -> Self {
        Self {
            kind,
            name,
            page: true,
        }
    }

    /// Shown by opening its link.
    pub const fn link(kind: &'static str, name: &'static str) -> Self {
        Self {
            kind,
            name,
            page: false,
        }
    }
}

/// A place an owner offers for others to fill, on a subject it draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Point {
    /// Small labelled chips at the end of the subject's row.
    Chips,
    /// A column on a list of such subjects, shown when some row has a value.
    Column,
    /// A titled block in the subject's page (its overview).
    Section,
    /// A whole tab on the subject's page.
    Tab,
    /// Rows in the subject's `x` menu, under the contributor's name.
    Actions,
    /// One line under the subject's title on its page: where it is, or what
    /// another part knows of it, in a few words.
    Line,
    /// The subject's mark on its row: an icon the owner draws in place of
    /// its own (the kind's), and a few words in place of its own detail
    /// (a repo that is cloned here, on its branch).
    Mark,
}

impl Point {
    /// How the parts section says it: `chips on`, `a tab on`.
    pub fn words(self) -> &'static str {
        match self {
            Self::Chips => "chips on",
            Self::Column => "a column on",
            Self::Section => "a section on",
            Self::Tab => "a tab on",
            Self::Actions => "actions on",
            Self::Line => "a line on",
            Self::Mark => "a mark on",
        }
    }
}

/// A chip: a few words with a feeling (`ABC-12 · Review`, warn).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip {
    pub text: String,
    pub tone: Tone,
    /// Where the chip goes, when it is a link: drawn as one the terminal
    /// opens (a chip).
    pub url: Option<String>,
    /// An icon before the words, drawn in its own colour on the chip
    /// (a chip with an icon); nothing with icons off.
    pub icon: Option<crate::ui::icon::Icon>,
    /// One of the contributor's actions on the subject (an [`Action`]'s
    /// `id`): a click on the chip does it, through `Effect::Act`.
    pub action: Option<String>,
}

impl Chip {
    pub fn new(text: impl Into<String>, tone: Tone) -> Self {
        Self {
            text: text.into(),
            tone,
            url: None,
            icon: None,
            action: None,
        }
    }

    /// A click on the chip does the contributor's action `id`.
    pub fn acts(mut self, id: impl Into<String>) -> Self {
        self.action = Some(id.into());
        self
    }

    /// An icon before the words, in its own colour.
    pub fn icon(mut self, icon: impl Into<Option<crate::ui::icon::Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    /// The chip opens `url`.
    pub fn link(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self
    }
}

/// One column's value on a subject's row, under `heading`: a few words
/// with a feeling, and what clicking it (or its `x` row) shows, if anything
/// (`note` · `N-1 · open`, showing the note). The owner cuts it to
/// the column and drops the column whole when the row has no room.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    pub heading: String,
    /// The value's words; the owner draws them in `tone`.
    pub text: String,
    pub tone: Tone,
    pub show: Option<Subject>,
    /// An icon before the value, drawn by the frontend in its own colour.
    pub icon: Option<crate::ui::icon::Icon>,
}

impl Cell {
    pub fn new(heading: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            heading: heading.into(),
            text: text.into(),
            tone: Tone::Plain,
            show: None,
            icon: None,
        }
    }

    /// An icon before the value, in its own colour.
    pub fn icon(mut self, icon: impl Into<Option<crate::ui::icon::Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn tone(mut self, tone: Tone) -> Self {
        self.tone = tone;
        self
    }

    /// What the cell opens when clicked.
    pub fn show(mut self, subject: Subject) -> Self {
        self.show = Some(subject);
        self
    }

}

/// A row of a contributed tab; `⏎` on it shows its subject, if it has one.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Renderer-neutral words ([`crate::view::Text`]); the owner paints them.
    pub line: crate::view::Text,
    pub subject: Option<Subject>,
}

/// A contributed row of an `x` menu. Choosing it calls the contributor's
/// [`Plugin::act_with_related`] with the subject, `id` and related subjects
/// from the other parts. `key` is kept only when no one else here uses it
/// (the owner, the host, an earlier part); otherwise the row is chosen from
/// the menu only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Action {
    pub key: Option<char>,
    pub what: String,
    pub id: String,
    /// A write to a service: what it does (the host mutes the row and says
    /// why while the mode refuses it).
    pub change: Option<crate::writes::Change>,
    /// What it is about, before its words where it is drawn (a menu row,
    /// an owner's list of actions); nothing with icons off.
    pub icon: Option<crate::ui::icon::Icon>,
}

impl Action {
    pub fn new(key: char, what: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            key: Some(key),
            what: what.into(),
            id: id.into(),
            change: None,
            icon: None,
        }
    }

    /// An icon before its words.
    pub fn icon(mut self, icon: impl Into<Option<crate::ui::icon::Icon>>) -> Self {
        self.icon = icon.into();
        self
    }

    /// It writes something new to a service.
    pub fn adds(mut self) -> Self {
        self.change = Some(crate::writes::Change::Add);
        self
    }

    /// It changes something there on a service.
    pub fn edits(mut self) -> Self {
        self.change = Some(crate::writes::Change::Edit);
        self
    }
}

/// The most lines of another part's section a detail pane shows
/// ([`Parts::detail_sections`]).
pub const DETAIL_SECTION_MOST: usize = 10;

/// What a part puts in a [`Point`].
#[derive(Debug, Clone, PartialEq)]
pub enum Contribution {
    Chips(Vec<Chip>),
    /// One column's value on this subject's row.
    Cell(Cell),
    /// A section under the owner's own: its mark (drawn before the title,
    /// as the owner's sections have one), title and lines. A detail pane
    /// shows at most [`DETAIL_SECTION_MOST`] of the lines; the page all.
    Section {
        icon: Option<crate::ui::icon::Icon>,
        title: String,
        lines: Vec<crate::view::Text>,
    },
    /// A tab of rows, drawn by the owner in its page body with its own keys
    /// (scrolling, `[`/`]`, `⏎` shows a row's subject).
    Tab {
        title: String,
        rows: Vec<Row>,
    },
    Actions(Vec<Action>),
    /// The line under the subject's title: words in a row, each in its tone
    /// after its icon (drawn as words, not as chips).
    Line(Vec<Chip>),
    /// The row's mark: its icon, and words in place of the owner's detail.
    Mark { icon: crate::ui::icon::Icon, words: Vec<Chip> },
}

/// A place a part offers others, in words for the parts section:
/// `Offer::new(Point::Chips, "book", "book rows")`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer {
    pub point: Point,
    /// The kind of subject it is offered on.
    pub on: &'static str,
    pub place: &'static str,
}

impl Offer {
    pub const fn new(point: Point, on: &'static str, place: &'static str) -> Self {
        Self { point, on, place }
    }
}

/// What a part adds to others, in words: `Adds::new("book", "note chips")`
/// reads "note chips on books" (the kind named as its owner names it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Adds {
    pub on: &'static str,
    pub what: &'static str,
}

impl Adds {
    pub const fn new(on: &'static str, what: &'static str) -> Self {
        Self { on, what }
    }
}

/// Whether a part can work yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setup {
    Ready,
    /// Compiled in, not configured: what is needed, in a sentence. Its
    /// screen shows a short set-up page and it contributes nothing.
    Needed(String),
}

/// How an owner answered [`Effect::Show`].
pub enum Showing {
    /// It opened a page on this screen; the host switches to it and
    /// remembers where you came from.
    Page {
        screen: &'static str,
        effects: Vec<Effect>,
    },
    /// It did what showing means for this kind (e.g. opened a link).
    Done(Vec<Effect>),
    /// It cannot; the host falls back to the subject's link.
    Cannot,
}

/// Whether a part works now.
pub fn ready(p: &dyn Plugin) -> bool {
    p.set_up() == Setup::Ready
}

/// The set-up part that owns `kind`, first in plugin order, and how.
pub fn owner_of(plugins: &[Box<dyn Plugin>], kind: &str) -> Option<(usize, Owned)> {
    plugins.iter().enumerate().find_map(|(i, p)| {
        p.owns()
            .into_iter()
            .find(|o| o.kind == kind)
            .filter(|_| ready(p.as_ref()))
            .map(|o| (i, o))
    })
}

/// Subjects in `text`, as the parts that own their kinds recognise them:
/// in order, the longer first where two start together, overlaps dropped.
pub fn recognise(plugins: &[Box<dyn Plugin>], text: &str) -> Vec<(Range<usize>, Subject)> {
    let mut found: Vec<(Range<usize>, Subject)> = plugins
        .iter()
        .filter(|p| ready(p.as_ref()))
        .flat_map(|p| {
            let owns: Vec<&str> = p.owns().iter().map(|o| o.kind).collect();
            p.recognise(text)
                .into_iter()
                .filter(move |(_, s)| owns.contains(&s.kind.as_str()))
        })
        .collect();
    found.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
    let mut out: Vec<(Range<usize>, Subject)> = Vec::new();
    for (r, s) in found {
        if out.last().is_none_or(|(last, _)| r.start >= last.end) {
            out.push((r, s));
        }
    }
    out
}

/// One resolved action input. Its private subject is also the exact input
/// delivered to the receiver; recording must not resolve the references again.
pub(crate) struct PreparedAction {
    part: usize,
    subject: Subject,
}

impl PreparedAction {
    pub(crate) fn subject(&self) -> &Subject {
        &self.subject
    }

    pub(crate) fn dispatch(self, plugins: &mut [Box<dyn Plugin>], action: &str) -> Vec<Effect> {
        plugins[self.part].act_with_related(&self.subject, action, &self.subject.related)
    }
}

/// Resolve one level once, preserving the action input's existing references
/// verbatim and the established kind/id ordering of newly gathered references.
pub(crate) fn prepare_action(
    plugins: &[Box<dyn Plugin>],
    part: usize,
    subject: &Subject,
) -> PreparedAction {
    let mut enriched = subject.clone();
    for related in Parts::new(plugins, Some(part)).related(subject) {
        if related != *subject && !enriched.related.contains(&related) {
            enriched.related.push(related);
        }
    }
    PreparedAction {
        part,
        subject: enriched,
    }
}

/// Run an action with the references known by the other ready parts, both
/// on the subject and through the related-action hook. Resolve one level only.
pub fn act(
    plugins: &mut [Box<dyn Plugin>],
    part: usize,
    subject: &Subject,
    action: &str,
) -> Vec<Effect> {
    prepare_action(plugins, part, subject).dispatch(plugins, action)
}

/// The other parts, as one drawing sees them: what is here, and what they
/// add to the subject being drawn. Handed to plugins in [`ViewCx`](crate::view::ViewCx).
#[derive(Clone, Copy)]
pub struct Parts<'a> {
    plugins: &'a [Box<dyn Plugin>],
    /// The part asking; its own contributions and relations are not asked for.
    me: Option<usize>,
}

impl<'a> Parts<'a> {
    pub fn new(plugins: &'a [Box<dyn Plugin>], me: Option<usize>) -> Self {
        Self { plugins, me }
    }

    /// No other parts: for drawing a plugin on its own in tests.
    pub fn none() -> Parts<'static> {
        Parts {
            plugins: &[],
            me: None,
        }
    }

    /// Whether a set-up part owns `kind`: things of that kind exist here.
    pub fn has(&self, kind: &str) -> bool {
        owner_of(self.plugins, kind).is_some()
    }

    /// Whether showing a subject of `kind` opens a page in radar, rather than
    /// its link.
    pub fn can_show(&self, kind: &str) -> bool {
        owner_of(self.plugins, kind).is_some_and(|(_, o)| o.page)
    }

    /// Subjects in `text` (see [`recognise`]).
    pub fn recognise(&self, text: &str) -> Vec<(Range<usize>, Subject)> {
        recognise(self.plugins, text)
    }

    /// Subjects the other set-up parts relate to `subject`, in plugin
    /// order. Each kind and id appears once; the first one's fields win.
    /// Only the given subject is queried, never its relations in turn.
    pub fn related(&self, subject: &Subject) -> Vec<Subject> {
        let mut seen = HashSet::new();
        self.plugins
            .iter()
            .enumerate()
            .filter(|(i, p)| Some(*i) != self.me && ready(p.as_ref()))
            .flat_map(|(_, p)| p.related(subject))
            .filter(|s| seen.insert(s.clone()))
            .collect()
    }

    /// What every other set-up part puts at `point` on `subject`, in plugin
    /// order, with the part's name.
    pub fn contributions(
        &self,
        point: Point,
        subject: &Subject,
    ) -> Vec<(&'static str, Contribution)> {
        self.plugins
            .iter()
            .enumerate()
            .filter(|(i, p)| Some(*i) != self.me && ready(p.as_ref()))
            .filter_map(|(_, p)| p.contribute(point, subject).map(|c| (p.name(), c)))
            .collect()
    }

    /// Every chip the other parts put on `subject`'s row.
    pub fn chips(&self, subject: &Subject) -> Vec<Chip> {
        self.contributions(Point::Chips, subject)
            .into_iter()
            .flat_map(|(_, c)| match c {
                Contribution::Chips(chips) => chips,
                _ => vec![],
            })
            .collect()
    }

    /// Each contributed column's cell on `subject`'s row.
    pub fn cells(&self, subject: &Subject) -> Vec<Cell> {
        self.contributions(Point::Column, subject)
            .into_iter()
            .filter_map(|(_, c)| match c {
                Contribution::Cell(cell) => Some(cell),
                _ => None,
            })
            .collect()
    }

    /// `(title, lines)` for each contributed section of `subject`'s page.
    pub fn sections(&self, subject: &Subject) -> Vec<(String, Vec<crate::view::Text>)> {
        self.contributions(Point::Section, subject)
            .into_iter()
            .filter_map(|(_, c)| match c {
                Contribution::Section { title, lines, .. } => Some((title, lines)),
                _ => None,
            })
            .collect()
    }

    /// Each contributed section of `subject` as a detail pane draws it: its
    /// mark and title, then at most [`DETAIL_SECTION_MOST`] lines and how
    /// many more the page has. A summary beside the owner's own sections,
    /// so no one part's words push the rest out of the pane.
    pub fn detail_sections(&self, subject: &Subject) -> Vec<crate::view::Block> {
        use crate::view::{Block, Ink, Text};
        self.contributions(Point::Section, subject)
            .into_iter()
            .filter_map(|(_, c)| match c {
                Contribution::Section { icon, title, mut lines } => {
                    if lines.len() > DETAIL_SECTION_MOST {
                        let more = lines.len() - (DETAIL_SECTION_MOST - 1);
                        lines.truncate(DETAIL_SECTION_MOST - 1);
                        lines.push(Text::of(format!("⋯ {more} more lines on its page"), Ink::Muted));
                    }
                    Some(Block::section_of(icon, &title, None, lines))
                }
                _ => None,
            })
            .collect()
    }

    /// Each contributed line under `subject`'s title, as its words, with the
    /// part's name (what a chip's action is handed back to, `Effect::Act`).
    pub fn lines(&self, subject: &Subject) -> Vec<(&'static str, Vec<Chip>)> {
        self.contributions(Point::Line, subject)
            .into_iter()
            .filter_map(|(name, c)| match c {
                Contribution::Line(line) => Some((name, line)),
                _ => None,
            })
            .collect()
    }

    /// The first mark another part puts on `subject`'s row: the icon to draw
    /// in place of the owner's, and the words in place of its detail.
    pub fn mark(&self, subject: &Subject) -> Option<(crate::ui::icon::Icon, Vec<Chip>)> {
        self.contributions(Point::Mark, subject).into_iter().find_map(|(_, c)| match c {
            Contribution::Mark { icon, words } => Some((icon, words)),
            _ => None,
        })
    }

    /// The actions the other parts add to `subject`, each part's under its
    /// name, with the keys the `x` menu keeps: a key the drawer's own rows
    /// use (`taken`), the host keeps, or an earlier part took is dropped.
    /// For an owner that lists them where it draws the subject; a click on
    /// one returns [`Effect::Press`](crate::Effect::Press) with its key.
    pub fn actions(&self, subject: &Subject, taken: &[char]) -> Vec<(&'static str, Vec<Action>)> {
        let mut taken = taken.to_vec();
        let mut out = Vec::new();
        for (name, c) in self.contributions(Point::Actions, subject) {
            let Contribution::Actions(mut actions) = c else { continue };
            for a in &mut actions {
                if let Some(k) = a.key {
                    if taken.contains(&k) || crate::host::host_key(k) {
                        a.key = None;
                    } else {
                        taken.push(k);
                    }
                }
            }
            if !actions.is_empty() {
                out.push((name, actions));
            }
        }
        out
    }

    /// `(title, rows)` for each contributed tab of `subject`'s page.
    pub fn tabs(&self, subject: &Subject) -> Vec<(String, Vec<Row>)> {
        self.contributions(Point::Tab, subject)
            .into_iter()
            .filter_map(|(_, c)| match c {
                Contribution::Tab { title, rows } => Some((title, rows)),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
