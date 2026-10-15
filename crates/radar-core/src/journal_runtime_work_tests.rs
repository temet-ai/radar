use super::*;
use crate::journal::work_map::{
    AssertionTarget, Basis, ChangeId, Handoff, HandoffRecord, ItemId, Mutation, RelationKind,
    ResolutionState, SemanticLabel, TagId, TagKey, TagTarget,
};
use crate::journal::{Query, StableCode, SubjectKey, SubjectRole, SubjectSnapshot};
use crate::work_api::{
    self, Batch, BatchState, DetailRequest, Error, Evidence, EvidenceRequest, Intent, ItemsRequest,
    Request, ResolveRequest, Response, TagsRequest,
};
use crate::writes::Mode;
use std::path::PathBuf;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("radar-work-ui-{}", ActionId::new().as_str())))
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
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn item(n: u64) -> ItemId {
    ItemId::parse(&format!("20000000-0000-4000-8000-{n:012x}")).unwrap()
}

fn tag(n: u64) -> TagId {
    TagId::parse(&format!("30000000-0000-4000-8000-{n:012x}")).unwrap()
}

fn change(n: u64) -> ChangeId {
    ChangeId::parse(&format!("40000000-0000-4000-8000-{n:012x}")).unwrap()
}

fn intent(n: u64, mutation: Mutation) -> Intent {
    Intent::asserted(change(n), mutation)
}

fn create(n: u64, kind: &'static str) -> Intent {
    intent(
        n,
        Mutation::CreateItem {
            item_id: item(n),
            kind: StableCode::new(kind),
        },
    )
}

fn task() -> Batch {
    Batch::new(vec![
        create(1, "task"),
        intent(
            2,
            Mutation::CreateTag {
                tag_id: tag(1),
                namespace: TagKey::new("work.title").unwrap(),
                key: TagKey::new(item(1).as_str()).unwrap(),
                label: SemanticLabel::new("Review explicitly linked work").unwrap(),
            },
        ),
        intent(
            3,
            Mutation::AssignTag {
                tag_id: tag(1),
                target: TagTarget::Item(item(1)),
            },
        ),
    ])
}

async fn apply(sink: &Sink, batch: Batch) -> work_api::ApplyReceipt {
    match sink.work(Request::Apply(batch), Mode::Write).await.unwrap() {
        Response::Applied(receipt) => receipt,
        other => panic!("unexpected {other:?}"),
    }
}

async fn inspect(sink: &Sink, batch: Batch) -> Result<BatchState, Error> {
    match sink
        .work(Request::InspectBatch(batch), Mode::ReadOnly)
        .await?
    {
        Response::BatchState(state) => Ok(state),
        other => panic!("unexpected {other:?}"),
    }
}

async fn items(sink: &Sink, request: ItemsRequest) -> work_api::ItemsResponse {
    match sink
        .work(Request::Items(request), Mode::ReadOnly)
        .await
        .unwrap()
    {
        Response::Items(page) => page,
        other => panic!("unexpected {other:?}"),
    }
}

async fn detail(
    sink: &Sink,
    item_id: ItemId,
    snapshot: Option<Snapshot>,
) -> work_api::DetailResponse {
    match sink
        .work(
            Request::Detail(DetailRequest { item_id, snapshot }),
            Mode::ReadOnly,
        )
        .await
        .unwrap()
    {
        Response::Detail(page) => page,
        other => panic!("unexpected {other:?}"),
    }
}

fn bounded(page: &Handoff) {
    assert!(page.records.len() <= work_api::EVIDENCE_LIMITS.records);
    assert!(serde_json::to_vec(page).unwrap().len() <= work_api::EVIDENCE_LIMITS.bytes);
    assert_eq!(page.authority, "observations_only");
    assert_eq!(page.execution_authorization, "unspecified");
    assert!(page.completeness_not_guaranteed);
}

