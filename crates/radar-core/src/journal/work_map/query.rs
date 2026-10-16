use super::*;
use rusqlite::types::Value;
use store::{action_source, load_change, requested_sequence};

impl Journal {
    pub fn work_map_snapshot(&self) -> Result<Snapshot> {
        let tx = self.connection.unchecked_transaction()?;
        let cut = snapshot(&tx)?;
        tx.commit()?;
        Ok(cut)
    }

    /// Load one claim by its stable identity at the optional captured cut.
    /// Later claims are absent; retraction status is relative to this snapshot.
    pub fn get_work_change(
        &self,
        id: &ChangeId,
        at: Option<&Snapshot>,
    ) -> Result<Option<ChangeRecord>> {
        validate_uuid(id.as_str())?;
        let tx = self.connection.unchecked_transaction()?;
        let cut = checked_snapshot(&tx, at)?;
        let sequence: Option<i64> = tx
            .query_row(
                "SELECT sequence FROM work_changes WHERE id=?1 AND sequence<=?2",
                params![id.as_str(), cut.map_sequence],
                |row| row.get(0),
            )
            .optional()?;
        let result = sequence
            .map(|sequence| load_change(&tx, sequence, &cut))
            .transpose()?;
        tx.commit()?;
        Ok(result)
    }

