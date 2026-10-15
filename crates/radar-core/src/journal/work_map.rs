//! Explicit, evidence-linked annotations over the private action journal.
//!
//! A subject is a scoped locator, not necessarily an enduring object. Nothing
//! here automatically registers subjects, guesses aliases, or authorizes work.
//! Labels are deliberately supplied, bounded, untrusted semantic annotations;
//! this does not relax the journal's ban on copying arbitrary authored content.

use super::*;

mod events;
pub use events::{EventCursor, EventEntry, EventFilter, EventPage};
#[cfg(all(test, unix))]
mod events_tests;
mod query;
#[cfg(all(test, unix))]
mod query_cli_tests;
mod store;
#[cfg(all(test, unix))]
mod tests;

pub const VERSION: u32 = 1;
pub const MAX_BATCH: usize = 16;
pub const MAX_SELECTION: usize = 32;
pub const MAX_TAG_FILTERS: usize = 8;
/// Catalog pages and each definition's label evidence have independent bounds.
pub const MAX_TAG_QUERY_LIMIT: usize = 32;
pub const TAG_LABEL_LIMIT: usize = 8;
pub const MAX_PAGE_BYTES: usize = 1_048_576;
pub(crate) const MAX_REQUEST_BYTES: usize = 8192;

identifier!(ItemId);
identifier!(TagId);
identifier!(ChangeId);

