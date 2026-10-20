//! A read-only, bounded view of original journal observations. The host owns
//! request IDs and I/O; this state never opens the journal or follows a link.

use serde::{Deserialize, Serialize};

use crate::journal::work_map::{EventCursor, EventEntry, EventFilter, EventPage, Snapshot};
use crate::journal::{Origin, Outcome, Reason, SessionOrigin, SubjectKey, SubjectRole, WriteMode};
use crate::ui::Scroll;
use crate::view::chrome::{Hint, HintKey};
use crate::view::{Chord, Ink, Key, Seg, Target, Text};

#[cfg(test)]
pub(crate) mod tests;

#[cfg(test)]
mod view_tests {
    use super::*;
    use crate::journal::work_map::ActionSource;
    use crate::journal::{
        ActionEvent, ActionId, EventId, Metadata, SessionId, StableCode, SubjectSnapshot,
    };
    use crate::view::Target;

    fn entry(sequence: i64, outcome: Outcome) -> EventEntry {
        let action = ActionId::parse("10000000-0000-4000-8000-000000000001").unwrap();
        let session = SessionId::parse("20000000-0000-4000-8000-000000000001").unwrap();
        let subject =
            SubjectKey::new("sample", "https://one.example.test", "card", "ABC-42").unwrap();
        EventEntry {
            source: ActionSource {
                action_id: action.clone(),
                session_id: session.clone(),
                origin: Origin::Host,
                kind: StableCode::new("card.refresh"),
                build_version: "0.1.0".into(),
                session_origin: SessionOrigin::Tui,
            },
            requested_sequence: 1,
            event: ActionEvent {
                event_version: 1,
                sequence,
                event_id: EventId::parse(&format!("40000000-0000-4000-8000-{sequence:012}"))
                    .unwrap(),
                action_id: action.clone(),
                session_id: session,
                parent_action_id: None,
                root_action_id: action.clone(),
                correlation_id: action,
                occurred_at_ms: 1_791_715_200_000,
                observed_at_ms: 1_791_715_200_000,
                recorded_at_ms: 1_791_715_200_000,
                observation_time_estimated: false,
                kind: StableCode::new("card.refresh"),
                origin: Origin::Host,
                outcome,
                metadata: Metadata::default(),
                subjects: vec![SubjectSnapshot::new(subject, SubjectRole::Primary)],
            },
        }
    }

    fn loaded() -> History {
        loaded_by(1)
    }

    /// An overlay opened by the host's request `id`, its first page read.
    fn loaded_by(id: u64) -> History {
        let (mut history, request) = History::new(EventFilter::default(), id);
        let snapshot: Snapshot = serde_json::from_value(serde_json::json!({
            "database_id": "30000000-0000-4000-8000-000000000001",
            "event_sequence": 9, "map_sequence": 0, "gap_sequence": 0,
        }))
        .unwrap();
        let page = EventPage {
            version: 1,
            snapshot,
            filter: request.filter.clone(),
            next: None,
            events: vec![
                entry(1, Outcome::Succeeded),
                entry(2, Outcome::Failed),
                entry(3, Outcome::Started),
            ],
            truncated: false,
            authority: "observations_only",
            task_instruction: "unspecified",
            execution_authorization: "unspecified",
            known_recording_gaps: false,
            completeness_not_guaranteed: true,
        };
        assert!(history.receive(request.id, Ok(page)));
        history
    }

    #[test]
    fn history_view_carries_rows_keys_and_state_with_targets() {
        let history = loaded();
        let view = history.view(80, 24);
        assert!(!view.rows.is_empty());
        for (i, row) in view.rows.iter().enumerate() {
            assert_eq!(row.target, history.row_target(i));
        }
        assert_eq!(view.selected, Some(0));
        assert!(!view.pending && view.error.is_none());
        assert!(!view.split && history.view(160, 40).split);
        assert_eq!(view.detail_keys_line, Some(3));
        let labels: Vec<_> = view.detail_keys.iter().map(|k| k.label.as_str()).collect();
        assert_eq!(labels, ["c", "i", "a"]);
        let footer: Vec<_> = view
            .footer
            .iter()
            .flat_map(|h| h.keys_of.iter().map(|k| k.target.as_str().to_owned()))
            .collect();
        assert!(footer.contains(&history.key_target("enter").0));
        assert!(footer.contains(&history.key_target("esc").0));
        // Loading and error states are data too.
        let (pending, _) = History::new(EventFilter::default(), 2);
        let view = pending.view(80, 24);
        assert!(view.pending && view.rows.is_empty() && view.selected.is_none());
        assert_eq!(view.boundary, "snapshot pending");
        assert!(view.state.starts_with("Loading"));
    }

    #[test]
    fn history_view_round_trips_through_json() {
        let view = loaded().view(160, 40);
        let json = serde_json::to_string(&view).unwrap();
        let back: HistoryView = serde_json::from_str(&json).unwrap();
        assert_eq!(back, view);
    }

    mod mnemonic_tests {
    use super::*;

