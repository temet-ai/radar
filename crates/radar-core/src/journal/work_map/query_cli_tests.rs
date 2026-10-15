use super::*;

struct TestDir(PathBuf);

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    journal: Journal,
    session: SessionId,
    recorder: ActionId,
    evidence: EventId,
    _temp: TestDir,
}

impl Fixture {
    fn new() -> Self {
        let temp =
            TestDir(std::env::temp_dir().join(format!("radar-work-map-lookup-{}", Uuid::new_v4())));
        let mut journal = Journal::open(&temp.0.join("actions.sqlite3")).unwrap();
        let session = journal.start_session_with(SessionOrigin::Cli).unwrap();
        let recorder = journal
            .begin(
                &session,
                ActionDescriptor::new("work-map.annotate", Origin::Host),
                None,
            )
            .unwrap();
        let evidence = journal.query(&Query::default()).unwrap()[0]
            .event_id
            .clone();
        Self {
            journal,
            session,
            recorder,
            evidence,
            _temp: temp,
        }
    }

    fn apply(&mut self, mutation: Mutation) -> ChangeRequest {
        let request = ChangeRequest {
            id: ChangeId::new(),
            provenance: Provenance {
                recording_action_id: self.recorder.clone(),
                evidence_event_id: self.evidence.clone(),
                basis: Basis::UserAsserted,
            },
            observed_at_ms: 1000,
            mutation,
        };
        self.journal.apply_work_map(&request).unwrap();
        request
    }

    fn item(&mut self) -> ItemId {
        let item_id = ItemId::new();
        self.apply(Mutation::CreateItem {
            item_id: item_id.clone(),
            kind: StableCode::new("card"),
        });
        item_id
    }

    fn tag(&mut self, label: &str) -> ChangeRequest {
        self.apply(Mutation::CreateTag {
            tag_id: TagId::new(),
            namespace: TagKey::new("alpha").unwrap(),
            key: TagKey::new("beta").unwrap(),
            label: SemanticLabel::new(label).unwrap(),
        })
    }
}

#[test]
fn request_shape_validation_is_private_bounded_and_does_not_resolve_references() {
    let fixture = Fixture::new();
    let before = fixture.journal.work_map_snapshot().unwrap();
    let mut request = ChangeRequest {
        id: ChangeId::new(),
        provenance: Provenance {
            recording_action_id: ActionId::new(),
            evidence_event_id: EventId::new(),
            basis: Basis::UserAsserted,
        },
        observed_at_ms: 1000,
        mutation: Mutation::CreateTag {
            tag_id: TagId::new(),
            namespace: TagKey::new("alpha").unwrap(),
            key: TagKey::new("beta").unwrap(),
            label: SemanticLabel::new("Alpha").unwrap(),
        },
    };
    validate_change_request(&request).unwrap();
    let secret = format!("wmshape{}", Uuid::new_v4().simple());
    let mut encoded = serde_json::to_value(&request).unwrap();
    encoded["mutation"]["label"] = serde_json::json!(secret);
    request = serde_json::from_value(encoded).unwrap();
    crate::log::secret(&secret);
    let error = validate_change_request(&request).unwrap_err();
    assert!(!format!("{error:#}").contains(&secret));
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), before);
}

#[test]
fn change_lookup_returns_exact_history_and_snapshot_relative_retraction() {
    let mut fixture = Fixture::new();
    let before = fixture.journal.work_map_snapshot().unwrap();
    let creation = fixture.tag("Alpha");
    let Mutation::CreateTag { tag_id, .. } = &creation.mutation else {
        unreachable!()
    };
    let renamed = fixture.apply(Mutation::RenameTag {
        tag_id: tag_id.clone(),
        label: SemanticLabel::new("Beta").unwrap(),
    });
    let old = fixture.journal.work_map_snapshot().unwrap();
    fixture.apply(Mutation::Retract {
        claim_id: renamed.id.clone(),
        expected: AssertionTarget::LabelAssertion {
            tag_id: tag_id.clone(),
        },
    });

    let current = fixture
        .journal
        .get_work_change(&renamed.id, None)
        .unwrap()
        .unwrap();
    assert_eq!(current.request, renamed);
    assert_eq!(current.version, VERSION);
    assert_eq!(current.recording.action_id, fixture.recorder);
    assert_eq!(current.evidence.event_id, fixture.evidence);
    assert!(current.retracted);
    let historical = fixture
        .journal
        .get_work_change(&renamed.id, Some(&old))
        .unwrap()
        .unwrap();
    assert_eq!(historical.request, renamed);
    assert_eq!(historical.sequence, current.sequence);
    assert_eq!(historical.recorded_at_ms, current.recorded_at_ms);
    assert!(!historical.retracted);
    assert!(
        fixture
            .journal
            .get_work_change(&creation.id, Some(&before))
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .journal
            .get_work_change(&ChangeId::new(), None)
            .unwrap()
            .is_none()
    );
}

