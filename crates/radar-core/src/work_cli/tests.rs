use super::*;
use crate::journal::work_map::{
    AssertionTarget, ItemId, RelationKind, SemanticLabel, TagId, TagTarget,
};
use crate::journal::{
    ActionDescriptor, Journal, Origin, Outcome, Query, SessionId, SessionOrigin, StableCode,
};
use serde_json::{Value, json};
use std::path::PathBuf;

struct Fixture {
    directory: PathBuf,
    journal: Journal,
    session: SessionId,
}
impl Fixture {
    fn new() -> Self {
        let directory =
            std::env::temp_dir().join(format!("radar-work-cli-{}", ChangeId::new().as_str()));
        let mut journal = Journal::open(&directory.join("actions.sqlite3")).unwrap();
        let session = journal.start_session_with(SessionOrigin::Cli).unwrap();
        Self {
            directory,
            journal,
            session,
        }
    }
    fn run(&mut self, command: Command, mode: Mode) -> Result<Value, Error> {
        execute(&mut self.journal, &self.session, command, mode, 100)
            .result
            .map(|reply| serde_json::from_slice(&reply.bytes).unwrap())
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}
fn intent(mutation: Mutation) -> Intent {
    Intent {
        change_id: ChangeId::new(),
        basis: Basis::UserAsserted,
        evidence: Evidence::Assertion,
        mutation,
    }
}
fn create(item: &ItemId) -> Intent {
    intent(Mutation::CreateItem {
        item_id: item.clone(),
        kind: StableCode::new("card"),
    })
}
fn batch(changes: Vec<Intent>) -> Command {
    Command::Apply(Batch {
        version: 1,
        changes,
    })
}
fn items(limit: usize, cursor: Option<ItemCursor>) -> Command {
    Command::Items(ItemsQuery {
        version: 1,
        filter: ItemFilter::default(),
        limit,
        cursor,
    })
}

#[test]
fn input_is_bounded_typed_and_never_echoes_parser_details() {
    let valid = json!({"version":1,"filter":{"all_tags":[]},"limit":10,"cursor":null});
    assert!(parse("items", valid.to_string().as_bytes()).is_ok());
    let mut extra = valid.clone();
    extra["secret-field-value"] = json!("secret-content");
    for input in [
        extra.to_string(),
        "{\"version\":\"secret-content\"}".into(),
        "secret-content".into(),
        " ".repeat(MAX_QUERY_BYTES + 1),
    ] {
        let error = parse("items", input.as_bytes()).unwrap_err().to_string();
        assert_eq!(error, "work: invalid or unsafe input");
    }
    let duplicate =
        r#"{"version":1,"version":1,"filter":{"all_tags":[]},"limit":10,"cursor":null}"#;
    assert!(parse("items", duplicate.as_bytes()).is_err());
    let mut nested = json!({"version":1,"subject":{"provider":"alpha","instance":"one","kind":"card","id":"42","secret-content":"hidden"},"action_id":ActionId::new()});
    assert!(parse("resolve", nested.to_string().as_bytes()).is_err());
    nested["subject"]
        .as_object_mut()
        .unwrap()
        .remove("secret-content");
    assert!(parse("resolve", nested.to_string().as_bytes()).is_ok());
}

#[test]
fn input_requires_explicit_evidence_and_distinct_stable_ids() {
    let one = create(&ItemId::new());
    let encoded = serde_json::to_vec(&Batch {
        version: 1,
        changes: vec![one.clone(), one.clone()],
    })
    .unwrap();
    assert!(parse("apply", encoded.as_slice()).is_err());
    for basis in [Basis::Observed, Basis::Inferred] {
        let mut ungrounded = one.clone();
        ungrounded.basis = basis;
        let encoded = serde_json::to_vec(&Batch {
            version: 1,
            changes: vec![ungrounded],
        })
        .unwrap();
        assert!(parse("apply", encoded.as_slice()).is_err());
    }
    let mut fixture = Fixture::new();
    let action = fixture
        .journal
        .begin(
            &fixture.session,
            ActionDescriptor::new("alpha.observe", Origin::Plugin("alpha".into())),
            None,
        )
        .unwrap();
    let event = fixture
        .journal
        .query(&Query {
            action_id: Some(action.clone()),
            limit: 1,
            ..Default::default()
        })
        .unwrap()
        .remove(0);
    let mut evidenced = one;
    evidenced.evidence = Evidence::Event {
        event_id: event.event_id.clone(),
    };
    let id = evidenced.change_id.clone();
    fixture.run(batch(vec![evidenced]), Mode::Write).unwrap();
    let record = fixture.journal.get_work_change(&id, None).unwrap().unwrap();
    assert_eq!(record.evidence.action.action_id, action);
    assert_ne!(record.recording.action_id, action);
    assert_eq!(
        record.evidence.action.origin,
        Origin::Plugin("alpha".into())
    );
}

#[test]
fn mutation_receipts_are_durable_and_retries_preserve_original_attribution() {
    let mut fixture = Fixture::new();
    let request = create(&ItemId::new());
    let first = fixture
        .run(batch(vec![request.clone()]), Mode::Write)
        .unwrap();
    let original = fixture
        .journal
        .get_work_change(&request.change_id, None)
        .unwrap()
        .unwrap();
    fixture.journal.finish_session(&fixture.session).unwrap();
    fixture.session = fixture
        .journal
        .start_session_with(SessionOrigin::Cli)
        .unwrap();
    let second = fixture
        .run(batch(vec![request.clone()]), Mode::Write)
        .unwrap();
    assert_eq!(
        first["changes"][0]["sequence"],
        second["changes"][0]["sequence"]
    );
    assert_eq!(second["changes"][0]["replayed"], true);
    assert_ne!(
        first["attempt"]["action_id"],
        second["attempt"]["action_id"]
    );
    assert_eq!(
        first["changes"][0]["original_recording_action_id"],
        second["changes"][0]["original_recording_action_id"]
    );
    assert_eq!(
        fixture
            .journal
            .get_work_change(&request.change_id, None)
            .unwrap()
            .unwrap()
            .request,
        original.request
    );
    let mut changed_evidence = request.clone();
    changed_evidence.evidence = Evidence::Event {
        event_id: original.request.provenance.evidence_event_id.clone(),
    };
    assert_eq!(
        fixture
            .run(batch(vec![changed_evidence]), Mode::Write)
            .unwrap_err(),
        Error::Conflict
    );
    let mut changed = request;
    changed.mutation = Mutation::CreateItem {
        item_id: ItemId::new(),
        kind: StableCode::new("card"),
    };
    assert_eq!(
        fixture.run(batch(vec![changed]), Mode::Write).unwrap_err(),
        Error::Conflict
    );
    assert_eq!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn modes_are_checked_inside_the_writer_and_denials_are_audited() {
    let mut fixture = Fixture::new();
    let new = create(&ItemId::new());
    assert_eq!(
        fixture
            .run(batch(vec![new.clone()]), Mode::ReadOnly)
            .unwrap_err(),
        Error::PermissionDenied
    );
    assert!(
        fixture
            .journal
            .get_work_change(&new.change_id, None)
            .unwrap()
            .is_none()
    );
    let events = fixture
        .journal
        .query(&Query {
            kind: Some(StableCode::new("work.apply")),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(events.last().unwrap().outcome, Outcome::Blocked);
    let tag = TagId::new();
    fixture
        .run(
            batch(vec![intent(Mutation::CreateTag {
                tag_id: tag.clone(),
                namespace: TagKey::new("area").unwrap(),
                key: TagKey::new("one").unwrap(),
                label: SemanticLabel::new("One").unwrap(),
            })]),
            Mode::Write,
        )
        .unwrap();
    let rename = intent(Mutation::RenameTag {
        tag_id: tag,
        label: SemanticLabel::new("Two").unwrap(),
    });
    assert_eq!(
        fixture
            .run(batch(vec![new.clone(), rename.clone()]), Mode::Write)
            .unwrap_err(),
        Error::PermissionDenied
    );
    assert!(
        fixture
            .journal
            .get_work_change(&new.change_id, None)
            .unwrap()
            .is_none()
    );
    fixture.run(batch(vec![new, rename]), Mode::Edit).unwrap();
    fixture.run(items(10, None), Mode::ReadOnly).unwrap();
}

#[test]
fn explicit_tag_relation_and_binding_flow_is_atomic_and_handoff_stays_observational() {
    let mut fixture = Fixture::new();
    let one = ItemId::new();
    let two = ItemId::new();
    let tag = TagId::new();
    let relation = intent(Mutation::RelateItems {
        from: one.clone(),
        to: two.clone(),
        kind: RelationKind::DependsOn,
    });
    let changes = vec![
        create(&one),
        create(&two),
        intent(Mutation::CreateTag {
            tag_id: tag.clone(),
            namespace: TagKey::new("area").unwrap(),
            key: TagKey::new("first").unwrap(),
            label: SemanticLabel::new("First").unwrap(),
        }),
        intent(Mutation::AssignTag {
            tag_id: tag.clone(),
            target: TagTarget::Item(one.clone()),
        }),
        relation.clone(),
    ];
    let receipt = fixture.run(batch(changes), Mode::Write).unwrap();
    let subject = SubjectKey::new("alpha", "one", "card", "42").unwrap();
    fixture
        .run(
            batch(vec![intent(Mutation::BindSubject {
                item_id: one.clone(),
                subject: subject.clone(),
                from_sequence: receipt["attempt"]["requested_sequence"].as_i64().unwrap(),
                through_sequence: None,
            })]),
            Mode::Write,
        )
        .unwrap();
    let filter = ItemFilter {
        all_tags: vec![tag],
    };
    assert_eq!(
        fixture
            .journal
            .query_items(&filter, 10, None)
            .unwrap()
            .items[0]
            .item_id,
        one
    );
    let page = fixture
        .run(
            Command::Handoff(HandoffQuery {
                version: 1,
                selection: Selection {
                    items: vec![one.clone()],
                    actions: vec![],
                },
                records: 100,
                bytes: 262_144,
                cursor: None,
            }),
            Mode::ReadOnly,
        )
        .unwrap();
    assert_eq!(page["authority"], "observations_only");
    assert_eq!(page["execution_authorization"], "unspecified");
    assert_eq!(page["completeness_not_guaranteed"], true);
    let retract = intent(Mutation::Retract {
        claim_id: relation.change_id,
        expected: AssertionTarget::Relation {
            from: one,
            to: two,
            kind: RelationKind::DependsOn,
        },
    });
    assert_eq!(
        fixture
            .run(batch(vec![retract.clone()]), Mode::Write)
            .unwrap_err(),
        Error::PermissionDenied
    );
    fixture.run(batch(vec![retract]), Mode::Edit).unwrap();
}

#[test]
fn queries_exclude_their_own_audit_and_keep_the_original_cut_on_continuation() {
    let mut fixture = Fixture::new();
    fixture
        .run(
            batch(vec![create(&ItemId::new()), create(&ItemId::new())]),
            Mode::Write,
        )
        .unwrap();
    let before = fixture.journal.work_map_snapshot().unwrap();
    let first = fixture.run(items(1, None), Mode::ReadOnly).unwrap();
    let cut: crate::journal::work_map::Snapshot =
        serde_json::from_value(first["snapshot"].clone()).unwrap();
    assert_eq!(cut, before);
    assert!(
        fixture
            .journal
            .work_map_snapshot()
            .unwrap()
            .event_sequence()
            > before.event_sequence()
    );
    fixture
        .run(batch(vec![create(&ItemId::new())]), Mode::Write)
        .unwrap();
    let cursor: ItemCursor = serde_json::from_value(first["next"].clone()).unwrap();
    let second = fixture.run(items(1, Some(cursor)), Mode::ReadOnly).unwrap();
    assert_eq!(second["snapshot"], first["snapshot"]);
    assert_eq!(second["truncated"], false);
    assert_eq!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .len(),
        3
    );
}

#[test]
fn unsafe_annotations_and_failed_batches_do_not_leave_partial_map_facts() {
    let mut fixture = Fixture::new();
    let safe = create(&ItemId::new());
    let impossible = intent(Mutation::AssignTag {
        tag_id: TagId::new(),
        target: TagTarget::Item(ItemId::new()),
    });
    assert_eq!(
        fixture
            .run(batch(vec![safe.clone(), impossible]), Mode::Write)
            .unwrap_err(),
        Error::OutcomeUnknown
    );
    assert!(
        fixture
            .journal
            .get_work_change(&safe.change_id, None)
            .unwrap()
            .is_none()
    );
    let mut unsafe_value = serde_json::to_value(create(&ItemId::new())).unwrap();
    unsafe_value["mutation"]["kind"] = json!("token=NEVER-ECHO-THIS");
    let unsafe_intent: Intent = serde_json::from_value(unsafe_value).unwrap();
    assert_eq!(
        fixture
            .run(batch(vec![unsafe_intent]), Mode::Write)
            .unwrap_err()
            .to_string(),
        "work: invalid or unsafe input"
    );
    assert!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn concurrent_identical_intents_commit_once_and_keep_separate_attempts() {
    let fixture = Fixture::new();
    let path = fixture.directory.join("actions.sqlite3");
    let request = create(&ItemId::new());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let request = request.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let mut journal = Journal::open(&path).unwrap();
                let session = journal.start_session_with(SessionOrigin::Cli).unwrap();
                barrier.wait();
                let reply = execute(
                    &mut journal,
                    &session,
                    batch(vec![request]),
                    Mode::Write,
                    100,
                )
                .result
                .unwrap();
                journal.finish_session(&session).unwrap();
                serde_json::from_slice::<Value>(&reply.bytes).unwrap()
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        results[0]["changes"][0]["sequence"],
        results[1]["changes"][0]["sequence"]
    );
    assert_ne!(
        results[0]["attempt"]["action_id"],
        results[1]["attempt"]["action_id"]
    );
    assert_eq!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[tokio::test]
async fn ordered_writer_acknowledges_commit_and_rejects_commands_after_close() {
    let directory =
        std::env::temp_dir().join(format!("radar-work-writer-{}", ChangeId::new().as_str()));
    let path = directory.join("actions.sqlite3");
    let mut recording =
        crate::journal_runtime::Recording::open_at(&path, SessionOrigin::Cli).unwrap();
    let request = create(&ItemId::new());
    let change = request.change_id.clone();
    recording
        .sink
        .command(batch(vec![request.clone()]), Mode::Write)
        .await
        .unwrap();
    let journal = Journal::open(&path).unwrap();
    assert!(journal.get_work_change(&change, None).unwrap().is_some());
    recording.finish();
    assert!(matches!(
        recording
            .sink
            .command(batch(vec![request]), Mode::Write)
            .await,
        Err(Error::Unavailable)
    ));
    drop(journal);
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn stale_preparation_recovers_equivalent_winner_and_keeps_new_changes_atomic() {
    let mut fixture = Fixture::new();
    let shared = create(&ItemId::new());
    let unique = create(&ItemId::new());
    let path = fixture.directory.join("actions.sqlite3");
    let winner = shared.clone();
    runtime::before_next_apply(move |_| {
        let mut other = Journal::open(&path).unwrap();
        let session = other.start_session_with(SessionOrigin::Cli).unwrap();
        execute(&mut other, &session, batch(vec![winner]), Mode::Write, 101)
            .result
            .unwrap();
        other.finish_session(&session).unwrap();
    });
    let result = fixture
        .run(batch(vec![shared.clone(), unique.clone()]), Mode::Write)
        .unwrap();
    assert_eq!(result["changes"][0]["replayed"], true);
    assert_eq!(result["changes"][1]["replayed"], false);
    assert!(
        fixture
            .journal
            .get_work_change(&unique.change_id, None)
            .unwrap()
            .is_some()
    );
    assert_eq!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .len(),
        2
    );
    let original = fixture
        .journal
        .get_work_change(&shared.change_id, None)
        .unwrap()
        .unwrap();
    assert_ne!(
        original.recording.action_id.as_str(),
        result["attempt"]["action_id"].as_str().unwrap()
    );
}

#[test]
fn stale_preparation_rejects_different_winner_without_committing_batch_siblings() {
    let mut fixture = Fixture::new();
    let shared = create(&ItemId::new());
    let unique = create(&ItemId::new());
    let mut winner = shared.clone();
    winner.mutation = Mutation::CreateItem {
        item_id: ItemId::new(),
        kind: StableCode::new("card"),
    };
    let path = fixture.directory.join("actions.sqlite3");
    runtime::before_next_apply(move |_| {
        let mut other = Journal::open(&path).unwrap();
        let session = other.start_session_with(SessionOrigin::Cli).unwrap();
        execute(&mut other, &session, batch(vec![winner]), Mode::Write, 101)
            .result
            .unwrap();
        other.finish_session(&session).unwrap();
    });
    assert_eq!(
        fixture
            .run(batch(vec![unique.clone(), shared]), Mode::Write)
            .unwrap_err(),
        Error::Conflict
    );
    assert!(
        fixture
            .journal
            .get_work_change(&unique.change_id, None)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .len(),
        1
    );
}

#[test]
fn failed_terminal_audit_survives_both_blocked_and_committed_results() {
    for mode in [Mode::ReadOnly, Mode::Write] {
        let mut fixture = Fixture::new();
        let fault = rusqlite::Connection::open(fixture.directory.join("actions.sqlite3")).unwrap();
        fault.execute_batch("CREATE TRIGGER fail_terminal BEFORE INSERT ON action_events WHEN NEW.outcome IN ('blocked','succeeded') BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        let request = create(&ItemId::new());
        let execution = execute(
            &mut fixture.journal,
            &fixture.session,
            batch(vec![request.clone()]),
            mode,
            100,
        );
        assert!(execution.recording_degraded);
        if mode == Mode::ReadOnly {
            assert!(matches!(execution.result, Err(Error::PermissionDenied)));
            assert!(
                fixture
                    .journal
                    .get_work_change(&request.change_id, None)
                    .unwrap()
                    .is_none()
            );
        } else {
            let reply = execution.result.unwrap();
            let receipt: Value = serde_json::from_slice(&reply.bytes).unwrap();
            assert_eq!(receipt["recording_degraded"], true);
            assert!(
                fixture
                    .journal
                    .get_work_change(&request.change_id, None)
                    .unwrap()
                    .is_some()
            );
        }
    }
}

#[tokio::test]
async fn dropped_acknowledgement_is_recovered_with_the_same_identity_after_reopen() {
    let directory = std::env::temp_dir().join(format!("radar-work-lost-{}", ChangeId::new().as_str()));
    let path = directory.join("actions.sqlite3");
    let mut recording =
        crate::journal_runtime::Recording::open_at(&path, SessionOrigin::Cli).unwrap();
    let request = create(&ItemId::new());
    let first = recording
        .sink
        .command(batch(vec![request.clone()]), Mode::Write)
        .await
        .unwrap();
    let original: Value = serde_json::from_slice(&first.bytes).unwrap();
    drop(first);
    recording.finish();
    let mut reopened =
        crate::journal_runtime::Recording::open_at(&path, SessionOrigin::Cli).unwrap();
    let retry = reopened
        .sink
        .command(batch(vec![request]), Mode::Write)
        .await
        .unwrap();
    let retry: Value = serde_json::from_slice(&retry.bytes).unwrap();
    assert_eq!(
        original["changes"][0]["sequence"],
        retry["changes"][0]["sequence"]
    );
    assert_eq!(retry["changes"][0]["replayed"], true);
    assert_ne!(
        original["attempt"]["action_id"],
        retry["attempt"]["action_id"]
    );
    reopened.finish();
    std::fs::remove_dir_all(directory).unwrap();
}

#[test]
fn environment_values_and_output_failures_have_safe_precise_classifications() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let invalid = OsString::from_vec(vec![0xff, b's', b'e', b'c', b'r', b'e', b't']);
    assert_eq!(
        environment_mode([(OsString::from("UNRELATED"), invalid.clone())]).unwrap(),
        None
    );
    assert_eq!(
        environment_mode([(OsString::from("RADAR_MODE"), invalid)]).unwrap_err(),
        Error::InvalidInput
    );
    assert_eq!(
        environment_mode([(OsString::from("RADAR_MODE"), OsString::from("invalid-secret"))])
            .unwrap_err(),
        Error::InvalidInput
    );
    struct Broken;
    impl std::io::Write for Broken {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("secret-output-detail"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert_eq!(
        write_response(Broken, b"{}", true).unwrap_err(),
        Error::CommittedReceiptUnavailable
    );
    assert_eq!(
        write_response(Broken, b"{}", false).unwrap_err(),
        Error::OutputUnavailable
    );
    assert!(
        !write_response(Broken, b"{}", true)
            .unwrap_err()
            .to_string()
            .contains("secret-output-detail")
    );
    struct BufferedFailure;
    impl std::io::Write for BufferedFailure {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Err(std::io::Error::other("private-flush-detail"))
        }
    }
    assert_eq!(
        write_response(BufferedFailure, b"{}", true).unwrap_err(),
        Error::CommittedReceiptUnavailable
    );
}

fn events(filter: EventFilter, records: usize, cursor: Option<EventCursor>) -> Command {
    Command::Events(Box::new(EventsQuery {
        version: 1,
        filter,
        records,
        bytes: 262_144,
        cursor,
    }))
}

#[test]
fn events_input_is_strict_and_requires_explicit_filter_fields() {
    let valid = serde_json::to_value(EventsQuery {
        version: 1,
        filter: EventFilter::default(),
        records: 10,
        bytes: 4096,
        cursor: None,
    })
    .unwrap();
    assert!(parse("events", valid.to_string().as_bytes()).is_ok());
    for mutation in ["omit", "extra", "version", "outcome", "subject"] {
        let mut invalid = valid.clone();
        match mutation {
            "omit" => {
                invalid["filter"].as_object_mut().unwrap().remove("outcome");
            }
            "extra" => {
                invalid["filter"]["private-field"] = json!("private-content");
            }
            "version" => {
                invalid["version"] = json!(2);
            }
            "outcome" => {
                invalid["filter"]["outcome"] = json!("complete");
            }
            _ => {
                invalid["filter"]["subject"] = json!({"provider":"sample","instance":"one","kind":"card","id":"42","private-field":"private-content"});
            }
        }
        let error = parse("events", invalid.to_string().as_bytes()).unwrap_err();
        assert_eq!(error.to_string(), "work: invalid or unsafe input");
    }
}

#[test]
fn events_discover_unbound_source_for_an_explicit_observed_binding_and_handoff() {
    use crate::journal::{SubjectRole, SubjectSnapshot};
    let mut fixture = Fixture::new();
    let subject = SubjectKey::new("sample", "https://one.example.test", "card", "42").unwrap();
    let action = fixture
        .journal
        .begin(
            &fixture.session,
            ActionDescriptor::new("card.inspect", Origin::Plugin("sample".into()))
                .with_subject(SubjectSnapshot::new(subject.clone(), SubjectRole::Primary)),
            None,
        )
        .unwrap();
    let outcome = fixture
        .journal
        .record(
            &action,
            Outcome::Partial,
            crate::journal::Metadata {
                truncated: true,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(
        fixture
            .journal
            .resolve_subject(&subject, &action, None)
            .unwrap()
            .state,
        crate::journal::work_map::ResolutionState::Unbound
    );
    let before = fixture.journal.work_map_snapshot().unwrap();
    let found = fixture
        .run(
            events(
                EventFilter {
                    subject: Some(subject.clone()),
                    outcome: Some(Outcome::Partial),
                    ..EventFilter::default()
                },
                10,
                None,
            ),
            Mode::ReadOnly,
        )
        .unwrap();
    assert_eq!(found["snapshot"], serde_json::to_value(&before).unwrap());
    assert_eq!(found["events"].as_array().unwrap().len(), 1);
    let source = &found["events"][0];
    assert_eq!(source["event"]["event_id"], outcome.as_str());
    assert_eq!(source["event"]["action_id"], action.as_str());
    assert_eq!(source["event"]["metadata"]["truncated"], true);
    let requested = source["requested_sequence"].as_i64().unwrap();
    assert!(requested < source["event"]["sequence"].as_i64().unwrap());
    assert!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .is_empty()
    );
    let item = ItemId::new();
    let binding = Intent {
        change_id: ChangeId::new(),
        basis: Basis::Observed,
        evidence: Evidence::Event {
            event_id: serde_json::from_value(source["event"]["event_id"].clone()).unwrap(),
        },
        mutation: Mutation::BindSubject {
            item_id: item.clone(),
            subject: subject.clone(),
            from_sequence: requested,
            through_sequence: Some(requested),
        },
    };
    fixture
        .run(batch(vec![create(&item), binding]), Mode::Write)
        .unwrap();
    let resolved = fixture
        .journal
        .resolve_subject(&subject, &action, None)
        .unwrap();
    assert_eq!(resolved.items, vec![item.clone()]);
    let handoff = fixture
        .run(
            Command::Handoff(HandoffQuery {
                version: 1,
                selection: Selection {
                    items: vec![item],
                    actions: vec![],
                },
                records: 100,
                bytes: 262_144,
                cursor: None,
            }),
            Mode::ReadOnly,
        )
        .unwrap();
    let original = handoff["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| {
            record["type"] == "action" && record["event"]["event_id"] == outcome.as_str()
        })
        .unwrap();
    assert_eq!(original["event"], source["event"]);
    assert_eq!(original["association"], "explicit_binding");
    assert_eq!(handoff["execution_authorization"], "unspecified");
    let audits = fixture
        .journal
        .query(&Query {
            kind: Some(StableCode::new("work.events")),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        audits.iter().map(|event| event.outcome).collect::<Vec<_>>(),
        vec![Outcome::Requested, Outcome::Started, Outcome::Succeeded]
    );
    assert!(audits.iter().all(|event| event.subjects.is_empty()));
}

#[test]
fn events_continuations_exclude_audits_and_later_gaps_and_do_not_change_map_facts() {
    let mut fixture = Fixture::new();
    fixture
        .run(batch(vec![create(&ItemId::new())]), Mode::Write)
        .unwrap();
    let before = fixture.journal.work_map_snapshot().unwrap();
    let first = fixture
        .run(events(EventFilter::default(), 1, None), Mode::ReadOnly)
        .unwrap();
    assert_eq!(first["snapshot"], serde_json::to_value(&before).unwrap());
    fixture
        .journal
        .record_gap(&fixture.session, 1, 0, 100)
        .unwrap();
    let mut count = first["events"].as_array().unwrap().len();
    let mut cursor = first["next"].clone();
    while !cursor.is_null() {
        let page = fixture
            .run(
                events(
                    EventFilter::default(),
                    1,
                    Some(serde_json::from_value(cursor).unwrap()),
                ),
                Mode::ReadOnly,
            )
            .unwrap();
        assert_eq!(page["snapshot"], first["snapshot"]);
        assert_eq!(page["known_recording_gaps"], false);
        assert!(
            page["events"]
                .as_array()
                .unwrap()
                .iter()
                .all(|entry| entry["event"]["kind"] != "work.events")
        );
        count += page["events"].as_array().unwrap().len();
        assert!(count <= 3);
        cursor = page["next"].clone();
    }
    assert_eq!(count, 3);
    assert_eq!(
        fixture.journal.work_map_snapshot().unwrap().map_sequence(),
        before.map_sequence()
    );
    let fresh = fixture
        .run(events(EventFilter::default(), 100, None), Mode::ReadOnly)
        .unwrap();
    assert_eq!(fresh["known_recording_gaps"], true);
}

#[test]
fn events_maximum_escaped_identity_continuations_fit_the_input_budget() {
    use crate::journal::{SubjectRole, SubjectSnapshot};
    let mut fixture = Fixture::new();
    let identity = format!("/{}", "\\".repeat(2047));
    let subject = SubjectKey::new("sample", &identity, "card", &identity).unwrap();
    for _ in 0..2 {
        fixture
            .journal
            .begin(
                &fixture.session,
                ActionDescriptor::new("card.inspect", Origin::Host)
                    .with_subject(SubjectSnapshot::new(subject.clone(), SubjectRole::Primary)),
                None,
            )
            .unwrap();
    }
    let mut query = EventsQuery {
        version: 1,
        filter: EventFilter {
            subject: Some(subject),
            ..EventFilter::default()
        },
        records: 1,
        bytes: 262_144,
        cursor: None,
    };
    let input = serde_json::to_vec(&query).unwrap();
    assert!(input.len() <= MAX_QUERY_BYTES);
    let first = fixture
        .run(parse("events", input.as_slice()).unwrap(), Mode::ReadOnly)
        .unwrap();
    query.cursor = Some(serde_json::from_value(first["next"].clone()).unwrap());
    let input = serde_json::to_vec(&query).unwrap();
    assert!(input.len() > MAX_QUERY_BYTES && input.len() <= MAX_EVENTS_QUERY_BYTES);
    let second = fixture
        .run(parse("events", input.as_slice()).unwrap(), Mode::ReadOnly)
        .unwrap();
    assert_eq!(second["snapshot"], first["snapshot"]);
    assert_eq!(second["events"].as_array().unwrap().len(), 1);
    assert!(second["next"].is_null());
    assert!(parse("events", " ".repeat(MAX_EVENTS_QUERY_BYTES + 1).as_bytes()).is_err());
}
