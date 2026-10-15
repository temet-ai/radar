use super::*;

impl Journal {
    pub fn apply_work_map(&mut self, request: &ChangeRequest) -> Result<i64> {
        Ok(self.apply_work_map_batch(std::slice::from_ref(request))?[0])
    }

    /// All new claims and corrections commit together, or none do. Exact
    /// caller-ID retries return their original sequences; conflicting retries
    /// fail the whole batch, including earlier new changes in this batch.
    pub fn apply_work_map_batch(&mut self, requests: &[ChangeRequest]) -> Result<Vec<i64>> {
        ensure!(
            !requests.is_empty() && requests.len() <= MAX_BATCH,
            "work map batch must contain 1..=16 changes"
        );
        let encoded = requests
            .iter()
            .map(validate_request)
            .collect::<Result<Vec<_>>>()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut sequences = Vec::with_capacity(requests.len());
        for (request, encoded) in requests.iter().zip(encoded) {
            let old: Option<(i64, String)> = tx
                .query_row(
                    "SELECT sequence,request_json FROM work_changes WHERE id=?1",
                    [request.id.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if let Some((sequence, old)) = old {
                ensure!(
                    old == encoded,
                    "work map mutation ID was reused with different content"
                );
                sequences.push(sequence);
                continue;
            }
            let recording = action_source(&tx, &request.provenance.recording_action_id)?;
            ensure_session_open(&tx, &recording.session_id)?;
            encode_safe_output(&recording)?;
            let evidence = evidence_source(&tx, &request.provenance.evidence_event_id)?;
            encode_safe_output(&evidence)?;
            validate_references(&tx, request, &evidence)?;
            let retracts = match &request.mutation {
                Mutation::Retract { claim_id, .. } => Some(claim_id.as_str()),
                _ => None,
            };
            tx.execute("INSERT INTO work_changes(id,recording_action_id,evidence_event_id,basis,kind,observed_at_ms,recorded_at_ms,request_json,retracts_id,version) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![request.id.as_str(),request.provenance.recording_action_id.as_str(),request.provenance.evidence_event_id.as_str(),request.provenance.basis.as_str(),request.mutation.kind(),request.observed_at_ms,now_ms()?,encoded,retracts,VERSION])?;
            let sequence = tx.last_insert_rowid();
            insert_detail(&tx, request)?;
            sequences.push(sequence);
        }
        tx.commit()?;
        Ok(sequences)
    }
}

pub(super) fn validate_request(request: &ChangeRequest) -> Result<String> {
    validate_uuid(request.id.as_str())?;
    validate_uuid(request.provenance.recording_action_id.as_str())?;
    validate_uuid(request.provenance.evidence_event_id.as_str())?;
    validate_timestamp(request.observed_at_ms)?;
    match &request.mutation {
        Mutation::CreateItem { item_id, kind } => {
            validate_uuid(item_id.as_str())?;
            validate_code(kind.as_str())?;
            ensure!(
                !crate::log::contains_secret(kind.as_str()),
                "work item kind may contain a credential"
            );
        }
        Mutation::BindSubject {
            item_id,
            subject,
            from_sequence,
            through_sequence,
        } => {
            validate_uuid(item_id.as_str())?;
            subject.validate()?;
            ensure!(
                [
                    &subject.provider,
                    &subject.instance,
                    &subject.kind,
                    &subject.id
                ]
                .iter()
                .all(|v| !crate::log::contains_secret(v)),
                "work binding identity may contain a credential"
            );
            ensure!(
                *from_sequence > 0 && through_sequence.is_none_or(|end| end >= *from_sequence),
                "invalid binding interval"
            );
        }
        Mutation::CreateTag {
            tag_id,
            namespace,
            key,
            label,
        } => {
            validate_uuid(tag_id.as_str())?;
            TagKey::new(namespace.as_str())?;
            TagKey::new(key.as_str())?;
            SemanticLabel::new(label.as_str())?;
        }
        Mutation::RenameTag { tag_id, label } => {
            validate_uuid(tag_id.as_str())?;
            SemanticLabel::new(label.as_str())?;
        }
        Mutation::AssignTag { tag_id, target } => {
            validate_uuid(tag_id.as_str())?;
            validate_tag_target(target)?;
        }
        Mutation::RelateItems { from, to, .. } => {
            validate_uuid(from.as_str())?;
            validate_uuid(to.as_str())?;
            ensure!(from != to, "self relations are not supported");
        }
        Mutation::Retract { claim_id, expected } => {
            validate_uuid(claim_id.as_str())?;
            validate_assertion_target(expected)?;
        }
    }
    let encoded = serde_json::to_string(request)?;
    ensure!(
        encoded.len() <= MAX_REQUEST_BYTES,
        "work map request exceeds the storage limit"
    );
    Ok(encoded)
}

fn validate_tag_target(target: &TagTarget) -> Result<()> {
    match target {
        TagTarget::Item(id) => {
            validate_uuid(id.as_str())?;
        }
        TagTarget::Action(id) => {
            validate_uuid(id.as_str())?;
        }
    }
    Ok(())
}
fn validate_assertion_target(target: &AssertionTarget) -> Result<()> {
    match target {
        AssertionTarget::Binding { item_id, subject } => {
            validate_uuid(item_id.as_str())?;
            subject.validate()?;
        }
        AssertionTarget::LabelAssertion { tag_id } => {
            validate_uuid(tag_id.as_str())?;
        }
        AssertionTarget::TagAssignment { tag_id, target } => {
            validate_uuid(tag_id.as_str())?;
            validate_tag_target(target)?;
        }
        AssertionTarget::Relation { from, to, .. } => {
            validate_uuid(from.as_str())?;
            validate_uuid(to.as_str())?;
        }
    }
    Ok(())
}

fn validate_references(
    tx: &Transaction<'_>,
    request: &ChangeRequest,
    evidence: &EvidenceSource,
) -> Result<()> {
    match &request.mutation {
        Mutation::BindSubject {
            item_id,
            subject,
            from_sequence,
            through_sequence,
        } => {
            let kind: String = tx
                .query_row(
                    "SELECT kind FROM work_items WHERE id=?1",
                    [item_id.as_str()],
                    |r| r.get(0),
                )
                .optional()?
                .context("binding item does not exist")?;
            ensure!(
                kind == subject.kind,
                "binding kind differs from immutable item kind"
            );
            let requested: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM action_events WHERE sequence=?1 AND outcome='requested')", [from_sequence], |r| r.get(0))?;
            ensure!(
                requested,
                "binding start must name an existing Requested-event sequence"
            );
            if let Some(end) = through_sequence {
                let high: i64 = tx.query_row(
                    "SELECT COALESCE(max(sequence),0) FROM action_events",
                    [],
                    |r| r.get(0),
                )?;
                ensure!(*end <= high, "binding end exceeds observed history");
            }
            if request.provenance.basis == Basis::Observed {
                let present: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM action_subjects ax JOIN subjects s ON s.id=ax.subject_id WHERE ax.action_id=?1 AND s.provider=?2 AND s.instance=?3 AND s.kind=?4 AND s.external_id=?5)",
                    params![evidence.action.action_id.as_str(),subject.provider,subject.instance,subject.kind,subject.id], |r| r.get(0))?;
                ensure!(
                    present,
                    "observed binding evidence does not contain the scoped subject"
                );
                let start = requested_sequence(tx, &evidence.action.action_id)?;
                ensure!(
                    start >= *from_sequence && through_sequence.is_none_or(|end| start <= end),
                    "observed binding evidence is outside the claimed action interval"
                );
            }
        }
        Mutation::Retract { claim_id, expected } => {
            let (encoded, retracted): (String,bool) = tx.query_row("SELECT request_json,EXISTS(SELECT 1 FROM work_changes r WHERE r.retracts_id=c.id) FROM work_changes c WHERE id=?1", [claim_id.as_str()], |r| Ok((r.get(0)?,r.get(1)?))).optional()?.context("work map retraction target does not exist")?;
            let original: ChangeRequest = serde_json::from_str(&encoded)?;
            ensure!(
                original.mutation.target().as_ref() == Some(expected),
                "work map retraction type or target mismatch"
            );
            ensure!(!retracted, "work map claim is already retracted");
            ensure!(
                request.provenance.basis != Basis::Inferred
                    || original.provenance.basis == Basis::Inferred,
                "an inference cannot withdraw a non-inferred claim"
            );
        }
        _ => {}
    }
    Ok(())
}

