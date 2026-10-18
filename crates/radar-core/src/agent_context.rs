//! What a part tells the host about one of its subjects so an agent can be
//! briefed on it (send to agent), and the two requests the review journey
//! makes of parts (resolve a change request; make a worktree at a sha).
//!
//! radar-core names no part: a part fills these types for its own subjects and
//! never reads another part's data. The brief is built from them in
//! the part that runs agent sessions, as plain information under the task.

use serde::{Deserialize, Serialize};

use crate::journal::SubjectKey;

/// The one action id every part uses for "send to agent", so the same key and
/// menu entry mean the same thing on every row that names a subject.
pub const SEND_TO_AGENT_ACTION: &str = "agent.send";

/// The action id for "review locally" on a message that links a change request.
pub const REVIEW_LOCALLY_ACTION: &str = "agent.review_locally";

/// Most bytes of text one field may carry; longer text is cut by the part
/// (and again by the brief builder) with `truncated` set.
pub const MAX_FIELD_BYTES: usize = 8 * 1024;
/// Most fields one context may carry.
pub const MAX_FIELDS: usize = 32;
/// Most bytes a whole context may carry.
pub const MAX_CONTEXT_BYTES: usize = 32 * 1024;

/// One named, bounded piece of a context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextField {
    /// Short name by which the brief refers to it (`"source_branch"`).
    pub name: String,
    pub value: String,
    /// The part cut `value` to fit the cap.
    #[serde(default)]
    pub truncated: bool,
}

impl ContextField {
    pub fn new(name: &str, value: impl Into<String>) -> Self {
        let mut value: String = value.into();
        let truncated = cap_text(&mut value, MAX_FIELD_BYTES);
        Self { name: name.to_owned(), value, truncated }
    }
}

/// What a part contributes for one subject.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentContext {
    /// The part's name for the kind of thing (the word is the part's,
    /// not radar-core's): shown as the heading of the quoted material.
    pub kind: String,
    /// The scoped identity the context is about.
    pub subject: SubjectKey,
    pub title: String,
    /// Where it lives on the web, if anywhere.
    pub link: Option<String>,
    pub fields: Vec<ContextField>,
}

impl AgentContext {
    pub fn new(kind: &str, subject: SubjectKey, title: &str) -> Self {
        Self { kind: kind.to_owned(), subject, title: title.to_owned(), link: None, fields: vec![] }
    }
    pub fn link(mut self, link: Option<String>) -> Self {
        self.link = link;
        self
    }
    /// Add a field; once the context is over [`MAX_FIELDS`] or
    /// [`MAX_CONTEXT_BYTES`] further fields are dropped, never the earlier ones.
    pub fn field(mut self, field: ContextField) -> Self {
        let used: usize = self.fields.iter().map(|f| f.name.len() + f.value.len()).sum();
        if self.fields.len() < MAX_FIELDS && used + field.name.len() + field.value.len() <= MAX_CONTEXT_BYTES {
            self.fields.push(field);
        }
        self
    }
}

/// Cut `text` to at most `max` bytes on a char boundary; true when it was cut.
pub fn cap_text(text: &mut String, max: usize) -> bool {
    if text.len() <= max {
        return false;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    true
}

/// The host asks the part that owns `subject` for its context. The answer
/// comes from what the part already holds; it never fetches to answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentContextRequest {
    pub subject: SubjectKey,
}

/// Resolve a change request (the review journey, step 2). Asked of the part
/// that owns the key; answered within that service's query budget.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeResolveRequest {
    pub subject: SubjectKey,
}

/// A part's answer to [`ChangeResolveRequest`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
// Ready is the common end state and the value is moved once per resolution.
#[allow(clippy::large_enum_variant)]
pub enum Resolve {
    Ready(ChangeResolution),
    /// The fetch is under way; ask again.
    Pending,
    /// Why it cannot be resolved, in words fit for the dock (gone, signed out).
    Failed(String),
}

/// A part's request (`Effect::StartReview`) that the host start a local
/// review. `origin` is the row it was asked on (a chat message), `changes`
/// the change requests that row links, as scoped keys the owning part resolves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewRequest {
    #[serde(default)]
    pub automatic: bool,
    pub origin: SubjectKey,
    /// Links found in the row's text. The host maps
    /// each to the part that recognises it and fills `changes`; a part that
    /// cannot name another part's subjects sends only these.
    pub links: Vec<String>,
    /// Scoped keys of change requests, filled by the host from `links`
    /// (a part that already knows them may fill them too).
    pub changes: Vec<SubjectKey>,
}

/// What the owner knows of a change request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangeResolution {
    pub subject: SubjectKey,
    pub title: String,
    pub link: Option<String>,
    /// Repository the source branch lives in (a project path, a clone URL).
    pub repository: String,
    pub source_branch: String,
    pub target_branch: String,
    /// The sha the review is of; the worktree is made at exactly this.
    pub head_sha: String,
    pub diff_stats: DiffStats,
    pub description: String,
    pub threads: Vec<ThreadSummary>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStats {
    pub files: u32,
    pub additions: u32,
    pub deletions: u32,
}

/// An open discussion thread on the change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub author: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub body: String,
}

/// Make, or reuse, a worktree of `branch` at `sha` in the workspace's
/// configured worktree place (the review journey, step 3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRequest {
    /// Repository the branch is in, as the change resolution named it.
    pub repository: String,
    pub branch: String,
    pub sha: String,
    /// The subject the worktree is for, for the journal.
    pub for_subject: Option<SubjectKey>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeReady {
    pub path: std::path::PathBuf,
    /// An existing worktree at this sha was reused.
    pub reused: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorktreeError {
    /// No repository here matches the request.
    UnknownRepository,
    /// The sha is not reachable from the branch after fetching.
    ShaNotFound,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> SubjectKey {
        SubjectKey::new("sample", "https://one.example.test", "card", "ABC-1").unwrap()
    }

    #[test]
    fn a_field_is_cut_on_a_char_boundary() {
        let f = ContextField::new("text", "é".repeat(MAX_FIELD_BYTES));
        assert!(f.truncated);
        assert!(f.value.len() <= MAX_FIELD_BYTES);
        assert!(f.value.chars().all(|c| c == 'é'));
    }

    #[test]
    fn a_context_drops_fields_over_its_cap_not_the_first() {
        let mut c = AgentContext::new("card", key(), "t");
        for i in 0..(MAX_FIELDS + 5) {
            c = c.field(ContextField::new(&format!("f{i}"), "x"));
        }
        assert_eq!(c.fields.len(), MAX_FIELDS);
        assert_eq!(c.fields[0].name, "f0");
    }
}
