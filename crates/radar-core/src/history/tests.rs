use super::*;
use crate::design::theme::Tier;
use crate::journal::work_map::ActionSource;
use crate::journal::{
    ActionEvent, ActionId, EventId, Metadata, SafeUrl, SessionId, StableCode, SubjectSnapshot,
};
use serde_json::json;

const AT: i64 = 1_791_715_200_000;

fn action(n: u64) -> ActionId {
    ActionId::parse(&format!("10000000-0000-4000-8000-{n:012}")).unwrap()
}

fn subject() -> SubjectKey {
    SubjectKey::new("sample", "https://one.example.test", "card", "ABC-42").unwrap()
}

fn filter() -> EventFilter {
    EventFilter {
        subject: Some(subject()),
        ..EventFilter::default()
    }
}

fn snapshot(sequence: i64) -> Snapshot {
    serde_json::from_value(json!({
        "database_id": "30000000-0000-4000-8000-000000000001",
        "event_sequence": sequence, "map_sequence": 0, "gap_sequence": 0,
    }))
    .unwrap()
}

fn cursor(filter: &EventFilter, after: i64, snapshot: &Snapshot) -> EventCursor {
    serde_json::from_value(
        json!({ "snapshot": snapshot, "filter": filter, "after_sequence": after }),
    )
    .unwrap()
}

pub(crate) fn entry(sequence: i64, outcome: Outcome) -> EventEntry {
    let action_id = action(if sequence < 4 { 1 } else { 2 });
    let session = SessionId::parse("20000000-0000-4000-8000-000000000001").unwrap();
    EventEntry {
        source: ActionSource {
            action_id: action_id.clone(),
            session_id: session.clone(),
            origin: Origin::Plugin("sample".into()),
            kind: StableCode::new("card.refresh"),
            build_version: "0.1.0".into(),
            session_origin: SessionOrigin::Tui,
        },
        requested_sequence: if sequence < 4 { 1 } else { 4 },
        event: ActionEvent {
            event_version: 1,
            sequence,
            event_id: EventId::parse(&format!("40000000-0000-4000-8000-{sequence:012}")).unwrap(),
            action_id,
            session_id: session,
            parent_action_id: (sequence >= 4).then(|| action(1)),
            root_action_id: action(1),
            correlation_id: action(1),
            occurred_at_ms: AT + sequence * 1_000,
            observed_at_ms: AT + sequence * 1_000,
            recorded_at_ms: AT + sequence * 1_000 + 12,
            observation_time_estimated: false,
            kind: StableCode::new(if sequence < 4 {
                "card.refresh"
            } else {
                "card.details"
            }),
            origin: Origin::Plugin("sample".into()),
            outcome,
            metadata: Metadata {
                mode: Some(WriteMode::ReadOnly),
                ..Metadata::default()
            },
            subjects: vec![SubjectSnapshot::new(subject(), SubjectRole::Primary)],
        },
    }
}

pub(crate) fn page(filter: EventFilter, events: Vec<EventEntry>, more: bool) -> EventPage {
    let snapshot = snapshot(100);
    EventPage {
        version: 1,
        snapshot: snapshot.clone(),
        filter: filter.clone(),
        next: more.then(|| cursor(&filter, events.last().unwrap().event.sequence, &snapshot)),
        events,
        truncated: more,
        authority: "observations_only",
        task_instruction: "unspecified",
        execution_authorization: "unspecified",
        known_recording_gaps: false,
        completeness_not_guaranteed: true,
    }
}

