//! Explicit local annotations and bounded, audited evidence queries.
//! Input is semantic data, never executable instructions or service payloads.

use std::fmt;
use std::io::Read;

use serde::{Deserialize, Serialize};

use crate::journal::work_map::{
    Basis, ChangeId, EventCursor, EventFilter, HandoffCursor, ItemCursor, ItemFilter, Mutation,
    Selection, TagKey,
};
use crate::journal::{ActionId, EventId, SubjectKey};
use crate::writes::Mode;

mod runtime;
pub use runtime::{ApplyReceipt, Attempt, ChangeReceipt};
pub(crate) use runtime::{execute, same_intent};

const VERSION: u32 = 1;
pub(crate) const MAX_INPUT_BYTES: usize = 16 * 8192 + 4096;
const MAX_QUERY_BYTES: usize = 16_384;
// A continuation repeats the full scoped filter in its database-bound cursor.
// Two maximum-size, JSON-escaped identities must still be resumable.
const MAX_EVENTS_QUERY_BYTES: usize = 32_768;
const MAX_OUTPUT_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Error {
    InvalidInput,
    PermissionDenied,
    Unavailable,
    Storage,
    Conflict,
    OutcomeUnknown,
    CommittedReceiptUnavailable,
    OutputUnavailable,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidInput => "work: invalid or unsafe input",
            Self::PermissionDenied => "work: this local change is not allowed by the active mode",
            Self::Unavailable => "work: the journal writer is unavailable",
            Self::Storage => "work: the journal operation could not be completed",
            Self::Conflict => "work: a change identity was reused with different intent",
            Self::OutcomeUnknown => "work: commit acknowledgement unavailable; inspect or retry the same change identities",
            Self::CommittedReceiptUnavailable => "work: changes committed; receipt unavailable; inspect the original change identities",
            Self::OutputUnavailable => "work: query output could not be written",
        })
    }
}
impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Evidence {
    /// Evidence that this claim was submitted, not that its content was observed.
    Assertion,
    Event {
        event_id: EventId,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Intent {
    pub change_id: ChangeId,
    pub basis: Basis,
    pub evidence: Evidence,
    pub mutation: Mutation,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub version: u32,
    pub changes: Vec<Intent>,
}

impl fmt::Debug for Intent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Intent")
            .field("change_id", &self.change_id)
            .field("basis", &self.basis)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for Batch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Batch")
            .field("version", &self.version)
            .field("changes", &self.changes.len())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct HandoffQuery {
    pub version: u32,
    pub selection: Selection,
    pub records: usize,
    pub bytes: usize,
    pub cursor: Option<HandoffCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ItemsQuery {
    pub version: u32,
    pub filter: ItemFilter,
    pub limit: usize,
    pub cursor: Option<ItemCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EventsQuery {
    pub version: u32,
    pub filter: EventFilter,
    pub records: usize,
    pub bytes: usize,
    pub cursor: Option<EventCursor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TagQuery {
    pub version: u32,
    pub namespace: TagKey,
    pub key: TagKey,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ChangeQuery {
    pub version: u32,
    pub change_id: ChangeId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ResolveQuery {
    pub version: u32,
    pub subject: SubjectKey,
    pub action_id: ActionId,
}

#[derive(Debug, Clone)]
pub(crate) enum Command {
    Apply(Batch),
    Handoff(HandoffQuery),
    Items(ItemsQuery),
    Events(Box<EventsQuery>),
    Tag(TagQuery),
    Change(ChangeQuery),
    Resolve(ResolveQuery),
}

impl Command {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Apply(_) => "work.apply",
            Self::Handoff(_) => "work.handoff",
            Self::Items(_) => "work.items",
            Self::Events(_) => "work.events",
            Self::Tag(_) => "work.tag",
            Self::Change(_) => "work.change",
            Self::Resolve(_) => "work.resolve",
        }
    }

    fn version(&self) -> u32 {
        match self {
            Self::Apply(v) => v.version,
            Self::Handoff(v) => v.version,
            Self::Items(v) => v.version,
            Self::Events(v) => v.version,
            Self::Tag(v) => v.version,
            Self::Change(v) => v.version,
            Self::Resolve(v) => v.version,
        }
    }
}

pub(crate) struct Reply {
    pub bytes: Vec<u8>,
}

pub(crate) struct Execution {
    pub result: Result<Reply, Error>,
    pub recording_degraded: bool,
}

pub(crate) fn command() -> clap::Command {
    let mut command = clap::Command::new("work")
        .about("Explicit local work annotations and audited evidence queries")
        .long_about("Explicit local work annotations and bounded evidence queries. Data commands record their own invocation and may create, migrate, or checkpoint the local journal and write diagnostic logs. Queries do not change map facts. Output is observations only, never permission to act. Input is one bounded JSON object on stdin; use explicit nulls for optional fields.")
        .subcommand_required(true)
        .subcommand(clap::Command::new("id").about("Print a new UUID without opening local state"));
    for (name, about) in [
        (
            "apply",
            "Atomically apply 1..16 explicitly identified local annotation intents",
        ),
        (
            "handoff",
            "Return one bounded observation-only evidence page",
        ),
        (
            "items",
            "List explicit identities, optionally filtered by tags",
        ),
        (
            "events",
            "Discover bounded original journal evidence by exact filters",
        ),
        ("tag", "Find a tag by its stable namespace and key"),
        ("change", "Find an original change receipt by identity"),
        (
            "resolve",
            "Resolve a scoped subject at an action's original request",
        ),
    ] {
        command = command.subcommand(
            clap::Command::new(name).about(about).arg(
                clap::Arg::new("stdin")
                    .long("stdin")
                    .required(true)
                    .action(clap::ArgAction::SetTrue)
                    .help("Read one bounded, versioned JSON object from stdin"),
            ),
        );
    }
    command
}

fn bounded_read(mut input: impl Read, limit: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::InvalidInput)?;
    if bytes.len() > limit {
        return Err(Error::InvalidInput);
    }
    Ok(bytes)
}

/// Round-trip equality also refuses ignored nested fields in shared types.
/// Values, unknown field names, and parser diagnostics must never be printed.
fn decode<T: for<'de> Deserialize<'de> + Serialize>(bytes: &[u8]) -> Result<T, Error> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|_| Error::InvalidInput)?;
    let parsed: T = serde_json::from_slice(bytes).map_err(|_| Error::InvalidInput)?;
    let canonical = serde_json::to_value(&parsed).map_err(|_| Error::InvalidInput)?;
    if value != canonical {
        return Err(Error::InvalidInput);
    }
    Ok(parsed)
}

pub(crate) fn parse(name: &str, input: impl Read) -> Result<Command, Error> {
    let bytes = bounded_read(
        input,
        match name {
            "apply" => MAX_INPUT_BYTES,
            "events" => MAX_EVENTS_QUERY_BYTES,
            _ => MAX_QUERY_BYTES,
        },
    )?;
    let command = match name {
        "apply" => Command::Apply(decode(&bytes)?),
        "handoff" => Command::Handoff(decode(&bytes)?),
        "items" => Command::Items(decode(&bytes)?),
        "events" => Command::Events(Box::new(decode(&bytes)?)),
        "tag" => Command::Tag(decode(&bytes)?),
        "change" => Command::Change(decode(&bytes)?),
        "resolve" => Command::Resolve(decode(&bytes)?),
        _ => return Err(Error::InvalidInput),
    };
    if command.version() != VERSION {
        return Err(Error::InvalidInput);
    }
    if let Command::Apply(batch) = &command {
        if batch.changes.is_empty() || batch.changes.len() > crate::journal::work_map::MAX_BATCH {
            return Err(Error::InvalidInput);
        }
        let mut ids = std::collections::HashSet::new();
        for intent in &batch.changes {
            if !ids.insert(intent.change_id.as_str())
                || (intent.basis != Basis::UserAsserted && intent.evidence == Evidence::Assertion)
                || serde_json::to_vec(intent)
                    .map_err(|_| Error::InvalidInput)?
                    .len()
                    > 8192
            {
                return Err(Error::InvalidInput);
            }
        }
    }
    Ok(command)
}

/// Read only core mode configuration. Never configure parts or resolve credentials.
pub(crate) fn mode() -> Result<Mode, Error> {
    let path = crate::runner::config_path();
    let configured = match std::fs::File::open(path) {
        Ok(file) => {
            let bytes = bounded_read(file, MAX_OUTPUT_BYTES)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| Error::InvalidInput)?;
            let table: toml::Table = toml::from_str(text).map_err(|_| Error::InvalidInput)?;
            match table.get("general").and_then(|v| v.get("mode")) {
                None => Mode::default(),
                Some(v) => v
                    .as_str()
                    .and_then(Mode::parse)
                    .ok_or(Error::InvalidInput)?,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Mode::default(),
        Err(_) => return Err(Error::Unavailable),
    };
    Ok(environment_mode(std::env::vars_os())?.unwrap_or(configured))
}

fn environment_mode(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Result<Option<Mode>, Error> {
    let mut relevant = Vec::new();
    for (key, value) in vars {
        let Ok(key) = key.into_string() else { continue };
        if key == crate::writes::MODE_VAR || (key.starts_with("RADAR_") && key.ends_with("NO_WRITES"))
        {
            let value = value.into_string().map_err(|_| Error::InvalidInput)?;
            if key == crate::writes::MODE_VAR && Mode::parse(&value).is_none() {
                return Err(Error::InvalidInput);
            }
            relevant.push((key, value));
        }
    }
    Ok(crate::writes::env_mode(relevant).map(|(mode, _)| mode))
}

pub(crate) fn write_response(
    mut output: impl std::io::Write,
    bytes: &[u8],
    applied: bool,
) -> Result<(), Error> {
    output
        .write_all(bytes)
        .and_then(|()| output.flush())
        .map_err(|_| {
            if applied {
                Error::CommittedReceiptUnavailable
            } else {
                Error::OutputUnavailable
            }
        })
}

#[cfg(all(test, unix))]
mod tests;
