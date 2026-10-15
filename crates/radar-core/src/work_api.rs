//! Renderer-neutral, bounded access to the explicit local work map.
//!
//! Plugins return requests; only the runner's ordered journal queue executes
//! them. Reads neither audit themselves nor flush recording gaps. Mutations use
//! the existing audited CLI engine, including its mode and exact-retry checks.
//! No label, grouping, binding, identity, relation or authority is inferred here.

use std::fmt;

use crate::journal::work_map::{
    Basis, ChangeRequest, Handoff, HandoffCursor, HandoffRecord, ItemCursor, ItemFilter, ItemId,
    ItemPage, Limits, Mutation, Provenance, Resolution, Selection, Snapshot, TagCursor, TagKey,
    TagPage,
};
use crate::journal::{ActionId, EventId, Journal, Outcome, Query, SessionId, SubjectKey};
use crate::writes::Mode;

pub use crate::work_cli::{ApplyReceipt, Attempt, Batch, ChangeReceipt, Evidence, Intent};

/// Each item page has a separate, bounded metadata page at its exact snapshot.
pub const ITEM_LIMIT: usize = 16;
/// Authored definitions are paged independently of item/evidence discovery.
pub const TAG_LIMIT: usize = 16;
/// This budget applies to each complete evidence page, including its envelope.
pub const EVIDENCE_LIMITS: Limits = Limits {
    records: 100,
    bytes: 262_144,
};
/// The selected item plus its neighbours never exceeds the storage selection cap.
pub const NEIGHBOUR_LIMIT: usize = 31;

#[derive(Clone, Default)]
pub struct ItemsRequest {
    pub filter: ItemFilter,
    pub cursor: Option<ItemCursor>,
    /// Retain the first page's snapshot when replaying an earlier page.
    pub snapshot: Option<Snapshot>,
}

#[derive(Clone)]
pub struct TagsRequest {
    pub namespace: TagKey,
    pub cursor: Option<TagCursor>,
    /// Share the list's cut; catalog paging does not refresh item membership.
    pub snapshot: Option<Snapshot>,
}

#[derive(Clone)]
pub struct DetailRequest {
    pub item_id: ItemId,
    /// Usually the selected list page's snapshot. None deliberately refreshes.
    pub snapshot: Option<Snapshot>,
}

#[derive(Clone)]
pub struct EvidenceRequest {
    /// Copy this from the corresponding primary or related Handoff unchanged.
    pub selection: Selection,
    pub cursor: Option<HandoffCursor>,
    pub snapshot: Option<Snapshot>,
}

#[derive(Clone)]
pub struct ResolveRequest {
    pub subject: SubjectKey,
    pub action_id: ActionId,
    pub snapshot: Option<Snapshot>,
}

#[derive(Clone)]
pub struct ResolveResponse {
    pub resolution: Resolution,
    /// The original Requested event, never a later outcome or current highwater.
    pub requested_sequence: i64,
}

#[derive(Clone)]
pub enum Request {
    Items(ItemsRequest),
    Tags(TagsRequest),
    Detail(DetailRequest),
    /// Independent continuation for either section; never expands another hop.
    Evidence(EvidenceRequest),
    Resolve(ResolveRequest),
    /// Reconcile retained identities against one authoritative snapshot before
    /// deciding whether to revalidate sources or retry the exact saved intent.
    InspectBatch(Batch),
    /// Allocate and retain all identities once, including across uncertain retries.
    Apply(Batch),
}

impl Request {
    pub fn is_mutation(&self) -> bool {
        matches!(self, Self::Apply(_))
    }
}

/// Authored labels remain claims in `evidence`, with provenance and retractions.
/// A missing label on a truncated page is unknown, not proof there is no label.
#[derive(Clone)]
pub struct ItemsResponse {
    pub page: ItemPage,
    pub evidence: Option<Handoff>,
}

#[derive(Clone)]
pub struct DetailResponse {
    pub selected: ItemId,
    pub primary: Handoff,
    /// At most one hop through live, non-inferred, explicitly asserted relations.
    /// Other relations appearing in this evidence never trigger further reads.
    pub related: Option<Handoff>,
    /// True if the first primary page or neighbour cap prevents full discovery.
    /// Each evidence page also retains its own independent truncation/cursor.
    pub expansion_truncated: bool,
}