    #[test]
    fn history_mnemonics_and_targeted_phrases_are_in_the_view() {
        let history = loaded();
        let controls = detail_controls(history.drawn);
        assert_eq!(controls.words(), "causal events · action events · all activity");
        for key in ["c", "i", "a"] {
            assert!(
                controls.0.iter().any(|seg| seg.target.as_ref() == Some(&history.key_target(key))),
                "{key}"
            );
        }
        let footer = history.footer();
        for hint in footer.iter().filter(|hint| crate::mnemonic::letter(&hint.keys).is_some()) {
            assert!(hint.mnemonic_at.is_some(), "{} {}", hint.keys, hint.label);
        }
        assert_eq!(hint_key(history.drawn, "↑").chord, Chord::new(Key::Up));
    }

    }

    #[test]
    fn history_targets_select_rows_and_press_shown_keys_only() {
        let mut history = loaded();
        let last = history.view(80, 24).rows.len() - 1;
        assert!(history.target(&history.row_target(last)).is_none());
        assert_eq!(history.view(80, 24).selected, Some(last));
        assert!(history.target(&history.row_target(last + 1)).is_none());
        assert_eq!(history.selected, last);
        let enter = history.target(&history.key_target("enter")).unwrap();
        assert_eq!(enter.key, Key::Enter);
        assert_eq!(
            history.target(&history.key_target("c")).unwrap().key,
            Key::Char('c')
        );
        // `tab` is not shown while the events have the keys.
        assert!(history.target(&history.key_target("tab")).is_none());
        assert!(history.target(&Target::new("chrome:menu:0")).is_none());
        // Malformed ones name nothing.
        for bad in ["chrome:history:row:0", "chrome:history:x.0:row:0", "chrome:history:1.0:pane:0"] {
            assert!(history.target(&Target::new(bad)).is_none(), "{bad}");
        }
        history.key(enter, 3);
        assert!(history.target(&history.key_target("tab")).is_some());
    }

    #[test]
    fn history_targets_drawn_for_an_earlier_page_or_overlay_do_nothing() {
        let mut history = loaded();
        let row = history.row_target(2);
        let key = history.key_target("enter");
        // A refresh: the page those targets were drawn for is gone.
        let refresh = history
            .key(Chord::char('r'), 2)
            .expect("a refresh reads again");
        assert!(history.target(&row).is_none() && history.target(&key).is_none());
        let reloaded = loaded().page.unwrap();
        assert!(history.receive(refresh.id, Ok(reloaded)));
        assert!(history.target(&row).is_none(), "the same rows, a new drawing");
        assert!(history.target(&key).is_none());
        assert_eq!((history.selected, history.focus), (0, Focus::Events));
        // The drawing shown now takes its own.
        assert!(history.target(&history.row_target(2)).is_none());
        assert_eq!(history.selected, 2);
        // An overlay closed and opened again (a later request) is another
        // drawing, even on its first page: the old overlay's targets miss.
        let mut reopened = loaded_by(3);
        assert!(reopened.target(&row).is_none() && reopened.target(&history.row_target(1)).is_none());
        assert!(reopened.target(&history.key_target("enter")).is_none());
        assert_eq!(reopened.selected, 0);
    }
}

pub const RECORDS_PER_PAGE: usize = 50;


/// Errors intentionally carry no raw database path, SQL, or error payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum ReadError {
    Disabled,
    Unavailable,
}

/// Which panel of the history overlay has the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "snake_case")]
pub enum HistoryFocus {
    Events,
    Detail,
}

/// The history overlay for one frame, as view data: what it says and what
/// can be pointed at, each by an opaque `chrome:history:` [`Target`] that
/// [`History::target`] resolves, and only while the page it was drawn for
/// is the one shown (a frontend echoes it back, never builds one). Bounded: one
/// page of at most [`RECORDS_PER_PAGE`] rows, a fixed set of detail fields,
/// each text already passed through the overlay's allowlist. Nothing here
/// names a terminal library; the renderer's overlay draws from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct HistoryView {
    pub version: u32,
    /// The room it was made for, in cells.
    pub width: u16,
    pub height: u16,
    /// `History · read only · UTC`.
    pub title: String,
    /// `snapshot through #12`, `snapshot pending`, `snapshot not loaded`.
    pub boundary: String,
    /// The scope in words (`exact subject …; since …`).
    pub scope: String,
    /// The notes under the scope: coverage, and known recording gaps.
    pub notes: Vec<Text>,
    pub focus: HistoryFocus,
    /// The keys help shows in place of the panels.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub help: bool,
    /// Wide enough to show the events and the detail side by side; else
    /// only the focused panel shows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub split: bool,
    /// `events · page 1 · oldest-first · more pages`.
    pub events_title: String,
    /// The event table's column headings, in order.
    pub columns: Vec<String>,
    pub rows: Vec<HistoryRow>,
    /// The selected row (an index into `rows`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<usize>,
    /// How far the event list is scrolled: the first row shown. A frontend
    /// still keeps the selected row in view.
    #[serde(default)]
    pub list_scroll: u16,
    /// A read is in flight.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pending: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ReadError>,
    /// What the overlay says about its state: loading, empty, or the error.
    pub state: String,
    pub detail_title: String,
    /// The selected event's detail, or the state and scope when none is.
    pub detail: Vec<Text>,
    /// The line of `detail` that names the scope keys, and those keys.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail_keys_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail_keys: Vec<HintKey>,
    /// How far the detail is scrolled, in lines.
    #[serde(default)]
    pub detail_scroll: u16,
    pub help_title: String,
    pub help_lines: Vec<Text>,
    #[serde(default)]
    pub help_scroll: u16,
    /// The keys along the bottom, each key with its target.
    pub footer: Vec<Hint>,
}

