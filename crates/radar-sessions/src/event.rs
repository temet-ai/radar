//! The event envelope every adapter normalises its provider's stream into.
//!
//! Provider ids and event ids stay opaque: they are compared and stored,
//! never parsed, and never reused as radar's own ids.

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        pub struct $name(pub String);
        impl $name {
            pub fn new(id: impl Into<String>) -> Self {
                Self(id.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

id!(
    /// radar's own id for a session; chosen by radar, handed to the provider when
    /// it lets us choose one.
    RadarSessionId
);
id!(
    /// The provider's id for a session. Opaque.
    ProviderSessionId
);
id!(
    /// The provider's id for one event. Opaque; dedupe key.
    ProviderEventId
);
id!(
    /// Which CLI: `copilot`, `codex`, `claude`, or a configured PTY command.
    ProviderId
);
id!(
    /// The provider's id for a permission request. Opaque; answers are
    /// idempotent by it.
    ProviderRequestId
);

/// Whether the provider keeps the event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Durability {
    /// Part of the provider's history: replayed on resume.
    Persisted,
    /// Live only (deltas, progress, usage): shown while it happens, never
    /// replayed, and marked unsaved until a persisted event replaces it.
    Ephemeral,
}

/// A sub-agent that produced an event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRef {
    pub id: String,
    pub name: Option<String>,
}

/// What a permission request asks to do. Always the exact thing, never a
/// paraphrase: the operator approves what they can read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target", rename_all = "snake_case")]
pub enum PermissionTarget {
    /// A shell command, as the provider will run it.
    Command { command: String, segments: Vec<String> },
    /// A file to read.
    Path { path: String },
    /// A file to write, with the diff when the provider gives one.
    Write { path: String, diff: Option<String> },
    Url { url: String },
    Mcp { server: String, tool: String, arguments: Option<String> },
    Other { description: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionKind {
    Shell,
    Read,
    Write,
    Url,
    Mcp,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionRequest {
    /// The provider's id for this request. May be delivered twice
    /// (reconnects); answers are idempotent by it.
    pub request_id: String,
    pub kind: PermissionKind,
    pub target: PermissionTarget,
    /// The tool call it belongs to, when the provider says.
    pub tool_call_id: Option<String>,
    /// Whether the dock may offer "approve for session" for this request.
    /// The adapter says: Copilot's `canOfferSessionApproval`, Codex's
    /// `acceptForSession` among the decisions it takes, always for Claude
    /// Code. Where the provider cannot remember an approval itself, radar does
    /// (the same kind and target in the same radar session).
    pub can_offer_session_approval: bool,
}

impl PermissionRequest {
    /// What an approval for the session covers: the kind and the exact
    /// thing asked for (the command, the path, the URL, the MCP tool;
    /// never a write's diff or a tool's arguments).
    pub fn session_key(&self) -> String {
        let target = match &self.target {
            PermissionTarget::Command { command, .. } => command.clone(),
            PermissionTarget::Path { path } | PermissionTarget::Write { path, .. } => path.clone(),
            PermissionTarget::Url { url } => url.clone(),
            PermissionTarget::Mcp { server, tool, .. } => format!("{server}/{tool}"),
            PermissionTarget::Other { description } => description.clone(),
        };
        format!("{:?}\u{0}{target}", self.kind)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum PermissionDecision {
    /// Allow this one request.
    ApproveOnce,
    /// Allow this request, and the same again for the rest of the session:
    /// the provider's own session approval where it has one (Copilot's
    /// approve-for-session, Codex's `acceptForSession`, Claude Code's
    /// suggested rules with destination `session`), else this request once
    /// and radar answers the same request again itself.
    ApproveForSession,
    /// Refuse; the reason goes back to the model.
    Deny { reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionAnswer {
    pub request_id: String,
    pub decision: PermissionDecision,
}

/// One choice a question offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionChoice {
    /// What is shown, and what goes back to the agent when it is picked.
    pub label: String,
    /// What the choice means, when the provider says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One question of a request. Claude Code and Codex may ask several at once;
/// Copilot asks one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// The provider's key for this question within its request (Codex's
    /// question id, Claude Code's question text, `0` for Copilot). Answers
    /// carry it back.
    pub id: String,
    /// A short label for the question (Claude Code and Codex give one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header: Option<String>,
    /// The question, exactly as the agent asked it.
    pub text: String,
    /// The choices offered; empty when only typed text answers it.
    pub choices: Vec<QuestionChoice>,
    /// The operator may answer in their own words.
    pub free_text: bool,
    /// Several choices may be picked together.
    pub multiple: bool,
    /// The answer is a secret (Codex): radar never shows it back.
    #[serde(default)]
    pub secret: bool,
}

/// An agent asks the operator something and waits for the answer. Nothing
/// answers it but the operator.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionRequest {
    /// The provider's id for the request. May be delivered twice; answers
    /// are idempotent by it.
    pub request_id: String,
    pub questions: Vec<Question>,
    /// The tool call it belongs to, when the provider says.
    pub tool_call_id: Option<String>,
}

/// The operator's answer to one question of a request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionReply {
    /// [`Question::id`].
    pub question_id: String,
    /// The labels picked, or the one text typed.
    pub values: Vec<String>,
    /// The values are the operator's own words, not choices.
    #[serde(default)]
    pub own_words: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum QuestionDecision {
    /// One reply per question of the request.
    Answer { replies: Vec<QuestionReply> },
    /// The operator will not answer; the reason goes back to the model.
    Decline { reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionAnswer {
    pub request_id: String,
    pub decision: QuestionDecision,
}

/// What happened. Typed; provider detail that radar does not draw goes in
/// [`Event::payload`], bounded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EventKind {
    UserMessage { text: String },
    TurnStarted,
    TurnEnded,
    /// A piece of an assistant message still being written (ephemeral).
    AssistantDelta { message_id: String, text: String },
    /// The finished assistant message (persisted); replaces its deltas.
    AssistantMessage { message_id: String, text: String },
    ReasoningDelta { message_id: String, text: String },
    Reasoning { message_id: String, text: String },
    /// A tool call began. `diff` is what an edit or a write will change, as
    /// diff lines (`+`, `-`, ` `), when the provider's input says.
    ToolStarted {
        call_id: String,
        name: String,
        summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
    },
    ToolCompleted { call_id: String, ok: bool, summary: String },
    PermissionRequested { request: PermissionRequest },
    PermissionResolved { request_id: String, approved: bool },
    /// The agent asks the operator a question and waits.
    QuestionAsked { request: QuestionRequest },
    /// The question is settled: `replies` as the provider reports them
    /// (empty when it does not say what was answered), or `None` when it
    /// was declined or cancelled.
    QuestionResolved { request_id: String, replies: Option<Vec<QuestionReply>> },
    Aborted,
    /// The session is idle again.
    Idle,
    Error { message: String, retryable: bool },
    Notice { text: String },
    Usage { input_tokens: Option<u64>, output_tokens: Option<u64> },
    ModelChanged { model: String },
    /// Anything else the provider said; `name` is its own event type.
    Other { name: String },
}

/// Lines of context kept around an edit's change in [`edit_diff`].
const EDIT_CONTEXT: usize = 2;

/// What an edit or write tool's input changes, as diff lines for
/// [`EventKind::ToolStarted`]'s `diff`: Claude Code's `Edit`
/// (`old_string`, `new_string`), `MultiEdit` (`edits`) and `Write`
/// (`content`); Copilot's `str_replace` (`old_str`, `new_str`) and `create`
/// (`file_text`). The lines both sides share at the start and the end are
/// context (`EDIT_CONTEXT` of them kept); `None` for any other input.
pub fn edit_diff(input: &serde_json::Value) -> Option<String> {
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(serde_json::Value::as_str).map(str::to_string);
    let pairs: Vec<(String, String)> = if let Some(edits) = input.get("edits").and_then(serde_json::Value::as_array) {
        edits.iter().filter_map(|e| Some((s(e, "old_string")?, s(e, "new_string")?))).collect()
    } else if let (Some(old), Some(new)) = (s(input, "old_string").or_else(|| s(input, "old_str")), s(input, "new_string").or_else(|| s(input, "new_str"))) {
        vec![(old, new)]
    } else {
        vec![(String::new(), s(input, "content").or_else(|| s(input, "file_text"))?)]
    };
    let mut out: Vec<String> = Vec::new();
    for (old, new) in pairs {
        let a: Vec<&str> = if old.is_empty() { vec![] } else { old.lines().collect() };
        let b: Vec<&str> = if new.is_empty() { vec![] } else { new.lines().collect() };
        let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
        let tail = a[head..].iter().rev().zip(b[head..].iter().rev()).take_while(|(x, y)| x == y).count();
        out.extend(a[head.saturating_sub(EDIT_CONTEXT)..head].iter().map(|l| format!(" {l}")));
        out.extend(a[head..a.len() - tail].iter().map(|l| format!("-{l}")));
        out.extend(b[head..b.len() - tail].iter().map(|l| format!("+{l}")));
        out.extend(a[a.len() - tail..][..tail.min(EDIT_CONTEXT)].iter().map(|l| format!(" {l}")));
    }
    Some(out.join("\n"))
}

/// One thing that happened in one session, in radar's words.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub radar_session_id: RadarSessionId,
    pub provider: ProviderId,
    pub provider_session_id: ProviderSessionId,
    /// The dedupe key. `None` for a provider that gives none; the store then
    /// cannot dedupe the event and says so.
    pub provider_event_id: Option<ProviderEventId>,
    /// Order the adapter observed it in, per session, from 1.
    pub sequence: u64,
    /// Unix seconds, from the injected clock.
    pub observed_at: i64,
    pub kind: EventKind,
    pub durability: Durability,
    pub agent: Option<AgentRef>,
    /// The provider event this one belongs to (a tool result's start).
    pub parent: Option<ProviderEventId>,
    /// Bounded provider detail; never credentials.
    pub payload: serde_json::Value,
}

#[cfg(test)]
mod edit_diff_tests {
    use super::edit_diff;
    use serde_json::json;

    #[test]
    fn an_edit_is_its_changed_lines_with_two_of_context_each_side() {
        let old = "a\nb\nc\nd\ne\nf";
        let new = "a\nb\nc\nX\ne\nf";
        assert_eq!(edit_diff(&json!({ "file_path": "x", "old_string": old, "new_string": new })).unwrap(), " b\n c\n-d\n+X\n e\n f");
        // Copilot's names for the same
        assert_eq!(edit_diff(&json!({ "path": "x", "old_str": "a", "new_str": "b" })).unwrap(), "-a\n+b");
    }

    #[test]
    fn a_write_is_all_added_and_several_edits_follow_each_other() {
        assert_eq!(edit_diff(&json!({ "file_path": "x", "content": "one\ntwo" })).unwrap(), "+one\n+two");
        let edits = json!({ "edits": [{ "old_string": "a", "new_string": "b" }, { "old_string": "c", "new_string": "" }] });
        assert_eq!(edit_diff(&edits).unwrap(), "-a\n+b\n-c");
        assert_eq!(edit_diff(&json!({ "command": "ls" })), None);
    }
}
