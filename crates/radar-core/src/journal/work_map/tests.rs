use super::*;

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("radar-work-map-{}", Uuid::new_v4())))
    }

    fn path(&self) -> PathBuf {
        self.0.join("actions.sqlite3")
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        // Only this test's unique temporary database is removed.
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    journal: Journal,
    session: SessionId,
    recorder: ActionId,
    evidence: EventId,
    temp: TestDir,
}

impl Fixture {
    fn new() -> Self {
        let temp = TestDir::new();
        let mut journal = Journal::open(&temp.path()).unwrap();
        let session = journal.start_session_with(SessionOrigin::Cli).unwrap();
        let recorder = journal
            .begin(
                &session,
                ActionDescriptor::new("work-map.annotate", Origin::Host),
                None,
            )
            .unwrap();
        let evidence = events(&journal, &recorder)[0].event_id.clone();
        Self {
            journal,
            session,
            recorder,
            evidence,
            temp,
        }
    }

    fn request(&self, mutation: Mutation) -> ChangeRequest {
        ChangeRequest {
            id: ChangeId::new(),
            provenance: Provenance {
                recording_action_id: self.recorder.clone(),
                evidence_event_id: self.evidence.clone(),
                basis: Basis::UserAsserted,
            },
            observed_at_ms: 1000,
            mutation,
        }
    }

    fn apply(&mut self, mutation: Mutation) -> ChangeRequest {
        let request = self.request(mutation);
        self.journal.apply_work_map(&request).unwrap();
        request
    }

    fn item(&mut self) -> ItemId {
        self.item_of_kind("card")
    }

    fn item_of_kind(&mut self, kind: &'static str) -> ItemId {
        let item_id = ItemId::new();
        self.apply(Mutation::CreateItem {
            item_id: item_id.clone(),
            kind: StableCode::new(kind),
        });
        item_id
    }

    fn tag(&mut self, namespace: &str, key: &str, label: &str) -> TagId {
        let tag_id = TagId::new();
        self.apply(Mutation::CreateTag {
            tag_id: tag_id.clone(),
            namespace: TagKey::new(namespace).unwrap(),
            key: TagKey::new(key).unwrap(),
            label: SemanticLabel::new(label).unwrap(),
        });
        tag_id
    }

    fn observe(&mut self, key: &SubjectKey) -> ActionEvent {
        let action = self
            .journal
            .begin(
                &self.session,
                ActionDescriptor::new("item.inspect", Origin::Plugin(key.provider.clone()))
                    .with_subject(SubjectSnapshot::new(key.clone(), SubjectRole::Primary)),
                None,
            )
            .unwrap();
        events(&self.journal, &action).remove(0)
    }

    fn bind(&mut self, item: &ItemId, key: &SubjectKey, event: &ActionEvent) -> ChangeRequest {
        let mut request = self.request(Mutation::BindSubject {
            item_id: item.clone(),
            subject: key.clone(),
            from_sequence: event.sequence,
            through_sequence: None,
        });
        request.provenance.evidence_event_id = event.event_id.clone();
        request.provenance.basis = Basis::Observed;
        self.journal.apply_work_map(&request).unwrap();
        request
    }

    fn assign(&mut self, tag: &TagId, target: TagTarget) -> ChangeRequest {
        self.apply(Mutation::AssignTag {
            tag_id: tag.clone(),
            target,
        })
    }
}

fn events(journal: &Journal, action: &ActionId) -> Vec<ActionEvent> {
    journal
        .query(&Query {
            action_id: Some(action.clone()),
            ..Query::default()
        })
        .unwrap()
}

fn key(provider: &str, instance: &str, kind: &str, id: &str) -> SubjectKey {
    SubjectKey::new(provider, instance, kind, id).unwrap()
}

fn selection(items: &[ItemId], actions: &[ActionId]) -> Selection {
    Selection {
        items: items.to_vec(),
        actions: actions.to_vec(),
    }
}

fn collect_handoff(journal: &Journal, selection: &Selection, limits: Limits) -> Vec<HandoffRecord> {
    let mut all = Vec::new();
    let mut cursor = None;
    let mut snapshot = None;
    loop {
        let page = journal.handoff(selection, limits, cursor.as_ref()).unwrap();
        assert!(page.records.len() <= limits.records);
        assert!(serde_json::to_vec(&page).unwrap().len() <= limits.bytes);
        assert_eq!(page.authority, "observations_only");
        assert_eq!(page.task_instruction, "unspecified");
        assert_eq!(page.execution_authorization, "unspecified");
        assert!(page.completeness_not_guaranteed);
        assert_eq!(page.truncated, page.next.is_some());
        for record in &page.records {
            if let HandoffRecord::Map { change } = record {
                assert_eq!(change.version, VERSION);
            }
        }
        if let Some(expected) = &snapshot {
            assert_eq!(&page.snapshot, expected);
        } else {
            snapshot = Some(page.snapshot.clone());
        }
        all.extend(page.records);
        match page.next {
            None => break,
            Some(next) => {
                assert_ne!(cursor.as_ref(), Some(&next), "cursor must make progress");
                // A persisted cursor retains its database, snapshot and selection.
                cursor = Some(serde_json::from_value(serde_json::to_value(next).unwrap()).unwrap());
            }
        }
        assert!(all.len() < 200, "small fixture must not loop forever");
    }
    all
}

fn map_changes(records: &[HandoffRecord]) -> Vec<&ChangeRecord> {
    records
        .iter()
        .filter_map(|record| match record {
            HandoffRecord::Map { change } => Some(change.as_ref()),
            _ => None,
        })
        .collect()
}

fn item_ids(page: &ItemPage) -> Vec<ItemId> {
    page.items
        .iter()
        .map(|entry| entry.item_id.clone())
        .collect()
}

#[test]
fn scoped_subjects_and_matching_labels_never_merge_identities() {
    let mut fixture = Fixture::new();
    let first = fixture.item();
    let second = fixture.item();
    let first_key = key("tracker", "https://one.example.test", "card", "ABC-12");
    let second_key = key("tracker", "https://two.example.test", "card", "ABC-12");
    let first_event = fixture.observe(&first_key);
    let second_event = fixture.observe(&second_key);
    fixture.bind(&first, &first_key, &first_event);
    fixture.bind(&second, &second_key, &second_event);
    let first_tag = fixture.tag("team-a", "review", "Review");
    let second_tag = fixture.tag("team-b", "review", "Review");
    fixture.assign(&first_tag, TagTarget::Item(first.clone()));
    fixture.assign(&second_tag, TagTarget::Item(second.clone()));

    for (subject, event, item, tag) in [
        (&first_key, &first_event, &first, &first_tag),
        (&second_key, &second_event, &second, &second_tag),
    ] {
        let resolution = fixture
            .journal
            .resolve_subject(subject, &event.action_id, None)
            .unwrap();
        assert_eq!(resolution.state, ResolutionState::Resolved);
        assert_eq!(resolution.items.as_slice(), std::slice::from_ref(item));
        let page = fixture
            .journal
            .query_items(
                &ItemFilter {
                    all_tags: vec![tag.clone()],
                },
                10,
                None,
            )
            .unwrap();
        assert_eq!(item_ids(&page).as_slice(), std::slice::from_ref(item));
    }
    assert_ne!(first_tag, second_tag);
    let conflicting = fixture.request(Mutation::CreateTag {
        tag_id: TagId::new(),
        namespace: TagKey::new("team-a").unwrap(),
        key: TagKey::new("review").unwrap(),
        label: SemanticLabel::new("A different label").unwrap(),
    });
    assert!(fixture.journal.apply_work_map(&conflicting).is_err());
    assert_eq!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .len(),
        2
    );
}

#[test]
fn tag_rename_retains_stable_identity_and_append_only_label_history() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let tag = fixture.tag("workflow", "needs-review", "Needs review");
    fixture.assign(&tag, TagTarget::Item(item.clone()));
    let renamed = fixture.apply(Mutation::RenameTag {
        tag_id: tag.clone(),
        label: SemanticLabel::new("Ready for review").unwrap(),
    });
    let records = collect_handoff(
        &fixture.journal,
        &selection(std::slice::from_ref(&item), &[]),
        Limits::default(),
    );
    let changes = map_changes(&records);
    assert!(changes.iter().any(|change| matches!(
        &change.request.mutation,
        Mutation::CreateTag { tag_id, label, .. }
            if tag_id == &tag && label.as_str() == "Needs review"
    )));
    assert!(changes.iter().any(|change| change.request == renamed));
    let stored: (String, String, i64) = fixture.journal.connection.query_row(
        "SELECT namespace,stable_key,(SELECT count(*) FROM work_tags) FROM work_tags WHERE id=?1",
        [tag.as_str()],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(stored, ("workflow".into(), "needs-review".into(), 1));
    assert_eq!(
        item_ids(
            &fixture
                .journal
                .query_items(
                    &ItemFilter {
                        all_tags: vec![tag]
                    },
                    10,
                    None
                )
                .unwrap()
        ),
        [item]
    );
}

#[test]
fn item_tag_filters_intersect_and_action_tags_do_not_propagate() {
    let mut fixture = Fixture::new();
    let first = fixture.item();
    let second = fixture.item();
    let third = fixture.item();
    let review = fixture.tag("workflow", "review", "Review");
    let priority = fixture.tag("workflow", "priority", "Priority");
    for item in [&first, &second] {
        fixture.assign(&review, TagTarget::Item(item.clone()));
    }
    for item in [&first, &third] {
        fixture.assign(&priority, TagTarget::Item(item.clone()));
    }
    let subject = key("tracker", "acme", "card", "ABC-12");
    let observed = fixture.observe(&subject);
    fixture.bind(&second, &subject, &observed);
    let action_assignment =
        fixture.assign(&priority, TagTarget::Action(observed.action_id.clone()));
    let filter = ItemFilter {
        all_tags: vec![review.clone(), priority.clone()],
    };
    assert_eq!(
        item_ids(&fixture.journal.query_items(&filter, 10, None).unwrap()),
        [first]
    );
    let action_records = collect_handoff(
        &fixture.journal,
        &selection(&[], &[observed.action_id]),
        Limits::default(),
    );
    assert!(
        map_changes(&action_records)
            .iter()
            .any(|change| change.request == action_assignment)
    );
    assert!(
        map_changes(&action_records)
            .iter()
            .all(|change| !matches!(change.request.mutation, Mutation::RelateItems { .. }))
    );
    let duplicate_filter = ItemFilter {
        all_tags: vec![review.clone(), review],
    };
    assert_eq!(
        fixture
            .journal
            .query_items(&duplicate_filter, 10, None)
            .unwrap()
            .items
            .len(),
        2
    );
}

