//! Private, append-only action history. The runner owns a single ordered writer;
//! reducers and parts describe actions, never open a database or write globals.
//!
//! Only this allowlisted vocabulary reaches storage. Never derive records from
//! `Debug` output, typed text, clipboard contents, settings, or task payloads.
//! A completed asynchronous task is not evidence that its action succeeded.

#[cfg(unix)]
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

mod schema;

/// Current on-disk schema. A newer database is refused without migrating it.
pub const SCHEMA_VERSION: u32 = 5;
/// Version of the public event vocabulary, independent of database migrations.
pub const EVENT_VERSION: u32 = 1;
const APPLICATION_ID: i64 = 0x47474a52;
pub(crate) const MAX_SUBJECTS: usize = 32;
const MAX_METADATA_BYTES: usize = 4096;
const MAX_QUERY_LIMIT: usize = 1000;
const MAX_TIMESTAMP_MS: i64 = 253_402_300_799_999;

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn parse(value: &str) -> Result<Self> {
                Ok(Self(Uuid::parse_str(value)?.to_string()))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

identifier!(SessionId);
identifier!(ActionId);
identifier!(EventId);

pub mod work_map;

/// A stable vocabulary identifier, never an action label or user-entered text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StableCode(String);

impl StableCode {
    /// Static callers are cheap and infallible; storage still validates the code.
    pub fn new(value: &'static str) -> Self {
        Self(value.to_owned())
    }

    /// For identifiers supplied by a part's declared vocabulary, not free text.
    pub fn try_new(value: &str) -> Result<Self> {
        validate_code(value)?;
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub type ActionKind = StableCode;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    Host,
    /// A part's declared id; never its configuration or account name.
    Plugin(String),
}

/// Explicit observations, not inferences from task completion or a notice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Requested,
    Confirmed,
    Started,
    Succeeded,
    Failed,
    Cancelled,
    Blocked,
    Partial,
    Unknown,
}

impl Outcome {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Requested | Self::Confirmed | Self::Started)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requested => "requested",
            Self::Confirmed => "confirmed",
            Self::Started => "started",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Blocked => "blocked",
            Self::Partial => "partial",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "requested" => Self::Requested,
            "confirmed" => Self::Confirmed,
            "started" => Self::Started,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "blocked" => Self::Blocked,
            "partial" => Self::Partial,
            "unknown" => Self::Unknown,
            _ => bail!("unknown journal outcome"),
        })
    }
}

