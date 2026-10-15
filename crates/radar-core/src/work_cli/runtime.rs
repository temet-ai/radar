//! Ordered runner-owned execution. Claims retain original provenance on replay.

use serde::{Deserialize, Serialize};

use super::{Batch, Command, Error, Evidence, Execution, Intent, MAX_OUTPUT_BYTES, Reply, VERSION};
use crate::journal::work_map::{
    ChangeId, ChangeRecord, ChangeRequest, Limits, Mutation, Provenance, Snapshot,
    encode_safe_output, validate_change_request,
};
use crate::journal::{
    ActionDescriptor, ActionId, EventId, Journal, Metadata, Origin, Outcome, Query, Reason,
    SessionId, StableCode, WriteMode,
};
use crate::writes::{Change, Mode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub action_id: ActionId,
    pub requested_event_id: EventId,
    pub requested_sequence: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeReceipt {
    pub change_id: ChangeId,
    pub sequence: i64,
    pub original_recording_action_id: ActionId,
    pub evidence_event_id: EventId,
    pub observed_at_ms: i64,
    pub replayed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyReceipt {
    pub version: u32,
    pub attempt: Attempt,
    pub changes: Vec<ChangeReceipt>,
    pub recording_degraded: bool,
}

#[derive(Serialize)]
struct Lookup<T: Serialize> {
    version: u32,
    snapshot: Snapshot,
    result: Option<T>,
}

enum Body {
    Applied(ApplyReceipt),
    Query(Vec<u8>),
}

pub(crate) fn execute(
    journal: &mut Journal,
    session: &SessionId,
    command: Command,
    mode: Mode,
    observed_at_ms: i64,
) -> Execution {
    let mut recording_degraded = false;
    let result = execute_inner(
        journal,
        session,
        command,
        mode,
        observed_at_ms,
        &mut recording_degraded,
    );
    Execution {
        result,
        recording_degraded,
    }
}

fn execute_inner(
    journal: &mut Journal,
    session: &SessionId,
    command: Command,
    mode: Mode,
    observed_at_ms: i64,
    audit_degraded: &mut bool,
) -> Result<Reply, Error> {
    // Capture before this invocation's audit event. Continuations use their own cut.
    let before = journal.work_map_snapshot().map_err(|_| {
        *audit_degraded = true;
        Error::Storage
    })?;
    let id = ActionId::new();
    let mut descriptor = ActionDescriptor::new(command.code(), Origin::Host);
    descriptor.metadata.command = Some(StableCode::new(command.code()));
    descriptor.metadata.mode = Some(match mode {
        Mode::ReadOnly => WriteMode::ReadOnly,
        Mode::Write => WriteMode::Write,
        Mode::Edit => WriteMode::Edit,
    });
    journal
        .begin_with_id_at(session, &id, descriptor, None, observed_at_ms)
        .map_err(|_| {
            *audit_degraded = true;
            Error::Storage
        })?;
    let requested = journal
        .query(&Query {
            action_id: Some(id.clone()),
            limit: 1,
            ..Default::default()
        })
        .map_err(|_| {
            *audit_degraded = true;
            Error::Storage
        })?
        .into_iter()
        .next()
        .filter(|event| event.outcome == Outcome::Requested)
        .ok_or_else(|| {
            *audit_degraded = true;
            Error::Storage
        })?;
    let attempt = Attempt {
        action_id: id.clone(),
        requested_event_id: requested.event_id,
        requested_sequence: requested.sequence,
    };
    let result = (|| {
        if let Command::Apply(batch) = &command
            && batch
                .changes
                .iter()
                .any(|intent| !mode.allows(permission(&intent.mutation)))
        {
            return Err(Error::PermissionDenied);
        }
        journal
            .record(&id, Outcome::Started, Metadata::default())
            .map_err(|_| {
                *audit_degraded = true;
                Error::Storage
            })?;
        match command {
            Command::Apply(batch) => {
                apply(journal, &attempt, &batch, observed_at_ms).map(Body::Applied)
            }
            command => query(journal, command, &before).map(Body::Query),
        }
    })();
    let (outcome, reason) = match &result {
        Ok(_) => (Outcome::Succeeded, None),
        Err(Error::PermissionDenied) => (Outcome::Blocked, Some(Reason::PermissionDenied)),
        Err(Error::InvalidInput | Error::Conflict) => {
            (Outcome::Blocked, Some(Reason::InvalidInput))
        }
        Err(Error::OutcomeUnknown) => (Outcome::Unknown, Some(Reason::Storage)),
        Err(_) => (Outcome::Failed, Some(Reason::Storage)),
    };
    let recording_degraded = journal
        .record(
            &id,
            outcome,
            Metadata {
                reason,
                ..Default::default()
            },
        )
        .is_err();
    *audit_degraded |= recording_degraded;
    let bytes = match result? {
        Body::Applied(mut receipt) => {
            receipt.recording_degraded = *audit_degraded;
            // The mutation is committed even if its response cannot be emitted.
            output(&receipt).map_err(|_| Error::CommittedReceiptUnavailable)?
        }
        Body::Query(bytes) => bytes,
    };
    Ok(Reply { bytes })
}

fn permission(mutation: &Mutation) -> Change {
    match mutation {
        Mutation::RenameTag { .. } | Mutation::Retract { .. } => Change::Edit,
        _ => Change::Add,
    }
}

fn output(value: &impl Serialize) -> Result<Vec<u8>, Error> {
    let bytes = encode_safe_output(value).map_err(|_| Error::Storage)?;
    if bytes.len() > MAX_OUTPUT_BYTES {
        return Err(Error::Storage);
    }
    Ok(bytes)
}

fn query(journal: &Journal, command: Command, before: &Snapshot) -> Result<Vec<u8>, Error> {
    match command {
        Command::Handoff(query) => {
            let at = query.cursor.is_none().then_some(before);
            let page = journal
                .handoff_at(
                    &query.selection,
                    Limits {
                        records: query.records,
                        bytes: query.bytes,
                    },
                    query.cursor.as_ref(),
                    at,
                )
                .map_err(|_| Error::Storage)?;
            output(&page)
        }
        Command::Items(query) => {
            let at = query.cursor.is_none().then_some(before);
            let page = journal
                .query_items_at(&query.filter, query.limit, query.cursor.as_ref(), at)
                .map_err(|_| Error::Storage)?;
            output(&page)
        }
        Command::Events(query) => {
            let at = query.cursor.is_none().then_some(before);
            let page = journal
                .query_events_at(
                    &query.filter,
                    Limits {
                        records: query.records,
                        bytes: query.bytes,
                    },
                    query.cursor.as_ref(),
                    at,
                )
                .map_err(|_| Error::Storage)?;
            output(&page)
        }
        Command::Tag(query) => output(&Lookup {
            version: VERSION,
            snapshot: before.clone(),
            result: journal
                .find_tag(&query.namespace, &query.key, Some(before))
                .map_err(|_| Error::Storage)?,
        }),
        Command::Change(query) => output(&Lookup {
            version: VERSION,
            snapshot: before.clone(),
            result: journal
                .get_work_change(&query.change_id, Some(before))
                .map_err(|_| Error::Storage)?,
        }),
        Command::Resolve(query) => {
            let resolution = journal
                .resolve_subject(&query.subject, &query.action_id, Some(before))
                .map_err(|_| Error::Storage)?;
            output(&resolution)
        }
        Command::Apply(_) => Err(Error::InvalidInput),
    }
}

pub(crate) fn same_intent(intent: &Intent, record: &ChangeRecord) -> bool {
    let request = &record.request;
    let submitted_assertion = record.evidence.outcome == Outcome::Requested
        && record.evidence.action.action_id == request.provenance.recording_action_id
        && record.recording.origin == Origin::Host
        && record.recording.kind.as_str() == "work.apply";
    let evidence_matches = match &intent.evidence {
        Evidence::Event { event_id } => {
            !submitted_assertion && event_id == &request.provenance.evidence_event_id
        }
        Evidence::Assertion => submitted_assertion,
    };
    intent.change_id == request.id
        && intent.mutation == request.mutation
        && intent.basis == request.provenance.basis
        && evidence_matches
}

fn existing(journal: &Journal, intents: &[Intent]) -> Result<Vec<Option<ChangeRecord>>, Error> {
    let mut records = Vec::with_capacity(intents.len());
    // All comparisons use one cut, even if another process is adding claims.
    let snapshot = journal
        .work_map_snapshot()
        .map_err(|_| Error::OutcomeUnknown)?;
    for intent in intents {
        let record = journal
            .get_work_change(&intent.change_id, Some(&snapshot))
            .map_err(|_| Error::OutcomeUnknown)?;
        if record
            .as_ref()
            .is_some_and(|record| !same_intent(intent, record))
        {
            return Err(Error::Conflict);
        }
        records.push(record);
    }
    Ok(records)
}

fn apply(
    journal: &mut Journal,
    attempt: &Attempt,
    batch: &Batch,
    observed_at_ms: i64,
) -> Result<ApplyReceipt, Error> {
    if batch.changes.is_empty() || batch.changes.len() > crate::journal::work_map::MAX_BATCH {
        return Err(Error::InvalidInput);
    }
    let mut ids = std::collections::HashSet::new();
    let mut requests = Vec::with_capacity(batch.changes.len());
    for intent in &batch.changes {
        if !ids.insert(intent.change_id.as_str())
            || (intent.evidence == Evidence::Assertion
                && intent.basis != crate::journal::work_map::Basis::UserAsserted)
        {
            return Err(Error::InvalidInput);
        }
        encode_safe_output(intent).map_err(|_| Error::InvalidInput)?;
        let evidence_event_id = match &intent.evidence {
            Evidence::Assertion => attempt.requested_event_id.clone(),
            Evidence::Event { event_id } => event_id.clone(),
        };
        let request = ChangeRequest {
            id: intent.change_id.clone(),
            provenance: Provenance {
                recording_action_id: attempt.action_id.clone(),
                evidence_event_id,
                basis: intent.basis,
            },
            observed_at_ms,
            mutation: intent.mutation.clone(),
        };
        validate_change_request(&request).map_err(|_| Error::InvalidInput)?;
        requests.push(request);
    }
    let mut retained = existing(journal, &batch.changes)?;
    #[cfg(test)]
    if let Some(hook) = BEFORE_APPLY.with(|slot| slot.borrow_mut().take()) {
        hook(journal);
    }
    loop {
        let known = retained.iter().filter(|record| record.is_some()).count();
        for (request, old) in requests.iter_mut().zip(&retained) {
            if let Some(old) = old {
                *request = old.request.clone();
            }
        }
        match journal.apply_work_map_batch(&requests) {
            Ok(sequences) => {
                let changes = requests
                    .iter()
                    .zip(sequences)
                    .zip(&retained)
                    .map(|((request, sequence), old)| ChangeReceipt {
                        change_id: request.id.clone(),
                        sequence,
                        original_recording_action_id: request
                            .provenance
                            .recording_action_id
                            .clone(),
                        evidence_event_id: request.provenance.evidence_event_id.clone(),
                        observed_at_ms: request.observed_at_ms,
                        replayed: old.is_some(),
                    })
                    .collect();
                return Ok(ApplyReceipt {
                    version: VERSION,
                    attempt: attempt.clone(),
                    changes,
                    recording_degraded: false,
                });
            }
            Err(_) => {
                let found = existing(journal, &batch.changes)?;
                let now_known = found.iter().filter(|record| record.is_some()).count();
                // A competing exact intent may have committed while we prepared
                // runtime provenance. Reuse its immutable request, never rewrite it.
                // Strict growth bounds this recovery loop by the batch length.
                if now_known <= known {
                    return Err(Error::OutcomeUnknown);
                }
                retained = found;
            }
        }
    }
}

#[cfg(test)]
type ApplyHook = Box<dyn FnOnce(&mut Journal)>;
#[cfg(test)]
thread_local! {
    static BEFORE_APPLY: std::cell::RefCell<Option<ApplyHook>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub(super) fn before_next_apply(hook: impl FnOnce(&mut Journal) + 'static) {
    BEFORE_APPLY.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}