#[tokio::test]
async fn work_inspect_reconciles_none_partial_and_all_without_writes_or_self_audit() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let batch = task();
    let before = items(&recording.sink, ItemsRequest::default())
        .await
        .page
        .snapshot;
    assert_eq!(
        inspect(&recording.sink, batch.clone()).await.unwrap(),
        BatchState::NonePresent
    );
    assert_eq!(
        items(&recording.sink, ItemsRequest::default())
            .await
            .page
            .snapshot,
        before
    );

    // An externally committed exact subset is not assumed to be an atomic
    // completion of this retained batch, and inspection fills in nothing.
    apply(&recording.sink, Batch::new(vec![batch.changes[0].clone()])).await;
    let partial = items(&recording.sink, ItemsRequest::default())
        .await
        .page
        .snapshot;
    assert_eq!(
        inspect(&recording.sink, batch.clone()).await.unwrap(),
        BatchState::Partial
    );
    assert_eq!(
        items(&recording.sink, ItemsRequest::default())
            .await
            .page
            .snapshot,
        partial
    );

    apply(&recording.sink, batch.clone()).await;
    let complete = items(&recording.sink, ItemsRequest::default())
        .await
        .page
        .snapshot;
    recording.sink.submit(JournalRecord::Gap { dropped: 3 });
    assert!(recording.sink.take_warning().is_some());
    assert_eq!(
        inspect(&recording.sink, batch.clone()).await.unwrap(),
        BatchState::AllPresent
    );
    let after = items(&recording.sink, ItemsRequest::default()).await;
    assert_eq!(after.page.snapshot, complete);
    assert!(!after.evidence.unwrap().known_recording_gaps);
    assert_eq!(recording.sink.dropped.load(Ordering::Relaxed), 3);
    assert!(recording.sink.take_warning().is_none());
    assert!(!Request::InspectBatch(batch.clone()).is_mutation());
    assert_eq!(
        format!("{:?}", Request::InspectBatch(batch)),
        "WorkRequest::InspectBatch"
    );
    assert_eq!(
        format!("{:?}", Response::BatchState(BatchState::AllPresent)),
        "WorkResponse::BatchState"
    );
}

#[tokio::test]
async fn work_inspect_recovers_committed_ids_after_restart_and_exact_replay() {
    let temp = TestDir::new();
    let mut recording = temp.recording();
    let saved = task();
    let first = apply(&recording.sink, saved.clone()).await;
    recording.finish();
    let recording = temp.recording();
    let before = items(&recording.sink, ItemsRequest::default())
        .await
        .page
        .snapshot;
    // The caller's stale cache need not know that its previous apply ran.
    assert_eq!(
        inspect(&recording.sink, saved.clone()).await.unwrap(),
        BatchState::AllPresent
    );
    assert_eq!(
        items(&recording.sink, ItemsRequest::default())
            .await
            .page
            .snapshot,
        before
    );
    let retried = apply(&recording.sink, saved).await;
    for (original, replayed) in first.changes.iter().zip(&retried.changes) {
        assert!(replayed.replayed);
        assert_eq!(replayed.change_id, original.change_id);
        assert_eq!(replayed.sequence, original.sequence);
        assert_eq!(
            replayed.original_recording_action_id,
            original.original_recording_action_id
        );
        assert_eq!(replayed.evidence_event_id, original.evidence_event_id);
    }
}

#[tokio::test]
async fn work_inspect_rejects_conflicting_mutation_basis_or_evidence_without_audit() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let saved = task();
    let receipt = apply(&recording.sink, saved.clone()).await;
    let before = items(&recording.sink, ItemsRequest::default())
        .await
        .page
        .snapshot;
    for conflict in ["mutation", "evidence", "basis"] {
        let mut batch = saved.clone();
        match conflict {
            "mutation" => {
                batch.changes[0].mutation = Mutation::CreateItem {
                    item_id: item(99),
                    kind: StableCode::new("task"),
                }
            }
            "evidence" => {
                batch.changes[0].evidence = Evidence::Event {
                    event_id: receipt.changes[0].evidence_event_id.clone(),
                }
            }
            "basis" => {
                batch.changes[0].basis = Basis::Observed;
                batch.changes[0].evidence = Evidence::Event {
                    event_id: receipt.changes[0].evidence_event_id.clone(),
                };
            }
            _ => unreachable!(),
        }
        assert_eq!(
            inspect(&recording.sink, batch).await.unwrap_err(),
            Error::Conflict
        );
        assert_eq!(
            items(&recording.sink, ItemsRequest::default())
                .await
                .page
                .snapshot,
            before
        );
    }
}

