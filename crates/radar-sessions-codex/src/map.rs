//! Codex app-server messages → radar's event kinds. Pure: no I/O, no clock, no
//! session state beyond what the caller hands in.
//!
//! The app-server gives notifications no event id, so each persisted one
//! gets a stable id built from the ids it does carry (`<item>:completed`,
//! `<turn>:ended`). A history replay builds the same ids, which is how the
//! store drops what it already has.

use radar_sessions::{
    Durability, EventKind, PermissionKind, PermissionRequest, PermissionTarget, Question,
    QuestionChoice, QuestionReply, QuestionRequest,
};
use serde_json::{Value, json};

/// Longest tool output or diff kept in an event payload.
pub const PAYLOAD_LIMIT: usize = 4096;

/// One event before the adapter stamps session, sequence and time on it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mapped {
    pub kind: EventKind,
    pub durability: Durability,
    pub event_id: Option<String>,
    pub parent: Option<String>,
    pub payload: Value,
}

impl Mapped {
    pub fn persisted(id: String, kind: EventKind) -> Self {
        Self {
            kind,
            durability: Durability::Persisted,
            event_id: Some(id),
            parent: None,
            payload: Value::Null,
        }
    }
    pub fn ephemeral(id: Option<String>, kind: EventKind) -> Self {
        Self {
            kind,
            durability: Durability::Ephemeral,
            event_id: id,
            parent: None,
            payload: Value::Null,
        }
    }
    fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }
    fn with_parent(mut self, parent: String) -> Self {
        self.parent = Some(parent);
        self
    }
}

pub fn str_at<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str)
}

