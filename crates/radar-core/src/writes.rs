//! Writes to a service, one behaviour in every part (docs/GUIDE.md 2.23):
//! the mode that says which writes radar may send, the question before one,
//! and the words said while one is sent and after.
//!
//! - **The mode.** radar is in one of three modes ([`Mode`]): **read only**
//!   (it changes nothing on any service), **write** (it may add things: a
//!   comment, a reply, a reaction, a new item) or **edit** (it may also
//!   change what is there: approve, resolve, move, assign, rebase, merge).
//!   Each write says which it is ([`Change::Add`] or [`Change::Edit`]) and
//!   [`Mode::allows`] decides, in this one place. The host holds the mode
//!   (`[general] mode`, or [`MODE_VAR`] for one run, or the old switches
//!   [`NO_WRITES`] and `RADAR_*_NO_WRITES`, which mean read only) and tells
//!   this thread ([`set_mode`]); a part asks [`not_sent`] before it sends.
//! - **The question**: [`Menu::write_confirm`](crate::Menu::write_confirm)
//!   for a write that sends text, showing all of it;
//!   [`Menu::write_plain`](crate::Menu::write_plain) for one that does not.
//!   The host adds the mode's note before `y` when the mode refuses it
//!   ([`confirm_note`]); a row of the `x` menu says so too ([`row_note`]).
//! - **The words**: `asking {service} to {what}…` ([`Service::asking`]),
//!   then `✓ {done}` ([`done`]) or `✗ {service} did not {what}: {why}`
//!   ([`failed`] of [`Service::did_not`]); a write the service takes and
//!   does later is `{service} accepted {what}; checking…` ([`Service::taken`])
//!   until it says how it went, or [`Service::no_answer`]; the notices carry the pack's
//!   icons ([`Effect::notify`]) when icons are on, the symbols otherwise.
//!   Refused or declined text is kept for the key that writes it again
//!   ([`kept`]).

use std::cell::RefCell;

use crate::plugin::Effect;
use crate::ui::icon::{self, Icon};

/// Sets the mode for one run: `read-only`, `write` or `edit`.
pub const MODE_VAR: &str = "RADAR_MODE";

/// The old switch: any value but empty or `0` means read only. Each part's
/// own `RADAR_{PART}_NO_WRITES` means the same.
pub const NO_WRITES: &str = "RADAR_NO_WRITES";

/// Whether a switch's value turns writes off: unset, empty or `0` leave
/// them on; anything else turns them off.
pub fn is_off(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.is_empty() && v != "0")
}

/// What a write does to a service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Change {
    /// Adds something: a comment, a reply, a reaction, a new item.
    Add,
    /// Changes something there: approve, resolve, move, assign, merge.
    Edit,
}

/// Which writes radar may send.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum Mode {
    /// Nothing is sent.
    ReadOnly,
    /// Things may be added.
    Write,
    /// Things may be added and changed.
    #[default]
    Edit,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::ReadOnly, Mode::Write, Mode::Edit];

    /// As said: `read only`, `write`, `edit`.
    pub fn word(self) -> &'static str {
        match self {
            Mode::ReadOnly => "read only",
            Mode::Write => "write",
            Mode::Edit => "edit",
        }
    }

    /// As written in the config and [`MODE_VAR`]: `read-only`, `write`, `edit`.
    pub fn key(self) -> &'static str {
        match self {
            Mode::ReadOnly => "read-only",
            Mode::Write => "write",
            Mode::Edit => "edit",
        }
    }

    /// `read-only` (or `read only`, `readonly`), `write`, `edit`.
    pub fn parse(s: &str) -> Option<Mode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "read-only" | "read only" | "readonly" | "read_only" => Some(Mode::ReadOnly),
            "write" => Some(Mode::Write),
            "edit" => Some(Mode::Edit),
            _ => None,
        }
    }

    /// What it lets radar do, in a line.
    pub fn about(self) -> &'static str {
        match self {
            Mode::ReadOnly => "changes nothing anywhere: every write is shown and asked, and none is sent",
            Mode::Write => "may add things: comments, replies, reactions, review asks, new items",
            Mode::Edit => "may also change what is there: approve, resolve, move, assign, rebase, merge",
        }
    }

    /// Its icon in the top bar and the menu.
    pub fn icon(self) -> Icon {
        match self {
            Mode::ReadOnly => Icon::ModeRead,
            Mode::Write => Icon::ModeWrite,
            Mode::Edit => Icon::ModeEdit,
        }
    }

    /// Whether a write that does `change` may be sent.
    pub fn allows(self, change: Change) -> bool {
        match change {
            Change::Add => self >= Mode::Write,
            Change::Edit => self >= Mode::Edit,
        }
    }

    /// Why it refuses `change`, if it does: `radar is read only`, `needs edit mode`.
    pub fn refuses(self, change: Change) -> Option<&'static str> {
        match (self.allows(change), self) {
            (true, _) => None,
            (false, Mode::ReadOnly) => Some("radar is read only"),
            (false, _) => Some("needs edit mode"),
        }
    }
}