/// Exact retained intent found at one cut. This is not permission to mutate or
/// proof that its source locators still belong to the current ready owners.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatchState {
    /// Every identity exists with the same mutation, basis and original evidence.
    AllPresent,
    /// No identity exists. Fresh owner validation is still required before apply.
    NonePresent,
    /// Keep the batch and inspect it; never silently discard or reallocate IDs.
    Partial,
}

#[derive(Clone)]
pub enum Response {
    Items(ItemsResponse),
    Tags(TagPage),
    Detail(DetailResponse),
    Evidence(Handoff),
    Resolved(ResolveResponse),
    BatchState(BatchState),
    Applied(ApplyReceipt),
}

// Never accidentally copy authored annotations or source identities to logs.
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Items(_) => "WorkRequest::Items",
            Self::Tags(_) => "WorkRequest::Tags",
            Self::Detail(_) => "WorkRequest::Detail",
            Self::Evidence(_) => "WorkRequest::Evidence",
            Self::Resolve(_) => "WorkRequest::Resolve",
            Self::InspectBatch(_) => "WorkRequest::InspectBatch",
            Self::Apply(_) => "WorkRequest::Apply",
        })
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Items(_) => "WorkResponse::Items",
            Self::Tags(_) => "WorkResponse::Tags",
            Self::Detail(_) => "WorkResponse::Detail",
            Self::Evidence(_) => "WorkResponse::Evidence",
            Self::Resolved(_) => "WorkResponse::Resolved",
            Self::BatchState(_) => "WorkResponse::BatchState",
            Self::Applied(_) => "WorkResponse::Applied",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Disabled,
    Unavailable,
    Storage,
    InvalidInput,
    PermissionDenied,
    Conflict,
    OutcomeUnknown,
    CommittedReceiptUnavailable,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Disabled => "Local work is unavailable because activity recording could not start; check private state-directory access and restart",
            Self::Unavailable => "The local journal is unavailable; retry when recording is ready",
            Self::Storage => "The local work operation could not be completed; check local journal storage before retrying",
            Self::InvalidInput => "The local work request is invalid or unsafe; review the request",
            Self::PermissionDenied => "This local change is blocked by the active mode; keep the intent and change mode before retrying",
            Self::Conflict => "A change identity already belongs to different intent; inspect the original change",
            Self::OutcomeUnknown => "Commit acknowledgement is missing; inspect or retry the same saved change identities",
            Self::CommittedReceiptUnavailable => "Changes committed but the receipt is unavailable; inspect or retry the same saved change identities",
        })
    }
}

impl std::error::Error for Error {}

impl From<crate::work_cli::Error> for Error {
    fn from(error: crate::work_cli::Error) -> Self {
        match error {
            crate::work_cli::Error::InvalidInput => Self::InvalidInput,
            crate::work_cli::Error::PermissionDenied => Self::PermissionDenied,
            crate::work_cli::Error::Unavailable => Self::Unavailable,
            crate::work_cli::Error::Storage | crate::work_cli::Error::OutputUnavailable => {
                Self::Storage
            }
            crate::work_cli::Error::Conflict => Self::Conflict,
            crate::work_cli::Error::OutcomeUnknown => Self::OutcomeUnknown,
            crate::work_cli::Error::CommittedReceiptUnavailable => {
                Self::CommittedReceiptUnavailable
            }
        }
    }
}

impl Intent {
    /// Deliberate local assertions are not claims of independent observation.
    pub fn asserted(change_id: crate::journal::work_map::ChangeId, mutation: Mutation) -> Self {
        Self {
            change_id,
            basis: Basis::UserAsserted,
            evidence: Evidence::Assertion,
            mutation,
        }
    }
}

impl Batch {
    pub fn new(changes: Vec<Intent>) -> Self {
        Self {
            version: crate::journal::work_map::VERSION,
            changes,
        }
    }
}

pub(crate) struct Execution {
    pub result: Result<Response, Error>,
    pub recording_degraded: bool,
}