fn sample() -> History {
    let (mut history, request) = History::new(filter(), 1);
    let outcomes = [
        Outcome::Requested,
        Outcome::Started,
        Outcome::Succeeded,
        Outcome::Requested,
        Outcome::Started,
        Outcome::Partial,
        Outcome::Unknown,
        Outcome::Failed,
        Outcome::Blocked,
        Outcome::Cancelled,
    ];
    let mut events: Vec<_> = outcomes
        .into_iter()
        .enumerate()
        .map(|(n, outcome)| entry(n as i64 + 1, outcome))
        .collect();
    events[5].event.metadata = Metadata {
        mode: Some(WriteMode::ReadOnly),
        reason: Some(Reason::Transport),
        count: Some(3),
        duration_ms: Some(250),
        truncated: true,
        redacted: true,
        omitted_subjects: 2,
        command: Some(StableCode::new("refresh.details")),
        url: Some(SafeUrl::parse("https://one.example.test/cards/ABC-42?token=omitted").unwrap()),
    };
    events[5].event.subjects.push(SubjectSnapshot::new(
        SubjectKey::new("sample", "https://two.example.test", "card", "ABC-42").unwrap(),
        SubjectRole::Context,
    ));
    let mut result = page(filter(), events, true);
    result.known_recording_gaps = true;
    assert!(history.receive(request.id, Ok(result)));
    history.select(5);
    history
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
fn press(history: &mut History, code: KeyCode, id: u64) -> Option<Request> {
    history.key(key(code), id)
}

fn draw(history: &History, width: u16, height: u16, theme: &Theme) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    history.render(area, &mut buf, theme);
    buf
}