#[test]
fn exact_retries_survive_reopen_and_conflicting_payloads_never_append() {
    let mut fixture = Fixture::new();
    let request = fixture.request(Mutation::CreateItem {
        item_id: ItemId::new(),
        kind: StableCode::new("card"),
    });
    let sequence = fixture.journal.apply_work_map(&request).unwrap();
    let snapshot = fixture.journal.work_map_snapshot().unwrap();
    assert_eq!(fixture.journal.apply_work_map(&request).unwrap(), sequence);
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), snapshot);
    fixture.journal.flush().unwrap();
    let mut reopened = Journal::open(&fixture.temp.path()).unwrap();
    assert_eq!(reopened.apply_work_map(&request).unwrap(), sequence);
    assert_eq!(reopened.work_map_snapshot().unwrap(), snapshot);

    let mut changed = request.clone();
    changed.mutation = Mutation::CreateItem {
        item_id: ItemId::new(),
        kind: StableCode::new("card"),
    };
    assert!(reopened.apply_work_map(&changed).is_err());
    changed = request.clone();
    changed.observed_at_ms += 1;
    assert!(reopened.apply_work_map(&changed).is_err());
    changed = request;
    changed.provenance.basis = Basis::Inferred;
    assert!(reopened.apply_work_map(&changed).is_err());
    assert_eq!(reopened.work_map_snapshot().unwrap(), snapshot);
    assert_eq!(
        reopened
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .len(),
        1
    );
    reopened.finish_session(&fixture.session).unwrap();
    assert!(reopened.apply_work_map(&changed).is_err());
    // Exact replay remains valid after the original recording context closes.
    changed.provenance.basis = Basis::UserAsserted;
    assert_eq!(reopened.apply_work_map(&changed).unwrap(), sequence);
}

#[test]
fn invalid_batch_rolls_back_every_fact_and_preserves_retry_ids() {
    let mut fixture = Fixture::new();
    let item = ItemId::new();
    let create = fixture.request(Mutation::CreateItem {
        item_id: item.clone(),
        kind: StableCode::new("card"),
    });
    let invalid = fixture.request(Mutation::AssignTag {
        tag_id: TagId::new(),
        target: TagTarget::Item(item.clone()),
    });
    let before = fixture.journal.work_map_snapshot().unwrap();
    assert!(
        fixture
            .journal
            .apply_work_map_batch(&[create.clone(), invalid])
            .is_err()
    );
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), before);
    assert!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .is_empty()
    );
    let sequence = fixture.journal.apply_work_map(&create).unwrap();
    assert_eq!(sequence, before.map_sequence() + 1);
    assert_eq!(
        fixture
            .journal
            .apply_work_map_batch(&[create.clone(), create])
            .unwrap(),
        [sequence, sequence]
    );
    let oversized = (0..=MAX_BATCH)
        .map(|_| {
            fixture.request(Mutation::CreateItem {
                item_id: ItemId::new(),
                kind: StableCode::new("card"),
            })
        })
        .collect::<Vec<_>>();
    assert!(fixture.journal.apply_work_map_batch(&oversized).is_err());
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
fn recording_and_evidence_producers_and_sessions_are_independent() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let subject = key("tracker", "https://acme.example.test", "card", "ABC-12");
    let source_session = fixture
        .journal
        .start_session_with(SessionOrigin::Tui)
        .unwrap();
    let action = ActionId::new();
    fixture
        .journal
        .begin_with_id_at(
            &source_session,
            &action,
            ActionDescriptor::new("card.inspect", Origin::Plugin("tracker".into()))
                .with_subject(SubjectSnapshot::new(subject.clone(), SubjectRole::Primary)),
            None,
            2000,
        )
        .unwrap();
    let requested = events(&fixture.journal, &action).remove(0);
    let evidence = fixture
        .journal
        .record_at(&action, Outcome::Succeeded, Metadata::default(), 1500)
        .unwrap();
    fixture.journal.finish_session(&source_session).unwrap();
    let mut request = fixture.request(Mutation::BindSubject {
        item_id: item.clone(),
        subject,
        from_sequence: requested.sequence,
        through_sequence: None,
    });
    request.provenance.basis = Basis::Observed;
    request.provenance.evidence_event_id = evidence.clone();
    fixture.journal.apply_work_map(&request).unwrap();
    let records = collect_handoff(
        &fixture.journal,
        &selection(&[item], &[]),
        Limits::default(),
    );
    let changes = map_changes(&records);
    let stored = changes
        .iter()
        .find(|change| change.request.id == request.id)
        .unwrap();
    assert_eq!(stored.recording.action_id, fixture.recorder);
    assert_eq!(stored.recording.session_id, fixture.session);
    assert_eq!(stored.recording.origin, Origin::Host);
    assert_eq!(stored.recording.session_origin, SessionOrigin::Cli);
    assert_eq!(stored.recording.kind.as_str(), "work-map.annotate");
    assert_eq!(stored.evidence.event_id, evidence);
    assert_eq!(stored.evidence.action.action_id, action);
    assert_eq!(stored.evidence.action.session_id, source_session);
    assert_eq!(
        stored.evidence.action.origin,
        Origin::Plugin("tracker".into())
    );
    assert_eq!(stored.evidence.action.session_origin, SessionOrigin::Tui);
    assert_eq!(stored.evidence.action.kind.as_str(), "card.inspect");
    assert_eq!(stored.evidence.outcome, Outcome::Succeeded);
    assert_eq!(stored.evidence.observed_at_ms, 1500);
    assert!(!stored.evidence.observation_time_estimated);
    assert_eq!(stored.recording.build_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(
        stored.evidence.action.build_version,
        env!("CARGO_PKG_VERSION")
    );
}

#[test]
fn observed_bindings_require_matching_evidence_and_real_request_boundaries() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let subject = key("tracker", "one", "card", "ABC-12");
    let event = fixture.observe(&subject);
    let terminal_id = fixture
        .journal
        .record(&event.action_id, Outcome::Succeeded, Metadata::default())
        .unwrap();
    let terminal = events(&fixture.journal, &event.action_id).pop().unwrap();
    let mut request = fixture.request(Mutation::BindSubject {
        item_id: item.clone(),
        subject: subject.clone(),
        from_sequence: event.sequence,
        through_sequence: None,
    });
    request.provenance.basis = Basis::Observed;
    // The annotation action has no subject; merely citing an event is insufficient.
    assert!(fixture.journal.apply_work_map(&request).is_err());
    request.provenance.evidence_event_id = terminal_id;
    for wrong in [
        key("other", "one", "card", "ABC-12"),
        key("tracker", "two", "card", "ABC-12"),
        key("tracker", "one", "collection", "ABC-12"),
        key("tracker", "one", "card", "ABC-13"),
    ] {
        request.mutation = Mutation::BindSubject {
            item_id: item.clone(),
            subject: wrong,
            from_sequence: event.sequence,
            through_sequence: None,
        };
        assert!(fixture.journal.apply_work_map(&request).is_err());
    }
    for (from, through) in [
        (0, None),
        (terminal.sequence, None),
        (event.sequence, Some(event.sequence - 1)),
        (event.sequence, Some(terminal.sequence + 1)),
    ] {
        request.mutation = Mutation::BindSubject {
            item_id: item.clone(),
            subject: subject.clone(),
            from_sequence: from,
            through_sequence: through,
        };
        assert!(fixture.journal.apply_work_map(&request).is_err());
    }
    request.mutation = Mutation::BindSubject {
        item_id: item,
        subject,
        from_sequence: event.sequence,
        through_sequence: Some(terminal.sequence),
    };
    fixture.journal.apply_work_map(&request).unwrap();
}

#[test]
fn missing_provenance_and_closed_recording_context_fail_without_partial_rows() {
    let mut fixture = Fixture::new();
    let mut request = fixture.request(Mutation::CreateItem {
        item_id: ItemId::new(),
        kind: StableCode::new("card"),
    });
    let before = fixture.journal.work_map_snapshot().unwrap();
    request.provenance.recording_action_id = ActionId::new();
    assert!(fixture.journal.apply_work_map(&request).is_err());
    request.provenance.recording_action_id = fixture.recorder.clone();
    request.provenance.evidence_event_id = EventId::new();
    assert!(fixture.journal.apply_work_map(&request).is_err());
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), before);
    request.provenance.evidence_event_id = fixture.evidence.clone();
    fixture.journal.finish_session(&fixture.session).unwrap();
    assert!(fixture.journal.apply_work_map(&request).is_err());
    assert_eq!(
        fixture.journal.work_map_snapshot().unwrap().map_sequence(),
        before.map_sequence()
    );
}

