//! Bounded discovery of original observations, independent of item bindings.

use super::*;
use rusqlite::types::Value;

/// All supplied predicates are exact and combined with AND. The outcome and
/// time range select events, never an inferred latest state of an action.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventFilter {
    pub session_id: Option<SessionId>,
    pub action_id: Option<ActionId>,
    pub root_action_id: Option<ActionId>,
    pub kind: Option<ActionKind>,
    pub subject: Option<SubjectKey>,
    pub outcome: Option<Outcome>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
}

impl EventFilter {
    fn validate(&self) -> Result<()> {
        for id in [
            self.session_id.as_ref().map(SessionId::as_str),
            self.action_id.as_ref().map(ActionId::as_str),
            self.root_action_id.as_ref().map(ActionId::as_str),
        ]
        .into_iter()
        .flatten()
        {
            validate_uuid(id)?;
        }
        if let Some(kind) = &self.kind {
            StableCode::try_new(kind.as_str())?;
        }
        if let Some(subject) = &self.subject {
            subject.validate()?;
        }
        for time in [self.since_ms, self.until_ms].into_iter().flatten() {
            validate_timestamp(time)?;
        }
        ensure!(
            !matches!((self.since_ms, self.until_ms), (Some(start), Some(end)) if start > end),
            "event query time range is reversed"
        );
        encode_safe_output(self)?;
        Ok(())
    }