/// One recorded event in the list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
pub struct HistoryRow {
    /// `2026-10-12 07:01:02`, UTC.
    pub observed: String,
    pub action: String,
    pub subject: String,
    pub outcome: String,
    pub outcome_ink: Ink,
    pub target: Target,
}

/// The targets the history overlay names. Each carries the [`Drawn`] it was
/// made for, so a target sent back after the page under it changed (another
/// page loaded, a refresh, the overlay closed and opened again) does nothing
/// instead of acting on whatever sits at the same place now. A renderer
/// only echoes them back; their shape is the overlay's own.
pub mod targets {
    use crate::view::Target;

    pub const PREFIX: &str = "chrome:history:";

    /// Which drawing of the overlay a target belongs to: the overlay (the
    /// host's request id that opened it, never reused) and its page
    /// generation, bumped each time a page loads or the scope restarts.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Drawn {
        pub(super) open: u64,
        pub(super) page: u64,
    }

    /// What a target points at, once its drawing is known.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Points<'a> {
        Row(usize),
        Key(&'a str),
    }

    /// Row `index` of the event list, as drawn in `drawn`.
    pub fn row(drawn: Drawn, index: usize) -> Target {
        Target(format!("{PREFIX}{}.{}:row:{index}", drawn.open, drawn.page))
    }
    /// The key labelled `label` (`j`, `enter`, `PgDn`, `?`), as drawn in `drawn`.
    pub fn key(drawn: Drawn, label: &str) -> Target {
        Target(format!("{PREFIX}{}.{}:key:{label}", drawn.open, drawn.page))
    }

    /// The drawing a target was made for and what it points at; `None` for
    /// anything that is not one of the overlay's targets.
    pub(super) fn parse(target: &Target) -> Option<(Drawn, Points<'_>)> {
        let rest = target.as_str().strip_prefix(PREFIX)?;
        let (drawn, what) = rest.split_once(':')?;
        let (open, page) = drawn.split_once('.')?;
        let drawn = Drawn {
            open: open.parse().ok()?,
            page: page.parse().ok()?,
        };
        let points = if let Some(index) = what.strip_prefix("row:") {
            Points::Row(index.parse().ok()?)
        } else {
            Points::Key(what.strip_prefix("key:")?)
        };
        Some((drawn, points))
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: u64,
    pub filter: EventFilter,
    pub cursor: Option<EventCursor>,
    /// Retained even for the first page, whose cursor is None. A previous-page
    /// read must not silently advance the snapshot under newly recorded events.
    pub snapshot: Option<Snapshot>,
}

#[derive(Debug)]
struct Pending {
    request: Request,
    index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Events,
    Detail,
}

/// Only the visible page is retained. Back navigation stores opaque cursors,
/// not prior event payloads; all pages retain the reader's original snapshot.
#[derive(Debug)]
pub struct History {
    filter: EventFilter,
    recent: bool,
    now_ms: i64,
    page: Option<EventPage>,
    cursors: Vec<Option<EventCursor>>,
    index: usize,
    pending: Option<Pending>,
    error: Option<ReadError>,
    selected: usize,
    focus: Focus,
    help: bool,
    list_scroll: Scroll,
    detail_scroll: Scroll,
    help_scroll: Scroll,
    /// The drawing the targets handed out now belong to.
    drawn: targets::Drawn,
}

impl History {
    /// `filter` is captured by the opener. It is never recomputed from a live
    /// selection, a work-map binding, a label, or an inferred task grouping.
    pub fn new(filter: EventFilter, request_id: u64) -> (Self, Request) {
        let mut history = Self {
            filter,
            recent: false,
            now_ms: 0,
            page: None,
            cursors: vec![None],
            index: 0,
            pending: None,
            error: None,
            selected: 0,
            focus: Focus::Events,
            help: false,
            list_scroll: Scroll::new(),
            detail_scroll: Scroll::new(),
            help_scroll: Scroll::new(),
            drawn: targets::Drawn {
                open: request_id,
                page: 0,
            },
        };
        let request = history.request(request_id, None, 0);
        (history, request)
    }