#[test]
fn retractions_check_claim_type_and_target_and_preserve_as_of_history() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let other = fixture.item();
    let tag = fixture.tag("workflow", "review", "Review");
    let assignment = fixture.assign(&tag, TagTarget::Item(item.clone()));
    let filter = ItemFilter {
        all_tags: vec![tag.clone()],
    };
    let before = fixture.journal.work_map_snapshot().unwrap();
    for expected in [
        AssertionTarget::LabelAssertion {
            tag_id: tag.clone(),
        },
        AssertionTarget::TagAssignment {
            tag_id: tag.clone(),
            target: TagTarget::Item(other),
        },
        AssertionTarget::TagAssignment {
            tag_id: tag.clone(),
            target: TagTarget::Action(fixture.recorder.clone()),
        },
    ] {
        let retract = fixture.request(Mutation::Retract {
            claim_id: assignment.id.clone(),
            expected,
        });
        assert!(fixture.journal.apply_work_map(&retract).is_err());
    }
    let chosen = selection(std::slice::from_ref(&item), &[]);
    let first = fixture
        .journal
        .handoff(
            &chosen,
            Limits {
                records: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    let cursor = first.next.clone().unwrap();
    let retract = fixture.apply(Mutation::Retract {
        claim_id: assignment.id.clone(),
        expected: AssertionTarget::TagAssignment {
            tag_id: tag.clone(),
            target: TagTarget::Item(item.clone()),
        },
    });
    assert!(
        fixture
            .journal
            .query_items(&filter, 10, None)
            .unwrap()
            .items
            .is_empty()
    );
    let historic = fixture
        .journal
        .handoff(&chosen, Limits::default(), Some(&cursor))
        .unwrap();
    assert_eq!(historic.snapshot, before);
    assert!(
        map_changes(&historic.records)
            .iter()
            .any(|change| change.request.id == assignment.id && !change.retracted)
    );
    let current = collect_handoff(&fixture.journal, &chosen, Limits::default());
    assert!(
        map_changes(&current)
            .iter()
            .any(|change| change.request.id == assignment.id && change.retracted)
    );
    assert!(
        map_changes(&current)
            .iter()
            .any(|change| change.request.id == retract.id)
    );
    let second_retraction = fixture.request(retract.mutation.clone());
    assert!(fixture.journal.apply_work_map(&second_retraction).is_err());
    let retract_retraction = fixture.request(Mutation::Retract {
        claim_id: retract.id,
        expected: AssertionTarget::TagAssignment {
            tag_id: tag,
            target: TagTarget::Item(item),
        },
    });
    assert!(fixture.journal.apply_work_map(&retract_retraction).is_err());
}

#[test]
fn reused_paths_rebind_atomically_and_late_results_keep_request_time_identity() {
    let mut fixture = Fixture::new();
    let old_item = fixture.item_of_kind("branch");
    let new_item = fixture.item_of_kind("branch");
    let subject = key("vcs", "/work/acme/api", "branch", "feature/review");
    let old_action = fixture.observe(&subject);
    let open_binding = fixture.bind(&old_item, &subject, &old_action);
    let new_action = fixture.observe(&subject);
    let before_rebind = fixture.journal.work_map_snapshot().unwrap();
    let retract = fixture.request(Mutation::Retract {
        claim_id: open_binding.id.clone(),
        expected: AssertionTarget::Binding {
            item_id: old_item.clone(),
            subject: subject.clone(),
        },
    });
    let mut closed = fixture.request(Mutation::BindSubject {
        item_id: old_item.clone(),
        subject: subject.clone(),
        from_sequence: old_action.sequence,
        through_sequence: Some(new_action.sequence - 1),
    });
    closed.provenance = open_binding.provenance.clone();
    let mut replacement = fixture.request(Mutation::BindSubject {
        item_id: new_item.clone(),
        subject: subject.clone(),
        from_sequence: new_action.sequence,
        through_sequence: None,
    });
    replacement.provenance.evidence_event_id = new_action.event_id.clone();
    replacement.provenance.basis = Basis::Observed;
    let batch = [retract, closed, replacement];
    let sequences = fixture.journal.apply_work_map_batch(&batch).unwrap();
    assert_eq!(
        fixture.journal.apply_work_map_batch(&batch).unwrap(),
        sequences
    );
    fixture
        .journal
        .record(
            &old_action.action_id,
            Outcome::Succeeded,
            Metadata::default(),
        )
        .unwrap();
    for (action, expected) in [
        (&old_action.action_id, &old_item),
        (&new_action.action_id, &new_item),
    ] {
        let resolved = fixture
            .journal
            .resolve_subject(&subject, action, None)
            .unwrap();
        assert_eq!(resolved.state, ResolutionState::Resolved);
        assert_eq!(resolved.items.as_slice(), std::slice::from_ref(expected));
    }
    let historical = fixture
        .journal
        .resolve_subject(&subject, &new_action.action_id, Some(&before_rebind))
        .unwrap();
    assert_eq!(historical.items.as_slice(), std::slice::from_ref(&old_item));
    let old_history = collect_handoff(
        &fixture.journal,
        &selection(&[old_item], &[]),
        Limits::default(),
    );
    assert!(old_history.iter().any(|record| matches!(record, HandoffRecord::Action { event, association: HistoryAssociation::ExplicitBinding, .. } if event.action_id == old_action.action_id && event.outcome == Outcome::Succeeded)));
    assert!(!old_history.iter().any(|record| matches!(record, HandoffRecord::Action { event, .. } if event.action_id == new_action.action_id)));
    let stale = batch
        .into_iter()
        .map(|mut request| {
            request.id = ChangeId::new();
            request
        })
        .collect::<Vec<_>>();
    let current = fixture.journal.work_map_snapshot().unwrap();
    assert!(fixture.journal.apply_work_map_batch(&stale).is_err());
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), current);
}

#[test]
fn conflicting_bindings_are_ambiguous_and_aliases_remain_explicit() {
    let mut fixture = Fixture::new();
    let first = fixture.item();
    let second = fixture.item();
    let subject = key("tracker", "acme", "card", "ABC-12");
    let action = fixture.observe(&subject);
    fixture.bind(&first, &subject, &action);
    fixture.bind(&first, &subject, &action);
    assert_eq!(
        fixture
            .journal
            .resolve_subject(&subject, &action.action_id, None)
            .unwrap()
            .state,
        ResolutionState::Resolved
    );
    fixture.bind(&second, &subject, &action);
    let ambiguous = fixture
        .journal
        .resolve_subject(&subject, &action.action_id, None)
        .unwrap();
    assert_eq!(ambiguous.state, ResolutionState::Ambiguous);
    assert_eq!(ambiguous.items.len(), 2);
    assert!(ambiguous.items.contains(&first));
    assert!(ambiguous.items.contains(&second));
    assert_eq!(ambiguous.claims.len(), 3);
    let records = collect_handoff(
        &fixture.journal,
        &selection(std::slice::from_ref(&first), &[]),
        Limits::default(),
    );
    assert!(records.iter().any(|record| matches!(record, HandoffRecord::Action { event, association: HistoryAssociation::AmbiguousBinding, .. } if event.action_id == action.action_id)));

    let alias = key("tracker", "acme", "card", "ABC-99");
    let renamed_action = fixture.observe(&alias);
    assert_eq!(
        fixture
            .journal
            .resolve_subject(&alias, &renamed_action.action_id, None)
            .unwrap()
            .state,
        ResolutionState::Unbound
    );
    fixture.bind(&first, &alias, &renamed_action);
    assert_eq!(
        fixture
            .journal
            .resolve_subject(&alias, &renamed_action.action_id, None)
            .unwrap()
            .items,
        [first]
    );
}

#[test]
fn handoff_continuation_keeps_all_highwaters_during_append_and_retraction() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let subject = key("tracker", "acme", "card", "ABC-12");
    let action = fixture.observe(&subject);
    fixture.bind(&item, &subject, &action);
    let tag = fixture.tag("workflow", "review", "Review");
    let assignment = fixture.assign(&tag, TagTarget::Item(item.clone()));
    fixture
        .journal
        .record_gap(&fixture.session, 2, 1, 1000)
        .unwrap();
    let chosen = selection(&[item], &[]);
    let expected = collect_handoff(&fixture.journal, &chosen, Limits::default());
    let limits = Limits {
        records: 2,
        ..Limits::default()
    };
    let first = fixture.journal.handoff(&chosen, limits, None).unwrap();
    assert!(first.known_recording_gaps);
    assert!(first.truncated);
    let frozen = first.snapshot.clone();
    assert!(frozen.event_sequence() > 0 && frozen.map_sequence() > 0 && frozen.gap_sequence() > 0);
    let mut cursor = first.next.unwrap();
    let mut actual = first.records;

    // An independent writer commits new observations and a correction between pages.
    let mut writer = Journal::open(&fixture.temp.path()).unwrap();
    writer
        .record(&action.action_id, Outcome::Succeeded, Metadata::default())
        .unwrap();
    let retract = fixture.request(Mutation::Retract {
        claim_id: assignment.id.clone(),
        expected: assignment.mutation.target().unwrap(),
    });
    writer.apply_work_map(&retract).unwrap();
    writer.record_gap(&fixture.session, 5, 0, 2000).unwrap();
    let current = writer.work_map_snapshot().unwrap();
    assert!(current.event_sequence() > frozen.event_sequence());
    assert!(current.map_sequence() > frozen.map_sequence());
    assert!(current.gap_sequence() > frozen.gap_sequence());

    loop {
        let page = fixture
            .journal
            .handoff(&chosen, limits, Some(&cursor))
            .unwrap();
        assert_eq!(page.snapshot, frozen);
        assert!(page.records.len() <= limits.records);
        actual.extend(page.records);
        match page.next {
            Some(next) => {
                assert_ne!(next, cursor);
                cursor = next;
            }
            None => break,
        }
        assert!(actual.len() <= expected.len());
    }
    assert_eq!(
        serde_json::to_value(&actual).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    assert!(
        map_changes(&actual)
            .iter()
            .any(|change| change.request.id == assignment.id && !change.retracted)
    );
    assert!(!actual.iter().any(|record| matches!(record, HandoffRecord::Action { event, .. } if event.outcome == Outcome::Succeeded)));
    let fresh = collect_handoff(&fixture.journal, &chosen, Limits::default());
    assert!(
        map_changes(&fresh)
            .iter()
            .any(|change| change.request.id == assignment.id && change.retracted)
    );
    assert_eq!(
        fresh
            .iter()
            .filter(|record| matches!(record, HandoffRecord::Gap { .. }))
            .count(),
        2
    );
}