#[test]
fn change_lookup_rejects_invalid_ids_cuts_and_excluded_dependencies() {
    let mut fixture = Fixture::new();
    let creation = fixture.tag("Alpha");
    let cut = fixture.journal.work_map_snapshot().unwrap();
    for text in ["invalid", "00000000-0000-4000-8000-00000000000A"] {
        let id = serde_json::from_value(serde_json::json!(text)).unwrap();
        assert!(fixture.journal.get_work_change(&id, None).is_err());
    }
    let other = Fixture::new().journal.work_map_snapshot().unwrap();
    let mut negative = cut.clone();
    negative.map_sequence = -1;
    let mut ahead = cut.clone();
    ahead.gap_sequence += 1;
    let mut excluded_evidence = cut.clone();
    excluded_evidence.event_sequence = 0;
    for invalid in [other, negative, ahead, excluded_evidence] {
        assert!(
            fixture
                .journal
                .get_work_change(&creation.id, Some(&invalid))
                .is_err()
        );
    }

    let target = fixture
        .journal
        .begin(
            &fixture.session,
            ActionDescriptor::new("card.inspect", Origin::Host),
            None,
        )
        .unwrap();
    let Mutation::CreateTag { tag_id, .. } = creation.mutation else {
        unreachable!()
    };
    let assignment = fixture.apply(Mutation::AssignTag {
        tag_id,
        target: TagTarget::Action(target),
    });
    let mut forged = fixture.journal.work_map_snapshot().unwrap();
    forged.event_sequence = cut.event_sequence;
    assert!(
        fixture
            .journal
            .get_work_change(&assignment.id, Some(&forged))
            .is_err()
    );
}

#[test]
fn change_lookup_rechecks_persisted_annotation_and_evidence_secrets() {
    for field in ["label", "evidence"] {
        let mut fixture = Fixture::new();
        let secret = format!("wmread{}", Uuid::new_v4().simple());
        if field == "evidence" {
            fixture
                .journal
                .record(
                    &fixture.recorder,
                    Outcome::Succeeded,
                    Metadata {
                        url: Some(
                            SafeUrl::parse(&format!("https://example.test/{secret}")).unwrap(),
                        ),
                        ..Metadata::default()
                    },
                )
                .unwrap();
            fixture.evidence = fixture.journal.query(&Query::default()).unwrap()[1]
                .event_id
                .clone();
        }
        let request = fixture.tag(if field == "label" { &secret } else { "Alpha" });
        let cut = fixture.journal.work_map_snapshot().unwrap();
        assert!(
            fixture
                .journal
                .get_work_change(&request.id, Some(&cut))
                .unwrap()
                .is_some()
        );
        crate::log::secret(&secret);
        for at in [None, Some(&cut)] {
            let error = fixture
                .journal
                .get_work_change(&request.id, at)
                .unwrap_err();
            assert!(!format!("{error:#}").contains(&secret));
        }
        assert_eq!(fixture.journal.work_map_snapshot().unwrap(), cut);
    }
}

#[test]
fn explicit_first_page_cut_excludes_later_audit_events_and_preserves_continuations() {
    let mut fixture = Fixture::new();
    let first_item = fixture.item();
    let second_item = fixture.item();
    let selection = Selection {
        items: vec![first_item.clone(), second_item.clone()],
        actions: vec![fixture.recorder.clone()],
    };
    let cut = fixture.journal.work_map_snapshot().unwrap();
    let expected = fixture
        .journal
        .handoff(&selection, Limits::default(), None)
        .unwrap();

    let audit = fixture
        .journal
        .begin(
            &fixture.session,
            ActionDescriptor::new("work-map.handoff", Origin::Host),
            None,
        )
        .unwrap();
    fixture
        .journal
        .record(&fixture.recorder, Outcome::Succeeded, Metadata::default())
        .unwrap();
    fixture.item();
    fixture
        .journal
        .record_gap(&fixture.session, 1, 0, 2000)
        .unwrap();
    let current = fixture.journal.work_map_snapshot().unwrap();
    assert!(current.event_sequence > cut.event_sequence);
    assert!(current.map_sequence > cut.map_sequence);
    assert!(current.gap_sequence > cut.gap_sequence);

    let filter = ItemFilter::default();
    let first = fixture
        .journal
        .query_items_at(&filter, 1, None, Some(&cut))
        .unwrap();
    assert_eq!(first.snapshot, cut);
    assert_eq!(first.items[0].item_id, first_item);
    let second = fixture
        .journal
        .query_items(&filter, 1, first.next.as_ref())
        .unwrap();
    assert_eq!(second.snapshot, cut);
    assert_eq!(second.items[0].item_id, second_item);
    assert!(!second.truncated);

    let limits = Limits {
        records: 1,
        ..Limits::default()
    };
    let first = fixture
        .journal
        .handoff_at(&selection, limits, None, Some(&cut))
        .unwrap();
    assert_eq!(first.snapshot, cut);
    assert!(!first.known_recording_gaps);
    let mut records = first.records;
    let mut cursor = first.next;
    while let Some(next) = cursor {
        let page = fixture
            .journal
            .handoff(&selection, limits, Some(&next))
            .unwrap();
        assert_eq!(page.snapshot, cut);
        assert!(!page.known_recording_gaps);
        records.extend(page.records);
        cursor = page.next;
        assert!(records.len() <= expected.records.len());
    }
    assert_eq!(
        serde_json::to_value(records).unwrap(),
        serde_json::to_value(expected.records).unwrap()
    );
    assert!(
        fixture
            .journal
            .handoff_at(
                &Selection {
                    items: vec![],
                    actions: vec![audit]
                },
                Limits::default(),
                None,
                Some(&cut)
            )
            .is_err()
    );
}