pub(crate) fn execute(
    journal: &mut Journal,
    session: &SessionId,
    request: Request,
    mode: Mode,
    observed_at_ms: i64,
) -> Execution {
    if let Request::Apply(batch) = request {
        // Reuse the CLI's strict bounded input gate as well as its mutation
        // engine. Typed callers must not bypass the version or batch checks.
        let command = batch_command(&batch);
        let command = match command {
            Ok(command) => command,
            Err(error) => {
                return Execution {
                    result: Err(error),
                    recording_degraded: false,
                };
            }
        };
        let execution = crate::work_cli::execute(journal, session, command, mode, observed_at_ms);
        return Execution {
            result: execution.result.map_err(Error::from).and_then(|reply| {
                serde_json::from_slice(&reply.bytes)
                    .map(Response::Applied)
                    .map_err(|_| Error::CommittedReceiptUnavailable)
            }),
            recording_degraded: execution.recording_degraded,
        };
    }
    Execution {
        result: read(journal, request),
        recording_degraded: false,
    }
}

fn read(journal: &Journal, request: Request) -> Result<Response, Error> {
    match request {
        Request::InspectBatch(batch) => inspect_batch(journal, batch).map(Response::BatchState),
        Request::Items(request) => {
            let page = journal
                .query_items_at(
                    &request.filter,
                    ITEM_LIMIT,
                    request.cursor.as_ref(),
                    request.snapshot.as_ref(),
                )
                .map_err(|_| Error::Storage)?;
            let evidence = if page.items.is_empty() {
                None
            } else {
                Some(handoff(
                    journal,
                    &Selection {
                        items: page.items.iter().map(|item| item.item_id.clone()).collect(),
                        actions: Vec::new(),
                    },
                    None,
                    Some(&page.snapshot),
                )?)
            };
            Ok(Response::Items(ItemsResponse { page, evidence }))
        }
        Request::Tags(request) => journal
            .query_tags_at(
                &request.namespace,
                TAG_LIMIT,
                request.cursor.as_ref(),
                request.snapshot.as_ref(),
            )
            .map(Response::Tags)
            .map_err(|_| Error::Storage),
        Request::Detail(request) => {
            let primary = handoff(
                journal,
                &Selection {
                    items: vec![request.item_id.clone()],
                    actions: Vec::new(),
                },
                None,
                request.snapshot.as_ref(),
            )?;
            let mut neighbours = Vec::new();
            for record in &primary.records {
                let HandoffRecord::Map { change } = record else {
                    continue;
                };
                if change.retracted || change.request.provenance.basis == Basis::Inferred {
                    continue;
                }
                let Mutation::RelateItems { from, to, .. } = &change.request.mutation else {
                    continue;
                };
                let other = if from == &request.item_id {
                    to
                } else if to == &request.item_id {
                    from
                } else {
                    continue;
                };
                if !neighbours.contains(other) {
                    neighbours.push(other.clone());
                }
            }
            let expansion_truncated = primary.truncated || neighbours.len() > NEIGHBOUR_LIMIT;
            neighbours.truncate(NEIGHBOUR_LIMIT);
            let related = if neighbours.is_empty() {
                None
            } else {
                Some(handoff(
                    journal,
                    &Selection {
                        items: neighbours,
                        actions: Vec::new(),
                    },
                    None,
                    Some(&primary.snapshot),
                )?)
            };
            Ok(Response::Detail(DetailResponse {
                selected: request.item_id,
                primary,
                related,
                expansion_truncated,
            }))
        }
        Request::Evidence(request) => handoff(
            journal,
            &request.selection,
            request.cursor.as_ref(),
            request.snapshot.as_ref(),
        )
        .map(Response::Evidence),
        Request::Resolve(request) => {
            let resolution = journal
                .resolve_subject(
                    &request.subject,
                    &request.action_id,
                    request.snapshot.as_ref(),
                )
                .map_err(|_| Error::Storage)?;
            crate::journal::work_map::encode_safe_output(&resolution)
                .map_err(|_| Error::Storage)?;
            let requested = journal
                .query(&Query {
                    action_id: Some(request.action_id),
                    limit: 1,
                    ..Default::default()
                })
                .map_err(|_| Error::Storage)?
                .into_iter()
                .next()
                .filter(|event| {
                    event.outcome == Outcome::Requested
                        && event.sequence <= resolution.snapshot.event_sequence()
                })
                .ok_or(Error::Storage)?;
            Ok(Response::Resolved(ResolveResponse {
                resolution,
                requested_sequence: requested.sequence,
            }))
        }
        Request::Apply(_) => Err(Error::InvalidInput),
    }
}