fn insert_detail(tx: &Transaction<'_>, request: &ChangeRequest) -> Result<()> {
    let change = request.id.as_str();
    match &request.mutation {
        Mutation::CreateItem { item_id, kind } => {
            tx.execute(
                "INSERT INTO work_items(id,kind,created_change_id) VALUES (?1,?2,?3)",
                params![item_id.as_str(), kind.as_str(), change],
            )?;
        }
        Mutation::BindSubject {
            item_id,
            subject,
            from_sequence,
            through_sequence,
        } => {
            tx.execute("INSERT INTO subjects(provider,instance,kind,external_id) VALUES (?1,?2,?3,?4) ON CONFLICT DO NOTHING", params![subject.provider,subject.instance,subject.kind,subject.id])?;
            let subject_id: i64 = tx.query_row("SELECT id FROM subjects WHERE provider=?1 AND instance=?2 AND kind=?3 AND external_id=?4", params![subject.provider,subject.instance,subject.kind,subject.id], |r| r.get(0))?;
            tx.execute("INSERT INTO work_item_bindings(change_id,item_id,subject_id,from_sequence,through_sequence) VALUES (?1,?2,?3,?4,?5)", params![change,item_id.as_str(),subject_id,from_sequence,through_sequence])?;
        }
        Mutation::CreateTag {
            tag_id,
            namespace,
            key,
            label,
        } => {
            tx.execute("INSERT INTO work_tags(id,namespace,stable_key,created_change_id) VALUES (?1,?2,?3,?4)", params![tag_id.as_str(),namespace.as_str(),key.as_str(),change])?;
            tx.execute(
                "INSERT INTO work_tag_labels(change_id,tag_id,label) VALUES (?1,?2,?3)",
                params![change, tag_id.as_str(), label.as_str()],
            )?;
        }
        Mutation::RenameTag { tag_id, label } => {
            tx.execute(
                "INSERT INTO work_tag_labels(change_id,tag_id,label) VALUES (?1,?2,?3)",
                params![change, tag_id.as_str(), label.as_str()],
            )?;
        }
        Mutation::AssignTag { tag_id, target } => {
            let (item, action) = match target {
                TagTarget::Item(id) => (Some(id.as_str()), None),
                TagTarget::Action(id) => (None, Some(id.as_str())),
            };
            tx.execute("INSERT INTO work_tag_assignments(change_id,tag_id,item_id,action_id) VALUES (?1,?2,?3,?4)", params![change,tag_id.as_str(),item,action])?;
        }
        Mutation::RelateItems { from, to, kind } => {
            tx.execute("INSERT INTO work_relations(change_id,from_item_id,to_item_id,kind) VALUES (?1,?2,?3,?4)", params![change,from.as_str(),to.as_str(),kind.as_str()])?;
        }
        Mutation::Retract { .. } => {}
    }
    Ok(())
}

