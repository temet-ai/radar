use super::*;
use crate::journal::{SubjectKey, SubjectRole, SubjectSnapshot};
use std::path::PathBuf;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("radar-history-{}", ActionId::new().as_str())))
    }

    fn path(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }

    fn recording(&self) -> Recording {
        Recording::open_at(&self.path(), SessionOrigin::Tui).unwrap()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // Only this uniquely named test fixture is removed.
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn bounded(page: &EventPage) {
    assert!(page.events.len() <= 50);
    assert!(serde_json::to_vec(page).unwrap().len() <= 262_144);
}

fn isolated_sink(capacity: usize) -> (Sink, mpsc::Receiver<Work>) {
    let (sender, receiver) = mpsc::sync_channel(capacity);
    (
        Sink {
            sender,
            warning: Arc::new(Mutex::new(None)),
            dropped: Arc::new(AtomicU64::new(0)),
            accepting: Arc::new(Mutex::new(true)),
        },
        receiver,
    )
}

#[tokio::test]
async fn history_pages_keep_their_snapshot_when_new_writes_arrive() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let expected: Vec<_> = (0..101)
        .map(|_| recording.sink.begin("test.observe", Origin::Host, None))
        .collect();
    let first = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    bounded(&first);
    assert_eq!(first.events.len(), 50);
    assert!(first.truncated);
    let snapshot = first.snapshot.clone();
    let late = recording.sink.begin("test.later", Origin::Host, None);
    let second = recording
        .sink
        .history(EventFilter::default(), first.next.clone(), None)
        .await
        .unwrap();
    bounded(&second);
    assert_eq!(second.events.len(), 50);
    assert_eq!(second.snapshot, snapshot);
    assert!(second.truncated);
    let third = recording
        .sink
        .history(EventFilter::default(), second.next.clone(), None)
        .await
        .unwrap();
    bounded(&third);
    assert_eq!(third.events.len(), 1);
    assert_eq!(third.snapshot, snapshot);
    assert!(!third.truncated);
    assert!(third.next.is_none());
    let found: Vec<_> = first
        .events
        .iter()
        .chain(&second.events)
        .chain(&third.events)
        .map(|entry| entry.event.action_id.clone())
        .collect();
    assert_eq!(found, expected);
    assert!(!found.contains(&late));
    let revisited = recording
        .sink
        .history(EventFilter::default(), None, Some(snapshot.clone()))
        .await
        .unwrap();
    bounded(&revisited);
    assert_eq!(revisited.snapshot, snapshot);
    assert_eq!(
        serde_json::to_value(&revisited).unwrap(),
        serde_json::to_value(&first).unwrap()
    );
    let refreshed = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(
        refreshed.snapshot.event_sequence(),
        snapshot.event_sequence() + 1
    );
    assert!(recording.sink.take_warning().is_none());
}

#[tokio::test]
async fn history_byte_budget_can_end_a_page_before_the_record_limit() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let mut descriptor = ActionDescriptor::new("test.large", Origin::Host);
    for index in 0..32 {
        descriptor.subjects.push(SubjectSnapshot::new(
            SubjectKey::new(
                "sample",
                "https://acme.example.test",
                "item",
                &format!("item-{index}-{}", "a".repeat(1000)),
            )
            .unwrap(),
            SubjectRole::Context,
        ));
    }
    let expected: Vec<_> = (0..12)
        .map(|_| recording.sink.begin_descriptor(descriptor.clone(), None))
        .collect();
    let mut page = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    assert!(!page.events.is_empty());
    assert!(page.events.len() < expected.len());
    assert!(page.truncated);
    let snapshot = page.snapshot.clone();
    let mut found = Vec::new();
    loop {
        bounded(&page);
        assert_eq!(page.snapshot, snapshot);
        found.extend(
            page.events
                .iter()
                .map(|entry| entry.event.action_id.clone()),
        );
        let Some(cursor) = page.next else {
            break;
        };
        page = recording
            .sink
            .history(EventFilter::default(), Some(cursor), None)
            .await
            .unwrap();
    }
    assert_eq!(found, expected);
}

#[tokio::test]
async fn history_sees_queued_outcomes_and_preserves_the_exact_filter() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let action = recording.sink.begin("test.observe", Origin::Host, None);
    recording.sink.outcome(&action, Outcome::Succeeded, None);
    recording.sink.begin("test.other", Origin::Host, None);
    let filter = EventFilter {
        action_id: Some(action.clone()),
        ..Default::default()
    };
    let page = recording
        .sink
        .history(filter.clone(), None, None)
        .await
        .unwrap();
    assert_eq!(page.filter, filter);
    assert_eq!(page.events.len(), 2);
    assert_eq!(page.events[0].event.action_id, action);
    assert_eq!(page.events[0].event.outcome, Outcome::Requested);
    assert_eq!(page.events[1].event.action_id, action);
    assert_eq!(page.events[1].event.outcome, Outcome::Succeeded);
    assert_eq!(
        page.events[0].requested_sequence,
        page.events[1].requested_sequence
    );
    let all = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(all.events.len(), 3);
    assert_eq!(all.snapshot, page.snapshot);
    assert!(
        all.events
            .iter()
            .all(|entry| entry.event.origin == Origin::Host)
    );
}