/// The mode the environment sets for this run, and the variable that set
/// it: [`MODE_VAR`] when it names a mode, else read only when [`NO_WRITES`]
/// or any `RADAR_*_NO_WRITES` is on.
pub fn env_mode(vars: impl IntoIterator<Item = (String, String)>) -> Option<(Mode, String)> {
    let mut off: Option<String> = None;
    let mut set: Option<(Mode, String)> = None;
    for (k, v) in vars {
        if k == MODE_VAR {
            if let Some(m) = Mode::parse(&v) {
                set = Some((m, k));
            }
        } else if k.starts_with("RADAR_") && k.ends_with("NO_WRITES") && is_off(Some(&v)) {
            // The shared name first, then any part's, by name.
            let better = match &off {
                None => true,
                Some(o) => k == NO_WRITES || (o != NO_WRITES && k < *o),
            };
            if better {
                off = Some(k);
            }
        }
    }
    set.or_else(|| off.map(|k| (Mode::ReadOnly, k)))
}

thread_local! {
    static MODE: RefCell<(Mode, Option<String>)> = const { RefCell::new((Mode::Edit, None)) };
}

/// Say the mode on this thread (the host, whenever it changes), and the
/// variable that set it, when one did.
pub fn set_mode(mode: Mode, by: Option<String>) {
    MODE.with(|m| *m.borrow_mut() = (mode, by));
}

/// The mode on this thread, and the variable that set it, if one did.
pub fn mode() -> (Mode, Option<String>) {
    MODE.with(|m| m.borrow().clone())
}

/// Why a write that does `change` is not sent now, if it is not:
/// `not sent: radar is read only`, `not sent: radar is in write mode; this needs
/// edit mode`. A part asks it just before it sends, and reports it as a
/// refusal (the text kept).
pub fn not_sent(change: Change) -> Option<String> {
    let (m, _) = mode();
    m.refuses(change).map(|_| match m {
        Mode::ReadOnly => "not sent: radar is read only".to_string(),
        _ => format!("not sent: radar is in {} mode; this needs edit mode", m.word()),
    })
}

/// The line a write's question shows before `y` while the mode refuses it.
pub fn confirm_note(change: Change) -> Option<String> {
    let (m, by) = mode();
    m.refuses(change)?;
    let set = by.map(|v| format!(" (set by {v})")).unwrap_or_default();
    Some(match m {
        Mode::ReadOnly => format!("radar is read only{set}: nothing will be sent"),
        _ => format!("radar is in {} mode{set}: this needs edit mode, nothing will be sent", m.word()),
    })
}

/// What an `x` menu row adds when the mode refuses it: `radar is read only`,
/// `needs edit mode`.
pub fn row_note(change: Change) -> Option<&'static str> {
    mode().0.refuses(change)
}

/// A service a part writes to, by its name in words.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Service {
    /// As said to the user: `asking {name} to …`.
    pub name: &'static str,
}

impl Service {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    /// While it is sent: `asking {service} to {what}…`.
    pub fn asking(&self, what: &str) -> String {
        format!("asking {} to {what}…", self.name)
    }

    /// The words when it was refused or not sent: `{service} did not
    /// {what}: {why}`, said with [`failed`] (or written with [`failed_line`]).
    pub fn did_not(&self, what: &str, why: &str) -> String {
        format!("{} did not {what}: {why}", self.name)
    }

    /// A write the service took but does later: `{service} accepted {what};
    /// checking…`, while the part asks until the service says how it went.
    pub fn taken(&self, what: &str) -> String {
        format!("{} accepted {what}; checking…", self.name)
    }

    /// No answer about a write it took: `no answer from {service} about
    /// {what}; the next sync will show it`.
    pub fn no_answer(&self, what: &str) -> String {
        format!("no answer from {} about {what}; the next sync will show it", self.name)
    }
}

/// The notice when it was taken: `✓ {words}`, the pack's icon in its
/// colour when icons are on.
pub fn done(words: &str) -> Effect {
    let (icon, mark) = icon::plain_or(Icon::RunPassed, "✓");
    Effect::notify(icon, format!("{mark}{words}"))
}