pub(super) fn requested_sequence(connection: &Connection, action: &ActionId) -> Result<i64> {
    connection.query_row("SELECT sequence FROM action_events WHERE action_id=?1 AND outcome='requested' ORDER BY sequence LIMIT 1", [action.as_str()], |r| r.get(0)).optional()?.context("action has no Requested observation")
}

pub(super) fn action_source(connection: &Connection, action: &ActionId) -> Result<ActionSource> {
    let (session,origin_kind,origin_id,kind,build,session_origin): (String,String,String,String,String,String) = connection.query_row("SELECT a.session_id,a.origin_kind,a.origin_id,a.kind,s.build_version,s.origin FROM actions a JOIN sessions s ON s.id=a.session_id WHERE a.id=?1", [action.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?))).optional()?.context("recording action does not exist")?;
    Ok(ActionSource {
        action_id: action.clone(),
        session_id: SessionId(session),
        origin: if origin_kind == "host" {
            Origin::Host
        } else {
            Origin::Plugin(origin_id)
        },
        kind: StableCode::try_new(&kind)?,
        build_version: build,
        session_origin: serde_json::from_value(serde_json::Value::String(session_origin))?,
    })
}
pub(super) fn evidence_source(connection: &Connection, event: &EventId) -> Result<EvidenceSource> {
    let (action,sequence,outcome,observed,recorded,estimated,event_version,metadata): (String,i64,String,i64,i64,bool,u32,String) = connection.query_row("SELECT action_id,sequence,outcome,occurred_at_ms,COALESCE(recorded_at_ms,occurred_at_ms),recorded_at_ms IS NULL,event_version,metadata_json FROM action_events WHERE id=?1", [event.as_str()], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?.context("work map evidence event does not exist")?;
    Ok(EvidenceSource {
        event_version,
        metadata: serde_json::from_str(&metadata)?,
        event_id: event.clone(),
        sequence,
        outcome: Outcome::parse(&outcome)?,
        observed_at_ms: observed,
        recorded_at_ms: recorded,
        observation_time_estimated: estimated,
        action: action_source(connection, &ActionId(action))?,
    })
}

pub(super) fn load_change(
    connection: &Connection,
    sequence: i64,
    cut: &Snapshot,
) -> Result<ChangeRecord> {
    let (encoded,recorded,retracted,version): (String,i64,bool,u32) = connection.query_row("SELECT request_json,recorded_at_ms,EXISTS(SELECT 1 FROM work_changes r WHERE r.retracts_id=c.id AND r.sequence<=?2),version FROM work_changes c WHERE sequence=?1 AND sequence<=?2", params![sequence,cut.map_sequence], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?)))?;
    ensure!(version == VERSION, "unsupported work map fact version");
    let request: ChangeRequest = serde_json::from_str(&encoded)?;
    validate_request(&request)?;
    let recording = action_source(connection, &request.provenance.recording_action_id)?;
    let evidence = evidence_source(connection, &request.provenance.evidence_event_id)?;
    ensure!(
        evidence.sequence <= cut.event_sequence,
        "snapshot excludes required claim evidence"
    );
    let record = ChangeRecord {
        version,
        sequence,
        request,
        recorded_at_ms: recorded,
        recording,
        evidence,
        retracted,
    };
    encode_safe_output(&record)?;
    Ok(record)
}