    /// Relative scope is explicit state: only refreshing moves its boundary.
    pub fn recent(now_ms: i64, request_id: u64) -> (Self, Request) {
        let (mut history, request) = Self::new(recent_filter(now_ms), request_id);
        history.recent = true;
        history.now_ms = now_ms;
        (history, request)
    }

    /// The host supplies its clock; reducers never read the system clock.
    pub fn set_now(&mut self, now_ms: i64) {
        self.now_ms = now_ms;
    }

    fn request(&mut self, id: u64, cursor: Option<EventCursor>, index: usize) -> Request {
        let request = Request {
            id,
            filter: self.filter.clone(),
            cursor,
            snapshot: self.page.as_ref().map(|page| page.snapshot.clone()),
        };
        self.pending = Some(Pending {
            request: request.clone(),
            index,
        });
        self.error = None;
        request
    }

    fn restart(&mut self, filter: EventFilter, id: u64) -> Request {
        self.filter = filter;
        self.recent = false;
        self.page = None;
        self.cursors = vec![None];
        self.index = 0;
        self.select(0);
        self.list_scroll.top();
        self.detail_scroll.top();
        self.redrawn();
        self.request(id, None, 0)
    }

    /// What the targets pointed at is gone (a page loaded, the scope
    /// restarted): the targets handed out before point at nothing now.
    fn redrawn(&mut self) {
        self.drawn.page = self.drawn.page.wrapping_add(1);
    }

    /// Row `index`'s target in the drawing shown now.
    pub fn row_target(&self, index: usize) -> Target {
        targets::row(self.drawn, index)
    }

    /// The target of the key labelled `label` in the drawing shown now.
    pub fn key_target(&self, label: &str) -> Target {
        targets::key(self.drawn, label)
    }

    fn restart_recent(&mut self, id: u64) -> Request {
        let request = self.restart(recent_filter(self.now_ms), id);
        self.recent = true;
        request
    }