/// Cut `s` to at most `limit` bytes on a character boundary.
pub fn bounded(s: &str, limit: usize) -> String {
    if s.len() <= limit {
        return s.to_string();
    }
    let mut end = limit;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// The text of a `userMessage` item's inputs.
fn user_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|p| match str_at(p, "type") {
                    Some("text") => str_at(p, "text").map(str::to_string),
                    // A skill picked from the slash menu: `/name`, as it was typed.
                    Some("skill") => str_at(p, "name").map(|n| format!("/{n}")),
                    Some(other) => Some(format!("[{other}]")),
                    None => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn join_strings(v: Option<&Value>) -> String {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

/// Item types that are not tool calls.
fn is_tool(kind: &str) -> bool {
    !matches!(
        kind,
        "userMessage"
            | "agentMessage"
            | "reasoning"
            | "plan"
            | "hookPrompt"
            | "functionCallOutput"
            | "enteredReviewMode"
            | "exitedReviewMode"
    )
}

fn tool_name(item: &Value) -> String {
    match str_at(item, "type").unwrap_or("tool") {
        "commandExecution" => "shell".into(),
        "fileChange" => "apply_patch".into(),
        "mcpToolCall" => match (str_at(item, "server"), str_at(item, "tool")) {
            (Some(s), Some(t)) => format!("{s}/{t}"),
            _ => "mcp".into(),
        },
        other => other.into(),
    }
}

fn changes_of(item: &Value) -> Vec<&Value> {
    item.get("changes")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

fn tool_summary(item: &Value) -> String {
    match str_at(item, "type").unwrap_or("") {
        "commandExecution" => str_at(item, "command").unwrap_or("").to_string(),
        "fileChange" => changes_of(item)
            .iter()
            .filter_map(|c| str_at(c, "path"))
            .collect::<Vec<_>>()
            .join(", "),
        _ => ["tool", "query", "title", "path", "url"]
            .iter()
            .find_map(|k| str_at(item, k))
            .unwrap_or("")
            .to_string(),
    }
}

/// Whether a finished tool item succeeded, and a short result line.
fn tool_result(item: &Value) -> (bool, String) {
    let status = str_at(item, "status").unwrap_or("completed");
    match str_at(item, "type").unwrap_or("") {
        "commandExecution" => {
            let code = item.get("exitCode").and_then(Value::as_i64);
            match (status, code) {
                ("declined", _) => (false, "declined".into()),
                ("failed", Some(c)) => (false, format!("exit {c}")),
                ("failed", None) => (false, "failed".into()),
                (_, Some(c)) => (c == 0, format!("exit {c}")),
                _ => (status == "completed", status.into()),
            }
        }
        _ => (status == "completed", status.to_string()),
    }
}

fn tool_started(item: &Value) -> Option<Mapped> {
    let id = str_at(item, "id")?;
    Some(
        Mapped::persisted(
            format!("{id}:started"),
            EventKind::ToolStarted {
                call_id: id.into(),
                name: tool_name(item),
                summary: tool_summary(item),
                // A file change carries its diff per file.
                diff: Some(changes_of(item).iter().filter_map(|c| str_at(c, "diff")).collect::<Vec<_>>().join("\n")).filter(|d| !d.is_empty()),
            },
        )
        .with_payload(started_payload(item)),
    )
}

fn started_payload(item: &Value) -> Value {
    match str_at(item, "type").unwrap_or("") {
        "commandExecution" => json!({ "cwd": item.get("cwd") }),
        "fileChange" => {
            let changes: Vec<Value> = changes_of(item)
                .iter()
                .map(|c| {
                    json!({
                        "path": c.get("path"),
                        "kind": c.get("kind"),
                        "diff": str_at(c, "diff").map(|d| bounded(d, PAYLOAD_LIMIT)),
                    })
                })
                .collect();
            json!({ "changes": changes })
        }
        _ => Value::Null,
    }
}

fn tool_completed(item: &Value) -> Option<Mapped> {
    let id = str_at(item, "id")?;
    let (ok, summary) = tool_result(item);
    let mut payload = json!({});
    if let Some(out) = str_at(item, "aggregatedOutput") {
        payload["output"] = json!(bounded(out, PAYLOAD_LIMIT));
    }
    Some(
        Mapped::persisted(
            format!("{id}:completed"),
            EventKind::ToolCompleted {
                call_id: id.into(),
                ok,
                summary,
            },
        )
        .with_parent(format!("{id}:started"))
        .with_payload(payload),
    )
}

/// A finished item as persisted events. Used by live `item/completed` and by
/// the history replay, so both build the same ids.
fn completed_item(item: &Value) -> Vec<Mapped> {
    let Some(id) = str_at(item, "id") else {
        return vec![];
    };
    let done = format!("{id}:completed");
    match str_at(item, "type").unwrap_or("") {
        "userMessage" => vec![Mapped::persisted(
            done,
            EventKind::UserMessage {
                text: user_text(item),
            },
        )],
        "agentMessage" => {
            let text = str_at(item, "text").unwrap_or("").to_string();
            let phase = item.get("phase").cloned().unwrap_or(Value::Null);
            vec![
                Mapped::persisted(
                    done,
                    EventKind::AssistantMessage {
                        message_id: id.into(),
                        text,
                    },
                )
                .with_payload(json!({ "phase": phase })),
            ]
        }
        "reasoning" => {
            let text = [
                join_strings(item.get("summary")),
                join_strings(item.get("content")),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
            if text.is_empty() {
                vec![]
            } else {
                vec![Mapped::persisted(
                    done,
                    EventKind::Reasoning {
                        message_id: id.into(),
                        text,
                    },
                )]
            }
        }
        "plan" => vec![
            Mapped::persisted(
                done,
                EventKind::Other {
                    name: "plan".into(),
                },
            )
            .with_payload(
                json!({ "text": str_at(item, "text").map(|t| bounded(t, PAYLOAD_LIMIT)) }),
            ),
        ],
        "hookPrompt" | "functionCallOutput" => vec![],
        // `/review`: the start says what is reviewed; the end repeats the review that the
        // agent message already carries.
        "enteredReviewMode" => vec![Mapped::persisted(
            done,
            EventKind::Notice {
                text: format!("Reviewing {}", str_at(item, "review").unwrap_or("the changes")),
            },
        )],
        "exitedReviewMode" => vec![],
        _ => {
            // A tool call seen only finished (a replay): both ends, so the
            // dock draws the same row whichever way it was learned.
            tool_started(item)
                .into_iter()
                .chain(tool_completed(item))
                .collect()
        }
    }
}

fn turn_events(turn: &Value) -> (Vec<Mapped>, Vec<Mapped>) {
    let id = str_at(turn, "id").unwrap_or("");
    let start = vec![Mapped::persisted(
        format!("{id}:started"),
        EventKind::TurnStarted,
    )];
    let mut end = Vec::new();
    match str_at(turn, "status") {
        Some("interrupted") => end.push(Mapped::persisted(
            format!("{id}:aborted"),
            EventKind::Aborted,
        )),
        Some("failed") => {
            let message = turn
                .get("error")
                .and_then(|e| str_at(e, "message"))
                .unwrap_or("the turn failed")
                .to_string();
            end.push(Mapped::persisted(
                format!("{id}:error"),
                EventKind::Error {
                    message,
                    retryable: false,
                },
            ));
        }
        _ => {}
    }
    end.push(Mapped::persisted(
        format!("{id}:ended"),
        EventKind::TurnEnded,
    ));
    (start, end)
}

/// A server notification as events. Noise (MCP start-up, rate limits, hooks,
/// account refreshes) maps to nothing.
pub fn notification(method: &str, params: &Value) -> Vec<Mapped> {
    match method {
        "turn/started" => params
            .get("turn")
            .map(|t| turn_events(t).0)
            .unwrap_or_default(),
        "turn/completed" => {
            let mut v = params
                .get("turn")
                .map(|t| turn_events(t).1)
                .unwrap_or_default();
            v.push(Mapped::ephemeral(None, EventKind::Idle));
            v
        }
        "item/started" => {
            let Some(item) = params.get("item") else {
                return vec![];
            };
            if is_tool(str_at(item, "type").unwrap_or("")) {
                tool_started(item).into_iter().collect()
            } else {
                vec![]
            }
        }
        "item/completed" => params.get("item").map(completed_item).unwrap_or_default(),
        "item/agentMessage/delta" => match (str_at(params, "itemId"), str_at(params, "delta")) {
            (Some(id), Some(d)) => {
                vec![Mapped::ephemeral(
                    None,
                    EventKind::AssistantDelta {
                        message_id: id.into(),
                        text: d.into(),
                    },
                )]
            }
            _ => vec![],
        },
        "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
            match (str_at(params, "itemId"), str_at(params, "delta")) {
                (Some(id), Some(d)) => {
                    vec![Mapped::ephemeral(
                        None,
                        EventKind::ReasoningDelta {
                            message_id: id.into(),
                            text: d.into(),
                        },
                    )]
                }
                _ => vec![],
            }
        }
        "thread/tokenUsage/updated" => {
            let total = params.get("tokenUsage").and_then(|u| u.get("total"));
            let n = |k: &str| total.and_then(|t| t.get(k)).and_then(Value::as_u64);
            vec![Mapped::ephemeral(
                None,
                EventKind::Usage {
                    input_tokens: n("inputTokens"),
                    output_tokens: n("outputTokens"),
                },
            )]
        }
        "error" => {
            let message = params
                .get("error")
                .and_then(|e| str_at(e, "message"))
                .unwrap_or("Codex reported an error")
                .to_string();
            let retryable = params
                .get("willRetry")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            vec![Mapped::ephemeral(
                None,
                EventKind::Error { message, retryable },
            )]
        }
        "warning" | "guardianWarning" | "configWarning" => str_at(params, "message")
            .or_else(|| str_at(params, "summary"))
            .map(|m| {
                vec![Mapped::ephemeral(
                    None,
                    EventKind::Notice {
                        text: bounded(m, 500),
                    },
                )]
            })
            .unwrap_or_default(),
        _ => vec![],
    }
}

/// A resumed thread's history as persisted events, oldest first.
pub fn history(thread: &Value) -> Vec<Mapped> {
    let mut out = Vec::new();
    for turn in thread
        .get("turns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (start, end) = turn_events(turn);
        out.extend(start);
        for item in turn
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            out.extend(completed_item(item));
        }
        // A turn still running has no end yet.
        if str_at(turn, "status") != Some("inProgress") {
            out.extend(end);
        }
    }
    out
}

/// The id of the turn still running in a resumed thread, if any.
pub fn running_turn(thread: &Value) -> Option<String> {
    thread
        .get("turns")?
        .as_array()?
        .iter()
        .rev()
        .find(|t| str_at(t, "status") == Some("inProgress"))
        .and_then(|t| str_at(t, "id").map(str::to_string))
}

/// What kind of server request this is, in radar's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestKind {
    Command,
    FileChange,
    Permissions,
}

/// A permission request from a server request, or `None` when the method is
/// not an approval. `changes` are the file changes the item announced when it
/// started (the approval request itself carries no diff).
pub fn permission_request(
    method: &str,
    params: &Value,
    changes: Option<&Value>,
) -> Option<(RequestKind, PermissionRequest, Vec<String>)> {
    let item = str_at(params, "itemId").unwrap_or("").to_string();
    let available: Vec<String> = params
        .get("availableDecisions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|d| match d {
                    Value::String(s) => s.clone(),
                    Value::Object(o) => o.keys().next().cloned().unwrap_or_default(),
                    _ => String::new(),
                })
                .collect()
        })
        .unwrap_or_default();
    // `acceptForSession` among the decisions offered; a request that lists none takes every
    // decision of the schema, which has it for commands and file changes.
    let can_session = available.is_empty() || available.iter().any(|d| d == "acceptForSession");
    match method {
        "item/commandExecution/requestApproval" => {
            let command = str_at(params, "command").unwrap_or("").to_string();
            let mut segments: Vec<String> = params
                .get("commandActions")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| str_at(c, "command"))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            if segments.is_empty() {
                segments.push(command.clone());
            }
            let request_id = str_at(params, "approvalId")
                .map(str::to_string)
                .unwrap_or_else(|| item.clone());
            Some((
                RequestKind::Command,
                PermissionRequest {
                    request_id,
                    kind: PermissionKind::Shell,
                    target: PermissionTarget::Command { command, segments },
                    tool_call_id: Some(item),
                    can_offer_session_approval: can_session,
                },
                available,
            ))
        }
        "item/fileChange/requestApproval" => {
            let list: Vec<&Value> = changes
                .and_then(Value::as_array)
                .map(|a| a.iter().collect())
                .unwrap_or_default();
            let mut path = list
                .iter()
                .filter_map(|c| str_at(c, "path"))
                .collect::<Vec<_>>()
                .join(", ");
            if path.is_empty() {
                path = str_at(params, "grantRoot").unwrap_or("").to_string();
            }
            let diff = {
                let d = list
                    .iter()
                    .filter_map(|c| str_at(c, "diff"))
                    .collect::<Vec<_>>()
                    .join("\n");
                (!d.is_empty()).then_some(d)
            };
            Some((
                RequestKind::FileChange,
                PermissionRequest {
                    request_id: item.clone(),
                    kind: PermissionKind::Write,
                    target: PermissionTarget::Write { path, diff },
                    tool_call_id: Some(item),
                    can_offer_session_approval: can_session,
                },
                available,
            ))
        }
        "item/permissions/requestApproval" => {
            let reason = str_at(params, "reason").unwrap_or("Codex asks for more permissions");
            let wanted = params
                .get("permissions")
                .map(|p| p.to_string())
                .unwrap_or_default();
            Some((
                RequestKind::Permissions,
                PermissionRequest {
                    request_id: item.clone(),
                    kind: PermissionKind::Other,
                    target: PermissionTarget::Other {
                        description: format!("{reason}: {}", bounded(&wanted, 500)),
                    },
                    tool_call_id: Some(item),
                    // A grant for the session (`scope: session`).
                    can_offer_session_approval: true,
                },
                available,
            ))
        }
        _ => None,
    }
}