pub(super) fn validate_schema(connection: &Connection) -> Result<()> {
    for sql in [
        "SELECT singleton,id FROM work_map_store LIMIT 0",
        "SELECT sequence,version,id,recording_action_id,evidence_event_id,basis,kind,observed_at_ms,recorded_at_ms,request_json,retracts_id FROM work_changes LIMIT 0",
        "SELECT id,kind,created_change_id FROM work_items LIMIT 0",
        "SELECT change_id,item_id,subject_id,from_sequence,through_sequence FROM work_item_bindings LIMIT 0",
        "SELECT id,namespace,stable_key,created_change_id FROM work_tags LIMIT 0",
        "SELECT change_id,tag_id,label FROM work_tag_labels LIMIT 0",
        "SELECT change_id,tag_id,item_id,action_id FROM work_tag_assignments LIMIT 0",
        "SELECT change_id,from_item_id,to_item_id,kind FROM work_relations LIMIT 0",
    ] {
        connection
            .prepare(sql)
            .context("incomplete work map schema")?;
    }
    let id: String =
        connection.query_row("SELECT id FROM work_map_store WHERE singleton=1", [], |r| {
            r.get(0)
        })?;
    Uuid::parse_str(&id).context("invalid work map database identity")?;
    for table in [
        "work_map_store",
        "work_changes",
        "work_items",
        "work_item_bindings",
        "work_tags",
        "work_tag_labels",
        "work_tag_assignments",
        "work_relations",
    ] {
        for operation in ["update", "delete"] {
            let name = format!("{table}_no_{operation}");
            let exists: bool = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger' AND name=?1)",
                [name],
                |r| r.get(0),
            )?;
            ensure!(exists, "missing work map immutability guard");
        }
    }
    Ok(())
}
