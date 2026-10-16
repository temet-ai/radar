use super::*;
use serde_json::{Value, json};

struct Fixture {
    journal: Journal,
    session: SessionId,
    directory: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let directory = std::env::temp_dir().join(format!("radar-events-{}", Uuid::new_v4()));
        let mut journal = Journal::open(&directory.join("actions.sqlite3")).unwrap();
        let session = journal.start_session_with(SessionOrigin::Cli).unwrap();
        Self {
            journal,
            session,
            directory,
        }
    }

    fn observe(&mut self, key: &SubjectKey, kind: &'static str, at: i64) -> ActionId {
        let action = ActionId::new();
        self.journal
            .begin_with_id_at(
                &self.session,
                &action,
                ActionDescriptor::new(kind, Origin::Plugin("sample".into()))
                    .with_subject(SubjectSnapshot::new(key.clone(), SubjectRole::Primary)),
                None,
                at,
            )
            .unwrap();
        action
    }

    fn page(&self, filter: &EventFilter) -> EventPage {
        self.journal
            .query_events(filter, Limits::default(), None)
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn key() -> SubjectKey {
    SubjectKey::new("sample", "https://one.example.test", "card", "42").unwrap()
}

fn value(value: &impl Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn events_match_exact_scopes_and_all_predicates_without_inferred_state() {
    let mut fixture = Fixture::new();
    let key = key();
    let action = fixture.observe(&key, "card.inspect", 100);
    fixture
        .journal
        .record_at(&action, Outcome::Started, Metadata::default(), 120)
        .unwrap();
    fixture
        .journal
        .record_at(
            &action,
            Outcome::Partial,
            Metadata {
                truncated: true,
                ..Metadata::default()
            },
            200,
        )
        .unwrap();
    for other in [
        SubjectKey::new("other", &key.instance, &key.kind, &key.id).unwrap(),
        SubjectKey::new(
            &key.provider,
            "https://two.example.test",
            &key.kind,
            &key.id,
        )
        .unwrap(),
        SubjectKey::new(&key.provider, &key.instance, "note", &key.id).unwrap(),
        SubjectKey::new(&key.provider, &key.instance, &key.kind, "43").unwrap(),
    ] {
        fixture.observe(&other, "card.inspect", 100);
    }
    fixture.observe(&key, "card.inspect.more", 100);
    let filter = EventFilter {
        session_id: Some(fixture.session.clone()),
        action_id: Some(action.clone()),
        root_action_id: Some(action.clone()),
        kind: Some(StableCode::new("card.inspect")),
        subject: Some(key.clone()),
        outcome: Some(Outcome::Requested),
        since_ms: Some(100),
        until_ms: Some(100),
    };
    let page = fixture.page(&filter);
    assert_eq!(page.events.len(), 1);
    assert_eq!(page.events[0].event.action_id, action);
    assert_eq!(page.events[0].event.outcome, Outcome::Requested);
    assert_eq!(
        page.events[0].requested_sequence,
        page.events[0].event.sequence
    );
    assert_eq!(page.events[0].source.session_origin, SessionOrigin::Cli);
    assert_eq!(page.authority, "observations_only");
    assert_eq!(page.execution_authorization, "unspecified");
    assert!(page.completeness_not_guaranteed);
    let events = fixture.page(&EventFilter {
        subject: Some(key),
        kind: filter.kind.clone(),
        ..EventFilter::default()
    });
    assert_eq!(events.events.len(), 3);
    assert_eq!(
        events.events[2].requested_sequence,
        events.events[0].event.sequence
    );
    assert!(events.events[2].event.metadata.truncated);
    assert!(events.events[2].event.sequence > events.events[2].requested_sequence);
    for changed in [
        EventFilter {
            kind: Some(StableCode::new("card.missing")),
            ..filter.clone()
        },
        EventFilter {
            session_id: Some(SessionId::new()),
            ..filter.clone()
        },
        EventFilter {
            root_action_id: Some(ActionId::new()),
            ..filter.clone()
        },
        EventFilter {
            outcome: Some(Outcome::Succeeded),
            ..filter.clone()
        },
        EventFilter {
            since_ms: Some(101),
            until_ms: Some(101),
            ..filter.clone()
        },
    ] {
        assert!(fixture.page(&changed).events.is_empty());
    }
}

#[test]
fn since_finds_every_event_observed_since_whatever_order_they_were_recorded_in() {
    let mut fixture = Fixture::new();
    let key = key();
    // Recorded in the order 50, 300, 100, 200: observed times do not follow sequence.
    let early = fixture.observe(&key, "card.inspect", 50);
    let late = fixture.observe(&key, "card.inspect", 300);
    let middle = fixture.observe(&key, "card.inspect", 100);
    let after = fixture.observe(&key, "card.inspect", 200);
    let ids = |since| -> Vec<ActionId> {
        fixture
            .page(&EventFilter {
                since_ms: Some(since),
                ..EventFilter::default()
            })
            .events
            .into_iter()
            .map(|e| e.event.action_id)
            .collect()
    };
    assert_eq!(ids(100), vec![late.clone(), middle.clone(), after.clone()]);
    assert_eq!(ids(250), vec![late]);
    assert_eq!(ids(301), Vec::<ActionId>::new());
    assert_eq!(ids(0).len(), 4);
    assert!(!ids(0).is_empty() && ids(0)[0] == early);
}

#[test]
fn event_outcomes_remain_distinct_and_original_event_fields_survive() {
    let mut fixture = Fixture::new();
    for outcome in [
        Outcome::Succeeded,
        Outcome::Failed,
        Outcome::Cancelled,
        Outcome::Blocked,
        Outcome::Partial,
        Outcome::Unknown,
    ] {
        let action = fixture.observe(&key(), "card.inspect", 100);
        fixture
            .journal
            .record_at(&action, Outcome::Confirmed, Metadata::default(), 90)
            .unwrap();
        fixture
            .journal
            .record_at(&action, Outcome::Started, Metadata::default(), 90)
            .unwrap();
        fixture
            .journal
            .record_at(
                &action,
                outcome,
                Metadata {
                    reason: Some(Reason::Unspecified),
                    count: Some(0),
                    ..Metadata::default()
                },
                80,
            )
            .unwrap();
        let page = fixture.page(&EventFilter {
            action_id: Some(action.clone()),
            ..EventFilter::default()
        });
        let original = fixture
            .journal
            .query(&Query {
                action_id: Some(action),
                ..Query::default()
            })
            .unwrap();
        assert_eq!(
            value(
                &page
                    .events
                    .iter()
                    .map(|entry| &entry.event)
                    .collect::<Vec<_>>()
            ),
            value(&original)
        );
        assert_eq!(
            page.events
                .iter()
                .map(|entry| entry.event.outcome)
                .collect::<Vec<_>>(),
            vec![
                Outcome::Requested,
                Outcome::Confirmed,
                Outcome::Started,
                outcome
            ]
        );
        assert!(
            page.events
                .windows(2)
                .all(|pair| pair[0].event.sequence < pair[1].event.sequence)
        );
        let only = fixture.page(&EventFilter {
            outcome: Some(outcome),
            ..EventFilter::default()
        });
        assert_eq!(only.events.len(), 1);
    }
}

#[test]
fn event_pages_keep_their_cut_after_reopen_and_concurrent_appends() {
    let mut fixture = Fixture::new();
    let first = fixture.observe(&key(), "card.inspect", 100);
    fixture.observe(&key(), "card.inspect", 100);
    fixture.observe(&key(), "card.inspect", 100);
    let filter = EventFilter::default();
    let expected = fixture.page(&filter);
    let one = fixture
        .journal
        .query_events(
            &filter,
            Limits {
                records: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    let mut other = Journal::open(&fixture.directory.join("actions.sqlite3")).unwrap();
    other
        .record_at(&first, Outcome::Succeeded, Metadata::default(), 50)
        .unwrap();
    other.record_gap(&fixture.session, 1, 0, 100).unwrap();
    let new_session = other.start_session().unwrap();
    other
        .begin(
            &new_session,
            ActionDescriptor::new("card.new", Origin::Host),
            None,
        )
        .unwrap();
    let evidence = expected.events[0].event.event_id.clone();
    other
        .apply_work_map(&ChangeRequest {
            id: ChangeId::new(),
            provenance: Provenance {
                recording_action_id: first,
                evidence_event_id: evidence,
                basis: Basis::UserAsserted,
            },
            observed_at_ms: 100,
            mutation: Mutation::CreateItem {
                item_id: ItemId::new(),
                kind: StableCode::new("card"),
            },
        })
        .unwrap();
    let reopened = Journal::open(&fixture.directory.join("actions.sqlite3")).unwrap();
    let mut entries = one.events;
    let mut cursor = one.next;
    while let Some(next) = cursor {
        let page = reopened
            .query_events(
                &filter,
                Limits {
                    records: 1,
                    ..Limits::default()
                },
                Some(&next),
            )
            .unwrap();
        assert_eq!(page.snapshot, expected.snapshot);
        assert!(!page.known_recording_gaps);
        entries.extend(page.events);
        cursor = page.next;
        assert!(entries.len() <= expected.events.len());
    }
    assert_eq!(value(&entries), value(&expected.events));
    let fresh = fixture.page(&filter);
    assert!(fresh.snapshot.event_sequence() > expected.snapshot.event_sequence());
    assert!(fresh.snapshot.map_sequence() > expected.snapshot.map_sequence());
    assert!(fresh.known_recording_gaps);
}

#[test]
fn event_cursors_reject_changed_filters_invalid_positions_and_cuts() {
    let mut fixture = Fixture::new();
    fixture.observe(&key(), "card.inspect", 100);
    fixture.observe(&key(), "card.inspect", 100);
    let filter = EventFilter::default();
    let page = fixture
        .journal
        .query_events(
            &filter,
            Limits {
                records: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    let cursor = page.next.unwrap();
    assert!(
        fixture
            .journal
            .query_events(
                &EventFilter {
                    outcome: Some(Outcome::Requested),
                    ..filter.clone()
                },
                Limits::default(),
                Some(&cursor)
            )
            .is_err()
    );
    for (field, replacement) in [
        ("after_sequence", json!(-1)),
        ("after_sequence", json!(page.snapshot.event_sequence() + 1)),
    ] {
        let mut forged = value(&cursor);
        forged[field] = replacement;
        let forged = serde_json::from_value(forged).unwrap();
        assert!(
            fixture
                .journal
                .query_events(&filter, Limits::default(), Some(&forged))
                .is_err()
        );
    }
    for field in ["event_sequence", "map_sequence", "gap_sequence"] {
        let mut forged = value(&cursor);
        forged["snapshot"][field] = json!(i64::MAX);
        let forged = serde_json::from_value(forged).unwrap();
        assert!(
            fixture
                .journal
                .query_events(&filter, Limits::default(), Some(&forged))
                .is_err()
        );
    }
    let other = Fixture::new();
    assert!(
        other
            .journal
            .query_events(&filter, Limits::default(), Some(&cursor))
            .is_err()
    );
    fixture.observe(&key(), "card.inspect", 100);
    let newer = fixture.journal.work_map_snapshot().unwrap();
    assert!(
        fixture
            .journal
            .query_events_at(&filter, Limits::default(), Some(&cursor), Some(&newer))
            .is_err()
    );
    let old = fixture
        .journal
        .query_events_at(&filter, Limits::default(), None, Some(&page.snapshot))
        .unwrap();
    assert_eq!(old.events.len(), 2);
    assert_eq!(old.snapshot, page.snapshot);
}

#[test]
fn event_gap_scope_covers_the_filtered_snapshot_not_just_this_page() {
    let mut fixture = Fixture::new();
    fixture.observe(&key(), "card.inspect", 100);
    let old_session = fixture.session.clone();
    fixture.session = fixture.journal.start_session().unwrap();
    let second = fixture.observe(&key(), "card.inspect", 200);
    fixture
        .journal
        .record_at(&second, Outcome::Partial, Metadata::default(), 250)
        .unwrap();
    fixture
        .journal
        .record_gap(&fixture.session, 2, 1, 300)
        .unwrap();
    let filter = EventFilter {
        kind: Some(StableCode::new("card.inspect")),
        subject: Some(key()),
        outcome: Some(Outcome::Requested),
        ..EventFilter::default()
    };
    let first = fixture
        .journal
        .query_events(
            &filter,
            Limits {
                records: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    assert_eq!(first.events[0].event.session_id, old_session);
    assert!(first.known_recording_gaps);
    let second = fixture
        .journal
        .query_events(&filter, Limits::default(), first.next.as_ref())
        .unwrap();
    assert!(second.known_recording_gaps);
    assert!(
        !fixture
            .page(&EventFilter {
                until_ms: Some(150),
                ..filter.clone()
            })
            .known_recording_gaps
    );
    assert!(
        !fixture
            .page(&EventFilter {
                session_id: Some(old_session),
                ..filter.clone()
            })
            .known_recording_gaps
    );
    assert!(
        !fixture
            .page(&EventFilter {
                outcome: Some(Outcome::Succeeded),
                ..filter.clone()
            })
            .known_recording_gaps
    );
    let empty = fixture.page(&EventFilter {
        kind: Some(StableCode::new("absent")),
        ..filter
    });
    assert!(empty.events.is_empty() && !empty.known_recording_gaps);
    assert!(empty.completeness_not_guaranteed);
}

#[test]
fn event_output_budget_includes_envelope_cursor_and_escaped_strings() {
    let mut fixture = Fixture::new();
    let key = SubjectKey::new(
        "sample",
        "local",
        "card",
        &format!("/tmp/{}", "雪\\\"".repeat(100)),
    )
    .unwrap();
    for _ in 0..3 {
        fixture.observe(&key, "card.inspect", 100);
    }
    let filter = EventFilter::default();
    let first = fixture
        .journal
        .query_events(
            &filter,
            Limits {
                records: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    let byte_limit = encode_safe_output(&first).unwrap().len();
    assert!(byte_limit > 1024);
    let cut = first.snapshot.clone();
    let exact = fixture
        .journal
        .query_events_at(
            &filter,
            Limits {
                records: 3,
                bytes: byte_limit,
            },
            None,
            Some(&cut),
        )
        .unwrap();
    assert_eq!(exact.events.len(), 1);
    assert_eq!(encode_safe_output(&exact).unwrap().len(), byte_limit);
    assert!(exact.truncated && exact.next.is_some());
    assert!(
        fixture
            .journal
            .query_events_at(
                &filter,
                Limits {
                    records: 3,
                    bytes: byte_limit - 1
                },
                None,
                Some(&cut)
            )
            .is_err()
    );
    let continued = fixture
        .journal
        .query_events(&filter, Limits::default(), exact.next.as_ref())
        .unwrap();
    assert_eq!(continued.events.len(), 2);
    assert!(!continued.truncated && continued.next.is_none());
    let single_filter = EventFilter {
        action_id: Some(continued.events[1].event.action_id.clone()),
        ..filter
    };
    let single = fixture.page(&single_filter);
    let exact_final_bytes = encode_safe_output(&single).unwrap().len();
    let exact_final = fixture
        .journal
        .query_events_at(
            &single_filter,
            Limits {
                records: 1,
                bytes: exact_final_bytes,
            },
            None,
            Some(&single.snapshot),
        )
        .unwrap();
    assert_eq!(
        encode_safe_output(&exact_final).unwrap().len(),
        exact_final_bytes
    );
    assert!(exact_final.next.is_none());
}

#[test]
fn event_query_limits_and_deserialized_filter_validation_are_bounded() {
    let fixture = Fixture::new();
    let filter = EventFilter::default();
    for limits in [
        Limits {
            records: 0,
            ..Limits::default()
        },
        Limits {
            records: 1001,
            ..Limits::default()
        },
        Limits {
            bytes: 1023,
            ..Limits::default()
        },
        Limits {
            bytes: MAX_PAGE_BYTES + 1,
            ..Limits::default()
        },
    ] {
        assert!(fixture.journal.query_events(&filter, limits, None).is_err());
    }
    for (field, replacement) in [
        ("session_id", json!("bad")),
        ("action_id", json!("00000000-0000-4000-8000-00000000000A")),
        ("root_action_id", json!("bad")),
        ("kind", json!("unsafe kind")),
        (
            "subject",
            json!({"provider":"sample","instance":"one","kind":"card","id":"token=private-value"}),
        ),
        ("since_ms", json!(-1)),
        ("until_ms", json!(i64::MAX)),
    ] {
        let mut invalid = value(&filter);
        invalid[field] = replacement;
        let invalid = serde_json::from_value(invalid).unwrap();
        assert!(
            fixture
                .journal
                .query_events(&invalid, Limits::default(), None)
                .is_err()
        );
    }
    assert!(
        fixture
            .journal
            .query_events(
                &EventFilter {
                    since_ms: Some(20),
                    until_ms: Some(10),
                    ..filter
                },
                Limits::default(),
                None
            )
            .is_err()
    );
}

#[test]
fn event_record_limit_uses_a_sequence_only_lookahead_at_the_maximum() {
    let mut fixture = Fixture::new();
    // One boundary test exercises the 1001st candidate without widening the
    // existing generic query's public 1000-event limit.
    for _ in 0..1001 {
        fixture.observe(&key(), "card.inspect", 100);
    }
    let filter = EventFilter::default();
    let first = fixture
        .journal
        .query_events(
            &filter,
            Limits {
                records: 1000,
                bytes: MAX_PAGE_BYTES,
            },
            None,
        )
        .unwrap();
    assert!(first.truncated);
    assert!(first.events.len() <= 1000);
    let mut count = first.events.len();
    let mut cursor = first.next;
    while let Some(next) = cursor {
        let page = fixture
            .journal
            .query_events(
                &filter,
                Limits {
                    records: 1000,
                    bytes: MAX_PAGE_BYTES,
                },
                Some(&next),
            )
            .unwrap();
        count += page.events.len();
        assert!(count <= 1001);
        cursor = page.next;
    }
    assert_eq!(count, 1001);
}

#[test]
fn event_output_rechecks_newly_registered_secrets_without_changing_history() {
    for field in ["subject", "metadata", "source"] {
        let mut fixture = Fixture::new();
        let secret = format!("eventsecret{}", Uuid::new_v4().simple());
        let key = if field == "subject" {
            SubjectKey::new("sample", "local", "card", &secret).unwrap()
        } else {
            key()
        };
        let action = fixture.observe(&key, "card.inspect", 100);
        if field == "metadata" {
            fixture
                .journal
                .record_at(
                    &action,
                    Outcome::Succeeded,
                    Metadata {
                        url: Some(
                            SafeUrl::parse(&format!("https://example.test/{secret}")).unwrap(),
                        ),
                        ..Metadata::default()
                    },
                    100,
                )
                .unwrap();
        }
        if field == "source" {
            fixture
                .journal
                .connection
                .execute(
                    "UPDATE sessions SET build_version=?1 WHERE id=?2",
                    params![secret, fixture.session.as_str()],
                )
                .unwrap();
        }
        let filter = EventFilter::default();
        let before = fixture.page(&filter);
        crate::log::secret(&secret);
        let error = fixture
            .journal
            .query_events(&filter, Limits::default(), None)
            .unwrap_err();
        assert!(!format!("{error:#}").contains(&secret));
        assert_eq!(
            fixture.journal.work_map_snapshot().unwrap(),
            before.snapshot
        );
        assert!(
            fixture
                .journal
                .query_events(
                    &EventFilter {
                        kind: Some(StableCode::try_new(&secret).unwrap()),
                        ..filter
                    },
                    Limits::default(),
                    None
                )
                .is_err()
        );
    }
}

#[test]
fn event_discovery_preserves_migrated_estimated_times_and_source_provenance() {
    let directory = std::env::temp_dir().join(format!("radar-events-legacy-{}", Uuid::new_v4()));
    private_directory(&directory).unwrap();
    let path = directory.join("actions.sqlite3");
    let connection = Connection::open(&path).unwrap();
    for schema in [
        super::super::schema::V1,
        super::super::schema::V2,
        super::super::schema::V3,
    ] {
        connection.execute_batch(schema).unwrap();
    }
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    let session = SessionId::new();
    let action = ActionId::new();
    let event = EventId::new();
    connection.execute("INSERT INTO sessions(id,started_at_ms,process_id,build_version,origin) VALUES (?1,1000,1,'legacy-build','unknown')", [session.as_str()]).unwrap();
    connection.execute("INSERT INTO actions(id,session_id,root_action_id,correlation_id,kind,origin_kind,origin_id,created_at_ms) VALUES (?1,?2,?1,?1,'card.inspect','plugin','sample',1000)", params![action.as_str(),session.as_str()]).unwrap();
    connection.execute("INSERT INTO action_events(id,action_id,occurred_at_ms,outcome,metadata_json) VALUES (?1,?2,1000,'requested','{}')", params![event.as_str(),action.as_str()]).unwrap();
    drop(connection);
    let fixture = Fixture {
        journal: Journal::open(&path).unwrap(),
        session,
        directory,
    };
    let page = fixture.page(&EventFilter::default());
    let entry = &page.events[0];
    assert_eq!(entry.event.event_id, event);
    assert_eq!(entry.event.event_version, 1);
    assert_eq!(entry.event.observed_at_ms, 1000);
    assert_eq!(entry.event.recorded_at_ms, 1000);
    assert!(entry.event.observation_time_estimated);
    assert_eq!(entry.requested_sequence, entry.event.sequence);
    assert_eq!(entry.source.build_version, "legacy-build");
    assert_eq!(entry.source.origin, Origin::Plugin("sample".into()));
    assert_eq!(entry.source.session_origin, SessionOrigin::Unknown);
}