    /// One predicate serves both event pagination and snapshot-wide gap scope.
    fn matching_sql(&self, highwater: i64) -> (String, Vec<Value>) {
        let mut sql = String::from(
            "FROM action_events e JOIN actions a ON a.id=e.action_id WHERE e.sequence<=?",
        );
        let mut values: Vec<Value> = vec![highwater.into()];
        for (column, value) in [
            (
                "a.session_id",
                self.session_id.as_ref().map(SessionId::as_str),
            ),
            ("a.id", self.action_id.as_ref().map(ActionId::as_str)),
            (
                "a.root_action_id",
                self.root_action_id.as_ref().map(ActionId::as_str),
            ),
            ("a.kind", self.kind.as_ref().map(StableCode::as_str)),
            ("e.outcome", self.outcome.map(Outcome::as_str)),
        ] {
            if let Some(value) = value {
                sql.push_str(&format!(" AND {column}=?"));
                values.push(value.to_owned().into());
            }
        }
        for (clause, value) in [
            ("e.occurred_at_ms>=?", self.since_ms),
            ("e.occurred_at_ms<=?", self.until_ms),
        ] {
            if let Some(value) = value {
                sql.push_str(&format!(" AND {clause}"));
                values.push(value.into());
            }
        }
        // No event before the first one observed since `since_ms` can match:
        // start there (events_time holds it) instead of at the journal's
        // first event. Observed times need not follow sequence, so the bound
        // is the least sequence among them, not the sequence at that time.
        if let Some(since) = self.since_ms {
            sql.push_str(" AND e.sequence>=(SELECT COALESCE(min(x.sequence),?+1) FROM action_events x WHERE x.occurred_at_ms>=?)");
            values.push(highwater.into());
            values.push(since.into());
        }
        if let Some(subject) = &self.subject {
            sql.push_str(" AND EXISTS (SELECT 1 FROM action_subjects ax JOIN subjects s ON s.id=ax.subject_id WHERE ax.action_id=a.id AND s.provider=? AND s.instance=? AND s.kind=? AND s.external_id=?)");
            for value in [
                &subject.provider,
                &subject.instance,
                &subject.kind,
                &subject.id,
            ] {
                values.push(value.clone().into());
            }
        }
        (sql, values)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventCursor {
    snapshot: Snapshot,
    filter: EventFilter,
    after_sequence: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventEntry {
    pub event: ActionEvent,
    pub source: ActionSource,
    /// Binding intervals use this original Requested sequence, not the
    /// sequence of a later outcome. It does not select an interval for a caller.
    pub requested_sequence: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventPage {
    pub version: u32,
    pub snapshot: Snapshot,
    pub filter: EventFilter,
    pub events: Vec<EventEntry>,
    pub truncated: bool,
    pub next: Option<EventCursor>,
    pub authority: &'static str,
    pub task_instruction: &'static str,
    pub execution_authorization: &'static str,
    /// Session-level gap evidence for the whole filtered snapshot, not only
    /// this page. It cannot identify which actions were lost.
    pub known_recording_gaps: bool,
    pub completeness_not_guaranteed: bool,
}

impl Journal {
    pub fn query_events(
        &self,
        filter: &EventFilter,
        limits: Limits,
        cursor: Option<&EventCursor>,
    ) -> Result<EventPage> {
        self.query_events_at(filter, limits, cursor, None)
    }

    /// Discover original events without creating or resolving item identities.
    /// The explicit first-page cut lets an audited caller exclude its own audit.
    pub fn query_events_at(
        &self,
        filter: &EventFilter,
        limits: Limits,
        cursor: Option<&EventCursor>,
        at: Option<&Snapshot>,
    ) -> Result<EventPage> {
        limits.validate()?;
        filter.validate()?;
        if let Some(cursor) = cursor {
            ensure!(
                cursor.filter == *filter && cursor.after_sequence >= 0,
                "event cursor belongs to a different filter or position"
            );
        }
        let tx = self.connection.unchecked_transaction()?;
        let cut = query::checked_page_snapshot(&tx, cursor.map(|c| &c.snapshot), at)?;
        let mut after = cursor.map_or(0, |c| c.after_sequence);
        ensure!(
            after <= cut.event_sequence,
            "event cursor exceeds its snapshot"
        );
        let (matching, values) = filter.matching_sql(cut.event_sequence);
        // The sessions with a matching event are found once, not once per gap.
        let mut gap_values = vec![Value::from(cut.gap_sequence)];
        gap_values.extend(values.iter().cloned());
        let known_recording_gaps: bool = tx.query_row(
            &format!("SELECT EXISTS(SELECT 1 FROM recording_gaps g WHERE g.sequence<=? AND g.session_id IN (SELECT a.session_id {matching}))"),
            rusqlite::params_from_iter(gap_values),
            |row| row.get(0),
        )?;
        let mut page_values = values;
        page_values.push(after.into());
        page_values.push(((limits.records + 1) as i64).into());
        let mut statement = tx.prepare(&format!(
            "SELECT e.sequence {matching} AND e.sequence>? ORDER BY e.sequence LIMIT ?"
        ))?;
        let sequences = statement
            .query_map(rusqlite::params_from_iter(page_values), |row| {
                row.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut page = EventPage {
            version: VERSION,
            snapshot: cut.clone(),
            filter: filter.clone(),
            events: Vec::new(),
            truncated: false,
            next: None,
            authority: "observations_only",
            task_instruction: "unspecified",
            execution_authorization: "unspecified",
            known_recording_gaps,
            completeness_not_guaranteed: true,
        };
        let next = |sequence| EventCursor {
            snapshot: cut.clone(),
            filter: filter.clone(),
            after_sequence: sequence,
        };
        let mut record_bytes = 0usize;
        for (index, sequence) in sequences.iter().copied().enumerate() {
            if page.events.len() == limits.records {
                page.truncated = true;
                page.next = Some(next(after));
                break;
            }
            // The selected immutable sequence must be loaded exactly; never
            // replace missing evidence with the next unrelated event.
            let event = self
                .query(&Query {
                    after_sequence: Some(sequence - 1),
                    limit: 1,
                    ..Query::default()
                })?
                .pop()
                .context("discovery event disappeared")?;
            ensure!(
                event.sequence == sequence,
                "discovery event sequence mismatch"
            );
            let entry = EventEntry {
                source: store::action_source(&tx, &event.action_id)?,
                requested_sequence: store::requested_sequence(&tx, &event.action_id)?,
                event,
            };
            ensure!(
                entry.requested_sequence <= entry.event.sequence,
                "event precedes its action request"
            );
            let bytes = encode_safe_output(&entry)?.len() + usize::from(!page.events.is_empty());
            let more = index + 1 < sequences.len();
            // Account for the complete compact envelope, including a cursor
            // only when another record remains. No growing payload is cloned.
            page.truncated = more;
            page.next = more.then(|| next(sequence));
            let envelope_bytes = encode_safe_output(&page.clone_without_events())?.len();
            if envelope_bytes + record_bytes + bytes > limits.bytes {
                ensure!(
                    !page.events.is_empty(),
                    "one event record exceeds the byte budget; request a larger bounded page"
                );
                page.truncated = true;
                page.next = Some(next(after));
                break;
            }
            page.events.push(entry);
            record_bytes += bytes;
            after = sequence;
        }
        ensure!(
            encode_safe_output(&page)?.len() <= limits.bytes,
            "event envelope exceeds the byte budget"
        );
        tx.commit()?;
        Ok(page)
    }
}

impl EventPage {
    fn clone_without_events(&self) -> Self {
        Self {
            version: self.version,
            snapshot: self.snapshot.clone(),
            filter: self.filter.clone(),
            events: Vec::new(),
            truncated: self.truncated,
            next: self.next.clone(),
            authority: self.authority,
            task_instruction: self.task_instruction,
            execution_authorization: self.execution_authorization,
            known_recording_gaps: self.known_recording_gaps,
            completeness_not_guaranteed: self.completeness_not_guaranteed,
        }
    }
}