#[test]
fn handoff_count_and_whole_envelope_bytes_are_bounded_without_losing_records() {
    let mut fixture = Fixture::new();
    let items = (0..8).map(|_| fixture.item()).collect::<Vec<_>>();
    let chosen = selection(&items, &[]);
    let expected = collect_handoff(&fixture.journal, &chosen, Limits::default());
    let limits = Limits {
        records: 100,
        bytes: 6000,
    };
    let first = fixture.journal.handoff(&chosen, limits, None).unwrap();
    assert!(first.truncated);
    assert!(!first.records.is_empty());
    assert!(first.records.len() < limits.records);
    assert!(serde_json::to_vec(&first).unwrap().len() <= limits.bytes);
    let byte_pages = collect_handoff(&fixture.journal, &chosen, limits);
    assert_eq!(
        serde_json::to_value(&byte_pages).unwrap(),
        serde_json::to_value(&expected).unwrap()
    );
    let record_pages = collect_handoff(
        &fixture.journal,
        &chosen,
        Limits {
            records: 1,
            ..Limits::default()
        },
    );
    assert_eq!(
        serde_json::to_value(record_pages).unwrap(),
        serde_json::to_value(expected).unwrap()
    );

    let subject = key("tracker", "acme", "card", &"x".repeat(1500));
    let action = fixture.observe(&subject);
    let oversized = selection(&[], &[action.action_id]);
    assert!(
        fixture
            .journal
            .handoff(
                &oversized,
                Limits {
                    records: 1,
                    bytes: 1024
                },
                None
            )
            .is_err()
    );
    assert_eq!(
        collect_handoff(&fixture.journal, &oversized, Limits::default()).len(),
        1
    );
    for invalid in [
        Limits {
            records: 0,
            bytes: 4096,
        },
        Limits {
            records: 1001,
            bytes: 4096,
        },
        Limits {
            records: 1,
            bytes: 1023,
        },
        Limits {
            records: 1,
            bytes: MAX_PAGE_BYTES + 1,
        },
    ] {
        assert!(fixture.journal.handoff(&chosen, invalid, None).is_err());
    }
}

#[test]
fn handoff_cursors_reject_wrong_database_selection_and_invalid_positions() {
    let mut fixture = Fixture::new();
    let first_item = fixture.item();
    let second_item = fixture.item();
    let tag = fixture.tag("workflow", "review", "Review");
    fixture.assign(&tag, TagTarget::Item(first_item.clone()));
    let chosen = selection(&[first_item], &[]);
    let first = fixture
        .journal
        .handoff(
            &chosen,
            Limits {
                records: 1,
                ..Limits::default()
            },
            None,
        )
        .unwrap();
    let cursor = first.next.unwrap();
    assert!(
        fixture
            .journal
            .handoff(
                &selection(&[second_item], &[]),
                Limits::default(),
                Some(&cursor)
            )
            .is_err()
    );
    let other = Fixture::new();
    assert!(
        other
            .journal
            .handoff(&chosen, Limits::default(), Some(&cursor))
            .is_err()
    );
    for (field, invalid) in [
        ("stage", serde_json::json!(3)),
        ("after_sequence", serde_json::json!(-1)),
        ("after_sequence", serde_json::json!(i64::MAX)),
    ] {
        let mut json = serde_json::to_value(&cursor).unwrap();
        json[field] = invalid;
        let invalid = serde_json::from_value(json).unwrap();
        assert!(
            fixture
                .journal
                .handoff(&chosen, Limits::default(), Some(&invalid))
                .is_err()
        );
    }
    for field in ["event_sequence", "map_sequence", "gap_sequence"] {
        let mut json = serde_json::to_value(&cursor).unwrap();
        json["snapshot"][field] = serde_json::json!(i64::MAX);
        let invalid = serde_json::from_value(json).unwrap();
        assert!(
            fixture
                .journal
                .handoff(&chosen, Limits::default(), Some(&invalid))
                .is_err()
        );
    }
    assert!(
        fixture
            .journal
            .handoff(&Selection::default(), Limits::default(), None)
            .is_err()
    );
    assert!(
        fixture
            .journal
            .handoff(&selection(&[ItemId::new()], &[]), Limits::default(), None)
            .is_err()
    );
    assert!(
        fixture
            .journal
            .handoff(&selection(&[], &[ActionId::new()]), Limits::default(), None)
            .is_err()
    );
    let too_many = selection(
        &(0..=MAX_SELECTION)
            .map(|_| ItemId::new())
            .collect::<Vec<_>>(),
        &[],
    );
    assert!(
        fixture
            .journal
            .handoff(&too_many, Limits::default(), None)
            .is_err()
    );
}

#[test]
fn item_pagination_freezes_tag_membership_and_rejects_cross_filter_cursors() {
    let mut fixture = Fixture::new();
    let items = (0..3).map(|_| fixture.item()).collect::<Vec<_>>();
    let tag = fixture.tag("workflow", "review", "Review");
    let assignments = items
        .iter()
        .map(|item| fixture.assign(&tag, TagTarget::Item(item.clone())))
        .collect::<Vec<_>>();
    let filter = ItemFilter {
        all_tags: vec![tag],
    };
    let first = fixture.journal.query_items(&filter, 1, None).unwrap();
    assert_eq!(item_ids(&first), [items[0].clone()]);
    assert_eq!(first.items[0].kind.as_str(), "card");
    assert!(first.truncated);
    let cursor = first.next.clone().unwrap();
    let new_item = fixture.item();
    fixture.assign(&filter.all_tags[0], TagTarget::Item(new_item));
    fixture.apply(Mutation::Retract {
        claim_id: assignments[1].id.clone(),
        expected: assignments[1].mutation.target().unwrap(),
    });
    let second = fixture
        .journal
        .query_items(&filter, 10, Some(&cursor))
        .unwrap();
    assert_eq!(second.snapshot, first.snapshot);
    assert_eq!(item_ids(&second), items[1..]);
    assert!(!second.truncated);
    assert!(second.next.is_none());
    assert!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 10, Some(&cursor))
            .is_err()
    );
    assert!(
        Fixture::new()
            .journal
            .query_items(&filter, 10, Some(&cursor))
            .is_err()
    );
    for limit in [0, MAX_QUERY_LIMIT + 1] {
        assert!(fixture.journal.query_items(&filter, limit, None).is_err());
    }
    let too_many = ItemFilter {
        all_tags: (0..=MAX_TAG_FILTERS).map(|_| TagId::new()).collect(),
    };
    assert!(fixture.journal.query_items(&too_many, 10, None).is_err());
    let mut invalid = serde_json::to_value(cursor).unwrap();
    invalid["after_sequence"] = serde_json::json!(i64::MAX);
    let invalid = serde_json::from_value(invalid).unwrap();
    assert!(
        fixture
            .journal
            .query_items(&filter, 10, Some(&invalid))
            .is_err()
    );
}

#[test]
fn inferred_claims_remain_evidence_without_becoming_identity_or_authority() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let other = fixture.item();
    let subject = key("tracker", "acme", "card", "ABC-12");
    let action = fixture.observe(&subject);
    let tag = fixture.tag("workflow", "review", "Review");
    for mutation in [
        Mutation::BindSubject {
            item_id: item.clone(),
            subject: subject.clone(),
            from_sequence: action.sequence,
            through_sequence: None,
        },
        Mutation::AssignTag {
            tag_id: tag.clone(),
            target: TagTarget::Item(item.clone()),
        },
        Mutation::RelateItems {
            from: item.clone(),
            to: other,
            kind: RelationKind::DependsOn,
        },
    ] {
        let mut request = fixture.request(mutation);
        request.provenance.basis = Basis::Inferred;
        fixture.journal.apply_work_map(&request).unwrap();
    }
    let resolved = fixture
        .journal
        .resolve_subject(&subject, &action.action_id, None)
        .unwrap();
    assert_eq!(resolved.state, ResolutionState::Unbound);
    assert!(resolved.items.is_empty());
    assert!(
        fixture
            .journal
            .query_items(
                &ItemFilter {
                    all_tags: vec![tag]
                },
                10,
                None
            )
            .unwrap()
            .items
            .is_empty()
    );
    let records = collect_handoff(
        &fixture.journal,
        &selection(&[item], &[action.action_id]),
        Limits::default(),
    );
    assert_eq!(
        map_changes(&records)
            .iter()
            .filter(|change| change.request.provenance.basis == Basis::Inferred)
            .count(),
        3
    );
    assert!(records.iter().any(|record| matches!(
        record,
        HandoffRecord::Action {
            association: HistoryAssociation::DirectSelection,
            ..
        }
    )));
}