fn text(buf: &Buffer) -> String {
    (buf.area.y..buf.area.bottom())
        .map(|y| crate::shell::row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn shot(name: &str, history: &History) {
    let view = history.view(160, 40);
    assert!(view.split, "wide enough for the events and the detail side by side");
    insta::assert_snapshot!(format!("history_{name}"), crate::headless::text::history(&view));
}

#[test]
fn history_screens_as_view_text() {
    let (mut recent, request) = History::recent(AT + 3_600_000, 100);
    recent.receive(
        request.id,
        Ok(page(request.filter, sample().page.unwrap().events, true)),
    );
    shot("recent_activity", &recent);
    let (mut recent, request) = History::recent(AT + 3_600_000, 101);
    recent.receive(request.id, Ok(page(request.filter, vec![], false)));
    shot("recent_empty", &recent);
    let mut all = sample();
    all.filter = EventFilter::default();
    all.page.as_mut().unwrap().filter = EventFilter::default();
    shot("all_activity", &all);
    let mut history = sample();
    shot("events", &history);
    press(&mut history, KeyCode::Tab, 2);
    shot("detail", &history);
    // The panel scrolls by the room the frontend reports; here it is set.
    history.detail_scroll.set(4);
    let scrolled = history.view(160, 40);
    assert_eq!((scrolled.focus, scrolled.detail_scroll), (HistoryFocus::Detail, 4));
    press(&mut history, KeyCode::Char('?'), 2);
    shot("help", &history);

    let (mut history, request) = History::new(EventFilter::default(), 20);
    shot("loading", &history);
    history.receive(request.id, Ok(page(EventFilter::default(), vec![], false)));
    shot("empty", &history);
    let retry = press(&mut history, KeyCode::Char('r'), 21).unwrap();
    history.receive(retry.id, Err(ReadError::Disabled));
    shot("disabled", &history);
    let retry = press(&mut history, KeyCode::Char('r'), 22).unwrap();
    history.receive(retry.id, Err(ReadError::Unavailable));
    shot("unavailable", &history);
}

#[test]
fn the_room_decides_only_whether_the_panels_sit_side_by_side() {
    let history = sample();
    let narrow = history.view(80, 24);
    let wide = history.view(160, 40);
    assert!(!narrow.split && wide.split);
    assert_eq!(
        crate::headless::text::history(&narrow),
        crate::headless::text::history(&wide),
        "the text form reads the same at both sizes"
    );
}

#[test]
fn recent_pagination_keeps_its_original_clock_boundary_until_explicit_refresh() {
    let now = AT + 3_600_000;
    let (mut history, first) = History::recent(now, 1);
    assert_eq!(first.filter.since_ms, Some(now - 86_400_000));
    assert!(first.snapshot.is_none());
    let page_one = page(
        first.filter.clone(),
        vec![entry(1, Outcome::Requested)],
        true,
    );
    let cut = page_one.snapshot.clone();
    history.receive(first.id, Ok(page_one.clone()));
    history.set_now(now + 600_000);
    let next = press(&mut history, KeyCode::Char('n'), 2).unwrap();
    assert_eq!(next.filter, first.filter);
    assert_eq!(next.snapshot, Some(cut.clone()));
    history.receive(
        next.id,
        Ok(page(next.filter, vec![entry(2, Outcome::Started)], false)),
    );
    let previous = press(&mut history, KeyCode::Char('p'), 3).unwrap();
    assert!(previous.cursor.is_none());
    assert_eq!(previous.filter, first.filter);
    assert_eq!(previous.snapshot, Some(cut));
    history.receive(previous.id, Ok(page_one));
    let refresh = press(&mut history, KeyCode::Char('r'), 4).unwrap();
    assert_eq!(refresh.filter.since_ms, Some(now + 600_000 - 86_400_000));
    assert!(refresh.snapshot.is_none());
    assert!(refresh.cursor.is_none());
    assert!(history.recent);
    assert!(
        history
            .scope()
            .starts_with("recent activity · last 24 hours")
    );
    assert!(press(&mut history, KeyCode::Char('r'), 5).is_none());
}

#[test]
fn recent_control_changes_scope_explicitly_and_clamps_the_epoch() {
    let (epoch, request) = History::recent(1000, 1);
    assert!(epoch.recent);
    assert_eq!(request.filter.since_ms, Some(0));
    let mut history = sample();
    history.set_now(AT);
    let recent = press(&mut history, KeyCode::Char('t'), 2).unwrap();
    assert_eq!(recent.filter, recent_filter(AT));
    assert!(history.recent);
    assert!(press(&mut history, KeyCode::Char('t'), 3).is_none());
    history.receive(
        recent.id,
        Ok(page(
            recent.filter,
            vec![entry(1, Outcome::Requested)],
            false,
        )),
    );
    let causal = press(&mut history, KeyCode::Char('c'), 4).unwrap();
    assert!(!history.recent);
    assert!(causal.filter.since_ms.is_none());
    press(&mut history, KeyCode::Char('t'), 5).unwrap();
    let all = press(&mut history, KeyCode::Char('a'), 6).unwrap();
    assert!(!history.recent);
    assert_eq!(all.filter, EventFilter::default());
}

#[test]
fn detail_prioritizes_subject_and_safe_facts_and_exposes_scoped_keys() {
    let mut history = sample();
    let detail = history
        .detail_text()
        .iter()
        .map(|line| line.words())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        detail.find("Subject: sample:card ABC-42").unwrap() < detail.find("Observed UTC").unwrap()
    );
    assert!(detail.find("Command").unwrap() < detail.find("Action ID").unwrap());
    assert!(detail.find("RECORDED SUBJECTS").unwrap() < detail.find("Action ID").unwrap());
    history.focus = Focus::Detail;
    for (width, height) in [(80, 24), (160, 40)] {
        let view = history.view(width, height);
        let screen = crate::headless::text::history(&view);
        assert!(screen.contains("tab events"));
        assert!(screen.contains("↑↓ scroll"));
        assert!(screen.contains("refresh"));
        assert!(screen.contains("esc close"));
        assert_eq!(view.detail_keys_line, Some(3));
        assert_eq!(view.detail_keys.len(), 3);
        for code in ['c', 'i', 'a'] {
            let shown = view
                .detail_keys
                .iter()
                .find(|key| key.label == code.to_string())
                .unwrap();
            let pressed = history.target(&shown.target).unwrap();
            assert_eq!(pressed.code, KeyCode::Char(code));
        }
    }
}

#[test]
fn footer_keys_open_detail_with_enter_and_return_to_events_with_tab() {
    let mut history = sample();
    let shown = |history: &History, label: &str| {
        history
            .view(160, 40)
            .footer
            .into_iter()
            .flat_map(|hint| hint.keys_of)
            .find(|key| key.label == label)
            .unwrap_or_else(|| panic!("{label} is in the footer"))
    };
    assert!(crate::headless::text::history(&history.view(160, 40)).contains("enter details"));
    let key = history.target(&shown(&history, "enter").target).unwrap();
    assert_eq!(key.code, KeyCode::Enter);
    assert!(history.key(key, 2).is_none());
    assert_eq!(history.focus, Focus::Detail);
    assert!(crate::headless::text::history(&history.view(160, 40)).contains("tab events"));
    let key = history.target(&shown(&history, "tab").target).unwrap();
    assert_eq!(key.code, KeyCode::Tab);
    assert!(history.key(key, 3).is_none());
    assert_eq!(history.focus, Focus::Events);
}

#[test]
fn stale_and_repeated_completions_cannot_replace_current_scope() {
    let (mut history, first) = History::new(filter(), 40);
    let all = press(&mut history, KeyCode::Char('a'), 41).unwrap();
    assert!(!history.receive(
        first.id,
        Ok(page(filter(), vec![entry(1, Outcome::Requested)], false))
    ));
    assert_eq!(history.pending.as_ref().unwrap().request.id, all.id);
    assert!(history.receive(
        all.id,
        Ok(page(
            EventFilter::default(),
            vec![entry(2, Outcome::Unknown)],
            false
        ))
    ));
    assert!(!history.receive(all.id, Err(ReadError::Unavailable)));
    assert_eq!(
        history.selected_entry().unwrap().event.outcome,
        Outcome::Unknown
    );
    assert!(history.error.is_none());

    let (mut reopened, current) = History::new(filter(), 42);
    assert!(!reopened.receive(all.id, Err(ReadError::Disabled)));
    assert_eq!(reopened.pending.as_ref().unwrap().request.id, current.id);
}

#[test]
fn repeated_reads_are_deduplicated_but_scope_changes_supersede_pending_reads() {
    let (mut history, _) = History::new(filter(), 1);
    for code in ['r', 'r', 'n', 'p', 'c', 'i'] {
        assert!(press(&mut history, KeyCode::Char(code), 2).is_none());
    }
    let all = press(&mut history, KeyCode::Char('a'), 3).unwrap();
    assert_eq!(all.filter, EventFilter::default());
    assert!(press(&mut history, KeyCode::Char('a'), 4).is_none());
    history.receive(
        all.id,
        Ok(page(all.filter, vec![entry(1, Outcome::Started)], true)),
    );
    let next = press(&mut history, KeyCode::Char('n'), 5).unwrap();
    assert!(press(&mut history, KeyCode::Char('n'), 6).is_none());
    let causal = press(&mut history, KeyCode::Char('c'), 7).unwrap();
    assert_eq!(causal.filter.root_action_id, Some(action(1)));
    assert!(!history.receive(next.id, Err(ReadError::Unavailable)));
}

#[test]
fn pagination_retains_snapshot_including_previous_first_page_without_retaining_events() {
    let (mut history, first) = History::new(filter(), 1);
    assert!(first.snapshot.is_none());
    let first_page = page(
        filter(),
        vec![entry(1, Outcome::Requested), entry(2, Outcome::Started)],
        true,
    );
    let cut = first_page.snapshot.clone();
    history.receive(first.id, Ok(first_page.clone()));
    let next = press(&mut history, KeyCode::Char('n'), 2).unwrap();
    assert_eq!(next.cursor, first_page.next);
    assert_eq!(next.snapshot, Some(cut.clone()));
    history.receive(
        next.id,
        Ok(page(filter(), vec![entry(3, Outcome::Succeeded)], false)),
    );
    assert_eq!(history.len(), 1);
    assert_eq!(history.index, 1);
    assert_eq!(history.cursors.len(), 2);
    assert!(press(&mut history, KeyCode::Char('n'), 3).is_none());
    let previous = press(&mut history, KeyCode::Char('p'), 4).unwrap();
    assert!(previous.cursor.is_none());
    assert_eq!(previous.snapshot, Some(cut));
    history.receive(previous.id, Ok(first_page));
    assert_eq!(history.len(), 2);
    assert_eq!(history.cursors.len(), 1);
    assert_eq!(history.index, 0);
    assert!(press(&mut history, KeyCode::Char('p'), 5).is_none());
    let refresh = press(&mut history, KeyCode::Char('r'), 6).unwrap();
    assert!(refresh.snapshot.is_none());
    assert!(refresh.cursor.is_none());
    assert!(history.page.is_none());
    assert_eq!(history.cursors.len(), 1);
}

#[test]
fn failed_pagination_keeps_displayed_page_selection_and_retry_position() {
    let mut history = sample();
    let before = history.selected_entry().unwrap().event.event_id.clone();
    let request = press(&mut history, KeyCode::Char('n'), 2).unwrap();
    history.receive(request.id, Err(ReadError::Unavailable));
    assert_eq!(history.index, 0);
    assert_eq!(history.cursors.len(), 1);
    assert_eq!(history.selected_entry().unwrap().event.event_id, before);
    let retry = press(&mut history, KeyCode::Char('n'), 3).unwrap();
    assert_eq!(retry.cursor, request.cursor);
    assert_eq!(retry.snapshot, request.snapshot);
}

#[test]
fn causal_scope_uses_only_explicit_root_and_action_scope_is_exact() {
    let mut history = sample();
    let selected = history.selected_entry().unwrap().event.clone();
    assert!(selected.parent_action_id.is_some());
    let causal = press(&mut history, KeyCode::Char('c'), 2).unwrap();
    assert_eq!(
        causal.filter,
        EventFilter {
            root_action_id: Some(selected.root_action_id),
            ..EventFilter::default()
        }
    );
    assert!(causal.snapshot.is_none());
    history.receive(
        causal.id,
        Ok(page(
            causal.filter.clone(),
            vec![entry(4, Outcome::Requested)],
            false,
        )),
    );
    assert!(press(&mut history, KeyCode::Char('c'), 3).is_none());
    let exact = press(&mut history, KeyCode::Char('i'), 4).unwrap();
    assert_eq!(
        exact.filter,
        EventFilter {
            action_id: Some(action(2)),
            ..EventFilter::default()
        }
    );
    let all = press(&mut history, KeyCode::Char('a'), 5).unwrap();
    assert_eq!(all.filter, EventFilter::default());
    assert!(all.snapshot.is_none());
}

#[test]
fn navigation_clamps_and_selection_changes_reset_detail_scroll() {
    let mut history = sample();
    press(&mut history, KeyCode::Home, 2);
    press(&mut history, KeyCode::Up, 2);
    assert_eq!(history.selected, 0);
    press(&mut history, KeyCode::End, 2);
    press(&mut history, KeyCode::Down, 2);
    assert_eq!(history.selected, 9);
    press(&mut history, KeyCode::Tab, 2);
    let _ = draw(&history, 80, 24, &Theme::new(Tier::Mono));
    press(&mut history, KeyCode::PageDown, 2);
    assert!(history.detail_scroll.at() > 0);
    let offset = history.detail_scroll.at();
    press(&mut history, KeyCode::Char('h'), 2);
    press(&mut history, KeyCode::Char('l'), 2);
    assert_eq!(history.detail_scroll.at(), offset);
    press(&mut history, KeyCode::Char('h'), 2);
    press(&mut history, KeyCode::Up, 2);
    assert_eq!(history.selected, 8);
    assert_eq!(history.detail_scroll.at(), 0);
}

#[test]
fn help_is_local_and_does_not_leak_navigation_or_read_keys() {
    let mut history = sample();
    let filter = history.filter.clone();
    let selected = history.selected;
    press(&mut history, KeyCode::Char('x'), 2);
    assert!(history.help);
    for code in ['a', 'c', 'i', 'r', 'n', 'p', 'j'] {
        assert!(press(&mut history, KeyCode::Char(code), 3).is_none());
    }
    assert_eq!(history.filter, filter);
    assert_eq!(history.selected, selected);
    assert!(history.pending.is_none());
    assert!(history.dismiss_help());
    assert!(!history.dismiss_help());
    history.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL), 4);
    assert_eq!(history.filter, filter);
}