#[tokio::test]
async fn history_reads_neither_audit_themselves_nor_flush_pending_gaps() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let action = recording.sink.begin("test.observe", Origin::Host, None);
    recording.sink.outcome(&action, Outcome::Succeeded, None);
    let before = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    recording.sink.submit(JournalRecord::Gap { dropped: 3 });
    assert!(recording.sink.take_warning().is_some());
    for _ in 0..3 {
        let page = recording
            .sink
            .history(EventFilter::default(), None, None)
            .await
            .unwrap();
        assert_eq!(page.snapshot, before.snapshot);
        assert_eq!(page.events.len(), 2);
        assert!(!page.known_recording_gaps);
    }
    assert_eq!(recording.sink.dropped.load(Ordering::Relaxed), 3);
    let later = recording.sink.begin("test.later", Origin::Host, None);
    recording.sink.outcome(&later, Outcome::Succeeded, None);
    let after = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(after.events.len(), 4);
    assert!(after.known_recording_gaps);
    assert_eq!(
        after.snapshot.gap_sequence(),
        before.snapshot.gap_sequence() + 1
    );
    assert_eq!(recording.sink.dropped.load(Ordering::Relaxed), 0);
    assert!(recording.sink.take_warning().is_none());
}

#[tokio::test]
async fn history_storage_failures_are_safe_and_do_not_count_as_failed_writes() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let action = recording.sink.begin("test.observe", Origin::Host, None);
    recording.sink.outcome(&action, Outcome::Succeeded, None);
    let before = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    let connection = rusqlite::Connection::open(temp.path()).unwrap();
    connection
        .execute_batch("ALTER TABLE recording_gaps RENAME TO test_hidden_gaps")
        .unwrap();
    let error = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap_err();
    assert!(matches!(error, ReadError::Unavailable));
    assert_eq!(format!("{error:?}"), "Unavailable");
    assert!(recording.sink.take_warning().is_none());
    connection
        .execute_batch("ALTER TABLE test_hidden_gaps RENAME TO recording_gaps")
        .unwrap();
    let after = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(after.snapshot, before.snapshot);
    assert_eq!(after.events.len(), before.events.len());
    let later = recording.sink.begin("test.later", Origin::Host, None);
    recording.sink.outcome(&later, Outcome::Succeeded, None);
    let later = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(
        later.snapshot.gap_sequence(),
        before.snapshot.gap_sequence()
    );
    assert!(!later.known_recording_gaps);
    assert!(recording.sink.take_warning().is_none());
}

#[tokio::test]
async fn history_full_or_disconnected_queue_is_unavailable_without_lost_records() {
    let (sink, receiver) = isolated_sink(1);
    assert!(sink.sender.try_send(Work::Finish).is_ok());
    assert!(matches!(
        sink.history(EventFilter::default(), None, None).await,
        Err(ReadError::Unavailable)
    ));
    drop(receiver);
    assert!(matches!(
        sink.history(EventFilter::default(), None, None).await,
        Err(ReadError::Unavailable)
    ));
    assert_eq!(sink.dropped.load(Ordering::Relaxed), 0);
    assert!(sink.take_warning().is_none());
}

#[tokio::test]
async fn history_lost_reply_is_unavailable_without_lost_records() {
    let (sink, receiver) = isolated_sink(1);
    let worker = std::thread::spawn(move || {
        let work = receiver.recv().unwrap();
        assert!(matches!(work, Work::History { .. }));
        drop(work);
    });
    assert!(matches!(
        sink.history(EventFilter::default(), None, None).await,
        Err(ReadError::Unavailable)
    ));
    worker.join().unwrap();
    assert_eq!(sink.dropped.load(Ordering::Relaxed), 0);
    assert!(sink.take_warning().is_none());
}

#[tokio::test]
async fn history_cannot_enqueue_after_the_writer_is_sealed() {
    let temp = TestDir::new();
    let mut recording = temp.recording();
    recording.finish();
    assert!(matches!(
        recording
            .sink
            .history(EventFilter::default(), None, None)
            .await,
        Err(ReadError::Unavailable)
    ));
    assert_eq!(recording.sink.dropped.load(Ordering::Relaxed), 0);
    assert!(recording.sink.take_warning().is_none());
}
