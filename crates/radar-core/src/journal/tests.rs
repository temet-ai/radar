use super::*;
use std::os::unix::fs::{PermissionsExt, symlink};

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("radar-journal-{}", Uuid::new_v4())))
    }

    fn path(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }

    fn open(&self) -> Journal {
        Journal::open(&self.path()).unwrap()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // Only this unique test fixture is removed, never a user's history.
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn descriptor() -> ActionDescriptor {
    ActionDescriptor::new("item.open", Origin::Host)
}

fn subject(instance: &str) -> SubjectSnapshot {
    SubjectSnapshot::new(
        SubjectKey::new("provider", instance, "item", "42").unwrap(),
        SubjectRole::Primary,
    )
}

#[test]
fn restart_keeps_ordered_events_and_typed_outcomes() {
    let temp = TestDir::new();
    let (session, action) = {
        let mut journal = temp.open();
        let session = journal.start_session().unwrap();
        let action = journal.begin(&session, descriptor(), None).unwrap();
        journal
            .record(&action, Outcome::Confirmed, Metadata::default())
            .unwrap();
        journal
            .record(&action, Outcome::Started, Metadata::default())
            .unwrap();
        journal
            .record(&action, Outcome::Succeeded, Metadata::default())
            .unwrap();
        journal.finish_session(&session).unwrap();
        journal.flush().unwrap();
        (session, action)
    };
    let journal = temp.open();
    let events = journal
        .query(&Query {
            action_id: Some(action),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(
        events.iter().map(|event| event.outcome).collect::<Vec<_>>(),
        [
            Outcome::Requested,
            Outcome::Confirmed,
            Outcome::Started,
            Outcome::Succeeded
        ]
    );
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    assert!(
        events
            .iter()
            .all(|event| event.session_id == session && event.occurred_at_ms > 0)
    );
    assert_ne!(events[0].event_id, events[1].event_id);
}

#[test]
fn caller_ids_inherit_parent_root_and_correlation() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    let root = ActionId::new();
    journal
        .begin_with_id(&session, &root, descriptor(), None)
        .unwrap();
    let child = journal.begin(&session, descriptor(), Some(&root)).unwrap();
    let leaf = journal.begin(&session, descriptor(), Some(&child)).unwrap();
    let events = journal
        .query(&Query {
            root_action_id: Some(root.clone()),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].action_id, leaf);
    assert_eq!(events[2].parent_action_id, Some(child));
    assert!(
        events
            .iter()
            .all(|event| event.root_action_id == root && event.correlation_id == root)
    );
    let other_session = journal.start_session().unwrap();
    assert!(
        journal
            .begin(&other_session, descriptor(), Some(&root))
            .is_err()
    );
    assert!(
        journal
            .begin(&session, descriptor(), Some(&ActionId::new()))
            .is_err()
    );
    assert_eq!(journal.query(&Query::default()).unwrap().len(), 3);
}

#[test]
fn scoped_subjects_do_not_collide_and_snapshots_do_not_change() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    let first = subject("https://one.example.test");
    let second = subject("https://two.example.test");
    journal
        .begin(
            &session,
            descriptor().with_subject(first.clone().with_url(
                SafeUrl::parse("https://one.example.test/item/42?secret=hidden").unwrap(),
            )),
            None,
        )
        .unwrap();
    journal
        .begin(&session, descriptor().with_subject(second.clone()), None)
        .unwrap();
    let third = journal
        .begin(
            &session,
            descriptor().with_subject(
                first
                    .clone()
                    .with_url(SafeUrl::parse("https://one.example.test/moved/42").unwrap()),
            ),
            None,
        )
        .unwrap();
    let events = journal
        .query(&Query {
            subject: Some(first.key),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].subjects[0].url.as_ref().unwrap().as_str(),
        "https://one.example.test/item/42"
    );
    assert_eq!(
        events[1].subjects[0].url.as_ref().unwrap().as_str(),
        "https://one.example.test/moved/42"
    );
    assert_eq!(events[1].action_id, third);
    assert_eq!(
        journal
            .query(&Query {
                subject: Some(second.key),
                ..Query::default()
            })
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn metadata_strips_credentials_and_rejects_unknown_fields_and_limits() {
    let safe =
        SafeUrl::parse("https://alice:private@example.test/item/42?token=private#private").unwrap();
    assert_eq!(safe.as_str(), "https://example.test/item/42");
    assert!(SafeUrl::parse("data:private").is_err());
    assert!(SafeUrl::parse(&format!("https://example.test/{}", "x".repeat(9000))).is_err());
    assert!(SafeUrl::parse("https://example.test/token%253Dprivate").is_err());
    assert!(serde_json::from_str::<Metadata>(r#"{"clipboard":"private"}"#).is_err());
    assert!(
        Metadata {
            duration_ms: Some(u64::MAX),
            ..Metadata::default()
        }
        .json()
        .is_err()
    );
    assert!(
        Metadata {
            command: Some(StableCode::new("user typed text")),
            ..Metadata::default()
        }
        .json()
        .is_err()
    );
    // Even a deserialized wrapper is sanitized again at the storage boundary.
    let unsafe_url: SafeUrl =
        serde_json::from_str(r#""https://alice:private@example.test/42?token=private#private""#)
            .unwrap();
    let json = Metadata {
        url: Some(unsafe_url),
        ..Metadata::default()
    }
    .json()
    .unwrap();
    assert!(!json.contains("private") && !json.contains("alice"));
}

#[test]
fn raw_private_values_do_not_reach_database_or_wal() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    let url = SafeUrl::parse("https://example.test/item?token=NEVER_STORE_THIS#NEVER_STORE_THIS")
        .unwrap();
    journal
        .begin(
            &session,
            descriptor().with_metadata(Metadata {
                url: Some(url),
                ..Metadata::default()
            }),
            None,
        )
        .unwrap();
    journal.flush().unwrap();
    for suffix in ["", "-wal"] {
        let path = PathBuf::from(format!("{}{suffix}", temp.path().display()));
        if let Ok(bytes) = fs::read(path) {
            assert!(
                !bytes
                    .windows(b"NEVER_STORE_THIS".len())
                    .any(|part| part == b"NEVER_STORE_THIS")
            );
        }
    }
}

#[test]
fn validates_vocabulary_identity_and_subject_limits_before_any_write() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    assert!(
        journal
            .begin(
                &session,
                ActionDescriptor::new("unbounded private text", Origin::Host),
                None
            )
            .is_err()
    );
    assert!(
        journal
            .begin(
                &session,
                ActionDescriptor::new("item.open", Origin::Plugin("a?secret=b".into())),
                None
            )
            .is_err()
    );
    let mut descriptor = descriptor();
    descriptor.subjects = vec![subject("account"); MAX_SUBJECTS + 1];
    assert!(journal.begin(&session, descriptor, None).is_err());
    assert!(SubjectKey::new("provider", "account?token=private", "item", "42").is_err());
    assert!(
        SubjectKey::new(
            "provider",
            "/home/Some User/project",
            "checkout",
            "/home/Some User/project"
        )
        .is_ok()
    );
    assert!(SubjectKey::new("provider", "account", "item", "42!7").is_ok());
    assert!(journal.query(&Query::default()).unwrap().is_empty());
}

#[test]
fn shutdown_only_finishes_its_explicit_session() {
    let temp = TestDir::new();
    let mut first = temp.open();
    let first_session = first.start_session().unwrap();
    let action = first.begin(&first_session, descriptor(), None).unwrap();
    first
        .record(&action, Outcome::Started, Metadata::default())
        .unwrap();
    let mut second = temp.open();
    assert_eq!(second.query(&Query::default()).unwrap().len(), 2);
    let second_session = second.start_session().unwrap();
    second.begin(&second_session, descriptor(), None).unwrap();
    second.finish_session(&second_session).unwrap();
    let first_events = first
        .query(&Query {
            session_id: Some(first_session.clone()),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(first_events.last().unwrap().outcome, Outcome::Started);
    first.finish_session(&first_session).unwrap();
    let events = first
        .query(&Query {
            action_id: Some(action),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(events.last().unwrap().outcome, Outcome::Unknown);
    assert_eq!(
        events.last().unwrap().metadata.reason,
        Some(Reason::NoTerminalObservation)
    );
    assert!(first.begin(&first_session, descriptor(), None).is_err());
}

#[test]
fn all_terminal_outcomes_are_explicit_and_cannot_be_rewritten() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    for outcome in [
        Outcome::Succeeded,
        Outcome::Failed,
        Outcome::Cancelled,
        Outcome::Blocked,
        Outcome::Partial,
        Outcome::Unknown,
    ] {
        let action = journal.begin(&session, descriptor(), None).unwrap();
        let event = journal
            .record(&action, outcome, Metadata::default())
            .unwrap();
        assert_eq!(
            journal
                .record(&action, outcome, Metadata::default())
                .unwrap(),
            event
        );
        let conflicting = if outcome == Outcome::Succeeded {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        };
        assert!(
            journal
                .record(&action, conflicting, Metadata::default())
                .is_err()
        );
    }
    assert_eq!(journal.query(&Query::default()).unwrap().len(), 12);
    let action = journal.begin(&session, descriptor(), None).unwrap();
    assert!(
        journal
            .record(&action, Outcome::Requested, Metadata::default())
            .is_err()
    );
    let started = journal
        .record(&action, Outcome::Started, Metadata::default())
        .unwrap();
    assert_eq!(
        journal
            .record(&action, Outcome::Started, Metadata::default())
            .unwrap(),
        started
    );
    assert!(
        journal
            .record(&action, Outcome::Confirmed, Metadata::default())
            .is_err()
    );
}

#[test]
fn migrates_existing_version_one_without_losing_history() {
    let temp = TestDir::new();
    private_directory(&temp.0).unwrap();
    let connection = Connection::open(temp.path()).unwrap();
    connection.execute_batch(schema::V1).unwrap();
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    connection.pragma_update(None, "user_version", 1).unwrap();
    connection
        .execute("INSERT INTO sessions VALUES ('existing',1,NULL,1)", [])
        .unwrap();
    drop(connection);
    let mut journal = temp.open();
    let version: u32 = journal
        .connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION);
    let old: i64 = journal
        .connection
        .query_row(
            "SELECT count(*) FROM sessions WHERE id='existing'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(old, 1);
    let session = journal.start_session().unwrap();
    journal
        .begin(
            &session,
            descriptor().with_subject(subject("account")),
            None,
        )
        .unwrap();
}

#[test]
fn refuses_newer_schema_without_downgrade() {
    let temp = TestDir::new();
    let journal = temp.open();
    journal
        .connection
        .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
        .unwrap();
    drop(journal);
    assert!(Journal::open(&temp.path()).is_err());
    let connection = Connection::open(temp.path()).unwrap();
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap();
    assert_eq!(version, SCHEMA_VERSION + 1);
}

#[test]
fn refuses_invalid_and_unrelated_databases_without_replacing_them() {
    let temp = TestDir::new();
    fs::create_dir_all(&temp.0).unwrap();
    fs::write(temp.path(), b"invalid database bytes").unwrap();
    assert!(Journal::open(&temp.path()).is_err());
    assert_eq!(fs::read(temp.path()).unwrap(), b"invalid database bytes");
    let other = TestDir::new();
    fs::create_dir_all(&other.0).unwrap();
    let connection = Connection::open(other.path()).unwrap();
    connection
        .execute("CREATE TABLE unrelated(value TEXT)", [])
        .unwrap();
    drop(connection);
    assert!(Journal::open(&other.path()).is_err());
    let connection = Connection::open(other.path()).unwrap();
    assert!(connection.prepare("SELECT value FROM unrelated").is_ok());
}

#[test]
fn database_enforces_foreign_keys_append_only_history_and_indexes() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    journal
        .begin(
            &session,
            descriptor().with_subject(subject("account")),
            None,
        )
        .unwrap();
    for sql in [
        "UPDATE action_events SET outcome='succeeded'",
        "DELETE FROM action_events",
        "UPDATE actions SET kind='other'",
        "DELETE FROM actions",
        "UPDATE action_subjects SET role='source'",
        "DELETE FROM action_subjects",
        "UPDATE subjects SET external_id='other'",
        "DELETE FROM subjects",
        "INSERT INTO action_events(id,action_id,occurred_at_ms,outcome,metadata_json) VALUES ('missing','missing',0,'requested','{}')",
    ] {
        assert!(journal.connection.execute(sql, []).is_err(), "{sql}");
    }
    for name in [
        "actions_session",
        "actions_kind",
        "actions_root",
        "actions_parent",
        "actions_correlation",
        "events_time",
        "events_action",
        "subject_actions",
    ] {
        let exists: bool = journal
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='index' AND name=?1)",
                [name],
                |row| row.get(0),
            )
            .unwrap();
        assert!(exists, "missing {name}");
    }
    assert!(
        journal
            .connection
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()
            .unwrap()
            .is_none()
    );
}

#[test]
fn directory_database_and_sidecars_are_private() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    journal.start_session().unwrap();
    assert_eq!(
        fs::metadata(&temp.0).unwrap().permissions().mode() & 0o777,
        0o700
    );
    for suffix in ["", "-wal", "-shm"] {
        let path = PathBuf::from(format!("{}{suffix}", temp.path().display()));
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

#[test]
fn rejects_symlinks_and_relative_paths() {
    let temp = TestDir::new();
    let target = TestDir::new();
    fs::create_dir_all(&temp.0).unwrap();
    fs::create_dir_all(&target.0).unwrap();
    fs::write(target.path(), b"keep").unwrap();
    symlink(target.path(), temp.path()).unwrap();
    assert!(Journal::open(&temp.path()).is_err());
    assert_eq!(fs::read(target.path()).unwrap(), b"keep");
    assert!(Journal::open(Path::new("actions.sqlite3")).is_err());
    let directory_link = TestDir::new();
    symlink(&target.0, &directory_link.0).unwrap();
    assert!(Journal::open(&directory_link.path()).is_err());
    // Avoid recursively removing the link's target in the test cleanup.
    fs::remove_file(&directory_link.0).unwrap();
}

#[test]
fn default_path_never_falls_back_to_the_current_directory() {
    assert!(Journal::path_from(None, None).is_err());
    assert!(
        Journal::path_from(
            Some(std::ffi::OsStr::new("relative")),
            Some(Path::new("/home/user"))
        )
        .is_err()
    );
    assert_eq!(
        Journal::path_from(Some(std::ffi::OsStr::new("/state")), None).unwrap(),
        Path::new("/state/radar/journal/actions.sqlite3")
    );
    assert_eq!(
        Journal::path_from(None, Some(Path::new("/home/user"))).unwrap(),
        Path::new("/home/user/.local/state/radar/journal/actions.sqlite3")
    );
}

#[test]
fn query_filters_and_sequence_pagination_are_bounded() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    for _ in 0..3 {
        journal.begin(&session, descriptor(), None).unwrap();
    }
    journal
        .begin(
            &session,
            ActionDescriptor::new("item.close", Origin::Host),
            None,
        )
        .unwrap();
    let first = journal
        .query(&Query {
            kind: Some(StableCode::new("item.open")),
            limit: 2,
            ..Query::default()
        })
        .unwrap();
    assert_eq!(first.len(), 2);
    let second = journal
        .query(&Query {
            kind: Some(StableCode::new("item.open")),
            after_sequence: Some(first[1].sequence),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(second.len(), 1);
    assert!(
        journal
            .query(&Query {
                since_ms: Some(i64::MAX),
                ..Query::default()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        journal
            .query(&Query {
                until_ms: Some(0),
                ..Query::default()
            })
            .unwrap()
            .is_empty()
    );
    assert!(
        journal
            .query(&Query {
                limit: 1001,
                ..Query::default()
            })
            .is_err()
    );
}

#[test]
fn session_provenance_and_event_versions_are_queryable() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session_with(SessionOrigin::Cli).unwrap();
    let action = journal
        .begin(
            &session,
            descriptor().with_metadata(Metadata {
                mode: Some(WriteMode::ReadOnly),
                redacted: true,
                omitted_subjects: 1,
                ..Metadata::default()
            }),
            None,
        )
        .unwrap();
    journal
        .record(&action, Outcome::Blocked, Metadata::default())
        .unwrap();
    let info = journal.session(&session).unwrap().unwrap();
    assert_eq!(info.origin, SessionOrigin::Cli);
    assert_eq!(info.build_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(info.event_version, EVENT_VERSION);
    assert!(info.ended_at_ms.is_none());
    let events = journal.query(&Query::default()).unwrap();
    assert!(
        events
            .iter()
            .all(|event| event.event_version == EVENT_VERSION)
    );
    assert_eq!(events[0].metadata.mode, Some(WriteMode::ReadOnly));
    assert!(events[0].metadata.redacted);
    assert_eq!(events[0].metadata.omitted_subjects, 1);
    journal.finish_session(&session).unwrap();
    assert!(
        journal
            .session(&session)
            .unwrap()
            .unwrap()
            .ended_at_ms
            .is_some()
    );
}

#[test]
fn rejects_damaged_schema_without_repairing_or_erasing_it() {
    let temp = TestDir::new();
    let journal = temp.open();
    journal
        .connection
        .execute_batch("DROP TRIGGER events_no_delete")
        .unwrap();
    drop(journal);
    assert!(Journal::open(&temp.path()).is_err());
    let connection = Connection::open(temp.path()).unwrap();
    let exists: bool = connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name='events_no_delete')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(!exists);
}

#[test]
fn unfinished_runtime_task_is_distinguished_from_an_unreported_intent() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    let action = journal
        .begin(
            &session,
            ActionDescriptor::new("runtime.task", Origin::Host),
            None,
        )
        .unwrap();
    journal
        .record(&action, Outcome::Started, Metadata::default())
        .unwrap();
    journal.finish_session(&session).unwrap();
    let events = journal
        .query(&Query {
            action_id: Some(action),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(events.last().unwrap().outcome, Outcome::Unknown);
    assert_eq!(
        events.last().unwrap().metadata.reason,
        Some(Reason::Interrupted)
    );
}

#[test]
fn caller_observation_time_survives_a_delayed_writer_and_clock_changes() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    let action = ActionId::new();
    let observed = 1_700_000_000_000;
    let before_write = now_ms().unwrap();
    journal
        .begin_with_id_at(&session, &action, descriptor(), None, observed)
        .unwrap();
    // Wall clocks can move backwards. Commit sequence remains authoritative.
    journal
        .record_at(
            &action,
            Outcome::Succeeded,
            Metadata::default(),
            observed - 100,
        )
        .unwrap();
    let events = journal
        .query(&Query {
            action_id: Some(action),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(events[0].observed_at_ms, observed);
    assert_eq!(events[1].observed_at_ms, observed - 100);
    assert!(
        events
            .iter()
            .all(|event| event.recorded_at_ms >= before_write)
    );
    assert!(events.iter().all(|event| !event.observation_time_estimated));
    assert!(events[0].sequence < events[1].sequence);
    let at_observation = journal
        .query(&Query {
            since_ms: Some(observed),
            until_ms: Some(observed),
            ..Query::default()
        })
        .unwrap();
    assert_eq!(at_observation.len(), 1);
    assert_eq!(at_observation[0].outcome, Outcome::Requested);
    for invalid in [-1, MAX_TIMESTAMP_MS + 1] {
        assert!(
            journal
                .begin_with_id_at(&session, &ActionId::new(), descriptor(), None, invalid)
                .is_err()
        );
    }
    assert_eq!(journal.query(&Query::default()).unwrap().len(), 2);
}

#[test]
fn gaps_survive_shutdown_and_reopening_even_without_action_records() {
    let temp = TestDir::new();
    let session = {
        let mut journal = temp.open();
        let session = journal.start_session().unwrap();
        journal.record_gap(&session, 3, 2, 1000).unwrap();
        journal.record_gap(&session, 0, 5, 2000).unwrap();
        assert!(journal.record_gap(&session, 0, 0, 3000).is_err());
        assert!(journal.record_gap(&session, u64::MAX, 0, 3000).is_err());
        journal.finish_session(&session).unwrap();
        journal.flush().unwrap();
        session
    };
    let journal = temp.open();
    let info = journal.session(&session).unwrap().unwrap();
    assert!(info.recording_degraded);
    assert_eq!(info.recording_gap_count, 2);
    assert_eq!(info.dropped_records, 3);
    assert_eq!(info.write_errors, 7);
    let first = journal.recording_gaps(&session, 0, 1).unwrap();
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].observed_at_ms, 1000);
    assert_eq!(first[0].dropped_records, 3);
    let next = journal
        .recording_gaps(&session, first[0].sequence, 100)
        .unwrap();
    assert_eq!(next.len(), 1);
    assert_eq!(next[0].write_errors, 5);
    assert!(journal.query(&Query::default()).unwrap().is_empty());
    assert!(
        journal
            .connection
            .execute("DELETE FROM recording_gaps", [])
            .is_err()
    );
    assert!(
        journal
            .connection
            .execute("UPDATE recording_gaps SET dropped_records=0", [])
            .is_err()
    );
}

#[test]
fn failed_gap_transaction_can_be_retried_without_partial_counts() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    journal.connection.execute_batch("CREATE TEMP TRIGGER fail_gap BEFORE UPDATE ON sessions BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
    assert!(journal.record_gap(&session, 4, 1, 1000).is_err());
    assert!(journal.recording_gaps(&session, 0, 100).unwrap().is_empty());
    assert!(
        !journal
            .session(&session)
            .unwrap()
            .unwrap()
            .recording_degraded
    );
    journal
        .connection
        .execute_batch("DROP TRIGGER fail_gap")
        .unwrap();
    journal.record_gap(&session, 4, 1, 1000).unwrap();
    let info = journal.session(&session).unwrap().unwrap();
    assert_eq!(info.recording_gap_count, 1);
    assert_eq!(info.dropped_records, 4);
    assert_eq!(info.write_errors, 1);
}

#[test]
fn old_writer_timestamps_are_preserved_and_explicitly_estimated() {
    let temp = TestDir::new();
    private_directory(&temp.0).unwrap();
    let connection = Connection::open(temp.path()).unwrap();
    connection.execute_batch(schema::V1).unwrap();
    connection.execute_batch(schema::V2).unwrap();
    connection.execute_batch(schema::V3).unwrap();
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    connection.pragma_update(None, "user_version", 3).unwrap();
    connection
        .execute(
            "INSERT INTO sessions(id,started_at_ms,process_id) VALUES ('existing',1000,1)",
            [],
        )
        .unwrap();
    connection.execute("INSERT INTO actions(id,session_id,root_action_id,correlation_id,kind,origin_kind,origin_id,created_at_ms) VALUES ('action','existing','action','action','item.open','host','host',1000)", []).unwrap();
    connection.execute("INSERT INTO action_events(id,action_id,occurred_at_ms,outcome,metadata_json) VALUES ('event','action',1000,'requested','{}')", []).unwrap();
    drop(connection);
    let journal = temp.open();
    let events = journal.query(&Query::default()).unwrap();
    assert_eq!(events[0].observed_at_ms, 1000);
    assert_eq!(events[0].recorded_at_ms, 1000);
    assert!(events[0].observation_time_estimated);
}

#[test]
fn shutdown_gap_can_be_recorded_after_session_close() {
    let temp = TestDir::new();
    let mut journal = temp.open();
    let session = journal.start_session().unwrap();
    journal.finish_session(&session).unwrap();
    journal.record_gap(&session, 0, 1, 1000).unwrap();
    let info = journal.session(&session).unwrap().unwrap();
    assert!(info.ended_at_ms.is_some());
    assert!(info.recording_degraded);
    assert_eq!(info.write_errors, 1);
}