#[test]
fn mouse_hits_stay_within_overlay_and_empty_space_preserves_selection() {
    let mut history = sample();
    let _ = draw(&history, 160, 40, &Theme::new(Tier::Mono));
    let (area, index) = history.rows.borrow()[2];
    assert!(
        history
            .mouse(MouseKind::Press, area.x + 2, area.y)
            .is_none()
    );
    assert_eq!(history.selected, index);
    let detail = history.detail_area.get();
    history.mouse(MouseKind::Press, detail.x + 1, detail.y + 1);
    assert_eq!(history.focus, Focus::Detail);
    assert_eq!(history.selected, index);
    history.mouse(MouseKind::ScrollDown, detail.x + 1, detail.y + 1);
    assert!(history.detail_scroll.at() > 0);
    let list = history.list_area.get();
    history.mouse(MouseKind::Press, list.x + 2, list.bottom() - 2);
    assert_eq!(history.focus, Focus::Events);
    assert_eq!(history.selected, index);
    let (hit, expected) = history
        .keys
        .borrow()
        .iter()
        .find(|(_, k)| k.code == KeyCode::Char('n'))
        .copied()
        .unwrap();
    assert_eq!(
        history.mouse(MouseKind::Press, hit.x, hit.y),
        Some(expected)
    );
}