#[test]
fn explicit_cursor_cuts_must_match_each_highwater_and_database_identity() {
    let mut fixture = Fixture::new();
    let selection = Selection {
        items: vec![fixture.item(), fixture.item()],
        actions: vec![],
    };
    let filter = ItemFilter::default();
    let cut = fixture.journal.work_map_snapshot().unwrap();
    let items = fixture
        .journal
        .query_items_at(&filter, 1, None, Some(&cut))
        .unwrap();
    let handoff = fixture
        .journal
        .handoff_at(
            &selection,
            Limits {
                records: 1,
                ..Limits::default()
            },
            None,
            Some(&cut),
        )
        .unwrap();
    assert!(items.next.is_some() && handoff.next.is_some());
    assert_eq!(
        fixture
            .journal
            .query_items_at(&filter, 1, items.next.as_ref(), Some(&cut))
            .unwrap()
            .snapshot,
        cut
    );
    assert_eq!(
        fixture
            .journal
            .handoff_at(
                &selection,
                Limits::default(),
                handoff.next.as_ref(),
                Some(&cut)
            )
            .unwrap()
            .snapshot,
        cut
    );
    fixture.item();
    fixture
        .journal
        .record(&fixture.recorder, Outcome::Succeeded, Metadata::default())
        .unwrap();
    fixture
        .journal
        .record_gap(&fixture.session, 1, 0, 2000)
        .unwrap();
    let current = fixture.journal.work_map_snapshot().unwrap();
    let mut different_event = cut.clone();
    different_event.event_sequence = current.event_sequence;
    let mut different_map = cut.clone();
    different_map.map_sequence = current.map_sequence;
    let mut different_gap = cut.clone();
    different_gap.gap_sequence = current.gap_sequence;
    let mut different_database = cut.clone();
    different_database.database_id = Uuid::new_v4().to_string();
    for different in [
        different_event,
        different_map,
        different_gap,
        different_database,
    ] {
        let errors = [
            fixture
                .journal
                .query_items_at(&filter, 1, items.next.as_ref(), Some(&different))
                .unwrap_err(),
            fixture
                .journal
                .handoff_at(
                    &selection,
                    Limits::default(),
                    handoff.next.as_ref(),
                    Some(&different),
                )
                .unwrap_err(),
        ];
        for error in errors {
            assert_eq!(
                error.to_string(),
                "cursor and explicit snapshot do not match"
            );
        }
    }
}

#[test]
fn explicit_page_cuts_keep_selection_and_budget_limits() {
    let mut fixture = Fixture::new();
    let item = fixture.item();
    let selection = Selection {
        items: vec![item.clone()],
        actions: vec![],
    };
    let cut = fixture.journal.work_map_snapshot().unwrap();
    for limit in [0, MAX_QUERY_LIMIT + 1] {
        assert!(
            fixture
                .journal
                .query_items_at(&ItemFilter::default(), limit, None, Some(&cut))
                .is_err()
        );
    }
    let too_many_tags = ItemFilter {
        all_tags: vec![TagId::new(); MAX_TAG_FILTERS + 1],
    };
    assert!(
        fixture
            .journal
            .query_items_at(&too_many_tags, 1, None, Some(&cut))
            .is_err()
    );
    for limits in [
        Limits {
            records: 0,
            ..Limits::default()
        },
        Limits {
            records: MAX_QUERY_LIMIT + 1,
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
        assert!(
            fixture
                .journal
                .handoff_at(&selection, limits, None, Some(&cut))
                .is_err()
        );
    }
    for invalid in [
        Selection::default(),
        Selection {
            items: vec![item; MAX_SELECTION + 1],
            actions: vec![],
        },
    ] {
        assert!(
            fixture
                .journal
                .handoff_at(&invalid, Limits::default(), None, Some(&cut))
                .is_err()
        );
    }
    assert_eq!(fixture.journal.work_map_snapshot().unwrap(), cut);
}