#[test]
fn card_collection_and_proposal_fixture_uses_only_explicit_edges() {
    let mut fixture = Fixture::new();
    let card = fixture.item();
    let collection = fixture.item_of_kind("collection");
    let proposal = fixture.item_of_kind("proposal");
    let card_key = key("tracker", "https://acme.example.test", "card", "ABC-12");
    let collection_key = key(
        "forge",
        "https://forge.example.test",
        "collection",
        "acme/api",
    );
    let proposal_key = key(
        "forge",
        "https://forge.example.test",
        "proposal",
        "acme/api!7",
    );
    let card_action = fixture.observe(&card_key);
    let collection_action = fixture.observe(&collection_key);
    let action = fixture
        .journal
        .begin(
            &fixture.session,
            ActionDescriptor::new("proposal.inspect", Origin::Plugin("forge".into()))
                .with_subject(SubjectSnapshot::new(
                    proposal_key.clone(),
                    SubjectRole::Primary,
                ))
                .with_subject(SubjectSnapshot::new(card_key.clone(), SubjectRole::Result))
                .with_subject(SubjectSnapshot::new(
                    collection_key.clone(),
                    SubjectRole::Context,
                )),
            Some(&collection_action.action_id),
        )
        .unwrap();
    let proposal_action = events(&fixture.journal, &action).remove(0);
    fixture
        .journal
        .record(
            &action,
            Outcome::Unknown,
            Metadata {
                reason: Some(Reason::NoTerminalObservation),
                ..Metadata::default()
            },
        )
        .unwrap();
    fixture.bind(&card, &card_key, &card_action);
    fixture.bind(&collection, &collection_key, &collection_action);
    fixture.bind(&proposal, &proposal_key, &proposal_action);
    let tag = fixture.tag("initiative", "api-review", "API review");
    for item in [&card, &collection, &proposal] {
        fixture.assign(&tag, TagTarget::Item(item.clone()));
    }
    let chosen = selection(&[card.clone(), collection.clone(), proposal.clone()], &[]);
    let before_edges = collect_handoff(&fixture.journal, &chosen, Limits::default());
    assert!(
        map_changes(&before_edges)
            .iter()
            .all(|change| !matches!(change.request.mutation, Mutation::RelateItems { .. }))
    );
    let references = fixture.apply(Mutation::RelateItems {
        from: proposal.clone(),
        to: card.clone(),
        kind: RelationKind::References,
    });
    let part_of = fixture.apply(Mutation::RelateItems {
        from: proposal.clone(),
        to: collection.clone(),
        kind: RelationKind::PartOf,
    });
    let records = collect_handoff(
        &fixture.journal,
        &chosen,
        Limits {
            records: 4,
            bytes: 20_000,
        },
    );
    let relations = map_changes(&records)
        .into_iter()
        .filter(|change| matches!(change.request.mutation, Mutation::RelateItems { .. }))
        .collect::<Vec<_>>();
    assert_eq!(relations.len(), 2);
    assert!(relations.iter().any(|change| change.request == references));
    assert!(relations.iter().any(|change| change.request == part_of));
    assert!(records.iter().any(|record| matches!(record, HandoffRecord::Action { event, .. } if event.action_id == action && event.outcome == Outcome::Unknown && event.parent_action_id == Some(collection_action.action_id.clone()))));
    assert_eq!(
        fixture
            .journal
            .query_items(
                &ItemFilter {
                    all_tags: vec![tag]
                },
                10,
                None
            )
            .unwrap()
            .items
            .len(),
        3
    );
    for mutation in [
        Mutation::RelateItems {
            from: card.clone(),
            to: card.clone(),
            kind: RelationKind::References,
        },
        Mutation::RelateItems {
            from: card.clone(),
            to: ItemId::new(),
            kind: RelationKind::References,
        },
        Mutation::AssignTag {
            tag_id: TagId::new(),
            target: TagTarget::Item(card.clone()),
        },
    ] {
        let request = fixture.request(mutation);
        assert!(fixture.journal.apply_work_map(&request).is_err());
    }
    fixture.apply(Mutation::Retract {
        claim_id: references.id.clone(),
        expected: AssertionTarget::Relation {
            from: proposal,
            to: card,
            kind: RelationKind::References,
        },
    });
    let corrected = collect_handoff(&fixture.journal, &chosen, Limits::default());
    assert!(
        map_changes(&corrected)
            .iter()
            .any(|change| change.request.id == references.id && change.retracted)
    );
}

#[test]
fn v4_migration_preserves_legacy_observations_without_inferred_items() {
    let temp = TestDir::new();
    private_directory(&temp.0).unwrap();
    let connection = Connection::open(temp.path()).unwrap();
    for migration in [schema::V1, schema::V2, schema::V3, schema::V4] {
        connection.execute_batch(migration).unwrap();
    }
    connection
        .pragma_update(None, "application_id", APPLICATION_ID)
        .unwrap();
    connection.pragma_update(None, "user_version", 4).unwrap();
    let session = SessionId::new();
    let action = ActionId::new();
    let event = EventId::new();
    connection
        .execute(
            "INSERT INTO sessions(id,started_at_ms,process_id) VALUES (?1,1000,1)",
            [session.as_str()],
        )
        .unwrap();
    connection.execute("INSERT INTO actions(id,session_id,root_action_id,correlation_id,kind,origin_kind,origin_id,created_at_ms) VALUES (?1,?2,?1,?1,'card.inspect','plugin','tracker',1000)", params![action.as_str(), session.as_str()]).unwrap();
    connection.execute("INSERT INTO action_events(id,action_id,occurred_at_ms,outcome,metadata_json) VALUES (?1,?2,1000,'requested','{}')", params![event.as_str(), action.as_str()]).unwrap();
    connection.execute("INSERT INTO subjects(id,provider,instance,kind,external_id) VALUES (1,'tracker','acme','card','ABC-12')", []).unwrap();
    connection.execute("INSERT INTO action_subjects(action_id,ordinal,subject_id,role) VALUES (?1,0,1,'primary')", [action.as_str()]).unwrap();
    drop(connection);

    let journal = Journal::open(&temp.path()).unwrap();
    let cut = journal.work_map_snapshot().unwrap();
    assert_eq!(cut.map_sequence(), 0);
    assert!(
        journal
            .query_items(&ItemFilter::default(), 10, None)
            .unwrap()
            .items
            .is_empty()
    );
    let subject = key("tracker", "acme", "card", "ABC-12");
    let resolution = journal.resolve_subject(&subject, &action, None).unwrap();
    assert_eq!(resolution.state, ResolutionState::Unbound);
    let original = events(&journal, &action);
    assert_eq!(original.len(), 1);
    assert_eq!(original[0].event_id, event);
    assert_eq!(original[0].session_id, session);
    assert_eq!(original[0].subjects[0].key, subject);
    assert!(original[0].observation_time_estimated);
    let records = collect_handoff(&journal, &selection(&[], &[action]), Limits::default());
    assert_eq!(records.len(), 1);
    assert!(
        matches!(&records[0], HandoffRecord::Action { event: observed, association: HistoryAssociation::DirectSelection, .. } if observed.event_id == event && observed.observation_time_estimated)
    );
    drop(journal);
    assert_eq!(
        Journal::open(&temp.path())
            .unwrap()
            .work_map_snapshot()
            .unwrap(),
        cut
    );
    assert_ne!(
        Fixture::new()
            .journal
            .work_map_snapshot()
            .unwrap()
            .database_id(),
        cut.database_id()
    );
}

#[test]
fn privacy_validation_is_repeated_after_deserialization_without_echoing_secrets() {
    let mut fixture = Fixture::new();
    for invalid in [
        "",
        " leading",
        "trailing ",
        "line\nbreak",
        "password=do-not-echo",
        "api_key=do-not-echo",
    ] {
        assert!(SemanticLabel::new(invalid).is_err());
    }
    assert!(SemanticLabel::new(&"x".repeat(129)).is_err());
    assert!(SemanticLabel::new(&"é".repeat(65)).is_err());
    assert!(SemanticLabel::new("Review ready").is_ok());
    for invalid in ["", "Review", "two words", "token=private", "line\nbreak"] {
        assert!(TagKey::new(invalid).is_err());
    }
    assert!(TagKey::new(&"x".repeat(97)).is_err());
    let original = fixture.request(Mutation::CreateTag {
        tag_id: TagId::new(),
        namespace: TagKey::new("workflow").unwrap(),
        key: TagKey::new("review").unwrap(),
        label: SemanticLabel::new("Review").unwrap(),
    });
    let before = fixture.journal.work_map_snapshot().unwrap();
    let cases = [
        ("label", "password=do-not-echo".to_owned()),
        ("label", "bad\nlabel".to_owned()),
        ("label", "x".repeat(129)),
        ("namespace", "free text".to_owned()),
        ("key", "Uppercase".to_owned()),
        ("tag_id", "not-a-uuid".to_owned()),
    ];
    for (field, value) in cases {
        let mut encoded = serde_json::to_value(&original).unwrap();
        encoded["mutation"][field] = serde_json::Value::String(value);
        let bypass: ChangeRequest = serde_json::from_value(encoded).unwrap();
        let error = fixture.journal.apply_work_map(&bypass).unwrap_err();
        assert!(!format!("{error:#}").contains("do-not-echo"));
        assert_eq!(fixture.journal.work_map_snapshot().unwrap(), before);
    }
    let mut encoded = serde_json::to_value(&original).unwrap();
    encoded["mutation"]["body"] = serde_json::json!("arbitrary service text");
    assert!(serde_json::from_value::<ChangeRequest>(encoded).is_err());
    let mut encoded = serde_json::to_value(&original).unwrap();
    encoded["provenance"]["producer"] = serde_json::json!("forged caller attribution");
    assert!(serde_json::from_value::<ChangeRequest>(encoded).is_err());

    let item = fixture.item();
    let request = fixture.request(Mutation::BindSubject {
        item_id: item,
        subject: key("tracker", "acme", "card", "ABC-12"),
        from_sequence: 1,
        through_sequence: None,
    });
    for (field, value) in [
        (
            "instance",
            "https://user:do-not-echo@example.test/?token=private",
        ),
        ("id", "password=do-not-echo"),
        ("provider", "free text"),
    ] {
        let mut encoded = serde_json::to_value(&request).unwrap();
        encoded["mutation"]["subject"][field] = serde_json::json!(value);
        let bypass = serde_json::from_value(encoded).unwrap();
        let error = fixture.journal.apply_work_map(&bypass).unwrap_err();
        assert!(!format!("{error:#}").contains("do-not-echo"));
    }
    for timestamp in [-1, MAX_TIMESTAMP_MS + 1] {
        let mut bypass = original.clone();
        bypass.observed_at_ms = timestamp;
        assert!(fixture.journal.apply_work_map(&bypass).is_err());
    }
}

#[test]
fn item_kind_is_immutable_and_fact_tables_reject_updates_and_deletes() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let other = fixture.item();
    let subject = key("tracker", "acme", "card", "ABC-12");
    let action = fixture.observe(&subject);
    fixture.bind(&item, &subject, &action);
    let tag = fixture.tag("workflow", "review", "Review");
    fixture.assign(&tag, TagTarget::Item(item.clone()));
    fixture.apply(Mutation::RelateItems {
        from: item.clone(),
        to: other,
        kind: RelationKind::References,
    });
    let different_kind = fixture.request(Mutation::CreateItem {
        item_id: item.clone(),
        kind: StableCode::new("collection"),
    });
    assert!(fixture.journal.apply_work_map(&different_kind).is_err());
    let wrong_key = key("forge", "acme", "collection", "acme/api");
    let wrong = fixture.request(Mutation::BindSubject {
        item_id: item,
        subject: wrong_key,
        from_sequence: action.sequence,
        through_sequence: None,
    });
    assert!(fixture.journal.apply_work_map(&wrong).is_err());
    for (table, column) in [
        ("work_map_store", "id"),
        ("work_changes", "id"),
        ("work_items", "id"),
        ("work_item_bindings", "change_id"),
        ("work_tags", "id"),
        ("work_tag_labels", "change_id"),
        ("work_tag_assignments", "change_id"),
        ("work_relations", "change_id"),
    ] {
        for sql in [
            format!("UPDATE {table} SET {column}={column}"),
            format!("DELETE FROM {table}"),
        ] {
            assert!(
                fixture.journal.connection.execute(&sql, []).is_err(),
                "{sql}"
            );
        }
    }
    assert!(
        fixture
            .journal
            .connection
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()
            .unwrap()
            .is_none()
    );
}