/// Attribution of a claim, never proof of permission or procedure approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Basis {
    Observed,
    UserAsserted,
    Inferred,
}
impl Basis {
    fn as_str(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::UserAsserted => "user_asserted",
            Self::Inferred => "inferred",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provenance {
    /// The actual operation recording this annotation. Its origin/session are
    /// loaded from the journal, never accepted from a caller's display name.
    pub recording_action_id: ActionId,
    /// Evidence can come from a different action or an earlier session.
    pub evidence_event_id: EventId,
    pub basis: Basis,
}

/// Lowercase, case-sensitive machine vocabulary, independent of the label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TagKey(String);
impl TagKey {
    pub fn new(value: &str) -> Result<Self> {
        ensure!(
            !value.is_empty()
                && value.len() <= 96
                && value
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b)),
            "tag keys must be bounded lowercase machine codes"
        );
        ensure!(
            !credential_pattern(value) && !crate::log::contains_secret(value),
            "work map annotation may contain a credential"
        );
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SemanticLabel(String);
impl SemanticLabel {
    pub fn new(value: &str) -> Result<Self> {
        ensure!(
            !value.is_empty()
                && value.len() <= 128
                && value.trim() == value
                && !value.chars().any(char::is_control)
                && !credential_pattern(value),
            "tag label must be a bounded deliberate annotation without controls or credentials"
        );
        ensure!(
            !credential_pattern(value) && !crate::log::contains_secret(value),
            "work map annotation may contain a credential"
        );
        Ok(Self(value.into()))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum TagTarget {
    Item(ItemId),
    Action(ActionId),
}

/// Explicit claims only. In particular, shared tags and action Result roles
/// never manufacture a relation, and these kinds never create causal actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    References,
    PartOf,
    DependsOn,
}
impl RelationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::References => "references",
            Self::PartOf => "part_of",
            Self::DependsOn => "depends_on",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AssertionTarget {
    Binding {
        item_id: ItemId,
        subject: SubjectKey,
    },
    LabelAssertion {
        tag_id: TagId,
    },
    TagAssignment {
        tag_id: TagId,
        target: TagTarget,
    },
    Relation {
        from: ItemId,
        to: ItemId,
        kind: RelationKind,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Mutation {
    CreateItem {
        item_id: ItemId,
        kind: StableCode,
    },
    /// The range is an explicit assertion about action-start sequences, not
    /// inferred from time. Unbounded ranges must be deliberately requested.
    BindSubject {
        item_id: ItemId,
        subject: SubjectKey,
        from_sequence: i64,
        through_sequence: Option<i64>,
    },
    CreateTag {
        tag_id: TagId,
        namespace: TagKey,
        key: TagKey,
        label: SemanticLabel,
    },
    RenameTag {
        tag_id: TagId,
        label: SemanticLabel,
    },
    AssignTag {
        tag_id: TagId,
        target: TagTarget,
    },
    RelateItems {
        from: ItemId,
        to: ItemId,
        kind: RelationKind,
    },
    /// A correction names both a claim and its expected typed target.
    /// Identity creation and retractions themselves cannot be retracted.
    Retract {
        claim_id: ChangeId,
        expected: AssertionTarget,
    },
}
impl Mutation {
    fn kind(&self) -> &'static str {
        match self {
            Self::CreateItem { .. } => "create_item",
            Self::BindSubject { .. } => "bind_subject",
            Self::CreateTag { .. } => "create_tag",
            Self::RenameTag { .. } => "rename_tag",
            Self::AssignTag { .. } => "assign_tag",
            Self::RelateItems { .. } => "relate_items",
            Self::Retract { .. } => "retract",
        }
    }
    fn target(&self) -> Option<AssertionTarget> {
        Some(match self {
            Self::BindSubject {
                item_id, subject, ..
            } => AssertionTarget::Binding {
                item_id: item_id.clone(),
                subject: subject.clone(),
            },
            Self::RenameTag { tag_id, .. } => AssertionTarget::LabelAssertion {
                tag_id: tag_id.clone(),
            },
            Self::AssignTag { tag_id, target } => AssertionTarget::TagAssignment {
                tag_id: tag_id.clone(),
                target: target.clone(),
            },
            Self::RelateItems { from, to, kind } => AssertionTarget::Relation {
                from: from.clone(),
                to: to.clone(),
                kind: *kind,
            },
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChangeRequest {
    /// Allocate once and keep it for retries. Different-payload reuse fails.
    pub id: ChangeId,
    pub provenance: Provenance,
    pub observed_at_ms: i64,
    pub mutation: Mutation,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionSource {
    pub action_id: ActionId,
    pub session_id: SessionId,
    pub origin: Origin,
    pub kind: ActionKind,
    pub build_version: String,
    pub session_origin: SessionOrigin,
}
#[derive(Debug, Clone, Serialize)]
pub struct EvidenceSource {
    pub event_version: u32,
    pub metadata: Metadata,
    pub event_id: EventId,
    pub sequence: i64,
    pub outcome: Outcome,
    pub observed_at_ms: i64,
    pub recorded_at_ms: i64,
    pub observation_time_estimated: bool,
    pub action: ActionSource,
}
#[derive(Debug, Clone, Serialize)]
pub struct ChangeRecord {
    pub version: u32,
    pub sequence: i64,
    pub request: ChangeRequest,
    pub recorded_at_ms: i64,
    pub recording: ActionSource,
    pub evidence: EvidenceSource,
    /// Status at the supplied snapshot. Older claims remain returned as history.
    pub retracted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    database_id: String,
    event_sequence: i64,
    map_sequence: i64,
    gap_sequence: i64,
}
impl Snapshot {
    pub fn database_id(&self) -> &str {
        &self.database_id
    }
    pub fn event_sequence(&self) -> i64 {
        self.event_sequence
    }
    pub fn map_sequence(&self) -> i64 {
        self.map_sequence
    }
    pub fn gap_sequence(&self) -> i64 {
        self.gap_sequence
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub records: usize,
    pub bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            records: 100,
            bytes: 262_144,
        }
    }
}
impl Limits {
    fn validate(self) -> Result<()> {
        ensure!(
            (1..=MAX_QUERY_LIMIT).contains(&self.records),
            "work map record limit must be 1..=1000"
        );
        ensure!(
            (1024..=MAX_PAGE_BYTES).contains(&self.bytes),
            "work map byte limit must be 1024..=1048576"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub items: Vec<ItemId>,
    pub actions: Vec<ActionId>,
}
impl Selection {
    fn canonical(&self) -> Result<Self> {
        ensure!(
            !self.items.is_empty() || !self.actions.is_empty(),
            "work map selection is empty"
        );
        ensure!(
            self.items.len() + self.actions.len() <= MAX_SELECTION,
            "work map selection exceeds 32 identities"
        );
        for id in &self.items {
            validate_uuid(id.as_str())?;
        }
        for id in &self.actions {
            validate_uuid(id.as_str())?;
        }
        let mut value = self.clone();
        value.items.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        value.items.dedup();
        value.actions.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        value.actions.dedup();
        for id in &value.items {
            validate_uuid(id.as_str())?;
        }
        for id in &value.actions {
            validate_uuid(id.as_str())?;
        }
        Ok(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffCursor {
    snapshot: Snapshot,
    selection: Selection,
    stage: u8,
    after_sequence: i64,
}
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAssociation {
    DirectSelection,
    ExplicitBinding,
    AmbiguousBinding,
}
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HandoffRecord {
    Map {
        change: Box<ChangeRecord>,
    },
    Action {
        event: Box<ActionEvent>,
        source: ActionSource,
        association: HistoryAssociation,
    },
    Gap {
        gap: RecordingGap,
    },
}
#[derive(Debug, Clone, Serialize)]
pub struct Handoff {
    pub version: u32,
    pub snapshot: Snapshot,
    pub selection: Selection,
    pub records: Vec<HandoffRecord>,
    pub truncated: bool,
    pub next: Option<HandoffCursor>,
    /// Always observations_only. This is evidence, not an approved procedure.
    pub authority: &'static str,
    /// Always unspecified. A caller must supply goal, constraints and authority.
    pub task_instruction: &'static str,
    pub execution_authorization: &'static str,
    pub known_recording_gaps: bool,
    /// True even without a known gap: absent data is never proof of completeness.
    pub completeness_not_guaranteed: bool,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionState {
    Unbound,
    Resolved,
    Ambiguous,
}
#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub snapshot: Snapshot,
    pub state: ResolutionState,
    pub items: Vec<ItemId>,
    pub claims: Vec<ChangeRecord>,
    pub truncated: bool,
}

/// Stable tag discovery. `creation` is original evidence, not a declaration
/// that its initial display label is still current or authoritative.
#[derive(Debug, Clone, Serialize)]
pub struct TagIdentity {
    pub snapshot: Snapshot,
    pub tag_id: TagId,
    pub namespace: TagKey,
    pub key: TagKey,
    pub creation: ChangeRecord,
}

/// One authored label claim, with stable evidence locators. This compact
/// catalog view never selects a canonical label or grants execution authority.
#[derive(Debug, Clone, Serialize)]
pub struct LabelClaim {
    pub change_id: ChangeId,
    pub sequence: i64,
    pub label: SemanticLabel,
    pub provenance: Provenance,
}

/// An authored definition exists independently of its current assignments.
#[derive(Debug, Clone, Serialize)]
pub struct TagEntry {
    pub tag_id: TagId,
    pub key: TagKey,
    pub created_sequence: i64,
    /// Historical initial label, separate from active rename claims.
    pub creation: LabelClaim,
    /// At most TAG_LABEL_LIMIT active, user-authored rename claims at the cut.
    /// The bounded sample is not necessarily the first or newest claims.
    pub renames: Vec<LabelClaim>,
    /// Further active claims exist. Agreement in this sample is not consensus.
    pub labels_truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TagCursor {
    snapshot: Snapshot,
    namespace: TagKey,
    after_key: TagKey,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagPage {
    pub snapshot: Snapshot,
    pub namespace: TagKey,
    /// Stable-key order within this exact namespace, never label/name order.
    pub tags: Vec<TagEntry>,
    /// Further authored definitions exist, independently of label truncation.
    pub truncated: bool,
    pub next: Option<TagCursor>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemFilter {
    pub all_tags: Vec<TagId>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemCursor {
    snapshot: Snapshot,
    filter: ItemFilter,
    after_sequence: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct ItemEntry {
    pub item_id: ItemId,
    pub kind: StableCode,
    pub created_sequence: i64,
}
#[derive(Debug, Clone, Serialize)]
pub struct ItemPage {
    pub snapshot: Snapshot,
    pub items: Vec<ItemEntry>,
    pub truncated: bool,
    pub next: Option<ItemCursor>,
}

/// Validate a typed request's shape without reading or changing journal state.
pub(crate) fn validate_change_request(request: &ChangeRequest) -> Result<()> {
    store::validate_request(request).map(|_| ())
}

/// Scan actual string values, not escaped JSON bytes. Each caller has already
/// bounded its record shape and field sizes; no history is materialized here.
pub(crate) fn encode_safe_output(value: &impl Serialize) -> Result<Vec<u8>> {
    fn check(value: &serde_json::Value) -> Result<()> {
        match value {
            serde_json::Value::String(text) => ensure!(
                !crate::log::contains_secret(text),
                "work map output may contain a registered credential"
            ),
            serde_json::Value::Array(values) => {
                for value in values {
                    check(value)?;
                }
            }
            serde_json::Value::Object(values) => {
                for (key, value) in values {
                    ensure!(
                        !crate::log::contains_secret(key),
                        "work map output may contain a registered credential"
                    );
                    check(value)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let value = serde_json::to_value(value)?;
    check(&value)?;
    Ok(serde_json::to_vec(&value)?)
}

fn validate_uuid(value: &str) -> Result<()> {
    ensure!(
        Uuid::parse_str(value)?.to_string() == value,
        "work map UUID must use canonical lowercase form"
    );
    Ok(())
}

pub(super) fn validate_schema(connection: &Connection) -> Result<()> {
    store::validate_schema(connection)
}