/// The server request that asks the operator questions.
pub const ASK_METHOD: &str = "item/tool/requestUserInput";

/// The questions of an `item/tool/requestUserInput` request (EXPERIMENTAL in the
/// schema: `questions[]` of `{id, header, question, options[{label, description}]?,
/// isOther, isSecret}`). The request is keyed by its item id. A question with no
/// options takes typed words only; `isOther` adds typed words to the options.
pub fn question_request(params: &Value) -> Option<QuestionRequest> {
    let item = str_at(params, "itemId")?.to_string();
    let questions = params
        .get("questions")
        .and_then(Value::as_array)?
        .iter()
        .map(|q| {
            let choices: Vec<QuestionChoice> = q
                .get("options")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|o| {
                            Some(QuestionChoice {
                                label: str_at(o, "label")?.to_string(),
                                description: str_at(o, "description")
                                    .filter(|d| !d.is_empty())
                                    .map(str::to_string),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let flag = |k: &str| q.get(k).and_then(Value::as_bool).unwrap_or(false);
            Question {
                id: str_at(q, "id").unwrap_or("").to_string(),
                header: str_at(q, "header")
                    .filter(|h| !h.is_empty())
                    .map(str::to_string),
                text: str_at(q, "question").unwrap_or("").to_string(),
                free_text: choices.is_empty() || flag("isOther"),
                choices,
                multiple: false,
                secret: flag("isSecret"),
            }
        })
        .collect();
    Some(QuestionRequest {
        request_id: item.clone(),
        questions,
        tool_call_id: Some(item),
    })
}

/// The JSON-RPC result that answers a question request: each question's answers by
/// its id. A declined request answers nothing (an empty map).
pub fn question_result(replies: Option<&[QuestionReply]>) -> Value {
    let mut answers = serde_json::Map::new();
    for r in replies.unwrap_or_default() {
        answers.insert(r.question_id.clone(), json!({ "answers": r.values }));
    }
    json!({ "answers": answers })
}

/// How the operator answered a permission request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Answer {
    Accept,
    /// Accept, and Codex's own session approval: `acceptForSession` (the same
    /// command, or the same files, run without asking), a grant `scope: session`.
    AcceptForSession,
    Refuse,
}

impl Answer {
    pub fn approved(self) -> bool {
        self != Answer::Refuse
    }
}

/// The JSON-RPC result that answers a permission request. Never a policy
/// amendment: radar writes nothing into Codex's execpolicy.
pub fn decision_result(
    kind: RequestKind,
    answer: Answer,
    available: &[String],
    requested: &Value,
) -> Value {
    // An empty list: the request did not say, and the schema's decisions all apply.
    let offers = |d: &str| available.is_empty() || available.iter().any(|a| a == d);
    match kind {
        RequestKind::Permissions => match answer {
            Answer::Accept => json!({ "permissions": requested, "scope": "turn" }),
            Answer::AcceptForSession => json!({ "permissions": requested, "scope": "session" }),
            Answer::Refuse => json!({ "permissions": {}, "scope": "turn" }),
        },
        _ => {
            let decision = if answer == Answer::AcceptForSession && offers("acceptForSession") {
                "acceptForSession"
            } else if answer.approved() {
                "accept"
            } else if offers("decline") {
                "decline"
            } else {
                // The server did not offer decline; cancel is the refusal it has.
                "cancel"
            };
            json!({ "decision": decision })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(m: &[Mapped]) -> Vec<&EventKind> {
        m.iter().map(|m| &m.kind).collect()
    }

    #[test]
    fn a_finished_command_is_started_and_completed_with_stable_ids() {
        let item = json!({"type":"commandExecution","id":"exec-1","command":"ls","status":"completed","exitCode":0,"aggregatedOutput":"a\n"});
        let live = notification("item/completed", &json!({ "item": item }));
        let ids: Vec<_> = live.iter().map(|m| m.event_id.clone().unwrap()).collect();
        assert_eq!(ids, ["exec-1:started", "exec-1:completed"]);
        assert!(
            matches!(&live[1].kind, EventKind::ToolCompleted { ok: true, summary, .. } if summary == "exit 0")
        );
        // The same item in a history replay maps to the same ids.
        let replay = history(&json!({"turns":[{"id":"t","status":"completed","items":[item]}]}));
        let replay_ids: Vec<_> = replay.iter().filter_map(|m| m.event_id.clone()).collect();
        assert_eq!(
            replay_ids,
            ["t:started", "exec-1:started", "exec-1:completed", "t:ended"]
        );
    }

    #[test]
    fn a_review_says_what_it_reviews_and_a_skill_turn_reads_as_the_command_typed() {
        // Shapes recorded from Codex 0.158.0 running review/start and a skill turn.
        let entered = notification("item/completed", &json!({"item":{"type":"enteredReviewMode","id":"r1","review":"current changes"}}));
        assert_eq!(kinds(&entered), [&EventKind::Notice { text: "Reviewing current changes".into() }]);
        assert!(notification("item/started", &json!({"item":{"type":"enteredReviewMode","id":"r1","review":"current changes"}})).is_empty());
        assert!(notification("item/completed", &json!({"item":{"type":"exitedReviewMode","id":"r2","review":"findings"}})).is_empty());

        let user = notification(
            "item/completed",
            &json!({"item":{"type":"userMessage","id":"u","content":[{"type":"skill","name":"skill-creator","path":"/s"},{"type":"text","text":"Reply ok","text_elements":[]}]}}),
        );
        assert_eq!(kinds(&user), [&EventKind::UserMessage { text: "/skill-creator\nReply ok".into() }]);
    }

    #[test]
    fn deltas_are_ephemeral_and_the_completed_message_is_persisted() {
        let d = notification(
            "item/agentMessage/delta",
            &json!({"itemId":"m","delta":"hi"}),
        );
        assert_eq!(d[0].durability, Durability::Ephemeral);
        assert_eq!(d[0].event_id, None);
        let c = notification(
            "item/completed",
            &json!({"item":{"type":"agentMessage","id":"m","text":"hi","phase":"final_answer"}}),
        );
        assert_eq!(c[0].durability, Durability::Persisted);
        assert_eq!(c[0].event_id.as_deref(), Some("m:completed"));
    }

    #[test]
    fn an_interrupted_turn_is_aborted_then_ended_then_idle() {
        let m = notification(
            "turn/completed",
            &json!({"turn":{"id":"t","status":"interrupted","items":[]}}),
        );
        assert_eq!(
            kinds(&m),
            [&EventKind::Aborted, &EventKind::TurnEnded, &EventKind::Idle]
        );
    }

    #[test]
    fn a_running_turn_in_history_has_no_end() {
        let h = history(&json!({"turns":[{"id":"t","status":"inProgress","items":[]}]}));
        assert_eq!(kinds(&h), [&EventKind::TurnStarted]);
        assert_eq!(
            running_turn(&json!({"turns":[{"id":"t","status":"inProgress"}]})).as_deref(),
            Some("t")
        );
    }

    #[test]
    fn noise_maps_to_nothing() {
        assert!(notification("mcpServer/startupStatus/updated", &json!({})).is_empty());
        assert!(notification("account/rateLimits/updated", &json!({})).is_empty());
    }

    #[test]
    fn output_is_bounded_on_a_character_boundary() {
        let s = "é".repeat(PAYLOAD_LIMIT);
        let b = bounded(&s, PAYLOAD_LIMIT);
        assert!(b.len() <= PAYLOAD_LIMIT + '…'.len_utf8());
    }

    #[test]
    fn a_command_approval_is_keyed_by_the_item_unless_an_approval_id_is_given() {
        let p = json!({"itemId":"exec-1","command":"ls -a","commandActions":[{"type":"listFiles","command":"ls -a"}],"availableDecisions":["accept",{"acceptWithExecpolicyAmendment":{"execpolicy_amendment":["ls"]}},"cancel"]});
        let (kind, req, avail) =
            permission_request("item/commandExecution/requestApproval", &p, None).unwrap();
        assert_eq!(kind, RequestKind::Command);
        assert_eq!(req.request_id, "exec-1");
        assert!(!req.can_offer_session_approval);
        assert_eq!(avail, ["accept", "acceptWithExecpolicyAmendment", "cancel"]);
        let p2 = json!({"itemId":"exec-1","approvalId":"ap-9","command":"ls"});
        assert_eq!(
            permission_request("item/commandExecution/requestApproval", &p2, None)
                .unwrap()
                .1
                .request_id,
            "ap-9"
        );
    }

    #[test]
    fn deny_declines_when_offered_cancels_when_not_and_approve_is_one_time() {
        let none = Value::Null;
        let offered = vec!["accept".to_string(), "decline".to_string()];
        let not_offered = vec!["accept".to_string(), "cancel".to_string()];
        assert_eq!(
            decision_result(RequestKind::Command, Answer::Refuse, &offered, &none),
            json!({"decision":"decline"})
        );
        assert_eq!(
            decision_result(RequestKind::Command, Answer::Refuse, &not_offered, &none),
            json!({"decision":"cancel"})
        );
        assert_eq!(
            decision_result(RequestKind::Command, Answer::Refuse, &[], &none),
            json!({"decision":"decline"})
        );
        assert_eq!(
            decision_result(RequestKind::FileChange, Answer::Accept, &offered, &none),
            json!({"decision":"accept"})
        );
    }

    #[test]
    fn approve_for_session_is_codexs_accept_for_session_where_it_is_offered() {
        let none = Value::Null;
        let with = vec!["accept".to_string(), "acceptForSession".to_string(), "decline".to_string()];
        let without = vec!["accept".to_string(), "decline".to_string()];
        let s = Answer::AcceptForSession;
        assert_eq!(decision_result(RequestKind::Command, s, &with, &none), json!({"decision":"acceptForSession"}));
        // Not offered: accepted once (radar remembers it for the session).
        assert_eq!(decision_result(RequestKind::Command, s, &without, &none), json!({"decision":"accept"}));
        // A file change lists no decisions: the schema's acceptForSession.
        assert_eq!(decision_result(RequestKind::FileChange, s, &[], &none), json!({"decision":"acceptForSession"}));
        let wanted = json!({"network": {"enabled": true}});
        assert_eq!(
            decision_result(RequestKind::Permissions, s, &[], &wanted),
            json!({"permissions": wanted, "scope": "session"})
        );
        let (_, req, _) = permission_request("item/fileChange/requestApproval", &json!({"itemId":"fc-1"}), None).unwrap();
        assert!(req.can_offer_session_approval, "a file change can be approved for the session");
        let (_, req, _) = permission_request(
            "item/commandExecution/requestApproval",
            &json!({"itemId":"c","command":"ls","availableDecisions":["accept","acceptForSession","cancel"]}),
            None,
        )
        .unwrap();
        assert!(req.can_offer_session_approval);
    }

    #[test]
    fn a_file_change_approval_shows_the_diff_the_item_announced() {
        let changes = json!([{"path":"a.txt","kind":{"type":"add"},"diff":"+hi"}]);
        let (_, req, _) = permission_request(
            "item/fileChange/requestApproval",
            &json!({"itemId":"fc-1"}),
            Some(&changes),
        )
        .unwrap();
        assert_eq!(
            req.target,
            PermissionTarget::Write {
                path: "a.txt".into(),
                diff: Some("+hi".into())
            }
        );
    }
}