#[tokio::test]
async fn work_inspect_validates_bounded_retained_input_before_identity_lookups() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let before = items(&recording.sink, ItemsRequest::default())
        .await
        .page
        .snapshot;
    let mut invalid_version = task();
    invalid_version.version = 2;
    let mut duplicate = task();
    duplicate.changes.push(duplicate.changes[0].clone());
    let mut invalid_basis = task();
    invalid_basis.changes[0].basis = Basis::Inferred;
    let mut unsafe_mutation = serde_json::to_value(task()).unwrap();
    unsafe_mutation["changes"][1]["mutation"]["label"] = serde_json::json!("unsafe\nlabel");
    let unsafe_mutation: Batch = serde_json::from_value(unsafe_mutation).unwrap();
    for invalid in [
        Batch::new(vec![]),
        Batch::new(
            (0..=crate::journal::work_map::MAX_BATCH)
                .map(|n| create(100 + n as u64, "task"))
                .collect(),
        ),
        invalid_version,
        duplicate,
        invalid_basis,
        unsafe_mutation,
    ] {
        assert_eq!(
            inspect(&recording.sink, invalid).await.unwrap_err(),
            Error::InvalidInput
        );
        assert_eq!(
            items(&recording.sink, ItemsRequest::default())
                .await
                .page
                .snapshot,
            before
        );
    }
}

#[tokio::test]
async fn work_create_restart_replay_preserves_claims_and_original_provenance() {
    let temp = TestDir::new();
    let mut recording = temp.recording();
    let saved = task();
    let first = apply(&recording.sink, saved.clone()).await;
    assert!(first.changes.iter().all(|change| !change.replayed));
    let before = items(&recording.sink, ItemsRequest::default()).await;
    assert_eq!(before.page.items.len(), 1);
    let evidence = before.evidence.as_ref().unwrap();
    assert_eq!(evidence.snapshot, before.page.snapshot);
    bounded(evidence);
    let claims: Vec<_> = evidence
        .records
        .iter()
        .filter_map(|record| match record {
            HandoffRecord::Map { change } => Some(change),
            _ => None,
        })
        .collect();
    assert_eq!(claims.len(), 3);
    assert!(
        claims
            .iter()
            .all(|change| change.request.provenance.basis == Basis::UserAsserted)
    );
    assert!(
        claims
            .iter()
            .any(|change| matches!(&change.request.mutation,
        Mutation::CreateTag { namespace, key, label, .. }
        if namespace.as_str() == "work.title" && key.as_str() == item(1).as_str()
            && label.as_str() == "Review explicitly linked work"))
    );
    recording.finish();

    let recording = temp.recording();
    let replay = apply(&recording.sink, saved.clone()).await;
    assert_ne!(replay.attempt.action_id, first.attempt.action_id);
    for (new, old) in replay.changes.iter().zip(&first.changes) {
        assert!(new.replayed);
        assert_eq!(new.sequence, old.sequence);
        assert_eq!(
            new.original_recording_action_id,
            old.original_recording_action_id
        );
        assert_eq!(new.evidence_event_id, old.evidence_event_id);
        assert_eq!(new.observed_at_ms, old.observed_at_ms);
    }
    let after = items(&recording.sink, ItemsRequest::default()).await;
    assert_eq!(after.page.items.len(), 1);
    assert_eq!(
        after.page.snapshot.database_id(),
        before.page.snapshot.database_id()
    );
    assert_eq!(
        after.page.snapshot.map_sequence(),
        before.page.snapshot.map_sequence()
    );

    let mut changed = saved;
    changed.changes[0].mutation = Mutation::CreateItem {
        item_id: item(1),
        kind: StableCode::new("record"),
    };
    assert_eq!(
        recording
            .sink
            .work(Request::Apply(changed), Mode::Edit)
            .await
            .unwrap_err(),
        Error::Conflict
    );
    assert_eq!(
        items(&recording.sink, ItemsRequest::default())
            .await
            .page
            .items[0]
            .kind
            .as_str(),
        "task"
    );
}