/// The notice when it was refused or not sent: `✗ {words}`, the pack's
/// icon in its colour when icons are on.
pub fn failed(words: &str) -> Effect {
    let (icon, mark) = icon::plain_or(Icon::Error, "✗");
    Effect::error(icon, format!("{mark}{words}"))
}

/// [`done`]'s words as one line of text, for a page that records it.
pub fn done_line(words: &str) -> String {
    format!("{}{words}", lead(Icon::RunPassed, "✓"))
}

/// [`failed`]'s words as one line of text, for a page that records it.
pub fn failed_line(words: &str) -> String {
    format!("{}{words}", lead(Icon::Error, "✗"))
}

/// Said after a refused or declined write whose text was kept:
/// ` · {key} has your text`.
pub fn kept(key: &str) -> String {
    format!(" · {key} has your text")
}

/// The icon's glyph and a space, or `fallback` and a space with icons off.
fn lead(i: Icon, fallback: &str) -> String {
    let l = icon::plain(i);
    if l.is_empty() { format!("{fallback} ") } else { l }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACME: Service = Service::new("Acme");

    fn env(vars: &[(&str, &str)]) -> Option<(Mode, String)> {
        env_mode(vars.iter().map(|(k, v)| (k.to_string(), v.to_string())))
    }

    #[test]
    fn each_mode_allows_what_it_says() {
        use Change::{Add, Edit};
        assert!(!Mode::ReadOnly.allows(Add) && !Mode::ReadOnly.allows(Edit));
        assert!(Mode::Write.allows(Add) && !Mode::Write.allows(Edit));
        assert!(Mode::Edit.allows(Add) && Mode::Edit.allows(Edit));
        assert_eq!(Mode::ReadOnly.refuses(Add), Some("radar is read only"));
        assert_eq!(Mode::Write.refuses(Edit), Some("needs edit mode"));
        assert_eq!(Mode::default(), Mode::Edit, "edit by default: as before modes");
        for m in Mode::ALL {
            assert_eq!(Mode::parse(m.key()), Some(m));
        }
        assert_eq!(Mode::parse("read only"), Some(Mode::ReadOnly));
        assert_eq!(Mode::parse("sometimes"), None);
    }

    #[test]
    fn the_environment_sets_the_mode_and_the_old_switches_mean_read_only() {
        assert_eq!(env(&[]), None, "unset: the config's");
        assert_eq!(env(&[("RADAR_MODE", "write")]), Some((Mode::Write, "RADAR_MODE".into())));
        assert_eq!(env(&[("RADAR_MODE", "read-only")]), Some((Mode::ReadOnly, "RADAR_MODE".into())));
        assert_eq!(env(&[("RADAR_MODE", "nonsense")]), None, "not a mode: the config's");
        assert_eq!(env(&[("RADAR_NO_WRITES", "")]), None, "empty: on");
        assert_eq!(env(&[("RADAR_NO_WRITES", "0")]), None, "0: on");
        assert_eq!(env(&[("RADAR_NO_WRITES", "1")]), Some((Mode::ReadOnly, "RADAR_NO_WRITES".into())));
        assert_eq!(env(&[("RADAR_NO_WRITES", "yes")]), Some((Mode::ReadOnly, "RADAR_NO_WRITES".into())), "any other value: off");
        assert_eq!(env(&[("RADAR_ACME_NO_WRITES", "1")]), Some((Mode::ReadOnly, "RADAR_ACME_NO_WRITES".into())));
        assert_eq!(env(&[("RADAR_ACME_NO_WRITES", "0")]), None);
        assert_eq!(
            env(&[("RADAR_ACME_NO_WRITES", "1"), ("RADAR_NO_WRITES", "1")]),
            Some((Mode::ReadOnly, "RADAR_NO_WRITES".into())),
            "the shared name is named"
        );
        assert_eq!(
            env(&[("RADAR_NO_WRITES", "1"), ("RADAR_MODE", "edit")]),
            Some((Mode::Edit, "RADAR_MODE".into())),
            "RADAR_MODE wins"
        );
        assert_eq!(env(&[("OTHER_NO_WRITES", "1")]), None, "only radar's");
    }

    #[test]
    fn the_mode_on_this_thread_refuses_before_sending_and_says_why() {
        assert_eq!(mode(), (Mode::Edit, None), "a fresh thread: edit");
        assert_eq!(not_sent(Change::Edit), None);
        assert_eq!(confirm_note(Change::Add), None);
        set_mode(Mode::ReadOnly, Some("RADAR_MODE".into()));
        assert_eq!(not_sent(Change::Add).as_deref(), Some("not sent: radar is read only"));
        assert_eq!(confirm_note(Change::Add).as_deref(), Some("radar is read only (set by RADAR_MODE): nothing will be sent"));
        assert_eq!(row_note(Change::Edit), Some("radar is read only"));
        set_mode(Mode::Write, None);
        assert_eq!(not_sent(Change::Add), None);
        assert_eq!(not_sent(Change::Edit).as_deref(), Some("not sent: radar is in write mode; this needs edit mode"));
        assert_eq!(
            confirm_note(Change::Edit).as_deref(),
            Some("radar is in write mode: this needs edit mode, nothing will be sent")
        );
        assert_eq!(row_note(Change::Edit), Some("needs edit mode"));
        assert_eq!(row_note(Change::Add), None);
    }

    #[test]
    fn the_words_around_a_write() {
        assert_eq!(ACME.asking("post a note on #12"), "asking Acme to post a note on #12…");
        // No theme on this thread: the symbols, not the pack's icons.
        assert_eq!(format!("{:?}", done("note posted on #12")), "Notify(✓ note posted on #12)");
        assert_eq!(ACME.did_not("post a note on #12", "busy"), "Acme did not post a note on #12: busy");
        assert_eq!(ACME.taken("merge !12"), "Acme accepted merge !12; checking…");
        assert_eq!(
            ACME.no_answer("merge !12"),
            "no answer from Acme about merge !12; the next sync will show it"
        );
        assert_eq!(
            format!("{:?}", failed(&ACME.did_not("post a note on #12", "busy"))),
            "Notice(Error: ✗ Acme did not post a note on #12: busy)"
        );
        assert_eq!(done_line("posted"), "✓ posted");
        assert_eq!(failed_line("refused"), "✗ refused");
        // With a pack in use the notice carries the icon, not a glyph in its
        // words (on a thread of its own: the pack in use is per thread).
        std::thread::spawn(|| {
            let mut host = crate::Host::new(vec![], 0).unwrap();
            host.load_config(Some("[theme]\nicons = \"unicode\"\n")).unwrap();
            let _ = host.build_theme(crate::design::theme::Tier::Mono);
            assert!(matches!(done("posted"), Effect::NotifyWith(Icon::RunPassed, ref w) if w == "posted"));
            assert!(matches!(failed("refused"), Effect::Notice(ref n) if n.icon == Some(Icon::Error) && n.text == "refused" && n.tone == crate::view::chrome::NoticeTone::Error));
        })
        .join()
        .unwrap();
        assert_eq!(kept("c"), " · c has your text");
    }

    fn rows(m: &crate::Menu) -> Vec<String> {
        m.items
            .iter()
            .map(|i| if i.note { format!("| {}", i.label) } else { format!("{} {} · {}", i.keys, i.label, i.hint) })
            .collect()
    }

    #[test]
    fn a_writes_question_shows_the_whole_text_and_offers_to_keep_writing() {
        let long = "x".repeat(80);
        let text = format!("First line\n\n  indented, kept as typed\n{long}");
        let m = crate::Menu::write_confirm("w", "post this on #12?", &text, "post it", "drop it");
        assert_eq!(m.title, "post this on #12?");
        assert_eq!(m.change, Some(Change::Add), "text adds");
        assert_eq!(
            rows(&m),
            [
                "| First line".to_string(),
                "| ".into(),
                "|   indented, kept as typed".into(),
                format!("| {}", "x".repeat(72)),
                format!("| {}", "x".repeat(8)),
                "y yes · yes, post it".into(),
                "e edit · keep writing".into(),
                "n no · no, drop it".into(),
            ]
        );
        assert_eq!(m.items.iter().filter(|i| !i.note).map(|i| i.value.as_str()).collect::<Vec<_>>(), ["yes", "edit", "no"]);
        // Wide characters are cut by the cells they take.
        assert_eq!(crate::plugin::note_lines(&"界".repeat(40), 72), ["界".repeat(36), "界".repeat(4)]);
    }

    #[test]
    fn a_plain_writes_question_and_a_note_before_its_choices() {
        let m = crate::Menu::write_plain("w", "approve #12?", vec!["as you".into()], "approve it", "leave it as it is")
            .noting(Some("radar is read only: nothing will be sent".into()));
        assert_eq!(m.change, Some(Change::Edit), "a plain write edits unless it says it adds");
        assert_eq!(crate::Menu::write_plain("w", "react?", vec![], "add it", "no").adds().change, Some(Change::Add));
        assert_eq!(
            rows(&m),
            [
                "| as you",
                "| radar is read only: nothing will be sent",
                "y yes · yes, approve it",
                "n no · no, leave it as it is",
            ]
        );
        let on = crate::Menu::write_plain("w", "approve #12?", vec![], "approve it", "no").noting(None);
        assert_eq!(on.items.len(), 2, "no note: none added");
    }
}