    /// Accept only the live request. IDs are supplied by the host's monotonic
    /// counter, so an answer from an older, closed overlay cannot match.
    pub fn receive(&mut self, id: u64, result: Result<EventPage, ReadError>) -> bool {
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.request.id != id)
        {
            return false;
        }
        let pending = self.pending.take().expect("matching request");
        match result {
            Ok(page)
                if page.filter == self.filter
                    && page.events.len() <= RECORDS_PER_PAGE
                    && pending
                        .request
                        .snapshot
                        .as_ref()
                        .is_none_or(|snapshot| *snapshot == page.snapshot) =>
            {
                self.cursors.truncate(pending.index + 1);
                if self.cursors.len() == pending.index {
                    self.cursors.push(pending.request.cursor);
                }
                self.index = pending.index;
                self.page = Some(page);
                self.error = None;
                self.select(0);
                self.list_scroll.top();
                self.detail_scroll.top();
                self.redrawn();
            }
            Ok(_) => self.error = Some(ReadError::Unavailable),
            Err(error) => self.error = Some(error),
        }
        true
    }

    pub fn key(&mut self, key: Chord, next_id: u64) -> Option<Request> {
        // The host handles Escape and Ctrl-C. Other host/plugin controls never
        // fall through this overlay, including x and ?.
        if key.ctrl || key.alt {
            return None;
        }
        if matches!(key.key, Key::Char('x' | '?')) {
            self.help = !self.help;
            self.help_scroll.top();
            return None;
        }
        if self.help {
            self.help_scroll.key(&key);
            return None;
        }
        match key.key {
            Key::Tab | Key::BackTab => {
                self.focus = if self.focus == Focus::Events {
                    Focus::Detail
                } else {
                    Focus::Events
                };
            }
            Key::Char('h') | Key::Left => self.focus = Focus::Events,
            Key::Char('l') | Key::Right | Key::Enter => self.focus = Focus::Detail,
            Key::PageUp | Key::PageDown => {
                self.detail_scroll.key(&key);
            }
            Key::Char('a') if self.filter != EventFilter::default() => {
                return Some(self.restart(EventFilter::default(), next_id));
            }
            Key::Char('t') if !self.recent => {
                return Some(self.restart_recent(next_id));
            }
            Key::Char('c') => {
                if let Some(entry) = self.selected_entry() {
                    let filter = EventFilter {
                        root_action_id: Some(entry.event.root_action_id.clone()),
                        ..EventFilter::default()
                    };
                    if self.filter != filter {
                        return Some(self.restart(filter, next_id));
                    }
                }
            }
            Key::Char('i') => {
                if let Some(entry) = self.selected_entry() {
                    let filter = EventFilter {
                        action_id: Some(entry.event.action_id.clone()),
                        ..EventFilter::default()
                    };
                    if self.filter != filter {
                        return Some(self.restart(filter, next_id));
                    }
                }
            }
            Key::Char('r') if self.pending.is_none() => {
                return Some(if self.recent {
                    self.restart_recent(next_id)
                } else {
                    self.restart(self.filter.clone(), next_id)
                });
            }
            Key::Char('n') if self.pending.is_none() => {
                if let Some(cursor) = self.page.as_ref().and_then(|page| page.next.clone()) {
                    return Some(self.request(next_id, Some(cursor), self.index + 1));
                }
            }
            Key::Char('p') if self.pending.is_none() && self.index > 0 => {
                return Some(self.request(
                    next_id,
                    self.cursors[self.index - 1].clone(),
                    self.index - 1,
                ));
            }
            _ if self.focus == Focus::Detail => {
                self.detail_scroll.key(&key);
            }
            Key::Char('j') | Key::Down => self.select(self.selected.saturating_add(1)),
            Key::Char('k') | Key::Up => self.select(self.selected.saturating_sub(1)),
            Key::Home | Key::Char('g') => self.select(0),
            Key::End | Key::Char('G') => self.select(self.len().saturating_sub(1)),
            _ => {}
        }
        None
    }

    /// Escape first leaves this view's local help, then closes the overlay.
    pub fn dismiss_help(&mut self) -> bool {
        let was_open = self.help;
        self.help = false;
        was_open
    }

    fn len(&self) -> usize {
        self.page.as_ref().map_or(0, |page| page.events.len())
    }

    fn select(&mut self, index: usize) {
        let index = index.min(self.len().saturating_sub(1));
        if self.selected != index {
            self.selected = index;
            self.detail_scroll.top();
        }
    }

    fn selected_entry(&self) -> Option<&EventEntry> {
        self.page.as_ref()?.events.get(self.selected)
    }

    fn scope(&self) -> String {
        let filter = &self.filter;
        if self.recent {
            return format!(
                "recent activity · last 24 hours since {}",
                timestamp(filter.since_ms.unwrap_or(0))
            );
        }
        let mut parts = Vec::new();
        if let Some(subject) = &filter.subject {
            parts.push(format!("exact subject {}", subject_text(subject)));
        }
        if let Some(id) = &filter.root_action_id {
            parts.push(format!("causal root {}", id.as_str()));
        }
        if let Some(id) = &filter.action_id {
            parts.push(format!("action {}", id.as_str()));
        }
        if let Some(id) = &filter.session_id {
            parts.push(format!("session {}", id.as_str()));
        }
        if let Some(kind) = &filter.kind {
            parts.push(format!("kind {}", safe_text(kind.as_str())));
        }
        if let Some(outcome) = filter.outcome {
            parts.push(format!("outcome {}", outcome.as_str()));
        }
        if let Some(since) = filter.since_ms {
            parts.push(format!("since {}", timestamp(since)));
        }
        if let Some(until) = filter.until_ms {
            parts.push(format!("until {}", timestamp(until)));
        }
        if parts.is_empty() {
            "all recorded events".into()
        } else {
            parts.join("; ")
        }
    }

    fn state_text(&self) -> &'static str {
        match self.error {
            Some(ReadError::Disabled) => {
                "History recording is disabled or unavailable for this session. Close this view to continue."
            }
            Some(ReadError::Unavailable) => {
                "History could not be read. Press r to retry, or esc to close."
            }
            None if self.pending.is_some() => "Loading recorded observations…",
            None if self.recent => {
                "No recorded events in this 24-hour window. Coverage may be incomplete. Press a for all activity or r to refresh."
            }
            None => {
                "No recorded events in this scope. Coverage may be incomplete. Press a for all events or r to refresh."
            }
        }
    }

    /// The overlay as view data at `width` by `height` cells. Pure: it reads
    /// the state and changes nothing.
    pub fn view(&self, width: u16, height: u16) -> HistoryView {
        let boundary = self.page.as_ref().map_or_else(
            || {
                if self.pending.is_some() {
                    "snapshot pending"
                } else {
                    "snapshot not loaded"
                }
                .into()
            },
            |page| format!("snapshot through #{}", page.snapshot.event_sequence()),
        );
        let mut notes = vec![Text::of(
            "Recorded observations; coverage may be incomplete. Not task intent or permission.",
            Ink::Muted,
        )];
        if self
            .page
            .as_ref()
            .is_some_and(|page| page.known_recording_gaps)
        {
            notes.push(Text::from(vec![
                Seg::new(
                    "Known recording gaps affect this scope; missing actions cannot be identified.",
                    Ink::Warn,
                )
                .strong(),
            ]));
        }
        let selected = self.selected_entry().map(|_| self.selected);
        HistoryView {
            version: crate::view::VERSION,
            width,
            height,
            title: "History · read only · UTC".into(),
            boundary,
            scope: self.scope(),
            notes,
            focus: match self.focus {
                Focus::Events => HistoryFocus::Events,
                Focus::Detail => HistoryFocus::Detail,
            },
            help: self.help,
            split: width >= 120,
            events_title: self.events_title(),
            columns: COLUMNS.iter().map(|c| (*c).into()).collect(),
            rows: self.rows_view(),
            selected,
            list_scroll: self.list_scroll.at(),
            pending: self.pending.is_some(),
            error: self.error,
            state: self.state_text().into(),
            detail_title: "event detail".into(),
            detail: self.detail_text(),
            detail_keys_line: selected.map(|_| 3),
            detail_keys: if selected.is_some() {
                ["c", "i", "a"].iter().map(|k| hint_key(self.drawn, k)).collect()
            } else {
                vec![]
            },
            detail_scroll: self.detail_scroll.at(),
            help_title: "history keys".into(),
            help_lines: self.help_text(),
            help_scroll: self.help_scroll.at(),
            footer: self.footer(),
        }
    }

    /// What a pointer at `target` does: a row selects its event (and
    /// focuses the list); a key comes back to be pressed through
    /// [`History::key`]. A target the overlay does not show now, or one
    /// made for an earlier drawing (another page, before a refresh, an
    /// overlay since closed), does nothing.
    pub fn target(&mut self, target: &Target) -> Option<Chord> {
        let (drawn, points) = targets::parse(target)?;
        if drawn != self.drawn {
            return None;
        }
        let label = match points {
            targets::Points::Row(index) => {
                if self.help || index >= self.len() {
                    return None;
                }
                self.focus = Focus::Events;
                self.select(index);
                return None;
            }
            targets::Points::Key(label) => label,
        };
        let drawn = self.drawn;
        let shown = self
            .footer()
            .into_iter()
            .flat_map(|hint| hint.keys_of)
            .chain(
                self.selected_entry()
                    .filter(|_| !self.help)
                    .into_iter()
                    .flat_map(|_| ["c", "i", "a"].map(|k| hint_key(drawn, k))),
            )
            .find(|key| key.label == label)?;
        Some(shown.chord)
    }

    fn events_title(&self) -> String {
        let mut title = format!("events · page {} · oldest-first", self.index + 1);
        if let Some(page) = &self.page {
            title.push_str(if page.next.is_some() {
                " · more pages"
            } else {
                " · last page"
            });
        }
        if self.pending.is_some() && self.page.is_some() {
            title.push_str(" · loading");
        }
        title
    }

    fn rows_view(&self) -> Vec<HistoryRow> {
        let Some(page) = &self.page else {
            return vec![];
        };
        page.events
            .iter()
            .take(RECORDS_PER_PAGE)
            .enumerate()
            .map(|(index, entry)| {
                let event = &entry.event;
                let subject = event
                    .subjects
                    .iter()
                    .find(|s| s.role == SubjectRole::Primary)
                    .or_else(|| event.subjects.first());
                let subject = subject.map_or_else(
                    || "no subject recorded".into(),
                    |s| {
                        format!(
                            "{}:{} {}",
                            safe_text(&s.key.provider),
                            safe_text(&s.key.kind),
                            safe_text(&s.key.id)
                        )
                    },
                );
                HistoryRow {
                    observed: crate::time::to_rfc3339(event.observed_at_ms.div_euclid(1000))
                        .replace('T', " ")
                        .trim_end_matches('Z')
                        .to_owned(),
                    action: safe_text(event.kind.as_str()),
                    subject,
                    outcome: event.outcome.as_str().into(),
                    outcome_ink: outcome_ink(event.outcome),
                    target: targets::row(self.drawn, index),
                }
            })
            .collect()
    }

    fn footer(&self) -> Vec<Hint> {
        let groups: Vec<(&str, &str)> = if self.help {
            vec![
                ("↑↓", "scroll"),
                ("PgUp/PgDn", "page"),
                ("x", "exit keys"),
                ("esc", "back"),
            ]
        } else {
            vec![
                (
                    "↑↓",
                    if self.focus == Focus::Detail {
                        "scroll"
                    } else {
                        "move"
                    },
                ),
                if self.focus == Focus::Detail {
                    ("tab", "events")
                } else {
                    ("enter", "details")
                },
                ("n", "next page"),
                ("p", "previous page"),
                ("r", "refresh"),
                ("x", "extra keys"),
                ("esc", "close"),
            ]
        };
        groups
            .into_iter()
            .map(|(keys, label)| Hint {
                mnemonic_at: crate::mnemonic::at(keys, label),
                mnemonic_shift: crate::mnemonic::at(keys, label).is_some() && crate::mnemonic::shifted(keys),
                keys: keys.into(),
                label: label.into(),
                icon: None,
                keys_of: crate::host::hint_labels(keys).into_iter().map(|k| hint_key(self.drawn, k)).collect(),
            })
            .collect()
    }

    fn detail_text(&self) -> Vec<Text> {
        let Some(entry) = self.selected_entry() else {
            return vec![
                Text::plain(self.state_text()),
                Text::new(),
                Text::plain(format!("Scope: {}", self.scope())),
            ];
        };
        let event = &entry.event;
        let metadata = &event.metadata;
        let primary = event
            .subjects
            .iter()
            .find(|subject| subject.role == SubjectRole::Primary)
            .or_else(|| event.subjects.first());
        let brief = primary.map_or_else(
            || "No subject recorded".into(),
            |subject| {
                format!(
                    "Subject: {}:{} {}",
                    safe_text(&subject.key.provider),
                    safe_text(&subject.key.kind),
                    safe_text(&subject.key.id),
                )
            },
        );
        let mut lines = vec![
            Text::from(vec![
                Seg::new(safe_text(event.kind.as_str()), Ink::Emphasis).strong(),
            ]),
            Text::of(event.outcome.as_str(), outcome_ink(event.outcome)),
            Text::plain(crate::ui::fit(&brief, 48).trim_end().to_owned()),
            detail_controls(self.drawn),
            Text::new(),
        ];
        let mut field = |label: &str, value: String| lines.push(kv(label, value));
        field("Observed UTC", timestamp(event.observed_at_ms));
        field("Recorded UTC", timestamp(event.recorded_at_ms));
        if event.observation_time_estimated {
            field(
                "Time quality",
                "observation time estimated from old writer time".into(),
            );
        }
        field(
            "Reason",
            metadata.reason.map_or("not recorded", reason_text).into(),
        );
        field(
            "Mode",
            metadata.mode.map_or("not recorded", mode_text).into(),
        );
        if let Some(count) = metadata.count {
            field("Count", count.to_string());
        }
        if let Some(duration) = metadata.duration_ms {
            field("Duration", format!("{duration} ms"));
        }
        if let Some(command) = &metadata.command {
            field("Command", safe_text(command.as_str()));
        }
        if let Some(url) = &metadata.url {
            field("Safe URL", safe_text(url.as_str()));
        }
        if metadata.redacted {
            field("Redacted", "unsafe details omitted".into());
        }
        if metadata.truncated {
            field("Truncated", "some details omitted".into());
        }
        if metadata.omitted_subjects > 0 {
            field("Omitted", format!("{} subjects", metadata.omitted_subjects));
        }
        lines.push(Text::new());
        lines.push(heading("recorded subjects"));
        if event.subjects.is_empty() {
            lines.push(Text::plain("No subject identity was recorded."));
        }
        for subject in &event.subjects {
            lines.push(Text::plain(format!(
                "{}: {}",
                role_text(subject.role),
                subject_text(&subject.key)
            )));
            if let Some(url) = &subject.url {
                lines.push(Text::of(safe_text(url.as_str()), Ink::Secondary));
            }
        }
        lines.push(Text::new());
        lines.push(heading("recording source and causal IDs"));
        let mut field = |label: &str, value: String| lines.push(kv(label, value));
        field("Source", origin_text(&entry.source.origin));
        field(
            "Session type",
            match entry.source.session_origin {
                SessionOrigin::Tui => "TUI",
                SessionOrigin::Cli => "CLI",
                SessionOrigin::Unknown => "unknown",
            }
            .into(),
        );
        field("Build", safe_text(&entry.source.build_version));
        field("Action ID", event.action_id.as_str().into());
        field(
            "Parent ID",
            event
                .parent_action_id
                .as_ref()
                .map_or("none (root action)", |id| id.as_str())
                .into(),
        );
        field("Root ID", event.root_action_id.as_str().into());
        field("Correlation", event.correlation_id.as_str().into());
        field("Session ID", event.session_id.as_str().into());
        field("Event ID", event.event_id.as_str().into());
        field("Sequence", event.sequence.to_string());
        field(
            "Requested at",
            format!("sequence {}", entry.requested_sequence),
        );
        lines.push(Text::new());
        lines.push(Text::plain("Parent and root IDs are explicit causal links. Shared subjects do not imply a shared task."));
        lines.push(Text::new());
        lines.push(Text::plain(format!("Scope: {}", self.scope())));
        if let Some(page) = &self.page {
            lines.push(Text::plain(format!(
                "Snapshot through event sequence {}. Refresh starts a new snapshot.",
                page.snapshot.event_sequence()
            )));
        }
        lines
    }

    fn help_text(&self) -> Vec<Text> {
        let mut lines = vec![heading("browse recorded observations")];
        for (key, what) in [
            (
                "↑↓",
                "move in the event list; scroll the focused detail",
            ),
            ("Home/End", "first/last event, or top/bottom of the detail"),
            ("tab", "switch between the event list and detail"),
            ("←→", "focus the event list or detail"),
            ("enter", "show the selected event's detail"),
            ("PgUp/PgDn", "scroll the event detail from either pane"),
            ("n", "next page in this fixed snapshot"),
            ("p", "previous page in this fixed snapshot"),
            ("r", "refresh this scope from its first page"),
            ("a", "show all recorded events"),
            ("t", "show recent activity from the last 24 hours"),
            (
                "c",
                "show events with the selected event's exact causal root",
            ),
            ("i", "show events for the selected exact action ID"),
            ("x", "extra history keys"),
            ("?", "show or hide these history keys"),
            ("esc", "back from keys; then close history"),
        ] {
            lines.push(crate::mnemonic::text(key, what, "text.primary"));
        }
        lines.push(Text::new());
        lines.push(Text::plain("Pages contain at most 50 original observations, ordered by recording sequence (oldest first). Times are UTC; clocks can differ from sequence order."));
        lines.push(Text::new());
        lines.push(Text::plain("Coverage may be incomplete. This view does not establish task intent, approval, or execution permission. A shared subject or work-map label never establishes a causal link."));
        lines.push(Text::new());
        lines.push(Text::plain(format!("Scope: {}", self.scope())));
        lines
    }
}