/// Safe error classification. Raw errors can contain credentials and payloads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Interrupted,
    NoTerminalObservation,
    Unsupported,
    Unavailable,
    PermissionDenied,
    InvalidInput,
    Transport,
    Storage,
    TaskCompleted,
    Unspecified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteMode {
    ReadOnly,
    Write,
    Edit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin {
    Tui,
    Cli,
    Unknown,
}

impl SessionOrigin {
    fn as_str(self) -> &'static str {
        match self {
            Self::Tui => "tui",
            Self::Cli => "cli",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: SessionId,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub process_id: u32,
    pub build_version: String,
    pub origin: SessionOrigin,
    pub event_version: u32,
    /// Known recording failures; false is not proof that no unobserved loss occurred.
    pub recording_degraded: bool,
    /// Durable gap batches, not an estimate of the number of missing actions.
    pub recording_gap_count: u64,
    pub dropped_records: u64,
    /// Failed writes may be downstream consequences of a missing begin record.
    pub write_errors: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecordingGap {
    pub sequence: i64,
    pub id: EventId,
    pub session_id: SessionId,
    pub observed_at_ms: i64,
    pub recorded_at_ms: i64,
    pub dropped_records: u64,
    pub write_errors: u64,
}

/// A web location with credentials, query parameters, and fragment removed.
/// A caller must still avoid paths that themselves contain a secret.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SafeUrl(String);

impl SafeUrl {
    pub fn parse(value: &str) -> Result<Self> {
        ensure!(value.len() <= 8192, "journal URL exceeds the input limit");
        let mut url = url::Url::parse(value).context("invalid journal URL")?;
        ensure!(
            matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
            "journal URLs must be web locations"
        );
        url.set_username("")
            .map_err(|_| anyhow::anyhow!("invalid journal URL"))?;
        url.set_password(None)
            .map_err(|_| anyhow::anyhow!("invalid journal URL"))?;
        url.set_query(None);
        url.set_fragment(None);
        let value = url.to_string();
        ensure!(value.len() <= 2048, "journal URL exceeds the storage limit");
        ensure!(
            !credential_pattern(&value),
            "journal URL path may contain a credential"
        );
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The complete metadata allowlist. There is deliberately no free-text or JSON
/// field. New facts must be explicitly modeled here and reviewed for privacy.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<WriteMode>,
    /// Some unsafe details were deliberately omitted. No omitted content is kept.
    #[serde(default, skip_serializing_if = "is_false")]
    pub redacted: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub omitted_subjects: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Reason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<SafeUrl>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<StableCode>,
}

impl Metadata {
    fn json(&self) -> Result<String> {
        if let Some(command) = &self.command {
            validate_code(command.as_str())?;
        }
        ensure!(
            self.duration_ms.is_none_or(|value| value <= 31_536_000_000),
            "journal duration exceeds the limit"
        );
        let mut safe = self.clone();
        if let Some(url) = &self.url {
            safe.url = Some(SafeUrl::parse(url.as_str())?);
            safe.redacted |= safe.url.as_ref() != self.url.as_ref();
        }
        let json = serde_json::to_string(&safe)?;
        ensure!(
            json.len() <= MAX_METADATA_BYTES,
            "journal metadata exceeds the limit"
        );
        Ok(json)
    }
}

/// Identity is scoped to a provider instance as well as its vocabulary and id.
/// A second server or account can legitimately contain the same kind and id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectKey {
    pub provider: String,
    pub instance: String,
    pub kind: String,
    pub id: String,
}

impl SubjectKey {
    pub fn new(provider: &str, instance: &str, kind: &str, id: &str) -> Result<Self> {
        let value = Self {
            provider: provider.to_owned(),
            instance: safe_identity(instance)?,
            kind: kind.to_owned(),
            id: safe_identity(id)?,
        };
        value.validate()?;
        Ok(value)
    }

    fn validate(&self) -> Result<()> {
        validate_code(&self.provider)?;
        validate_code(&self.kind)?;
        ensure!(
            safe_identity(&self.instance)? == self.instance,
            "unsafe subject instance"
        );
        ensure!(safe_identity(&self.id)? == self.id, "unsafe subject id");
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubjectRole {
    Primary,
    Source,
    Destination,
    Context,
    Result,
}

impl SubjectRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Source => "source",
            Self::Destination => "destination",
            Self::Context => "context",
            Self::Result => "result",
        }
    }
}

/// Action-time identity and location. Display labels and arbitrary subject
/// fields are intentionally not copied: they can contain authored private text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectSnapshot {
    pub key: SubjectKey,
    pub role: SubjectRole,
    pub url: Option<SafeUrl>,
}

impl SubjectSnapshot {
    pub fn new(key: SubjectKey, role: SubjectRole) -> Self {
        Self {
            key,
            role,
            url: None,
        }
    }

    pub fn with_url(mut self, url: SafeUrl) -> Self {
        self.url = Some(url);
        self
    }

    /// Validate an owner's returned snapshot before it enters a host record.
    /// Deserialization and public identity fields can bypass constructors.
    pub(crate) fn validate(&self) -> Result<()> {
        self.key.validate()?;
        if let Some(url) = &self.url {
            ensure!(SafeUrl::parse(url.as_str())? == *url, "unsafe subject URL");
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ActionDescriptor {
    pub kind: ActionKind,
    pub origin: Origin,
    pub subjects: Vec<SubjectSnapshot>,
    pub metadata: Metadata,
}

impl ActionDescriptor {
    /// Validate a bounded provider fact before retaining it in a CLI envelope.
    /// Public fields and deserialization can bypass the typed constructors.
    pub(crate) fn validate(&self) -> Result<()> {
        validate_code(self.kind.as_str())?;
        if let Origin::Plugin(id) = &self.origin {
            validate_code(id)?;
        }
        self.metadata.json()?;
        if let Some(url) = &self.metadata.url {
            ensure!(SafeUrl::parse(url.as_str())? == *url, "unsafe metadata URL");
        }
        ensure!(self.subjects.len() <= MAX_SUBJECTS, "too many journal subjects");
        for subject in &self.subjects {
            subject.validate()?;
        }
        Ok(())
    }

    pub fn new(kind: &'static str, origin: Origin) -> Self {
        Self {
            kind: StableCode::new(kind),
            origin,
            subjects: Vec::new(),
            metadata: Metadata::default(),
        }
    }

    pub fn with_subject(mut self, subject: SubjectSnapshot) -> Self {
        self.subjects.push(subject);
        self
    }

    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }
}

/// Ordered, immutable facts. `sequence` is global to this database and remains
/// the ordering authority when clocks change or timestamps coincide.
#[derive(Debug, Clone, Serialize)]
pub struct ActionEvent {
    pub event_version: u32,
    pub sequence: i64,
    pub event_id: EventId,
    pub action_id: ActionId,
    pub session_id: SessionId,
    pub parent_action_id: Option<ActionId>,
    pub root_action_id: ActionId,
    pub correlation_id: ActionId,
    /// Compatibility name for the observation timestamp, not the commit time.
    pub occurred_at_ms: i64,
    pub observed_at_ms: i64,
    pub recorded_at_ms: i64,
    /// Older schemas knew only writer time; migration does not invent an exact
    /// observation time for those retained events.
    pub observation_time_estimated: bool,
    pub kind: ActionKind,
    pub origin: Origin,
    pub outcome: Outcome,
    pub metadata: Metadata,
    pub subjects: Vec<SubjectSnapshot>,
}

#[derive(Debug, Clone, Default)]
pub struct Query {
    pub session_id: Option<SessionId>,
    pub action_id: Option<ActionId>,
    pub root_action_id: Option<ActionId>,
    pub kind: Option<ActionKind>,
    pub subject: Option<SubjectKey>,
    pub since_ms: Option<i64>,
    pub until_ms: Option<i64>,
    /// Exclusive pagination cursor, independent of the wall clock.
    pub after_sequence: Option<i64>,
    /// Zero means the default page of 100; at most 1000 events per call.
    pub limit: usize,
}

/// Synchronous and deliberately not shared. Move it to the runner's ordered
/// blocking writer. It does no recovery inference about other running sessions.
pub struct Journal {
    connection: Connection,
    path: PathBuf,
}

impl Journal {
    pub fn default_path() -> Result<PathBuf> {
        Self::path_from(
            std::env::var_os("XDG_STATE_HOME").as_deref(),
            dirs::home_dir().as_deref(),
        )
    }

    fn path_from(state: Option<&std::ffi::OsStr>, home: Option<&Path>) -> Result<PathBuf> {
        if let Some(state) = state.filter(|value| !value.is_empty()) {
            let state = Path::new(state);
            ensure!(
                state.is_absolute(),
                "XDG_STATE_HOME must be an absolute path"
            );
            return Ok(state.join("radar").join("journal").join("actions.sqlite3"));
        }
        let home = home
            .filter(|path| path.is_absolute())
            .context("no private state directory is available")?;
        Ok(home
            .join(".local")
            .join("state")
            .join("radar")
            .join("journal")
            .join("actions.sqlite3"))
    }

    pub fn open(path: &Path) -> Result<Self> {
        ensure!(path.is_absolute(), "journal path must be absolute");
        private_directory(path.parent().context("journal path needs a directory")?)?;
        private_file(path, true)?;
        private_sidecars(path)?;
        let mut connection = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .context("cannot open the action journal")?;
        connection.busy_timeout(Duration::from_secs(2))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "trusted_schema", false)?;
        migrate(&mut connection)?;
        validate_schema(&connection)?;
        let check: String = connection.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
        ensure!(check == "ok", "action journal integrity check failed");
        let mode: String = connection.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        ensure!(
            mode.eq_ignore_ascii_case("wal"),
            "action journal requires WAL mode"
        );
        connection.pragma_update(None, "synchronous", "FULL")?;
        private_sidecars(path)?;
        Ok(Self {
            connection,
            path: path.to_owned(),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn start_session(&mut self) -> Result<SessionId> {
        self.start_session_with(SessionOrigin::Unknown)
    }

    pub fn start_session_with(&mut self, origin: SessionOrigin) -> Result<SessionId> {
        let id = SessionId::new();
        self.connection.execute(
            "INSERT INTO sessions(id,started_at_ms,process_id,build_version,origin,event_version) VALUES (?1,?2,?3,?4,?5,?6)",
            params![id.as_str(), now_ms()?, i64::from(std::process::id()), env!("CARGO_PKG_VERSION"), origin.as_str(), EVENT_VERSION],
        )?;
        Ok(id)
    }

    pub fn session(&self, id: &SessionId) -> Result<Option<SessionInfo>> {
        let mut statement = self.connection.prepare(
            "SELECT started_at_ms,ended_at_ms,process_id,build_version,origin,event_version,
             recording_degraded,recording_gap_count,dropped_records,write_errors FROM sessions WHERE id=?1",
        )?;
        let mut rows = statement.query([id.as_str()])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };
        let origin: String = row.get(4)?;
        Ok(Some(SessionInfo {
            id: id.clone(),
            started_at_ms: row.get(0)?,
            ended_at_ms: row.get(1)?,
            process_id: row.get(2)?,
            build_version: row.get(3)?,
            origin: serde_json::from_value(serde_json::Value::String(origin))?,
            event_version: row.get(5)?,
            recording_degraded: row.get(6)?,
            recording_gap_count: row.get(7)?,
            dropped_records: row.get(8)?,
            write_errors: row.get(9)?,
        }))
    }

    pub fn begin(
        &mut self,
        session: &SessionId,
        descriptor: ActionDescriptor,
        parent: Option<&ActionId>,
    ) -> Result<ActionId> {
        let id = ActionId::new();
        self.begin_with_id(session, &id, descriptor, parent)?;
        Ok(id)
    }

    /// Queues can allocate ids before submitting any disk work. Enqueue this
    /// before children or outcomes using the same ordered writer.
    pub fn begin_with_id(
        &mut self,
        session: &SessionId,
        id: &ActionId,
        descriptor: ActionDescriptor,
        parent: Option<&ActionId>,
    ) -> Result<()> {
        self.begin_with_id_at(session, id, descriptor, parent, now_ms()?)
    }

    /// The runner stamps this before enqueueing. The reducer need not read a
    /// clock, and writer backpressure does not alter the observation timestamp.
    pub fn begin_with_id_at(
        &mut self,
        session: &SessionId,
        id: &ActionId,
        descriptor: ActionDescriptor,
        parent: Option<&ActionId>,
        observed_at_ms: i64,
    ) -> Result<()> {
        validate_timestamp(observed_at_ms)?;
        validate_code(descriptor.kind.as_str())?;
        let origin = match &descriptor.origin {
            Origin::Host => "host",
            Origin::Plugin(value) => {
                validate_code(value)?;
                value.as_str()
            }
        };
        let metadata = descriptor.metadata.json()?;
        ensure!(
            descriptor.subjects.len() <= MAX_SUBJECTS,
            "too many journal subjects"
        );
        for subject in &descriptor.subjects {
            subject.key.validate()?;
        }
        SessionId::parse(session.as_str())?;
        ActionId::parse(id.as_str())?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure_session_open(&tx, session)?;
        let (root, correlation) = if let Some(parent) = parent {
            let found: Option<(String, String, String)> = tx
                .query_row(
                    "SELECT session_id,root_action_id,correlation_id FROM actions WHERE id=?1",
                    [parent.as_str()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let (parent_session, root, correlation) =
                found.context("journal parent action does not exist")?;
            ensure!(
                parent_session == session.as_str(),
                "journal parent belongs to another session"
            );
            (root, correlation)
        } else {
            (id.as_str().to_owned(), id.as_str().to_owned())
        };
        tx.execute(
            "INSERT INTO actions(id,session_id,parent_action_id,root_action_id,correlation_id,kind,origin_kind,origin_id,created_at_ms)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![id.as_str(), session.as_str(), parent.map(ActionId::as_str), root, correlation,
                descriptor.kind.as_str(), if matches!(descriptor.origin, Origin::Host) { "host" } else { "plugin" }, origin, observed_at_ms],
        )?;
        for (ordinal, subject) in descriptor.subjects.iter().enumerate() {
            tx.execute(
                "INSERT INTO subjects(provider,instance,kind,external_id) VALUES (?1,?2,?3,?4) ON CONFLICT DO NOTHING",
                params![subject.key.provider, subject.key.instance, subject.key.kind, subject.key.id],
            )?;
            let subject_id: i64 = tx.query_row(
                "SELECT id FROM subjects WHERE provider=?1 AND instance=?2 AND kind=?3 AND external_id=?4",
                params![subject.key.provider, subject.key.instance, subject.key.kind, subject.key.id], |row| row.get(0),
            )?;
            let safe_url = subject
                .url
                .as_ref()
                .map(|url| SafeUrl::parse(url.as_str()))
                .transpose()?;
            tx.execute(
                "INSERT INTO action_subjects(action_id,ordinal,subject_id,role,url_snapshot) VALUES (?1,?2,?3,?4,?5)",
                params![id.as_str(), ordinal as i64, subject_id, subject.role.as_str(), safe_url.as_ref().map(SafeUrl::as_str)],
            )?;
        }
        append_event(&tx, id, Outcome::Requested, &metadata, observed_at_ms)?;
        tx.commit()?;
        Ok(())
    }

    pub fn record(
        &mut self,
        action: &ActionId,
        outcome: Outcome,
        metadata: Metadata,
    ) -> Result<EventId> {
        self.record_at(action, outcome, metadata, now_ms()?)
    }

    pub fn record_at(
        &mut self,
        action: &ActionId,
        outcome: Outcome,
        metadata: Metadata,
        observed_at_ms: i64,
    ) -> Result<EventId> {
        validate_timestamp(observed_at_ms)?;
        ensure!(
            outcome != Outcome::Requested,
            "an action can only be requested once"
        );
        let json = metadata.json()?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (session, previous, previous_metadata, previous_id): (String, String, String, String) = tx.query_row(
            "SELECT a.session_id,e.outcome,e.metadata_json,e.id FROM actions a JOIN action_events e ON e.action_id=a.id
             WHERE a.id=?1 ORDER BY e.sequence DESC LIMIT 1", [action.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        ).optional()?.context("journal action does not exist")?;
        ensure_session_open(&tx, &SessionId(session))?;
        let previous = Outcome::parse(&previous)?;
        if previous == outcome && previous_metadata == json {
            return Ok(EventId(previous_id));
        }
        ensure!(
            !previous.is_terminal(),
            "journal action already has a terminal outcome"
        );
        ensure!(
            previous != outcome,
            "duplicate journal lifecycle observation"
        );
        ensure!(
            !(previous == Outcome::Started && outcome == Outcome::Confirmed),
            "journal confirmation must precede execution"
        );
        let id = append_event(&tx, action, outcome, &json, observed_at_ms)?;
        tx.commit()?;
        Ok(id)
    }

    /// Only the runtime that owns this session calls this after draining its
    /// queue. Opening a database never declares another session interrupted.
    pub fn finish_session(&mut self, session: &SessionId) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        ensure_session_open(&tx, session)?;
        let pending = {
            let mut statement = tx.prepare(
                "SELECT a.id,a.kind FROM actions a JOIN action_events e ON e.action_id=a.id
                 WHERE a.session_id=?1 AND e.sequence=(SELECT max(last.sequence) FROM action_events last WHERE last.action_id=a.id)
                 AND e.outcome IN ('requested','confirmed','started') ORDER BY e.sequence",
            )?;
            statement
                .query_map([session.as_str()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (action, kind) in pending {
            let metadata = Metadata {
                reason: Some(if kind == "runtime.task" {
                    Reason::Interrupted
                } else {
                    Reason::NoTerminalObservation
                }),
                ..Metadata::default()
            }
            .json()?;
            append_event(
                &tx,
                &ActionId(action),
                Outcome::Unknown,
                &metadata,
                now_ms()?,
            )?;
        }
        tx.execute(
            "UPDATE sessions SET ended_at_ms=?2 WHERE id=?1",
            params![session.as_str(), now_ms()?],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Each write commits synchronously. This additionally checkpoints without
    /// deleting history; a busy reader can defer pages to a later checkpoint.
    pub fn flush(&mut self) -> Result<()> {
        self.connection
            .execute_batch("PRAGMA wal_checkpoint(PASSIVE)")?;
        private_sidecars(&self.path)
    }

    /// Persist only known counts; these are not a claim about how many actions
    /// are missing. The caller retains its counts until this transaction commits.
    /// A known closed session is allowed so a shutdown-flush failure can still
    /// be recorded if the database becomes writable before the writer exits.
    pub fn record_gap(
        &mut self,
        session: &SessionId,
        dropped_records: u64,
        write_errors: u64,
        observed_at_ms: i64,
    ) -> Result<()> {
        validate_timestamp(observed_at_ms)?;
        ensure!(dropped_records > 0 || write_errors > 0, "empty journal gap");
        let dropped =
            i64::try_from(dropped_records).context("journal dropped count exceeds the limit")?;
        let errors =
            i64::try_from(write_errors).context("journal error count exceeds the limit")?;
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (old_dropped, old_errors, batches): (i64, i64, i64) = tx
            .query_row(
                "SELECT dropped_records,write_errors,recording_gap_count FROM sessions WHERE id=?1",
                [session.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?
            .context("journal session does not exist")?;
        let total_dropped = old_dropped
            .checked_add(dropped)
            .context("journal dropped count exceeds the limit")?;
        let total_errors = old_errors
            .checked_add(errors)
            .context("journal error count exceeds the limit")?;
        let batches = batches
            .checked_add(1)
            .context("journal gap count exceeds the limit")?;
        tx.execute(
            "INSERT INTO recording_gaps(id,session_id,observed_at_ms,recorded_at_ms,dropped_records,write_errors)
             VALUES (?1,?2,?3,?4,?5,?6)",
            params![EventId::new().as_str(), session.as_str(), observed_at_ms, now_ms()?, dropped, errors],
        )?;
        tx.execute(
            "UPDATE sessions SET recording_degraded=1,recording_gap_count=?2,dropped_records=?3,write_errors=?4 WHERE id=?1",
            params![session.as_str(), batches, total_dropped, total_errors],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn recording_gaps(
        &self,
        session: &SessionId,
        after_sequence: i64,
        limit: usize,
    ) -> Result<Vec<RecordingGap>> {
        ensure!(
            (1..=MAX_QUERY_LIMIT).contains(&limit),
            "journal gap query limit must be 1..=1000"
        );
        let mut statement = self.connection.prepare(
            "SELECT sequence,id,observed_at_ms,recorded_at_ms,dropped_records,write_errors FROM recording_gaps
             WHERE session_id=?1 AND sequence>?2 ORDER BY sequence LIMIT ?3",
        )?;
        let gaps = statement
            .query_map(
                params![session.as_str(), after_sequence, limit as i64],
                |row| {
                    Ok(RecordingGap {
                        sequence: row.get(0)?,
                        id: EventId(row.get(1)?),
                        session_id: session.clone(),
                        observed_at_ms: row.get(2)?,
                        recorded_at_ms: row.get(3)?,
                        dropped_records: row.get(4)?,
                        write_errors: row.get(5)?,
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(gaps)
    }

    pub fn query(&self, query: &Query) -> Result<Vec<ActionEvent>> {
        ensure!(
            query.limit <= MAX_QUERY_LIMIT,
            "journal query limit exceeds 1000"
        );
        if let Some(subject) = &query.subject {
            subject.validate()?;
        }
        let mut sql = String::from(
            "SELECT e.sequence,e.id,a.id,a.session_id,a.parent_action_id,a.root_action_id,a.correlation_id,
             e.occurred_at_ms,a.kind,a.origin_kind,a.origin_id,e.outcome,e.metadata_json,e.event_version,
             COALESCE(e.recorded_at_ms,e.occurred_at_ms),e.recorded_at_ms IS NULL
             FROM action_events e JOIN actions a ON a.id=e.action_id WHERE 1=1",
        );
        let mut values: Vec<rusqlite::types::Value> = Vec::new();
        for (clause, value) in [
            (
                "a.session_id",
                query.session_id.as_ref().map(SessionId::as_str),
            ),
            ("a.id", query.action_id.as_ref().map(ActionId::as_str)),
            (
                "a.root_action_id",
                query.root_action_id.as_ref().map(ActionId::as_str),
            ),
            ("a.kind", query.kind.as_ref().map(StableCode::as_str)),
        ] {
            if let Some(value) = value {
                sql.push_str(&format!(" AND {clause}=?"));
                values.push(value.to_owned().into());
            }
        }
        for (clause, value) in [
            ("e.occurred_at_ms>=?", query.since_ms),
            ("e.occurred_at_ms<=?", query.until_ms),
            ("e.sequence>?", query.after_sequence),
        ] {
            if let Some(value) = value {
                sql.push_str(&format!(" AND {clause}"));
                values.push(value.into());
            }
        }
        if let Some(subject) = &query.subject {
            sql.push_str(" AND EXISTS (SELECT 1 FROM action_subjects ax JOIN subjects s ON s.id=ax.subject_id
                WHERE ax.action_id=a.id AND s.provider=? AND s.instance=? AND s.kind=? AND s.external_id=?)");
            for value in [
                &subject.provider,
                &subject.instance,
                &subject.kind,
                &subject.id,
            ] {
                values.push(value.clone().into());
            }
        }
        sql.push_str(" ORDER BY e.sequence LIMIT ?");
        values.push((if query.limit == 0 { 100 } else { query.limit } as i64).into());
        let mut statement = self.connection.prepare(&sql)?;
        let mut rows = statement.query(rusqlite::params_from_iter(values))?;
        let mut events = Vec::new();
        while let Some(row) = rows.next()? {
            let origin_kind: String = row.get(9)?;
            let origin_id: String = row.get(10)?;
            let action_id = ActionId(row.get(2)?);
            events.push(ActionEvent {
                event_version: row.get(13)?,
                sequence: row.get(0)?,
                event_id: EventId(row.get(1)?),
                subjects: self.subjects(&action_id)?,
                action_id,
                session_id: SessionId(row.get(3)?),
                parent_action_id: row.get::<_, Option<String>>(4)?.map(ActionId),
                root_action_id: ActionId(row.get(5)?),
                correlation_id: ActionId(row.get(6)?),
                occurred_at_ms: row.get(7)?,
                observed_at_ms: row.get(7)?,
                recorded_at_ms: row.get(14)?,
                observation_time_estimated: row.get(15)?,
                kind: StableCode(row.get(8)?),
                origin: if origin_kind == "host" {
                    Origin::Host
                } else {
                    Origin::Plugin(origin_id)
                },
                outcome: Outcome::parse(&row.get::<_, String>(11)?)?,
                metadata: serde_json::from_str(&row.get::<_, String>(12)?)?,
            });
        }
        Ok(events)
    }

    fn subjects(&self, action: &ActionId) -> Result<Vec<SubjectSnapshot>> {
        let mut statement = self.connection.prepare(
            "SELECT s.provider,s.instance,s.kind,s.external_id,ax.role,ax.url_snapshot
             FROM action_subjects ax JOIN subjects s ON s.id=ax.subject_id
             WHERE ax.action_id=?1 ORDER BY ax.ordinal",
        )?;
        let mut rows = statement.query([action.as_str()])?;
        let mut subjects = Vec::new();
        while let Some(row) = rows.next()? {
            let role: String = row.get(4)?;
            subjects.push(SubjectSnapshot {
                key: SubjectKey {
                    provider: row.get(0)?,
                    instance: row.get(1)?,
                    kind: row.get(2)?,
                    id: row.get(3)?,
                },
                role: serde_json::from_value(serde_json::Value::String(role))?,
                url: row.get::<_, Option<String>>(5)?.map(SafeUrl),
            });
        }
        Ok(subjects)
    }
}

fn validate_code(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 96
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)),
        "journal vocabulary must be a bounded stable code"
    );
    Ok(())
}

fn safe_identity(value: &str) -> Result<String> {
    ensure!(
        !value.is_empty() && value.len() <= 2048,
        "journal identity exceeds the limit"
    );
    if value.starts_with("https://") || value.starts_with("http://") {
        return Ok(SafeUrl::parse(value)?.0);
    }
    ensure!(
        !credential_pattern(value),
        "journal identity may contain a credential"
    );
    // Local paths are useful identities, including spaces and Unicode. They
    // remain explicit subject inputs, never copied from arbitrary payloads.
    let local_path = Path::new(value).is_absolute()
        || (value.as_bytes().get(1) == Some(&b':')
            && matches!(value.as_bytes().get(2), Some(b'\\' | b'/')));
    ensure!(
        !value.chars().any(char::is_control)
            && !value.contains(['?', '#'])
            && (local_path
                || value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-/:!".contains(&byte))),
        "journal identity must be a stable id, not free text"
    );
    Ok(value.to_owned())
}

fn is_false(value: &bool) -> bool {
    !value
}
fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn credential_pattern(value: &str) -> bool {
    // Decode a bounded number of percent-encoding layers before checking a
    // location. This is conservative: an ambiguous identity is better omitted.
    let mut decoded = value.to_ascii_lowercase();
    for _ in 0..8 {
        let bytes = decoded.as_bytes();
        let mut next = Vec::with_capacity(bytes.len());
        let mut cursor = 0;
        while cursor < bytes.len() {
            if bytes[cursor] == b'%'
                && cursor + 2 < bytes.len()
                && let (Some(high), Some(low)) = (
                    (bytes[cursor + 1] as char).to_digit(16),
                    (bytes[cursor + 2] as char).to_digit(16),
                )
            {
                next.push((high * 16 + low) as u8);
                cursor += 3;
            } else {
                next.push(bytes[cursor]);
                cursor += 1;
            }
        }
        let next = String::from_utf8_lossy(&next).to_ascii_lowercase();
        if next == decoded {
            break;
        }
        decoded = next;
    }
    decoded.chars().any(char::is_control)
        || decoded.contains("%25")
        || [
            "token=",
            "password=",
            "passwd=",
            "secret=",
            "api_key=",
            "apikey=",
            "authorization=",
            "bearer ",
        ]
        .iter()
        .any(|pattern| decoded.contains(pattern))
}

fn now_ms() -> Result<i64> {
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    i64::try_from(millis).context("journal timestamp exceeds the storage range")
}

fn validate_timestamp(value: i64) -> Result<()> {
    ensure!(
        (0..=MAX_TIMESTAMP_MS).contains(&value),
        "journal observation timestamp exceeds the supported range"
    );
    Ok(())
}

fn ensure_session_open(tx: &Transaction<'_>, session: &SessionId) -> Result<()> {
    let open: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM sessions WHERE id=?1 AND ended_at_ms IS NULL)",
        [session.as_str()],
        |row| row.get(0),
    )?;
    ensure!(open, "journal session does not exist or has ended");
    Ok(())
}

fn append_event(
    tx: &Transaction<'_>,
    action: &ActionId,
    outcome: Outcome,
    metadata: &str,
    observed_at_ms: i64,
) -> Result<EventId> {
    let id = EventId::new();
    tx.execute(
        "INSERT INTO action_events(id,action_id,occurred_at_ms,outcome,metadata_json,event_version,recorded_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7)",
        params![id.as_str(), action.as_str(), observed_at_ms, outcome.as_str(), metadata, EVENT_VERSION, now_ms()?],
    )?;
    Ok(id)
}

fn migrate(connection: &mut Connection) -> Result<()> {
    // Read the version only after taking the migration lock. Two processes can
    // start together, and the second must see the first one's committed schema.
    let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let version: u32 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    ensure!(
        version <= SCHEMA_VERSION,
        "action journal schema is newer than this application"
    );
    let application: i64 = tx.pragma_query_value(None, "application_id", |row| row.get(0))?;
    ensure!(
        application == APPLICATION_ID || (application == 0 && version == 0),
        "file is not an action journal"
    );
    if version == 0 {
        let tables: i64 = tx.query_row(
            "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
            [],
            |row| row.get(0),
        )?;
        ensure!(tables == 0, "refusing to initialize an unrelated database");
    }
    if version < 1 {
        tx.execute_batch(schema::V1)?;
    }
    if version < 2 {
        tx.execute_batch(schema::V2)?;
    }
    if version < 3 {
        tx.execute_batch(schema::V3)?;
    }
    if version < 4 {
        tx.execute_batch(schema::V4)?;
    }
    if version < 5 {
        tx.execute_batch(schema::V5)?;
        tx.execute(
            "INSERT INTO work_map_store(singleton,id) VALUES (1,?1)",
            [Uuid::new_v4().to_string()],
        )?;
    }
    tx.pragma_update(None, "application_id", APPLICATION_ID)?;
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

fn validate_schema(connection: &Connection) -> Result<()> {
    work_map::validate_schema(connection)?;
    // A valid SQLite header alone is insufficient: missing tables, broken
    // foreign keys, or removed immutability guards must not be silently ignored.
    for sql in [
        "SELECT id,started_at_ms,ended_at_ms,process_id,build_version,origin,event_version,recording_degraded,recording_gap_count,dropped_records,write_errors FROM sessions LIMIT 0",
        "SELECT id,session_id,parent_action_id,root_action_id,correlation_id,kind,origin_kind,origin_id,created_at_ms FROM actions LIMIT 0",
        "SELECT sequence,id,action_id,occurred_at_ms,outcome,metadata_json,event_version,recorded_at_ms FROM action_events LIMIT 0",
        "SELECT id,provider,instance,kind,external_id FROM subjects LIMIT 0",
        "SELECT action_id,ordinal,subject_id,role,url_snapshot FROM action_subjects LIMIT 0",
        "SELECT sequence,id,session_id,observed_at_ms,recorded_at_ms,dropped_records,write_errors FROM recording_gaps LIMIT 0",
    ] {
        connection
            .prepare(sql)
            .context("incomplete action journal schema")?;
    }
    for trigger in [
        "actions_no_update",
        "actions_no_delete",
        "events_no_update",
        "events_no_delete",
        "subjects_no_update",
        "subjects_no_delete",
        "action_subjects_no_update",
        "action_subjects_no_delete",
        "recording_gaps_no_update",
        "recording_gaps_no_delete",
    ] {
        let exists: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger' AND name=?1)",
            [trigger],
            |row| row.get(0),
        )?;
        ensure!(exists, "action journal immutability guard is missing");
    }
    ensure!(
        connection
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()?
            .is_none(),
        "action journal foreign-key check failed"
    );
    Ok(())
}

#[cfg(unix)]
fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    ensure!(
        path.file_name().is_some(),
        "journal requires a dedicated directory"
    );
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let metadata = fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "journal directory must not be a symbolic link"
    );
    ensure!(
        metadata.permissions().mode() & 0o1000 == 0,
        "journal requires a dedicated private directory"
    );
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn private_directory(_: &Path) -> Result<()> {
    bail!("private action journal permissions are not supported on this platform yet")
}

#[cfg(unix)]
pub(crate) fn private_file(path: &Path, create: bool) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    if create {
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink() && metadata.nlink() == 1,
                "journal file must be a regular private file"
            );
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Err(error) if !create && error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

#[cfg(not(unix))]
pub(crate) fn private_file(_: &Path, _: bool) -> Result<()> {
    bail!("private action journal permissions are not supported on this platform yet")
}

pub(crate) fn private_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm", "-journal"] {
        let mut name = path.as_os_str().to_owned();
        name.push(suffix);
        private_file(Path::new(&name), false)?;
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests;