#[test]
fn forged_event_highwaters_cannot_exclude_map_target_or_binding_dependencies() {
    for dependency in ["action_target", "binding_start", "binding_end"] {
        let mut fixture = Fixture::new();
        let item = fixture.item();
        fixture.item();
        let tag = fixture.tag("workflow", "review", "Review");
        fixture.assign(&tag, TagTarget::Item(item.clone()));
        let old_cut = fixture.journal.work_map_snapshot().unwrap();
        let subject = key("tracker", "acme", "card", "ABC-12");
        let later = fixture.observe(&subject);
        fixture
            .journal
            .record(&later.action_id, Outcome::Succeeded, Metadata::default())
            .unwrap();
        let later_end = fixture
            .journal
            .work_map_snapshot()
            .unwrap()
            .event_sequence();
        let mutation = match dependency {
            "action_target" => Mutation::AssignTag {
                tag_id: tag,
                target: TagTarget::Action(later.action_id),
            },
            "binding_start" => Mutation::BindSubject {
                item_id: item.clone(),
                subject: subject.clone(),
                from_sequence: later.sequence,
                through_sequence: None,
            },
            _ => Mutation::BindSubject {
                item_id: item.clone(),
                subject: subject.clone(),
                from_sequence: old_cut.event_sequence(),
                through_sequence: Some(later_end),
            },
        };
        // All attribution predates the target/range dependency. Checking only
        // recording and evidence sources would accept the forged snapshot.
        let request = fixture.apply(mutation);
        assert_eq!(request.provenance.basis, Basis::UserAsserted);
        assert_eq!(request.provenance.recording_action_id, fixture.recorder);
        assert_eq!(request.provenance.evidence_event_id, fixture.evidence);

        let chosen = selection(std::slice::from_ref(&item), &[]);
        let page = fixture
            .journal
            .handoff(
                &chosen,
                Limits {
                    records: 1,
                    ..Limits::default()
                },
                None,
            )
            .unwrap();
        let mut handoff_json = serde_json::to_value(page.next.unwrap()).unwrap();
        handoff_json["snapshot"]["event_sequence"] = serde_json::json!(old_cut.event_sequence());
        let forged_handoff = serde_json::from_value(handoff_json).unwrap();
        assert!(
            fixture
                .journal
                .handoff(&chosen, Limits::default(), Some(&forged_handoff))
                .is_err(),
            "handoff accepted missing {dependency} dependency"
        );

        let items = fixture
            .journal
            .query_items(&ItemFilter::default(), 1, None)
            .unwrap();
        let mut item_json = serde_json::to_value(items.next.unwrap()).unwrap();
        item_json["snapshot"]["event_sequence"] = serde_json::json!(old_cut.event_sequence());
        let forged_items = serde_json::from_value(item_json).unwrap();
        assert!(
            fixture
                .journal
                .query_items(&ItemFilter::default(), 1, Some(&forged_items))
                .is_err(),
            "item cursor accepted missing {dependency} dependency"
        );

        let mut snapshot_json = serde_json::to_value(page.snapshot).unwrap();
        snapshot_json["event_sequence"] = serde_json::json!(old_cut.event_sequence());
        let forged_snapshot = serde_json::from_value(snapshot_json).unwrap();
        assert!(
            fixture
                .journal
                .resolve_subject(&subject, &fixture.recorder, Some(&forged_snapshot))
                .is_err(),
            "resolution accepted missing {dependency} dependency"
        );
    }
}

#[test]
fn registered_secret_values_and_shapes_are_rejected_in_annotations_and_handoffs() {
    let mut fixture = Fixture::new();
    // Unique synthetic registrations do not change other parallel tests' words.
    let prefix = format!("wmshape-{}-", Uuid::new_v4().simple());
    let shaped = format!("{prefix}abcdefgh");
    let secret = format!("wmsecret{}", Uuid::new_v4().simple());
    crate::log::token_shape(&prefix);
    crate::log::secret(&secret);
    let original = fixture.request(Mutation::CreateTag {
        tag_id: TagId::new(),
        namespace: TagKey::new("workflow").unwrap(),
        key: TagKey::new("review").unwrap(),
        label: SemanticLabel::new("Review").unwrap(),
    });
    let before = fixture.journal.work_map_snapshot().unwrap();
    for value in [&shaped, &secret] {
        let embedded = format!("group-{value}");
        for error in [
            SemanticLabel::new(&embedded).unwrap_err(),
            TagKey::new(&embedded).unwrap_err(),
        ] {
            assert!(!format!("{error:#}").contains(value.as_str()));
        }
        for field in ["namespace", "key", "label"] {
            let mut encoded = serde_json::to_value(&original).unwrap();
            encoded["mutation"][field] = serde_json::json!(embedded);
            let bypass = serde_json::from_value(encoded).unwrap();
            let error = fixture.journal.apply_work_map(&bypass).unwrap_err();
            assert!(!format!("{error:#}").contains(value.as_str()));
            assert_eq!(fixture.journal.work_map_snapshot().unwrap(), before);
        }
    }

    let item = fixture.item();
    let newly_sensitive = format!("wmstored{}", Uuid::new_v4().simple());
    let tag = fixture.tag("workflow", "previously-safe", &newly_sensitive);
    fixture.assign(&tag, TagTarget::Item(item.clone()));
    let chosen = selection(&[item], &[]);
    let before_registration = fixture
        .journal
        .handoff(&chosen, Limits::default(), None)
        .unwrap();
    assert!(
        map_changes(&before_registration.records)
            .iter()
            .any(|change| matches!(
                &change.request.mutation,
                Mutation::CreateTag { label, .. } if label.as_str() == newly_sensitive
            ))
    );
    crate::log::secret(&newly_sensitive);
    let error = fixture
        .journal
        .handoff(&chosen, Limits::default(), None)
        .unwrap_err();
    assert!(!format!("{error:#}").contains(&newly_sensitive));
    // Failed export preserves history; it never silently drops the offending fact.
    assert_eq!(
        fixture.journal.work_map_snapshot().unwrap(),
        before_registration.snapshot
    );
}

#[test]
fn tag_lookup_recovers_stable_identity_across_rename_restart_and_snapshots() {
    let mut fixture = Fixture::new();
    let namespace = TagKey::new("workflow").unwrap();
    let key = TagKey::new("review").unwrap();
    let before = fixture.journal.work_map_snapshot().unwrap();
    assert!(
        fixture
            .journal
            .find_tag(&namespace, &key, None)
            .unwrap()
            .is_none()
    );
    let tag_id = TagId::new();
    let creation = fixture.apply(Mutation::CreateTag {
        tag_id: tag_id.clone(),
        namespace: namespace.clone(),
        key: key.clone(),
        label: SemanticLabel::new("Needs review").unwrap(),
    });
    let created = fixture
        .journal
        .find_tag(&namespace, &key, None)
        .unwrap()
        .unwrap();
    assert_eq!(created.tag_id, tag_id);
    assert_eq!(created.namespace, namespace);
    assert_eq!(created.key, key);
    assert_eq!(created.creation.version, VERSION);
    assert_eq!(created.creation.request, creation);
    assert!(!created.creation.retracted);
    assert_eq!(
        created.snapshot,
        fixture.journal.work_map_snapshot().unwrap()
    );
    assert!(
        fixture
            .journal
            .find_tag(&namespace, &key, Some(&before))
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .journal
            .find_tag(&TagKey::new("other").unwrap(), &key, None)
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .journal
            .find_tag(&namespace, &TagKey::new("missing").unwrap(), None)
            .unwrap()
            .is_none()
    );

    fixture.apply(Mutation::RenameTag {
        tag_id: tag_id.clone(),
        label: SemanticLabel::new("Ready for review").unwrap(),
    });
    let renamed = fixture
        .journal
        .find_tag(&namespace, &key, None)
        .unwrap()
        .unwrap();
    assert_eq!(renamed.tag_id, tag_id);
    assert_eq!(renamed.creation.request, creation);
    assert!(renamed.snapshot.map_sequence() > created.snapshot.map_sequence());
    let historical = fixture
        .journal
        .find_tag(&namespace, &key, Some(&created.snapshot))
        .unwrap()
        .unwrap();
    assert_eq!(historical.snapshot, created.snapshot);
    assert_eq!(historical.creation.request, creation);
    fixture.journal.flush().unwrap();
    let reopened = Journal::open(&fixture.temp.path()).unwrap();
    let recovered = reopened.find_tag(&namespace, &key, None).unwrap().unwrap();
    assert_eq!(recovered.tag_id, tag_id);
    assert_eq!(recovered.namespace, namespace);
    assert_eq!(recovered.key, key);
    assert_eq!(recovered.creation.request, creation);
    assert_eq!(recovered.creation.sequence, created.creation.sequence);
    assert_eq!(recovered.snapshot, renamed.snapshot);
    let other = Fixture::new().journal.work_map_snapshot().unwrap();
    assert!(reopened.find_tag(&namespace, &key, Some(&other)).is_err());
}