const COLUMNS: [&str; 4] = ["observed UTC", "action", "subject", "outcome"];

/// A key the overlay names, by its label (`j`, `enter`, `PgDn`, `?`), with
/// its target in drawing `drawn`.
fn hint_key(drawn: targets::Drawn, label: &str) -> HintKey {
    let key = match label {
        "tab" => Key::Tab,
        "enter" => Key::Enter,
        "esc" => Key::Esc,
        "PgUp" => Key::PageUp,
        "PgDn" => Key::PageDown,
        "↑" => Key::Up,
        "↓" => Key::Down,
        "←" => Key::Left,
        "→" => Key::Right,
        _ => Key::Char(label.chars().next().expect("nonempty key")),
    };
    HintKey {
        label: label.into(),
        chord: Chord::new(key),
        action: None,
        target: targets::key(drawn, label),
    }
}

/// `Label        value`: the label muted in a 13-cell column.
fn kv(label: &str, value: String) -> Text {
    Text::from(vec![
        Seg::new(crate::ui::pad(label, 13), Ink::Muted),
        Seg::new(value, Ink::Plain),
    ])
}

/// A section heading in muted capitals.
fn heading(text: &str) -> Text {
    Text::from(vec![Seg::new(text.to_uppercase(), Ink::Muted).strong()])
}