#[tokio::test]
async fn work_modes_block_whole_batches_and_audit_blocked_replays() {
    let temp = TestDir::new();
    let recording = temp.recording();
    assert_eq!(
        recording
            .sink
            .work(Request::Apply(task()), Mode::ReadOnly)
            .await
            .unwrap_err(),
        Error::PermissionDenied
    );
    assert!(
        items(&recording.sink, ItemsRequest::default())
            .await
            .page
            .items
            .is_empty()
    );
    apply(&recording.sink, task()).await;
    assert_eq!(
        recording
            .sink
            .work(Request::Apply(task()), Mode::ReadOnly)
            .await
            .unwrap_err(),
        Error::PermissionDenied
    );
    let batch = Batch::new(vec![
        create(10, "task"),
        intent(
            11,
            Mutation::RenameTag {
                tag_id: tag(1),
                label: SemanticLabel::new("New deliberate label").unwrap(),
            },
        ),
    ]);
    assert_eq!(
        recording
            .sink
            .work(Request::Apply(batch), Mode::Write)
            .await
            .unwrap_err(),
        Error::PermissionDenied
    );
    assert_eq!(
        items(&recording.sink, ItemsRequest::default())
            .await
            .page
            .items
            .len(),
        1
    );
    let events = recording
        .sink
        .history(EventFilter::default(), None, None)
        .await
        .unwrap();
    assert_eq!(
        events
            .events
            .iter()
            .filter(|entry| entry.event.outcome == Outcome::Blocked)
            .count(),
        3
    );
    assert_eq!(
        events
            .events
            .iter()
            .filter(|entry| entry.event.outcome == Outcome::Requested)
            .count(),
        4
    );
}