#[test]
fn tag_lookup_revalidates_saved_creation_label_namespace_and_key_without_echoing() {
    for field in ["label", "namespace", "key"] {
        let mut fixture = Fixture::new();
        let newly_sensitive = format!("wmlookup{}", Uuid::new_v4().simple());
        let namespace = TagKey::new(if field == "namespace" {
            &newly_sensitive
        } else {
            "workflow"
        })
        .unwrap();
        let key = TagKey::new(if field == "key" {
            &newly_sensitive
        } else {
            "review"
        })
        .unwrap();
        let label = SemanticLabel::new(if field == "label" {
            &newly_sensitive
        } else {
            "Review"
        })
        .unwrap();
        let tag_id = TagId::new();
        fixture.apply(Mutation::CreateTag {
            tag_id: tag_id.clone(),
            namespace: namespace.clone(),
            key: key.clone(),
            label,
        });
        // A later safe display label cannot hide unsafe historical creation data.
        fixture.apply(Mutation::RenameTag {
            tag_id: tag_id.clone(),
            label: SemanticLabel::new("Ready for review").unwrap(),
        });
        let initial = fixture
            .journal
            .find_tag(&namespace, &key, None)
            .unwrap()
            .unwrap();
        assert_eq!(initial.tag_id, tag_id);
        crate::log::secret(&newly_sensitive);
        let error = fixture
            .journal
            .find_tag(&namespace, &key, None)
            .unwrap_err();
        assert!(!format!("{error:#}").contains(&newly_sensitive));
        let historical_error = fixture
            .journal
            .find_tag(&namespace, &key, Some(&initial.snapshot))
            .unwrap_err();
        assert!(!format!("{historical_error:#}").contains(&newly_sensitive));
        assert_eq!(
            fixture.journal.work_map_snapshot().unwrap(),
            initial.snapshot
        );
    }
}

#[test]
fn handoff_checks_raw_subject_strings_before_json_escaping_registered_secrets() {
    let mut fixture = Fixture::new();
    let raw_secret = format!("wmraw{}\"quoted\\tail", Uuid::new_v4().simple());
    let path = format!("/tmp/work-map/{raw_secret}");
    let subject = key("local", "/work/acme", "path", &path);
    let action = fixture.observe(&subject);
    let chosen = selection(&[], std::slice::from_ref(&action.action_id));
    let before = fixture
        .journal
        .handoff(&chosen, Limits::default(), None)
        .unwrap();
    assert!(before.records.iter().any(|record| matches!(
        record,
        HandoffRecord::Action { event, .. } if event.subjects[0].key.id == path
    )));
    // Searching serialized JSON would miss this exact registered raw value.
    assert!(
        !serde_json::to_string(&before)
            .unwrap()
            .contains(&raw_secret)
    );
    crate::log::secret(&raw_secret);
    let error = fixture
        .journal
        .handoff(&chosen, Limits::default(), None)
        .unwrap_err();
    let message = format!("{error:#}");
    assert!(!message.contains(&raw_secret));
    assert!(!message.contains(&serde_json::to_string(&raw_secret).unwrap()));
    assert!(!message.contains("wmraw"));
    // The new export guard neither rewrites nor changes the legacy journal API.
    assert_eq!(
        events(&fixture.journal, &action.action_id)[0].subjects[0]
            .key
            .id,
        path
    );
}

#[test]
fn map_resolution_and_tag_lookup_recheck_registered_secrets_in_evidence_metadata() {
    for field in ["url", "command"] {
        let mut fixture = Fixture::new();
        let item = fixture.item();
        let secret = format!("wmevidence{}", Uuid::new_v4().simple());
        let metadata = if field == "url" {
            Metadata {
                url: Some(SafeUrl::parse(&format!("https://example.test/{secret}")).unwrap()),
                ..Metadata::default()
            }
        } else {
            Metadata {
                command: Some(StableCode::try_new(&format!("inspect.{secret}")).unwrap()),
                ..Metadata::default()
            }
        };
        let subject = key("tracker", "acme", "card", "ABC-12");
        let action = fixture
            .journal
            .begin(
                &fixture.session,
                ActionDescriptor::new("card.inspect", Origin::Plugin("tracker".into()))
                    .with_subject(SubjectSnapshot::new(subject.clone(), SubjectRole::Primary))
                    .with_metadata(metadata.clone()),
                None,
            )
            .unwrap();
        let event = events(&fixture.journal, &action).remove(0);
        fixture.bind(&item, &subject, &event);
        fixture.evidence = event.event_id;
        let tag = fixture.tag("workflow", "review", "Review");
        fixture.assign(&tag, TagTarget::Item(item.clone()));
        let namespace = TagKey::new("workflow").unwrap();
        let tag_key = TagKey::new("review").unwrap();
        let chosen = selection(&[item], &[]);
        let before = fixture
            .journal
            .handoff(&chosen, Limits::default(), None)
            .unwrap();
        assert!(
            map_changes(&before.records)
                .iter()
                .any(|change| change.evidence.metadata == metadata)
        );
        let resolution = fixture
            .journal
            .resolve_subject(&subject, &action, None)
            .unwrap();
        assert_eq!(resolution.claims[0].evidence.metadata, metadata);
        let identity = fixture
            .journal
            .find_tag(&namespace, &tag_key, None)
            .unwrap()
            .unwrap();
        assert_eq!(identity.creation.evidence.metadata, metadata);
        crate::log::secret(&secret);
        let errors = [
            fixture
                .journal
                .handoff(&chosen, Limits::default(), None)
                .unwrap_err(),
            fixture
                .journal
                .resolve_subject(&subject, &action, None)
                .unwrap_err(),
            fixture
                .journal
                .find_tag(&namespace, &tag_key, None)
                .unwrap_err(),
        ];
        for error in errors {
            assert!(!format!("{error:#}").contains(&secret));
        }
        assert_eq!(events(&fixture.journal, &action)[0].metadata, metadata);
        assert_eq!(
            fixture.journal.work_map_snapshot().unwrap(),
            before.snapshot
        );
    }
}

#[test]
fn context_only_item_handoff_preserves_supplied_reference_evidence() {
    let mut fixture = Fixture::new();
    let item = fixture.item_of_kind("note");
    let context = key("catalog", "one", "note", "42");
    let action = fixture
        .journal
        .begin(
            &fixture.session,
            ActionDescriptor::new("plugin.action", Origin::Plugin("reader".into()))
                .with_subject(SubjectSnapshot::new(context.clone(), SubjectRole::Context)),
            None,
        )
        .unwrap();
    let observation = events(&fixture.journal, &action).remove(0);
    let before_binding = collect_handoff(
        &fixture.journal,
        &selection(std::slice::from_ref(&item), &[]),
        Limits::default(),
    );
    assert!(!before_binding.iter().any(
        |record| matches!(record, HandoffRecord::Action { event, .. } if event.action_id == action)
    ));
    // Only the deliberately supplied binding associates the item with this action.
    fixture.bind(&item, &context, &observation);
    let cut = fixture.journal.work_map_snapshot().unwrap();
    let handoff = collect_handoff(
        &fixture.journal,
        &selection(&[item], &[]),
        Limits::default(),
    );
    let event = handoff
        .iter()
        .find_map(|record| match record {
            HandoffRecord::Action {
                event,
                association: HistoryAssociation::ExplicitBinding,
                ..
            } if event.action_id == action => Some(event),
            _ => None,
        })
        .unwrap();
    assert_eq!(event.outcome, Outcome::Requested);
    assert_eq!(
        event.subjects,
        [SubjectSnapshot::new(context, SubjectRole::Context)]
    );
    assert!(map_changes(&handoff).iter().all(|change| !matches!(
        change.request.mutation,
        Mutation::RelateItems { .. } | Mutation::AssignTag { .. }
    )));
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), cut);
}

#[test]
fn tag_catalog_pages_authored_definitions_independently_of_items_and_restarts() {
    let mut fixture = Fixture::new();
    let namespace = TagKey::new("work.group").unwrap();
    let mut expected = Vec::new();
    // More definitions than any permitted catalog page; no assignments exist.
    for index in (0..MAX_TAG_QUERY_LIMIT + 3).rev() {
        let key = format!("group-{index:03}");
        let id = fixture.tag(namespace.as_str(), &key, "Same display name");
        expected.push((key, id));
    }
    fixture.tag("work.title", "group-000", "Same display name");
    for basis in [Basis::Inferred, Basis::Observed] {
        let mut request = fixture.request(Mutation::CreateTag {
            tag_id: TagId::new(),
            namespace: namespace.clone(),
            key: TagKey::new(basis.as_str()).unwrap(),
            label: SemanticLabel::new("Not authored").unwrap(),
        });
        request.provenance.basis = basis;
        fixture.journal.apply_work_map(&request).unwrap();
    }
    assert!(
        fixture
            .journal
            .query_items(&ItemFilter::default(), 1, None)
            .unwrap()
            .items
            .is_empty()
    );
    expected.sort_by(|a, b| a.0.cmp(&b.0));
    let first = fixture
        .journal
        .query_tags(&namespace, MAX_TAG_QUERY_LIMIT, None)
        .unwrap();
    assert_eq!(first.tags.len(), MAX_TAG_QUERY_LIMIT);
    assert!(first.truncated);
    assert!(
        first
            .tags
            .iter()
            .all(|tag| tag.renames.is_empty() && !tag.labels_truncated)
    );
    let cursor: TagCursor =
        serde_json::from_value(serde_json::to_value(first.next.as_ref().unwrap()).unwrap())
            .unwrap();
    let journal = Journal::open(&fixture.temp.path()).unwrap();
    let second = journal
        .query_tags(&namespace, MAX_TAG_QUERY_LIMIT, Some(&cursor))
        .unwrap();
    assert_eq!(second.snapshot, first.snapshot);
    assert!(!second.truncated);
    assert!(second.next.is_none());
    let actual = first
        .tags
        .iter()
        .chain(&second.tags)
        .map(|tag| (tag.key.as_str().to_owned(), tag.tag_id.clone()))
        .collect::<Vec<_>>();
    assert_eq!(actual, expected);
    assert!(encode_safe_output(&first).unwrap().len() <= MAX_PAGE_BYTES);
}