/// Each phrase keeps its generation-scoped target on every styled segment.
fn detail_controls(drawn: targets::Drawn) -> Text {
    let mut text = Text::new();
    for (index, (key, words)) in [("c", "causal events"), ("i", "action events"), ("a", "all activity")].iter().enumerate() {
        if index > 0 { text.add(Seg::new(" · ", Ink::AccentSecondary)); }
        let mut phrase = crate::mnemonic::text(key, words, "accent.secondary");
        for seg in &mut phrase.0 { seg.target = Some(targets::key(drawn, key)); }
        text.extend(phrase);
    }
    text
}

fn recent_filter(now_ms: i64) -> EventFilter {
    EventFilter {
        since_ms: Some(now_ms.saturating_sub(86_400_000).max(0)),
        ..EventFilter::default()
    }
}

fn timestamp(ms: i64) -> String {
    let seconds = crate::time::to_rfc3339(ms.div_euclid(1000));
    format!(
        "{}.{:03}Z",
        seconds.trim_end_matches('Z'),
        ms.rem_euclid(1000)
    )
}

fn safe_text(value: &str) -> String {
    // Storage already validates these allowlisted fields. Fail closed for a
    // malformed completion or a secret registered since an older event.
    if value.len() > 8192
        || value.chars().any(char::is_control)
        || crate::log::contains_secret(value)
    {
        "[withheld]".into()
    } else {
        value.into()
    }
}