#[test]
fn incoming_wrong_filter_and_oversized_page_fail_without_replacing_evidence() {
    let (mut history, request) = History::new(filter(), 1);
    history.receive(
        request.id,
        Ok(page(
            EventFilter::default(),
            vec![entry(1, Outcome::Succeeded)],
            false,
        )),
    );
    assert_eq!(history.error, Some(ReadError::Unavailable));
    assert!(history.page.is_none());
    let retry = press(&mut history, KeyCode::Char('r'), 2).unwrap();
    let too_many = (1..=51).map(|n| entry(n, Outcome::Requested)).collect();
    history.receive(retry.id, Ok(page(filter(), too_many, false)));
    assert_eq!(history.error, Some(ReadError::Unavailable));
    assert!(history.page.is_none());
}

#[test]
fn a_matching_request_cannot_advance_its_fixed_snapshot() {
    let mut history = sample();
    let original = history.selected_entry().unwrap().event.event_id.clone();
    let request = press(&mut history, KeyCode::Char('n'), 2).unwrap();
    let mut changed = page(filter(), vec![entry(11, Outcome::Requested)], false);
    changed.snapshot = snapshot(101);
    history.receive(request.id, Ok(changed));
    assert_eq!(history.error, Some(ReadError::Unavailable));
    assert_eq!(history.selected_entry().unwrap().event.event_id, original);
    assert_eq!(history.index, 0);
}