#[test]
fn tag_catalog_cut_pins_definitions_labels_and_filtered_items_under_later_writes() {
    let mut fixture = Fixture::new();
    let namespace = TagKey::new("work.group").unwrap();
    let first_tag = fixture.tag("work.group", "a", "Same name");
    let second_tag = fixture.tag("work.group", "b", "Same name");
    let item = fixture.item();
    fixture.assign(&first_tag, TagTarget::Item(item));
    let renamed = fixture.apply(Mutation::RenameTag {
        tag_id: second_tag.clone(),
        label: SemanticLabel::new("Original rename").unwrap(),
    });
    let first = fixture.journal.query_tags(&namespace, 1, None).unwrap();
    let before_items = fixture
        .journal
        .query_items_at(
            &ItemFilter {
                all_tags: vec![second_tag.clone()],
            },
            1,
            None,
            Some(&first.snapshot),
        )
        .unwrap();
    assert!(before_items.items.is_empty());
    fixture.apply(Mutation::Retract {
        claim_id: renamed.id.clone(),
        expected: AssertionTarget::LabelAssertion {
            tag_id: second_tag.clone(),
        },
    });
    fixture.apply(Mutation::RenameTag {
        tag_id: second_tag.clone(),
        label: SemanticLabel::new("Later rename").unwrap(),
    });
    fixture.tag("work.group", "aa", "Inserted before cursor target");
    let later_item = fixture.item();
    fixture.assign(&second_tag, TagTarget::Item(later_item.clone()));
    let second = fixture
        .journal
        .query_tags_at(&namespace, 1, first.next.as_ref(), Some(&first.snapshot))
        .unwrap();
    assert_eq!(second.snapshot, first.snapshot);
    assert_eq!(second.tags[0].tag_id, second_tag);
    assert_eq!(second.tags[0].renames.len(), 1);
    assert_eq!(second.tags[0].renames[0].change_id, renamed.id);
    assert!(!second.truncated);
    assert!(
        fixture
            .journal
            .query_items_at(
                &ItemFilter {
                    all_tags: vec![second_tag.clone()]
                },
                1,
                None,
                Some(&second.snapshot)
            )
            .unwrap()
            .items
            .is_empty()
    );
    let refreshed = fixture.journal.query_tags(&namespace, 10, None).unwrap();
    assert_eq!(refreshed.tags.len(), 3);
    assert_eq!(refreshed.tags[2].renames[0].label.as_str(), "Later rename");
    assert_eq!(
        item_ids(
            &fixture
                .journal
                .query_items_at(
                    &ItemFilter {
                        all_tags: vec![second_tag]
                    },
                    1,
                    None,
                    Some(&refreshed.snapshot)
                )
                .unwrap()
        ),
        vec![later_item]
    );
}

#[test]
fn tag_catalog_labels_are_bounded_active_authored_claims_not_a_canonical_name() {
    let mut fixture = Fixture::new();
    let namespace = TagKey::new("work.group").unwrap();
    let tag_id = fixture.tag("work.group", "group", "Creation label");
    let retired = fixture.apply(Mutation::RenameTag {
        tag_id: tag_id.clone(),
        label: SemanticLabel::new("Retired").unwrap(),
    });
    fixture.apply(Mutation::Retract {
        claim_id: retired.id.clone(),
        expected: AssertionTarget::LabelAssertion {
            tag_id: tag_id.clone(),
        },
    });
    for basis in [Basis::Inferred, Basis::Observed] {
        let mut request = fixture.request(Mutation::RenameTag {
            tag_id: tag_id.clone(),
            label: SemanticLabel::new("Not authored").unwrap(),
        });
        request.provenance.basis = basis;
        fixture.journal.apply_work_map(&request).unwrap();
    }
    let no_rename = fixture.journal.query_tags(&namespace, 1, None).unwrap();
    assert!(no_rename.tags[0].renames.is_empty());
    assert!(!no_rename.tags[0].labels_truncated);
    let mut live = Vec::new();
    for index in 0..TAG_LABEL_LIMIT + 2 {
        live.push(fixture.apply(Mutation::RenameTag {
            tag_id: tag_id.clone(),
            label: SemanticLabel::new(&format!("Claim {index}")).unwrap(),
        }));
    }
    let page = fixture.journal.query_tags(&namespace, 1, None).unwrap();
    let entry = &page.tags[0];
    assert_eq!(entry.creation.label.as_str(), "Creation label");
    assert_eq!(entry.renames.len(), TAG_LABEL_LIMIT);
    assert!(entry.labels_truncated);
    assert!(
        !page.truncated,
        "label and definition bounds are independent"
    );
    assert!(entry.renames.iter().all(
        |claim| claim.provenance.basis == Basis::UserAsserted && claim.change_id != retired.id
    ));
    assert!(
        entry
            .renames
            .windows(2)
            .all(|pair| pair[0].sequence < pair[1].sequence)
    );
    for request in live.iter().take(3) {
        fixture.apply(Mutation::Retract {
            claim_id: request.id.clone(),
            expected: AssertionTarget::LabelAssertion {
                tag_id: tag_id.clone(),
            },
        });
    }
    let refreshed = fixture.journal.query_tags(&namespace, 1, None).unwrap();
    assert_eq!(refreshed.tags[0].renames.len(), TAG_LABEL_LIMIT - 1);
    assert!(!refreshed.tags[0].labels_truncated);
    let original = fixture
        .journal
        .query_tags_at(&namespace, 1, None, Some(&page.snapshot))
        .unwrap();
    assert_eq!(
        encode_safe_output(&original).unwrap(),
        encode_safe_output(&page).unwrap()
    );
}

#[test]
fn tag_catalog_rejects_wrong_namespace_database_snapshot_and_invalid_bounds() {
    let mut fixture = Fixture::new();
    let namespace = TagKey::new("work.group").unwrap();
    fixture.tag("work.group", "a", "First");
    fixture.tag("work.group", "b", "Second");
    let page = fixture.journal.query_tags(&namespace, 1, None).unwrap();
    let cursor = page.next.as_ref().unwrap();
    assert!(
        fixture
            .journal
            .query_tags(&TagKey::new("work.title").unwrap(), 1, Some(cursor))
            .is_err()
    );
    let other = Fixture::new();
    assert!(
        other
            .journal
            .query_tags(&namespace, 1, Some(cursor))
            .is_err()
    );
    fixture.tag("work.group", "c", "Third");
    let latest = fixture.journal.work_map_snapshot().unwrap();
    assert!(
        fixture
            .journal
            .query_tags_at(&namespace, 1, Some(cursor), Some(&latest))
            .is_err()
    );
    for limit in [0, MAX_TAG_QUERY_LIMIT + 1] {
        assert!(fixture.journal.query_tags(&namespace, limit, None).is_err());
    }
    let invalid: TagKey = serde_json::from_value(serde_json::json!("WORK.GROUP")).unwrap();
    assert!(fixture.journal.query_tags(&invalid, 1, None).is_err());
    let mut forged = serde_json::to_value(cursor).unwrap();
    forged["after_key"] = serde_json::json!("");
    assert!(
        fixture
            .journal
            .query_tags(
                &namespace,
                1,
                Some(&serde_json::from_value(forged).unwrap())
            )
            .is_err()
    );
    let item_page = fixture
        .journal
        .query_items(&ItemFilter::default(), 1, None)
        .unwrap();
    assert!(serde_json::from_value::<TagCursor>(serde_json::to_value(item_page).unwrap()).is_err());
}

#[test]
fn tag_catalog_empty_and_populated_reads_are_query_only_and_never_audit() {
    let mut fixture = Fixture::new();
    let namespace = TagKey::new("work.group").unwrap();
    let empty = fixture.journal.query_tags(&namespace, 1, None).unwrap();
    assert!(empty.tags.is_empty());
    assert!(!empty.truncated);
    assert!(empty.next.is_none());
    let tag = fixture.tag("work.group", "group", "Empty reusable group");
    let item = fixture.item();
    let assignment = fixture.assign(&tag, TagTarget::Item(item.clone()));
    fixture.apply(Mutation::Retract {
        claim_id: assignment.id,
        expected: AssertionTarget::TagAssignment {
            tag_id: tag.clone(),
            target: TagTarget::Item(item.clone()),
        },
    });
    let mut inferred = fixture.request(Mutation::AssignTag {
        tag_id: tag.clone(),
        target: TagTarget::Item(item),
    });
    inferred.provenance.basis = Basis::Inferred;
    fixture.journal.apply_work_map(&inferred).unwrap();
    assert!(
        fixture
            .journal
            .query_items(
                &ItemFilter {
                    all_tags: vec![tag]
                },
                1,
                None
            )
            .unwrap()
            .items
            .is_empty()
    );
    let cut = fixture.journal.work_map_snapshot().unwrap();
    let changes = fixture.journal.connection.total_changes();
    fixture
        .journal
        .connection
        .pragma_update(None, "query_only", true)
        .unwrap();
    let page = fixture.journal.query_tags(&namespace, 1, None).unwrap();
    assert_eq!(page.tags.len(), 1);
    assert_eq!(page.snapshot, cut);
    let old = fixture
        .journal
        .query_tags_at(&namespace, 1, None, Some(&empty.snapshot))
        .unwrap();
    assert!(old.tags.is_empty());
    assert_eq!(fixture.journal.connection.total_changes(), changes);
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), cut);
}

#[test]
fn tag_catalog_rechecks_registered_secrets_in_creation_and_active_labels() {
    for field in ["creation", "rename", "key", "namespace"] {
        let mut fixture = Fixture::new();
        let sensitive = format!("wmcatalog{}", Uuid::new_v4().simple());
        let namespace = TagKey::new(if field == "namespace" {
            &sensitive
        } else {
            "work.group"
        })
        .unwrap();
        let tag = fixture.tag(
            namespace.as_str(),
            if field == "key" { &sensitive } else { "group" },
            if field == "creation" {
                &sensitive
            } else {
                "Original"
            },
        );
        fixture.apply(Mutation::RenameTag {
            tag_id: tag,
            label: SemanticLabel::new(if field == "rename" {
                &sensitive
            } else {
                "Renamed"
            })
            .unwrap(),
        });
        let page = fixture.journal.query_tags(&namespace, 1, None).unwrap();
        crate::log::secret(&sensitive);
        let error = fixture
            .journal
            .query_tags_at(&namespace, 1, None, Some(&page.snapshot))
            .unwrap_err();
        assert!(!format!("{error:#}").contains(&sensitive));
        assert_eq!(fixture.journal.work_map_snapshot().unwrap(), page.snapshot);
    }
}