fn subject_text(subject: &SubjectKey) -> String {
    format!(
        "{}:{} {} @ {}",
        safe_text(&subject.provider),
        safe_text(&subject.kind),
        safe_text(&subject.id),
        safe_text(&subject.instance)
    )
}

fn origin_text(origin: &Origin) -> String {
    match origin {
        Origin::Host => "host".into(),
        Origin::Plugin(id) => format!("part {}", safe_text(id)),
    }
}

fn outcome_ink(outcome: Outcome) -> Ink {
    match outcome {
        Outcome::Succeeded => Ink::Good,
        Outcome::Failed => Ink::Bad,
        Outcome::Partial | Outcome::Unknown | Outcome::Blocked => Ink::Warn,
        Outcome::Cancelled => Ink::Muted,
        Outcome::Requested | Outcome::Confirmed | Outcome::Started => Ink::Info,
    }
}

fn mode_text(mode: WriteMode) -> &'static str {
    match mode {
        WriteMode::ReadOnly => "read only",
        WriteMode::Write => "write",
        WriteMode::Edit => "edit",
    }
}

fn reason_text(reason: Reason) -> &'static str {
    match reason {
        Reason::Interrupted => "interrupted",
        Reason::NoTerminalObservation => "no terminal observation",
        Reason::Unsupported => "unsupported",
        Reason::Unavailable => "unavailable",
        Reason::PermissionDenied => "permission denied",
        Reason::InvalidInput => "invalid input",
        Reason::Transport => "transport",
        Reason::Storage => "storage",
        Reason::TaskCompleted => "task completed (outcome not inferred)",
        Reason::Unspecified => "unspecified",
    }
}

fn role_text(role: SubjectRole) -> &'static str {
    match role {
        SubjectRole::Primary => "primary",
        SubjectRole::Source => "source",
        SubjectRole::Destination => "destination",
        SubjectRole::Context => "context",
        SubjectRole::Result => "result",
    }
}