#[tokio::test]
async fn work_item_pages_and_enrichment_keep_snapshot_across_writes_and_restart() {
    let temp = TestDir::new();
    let mut recording = temp.recording();
    apply(&recording.sink, task()).await;
    for chunk in (10..43).collect::<Vec<_>>().chunks(16) {
        apply(
            &recording.sink,
            Batch::new(chunk.iter().map(|&n| create(n, "task")).collect()),
        )
        .await;
    }
    let first = items(&recording.sink, ItemsRequest::default()).await;
    assert_eq!(first.page.items.len(), work_api::ITEM_LIMIT);
    assert!(first.page.truncated);
    let snapshot = first.page.snapshot.clone();
    apply(&recording.sink, Batch::new(vec![create(90, "task")])).await;
    recording.finish();
    let recording = temp.recording();
    let second = items(
        &recording.sink,
        ItemsRequest {
            cursor: first.page.next,
            snapshot: Some(snapshot.clone()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(second.page.snapshot, snapshot);
    assert_eq!(second.evidence.as_ref().unwrap().snapshot, snapshot);
    let third = items(
        &recording.sink,
        ItemsRequest {
            cursor: second.page.next,
            ..Default::default()
        },
    )
    .await;
    assert_eq!(third.page.snapshot, snapshot);
    assert_eq!(third.page.items.len(), 2);
    assert!(
        !third
            .page
            .items
            .iter()
            .any(|entry| entry.item_id == item(90))
    );
    assert!(!third.page.truncated);
    let replay = items(
        &recording.sink,
        ItemsRequest {
            snapshot: Some(snapshot.clone()),
            ..Default::default()
        },
    )
    .await;
    assert_eq!(replay.page.snapshot, snapshot);
    assert_eq!(replay.page.items[0].item_id, item(1));
}

#[tokio::test]
async fn work_detail_resolves_exact_source_and_expands_only_one_explicit_hop() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let key = SubjectKey::new(
        "forge",
        "https://example.test",
        "pull_request",
        "org/repo/7",
    )
    .unwrap();
    let mut descriptor = ActionDescriptor::new("test.source", Origin::Plugin("forge".into()));
    descriptor
        .subjects
        .push(SubjectSnapshot::new(key.clone(), SubjectRole::Primary));
    let action = recording.sink.begin_descriptor(descriptor, None);
    recording
        .sink
        .outcome(&action, Outcome::Unknown, Some(Reason::Transport));
    let resolved = match recording
        .sink
        .work(
            Request::Resolve(ResolveRequest {
                subject: key.clone(),
                action_id: action.clone(),
                snapshot: None,
            }),
            Mode::ReadOnly,
        )
        .await
        .unwrap()
    {
        Response::Resolved(response) => response,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(resolved.resolution.state, ResolutionState::Unbound);
    let evidence = recording
        .sink
        .history(
            EventFilter {
                action_id: Some(action.clone()),
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
    apply(&recording.sink, task()).await;
    apply(
        &recording.sink,
        Batch::new(vec![
            create(10, "pull_request"),
            create(11, "repository"),
            create(12, "record"),
            intent(
                20,
                Mutation::BindSubject {
                    item_id: item(10),
                    subject: key.clone(),
                    from_sequence: resolved.requested_sequence,
                    through_sequence: None,
                },
            ),
            intent(
                21,
                Mutation::RelateItems {
                    from: item(1),
                    to: item(10),
                    kind: RelationKind::References,
                },
            ),
            intent(
                22,
                Mutation::RelateItems {
                    from: item(10),
                    to: item(11),
                    kind: RelationKind::PartOf,
                },
            ),
            intent(
                23,
                Mutation::RelateItems {
                    from: item(1),
                    to: item(12),
                    kind: RelationKind::References,
                },
            ),
        ]),
    )
    .await;
    recording
        .sink
        .work(
            Request::Apply(Batch::new(vec![intent(
                24,
                Mutation::Retract {
                    claim_id: change(23),
                    expected: AssertionTarget::Relation {
                        from: item(1),
                        to: item(12),
                        kind: RelationKind::References,
                    },
                },
            )])),
            Mode::Edit,
        )
        .await
        .unwrap();
    let mut inferred = intent(
        25,
        Mutation::RelateItems {
            from: item(1),
            to: item(12),
            kind: RelationKind::References,
        },
    );
    inferred.basis = Basis::Inferred;
    inferred.evidence = Evidence::Event {
        event_id: evidence.events[0].event.event_id.clone(),
    };
    apply(&recording.sink, Batch::new(vec![inferred])).await;
    let initial = items(&recording.sink, ItemsRequest::default()).await;
    recording.sink.submit(JournalRecord::Gap { dropped: 2 });
    // A subsequent write flushes the real gap, but stays outside the saved cut.
    recording.sink.begin("test.after_cut", Origin::Host, None);
    let page = detail(
        &recording.sink,
        item(1),
        Some(initial.page.snapshot.clone()),
    )
    .await;
    assert!(!page.expansion_truncated);
    assert_eq!(page.primary.snapshot, initial.page.snapshot);
    let related = page.related.unwrap();
    assert_eq!(related.snapshot, page.primary.snapshot);
    assert_eq!(related.selection.items, vec![item(10)]);
    assert!(!related.known_recording_gaps);
    assert!(related.records.iter().any(|record| matches!(record, HandoffRecord::Map { change }
        if matches!(&change.request.mutation, Mutation::BindSubject { item_id, subject, .. } if item_id == &item(10) && subject == &key))));
    assert!(related.records.iter().any(|record| matches!(record, HandoffRecord::Action { event, .. } if event.action_id == action && event.outcome == Outcome::Unknown)));
    assert!(!related.records.iter().any(|record| matches!(record, HandoffRecord::Map { change }
        if matches!(&change.request.mutation, Mutation::CreateItem { item_id, .. } if item_id == &item(11)))));
    bounded(&page.primary);
    bounded(&related);
    let now = detail(&recording.sink, item(1), None).await;
    assert!(now.primary.known_recording_gaps);
    let resolved = match recording
        .sink
        .work(
            Request::Resolve(ResolveRequest {
                subject: key,
                action_id: action,
                snapshot: Some(page.primary.snapshot),
            }),
            Mode::ReadOnly,
        )
        .await
        .unwrap()
    {
        Response::Resolved(response) => response,
        other => panic!("unexpected {other:?}"),
    };
    assert_eq!(resolved.resolution.state, ResolutionState::Resolved);
    assert_eq!(resolved.resolution.items, vec![item(10)]);
}

#[tokio::test]
async fn work_detail_neighbour_cap_and_history_continuations_are_explicitly_bounded() {
    let temp = TestDir::new();
    let recording = temp.recording();
    apply(&recording.sink, task()).await;
    for n in 10..44 {
        apply(
            &recording.sink,
            Batch::new(vec![
                create(n, "record"),
                intent(
                    100 + n,
                    Mutation::RelateItems {
                        from: item(1),
                        to: item(n),
                        kind: RelationKind::References,
                    },
                ),
            ]),
        )
        .await;
    }
    let page = detail(&recording.sink, item(1), None).await;
    assert!(page.expansion_truncated);
    assert!(!page.primary.truncated);
    assert_eq!(
        page.related.as_ref().unwrap().selection.items.len(),
        work_api::NEIGHBOUR_LIMIT
    );

    // Repeated authored label assertions intentionally make the primary map
    // longer than one page. They are not collapsed to a guessed latest label.
    for chunk in (200..325).collect::<Vec<_>>().chunks(16) {
        apply(
            &recording.sink,
            Batch::new(
                chunk
                    .iter()
                    .map(|&n| {
                        intent(
                            n,
                            Mutation::AssignTag {
                                tag_id: tag(1),
                                target: TagTarget::Item(item(1)),
                            },
                        )
                    })
                    .collect(),
            ),
        )
        .await;
    }
    let detail = detail(&recording.sink, item(1), None).await;
    assert!(detail.expansion_truncated);
    assert!(detail.primary.truncated);
    let snapshot = detail.primary.snapshot.clone();
    apply(
        &recording.sink,
        Batch::new(vec![intent(
            400,
            Mutation::AssignTag {
                tag_id: tag(1),
                target: TagTarget::Item(item(1)),
            },
        )]),
    )
    .await;
    let mut page = detail.primary;
    let mut count = 0;
    loop {
        bounded(&page);
        assert_eq!(page.snapshot, snapshot);
        count += page.records.len();
        assert!(!page.records.iter().any(|record| matches!(record, HandoffRecord::Map { change: record } if record.request.id == change(400))));
        let Some(cursor) = page.next else { break };
        page = match recording
            .sink
            .work(
                Request::Evidence(EvidenceRequest {
                    selection: page.selection,
                    cursor: Some(cursor),
                    snapshot: Some(snapshot.clone()),
                }),
                Mode::ReadOnly,
            )
            .await
            .unwrap()
        {
            Response::Evidence(page) => page,
            other => panic!("unexpected {other:?}"),
        };
    }
    assert_eq!(count, 3 + 34 + 125);
}

#[tokio::test]
async fn work_reads_do_not_audit_or_flush_pending_recording_gaps() {
    let temp = TestDir::new();
    let recording = temp.recording();
    apply(&recording.sink, task()).await;
    let before = items(&recording.sink, ItemsRequest::default()).await;
    recording.sink.submit(JournalRecord::Gap { dropped: 3 });
    assert!(recording.sink.take_warning().is_some());
    for _ in 0..3 {
        let page = items(&recording.sink, ItemsRequest::default()).await;
        let detail = detail(&recording.sink, item(1), None).await;
        assert_eq!(page.page.snapshot, before.page.snapshot);
        assert_eq!(detail.primary.snapshot, before.page.snapshot);
        assert!(!detail.primary.known_recording_gaps);
    }
    assert_eq!(recording.sink.dropped.load(Ordering::Relaxed), 3);
    assert!(recording.sink.take_warning().is_none());
}

#[tokio::test]
async fn work_read_errors_do_not_count_as_missing_writes_and_sealed_queue_rejects() {
    let temp = TestDir::new();
    let mut recording = temp.recording();
    apply(&recording.sink, task()).await;
    let before = items(&recording.sink, ItemsRequest::default()).await;
    let connection = rusqlite::Connection::open(temp.path()).unwrap();
    connection
        .execute_batch("ALTER TABLE work_items RENAME TO test_hidden_items")
        .unwrap();
    let error = recording
        .sink
        .work(Request::Items(ItemsRequest::default()), Mode::ReadOnly)
        .await
        .unwrap_err();
    assert_eq!(error, Error::Storage);
    assert!(!error.to_string().contains("work_items"));
    assert!(recording.sink.take_warning().is_none());
    connection
        .execute_batch("ALTER TABLE test_hidden_items RENAME TO work_items")
        .unwrap();
    let after = items(&recording.sink, ItemsRequest::default()).await;
    assert_eq!(before.page.snapshot, after.page.snapshot);
    recording.finish();
    assert_eq!(
        recording
            .sink
            .work(Request::Items(ItemsRequest::default()), Mode::ReadOnly)
            .await
            .unwrap_err(),
        Error::Unavailable
    );
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
async fn work_queue_rejection_and_lost_mutation_acknowledgement_are_distinct() {
    let (sink, receiver) = isolated_sink(1);
    assert!(sink.sender.try_send(Work::Finish).is_ok());
    assert_eq!(
        sink.work(Request::Apply(task()), Mode::Write)
            .await
            .unwrap_err(),
        Error::Unavailable
    );
    drop(receiver);
    assert_eq!(
        sink.work(Request::Items(ItemsRequest::default()), Mode::ReadOnly)
            .await
            .unwrap_err(),
        Error::Unavailable
    );
    assert_eq!(sink.dropped.load(Ordering::Relaxed), 0);
    assert!(sink.take_warning().is_none());
    for (request, expected) in [
        (Request::Apply(task()), Error::OutcomeUnknown),
        (Request::Items(ItemsRequest::default()), Error::Unavailable),
    ] {
        let (sink, receiver) = isolated_sink(1);
        let worker = std::thread::spawn(move || {
            drop(receiver.recv().unwrap());
        });
        assert_eq!(sink.work(request, Mode::Write).await.unwrap_err(), expected);
        worker.join().unwrap();
        assert_eq!(sink.dropped.load(Ordering::Relaxed), 0);
        assert!(sink.take_warning().is_none());
    }
}

#[tokio::test]
async fn work_typed_request_uses_cli_validation_without_logging_annotations() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let mut batch = task();
    assert!(!format!("{batch:?}").contains("Review explicitly"));
    assert!(!format!("{:?}", batch.changes[1]).contains("Review explicitly"));
    assert_eq!(
        format!("{:?}", Request::Apply(batch.clone())),
        "WorkRequest::Apply"
    );
    batch.version = 2;
    assert_eq!(
        recording
            .sink
            .work(Request::Apply(batch), Mode::Edit)
            .await
            .unwrap_err(),
        Error::InvalidInput
    );
    assert!(
        items(&recording.sink, ItemsRequest::default())
            .await
            .page
            .items
            .is_empty()
    );
    let journal = Journal::open(&temp.path()).unwrap();
    assert!(journal.query(&Query::default()).unwrap().is_empty());
}

#[tokio::test]
async fn work_evidence_byte_bound_and_cursor_scope_survive_later_outcomes() {
    let temp = TestDir::new();
    let recording = temp.recording();
    let mut descriptor = ActionDescriptor::new("test.large_source", Origin::Host);
    for n in 0..32 {
        descriptor.subjects.push(SubjectSnapshot::new(
            SubjectKey::new(
                "sample",
                "https://example.test",
                "item",
                &format!("item-{n}-{}", "a".repeat(1000)),
            )
            .unwrap(),
            SubjectRole::Context,
        ));
    }
    let actions: Vec<_> = (0..12)
        .map(|_| recording.sink.begin_descriptor(descriptor.clone(), None))
        .collect();
    let selection = crate::journal::work_map::Selection {
        items: Vec::new(),
        actions: actions.clone(),
    };
    let mut page = match recording
        .sink
        .work(
            Request::Evidence(EvidenceRequest {
                selection: selection.clone(),
                cursor: None,
                snapshot: None,
            }),
            Mode::ReadOnly,
        )
        .await
        .unwrap()
    {
        Response::Evidence(page) => page,
        other => panic!("unexpected {other:?}"),
    };
    assert!(page.truncated);
    assert!(!page.records.is_empty());
    assert!(page.records.len() < actions.len());
    let snapshot = page.snapshot.clone();
    let cursor = page.next.clone().unwrap();
    let mut changed = selection.clone();
    changed.actions.pop();
    assert_eq!(
        recording
            .sink
            .work(
                Request::Evidence(EvidenceRequest {
                    selection: changed,
                    cursor: Some(cursor),
                    snapshot: Some(snapshot.clone()),
                }),
                Mode::ReadOnly
            )
            .await
            .unwrap_err(),
        Error::Storage
    );
    for action in &actions {
        recording.sink.outcome(action, Outcome::Succeeded, None);
    }
    let mut found = Vec::new();
    loop {
        bounded(&page);
        assert_eq!(page.snapshot, snapshot);
        for record in &page.records {
            let HandoffRecord::Action { event, .. } = record else {
                panic!("unexpected map or gap")
            };
            assert_eq!(event.outcome, Outcome::Requested);
            found.push(event.action_id.clone());
        }
        let Some(cursor) = page.next else { break };
        page = match recording
            .sink
            .work(
                Request::Evidence(EvidenceRequest {
                    selection: selection.clone(),
                    cursor: Some(cursor),
                    snapshot: Some(snapshot.clone()),
                }),
                Mode::ReadOnly,
            )
            .await
            .unwrap()
        {
            Response::Evidence(page) => page,
            other => panic!("unexpected {other:?}"),
        };
    }
    assert_eq!(found, actions);
    assert!(recording.sink.take_warning().is_none());
}

#[tokio::test]
async fn work_tag_catalog_is_independent_bounded_and_read_only_through_the_queue() {
    let temp = TestDir::new();
    let mut recording = temp.recording();
    let namespace = TagKey::new("work.group").unwrap();
    let empty_cut = items(&recording.sink, ItemsRequest::default())
        .await
        .page
        .snapshot;
    let empty_request = Request::Tags(TagsRequest {
        namespace: namespace.clone(),
        cursor: None,
        snapshot: Some(empty_cut.clone()),
    });
    assert!(!empty_request.is_mutation());
    assert_eq!(format!("{empty_request:?}"), "WorkRequest::Tags");
    let Response::Tags(empty) = recording
        .sink
        .work(empty_request, Mode::ReadOnly)
        .await
        .unwrap()
    else {
        panic!("expected empty tag catalog");
    };
    assert!(empty.tags.is_empty());
    assert_eq!(empty.snapshot, empty_cut);
    assert!(!empty.truncated);
    for index in 0..=work_api::TAG_LIMIT {
        apply(
            &recording.sink,
            Batch::new(vec![intent(
                100 + index as u64,
                Mutation::CreateTag {
                    tag_id: tag(100 + index as u64),
                    namespace: namespace.clone(),
                    key: TagKey::new(&format!("group-{index:03}")).unwrap(),
                    label: SemanticLabel::new("Reusable empty group").unwrap(),
                },
            )]),
        )
        .await;
    }
    let item_page = items(&recording.sink, ItemsRequest::default()).await;
    assert!(item_page.page.items.is_empty());
    let cut = item_page.page.snapshot;
    let mut saved_cursor = None;
    for mode in Mode::ALL {
        let Response::Tags(first) = recording
            .sink
            .work(
                Request::Tags(TagsRequest {
                    namespace: namespace.clone(),
                    cursor: None,
                    snapshot: Some(cut.clone()),
                }),
                mode,
            )
            .await
            .unwrap()
        else {
            panic!("expected tag catalog");
        };
        assert_eq!(first.tags.len(), work_api::TAG_LIMIT);
        assert_eq!(first.snapshot, cut);
        assert!(first.truncated);
        saved_cursor = first.next.clone();
        let Response::Tags(second) = recording
            .sink
            .work(
                Request::Tags(TagsRequest {
                    namespace: namespace.clone(),
                    cursor: first.next,
                    snapshot: Some(cut.clone()),
                }),
                mode,
            )
            .await
            .unwrap()
        else {
            panic!("expected second tag catalog page");
        };
        assert_eq!(second.tags.len(), 1);
        assert_eq!(second.tags[0].tag_id, tag(100 + work_api::TAG_LIMIT as u64));
        assert_eq!(second.snapshot, cut);
        assert!(!second.truncated);
        assert_eq!(
            items(&recording.sink, ItemsRequest::default())
                .await
                .page
                .snapshot,
            cut
        );
    }
    assert_eq!(
        recording
            .sink
            .work(
                Request::Tags(TagsRequest {
                    namespace: TagKey::new("work.title").unwrap(),
                    cursor: saved_cursor.clone(),
                    snapshot: Some(cut.clone()),
                }),
                Mode::ReadOnly
            )
            .await
            .unwrap_err(),
        Error::Storage
    );
    recording.finish();
    let recording = temp.recording();
    let Response::Tags(after_restart) = recording
        .sink
        .work(
            Request::Tags(TagsRequest {
                namespace,
                cursor: saved_cursor,
                snapshot: Some(cut.clone()),
            }),
            Mode::ReadOnly,
        )
        .await
        .unwrap()
    else {
        panic!("expected persisted tag catalog");
    };
    assert_eq!(after_restart.snapshot, cut);
    assert_eq!(after_restart.tags.len(), 1);
}