    /// Resolve the subject at the action's Requested sequence. Conflicting
    /// explicit bindings remain ambiguous; inferred bindings never resolve it.
    pub fn resolve_subject(
        &self,
        key: &SubjectKey,
        action: &ActionId,
        at: Option<&Snapshot>,
    ) -> Result<Resolution> {
        key.validate()?;
        validate_uuid(action.as_str())?;
        let tx = self.connection.unchecked_transaction()?;
        let cut = checked_snapshot(&tx, at)?;
        let start = requested_sequence(&tx, action)?;
        ensure!(
            start <= cut.event_sequence,
            "action is outside the snapshot"
        );
        let mut statement = tx.prepare("SELECT c.sequence,b.item_id FROM work_item_bindings b JOIN subjects s ON s.id=b.subject_id JOIN work_changes c ON c.id=b.change_id WHERE s.provider=?1 AND s.instance=?2 AND s.kind=?3 AND s.external_id=?4 AND c.sequence<=?5 AND c.basis<>'inferred' AND b.from_sequence<=?6 AND (b.through_sequence IS NULL OR b.through_sequence>=?6) AND NOT EXISTS(SELECT 1 FROM work_changes r WHERE r.retracts_id=c.id AND r.sequence<=?5) ORDER BY c.sequence LIMIT 33")?;
        let rows = statement
            .query_map(
                params![
                    key.provider,
                    key.instance,
                    key.kind,
                    key.id,
                    cut.map_sequence,
                    start
                ],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)),
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let truncated = rows.len() > MAX_SELECTION;
        let mut items = Vec::new();
        let mut claims = Vec::new();
        for (sequence, id) in rows.into_iter().take(MAX_SELECTION) {
            let id = ItemId::parse(&id)?;
            if !items.contains(&id) {
                items.push(id);
            }
            claims.push(load_change(&tx, sequence, &cut)?);
        }
        let state = if truncated || items.len() > 1 {
            ResolutionState::Ambiguous
        } else if items.is_empty() {
            ResolutionState::Unbound
        } else {
            ResolutionState::Resolved
        };
        tx.commit()?;
        Ok(Resolution {
            snapshot: cut,
            state,
            items,
            claims,
            truncated,
        })
    }

    /// Find the durable tag identity after a restart. Label edits remain
    /// evidence claims in handoff; this does not choose a canonical label.
    pub fn find_tag(
        &self,
        namespace: &TagKey,
        key: &TagKey,
        at: Option<&Snapshot>,
    ) -> Result<Option<TagIdentity>> {
        TagKey::new(namespace.as_str())?;
        TagKey::new(key.as_str())?;
        let tx = self.connection.unchecked_transaction()?;
        let cut = checked_snapshot(&tx, at)?;
        let found: Option<(String,i64)> = tx.query_row("SELECT t.id,c.sequence FROM work_tags t JOIN work_changes c ON c.id=t.created_change_id WHERE t.namespace=?1 AND t.stable_key=?2 AND c.sequence<=?3 LIMIT 1", params![namespace.as_str(),key.as_str(),cut.map_sequence], |r| Ok((r.get(0)?,r.get(1)?))).optional()?;
        let result = found
            .map(|(id, sequence)| -> Result<TagIdentity> {
                Ok(TagIdentity {
                    snapshot: cut.clone(),
                    tag_id: TagId::parse(&id)?,
                    namespace: namespace.clone(),
                    key: key.clone(),
                    creation: load_change(&tx, sequence, &cut)?,
                })
            })
            .transpose()?;
        tx.commit()?;
        Ok(result)
    }

    /// Discover authored definitions in one exact namespace, including tags
    /// with no assignments. Label claims never determine identity or membership.
    pub fn query_tags(
        &self,
        namespace: &TagKey,
        limit: usize,
        cursor: Option<&TagCursor>,
    ) -> Result<TagPage> {
        self.query_tags_at(namespace, limit, cursor, None)
    }

    /// Share the same captured cut as item/evidence reads. Paging retains this
    /// cut, including label retractions, even when later writes arrive.
    pub fn query_tags_at(
        &self,
        namespace: &TagKey,
        limit: usize,
        cursor: Option<&TagCursor>,
        at: Option<&Snapshot>,
    ) -> Result<TagPage> {
        TagKey::new(namespace.as_str())?;
        ensure!(
            (1..=MAX_TAG_QUERY_LIMIT).contains(&limit),
            "tag query limit must be 1..=32"
        );
        if let Some(cursor) = cursor {
            TagKey::new(cursor.after_key.as_str())?;
            ensure!(
                cursor.namespace == *namespace,
                "tag cursor belongs to a different namespace"
            );
        }
        let tx = self.connection.unchecked_transaction()?;
        let cut = checked_page_snapshot(&tx, cursor.map(|c| &c.snapshot), at)?;
        let after = cursor.map_or("", |c| c.after_key.as_str());
        // The existing (namespace, stable_key) index supplies bounded keyset
        // pages. No item page or assignment history is needed for discovery.
        let mut statement = tx.prepare(
            "SELECT t.id,t.stable_key,c.id,c.sequence,l.label,c.recording_action_id,c.evidence_event_id FROM work_tags t JOIN work_changes c ON c.id=t.created_change_id JOIN work_tag_labels l ON l.change_id=c.id WHERE t.namespace=?1 AND t.stable_key>?2 AND c.sequence<=?3 AND c.basis='user_asserted' ORDER BY t.stable_key LIMIT ?4",
        )?;
        let rows = statement
            .query_map(
                params![
                    namespace.as_str(),
                    after,
                    cut.map_sequence,
                    (limit + 1) as i64
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        RawLabelClaim::read(row, 2)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let truncated = rows.len() > limit;
        let mut tags = Vec::with_capacity(limit);
        for (id, key, creation) in rows.into_iter().take(limit) {
            let tag_id = TagId::parse(&id)?;
            let creation = creation.into_claim()?;
            // Index order avoids sorting/materializing the complete label
            // history. Only this bounded active sample is sorted by sequence.
            let mut statement = tx.prepare(
                "SELECT c.id,c.sequence,l.label,c.recording_action_id,c.evidence_event_id FROM work_tag_labels l JOIN work_changes c ON c.id=l.change_id WHERE l.tag_id=?1 AND c.kind='rename_tag' AND c.sequence<=?2 AND c.basis='user_asserted' AND NOT EXISTS(SELECT 1 FROM work_changes r WHERE r.retracts_id=c.id AND r.sequence<=?2) ORDER BY l.change_id LIMIT ?3",
            )?;
            let rows = statement
                .query_map(
                    params![id, cut.map_sequence, (TAG_LABEL_LIMIT + 1) as i64],
                    |row| RawLabelClaim::read(row, 0),
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            let labels_truncated = rows.len() > TAG_LABEL_LIMIT;
            let mut renames = rows
                .into_iter()
                .take(TAG_LABEL_LIMIT)
                .map(RawLabelClaim::into_claim)
                .collect::<Result<Vec<_>>>()?;
            renames.sort_by_key(|claim| claim.sequence);
            tags.push(TagEntry {
                tag_id,
                key: TagKey::new(&key)?,
                created_sequence: creation.sequence,
                creation,
                renames,
                labels_truncated,
            });
        }
        let next = if truncated {
            Some(TagCursor {
                snapshot: cut.clone(),
                namespace: namespace.clone(),
                after_key: tags.last().context("missing tag page cursor")?.key.clone(),
            })
        } else {
            None
        };
        let page = TagPage {
            snapshot: cut,
            namespace: namespace.clone(),
            tags,
            truncated,
            next,
        };
        ensure!(
            encode_safe_output(&page)?.len() <= MAX_PAGE_BYTES,
            "tag catalog exceeds its byte bound"
        );
        tx.commit()?;
        Ok(page)
    }

    /// AND together at most eight explicit, non-inferred tag assignments.
    /// A page has at most 1000 small identity rows; labels are read via handoff.
    pub fn query_items(
        &self,
        filter: &ItemFilter,
        limit: usize,
        cursor: Option<&ItemCursor>,
    ) -> Result<ItemPage> {
        self.query_items_at(filter, limit, cursor, None)
    }

    /// Start at a captured cut or continue the cursor's original snapshot.
    /// An explicitly supplied cut must match an accompanying cursor exactly.
    pub fn query_items_at(
        &self,
        filter: &ItemFilter,
        limit: usize,
        cursor: Option<&ItemCursor>,
        at: Option<&Snapshot>,
    ) -> Result<ItemPage> {
        ensure!(
            (1..=MAX_QUERY_LIMIT).contains(&limit),
            "item query limit must be 1..=1000"
        );
        ensure!(
            filter.all_tags.len() <= MAX_TAG_FILTERS,
            "item query has too many tag filters"
        );
        for id in &filter.all_tags {
            validate_uuid(id.as_str())?;
        }
        let mut filter = filter.clone();
        filter.all_tags.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        filter.all_tags.dedup();
        for id in &filter.all_tags {
            validate_uuid(id.as_str())?;
        }
        if let Some(cursor) = cursor {
            ensure!(
                cursor.filter == filter && cursor.after_sequence >= 0,
                "item cursor belongs to a different selection"
            );
        }
        let tx = self.connection.unchecked_transaction()?;
        let cut = checked_page_snapshot(&tx, cursor.map(|c| &c.snapshot), at)?;
        let after = cursor.map_or(0, |c| c.after_sequence);
        ensure!(
            after <= cut.map_sequence,
            "item cursor exceeds its snapshot"
        );
        let mut sql = String::from(
            "SELECT i.id,i.kind,c.sequence FROM work_items i JOIN work_changes c ON c.id=i.created_change_id WHERE c.sequence<=?1 AND c.sequence>?2",
        );
        let mut values: Vec<Value> = vec![cut.map_sequence.into(), after.into()];
        for tag in &filter.all_tags {
            values.push(tag.as_str().to_owned().into());
            sql.push_str(&format!(" AND EXISTS(SELECT 1 FROM work_tag_assignments a JOIN work_changes ac ON ac.id=a.change_id WHERE a.item_id=i.id AND a.tag_id=?{} AND ac.sequence<=?1 AND ac.basis<>'inferred' AND NOT EXISTS(SELECT 1 FROM work_changes r WHERE r.retracts_id=ac.id AND r.sequence<=?1))",values.len()));
        }
        values.push(((limit + 1) as i64).into());
        sql.push_str(&format!(" ORDER BY c.sequence LIMIT ?{}", values.len()));
        let mut statement = tx.prepare(&sql)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(values), |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let truncated = rows.len() > limit;
        let items = rows
            .into_iter()
            .take(limit)
            .map(|(id, kind, sequence)| {
                ensure!(
                    !crate::log::contains_secret(&kind),
                    "work item kind may contain a credential"
                );
                Ok(ItemEntry {
                    item_id: ItemId::parse(&id)?,
                    kind: StableCode::try_new(&kind)?,
                    created_sequence: sequence,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let next = if truncated {
            Some(ItemCursor {
                snapshot: cut.clone(),
                filter,
                after_sequence: items
                    .last()
                    .context("missing item page cursor")?
                    .created_sequence,
            })
        } else {
            None
        };
        let page = ItemPage {
            snapshot: cut,
            items,
            truncated,
            next,
        };
        encode_safe_output(&page)?;
        tx.commit()?;
        Ok(page)
    }

    /// A flat, globally bounded evidence page. No recursive graph expansion,
    /// executable plan, current business-state claim, or implicit approval.
    pub fn handoff(
        &self,
        selection: &Selection,
        limits: Limits,
        cursor: Option<&HandoffCursor>,
    ) -> Result<Handoff> {
        self.handoff_at(selection, limits, cursor, None)
    }

    /// Start an evidence page at a captured cut before recording its audit.
    /// Continuations retain their original cut, including all three highwaters.
    pub fn handoff_at(
        &self,
        selection: &Selection,
        limits: Limits,
        cursor: Option<&HandoffCursor>,
        at: Option<&Snapshot>,
    ) -> Result<Handoff> {
        limits.validate()?;
        let selection = selection.canonical()?;
        if let Some(cursor) = cursor {
            ensure!(
                cursor.selection == selection && cursor.stage <= 2 && cursor.after_sequence >= 0,
                "handoff cursor belongs to a different selection"
            );
        }
        let tx = self.connection.unchecked_transaction()?;
        let cut = checked_page_snapshot(&tx, cursor.map(|c| &c.snapshot), at)?;
        validate_selection(&tx, &selection, &cut)?;
        let (cte, values) = selection_cte(&selection, &cut);
        let gap_sql = format!(
            "{cte} SELECT EXISTS(SELECT 1 FROM recording_gaps g WHERE g.sequence<=?3 AND EXISTS(SELECT 1 FROM actions a JOIN source_actions sa ON sa.id=a.id WHERE a.session_id=g.session_id))"
        );
        let known_recording_gaps: bool =
            tx.query_row(&gap_sql, rusqlite::params_from_iter(values.iter()), |r| {
                r.get(0)
            })?;
        let mut result = Handoff {
            version: VERSION,
            snapshot: cut.clone(),
            selection: selection.clone(),
            records: Vec::new(),
            truncated: false,
            next: None,
            authority: "observations_only",
            task_instruction: "unspecified",
            execution_authorization: "unspecified",
            known_recording_gaps,
            completeness_not_guaranteed: true,
        };
        let mut stage = cursor.map_or(0, |c| c.stage);
        let mut after = cursor.map_or(0, |c| c.after_sequence);
        let high = match stage {
            0 => cut.map_sequence,
            1 => cut.event_sequence,
            _ => cut.gap_sequence,
        };
        ensure!(after <= high, "handoff cursor exceeds its snapshot");
        let mut record_bytes = 0usize;
        'pages: while stage <= 2 {
            let mut page_values = values.clone();
            page_values.push(after.into());
            let after_index = page_values.len();
            page_values.push(((limits.records - result.records.len() + 1) as i64).into());
            let limit_index = page_values.len();
            let tail = match stage {
                0 => format!(
                    "SELECT c.sequence FROM work_changes c JOIN selected_changes sc ON sc.id=c.id WHERE c.sequence>?{after_index} ORDER BY c.sequence LIMIT ?{limit_index}"
                ),
                1 => format!(
                    "SELECT e.sequence FROM action_events e JOIN selected_actions a ON a.id=e.action_id WHERE e.sequence<=?1 AND e.sequence>?{after_index} ORDER BY e.sequence LIMIT ?{limit_index}"
                ),
                _ => format!(
                    "SELECT g.sequence FROM recording_gaps g WHERE g.sequence<=?3 AND g.sequence>?{after_index} AND EXISTS(SELECT 1 FROM actions a JOIN source_actions sa ON sa.id=a.id WHERE a.session_id=g.session_id) ORDER BY g.sequence LIMIT ?{limit_index}"
                ),
            };
            let mut statement = tx.prepare(&format!("{cte} {tail}"))?;
            let sequences = statement
                .query_map(rusqlite::params_from_iter(page_values), |r| {
                    r.get::<_, i64>(0)
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            for sequence in sequences {
                let next = HandoffCursor {
                    snapshot: cut.clone(),
                    selection: selection.clone(),
                    stage,
                    after_sequence: after,
                };
                if result.records.len() == limits.records {
                    result.truncated = true;
                    result.next = Some(next);
                    break 'pages;
                }
                let record = match stage {
                    0 => HandoffRecord::Map {
                        change: Box::new(load_change(&tx, sequence, &cut)?),
                    },
                    1 => {
                        let mut events = self.query(&Query {
                            after_sequence: Some(sequence - 1),
                            limit: 1,
                            ..Query::default()
                        })?;
                        let event = events.pop().context("handoff event disappeared")?;
                        ensure!(
                            event.sequence == sequence,
                            "handoff event sequence mismatch"
                        );
                        let source = action_source(&tx, &event.action_id)?;
                        let association = if selection.actions.contains(&event.action_id) {
                            HistoryAssociation::DirectSelection
                        } else if ambiguous_action(&tx, &event.action_id, &cut)? {
                            HistoryAssociation::AmbiguousBinding
                        } else {
                            HistoryAssociation::ExplicitBinding
                        };
                        HandoffRecord::Action {
                            event: Box::new(event),
                            source,
                            association,
                        }
                    }
                    _ => HandoffRecord::Gap {
                        gap: load_gap(&tx, sequence)?,
                    },
                };
                let bytes =
                    encode_safe_output(&record)?.len() + usize::from(!result.records.is_empty());
                // Serialize only the small empty envelope for exact accounting;
                // repeated serialization of the growing payload is unnecessary.
                let preview = Handoff {
                    version: VERSION,
                    snapshot: cut.clone(),
                    selection: selection.clone(),
                    records: Vec::new(),
                    truncated: true,
                    next: Some(HandoffCursor {
                        after_sequence: sequence,
                        ..next.clone()
                    }),
                    authority: "observations_only",
                    task_instruction: "unspecified",
                    execution_authorization: "unspecified",
                    known_recording_gaps,
                    completeness_not_guaranteed: true,
                };
                let envelope_bytes = encode_safe_output(&preview)?.len();
                if envelope_bytes + record_bytes + bytes > limits.bytes {
                    ensure!(
                        !result.records.is_empty(),
                        "one handoff record exceeds the byte budget; request a larger bounded page"
                    );
                    result.truncated = true;
                    result.next = Some(next);
                    break 'pages;
                }
                result.records.push(record);
                record_bytes += bytes;
                after = sequence;
            }
            stage += 1;
            after = 0;
        }
        ensure!(
            encode_safe_output(&result)?.len() <= limits.bytes,
            "handoff envelope exceeds the byte budget"
        );
        tx.commit()?;
        Ok(result)
    }
}

pub(super) fn checked_page_snapshot(
    connection: &Connection,
    cursor: Option<&Snapshot>,
    at: Option<&Snapshot>,
) -> Result<Snapshot> {
    if let (Some(cursor), Some(at)) = (cursor, at) {
        ensure!(cursor == at, "cursor and explicit snapshot do not match");
    }
    checked_snapshot(connection, cursor.or(at))
}

fn snapshot(connection: &Connection) -> Result<Snapshot> {
    Ok(Snapshot {
        database_id: connection.query_row(
            "SELECT id FROM work_map_store WHERE singleton=1",
            [],
            |r| r.get(0),
        )?,
        event_sequence: connection.query_row(
            "SELECT COALESCE(max(sequence),0) FROM action_events",
            [],
            |r| r.get(0),
        )?,
        map_sequence: connection.query_row(
            "SELECT COALESCE(max(sequence),0) FROM work_changes",
            [],
            |r| r.get(0),
        )?,
        gap_sequence: connection.query_row(
            "SELECT COALESCE(max(sequence),0) FROM recording_gaps",
            [],
            |r| r.get(0),
        )?,
    })
}
fn checked_snapshot(connection: &Connection, saved: Option<&Snapshot>) -> Result<Snapshot> {
    let current = snapshot(connection)?;
    let Some(saved) = saved else {
        return Ok(current);
    };
    ensure!(
        saved.database_id == current.database_id,
        "cursor belongs to a different journal database"
    );
    ensure!(
        (0..=current.event_sequence).contains(&saved.event_sequence)
            && (0..=current.map_sequence).contains(&saved.map_sequence)
            && (0..=current.gap_sequence).contains(&saved.gap_sequence),
        "snapshot highwater is invalid"
    );
    let inconsistent: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM work_changes c JOIN action_events e ON e.id=c.evidence_event_id WHERE c.sequence<=?1 AND (e.sequence>?2 OR EXISTS(SELECT 1 FROM action_events start WHERE start.action_id=c.recording_action_id AND start.outcome='requested' AND start.sequence>?2)))",params![saved.map_sequence,saved.event_sequence],|r|r.get(0))?;
    ensure!(
        !inconsistent,
        "snapshot excludes provenance required by its map claims"
    );
    let excluded_target: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM work_tag_assignments a JOIN work_changes c ON c.id=a.change_id JOIN action_events start ON start.action_id=a.action_id AND start.outcome='requested' WHERE c.sequence<=?1 AND start.sequence>?2) OR EXISTS(SELECT 1 FROM work_item_bindings b JOIN work_changes c ON c.id=b.change_id WHERE c.sequence<=?1 AND (b.from_sequence>?2 OR b.through_sequence>?2))", params![saved.map_sequence,saved.event_sequence], |r| r.get(0))?;
    ensure!(
        !excluded_target,
        "snapshot excludes an action or interval required by its map claims"
    );
    Ok(saved.clone())
}
fn validate_selection(
    connection: &Connection,
    selection: &Selection,
    cut: &Snapshot,
) -> Result<()> {
    for item in &selection.items {
        let present: bool = connection.query_row("SELECT EXISTS(SELECT 1 FROM work_items i JOIN work_changes c ON c.id=i.created_change_id WHERE i.id=?1 AND c.sequence<=?2)",params![item.as_str(),cut.map_sequence],|r|r.get(0))?;
        ensure!(present, "selected work item is outside the snapshot");
    }
    for action in &selection.actions {
        ensure!(
            requested_sequence(connection, action)? <= cut.event_sequence,
            "selected action is outside the snapshot"
        );
    }
    Ok(())
}
fn placeholders(values: &mut Vec<Value>, ids: impl Iterator<Item = String>) -> String {
    let mut cells = Vec::new();
    for id in ids {
        values.push(id.into());
        cells.push(format!("(?{})", values.len()));
    }
    if cells.is_empty() {
        "SELECT NULL WHERE 0".into()
    } else {
        format!("VALUES {}", cells.join(","))
    }
}
fn selection_cte(selection: &Selection, cut: &Snapshot) -> (String, Vec<Value>) {
    let mut values = vec![
        cut.event_sequence.into(),
        cut.map_sequence.into(),
        cut.gap_sequence.into(),
    ];
    let items = placeholders(
        &mut values,
        selection.items.iter().map(|id| id.as_str().to_owned()),
    );
    let actions = placeholders(
        &mut values,
        selection.actions.iter().map(|id| id.as_str().to_owned()),
    );
    // The selected and source actions are built once from the selection's
    // side (its bindings, its changes) and kept: tested per row of `actions`
    // instead, every page and every recording gap scanned the whole journal
    // (9 s for one item on a journal of 300,000 events). What they hold is
    // bounded by the history bound to the selection, not by the journal.
    let sql = format!("WITH chosen_items(id) AS ({items}), chosen_actions(id) AS ({actions}),
    selected_actions(id) AS MATERIALIZED (
      SELECT a.id FROM actions a WHERE a.id IN (SELECT id FROM chosen_actions)
      UNION
      SELECT ax.action_id FROM work_item_bindings b JOIN work_changes bc ON bc.id=b.change_id
        JOIN action_subjects ax ON ax.subject_id=b.subject_id
        JOIN action_events start ON start.action_id=ax.action_id AND start.outcome='requested'
        WHERE b.item_id IN (SELECT id FROM chosen_items) AND bc.sequence<=?2 AND bc.basis<>'inferred'
        AND start.sequence<=?1 AND start.sequence>=b.from_sequence AND (b.through_sequence IS NULL OR start.sequence<=b.through_sequence)
        AND NOT EXISTS(SELECT 1 FROM work_changes r WHERE r.retracts_id=bc.id AND r.sequence<=?2)
    ), selected_tags(id) AS NOT MATERIALIZED (
      SELECT t.id FROM work_tags t WHERE EXISTS(
        SELECT 1 FROM work_tag_assignments a JOIN work_changes c ON c.id=a.change_id
        WHERE a.tag_id=t.id AND c.sequence<=?2 AND (a.item_id IN (SELECT id FROM chosen_items) OR EXISTS(SELECT 1 FROM selected_actions sa WHERE sa.id=a.action_id))
      )
    ), base_changes(id) AS NOT MATERIALIZED (
      SELECT c.id FROM work_changes c WHERE c.sequence<=?2 AND (
        EXISTS(SELECT 1 FROM work_items i WHERE i.created_change_id=c.id AND i.id IN (SELECT id FROM chosen_items)) OR
        EXISTS(SELECT 1 FROM work_item_bindings b WHERE b.change_id=c.id AND b.item_id IN (SELECT id FROM chosen_items)) OR
        EXISTS(SELECT 1 FROM work_tags t WHERE t.created_change_id=c.id AND EXISTS(SELECT 1 FROM selected_tags st WHERE st.id=t.id)) OR
        EXISTS(SELECT 1 FROM work_tag_labels l WHERE l.change_id=c.id AND EXISTS(SELECT 1 FROM selected_tags st WHERE st.id=l.tag_id)) OR
        EXISTS(SELECT 1 FROM work_tag_assignments a WHERE a.change_id=c.id AND (a.item_id IN (SELECT id FROM chosen_items) OR EXISTS(SELECT 1 FROM selected_actions sa WHERE sa.id=a.action_id))) OR
        EXISTS(SELECT 1 FROM work_relations r WHERE r.change_id=c.id AND (r.from_item_id IN (SELECT id FROM chosen_items) OR r.to_item_id IN (SELECT id FROM chosen_items)))
      )
    ), selected_changes(id) AS NOT MATERIALIZED (
      SELECT c.id FROM work_changes c WHERE c.sequence<=?2 AND (EXISTS(SELECT 1 FROM base_changes b WHERE b.id=c.id) OR EXISTS(SELECT 1 FROM base_changes b WHERE b.id=c.retracts_id))
    ), source_actions(id) AS MATERIALIZED (
      SELECT id FROM selected_actions
      UNION SELECT c.recording_action_id FROM work_changes c JOIN selected_changes sc ON sc.id=c.id
      UNION SELECT e.action_id FROM work_changes c JOIN selected_changes sc ON sc.id=c.id JOIN action_events e ON e.id=c.evidence_event_id
    )");
    (sql, values)
}
fn ambiguous_action(connection: &Connection, action: &ActionId, cut: &Snapshot) -> Result<bool> {
    let start = requested_sequence(connection, action)?;
    Ok(connection.query_row("SELECT EXISTS(SELECT 1 FROM action_subjects ax JOIN work_item_bindings b ON b.subject_id=ax.subject_id JOIN work_changes c ON c.id=b.change_id WHERE ax.action_id=?1 AND c.sequence<=?2 AND c.basis<>'inferred' AND b.from_sequence<=?3 AND (b.through_sequence IS NULL OR b.through_sequence>=?3) AND NOT EXISTS(SELECT 1 FROM work_changes r WHERE r.retracts_id=c.id AND r.sequence<=?2) GROUP BY b.subject_id HAVING count(DISTINCT b.item_id)>1 LIMIT 1)",params![action.as_str(),cut.map_sequence,start],|r|r.get(0))?)
}
fn load_gap(connection: &Connection, sequence: i64) -> Result<RecordingGap> {
    Ok(connection.query_row("SELECT sequence,id,session_id,observed_at_ms,recorded_at_ms,dropped_records,write_errors FROM recording_gaps WHERE sequence=?1",[sequence],|r|Ok(RecordingGap {sequence:r.get(0)?,id:EventId(r.get(1)?),session_id:SessionId(r.get(2)?),observed_at_ms:r.get(3)?,recorded_at_ms:r.get(4)?,dropped_records:r.get(5)?,write_errors:r.get(6)?}))?)
}

/// Only fixed-size, validated claim fields are materialized by catalog reads.
struct RawLabelClaim {
    id: String,
    sequence: i64,
    label: String,
    recording_action_id: String,
    evidence_event_id: String,
}

impl RawLabelClaim {
    fn read(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(offset)?,
            sequence: row.get(offset + 1)?,
            label: row.get(offset + 2)?,
            recording_action_id: row.get(offset + 3)?,
            evidence_event_id: row.get(offset + 4)?,
        })
    }

    fn into_claim(self) -> Result<LabelClaim> {
        Ok(LabelClaim {
            change_id: ChangeId::parse(&self.id)?,
            sequence: self.sequence,
            label: SemanticLabel::new(&self.label)?,
            provenance: Provenance {
                recording_action_id: ActionId::parse(&self.recording_action_id)?,
                evidence_event_id: EventId::parse(&self.evidence_event_id)?,
                basis: Basis::UserAsserted,
            },
        })
    }
}