/// The same bounded typed-input gate used before invoking the CLI engine.
fn batch_command(batch: &Batch) -> Result<crate::work_cli::Command, Error> {
    if batch.changes.is_empty() || batch.changes.len() > crate::journal::work_map::MAX_BATCH {
        return Err(Error::InvalidInput);
    }
    let mut bytes = InputBytes(Vec::new());
    serde_json::to_writer(&mut bytes, batch).map_err(|_| Error::InvalidInput)?;
    crate::work_cli::parse("apply", bytes.0.as_slice()).map_err(Error::from)
}

fn inspect_batch(journal: &Journal, batch: Batch) -> Result<BatchState, Error> {
    let crate::work_cli::Command::Apply(batch) = batch_command(&batch)? else {
        return Err(Error::InvalidInput);
    };
    // Public/deserialized types may bypass their constructors. Reuse the
    // authoritative shape validator without creating an audit action or claim.
    // The change UUID is retyped solely as a validation placeholder: this
    // provenance never enters the journal, returned evidence, or comparisons.
    for intent in &batch.changes {
        crate::journal::work_map::encode_safe_output(intent).map_err(|_| Error::InvalidInput)?;
        let request = ChangeRequest {
            id: intent.change_id.clone(),
            provenance: Provenance {
                recording_action_id: ActionId::parse(intent.change_id.as_str())
                    .map_err(|_| Error::InvalidInput)?,
                evidence_event_id: match &intent.evidence {
                    Evidence::Assertion => EventId::parse(intent.change_id.as_str())
                        .map_err(|_| Error::InvalidInput)?,
                    Evidence::Event { event_id } => event_id.clone(),
                },
                basis: intent.basis,
            },
            observed_at_ms: 0,
            mutation: intent.mutation.clone(),
        };
        crate::journal::work_map::validate_change_request(&request)
            .map_err(|_| Error::InvalidInput)?;
    }
    let snapshot = journal.work_map_snapshot().map_err(|_| Error::Storage)?;
    let mut present = 0;
    for intent in &batch.changes {
        if let Some(record) = journal
            .get_work_change(&intent.change_id, Some(&snapshot))
            .map_err(|_| Error::Storage)?
        {
            if !crate::work_cli::same_intent(intent, &record) {
                return Err(Error::Conflict);
            }
            present += 1;
        }
    }
    Ok(if present == batch.changes.len() {
        BatchState::AllPresent
    } else if present == 0 {
        BatchState::NonePresent
    } else {
        BatchState::Partial
    })
}

fn handoff(
    journal: &Journal,
    selection: &Selection,
    cursor: Option<&HandoffCursor>,
    snapshot: Option<&Snapshot>,
) -> Result<Handoff, Error> {
    journal
        .handoff_at(selection, EVIDENCE_LIMITS, cursor, snapshot)
        .map_err(|_| Error::Storage)
}

/// Typed inputs may have been deserialized without constructors. Do not make an
/// unbounded serialization copy before the CLI's existing strict input gate.
struct InputBytes(Vec<u8>);

impl std::io::Write for InputBytes {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > crate::work_cli::MAX_INPUT_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("work input exceeds its bound"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Scratch-journal integration fixtures for consumers of the semantic bridge.
/// Production callers use plugin effects and the runner-owned queue instead.
#[cfg(feature = "test-support")]
pub mod test_support {
    use super::{Error, Request, Response};
    use crate::journal::{Journal, SessionId};
    use crate::writes::Mode;

    /// Runs the same input validation, audited mutation engine, permission
    /// checks, snapshot reads and receipt decoding as the production writer.
    pub fn execute(
        journal: &mut Journal,
        session: &SessionId,
        request: Request,
        mode: Mode,
        observed_at_ms: i64,
    ) -> Result<Response, Error> {
        super::execute(journal, session, request, mode, observed_at_ms).result
    }
}