#[test]
fn detail_uses_allowlisted_facts_without_invented_success_or_payloads() {
    let mut history = sample();
    history.select(6);
    let detail = history
        .detail_text()
        .iter()
        .map(|line| line.words())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(detail.contains("unknown"));
    assert!(!detail.contains("succeeded"));
    assert!(detail.contains("Parent ID"));
    assert!(detail.contains("Root ID"));
    assert!(detail.contains("Recorded UTC"));
    assert!(detail.contains("Observed UTC"));
    assert_eq!(timestamp(0), "1970-01-01T00:00:00.000Z");
    assert_eq!(timestamp(-1), "1969-12-31T23:59:59.999Z");
    assert_eq!(safe_text("unsafe\nraw payload"), "[withheld]");
    assert_eq!(safe_text("Bearer abcdef01234567890123456789"), "[withheld]");
}

#[test]
fn rendering_stays_inside_area_at_tiny_widths_and_with_long_unicode_identities() {
    let mut history = sample();
    history.filter.subject = Some(
        SubjectKey::new(
            "sample",
            &format!("https://one.example.test/{}", "instance/".repeat(30)),
            "folder",
            &format!("C:/work/{}", "界".repeat(80)),
        )
        .unwrap(),
    );
    for (width, height) in [(1, 1), (2, 2), (20, 4), (40, 12), (80, 24), (160, 40)] {
        let _ = draw(&history, width, height, &Theme::new(Tier::Mono));
    }
    let screen = text(&draw(&history, 80, 24, &Theme::new(Tier::Mono)));
    assert!(screen.contains("coverage may be incomplete"));
    assert!(screen.contains("Known recording gaps"));
}
